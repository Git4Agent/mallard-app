//! The website bridge for a project linked to a Mallard Cloud storage: the
//! two actions the desktop offers on its repository, Publish version and
//! the Owner's discard of the draft, and the reads that let a project be
//! connected to a repository by its website address: the account's
//! repositories, one address resolved, and one invitation code entered.
use super::service::repository;
use mallard_domain::{sha256_hex, BundleId, LocalProjectId, Mainline, RecordName, StorageId};
use mallard_infrastructure::managed_cloud::address::{parse_repository_address, AddressError};
use mallard_infrastructure::managed_cloud::client::{
    DiscardedDraft, ManagedCloudClient, PublicFilter, PublicSort, PublishVersion,
    RepositoryListing, RepositorySummary,
};
use mallard_infrastructure::managed_cloud::names;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_core::publication::{RecordStream, MAX_LISTED_RECORDS};
use mallard_infrastructure::storage_providers::managed_cloud::{client_for, provider_key};
use serde::Serialize;

/// How a cloud repository's Git remote compares with the local project's.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitRemoteMatch {
    /// The same remote, or both without one.
    Matches,
    /// A different remote, or one side without one.
    Differs,
    /// The repository withholds its remote (a public git project whose
    /// GitHub address the service has not confirmed).
    Unknown,
}

/// One repository as the address picker lists or resolves it: the address
/// and what connecting to it would mean. Never a bundle id in the copy; the
/// id rides along for the connect request.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ManagedCloudRepository {
    pub repository_id: String,
    pub owner_login: String,
    pub name: String,
    /// `{owner_login}/{name}`, the service's spelling.
    pub address: String,
    /// What a row prints: the display name of the head the account may
    /// read (the project's title as it was pushed), else the address
    /// segment. A display-name change travels with the next push and moves
    /// neither `name` nor `repository_id`.
    pub display_name: String,
    /// The repository's page on the website, the name percent-encoded.
    pub website_url: String,
    pub bundle_id: Option<String>,
    pub repository_class: String,
    pub visibility: String,
    /// The account's role; `None` when it is not a collaborator.
    pub role: Option<String>,
    /// Section 4.3: whether Push is accepted: a collaborator, or anyone
    /// signed in on a public repository.
    pub can_push: bool,
    /// The remote's fingerprint when it is known: the summary's for a
    /// collaborator, the digest of the disclosed GitHub address otherwise.
    pub repository_fingerprint: Option<String>,
    pub git_remote: GitRemoteMatch,
    /// The repository's GitHub address, `https://github.com/{owner}/{name}`,
    /// when the service has linked it: the only source a new project clones.
    pub github_url: Option<String>,
    pub revision: u64,
    pub updated_at: u64,
    pub resources: u64,
    pub registration_only: bool,
    /// The Owner's line, else GitHub's.
    pub description: Option<String>,
    pub star_count: u64,
    pub viewer_starred: bool,
}

/// What one pasted address resolves to.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManagedCloudRepositoryResolution {
    /// The account belongs to it: Pull and Push both work.
    Collaborator { repository: ManagedCloudRepository },
    /// A public repository the account does not belong to: anyone signed
    /// in pushes to it (section 4.3), so Pull and Push both work here too.
    Public { repository: ManagedCloudRepository },
    /// Nothing at that address the account may see.
    NotFound { address: String },
    /// The address is a skill, plugin intent, or pet, not a project.
    NotAProject { repository: ManagedCloudRepository },
    /// The address names another Mallard Cloud host than this storage's.
    WrongService { host: String, expected: String },
    /// A `github.com` address.
    GitHub,
    /// Not an address.
    Invalid,
}

/// The remote fingerprint the service lets this caller know: the
/// collaborator's summary carries it; the public face withholds it but
/// discloses the canonical GitHub address, whose SHA-256 is the same
/// value; a non-git project has none. `Err(())` when it is withheld.
fn known_remote_fingerprint(summary: &RepositorySummary) -> Result<Option<String>, ()> {
    if let Some(fingerprint) = &summary.repository_fingerprint {
        return Ok(Some(fingerprint.clone()));
    }
    if let Some(url) = &summary.github_url {
        return Ok(Some(sha256_hex(url.as_bytes())));
    }
    if summary.repository_class == "non_git_project" {
        return Ok(None);
    }
    Err(())
}

fn describe_repository(
    origin: &str,
    summary: &RepositorySummary,
    local_fingerprint: Option<&str>,
) -> ManagedCloudRepository {
    let remote = known_remote_fingerprint(summary);
    let git_remote = match &remote {
        Ok(remote) => {
            if remote.as_deref() == local_fingerprint {
                GitRemoteMatch::Matches
            } else {
                GitRemoteMatch::Differs
            }
        }
        Err(()) => GitRemoteMatch::Unknown,
    };
    let tag = summary
        .private_summary
        .as_ref()
        .or(summary.public_summary.as_ref());
    let head = summary
        .private_head
        .as_ref()
        .or(summary.public_head.as_ref());
    ManagedCloudRepository {
        repository_id: summary.id.clone(),
        owner_login: summary.owner.github_login.clone(),
        name: summary.name.clone(),
        address: format!("{}/{}", summary.owner.github_login, summary.name),
        display_name: tag
            .map(|tag| tag.display_name.clone())
            .filter(|display_name| !display_name.is_empty())
            .unwrap_or_else(|| summary.name.clone()),
        website_url: format!(
            "{origin}/{}/{}",
            names::encode_segment(&summary.owner.github_login),
            names::encode_segment(&summary.name)
        ),
        bundle_id: summary.bundle_id.clone(),
        repository_class: summary.repository_class.clone(),
        visibility: summary.visibility.clone(),
        role: summary.role.clone(),
        can_push: summary.writer,
        repository_fingerprint: remote.unwrap_or_default(),
        git_remote,
        github_url: summary.github_url.clone(),
        revision: tag.map(|tag| tag.revision).unwrap_or(0),
        updated_at: head
            .map(|head| head.updated_at)
            .or(tag.map(|tag| tag.updated_at))
            .unwrap_or(0),
        resources: tag.map(|tag| tag.resources).unwrap_or(0),
        registration_only: tag.map(|tag| tag.registration_only).unwrap_or(true),
        description: summary.blurb(),
        star_count: summary.star_count,
        viewer_starred: summary.viewer_starred,
    }
}

/// The project repositories the account starred on the website, one page
/// of its Starred list, the Catalog items left out.
pub async fn list_starred_projects(
    client: &ManagedCloudClient,
    cursor: Option<&str>,
) -> Result<RepositoryListing, String> {
    let mut page = client.list_starred(cursor).await?;
    page.rows.retain(|row| row.repository_class != "catalog");
    Ok(page)
}

/// The released projects, newest first or by stars, one page at a time.
pub async fn list_public_projects(
    client: &ManagedCloudClient,
    sort: PublicSort,
    cursor: Option<&str>,
) -> Result<RepositoryListing, String> {
    let filter = PublicFilter {
        class: "project",
        provider: None,
        kind: None,
        sort,
    };
    Ok(client.list_public(&filter, cursor).await?)
}

/// The project repositories the account belongs to, newest first.
pub async fn list_repositories(
    client: &ManagedCloudClient,
    origin: &str,
    local_fingerprint: Option<&str>,
) -> Result<Vec<ManagedCloudRepository>, String> {
    let mut rows = client
        .list_repositories("project")
        .await?
        .iter()
        .filter(|summary| summary.bundle_id.is_some())
        .map(|summary| describe_repository(origin, summary, local_fingerprint))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.address.cmp(&right.address))
    });
    Ok(rows)
}

pub async fn resolve_repository(
    client: &ManagedCloudClient,
    origin: &str,
    address: &str,
    local_fingerprint: Option<&str>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let parsed = match parse_repository_address(address, origin) {
        Ok(parsed) => parsed,
        Err(AddressError::WrongService { host, expected }) => {
            return Ok(ManagedCloudRepositoryResolution::WrongService { host, expected });
        }
        Err(AddressError::GitHub) => return Ok(ManagedCloudRepositoryResolution::GitHub),
        Err(AddressError::Invalid) => return Ok(ManagedCloudRepositoryResolution::Invalid),
    };
    let Some(summary) = client
        .repository_by_name(&parsed.owner, &parsed.name)
        .await?
    else {
        return Ok(ManagedCloudRepositoryResolution::NotFound {
            address: parsed.to_string(),
        });
    };
    let repository = describe_repository(origin, &summary, local_fingerprint);
    Ok(
        if summary.repository_class == "catalog" || summary.bundle_id.is_none() {
            ManagedCloudRepositoryResolution::NotAProject { repository }
        } else if summary.role.is_some() {
            ManagedCloudRepositoryResolution::Collaborator { repository }
        } else {
            ManagedCloudRepositoryResolution::Public { repository }
        },
    )
}

/// Section 4.3: enter a private repository's invitation code. The account
/// joins it as a Member and the repository comes back described the way a
/// resolved address is, so the picker selects it.
pub async fn accept_repository_invitation(
    client: &ManagedCloudClient,
    origin: &str,
    code: &str,
    local_fingerprint: Option<&str>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("Enter the invitation code.".to_string());
    }
    let summary = client.accept_invitation(code).await?;
    let repository = describe_repository(origin, &summary, local_fingerprint);
    Ok(
        if summary.repository_class == "catalog" || summary.bundle_id.is_none() {
            ManagedCloudRepositoryResolution::NotAProject { repository }
        } else {
            ManagedCloudRepositoryResolution::Collaborator { repository }
        },
    )
}

/// The client and origin of a Mallard Cloud storage.
pub fn storage_client(storage_id: &StorageId) -> Result<(ManagedCloudClient, String), String> {
    let config = repository()?.load_config()?;
    let Some(storage) = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
    else {
        return Err("This storage no longer exists.".to_string());
    };
    if storage.provider != provider_key() {
        return Err("This storage is not Mallard Cloud.".to_string());
    }
    let (settings, client) = client_for(&storage.provider_config)?;
    Ok((client, settings.service_origin))
}

pub fn valid_local_fingerprint(value: &Option<String>) -> Result<Option<&str>, String> {
    match value.as_deref() {
        None => Ok(None),
        Some(text) if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(Some(text))
        }
        Some(_) => Err("repository fingerprint must be a SHA-256 hex digest".to_string()),
    }
}

struct Target {
    client: ManagedCloudClient,
    repository_id: String,
    /// The project's bundle, whose record stream the repository holds.
    bundle_id: BundleId,
}

/// The repository a link's action names: the link's pin, exactly. `None`
/// on a storage that is not Mallard Cloud.
fn target(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<Option<Target>, String> {
    let config = repository.load_config()?;
    let Some(storage) = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
    else {
        return Err("This storage no longer exists.".to_string());
    };
    if storage.provider != provider_key() {
        return Ok(None);
    }
    let Some(link) = config
        .links
        .iter()
        .find(|link| &link.local_project_id == local_project_id && &link.storage_id == storage_id)
    else {
        return Err("This project is not linked to the storage.".to_string());
    };
    let (_, client) = client_for(&storage.provider_config)?;
    Ok(Some(Target {
        client,
        repository_id: link
            .managed_cloud_repository_id
            .clone()
            .ok_or("Cloud link has no repository ID. Reconnect the repository.")?,
        bundle_id: link.bundle_id.clone(),
    }))
}

/// What Publish answered: the version the draft was frozen as.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ManagedCloudPublishedVersion {
    pub repository_id: String,
    pub version: u64,
}

/// The service refuses a longer name.
const VERSION_NAME_CHARS: usize = 80;

/// The draft of a repository as one listing of the project's stream shows
/// it: the selected record, and the repository revision the listing carried.
async fn listed_draft(target: &Target) -> Result<(RecordName, u64), String> {
    let stream = RecordStream::project(&target.bundle_id)?;
    let prefix = stream.prefix().as_str();
    let mut names = Vec::new();
    let mut revision = None;
    let mut cursor: Option<String> = None;
    loop {
        let page = target
            .client
            .list_keys(&target.repository_id, prefix, cursor.as_deref())
            .await?;
        if revision.is_none() {
            revision = page.link.map(|link| link.repository_revision);
        }
        names.extend(page.keys.iter().filter_map(|key| {
            let name = key.strip_prefix(prefix)?.strip_prefix('/')?;
            RecordName::parse(name)
        }));
        if names.len() > MAX_LISTED_RECORDS {
            return Err(
                "publication listing exceeds its object limit; the view is incomplete".to_string(),
            );
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let revision = revision.ok_or("Mallard Cloud omitted the repository revision")?;
    let draft = Mainline::select(names)
        .selected()
        .cloned()
        .ok_or("Nothing is pushed to Mallard Cloud yet.")?;
    Ok((draft, revision))
}

/// Publish: freeze the draft as the next version, for the team. One listing
/// of the project's stream gives the draft and the repository revision; one
/// request names both, and is never sent again. Nothing becomes public, and
/// nothing on this computer is read, written or deleted.
pub async fn publish_version(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    name: Option<&str>,
) -> Result<ManagedCloudPublishedVersion, String> {
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    if name.is_some_and(|name| name.chars().count() > VERSION_NAME_CHARS) {
        return Err(format!(
            "A version's name holds at most {VERSION_NAME_CHARS} characters."
        ));
    }
    let Some(target) = target(repository, local_project_id, storage_id)? else {
        return Err("This storage is not Mallard Cloud.".to_string());
    };
    let (draft, expected_repository_revision) = listed_draft(&target).await?;
    let published = target
        .client
        .publish_version(
            &target.repository_id,
            &PublishVersion {
                commit_id: draft.commit_id(),
                name,
                expected_repository_revision,
            },
        )
        .await?;
    Ok(ManagedCloudPublishedVersion {
        repository_id: target.repository_id,
        version: published.version,
    })
}

/// One request to the service, which does the removing: every push after
/// the latest version goes from Mallard Cloud, and the draft is that
/// version again. Nothing on this computer is read, written or deleted; the
/// link's records of the removed pushes are stale from here on and read as
/// absent at the next comparison.
pub async fn discard_draft(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    expected_repository_revision: u64,
) -> Result<DiscardedDraft, String> {
    let Some(target) = target(repository, local_project_id, storage_id)? else {
        return Err("This storage is not Mallard Cloud.".to_string());
    };
    Ok(target
        .client
        .discard_private_snapshot(&target.repository_id, expected_repository_revision)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> RepositorySummary {
        serde_json::from_value(serde_json::json!({
            "id": "repo_1", "owner": {"user_id": "user_1", "github_login": "octocat"}, "name": "Project", "repository_class": "git_project", "bundle_id": "b".repeat(32),
            "revision": 7, "state": "active", "visibility": "private", "writer": true, "public_version": null,
            "private_head": {
                "scope": "private", "revision": 3, "commit_id": "commit-aaaaaaaa", "manifest_sha256": "a".repeat(64),
                "manifest_size": 10, "updated_at": 500, "kind": "project", "bundle_id": "b".repeat(32)
            },
            "private_summary": {"display_name": "Project", "kind": "project", "revision": 3, "updated_at": 500, "resources": 1, "files": 1, "registration_only": false, "providers": ["codex"]},
        }))
        .unwrap()
    }

    fn public_summary(github_url: Option<&str>) -> RepositorySummary {
        serde_json::from_value(serde_json::json!({
            "id": "repo_2", "owner": {"user_id": "user_9", "github_login": "rikaqu0223-arch"}, "name": "ledger-api",
            "repository_class": "git_project", "bundle_id": "c".repeat(32), "repository_fingerprint": null, "github_url": github_url,
            "revision": 3, "state": "active", "visibility": "public", "public_version": 2, "role": null, "writer": false,
            "public_head": {"scope": "version", "version": 2, "revision": 2, "commit_id": "commit-pub", "manifest_sha256": "a".repeat(64), "manifest_size": 10, "updated_at": 900, "kind": "project", "bundle_id": "c".repeat(32)},
            "public_summary": {"display_name": "ledger-api", "kind": "project", "revision": 2, "updated_at": 900, "resources": 1, "files": 1, "registration_only": false, "providers": []},
        }))
        .unwrap()
    }

    #[test]
    fn a_repository_is_described_by_address_and_remote_match() {
        let origin = "https://mallard-ai.com";
        let mine = summary();
        let local = "a".repeat(64);
        let described = describe_repository(origin, &mine, None);
        assert_eq!(described.address, "octocat/Project");
        assert_eq!(
            described.website_url,
            "https://mallard-ai.com/octocat/Project"
        );
        assert_eq!(
            described.git_remote,
            GitRemoteMatch::Unknown,
            "a git project that withholds its fingerprint and has no GitHub address"
        );
        assert_eq!(
            (
                described.revision,
                described.updated_at,
                described.resources
            ),
            (3, 500, 1)
        );
        let with_fingerprint = |fingerprint: &str| RepositorySummary {
            repository_fingerprint: Some(fingerprint.to_owned()),
            ..summary()
        };
        assert_eq!(
            describe_repository(origin, &with_fingerprint(&local), Some(&local)).git_remote,
            GitRemoteMatch::Matches
        );
        assert_eq!(
            describe_repository(origin, &with_fingerprint(&local), Some(&"b".repeat(64)))
                .git_remote,
            GitRemoteMatch::Differs
        );
        assert_eq!(
            describe_repository(origin, &with_fingerprint(&local), None).git_remote,
            GitRemoteMatch::Differs,
            "a git project against a folder without a remote"
        );

        // The public face withholds the fingerprint but discloses the GitHub
        // address, whose digest is the fingerprint the checkout computes.
        let url = "https://github.com/rikaqu0223-arch/ledger-api";
        let checkout = sha256_hex(url.as_bytes());
        let public = describe_repository(origin, &public_summary(Some(url)), Some(&checkout));
        assert_eq!(public.git_remote, GitRemoteMatch::Matches);
        assert_eq!(
            public.repository_fingerprint.as_deref(),
            Some(checkout.as_str())
        );
        assert_eq!(public.role, None);
        assert!(
            !public.can_push,
            "the anonymous public face is not a writer"
        );
        assert_eq!(public.updated_at, 900);
        let other = describe_repository(origin, &public_summary(Some(url)), Some(&local));
        assert_eq!(other.git_remote, GitRemoteMatch::Differs);
        let withheld = describe_repository(origin, &public_summary(None), Some(&local));
        assert_eq!(withheld.git_remote, GitRemoteMatch::Unknown);
        assert_eq!(withheld.repository_fingerprint, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_name_of_any_script_is_created_resolved_and_suffixed_by_key() {
        use mallard_infrastructure::managed_cloud::client::CreateRepository;
        use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
        use unicode_normalization::UnicodeNormalization;
        let stub = StubManagedCloud::start().await;
        let (_, owner_token) = stub.state.lock().unwrap().add_user("Rika321");
        let client = ManagedCloudClient::new(
            &stub.endpoint,
            Some(mallard_domain::SecretValue::parse(&owner_token).unwrap()),
        )
        .unwrap();
        let origin = stub.endpoint.clone();
        let create = |name: &str, bundle: char| CreateRepository {
            repository_class: "non_git_project",
            github_url: None,
            name: name.into(),
            bundle_id: Some(bundle.to_string().repeat(32)),
            repository_fingerprint: None,
            catalog: None,
        };
        // The title is kept in its script; a decomposed spelling of a held
        // name is the same key and gets the suffix; nothing usable is refused.
        let created = client
            .create_repository(&create("大黄", 'a'))
            .await
            .unwrap();
        assert_eq!(created.name, "大黄");
        let second = client
            .create_repository(&create("Caf\u{e9}", 'b'))
            .await
            .unwrap();
        assert_eq!(second.name, "Caf\u{e9}");
        let third = client
            .create_repository(&create("cafe\u{301}", 'c'))
            .await
            .unwrap();
        assert_eq!(third.name, "cafe\u{301}-2".nfc().collect::<String>());
        assert!(client
            .create_repository(&create("🦆🦆", 'd'))
            .await
            .is_err());

        // The raw and the encoded spelling resolve to the one repository,
        // whose page link is encoded and whose row name is the title.
        for input in [
            "Rika321/大黄".to_owned(),
            "Rika321/%E5%A4%A7%E9%BB%84".to_owned(),
            format!("{origin}/rika321/%E5%A4%A7%E9%BB%84/"),
        ] {
            let resolved = resolve_repository(&client, &origin, &input, None)
                .await
                .unwrap();
            let ManagedCloudRepositoryResolution::Collaborator { repository } = resolved else {
                panic!("{input}: {resolved:?}");
            };
            assert_eq!(repository.repository_id, created.id, "{input}");
            assert_eq!(repository.address, "Rika321/大黄");
            assert_eq!(repository.display_name, "大黄");
            assert_eq!(
                repository.website_url,
                format!("{origin}/Rika321/%E5%A4%A7%E9%BB%84")
            );
        }
        let ManagedCloudRepositoryResolution::Collaborator { repository } =
            resolve_repository(&client, &origin, "Rika321/CAFE%CC%81", None)
                .await
                .unwrap()
        else {
            panic!("a decomposed, upper-cased spelling is the same key");
        };
        assert_eq!(repository.repository_id, second.id);
    }

    #[tokio::test]
    async fn an_address_resolves_to_the_face_the_account_may_see() {
        use mallard_infrastructure::managed_cloud::client::CreateRepository;
        use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
        let stub = StubManagedCloud::start().await;
        let (owner, owner_token) = stub.state.lock().unwrap().add_user("Rika321");
        let (_, guest_token) = stub.state.lock().unwrap().add_user("guest");
        let client = |token: &str| {
            ManagedCloudClient::new(
                &stub.endpoint,
                Some(mallard_domain::SecretValue::parse(token).unwrap()),
            )
            .unwrap()
        };
        let origin = stub.endpoint.clone();
        let github = "https://github.com/Rika321/HorrorGame";
        let fingerprint = sha256_hex(github.as_bytes());
        let created = client(&owner_token)
            .create_repository(&CreateRepository {
                repository_class: "git_project",
                github_url: None,
                name: "Horror Game".into(),
                bundle_id: Some("b".repeat(32)),
                repository_fingerprint: Some(fingerprint.clone()),
                catalog: None,
            })
            .await
            .unwrap();
        assert_eq!(created.name, "Horror-Game");

        // The owner, by every spelling, including the page address.
        let resolved = resolve_repository(
            &client(&owner_token),
            &origin,
            &format!("{origin}/rika321/horror-game/sessions/x?commit=1"),
            Some(&fingerprint),
        )
        .await
        .unwrap();
        let ManagedCloudRepositoryResolution::Collaborator { repository } = resolved else {
            panic!("owner resolves to a collaborator: {resolved:?}");
        };
        assert_eq!(
            repository.address, "Rika321/Horror-Game",
            "the service's spelling"
        );
        assert_eq!(repository.role.as_deref(), Some("owner"));
        assert_eq!(repository.git_remote, GitRemoteMatch::Matches);
        assert_eq!(
            repository.bundle_id.as_deref(),
            Some("b".repeat(32).as_str())
        );
        assert!(repository.registration_only, "nothing pushed yet");

        // A stranger sees nothing while it is private …
        let guest = client(&guest_token);
        assert_eq!(
            resolve_repository(&guest, &origin, "Rika321/Horror-Game", None)
                .await
                .unwrap(),
            ManagedCloudRepositoryResolution::NotFound {
                address: "Rika321/Horror-Game".into()
            }
        );
        // … and the public face once it is released, with the remote
        // verified through the disclosed GitHub address.
        {
            use mallard_infrastructure::managed_cloud::stub::Publication;
            let mut state = stub.state.lock().unwrap();
            let manifest = b"{}";
            let record_name = format!(
                "0000000001.commit-{}-{}.root.0000000000700.json",
                "1".repeat(24),
                "2".repeat(64)
            );
            let publication = Publication {
                record_name: record_name.clone(),
                commit_id: format!("commit-{}", "1".repeat(24)),
                manifest_revision: 1,
                manifest_key: format!("_manifests/1-commit-{}.json", "1".repeat(24)),
                manifest_sha256: sha256_hex(manifest),
                manifest_size: manifest.len() as u64,
                summary: serde_json::json!({"display_name": "Horror-Game", "kind": "project", "revision": 1, "updated_at": 700, "resources": 0, "files": 0, "registration_only": true, "providers": []}),
                created_by: owner.id.clone(),
                created_at: 700,
                state: "available".into(),
            };
            let repository = state.repositories.get_mut(&created.id).unwrap();
            repository.publications.push(publication);
            repository.private_head = Some(record_name);
            repository.private_revision = 1;
            state.release(&created.id, Some(github));
        }
        let stranger =
            resolve_repository(&guest, &origin, "Rika321/Horror-Game", Some(&fingerprint))
                .await
                .unwrap();
        let ManagedCloudRepositoryResolution::Public { repository } = stranger else {
            panic!("a released repository shows its public face: {stranger:?}");
        };
        assert_eq!(repository.role, None);
        assert!(
            repository.can_push,
            "anyone signed in pushes to a public repository (section 4.3)"
        );
        assert_eq!(repository.visibility, "public");
        assert_eq!(repository.git_remote, GitRemoteMatch::Matches);
        assert_eq!(
            repository.repository_fingerprint.as_deref(),
            Some(fingerprint.as_str())
        );
        // The disclosed address is what a new project clones.
        assert_eq!(repository.github_url.as_deref(), Some(github));
        assert_eq!(repository.updated_at, 700);
        // With another checkout the disclosed remote differs.
        let other = resolve_repository(
            &guest,
            &origin,
            "Rika321/Horror-Game",
            Some(&"d".repeat(64)),
        )
        .await
        .unwrap();
        assert!(matches!(
            other,
            ManagedCloudRepositoryResolution::Public {
                repository: ManagedCloudRepository {
                    git_remote: GitRemoteMatch::Differs,
                    ..
                }
            }
        ));

        // Refusals happen before any request.
        let before = stub.requests().len();
        assert_eq!(
            resolve_repository(&guest, &origin, "https://staging.mallard-ai.com/a/b", None)
                .await
                .unwrap(),
            ManagedCloudRepositoryResolution::WrongService {
                host: "staging.mallard-ai.com".into(),
                expected: origin.trim_start_matches("http://").to_owned(),
            }
        );
        assert_eq!(
            resolve_repository(
                &guest,
                &origin,
                "https://github.com/Rika321/HorrorGame",
                None
            )
            .await
            .unwrap(),
            ManagedCloudRepositoryResolution::GitHub
        );
        assert_eq!(
            resolve_repository(&guest, &origin, "HorrorGame", None)
                .await
                .unwrap(),
            ManagedCloudRepositoryResolution::Invalid
        );
        assert_eq!(
            stub.requests().len(),
            before,
            "no request for a refused address"
        );

        // The listing names the same repository for its owner and none for the stranger.
        let listed = list_repositories(&client(&owner_token), &origin, Some(&fingerprint))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].address, "Rika321/Horror-Game");
        assert_eq!(listed[0].git_remote, GitRemoteMatch::Matches);
        assert!(listed[0].can_push);
        assert!(list_repositories(&guest, &origin, None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_invitation_code_joins_a_private_repository_and_selects_it() {
        use mallard_infrastructure::managed_cloud::client::CreateRepository;
        use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
        let stub = StubManagedCloud::start().await;
        let (owner, owner_token) = stub.state.lock().unwrap().add_user("Tom");
        let (_, guest_token) = stub.state.lock().unwrap().add_user("Jeff");
        let client = |token: &str| {
            ManagedCloudClient::new(
                &stub.endpoint,
                Some(mallard_domain::SecretValue::parse(token).unwrap()),
            )
            .unwrap()
        };
        let origin = stub.endpoint.clone();
        let created = client(&owner_token)
            .create_repository(&CreateRepository {
                repository_class: "non_git_project",
                github_url: None,
                name: "B".into(),
                bundle_id: Some("e".repeat(32)),
                repository_fingerprint: None,
                catalog: None,
            })
            .await
            .unwrap();
        let guest = client(&guest_token);

        // Nothing to see, and a wrong code is refused with the service's sentence.
        assert_eq!(
            resolve_repository(&guest, &origin, "Tom/B", None)
                .await
                .unwrap(),
            ManagedCloudRepositoryResolution::NotFound {
                address: "Tom/B".into()
            }
        );
        let refused =
            accept_repository_invitation(&guest, &origin, "AAAAA-AAAAA-AAAAA-AAAAB", None)
                .await
                .unwrap_err();
        assert!(refused.contains("unknown or was revoked"), "{refused}");
        assert!(accept_repository_invitation(&guest, &origin, "  ", None)
            .await
            .is_err());

        // Tom sets B's code; Jeff enters it, spaced and lower-cased, and is a Member.
        let code = stub
            .state
            .lock()
            .unwrap()
            .set_invite_code(&created.id, &owner.id);
        let joined = accept_repository_invitation(
            &guest,
            &origin,
            &code.to_lowercase().replace('-', " "),
            None,
        )
        .await
        .unwrap();
        let ManagedCloudRepositoryResolution::Collaborator { ref repository } = joined else {
            panic!("the code joins as a collaborator: {joined:?}");
        };
        assert_eq!(repository.address, "Tom/B");
        assert_eq!(repository.role.as_deref(), Some("member"));
        assert!(repository.can_push);
        assert_eq!(repository.git_remote, GitRemoteMatch::Matches);
        // Entering it again changes nothing, and the address now resolves.
        let again = accept_repository_invitation(&guest, &origin, &code, None)
            .await
            .unwrap();
        assert_eq!(again, joined);
        assert!(matches!(
            resolve_repository(&guest, &origin, "tom/b", None)
                .await
                .unwrap(),
            ManagedCloudRepositoryResolution::Collaborator { .. }
        ));
        assert_eq!(
            list_repositories(&guest, &origin, None)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn starred_and_public_projects_list_as_rows_that_a_pick_resolves() {
        use mallard_infrastructure::managed_cloud::client::{CreateRepository, PublicSort};
        use mallard_infrastructure::managed_cloud::stub::{Publication, StubManagedCloud};
        let stub = StubManagedCloud::start().await;
        let (owner, owner_token) = stub.state.lock().unwrap().add_user("Rika321");
        let (guest, guest_token) = stub.state.lock().unwrap().add_user("guest");
        let client = |token: &str| {
            ManagedCloudClient::new(
                &stub.endpoint,
                Some(mallard_domain::SecretValue::parse(token).unwrap()),
            )
            .unwrap()
        };
        let origin = stub.endpoint.clone();
        let github = "https://github.com/Rika321/Portal";
        let created = client(&owner_token)
            .create_repository(&CreateRepository {
                repository_class: "git_project",
                github_url: None,
                name: "Portal".into(),
                bundle_id: Some("b".repeat(32)),
                repository_fingerprint: Some(sha256_hex(github.as_bytes())),
                catalog: None,
            })
            .await
            .unwrap();
        // Nothing public yet: the guest's listings are empty.
        let guest_client = client(&guest_token);
        assert!(list_starred_projects(&guest_client, None)
            .await
            .unwrap()
            .rows
            .is_empty());
        assert!(
            list_public_projects(&guest_client, PublicSort::Newest, None)
                .await
                .unwrap()
                .rows
                .is_empty()
        );
        // Released, described, and starred by the guest.
        {
            let mut state = stub.state.lock().unwrap();
            let manifest = b"{}";
            let record_name = format!(
                "0000000001.commit-{}-{}.root.0000000000700.json",
                "1".repeat(24),
                "2".repeat(64)
            );
            let publication = Publication {
                record_name: record_name.clone(),
                commit_id: format!("commit-{}", "1".repeat(24)),
                manifest_revision: 1,
                manifest_key: format!("_manifests/1-commit-{}.json", "1".repeat(24)),
                manifest_sha256: sha256_hex(manifest),
                manifest_size: manifest.len() as u64,
                summary: serde_json::json!({"display_name": "Portal", "kind": "project", "revision": 1, "updated_at": 700, "resources": 2, "files": 2, "registration_only": false, "providers": ["codex"]}),
                created_by: owner.id.clone(),
                created_at: 700,
                state: "available".into(),
            };
            let repository = state.repositories.get_mut(&created.id).unwrap();
            repository.publications.push(publication);
            repository.private_head = Some(record_name);
            repository.private_revision = 1;
            state.release(&created.id, Some(github));
            state.describe(&created.id, "Customer portal.");
            state.star(&guest.id, &created.id);
        }
        let starred = list_starred_projects(&guest_client, None).await.unwrap();
        assert_eq!(starred.rows.len(), 1);
        let row = &starred.rows[0];
        assert_eq!(row.address, "Rika321/Portal");
        assert_eq!(row.website_url, format!("{origin}/Rika321/Portal"));
        assert_eq!(row.repository_class, "git_project");
        assert_eq!(row.visibility, "public");
        assert_eq!(row.role, None, "a star is not a membership");
        assert_eq!(row.github_url.as_deref(), Some(github));
        assert_eq!(row.description.as_deref(), Some("Customer portal."));
        assert_eq!((row.star_count, row.viewer_starred), (1, true));
        assert_eq!((row.revision, row.updated_at), (1, 700));
        assert_eq!(starred.next_cursor, None);
        let public = list_public_projects(&guest_client, PublicSort::Stars, None)
            .await
            .unwrap();
        assert_eq!(
            public
                .rows
                .iter()
                .map(|row| row.address.as_str())
                .collect::<Vec<_>>(),
            vec!["Rika321/Portal"]
        );
        assert_eq!(public.rows[0].role, None);
        // The owner's own row carries the line and the count, not the star.
        let mine = list_repositories(&client(&owner_token), &origin, None)
            .await
            .unwrap();
        assert_eq!(mine[0].description.as_deref(), Some("Customer portal."));
        assert_eq!((mine[0].star_count, mine[0].viewer_starred), (1, false));
        // A listed row is picked by resolving its address: the public face.
        let picked = resolve_repository(&guest_client, &origin, &row.address, None)
            .await
            .unwrap();
        let ManagedCloudRepositoryResolution::Public { repository } = picked else {
            panic!("a public row resolves to its public face: {picked:?}");
        };
        assert_eq!(
            repository.bundle_id.as_deref(),
            Some("b".repeat(32).as_str())
        );
        assert_eq!(repository.description.as_deref(), Some("Customer portal."));
        assert!(repository.viewer_starred);
    }
}

//! The website bridge for a project linked to a Mallard Cloud storage: the
//! repository's page and where its release stands, read from the service
//! on demand, the two actions the desktop offers, a release request and
//! the Owner's discard of a draft over a public version, and the reads
//! that let a project be connected to a repository by its website address:
//! the account's repositories, one address resolved, and one invitation
//! code entered.
use super::service::repository;
use mallard_domain::{sha256_hex, LocalProjectId, StorageId};
use mallard_infrastructure::managed_cloud::address::{parse_repository_address, AddressError};
use mallard_infrastructure::managed_cloud::client::{
    DiscardedDraft, ManagedCloudClient, RepositorySummary,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::managed_cloud::{client_for, provider_key};
use serde::Serialize;

use mallard_infrastructure::managed_cloud::client::ManagedCloudLink;

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
        website_url: format!("{origin}/{}/{}", summary.owner.github_login, summary.name),
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
    }
}

/// The project repositories the account belongs to, newest first.
async fn list_repositories(
    client: &ManagedCloudClient,
    origin: &str,
    local_fingerprint: Option<&str>,
) -> Result<Vec<ManagedCloudRepository>, String> {
    let mut rows = client
        .list_project_repositories()
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

async fn resolve_repository(
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
async fn accept_repository_invitation(
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
fn storage_client(storage_id: &StorageId) -> Result<(ManagedCloudClient, String), String> {
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

fn valid_local_fingerprint(value: &Option<String>) -> Result<Option<&str>, String> {
    match value.as_deref() {
        None => Ok(None),
        Some(text) if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(Some(text))
        }
        Some(_) => Err("repository fingerprint must be a SHA-256 hex digest".to_string()),
    }
}

/// The project repositories the account belongs to on a Mallard Cloud
/// storage, each compared with the local project's Git remote.
#[tauri::command]
pub async fn list_managed_cloud_repositories(
    storage_id: StorageId,
    local_repository_fingerprint: Option<String>,
) -> Result<Vec<ManagedCloudRepository>, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    list_repositories(&client, &origin, local).await
}

/// One pasted address, resolved against a Mallard Cloud storage: what is
/// there, whether the account belongs to it, and how its Git remote
/// compares with the local project's.
#[tauri::command]
pub async fn resolve_managed_cloud_repository(
    storage_id: StorageId,
    address: String,
    local_repository_fingerprint: Option<String>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    resolve_repository(&client, &origin, &address, local).await
}

/// One invitation code entered against a Mallard Cloud storage: the
/// account joins the private repository the code belongs to, which comes
/// back the way `resolve_managed_cloud_repository` describes one.
#[tauri::command]
pub async fn accept_managed_cloud_invitation(
    storage_id: StorageId,
    code: String,
    local_repository_fingerprint: Option<String>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    accept_repository_invitation(&client, &origin, &code, local).await
}

struct Target {
    client: ManagedCloudClient,
    repository_id: String,
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
    }))
}

/// Ask the Owner (and then Super Admin) to make the current private
/// snapshot public. Names the repository revision the link showed.
#[tauri::command]
pub async fn request_managed_cloud_release(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    expected_repository_revision: u64,
) -> Result<ManagedCloudLink, String> {
    let Some(target) = target(&repository()?, &local_project_id, &storage_id)? else {
        return Err("This storage is not Mallard Cloud.".to_string());
    };
    target
        .client
        .create_release_request(&target.repository_id, expected_repository_revision)
        .await?;
    let summary = target.client.get_repository(&target.repository_id).await?;
    Ok(target.client.link_from_summary(&summary))
}

/// One request to the service, which does the removing: every push after
/// the public version goes from Mallard Cloud, and the draft is the public
/// version again. Nothing on this computer is read, written or deleted; the
/// link's records of the removed pushes are stale from here on and read as
/// absent at the next comparison.
pub(super) async fn discard_draft(
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

/// The Owner's discard of a draft over a public version, confirmed in the
/// workspace. Names the repository by the link's pin and the repository
/// revision the status showed.
#[tauri::command]
pub async fn discard_managed_cloud_draft(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    expected_repository_revision: u64,
) -> Result<DiscardedDraft, String> {
    discard_draft(
        &repository()?,
        &local_project_id,
        &storage_id,
        expected_repository_revision,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(
        open: Option<&str>,
        public_commit: Option<&str>,
        registration_only: bool,
    ) -> RepositorySummary {
        let head = |commit: &str| {
            serde_json::json!({
                "scope": "private", "revision": 3, "commit_id": commit, "manifest_sha256": "a".repeat(64),
                "manifest_size": 10, "updated_at": 500, "kind": "project", "bundle_id": "b".repeat(32)
            })
        };
        serde_json::from_value(serde_json::json!({
            "id": "repo_1", "owner": {"user_id": "user_1", "github_login": "octocat"}, "name": "Project", "repository_class": "git_project", "bundle_id": "b".repeat(32),
            "revision": 7, "state": "active", "visibility": if public_commit.is_some() { "public" } else { "private" }, "writer": true,
            "private_revision": 3, "public_revision": 1,
            "private_head": head("commit-aaaaaaaa"),
            "public_head": public_commit.map(head),
            "private_summary": {"display_name": "Project", "kind": "project", "revision": 3, "updated_at": 500, "resources": 1, "files": 1, "registration_only": registration_only, "providers": ["codex"]},
            "open_release_request": open.map(|state| serde_json::json!({"id": "rel_1", "state": state, "requested_by": "user_2", "created_at": 600})),
        }))
        .unwrap()
    }

    fn public_summary(github_url: Option<&str>) -> RepositorySummary {
        serde_json::from_value(serde_json::json!({
            "id": "repo_2", "owner": {"user_id": "user_9", "github_login": "rikaqu0223-arch"}, "name": "ledger-api",
            "repository_class": "git_project", "bundle_id": "c".repeat(32), "repository_fingerprint": null, "github_url": github_url,
            "revision": 3, "state": "active", "visibility": "public", "private_revision": 0, "public_revision": 2, "role": null, "writer": false,
            "public_head": {"scope": "public", "revision": 2, "commit_id": "commit-pub", "manifest_sha256": "a".repeat(64), "manifest_size": 10, "updated_at": 900, "kind": "project", "bundle_id": "c".repeat(32)},
            "public_summary": {"display_name": "ledger-api", "kind": "project", "revision": 2, "updated_at": 900, "resources": 1, "files": 1, "registration_only": false, "providers": []},
        }))
        .unwrap()
    }

    #[test]
    fn a_repository_is_described_by_address_and_remote_match() {
        let origin = "https://mallard-ai.com";
        let mine = summary(None, None, false);
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
            ..summary(None, None, false)
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
        assert!(!public.can_push, "the anonymous public face is not a writer");
        assert_eq!(public.updated_at, 900);
        let other = describe_repository(origin, &public_summary(Some(url)), Some(&local));
        assert_eq!(other.git_remote, GitRemoteMatch::Differs);
        let withheld = describe_repository(origin, &public_summary(None), Some(&local));
        assert_eq!(withheld.git_remote, GitRemoteMatch::Unknown);
        assert_eq!(withheld.repository_fingerprint, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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
        let refused = accept_repository_invitation(&guest, &origin, "AAAAA-AAAAA-AAAAA-AAAAB", None)
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
        assert_eq!(list_repositories(&guest, &origin, None).await.unwrap().len(), 1);
    }
}

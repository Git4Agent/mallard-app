//! Cloud repository connections for Library items. The service client and
//! publication protocol are shared with Projects; no project identity enters here.
//!
//! An item's repository is chosen once: Connect, Create, or the first Push
//! saves it, and it is fixed from then on (a project's link works the same
//! way). Nothing here disconnects.
use super::*;
use mallard_infrastructure::managed_cloud::address::{parse_repository_address, AddressError};
use mallard_infrastructure::managed_cloud::client::{
    CreateCatalogItem, CreateRepository, DiscardedDraft, ErrorKind, ListingCatalog,
    ManagedCloudClient, ManagedCloudStatus, PublicFilter, PublicSort, RepositoryListing,
    RepositorySummary,
};
use mallard_infrastructure::storage_providers::managed_cloud::{client_for, provider_key};

/// The refusal a second binding meets.
pub const CLOUD_BINDING_FIXED: &str =
    "This item already has its Mallard Cloud repository, and it can't be changed.";

/// One Mallard Cloud repository of a Catalog item, as the connect dialog and
/// Add list or resolve it: its address, the account's standing, the item it
/// holds, and the coordinate it was registered under.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogCloudRepository {
    pub repository_id: String,
    pub owner_login: String,
    pub name: String,
    /// `{owner_login}/{name}`, the service's spelling.
    pub address: String,
    pub website_url: String,
    pub visibility: String,
    /// The account's role; `None` for a public repository it does not
    /// belong to, where it still pushes as a contributor.
    pub role: Option<String>,
    pub can_push: bool,
    /// The version the account reads: the draft's for a collaborator, the
    /// public one otherwise.
    pub revision: u64,
    pub updated_at: u64,
    /// Created, nothing pushed yet.
    pub registration_only: bool,
    /// The Owner's line, else GitHub's.
    pub description: Option<String>,
    pub star_count: u64,
    pub viewer_starred: bool,
    pub resource_id: CatalogResourceId,
    pub display_name: String,
    #[serde(flatten)]
    pub coordinate: CatalogCoordinate,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    /// Whether this build registers the coordinate.
    pub available: bool,
}

/// What one typed address or invitation code answered. Whether the item
/// fits the chooser that asked (its kind, its format, its item) is the
/// chooser's to say: connecting checks it again.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogCloudResolution {
    /// A Catalog item's repository the account may read.
    Found {
        repository: Box<CatalogCloudRepository>,
    },
    /// Nothing at that address the account may see.
    NotFound { address: String },
    /// A project's repository, or one that holds no Catalog item.
    NotCatalog { address: String },
    /// The address names another Mallard Cloud host than this storage's.
    WrongService { host: String, expected: String },
    /// A `github.com` address.
    Github,
    /// Not an address.
    Invalid,
}

/// Where a repository is saved on this computer: the row that connected or
/// added it, and the item it is there.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogCloudPlace {
    pub scope_id: CatalogScopeId,
    pub storage_id: StorageId,
    pub resource_id: CatalogResourceId,
}

/// The item a repository holds, when it is one of this coordinate's.
fn item_of(
    coordinate: &CatalogCoordinate,
    summary: &RepositorySummary,
) -> Result<CatalogResourceId, String> {
    let binding = summary
        .catalog
        .as_ref()
        .filter(|_| summary.repository_class == "catalog")
        .ok_or("This address is not a Catalog item.")?;
    if binding.provider != coordinate.provider.as_str()
        || binding.resource_kind != coordinate.resource_kind.as_str()
        || binding.format != coordinate.format.as_str()
        || binding.format_version != coordinate.format_version
    {
        return Err("This repository uses another provider, resource kind, or format. Open its matching Catalog row.".to_string());
    }
    let resource_id = CatalogResourceId::parse(binding.resource_id.clone())?;
    let stream = CatalogResourceRepositoryId::for_item(coordinate, &resource_id);
    if binding.library_repository_id.as_deref() != Some(stream.as_str()) {
        return Err("The Cloud repository has an invalid item identity.".to_string());
    }
    Ok(resource_id)
}

/// A repository summary as a chooser row; `None` for anything that is not a
/// Catalog item's repository.
fn repository_row(
    client: &ManagedCloudClient,
    registry: &CatalogRegistry,
    summary: &RepositorySummary,
) -> Option<CatalogCloudRepository> {
    if summary.repository_class != "catalog" {
        return None;
    }
    let binding = summary.catalog.as_ref()?;
    let coordinate = CatalogCoordinate::new(
        binding.provider.clone(),
        binding.resource_kind.clone(),
        binding.format.clone(),
        binding.format_version,
    )
    .ok()?;
    let resource_id = CatalogResourceId::parse(binding.resource_id.clone()).ok()?;
    let tag = summary
        .private_summary
        .as_ref()
        .or(summary.public_summary.as_ref());
    let head = summary
        .private_head
        .as_ref()
        .or(summary.public_head.as_ref());
    Some(CatalogCloudRepository {
        repository_id: summary.id.clone(),
        owner_login: summary.owner.github_login.clone(),
        name: summary.name.clone(),
        address: format!("{}/{}", summary.owner.github_login, summary.name),
        website_url: client.link_from_summary(summary).website_url,
        visibility: summary.visibility.clone(),
        role: summary.role.clone(),
        can_push: summary.writer,
        revision: tag.map(|tag| tag.revision).unwrap_or(0),
        updated_at: head
            .map(|head| head.updated_at)
            .or(tag.map(|tag| tag.updated_at))
            .unwrap_or(0),
        registration_only: tag.map(|tag| tag.registration_only).unwrap_or(true),
        description: summary.blurb(),
        star_count: summary.star_count,
        viewer_starred: summary.viewer_starred,
        resource_id,
        display_name: tag
            .map(|tag| tag.display_name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| summary.name.clone()),
        labels: registry.labels_for(&coordinate),
        available: registry.capability(&coordinate).is_some(),
        coordinate,
    })
}

/// Whether a listed repository's coordinate is this row's.
fn same_coordinate(listed: &ListingCatalog, coordinate: &CatalogCoordinate) -> bool {
    listed.provider == coordinate.provider.as_str()
        && listed.resource_kind == coordinate.resource_kind.as_str()
        && listed.format == coordinate.format.as_str()
        && listed.format_version == coordinate.format_version
}

fn newest_first(rows: &mut [CatalogCloudRepository]) {
    rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.address.cmp(&right.address))
    });
}

const REPOSITORY_NAME_MAX: usize = 100;

fn valid_repository_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= REPOSITORY_NAME_MAX
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A valid name as it is; any other cleaned to the service's characters.
/// `normalizeRepositoryName` in the frontend is the same rule.
fn normalize_repository_name(requested: &str) -> String {
    let trimmed = requested.trim();
    if valid_repository_name(trimmed) {
        return trimmed.to_string();
    }
    let mut cleaned = String::new();
    for character in trimmed.chars() {
        let kept = character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-');
        let next = if kept { character } else { '-' };
        if next == '-' && cleaned.ends_with('-') {
            continue;
        }
        cleaned.push(next);
    }
    let mut name = cleaned.trim_matches('-').to_string();
    if name.len() > REPOSITORY_NAME_MAX {
        name.truncate(REPOSITORY_NAME_MAX);
        name = name.trim_end_matches('-').to_string();
    }
    if valid_repository_name(&name) {
        name
    } else {
        "repository".to_string()
    }
}

/// The name the service gives a new repository: the cleaned name, suffixed
/// `-2`, `-3`, … while the account owns one by that name (compared without
/// case). The service stays the authority; this is the review's preview,
/// and `predictRepositoryAddress` in the frontend is the same rule.
pub(super) fn predicted_repository_name(requested: &str, owned: &BTreeSet<String>) -> String {
    let base = normalize_repository_name(requested);
    let mut name = base.clone();
    let mut suffix = 2;
    while owned.contains(&name.to_lowercase()) {
        let tail = format!("-{suffix}");
        let stem = base[..base.len().min(REPOSITORY_NAME_MAX - tail.len())].trim_end_matches('-');
        name = format!("{stem}{tail}");
        suffix += 1;
    }
    name
}

impl StorageCatalogService {
    /// The client and service origin of a Mallard Cloud storage.
    fn cloud_client(&self, storage_id: &StorageId) -> Result<(ManagedCloudClient, String), String> {
        let storage = storage_for(&self.state, storage_id)?;
        if storage.provider != provider_key() {
            return Err("Choose a Mallard Cloud storage.".to_string());
        }
        let (settings, client) = client_for(&storage.provider_config)?;
        Ok((client, settings.service_origin))
    }

    fn cloud_context(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<(CatalogCoordinate, ManagedCloudClient), String> {
        let resolved = resolve_scope(&self.catalog, scope_id)?;
        require_capability(&self.registry, &resolved.scope)?;
        let (client, _) = self.cloud_client(storage_id)?;
        Ok((resolved.scope.coordinate(), client))
    }

    /// The repositories this account belongs to that hold one item: what
    /// Connect lists, and the ones a choice between several is made from.
    /// Newest first.
    pub async fn list_cloud_repositories(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
    ) -> Result<Vec<CatalogCloudRepository>, String> {
        let (coordinate, client) = self.cloud_context(scope_id, storage_id)?;
        let mut rows = client
            .list_repositories("catalog")
            .await?
            .iter()
            .filter_map(|summary| repository_row(&client, &self.registry, summary))
            .filter(|row| row.coordinate == coordinate && &row.resource_id == resource_id)
            .collect::<Vec<_>>();
        newest_first(&mut rows);
        Ok(rows)
    }

    /// Shared with you: someone else's item repositories this account
    /// belongs to that no Catalog row on this storage has saved, of every
    /// coordinate this build opens. Newest first.
    pub async fn list_cloud_shared(
        &self,
        storage_id: &StorageId,
    ) -> Result<Vec<CatalogCloudRepository>, String> {
        let (client, _) = self.cloud_client(storage_id)?;
        let links = self.catalog.load_scope_links()?;
        let saved = links
            .links
            .iter()
            .filter(|link| &link.storage_id == storage_id)
            .flat_map(|link| link.cloud_repositories.values())
            .collect::<BTreeSet<_>>();
        let mut rows = client
            .list_repositories("catalog")
            .await?
            .iter()
            .filter_map(|summary| repository_row(&client, &self.registry, summary))
            .filter(|row| {
                row.available
                    && row.role.as_deref().is_some_and(|role| role != "owner")
                    && !saved.contains(&row.repository_id)
            })
            .collect::<Vec<_>>();
        newest_first(&mut rows);
        Ok(rows)
    }

    /// The coordinate a listing is narrowed to — a row's — and the client.
    /// Add Catalog resource's door names no row: every coordinate this
    /// build opens is kept.
    fn listing_scope(
        &self,
        scope_id: Option<&CatalogScopeId>,
        storage_id: &StorageId,
    ) -> Result<(Option<CatalogCoordinate>, ManagedCloudClient), String> {
        match scope_id {
            Some(scope_id) => {
                let (coordinate, client) = self.cloud_context(scope_id, storage_id)?;
                Ok((Some(coordinate), client))
            }
            None => {
                let (client, _) = self.cloud_client(storage_id)?;
                Ok((None, client))
            }
        }
    }

    /// Whether a listed Catalog repository belongs in the listing: the
    /// row's coordinate, or any coordinate this build registers.
    fn listed_here(&self, listed: &ListingCatalog, coordinate: Option<&CatalogCoordinate>) -> bool {
        match coordinate {
            Some(coordinate) => same_coordinate(listed, coordinate),
            None => CatalogCoordinate::new(
                listed.provider.clone(),
                listed.resource_kind.clone(),
                listed.format.clone(),
                listed.format_version,
            )
            .ok()
            .is_some_and(|coordinate| self.registry.capability(&coordinate).is_some()),
        }
    }

    /// Starred: the Catalog items the account starred on the website, one
    /// page of its Starred list, the projects and the other kinds left out.
    pub async fn list_cloud_starred(
        &self,
        scope_id: Option<&CatalogScopeId>,
        storage_id: &StorageId,
        cursor: Option<&str>,
    ) -> Result<RepositoryListing, String> {
        let (coordinate, client) = self.listing_scope(scope_id, storage_id)?;
        let mut page = client.list_starred(cursor).await?;
        page.rows.retain(|row| {
            row.repository_class == "catalog"
                && row
                    .catalog
                    .as_ref()
                    .is_some_and(|listed| self.listed_here(listed, coordinate.as_ref()))
        });
        Ok(page)
    }

    /// Public: the released Catalog items, newest first or by stars, one
    /// page at a time. The service narrows by a row's provider and kind;
    /// the format is checked here.
    pub async fn list_cloud_public(
        &self,
        scope_id: Option<&CatalogScopeId>,
        storage_id: &StorageId,
        sort: PublicSort,
        cursor: Option<&str>,
    ) -> Result<RepositoryListing, String> {
        let (coordinate, client) = self.listing_scope(scope_id, storage_id)?;
        let filter = PublicFilter {
            class: "catalog",
            provider: coordinate.as_ref().map(|c| c.provider.as_str()),
            kind: coordinate.as_ref().map(|c| c.resource_kind.as_str()),
            sort,
        };
        let mut page = client.list_public(&filter, cursor).await?;
        page.rows.retain(|row| {
            row.catalog
                .as_ref()
                .is_some_and(|listed| self.listed_here(listed, coordinate.as_ref()))
        });
        Ok(page)
    }

    fn resolution(
        &self,
        client: &ManagedCloudClient,
        summary: &RepositorySummary,
    ) -> CatalogCloudResolution {
        match repository_row(client, &self.registry, summary) {
            Some(repository) => CatalogCloudResolution::Found {
                repository: Box::new(repository),
            },
            None => CatalogCloudResolution::NotCatalog {
                address: format!("{}/{}", summary.owner.github_login, summary.name),
            },
        }
    }

    /// One typed address, resolved against the storage's Mallard Cloud.
    pub async fn resolve_cloud_repository(
        &self,
        storage_id: &StorageId,
        address: &str,
    ) -> Result<CatalogCloudResolution, String> {
        let (client, origin) = self.cloud_client(storage_id)?;
        let parsed = match parse_repository_address(address, &origin) {
            Ok(parsed) => parsed,
            Err(AddressError::WrongService { host, expected }) => {
                return Ok(CatalogCloudResolution::WrongService { host, expected })
            }
            Err(AddressError::GitHub) => return Ok(CatalogCloudResolution::Github),
            Err(AddressError::Invalid) => return Ok(CatalogCloudResolution::Invalid),
        };
        let Some(summary) = client
            .repository_by_name(&parsed.owner, &parsed.name)
            .await?
        else {
            return Ok(CatalogCloudResolution::NotFound {
                address: parsed.to_string(),
            });
        };
        Ok(self.resolution(&client, &summary))
    }

    /// One invitation code: the account joins the repository it belongs to,
    /// which answers as a resolved address does. Joining is harmless
    /// whatever the repository holds; the answer says what that is.
    pub async fn join_cloud_repository(
        &self,
        storage_id: &StorageId,
        code: &str,
    ) -> Result<CatalogCloudResolution, String> {
        let (client, _) = self.cloud_client(storage_id)?;
        let code = code.trim();
        if code.is_empty() {
            return Err("Enter the invitation code.".to_string());
        }
        let summary = client.accept_invitation(code).await?;
        Ok(self.resolution(&client, &summary))
    }

    /// The row on this storage that saved a repository, and its item: where
    /// a link that names the repository opens. `None` when no row has. It
    /// reads this computer's saved connections and asks the service nothing.
    pub fn find_cloud_repository(
        &self,
        storage_id: &StorageId,
        repository_id: &str,
    ) -> Result<Option<CatalogCloudPlace>, String> {
        let links = self.catalog.load_scope_links()?;
        Ok(links
            .links
            .iter()
            .filter(|link| &link.storage_id == storage_id)
            .find_map(|link| {
                link.cloud_repositories
                    .iter()
                    .find(|(_, saved)| saved.as_str() == repository_id)
                    .map(|(resource_id, _)| CatalogCloudPlace {
                        scope_id: link.scope_id.clone(),
                        storage_id: link.storage_id.clone(),
                        resource_id: resource_id.clone(),
                    })
            }))
    }

    /// Save the repository an item syncs with. Saving what is saved already
    /// changes nothing; any other repository is refused.
    fn save_cloud_binding(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
        expected_link_revision: u64,
    ) -> Result<(), String> {
        let _operation = self.state.acquire_host_operation()?;
        self.catalog.mutate_scope_links(|document| {
            let link = document
                .links
                .iter_mut()
                .find(|link| &link.scope_id == scope_id && &link.storage_id == storage_id)
                .ok_or("Connect this Catalog row to the storage first.")?;
            match link.cloud_repositories.get(resource_id) {
                Some(saved) if saved == repository_id => return Ok(()),
                Some(_) => return Err(CLOUD_BINDING_FIXED.to_string()),
                None => {}
            }
            if link.revision != expected_link_revision {
                return Err("The connection changed. Refresh and connect again.".to_string());
            }
            link.cloud_repositories
                .insert(resource_id.clone(), repository_id.to_owned());
            link.accepted_items.remove(resource_id);
            link.accepted_resources.remove(resource_id);
            link.revision = link.revision.saturating_add(1);
            link.updated_at = now_secs();
            Ok(())
        })
    }

    /// Connect an item to an existing repository: Connect on an item's page,
    /// a choice between several, and Add. Nothing is pulled.
    pub async fn connect_cloud_repository(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
        expected_link_revision: u64,
    ) -> Result<(), String> {
        let (coordinate, client) = self.cloud_context(scope_id, storage_id)?;
        let summary = client.get_repository(repository_id).await?;
        if &item_of(&coordinate, &summary)? != resource_id {
            return Err("The repository names another item.".to_string());
        }
        // Someone else's public item: put it on the account's Catalog on
        // the service, so the Library listing carries it on every computer,
        // not only this machine's binding.
        if summary.role.is_none() {
            client.add_catalog_item(repository_id).await?;
        }
        self.save_cloud_binding(
            scope_id,
            storage_id,
            resource_id,
            repository_id,
            expected_link_revision,
        )
    }

    /// Create a local item's private repository now and save it, as a
    /// project's Link registers its own. The request is the one the first
    /// Push makes. Safe to repeat: an item that has a repository gets it
    /// back, and so does an account that already holds one for the item.
    pub async fn create_cloud_repository(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        expected_link_revision: u64,
    ) -> Result<CatalogCloudRepository, String> {
        let (coordinate, client) = self.cloud_context(scope_id, storage_id)?;
        let saved = self
            .catalog
            .load_scope_links()?
            .link_for(scope_id, storage_id)
            .ok_or("Connect this Catalog row to the storage first.")?
            .cloud_repositories
            .get(resource_id)
            .cloned();
        let summary = match saved {
            Some(repository_id) => client.get_repository(&repository_id).await?,
            None => {
                let stream = CatalogResourceRepositoryId::for_item(&coordinate, resource_id);
                let spec = CreateRepository {
                    repository_class: "catalog",
                    name: self.local_display_name(scope_id, resource_id)?,
                    bundle_id: None,
                    repository_fingerprint: None,
                    catalog: Some(CreateCatalogItem {
                        provider: coordinate.provider.as_str().to_owned(),
                        resource_kind: coordinate.resource_kind.as_str().to_owned(),
                        format: coordinate.format.as_str().to_owned(),
                        format_version: coordinate.format_version,
                        resource_id: resource_id.as_str().to_owned(),
                        library_repository_id: stream.as_str().to_owned(),
                    }),
                    github_url: None,
                };
                match client.create_repository(&spec).await {
                    Ok(created) => created,
                    Err(error) if error.kind == ErrorKind::AccountExists => {
                        let existing = client
                            .repository_by_library(stream.as_str())
                            .await?
                            .ok_or_else(|| error.message())?;
                        client.get_repository(&existing).await?
                    }
                    Err(error) => return Err(error.message()),
                }
            }
        };
        if &item_of(&coordinate, &summary)? != resource_id {
            return Err("The repository names another item.".to_string());
        }
        let row = repository_row(&client, &self.registry, &summary)
            .ok_or("This address is not a Catalog item.")?;
        self.save_cloud_binding(
            scope_id,
            storage_id,
            resource_id,
            &summary.id,
            expected_link_revision,
        )?;
        Ok(row)
    }

    /// The name a local item shows, which names its new repository as the
    /// first Push would.
    fn local_display_name(
        &self,
        scope_id: &CatalogScopeId,
        resource_id: &CatalogResourceId,
    ) -> Result<String, String> {
        let ResolvedScope { scope, .. } = resolve_scope(&self.catalog, scope_id)?;
        let capability = require_capability(&self.registry, &scope)?;
        let context = resolve_profile_context(
            &self.registry,
            &self.catalog.load_profiles()?,
            &scope.profile_id,
            &scope.provider,
            &self.user_home,
        )
        .map_err(|blocker| blocker.to_string())?;
        let inventory = inventory_local(&self.registry, &capability, &context)
            .map_err(|blocker| blocker.to_string())?;
        let record = inventory
            .resources
            .get(resource_id)
            .ok_or("This item is not on this Mac.")?;
        capability.codec.display_name(&record.resource)
    }

    fn cloud_target(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
    ) -> Result<ManagedCloudClient, String> {
        let (_, client) = self.cloud_context(scope_id, storage_id)?;
        let links = self.catalog.load_scope_links()?;
        let link = links
            .link_for(scope_id, storage_id)
            .ok_or("Connect the repository first.")?;
        if link.cloud_repositories.get(resource_id).map(String::as_str) != Some(repository_id) {
            return Err(
                "The repository connection changed. Connect it before using Cloud actions."
                    .to_string(),
            );
        }
        Ok(client)
    }

    pub async fn cloud_status(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
    ) -> Result<ManagedCloudStatus, String> {
        let (coordinate, client) = self.cloud_context(scope_id, storage_id)?;
        // Read-only status can be inspected before connecting. Verify the
        // binding before exposing any controls for an arbitrary address.
        let summary = match client.get_repository(repository_id).await {
            Ok(summary) => summary,
            Err(error) => return Ok(ManagedCloudStatus::failed(repository_id, &error)),
        };
        if item_of(&coordinate, &summary)? != *resource_id {
            return Err("The repository names another item.".to_string());
        }
        let stream = CatalogResourceRepositoryId::for_item(&coordinate, resource_id);
        let prefix = format!(".mallard/v1/library/{}/_commits", stream.as_str());
        Ok(match client.list_keys(repository_id, &prefix, None).await {
            Ok(listing) => ManagedCloudStatus::observed(repository_id, &listing),
            Err(error) => ManagedCloudStatus::failed(repository_id, &error),
        })
    }

    pub async fn request_cloud_release(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
        expected_revision: u64,
    ) -> Result<(), String> {
        let client = self.cloud_target(scope_id, storage_id, resource_id, repository_id)?;
        client
            .create_release_request(repository_id, expected_revision)
            .await?;
        Ok(())
    }

    pub async fn discard_cloud_draft(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
        repository_id: &str,
        expected_revision: u64,
    ) -> Result<DiscardedDraft, String> {
        let client = self.cloud_target(scope_id, storage_id, resource_id, repository_id)?;
        Ok(client
            .discard_private_snapshot(repository_id, expected_revision)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicted_names_follow_the_frontend_rule() {
        let none = BTreeSet::new();
        assert_eq!(predicted_repository_name("Duck", &none), "Duck");
        assert_eq!(
            predicted_repository_name("Horror Game", &none),
            "Horror-Game"
        );
        assert_eq!(predicted_repository_name("My--Repo", &none), "My--Repo");
        assert_eq!(predicted_repository_name("///", &none), "repository");
        let owned = BTreeSet::from(["duck".to_string(), "duck-2".to_string()]);
        assert_eq!(predicted_repository_name("Duck", &owned), "Duck-3");
        let long = "a".repeat(120);
        assert_eq!(predicted_repository_name(&long, &none).len(), 100);
        let owned = BTreeSet::from(["a".repeat(100)]);
        assert_eq!(
            predicted_repository_name(&long, &owned),
            format!("{}-2", "a".repeat(98))
        );
    }
}

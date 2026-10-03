//! Open in Mallard: the `mallard://` link the website hands the desktop
//! (docs/PLAN_DEEP_LINK_DESTINATIONS.md, PLAN_DEEP_LINK_SHARE_YOURS.md). A
//! link names a page: a repository's, or the listing. Here it is read and
//! kept for the window to take, and a repository's is answered: what each
//! account on the page's host sees at that address. The listing's asks the
//! service nothing. It only ever reads. The window decides where it lands,
//! and every write stays behind a verb the person presses.
use super::service::repository;
use crate::activity_log::ActivityLogScope;
use mallard_domain::{StorageConfig, StorageId};
use mallard_infrastructure::activity_log::ActivityLogType;
use mallard_infrastructure::managed_cloud::address::{
    origin_host, parse_page_address, RepositoryAddress,
};
use mallard_infrastructure::managed_cloud::app_link::{parse_app_link, AppLink, ListingKind};
use mallard_infrastructure::managed_cloud::client::RepositorySummary;
use mallard_infrastructure::managed_cloud::default_origin;
use mallard_infrastructure::storage_providers::managed_cloud::{client_for, provider_key};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// Tells the window a link is waiting; it carries nothing.
pub const APP_LINK_EVENT: &str = "app-link";

/// A link once read: the page it names, or that it is not one. The raw text
/// goes no further than `receive`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppLinkArrival {
    Repository {
        /// The page's host, as `origin_host` spells one.
        host: String,
        /// `owner/name`, as the link spelled it.
        address: String,
    },
    /// The website's listing (Share yours). Nothing is asked of the
    /// service: the window opens its own list of what could be shared.
    Listing {
        host: String,
        /// The kind the listing's chips were filtered to, when one was on.
        listing_kind: Option<ListingKind>,
        /// The host is the service a new storage here signs in to, so
        /// with no account on it the way forward is setup.
        can_set_up: bool,
    },
    Invalid,
}

/// The link waiting for the window: one at a time, the newest.
#[derive(Default)]
pub struct AppLinkInbox(Mutex<Option<AppLinkArrival>>);

impl AppLinkInbox {
    fn put(&self, arrival: AppLinkArrival) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(arrival);
    }

    fn take(&self) -> Option<AppLinkArrival> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// Whether `host` is the service a new Mallard Cloud storage signs in to.
fn is_this_builds_service(host: &str) -> bool {
    origin_host(&default_origin()).as_deref() == Some(host)
}

fn read_link(link: &str) -> AppLinkArrival {
    match parse_app_link(link) {
        Some(AppLink::Repository { host, address }) => AppLinkArrival::Repository {
            host,
            address: address.to_string(),
        },
        Some(AppLink::Listing { host, kind }) => AppLinkArrival::Listing {
            can_set_up: is_this_builds_service(&host),
            host,
            listing_kind: kind,
        },
        None => AppLinkArrival::Invalid,
    }
}

/// Bring the window forward: a link is a request to look at Mallard.
pub fn show_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// A link the system handed over: read it, keep it for the window, and say
/// one is waiting. Only the host and `owner/name` are logged.
pub fn receive<R: tauri::Runtime>(app: &tauri::AppHandle<R>, link: &str) {
    let arrival = read_link(link);
    let scope = ActivityLogScope::new(ActivityLogType::System);
    match &arrival {
        AppLinkArrival::Repository { host, address } => scope.emit(
            app,
            "info",
            "system.app_link_received",
            &format!("Link received: {address} on {host}"),
        ),
        AppLinkArrival::Listing { host, .. } => scope.emit(
            app,
            "info",
            "system.app_link_received",
            &format!("Link received: the listing on {host}"),
        ),
        AppLinkArrival::Invalid => scope.emit(
            app,
            "warn",
            "system.app_link_refused",
            "A link Mallard can't open was refused",
        ),
    }
    app.state::<AppLinkInbox>().put(arrival);
    show_window(app);
    let _ = app.emit(APP_LINK_EVENT, ());
}

/// Take links from the system from here on. Called once, at setup.
pub fn listen<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri_plugin_deep_link::DeepLinkExt;
    // macOS registers the scheme from the bundle and Windows at install.
    // Linux has neither, and nor does a Windows development build.
    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    let _ = app.deep_link().register_all();
    // A link that started the app arrived before this ran.
    if let Ok(Some(links)) = app.deep_link().get_current() {
        if let Some(link) = links.last() {
            receive(app, link.as_str());
        }
    }
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        if let Some(link) = event.urls().last() {
            receive(&handle, link.as_str());
        }
    });
}

/// The link that is waiting, once: at startup, and whenever `app-link` fires.
#[tauri::command]
pub fn take_app_link(inbox: tauri::State<'_, AppLinkInbox>) -> Option<AppLinkArrival> {
    inbox.take()
}

/// The Catalog coordinate and item a repository holds, in the service's
/// words; the Catalog reads them against its own registry.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AppLinkCatalogItem {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
    pub resource_id: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AppLinkRepository {
    pub repository_id: String,
    /// `{owner}/{name}`, the service's spelling.
    pub address: String,
    /// `git_project`, `non_git_project`, or `catalog`.
    pub repository_class: String,
    /// A project's bundle; none for a Catalog item.
    pub bundle_id: Option<String>,
    /// A Catalog item's coordinate; none for a project.
    pub catalog: Option<AppLinkCatalogItem>,
    pub visibility: String,
    /// The account's role; `None` when it is not a collaborator.
    pub role: Option<String>,
}

/// What one account sees at the address.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppLinkSight {
    Found {
        repository: Box<AppLinkRepository>,
    },
    /// Nothing there this account may see: private, removed, or mistyped.
    NotFound,
    /// The account could not be asked: signed out, or the service is away.
    Failed {
        message: String,
    },
}

/// One Mallard Cloud storage on the link's host, and what it sees.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AppLinkAccount {
    pub storage_id: StorageId,
    pub storage_name: String,
    pub account_login: String,
    pub sight: AppLinkSight,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AppLinkAnswer {
    pub host: String,
    /// `owner/name`, as the link spelled it.
    pub address: String,
    /// One per storage that signs in to `host`, in the config's order.
    /// Empty when none does: no request was sent.
    pub accounts: Vec<AppLinkAccount>,
    /// The host is the service a new storage here signs in to, so with no
    /// account the way forward is setup. A link never introduces a service.
    pub can_set_up: bool,
}

fn describe(summary: &RepositorySummary) -> AppLinkRepository {
    let catalog = summary
        .catalog
        .as_ref()
        .filter(|_| summary.repository_class == "catalog")
        .map(|binding| AppLinkCatalogItem {
            provider: binding.provider.clone(),
            resource_kind: binding.resource_kind.clone(),
            format: binding.format.clone(),
            format_version: binding.format_version,
            resource_id: binding.resource_id.clone(),
        });
    AppLinkRepository {
        repository_id: summary.id.clone(),
        address: format!("{}/{}", summary.owner.github_login, summary.name),
        repository_class: summary.repository_class.clone(),
        bundle_id: summary.bundle_id.clone(),
        catalog,
        visibility: summary.visibility.clone(),
        role: summary.role.clone(),
    }
}

/// Ask each Mallard Cloud storage whose saved service is `host`. A storage
/// on any other host is not asked: its token never leaves for a host a
/// link named.
async fn ask_accounts(
    storages: &[StorageConfig],
    host: &str,
    address: &RepositoryAddress,
) -> Vec<AppLinkAccount> {
    let mut accounts = Vec::new();
    for storage in storages {
        if storage.provider != provider_key() {
            continue;
        }
        let settings = &storage.provider_config.settings;
        if settings
            .get_str("service_origin")
            .and_then(origin_host)
            .as_deref()
            != Some(host)
        {
            continue;
        }
        let sight = match client_for(&storage.provider_config) {
            Ok((_, client)) => match client
                .repository_by_name(&address.owner, &address.name)
                .await
            {
                Ok(Some(summary)) => AppLinkSight::Found {
                    repository: Box::new(describe(&summary)),
                },
                Ok(None) => AppLinkSight::NotFound,
                Err(error) => AppLinkSight::Failed {
                    message: error.message(),
                },
            },
            Err(message) => AppLinkSight::Failed { message },
        };
        accounts.push(AppLinkAccount {
            storage_id: storage.id.clone(),
            storage_name: storage.name.clone(),
            account_login: settings
                .get_str("account_login")
                .unwrap_or_default()
                .to_owned(),
            sight,
        });
    }
    accounts
}

/// What the accounts on a link's host see at its address. The webview hands
/// back the two values `take_app_link` gave it; they are read again as a
/// page address, so nothing but a repository page can be asked for.
#[tauri::command]
pub async fn resolve_app_link(host: String, address: String) -> Result<AppLinkAnswer, String> {
    let (host, address) = parse_page_address(&format!("https://{host}/{address}"))
        .map_err(|_| "Mallard can't open that link.".to_string())?;
    let config = repository()?.load_config()?;
    let accounts = ask_accounts(&config.storages, &host, &address).await;
    Ok(AppLinkAnswer {
        can_set_up: is_this_builds_service(&host),
        address: address.to_string(),
        host,
        accounts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{BoundedConfigObject, SecretMap, SecretValue, StorageProviderConfig};
    use mallard_infrastructure::managed_cloud::client::{CreateRepository, ManagedCloudClient};
    use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
    use mallard_infrastructure::storage_providers::managed_cloud::{
        token_key, ManagedCloudSettingsV1,
    };

    #[test]
    fn a_link_is_read_once_and_the_newest_waits() {
        assert_eq!(
            read_link("mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fx%3Fcommit%3D1"),
            AppLinkArrival::Repository {
                host: "mallard-ai.com".into(),
                address: "Rika321/HorrorGame".into()
            }
        );
        assert_eq!(
            read_link("mallard://pull?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb"),
            AppLinkArrival::Invalid
        );
        // The listing: its kind in the website's own word, and whether this
        // build could sign in to the host. Nothing else rides along.
        assert_eq!(
            serde_json::to_value(read_link(
                "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%3Ftype%3Ddesktop_pet%26scope%3Dyours"
            ))
            .unwrap(),
            serde_json::json!({ "kind": "listing", "host": "mallard-ai.com", "listing_kind": "desktop_pet", "can_set_up": true })
        );
        assert_eq!(
            serde_json::to_value(read_link(
                "mallard://open?address=https%3A%2F%2Fstaging.mallard-ai.com%2Frepositories"
            ))
            .unwrap(),
            serde_json::json!({ "kind": "listing", "host": "staging.mallard-ai.com", "listing_kind": null, "can_set_up": false }),
            "a link never introduces a service"
        );
        // Serialized as the window reads it.
        assert_eq!(
            serde_json::to_value(read_link("mallard://nonsense")).unwrap(),
            serde_json::json!({ "kind": "invalid" })
        );

        let inbox = AppLinkInbox::default();
        assert_eq!(inbox.take(), None);
        inbox.put(AppLinkArrival::Invalid);
        let newest = read_link("mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb");
        inbox.put(newest.clone());
        assert_eq!(
            inbox.take(),
            Some(newest),
            "a newer link replaces the one waiting"
        );
        assert_eq!(inbox.take(), None, "taken once");
    }

    #[test]
    fn a_catalog_item_carries_its_coordinate_and_a_project_its_bundle() {
        let item: RepositorySummary = serde_json::from_value(serde_json::json!({
            "id": "repo_pet", "owner": {"user_id": "user_1", "github_login": "maya"}, "name": "Mochi",
            "repository_class": "catalog", "revision": 5, "state": "active", "visibility": "public",
            "private_revision": 0, "public_revision": 5, "role": null,
            "catalog": {"provider": "codex", "resource_kind": "desktop_pet", "format": "openai.codex-pet", "format_version": 2, "resource_id": "mochi", "library_repository_id": "lib"},
        }))
        .unwrap();
        let described = describe(&item);
        assert_eq!(described.address, "maya/Mochi");
        assert_eq!(described.bundle_id, None);
        assert_eq!(described.role, None);
        assert_eq!(
            described.catalog,
            Some(AppLinkCatalogItem {
                provider: "codex".into(),
                resource_kind: "desktop_pet".into(),
                format: "openai.codex-pet".into(),
                format_version: 2,
                resource_id: "mochi".into(),
            })
        );

        let project: RepositorySummary = serde_json::from_value(serde_json::json!({
            "id": "repo_1", "owner": {"user_id": "user_1", "github_login": "octocat"}, "name": "Project",
            "repository_class": "git_project", "bundle_id": "b".repeat(32), "revision": 7, "state": "active",
            "visibility": "private", "private_revision": 3, "public_revision": 0, "role": "owner",
        }))
        .unwrap();
        let described = describe(&project);
        assert_eq!(described.catalog, None);
        assert_eq!(described.bundle_id, Some("b".repeat(32)));
        assert_eq!(described.role.as_deref(), Some("owner"));
    }

    fn cloud_storage(id: &str, origin: &str, login: &str, token: Option<&str>) -> StorageConfig {
        let mut secrets = SecretMap::empty();
        if let Some(token) = token {
            secrets
                .insert(token_key(), SecretValue::parse(token).unwrap())
                .unwrap();
        }
        StorageConfig {
            id: StorageId::parse(id).unwrap(),
            name: format!("Mallard Cloud {login}"),
            provider: provider_key(),
            provider_config: StorageProviderConfig::new(
                1,
                BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
                    service_origin: origin.into(),
                    account_id: format!("user_{login}"),
                    account_login: login.into(),
                })
                .unwrap(),
                secrets,
            ),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn each_account_on_the_links_host_is_asked_and_no_other() {
        let stub = StubManagedCloud::start().await;
        let (_, owner_token) = stub.state.lock().unwrap().add_user("Rika321");
        let (_, guest_token) = stub.state.lock().unwrap().add_user("guest");
        let created = ManagedCloudClient::new(
            &stub.endpoint,
            Some(SecretValue::parse(&owner_token).unwrap()),
        )
        .unwrap()
        .create_repository(&CreateRepository {
            repository_class: "git_project",
            github_url: None,
            name: "portal".into(),
            bundle_id: Some("b".repeat(32)),
            repository_fingerprint: Some("f".repeat(64)),
            catalog: None,
        })
        .await
        .unwrap();

        let host = origin_host(&stub.endpoint).unwrap();
        let storages = vec![
            cloud_storage(
                "storage-owner",
                &stub.endpoint,
                "Rika321",
                Some(&owner_token),
            ),
            cloud_storage("storage-guest", &stub.endpoint, "guest", Some(&guest_token)),
            cloud_storage("storage-signed-out", &stub.endpoint, "lapsed", None),
            // Another service's account: its token must never be sent here.
            cloud_storage(
                "storage-elsewhere",
                "https://elsewhere.example",
                "Rika321",
                Some("elsewhere-token-canary-0123456789"),
            ),
        ];
        let address = RepositoryAddress {
            owner: "rika321".into(),
            name: "PORTAL".into(),
        };
        stub.clear_log();
        let accounts = ask_accounts(&storages, &host, &address).await;
        assert_eq!(
            accounts
                .iter()
                .map(|account| (account.storage_id.as_str(), account.account_login.as_str()))
                .collect::<Vec<_>>(),
            [
                ("storage-owner", "Rika321"),
                ("storage-guest", "guest"),
                ("storage-signed-out", "lapsed"),
            ]
        );
        let AppLinkSight::Found { repository } = &accounts[0].sight else {
            panic!("the owner sees it: {:?}", accounts[0].sight);
        };
        assert_eq!(repository.repository_id, created.id);
        assert_eq!(
            repository.address, "Rika321/portal",
            "the service's spelling"
        );
        assert_eq!(repository.repository_class, "git_project");
        assert_eq!(repository.bundle_id, Some("b".repeat(32)));
        assert_eq!(repository.role.as_deref(), Some("owner"));
        assert_eq!(
            accounts[1].sight,
            AppLinkSight::NotFound,
            "private, and the guest is no member"
        );
        assert!(
            matches!(accounts[2].sight, AppLinkSight::Failed { .. }),
            "a storage without its token is still an account on the host"
        );
        let requests = stub.request_lines();
        assert_eq!(
            requests.len(),
            2,
            "one read per signed-in account: {requests:?}"
        );
        assert!(
            requests.iter().all(|line| line.starts_with("GET ")),
            "a link only reads: {requests:?}"
        );

        // A host no storage signs in to: nobody is asked.
        stub.clear_log();
        assert!(ask_accounts(&storages, "staging.mallard-ai.com", &address)
            .await
            .is_empty());
        assert!(stub.request_lines().is_empty());
    }
}

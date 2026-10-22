//! Open in Mallard: the `mallard://` link the system hands the desktop. Here
//! it is read, logged by host and `owner/name` only, kept for the window to
//! take, and the window is brought forward. Reading and answering a link is
//! `mallard_application::project_sync::app_link`.
use crate::event_sink::DesktopSink;
use mallard_application::activity_log::ActivityLogScope;
use mallard_application::project_sync::app_link::read_link;
pub use mallard_application::project_sync::app_link::{AppLinkAnswer, AppLinkArrival};
use mallard_application::project_sync::service::repository;
use mallard_infrastructure::activity_log::ActivityLogType;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// Tells the window a link is waiting; it carries nothing.
pub const APP_LINK_EVENT: &str = "app-link";

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

/// Bring the window forward: a link is a request to look at Mallard.
pub fn show_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// A link the system handed over: read it, keep it for the window, and say
/// one is waiting. Only the host and `owner/name` are logged, and that a
/// link names a session.
pub fn receive<R: tauri::Runtime>(app: &tauri::AppHandle<R>, link: &str) {
    let arrival = read_link(link);
    let scope = ActivityLogScope::new(ActivityLogType::System);
    let sink = DesktopSink::new(app.clone());
    match &arrival {
        AppLinkArrival::Repository { host, address } => scope.emit(
            &sink,
            "info",
            "system.app_link_received",
            &format!("Link received: {address} on {host}"),
        ),
        AppLinkArrival::Session { host, address, .. } => scope.emit(
            &sink,
            "info",
            "system.app_link_received",
            &format!("Link received: a session of {address} on {host}"),
        ),
        AppLinkArrival::Listing { host, .. } => scope.emit(
            &sink,
            "info",
            "system.app_link_received",
            &format!("Link received: the listing on {host}"),
        ),
        AppLinkArrival::Invalid => scope.emit(
            &sink,
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

/// What the accounts on a link's host see at its address. The webview hands
/// back the two values `take_app_link` gave it.
#[tauri::command]
pub async fn resolve_app_link(host: String, address: String) -> Result<AppLinkAnswer, String> {
    mallard_application::project_sync::app_link::resolve(&repository()?, &host, &address).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_read_once_and_the_newest_waits() {
        assert_eq!(
            read_link("mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Frelease%3Fcommit%3D1"),
            AppLinkArrival::Repository {
                host: "mallard-ai.com".into(),
                address: "Rika321/HorrorGame".into()
            }
        );
        // A session's page: its repository, and the session as the window
        // reads it. The page's query does not ride along.
        assert_eq!(
            serde_json::to_value(read_link(
                "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%253Aconversation%253Acloudthread-0a1b2c3d%3Fcommit%3D9f3c2d1%26scope%3Dprivate"
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "session",
                "host": "mallard-ai.com",
                "address": "Rika321/HorrorGame",
                "provider": "codex",
                "cloud_thread_id": "cloudthread-0a1b2c3d",
                "resource_id": "codex:conversation:cloudthread-0a1b2c3d",
            })
        );
        // A session id the website never writes is not a link, and never
        // falls back to the repository.
        assert_eq!(
            read_link("mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fx%3Fcommit%3D1"),
            AppLinkArrival::Invalid
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
}

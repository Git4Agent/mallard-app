//! Bounded, read-only Dropbox namespace discovery.
use crate::dropbox_auth::{validate_id, AuthError, DropboxClient};
use crate::storage_core::{classify_storage_marker, MarkerState};
use mallard_domain::SecretValue;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::{timeout_at, Instant};

#[derive(Clone, Serialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    #[serde(skip)]
    pub home: String,
    #[serde(skip)]
    pub root: String,
    #[serde(skip)]
    pub team: bool,
}

fn text(value: &Value, limit: usize) -> Result<String, AuthError> {
    let value = value.as_str().ok_or(AuthError::Invalid)?;
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(AuthError::Invalid);
    }
    Ok(value.into())
}

pub async fn account(
    client: &DropboxClient,
    token: &SecretValue,
    expected: &str,
) -> Result<Account, AuthError> {
    let value = client
        .rpc("users/get_current_account", Value::Null, token, None)
        .await?;
    let id = text(&value["account_id"], 256)?;
    let home = text(&value["root_info"]["home_namespace_id"], 256)?;
    let root = text(&value["root_info"]["root_namespace_id"], 256)?;
    for id in [&id, &home, &root] {
        validate_id(id)?;
    }
    if id != expected || !matches!(value["root_info"][".tag"].as_str(), Some("user" | "team")) {
        return Err(AuthError::Invalid);
    }
    Ok(Account {
        id,
        home,
        root,
        name: text(&value["name"]["display_name"], 512)?,
        email: text(&value["email"], 320)?,
        team: value["root_info"][".tag"] == "team",
    })
}

#[derive(Clone)]
pub struct Folder {
    pub namespace: String,
    pub name: String,
    pub access: String,
    pub mounted: bool,
    pub marker: MarkerState,
    pub detail: Option<String>,
}

pub struct Scan {
    pub folders: Vec<Folder>,
    pub partial: bool,
}

pub async fn marker(
    client: &DropboxClient,
    token: &SecretValue,
    namespace: &str,
) -> Result<(MarkerState, Option<String>), AuthError> {
    Ok(match client.read_storage_marker(namespace, token).await {
        Ok(Some(bytes)) => classify_storage_marker(&bytes),
        Ok(None) => (MarkerState::Missing, None),
        Err(AuthError::Reconnect) => return Err(AuthError::Reconnect),
        Err(AuthError::Invalid) => (MarkerState::Invalid, None),
        Err(AuthError::Permission) => (MarkerState::Unreadable, Some("Access denied".into())),
        Err(_) => (
            MarkerState::Unreadable,
            Some("Could not read marker".into()),
        ),
    })
}

pub async fn discover(
    client: &DropboxClient,
    token: &SecretValue,
    expected: &str,
) -> Result<Scan, AuthError> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let account = timeout_at(deadline, account(client, token, expected))
        .await
        .map_err(|_| AuthError::Remote)??;
    let mut scan = Scan {
        folders: vec![],
        partial: false,
    };
    let mut seen = BTreeSet::new();
    if account.team && account.root != account.home {
        seen.insert(account.root.clone());
        scan.folders.push(Folder {
            namespace: account.root,
            name: "Team space root".into(),
            access: "unknown".into(),
            mounted: true,
            marker: MarkerState::Unreadable,
            detail: None,
        });
    }
    for (first, next) in [
        ("sharing/list_folders", "sharing/list_folders/continue"),
        (
            "sharing/list_mountable_folders",
            "sharing/list_mountable_folders/continue",
        ),
    ] {
        let mut cursor: Option<String> = None;
        let mut cursors = BTreeSet::new();
        let listed: Result<(), AuthError> = async {
            for page in 0..100 {
                let (route, args) = match &cursor {
                    Some(cursor) => (next, json!({"cursor":cursor})),
                    None => (first, json!({"limit":100})),
                };
                let value = timeout_at(deadline, client.rpc(route, args, token, None))
                    .await
                    .map_err(|_| AuthError::Remote)??;
                let entries = value["entries"]
                    .as_array()
                    .filter(|entries| entries.len() <= 100)
                    .ok_or(AuthError::Invalid)?;
                for entry in entries {
                    let namespace = text(&entry["shared_folder_id"], 256)?;
                    validate_id(&namespace)?;
                    if namespace == account.home {
                        continue;
                    }
                    if !seen.insert(namespace.clone()) {
                        continue;
                    }
                    if scan.folders.len() >= 1000 {
                        return Err(AuthError::Invalid);
                    }
                    let access = match entry["access_type"][".tag"].as_str() {
                        Some(access @ ("owner" | "editor" | "viewer" | "viewer_no_comment")) => {
                            access
                        }
                        _ => "unknown",
                    };
                    scan.folders.push(Folder {
                        namespace,
                        name: text(&entry["name"], 1024)?,
                        access: access.into(),
                        mounted: entry["path_lower"].is_string(),
                        marker: MarkerState::Unreadable,
                        detail: None,
                    });
                }
                if value["cursor"].is_null() {
                    return Ok(());
                }
                let next_cursor = text(&value["cursor"], 4096)?;
                if !cursors.insert(next_cursor.clone()) || page == 99 {
                    return Err(AuthError::Invalid);
                }
                cursor = Some(next_cursor);
            }
            Ok(())
        }
        .await;
        if let Err(error) = listed {
            if error == AuthError::Reconnect {
                return Err(error);
            }
            scan.partial = true;
        }
    }
    for batch in scan.folders.chunks_mut(4) {
        let mut jobs = tokio::task::JoinSet::new();
        for (index, folder) in batch.iter().enumerate() {
            let client = client.clone();
            let token = token.clone();
            let namespace = folder.namespace.clone();
            jobs.spawn(async move {
                let found = timeout_at(deadline, marker(&client, &token, &namespace))
                    .await
                    .unwrap_or(Ok((MarkerState::Unreadable, Some("Scan timed out".into()))));
                (index, found)
            });
        }
        while let Some(result) = jobs.join_next().await {
            let (index, found) = result.map_err(|_| AuthError::Local)?;
            let (marker, detail) = found?;
            scan.partial |= marker == MarkerState::Unreadable;
            batch[index].marker = marker;
            batch[index].detail = detail;
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dropbox_setup::mock::MockDropboxSetup;
    fn entry(id: &str, mounted: bool) -> Value {
        let mut value = json!({"shared_folder_id":id,"name":format!("Folder {id}"),"access_type":{".tag":"editor"}});
        if mounted {
            value["path_lower"] = json!(format!("/{id}"));
        }
        value
    }
    fn seed(mock: &MockDropboxSetup, route: &str, pages: Vec<(u16, Value)>) {
        mock.state
            .lock()
            .unwrap()
            .responses
            .insert(route.into(), pages);
    }
    fn run(mock: &MockDropboxSetup) -> Result<Scan, AuthError> {
        mock.runtime.block_on(discover(
            &mock.client(),
            &SecretValue::parse("test-token").unwrap(),
            "dbid:fixture",
        ))
    }
    #[test]
    fn discovery_pages_deduplicates_and_only_reads_fixed_markers() {
        let mock = MockDropboxSetup::new();
        seed(
            &mock,
            "sharing/list_folders",
            vec![(
                200,
                json!({"entries":[entry("one",true),entry("home",true)],"cursor":"next"}),
            )],
        );
        seed(
            &mock,
            "sharing/list_folders/continue",
            vec![(200, json!({"entries":[entry("two",true)]}))],
        );
        seed(
            &mock,
            "sharing/list_mountable_folders",
            vec![(
                200,
                json!({"entries":[entry("two",false),entry("three",false)]}),
            )],
        );
        seed(
            &mock,
            "marker:one",
            vec![(200, json!({"format":"mallard-storage","layout_version":1}))],
        );
        seed(
            &mock,
            "marker:two",
            vec![(200, json!({"format":"mallard-storage","layout_version":2}))],
        );
        let scan = run(&mock).unwrap();
        assert!(!scan.partial);
        assert_eq!(scan.folders.len(), 3);
        assert_eq!(scan.folders[0].marker, MarkerState::Storage);
        assert_eq!(scan.folders[1].marker, MarkerState::Unsupported);
        assert_eq!(scan.folders[2].marker, MarkerState::Missing);
        assert!(!scan.folders[2].mounted);
        assert_eq!(mock.count("files/download"), 3);
        for route in [
            "files/create_folder_v2",
            "sharing/share_folder",
            "files/upload",
            "sharing/mount_folder",
        ] {
            assert_eq!(mock.count(route), 0);
        }
    }
    #[test]
    fn partial_scan_keeps_valid_candidates_and_bounds_repeated_cursors() {
        let mock = MockDropboxSetup::new();
        seed(
            &mock,
            "sharing/list_folders",
            vec![(200, json!({"entries":[entry("one",true)],"cursor":"same"}))],
        );
        seed(
            &mock,
            "sharing/list_folders/continue",
            vec![(200, json!({"entries":[],"cursor":"same"}))],
        );
        seed(
            &mock,
            "sharing/list_mountable_folders",
            vec![(403, json!({"error":{".tag":"no_permission"}}))],
        );
        seed(
            &mock,
            "marker:one",
            vec![(200, json!({"format":"mallard-storage","layout_version":1}))],
        );
        let scan = run(&mock).unwrap();
        assert!(scan.partial);
        assert_eq!(scan.folders[0].marker, MarkerState::Storage);
        assert_eq!(mock.count("sharing/list_folders/continue"), 1);
    }
    #[test]
    fn team_root_is_scanned_but_personal_home_is_excluded() {
        let mock = MockDropboxSetup::new();
        seed(
            &mock,
            "users/get_current_account",
            vec![(
                200,
                json!({"account_id":"dbid:fixture","name":{"display_name":"Test User"},"email":"test@example.test","root_info":{".tag":"team","home_namespace_id":"home","root_namespace_id":"team"}}),
            )],
        );
        for route in ["sharing/list_folders", "sharing/list_mountable_folders"] {
            seed(&mock, route, vec![(200, json!({"entries":[]}))]);
        }
        let scan = run(&mock).unwrap();
        assert_eq!(scan.folders.len(), 1);
        assert_eq!(scan.folders[0].namespace, "team");
    }
    #[test]
    fn revoked_token_stops_listing_and_marker_reads() {
        for revoke_marker in [false, true] {
            let mock = MockDropboxSetup::new();
            let denied = (401, json!({"error":{".tag":"invalid_access_token"}}));
            seed(
                &mock,
                "sharing/list_folders",
                vec![if revoke_marker {
                    (200, json!({"entries":[entry("one",true)]}))
                } else {
                    denied.clone()
                }],
            );
            if revoke_marker {
                seed(
                    &mock,
                    "sharing/list_mountable_folders",
                    vec![(200, json!({"entries":[]}))],
                );
                seed(&mock, "marker:one", vec![denied]);
            }
            assert!(matches!(run(&mock), Err(AuthError::Reconnect)));
            assert_eq!(mock.count("files/create_folder_v2"), 0);
        }
    }
    #[test]
    fn oversized_and_unreadable_markers_are_not_selectable() {
        let mock = MockDropboxSetup::new();
        seed(
            &mock,
            "sharing/list_folders",
            vec![(
                200,
                json!({"entries":[entry("one",true),entry("two",true)]}),
            )],
        );
        seed(
            &mock,
            "sharing/list_mountable_folders",
            vec![(200, json!({"entries":[]}))],
        );
        seed(
            &mock,
            "marker:one",
            vec![(200, json!({"padding":"x".repeat(5000)}))],
        );
        seed(
            &mock,
            "marker:two",
            vec![(403, json!({"error":{".tag":"no_permission"}}))],
        );
        let scan = run(&mock).unwrap();
        assert!(scan.partial);
        assert_eq!(scan.folders[0].marker, MarkerState::Invalid);
        assert_eq!(scan.folders[1].marker, MarkerState::Unreadable);
    }
}

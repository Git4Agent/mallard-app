//! macOS requires a normal MCP child of Desktop's own Core to call app tools.
//! Register that helper through Codex's config API, then use the shared move.

use super::super::app_server;
use crate::codex_app::InstalledMacApp;
use crate::persistence::{ensure_no_symlinks, write_bytes_atomic};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const SERVER: &str = "mallard_project_placement";
const DIRECTORY: &str = ".mallard-project-placement";
const HELPER: &str = include_str!("macos_helper.mjs");
const MAX_FRAME: u64 = 8 * 1024 * 1024;
const CONNECTION_ERROR: &str = "Could not connect to the Mallard project placement helper in the selected Codex home. Reopen the chat in Codex Desktop, then try again.";

fn profile_directory(home: &Path) -> Result<PathBuf, String> {
    if !home.is_absolute() || !home.is_dir() {
        return Err("Codex home must be an existing absolute directory.".into());
    }
    ensure_no_symlinks(home, home)?;
    let canonical = fs::canonicalize(home).map_err(|_| "Could not resolve Codex home.")?;
    Ok(canonical.join(DIRECTORY))
}

fn private_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CONNECTION_ERROR)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("The Mallard project placement directory must be private and owned by the current user.".into());
    }
    Ok(())
}

fn read_private(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| CONNECTION_ERROR)?;
    let metadata = file.metadata().map_err(|_| CONNECTION_ERROR)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > limit
    {
        return Err(CONNECTION_ERROR.into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CONNECTION_ERROR)?;
    if bytes.len() as u64 > limit {
        return Err(CONNECTION_ERROR.into());
    }
    Ok(bytes)
}

// This read is only a barrier between Desktop's two deep links. Project IDs
// and membership still come from Desktop's native tools, never this file.
pub(in crate::providers::codex) fn project_is_registered(
    home: &Path,
    project_root: &Path,
) -> Result<bool, String> {
    const ERROR: &str = "Could not read Codex Desktop's project registration.";
    let path = home.join(".codex-global-state.json");
    ensure_no_symlinks(home, &path).map_err(|_| ERROR)?;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(ERROR.into()),
    };
    let metadata = file.metadata().map_err(|_| ERROR)?;
    if !metadata.is_file() || metadata.len() > MAX_FRAME {
        return Err(ERROR.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FRAME + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 > MAX_FRAME {
        return Err(ERROR.into());
    }
    let state: Value = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    let Some(projects) = state.get("local-projects").and_then(Value::as_object) else {
        return Ok(false);
    };
    if projects.len() > 10_000 {
        return Err(ERROR.into());
    }
    Ok(projects.values().any(|project| {
        project["rootPaths"].as_array().is_some_and(|roots| {
            roots
                .iter()
                .filter_map(Value::as_str)
                .any(|root| crate::local_paths::same_canonical_path(Path::new(root), project_root))
        })
    }))
}

pub(in crate::providers::codex) fn wait_for_project_registration(
    home: &Path,
    project_root: &Path,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or(
            "Codex Desktop did not register the project folder in time. Try Open in Codex again.",
        )?;
        if project_is_registered(home, project_root)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100).min(remaining));
    }
}

pub(in crate::providers::codex) fn prepare(
    app: &InstalledMacApp,
    home: &Path,
) -> Result<(), String> {
    let directory = profile_directory(home)?;
    let home = directory.parent().ok_or(CONNECTION_ERROR)?;
    match DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("Could not create the Codex project placement directory.".into()),
    }
    private_directory(&directory)?;
    let script = directory.join("helper.mjs");
    let node = app.bundle_path.join("Contents/Resources/cua_node/bin/node");
    let node_metadata = fs::symlink_metadata(&node)
        .map_err(|_| "Codex Desktop's bundled Node runtime is unavailable.")?;
    if !node_metadata.is_file() || node_metadata.file_type().is_symlink() {
        return Err("Codex Desktop's bundled Node runtime is invalid.".into());
    }
    let config_path = home.join("config.toml");
    if let Some(metadata) = ensure_no_symlinks(home, &config_path)? {
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err("Codex configuration must be an ordinary, bounded file.".into());
        }
    }
    let desired = json!({"command": node, "args": [&script, home],
        "env_vars": ["CODEX_HOME", "CODEX_APP_TOOLS_PIPE_PATH"], "startup_timeout_sec": 20.0});
    let read = app_server::request(
        &app.bundled_cli(),
        None,
        home,
        "config/read",
        json!({"includeLayers":true}),
        Duration::from_secs(15),
    )?;
    let user = read["layers"]
        .as_array()
        .and_then(|layers| {
            layers.iter().find(|layer| {
                layer["name"]["type"] == "user"
                    && layer["name"]["file"] == config_path.to_string_lossy().as_ref()
                    && layer["name"]["profile"].is_null()
            })
        })
        .ok_or("Codex did not identify the selected home's configuration.")?;
    let existing = &user["config"]["mcp_servers"][SERVER];
    if !existing.is_null() && existing != &desired {
        return Err("The Mallard project placement MCP entry has different settings. Remove that entry in Codex settings, then try again.".into());
    }
    if let Some(metadata) = ensure_no_symlinks(&directory, &script)? {
        if !metadata.is_file() {
            return Err("The Codex project placement helper must be an ordinary file.".into());
        }
        let previous = read_private(&script, 128 * 1024)?;
        if !previous
            .starts_with(b"// Mallard Codex project placement helper. Managed by Mallard.\n")
        {
            return Err("The Codex project placement helper is not managed by Mallard.".into());
        }
        if previous != HELPER.as_bytes() {
            write_bytes_atomic(&directory, &script, HELPER.as_bytes(), 128 * 1024)?;
        }
    } else {
        write_bytes_atomic(&directory, &script, HELPER.as_bytes(), 128 * 1024)?;
    }
    if existing == &desired {
        return Ok(());
    }
    let version = user["version"]
        .as_str()
        .ok_or("Codex configuration version is missing.")?;
    let written = app_server::request(
        &app.bundled_cli(),
        None,
        home,
        "config/value/write",
        json!({"keyPath":format!("mcp_servers.{SERVER}"), "value":desired,
            "mergeStrategy":"replace", "filePath":config_path, "expectedVersion":version}),
        Duration::from_secs(15),
    )?;
    if written["status"] != "ok" || written["filePath"] != config_path.to_string_lossy().as_ref() {
        return Err("Codex did not confirm the project placement helper configuration.".into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bridge {
    port: u16,
    token: String,
    home: PathBuf,
}

fn bridges(directory: &Path) -> Result<Vec<Bridge>, String> {
    private_directory(directory)?;
    let mut candidates = Vec::new();
    for (count, entry) in fs::read_dir(directory)
        .map_err(|_| CONNECTION_ERROR)?
        .enumerate()
    {
        if count >= 256 {
            return Err("Too many Codex project placement helper records.".into());
        }
        let entry = entry.map_err(|_| CONNECTION_ERROR)?;
        let name = entry.file_name();
        let Some(identity) = name
            .to_str()
            .and_then(|s| s.strip_prefix("bridge-"))
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        let Some((pid, id)) = identity.split_once('-') else {
            continue;
        };
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        if pid <= 0 || super::super::actions::validate_thread_uuid(id).is_err() {
            continue;
        }
        let Ok(bytes) = read_private(&entry.path(), 4096) else {
            continue;
        };
        let Ok(bridge) = serde_json::from_slice::<Bridge>(&bytes) else {
            continue;
        };
        if bridge.port != 0
            && bridge.home == directory.parent().ok_or(CONNECTION_ERROR)?
            && bridge.token.len() == 64
            && bridge.token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            // Signal zero only queries liveness. Remove a stopped helper's
            // private record only if its exact content digest still matches.
            if unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                let expected = Sha256::digest(&bytes);
                if read_private(&entry.path(), 4096)
                    .is_ok_and(|current| Sha256::digest(current) == expected)
                {
                    let _ = fs::remove_file(entry.path());
                }
                continue;
            }
            candidates.push(bridge);
        }
    }
    Ok(candidates)
}

struct Desktop {
    stream: BufReader<TcpStream>,
}

impl Desktop {
    async fn exchange(&mut self, value: Value, timeout: Duration) -> Result<Value, String> {
        tokio::time::timeout(timeout, async {
            let bytes = serde_json::to_vec(&value).map_err(|_| CONNECTION_ERROR)?;
            if bytes.len() >= 64 * 1024 {
                return Err(CONNECTION_ERROR.into());
            }
            self.stream
                .get_mut()
                .write_all(&bytes)
                .await
                .map_err(|_| CONNECTION_ERROR)?;
            self.stream
                .get_mut()
                .write_all(b"\n")
                .await
                .map_err(|_| CONNECTION_ERROR)?;
            let mut line = Vec::new();
            (&mut self.stream)
                .take(MAX_FRAME + 1)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| CONNECTION_ERROR)?;
            if line.len() as u64 > MAX_FRAME || !line.ends_with(b"\n") {
                return Err(CONNECTION_ERROR.into());
            }
            let response: Value = serde_json::from_slice(&line).map_err(|_| CONNECTION_ERROR)?;
            if response.get("error").is_some() {
                return Err(CONNECTION_ERROR.into());
            }
            Ok(response)
        })
        .await
        .map_err(|_| {
            "Codex Desktop project placement timed out; it may already have been applied."
                .to_string()
        })?
    }

    async fn call(&mut self, tool: &str, args: Value, timeout: Duration) -> Result<Value, String> {
        let response = self
            .exchange(json!({"tool":tool,"arguments":args}), timeout)
            .await?;
        let result = &response["result"];
        if result["success"] != true {
            return Err("Codex Desktop rejected project placement.".into());
        }
        let content = result["contentItems"].as_array().ok_or(CONNECTION_ERROR)?;
        let text = content
            .iter()
            .find(|item| item["type"] == "inputText")
            .and_then(|item| item["text"].as_str())
            .ok_or(CONNECTION_ERROR)?;
        serde_json::from_str(text)
            .map_err(|_| "Codex Desktop returned an invalid placement response.".into())
    }
}

async fn connect(
    home: &Path,
    thread_id: &str,
    project_root: Option<&Path>,
    existing_helpers: &[String],
    deadline: Instant,
) -> Result<Desktop, String> {
    let directory = profile_directory(home)?;
    while Instant::now() < deadline {
        for bridge in bridges(&directory)? {
            if existing_helpers.contains(&bridge.token) {
                continue;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = remaining.min(Duration::from_millis(500));
            let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, bridge.port);
            let Ok(Ok(stream)) = tokio::time::timeout(timeout, TcpStream::connect(address)).await
            else {
                continue;
            };
            let mut desktop = Desktop {
                stream: BufReader::new(stream),
            };
            if let Ok(reply) = desktop
                .exchange(
                    json!({"home":bridge.home,"token":bridge.token,
                "threadId":thread_id}),
                    timeout,
                )
                .await
            {
                if reply["ready"] == true && reply["home"] == bridge.home.to_string_lossy().as_ref()
                {
                    // Another window in the same home can retain the project
                    // list from before registration. Use a helper that sees
                    // the exact destination before sending any move.
                    if let Some(root) = project_root {
                        let timeout = deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_secs(2));
                        let Ok(listing) = desktop.call("list_projects", json!({}), timeout).await
                        else {
                            continue;
                        };
                        if super::project_id_in_listing(listing, root)?.is_none() {
                            continue;
                        }
                    }
                    return Ok(desktop);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(CONNECTION_ERROR.into())
}

pub(in crate::providers::codex) fn assign_and_open(
    home: &Path,
    thread_id: &str,
    project_root: &Path,
    existing_helpers: &[String],
) -> Result<(), String> {
    super::super::actions::validate_thread_uuid(thread_id)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the Codex Desktop connection.")?;
    let mut desktop = runtime.block_on(connect(
        home,
        thread_id,
        Some(project_root),
        existing_helpers,
        deadline,
    ))?;
    super::assign_with_request(thread_id, project_root, deadline, |tool, args, timeout| {
        runtime.block_on(desktop.call(tool, args, timeout))
    })
    .map_err(|error| format!("Could not open the Codex chat in its project: {error}"))
}

pub(in crate::providers::codex) fn existing_helpers(home: &Path) -> Result<Vec<String>, String> {
    Ok(bridges(&profile_directory(home)?)?
        .into_iter()
        .map(|bridge| bridge.token)
        .collect())
}

#[cfg(test)]
fn fixture_request(
    home: &Path,
    id: &str,
    project: Option<&Path>,
    tool: &str,
    args: Value,
) -> Value {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut desktop = connect(
            home,
            id,
            project,
            &[],
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .unwrap();
        desktop
            .call(tool, args, Duration::from_secs(10))
            .await
            .unwrap()
    })
}

#[cfg(test)]
pub(in crate::providers::codex) fn fixture_project(
    home: &Path,
    id: &str,
    project_root: &Path,
    registered: bool,
) -> Option<String> {
    let listing = fixture_request(
        home,
        id,
        registered.then_some(project_root),
        "list_projects",
        json!({}),
    );
    let matches = listing["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|project| {
            project["projectKind"] == "local"
                && project["hostId"] == "local"
                && project["path"] == project_root.to_string_lossy().as_ref()
        })
        .collect::<Vec<_>>();
    assert!(
        matches.len() <= 1,
        "fixture folder must have at most one project"
    );
    matches.first().map(|project| {
        let id = project["projectId"].as_str().unwrap();
        assert!(!id.is_empty());
        id.to_string()
    })
}

#[cfg(test)]
pub(in crate::providers::codex) fn fixture_thread(
    home: &Path,
    id: &str,
    project: Option<&Path>,
) -> Value {
    let listing = fixture_request(home, id, project, "list_threads", json!({"limit":50}));
    listing["pinnedThreads"]
        .as_array()
        .unwrap()
        .iter()
        .chain(listing["threads"].as_array().unwrap())
        .find(|thread| thread["id"] == id)
        .expect("fixture chat must be visible")
        .clone()
}

#[cfg(test)]
pub(in crate::providers::codex) fn verify_fixture_request_boundaries(home: &Path, id: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let bridge = bridges(&profile_directory(home).unwrap())
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        for handshake in [
            json!({"home":home.join("different"),"token":bridge.token,"threadId":id}),
            json!({"home":home,"token":"invalid","threadId":id}),
            json!({"home":home,"token":bridge.token,"threadId":"invalid"}),
        ] {
            let stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, bridge.port))
                .await
                .unwrap();
            let mut desktop = Desktop {
                stream: BufReader::new(stream),
            };
            assert!(desktop
                .exchange(handshake, Duration::from_secs(5))
                .await
                .is_err());
        }
        let mut desktop = connect(
            home,
            id,
            None,
            &[],
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .unwrap();
        // Registration polling can exceed the former four-call total limit.
        for _ in 0..5 {
            desktop
                .call("list_projects", json!({}), Duration::from_secs(5))
                .await
                .unwrap();
        }
        for _ in 0..3 {
            desktop
                .call("list_threads", json!({"limit":1}), Duration::from_secs(5))
                .await
                .unwrap();
        }
        assert!(desktop
            .call("list_threads", json!({"limit":1}), Duration::from_secs(5))
            .await
            .is_err());
        let mut desktop = connect(
            home,
            id,
            None,
            &[],
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(desktop
            .call(
                "move_thread_to_project",
                json!({"source":"codex","hostId":"local",
            "threadId":"019f7798-5437-7632-9dbc-5b589cf68bf0","projectId":"different"}),
                Duration::from_secs(5)
            )
            .await
            .is_err());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_waits_for_the_exact_folder_and_never_changes_desktop_state() {
        let temp = tempfile::tempdir().unwrap();
        let canonical_home = fs::canonicalize(temp.path()).unwrap();
        let home = canonical_home.as_path();
        let project = home.join("new-project");
        fs::create_dir(&project).unwrap();
        let state = home.join(".codex-global-state.json");
        assert!(!project_is_registered(home, &project).unwrap());
        fs::write(
            &state,
            br#"{"local-projects":{"other":{"rootPaths":["/unrelated"]}}}"#,
        )
        .unwrap();
        let before = fs::read(&state).unwrap();
        assert!(wait_for_project_registration(
            home,
            &project,
            Instant::now() + Duration::from_millis(150)
        )
        .is_err());
        assert_eq!(fs::read(&state).unwrap(), before);

        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                write_bytes_atomic(
                    home,
                    &state,
                    &serde_json::to_vec(&json!({"local-projects": {
                        "new": {"rootPaths": [fs::canonicalize(&project).unwrap()]}
                    }}))
                    .unwrap(),
                    MAX_FRAME,
                )
                .unwrap();
            });
            wait_for_project_registration(home, &project, Instant::now() + Duration::from_secs(5))
                .unwrap();
        });
        let registered = fs::read(&state).unwrap();
        wait_for_project_registration(home, &project, Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert_eq!(fs::read(&state).unwrap(), registered);
    }

    #[test]
    fn registration_rejects_symlinks_special_files_and_oversized_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let state = home.join(".codex-global-state.json");
        let outside = home.join("outside.json");
        fs::write(&outside, b"{}").unwrap();
        std::os::unix::fs::symlink(&outside, &state).unwrap();
        assert!(project_is_registered(home, home).is_err());
        fs::remove_file(&state).unwrap();
        fs::create_dir(&state).unwrap();
        assert!(project_is_registered(home, home).is_err());
        fs::remove_dir(&state).unwrap();
        fs::File::create(&state)
            .unwrap()
            .set_len(MAX_FRAME + 1)
            .unwrap();
        assert!(project_is_registered(home, home).is_err());
    }

    #[test]
    fn helper_records_are_private_bounded_and_bound_to_the_selected_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let directory = profile_directory(&home).unwrap();
        DirBuilder::new().mode(0o700).create(&directory).unwrap();
        let path = directory.join(format!(
            "bridge-{}-019f7798-5437-7632-9dbc-5b589cf68bf0.json",
            std::process::id()
        ));
        let value = json!({"port":1234,"token":"a".repeat(64),"home":home});
        write_bytes_atomic(
            &directory,
            &path,
            &serde_json::to_vec(&value).unwrap(),
            4096,
        )
        .unwrap();
        assert_eq!(bridges(&directory).unwrap().len(), 1);
        let wrong = json!({"home":directory,"port":1234,"token":"a".repeat(64)});
        write_bytes_atomic(
            &directory,
            &path,
            &serde_json::to_vec(&wrong).unwrap(),
            4096,
        )
        .unwrap();
        assert!(bridges(&directory).unwrap().is_empty());
        fs::remove_file(&path).unwrap();
        let outside = home.join("outside.json");
        fs::write(&outside, serde_json::to_vec(&value).unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(bridges(&directory).unwrap().is_empty());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(bridges(&directory).is_err());
    }

    #[test]
    fn records_of_stopped_helpers_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let directory = profile_directory(&home).unwrap();
        DirBuilder::new().mode(0o700).create(&directory).unwrap();
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let path = directory.join(format!(
            "bridge-{pid}-019f7798-5437-7632-9dbc-5b589cf68bf0.json"
        ));
        let value = json!({"port":1234,"token":"a".repeat(64),"home":home});
        write_bytes_atomic(
            &directory,
            &path,
            &serde_json::to_vec(&value).unwrap(),
            4096,
        )
        .unwrap();
        assert!(bridges(&directory).unwrap().is_empty());
        assert!(!path.exists());
    }
}

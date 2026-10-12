//! Windows Desktop connection bound to the selected provider home.
//!
//! Restart Manager only identifies database owners; never request shutdown or
//! restart. A matching Core must be a live child of the registered Desktop,
//! and the connected pipe must be owned by that same Desktop process.

use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE, STILL_ACTIVE};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const PROFILE_UNAVAILABLE: &str = "Could not verify Codex Desktop uses the selected CODEX_HOME. Open Desktop with that profile, then try again.";

struct Process {
    handle: OwnedHandle,
    pid: u32,
    started: u64,
    image: PathBuf,
}

impl Process {
    fn open(pid: u32) -> Option<Self> {
        // Query-only handles also keep process identity stable if a PID is reused.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return None;
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let [mut started, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
        if unsafe { GetProcessTimes(raw, &mut started, &mut exited, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let mut image = vec![0u16; 32_768];
        let mut size = image.len() as u32;
        if unsafe { QueryFullProcessImageNameW(raw, 0, image.as_mut_ptr(), &mut size) } == 0 {
            return None;
        }
        let process = Self {
            handle,
            pid,
            started: filetime(started),
            image: PathBuf::from(OsString::from_wide(&image[..size as usize])),
        };
        process.alive().then_some(process)
    }

    fn alive(&self) -> bool {
        let mut code = 0;
        unsafe {
            GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) != 0
                && code == STILL_ACTIVE as u32
        }
    }
}

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

struct Profile {
    core: Process,
    desktop: Process,
}

impl Profile {
    fn alive(&self) -> bool {
        self.core.alive() && self.desktop.alive()
    }
}

fn database_owners(home: &Path) -> Result<Vec<RM_PROCESS_INFO>, String> {
    let entries = std::fs::read_dir(home).map_err(|_| PROFILE_UNAVAILABLE)?;
    let mut paths = Vec::new();
    for (count, entry) in entries.enumerate() {
        if count >= 10_000 {
            return Err("The selected Codex home contains too many entries.".into());
        }
        let entry = entry.map_err(|_| PROFILE_UNAVAILABLE)?;
        let name = entry.file_name();
        let Some(version) = name
            .to_str()
            .and_then(|s| s.strip_prefix("state_"))
            .and_then(|s| s.strip_suffix(".sqlite"))
        else {
            continue;
        };
        if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|_| PROFILE_UNAVAILABLE)?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("The Codex state database must be an ordinary local file.".into());
        }
        if paths.len() >= 16 {
            return Err("The selected Codex home contains too many state databases.".into());
        }
        paths.push(
            entry
                .path()
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>(),
        );
    }
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    let mut session_id = 0;
    if unsafe { RmStartSession(&mut session_id, 0, key.as_mut_ptr()) } != 0 {
        return Err("Windows could not inspect the Codex profile's owner.".into());
    }
    let session = Session(session_id);
    let names = paths.iter().map(|path| path.as_ptr()).collect::<Vec<_>>();
    if unsafe {
        RmRegisterResources(
            session.0,
            names.len() as u32,
            names.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    } != 0
    {
        return Err("Windows could not inspect the Codex profile's owner.".into());
    }
    let mut owners = vec![RM_PROCESS_INFO::default(); 128];
    let (mut needed, mut count, mut reasons) = (0, owners.len() as u32, 0);
    if unsafe {
        RmGetList(
            session.0,
            &mut needed,
            &mut count,
            owners.as_mut_ptr(),
            &mut reasons,
        )
    } != 0
    {
        return Err(
            "Windows could not establish the Codex profile's owner within its limit.".into(),
        );
    }
    owners.truncate(count as usize);
    Ok(owners)
}

fn process_parents() -> Result<HashMap<u32, u32>, String> {
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(PROFILE_UNAVAILABLE.into());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    let mut parents = HashMap::new();
    while more != 0 {
        if parents.len() >= 16_384 {
            return Err("Windows process listing exceeded its limit.".into());
        }
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    Ok(parents)
}

fn profile(executable: &Path, home: &Path) -> Result<Option<Profile>, String> {
    let owners = database_owners(home)?;
    if owners.is_empty() {
        return Ok(None);
    }
    let parents = process_parents()?;
    let mut matched: Option<Profile> = None;
    for owner in owners {
        let Some(core) = Process::open(owner.Process.dwProcessId) else {
            continue;
        };
        if core.started != filetime(owner.Process.ProcessStartTime)
            || !core
                .image
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
        {
            continue;
        }
        let Some(desktop) = parents.get(&core.pid).and_then(|pid| Process::open(*pid)) else {
            continue;
        };
        if desktop.started >= core.started
            || !crate::local_paths::same_canonical_path(&desktop.image, executable)
        {
            continue;
        }
        if matched
            .as_ref()
            .is_some_and(|p| p.desktop.pid != desktop.pid)
        {
            return Err("More than one Codex Desktop is using the selected profile.".into());
        }
        matched = Some(Profile { core, desktop });
    }
    Ok(matched)
}

fn pipe_paths() -> Result<BTreeSet<PathBuf>, String> {
    let mut paths = BTreeSet::new();
    for (count, entry) in std::fs::read_dir(r"\\.\pipe\")
        .map_err(|_| PROFILE_UNAVAILABLE)?
        .enumerate()
    {
        if count >= 16_384 {
            return Err("Windows pipe listing exceeded its limit.".into());
        }
        let entry = entry.map_err(|_| PROFILE_UNAVAILABLE)?;
        if entry.file_name().to_str().is_some_and(|name| {
            name.strip_prefix("codex-browser-use-")
                .is_some_and(|id| super::super::actions::validate_thread_uuid(id).is_ok())
        }) {
            paths.insert(entry.path());
        }
    }
    Ok(paths)
}

fn owned_by(pipe: &NamedPipeClient, pid: u32) -> bool {
    let mut owner = 0;
    unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut owner) != 0 && owner == pid }
}

pub(super) struct Desktop {
    pipe: NamedPipeClient,
    profile: Profile,
    id: u32,
    correlation: String,
}

pub(super) async fn connect(
    executable: &Path,
    home: &Path,
    deadline: Instant,
) -> Result<Desktop, String> {
    loop {
        if Instant::now() >= deadline {
            return Err(PROFILE_UNAVAILABLE.into());
        }
        if let Some(profile) = profile(executable, home)? {
            for path in pipe_paths()? {
                let Ok(mut pipe) = ClientOptions::new().open(path) else {
                    continue;
                };
                if owned_by(&pipe, profile.desktop.pid) && profile.alive() {
                    // Desktop initializes its tool dispatcher during discovery.
                    // This read-only handshake must precede the first call.
                    let timeout = deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(5));
                    let handshake = tokio::time::timeout(
                        timeout,
                        exchange(
                            &mut pipe,
                            0,
                            json!({"jsonrpc":"2.0", "id":0, "method":"tools/list",
                            "params":{"threadStartKind":"all"}}),
                        ),
                    )
                    .await;
                    let Ok(Ok(catalog)) = handshake else { continue };
                    if !supports_placement(&catalog) {
                        return Err(
                            "This Codex Desktop does not support native project placement.".into(),
                        );
                    }
                    let mut random = [0u8; 16];
                    getrandom::fill(&mut random)
                        .map_err(|_| "Could not identify the Desktop request.")?;
                    return Ok(Desktop {
                        pipe,
                        profile,
                        id: 0,
                        correlation: random.iter().map(|byte| format!("{byte:02x}")).collect(),
                    });
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

impl Desktop {
    pub(super) async fn call(
        &mut self,
        thread_id: &str,
        tool: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if !self.profile.alive() || !owned_by(&self.pipe, self.profile.desktop.pid) {
            return Err(PROFILE_UNAVAILABLE.into());
        }
        self.id += 1;
        let request = json!({"jsonrpc":"2.0", "id":self.id, "method":"tools/call", "params":{
            "namespace":"codex_app", "tool":tool, "arguments":arguments,
            "callerSource":"codex", "hostId":"local", "threadId":thread_id,
            "turnId":format!("mallard-{}", self.correlation),
            "callId":format!("mallard-{}-{}", self.correlation, self.id),
        }});
        let result = tokio::time::timeout(timeout, exchange(&mut self.pipe, self.id, request))
            .await
            .map_err(|_| {
                "Codex Desktop did not respond in time; the request was not repeated.".to_string()
            })??;
        tool_result(result)
    }
}

fn supports_placement(catalog: &Value) -> bool {
    catalog["tools"].as_array().is_some_and(|tools| {
        tools.len() <= 1024
            && [
                "list_projects",
                "move_thread_to_project",
                "navigate_to_codex_page",
            ]
            .iter()
            .all(|name| {
                tools
                    .iter()
                    .any(|tool| tool["namespace"] == "codex_app" && tool["name"] == *name)
            })
    })
}

fn tool_result(result: Value) -> Result<Value, String> {
    if result["success"] != true {
        // Do not log tool output, which can contain paths or conversations.
        return Err("Codex Desktop refused the operation. Check that the chat and project are available in Desktop.".into());
    }
    let text = result["contentItems"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["type"] == "inputText"))
        .and_then(|item| item["text"].as_str())
        .ok_or("Invalid Codex Desktop tool response.")?;
    serde_json::from_str(text).map_err(|_| "Invalid Codex Desktop tool response.".into())
}

async fn exchange(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    id: u32,
    request: Value,
) -> Result<Value, String> {
    let payload = serde_json::to_vec(&request).map_err(|_| "Invalid Desktop request.")?;
    if payload.len() > 64 * 1024 {
        return Err("Desktop request exceeded its limit.".into());
    }
    stream
        .write_u32_le(payload.len() as u32)
        .await
        .map_err(|_| "Codex Desktop connection closed.")?;
    stream
        .write_all(&payload)
        .await
        .map_err(|_| "Codex Desktop connection closed.")?;
    for _ in 0..32 {
        let length = stream
            .read_u32_le()
            .await
            .map_err(|_| "Codex Desktop connection closed.")? as usize;
        if length == 0 || length > MAX_FRAME {
            return Err("Codex Desktop response exceeded its limit.".into());
        }
        let mut bytes = vec![0; length];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|_| "Codex Desktop connection closed.")?;
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid Codex Desktop response.")?;
        if response["id"] != id {
            continue;
        }
        if response.get("error").is_some() {
            return Err("Codex Desktop could not handle the native request.".into());
        }
        return response
            .get("result")
            .cloned()
            .ok_or("Invalid Codex Desktop response.".into());
    }
    Err("Codex Desktop did not acknowledge the request.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "installed Windows Desktop; only the isolated native-placement POC fixture"]
    fn native_desktop_placement_uses_the_selected_home() {
        let root = crate::local_paths::canonicalize(
            std::env::var_os("MALLARD_NATIVE_PLACEMENT_ROOT").expect("fixture root"),
        )
        .unwrap();
        let temp_root = crate::local_paths::canonicalize(std::env::temp_dir()).unwrap();
        assert!(root.starts_with(temp_root));
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("mallard-native-placement-"));
        let state_path = root.join("poc-state.json");
        assert!(std::fs::metadata(&state_path).unwrap().len() < 64 * 1024);
        let state: Value = serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
        assert_eq!(state["marker"], "mallard-native-placement-v1");
        let home = crate::local_paths::canonicalize(state["home"].as_str().unwrap()).unwrap();
        let project = crate::local_paths::canonicalize(state["project"].as_str().unwrap()).unwrap();
        assert_eq!(home, root.join("selected-home"));
        assert_eq!(project, root.join("Placement fixture"));
        let app = crate::providers::windows_app::scheme_handler("codex").unwrap();
        assert!(crate::local_paths::same_canonical_path(
            &app.executable,
            Path::new(state["executable"].as_str().unwrap())
        ));
        let executable = &app.executable;
        let thread = state["threadId"].as_str().unwrap();
        let cli = Path::new(state["cli"].as_str().unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut desktop = runtime
            .block_on(connect(
                executable,
                &home,
                Instant::now() + Duration::from_secs(30),
            ))
            .unwrap();
        // Never operate on a Desktop merely inferred from a thread ID.
        assert_eq!(
            desktop.profile.desktop.pid,
            state["desktopPid"].as_u64().unwrap() as u32
        );
        let request = |method, params| {
            super::super::super::app_server::request(
                cli,
                None,
                &home,
                method,
                params,
                Duration::from_secs(15),
            )
            .unwrap()
        };
        let before = request(
            "thread/read",
            json!({"threadId":thread, "includeTurns":true}),
        );
        assert!(!before["thread"]["turns"].as_array().unwrap().is_empty());
        let call = |desktop: &mut Desktop, tool, args| {
            runtime
                .block_on(desktop.call(thread, tool, args, Duration::from_secs(20)))
                .unwrap()
        };
        let membership = |desktop: &mut Desktop| {
            let listing = call(desktop, "list_threads", json!({"limit":50}));
            listing["pinnedThreads"]
                .as_array()
                .unwrap()
                .iter()
                .chain(listing["threads"].as_array().unwrap())
                .find(|item| item["id"] == thread && item["hostId"] == "local")
                .expect("fixture chat in native listing")["projectId"]
                .clone()
        };
        call(
            &mut desktop,
            "move_thread_to_project",
            json!({"source":"codex", "hostId":"local", "threadId":thread, "projectId":null}),
        );
        assert!(membership(&mut desktop).is_null());
        let projects = request("project/list", json!({"limit":100}));
        let backend_project = projects["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["roots"].as_array().unwrap().iter().any(|root| {
                    crate::local_paths::same_canonical_path(
                        Path::new(root["path"].as_str().unwrap()),
                        &project,
                    )
                })
            })
            .unwrap();
        let updated = request(
            "thread/metadata/update",
            json!({"threadId":thread, "projectId":backend_project["id"]}),
        );
        assert_eq!(updated["thread"]["projectId"], backend_project["id"]);
        assert!(
            membership(&mut desktop).is_null(),
            "backend-only assignment must reproduce the bug"
        );

        for _ in 0..2 {
            super::super::assign_and_open(executable, &home, thread, &project, || Ok(())).unwrap();
            assert_eq!(membership(&mut desktop), state["fixtureProjectId"]);
        }
        // A copied rollout has the same chat ID but does not prove which home
        // the running Desktop uses.
        let other = tempfile::tempdir_in(&root).unwrap();
        std::fs::copy(
            state["threadPath"].as_str().unwrap(),
            other.path().join("same-chat.jsonl"),
        )
        .unwrap();
        std::fs::write(other.path().join("state_5.sqlite"), b"fixture").unwrap();
        assert!(runtime
            .block_on(connect(
                executable,
                other.path(),
                Instant::now() + Duration::from_millis(300)
            ))
            .is_err());
        assert_eq!(membership(&mut desktop), state["fixtureProjectId"]);
        let after = request(
            "thread/read",
            json!({"threadId":thread, "includeTurns":true}),
        );
        for key in ["id", "name", "preview", "turns"] {
            assert_eq!(before["thread"][key], after["thread"][key], "{key} changed");
        }
    }

    #[test]
    fn discovers_only_processes_holding_the_selected_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state_5.sqlite");
        std::fs::write(&path, b"fixture").unwrap();
        let _held = std::fs::File::open(path).unwrap();
        let owners = database_owners(temp.path()).unwrap();
        assert!(owners
            .iter()
            .any(|owner| owner.Process.dwProcessId == std::process::id()));
        let other = tempfile::tempdir().unwrap();
        assert!(database_owners(other.path()).unwrap().is_empty());
        // An arbitrary process opening the database is insufficient.
        assert!(profile(&std::env::current_exe().unwrap(), temp.path())
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn validates_pipe_owner_before_sending_any_request() {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = format!(r"\\.\pipe\mallard-placement-test-{}", std::process::id());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        assert!(owned_by(&client, std::process::id()));
        assert!(!owned_by(&client, 0));
    }

    #[tokio::test]
    async fn refuses_oversized_native_frames() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let reply = tokio::spawn(async move {
            let size = server.read_u32_le().await.unwrap();
            let mut request = vec![0; size as usize];
            server.read_exact(&mut request).await.unwrap();
            server.write_u32_le(MAX_FRAME as u32 + 1).await.unwrap();
        });
        assert!(exchange(&mut client, 1, json!({"id":1}))
            .await
            .unwrap_err()
            .contains("limit"));
        reply.await.unwrap();
    }
}

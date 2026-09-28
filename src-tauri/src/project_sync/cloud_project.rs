//! A project got from a Mallard Cloud repository: where its folder may go,
//! and the folder itself. A repository whose GitHub address the service
//! knows is cloned — with gh when it is installed, then git — and anything
//! else, or a clone that does not work, gets an empty folder, exactly as a
//! project without Git does. A folder the person already has is connected
//! as it is, and never comes through here. Connecting is setup's own finish,
//! which the page runs next with the repository chosen; nothing here writes
//! Mallard state.
use super::service::{
    origin_url, probe_project_folder_without_git, repository,
    repository_fingerprint_from_remote_url, ProjectFolderProbe,
};
use mallard_domain::{sha256_hex, LocalProjectId};
use mallard_infrastructure::codex_binary::find_binary;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::project_activity::normalize_github_repository_url;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

/// Where a new project's folder may go.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CloudProjectFolder {
    /// Not somewhere a project can go, in a few words.
    Refused { message: String },
    /// Nothing there yet, under a folder setup accepts.
    New,
    /// An empty folder.
    Empty,
    /// Already this repository's checkout: nothing to clone.
    Checkout,
    /// Holds files. `suggestion` is the first free `name-N` beside it, empty
    /// when there is none. Connected as it is, the folder's own origin —
    /// its fingerprint, for setup's acknowledgement rule, and `git_remote`,
    /// for the Folder line — is what the repository is checked against.
    Occupied {
        suggestion: String,
        local_fingerprint: Option<String>,
        git_remote: Option<String>,
    },
    /// Already a Mallard project: somewhere to open, not to set up again.
    Project { project_id: LocalProjectId },
}

/// The Folder field's verdict, and whether a clone can be tried at all.
#[derive(Serialize, Clone, Debug)]
pub struct CloudProjectFolderProbe {
    pub folder: CloudProjectFolder,
    /// Whether Git runs on this computer; gh clones through it too.
    pub can_clone: bool,
}

#[derive(Deserialize, Debug)]
pub struct CloudProjectFolderRequest {
    pub destination: String,
    /// `https://github.com/{owner}/{name}`, when the service knows it.
    pub github_url: Option<String>,
    pub repository_fingerprint: Option<String>,
    /// Names this run for Cancel.
    pub operation_id: String,
}

/// The folder that is now there.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CloudProjectFolderResult {
    Cloned {
        canonical_path: String,
    },
    /// Empty: made here, or already there and empty. `failure` says why a
    /// GitHub repository was not cloned.
    Created {
        canonical_path: String,
        failure: Option<CloneFailure>,
    },
    /// Already this repository's checkout.
    Existing {
        canonical_path: String,
    },
}

/// Why a GitHub repository ended up in an empty folder, and the command
/// that clones into it later.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CloneFailure {
    pub reason: CloneFailureReason,
    pub command: String,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloneFailureReason {
    /// Neither gh nor git runs here.
    NoGit,
    /// The host said no: no access, or not signed in.
    Refused,
    /// The host could not be reached.
    Unreachable,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum CloudProjectFolderProgress {
    Cloning {
        percent: Option<u8>,
    },
    /// Making the empty folder: no clone, or none worked.
    Creating,
}

/* ——— the destination ——————————————————————————————————————————————— */

pub(super) fn probe_folder(
    repository: &LocalStateRepository,
    destination: &str,
    fingerprint: Option<&str>,
) -> Result<CloudProjectFolder, String> {
    let refused = |message: &str| {
        Ok(CloudProjectFolder::Refused {
            message: message.to_string(),
        })
    };
    match probe_project_folder_without_git(repository, destination)? {
        ProjectFolderProbe::Invalid { .. } => refused("Enter a full path."),
        ProjectFolderProbe::Unreadable { .. } => refused("Can’t read this folder."),
        ProjectFolderProbe::NotADirectory { .. } => refused("That’s a file."),
        ProjectFolderProbe::MallardData { .. } => refused("That’s Mallard’s own data."),
        ProjectFolderProbe::Missing => {
            // One new folder, under a parent that is already there.
            let path = Path::new(destination);
            let (Some(parent), Some(_)) = (path.parent(), path.file_name()) else {
                return refused("Enter a full path.");
            };
            match probe_project_folder_without_git(repository, &parent.to_string_lossy())? {
                ProjectFolderProbe::Folder { .. } => Ok(CloudProjectFolder::New),
                ProjectFolderProbe::Missing => refused("Its parent folder doesn’t exist."),
                ProjectFolderProbe::MallardData { .. } => {
                    refused("That’s inside Mallard’s own data.")
                }
                _ => refused("Its parent can’t hold a project."),
            }
        }
        ProjectFolderProbe::Folder {
            canonical_path,
            is_git_repository,
            existing_project_ids,
            ..
        } => {
            if let Some(project_id) = existing_project_ids.into_iter().next() {
                return Ok(CloudProjectFolder::Project { project_id });
            }
            let canonical = PathBuf::from(&canonical_path);
            if folder_is_empty(&canonical)? {
                return Ok(CloudProjectFolder::Empty);
            }
            let origin = origin_url(&canonical);
            let local_fingerprint = origin
                .as_deref()
                .map(repository_fingerprint_from_remote_url);
            if is_git_repository
                && fingerprint.is_some()
                && local_fingerprint.as_deref() == fingerprint
            {
                return Ok(CloudProjectFolder::Checkout);
            }
            Ok(CloudProjectFolder::Occupied {
                suggestion: free_sibling(Path::new(destination)).unwrap_or_default(),
                local_fingerprint,
                git_remote: origin.as_deref().map(remote_display),
            })
        }
    }
}

/// A remote as the Folder line shows it: host and path only. The scheme,
/// user, query, fragment and `.git` are left off — a token can sit in any of
/// them — and a GitHub remote reads in its canonical form. A remote with no
/// host (a path on this computer) stays a path.
fn remote_display(url: &str) -> String {
    let url = url.trim();
    if let Some(canonical) = normalize_github_repository_url(url) {
        return canonical.trim_start_matches("https://").to_string();
    }
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let rest = rest.split(['?', '#']).next().unwrap_or_default();
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        (authority.rsplit('@').next().unwrap_or_default(), path)
    } else if let Some((authority, path)) = url
        .split_once(':')
        // `user@host:path`; a drive letter (`C:\…`) or a path is not a host.
        .filter(|(authority, _)| authority.len() > 1 && !authority.contains(['/', '\\']))
    {
        (authority.rsplit('@').next().unwrap_or_default(), path)
    } else {
        return url.strip_suffix(".git").unwrap_or(url).to_string();
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    match (host.is_empty(), path.is_empty()) {
        (true, _) => format!("/{path}"),
        (false, true) => host.to_string(),
        (false, false) => format!("{host}/{path}"),
    }
}

fn folder_is_empty(folder: &Path) -> Result<bool, String> {
    let mut entries =
        fs::read_dir(folder).map_err(|error| format!("read '{}': {}", folder.display(), error))?;
    Ok(entries.next().is_none())
}

/// Absent, or an empty folder: somewhere a clone may still land.
fn absent_or_empty(destination: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!("read '{}': {}", destination.display(), error)),
        Ok(metadata) => Ok(metadata.is_dir() && folder_is_empty(destination)?),
    }
}

/// The first `name-N` beside the destination that nothing occupies.
fn free_sibling(destination: &Path) -> Option<String> {
    let parent = destination.parent()?;
    let name = destination.file_name()?.to_string_lossy();
    (2..=99)
        .map(|n| parent.join(format!("{name}-{n}")))
        .find(|candidate| {
            matches!(
                fs::symlink_metadata(candidate),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            )
        })
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

/* ——— the tools ————————————————————————————————————————————————————— */

#[derive(Clone, Debug, Default)]
pub(super) struct CloneTools {
    gh: Option<PathBuf>,
    git: Option<PathBuf>,
}

static CLONE_TOOLS: OnceLock<CloneTools> = OnceLock::new();

/// Found once per run of the app: a login-shell lookup is too slow to repeat
/// on every keystroke of the Folder field.
fn clone_tools() -> CloneTools {
    CLONE_TOOLS
        .get_or_init(|| CloneTools {
            gh: find_binary("gh").ok(),
            git: usable_git(),
        })
        .clone()
}

/// A git that runs. On macOS `/usr/bin/git` is a stub until Apple's
/// command line tools are installed, and running it offers to install them,
/// so it counts only once `xcode-select -p` names a developer folder.
fn usable_git() -> Option<PathBuf> {
    let found = find_binary("git").ok();
    #[cfg(target_os = "macos")]
    {
        let stub = Path::new("/usr/bin/git");
        let developer_tools = || {
            std::process::Command::new("/usr/bin/xcode-select")
                .arg("-p")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        found
            .into_iter()
            .chain(["/opt/homebrew/bin/git", "/usr/local/bin/git"].map(PathBuf::from))
            .find(|candidate| {
                if candidate == stub {
                    developer_tools()
                } else {
                    candidate.is_file()
                }
            })
    }
    #[cfg(not(target_os = "macos"))]
    found
}

/* ——— the folder ———————————————————————————————————————————————————— */

/// What gets cloned: `owner/name` for gh, the HTTPS URL for git.
#[derive(Clone, Debug)]
pub(super) struct CloneSource {
    gh_repository: Option<String>,
    git_url: String,
}

/// The service's GitHub address, only in its canonical form, and only when
/// it is the repository's own remote.
fn github_source(url: &str, fingerprint: Option<&str>) -> Result<CloneSource, String> {
    let canonical = normalize_github_repository_url(url)
        .filter(|canonical| canonical == url)
        .ok_or_else(|| "The repository's GitHub address isn't valid.".to_string())?;
    if fingerprint.is_some_and(|fingerprint| sha256_hex(canonical.as_bytes()) != fingerprint) {
        return Err("The GitHub address isn't this repository's remote.".to_string());
    }
    Ok(CloneSource {
        gh_repository: canonical
            .strip_prefix("https://github.com/")
            .map(str::to_owned),
        git_url: format!("{canonical}.git"),
    })
}

enum CloneError {
    Cancelled,
    Failed(CloneFailureReason),
}

/// Makes the folder: tries each clone in turn, and when none works (or
/// there is nothing to clone) leaves the folder empty. A cancelled clone
/// stops the run; git removes what it wrote on the way out.
pub(super) async fn get_folder(
    repository: LocalStateRepository,
    tools: CloneTools,
    destination: String,
    fingerprint: Option<String>,
    source: Option<CloneSource>,
    mut cancellation: watch::Receiver<bool>,
    on_progress: &(dyn Fn(CloudProjectFolderProgress) + Send + Sync),
) -> Result<CloudProjectFolderResult, String> {
    let folder = {
        let destination = destination.clone();
        let fingerprint = fingerprint.clone();
        tauri::async_runtime::spawn_blocking(move || {
            probe_folder(&repository, &destination, fingerprint.as_deref())
        })
        .await
        .map_err(|error| format!("project-sync worker failed: {error}"))??
    };
    let path = PathBuf::from(&destination);
    let existed = matches!(folder, CloudProjectFolder::Empty);
    match folder {
        CloudProjectFolder::Refused { message } => return Err(message),
        CloudProjectFolder::Project { .. } => return Err("Already a project here.".to_string()),
        CloudProjectFolder::Occupied { .. } => {
            return Err("This folder already holds files.".to_string())
        }
        CloudProjectFolder::Checkout => {
            return Ok(CloudProjectFolderResult::Existing {
                canonical_path: canonical(&path)?,
            })
        }
        CloudProjectFolder::New | CloudProjectFolder::Empty => {}
    }

    let mut failure = None;
    if let Some(source) = &source {
        let command = retry_command(&tools, source, &destination);
        match &tools.git {
            None => {
                failure = Some(CloneFailure {
                    reason: CloneFailureReason::NoGit,
                    command,
                });
            }
            Some(git) => {
                let mut attempts: Vec<(PathBuf, Vec<OsString>)> = Vec::new();
                if let (Some(gh), Some(repository)) = (&tools.gh, &source.gh_repository) {
                    attempts.push((
                        gh.clone(),
                        [
                            "repo",
                            "clone",
                            repository.as_str(),
                            destination.as_str(),
                            "--",
                            "--progress",
                        ]
                        .map(OsString::from)
                        .to_vec(),
                    ));
                }
                attempts.push((
                    git.clone(),
                    [
                        "clone",
                        "--progress",
                        "--",
                        source.git_url.as_str(),
                        destination.as_str(),
                    ]
                    .map(OsString::from)
                    .to_vec(),
                ));
                let mut reason = CloneFailureReason::Failed;
                for (program, args) in attempts {
                    if !absent_or_empty(&path)? {
                        return Err("The folder isn’t empty after a clone that failed.".to_string());
                    }
                    on_progress(CloudProjectFolderProgress::Cloning { percent: None });
                    match run_clone(
                        &program,
                        &args,
                        git.parent(),
                        &mut cancellation,
                        on_progress,
                    )
                    .await
                    {
                        Ok(()) if path.join(".git").is_dir() => {
                            return Ok(CloudProjectFolderResult::Cloned {
                                canonical_path: canonical(&path)?,
                            })
                        }
                        Ok(()) => reason = CloneFailureReason::Failed,
                        Err(CloneError::Cancelled) => {
                            settle_after_cancel(&path).await;
                            return Err("Stopped.".to_string());
                        }
                        Err(CloneError::Failed(failed)) => reason = failed,
                    }
                }
                failure = Some(CloneFailure { reason, command });
            }
        }
    }

    on_progress(CloudProjectFolderProgress::Creating);
    if !existed {
        match fs::create_dir(&path) {
            Ok(()) => {}
            // A clone that failed can leave the folder it made, empty.
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists && absent_or_empty(&path)? => {
            }
            Err(error) => return Err(format!("Couldn’t create the folder: {error}.")),
        }
    }
    Ok(CloudProjectFolderResult::Created {
        canonical_path: canonical(&path)?,
        failure,
    })
}

fn canonical(path: &Path) -> Result<String, String> {
    fs::canonicalize(path)
        .map(|canonical| canonical.to_string_lossy().into_owned())
        .map_err(|error| format!("resolve '{}': {}", path.display(), error))
}

/// The clone to run by hand later, the way this computer can.
fn retry_command(tools: &CloneTools, source: &CloneSource, destination: &str) -> String {
    let destination = shell_quote(destination);
    match (&tools.gh, &source.gh_repository) {
        (Some(_), Some(repository)) => format!("gh repo clone {repository} {destination}"),
        _ => format!("git clone {} {destination}", source.git_url),
    }
}

fn shell_quote(value: &str) -> String {
    let plain = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/._-~:\\".contains(&byte));
    if plain {
        value.to_string()
    } else if cfg!(windows) {
        format!("\"{value}\"")
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// One clone, prompts off: without a terminal a sign-in prompt would wait
/// forever, so git, gh, ssh and Git Credential Manager are all told not to
/// ask, and the process leads its own session so it has no terminal at all.
async fn run_clone(
    program: &Path,
    args: &[OsString],
    git_folder: Option<&Path>,
    cancellation: &mut watch::Receiver<bool>,
    on_progress: &(dyn Fn(CloudProjectFolderProgress) + Send + Sync),
) -> Result<(), CloneError> {
    if *cancellation.borrow() {
        return Err(CloneError::Cancelled);
    }
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GH_PROMPT_DISABLED", "1")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .kill_on_drop(true);
    // gh runs git itself: the git found here goes first on its PATH.
    if let Some(folder) = git_folder {
        let mut paths = vec![folder.to_path_buf()];
        paths.extend(
            std::env::var_os("PATH")
                .iter()
                .flat_map(std::env::split_paths),
        );
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("PATH", joined);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches only the child.
        unsafe {
            command.as_std_mut().pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| CloneError::Failed(CloneFailureReason::Failed))?;
    let process_id = child.id();
    let mut stderr = child
        .stderr
        .take()
        .ok_or(CloneError::Failed(CloneFailureReason::Failed))?;
    let outcome = tokio::select! {
        biased;
        _ = async {
            while !*cancellation.borrow_and_update() {
                if cancellation.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        } => None,
        finished = async {
            let read = async {
                let mut tail = Vec::new();
                let mut line = Vec::new();
                let mut last = None;
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stderr.read(&mut buffer).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    for &byte in &buffer[..read] {
                        if byte == b'\r' || byte == b'\n' {
                            let percent = receiving_percent(&line);
                            if percent.is_some() && percent != last {
                                last = percent;
                                on_progress(CloudProjectFolderProgress::Cloning { percent });
                            }
                            line.clear();
                        } else if line.len() < 512 {
                            line.push(byte);
                        }
                    }
                    tail.extend_from_slice(&buffer[..read]);
                    if tail.len() > 16 * 1024 {
                        tail.drain(..tail.len() - 16 * 1024);
                    }
                }
                tail
            };
            tokio::join!(read, child.wait())
        } => Some(finished),
    };
    match outcome {
        None => {
            stop(&mut child, process_id).await;
            Err(CloneError::Cancelled)
        }
        Some((_, Ok(status))) if status.success() => Ok(()),
        Some((tail, _)) => Err(CloneError::Failed(classify(&String::from_utf8_lossy(
            &tail,
        )))),
    }
}

/// `Receiving objects:  48% (…)` → 48.
fn receiving_percent(line: &[u8]) -> Option<u8> {
    let text = std::str::from_utf8(line).ok()?;
    let rest = text.trim_start().strip_prefix("Receiving objects:")?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse::<u8>().ok().filter(|percent| *percent <= 100)
}

fn classify(stderr: &str) -> CloneFailureReason {
    let text = stderr.to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if any(&[
        "could not resolve host",
        "could not resolve hostname",
        "failed to connect",
        "network is unreachable",
        "timed out",
        "connection refused",
    ]) {
        CloneFailureReason::Unreachable
    } else if any(&[
        "authentication failed",
        "permission denied",
        "repository not found",
        "could not read username",
        "terminal prompts disabled",
        "could not read from remote repository",
        "could not resolve to a repository",
        "returned error: 403",
        "returned error: 401",
        "gh auth login",
        "host key verification failed",
    ]) {
        CloneFailureReason::Refused
    } else {
        CloneFailureReason::Failed
    }
}

/// Stops a clone the way a terminal's Ctrl-C would: git removes the folder
/// it was writing on SIGTERM. Anything still running after ten seconds is
/// killed.
async fn stop(child: &mut tokio::process::Child, process_id: Option<u32>) {
    #[cfg(unix)]
    {
        if let Some(pid) = process_id {
            // The clone leads its own session and group; gh's git is in it too.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
            if tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .is_ok()
            {
                return;
            }
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(pid) = process_id {
            let taskkill = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap_or_else(|| "C:\\Windows".into())
                .join("System32/taskkill.exe");
            let mut command = tokio::process::Command::new(taskkill);
            command
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .kill_on_drop(true);
            let _ = tokio::time::timeout(Duration::from_secs(5), command.status()).await;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// gh can exit before the git it started has finished removing its folder;
/// wait a moment for that, so Try again starts from a clean destination.
async fn settle_after_cancel(destination: &Path) {
    for _ in 0..50 {
        if absent_or_empty(destination).unwrap_or(true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/* ——— one run at a time, cancellable ———————————————————————————————— */

#[derive(Default)]
pub struct CloudProjectControl(Mutex<ControlState>);

#[derive(Default)]
struct ControlState {
    active: Option<(String, watch::Sender<bool>)>,
    /// A Cancel that arrived before its run began.
    cancelled: Option<String>,
}

struct Run<'a> {
    control: &'a CloudProjectControl,
    cancellation: watch::Receiver<bool>,
}

impl CloudProjectControl {
    fn begin(&self, operation_id: &str) -> Result<Run<'_>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "Another project is being set up.".to_string())?;
        if state.active.is_some() {
            return Err("Another project is being set up.".to_string());
        }
        if state.cancelled.take().as_deref() == Some(operation_id) {
            return Err("Stopped.".to_string());
        }
        let (sender, cancellation) = watch::channel(false);
        state.active = Some((operation_id.to_string(), sender));
        Ok(Run {
            control: self,
            cancellation,
        })
    }

    fn cancel(&self, operation_id: &str) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        match &state.active {
            Some((active, sender)) if active == operation_id => {
                sender.send_replace(true);
            }
            Some(_) => {}
            None => state.cancelled = Some(operation_id.to_string()),
        }
    }
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.0.lock() {
            state.active = None;
        }
    }
}

fn valid_fingerprint(fingerprint: Option<String>) -> Result<Option<String>, String> {
    match fingerprint {
        Some(value)
            if value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            Ok(Some(value))
        }
        Some(_) => Err("repository fingerprint must be a SHA-256 hex digest".to_string()),
        None => Ok(None),
    }
}

fn valid_operation_id(operation_id: &str) -> Result<(), String> {
    if !operation_id.is_empty()
        && operation_id.len() <= 64
        && operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        Err("operation id must be 1-64 letters, digits, or hyphens".to_string())
    }
}

/* ——— commands ——————————————————————————————————————————————————————— */

/// The Folder field's verdict for a new project's destination. Reads only.
#[tauri::command]
pub async fn probe_cloud_project_folder(
    destination: String,
    repository_fingerprint: Option<String>,
) -> Result<CloudProjectFolderProbe, String> {
    let fingerprint = valid_fingerprint(repository_fingerprint)?;
    let repository = repository()?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(CloudProjectFolderProbe {
            folder: probe_folder(&repository, &destination, fingerprint.as_deref())?,
            can_clone: clone_tools().git.is_some(),
        })
    })
    .await
    .map_err(|error| format!("project-sync worker failed: {error}"))?
}

/// Makes the new project's folder: a clone when one works, else empty.
#[tauri::command]
pub async fn get_cloud_project_folder(
    control: tauri::State<'_, CloudProjectControl>,
    request: CloudProjectFolderRequest,
    on_progress: tauri::ipc::Channel<CloudProjectFolderProgress>,
) -> Result<CloudProjectFolderResult, String> {
    valid_operation_id(&request.operation_id)?;
    let fingerprint = valid_fingerprint(request.repository_fingerprint)?;
    let source = request
        .github_url
        .as_deref()
        .map(|url| github_source(url, fingerprint.as_deref()))
        .transpose()?;
    let run = control.begin(&request.operation_id)?;
    let repository = repository()?;
    let tools = tauri::async_runtime::spawn_blocking(clone_tools)
        .await
        .map_err(|error| format!("project-sync worker failed: {error}"))?;
    get_folder(
        repository,
        tools,
        request.destination,
        fingerprint,
        source,
        run.cancellation.clone(),
        &|progress| {
            let _ = on_progress.send(progress);
        },
    )
    .await
}

/// Stops the clone of that run; git removes what it wrote.
#[tauri::command]
pub fn cancel_cloud_project_folder(
    control: tauri::State<'_, CloudProjectControl>,
    operation_id: String,
) -> Result<(), String> {
    valid_operation_id(&operation_id)?;
    control.cancel(&operation_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::service::{
        create_setup_draft_with_repository, finalize_project_setup_with_repository,
        update_setup_draft_with_repository,
    };
    use super::*;
    use mallard_domain::{DraftProfileSelection, ProviderKey};
    use std::collections::BTreeMap;
    use std::process::Command;

    struct Fixture {
        _temp: tempfile::TempDir,
        repository: LocalStateRepository,
        root: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let repository = LocalStateRepository::from_home_dir(root.join("home")).unwrap();
        fs::create_dir_all(root.join("code")).unwrap();
        Fixture {
            _temp: temp,
            repository,
            root,
        }
    }

    fn git(folder: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(folder)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Mallard")
            .env("GIT_AUTHOR_EMAIL", "mallard@example.test")
            .env("GIT_COMMITTER_NAME", "Mallard")
            .env("GIT_COMMITTER_EMAIL", "mallard@example.test")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with one commit, served as a bare clone on disk.
    fn bare_remote(root: &Path) -> String {
        let work = root.join("remote-work");
        fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "--initial-branch=main"]);
        fs::write(work.join("README.md"), "portal\n").unwrap();
        git(&work, &["add", "README.md"]);
        git(&work, &["commit", "-m", "first"]);
        git(root, &["clone", "--bare", "remote-work", "remote.git"]);
        format!("file://{}", root.join("remote.git").display())
    }

    fn git_only() -> CloneTools {
        CloneTools {
            gh: None,
            git: find_binary("git").ok(),
        }
    }

    fn source(url: &str) -> CloneSource {
        CloneSource {
            gh_repository: None,
            git_url: url.to_string(),
        }
    }

    async fn get(
        fixture: &Fixture,
        tools: CloneTools,
        destination: &Path,
        source: Option<CloneSource>,
    ) -> Result<CloudProjectFolderResult, String> {
        let progress = Mutex::new(Vec::new());
        get_folder(
            fixture.repository.clone(),
            tools,
            destination.to_string_lossy().into_owned(),
            None,
            source,
            watch::channel(false).1,
            &|event| progress.lock().unwrap().push(event),
        )
        .await
    }

    #[test]
    fn the_destination_is_new_empty_a_checkout_or_occupied() {
        let fixture = fixture();
        let probe = |destination: &Path, fingerprint: Option<&str>| {
            probe_folder(
                &fixture.repository,
                &destination.to_string_lossy(),
                fingerprint,
            )
            .unwrap()
        };
        let portal = fixture.root.join("code/portal");
        assert_eq!(probe(&portal, None), CloudProjectFolder::New);
        assert_eq!(
            probe(&fixture.root.join("missing/portal"), None),
            CloudProjectFolder::Refused {
                message: "Its parent folder doesn’t exist.".into()
            }
        );
        assert!(matches!(
            probe(Path::new("code/portal"), None),
            CloudProjectFolder::Refused { .. }
        ));
        assert!(matches!(
            probe(&fixture.repository.root().join("portal"), None),
            CloudProjectFolder::Refused { .. }
        ));

        fs::create_dir(&portal).unwrap();
        assert_eq!(probe(&portal, None), CloudProjectFolder::Empty);

        fs::write(portal.join("notes.txt"), "mine").unwrap();
        fs::create_dir(fixture.root.join("code/portal-2")).unwrap();
        let suggestion = fixture
            .root
            .join("code/portal-3")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            probe(&portal, None),
            CloudProjectFolder::Occupied {
                suggestion: suggestion.clone(),
                local_fingerprint: None,
                git_remote: None,
            }
        );

        // This repository's checkout, by its origin's fingerprint.
        git(&portal, &["init", "--initial-branch=main"]);
        git(
            &portal,
            &["remote", "add", "origin", "git@github.com:acme/portal.git"],
        );
        let fingerprint = sha256_hex(b"https://github.com/acme/portal");
        assert_eq!(
            probe(&portal, Some(&fingerprint)),
            CloudProjectFolder::Checkout
        );
        // Another repository's folder carries its own origin, which the page
        // checks the repository against before connecting it as it is.
        assert_eq!(
            probe(&portal, Some(&sha256_hex(b"https://github.com/acme/other"))),
            CloudProjectFolder::Occupied {
                suggestion,
                local_fingerprint: Some(fingerprint),
                git_remote: Some("github.com/acme/portal".into()),
            }
        );
    }

    #[test]
    fn a_folder_that_is_already_a_project_is_one_to_open() {
        let fixture = fixture();
        let app = fixture.root.join("code/app");
        let codex_home = fixture.root.join("codex-profile/.codex");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        let mut draft =
            create_setup_draft_with_repository(&fixture.repository, &app.to_string_lossy())
                .unwrap()
                .draft;
        draft.profiles = BTreeMap::from([(
            ProviderKey::parse("codex").unwrap(),
            DraftProfileSelection::Pending {
                path: codex_home.to_string_lossy().into_owned(),
                display_name: String::new(),
            },
        )]);
        let draft = update_setup_draft_with_repository(&fixture.repository, draft).unwrap();
        let detail = finalize_project_setup_with_repository(
            &fixture.repository,
            &draft.draft_id,
            draft.revision,
        )
        .unwrap();
        assert_eq!(
            probe_folder(&fixture.repository, &app.to_string_lossy(), None).unwrap(),
            CloudProjectFolder::Project {
                project_id: detail.project.local_project_id
            }
        );
    }

    #[test]
    fn a_remote_shows_as_host_and_path_only() {
        for (remote, shown) in [
            ("git@github.com:acme/portal.git", "github.com/acme/portal"),
            (
                "https://x-access-token:secret@github.com/acme/portal.git",
                "github.com/acme/portal",
            ),
            (
                "https://token@gitlab.com/acme/portal.git?private_token=x#top",
                "gitlab.com/acme/portal",
            ),
            (
                "ssh://git@gitlab.example.com:2222/acme/portal.git",
                "gitlab.example.com:2222/acme/portal",
            ),
            ("git@gitlab.com:acme/portal.git", "gitlab.com/acme/portal"),
            ("file:///srv/git/portal.git", "/srv/git/portal"),
            ("/srv/git/portal.git", "/srv/git/portal"),
            ("C:\\repos\\portal", "C:\\repos\\portal"),
        ] {
            assert_eq!(remote_display(remote), shown, "{remote}");
        }
    }

    #[test]
    fn only_the_repository_s_own_github_address_is_cloned() {
        let url = "https://github.com/acme/portal";
        let fingerprint = sha256_hex(url.as_bytes());
        let source = github_source(url, Some(&fingerprint)).unwrap();
        assert_eq!(source.gh_repository.as_deref(), Some("acme/portal"));
        assert_eq!(source.git_url, "https://github.com/acme/portal.git");
        assert!(github_source(url, Some(&sha256_hex(b"https://github.com/acme/other"))).is_err());
        assert!(github_source("https://github.com/acme/portal.git", None).is_err());
        assert!(github_source("https://example.test/acme/portal", None).is_err());
        assert!(github_source("--upload-pack=touch /tmp/x", None).is_err());
    }

    #[test]
    fn progress_failures_and_commands_read_as_they_should() {
        assert_eq!(
            receiving_percent(b"Receiving objects:  48% (12/25)"),
            Some(48)
        );
        assert_eq!(
            receiving_percent(b"remote: Counting objects: 10% (1/10)"),
            None
        );
        assert_eq!(
            classify("fatal: unable to access 'https://github.com/a/b/': Could not resolve host: github.com"),
            CloneFailureReason::Unreachable
        );
        assert_eq!(
            classify("remote: Repository not found.\nfatal: repository 'https://github.com/a/b.git/' not found"),
            CloneFailureReason::Refused
        );
        assert_eq!(
            classify("fatal: something else"),
            CloneFailureReason::Failed
        );
        let tools = CloneTools {
            gh: Some(PathBuf::from("/opt/homebrew/bin/gh")),
            git: None,
        };
        let source = github_source("https://github.com/acme/portal", None).unwrap();
        assert_eq!(
            retry_command(&tools, &source, "/Users/me/code/portal"),
            "gh repo clone acme/portal /Users/me/code/portal"
        );
        assert_eq!(
            retry_command(&CloneTools::default(), &source, "/Users/me/My Code/portal"),
            "git clone https://github.com/acme/portal.git '/Users/me/My Code/portal'"
        );
    }

    #[tokio::test]
    async fn a_clone_that_works_lands_as_a_checkout() {
        let fixture = fixture();
        let remote = bare_remote(&fixture.root);
        let destination = fixture.root.join("code/portal");
        let result = get(&fixture, git_only(), &destination, Some(source(&remote)))
            .await
            .unwrap();
        assert!(matches!(result, CloudProjectFolderResult::Cloned { .. }));
        assert!(destination.join(".git").is_dir());
        assert!(destination.join("README.md").is_file());
    }

    #[tokio::test]
    async fn a_clone_that_fails_leaves_an_empty_folder_and_says_why() {
        let fixture = fixture();
        let missing = format!("file://{}", fixture.root.join("nowhere.git").display());
        let destination = fixture.root.join("code/portal");
        let result = get(&fixture, git_only(), &destination, Some(source(&missing)))
            .await
            .unwrap();
        let CloudProjectFolderResult::Created {
            failure: Some(failure),
            ..
        } = result
        else {
            panic!("an empty folder with a reason: {result:?}");
        };
        assert!(failure.command.starts_with("git clone "));
        assert!(destination.is_dir());
        assert!(folder_is_empty(&destination).unwrap());

        // An existing empty folder stays, empty, the same way.
        let result = get(&fixture, git_only(), &destination, Some(source(&missing)))
            .await
            .unwrap();
        assert!(matches!(
            result,
            CloudProjectFolderResult::Created {
                failure: Some(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn no_git_or_no_github_address_means_an_empty_folder() {
        let fixture = fixture();
        let github = github_source("https://github.com/acme/portal", None).unwrap();
        let destination = fixture.root.join("code/portal");
        let result = get(&fixture, CloneTools::default(), &destination, Some(github))
            .await
            .unwrap();
        let CloudProjectFolderResult::Created {
            failure: Some(failure),
            ..
        } = result
        else {
            panic!("no git: an empty folder with a reason: {result:?}");
        };
        assert_eq!(failure.reason, CloneFailureReason::NoGit);

        let destination = fixture.root.join("code/field-notes");
        let result = get(&fixture, git_only(), &destination, None).await.unwrap();
        assert!(matches!(
            result,
            CloudProjectFolderResult::Created { failure: None, .. }
        ));
        assert!(folder_is_empty(&destination).unwrap());
    }

    #[tokio::test]
    async fn an_occupied_or_refused_destination_is_never_touched() {
        let fixture = fixture();
        let destination = fixture.root.join("code/portal");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("notes.txt"), "mine").unwrap();
        assert!(get(&fixture, git_only(), &destination, None).await.is_err());
        assert_eq!(
            fs::read_to_string(destination.join("notes.txt")).unwrap(),
            "mine"
        );
        assert!(get(
            &fixture,
            git_only(),
            &fixture.root.join("missing/portal"),
            None
        )
        .await
        .is_err());
        assert!(!fixture.root.join("missing").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_stops_a_running_clone_and_leaves_no_folder() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = fixture();
        let slow = fixture.root.join("slow-git");
        fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&slow, fs::Permissions::from_mode(0o755)).unwrap();
        let tools = CloneTools {
            gh: None,
            git: Some(slow),
        };
        let destination = fixture.root.join("code/portal");
        let (sender, cancellation) = watch::channel(false);
        let started = std::time::Instant::now();
        let run = get_folder(
            fixture.repository.clone(),
            tools,
            destination.to_string_lossy().into_owned(),
            None,
            Some(source("https://github.com/acme/portal.git")),
            cancellation,
            &|_| {},
        );
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            sender.send_replace(true);
        };
        let (result, _) = tokio::join!(run, cancel);
        assert_eq!(result, Err("Stopped.".to_string()));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!destination.exists(), "a cancelled run makes no folder");
    }

    #[test]
    fn a_cancel_before_the_run_begins_still_stops_it() {
        let control = CloudProjectControl::default();
        control.cancel("run-1");
        assert!(control.begin("run-1").is_err());
        let run = control.begin("run-2").unwrap();
        assert!(control.begin("run-3").is_err(), "one run at a time");
        control.cancel("run-2");
        assert!(*run.cancellation.borrow());
    }
}

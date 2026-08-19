//! Plan-time Codex provider thread ID allocation.
//!
//! All IDs are minted during planning and persisted in the plan; apply and
//! recovery never mint or invoke Codex. The preferred path asks one bounded
//! headless CLI session — disposable provider home, operating-system home,
//! and CWD, plugins disabled, `initialize`/`initialized`, `thread/start` with
//! `ephemeral: true`, no `turn/start` — and accepts only a canonical lowercase
//! RFC 9562 UUIDv7. On any failure it generates cryptographically random
//! UUIDv7 values locally under the same validation. Both paths collision-check
//! against the caller's inventory.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const APP_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_APP_SERVER_LINE_BYTES: usize = 1024 * 1024;
const MAX_APP_SERVER_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_APP_SERVER_MESSAGES: usize = 4_096;
const MAX_COLLISION_RETRIES: usize = 16;
const APP_SERVER_REMOVED_ENV_KEYS: &[&str] = &[
    "AIMOCK_DUMMY_KEY_MARKER",
    "AIMOCK_PROVIDER_ANTHROPIC_KEY",
    "AIMOCK_PROVIDER_AZURE_KEY",
    "AIMOCK_PROVIDER_COHERE_KEY",
    "AIMOCK_PROVIDER_ELEVENLABS_KEY",
    "AIMOCK_PROVIDER_FAL_KEY",
    "AIMOCK_PROVIDER_GEMINI_KEY",
    "AIMOCK_PROVIDER_GROK_KEY",
    "AIMOCK_PROVIDER_OLLAMA_KEY",
    "AIMOCK_PROVIDER_OPENAI_KEY",
    "AIMOCK_PROVIDER_OPENROUTER_KEY",
    "AIMOCK_PROVIDER_VEO_KEY",
    "ALL_PROXY",
    "ANTHROPIC_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_ENDPOINT",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CONFIG_DIR",
    "CODEX_API_KEY",
    "CODEX_HOME",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "MALLARD_CODEX_PROVIDER_API_KEY",
    "MALLARD_CODEX_PROVIDER_URL",
    "MALLARD_LIVE_CODEX_PROVIDER_KEY",
    "MALLARD_LIVE_CODEX_PROVIDER_URL",
    "MALLARD_LIVE_CODEX_SOURCE_HOME",
    "MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E",
    "MALLARD_RUN_LIVE_CODEX_E2E",
    "MALLARD_TUZI_POC_KEY",
    "NO_PROXY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORGANIZATION",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "all_proxy",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// Validate one canonical lowercase RFC 9562 UUIDv7.
pub fn validate_uuid_v7(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let dashes = [8_usize, 13, 18, 23];
    if bytes.len() != 36 {
        return Err("Codex thread id is not a canonical UUID".to_string());
    }
    for (index, byte) in bytes.iter().enumerate() {
        if dashes.contains(&index) {
            if *byte != b'-' {
                return Err("Codex thread id is not a canonical UUID".to_string());
            }
        } else if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte) {
            return Err("Codex thread id is not a lowercase hexadecimal UUID".to_string());
        }
    }
    if bytes[14] != b'7' {
        return Err("Codex thread id is not a version 7 UUID".to_string());
    }
    if !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return Err("Codex thread id has an invalid UUID variant".to_string());
    }
    Ok(())
}

/// Generate one cryptographically random UUIDv7 (random timestamp-less
/// variant: all non-version bits random, which RFC 9562 permits for
/// unordered identifiers and which cannot collide with clock-adjacent CLI
/// allocations except by chance).
pub fn generate_random_uuid_v7() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("random Codex thread id: {}", error))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    validate_uuid_v7(&uuid)?;
    Ok(uuid)
}

/// Allocate `count` fresh Codex provider thread IDs, preferring one bounded
/// headless CLI session. `is_taken` must answer against provider inventory,
/// indexes, active plans, and journals; the allocator additionally never
/// repeats an ID within one batch.
pub fn allocate_codex_thread_ids(
    count: usize,
    is_taken: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    allocate_with(count, is_taken, CliAllocator::start().ok())
}

/// The local-only fallback path, also used directly by tests: identical
/// validation and collision behavior without invoking Codex.
pub fn allocate_codex_thread_ids_local(
    count: usize,
    is_taken: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    allocate_with(count, is_taken, None)
}

fn allocate_with(
    count: usize,
    is_taken: &dyn Fn(&str) -> bool,
    mut cli: Option<CliAllocator>,
) -> Result<Vec<String>, String> {
    let mut allocated: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        let mut accepted = None;
        for _ in 0..MAX_COLLISION_RETRIES {
            let candidate = match cli.as_mut().and_then(|cli| cli.mint().ok()) {
                Some(candidate) => candidate,
                None => {
                    // Any CLI failure permanently falls back to local
                    // generation for the rest of the batch.
                    cli = None;
                    generate_random_uuid_v7()?
                }
            };
            if validate_uuid_v7(&candidate).is_err() {
                cli = None;
                continue;
            }
            if is_taken(&candidate) || allocated.iter().any(|existing| existing == &candidate) {
                continue;
            }
            accepted = Some(candidate);
            break;
        }
        allocated.push(accepted.ok_or_else(|| {
            "Codex thread id allocation exhausted its collision retries".to_string()
        })?);
    }
    Ok(allocated)
}

/// One bounded headless app-server session over a disposable home.
struct CliAllocator {
    client: AppServerClient,
    _home: tempfile::TempDir,
    _process_home: tempfile::TempDir,
    _cwd: tempfile::TempDir,
}

impl CliAllocator {
    fn start() -> Result<Self, String> {
        let executable = crate::codex_binary::find_binary("codex")?;
        let home = tempfile::tempdir()
            .map_err(|error| format!("create disposable Codex home: {}", error))?;
        let process_home = tempfile::tempdir()
            .map_err(|error| format!("create disposable process home: {}", error))?;
        let cwd = tempfile::tempdir()
            .map_err(|error| format!("create disposable Codex cwd: {}", error))?;
        let client =
            AppServerClient::start(&executable, home.path(), process_home.path(), cwd.path())?;
        Ok(Self {
            client,
            _home: home,
            _process_home: process_home,
            _cwd: cwd,
        })
    }

    fn mint(&mut self) -> Result<String, String> {
        let response = self.client.request(
            "thread/start",
            json!({ "ephemeral": true }),
            APP_SERVER_REQUEST_TIMEOUT,
        )?;
        let thread_id = response
            .pointer("/result/thread/id")
            .or_else(|| response.pointer("/result/threadId"))
            .or_else(|| response.pointer("/result/thread_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex thread/start returned no thread id".to_string())?;
        validate_uuid_v7(thread_id)?;
        Ok(thread_id.to_string())
    }
}

#[derive(Debug)]
enum AppServerMessage {
    Line(Vec<u8>),
    Oversized,
    StderrOversized,
    Io,
    Eof,
}

struct AppServerClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<AppServerMessage>,
    next_id: u64,
    received_bytes: usize,
    received_messages: usize,
}

impl AppServerClient {
    fn start(
        executable: &Path,
        codex_home: &Path,
        process_home: &Path,
        project: &Path,
    ) -> Result<Self, String> {
        let mut child = app_server_command(executable, codex_home, process_home, project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start Codex app-server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex app-server stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex app-server stdout is unavailable".to_string())?;
        let (sender, receiver) = mpsc::channel();
        if let Some(stderr) = child.stderr.take() {
            let sender = sender.clone();
            thread::spawn(move || {
                let mut reader = stderr;
                let mut buffer = [0_u8; 8192];
                let mut received = 0_usize;
                let mut oversized_reported = false;
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            received = received.saturating_add(count);
                            if received > MAX_APP_SERVER_OUTPUT_BYTES && !oversized_reported {
                                oversized_reported = true;
                                let _ = sender.send(AppServerMessage::StderrOversized);
                            }
                        }
                        Err(_) => {
                            let _ = sender.send(AppServerMessage::Io);
                            break;
                        }
                    }
                }
            });
        }
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match read_bounded_line(&mut reader, &mut line, MAX_APP_SERVER_LINE_BYTES) {
                    Ok(0) => {
                        let _ = sender.send(AppServerMessage::Eof);
                        break;
                    }
                    Ok(_) => {
                        if sender.send(AppServerMessage::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(BoundedLineError::Oversized) => {
                        let _ = sender.send(AppServerMessage::Oversized);
                        break;
                    }
                    Err(BoundedLineError::Io) => {
                        let _ = sender.send(AppServerMessage::Io);
                        break;
                    }
                }
            }
        });
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout: receiver,
            next_id: 1,
            received_bytes: 0,
            received_messages: 0,
        };
        let initialize = client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "mallard",
                    "title": "Mallard",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
            APP_SERVER_REQUEST_TIMEOUT,
        )?;
        if initialize.get("result").is_none() {
            return Err(app_server_error("initialize", &initialize));
        }
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_json(&json!({ "method": method, "id": id, "params": params }))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Codex app-server {method} timed out"));
            }
            let message = match self.stdout.recv_timeout(remaining) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("Codex app-server {method} timed out"))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("Codex app-server closed during {method}"))
                }
            };
            let line = match message {
                AppServerMessage::Line(line) => line,
                AppServerMessage::Oversized => {
                    return Err("Codex app-server emitted an oversized JSONL line".to_string())
                }
                AppServerMessage::StderrOversized => {
                    return Err("Codex app-server stderr exceeded its bounded limit".to_string())
                }
                AppServerMessage::Io => {
                    return Err("Codex app-server output could not be read".to_string())
                }
                AppServerMessage::Eof => {
                    return Err(format!("Codex app-server exited during {method}"))
                }
            };
            self.received_messages = self.received_messages.saturating_add(1);
            self.received_bytes = self.received_bytes.saturating_add(line.len());
            if self.received_messages > MAX_APP_SERVER_MESSAGES
                || self.received_bytes > MAX_APP_SERVER_OUTPUT_BYTES
            {
                return Err("Codex app-server output exceeded its bounded limit".to_string());
            }
            let value: Value = serde_json::from_slice(&line)
                .map_err(|_| "Codex app-server emitted malformed JSON".to_string())?;
            if let Some(response_id) = value.get("id") {
                if response_id.as_u64() == Some(id) {
                    return Ok(value);
                }
                return Err(format!(
                    "Codex app-server {method} returned an unexpected response id"
                ));
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json(&json!({ "method": method, "params": params }))
    }

    fn write_json(&mut self, value: &Value) -> Result<(), String> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "Codex app-server stdin is closed".to_string())?;
        serde_json::to_writer(&mut *stdin, value)
            .map_err(|error| format!("encode Codex app-server request: {error}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("write Codex app-server request: {error}"))
    }
}

fn app_server_command(
    executable: &Path,
    codex_home: &Path,
    process_home: &Path,
    project: &Path,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .arg("--stdio")
        .arg("--config")
        .arg("features.plugins=false")
        .current_dir(project);
    for key in APP_SERVER_REMOVED_ENV_KEYS {
        command.env_remove(key);
    }
    command
        .env("CODEX_HOME", codex_home)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home);
    command
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum BoundedLineError {
    Oversized,
    Io,
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> Result<usize, BoundedLineError> {
    loop {
        let available = reader.fill_buf().map_err(|_| BoundedLineError::Io)?;
        if available.is_empty() {
            return Ok(output.len());
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if output.len().saturating_add(take) > limit {
            return Err(BoundedLineError::Oversized);
        }
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if output.last() == Some(&b'\n') {
            while output
                .last()
                .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
            {
                output.pop();
            }
            return Ok(output.len());
        }
    }
}

fn app_server_error(method: &str, response: &Value) -> String {
    let code = response
        .pointer("/error/code")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!("Codex app-server {method} failed ({code})")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write one executable fake `codex app-server` whose behavior is the
    /// given shell body. The script records its environment and every
    /// request line into `capture`.
    #[cfg(unix)]
    fn fake_app_server(directory: &Path, capture: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            "#!/bin/sh\nCAPTURE={capture}\nprintf 'CODEX_HOME=%s\\nHOME=%s\\nUSERPROFILE=%s\\nCWD=%s\\nARGS=%s\\n' \"$CODEX_HOME\" \"$HOME\" \"$USERPROFILE\" \"$(pwd)\" \"$*\" >> \"$CAPTURE\"\nN=1\nwhile IFS= read -r line; do\n  printf 'REQ %s\\n' \"$line\" >> \"$CAPTURE\"\n{body}\ndone\n",
            capture = capture.display(),
            body = body,
        );
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("codex");
        std::fs::write(&path, script).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn fake_allocator_error(executable: &Path) -> String {
        fake_allocator(executable).map(|_| ()).unwrap_err()
    }

    #[cfg(unix)]
    fn fake_allocator(executable: &Path) -> Result<CliAllocator, String> {
        let home = tempfile::tempdir().unwrap();
        let process_home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let client =
            AppServerClient::start(executable, home.path(), process_home.path(), cwd.path())?;
        Ok(CliAllocator {
            client,
            _home: home,
            _process_home: process_home,
            _cwd: cwd,
        })
    }

    #[test]
    fn app_server_command_is_plugin_free_and_environment_isolated() {
        use std::collections::BTreeMap;
        use std::ffi::OsString;

        let parent = tempfile::tempdir().unwrap();
        let executable = parent.path().join("codex");
        let codex_home = parent.path().join("codex-home");
        let process_home = parent.path().join("process-home");
        let project = parent.path().join("project");
        let command = app_server_command(&executable, &codex_home, &process_home, &project);

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "app-server",
                "--stdio",
                "--config",
                "features.plugins=false"
            ]
            .map(std::ffi::OsStr::new)
        );
        assert_eq!(command.get_current_dir(), Some(project.as_path()));

        let environment = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect::<BTreeMap<OsString, Option<OsString>>>();
        assert_eq!(
            environment.get(&OsString::from("CODEX_HOME")),
            Some(&Some(codex_home.into_os_string()))
        );
        assert_eq!(
            environment.get(&OsString::from("HOME")),
            Some(&Some(process_home.clone().into_os_string()))
        );
        assert_eq!(
            environment.get(&OsString::from("USERPROFILE")),
            Some(&Some(process_home.into_os_string()))
        );
        for key in APP_SERVER_REMOVED_ENV_KEYS {
            if *key != "CODEX_HOME" {
                assert_eq!(
                    environment.get(&OsString::from(*key)),
                    Some(&None),
                    "allocator command must remove inherited {key}"
                );
            }
        }
    }

    /// The successful CLI path: initialize/initialized handshake, bounded
    /// `thread/start` requests with `ephemeral: true`, never `turn/start`,
    /// and a disposable home and working directory.
    #[cfg(unix)]
    #[test]
    fn fake_app_server_success_path_is_ephemeral_and_home_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let capture = directory.path().join("capture.log");
        let body = concat!(
            "  case \"$line\" in\n",
            "    *'\"initialize\"'*) printf '{\"id\":%s,\"result\":{}}\\n' \"$N\"; N=$((N+1));;\n",
            "    *'\"thread/start\"'*) printf '{\"id\":%s,\"result\":{\"thread\":{\"id\":\"019811aa-0000-7000-8000-00000000000%s\"}}}\\n' \"$N\" \"$N\"; N=$((N+1));;\n",
            "  esac",
        );
        let executable = fake_app_server(directory.path(), &capture, body);
        let cli = fake_allocator(&executable).unwrap();
        let disposable_home = cli._home.path().to_path_buf();
        let disposable_process_home = cli._process_home.path().to_path_buf();
        let disposable_cwd = cli._cwd.path().to_path_buf();
        let none_taken = |_: &str| false;
        let batch = allocate_with(2, &none_taken, Some(cli)).unwrap();
        assert_eq!(
            batch,
            vec![
                "019811aa-0000-7000-8000-000000000002".to_string(),
                "019811aa-0000-7000-8000-000000000003".to_string(),
            ]
        );

        let log = std::fs::read_to_string(&capture).unwrap();
        let home_line = format!("CODEX_HOME={}", disposable_home.display());
        assert!(log.contains(&home_line), "{log}");
        let process_home_line = format!("HOME={}", disposable_process_home.display());
        assert!(log.contains(&process_home_line), "{log}");
        let userprofile_line = format!("USERPROFILE={}", disposable_process_home.display());
        assert!(log.contains(&userprofile_line), "{log}");
        assert!(
            log.contains("ARGS=app-server --stdio --config features.plugins=false"),
            "{log}"
        );
        assert!(!log.contains("turn/start"), "{log}");
        assert!(log.contains("\"initialize\""), "{log}");
        assert!(log.contains("\"initialized\""), "{log}");
        assert_eq!(log.matches("\"ephemeral\":true").count(), 2, "{log}");
        // The disposable home and cwd never point at the ambient values.
        if let Ok(real_home) = std::env::var("CODEX_HOME") {
            assert_ne!(disposable_home.to_string_lossy(), real_home);
        }
        if let Ok(real_home) = std::env::var("HOME") {
            assert_ne!(disposable_process_home.to_string_lossy(), real_home);
        }
        assert_ne!(
            disposable_home, disposable_process_home,
            "provider and process homes must be separate"
        );
        assert_ne!(
            disposable_cwd,
            std::env::current_dir().unwrap(),
            "allocator must not run in the real working directory"
        );
    }

    /// Every CLI failure mode is a typed error or a clean local fallback:
    /// missing binary, initialize error, immediate exit, malformed output,
    /// an unanswered request, a non-v7 mint, and a permanently colliding
    /// mint.
    #[cfg(unix)]
    #[test]
    fn fake_app_server_failures_fall_back_or_error_typed() {
        let directory = tempfile::tempdir().unwrap();
        let capture = directory.path().join("capture.log");

        // Missing binary.
        let error = fake_allocator_error(&directory.path().join("missing"));
        assert!(error.contains("start Codex app-server"), "{error}");

        // Initialize returns an error object.
        let init_error = fake_app_server(
            &directory.path().join("init-error-dir"),
            &capture,
            "  printf '{\"id\":%s,\"error\":{\"code\":-32600}}\\n' \"$N\"; N=$((N+1))",
        );
        let error = fake_allocator_error(&init_error);
        assert!(error.contains("initialize failed"), "{error}");

        // Early exit before any response.
        let eof = fake_app_server(&directory.path().join("eof-dir"), &capture, "  exit 0");
        let error = fake_allocator_error(&eof);
        assert!(error.contains("exited during initialize"), "{error}");

        // Malformed JSON output.
        let malformed = fake_app_server(
            &directory.path().join("malformed-dir"),
            &capture,
            "  printf 'not json at all\\n'",
        );
        let error = fake_allocator_error(&malformed);
        assert!(error.contains("malformed JSON"), "{error}");

        // An unanswered request times out without hanging the caller.
        let silent = fake_app_server(
            &directory.path().join("silent-dir"),
            &capture,
            concat!(
                "  case \"$line\" in\n",
                "    *'\"initialize\"'*) printf '{\"id\":%s,\"result\":{}}\\n' \"$N\"; N=$((N+1));;\n",
                "  esac",
            ),
        );
        let mut cli = fake_allocator(&silent).unwrap();
        let error = cli
            .client
            .request(
                "thread/start",
                json!({ "ephemeral": true }),
                Duration::from_millis(250),
            )
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        drop(cli);

        // A non-v7 mint permanently falls back to local generation.
        let wrong_version = fake_app_server(
            &directory.path().join("wrong-version-dir"),
            &capture,
            concat!(
                "  case \"$line\" in\n",
                "    *'\"initialize\"'*) printf '{\"id\":%s,\"result\":{}}\\n' \"$N\"; N=$((N+1));;\n",
                "    *'\"thread/start\"'*) printf '{\"id\":%s,\"result\":{\"thread\":{\"id\":\"20af57c5-3cf7-4914-9a48-a25581cbcc25\"}}}\\n' \"$N\"; N=$((N+1));;\n",
                "  esac",
            ),
        );
        let cli = fake_allocator(&wrong_version).unwrap();
        let none_taken = |_: &str| false;
        let batch = allocate_with(1, &none_taken, Some(cli)).unwrap();
        validate_uuid_v7(&batch[0]).unwrap();
        assert_ne!(batch[0], "20af57c5-3cf7-4914-9a48-a25581cbcc25");

        // A CLI that only ever mints one occupied id exhausts the bounded
        // collision retries instead of accepting it.
        let colliding = fake_app_server(
            &directory.path().join("colliding-dir"),
            &capture,
            concat!(
                "  case \"$line\" in\n",
                "    *'\"initialize\"'*) printf '{\"id\":%s,\"result\":{}}\\n' \"$N\"; N=$((N+1));;\n",
                "    *'\"thread/start\"'*) printf '{\"id\":%s,\"result\":{\"thread\":{\"id\":\"019811aa-0000-7000-8000-0000000000ff\"}}}\\n' \"$N\"; N=$((N+1));;\n",
                "  esac",
            ),
        );
        let cli = fake_allocator(&colliding).unwrap();
        let occupied = |candidate: &str| candidate == "019811aa-0000-7000-8000-0000000000ff";
        let error = allocate_with(1, &occupied, Some(cli)).unwrap_err();
        assert!(error.contains("collision retries"), "{error}");
    }

    #[test]
    fn random_uuid_v7_values_are_canonical_and_collision_checked() {
        let first = generate_random_uuid_v7().unwrap();
        validate_uuid_v7(&first).unwrap();
        assert_eq!(first.as_bytes()[14], b'7');

        // Uppercase, wrong version, and wrong variant are rejected.
        assert!(validate_uuid_v7(&first.to_uppercase()).is_err());
        let mut wrong_version = first.clone().into_bytes();
        wrong_version[14] = b'4';
        assert!(validate_uuid_v7(std::str::from_utf8(&wrong_version).unwrap()).is_err());
        let mut wrong_variant = first.clone().into_bytes();
        wrong_variant[19] = b'0';
        assert!(validate_uuid_v7(std::str::from_utf8(&wrong_variant).unwrap()).is_err());

        // Collision predicate is honored: reject everything once, then the
        // allocator errors out after bounded retries.
        let all_taken = |_: &str| true;
        assert!(allocate_codex_thread_ids_local(1, &all_taken).is_err());
        let none_taken = |_: &str| false;
        let batch = allocate_codex_thread_ids_local(3, &none_taken).unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(
            batch
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        for id in &batch {
            validate_uuid_v7(id).unwrap();
        }
    }
}

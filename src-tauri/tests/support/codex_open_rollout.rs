//! A separate process retains one rollout handle across writes and sync.
//! All input is native AIMock output under the disposable test root.

use super::*;
use std::io::{BufRead, Write};
use std::thread::JoinHandle;

const GATE: &str = "MALLARD_OPEN_ROLLOUT_FIXTURE";
const REPLY: &str = "OPEN_ROLLOUT_REPLY ";
const TIMEOUT: Duration = Duration::from_secs(30);
const LINE_LIMIT: u64 = 1024 * 1024;

struct FixtureProcess {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Value>,
    reader: Option<JoinHandle<()>>,
    sequence: u64,
}

impl FixtureProcess {
    fn start(root: &Path) -> Result<Self, &'static str> {
        let mut process = Self::launch(
            &std::env::current_exe().map_err(|_| "fixture executable")?,
            root,
        )?;
        process.request_with_timeout(json!({"operation": "ready"}), TIMEOUT)?;
        Ok(process)
    }

    fn launch(executable: &Path, root: &Path) -> Result<Self, &'static str> {
        let mut child = Command::new(executable)
            .args([
                "open_rollout::fixture_worker",
                "--ignored",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(GATE, root)
            .env("HOME", root)
            .env("USERPROFILE", root)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "fixture spawn failed")?;
        let input = child.stdin.take().expect("piped fixture input");
        let output = child.stdout.take().expect("piped fixture output");
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                let Ok(count) = output
                    .by_ref()
                    .take(LINE_LIMIT + 1)
                    .read_until(b'\n', &mut line)
                else {
                    break;
                };
                if count == 0 || count as u64 > LINE_LIMIT {
                    break;
                }
                let Ok(line) = std::str::from_utf8(&line) else {
                    break;
                };
                if let Some(json) = line.strip_prefix(REPLY) {
                    let Ok(value) = serde_json::from_str(json) else {
                        break;
                    };
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            output: receiver,
            reader: Some(reader),
            sequence: 0,
        })
    }

    fn request_with_timeout(
        &mut self,
        mut request: Value,
        timeout: Duration,
    ) -> Result<Value, &'static str> {
        if self
            .child
            .try_wait()
            .map_err(|_| "fixture status")?
            .is_some()
        {
            return Err("fixture exited early");
        }
        self.sequence += 1;
        request["sequence"] = json!(self.sequence);
        writeln!(self.input, "{request}").map_err(|_| "fixture input closed")?;
        self.input.flush().map_err(|_| "fixture input flush")?;
        let reply = self
            .output
            .recv_timeout(timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "fixture response timeout",
                mpsc::RecvTimeoutError::Disconnected => "fixture exited early",
            })?;
        if reply["sequence"] != self.sequence || reply["ok"] != true {
            return Err("fixture rejected request");
        }
        Ok(reply["value"].clone())
    }

    fn request(&mut self, request: Value) -> Value {
        self.request_with_timeout(request, TIMEOUT)
            .expect("fixture request failed")
    }

    fn held(&mut self, size: usize) {
        let status = self.request(json!({"operation": "status"}));
        assert_eq!(status["size"], size as u64, "retained handle size");
        assert_eq!(status["pid"], self.child.id(), "writer process identity");
        assert_ne!(self.child.id(), std::process::id());
    }

    fn stop(&mut self) {
        self.request(json!({"operation": "close"}));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("fixture exit status") {
                assert!(status.success(), "fixture shutdown failed");
                break;
            }
            assert!(Instant::now() < deadline, "fixture shutdown timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        // Also runs after an assertion or response timeout.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn fixture_path(root: &Path, value: &Value) -> PathBuf {
    let relative = Path::new(value.as_str().expect("fixture path string"));
    assert!(!relative.as_os_str().is_empty());
    let mut path = root.to_path_buf();
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            panic!("fixture path component")
        };
        path.push(part);
        let metadata = std::fs::symlink_metadata(&path).expect("fixture path metadata");
        assert!(!metadata.file_type().is_symlink(), "fixture symlink");
        assert!(
            metadata.is_file() || metadata.is_dir(),
            "fixture special file"
        );
    }
    assert!(path.canonicalize().expect("fixture path").starts_with(root));
    path
}

#[test]
#[ignore = "private child process for the Codex open-rollout test"]
fn fixture_worker() {
    let Some(root) = std::env::var_os(GATE) else {
        return;
    };
    let root = PathBuf::from(root).canonicalize().expect("fixture root");
    assert!(
        root.join("open-rollout-owner").is_file(),
        "fixture owner missing"
    );
    let mut writer: Option<std::fs::File> = None;
    let mut native = Vec::new();
    let mut offset = 0;
    let mut input = std::io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let count = input
            .by_ref()
            .take(LINE_LIMIT + 1)
            .read_until(b'\n', &mut line)
            .expect("fixture input");
        if count == 0 {
            break;
        }
        assert!(count as u64 <= LINE_LIMIT, "fixture request limit");
        let request: Value = serde_json::from_slice(&line).expect("fixture request JSON");
        let operation = request["operation"].as_str().expect("fixture operation");
        let value = match operation {
            "ready" => json!({"pid": std::process::id()}),
            "open" => {
                assert!(writer.is_none(), "writer already open");
                let path = fixture_path(&root, &request["path"]);
                native = read_bounded_file(
                    &fixture_path(&root, &request["native"]),
                    MAX_CONVERSATION_FILE_BYTES,
                    "native replay bytes",
                );
                let file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .expect("open retained writer");
                offset = file.metadata().expect("writer metadata").len() as usize;
                assert!(offset <= native.len());
                writer = Some(file);
                json!({"pid": std::process::id(), "size": offset})
            }
            "append" => {
                let end = request["end"].as_u64().expect("append end") as usize;
                assert!(end > offset && end <= native.len(), "append bounds");
                let file = writer.as_mut().expect("writer open");
                file.write_all(&native[offset..end])
                    .expect("append native bytes");
                file.sync_all().expect("flush retained writer");
                offset = end;
                json!({"size": offset})
            }
            "status" => json!({
                "pid": std::process::id(),
                "size": writer.as_ref().expect("writer open").metadata().expect("retained handle metadata").len(),
            }),
            "compare" => {
                let home = fixture_path(&root, &request["machine"]);
                let repository =
                    LocalStateRepository::from_home_dir(&home).expect("fixture repository");
                let project =
                    LocalProjectId::parse(request["project"].as_str().expect("project id"))
                        .expect("valid project");
                let storage = StorageId::parse(request["storage"].as_str().expect("storage id"))
                    .expect("valid storage");
                serde_json::to_value(
                    ProjectSyncService::new(&repository)
                        .compare_threads(&project, &storage)
                        .expect("fixture comparison"),
                )
                .expect("comparison JSON")
            }
            "close" => {
                drop(writer.take());
                Value::Null
            }
            // Failure-control commands are private to this disposable worker.
            "exit" => return,
            "no_reply" => continue,
            _ => panic!("unknown fixture operation"),
        };
        println!(
            "\n{REPLY}{}",
            json!({"sequence": request["sequence"], "ok": true, "value": value})
        );
        std::io::stdout().flush().expect("fixture reply flush");
        if operation == "close" {
            break;
        }
    }
}

fn compare(process: &mut FixtureProcess, project: &LocalProjectId, storage: &StorageId) -> Value {
    process.request(json!({
        "operation": "compare", "machine": "machine-a", "project": project, "storage": storage,
    }))
}

fn assert_states(comparison: &Value, active: &str, expected: ThreadSyncState) {
    let entries = comparison["entries"]
        .as_array()
        .expect("comparison entries");
    assert_eq!(entries.len(), 2, "active and quiet sessions");
    for row in entries {
        assert_eq!(
            row["state"],
            if row["provider_thread_id"].as_str() == Some(active) {
                json!(expected)
            } else {
                json!(ThreadSyncState::Synced)
            }
        );
    }
    assert!(entries
        .iter()
        .any(|row| row["provider_thread_id"].as_str() == Some(active)));
}

fn cut_before(bytes: &[u8], marker: &str) -> usize {
    let mut offset = 0;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line
            .windows(marker.len())
            .any(|part| part == marker.as_bytes())
        {
            return offset;
        }
        offset += line.len();
    }
    panic!("native turn boundary missing");
}

pub fn run(parent: &Path, source_home: &Path, project: &Path, id: &str) {
    eprintln!("live Codex phase: external open rollout writer, two appends, Push/Pull and process restart");
    let fixture = tempfile::tempdir_in(parent).expect("open rollout fixture");
    let root = fixture.path().canonicalize().expect("canonical fixture");
    std::fs::write(root.join("open-rollout-owner"), b"1\n").unwrap();
    for name in [
        "machine-a",
        "machine-b",
        "codex-a",
        "codex-b",
        "project-b",
        "store",
    ] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let bytes = read_bounded_file(
        &rollout_path(source_home, id),
        MAX_CONVERSATION_FILE_BYTES,
        "native three-turn rollout",
    );
    let first = cut_before(&bytes, "LIVE_USER_R2");
    let second = cut_before(&bytes, "LIVE_USER_R5");
    assert!(first > 0 && first < second && second < bytes.len());
    let expected = [
        marker_turn("LIVE_USER_R1", "MALLARD_BASE"),
        marker_turn("LIVE_USER_R2", "MALLARD_REMOTE_BRANCH"),
        marker_turn("LIVE_USER_R5", "MALLARD_SECOND_LOCAL_BRANCH"),
    ]
    .concat();
    // Check each cut against the existing exact native timeline oracle.
    for (end, turns) in [(first, 1), (second, 2), (bytes.len(), 3)] {
        let actual = timeline_history_from_standalone(&bytes[..end], project, id)
            .unwrap_or_else(|_| panic!("native replay projection"));
        assert!(
            actual == expected[..turns * 2],
            "native replay boundary history"
        );
    }
    std::fs::write(root.join("native.jsonl"), &bytes).unwrap();
    let home_a = root.join("codex-a");
    let home_b = root.join("codex-b");
    let sessions = home_a.join("sessions/2026/09/29");
    std::fs::create_dir_all(&sessions).unwrap();
    let path = sessions.join(format!("rollout-2026-09-29T01-00-00-{id}.jsonl"));
    std::fs::write(&path, &bytes[..first]).unwrap();
    let quiet_id = "019fb029-0000-7000-8000-000000000099";
    let quiet = sessions.join(format!("rollout-2026-09-29T01-00-00-{quiet_id}.jsonl"));
    let quiet_bytes = String::from_utf8(bytes[..first].to_vec())
        .unwrap()
        .replace(id, quiet_id);
    std::fs::write(&quiet, quiet_bytes.as_bytes()).unwrap();
    let repo_a = LocalStateRepository::from_home_dir(&root.join("machine-a")).unwrap();
    let repo_b = LocalStateRepository::from_home_dir(&root.join("machine-b")).unwrap();
    let a = ProjectSyncService::new(&repo_a);
    let b = ProjectSyncService::new(&repo_b);
    let storage = local_storage(&root.join("store"));
    let storage_id = storage.id.clone();
    let setup_a = finish_setup(
        &repo_a,
        &a,
        project,
        &home_a,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let project_a = &setup_a.project.local_project_id;
    assert!(push(&a, project_a, &storage_id).success);
    let setup_b = finish_setup(
        &repo_b,
        &b,
        &root.join("project-b"),
        &home_b,
        storage,
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: setup_a.project.bundle_id.clone(),
            display_name: setup_a.project.display_name.clone(),
            repository_fingerprint: setup_a.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    let binding_b = setup_b.binding.as_ref().unwrap();
    pull_and_apply(&b, &storage_id, &setup_a.project.bundle_id, binding_b);
    let local = logical_thread_id(&repo_a, project_a, id);
    let cloud = cloud_thread_id_of(&repo_a, project_a, &local);
    let resource_a = mallard_domain::conversation_resource_id(&codex_key(), &local).unwrap();
    let mut writer = FixtureProcess::start(&root).unwrap();
    writer.request(json!({"operation": "open", "path": path.strip_prefix(&root).unwrap(), "native": "native.jsonl"}));
    let mut reader = FixtureProcess::start(&root).unwrap();
    writer.held(first);
    for _ in 0..2 {
        assert_states(
            &compare(&mut reader, project_a, &storage_id),
            id,
            ThreadSyncState::Synced,
        );
    }
    writer.held(first);
    reverted_rollouts::assert_detail_history(&a, project_a, &resource_a, &expected[..2]);

    for (end, turns) in [(second, 2), (bytes.len(), 3)] {
        // The same descriptor is used for both appends. No cache invalidation
        // or event injection occurs. The very first comparison must see it.
        writer.request(json!({"operation": "append", "end": end}));
        writer.held(end);
        assert_states(
            &compare(&mut reader, project_a, &storage_id),
            id,
            ThreadSyncState::LocalAhead,
        );
        writer.held(end);
        reverted_rollouts::assert_detail_history(
            &a,
            project_a,
            &resource_a,
            &expected[..turns * 2],
        );
        assert_resolved_local_alias_history(
            &home_a,
            project,
            id,
            None,
            &expected[..turns * 2],
            0,
            0,
        );
        // A new OS process must agree while the writer remains open.
        reader.stop();
        reader = FixtureProcess::start(&root).unwrap();
        assert_states(
            &compare(&mut reader, project_a, &storage_id),
            id,
            ThreadSyncState::LocalAhead,
        );
        writer.held(end);
        assert!(push(&a, project_a, &storage_id).success);
        writer.held(end);
        pull_and_apply(&b, &storage_id, &setup_a.project.bundle_id, binding_b);
        writer.held(end);
        let local_b = local_thread_of_cloud(&repo_b, &setup_b.project.local_project_id, &cloud);
        let id_b = provider_thread_id(&repo_b, &setup_b.project.local_project_id, &local_b);
        let resource_b = mallard_domain::conversation_resource_id(&codex_key(), &local_b).unwrap();
        reverted_rollouts::assert_detail_history(
            &b,
            &setup_b.project.local_project_id,
            &resource_b,
            &expected[..turns * 2],
        );
        assert_resolved_local_alias_history(
            &home_b,
            &root.join("project-b"),
            &id_b,
            None,
            &expected[..turns * 2],
            1,
            0,
        );
        assert_states(
            &compare(&mut reader, project_a, &storage_id),
            id,
            ThreadSyncState::Synced,
        );
        assert_states(
            &serde_json::to_value(
                b.compare_threads(&setup_b.project.local_project_id, &storage_id)
                    .unwrap(),
            )
            .unwrap(),
            &id_b,
            ThreadSyncState::Synced,
        );
        writer.held(end);
    }
    assert!(
        std::fs::read(&path).unwrap() == bytes,
        "replay changed native bytes"
    );
    assert!(
        std::fs::read(&quiet).unwrap() == quiet_bytes.as_bytes(),
        "quiet sibling changed"
    );
    writer.stop();
    assert_states(
        &compare(&mut reader, project_a, &storage_id),
        id,
        ThreadSyncState::Synced,
    );
    assert_resolved_local_alias_history(&home_a, project, id, None, &expected, 0, 0);
    reader.stop();
}

fn worker_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("open-rollout-owner"), b"1\n").unwrap();
    root
}

#[test]
fn writer_start_failure_is_reported() {
    let root = worker_root();
    assert!(FixtureProcess::launch(&root.path().join("missing-executable"), root.path()).is_err());
}

#[test]
fn writer_early_exit_is_reported() {
    let root = worker_root();
    let mut child = FixtureProcess::start(root.path()).unwrap();
    assert_eq!(
        child.request_with_timeout(json!({"operation": "exit"}), TIMEOUT),
        Err("fixture exited early")
    );
}

#[test]
fn writer_timeout_is_reported_and_drop_reaps_child() {
    let root = worker_root();
    let mut child = FixtureProcess::start(root.path()).unwrap();
    assert_eq!(
        child.request_with_timeout(json!({"operation": "no_reply"}), Duration::from_millis(100)),
        Err("fixture response timeout")
    );
    let pid = child.child.id();
    drop(child);
    #[cfg(unix)]
    assert_eq!(
        unsafe { libc::kill(pid as i32, 0) },
        -1,
        "fixture child leaked"
    );
    #[cfg(not(unix))]
    let _ = pid;
}

#[test]
fn writer_retains_handle_for_multiple_appends_and_closes() {
    let root = worker_root();
    std::fs::write(root.path().join("native"), b"one\ntwo\nthree\n").unwrap();
    std::fs::write(root.path().join("rollout"), b"one\n").unwrap();
    let mut child = FixtureProcess::start(root.path()).unwrap();
    child.request(json!({"operation": "open", "path": "rollout", "native": "native"}));
    child.held(4);
    for end in [8, 14] {
        child.request(json!({"operation": "append", "end": end}));
        child.held(end);
    }
    child.stop();
    assert!(child.child.try_wait().unwrap().unwrap().success());
    assert_eq!(
        std::fs::read(root.path().join("rollout")).unwrap(),
        b"one\ntwo\nthree\n"
    );
}

//! One short-lived child process with a deadline and a capped stdout.
//!
//! The child runs in a process group of its own, so a timeout kills
//! everything it started — a launcher script cannot leave the program it
//! exec'd running — and the child is always reaped before this returns.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Time past the deadline for the reader to see end-of-file after the child
/// exited on time.
const OUTPUT_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundedRunError {
    Spawn,
    TimedOut,
    OutputLimit,
    Output,
}

#[derive(Debug)]
pub(crate) struct BoundedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
}

/// Run `command` with stdin and stderr closed, reading at most
/// `output_limit` bytes of stdout. Past `timeout` the process group is
/// killed and reaped and the run fails.
pub(crate) fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    output_limit: usize,
) -> Result<BoundedOutput, BoundedRunError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    // A GUI app spawning a console program would otherwise flash a window.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(command, 0x08000000); // CREATE_NO_WINDOW
    let deadline = Instant::now() + timeout;
    let mut child = command.spawn().map_err(|_| BoundedRunError::Spawn)?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(BoundedRunError::Output);
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(output_limit as u64 + 1)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = sender.send(result);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) | Err(_) => {
                terminate(&mut child);
                return Err(BoundedRunError::TimedOut);
            }
        }
    };
    let output = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()) + OUTPUT_GRACE)
        .map_err(|_| BoundedRunError::Output)?
        .map_err(|_| BoundedRunError::Output)?;
    if output.len() > output_limit {
        return Err(BoundedRunError::OutputLimit);
    }
    Ok(BoundedOutput {
        status,
        stdout: output,
    })
}

/// Kill the child's whole process group, then reap the child.
fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        // SAFETY: killpg only sends a signal. The child is not reaped yet, so
        // its process group still exists and its id cannot have been reused.
        unsafe {
            libc::killpg(group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn script(directory: &Path, body: &str) -> PathBuf {
        let path = directory.join("child");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn captures_stdout_and_status() {
        let temp = tempfile::tempdir().unwrap();
        let child = script(temp.path(), "echo hello; exit 3");
        let output = run_bounded(&mut Command::new(child), Duration::from_secs(5), 64).unwrap();
        assert_eq!(output.stdout, b"hello\n");
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn output_past_the_limit_fails() {
        let temp = tempfile::tempdir().unwrap();
        let child = script(temp.path(), "head -c 100 /dev/zero");
        assert_eq!(
            run_bounded(&mut Command::new(child), Duration::from_secs(5), 64).unwrap_err(),
            BoundedRunError::OutputLimit
        );
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            run_bounded(
                &mut Command::new(temp.path().join("missing")),
                Duration::from_secs(5),
                64
            )
            .unwrap_err(),
            BoundedRunError::Spawn
        );
    }

    #[test]
    fn a_timeout_kills_what_the_child_started() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("grandchild.pid");
        // The timeout leaves the shell ample time to record the grandchild
        // even on a loaded machine; the rename keeps a half-written pid out.
        let child = script(
            temp.path(),
            &format!(
                "sleep 30 &\necho $! > '{pid}.part'\nmv '{pid}.part' '{pid}'\nsleep 30",
                pid = pid_file.display()
            ),
        );
        let started = Instant::now();
        assert_eq!(
            run_bounded(&mut Command::new(child), Duration::from_millis(1500), 64).unwrap_err(),
            BoundedRunError::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        let grandchild: libc::pid_t = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The orphaned grandchild is reaped by init once killed.
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(grandchild) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(grandchild), "the grandchild outlived the timeout");
    }
}

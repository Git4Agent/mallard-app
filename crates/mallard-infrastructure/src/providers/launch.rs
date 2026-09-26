//! Shared terminal-launch helpers for provider session actions.
//!
//! A launch is structured (working directory, environment, program,
//! arguments) and rendered per platform: a POSIX shell line for macOS
//! Terminal, a `cmd.exe` line for a new Windows console. The rendered line
//! is also what the activity log records, so the command it teaches is the
//! one that runs.

use std::path::{Path, PathBuf};

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn is_bare_word(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn shell_word(value: &str) -> String {
    if is_bare_word(value) {
        value.to_string()
    } else {
        shell_quote(value)
    }
}

/// A command to run interactively in a new terminal window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCommand {
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub program: String,
    pub args: Vec<String>,
}

impl TerminalCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            cwd: None,
            env: Vec::new(),
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn cwd(mut self, cwd: &Path) -> Self {
        self.cwd = Some(cwd.to_path_buf());
        self
    }

    pub fn env(mut self, name: &str, value: &Path) -> Self {
        self.env
            .push((name.to_string(), value.to_string_lossy().into_owned()));
        self
    }

    pub fn arg(mut self, value: impl Into<String>) -> Self {
        self.args.push(value.into());
        self
    }

    /// The POSIX shell line: `cd 'dir' && NAME='value' 'program' args…`.
    pub fn shell_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(cwd) = &self.cwd {
            parts.push(format!("cd {} &&", shell_quote(&cwd.to_string_lossy())));
        }
        for (name, value) in &self.env {
            parts.push(format!("{name}={}", shell_quote(value)));
        }
        parts.push(shell_word(&self.program));
        parts.extend(self.args.iter().map(|arg| shell_word(arg)));
        parts.join(" ")
    }

    /// The `cmd.exe` line: `cd /d "dir" && set "NAME=value" && "program" args…`.
    /// Values that `cmd` would reinterpret even inside quotes fail closed.
    pub fn cmd_line(&self) -> Result<String, String> {
        fn cmd_token(value: &str) -> Result<String, String> {
            if value.is_empty()
                || value
                    .bytes()
                    .any(|byte| matches!(byte, b'"' | b'%' | b'!' | b'^') || byte < 0x20)
            {
                return Err(format!(
                    "'{}' contains characters that cannot be passed through cmd.exe safely",
                    value.escape_debug()
                ));
            }
            Ok(if is_bare_word(value) {
                value.to_string()
            } else {
                format!("\"{value}\"")
            })
        }
        let mut parts = Vec::new();
        if let Some(cwd) = &self.cwd {
            parts.push(format!("cd /d {} &&", cmd_token(&cwd.to_string_lossy())?));
        }
        for (name, value) in &self.env {
            cmd_token(value)?;
            parts.push(format!("set \"{name}={value}\" &&"));
        }
        parts.push(cmd_token(&self.program)?);
        for arg in &self.args {
            parts.push(cmd_token(arg)?);
        }
        Ok(parts.join(" "))
    }

    /// The line this platform will run, for logs and manual guidance.
    pub fn display_line(&self) -> String {
        if cfg!(windows) {
            self.cmd_line().unwrap_or_else(|_| self.shell_line())
        } else {
            self.shell_line()
        }
    }
}

/// Run one command interactively in a fresh macOS Terminal window.
#[cfg(target_os = "macos")]
pub fn launch_terminal(command: &TerminalCommand) -> Result<(), String> {
    let line = command.shell_line();
    let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("tell application \"Terminal\" to do script \"{}\"", escaped);
    let output = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &script,
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .output()
        .map_err(|error| format!("open Terminal: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Terminal could not run the session command: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Run one command interactively in a new Windows console. `cmd /K` keeps
/// the window open after the program exits so errors stay readable, and
/// Windows 11 routes new consoles to the user's default terminal application.
#[cfg(windows)]
pub fn launch_terminal(command: &TerminalCommand) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let line = command.cmd_line()?;
    // The outer quotes survive cmd's own quote stripping and keep the inner
    // quoted tokens intact.
    std::process::Command::new("cmd.exe")
        .raw_arg(format!("/K \"{line}\""))
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map_err(|error| format!("open a console window: {error}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn launch_terminal(_command: &TerminalCommand) -> Result<(), String> {
    Err("Open in Terminal is supported only on macOS and Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_line_quotes_only_what_needs_it() {
        let command = TerminalCommand::new("/usr/local/bin/codex")
            .env("CODEX_HOME", Path::new("/Users/dev/my codex's home"))
            .arg("resume")
            .arg("0193b6c1-5352-7b12-9c6e-000000000001")
            .arg("-C")
            .arg("/Users/dev/my project");
        assert_eq!(
            command.shell_line(),
            "CODEX_HOME='/Users/dev/my codex'\\''s home' '/usr/local/bin/codex' resume \
             0193b6c1-5352-7b12-9c6e-000000000001 -C '/Users/dev/my project'"
        );
        let with_cwd = TerminalCommand::new("claude")
            .cwd(Path::new("/Users/dev/proj"))
            .env("CLAUDE_CONFIG_DIR", Path::new("/Users/dev/.claude"))
            .arg("--resume")
            .arg("abc");
        assert_eq!(
            with_cwd.shell_line(),
            "cd '/Users/dev/proj' && CLAUDE_CONFIG_DIR='/Users/dev/.claude' claude --resume abc"
        );
    }

    #[test]
    fn cmd_line_quotes_paths_and_rejects_cmd_metacharacters() {
        let command = TerminalCommand::new(r"C:\Users\dev\AppData\Roaming\npm\codex.cmd")
            .cwd(Path::new(r"C:\Users\dev\my project"))
            .env("CODEX_HOME", Path::new(r"C:\Users\dev\.codex"))
            .arg("resume")
            .arg("0193b6c1-5352-7b12-9c6e-000000000001");
        assert_eq!(
            command.cmd_line().unwrap(),
            r#"cd /d "C:\Users\dev\my project" && set "CODEX_HOME=C:\Users\dev\.codex" && "C:\Users\dev\AppData\Roaming\npm\codex.cmd" resume 0193b6c1-5352-7b12-9c6e-000000000001"#
        );
        for bad in [r#"C:\a"b"#, "C:\\%TEMP%", "C:\\a!b", "C:\\a^b", "C:\\a\nb"] {
            assert!(TerminalCommand::new("x").arg(bad).cmd_line().is_err(), "{bad}");
        }
        // Ampersands are literal inside quotes; they must not fail.
        assert_eq!(
            TerminalCommand::new("x").arg("C:\\a&b").cmd_line().unwrap(),
            r#"x "C:\a&b""#
        );
    }
}

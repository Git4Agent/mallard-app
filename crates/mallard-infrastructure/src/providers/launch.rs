//! Shared terminal-launch helpers for provider session actions.
//!
//! A launch is structured (working directory, environment, program,
//! arguments) and rendered per platform: a POSIX shell line for macOS
//! Terminal, a PowerShell line for Windows Terminal. The rendered line
//! is also what the activity log records, so the command it teaches is the
//! one that runs.

use std::path::{Path, PathBuf};

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Percent-encode one URI component, as `encodeURIComponent` does except that
/// only unreserved characters stay bare: deep-link query values carry paths.
pub fn percent_encode_uri_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
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

fn powershell_word(value: &str) -> String {
    if is_bare_word(value) {
        return value.to_string();
    }
    powershell_quote(value)
}

pub(super) fn powershell_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for character in value.chars() {
        // PowerShell treats typographic single quotes as delimiters too.
        if matches!(
            character,
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}'
        ) {
            quoted.push(character);
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
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

    /// The PowerShell line, with literal paths and no string interpolation.
    pub fn powershell_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(cwd) = &self.cwd {
            parts.push(format!(
                "Set-Location -LiteralPath {} -ErrorAction Stop;",
                powershell_word(&cwd.to_string_lossy())
            ));
        }
        for (name, value) in &self.env {
            parts.push(format!("$env:{name} = {};", powershell_word(value)));
        }
        parts.push(format!("& {}", powershell_word(&self.program)));
        parts.extend(self.args.iter().map(|arg| powershell_word(arg)));
        parts.join(" ")
    }

    /// The line this platform will run, for logs and manual guidance.
    pub fn display_line(&self) -> String {
        if cfg!(windows) {
            self.powershell_line()
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

#[cfg(windows)]
fn powershell_command(command: &TerminalCommand) -> Result<std::process::Command, String> {
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| "Windows SystemRoot is unavailable; cannot open PowerShell".to_string())?;
    let executable = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    // EncodedCommand takes UTF-16LE, avoiding another layer of command-line
    // quoting between CreateProcess and the PowerShell parser.
    let bytes: Vec<u8> = command
        .powershell_line()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut process = std::process::Command::new(executable);
    process.args([
        "-NoLogo",
        "-NoProfile",
        "-EncodedCommand",
        &aws_smithy_types::base64::encode(bytes),
    ]);
    Ok(process)
}

/// Run one command in a new Windows Terminal window, with PowerShell keeping
/// the tab open after the program exits so errors stay readable. Terminal
/// supplies the interactive handles instead of inheriting Mallard's stdio.
#[cfg(windows)]
pub fn launch_terminal(command: &TerminalCommand) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let terminal = dirs::data_local_dir()
        .ok_or_else(|| "cannot locate the Windows Terminal app execution alias".to_string())?
        .join("Microsoft")
        .join("WindowsApps")
        .join("wt.exe");
    let powershell = powershell_command(command)?;
    std::process::Command::new(terminal)
        // Use the user's normal environment, not a parent shell's module
        // paths (PowerShell 7 modules cannot load in Windows PowerShell).
        // The command itself pins the provider home and project directory.
        .args(["-w", "new", "new-tab", "--reloadEnvironment"])
        .arg(powershell.get_program())
        .arg("-NoExit")
        .args(powershell.get_args())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| {
            format!(
                "open Windows Terminal: {error}. Install Windows Terminal and enable its wt.exe app execution alias."
            )
        })?;
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
    fn powershell_line_quotes_literal_paths_and_arguments() {
        let command = TerminalCommand::new(r"C:\Program Files\Codex\codex.exe")
            .cwd(Path::new(r"C:\Users\dev\my project's [files]"))
            .env("CODEX_HOME", Path::new(r"C:\Users\dev\codex's home"))
            .arg("resume")
            .arg("0193b6c1-5352-7b12-9c6e-000000000001");
        assert_eq!(
            command.powershell_line(),
            r"Set-Location -LiteralPath 'C:\Users\dev\my project''s [files]' -ErrorAction Stop; $env:CODEX_HOME = 'C:\Users\dev\codex''s home'; & 'C:\Program Files\Codex\codex.exe' resume 0193b6c1-5352-7b12-9c6e-000000000001"
        );
        assert_eq!(
            TerminalCommand::new("codex")
                .arg(r"C:\测试 ‘one’ ‚two‛ $HOME $(whoami); & %TEMP% ! ^ `")
                .powershell_line(),
            "& codex 'C:\\测试 ‘‘one’’ ‚‚two‛‛ $HOME $(whoami); & %TEMP% ! ^ `'"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_powershell_preserves_context_and_literals() {
        // Run this test executable as the native child, using test filters
        // as inert argv. No installed provider or real provider home is used.
        const OUTPUT: &str = "MALLARD_TERMINAL_TEST_OUTPUT";
        if let Some(path) = std::env::var_os(OUTPUT) {
            let result = serde_json::json!({
                "cwd": std::env::current_dir().unwrap(),
                "codex_home": std::env::var("CODEX_HOME").unwrap(),
                "claude_home": std::env::var("CLAUDE_CONFIG_DIR").unwrap(),
                "args": std::env::args().collect::<Vec<_>>(),
            });
            std::fs::write(path, serde_json::to_vec(&result).unwrap()).unwrap();
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project's [files] 测试");
        std::fs::create_dir(&cwd).unwrap();
        let home = temp.path().join("agent's ‘home’ ‚test‛ $HOME %TEMP% ! ^ &");
        let output_path = temp.path().join("result.json");
        let argument = "literal ‘one’ ‚two‛ ' $HOME $(throw 'expanded'); & %TEMP% ! ^ ` 测试";
        let executable = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let command = TerminalCommand::new(executable.to_string_lossy())
            .cwd(&cwd)
            .env("CODEX_HOME", &home)
            .env("CLAUDE_CONFIG_DIR", &home)
            .arg("--exact")
            .arg("providers::launch::tests::windows_powershell_preserves_context_and_literals")
            .arg("--nocapture")
            .arg(argument);
        let mut process = powershell_command(&command).unwrap();
        process.env(OUTPUT, &output_path);
        let output = process.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output_path).unwrap()).unwrap();
        assert_eq!(Path::new(result["cwd"].as_str().unwrap()), cwd);
        assert_eq!(result["codex_home"], home.to_string_lossy().as_ref());
        assert_eq!(result["claude_home"], home.to_string_lossy().as_ref());
        assert_eq!(result["args"].as_array().unwrap().last().unwrap(), argument);

        std::fs::remove_file(&output_path).unwrap();
        let command = command.cwd(&temp.path().join("missing"));
        let output = powershell_command(&command)
            .unwrap()
            .env(OUTPUT, &output_path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!output_path.exists(), "a failed cd still ran the CLI");
    }
}

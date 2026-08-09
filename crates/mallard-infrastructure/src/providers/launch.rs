//! Shared shell-launch helpers for provider session actions.

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Run one already-quoted shell command in a fresh macOS Terminal window.
#[cfg(target_os = "macos")]
pub fn launch_terminal_command(command: &str) -> Result<(), String> {
    let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
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

#[cfg(not(target_os = "macos"))]
pub fn launch_terminal_command(_command: &str) -> Result<(), String> {
    Err("Open in Terminal is currently supported only on macOS".to_string())
}

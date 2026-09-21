//! Windows desktop-app launches through registered URL scheme handlers.
//!
//! macOS pins a profile home into a desktop app with `open --env`. Windows
//! has no equivalent, so Mallard resolves the executable the app registered
//! for its URL scheme (`codex://`, `claude://`) and starts that executable
//! directly with the environment set and the deep link as its argument.

use std::path::{Path, PathBuf};

/// Parse `reg query HKCR\<scheme>\shell\open\command /ve` output down to the
/// handler executable. The default value is normally `"C:\…\App.exe" "%1"`.
pub fn parse_scheme_handler_command(output: &str) -> Result<PathBuf, String> {
    let command = output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("(Default)")?.trim_start();
            let rest = rest.strip_prefix("REG_SZ")?.trim_start();
            Some(rest.trim_end())
        })
        .next()
        .ok_or_else(|| "URL scheme has no registered handler command".to_string())?;
    let executable = if let Some(quoted) = command.strip_prefix('"') {
        quoted
            .split_once('"')
            .map(|(path, _)| path)
            .ok_or_else(|| "URL scheme handler command is malformed".to_string())?
    } else {
        command.split_whitespace().next().unwrap_or_default()
    };
    let path = PathBuf::from(executable);
    let drive_absolute = crate::session_path_map::root_family(executable)
        == Some(crate::session_path_map::RootFamily::Windows);
    if !drive_absolute
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
    {
        return Err(format!(
            "URL scheme handler '{}' is not an absolute .exe path",
            path.display()
        ));
    }
    Ok(path)
}

/// The executable registered for a URL scheme, verified to exist.
#[cfg(windows)]
pub fn scheme_handler_executable(scheme: &str) -> Result<PathBuf, String> {
    if scheme.is_empty() || !scheme.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Err("URL scheme must be lower-case ASCII letters".to_string());
    }
    let key = format!("HKCR\\{scheme}\\shell\\open\\command");
    let output = std::process::Command::new("reg.exe")
        .args(["query", &key, "/ve"])
        .output()
        .map_err(|error| format!("query the {scheme}:// handler: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "no application is registered for {scheme}:// links; install the desktop app"
        ));
    }
    let path = parse_scheme_handler_command(&String::from_utf8_lossy(&output.stdout))?;
    if !path.is_file() {
        return Err(format!(
            "the {scheme}:// handler '{}' is not installed",
            path.display()
        ));
    }
    Ok(path)
}

/// Start a desktop app executable with pinned environment and arguments.
/// The process is detached; the app owns its own lifetime.
#[cfg(windows)]
pub fn launch_app(executable: &Path, env: &[(&str, &Path)], args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(executable);
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("start '{}': {error}", executable.display()))?;
    Ok(())
}

/// A `cmd.exe`-style line describing `launch_app`, for logs and guidance.
/// The launch itself spawns the executable directly, so percent-encoded deep
/// links need no `cmd` escaping here; tokens are only quoted.
pub fn display_launch(executable: &Path, env: &[(&str, &Path)], args: &[&str]) -> String {
    fn quoted(value: &str) -> String {
        if value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            value.to_string()
        } else {
            format!("\"{value}\"")
        }
    }
    let mut parts = Vec::new();
    for (name, value) in env {
        parts.push(format!("set \"{name}={}\" &&", value.to_string_lossy()));
    }
    parts.push(quoted(&executable.to_string_lossy()));
    parts.extend(args.iter().map(|arg| quoted(arg)));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_commands_parse_to_their_executable() {
        let output = "\r\nHKEY_CLASSES_ROOT\\codex\\shell\\open\\command\r\n    (Default)    REG_SZ    \"C:\\Users\\dev\\AppData\\Local\\Programs\\Codex\\Codex.exe\" \"%1\"\r\n\r\n";
        assert_eq!(
            parse_scheme_handler_command(output).unwrap(),
            PathBuf::from(r"C:\Users\dev\AppData\Local\Programs\Codex\Codex.exe")
        );
        let unquoted = "    (Default)    REG_SZ    C:\\Apps\\Claude.exe %1";
        assert_eq!(
            parse_scheme_handler_command(unquoted).unwrap(),
            PathBuf::from(r"C:\Apps\Claude.exe")
        );
        assert!(parse_scheme_handler_command("ERROR: The system was unable to find the specified registry key or value.").is_err());
        assert!(parse_scheme_handler_command("    (Default)    REG_SZ    \"C:\\Apps\\run.bat\" \"%1\"").is_err());
        assert!(parse_scheme_handler_command("    (Default)    REG_SZ    \"Codex.exe\" \"%1\"").is_err());
    }

    #[test]
    fn display_lines_pin_the_environment() {
        let line = display_launch(
            Path::new(r"C:\Apps\Codex.exe"),
            &[("CODEX_HOME", Path::new(r"C:\Users\dev\.codex"))],
            &["codex://threads/new?path=C%3A%5Cproj"],
        );
        assert_eq!(
            line,
            r#"set "CODEX_HOME=C:\Users\dev\.codex" && "C:\Apps\Codex.exe" "codex://threads/new?path=C%3A%5Cproj""#
        );
    }
}

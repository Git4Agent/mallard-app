//! Windows desktop-app launches through registered URL scheme handlers.
//!
//! macOS pins a profile home into a desktop app with `open --env`. Windows
//! resolves the selected URL handler and starts it with the profile environment.
//! MSIX executables must start inside their package context. Package activation
//! discards the caller's environment, so a package-local helper sets it on the
//! child explicitly and acknowledges process creation before we report success.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct WindowsApp {
    pub executable: PathBuf,
    package: Option<PackageApp>,
}

#[derive(Clone, Debug)]
struct PackageApp {
    family: String,
    app_id: String,
}

#[cfg(windows)]
impl WindowsApp {
    pub(crate) fn is_packaged(&self) -> bool {
        self.package.is_some()
    }

    /// Packaged desktop apps keep their roaming data inside the package's
    /// LocalCache; ordinary desktop installs use the user's roaming directory.
    pub(crate) fn roaming_data_dir(&self, local: &Path, roaming: &Path) -> Option<PathBuf> {
        if !local.is_absolute() || !roaming.is_absolute() {
            return None;
        }
        match &self.package {
            None => Some(roaming.to_path_buf()),
            Some(package) => {
                let mut parts = Path::new(&package.family).components();
                if !matches!(parts.next(), Some(std::path::Component::Normal(_)))
                    || parts.next().is_some()
                {
                    return None;
                }
                Some(
                    local
                        .join("Packages")
                        .join(&package.family)
                        .join("LocalCache/Roaming"),
                )
            }
        }
    }
}

#[cfg(any(windows, test))]
fn handler_executable_path(executable: &str) -> Result<PathBuf, String> {
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

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn from_wide(value: &[u16]) -> Result<String, String> {
    let end = value
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("URL scheme handler string is not terminated")?;
    String::from_utf16(&value[..end])
        .map_err(|_| "URL scheme handler string is not valid UTF-16".to_string())
}

#[cfg(windows)]
fn association_string(
    scheme: &str,
    kind: windows_sys::Win32::UI::Shell::ASSOCSTR,
) -> Result<String, String> {
    use windows_sys::Win32::UI::Shell::{
        AssocQueryStringW, ASSOCF_IS_PROTOCOL, ASSOCF_NOFIXUPS, ASSOCF_NOTRUNCATE,
    };
    let scheme_wide = wide(scheme);
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: the input is terminated and the output has `length` WCHARs.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_NOFIXUPS | ASSOCF_NOTRUNCATE,
            kind,
            scheme_wide.as_ptr(),
            std::ptr::null(),
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(format!(
            "cannot resolve the {scheme}:// desktop app (HRESULT {result:#010x})"
        ));
    }
    from_wide(&buffer)
}

#[cfg(windows)]
fn registration_string(key: &str, name: &str) -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CLASSES_ROOT, RRF_RT_REG_SZ};
    let key = wide(key);
    let name = wide(name);
    let mut buffer = vec![0u16; 32_768];
    let mut bytes = (buffer.len() * size_of::<u16>()) as u32;
    // SAFETY: both inputs are terminated; `bytes` is the output capacity.
    let result = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    match result {
        ERROR_SUCCESS => from_wide(&buffer).map(Some),
        ERROR_FILE_NOT_FOUND => Ok(None),
        _ => Err(format!(
            "read URL scheme registration: {}",
            std::io::Error::from_raw_os_error(result as i32)
        )),
    }
}

#[cfg(windows)]
fn packaged_handler_path(root: &Path, executable: &str) -> Result<PathBuf, String> {
    let relative = Path::new(executable);
    if executable.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return Err("packaged URL handler must be relative to its package".to_string());
    }
    handler_executable_path(&root.join(relative).to_string_lossy())
}

/// The selected URL handler, including the identity needed to activate MSIX apps.
#[cfg(windows)]
pub fn scheme_handler(scheme: &str) -> Result<WindowsApp, String> {
    if scheme.is_empty() || !scheme.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Err("URL scheme must be lower-case ASCII letters".to_string());
    }
    use windows_sys::Win32::Storage::Packaging::Appx::GetPackagePathByFullName;
    use windows_sys::Win32::UI::Shell::{ASSOCSTR_EXECUTABLE, ASSOCSTR_PROGID};

    // Ask Windows for the selected association, including MSIX registrations.
    let prog_id = association_string(scheme, ASSOCSTR_PROGID)?;
    let key = format!("{prog_id}\\Shell\\open");
    // MSIX handlers register a package and relative executable, not a command.
    let mut package_app = None;
    let path = if let Some(package) = registration_string(&key, "PackageId")? {
        let identity = registration_string(&key, "AppUserModelID")?
            .ok_or("packaged URL handler has no application identity")?;
        let (family, app_id) = identity
            .split_once('!')
            .filter(|(family, app_id)| !family.is_empty() && !app_id.is_empty())
            .ok_or("packaged URL handler has an invalid application identity")?;
        package_app = Some(PackageApp {
            family: family.to_string(),
            app_id: app_id.to_string(),
        });
        let executable = registration_string(&key, "PackageRelativeExecutable")?
            .ok_or("packaged URL handler has no executable")?;
        let package = wide(&package);
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        // SAFETY: the package name is terminated; the output has `length` WCHARs.
        let result =
            unsafe { GetPackagePathByFullName(package.as_ptr(), &mut length, buffer.as_mut_ptr()) };
        if result != 0 {
            return Err(format!(
                "locate the {scheme}:// app package: {}",
                std::io::Error::from_raw_os_error(result as i32)
            ));
        }
        packaged_handler_path(Path::new(&from_wide(&buffer)?), &executable)?
    } else {
        handler_executable_path(&association_string(scheme, ASSOCSTR_EXECUTABLE)?)?
    };
    if !path.is_file() {
        return Err(format!(
            "the {scheme}:// handler '{}' is not installed",
            path.display()
        ));
    }
    Ok(WindowsApp {
        executable: path,
        package: package_app,
    })
}

/// Start a desktop app executable with pinned environment and arguments.
/// The process is detached; the app owns its own lifetime.
#[cfg(windows)]
pub fn launch_app(app: &WindowsApp, env: &[(&str, &Path)], args: &[&str]) -> Result<(), String> {
    if let Some(package) = &app.package {
        return launch_packaged_app(package, &app.executable, env, args);
    }
    let mut command = std::process::Command::new(&app.executable);
    // Console-subsystem launchers must show only the desktop app's own UI.
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    for (name, value) in env {
        command.env(name, value);
    }
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("start '{}': {error}", app.executable.display()))?;
    Ok(())
}

/// Quote one argument for the Windows argv parser used by ProcessStartInfo.
fn windows_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            slashes * if character == '"' { 2 } else { 1 },
        ));
        if character == '"' {
            quoted.push('\\');
        }
        quoted.push(character);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

fn child_launch_script(executable: &Path, env: &[(&str, &Path)], args: &[&str]) -> String {
    use super::launch::powershell_quote as quote;
    let arguments = args
        .iter()
        .map(|arg| windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = format!(
        "$ErrorActionPreference = 'Stop'; $p = New-Object System.Diagnostics.ProcessStartInfo; \
         $p.UseShellExecute = $false; $p.CreateNoWindow = $true; $p.FileName = {}; $p.Arguments = {}; ",
        quote(&executable.to_string_lossy()),
        quote(&arguments),
    );
    for (name, value) in env {
        script.push_str(&format!(
            "$p.EnvironmentVariables[{}] = {}; ",
            quote(name),
            quote(&value.to_string_lossy()),
        ));
    }
    script.push_str("[System.Diagnostics.Process]::Start($p) | Out-Null");
    script
}

fn package_launch_script(package: &PackageApp, child_script: &str) -> String {
    use super::launch::powershell_quote as quote;
    // Encode in PowerShell so the displayed command keeps paths readable for
    // guidance and log redaction; no user-controlled text is evaluated as code.
    format!(
        "Invoke-CommandInDesktopPackage -PackageFamilyName {} -AppId {} \
         -Command ([System.IO.Path]::Combine($PSHOME, 'powershell.exe')) \
         -Args ('-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand ' + \
         [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes({}))) -ErrorAction Stop",
        quote(&package.family),
        quote(&package.app_id),
        quote(child_script),
    )
}

#[cfg(windows)]
fn receipt_script(body: &str, receipt: &Path) -> String {
    use super::launch::powershell_quote as quote;
    format!(
        "try {{ {body}; $result = 'ok' }} catch {{ $result = $_.Exception.Message }}; \
         [IO.File]::WriteAllText({}, $result.Substring(0, [Math]::Min(4096, $result.Length))); \
         [IO.File]::Move({}, {})",
        quote(&receipt.with_extension("tmp").to_string_lossy()),
        quote(&receipt.with_extension("tmp").to_string_lossy()),
        quote(&receipt.to_string_lossy()),
    )
}

#[cfg(windows)]
fn run_package_script(script: &str, receipt: &Path) -> Result<(), String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let system_root = std::env::var_os("SystemRoot").ok_or("Windows SystemRoot is unavailable")?;
    let powershell =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut launcher = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(aws_smithy_types::base64::encode(bytes))
        // PowerShell 7's module path cannot load the Windows PowerShell Appx module.
        .env_remove("PSModulePath")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: only the app should show a window.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start Windows package launcher: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if receipt.is_file() {
            // The helper publishes at most 4096 UTF-16 code units atomically.
            let mut result = String::new();
            std::fs::File::open(receipt)
                .and_then(|file| file.take(16 * 1024 + 1).read_to_string(&mut result))
                .map_err(|error| format!("read Windows app launch result: {error}"))?;
            if result.len() > 16 * 1024 {
                return Err("Windows app launch result is too large".to_string());
            }
            return if result == "ok" {
                Ok(())
            } else {
                Err(format!("start packaged desktop app: {result}"))
            };
        }
        if let Some(status) = launcher
            .try_wait()
            .map_err(|error| format!("wait for package launcher: {error}"))?
        {
            if !status.success() {
                return Err(format!("Windows package launcher failed ({status})"));
            }
        }
        if Instant::now() >= deadline {
            let _ = launcher.kill();
            let _ = launcher.wait();
            return Err(
                "Windows package launcher did not confirm app startup within 20 seconds"
                    .to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn launch_packaged_app(
    package: &PackageApp,
    executable: &Path,
    env: &[(&str, &Path)],
    args: &[&str],
) -> Result<(), String> {
    let temp =
        tempfile::tempdir().map_err(|error| format!("prepare Windows app launch: {error}"))?;
    let receipt = temp.path().join("launch-result");
    let child = receipt_script(&child_launch_script(executable, env, args), &receipt);
    // Invoke-CommandInDesktopPackage starts the helper asynchronously. Both the
    // activator and the helper must surface their errors to the calling command.
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; \
         try {{ {} }} catch {{ {} }}",
        package_launch_script(package, &child),
        receipt_script("throw $_", &receipt),
    );
    run_package_script(&script, &receipt)
}

/// The launch command, with profile paths readable for logs and guidance.
pub fn display_launch(app: &WindowsApp, env: &[(&str, &Path)], args: &[&str]) -> String {
    if let Some(package) = &app.package {
        return package_launch_script(package, &child_launch_script(&app.executable, env, args));
    }
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
    parts.push(quoted(&app.executable.to_string_lossy()));
    parts.extend(args.iter().map(|arg| quoted(arg)));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn roaming_data_uses_the_registered_package_family_without_changing_profile_homes() {
        let mut app = WindowsApp {
            executable: PathBuf::from(r"C:\Apps\Claude.exe"),
            package: None,
        };
        let local = Path::new(r"C:\Users\dev\AppData\Local");
        let roaming = Path::new(r"C:\Users\dev\AppData\Roaming");
        assert!(!app.is_packaged());
        assert_eq!(
            app.roaming_data_dir(local, roaming),
            Some(roaming.to_path_buf())
        );
        app.package = Some(PackageApp {
            family: "Claude_testpublisher".into(),
            app_id: "App".into(),
        });
        assert!(app.is_packaged());
        assert_eq!(
            app.roaming_data_dir(local, roaming),
            Some(local.join("Packages/Claude_testpublisher/LocalCache/Roaming"))
        );
        for family in ["", "..", r"..\outside", r"C:\outside", r"family\child"] {
            app.package.as_mut().unwrap().family = family.into();
            assert_eq!(app.roaming_data_dir(local, roaming), None);
        }
        assert_eq!(app.roaming_data_dir(Path::new("."), roaming), None);
    }

    #[test]
    fn handler_paths_must_be_absolute_executables() {
        assert_eq!(
            handler_executable_path(r"C:\Program Files\WindowsApps\Codex_1.0\app\ChatGPT.exe")
                .unwrap(),
            PathBuf::from(r"C:\Program Files\WindowsApps\Codex_1.0\app\ChatGPT.exe")
        );
        assert_eq!(
            handler_executable_path(r"C:\Apps\Claude.exe").unwrap(),
            PathBuf::from(r"C:\Apps\Claude.exe")
        );
        assert!(handler_executable_path(r"C:\Apps\run.bat").is_err());
        assert!(handler_executable_path("Codex.exe").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn packaged_handler_uses_its_registered_relative_executable() {
        let root = Path::new(r"C:\Program Files\WindowsApps\Codex_1.0");
        assert_eq!(
            packaged_handler_path(root, r"app\ChatGPT.exe").unwrap(),
            root.join(r"app\ChatGPT.exe")
        );
        for bad in [
            "",
            r"..\Other.exe",
            r"C:\Other.exe",
            r"\Other.exe",
            r"C:Other.exe",
            r"app\run.bat",
        ] {
            assert!(packaged_handler_path(root, bad).is_err(), "{bad}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn launch_fixture_process() {
        let Some(output) = std::env::var_os("MALLARD_LAUNCH_TEST_OUTPUT") else {
            return;
        };
        let output = PathBuf::from(output);
        let result = serde_json::json!({
            "home": PathBuf::from(std::env::var_os("CODEX_HOME").unwrap()),
            "claude_home": std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            "argument": std::env::args().last().unwrap(),
            "arguments": std::env::args().collect::<Vec<_>>(),
        });
        let pending = output.with_extension("tmp");
        std::fs::write(&pending, serde_json::to_vec(&result).unwrap()).unwrap();
        std::fs::rename(pending, output).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn launch_passes_selected_home_and_deep_link_to_child() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("selected home's %profile% 测试");
        let output = temp.path().join("launch.json");
        let deep_link = "codex://threads/new?path=C%3A%5Cmy%20project";
        launch_app(
            &WindowsApp {
                executable: std::env::current_exe().unwrap(),
                package: None,
            },
            &[
                ("CODEX_HOME", &home),
                ("MALLARD_LAUNCH_TEST_OUTPUT", &output),
            ],
            &[
                "--exact",
                "providers::windows_app::tests::launch_fixture_process",
                "--skip",
                deep_link,
            ],
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !output.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(result["home"], serde_json::json!(home));
        assert_eq!(result["argument"], deep_link);
    }

    #[cfg(windows)]
    fn verify_package_helper(package: Option<PackageApp>) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home's ‘quotes’ %profile% $(bad) 测试");
        let claude_home = temp.path().join("claude home");
        let output = temp.path().join("child.json");
        let receipt = temp.path().join("receipt");
        let project = temp.path().join("project's %folder% & 测试");
        std::fs::create_dir(&project).unwrap();
        let deep_link = crate::providers::codex::actions::codex_project_deep_link(
            &std::fs::canonicalize(&project).unwrap(),
        );
        // Also exercise Windows argv quoting of embedded quotes and trailing slashes.
        let argument = "codex://threads/new?path=C%3A%5Cproject & \"quotes\" \\";
        let env = [
            ("CODEX_HOME", home.as_path()),
            ("CLAUDE_CONFIG_DIR", claude_home.as_path()),
            ("MALLARD_LAUNCH_TEST_OUTPUT", output.as_path()),
        ];
        let args = [
            "--exact",
            "providers::windows_app::tests::launch_fixture_process",
            "--skip",
            argument,
            "--skip",
            deep_link.as_str(),
        ];
        let executable = std::env::current_exe().unwrap();
        if let Some(package) = &package {
            launch_packaged_app(package, &executable, &env, &args).unwrap();
        } else {
            let script = receipt_script(&child_launch_script(&executable, &env, &args), &receipt);
            run_package_script(&script, &receipt).unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !output.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(result["home"], serde_json::json!(home));
        assert_eq!(result["claude_home"], serde_json::json!(claude_home));
        assert_eq!(result["arguments"][4], argument);
        let received = url::Url::parse(result["argument"].as_str().unwrap()).unwrap();
        assert_eq!(received.scheme(), "codex");
        assert_eq!(received.host_str(), Some("threads"));
        assert_eq!(received.path(), "/new");
        assert_eq!(
            received.query_pairs().collect::<Vec<_>>(),
            vec![(
                "path".into(),
                crate::local_paths::canonicalize(&project)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            )],
        );

        let missing = temp.path().join("missing.exe");
        let error = if let Some(package) = &package {
            launch_packaged_app(package, &missing, &env, &[])
        } else {
            let receipt = temp.path().join("failure");
            run_package_script(
                &receipt_script(&child_launch_script(&missing, &env, &[]), &receipt),
                &receipt,
            )
        }
        .unwrap_err();
        assert!(error.starts_with("start packaged desktop app:"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn package_helper_preserves_homes_and_arguments_and_reports_start_errors() {
        verify_package_helper(None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires installed Codex and Claude MSIX packages; launches only a temporary-home fixture"]
    fn installed_package_contexts_preserve_homes_and_report_start_errors() {
        for scheme in ["codex", "claude"] {
            let app = scheme_handler(scheme).unwrap();
            assert!(app.package.is_some(), "{scheme} must be installed as MSIX");
            verify_package_helper(app.package);
        }
    }

    #[test]
    fn display_lines_pin_the_environment() {
        let line = display_launch(
            &WindowsApp {
                executable: PathBuf::from(r"C:\Apps\Codex.exe"),
                package: None,
            },
            &[("CODEX_HOME", Path::new(r"C:\Users\dev\.codex"))],
            &["codex://threads/new?path=C%3A%5Cproj"],
        );
        assert_eq!(
            line,
            r#"set "CODEX_HOME=C:\Users\dev\.codex" && "C:\Apps\Codex.exe" "codex://threads/new?path=C%3A%5Cproj""#
        );
    }
}

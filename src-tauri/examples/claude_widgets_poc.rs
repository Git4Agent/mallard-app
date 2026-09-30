//! Isolated widget rendering experiment. It does not call Mallard services.
//!
//! Start the loopback server with `node poc/claude-widgets/server.mjs`.
//! Run this example with `--server-url <URL> --interactive` to test frames.

use serde::Serialize;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use tauri::{State, WebviewUrl, WebviewWindowBuilder};

const MAX_WIDGET_BYTES: u64 = 1024 * 1024;

struct PocState {
    widget_code: String,
    server_url: String,
    interactive_enabled: bool,
    report_path: Option<PathBuf>,
}

#[derive(Serialize)]
struct PocData {
    widget_code: String,
    server_url: String,
    interactive_enabled: bool,
}

#[tauri::command]
fn poc_data(state: State<'_, PocState>) -> PocData {
    PocData {
        widget_code: state.widget_code.clone(),
        server_url: state.server_url.clone(),
        interactive_enabled: state.interactive_enabled,
    }
}

#[tauri::command]
fn poc_probe() -> &'static str {
    "parent-command-ok"
}

#[tauri::command]
fn poc_report(report: Value, state: State<'_, PocState>) -> Result<(), String> {
    let object = report.as_object().ok_or("report must be an object")?;
    if object.len() > 16
        || object.values().any(|value| match value {
            Value::Bool(_) | Value::Null | Value::Number(_) => false,
            Value::String(text) => text.len() > 24,
            _ => true,
        })
    {
        return Err("report contains unsupported data".into());
    }
    let text = serde_json::to_string_pretty(&report).map_err(|_| "encode POC report")?;
    if let Some(path) = &state.report_path {
        std::fs::write(path, &text).map_err(|_| "write POC report")?;
    }
    println!("POC_REPORT {}", serde_json::to_string(&report).unwrap());
    Ok(())
}

fn read_widget(path: &str) -> Result<String, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| "open widget input")?;
    let metadata = file.metadata().map_err(|_| "inspect widget input")?;
    if !metadata.is_file() || metadata.len() > MAX_WIDGET_BYTES {
        return Err("widget input must be a regular file below 1 MiB".into());
    }
    let mut text = String::new();
    file.take(MAX_WIDGET_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| "widget input is not UTF-8")?;
    if text.len() as u64 > MAX_WIDGET_BYTES {
        return Err("widget input exceeds 1 MiB".into());
    }
    Ok(text)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| -> Option<&str> {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
    };
    let interactive = args.iter().any(|arg| arg == "--interactive");
    let server_url = option("--server-url").unwrap_or("").to_string();
    if interactive {
        let url = tauri::Url::parse(&server_url)?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || url.path() != "/"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("interactive mode requires the POC loopback origin".into());
        }
    }
    let example = option("--example").unwrap_or("diagram");
    if !matches!(example, "diagram" | "interactive" | "isolation") {
        return Err("unknown widget example".into());
    }
    let widget_code = match option("--widget-file") {
        Some(path) => read_widget(path)?,
        None => include_str!("../../poc/claude-widgets/web/diagram.svg").to_string(),
    };
    let state = PocState {
        widget_code,
        server_url: server_url.clone(),
        interactive_enabled: interactive,
        report_path: option("--report-path").map(PathBuf::from),
    };
    let mut context = tauri::generate_context!("../poc/claude-widgets/tauri.conf.json");
    if interactive {
        // Only this example allows its loopback frame origin. The production
        // app configuration continues to use frame-src 'none'.
        let csp = context
            .config()
            .app
            .security
            .csp
            .as_ref()
            .unwrap()
            .to_string();
        context.config_mut().app.security.csp = Some(tauri::utils::config::Csp::Policy(
            csp.replace("frame-src 'none'", &format!("frame-src {server_url}")),
        ));
    }
    let profile = tempfile::tempdir()?;
    let profile_path = profile.path().to_path_buf();
    let initialization = format!("window.POC_EXAMPLE = {};", serde_json::to_string(example)?);
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![poc_data, poc_probe, poc_report])
        .setup(move |app| {
            let window =
                WebviewWindowBuilder::new(app, "widget-poc", WebviewUrl::App("index.html".into()))
                    .title("Mallard · Claude widgets · POC")
                    .inner_size(1120.0, 860.0)
                    .min_inner_size(680.0, 560.0)
                    .data_directory(profile_path)
                    .initialization_script(initialization)
                    .on_navigation(|url| matches!(url.scheme(), "tauri" | "http" | "about"))
                    .build()?;
            window.set_focus()?;
            Ok(())
        })
        .run(context)?;
    drop(profile);
    Ok(())
}

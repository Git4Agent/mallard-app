use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use toml::{Table, Value};
use walkdir::WalkDir;

const MANAGED_MARKETPLACES: &[&str] =
    &["openai-curated", "openai-bundled", "openai-primary-runtime"];

const NODE_REPL: &str = "node_repl";
const COMPUTER_USE: &str = "computer-use";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ConfigIssue {
    pub id: String,
    pub code: String,
    pub message: String,
}

pub fn managed_marketplace_name(name: &str) -> bool {
    MANAGED_MARKETPLACES.contains(&name)
}

fn parse_table(bytes: &[u8], description: &str) -> Result<Table, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{} is not valid UTF-8: {}", description, error))?;
    text.parse::<Table>()
        .map_err(|error| format!("parse {}: {}", description, error))
}

fn serialize_table(table: &Table) -> Result<Vec<u8>, String> {
    let mut text = toml::to_string_pretty(table)
        .map_err(|error| format!("serialize Codex config: {}", error))?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text.into_bytes())
}

fn value_table(value: &Value) -> Option<&Table> {
    value.as_table()
}

fn value_table_mut(value: &mut Value) -> Option<&mut Table> {
    value.as_table_mut()
}

fn table_string<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(Value::as_str)
}

fn file_name_is(path: &str, expected: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == expected || name == format!("{}.exe", expected))
}

fn node_repl_fingerprint(value: &Value) -> bool {
    let Some(table) = value_table(value) else {
        return false;
    };
    let Some(command) = table_string(table, "command") else {
        return false;
    };
    if !file_name_is(command, "node_repl") {
        return false;
    }
    let Some(env) = table.get("env").and_then(Value::as_table) else {
        return false;
    };
    env.get("CODEX_HOME").and_then(Value::as_str).is_some()
        && env
            .get("NODE_REPL_TRUSTED_CODE_PATHS")
            .and_then(Value::as_str)
            .is_some()
        && env
            .get("NODE_REPL_NODE_PATH")
            .and_then(Value::as_str)
            .is_some()
}

fn computer_use_fingerprint(value: &Value) -> bool {
    let Some(table) = value_table(value) else {
        return false;
    };
    let Some(command) = table_string(table, "command") else {
        return false;
    };
    let managed_command = command.contains("Codex Computer Use.app/")
        && file_name_is(command, "SkyComputerUseClient");
    let mcp_arg = table
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("mcp")));
    managed_command && mcp_arg
}

fn managed_mcp_fingerprint(name: &str, value: &Value) -> bool {
    match name {
        NODE_REPL => node_repl_fingerprint(value),
        COMPUTER_USE => computer_use_fingerprint(value),
        _ => false,
    }
}

fn child_table_mut<'a>(root: &'a mut Table, key: &str) -> Result<&'a mut Table, String> {
    if !root.contains_key(key) {
        root.insert(key.to_string(), Value::Table(Table::new()));
    }
    root.get_mut(key)
        .and_then(Value::as_table_mut)
        .ok_or_else(|| format!("Codex config key '{}' is not a table", key))
}

pub fn explicitly_disabled_plugin_ids_from_bytes(bytes: &[u8]) -> Result<Vec<String>, String> {
    let table = parse_table(bytes, "Codex config.toml")?;
    let mut ids: Vec<String> = table
        .get("plugins")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|plugins| plugins.iter())
        .filter_map(|(id, value)| {
            let disabled = value
                .as_table()
                .and_then(|plugin| plugin.get("enabled"))
                .and_then(Value::as_bool)
                == Some(false);
            let shaped = id
                .rsplit_once('@')
                .is_some_and(|(plugin, marketplace)| !plugin.is_empty() && !marketplace.is_empty());
            (disabled && shaped).then(|| id.clone())
        })
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn read_optional_config(path: &Path) -> Result<(Vec<u8>, Table), String> {
    if !path.exists() {
        return Ok((Vec::new(), Table::new()));
    }
    let bytes = fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))?;
    let table = parse_table(&bytes, &format!("'{}'", path.display()))?;
    Ok((bytes, table))
}

fn unique_backup_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    for suffix in 0u32.. {
        let extra = if suffix == 0 {
            String::new()
        } else {
            format!("-{}", suffix)
        };
        let candidate =
            path.with_file_name(format!("{}.bak-agent-sync-{}{}", name, timestamp, extra));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded backup suffix loop")
}

fn atomic_replace_with_backup(path: &Path, previous: &[u8], next: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create '{}': {}", parent.display(), error))?;

    if !previous.is_empty() || path.exists() {
        let backup = unique_backup_path(path);
        fs::write(&backup, previous).map_err(|error| {
            format!(
                "backup '{}' to '{}': {}",
                path.display(),
                backup.display(),
                error
            )
        })?;
        if let Ok(metadata) = fs::metadata(path) {
            let _ = fs::set_permissions(&backup, metadata.permissions());
        }
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let tmp = parent.join(format!(
        ".{}.agent-sync-tmp-{}-{}",
        file_name,
        std::process::id(),
        nonce
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = fs::File::create(&tmp)
            .map_err(|error| format!("create '{}': {}", tmp.display(), error))?;
        if let Ok(metadata) = fs::metadata(path) {
            let _ = file.set_permissions(metadata.permissions());
        }
        file.write_all(next)
            .map_err(|error| format!("write '{}': {}", tmp.display(), error))?;
        file.sync_all()
            .map_err(|error| format!("sync '{}': {}", tmp.display(), error))?;
        fs::rename(&tmp, path)
            .map_err(|error| format!("replace '{}': {}", path.display(), error))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

pub fn rebind_managed_marketplaces(
    config_path: &Path,
    sources: &BTreeMap<String, PathBuf>,
) -> Result<Vec<String>, String> {
    for name in sources.keys() {
        if !managed_marketplace_name(name) {
            return Err(format!("'{}' is not an OpenAI-managed marketplace", name));
        }
    }
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let (previous, mut config) = read_optional_config(config_path)?;
    let marketplaces = child_table_mut(&mut config, "marketplaces")?;
    let mut changed = Vec::new();

    for (name, source) in sources {
        if !source.is_absolute() {
            return Err(format!(
                "managed marketplace '{}' source must be absolute: {}",
                name,
                source.display()
            ));
        }
        if !marketplaces.contains_key(name) {
            marketplaces.insert(name.clone(), Value::Table(Table::new()));
        }
        let entry = marketplaces
            .get_mut(name)
            .and_then(value_table_mut)
            .ok_or_else(|| format!("marketplaces.{} is not a table", name))?;
        if let Some(kind) = entry.get("source_type").and_then(Value::as_str) {
            if kind != "local" {
                return Err(format!(
                    "managed marketplace '{}' has unexpected source_type '{}'",
                    name, kind
                ));
            }
        }

        if entry.get("source_type").and_then(Value::as_str) != Some("local") {
            entry.insert(
                "source_type".to_string(),
                Value::String("local".to_string()),
            );
            changed.push(format!("marketplaces.{}.source_type", name));
        }
        let source = source.to_string_lossy().into_owned();
        if entry.get("source").and_then(Value::as_str) != Some(source.as_str()) {
            entry.insert("source".to_string(), Value::String(source));
            changed.push(format!("marketplaces.{}.source", name));
        }
    }

    if changed.is_empty() {
        return Ok(changed);
    }
    changed.sort();
    changed.dedup();
    let next = serialize_table(&config)?;
    atomic_replace_with_backup(config_path, &previous, &next)?;
    Ok(changed)
}

pub fn remove_explicit_curated_marketplace(config_path: &Path) -> Result<Vec<String>, String> {
    let (previous, mut config) = read_optional_config(config_path)?;
    let Some(marketplaces) = config.get_mut("marketplaces").and_then(Value::as_table_mut) else {
        return Ok(Vec::new());
    };
    let Some(entry) = marketplaces.get("openai-curated") else {
        return Ok(Vec::new());
    };
    if entry
        .as_table()
        .and_then(|table| table.get("source_type"))
        .and_then(Value::as_str)
        != Some("local")
    {
        return Err(
            "refusing to remove non-local marketplaces.openai-curated registration".to_string(),
        );
    }
    marketplaces.remove("openai-curated");
    if marketplaces.is_empty() {
        config.remove("marketplaces");
    }
    let next = serialize_table(&config)?;
    atomic_replace_with_backup(config_path, &previous, &next)?;
    Ok(vec!["marketplaces.openai-curated".to_string()])
}

fn locate_computer_use_app(target_home: &Path) -> Option<PathBuf> {
    let root = target_home.join("plugins/cache/openai-bundled/computer-use");
    if !root.is_dir() {
        return None;
    }
    let mut matches: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir() && entry.file_name() == "Codex Computer Use.app")
        .map(|entry| entry.into_path())
        .collect();
    matches.sort();
    matches.pop()
}

fn set_env_string(env: &mut Table, key: &str, value: String) {
    env.insert(key.to_string(), Value::String(value));
}

fn rewrite_node_repl_template(
    value: &mut Value,
    target_home: &Path,
    computer_use_app: Option<&Path>,
) -> Result<(), String> {
    let table = value
        .as_table_mut()
        .ok_or_else(|| "managed node_repl template is not a table".to_string())?;
    let env = table
        .get_mut("env")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| "managed node_repl template has no env table".to_string())?;
    let target_home = target_home.to_string_lossy().into_owned();
    set_env_string(env, "CODEX_HOME", target_home.clone());
    set_env_string(env, "NODE_REPL_TRUSTED_CODE_PATHS", target_home);
    match computer_use_app {
        Some(path) => set_env_string(
            env,
            "SKY_CUA_SERVICE_PATH",
            path.to_string_lossy().into_owned(),
        ),
        None => {
            env.remove("SKY_CUA_SERVICE_PATH");
        }
    }
    Ok(())
}

fn rewrite_computer_use_template(value: &mut Value, app: &Path) -> Result<(), String> {
    let table = value
        .as_table_mut()
        .ok_or_else(|| "managed computer-use template is not a table".to_string())?;
    let executable = app.join(
        "Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
    );
    let cwd = app
        .parent()
        .ok_or_else(|| "managed computer-use app has no parent directory".to_string())?;
    table.insert(
        "command".to_string(),
        Value::String(executable.to_string_lossy().into_owned()),
    );
    table.insert(
        "cwd".to_string(),
        Value::String(cwd.to_string_lossy().into_owned()),
    );
    Ok(())
}

fn validate_runtime_path(label: &str, raw: &str) -> Result<(), String> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(format!("{} must be an absolute target runtime path", label));
    }
    if !path.exists() {
        return Err(format!(
            "{} points to missing target runtime path '{}'",
            label, raw
        ));
    }
    Ok(())
}

fn validate_runtime_paths(name: &str, value: &Value) -> Result<(), String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("managed MCP '{}' is not a table", name))?;
    if let Some(command) = table.get("command").and_then(Value::as_str) {
        validate_runtime_path(&format!("mcp_servers.{}.command", name), command)?;
    }
    if let Some(cwd) = table.get("cwd").and_then(Value::as_str) {
        validate_runtime_path(&format!("mcp_servers.{}.cwd", name), cwd)?;
    }
    if let Some(env) = table.get("env").and_then(Value::as_table) {
        for key in [
            "NODE_REPL_NODE_MODULE_DIRS",
            "NODE_REPL_NODE_PATH",
            "CODEX_CLI_PATH",
            "CODEX_HOME",
            "NODE_REPL_TRUSTED_CODE_PATHS",
            "SKY_CUA_SERVICE_PATH",
        ] {
            if let Some(raw) = env.get(key).and_then(Value::as_str) {
                validate_runtime_path(&format!("mcp_servers.{}.env.{}", name, key), raw)?;
            }
        }
    }
    Ok(())
}

pub fn repair_managed_mcp_from_default(
    target_config: &Path,
    default_config: &Path,
    target_home: &Path,
) -> Result<Vec<String>, String> {
    let default_bytes = fs::read(default_config).map_err(|error| {
        format!(
            "read default Codex config '{}': {}",
            default_config.display(),
            error
        )
    })?;
    let default = parse_table(&default_bytes, "default Codex config.toml")?;
    let default_servers = default.get("mcp_servers").and_then(Value::as_table);
    let (previous, mut target) = read_optional_config(target_config)?;

    let computer_use_app = locate_computer_use_app(target_home);
    let computer_use_enabled = target
        .get("plugins")
        .and_then(Value::as_table)
        .and_then(|plugins| plugins.get("computer-use@openai-bundled"))
        .and_then(Value::as_table)
        .and_then(|plugin| plugin.get("enabled"))
        .and_then(Value::as_bool)
        == Some(true);
    if computer_use_enabled && computer_use_app.is_none() {
        return Err(format!(
            "computer-use@openai-bundled is enabled but no Codex Computer Use.app payload exists under '{}'",
            target_home
                .join("plugins/cache/openai-bundled/computer-use")
                .display()
        ));
    }
    let mut templates = BTreeMap::new();
    for name in [NODE_REPL, COMPUTER_USE] {
        let Some(mut template) = default_servers
            .and_then(|servers| servers.get(name))
            .cloned()
        else {
            continue;
        };
        if !managed_mcp_fingerprint(name, &template) {
            continue;
        }
        if name == NODE_REPL {
            rewrite_node_repl_template(&mut template, target_home, computer_use_app.as_deref())?;
        } else if name == COMPUTER_USE {
            let Some(app) = computer_use_app.as_deref() else {
                continue;
            };
            rewrite_computer_use_template(&mut template, app)?;
        }
        validate_runtime_paths(name, &template)?;
        templates.insert(name.to_string(), template);
    }
    if templates.is_empty() {
        return Ok(Vec::new());
    }

    let target_servers = child_table_mut(&mut target, "mcp_servers")?;
    let mut changed = Vec::new();
    for (name, template) in templates {
        // A same-name user-authored MCP server is portable state. Only replace
        // an absent block or a block carrying the managed fingerprint.
        if target_servers
            .get(&name)
            .is_some_and(|value| !managed_mcp_fingerprint(&name, value))
        {
            continue;
        }
        if target_servers.get(&name) != Some(&template) {
            target_servers.insert(name.clone(), template);
            changed.push(format!("mcp_servers.{}", name));
        }
    }

    if changed.is_empty() {
        return Ok(changed);
    }
    changed.sort();
    let next = serialize_table(&target)?;
    atomic_replace_with_backup(target_config, &previous, &next)?;
    Ok(changed)
}

fn issue(id: impl Into<String>, code: &str, message: impl Into<String>) -> ConfigIssue {
    ConfigIssue {
        id: id.into(),
        code: code.to_string(),
        message: message.into(),
    }
}

fn codex_home_ancestor(path: &Path) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if component.as_os_str() == ".codex" {
            return Some(prefix);
        }
    }
    None
}

fn same_approved_path(actual: &Path, expected: &Path) -> bool {
    match (actual.canonicalize(), expected.canonicalize()) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => actual == expected,
    }
}

fn within_approved_path(actual: &Path, expected_root: &Path) -> bool {
    match (actual.canonicalize(), expected_root.canonicalize()) {
        (Ok(actual), Ok(expected)) => actual.starts_with(expected),
        _ => actual.starts_with(expected_root),
    }
}

fn managed_marketplace_home_is_allowed_for_default(
    name: &str,
    source: &Path,
    target_home: &Path,
    default_home: &Path,
) -> bool {
    let machine_home = default_home.parent().unwrap_or(default_home);
    match name {
        "openai-curated" => [
            target_home.join(".tmp/plugins"),
            default_home.join(".tmp/plugins"),
        ]
        .iter()
        .any(|expected| same_approved_path(source, expected)),
        "openai-bundled" => [
            target_home.join(".tmp/bundled-marketplaces/openai-bundled"),
            default_home.join(".tmp/bundled-marketplaces/openai-bundled"),
        ]
        .iter()
        .any(|expected| same_approved_path(source, expected)),
        "openai-primary-runtime" => {
            within_approved_path(source, &machine_home.join(".cache/codex-runtimes"))
        }
        _ => false,
    }
}

#[cfg(test)]
fn managed_marketplace_home_is_allowed(name: &str, source: &Path, target_home: &Path) -> bool {
    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .is_some_and(|default_home| {
            managed_marketplace_home_is_allowed_for_default(
                name,
                source,
                target_home,
                &default_home,
            )
        })
}

fn inspect_runtime_path(issues: &mut Vec<ConfigIssue>, id: String, raw: &str, target_home: &Path) {
    let path = Path::new(raw);
    if !path.is_absolute() {
        issues.push(issue(
            id,
            "managed_runtime_path_invalid",
            format!("Managed runtime path '{}' is not absolute", raw),
        ));
        return;
    }
    if let Some(home) = codex_home_ancestor(path) {
        if home != target_home {
            issues.push(issue(
                id,
                "managed_path_wrong_home",
                format!("Managed path '{}' points into a different Codex home", raw),
            ));
            return;
        }
    }
    if !path.exists() {
        issues.push(issue(
            id,
            "managed_runtime_path_missing",
            format!("Managed runtime path '{}' does not exist", raw),
        ));
    }
}

pub fn inspect_managed_config_with_default(
    config_path: &Path,
    target_home: &Path,
    default_home: &Path,
) -> Vec<ConfigIssue> {
    inspect_managed_config_inner(config_path, target_home, Some(default_home))
}

fn inspect_managed_config_inner(
    config_path: &Path,
    target_home: &Path,
    default_home: Option<&Path>,
) -> Vec<ConfigIssue> {
    if !config_path.exists() {
        return Vec::new();
    }
    let bytes = match fs::read(config_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return vec![issue(
                "config.toml",
                "config_unreadable",
                format!("Cannot read '{}': {}", config_path.display(), error),
            )]
        }
    };
    let config = match parse_table(&bytes, &format!("'{}'", config_path.display())) {
        Ok(config) => config,
        Err(error) => return vec![issue("config.toml", "config_invalid", error)],
    };
    let mut issues = Vec::new();

    if let Some(marketplaces) = config.get("marketplaces").and_then(Value::as_table) {
        for name in MANAGED_MARKETPLACES {
            let Some(entry) = marketplaces.get(*name).and_then(Value::as_table) else {
                continue;
            };
            let id = format!("marketplaces.{}.source", name);
            if entry.get("source_type").and_then(Value::as_str) != Some("local") {
                issues.push(issue(
                    id.clone(),
                    "managed_marketplace_source_type",
                    format!(
                        "Managed marketplace '{}' must use a target-local source",
                        name
                    ),
                ));
                continue;
            }
            let Some(raw) = entry.get("source").and_then(Value::as_str) else {
                issues.push(issue(
                    id,
                    "managed_marketplace_source_missing",
                    format!("Managed marketplace '{}' has no local source path", name),
                ));
                continue;
            };
            let path = Path::new(raw);
            if !path.is_absolute() {
                issues.push(issue(
                    id,
                    "managed_marketplace_source_invalid",
                    format!("Managed marketplace '{}' source is not absolute", name),
                ));
                continue;
            }
            if !default_home.is_some_and(|default_home| {
                managed_marketplace_home_is_allowed_for_default(
                    name,
                    path,
                    target_home,
                    default_home,
                )
            }) {
                issues.push(issue(
                    id,
                    "managed_marketplace_wrong_home",
                    format!(
                        "Managed marketplace '{}' points into another Codex home: {}",
                        name, raw
                    ),
                ));
                continue;
            }
            if !path.exists() {
                issues.push(issue(
                    id,
                    "managed_marketplace_source_missing",
                    format!(
                        "Managed marketplace '{}' source '{}' does not exist",
                        name, raw
                    ),
                ));
            }
        }
    }

    if let Some(servers) = config.get("mcp_servers").and_then(Value::as_table) {
        if let Some(node) = servers
            .get(NODE_REPL)
            .filter(|value| node_repl_fingerprint(value))
        {
            let table = node.as_table().expect("fingerprint requires table");
            if let Some(command) = table.get("command").and_then(Value::as_str) {
                inspect_runtime_path(
                    &mut issues,
                    "mcp_servers.node_repl.command".to_string(),
                    command,
                    target_home,
                );
            }
            if let Some(env) = table.get("env").and_then(Value::as_table) {
                let target = target_home.to_string_lossy();
                for (key, code) in [
                    ("CODEX_HOME", "managed_mcp_home_mismatch"),
                    (
                        "NODE_REPL_TRUSTED_CODE_PATHS",
                        "managed_mcp_trusted_path_mismatch",
                    ),
                ] {
                    let id = format!("mcp_servers.node_repl.env.{}", key);
                    if env.get(key).and_then(Value::as_str) != Some(target.as_ref()) {
                        issues.push(issue(
                            id,
                            code,
                            format!("{} must point at selected Codex home '{}'", key, target),
                        ));
                    }
                }
                for key in [
                    "NODE_REPL_NODE_MODULE_DIRS",
                    "NODE_REPL_NODE_PATH",
                    "CODEX_CLI_PATH",
                    "SKY_CUA_SERVICE_PATH",
                ] {
                    if let Some(raw) = env.get(key).and_then(Value::as_str) {
                        inspect_runtime_path(
                            &mut issues,
                            format!("mcp_servers.node_repl.env.{}", key),
                            raw,
                            target_home,
                        );
                    }
                }

                let computer_use_enabled = config
                    .get("plugins")
                    .and_then(Value::as_table)
                    .and_then(|plugins| plugins.get("computer-use@openai-bundled"))
                    .and_then(Value::as_table)
                    .and_then(|plugin| plugin.get("enabled"))
                    .and_then(Value::as_bool)
                    == Some(true);
                if computer_use_enabled && !env.contains_key("SKY_CUA_SERVICE_PATH") {
                    issues.push(issue(
                        "mcp_servers.node_repl.env.SKY_CUA_SERVICE_PATH",
                        "managed_mcp_service_missing",
                        "Computer Use is enabled but its target-local service path is missing",
                    ));
                }
            }
        }
        if let Some(computer) = servers
            .get(COMPUTER_USE)
            .filter(|value| computer_use_fingerprint(value))
        {
            let table = computer.as_table().expect("fingerprint requires table");
            for key in ["command", "cwd"] {
                if let Some(raw) = table.get(key).and_then(Value::as_str) {
                    inspect_runtime_path(
                        &mut issues,
                        format!("mcp_servers.computer-use.{}", key),
                        raw,
                        target_home,
                    );
                }
            }
        }
    }

    issues.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.code.cmp(&right.code))
    });
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup_files(config: &Path) -> Vec<PathBuf> {
        let parent = config.parent().unwrap();
        let prefix = format!(
            "{}.bak-agent-sync-",
            config.file_name().unwrap().to_string_lossy()
        );
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .collect()
    }

    #[test]
    fn explicitly_disabled_plugin_ids_are_narrow_and_sorted() {
        let raw = br#"
[plugins."slack@openai-curated"]
enabled = true
[plugins."documents@openai-primary-runtime"]
enabled = false
[plugins."other@team"]
enabled = false
[plugins."tool@team"]
enabled = true
"#;
        assert_eq!(
            explicitly_disabled_plugin_ids_from_bytes(raw).unwrap(),
            vec![
                "documents@openai-primary-runtime".to_string(),
                "other@team".to_string()
            ]
        );
    }

    #[test]
    fn rebind_updates_only_managed_paths_with_backup_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let source = dir.path().join("managed-source");
        fs::create_dir_all(&source).unwrap();
        let original = br#"
model = "gpt"
[marketplaces.openai-bundled]
source_type = "local"
source = "/old/.codex/bundled"
[marketplaces.team]
source_type = "git"
repository = "owner/repo"
"#;
        fs::write(&config, original).unwrap();
        let sources = BTreeMap::from([("openai-bundled".to_string(), source.clone())]);

        let changed = rebind_managed_marketplaces(&config, &sources).unwrap();
        assert_eq!(
            changed,
            vec!["marketplaces.openai-bundled.source".to_string()]
        );
        let updated = fs::read_to_string(&config).unwrap();
        assert!(updated.contains(source.to_string_lossy().as_ref()));
        assert!(updated.contains("repository = \"owner/repo\""));
        let backups = backup_files(&config);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), original);

        assert!(rebind_managed_marketplaces(&config, &sources)
            .unwrap()
            .is_empty());
        assert_eq!(backup_files(&config).len(), 1);
    }

    #[test]
    fn explicit_local_curated_table_is_removed_but_custom_state_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(
            &config,
            "[marketplaces.openai-curated]\nsource_type = 'local'\nsource = '/stale/plugins'\n[marketplaces.team]\nsource_type = 'git'\nsource = 'owner/team'\n",
        )
        .unwrap();

        assert_eq!(
            remove_explicit_curated_marketplace(&config).unwrap(),
            ["marketplaces.openai-curated"]
        );
        let updated = fs::read_to_string(&config).unwrap();
        assert!(!updated.contains("openai-curated"));
        assert!(updated.contains("marketplaces.team"));
        assert!(remove_explicit_curated_marketplace(&config)
            .unwrap()
            .is_empty());

        fs::write(
            &config,
            "[marketplaces.openai-curated]\nsource_type = 'git'\nsource = 'owner/spoof'\n",
        )
        .unwrap();
        assert!(remove_explicit_curated_marketplace(&config).is_err());
    }

    fn managed_default_config(runtime: &Path) -> String {
        format!(
            r#"
[mcp_servers.node_repl]
command = "{node_repl}"
args = []

[mcp_servers.node_repl.env]
CODEX_HOME = "/default/.codex"
NODE_REPL_TRUSTED_CODE_PATHS = "/default/.codex"
NODE_REPL_NODE_MODULE_DIRS = "{modules}"
NODE_REPL_NODE_PATH = "{node}"
CODEX_CLI_PATH = "{cli}"
SKY_CUA_SERVICE_PATH = "/default/.codex/plugins/cache/old/Codex Computer Use.app"

[mcp_servers.computer-use]
command = "./Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient"
args = ["mcp"]
cwd = "."
enabled = false
"#,
            node_repl = runtime.join("node_repl").display(),
            modules = runtime.join("node_modules").display(),
            node = runtime.join("node").display(),
            cli = runtime.join("codex").display(),
        )
    }

    fn seed_runtime(runtime: &Path) {
        fs::create_dir_all(runtime.join("node_modules")).unwrap();
        for name in ["node_repl", "node", "codex"] {
            fs::write(runtime.join(name), b"runtime").unwrap();
        }
    }

    #[test]
    fn managed_mcp_repair_uses_target_template_paths_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        let default_home = dir.path().join("default/.codex");
        let runtime = dir.path().join("runtime");
        seed_runtime(&runtime);
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        let app = target_home
            .join("plugins/cache/openai-bundled/computer-use/1.2.3/Codex Computer Use.app");
        let computer_use_binary = app.join(
            "Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
        );
        fs::create_dir_all(computer_use_binary.parent().unwrap()).unwrap();
        fs::write(&computer_use_binary, b"runtime").unwrap();
        let target_config = target_home.join("config.toml");
        let default_config = default_home.join("config.toml");
        fs::write(
            &target_config,
            "model = \"gpt\"\n[mcp_servers.user]\ncommand = \"custom\"\n",
        )
        .unwrap();
        fs::write(&default_config, managed_default_config(&runtime)).unwrap();

        let changed =
            repair_managed_mcp_from_default(&target_config, &default_config, &target_home).unwrap();
        assert_eq!(
            changed,
            vec![
                "mcp_servers.computer-use".to_string(),
                "mcp_servers.node_repl".to_string()
            ]
        );
        let repaired = fs::read_to_string(&target_config).unwrap();
        assert!(repaired.contains(&format!("CODEX_HOME = \"{}\"", target_home.display())));
        assert!(repaired.contains(app.to_string_lossy().as_ref()));
        assert!(repaired.contains("[mcp_servers.user]"));
        assert!(inspect_managed_config_inner(&target_config, &target_home, None).is_empty());
        assert_eq!(backup_files(&target_config).len(), 1);

        assert!(
            repair_managed_mcp_from_default(&target_config, &default_config, &target_home,)
                .unwrap()
                .is_empty()
        );
        assert_eq!(backup_files(&target_config).len(), 1);
    }

    #[test]
    fn managed_mcp_repair_preserves_same_name_custom_server() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        let default_home = dir.path().join("default/.codex");
        let runtime = dir.path().join("runtime");
        seed_runtime(&runtime);
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        let target_config = target_home.join("config.toml");
        let default_config = default_home.join("config.toml");
        fs::write(
            &target_config,
            "[mcp_servers.node_repl]\ncommand = \"/custom/node_repl\"\nargs = [\"custom\"]\n",
        )
        .unwrap();
        fs::write(&default_config, managed_default_config(&runtime)).unwrap();

        let changed =
            repair_managed_mcp_from_default(&target_config, &default_config, &target_home).unwrap();
        assert!(changed.is_empty());
        let repaired = fs::read_to_string(&target_config).unwrap();
        assert!(repaired.contains("command = \"/custom/node_repl\""));
    }

    #[test]
    fn managed_mcp_repair_rejects_missing_absolute_runtime_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        let default_home = dir.path().join("default/.codex");
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        let target_config = target_home.join("config.toml");
        let default_config = default_home.join("config.toml");
        let original = b"model = \"keep\"\n";
        fs::write(&target_config, original).unwrap();
        fs::write(
            &default_config,
            managed_default_config(&dir.path().join("missing-runtime")),
        )
        .unwrap();

        let error = repair_managed_mcp_from_default(&target_config, &default_config, &target_home)
            .unwrap_err();
        assert!(error.contains("missing target runtime path"), "{error}");
        assert_eq!(fs::read(&target_config).unwrap(), original);
        assert!(backup_files(&target_config).is_empty());
    }

    #[test]
    fn managed_mcp_repair_rejects_relative_runtime_paths_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        let default_home = dir.path().join("default/.codex");
        let runtime = dir.path().join("runtime");
        seed_runtime(&runtime);
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        let target_config = target_home.join("config.toml");
        let default_config = default_home.join("config.toml");
        let original = b"model = \"keep\"\n";
        fs::write(&target_config, original).unwrap();
        let relative = managed_default_config(&runtime).replace(
            runtime.join("node_repl").to_string_lossy().as_ref(),
            "node_repl",
        );
        fs::write(&default_config, relative).unwrap();

        let error = repair_managed_mcp_from_default(&target_config, &default_config, &target_home)
            .unwrap_err();

        assert!(error.contains("must be an absolute"), "{error}");
        assert_eq!(fs::read(&target_config).unwrap(), original);
        assert!(backup_files(&target_config).is_empty());
    }

    #[test]
    fn managed_mcp_repair_requires_enabled_computer_use_payload_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        let default_home = dir.path().join("default/.codex");
        let runtime = dir.path().join("runtime");
        seed_runtime(&runtime);
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        let target_config = target_home.join("config.toml");
        let default_config = default_home.join("config.toml");
        let original = br#"
model = "keep"
[plugins."computer-use@openai-bundled"]
enabled = true
"#;
        fs::write(&target_config, original).unwrap();
        fs::write(&default_config, managed_default_config(&runtime)).unwrap();

        let error = repair_managed_mcp_from_default(&target_config, &default_config, &target_home)
            .unwrap_err();
        assert!(
            error.contains("enabled but no Codex Computer Use.app payload exists"),
            "{error}"
        );
        assert_eq!(fs::read(&target_config).unwrap(), original);
        assert!(backup_files(&target_config).is_empty());
    }

    #[test]
    fn inspection_reports_wrong_home_and_missing_managed_paths() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("machine-b/.codex");
        let source_home = dir.path().join("machine-a/.codex");
        fs::create_dir_all(&target_home).unwrap();
        let config = target_home.join("config.toml");
        fs::write(
            &config,
            format!(
                r#"
[marketplaces.openai-bundled]
source_type = "local"
source = "{source}/.tmp/bundled-marketplaces/openai-bundled"

[plugins."computer-use@openai-bundled"]
enabled = true

[mcp_servers.node_repl]
command = "{missing}/node_repl"

[mcp_servers.node_repl.env]
CODEX_HOME = "{source}"
NODE_REPL_TRUSTED_CODE_PATHS = "{source}"
NODE_REPL_NODE_PATH = "{missing}/node"
"#,
                source = source_home.display(),
                missing = dir.path().join("missing-runtime").display(),
            ),
        )
        .unwrap();

        let issues = inspect_managed_config_inner(&config, &target_home, None);
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        for expected in [
            "managed_marketplace_wrong_home",
            "managed_mcp_home_mismatch",
            "managed_mcp_trusted_path_mismatch",
            "managed_mcp_service_missing",
            "managed_runtime_path_missing",
        ] {
            assert!(codes.contains(&expected), "missing {expected}: {issues:?}");
        }
    }

    #[test]
    fn inspection_turns_parse_failures_into_structured_issues() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(&config, "[broken").unwrap();
        let issues = inspect_managed_config_inner(&config, dir.path(), None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "config.toml");
        assert_eq!(issues[0].code, "config_invalid");
    }

    #[test]
    fn inspection_rejects_relative_managed_runtime_paths() {
        let dir = tempfile::tempdir().unwrap();
        let target_home = dir.path().join("target/.codex");
        fs::create_dir_all(&target_home).unwrap();
        let config = target_home.join("config.toml");
        fs::write(
            &config,
            format!(
                "[mcp_servers.node_repl]\ncommand = 'node_repl'\n[mcp_servers.node_repl.env]\nCODEX_HOME = {:?}\nNODE_REPL_TRUSTED_CODE_PATHS = {:?}\nNODE_REPL_NODE_PATH = 'node'\n",
                target_home.to_string_lossy(),
                target_home.to_string_lossy(),
            ),
        )
        .unwrap();

        let issues = inspect_managed_config_inner(&config, &target_home, None);

        assert!(issues
            .iter()
            .any(|issue| issue.code == "managed_runtime_path_invalid"));
    }

    #[test]
    fn managed_marketplace_sources_allow_selected_or_machine_default_home() {
        let default_home = dirs::home_dir().unwrap().join(".codex");
        let target_home = default_home.parent().unwrap().join("custom-profile/.codex");
        assert!(managed_marketplace_home_is_allowed(
            "openai-bundled",
            &target_home.join(".tmp/bundled-marketplaces/openai-bundled"),
            &target_home,
        ));
        assert!(managed_marketplace_home_is_allowed(
            "openai-bundled",
            &default_home.join(".tmp/bundled-marketplaces/openai-bundled"),
            &target_home,
        ));
        assert!(!managed_marketplace_home_is_allowed(
            "openai-bundled",
            &default_home
                .parent()
                .unwrap()
                .join("other/.codex/.tmp/bundled-marketplaces/openai-bundled"),
            &target_home,
        ));
        assert!(!managed_marketplace_home_is_allowed(
            "openai-bundled",
            Path::new("/tmp/unrelated-openai-bundled"),
            &target_home,
        ));
    }
}

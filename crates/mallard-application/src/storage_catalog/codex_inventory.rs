//! Codex Library inventory from the selected provider home's local files.
//!
//! Config entries and remote install markers identify local installation intent.
//! A valid cached manifest confirms local presence. Cache-only packages are not
//! promoted to installed resources. Account state and active cached versions are
//! not guessed. None of these observations changes the schema-1 portable intent.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::adapters::{MarketplaceObservation, PluginObservation, ProviderInventory};
use super::domain::ProviderKey;
use super::plugin_intent::{validate_canonical_plugin_id, validate_plugin_namespace};
use super::profiles::prospective_canonical;
use super::provider_state::{child_directories, read_optional_state_file, MAX_STATE_ENTRIES};

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    plugins: BTreeMap<String, PluginConfig>,
    #[serde(default)]
    marketplaces: BTreeMap<String, MarketplaceConfig>,
}

#[derive(Default, Deserialize)]
struct PluginConfig {
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct MarketplaceConfig {
    source_type: String,
    source: String,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

#[derive(Deserialize)]
struct RemoteInstallMarker {
    schema_version: u8,
    remote_plugin_id: String,
}

#[derive(Deserialize)]
struct PluginManifest {
    name: String,
    version: Option<String>,
}

struct Reader<'a> {
    home: &'a Path,
    remaining_bytes: usize,
    remaining_entries: usize,
}

impl Reader<'_> {
    fn read(&mut self, parts: &[&str]) -> Result<Option<Vec<u8>>, String> {
        let bytes = read_optional_state_file(self.home, parts)?;
        if let Some(bytes) = &bytes {
            self.remaining_bytes = self
                .remaining_bytes
                .checked_sub(bytes.len())
                .ok_or("Codex plugin inventory exceeds its total read limit")?;
        }
        Ok(bytes)
    }

    fn directories(&mut self, parts: &[&str]) -> Result<Vec<String>, String> {
        child_directories(self.home, parts, &mut self.remaining_entries)
    }
}

pub(super) fn discover(home: &Path) -> Result<ProviderInventory, String> {
    let mut reader = Reader {
        home,
        remaining_bytes: 32 * 1024 * 1024,
        remaining_entries: MAX_STATE_ENTRIES,
    };
    let config = match reader.read(&["config.toml"])? {
        Some(bytes) => {
            let text = std::str::from_utf8(&bytes).map_err(|_| "Codex config is not UTF-8")?;
            // Do not return TOML parser diagnostics: they can contain config secrets.
            toml::from_str::<Config>(text)
                .map_err(|_| "invalid Codex plugin or marketplace config")?
        }
        None => Config::default(),
    };
    if config.plugins.len() + config.marketplaces.len() > MAX_STATE_ENTRIES {
        return Err("Codex config exceeds its inventory entry limit".to_string());
    }
    let mut inventory = ProviderInventory::default();
    for (name, marketplace) in config.marketplaces {
        validate_plugin_namespace("marketplace", &name)?;
        let root = if marketplace.source_type == "local" {
            let source = Path::new(&marketplace.source);
            if !source.is_absolute() {
                return Err("Codex local marketplace path must be absolute".to_string());
            }
            Some(prospective_canonical(source)?)
        } else {
            None
        };
        inventory.marketplaces.push(MarketplaceObservation {
            name,
            source_type: marketplace.source_type,
            source: marketplace.source,
            git_ref: marketplace.git_ref,
            root,
        });
    }
    for plugin_id in config.plugins.keys() {
        validate_canonical_plugin_id("configured plugin", plugin_id)?;
    }
    let mut candidates = config.plugins;
    for market in reader.directories(&["plugins", "cache"])? {
        validate_plugin_namespace("cached marketplace", &market)?;
        for name in reader.directories(&["plugins", "cache", &market])? {
            let plugin_id = format!("{name}@{market}");
            validate_canonical_plugin_id("cached plugin", &plugin_id)?;
            let Some(bytes) = reader.read(&[
                "plugins",
                "cache",
                &market,
                &name,
                ".codex-remote-plugin-install.json",
            ])?
            else {
                continue;
            };
            let marker: RemoteInstallMarker = serde_json::from_slice(&bytes)
                .map_err(|_| "invalid Codex remote plugin install marker")?;
            if marker.schema_version != 1
                || marker.remote_plugin_id.trim().is_empty()
                || marker.remote_plugin_id.len() > 512
            {
                return Err("unsupported Codex remote plugin install marker".to_string());
            }
            candidates.entry(plugin_id).or_default();
        }
    }
    if candidates.len() > MAX_STATE_ENTRIES {
        return Err("too many Codex plugin records".to_string());
    }
    for (plugin_id, settings) in candidates {
        let (name, market) = plugin_id
            .split_once('@')
            .expect("validated plugin identity");
        let mut versions = Vec::new();
        for version in reader.directories(&["plugins", "cache", market, name])? {
            if version.len() > 256
                || !version
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".+_-".contains(&c))
            {
                return Err("invalid Codex cached plugin version directory".to_string());
            }
            let Some(bytes) = reader.read(&[
                "plugins",
                "cache",
                market,
                name,
                &version,
                ".codex-plugin",
                "plugin.json",
            ])?
            else {
                continue;
            };
            let manifest: PluginManifest = serde_json::from_slice(&bytes)
                .map_err(|_| "invalid Codex cached plugin manifest")?;
            if manifest.name != name {
                return Err("Codex cached plugin manifest identity mismatch".to_string());
            }
            versions.push(manifest.version);
        }
        if versions.is_empty() {
            inventory.warnings.push(format!("Plugin '{plugin_id}' has installation metadata but no readable cached manifest; it was not listed as installed"));
            continue;
        }
        if settings.enabled.is_none() {
            inventory.warnings.push(format!("Plugin '{plugin_id}' is present locally; its enablement is not recorded in this home's config"));
        }
        let observed_version = if versions.len() == 1 {
            versions.pop().flatten()
        } else {
            None
        };
        inventory.plugins.push(PluginObservation {
            provider: ProviderKey::parse("codex").expect("static provider"),
            namespace: market.to_string(),
            install_root: Some(home.join("plugins/cache").join(market).join(name)),
            plugin_id,
            installed: true,
            enabled: settings.enabled,
            observed_version,
        });
    }
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(home: &Path, path: &str, content: &str) {
        let path = home.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn cached(home: &Path, name: &str) {
        write(
            home,
            &format!("plugins/cache/acme/{name}/1.0/.codex-plugin/plugin.json"),
            &format!(r#"{{"name":"{name}","version":"1.0"}}"#),
        );
    }

    #[test]
    fn local_inventory_distinguishes_installation_from_cache_and_keeps_unknown_enablement() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write(home, "config.toml", "[plugins.\"configured@acme\"]\nenabled=false\n[plugins.\"missing@acme\"]\nenabled=true\n[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n");
        for name in ["configured", "remote", "cache-only"] {
            cached(home, name);
        }
        write(
            home,
            "plugins/cache/acme/remote/.codex-remote-plugin-install.json",
            r#"{"schema_version":1,"remote_plugin_id":"fixture"}"#,
        );
        let result = discover(home).unwrap();
        assert_eq!(result.plugins.len(), 2);
        assert_eq!(result.plugins[0].plugin_id, "configured@acme");
        assert_eq!(result.plugins[0].enabled, Some(false));
        assert_eq!(result.plugins[1].enabled, None);
        assert_eq!(result.plugins[1].observed_version.as_deref(), Some("1.0"));
        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.marketplaces[0].source_type, "git");
        assert_eq!(result.ownership_roots().len(), 2);
    }

    #[test]
    fn empty_home_needs_no_config_or_cli() {
        assert!(discover(tempfile::tempdir().unwrap().path())
            .unwrap()
            .plugins
            .is_empty());
    }

    #[test]
    fn malformed_and_oversized_state_fail_without_echoing_config_secrets() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "config.toml", "token = SECRET-CANARY[");
        let error = discover(temp.path()).unwrap_err();
        assert!(!error.contains("SECRET-CANARY"));
        write(
            temp.path(),
            "config.toml",
            &" ".repeat(super::super::provider_state::MAX_PLUGIN_STATE_BYTES as usize + 1),
        );
        assert!(discover(temp.path()).is_err());
    }

    #[test]
    fn mismatched_manifests_and_unknown_markers_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path(),
            "config.toml",
            "[plugins.\"tools@acme\"]\nenabled=true\n",
        );
        write(
            temp.path(),
            "plugins/cache/acme/tools/1/.codex-plugin/plugin.json",
            r#"{"name":"wrong"}"#,
        );
        assert!(discover(temp.path())
            .unwrap_err()
            .contains("identity mismatch"));
        write(
            temp.path(),
            "plugins/cache/acme/tools/.codex-remote-plugin-install.json",
            r#"{"schema_version":2,"remote_plugin_id":"fixture"}"#,
        );
        assert!(discover(temp.path()).unwrap_err().contains("unsupported"));
    }

    #[test]
    #[cfg(unix)]
    fn state_reads_reject_parent_and_leaf_symlinks_and_special_files() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        write(&outside, "config.toml", "");
        symlink(outside.join("config.toml"), home.join("config.toml")).unwrap();
        assert!(discover(&home).is_err());
        std::fs::remove_file(home.join("config.toml")).unwrap();
        symlink(&outside, home.join("plugins")).unwrap();
        assert!(discover(&home).is_err());
        std::fs::remove_file(home.join("plugins")).unwrap();
        let fifo = std::ffi::CString::new(home.join("config.toml").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(discover(&home).is_err());
    }

    #[test]
    fn directory_enumeration_has_a_scan_wide_entry_limit() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir_all(temp.path().join("plugins").join(name)).unwrap();
        }
        let mut remaining = 1;
        assert!(child_directories(temp.path(), &["plugins"], &mut remaining).is_err());
    }
}

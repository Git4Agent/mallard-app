//! What an installed plugin's own manifest says about it, for the plugin's
//! page on Mallard Cloud. A plugin intent stores what to install, never what
//! the plugin is; after a push, the desktop reads the installed copy's
//! manifest and sends a closed set of display fields with the commit
//! (`PUT /v1/repositories/{id}/listings/{commit_id}`). The service reads the
//! plugin's GitHub source first and keeps this as the fallback, which is the
//! only one a plugin from a catalog the service cannot read (OpenAI's own) has.
//!
//! Only these fields leave the machine: display name, developer, description,
//! category, and the website, privacy and terms links (https only). Never a
//! path, a version, the `repository` field, or anything else in the file.
//! The file is read through the provider-state reader: relative to the
//! canonical home, bounded, and without following links.

use std::path::{Component, Path};

use mallard_infrastructure::managed_cloud::client::PluginListing;
use serde_json::Value;

use super::adapters::ProviderInventory;
use super::provider_state::read_optional_state_file;

const NAME_CHARS: usize = 80;
const CATEGORY_CHARS: usize = 40;
const DESCRIPTION_CHARS: usize = 350;
const URL_CHARS: usize = 512;

/// One line of plain text, capped by character: control characters and runs
/// of space become one space; empty is absent.
fn line(value: &Value, cap: usize) -> Option<String> {
    let text = value.as_str()?;
    let mut tidy = String::new();
    for word in text
        .split(|ch: char| ch.is_control() || ch.is_whitespace())
        .filter(|word| !word.is_empty())
    {
        if !tidy.is_empty() {
            tidy.push(' ');
        }
        tidy.push_str(word);
    }
    if tidy.is_empty() {
        return None;
    }
    if tidy.chars().count() <= cap {
        return Some(tidy);
    }
    let cut: String = tidy.chars().take(cap - 1).collect();
    Some(format!("{}\u{2026}", cut.trim_end()))
}

/// An https link with a host and no credentials, in plain URL characters;
/// anything else is absent rather than sent for the service to refuse.
fn https(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    let rest = text.strip_prefix("https://")?;
    if text.len() > URL_CHARS
        || !text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(ch))
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(text.to_owned())
}

/// The allowlisted fields of a plugin's own manifest. Codex keeps its display
/// fields under `interface`; Claude's manifest has a description and an
/// author. `None` when it says none of them.
pub(super) fn listing_from_manifest(manifest: &Value) -> Option<PluginListing> {
    let face = &manifest["interface"];
    let author = match &manifest["author"] {
        Value::Object(fields) => fields.get("name").cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    let listing = PluginListing {
        display_name: line(&face["displayName"], NAME_CHARS),
        developer: line(&face["developerName"], NAME_CHARS).or_else(|| line(&author, NAME_CHARS)),
        description: line(&manifest["description"], DESCRIPTION_CHARS)
            .or_else(|| line(&face["shortDescription"], DESCRIPTION_CHARS)),
        category: line(&face["category"], CATEGORY_CHARS),
        website_url: https(&face["websiteURL"]).or_else(|| https(&manifest["homepage"])),
        privacy_url: https(&face["privacyPolicyURL"]),
        terms_url: https(&face["termsOfServiceURL"]),
    };
    (listing != PluginListing::default()).then_some(listing)
}

/// The installed copy's manifest, as the provider keeps it:
/// Codex `plugins/cache/<marketplace>/<name>/<version>/.codex-plugin/plugin.json`
/// for the one version the inventory observed; Claude
/// `<installPath>/.claude-plugin/plugin.json` when the recorded install path
/// lies inside the canonical home. Anything unreadable is simply absent: the
/// listing is display only and never blocks or fails a push.
pub(super) fn installed_listing(
    provider: &str,
    home: &Path,
    inventory: &ProviderInventory,
    plugin_id: &str,
) -> Option<PluginListing> {
    let observation = inventory.observation(plugin_id)?;
    if !observation.installed {
        return None;
    }
    let bytes = match provider {
        "codex" => {
            let (name, marketplace) = plugin_id.split_once('@')?;
            let version = observation.observed_version.as_deref()?;
            read_optional_state_file(
                home,
                &[
                    "plugins",
                    "cache",
                    marketplace,
                    name,
                    version,
                    ".codex-plugin",
                    "plugin.json",
                ],
            )
            .ok()??
        }
        "claude" => {
            let relative = observation
                .install_root
                .as_deref()?
                .strip_prefix(home)
                .ok()?;
            let mut parts = Vec::new();
            for component in relative.components() {
                match component {
                    Component::Normal(part) => parts.push(part.to_str()?),
                    _ => return None,
                }
            }
            parts.extend([".claude-plugin", "plugin.json"]);
            read_optional_state_file(home, &parts).ok()??
        }
        _ => return None,
    };
    listing_from_manifest(&serde_json::from_slice(&bytes).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_catalog::adapters::PluginObservation;
    use crate::storage_catalog::domain::ProviderKey;
    use serde_json::json;

    #[test]
    fn reads_only_the_allowlisted_display_fields() {
        let listing = listing_from_manifest(&json!({
            "name": "work-pets",
            "version": "0.1.6",
            "description": "Create and manage\n animated companion pets.\u{0}",
            "author": {"name": "OpenAI", "email": "support@openai.com"},
            "homepage": "https://openai.com/",
            "repository": "https://github.com/openai/openai/tree/master/secret",
            "interface": {
                "displayName": "Pets",
                "developerName": "OpenAI",
                "category": "Productivity",
                "privacyPolicyURL": "https://openai.com/policies/row-privacy-policy/",
                "termsOfServiceURL": "http://insecure.example/terms",
                "logo": "./assets/egg.svg",
            },
        }))
        .unwrap();
        assert_eq!(
            listing,
            PluginListing {
                display_name: Some("Pets".into()),
                developer: Some("OpenAI".into()),
                description: Some("Create and manage animated companion pets.".into()),
                category: Some("Productivity".into()),
                website_url: Some("https://openai.com/".into()),
                privacy_url: Some("https://openai.com/policies/row-privacy-policy/".into()),
                terms_url: None,
            }
        );
        let sent = serde_json::to_string(&listing).unwrap();
        assert!(
            !sent.contains("secret") && !sent.contains("egg.svg") && !sent.contains("support@")
        );
    }

    #[test]
    fn takes_claude_author_and_caps_long_text() {
        let long = "word ".repeat(200);
        let listing = listing_from_manifest(&json!({
            "name": "ponytail",
            "description": long,
            "author": "Dietrich Gebert",
            "homepage": "https://user:secret@example.com/",
        }))
        .unwrap();
        assert_eq!(listing.developer.as_deref(), Some("Dietrich Gebert"));
        assert_eq!(
            listing.description.as_ref().unwrap().chars().count(),
            DESCRIPTION_CHARS
        );
        assert!(listing.description.unwrap().ends_with('\u{2026}'));
        assert_eq!(listing.website_url, None);
        assert_eq!(
            listing_from_manifest(&json!({"name": "bare", "version": "1"})),
            None
        );
    }

    fn observation(
        provider: &str,
        plugin_id: &str,
        version: Option<&str>,
        root: Option<std::path::PathBuf>,
    ) -> ProviderInventory {
        ProviderInventory {
            plugins: vec![PluginObservation {
                provider: ProviderKey::parse(provider).unwrap(),
                namespace: plugin_id.split_once('@').unwrap().1.to_string(),
                plugin_id: plugin_id.to_string(),
                installed: true,
                enabled: Some(true),
                observed_version: version.map(str::to_string),
                install_root: root,
            }],
            ..ProviderInventory::default()
        }
    }

    #[test]
    fn reads_the_installed_copy_inside_the_home_only() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let codex = home.join("plugins/cache/acme/tools/1.2.0/.codex-plugin");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("plugin.json"),
            r#"{"name":"tools","description":"Tools for acme."}"#,
        )
        .unwrap();
        let inventory = observation(
            "codex",
            "tools@acme",
            Some("1.2.0"),
            Some(home.join("plugins/cache/acme/tools")),
        );
        assert_eq!(
            installed_listing("codex", &home, &inventory, "tools@acme")
                .and_then(|listing| listing.description),
            Some("Tools for acme.".to_string())
        );
        // No single observed version: nothing is guessed.
        let unknown = observation(
            "codex",
            "tools@acme",
            None,
            Some(home.join("plugins/cache/acme/tools")),
        );
        assert_eq!(
            installed_listing("codex", &home, &unknown, "tools@acme"),
            None
        );

        let claude = home.join("plugins/cache/acme/tools/2/.claude-plugin");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(
            claude.join("plugin.json"),
            r#"{"name":"tools","author":{"name":"Acme"}}"#,
        )
        .unwrap();
        let inside = observation(
            "claude",
            "tools@acme",
            Some("2"),
            Some(home.join("plugins/cache/acme/tools/2")),
        );
        assert_eq!(
            installed_listing("claude", &home, &inside, "tools@acme")
                .and_then(|listing| listing.developer),
            Some("Acme".to_string())
        );
        // An install path outside the home, or one that climbs out of it, is never read.
        let outside = tempfile::tempdir().unwrap();
        let elsewhere = observation(
            "claude",
            "tools@acme",
            Some("2"),
            Some(outside.path().to_path_buf()),
        );
        assert_eq!(
            installed_listing("claude", &home, &elsewhere, "tools@acme"),
            None
        );
        let climbing = observation(
            "claude",
            "tools@acme",
            Some("2"),
            Some(home.join("plugins/../../etc")),
        );
        assert_eq!(
            installed_listing("claude", &home, &climbing, "tools@acme"),
            None
        );
    }
}

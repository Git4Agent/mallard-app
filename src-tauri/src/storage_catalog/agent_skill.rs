//! The `agentskills.io.skill` format codec, version 1: bounded Agent Skill
//! directory capture, typed identity and descriptor, semantic rename,
//! materialization, and validation.
//!
//! Capture preserves the exact validated tree; it never rewrites frontmatter
//! to convert a skill between providers. The one deliberate transform is the
//! reviewed copy rename, which changes the directory name and the `SKILL.md`
//! frontmatter `name` together so the copy stays a valid, independently
//! addressable Agent Skill.
//!
//! The portable identity keeps the target's root class (`provider-home`,
//! `shared-user`) so equal names across two roots stay distinct resources;
//! which machine-local directory a root class maps to is the adapter's
//! business, never this codec's.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::codec::{
    content_warnings, read_directory_tree, CapturedFile, CapturedResource, CatalogFormatCodec,
    TreeLimits,
};
use super::domain::{
    validate_component_name, validate_root_class, BoundedPortableObject, CatalogResourceEnvelope,
};

pub const FORMAT_KEY: &str = "agentskills.io.skill";
pub const KIND_KEY: &str = "agent_skill";
pub const MAX_SKILL_FILES: usize = 256;
pub const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_SKILL_TREE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;

const LIMITS: TreeLimits = TreeLimits {
    max_files: MAX_SKILL_FILES,
    max_file_bytes: MAX_SKILL_FILE_BYTES,
    max_total_bytes: MAX_SKILL_TREE_BYTES,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillIdentity {
    pub root: String,
    pub effective_name: String,
    pub install_directory: String,
}

/// Version 1 carries nothing beyond the identity and the exact tree.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillDescriptor {}

pub fn validate_skill_name(label: &str, value: &str) -> Result<(), String> {
    validate_component_name(label, value, 128)
}

/// The `name:` declaration in SKILL.md's leading YAML frontmatter block.
pub fn declared_skill_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// The reviewed copy transform over the tree: a rewritten frontmatter `name`
/// when one is declared, preserving every other byte. A skill without a
/// declaration keeps its bytes; the directory rename is its whole identity
/// change.
pub fn rename_skill_tree(
    files: &BTreeMap<String, CapturedFile>,
    new_name: &str,
) -> Result<BTreeMap<String, CapturedFile>, String> {
    validate_skill_name("fork copy name", new_name)?;
    let mut renamed = files.clone();
    let Some(skill_md) = renamed.get_mut("SKILL.md") else {
        return Err("skill copy source has no SKILL.md".to_string());
    };
    let contents = std::str::from_utf8(&skill_md.bytes)
        .map_err(|_| "SKILL.md is not UTF-8; cannot rewrite its name".to_string())?;
    if declared_skill_name(contents).is_some() {
        let mut rewritten = String::with_capacity(contents.len());
        let mut inside_frontmatter = false;
        let mut frontmatter_closed = false;
        let mut replaced = false;
        for (index, line) in contents.lines().enumerate() {
            let trimmed = line.trim();
            if index == 0 && trimmed == "---" {
                inside_frontmatter = true;
            } else if inside_frontmatter && !frontmatter_closed && trimmed == "---" {
                frontmatter_closed = true;
            } else if inside_frontmatter
                && !frontmatter_closed
                && !replaced
                && trimmed.starts_with("name:")
            {
                let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
                rewritten.push_str(&format!("{}name: {}\n", indent, new_name));
                replaced = true;
                continue;
            }
            rewritten.push_str(line);
            rewritten.push('\n');
        }
        if !contents.ends_with('\n') {
            rewritten.pop();
        }
        skill_md.bytes = rewritten.into_bytes();
    }
    Ok(renamed)
}

fn capture_from_tree(
    root_class: &str,
    install_directory: &str,
    files: BTreeMap<String, CapturedFile>,
) -> Result<CapturedResource, String> {
    validate_root_class("skill root class", root_class)?;
    validate_skill_name("skill install directory", install_directory)?;
    let skill_md = files
        .get("SKILL.md")
        .ok_or_else(|| "no SKILL.md declaration; not a recognizable skill".to_string())?;
    if skill_md.bytes.len() as u64 > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md exceeds the supported size".to_string());
    }
    let declared_name = std::str::from_utf8(&skill_md.bytes)
        .ok()
        .and_then(declared_skill_name);
    if let Some(declared) = &declared_name {
        validate_skill_name("declared skill name", declared)?;
    }
    let effective_name = declared_name.unwrap_or_else(|| install_directory.to_string());
    let identity = AgentSkillIdentity {
        root: root_class.to_string(),
        effective_name: effective_name.clone(),
        install_directory: install_directory.to_string(),
    };
    let warnings = content_warnings(&files);
    Ok(CapturedResource {
        identity: BoundedPortableObject::encode(&identity)?,
        descriptor: BoundedPortableObject::encode(&AgentSkillDescriptor::default())?,
        files,
        display_name: effective_name,
        warnings,
        notes: Vec::new(),
    })
}

fn decode_identity(envelope: &CatalogResourceEnvelope) -> Result<AgentSkillIdentity, String> {
    let identity: AgentSkillIdentity = envelope
        .identity
        .decode()
        .map_err(|error| format!("skill identity: {}", error))?;
    validate_root_class("skill root class", &identity.root)?;
    validate_skill_name("skill effective name", &identity.effective_name)?;
    validate_skill_name("skill install directory", &identity.install_directory)?;
    Ok(identity)
}

fn root_label(root: &str) -> String {
    match root {
        "provider-home" => "Provider home".to_string(),
        "shared-user" => "Shared user".to_string(),
        other => other.to_string(),
    }
}

pub struct AgentSkillCodec;

impl CatalogFormatCodec for AgentSkillCodec {
    fn validate(&self, envelope: &CatalogResourceEnvelope) -> Result<(), String> {
        let identity = decode_identity(envelope)?;
        let _descriptor: AgentSkillDescriptor = envelope
            .descriptor
            .decode()
            .map_err(|error| format!("skill descriptor: {}", error))?;
        if envelope.files.is_empty() || envelope.files.len() > MAX_SKILL_FILES {
            return Err(format!(
                "skill '{}' must own between 1 and {} files",
                identity.effective_name, MAX_SKILL_FILES
            ));
        }
        if envelope.file("SKILL.md").is_none() {
            return Err(format!(
                "skill '{}' has no SKILL.md declaration",
                identity.effective_name
            ));
        }
        let mut total: u64 = 0;
        for file in &envelope.files {
            file.validate(&format!("skill file '{}'", file.path), MAX_SKILL_FILE_BYTES)?;
            total = total.saturating_add(file.size);
        }
        if total > MAX_SKILL_TREE_BYTES {
            return Err(format!(
                "skill '{}' exceeds the skill tree byte limit",
                identity.effective_name
            ));
        }
        Ok(())
    }

    fn display_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(decode_identity(envelope)?.effective_name)
    }

    fn member_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(decode_identity(envelope)?.install_directory)
    }

    fn root_class(&self, envelope: &CatalogResourceEnvelope) -> Result<Option<String>, String> {
        Ok(Some(decode_identity(envelope)?.root))
    }

    fn resource_label(&self, envelope: &CatalogResourceEnvelope) -> Option<String> {
        decode_identity(envelope)
            .ok()
            .map(|identity| root_label(&identity.root))
    }

    fn collision_key(&self, captured: &CapturedResource) -> Option<String> {
        Some(captured.display_name.to_lowercase())
    }

    fn capture_directory(
        &self,
        directory: &Path,
        root_class: &str,
    ) -> Result<CapturedResource, String> {
        let install_directory = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "skill directory name is not UTF-8".to_string())?;
        let files = read_directory_tree(directory, &LIMITS)
            .map_err(|error| error.replace("payload", "skill"))?;
        capture_from_tree(root_class, install_directory, files)
    }

    fn rename(
        &self,
        captured: &CapturedResource,
        new_name: &str,
    ) -> Result<CapturedResource, String> {
        let identity: AgentSkillIdentity = captured
            .identity
            .decode()
            .map_err(|error| format!("skill identity: {}", error))?;
        let files = rename_skill_tree(&captured.files, new_name)?;
        capture_from_tree(&identity.root, new_name, files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_catalog::domain::CatalogCoordinate;
    use std::fs;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn coordinate() -> CatalogCoordinate {
        CatalogCoordinate::new("codex", KIND_KEY, FORMAT_KEY, 1).unwrap()
    }

    #[test]
    fn capture_declares_identity_from_the_root_class_and_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review-dir");
        write(&dir.join("SKILL.md"), b"---\nname: review\n---\nBody\n");
        write(&dir.join("scripts/run.sh"), b"#!/bin/sh\necho ok\n");
        let captured = AgentSkillCodec
            .capture_directory(&dir, "provider-home")
            .unwrap();
        assert_eq!(captured.display_name, "review");
        let identity: AgentSkillIdentity = captured.identity.decode().unwrap();
        assert_eq!(identity.root, "provider-home");
        assert_eq!(identity.install_directory, "review-dir");
        assert_eq!(captured.warnings[0].code, "executable_content");
        assert_eq!(
            AgentSkillCodec.collision_key(&captured).as_deref(),
            Some("review")
        );

        let envelope = captured.to_envelope(&coordinate());
        AgentSkillCodec.validate(&envelope).unwrap();
        assert_eq!(AgentSkillCodec.display_name(&envelope).unwrap(), "review");
        assert_eq!(
            AgentSkillCodec.member_name(&envelope).unwrap(),
            "review-dir"
        );
        assert_eq!(
            AgentSkillCodec.root_class(&envelope).unwrap().as_deref(),
            Some("provider-home")
        );
        assert_eq!(
            AgentSkillCodec.resource_label(&envelope).as_deref(),
            Some("Provider home")
        );

        // The same tree under another root class is another resource.
        let shared = AgentSkillCodec
            .capture_directory(&dir, "shared-user")
            .unwrap()
            .to_envelope(&coordinate());
        assert_ne!(shared.resource_id, envelope.resource_id);
    }

    #[test]
    fn typed_validation_rejects_unknown_fields_and_missing_declarations() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review");
        write(&dir.join("SKILL.md"), b"---\nname: review\n---\n");
        let envelope = AgentSkillCodec
            .capture_directory(&dir, "provider-home")
            .unwrap()
            .to_envelope(&coordinate());
        let mut extra = envelope.clone();
        extra.identity = BoundedPortableObject::from_json(&serde_json::json!({
            "root": "provider-home",
            "effective_name": "review",
            "install_directory": "review",
            "surprise": true
        }))
        .unwrap();
        assert!(AgentSkillCodec.validate(&extra).is_err());
        let mut descriptor = envelope.clone();
        descriptor.descriptor =
            BoundedPortableObject::from_json(&serde_json::json!({ "theme": "pond" })).unwrap();
        assert!(AgentSkillCodec.validate(&descriptor).is_err());
        let mut no_declaration = envelope.clone();
        no_declaration.files[0].path = "README.md".to_string();
        assert!(AgentSkillCodec
            .validate(&no_declaration)
            .unwrap_err()
            .contains("no SKILL.md"));

        let empty = temp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(AgentSkillCodec
            .capture_directory(&empty, "provider-home")
            .unwrap_err()
            .contains("not a recognizable skill"));
        assert!(AgentSkillCodec
            .capture_directory(&dir, "Not A Class")
            .is_err());
    }

    #[test]
    fn rename_rewrites_directory_and_frontmatter_together() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review");
        write(
            &dir.join("SKILL.md"),
            b"---\nname: review\ndescription: keep\n---\nBody stays byte-identical.\n",
        );
        write(&dir.join("extra.md"), b"unchanged");
        let captured = AgentSkillCodec
            .capture_directory(&dir, "provider-home")
            .unwrap();
        let renamed = AgentSkillCodec
            .rename(&captured, "review-local-copy")
            .unwrap();
        let skill_md = String::from_utf8(renamed.files["SKILL.md"].bytes.clone()).unwrap();
        assert!(skill_md.contains("name: review-local-copy"));
        assert!(skill_md.contains("description: keep"));
        assert!(skill_md.contains("Body stays byte-identical."));
        assert_eq!(renamed.files["extra.md"].bytes, b"unchanged");
        assert_eq!(renamed.display_name, "review-local-copy");
        let identity: AgentSkillIdentity = renamed.identity.decode().unwrap();
        assert_eq!(identity.install_directory, "review-local-copy");
        assert_eq!(identity.root, "provider-home");
        assert_ne!(
            renamed.to_envelope(&coordinate()).resource_id,
            captured.to_envelope(&coordinate()).resource_id
        );
    }

    #[test]
    fn skills_without_a_declaration_keep_their_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("legacy");
        write(&dir.join("SKILL.md"), b"No frontmatter here.\n");
        let captured = AgentSkillCodec
            .capture_directory(&dir, "provider-home")
            .unwrap();
        assert_eq!(captured.display_name, "legacy");
        let renamed = AgentSkillCodec
            .rename(&captured, "legacy-local-copy")
            .unwrap();
        assert_eq!(renamed.files["SKILL.md"].bytes, b"No frontmatter here.\n");
        assert_eq!(renamed.display_name, "legacy-local-copy");
    }

    #[test]
    fn materialize_and_recapture_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("skill");
        write(&dir.join("SKILL.md"), b"---\nname: skill\n---\n");
        write(&dir.join("deep/file.txt"), b"content");
        let captured = AgentSkillCodec
            .capture_directory(&dir, "provider-home")
            .unwrap();
        let envelope = captured.to_envelope(&coordinate());
        let stage = temp.path().join("stage").join("skill");
        fs::create_dir_all(&stage).unwrap();
        AgentSkillCodec
            .materialize(&stage, &envelope, &captured.files)
            .unwrap();
        let restaged = AgentSkillCodec
            .capture_directory(&stage, "provider-home")
            .unwrap()
            .to_envelope(&coordinate());
        assert_eq!(restaged, envelope);
    }
}

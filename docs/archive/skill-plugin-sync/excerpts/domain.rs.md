# Excerpts: src-tauri/src/project_sync/domain.rs

Skill/plugin-sync code removed from `domain.rs`. Line numbers are pre-removal.

## validate_skill_name (lines 516–536)
```rust
/// Validate either the effective name or install-directory component of a
/// global custom skill. They are separate values, but both use the providers'
/// current portable single-component grammar.
pub fn validate_skill_name(label: &str, value: &str) -> Result<(), String> {
    let valid_start = value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if !valid_start
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || value == "."
        || value == ".."
        || value.ends_with('.')
    {
        return Err(format!("invalid {}: '{}'", label, value));
    }
    Ok(())
}
```

## ResourceKind::{StandaloneSkill, Plugin} (lines 633–644)
```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    CodexConversation,
    ClaudeConversation,
    ProjectContentFile,
    ProjectContentDirectory,
    ProjectMemory,
    StandaloneSkill,
    Plugin,
    Requirement,
}
```

## ResourceScope::Dependency (lines 646–653)
```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceScope {
    Project,
    ProviderState,
    Dependency,
    Requirement,
}
```

## Provenance::{StandaloneSnapshot, Plugin} + validation (lines 666–710)
```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    ProjectLocal {
        relative_path: String,
    },
    StandaloneSnapshot {
        stable_key: String,
        source_digest: String,
    },
    Plugin {
        provider: Provider,
        plugin_id: String,
    },
    Unknown,
}

impl Provenance {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::ProjectLocal { relative_path } => {
                if relative_path.is_empty()
                    || Path::new(relative_path).is_absolute()
                    || Path::new(relative_path).components().any(|component| {
                        matches!(component, Component::CurDir | Component::ParentDir)
                    })
                {
                    return Err("invalid project-local provenance path".to_string());
                }
            }
            Self::StandaloneSnapshot {
                stable_key,
                source_digest,
            } => {
                validate_display_text("standalone skill stable key", stable_key, false)?;
                validate_sha256("standalone skill source digest", source_digest)?;
            }
            Self::Plugin { plugin_id, .. } => {
                validate_display_text("plugin id", plugin_id, false)?;
            }
            Self::Unknown => {}
        }
        Ok(())
    }
}
```

## BundleManifest::validate custom-skill descriptor checks (lines 3448–3462)
```rust
            if descriptor.kind == ResourceKind::StandaloneSkill {
                if descriptor.provider.is_none() {
                    return Err(format!("custom skill '{}' lacks its provider", id));
                }
                let effective_name = descriptor
                    .metadata
                    .get("effective_name")
                    .ok_or_else(|| format!("custom skill '{}' lacks its effective name", id))?;
                let install_dir_name = descriptor
                    .metadata
                    .get("install_dir_name")
                    .ok_or_else(|| format!("custom skill '{}' lacks its install directory", id))?;
                validate_skill_name("custom skill effective name", effective_name)?;
                validate_skill_name("custom skill install directory", install_dir_name)?;
            }
```

## RestoreActionType/RestoreActionKind skill arms (lines 3856–3960, selected)
```rust
pub enum RestoreActionType {
    WriteFile,
    WriteProjectFile,
    EnsureProjectDirectory,
    DeleteProjectFile,
    DeleteProjectDirectory,
    MergeFile,
    MaterializeConversation,
    InstallCustomSkill,
    OverwriteCustomSkill,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestoreActionKind {
    WriteFile {
        logical_path: LogicalPath,
    },
    WriteProjectFile {
        logical_path: LogicalPath,
        mode: u32,
        source_mtime: u64,
    },
    EnsureProjectDirectory {
        logical_path: LogicalPath,
        mode: u32,
        source_mtime: u64,
    },
    DeleteProjectFile {
        logical_path: LogicalPath,
        last_sha256: String,
    },
    DeleteProjectDirectory {
        logical_path: LogicalPath,
    },
    MergeFile {
        logical_path: LogicalPath,
    },
    MaterializeConversation {
        provider: Provider,
        logical_path: LogicalPath,
    },
    /// Materialize a complete custom-skill directory into the mapped provider
    /// home's `skills/<name>`. The whole directory is one unit: staging,
    /// verification, and rename happen inside a single approved action, never
    /// as independently selectable file writes.
    InstallCustomSkill {
        provider: Provider,
        skill_name: String,
    },
    /// Replace an existing, different custom-skill directory. Approval is
    /// pinned to the expected target tree digest and a recoverable local
    /// backup is taken before the swap.
    OverwriteCustomSkill {
        provider: Provider,
        skill_name: String,
    },
    Manual {
        message: String,
    },
}

impl RestoreActionKind {
    pub fn action_type(&self) -> RestoreActionType {
        match self {
            Self::WriteFile { .. } => RestoreActionType::WriteFile,
            Self::WriteProjectFile { .. } => RestoreActionType::WriteProjectFile,
            Self::EnsureProjectDirectory { .. } => RestoreActionType::EnsureProjectDirectory,
            Self::DeleteProjectFile { .. } => RestoreActionType::DeleteProjectFile,
            Self::DeleteProjectDirectory { .. } => RestoreActionType::DeleteProjectDirectory,
            Self::MergeFile { .. } => RestoreActionType::MergeFile,
            Self::MaterializeConversation { .. } => RestoreActionType::MaterializeConversation,
            Self::InstallCustomSkill { .. } => RestoreActionType::InstallCustomSkill,
            Self::OverwriteCustomSkill { .. } => RestoreActionType::OverwriteCustomSkill,
            Self::Manual { .. } => RestoreActionType::Manual,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::WriteProjectFile { mode, .. } | Self::EnsureProjectDirectory { mode, .. } => {
                if mode & !0o777 != 0 {
                    Err("project-content mode contains unsafe bits".to_string())
                } else {
                    Ok(())
                }
            }
            Self::DeleteProjectFile { last_sha256, .. } => {
                validate_sha256("project-content deletion", last_sha256)
            }
            Self::InstallCustomSkill { skill_name, .. }
            | Self::OverwriteCustomSkill { skill_name, .. } => {
                validate_skill_name("custom skill name", skill_name)
            }
            Self::Manual { message } => validate_display_text("manual action", message, false),
            _ => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RestoreAction {
    pub action_id: ActionId,
    pub resource_id: ResourceId,
```

## RestoreAction explicit-approval check including skill actions (lines 3995–4015)
```rust
            .expected_target_mode
            .is_some_and(|mode| mode & !0o777 != 0)
        {
            return Err("restore target mode contains unsafe bits".to_string());
        }
        if matches!(
            self.kind.action_type(),
            RestoreActionType::InstallCustomSkill
                | RestoreActionType::OverwriteCustomSkill
                | RestoreActionType::WriteProjectFile
                | RestoreActionType::EnsureProjectDirectory
                | RestoreActionType::DeleteProjectFile
                | RestoreActionType::DeleteProjectDirectory
        ) && !self.requires_explicit_approval
        {
            return Err(format!(
                "action '{}' requires explicit approval",
                self.action_id
            ));
        }
        Ok(())
```

## Dependency layer: DependencyActionKind, DependencyAction, DependencyPlan, DependencyApplyReceipt, DependencyApplicationRecord, DependencyApplications (lines 4120–4349)
```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyActionKind {
    InstallCodexPlugin,
    InstallClaudePlugin,
    InstallStandaloneSkill,
    CheckBinary,
    CheckEnvironment,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyAction {
    pub action_id: ActionId,
    pub resource_id: ResourceId,
    pub kind: DependencyActionKind,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub argv: Vec<String>,
    pub requires_explicit_approval: bool,
}

impl DependencyAction {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("dependency display name", &self.display_name, false)?;
        if self.argv.len() > 64
            || self.argv.iter().any(|argument| {
                argument.len() > 4_096 || argument.chars().any(|character| character == '\0')
            })
        {
            return Err(format!(
                "dependency action '{}' has invalid arguments",
                self.action_id
            ));
        }
        match self.kind {
            DependencyActionKind::InstallCodexPlugin => {
                if self.provider != Some(Provider::Codex) {
                    return Err("Codex plugin dependency has the wrong provider".to_string());
                }
            }
            DependencyActionKind::InstallClaudePlugin => {
                if self.provider != Some(Provider::Claude) {
                    return Err("Claude plugin dependency has the wrong provider".to_string());
                }
            }
            DependencyActionKind::InstallStandaloneSkill if self.provider.is_none() => {
                return Err("standalone-skill dependency lacks a provider".to_string());
            }
            _ => {}
        }
        if matches!(
            self.kind,
            DependencyActionKind::InstallCodexPlugin
                | DependencyActionKind::InstallClaudePlugin
                | DependencyActionKind::InstallStandaloneSkill
        ) && !self.requires_explicit_approval
        {
            return Err(format!(
                "dependency action '{}' requires explicit approval",
                self.action_id
            ));
        }
        Ok(())
    }
}

/// Immutable, revision-pinned dependency approval surface. It is separate
/// from a restore plan because plugin installation can be retried without
/// rebuilding or reapplying project files.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyPlan {
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub expected_host_identity_revision: u64,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub actions: Vec<DependencyAction>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl DependencyPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != DEPENDENCY_PLAN_SCHEMA {
            return Err(format!(
                "unsupported dependency-plan schema {}",
                self.schema_version
            ));
        }
        validate_named_id("commit id", &self.commit_id)?;
        validate_sha256("dependency-plan manifest", &self.manifest_sha256)?;
        if self.created_at > self.expires_at
            || self.actions.len() > MAX_ACTIONS
            || self.blockers.len() > MAX_ACTIONS
            || self.warnings.len() > MAX_ACTIONS
        {
            return Err("invalid dependency plan lifetime or collection size".to_string());
        }
        if self.blockers.iter().chain(&self.warnings).any(|message| {
            message.len() > 8_192 || message.chars().any(|character| character == '\0')
        }) {
            return Err("dependency plan contains an invalid message".to_string());
        }
        let mut ids = HashSet::new();
        for action in &self.actions {
            action.validate()?;
            if !ids.insert(action.action_id.clone()) {
                return Err(format!(
                    "duplicate dependency action '{}'",
                    action.action_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyApplyReceipt {
    pub action_id: ActionId,
    pub status: ActionStatus,
    pub applied_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DependencyApplyReceipt {
    pub fn validate(&self) -> Result<(), String> {
        if self.error.as_ref().is_some_and(|error| {
            error.len() > 8_192 || error.chars().any(|character| character == '\0')
        }) {
            return Err("invalid dependency receipt error".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyApplicationRecord {
    pub plan_id: PlanId,
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub applied_at: u64,
    #[serde(default)]
    pub receipts: Vec<DependencyApplyReceipt>,
}

impl DependencyApplicationRecord {
    pub fn validate(&self) -> Result<(), String> {
        validate_named_id("commit id", &self.commit_id)?;
        validate_sha256("dependency application manifest", &self.manifest_sha256)?;
        if self.receipts.len() > MAX_ACTIONS {
            return Err("dependency application has too many receipts".to_string());
        }
        let mut ids = HashSet::new();
        for receipt in &self.receipts {
            receipt.validate()?;
            if !ids.insert(receipt.action_id.clone()) {
                return Err(format!(
                    "duplicate dependency receipt '{}'",
                    receipt.action_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyApplications {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub records: Vec<DependencyApplicationRecord>,
}

impl Default for DependencyApplications {
    fn default() -> Self {
        Self {
            schema: LOCAL_SCHEMA,
            revision: 0,
            records: Vec::new(),
        }
    }
}

impl DependencyApplications {
    pub fn validate(&self, config: &SyncConfig) -> Result<(), String> {
        if self.schema != LOCAL_SCHEMA || self.records.len() > 100_000 {
            return Err("invalid dependency applications document".to_string());
        }
        let mut plans = HashSet::new();
        for record in &self.records {
            record.validate()?;
            if !plans.insert(record.plan_id.clone()) {
                return Err(format!(
                    "duplicate dependency application for plan '{}'",
                    record.plan_id
                ));
            }
            if let Some(project) = config.project(&record.local_project_id) {
                if project.bundle_id != record.bundle_id {
                    return Err("dependency application/project bundle mismatch".to_string());
                }
            }
        }
        Ok(())
    }
}
```

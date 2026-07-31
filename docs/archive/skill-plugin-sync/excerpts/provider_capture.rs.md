# Excerpts: src-tauri/src/project_sync/provider_capture.rs

Skill/plugin capture code removed from `provider_capture.rs`. Line numbers are pre-removal.

## Capture-side dependency model: DependencyKind, DependencyScope, DependencyAction, CaptureApplyPolicy::Dependency (lines 58–98)
```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureApplyPolicy {
    SafeFile,
    Merge,
    Review,
    Dependency,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    CodexPlugin,
    ClaudePlugin,
    StandaloneSkill,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyScope {
    ProviderHome,
}

/// An executable operation is portable intent, never a shell command. The
/// eventual dependency runner must pass `argv` directly to a process API and
/// require the action's explicit approval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyAction {
    pub action_id: String,
    pub resource_id: String,
    pub provider: Provider,
    pub kind: DependencyKind,
    pub scope: DependencyScope,
    pub display_name: String,
    pub program: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    pub requires_review: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_logical_prefix: Option<String>,
}
```

## ResourceCandidate.dependency field (lines 115–116) and CapturedResources.dependency_actions (line 168)
```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency: Option<DependencyAction>,
    /// Kind-specific descriptor facts (provenance, naming evidence, plugin
    /// source data). Copied verbatim into the bundle descriptor metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}
    /// not leak into portable presentation fields.
    pub conversations: BTreeMap<String, CapturedConversation>,
    pub dependency_actions: Vec<DependencyAction>,
    pub unavailable_resource_ids: Vec<String>,
    pub warnings: Vec<String>,
    pub(crate) excluded_codex_subagent_ids: BTreeSet<String>,
}
```

## StandaloneSkillSource + CaptureRequest capability fields (lines 185–240)
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneSkillSource {
    pub provider: Provider,
    /// Runtime-visible capability name declared by the provider-supported
    /// `SKILL.md` metadata. This is the portable identity and may differ from
    /// the directory used to install the skill.
    pub effective_name: String,
    /// Physical directory name below the provider home's `skills/` root.
    /// Preserve it on restore because skill content may refer to its own
    /// installed path.
    pub install_dir_name: String,
    /// Stable provenance identifier (for example a sanitized Git URL plus
    /// subdirectory). It must not include a content digest.
    pub stable_key: String,
    pub source_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub project_root: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub claude_home: Option<PathBuf>,
    /// Canonical nested registrations owned by other bundles. A session whose
    /// cwd falls below one is excluded from this project's inventory.
    pub excluded_project_roots: Vec<PathBuf>,
    pub standalone_skills: Vec<StandaloneSkillSource>,
    /// Inventoried global plugins from the mapped provider homes. Observations
    /// of a plugin also declared in project config coalesce into one resource.
    pub global_plugins: Vec<global_inventory::GlobalPluginSource>,
    /// Skill candidates the ownership classifier refused; surfaced as blocked
    /// resources so the UI can show the evidence, never captured.
    pub blocked_global_skills: Vec<global_inventory::BlockedSkillCandidate>,
    /// Ordinary project content is intentionally lazy. Normal setup and
    /// provider inventory leave this false; the explicit Project files scan
    /// and a reviewed Push turn it on.
    pub include_project_content: bool,
    /// Additional canonical roots that generic discovery must not enter.
    pub excluded_content_roots: Vec<PathBuf>,
}

impl CaptureRequest {
    #[cfg(test)]
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            codex_home: None,
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        }
    }
}
```

## Capture loop: dependency digest input + action collection (lines 468–475, 533–535)
```rust
        let content_sha256 = resource_digest(
            &resource_files,
            captured_directory.as_ref(),
            discovered.descriptor.dependency.as_ref(),
        );
        if let Some(action) = &discovered.descriptor.dependency {
            result.dependency_actions.push(action.clone());
        }
    result.unavailable_resource_ids.sort();
    result
        .dependency_actions
        .sort_by(|a, b| a.action_id.cmp(&b.action_id));
    result.warnings.sort();
```

## domain_resources capability arms + dependency metadata (lines 580–634)
```rust
        let scope = match candidate.kind {
            CaptureResourceKind::Conversation
            | CaptureResourceKind::Memory
            | CaptureResourceKind::StandaloneSkill => domain::ResourceScope::ProviderState,
            CaptureResourceKind::Plugin => domain::ResourceScope::Dependency,
            _ => domain::ResourceScope::Project,
        };
        let provenance = match candidate.kind {
            CaptureResourceKind::Plugin => domain::Provenance::Plugin {
                provider: provider.ok_or_else(|| "plugin lacks provider".to_string())?,
                plugin_id: candidate.display_name.clone(),
            },
            CaptureResourceKind::StandaloneSkill => domain::Provenance::StandaloneSnapshot {
                stable_key: candidate
                    .metadata
                    .get("stable_key")
                    .cloned()
                    .unwrap_or_else(|| candidate.display_name.clone()),
                source_digest: resource.content_sha256.clone(),
            },
            CaptureResourceKind::ProjectContentFile
            | CaptureResourceKind::ProjectContentDirectory => {
                let relative_path = candidate
                    .logical_paths
                    .first()
                    .and_then(|path| path.strip_prefix("project/"))
                    .unwrap_or(&candidate.display_name)
                    .to_string();
                domain::Provenance::ProjectLocal { relative_path }
            }
            _ => domain::Provenance::Unknown,
        };
        let mut metadata = candidate.metadata.clone();
        metadata.retain(|key, _| !key.starts_with("_local_"));
        metadata.insert(
            "content_sha256".to_string(),
            resource.content_sha256.clone(),
        );
        if let Some(action) = &candidate.dependency {
            metadata.insert(
                "dependency_kind".to_string(),
                dependency_kind_name(&action.kind).to_string(),
            );
            metadata.insert(
                "dependency_scope".to_string(),
                dependency_scope_name(&action.scope).to_string(),
            );
            if let Some(program) = &action.program {
                metadata.insert("dependency_program".to_string(), program.clone());
            }
            metadata.insert(
                "dependency_argv_json".to_string(),
                serde_json::to_string(&action.argv).map_err(|e| e.to_string())?,
            );
        }
```

## domain_dependency_actions + kind/scope/apply-policy mappings (lines 653–726)
```rust
#[cfg(test)]
pub fn domain_dependency_actions(
    captured: &CapturedResources,
) -> Result<Vec<domain::DependencyAction>, String> {
    captured
        .dependency_actions
        .iter()
        .map(|action| {
            Ok(domain::DependencyAction {
                action_id: domain::ActionId::parse(action.action_id.clone())?,
                resource_id: domain::ResourceId::parse(action.resource_id.clone())?,
                kind: match action.kind {
                    DependencyKind::CodexPlugin => domain::DependencyActionKind::InstallCodexPlugin,
                    DependencyKind::ClaudePlugin => {
                        domain::DependencyActionKind::InstallClaudePlugin
                    }
                    DependencyKind::StandaloneSkill => {
                        domain::DependencyActionKind::InstallStandaloneSkill
                    }
                },
                display_name: action.display_name.clone(),
                provider: Some(domain_provider(action.provider)),
                argv: action.argv.clone(),
                requires_explicit_approval: action.requires_review,
            })
        })
        .collect()
}

fn domain_provider(provider: Provider) -> domain::Provider {
    match provider {
        Provider::Codex => domain::Provider::Codex,
        Provider::Claude => domain::Provider::Claude,
    }
}

fn domain_kind(kind: &CaptureResourceKind, provider: Option<Provider>) -> domain::ResourceKind {
    match kind {
        CaptureResourceKind::ProjectContentFile => domain::ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => {
            domain::ResourceKind::ProjectContentDirectory
        }
        CaptureResourceKind::Conversation => match provider {
            Some(Provider::Claude) => domain::ResourceKind::ClaudeConversation,
            _ => domain::ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => domain::ResourceKind::ProjectMemory,
        CaptureResourceKind::StandaloneSkill => domain::ResourceKind::StandaloneSkill,
        CaptureResourceKind::Plugin => domain::ResourceKind::Plugin,
    }
}

fn domain_apply_policy(policy: &CaptureApplyPolicy) -> domain::ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => domain::ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => domain::ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => domain::ApplyPolicy::ExplicitReview,
        CaptureApplyPolicy::Dependency => domain::ApplyPolicy::ExplicitInstall,
    }
}

fn dependency_kind_name(kind: &DependencyKind) -> &'static str {
    match kind {
        DependencyKind::CodexPlugin => "codex_plugin",
        DependencyKind::ClaudePlugin => "claude_plugin",
        DependencyKind::StandaloneSkill => "standalone_skill",
    }
}

fn dependency_scope_name(scope: &DependencyScope) -> &'static str {
    match scope {
        DependencyScope::ProviderHome => "provider_home",
    }
}
```

## discover_full capability loops (lines 769–777)
```rust
    for skill in &request.standalone_skills {
        discover_standalone_skill(skill, &canonical_project, &mut discovery)?;
    }
    for plugin in &request.global_plugins {
        discover_global_plugin(plugin, &mut discovery)?;
    }
    for blocked in &request.blocked_global_skills {
        discover_blocked_global_skill(blocked, &mut discovery)?;
    }
```

## dependency_resource helper (lines 1291–1314)
```rust
fn dependency_resource(
    resource_id: String,
    display_name: String,
    provider: Provider,
    kind: CaptureResourceKind,
    action: DependencyAction,
) -> DiscoveredResource {
    DiscoveredResource {
        descriptor: ResourceCandidate {
            metadata: BTreeMap::new(),
            resource_id,
            kind,
            provider: Some(provider),
            display_name,
            relative_cwd: None,
            apply_policy: CaptureApplyPolicy::Dependency,
            selected_by_default: false,
            blocked_reason: None,
            logical_paths: Vec::new(),
            dependency: Some(action),
        },
        files: Vec::new(),
    }
}
```

## discover_standalone_skill / discover_global_plugin / discover_blocked_global_skill (lines 1663–1882)
```rust
fn discover_standalone_skill(
    skill: &StandaloneSkillSource,
    project_root: &Path,
    discovery: &mut Discovery,
) -> Result<(), String> {
    domain::validate_skill_name("standalone skill effective name", &skill.effective_name)?;
    domain::validate_skill_name(
        "standalone skill install directory",
        &skill.install_dir_name,
    )?;
    if skill.stable_key.is_empty() || skill.stable_key.len() > 1024 {
        return Err(format!(
            "invalid stable provenance for skill '{}'",
            skill.effective_name
        ));
    }
    let canonical_source = canonical_existing_dir(&skill.source_dir, "standalone skill")?;
    if canonical_source.starts_with(project_root) {
        return Err(format!(
            "standalone skill '{}' is already project-local",
            skill.source_dir.display()
        ));
    }
    let files = collect_regular_tree_files(&canonical_source, canonical_source.clone())?;
    // Global custom skills restore into their original physical directory
    // below the mapped provider home's `skills/` root. Runtime identity is
    // independent and comes from `effective_name`.
    let target_root = format!(
        "state/{}/skills/{}",
        provider_name(skill.provider),
        skill.install_dir_name
    );
    let logical_paths = files
        .iter()
        .map(|path| {
            normalized_relative(&canonical_source, path)
                .map(|rel| format!("{}/{}", target_root, rel))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Identity is provider + effective skill name: editing content preserves
    // it, renaming the skill is an explicit remove/add. The stable key stays
    // provenance metadata only.
    let resource_id = format!(
        "{}:standalone-skill:{}",
        provider_name(skill.provider),
        resource_id_component(&skill.effective_name)
    );
    let executable = files.iter().any(|path| is_executable_path(path));
    let dependency = executable.then(|| DependencyAction {
        action_id: format!("dependency:{}", resource_id),
        resource_id: resource_id.clone(),
        provider: skill.provider,
        kind: DependencyKind::StandaloneSkill,
        scope: DependencyScope::ProviderHome,
        display_name: skill.effective_name.clone(),
        program: None,
        argv: Vec::new(),
        requires_review: true,
        payload_logical_prefix: Some(target_root.clone()),
    });
    let mut metadata = BTreeMap::new();
    metadata.insert("stable_key".to_string(), skill.stable_key.clone());
    metadata.insert("effective_name".to_string(), skill.effective_name.clone());
    metadata.insert(
        "install_dir_name".to_string(),
        skill.install_dir_name.clone(),
    );
    metadata.insert(
        "provider_adapter_version".to_string(),
        global_inventory::PROVIDER_ADAPTER_VERSION.to_string(),
    );
    metadata.insert(
        "ownership_evidence".to_string(),
        "no plugin claim; regular global skills directory".to_string(),
    );
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                metadata,
                resource_id,
                kind: CaptureResourceKind::StandaloneSkill,
                provider: Some(skill.provider),
                display_name: skill.effective_name.clone(),
                relative_cwd: None,
                apply_policy: if executable {
                    CaptureApplyPolicy::Dependency
                } else {
                    CaptureApplyPolicy::SafeFile
                },
                selected_by_default: false,
                blocked_reason: inspect_candidate_files(&canonical_source, &files).err(),
                logical_paths: logical_paths.clone(),
                dependency,
            },
            files: files
                .into_iter()
                .zip(logical_paths)
                .map(|(physical_path, logical_path)| SourceFile {
                    physical_path,
                    logical_path,
                    opaque_content: true,
                    derived_bytes: None,
                })
                .collect(),
        },
    )
}

/// Register one inventoried global plugin as portable install intent.
fn discover_global_plugin(
    plugin: &global_inventory::GlobalPluginSource,
    discovery: &mut Discovery,
) -> Result<(), String> {
    let resource_id = format!(
        "{}:plugin:{}",
        provider_name(plugin.provider),
        resource_id_component(&plugin.plugin_id)
    );
    let mut metadata = BTreeMap::new();
    metadata.insert("plugin_origin".to_string(), "global".to_string());
    if let Some(marketplace) = &plugin.marketplace {
        metadata.insert("plugin_marketplace".to_string(), marketplace.clone());
    }
    if let Some(source_type) = &plugin.source_type {
        metadata.insert("plugin_source_type".to_string(), source_type.clone());
    }
    if let Some(source) = &plugin.source {
        metadata.insert("plugin_source".to_string(), source.clone());
    }
    if let Some(version) = &plugin.observed_version {
        // Observational only: install may resolve a different marketplace
        // version unless the native CLI can pin exactly.
        metadata.insert("plugin_observed_version".to_string(), version.clone());
    }
    metadata.insert("plugin_enabled".to_string(), plugin.enabled.to_string());
    if !plugin.provided_skills.is_empty() {
        metadata.insert(
            "plugin_provided_skills_json".to_string(),
            serde_json::to_string(&plugin.provided_skills).map_err(|e| e.to_string())?,
        );
    }
    let (kind, program, argv) = match plugin.provider {
        Provider::Codex => (
            DependencyKind::CodexPlugin,
            "codex",
            vec![
                "plugin".to_string(),
                "add".to_string(),
                plugin.plugin_id.clone(),
            ],
        ),
        Provider::Claude => (
            DependencyKind::ClaudePlugin,
            "claude",
            vec![
                "plugin".to_string(),
                "install".to_string(),
                plugin.plugin_id.clone(),
            ],
        ),
    };
    let action = DependencyAction {
        action_id: format!("dependency:{}", resource_id),
        resource_id: resource_id.clone(),
        provider: plugin.provider,
        kind,
        scope: DependencyScope::ProviderHome,
        display_name: plugin.plugin_id.clone(),
        program: Some(program.to_string()),
        argv,
        requires_review: true,
        payload_logical_prefix: None,
    };
    let mut resource = dependency_resource(
        resource_id,
        plugin.plugin_id.clone(),
        plugin.provider,
        CaptureResourceKind::Plugin,
        action,
    );
    resource.descriptor.metadata = metadata;
    insert_resource(discovery, resource)
}

/// Surface an unclassifiable global skill candidate as a blocked resource so
/// review can show the ownership evidence. It carries no files and can never
/// be captured.
fn discover_blocked_global_skill(
    blocked: &global_inventory::BlockedSkillCandidate,
    discovery: &mut Discovery,
) -> Result<(), String> {
    let resource_id = format!(
        "{}:standalone-skill:{}",
        provider_name(blocked.provider),
        resource_id_component(&blocked.name)
    );
    if discovery.resources.contains_key(&resource_id) {
        return Ok(());
    }
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                metadata: BTreeMap::new(),
                resource_id,
                kind: CaptureResourceKind::StandaloneSkill,
                provider: Some(blocked.provider),
                display_name: blocked.name.clone(),
                relative_cwd: None,
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: false,
                blocked_reason: Some(blocked.reason.clone()),
                logical_paths: Vec::new(),
                dependency: None,
            },
            files: Vec::new(),
        },
    )
}
```

## resource_digest dependency input (lines 2541–2565)
```rust
fn resource_digest(
    files: &[CapturedFile],
    directory: Option<&CapturedDirectory>,
    dependency: Option<&DependencyAction>,
) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update((file.logical_path.len() as u64).to_be_bytes());
        hasher.update(file.logical_path.as_bytes());
        hasher.update((file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
        hasher.update(file.mode.to_be_bytes());
    }
    if let Some(directory) = directory {
        hasher.update((directory.logical_path.len() as u64).to_be_bytes());
        hasher.update(directory.logical_path.as_bytes());
        hasher.update(directory.mode.to_be_bytes());
    }
    if let Some(dependency) = dependency {
        if let Ok(bytes) = serde_json::to_vec(dependency) {
            hasher.update(bytes);
        }
    }
    hex_digest(hasher.finalize().as_slice())
}
```

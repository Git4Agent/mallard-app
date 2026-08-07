# Excerpts: src-tauri/src/project_sync/commands.rs

Capability status + dependency plan/apply layer removed from `commands.rs`. Line numbers are pre-removal.

## Capability report DTOs (lines 432–488)
```rust
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CapabilityProfileContext {
    pub provider: Provider,
    pub profile_id: LocalProviderProfileId,
    pub display_name: String,
    pub path: String,
    pub shared_project_count: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct CapabilityStatusItem {
    #[serde(flatten)]
    pub descriptor: ResourceDescriptor,
    pub category: String,
    pub state: String,
    pub local_present: bool,
    pub storage_present: bool,
    pub selected_in_recipe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provided_skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CapabilityStatusReport {
    pub project_id: LocalProjectId,
    pub project_name: String,
    #[serde(default)]
    pub profiles: Vec<CapabilityProfileContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<StorageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    pub compared_at: u64,
    pub items: Vec<CapabilityStatusItem>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
```

## DependencyResult DTO (lines 735–743)
```rust
#[derive(Serialize, Clone, Debug)]
pub struct DependencyResult {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub applied_action_ids: Vec<ActionId>,
    #[serde(default)]
    pub failed_actions: Vec<FailedAction>,
}
```

## get_project_capability_status command (lines 1732–1747)
```rust
#[tauri::command]
pub async fn get_project_capability_status(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
) -> Result<CapabilityStatusReport, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_project_capability_status_with_repository(
            &repository,
            &local_project_id,
            storage_id.as_ref(),
        )
    })
    .await
}
```

## plan_dependencies / apply_dependency_actions commands (lines 2638–2654)
```rust
#[tauri::command]
pub async fn plan_dependencies(
    app: tauri::AppHandle,
    restore_plan_id: PlanId,
) -> Result<DependencyPlan, String> {
    let repository = repository(&app)?;
    run_blocking(move || plan_dependencies_with_repository(&repository, &restore_plan_id)).await
}

#[tauri::command]
pub async fn apply_dependency_actions(
    app: tauri::AppHandle,
    plan_id: PlanId,
    action_ids: Vec<ActionId>,
) -> Result<DependencyResult, String> {
    apply_dependency_actions_with_repository(&repository(&app)?, &plan_id, action_ids).await
}
```

## Inventory mapping: capability arms in inventory_from_candidates (lines 3864–3902), provided_by/install_behavior (3925–3930), kind/category maps (4057–4093)
```rust
        let scope = match candidate.kind {
            CaptureResourceKind::Conversation
            | CaptureResourceKind::Memory
            | CaptureResourceKind::StandaloneSkill => ResourceScope::ProviderState,
            CaptureResourceKind::Plugin => ResourceScope::Dependency,
            _ => ResourceScope::Project,
        };
        let provenance = match candidate.kind {
            CaptureResourceKind::Plugin => Provenance::Plugin {
                provider: provider
                    .ok_or_else(|| "plugin candidate lacks a provider".to_string())?,
                plugin_id: candidate.display_name.clone(),
            },
            _ => candidate
                .logical_paths
                .first()
                .and_then(|path| path.strip_prefix("project/"))
                .map(|relative_path| Provenance::ProjectLocal {
                    relative_path: relative_path.to_string(),
                })
                .unwrap_or(Provenance::Unknown),
        };
        let description = match candidate.kind {
            CaptureResourceKind::StandaloneSkill => Some(
                "Global custom skill from the mapped provider home. Restoring installs it \
                 for every project sharing that provider profile."
                    .to_string(),
            ),
            CaptureResourceKind::Plugin if candidate.metadata.contains_key("plugin_origin") => {
                Some(
                    "Installed global plugin. Only portable install intent syncs; pull \
                     reinstalls through the provider's own CLI."
                        .to_string(),
                )
            }
            _ => None,
        };
        let apply_policy = recipe
            .entries
// ...
            category: resource_category(kind).to_string(),
            description,
            logical_paths: candidate.logical_paths.clone(),
            default_selected: candidate.selected_by_default,
            blocked_reason: candidate.blocked_reason.clone(),
            provided_by: provider.map(|provider| format!("{:?}", provider)),
            install_behavior: candidate
                .dependency
                .as_ref()
                .map(|_| "Requires explicit approval during restore".to_string()),
            descriptor,
        });
    }
// ...
fn capture_resource_kind(
    kind: &CaptureResourceKind,
    provider: Option<CaptureProvider>,
) -> ResourceKind {
    match kind {
        CaptureResourceKind::ProjectContentFile => ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => ResourceKind::ProjectContentDirectory,
        CaptureResourceKind::Conversation => match provider {
            Some(CaptureProvider::Claude) => ResourceKind::ClaudeConversation,
            _ => ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => ResourceKind::ProjectMemory,
        CaptureResourceKind::StandaloneSkill => ResourceKind::StandaloneSkill,
        CaptureResourceKind::Plugin => ResourceKind::Plugin,
    }
}

fn capture_apply_policy(policy: &CaptureApplyPolicy) -> ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => ApplyPolicy::ExplicitReview,
        CaptureApplyPolicy::Dependency => ApplyPolicy::ExplicitInstall,
    }
}

fn resource_category(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::CodexConversation
        | ResourceKind::ClaudeConversation
        | ResourceKind::ProjectMemory => "conversations",
        ResourceKind::StandaloneSkill => "skills",
        ResourceKind::Plugin => "plugins",
        ResourceKind::ProjectContentFile | ResourceKind::ProjectContentDirectory => "project_files",
        ResourceKind::Requirement => "tools",
    }
}
```

## Capability status section: digests, classification, report build (lines 5450–5908)
```rust
fn is_capability_kind(kind: ResourceKind) -> bool {
    matches!(kind, ResourceKind::StandaloneSkill | ResourceKind::Plugin)
}

fn capability_state_name(state: ThreadSyncState) -> &'static str {
    match state {
        ThreadSyncState::Synced => "synced",
        ThreadSyncState::LocalOnly => "local_only",
        ThreadSyncState::LocalAhead => "local_ahead",
        ThreadSyncState::StorageOnly => "storage_only",
        ThreadSyncState::StorageAhead => "storage_ahead",
        ThreadSyncState::Diverged => "diverged",
        ThreadSyncState::Unavailable => "unavailable",
        ThreadSyncState::Unknown => "unknown",
    }
}

fn descriptor_capability_digest(descriptor: Option<&ResourceDescriptor>) -> Option<String> {
    let descriptor = descriptor?;
    if descriptor.kind != ResourceKind::Plugin {
        return descriptor.metadata.get("content_sha256").cloned();
    }

    // Plugin payloads remain installer-owned. Compare only normalized,
    // portable installation intent; observed versions are diagnostic and may
    // legitimately resolve differently on two machines.
    let mut hasher = Sha256::new();
    hasher.update(descriptor.resource_id.as_str().as_bytes());
    if let Some(provider) = descriptor.provider {
        hasher.update(provider_name(provider).as_bytes());
    }
    for key in [
        "plugin_marketplace",
        "plugin_source_type",
        "plugin_source",
        "dependency_program",
        "dependency_argv_json",
    ] {
        if let Some(value) = descriptor.metadata.get(key) {
            hasher.update((key.len() as u64).to_be_bytes());
            hasher.update(key.as_bytes());
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
    Some(hex_digest(hasher.finalize().as_slice()))
}

fn classify_capability_sync_state(
    local_digest: Option<&str>,
    storage_digest: Option<&str>,
    base_known: bool,
    base_digest: Option<&str>,
) -> ThreadSyncState {
    let state = classify_thread_sync_state(local_digest, storage_digest, base_known, base_digest);
    if state == ThreadSyncState::Unknown && local_digest.is_some() && storage_digest.is_some() {
        ThreadSyncState::Diverged
    } else {
        state
    }
}

fn descriptor_version(descriptor: Option<&ResourceDescriptor>) -> Option<String> {
    descriptor.and_then(|descriptor| descriptor.metadata.get("plugin_observed_version").cloned())
}

fn descriptor_provided_skills(descriptor: Option<&ResourceDescriptor>) -> Vec<String> {
    descriptor
        .and_then(|descriptor| descriptor.metadata.get("plugin_provided_skills_json"))
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn get_project_capability_status_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: Option<&StorageId>,
) -> Result<CapabilityStatusReport, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    let machine = repository.load_bindings()?;
    let binding = machine
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovery = provider_capture::discover_full(&capture_request)?;
    let storage = storage_id
        .map(|storage_id| {
            let remote = storage_engine(repository, storage_id).and_then(|(_, engine)| {
                let current = engine.inspect_optional(&project.bundle_id)?;
                Ok((engine, current))
            });
            (storage_id, remote)
        })
        .map(|(storage_id, remote)| CapabilityStorageContext { storage_id, remote });
    build_capability_status(
        &config,
        local_project_id,
        project,
        &machine,
        &binding,
        &capture_request,
        &discovery,
        storage,
    )
}

/// Storage side of a capability comparison: the engine and one already-read
/// remote snapshot, or the error that made the storage unreachable (which
/// degrades the report to `not_compared`, never fails it).
struct CapabilityStorageContext<'a> {
    storage_id: &'a StorageId,
    remote: Result<(StorageEngine, Option<BundleSnapshot>), String>,
}

/// Capability section over a shared Discovery. Skill and plugin bytes are
/// read and hashed here; a capture failure degrades to defaults with a
/// warning exactly like the standalone command always has.
#[allow(clippy::too_many_arguments)]
fn build_capability_status(
    config: &SyncConfig,
    local_project_id: &LocalProjectId,
    project: LocalProjectRegistration,
    machine: &MachineProjectState,
    binding: &ProjectBinding,
    capture_request: &CaptureRequest,
    discovery: &provider_capture::Discovery,
    storage: Option<CapabilityStorageContext<'_>>,
) -> Result<CapabilityStatusReport, String> {
    let mut warnings = Vec::new();
    let mut profiles = Vec::new();
    for (provider, profile_id) in &binding.profile_ids {
        let Some(profile) = machine
            .profiles
            .iter()
            .find(|candidate| &candidate.profile_id == profile_id)
        else {
            warnings.push(format!("provider profile '{}' is unavailable", profile_id));
            continue;
        };
        let shared_project_count = machine
            .bindings
            .iter()
            .filter(|candidate| {
                candidate.state == BindingState::Active
                    && candidate.profile_ids.values().any(|id| id == profile_id)
            })
            .map(|candidate| candidate.local_project_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        profiles.push(CapabilityProfileContext {
            provider: *provider,
            profile_id: profile_id.clone(),
            display_name: profile.display_name.clone(),
            path: profile.path.clone(),
            shared_project_count,
        });
    }
    profiles.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    let discovered = discovery.project_inventory();
    let inventory = inventory_from_candidates(
        binding.project_root.clone(),
        Some(project.bundle_id.clone()),
        &discovered.resources,
        project.recipe.clone(),
        discovered.warnings.clone(),
    )?;
    warnings.extend(inventory.warnings.clone());

    let selected_capabilities = discovered
        .resources
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.kind,
                CaptureResourceKind::StandaloneSkill | CaptureResourceKind::Plugin
            )
        })
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let captured = match provider_capture::capture_from_discovery(
        capture_request,
        discovery,
        &selected_capabilities,
    ) {
        Ok(captured) => captured,
        Err(error) => {
            warnings.push(format!(
                "Skill and plugin content could not be fully compared: {error}"
            ));
            provider_capture::CapturedResources::default()
        }
    };
    warnings.extend(captured.warnings.clone());
    let local_captured = provider_capture::domain_resources(&captured)?;
    let unavailable = captured
        .unavailable_resource_ids
        .iter()
        .map(|resource_id| ResourceId::parse(resource_id.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;

    let mut local_inventory = inventory
        .resources
        .into_iter()
        .filter(|resource| is_capability_kind(resource.descriptor.kind))
        .map(|mut resource| {
            if let Some(captured) = local_captured.get(&resource.descriptor.resource_id) {
                resource.descriptor = captured.clone();
            }
            (resource.descriptor.resource_id.clone(), resource)
        })
        .collect::<BTreeMap<_, _>>();

    let storage_id = storage.as_ref().map(|context| context.storage_id.clone());
    let mut storage_name = None;
    let mut current = None;
    let mut base_manifest = None;
    let mut effective_recipe = project.recipe.clone();
    let mut storage_comparison_available = false;
    let base_revision = storage_id
        .as_ref()
        .and_then(|storage_id| project.recipe_bases.get(storage_id))
        .map(|base| base.revision);

    if let Some(context) = storage {
        let storage_id = context.storage_id;
        require_project_link(config, &project, storage_id)?;
        storage_name = config
            .storages
            .iter()
            .find(|storage| storage.id == *storage_id)
            .map(|storage| storage.name.clone());
        effective_recipe = config
            .links
            .iter()
            .find(|link| {
                link.local_project_id == *local_project_id && link.storage_id == *storage_id
            })
            .and_then(|link| link.recipe.clone())
            .unwrap_or_else(|| project.recipe.clone());

        match context.remote {
            Ok((engine, snapshot)) => {
                current = snapshot;
                storage_comparison_available = true;
                if let Some(base) = project.recipe_bases.get(storage_id) {
                    if base.binding_revision != Some(binding.revision) {
                        warnings.push(
                            "The project folder or agent profile changed after the last reviewed sync; skill and plugin direction is unavailable until the next Pull or Push."
                                .to_string(),
                        );
                    } else if current.as_ref().is_some_and(|snapshot| {
                        snapshot.head.revision == base.revision
                            && snapshot.head.manifest_sha256 == base.manifest_sha256
                    }) {
                        base_manifest = current.as_ref().map(|snapshot| snapshot.manifest.clone());
                    } else if let Some(commit_id) = base.commit_id.as_deref() {
                        match engine.inspect_manifest_version(
                            &project.bundle_id,
                            base.revision,
                            commit_id,
                            &base.manifest_sha256,
                        ) {
                            Ok(manifest) => base_manifest = Some(manifest),
                            Err(error) => warnings.push(format!(
                                "The reviewed skill and plugin base could not be loaded: {error}"
                            )),
                        }
                    } else {
                        warnings.push(
                            "The reviewed base predates directional comparison; Pull or Push once to establish it."
                                .to_string(),
                        );
                    }
                }
            }
            Err(error) => warnings.push(format!(
                "Skills and plugins could not be compared with storage: {error}"
            )),
        }
    }

    let remote_resources = current
        .as_ref()
        .map(|snapshot| {
            snapshot
                .manifest
                .resources
                .iter()
                .filter(|(_, descriptor)| is_capability_kind(descriptor.kind))
                .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let base_resources = base_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .resources
                .iter()
                .filter(|(_, descriptor)| is_capability_kind(descriptor.kind))
                .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut remote_paths = BTreeMap::<ResourceId, Vec<String>>::new();
    if let Some(snapshot) = &current {
        for (path, file) in &snapshot.manifest.files {
            if remote_resources.contains_key(&file.resource_id) {
                remote_paths
                    .entry(file.resource_id.clone())
                    .or_default()
                    .push(path.as_str().to_string());
            }
        }
    }

    let resource_ids = local_inventory
        .keys()
        .chain(remote_resources.keys())
        .chain(base_resources.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut items = Vec::new();
    for resource_id in resource_ids {
        let local_inventory_resource = local_inventory.remove(&resource_id);
        let local_descriptor = local_captured.get(&resource_id).or_else(|| {
            local_inventory_resource
                .as_ref()
                .map(|resource| &resource.descriptor)
        });
        let remote_descriptor = remote_resources.get(&resource_id);
        let base_descriptor = base_resources.get(&resource_id);
        let local_present = local_inventory_resource.is_some();
        let storage_present = remote_descriptor.is_some();
        if !local_present && !storage_present {
            continue;
        }
        let descriptor = local_descriptor
            .or(remote_descriptor)
            .or(base_descriptor)
            .cloned()
            .ok_or_else(|| format!("capability '{}' has no descriptor", resource_id))?;
        let blocked_reason = local_inventory_resource
            .as_ref()
            .and_then(|resource| resource.blocked_reason.clone());
        let local_digest = descriptor_capability_digest(local_descriptor);
        let storage_digest = descriptor_capability_digest(remote_descriptor);
        let base_digest = descriptor_capability_digest(base_descriptor);
        let incomplete = unavailable.contains(&resource_id)
            || (local_present && local_digest.is_none())
            || (storage_present && storage_digest.is_none());
        let state = if blocked_reason.is_some() {
            "blocked"
        } else if !storage_comparison_available {
            "not_compared"
        } else if incomplete {
            "unknown"
        } else {
            let base_resource_known =
                base_manifest.is_some() && (base_descriptor.is_none() || base_digest.is_some());
            capability_state_name(classify_capability_sync_state(
                local_digest.as_deref(),
                storage_digest.as_deref(),
                base_resource_known,
                base_digest.as_deref(),
            ))
        };
        let local_version = descriptor_version(local_descriptor);
        let storage_version = descriptor_version(remote_descriptor);
        let enabled = local_descriptor
            .and_then(|descriptor| descriptor.metadata.get("plugin_enabled"))
            .and_then(|value| value.parse::<bool>().ok());
        let mut provided_skills = descriptor_provided_skills(local_descriptor);
        provided_skills.extend(descriptor_provided_skills(remote_descriptor));
        provided_skills.sort();
        provided_skills.dedup();
        let message = blocked_reason.clone().or_else(|| {
            if descriptor.kind == ResourceKind::Plugin && enabled == Some(false) {
                Some("Installed locally but disabled in the provider configuration".to_string())
            } else if descriptor.kind == ResourceKind::Plugin
                && local_version.is_some()
                && storage_version.is_some()
                && local_version != storage_version
            {
                Some(
                    "Observed plugin versions differ; native installation may resolve a newer version"
                        .to_string(),
                )
            } else {
                None
            }
        });
        let logical_paths = local_inventory_resource
            .as_ref()
            .map(|resource| resource.logical_paths.clone())
            .filter(|paths| !paths.is_empty())
            .or_else(|| remote_paths.get(&resource_id).cloned())
            .unwrap_or_default();

        items.push(CapabilityStatusItem {
            category: resource_category(descriptor.kind).to_string(),
            descriptor,
            state: state.to_string(),
            local_present,
            storage_present,
            selected_in_recipe: effective_recipe.entries.contains_key(&resource_id),
            blocked_reason,
            logical_paths,
            local_digest,
            storage_digest,
            local_version,
            storage_version,
            enabled,
            provided_skills,
            message,
        });
    }
    items.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.descriptor.provider.cmp(&right.descriptor.provider))
            .then_with(|| {
                left.descriptor
                    .display_name
                    .to_lowercase()
                    .cmp(&right.descriptor.display_name.to_lowercase())
            })
    });
    warnings.sort();
    warnings.dedup();

    Ok(CapabilityStatusReport {
        project_id: project.local_project_id,
        project_name: project.local_alias.unwrap_or(project.display_name),
        profiles,
        storage_id,
        storage_name,
        revision: current.as_ref().map(|snapshot| snapshot.head.revision),
        base_revision,
        compared_at: now_secs(),
        items,
        warnings,
    })
}
```

## PushReviewContext capability fields + population (lines 6021–6022, 6203–6220)
```rust
#[derive(Serialize, Clone, Debug)]
pub struct PushReviewContext {
    pub project: ProjectDetail,
    pub inventory: ResourceInventory,
    pub destination_status: PushDestinationStatus,
    pub thread_comparison: Option<ThreadSyncComparison>,
    pub thread_comparison_error: Option<String>,
    pub capability_report: Option<CapabilityStatusReport>,
    pub capability_report_error: Option<String>,
    pub project_files: Option<ProjectContentInventory>,
    pub project_files_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_push_plan_id: Option<PlanId>,
    #[serde(default)]
    pub conversation_action_ids: BTreeMap<ResourceId, ActionId>,
    pub prepared_at: u64,
}

// ...
    // Capabilities degrade internally on storage failure ("not_compared"),
    // matching the standalone command.
    let mut capability_report = None;
    let mut capability_report_error = None;
    match build_capability_status(
        &config,
        local_project_id,
        project.clone(),
        &machine,
        &binding,
        &capture_request,
        &discovery,
        Some(CapabilityStorageContext { storage_id, remote }),
    ) {
        Ok(report) => capability_report = Some(report),
        Err(error) => capability_report_error = Some(error),
    }

```

## Materialization/readiness action-type arms (lines 8341–8342, 9127–9129)
```rust
            RestoreActionType::WriteFile
            | RestoreActionType::MergeFile
            | RestoreActionType::MaterializeConversation
            | RestoreActionType::InstallCustomSkill
            | RestoreActionType::OverwriteCustomSkill => receipt.status == ActionStatus::Applied,
            RestoreActionType::Manual => {
// ...
            match receipt.action_type {
                RestoreActionType::WriteFile
                | RestoreActionType::MergeFile
                | RestoreActionType::MaterializeConversation => true,
                RestoreActionType::WriteProjectFile
                | RestoreActionType::EnsureProjectDirectory
                | RestoreActionType::DeleteProjectFile
                | RestoreActionType::DeleteProjectDirectory => {
                    matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked)
                }
                RestoreActionType::InstallCustomSkill
                | RestoreActionType::OverwriteCustomSkill
                | RestoreActionType::Manual => false,
            }
```

## Dependency plan build + apply pipeline (lines 8640–8899)
```rust
fn plan_dependencies_with_repository(
    repository: &LocalStateRepository,
    restore_plan_id: &PlanId,
) -> Result<DependencyPlan, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let (restore_plan, binding, snapshot) = restore_support_context(repository, restore_plan_id)?;
    let created_at = now_secs();
    let plan = DependencyPlan {
        schema_version: DEPENDENCY_PLAN_SCHEMA,
        // `generated_named_id` accepts only lowercase alphanumeric prefixes.
        // The plan type and its persistence directory already distinguish
        // dependency plans from restore plans.
        plan_id: PlanId::parse(generated_named_id("plan")?)?,
        storage_id: restore_plan.storage_id,
        bundle_id: restore_plan.bundle_id,
        host_id: binding.host_id.clone(),
        replica_id: binding.replica_id,
        expected_host_identity_revision: restore_plan.expected_host_identity_revision,
        revision: restore_plan.revision,
        commit_id: restore_plan.commit_id,
        manifest_sha256: restore_plan.manifest_sha256,
        binding_revision: binding.revision,
        created_at,
        expires_at: restore_plan
            .expires_at
            .min(created_at.saturating_add(PLAN_LIFETIME_SECS)),
        actions: dependencies_from_manifest(&snapshot.manifest)?,
        blockers: Vec::new(),
        warnings: Vec::new(),
    };
    plan.validate()?;
    repository.save_dependency_plan(&plan)?;
    Ok(plan)
}

struct PreparedDependencyApplication {
    plan: DependencyPlan,
    binding: ProjectBinding,
    selected: BTreeSet<ActionId>,
    previous: Option<DependencyApplicationRecord>,
    resources: BTreeMap<ResourceId, ResourceDescriptor>,
    applied_at: u64,
}

fn prepare_dependency_application(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    action_ids: Vec<ActionId>,
) -> Result<PreparedDependencyApplication, String> {
    let plan = repository.load_dependency_plan(plan_id)?;
    let applied_at = now_secs();
    if applied_at < plan.created_at || applied_at > plan.expires_at {
        return Err("dependency plan has expired or is not active yet".to_string());
    }
    let bindings = repository.load_bindings()?;
    let binding = bindings
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == plan.host_id
                && binding.replica_id == plan.replica_id
                && binding.state == BindingState::Active
        })
        .cloned()
        .ok_or_else(|| "dependency plan's project binding is no longer active".to_string())?;
    if binding.revision != plan.binding_revision || binding.bundle_id != plan.bundle_id {
        return Err("dependency plan's binding changed after planning".to_string());
    }
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != plan.host_id || identity.revision != plan.expected_host_identity_revision
    {
        return Err("dependency plan's host identity changed after planning".to_string());
    }
    let previous = repository
        .load_dependency_applications()?
        .records
        .into_iter()
        .find(|record| record.plan_id == plan.plan_id);
    if previous.as_ref().is_some_and(|record| {
        record.local_project_id != binding.local_project_id
            || record.storage_id != plan.storage_id
            || record.bundle_id != plan.bundle_id
            || record.host_id != plan.host_id
            || record.replica_id != plan.replica_id
            || record.revision != plan.revision
            || record.commit_id != plan.commit_id
            || record.manifest_sha256 != plan.manifest_sha256
            || record.binding_revision != plan.binding_revision
    }) {
        return Err(
            "stored dependency receipt does not match this plan's host and snapshot".to_string(),
        );
    }
    let binding = resolve_project_binding(repository, &binding)?;
    let (_, snapshot) = inspect_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    validate_dependency_plan_pin(&plan, &snapshot, &binding)?;
    let selected = unique_dependency_actions(&action_ids, &plan.actions)?;

    Ok(PreparedDependencyApplication {
        plan,
        binding,
        selected,
        previous,
        resources: snapshot.manifest.resources,
        applied_at,
    })
}

async fn apply_dependency_actions_with_repository(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    action_ids: Vec<ActionId>,
) -> Result<DependencyResult, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let prepare_repository = repository.clone();
    let prepare_plan_id = plan_id.clone();
    let prepared = run_blocking(move || {
        prepare_dependency_application(&prepare_repository, &prepare_plan_id, action_ids)
    })
    .await?;
    let mut receipts = Vec::with_capacity(prepared.plan.actions.len());
    for action in &prepared.plan.actions {
        if let Some(applied) = prepared.previous.as_ref().and_then(|record| {
            record.receipts.iter().find(|receipt| {
                receipt.action_id == action.action_id && receipt.status == ActionStatus::Applied
            })
        }) {
            // A retry never re-runs an action that this exact pinned plan
            // already applied successfully.
            receipts.push(applied.clone());
            continue;
        }
        if !prepared.selected.contains(&action.action_id) {
            receipts.push(DependencyApplyReceipt {
                action_id: action.action_id.clone(),
                status: ActionStatus::Skipped,
                applied_at: prepared.applied_at,
                error: None,
            });
            continue;
        }
        let resource = prepared.resources.get(&action.resource_id).ok_or_else(|| {
            format!(
                "dependency action '{}' references a missing bundle resource",
                action.action_id
            )
        })?;
        let result = execute_dependency_action(
            repository,
            &prepared.plan,
            &prepared.binding,
            action,
            resource,
        )
        .await;
        receipts.push(DependencyApplyReceipt {
            action_id: action.action_id.clone(),
            status: if result.is_ok() {
                ActionStatus::Applied
            } else {
                ActionStatus::Failed
            },
            applied_at: prepared.applied_at,
            error: result.err(),
        });
    }

    let finalize_repository = repository.clone();
    run_blocking(move || finalize_dependency_application(&finalize_repository, prepared, receipts))
        .await
}

fn finalize_dependency_application(
    repository: &LocalStateRepository,
    prepared: PreparedDependencyApplication,
    receipts: Vec<DependencyApplyReceipt>,
) -> Result<DependencyResult, String> {
    let PreparedDependencyApplication {
        plan,
        binding,
        applied_at,
        ..
    } = prepared;
    let record = DependencyApplicationRecord {
        plan_id: plan.plan_id.clone(),
        local_project_id: binding.local_project_id,
        storage_id: plan.storage_id.clone(),
        bundle_id: plan.bundle_id.clone(),
        host_id: binding.host_id,
        replica_id: binding.replica_id,
        revision: plan.revision,
        commit_id: plan.commit_id.clone(),
        manifest_sha256: plan.manifest_sha256.clone(),
        binding_revision: plan.binding_revision,
        applied_at,
        receipts: receipts.clone(),
    };
    repository.mutate_dependency_applications(|_, applications| {
        if let Some(current) = applications
            .records
            .iter_mut()
            .find(|current| current.plan_id == record.plan_id)
        {
            if current.local_project_id != record.local_project_id
                || current.storage_id != record.storage_id
                || current.bundle_id != record.bundle_id
                || current.host_id != record.host_id
                || current.replica_id != record.replica_id
                || current.revision != record.revision
                || current.commit_id != record.commit_id
                || current.manifest_sha256 != record.manifest_sha256
                || current.binding_revision != record.binding_revision
            {
                return Err(
                    "dependency receipt scope changed while actions were running".to_string(),
                );
            }
            *current = record;
        } else {
            applications.records.push(record);
        }
        Ok(())
    })?;
    let applied_action_ids = receipts
        .iter()
        .filter(|receipt| receipt.status == ActionStatus::Applied)
        .map(|receipt| receipt.action_id.clone())
        .collect::<Vec<_>>();
    let failed_actions = receipts
        .iter()
        .filter(|receipt| receipt.status == ActionStatus::Failed)
        .map(|receipt| FailedAction {
            action_id: receipt.action_id.clone(),
            display_name: plan
                .actions
                .iter()
                .find(|action| action.action_id == receipt.action_id)
                .map(|action| action.display_name.clone()),
            message: receipt
                .error
                .clone()
                .unwrap_or_else(|| "Dependency action failed".to_string()),
        })
        .collect::<Vec<_>>();
    let success = failed_actions.is_empty();
    Ok(DependencyResult {
        success,
        message: if success {
            format!("Applied {} dependency actions", applied_action_ids.len())
        } else {
            format!(
                "Applied {} dependencies; {} failed",
                applied_action_ids.len(),
                failed_actions.len()
            )
        },
        applied_action_ids,
        failed_actions,
    })
}
```

## Dependency validation + native plugin installers (lines 9407–9742)
```rust
fn unique_dependency_actions(
    requested: &[ActionId],
    actions: &[DependencyAction],
) -> Result<BTreeSet<ActionId>, String> {
    let selected = requested.iter().cloned().collect::<BTreeSet<_>>();
    if selected.len() != requested.len() {
        return Err("approved dependency actions contain duplicates".to_string());
    }
    let available = actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = selected.iter().find(|id| !available.contains(*id)) {
        return Err(format!("dependency plan has no action '{}'", unknown));
    }
    Ok(selected)
}

fn validate_dependency_plan_pin(
    plan: &DependencyPlan,
    snapshot: &BundleSnapshot,
    binding: &ProjectBinding,
) -> Result<(), String> {
    if plan.storage_id != snapshot.storage_id
        || plan.bundle_id != snapshot.head.bundle_id
        || plan.bundle_id != binding.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.revision != snapshot.head.revision
        || plan.commit_id != snapshot.head.commit_id
        || plan.manifest_sha256 != snapshot.head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err("dependency plan no longer matches the bundle or binding".to_string());
    }
    Ok(())
}

async fn execute_dependency_action(
    repository: &LocalStateRepository,
    plan: &DependencyPlan,
    binding: &ProjectBinding,
    action: &DependencyAction,
    resource: &ResourceDescriptor,
) -> Result<(), String> {
    action.validate()?;
    match action.kind {
        DependencyActionKind::InstallStandaloneSkill => {
            let materialized = repository
                .load_materializations()?
                .records
                .iter()
                .any(|record| {
                    record.host_id == binding.host_id
                        && record.replica_id == binding.replica_id
                        && record.bundle_id == plan.bundle_id
                        && record.revision == plan.revision
                        && record.manifest_sha256 == plan.manifest_sha256
                        && record.binding_revision == binding.revision
                        && record.receipts.iter().any(|receipt| {
                            receipt.resource_id == action.resource_id
                                && receipt.status == ActionStatus::Applied
                        })
                });
            if materialized {
                Ok(())
            } else {
                Err("restore the standalone-skill payload before approving it".to_string())
            }
        }
        DependencyActionKind::InstallCodexPlugin => {
            run_codex_plugin_install(action, binding, resource).await
        }
        DependencyActionKind::InstallClaudePlugin => {
            run_plugin_install("claude", action, binding).await
        }
        DependencyActionKind::CheckBinary
        | DependencyActionKind::CheckEnvironment
        | DependencyActionKind::Manual => {
            Err("this dependency requires a provider-specific manual check".to_string())
        }
    }
}

fn validate_plugin_install_intent(action: &DependencyAction) -> Result<(), String> {
    if !portable_plugin_id(&action.display_name) {
        return Err("plugin identifier is not safe for native installation".to_string());
    }
    // Global plugins install without a scope flag. Anything else is rejected
    // before a process is launched.
    let supported_argument_sets: Vec<Vec<String>> = match action.kind {
        DependencyActionKind::InstallCodexPlugin => vec![vec![
            "plugin".to_string(),
            "add".to_string(),
            action.display_name.clone(),
        ]],
        DependencyActionKind::InstallClaudePlugin => vec![vec![
            "plugin".to_string(),
            "install".to_string(),
            action.display_name.clone(),
        ]],
        _ => return Err("dependency is not a plugin installation".to_string()),
    };
    if !supported_argument_sets.contains(&action.argv) {
        return Err("plugin install arguments differ from the supported intent".to_string());
    }
    Ok(())
}

fn codex_plugin_lock_for_action(
    action: &DependencyAction,
    resource: &ResourceDescriptor,
) -> Result<codex_plugins::CodexPluginLock, String> {
    validate_plugin_install_intent(action)?;
    if action.kind != DependencyActionKind::InstallCodexPlugin
        || resource.resource_id != action.resource_id
        || resource.kind != ResourceKind::Plugin
        || resource.provider != Some(Provider::Codex)
    {
        return Err("Codex plugin dependency does not match its bundle resource".to_string());
    }
    let (_, marketplace) = action
        .display_name
        .rsplit_once('@')
        .ok_or_else(|| "Codex plugin identifier has no marketplace".to_string())?;
    if resource
        .metadata
        .get("plugin_marketplace")
        .is_some_and(|captured| captured != marketplace)
    {
        return Err("Codex plugin marketplace metadata does not match its identifier".to_string());
    }

    let mut lock = codex_plugins::empty_lock();
    lock.plugins.push(codex_plugins::CodexPluginIntent {
        id: action.display_name.clone(),
        observed_version: resource.metadata.get("plugin_observed_version").cloned(),
    });
    match resource
        .metadata
        .get("plugin_source_type")
        .map(String::as_str)
    {
        Some("git") => {
            let source = resource.metadata.get("plugin_source").ok_or_else(|| {
                format!(
                    "Codex plugin '{}' is missing its portable marketplace source",
                    action.display_name
                )
            })?;
            lock.marketplaces
                .push(codex_plugins::CodexMarketplaceIntent {
                    name: marketplace.to_string(),
                    repository: source.clone(),
                    git_ref: None,
                });
        }
        Some("local") | None => {}
        Some(source_type) => {
            return Err(format!(
                "Codex plugin '{}' has unsupported marketplace source type '{}'",
                action.display_name, source_type
            ));
        }
    }
    codex_plugins::validate_lock(&lock)?;
    Ok(lock)
}

// Dependency receipt errors are capped at 8 KiB. Reserve room for the plugin
// identifier and the stable error prefix wrapped around these diagnostics.
const MAX_PLUGIN_DIAGNOSTIC_BYTES: usize = 7 * 1024;

fn append_plugin_diagnostic(diagnostics: &mut String, level: &str, message: &str) {
    if diagnostics.len() >= MAX_PLUGIN_DIAGNOSTIC_BYTES {
        return;
    }
    let message = codex_plugins::redact(message.trim());
    if message.is_empty() {
        return;
    }
    let line = format!("{}: {}\n", level, message);
    let remaining = MAX_PLUGIN_DIAGNOSTIC_BYTES - diagnostics.len();
    let mut end = line.len().min(remaining);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    diagnostics.push_str(&line[..end]);
}

fn plugin_failure_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let mut diagnostics = String::new();
    for line in String::from_utf8_lossy(stderr)
        .lines()
        .chain(String::from_utf8_lossy(stdout).lines())
    {
        append_plugin_diagnostic(&mut diagnostics, "installer", line);
    }
    diagnostics.trim().to_string()
}

async fn run_codex_plugin_install(
    action: &DependencyAction,
    binding: &ProjectBinding,
    resource: &ResourceDescriptor,
) -> Result<(), String> {
    let lock = codex_plugin_lock_for_action(action, resource)?;
    let plugin_id = action.display_name.clone();
    let target_home = binding
        .codex_home
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "Codex plugin installation requires a configured Codex home".to_string())?;
    let default_home = dirs::home_dir()
        .map(|home| home.join(".codex"))
        .ok_or_else(|| "could not resolve the default Codex home".to_string())?;

    tokio::task::spawn_blocking(move || {
        let codex = codex_plugins::find_binary("codex")?;
        let target_runner = codex_plugins::ProcessRunner::for_binary(codex.clone())
            .with_codex_home(Some(target_home.clone()));
        let default_runner = codex_plugins::ProcessRunner::for_binary(codex)
            .with_codex_home(Some(default_home.clone()));
        let mut diagnostics = String::new();
        let mut log =
            |level: &str, message: &str| append_plugin_diagnostic(&mut diagnostics, level, message);
        let repair = codex_plugins::apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        );
        drop(log);
        let report = repair.map_err(|error| {
            append_plugin_diagnostic(&mut diagnostics, "error", &error);
            format!(
                "codex plugin '{}' setup failed: {}",
                plugin_id,
                diagnostics.trim()
            )
        })?;
        let satisfied = report
            .plugins_installed
            .iter()
            .chain(&report.already_present)
            .any(|installed| installed == &plugin_id);
        let failed = report.failed.iter().any(|failed| failed == &plugin_id);
        let blocked = report
            .blocked_plugins
            .iter()
            .any(|blocked| blocked.id == plugin_id);
        if satisfied && !failed && !blocked {
            return Ok(());
        }
        let detail = diagnostics.trim();
        Err(if detail.is_empty() {
            format!(
                "codex plugin '{}' was not present after installation",
                plugin_id
            )
        } else {
            format!("codex plugin '{}' was not installed: {}", plugin_id, detail)
        })
    })
    .await
    .map_err(|error| format!("Codex plugin installer task failed: {}", error))?
}

async fn run_plugin_install(
    program: &str,
    action: &DependencyAction,
    binding: &ProjectBinding,
) -> Result<(), String> {
    validate_plugin_install_intent(action)?;
    let current_dir = match program {
        "claude" => binding.claude_home.as_deref(),
        _ => binding.codex_home.as_deref(),
    }
    .map(PathBuf::from)
    .or_else(dirs::home_dir)
    .ok_or_else(|| {
        format!(
            "could not resolve a working directory for the {} plugin installer",
            program
        )
    })?;
    let mut command = Command::new(program);
    command
        .args(&action.argv)
        .current_dir(&current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(codex_home) = &binding.codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    if let Some(claude_home) = &binding.claude_home {
        command.env("CLAUDE_CONFIG_DIR", claude_home);
    }
    let output = timeout(DEFAULT_OPERATION_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{} plugin installation timed out", program))?
        .map_err(|error| format!("start {} plugin installer: {}", program, error))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = plugin_failure_detail(&output.stdout, &output.stderr);
        let status = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated".to_string());
        Err(if detail.is_empty() {
            format!("{} plugin installer exited with status {}", program, status)
        } else {
            format!(
                "{} plugin installer exited with status {}: {}",
                program, status, detail
            )
        })
    }
}

fn portable_plugin_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.starts_with('-')
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'/' | b'.' | b'_' | b'-' | b':')
        })
}

```

## capture_request_for_binding global-inventory wiring + capture_request_with_global_inventory (lines 9778–9859)
```rust
fn capture_request_for_binding(
    repository: &LocalStateRepository,
    binding: &ProjectBinding,
) -> Result<CaptureRequest, String> {
    let binding = resolve_project_binding(repository, binding)?;
    let resolved_project = fs_canonicalize(Path::new(&binding.project_root))?;
    if resolved_project != PathBuf::from(&binding.canonical_project_root) {
        return Err("project binding resolves to a different checkout".to_string());
    }
    let excluded_project_roots = repository
        .load_bindings()?
        .bindings
        .into_iter()
        .filter(|other| {
            other.state == BindingState::Active && other.replica_id != binding.replica_id
        })
        .map(|other| PathBuf::from(other.canonical_project_root))
        .filter(|root| root.starts_with(&resolved_project) && root != &resolved_project)
        .collect();
    let mut request = capture_request_with_global_inventory(
        resolved_project,
        binding.codex_home.as_ref().map(PathBuf::from),
        binding.claude_home.as_ref().map(PathBuf::from),
        excluded_project_roots,
    );
    let config = repository.load_config()?;
    request
        .excluded_content_roots
        .push(repository.root().to_path_buf());
    request.excluded_content_roots.extend(
        config
            .storages
            .iter()
            .filter(|storage| storage.kind == StorageKind::Local)
            .map(|storage| PathBuf::from(&storage.local_dir)),
    );
    if let Some(home) = &binding.codex_home {
        request.excluded_content_roots.push(PathBuf::from(home));
    }
    if let Some(home) = &binding.claude_home {
        request.excluded_content_roots.push(PathBuf::from(home));
    }
    Ok(request)
}

/// Build a capture request whose global plugin and custom-skill candidates
/// come from an ownership-ordered inventory of the mapped provider homes.
/// Discovery keeps every global resource unselected by default; the recipe
/// remains the only selection authority.
fn capture_request_with_global_inventory(
    project_root: PathBuf,
    codex_home: Option<PathBuf>,
    claude_home: Option<PathBuf>,
    excluded_project_roots: Vec<PathBuf>,
) -> CaptureRequest {
    let mut standalone_skills = Vec::new();
    let mut global_plugins = Vec::new();
    let mut blocked_global_skills = Vec::new();
    for (provider, home) in [
        (CaptureProvider::Codex, codex_home.as_deref()),
        (CaptureProvider::Claude, claude_home.as_deref()),
    ] {
        let Some(home) = home else { continue };
        let inventory = global_inventory::inventory_provider_home(provider, home);
        standalone_skills.extend(inventory.standalone_skills);
        global_plugins.extend(inventory.plugins);
        blocked_global_skills.extend(inventory.blocked_skills);
    }
    CaptureRequest {
        project_root,
        codex_home,
        claude_home,
        excluded_project_roots,
        standalone_skills,
        global_plugins,
        blocked_global_skills,
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
```

## commands.rs: bundle-readiness dependency verification (pre-removal lines ~8061–8146)
```rust
    let dependency_applications = repository.load_dependency_applications()?;
    let mut applied_dependencies = BTreeSet::new();
    for application in dependency_applications
        .records
        .iter()
        .filter(|application| {
            application.host_id == binding.host_id
                && application.replica_id == binding.replica_id
                && application.bundle_id == *bundle_id
                && application.revision == head.revision
                && application.manifest_sha256 == head.manifest_sha256
                && application.binding_revision == binding.revision
        })
    {
        applied_dependencies.extend(
            application
                .receipts
                .iter()
                .filter(|receipt| receipt.status == ActionStatus::Applied)
                .map(|receipt| receipt.action_id.clone()),
        );
    }
    let mut issues = Vec::new();
    let needs_codex_home = snapshot
        .manifest
        .resources
        .values()
        .any(|resource| resource.kind == ResourceKind::CodexConversation);
    let needs_claude_home = snapshot
        .manifest
        .resources
        .values()
        .any(|resource| resource.kind == ResourceKind::ClaudeConversation);
    add_provider_home_issue(
        &mut issues,
        Provider::Codex,
        needs_codex_home,
        binding.codex_home.as_deref(),
    );
    add_provider_home_issue(
        &mut issues,
        Provider::Claude,
        needs_claude_home,
        binding.claude_home.as_deref(),
    );
    if !snapshot.manifest.files.is_empty() {
        match materialization.as_ref() {
            None => issues.push(BundleReadinessIssue {
                issue_id: "restore-files".to_string(),
                category: "project_files".to_string(),
                title: "Pull has not been applied yet".to_string(),
                detail: Some(
                    "Open the Pull review and apply the changes you want on this machine."
                        .to_string(),
                ),
                severity: "warning".to_string(),
                provider: None,
                resource_id: None,
                action: Some("plan_restore".to_string()),
            }),
            Some(record) if record.status == MaterializationStatus::Partial => {
                add_partial_materialization_issues(&mut issues, record, snapshot);
            }
            Some(_) => {}
        }
    }
    for action in &dependencies_from_manifest(&snapshot.manifest)? {
        if !applied_dependencies.contains(&action.action_id) {
            issues.push(BundleReadinessIssue {
                issue_id: format!("dependency-{}", action.action_id),
                category: match action.kind {
                    DependencyActionKind::InstallCodexPlugin
                    | DependencyActionKind::InstallClaudePlugin => "plugins",
                    DependencyActionKind::InstallStandaloneSkill => "skills",
                    _ => "tools",
                }
                .to_string(),
                title: format!("{} needs setup", action.display_name),
                detail: Some("Review and approve the pinned dependency action.".to_string()),
                severity: "warning".to_string(),
                provider: action.provider,
                resource_id: Some(action.resource_id.clone()),
                action: Some("apply_dependency".to_string()),
            });
        }
    }
```

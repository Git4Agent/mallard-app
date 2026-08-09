//! Per-registration/link reconciliation and immutable review construction.
//!
//! Every Push and Pull plan addresses exactly one remote Library repository.
//! Fork addresses exactly one local registration. No function in this module
//! accepts a storage-wide resource set.

use super::adapters::adapter_for;
use super::domain::{
    pet_resource_id, skill_resource_id, CatalogClaimPin, CatalogForkSourceKind, CatalogLinkState,
    CatalogPlanActionId, CatalogPlanId, CatalogPreservationPin, CatalogPullAction,
    CatalogPullActionKind, CatalogPullDisposition, CatalogPullItem, CatalogPushRow,
    CatalogResource, CatalogResourceForkPlan, CatalogResourceId, CatalogResourceLink,
    CatalogResourcePullPlan, CatalogResourcePushPlan, CatalogResourceRegistration,
    CatalogTargetExpectation, LibraryHeadCoordinate, LocalCatalogTargetId, PluginApplyMode,
    CATALOG_SCHEMA, MAX_GENERATED_NAME_ATTEMPTS,
};
use super::inventory::{CatalogLocalInventory, LocalResourceRecord};
use super::profiles::ResolvedCatalogContext;
use mallard_domain::StorageKind;
use std::path::PathBuf;

pub const CATALOG_RESOURCE_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

pub fn relation_for_state(state: CatalogLinkState) -> super::domain::CatalogResourceRelation {
    use super::domain::CatalogResourceRelation as Relation;
    match state {
        CatalogLinkState::New => Relation::LocalOnly,
        CatalogLinkState::InSync => Relation::Matching,
        CatalogLinkState::LocalChanged => Relation::LocalChanged,
        CatalogLinkState::StorageChanged => Relation::StorageChanged,
        CatalogLinkState::Diverged => Relation::ChangedBoth,
        CatalogLinkState::LocalMissing | CatalogLinkState::UnreviewedRemote => {
            Relation::StorageOnly
        }
        CatalogLinkState::RemoteMissing => Relation::LocalOnly,
        CatalogLinkState::Blocked => Relation::Blocked,
    }
}

pub fn derive_link_state(
    accepted_content_sha256: Option<&str>,
    local_content_sha256: Option<&str>,
    remote_content_sha256: Option<&str>,
    remote_was_accepted: bool,
) -> CatalogLinkState {
    match (
        accepted_content_sha256,
        local_content_sha256,
        remote_content_sha256,
    ) {
        (None, Some(_), None) => CatalogLinkState::New,
        (None, None, None) => CatalogLinkState::LocalMissing,
        (None, local, Some(remote)) => {
            if local == Some(remote) {
                CatalogLinkState::InSync
            } else {
                CatalogLinkState::UnreviewedRemote
            }
        }
        (Some(_), None, Some(_)) => CatalogLinkState::LocalMissing,
        (Some(_), Some(_), None) | (Some(_), None, None) => {
            if remote_was_accepted {
                CatalogLinkState::RemoteMissing
            } else {
                CatalogLinkState::New
            }
        }
        (Some(_), Some(local), Some(remote)) if local == remote => CatalogLinkState::InSync,
        (Some(base), Some(local), Some(remote)) if local == base && remote != base => {
            CatalogLinkState::StorageChanged
        }
        (Some(base), Some(local), Some(remote)) if local != base && remote == base => {
            CatalogLinkState::LocalChanged
        }
        (Some(_), Some(_), Some(_)) => CatalogLinkState::Diverged,
    }
}

fn scope_label(resource: &CatalogResource) -> Option<String> {
    match resource {
        CatalogResource::AgentSkill(skill) => {
            Some(skill.descriptor.scope.display_label().to_string())
        }
        CatalogResource::PluginInstallIntent(_) => None,
        CatalogResource::CodexPetV1(_) => Some("v1".to_string()),
        CatalogResource::CodexPetV2(_) => Some("v2".to_string()),
    }
}

pub fn target_for_resource(
    context: &ResolvedCatalogContext,
    resource: &CatalogResource,
) -> Result<(LocalCatalogTargetId, Option<String>), String> {
    match resource {
        CatalogResource::AgentSkill(skill) => {
            let root = context.skill_root(skill.descriptor.scope).ok_or_else(|| {
                format!(
                    "no {} skill root is active for this profile",
                    skill.descriptor.scope.display_label()
                )
            })?;
            Ok((
                LocalCatalogTargetId::SkillDirectory {
                    scope: skill.descriptor.scope,
                    canonical_root: root.to_string_lossy().to_string(),
                    install_directory: skill.descriptor.install_directory.clone(),
                },
                Some(
                    root.join(&skill.descriptor.install_directory)
                        .to_string_lossy()
                        .to_string(),
                ),
            ))
        }
        CatalogResource::PluginInstallIntent(plugin) => {
            let descriptor = &plugin.descriptor;
            let profile = context.profile(descriptor.provider).ok_or_else(|| {
                format!(
                    "no {} profile is active",
                    descriptor.provider.display_label()
                )
            })?;
            Ok((
                LocalCatalogTargetId::Plugin {
                    provider: descriptor.provider,
                    canonical_home: profile.canonical_home.to_string_lossy().to_string(),
                    namespace: descriptor.provider_native_namespace.clone(),
                    plugin_id: descriptor.plugin_id.clone(),
                },
                None,
            ))
        }
        CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => {
            let profile = context
                .codex
                .as_ref()
                .ok_or("no Codex profile is active for this pet")?;
            Ok((
                LocalCatalogTargetId::PetDirectory {
                    canonical_home: profile.canonical_home.to_string_lossy().to_string(),
                    pet_id: pet.descriptor.pet_id.clone(),
                },
                Some(
                    profile
                        .canonical_home
                        .join("pets")
                        .join(&pet.descriptor.pet_id)
                        .to_string_lossy()
                        .to_string(),
                ),
            ))
        }
    }
}

pub fn generate_copy_name(
    base_name: &str,
    is_taken: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    let mut stem = base_name.to_string();
    while stem.len() > 96 {
        stem.pop();
    }
    let first = format!("{}-local-copy", stem);
    if !is_taken(&first) {
        return Ok(first);
    }
    for attempt in 2..=MAX_GENERATED_NAME_ATTEMPTS {
        let candidate = format!("{}-{}", first, attempt);
        if !is_taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not generate an unused copy name for '{}'",
        base_name
    ))
}

pub fn transformed_copy_identity(
    record: &LocalResourceRecord,
    copy_name: &str,
) -> Result<(CatalogResourceId, String), String> {
    match (&record.target, &record.skill_tree, &record.pet) {
        (LocalCatalogTargetId::SkillDirectory { scope, .. }, Some(tree), _) => {
            let renamed = super::skill_tree::rename_skill_tree(&tree.files, copy_name)?;
            let resource = super::skill_tree::skill_resource_from_tree(
                *scope, copy_name, copy_name, &renamed,
            )?;
            Ok((
                skill_resource_id(*scope, copy_name, copy_name),
                resource.content_sha256,
            ))
        }
        (LocalCatalogTargetId::PetDirectory { .. }, _, Some(pet)) => {
            let renamed = super::codex_pet::renamed_pet(pet, copy_name)?;
            let resource = super::codex_pet::pet_resource_from_capture(&renamed)?;
            Ok((
                pet_resource_id(pet.format, copy_name),
                resource.content_sha256,
            ))
        }
        _ => Err("the copy source has no captured payload".to_string()),
    }
}

pub struct ResourcePushPlanInputs<'a> {
    pub registration: &'a CatalogResourceRegistration,
    pub link: &'a CatalogResourceLink,
    pub storage_kind: StorageKind,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub state: CatalogLinkState,
    pub observed_head: Option<LibraryHeadCoordinate>,
    pub now: u64,
}

pub fn build_resource_push_plan(
    inputs: ResourcePushPlanInputs<'_>,
) -> Result<super::domain::CatalogResourcePushPlan, String> {
    let record = inputs
        .inventory
        .resources
        .get(&inputs.registration.resource_id)
        .ok_or("the linked resource is missing locally and cannot be pushed")?;
    if record.target != inputs.registration.local_target
        || record.resource.provider() != inputs.registration.provider
        || record.resource.kind_tag() != inputs.registration.resource_kind
    {
        return Err("the local resource no longer matches its registration".to_string());
    }
    let selectable = matches!(
        inputs.state,
        CatalogLinkState::New | CatalogLinkState::LocalChanged
    );
    let blocked_reason = match inputs.state {
        CatalogLinkState::StorageChanged
        | CatalogLinkState::Diverged
        | CatalogLinkState::UnreviewedRemote => {
            Some("Storage changed; review Pull before pushing".to_string())
        }
        CatalogLinkState::RemoteMissing if inputs.link.accepted_head.is_some() => {
            Some("The accepted remote repository is missing".to_string())
        }
        CatalogLinkState::Blocked => Some("This link is blocked".to_string()),
        CatalogLinkState::LocalMissing => Some("The local resource is missing".to_string()),
        _ => None,
    };
    let row = CatalogPushRow {
        resource_id: record.resource.resource_id(),
        kind: record.resource.kind_tag(),
        display_name: record.resource.display_name(),
        provider: record.resource.provider(),
        scope_label: scope_label(&record.resource),
        relation: relation_for_state(inputs.state),
        selectable,
        selected_by_default: selectable,
        held_for_pull: blocked_reason.is_some(),
        removal_candidate: false,
        local_content_sha256: Some(record.resource.content_sha256().to_string()),
        storage_content_sha256: inputs.link.accepted_content_sha256.as_ref().cloned(),
        blocked_reason,
        warnings: record.warnings.clone(),
        detail: Some(match inputs.storage_kind {
            StorageKind::Local => "Publish this resource to the linked local storage".to_string(),
            StorageKind::S3 => "Publish this resource to the linked cloud storage".to_string(),
        }),
    };
    let plan = CatalogResourcePushPlan {
        schema_version: CATALOG_SCHEMA,
        plan_id: CatalogPlanId::generate()?,
        registration_id: inputs.registration.registration_id.clone(),
        link_id: inputs.link.link_id.clone(),
        registration_revision: inputs.registration.revision,
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        observed_head: inputs.observed_head,
        source_stamp: inputs.inventory.source_stamp.clone(),
        resource: record.resource.clone(),
        warning_digests: record
            .warnings
            .iter()
            .map(|warning| warning.digest.clone())
            .collect(),
        row,
        created_at: inputs.now,
        expires_at: inputs
            .now
            .saturating_add(CATALOG_RESOURCE_PLAN_LIFETIME_SECS),
    };
    plan.validate()?;
    Ok(plan)
}

fn preservation_pin(
    occupant: &LocalResourceRecord,
    storage_digest: &str,
    source_name: &str,
    requested_copy_name: Option<&str>,
    name_taken: &dyn Fn(&LocalCatalogTargetId, &str) -> bool,
) -> Result<CatalogPreservationPin, String> {
    let taken = |candidate: &str| name_taken(&occupant.target, candidate);
    let copy_name = match requested_copy_name {
        Some(requested) => {
            super::domain::validate_component_name("preservation copy name", requested, 128)?;
            if taken(requested) {
                return Err(format!(
                    "the copy name '{}' is already in use; choose another",
                    requested
                ));
            }
            requested.to_string()
        }
        None => generate_copy_name(source_name, &taken)?,
    };
    let (copy_resource_id, transformed_content_sha256) =
        transformed_copy_identity(occupant, &copy_name)?;
    let copy_target_path = match &occupant.target {
        LocalCatalogTargetId::SkillDirectory { canonical_root, .. } => {
            PathBuf::from(canonical_root).join(&copy_name)
        }
        LocalCatalogTargetId::PetDirectory { canonical_home, .. } => {
            PathBuf::from(canonical_home).join("pets").join(&copy_name)
        }
        LocalCatalogTargetId::Plugin { .. } => {
            return Err("plugins never use preservation copies".to_string())
        }
    };
    Ok(CatalogPreservationPin {
        copy_name,
        copy_resource_id,
        copy_target_path: copy_target_path.to_string_lossy().to_string(),
        transformed_content_sha256,
        original_content_sha256: occupant.resource.content_sha256().to_string(),
        storage_content_sha256: storage_digest.to_string(),
        reusable_witness: false,
    })
}

pub struct ResourcePullPlanInputs<'a> {
    pub registration: &'a CatalogResourceRegistration,
    pub link: &'a CatalogResourceLink,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub remote_resource: &'a CatalogResource,
    pub head: LibraryHeadCoordinate,
    pub state: CatalogLinkState,
    pub conflicting_claims: Vec<CatalogClaimPin>,
    pub requested_copy_name: Option<&'a str>,
    pub name_taken: &'a dyn Fn(&LocalCatalogTargetId, &str) -> bool,
    pub now: u64,
}

pub fn build_resource_pull_plan(
    inputs: ResourcePullPlanInputs<'_>,
) -> Result<CatalogResourcePullPlan, String> {
    if inputs.remote_resource.resource_id() != inputs.registration.resource_id
        || inputs.remote_resource.provider() != inputs.registration.provider
        || inputs.remote_resource.kind_tag() != inputs.registration.resource_kind
    {
        return Err("the remote resource does not match its registration".to_string());
    }
    let local = inputs
        .inventory
        .resources
        .get(&inputs.registration.resource_id);
    let mut item = CatalogPullItem {
        resource_id: inputs.remote_resource.resource_id(),
        kind: inputs.remote_resource.kind_tag(),
        display_name: inputs.remote_resource.display_name(),
        provider: inputs.remote_resource.provider(),
        scope_label: scope_label(inputs.remote_resource),
        relation: relation_for_state(inputs.state),
        disposition: CatalogPullDisposition::KeptLocal,
        actions: Vec::new(),
        default_action_id: None,
        detail: None,
    };

    match inputs.remote_resource {
        CatalogResource::PluginInstallIntent(plugin) => {
            build_plugin_item(&inputs, &mut item, plugin, local)?;
        }
        CatalogResource::AgentSkill(_) => {
            build_directory_item(&inputs, &mut item, local, false)?;
        }
        CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => {
            let occupant = inputs.inventory.resources.values().find(|record| {
                matches!(
                    &record.target,
                    LocalCatalogTargetId::PetDirectory { pet_id, .. }
                        if pet_id == &pet.descriptor.pet_id
                )
            });
            let cross_format = occupant.is_some_and(|record| {
                record.resource.pet_format() != inputs.remote_resource.pet_format()
            });
            build_directory_item(
                &inputs,
                &mut item,
                if cross_format { occupant } else { local },
                cross_format,
            )?;
        }
    }

    let plan = CatalogResourcePullPlan {
        schema_version: CATALOG_SCHEMA,
        plan_id: CatalogPlanId::generate()?,
        registration_id: inputs.registration.registration_id.clone(),
        link_id: inputs.link.link_id.clone(),
        registration_revision: inputs.registration.revision,
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        head: inputs.head,
        resource: inputs.remote_resource.clone(),
        item,
        conflicting_claims: inputs.conflicting_claims,
        created_at: inputs.now,
        expires_at: inputs
            .now
            .saturating_add(CATALOG_RESOURCE_PLAN_LIFETIME_SECS),
    };
    plan.validate()?;
    Ok(plan)
}

fn push_action(
    item: &mut CatalogPullItem,
    kind: CatalogPullActionKind,
    destructive: bool,
    target: &LocalCatalogTargetId,
    target_path: Option<&str>,
    expected: super::domain::CatalogTargetExpectation,
    incoming_digest: &str,
) -> Result<CatalogPlanActionId, String> {
    let action_id = CatalogPlanActionId::generate()?;
    item.actions.push(CatalogPullAction {
        action_id: action_id.clone(),
        resource_id: item.resource_id.clone(),
        kind,
        requires_destructive_approval: destructive,
        target: target.clone(),
        target_path: target_path.map(str::to_string),
        expected_target_state: expected,
        incoming_content_sha256: incoming_digest.to_string(),
    });
    Ok(action_id)
}

fn build_directory_item(
    inputs: &ResourcePullPlanInputs<'_>,
    item: &mut CatalogPullItem,
    occupant: Option<&LocalResourceRecord>,
    cross_format: bool,
) -> Result<(), String> {
    let (target, target_path) = target_for_resource(inputs.context, inputs.remote_resource)?;
    let target_path = target_path.ok_or("directory resource has no target path")?;
    let incoming_digest = inputs.remote_resource.content_sha256();
    let local_digest = occupant.map(|record| record.resource.content_sha256());
    let is_pet = inputs.remote_resource.pet_format().is_some();
    match inputs.state {
        CatalogLinkState::InSync => item.disposition = CatalogPullDisposition::AlreadyMatching,
        CatalogLinkState::LocalChanged
        | CatalogLinkState::New
        | CatalogLinkState::RemoteMissing => {
            item.disposition = CatalogPullDisposition::KeptLocal;
            item.detail = Some("Your local version stays unchanged".to_string());
        }
        CatalogLinkState::LocalMissing | CatalogLinkState::UnreviewedRemote
            if occupant.is_none() =>
        {
            item.disposition = CatalogPullDisposition::Executable;
            let kind = if is_pet {
                CatalogPullActionKind::InstallPet
            } else {
                CatalogPullActionKind::InstallSkill
            };
            let action_id = push_action(
                item,
                kind,
                false,
                &target,
                Some(&target_path),
                CatalogTargetExpectation::Absent,
                incoming_digest,
            )?;
            item.default_action_id = Some(action_id);
        }
        CatalogLinkState::StorageChanged => {
            let Some(local_digest) = local_digest else {
                item.disposition = CatalogPullDisposition::Blocked {
                    reason: "the registered local target could not be observed".to_string(),
                };
                return Ok(());
            };
            item.disposition = CatalogPullDisposition::Executable;
            let kind = if is_pet {
                CatalogPullActionKind::UpdatePet
            } else {
                CatalogPullActionKind::UpdateSkill
            };
            let action_id = push_action(
                item,
                kind,
                false,
                &target,
                Some(&target_path),
                CatalogTargetExpectation::Sha256 {
                    sha256: local_digest.to_string(),
                },
                incoming_digest,
            )?;
            item.default_action_id = Some(action_id);
        }
        CatalogLinkState::Diverged
        | CatalogLinkState::UnreviewedRemote
        | CatalogLinkState::LocalMissing
            if occupant.is_some() || cross_format =>
        {
            let occupant = occupant.ok_or("the conflicting local resource disappeared")?;
            let local_digest = occupant.resource.content_sha256();
            item.disposition = CatalogPullDisposition::Executable;
            let pin = preservation_pin(
                occupant,
                incoming_digest,
                &occupant.resource.display_name(),
                inputs.requested_copy_name,
                inputs.name_taken,
            )?;
            item.detail = Some(format!(
                "Keep the local version as '{}' and overwrite with the stored {} resource",
                pin.copy_name,
                inputs
                    .remote_resource
                    .pet_format()
                    .map_or("skill", |format| format.display_label())
            ));
            let preserve = if is_pet {
                CatalogPullActionKind::PreservePetAsCopyAndOverwrite { copy: pin }
            } else {
                CatalogPullActionKind::PreserveSkillAsCopyAndOverwrite { copy: pin }
            };
            let action_id = push_action(
                item,
                preserve,
                false,
                &target,
                Some(&target_path),
                CatalogTargetExpectation::Sha256 {
                    sha256: local_digest.to_string(),
                },
                incoming_digest,
            )?;
            let overwrite = if is_pet {
                CatalogPullActionKind::OverwritePetWithStorage
            } else {
                CatalogPullActionKind::OverwriteSkillWithStorage
            };
            push_action(
                item,
                overwrite,
                true,
                &target,
                Some(&target_path),
                CatalogTargetExpectation::Sha256 {
                    sha256: local_digest.to_string(),
                },
                incoming_digest,
            )?;
            item.default_action_id = Some(action_id);
        }
        CatalogLinkState::Blocked => {
            item.disposition = CatalogPullDisposition::Blocked {
                reason: "this resource link is blocked".to_string(),
            }
        }
        _ => {
            item.disposition = CatalogPullDisposition::Blocked {
                reason: "this link state cannot be pulled".to_string(),
            }
        }
    }
    Ok(())
}

fn build_plugin_item(
    inputs: &ResourcePullPlanInputs<'_>,
    item: &mut CatalogPullItem,
    plugin: &super::domain::PluginInstallIntentResource,
    local: Option<&LocalResourceRecord>,
) -> Result<(), String> {
    use super::domain::PluginResolutionPolicy;
    let descriptor = &plugin.descriptor;
    let locally_present = local.is_some();
    let (target, _) = target_for_resource(inputs.context, inputs.remote_resource)?;
    if descriptor.resolution == PluginResolutionPolicy::ManualOnly {
        item.disposition = CatalogPullDisposition::Manual {
            message: format!(
                "'{}' has no portable marketplace source; install it with the {} CLI",
                descriptor.plugin_id,
                descriptor.provider.display_label()
            ),
        };
        return Ok(());
    }
    let marketplace_registered = inputs
        .inventory
        .plugin_inventories
        .get(&descriptor.provider)
        .and_then(|inventory| inventory.marketplace(&descriptor.provider_native_namespace))
        .is_some();
    let adapter = adapter_for(descriptor.provider);
    let mode = if locally_present {
        PluginApplyMode::OverwriteExisting
    } else {
        PluginApplyMode::InstallMissing
    };
    match inputs.state {
        CatalogLinkState::InSync => {
            item.disposition = CatalogPullDisposition::AlreadyMatching;
            return Ok(());
        }
        CatalogLinkState::LocalChanged
        | CatalogLinkState::New
        | CatalogLinkState::RemoteMissing => {
            item.disposition = CatalogPullDisposition::KeptLocal;
            return Ok(());
        }
        CatalogLinkState::Blocked => {
            item.disposition = CatalogPullDisposition::Blocked {
                reason: "this plugin link is blocked".to_string(),
            };
            return Ok(());
        }
        _ => {}
    }
    let native = match adapter.plan_plugin_apply(descriptor, marketplace_registered, mode) {
        Ok(plan) => plan,
        Err(blocker) => {
            item.disposition = CatalogPullDisposition::Manual {
                message: blocker.to_string(),
            };
            return Ok(());
        }
    };
    let destructive = locally_present
        && matches!(
            inputs.state,
            CatalogLinkState::Diverged | CatalogLinkState::UnreviewedRemote
        );
    let kind = if destructive {
        CatalogPullActionKind::OverwritePluginWithStorage { plan: native }
    } else if locally_present {
        CatalogPullActionKind::UpdatePlugin { plan: native }
    } else {
        CatalogPullActionKind::InstallPlugin { plan: native }
    };
    item.disposition = CatalogPullDisposition::Executable;
    if destructive {
        item.detail = Some(
            "Overwrite reinstalls the stored intent through the provider; Skip keeps local"
                .to_string(),
        );
    }
    let action_id = push_action(
        item,
        kind,
        destructive,
        &target,
        None,
        local.map_or(CatalogTargetExpectation::Absent, |record| {
            CatalogTargetExpectation::Sha256 {
                sha256: record.resource.content_sha256().to_string(),
            }
        }),
        &plugin.content_sha256,
    )?;
    item.default_action_id = Some(action_id);
    Ok(())
}

pub fn validate_resource_pull_selection<'a>(
    plan: &'a CatalogResourcePullPlan,
    approved_action_id: Option<&CatalogPlanActionId>,
    destructive_acknowledged: bool,
) -> Result<Vec<&'a CatalogPullAction>, String> {
    let Some(action_id) = approved_action_id else {
        return Ok(Vec::new());
    };
    let action = plan
        .action(action_id)
        .ok_or("the selected action does not belong to this Pull review")?;
    if action.requires_destructive_approval && !destructive_acknowledged {
        return Err("the overwrite requires explicit acknowledgement".to_string());
    }
    Ok(vec![action])
}

pub struct ResourceForkPlanInputs<'a> {
    pub registration: &'a CatalogResourceRegistration,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub requested_name: &'a str,
    pub name_taken: &'a dyn Fn(&LocalCatalogTargetId, &str) -> bool,
    pub now: u64,
}

pub fn build_resource_fork_plan(
    inputs: ResourceForkPlanInputs<'_>,
) -> Result<CatalogResourceForkPlan, String> {
    let record = inputs
        .inventory
        .resources
        .get(&inputs.registration.resource_id)
        .ok_or("the Fork source is missing locally")?;
    if record.target != inputs.registration.local_target {
        return Err("the Fork source no longer matches its registration".to_string());
    }
    let source_kind = match &record.resource {
        CatalogResource::AgentSkill(_) => CatalogForkSourceKind::AgentSkill,
        CatalogResource::CodexPetV1(_) => CatalogForkSourceKind::CodexPetV1,
        CatalogResource::CodexPetV2(_) => CatalogForkSourceKind::CodexPetV2,
        CatalogResource::PluginInstallIntent(_) => {
            return Err("plugins do not support Fork".to_string())
        }
    };
    super::domain::validate_component_name("Fork copy name", inputs.requested_name, 128)?;
    if (inputs.name_taken)(&record.target, inputs.requested_name) {
        return Err(format!(
            "the name '{}' is already in use; choose another",
            inputs.requested_name
        ));
    }
    let source_path = record
        .target_path
        .as_ref()
        .ok_or("the Fork source has no local directory")?;
    let (copy_resource_id, transformed_content_sha256) =
        transformed_copy_identity(record, inputs.requested_name)?;
    let copy_target_path = match &record.target {
        LocalCatalogTargetId::SkillDirectory { canonical_root, .. } => {
            PathBuf::from(canonical_root).join(inputs.requested_name)
        }
        LocalCatalogTargetId::PetDirectory { canonical_home, .. } => PathBuf::from(canonical_home)
            .join("pets")
            .join(inputs.requested_name),
        LocalCatalogTargetId::Plugin { .. } => {
            return Err("plugins do not support Fork".to_string())
        }
    };
    let plan = CatalogResourceForkPlan {
        schema_version: CATALOG_SCHEMA,
        plan_id: CatalogPlanId::generate()?,
        registration_id: inputs.registration.registration_id.clone(),
        registration_revision: inputs.registration.revision,
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        source_kind,
        source_resource_id: inputs.registration.resource_id.clone(),
        source_display_name: record.resource.display_name(),
        source_target: record.target.clone(),
        source_path: source_path.to_string_lossy().to_string(),
        source_content_sha256: record.resource.content_sha256().to_string(),
        copy_name: inputs.requested_name.to_string(),
        copy_resource_id,
        copy_target_path: copy_target_path.to_string_lossy().to_string(),
        transformed_content_sha256,
        created_at: inputs.now,
        expires_at: inputs
            .now
            .saturating_add(CATALOG_RESOURCE_PLAN_LIFETIME_SECS),
    };
    plan.validate()?;
    Ok(plan)
}

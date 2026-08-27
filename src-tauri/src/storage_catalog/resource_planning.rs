//! Scope-level reconciliation and immutable review construction.
//!
//! Every Push and Pull plan addresses exactly one scope link — one storage,
//! one kind repository — and carries one row or item per resource. The
//! three-way comparison itself stays per resource: `derive_link_state` runs
//! over each resource's accepted base, local capture, and stored copy.

use super::adapters::adapter_for;
use super::domain::{
    pet_resource_id, skill_resource_id, CatalogClaim, CatalogClaimPin, CatalogForkSourceKind,
    CatalogLinkState, CatalogPlanActionId, CatalogPlanId, CatalogPreservationPin,
    CatalogPullAction, CatalogPullActionKind, CatalogPullDisposition, CatalogPullItem,
    CatalogPushRow, CatalogResource, CatalogResourceForkPlan, CatalogResourceId, CatalogScope,
    CatalogScopeLink, CatalogScopePullItem, CatalogScopePullPlan, CatalogScopePushPlan,
    CatalogScopePushRow, CatalogTargetExpectation, LibraryHeadCoordinate, LocalCatalogTargetId,
    PluginApplyMode, CATALOG_SCHEMA, MAX_GENERATED_NAME_ATTEMPTS,
};
use super::inventory::{CatalogLocalInventory, LocalResourceRecord};
use super::profiles::ResolvedCatalogContext;
use mallard_domain::{StorageId, StorageKind};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const CATALOG_RESOURCE_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

pub fn relation_for_state(state: CatalogLinkState) -> super::domain::CatalogResourceRelation {
    use super::domain::CatalogResourceRelation as Relation;
    match state {
        CatalogLinkState::New | CatalogLinkState::RemoteMissing => Relation::LocalOnly,
        CatalogLinkState::InSync => Relation::Matching,
        CatalogLinkState::LocalChanged => Relation::LocalChanged,
        CatalogLinkState::StorageChanged => Relation::StorageChanged,
        CatalogLinkState::Diverged => Relation::ChangedBoth,
        CatalogLinkState::LocalMissing | CatalogLinkState::UnreviewedRemote => {
            Relation::StorageOnly
        }
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
        (None, None, None) | (Some(_), None, Some(_)) => CatalogLinkState::LocalMissing,
        (None, local, Some(remote)) => {
            if local == Some(remote) {
                CatalogLinkState::InSync
            } else {
                CatalogLinkState::UnreviewedRemote
            }
        }
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

/// The per-resource state a scope link derives for one resource id.
pub fn scope_resource_state(
    link: &CatalogScopeLink,
    resource_id: &CatalogResourceId,
    local_content_sha256: Option<&str>,
    remote_content_sha256: Option<&str>,
) -> CatalogLinkState {
    let accepted = link.accepted_resources.get(resource_id);
    derive_link_state(
        accepted.map(String::as_str),
        local_content_sha256,
        remote_content_sha256,
        accepted.is_some(),
    )
}

pub(crate) fn scope_label(resource: &CatalogResource) -> Option<String> {
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

/// Cross-storage claim pins for one target: every other storage that
/// materialized this target at a different content digest.
pub fn claim_pins_for_target(
    claims: &[CatalogClaim],
    current_storage_id: &StorageId,
    target: &LocalCatalogTargetId,
    incoming_content_sha256: &str,
) -> Vec<CatalogClaimPin> {
    let mut pins: Vec<_> = claims
        .iter()
        .filter(|claim| {
            claim.storage_id != *current_storage_id
                && claim.target == *target
                && claim.content_sha256 != incoming_content_sha256
        })
        .map(|claim| CatalogClaimPin {
            storage_id: claim.storage_id.clone(),
            content_sha256: claim.content_sha256.clone(),
        })
        .collect();
    pins.sort();
    pins
}

/// The claim pins a Pull item must carry: none when the target is vacant
/// (nothing to protect), else the cross-storage pins for it.
pub fn conflicting_claim_pins(
    claims: &[CatalogClaim],
    inventory: &CatalogLocalInventory,
    storage_id: &StorageId,
    target: &LocalCatalogTargetId,
    incoming_content_sha256: &str,
) -> Vec<CatalogClaimPin> {
    let target_is_occupied = inventory
        .resources
        .values()
        .any(|record| record.target == *target);
    if !target_is_occupied {
        return Vec::new();
    }
    claim_pins_for_target(claims, storage_id, target, incoming_content_sha256)
}

/// The local records that belong to a scope of this kind. The inventory keeps
/// cross-format pet occupants so conflict handling can see them; the scope's
/// own rows must not include them.
pub fn scope_local_records<'a>(
    inventory: &'a CatalogLocalInventory,
    kind: super::domain::CatalogResourceKindTag,
) -> impl Iterator<Item = (&'a CatalogResourceId, &'a LocalResourceRecord)> + 'a {
    inventory
        .resources
        .iter()
        .filter(move |(_, record)| record.resource.kind_tag() == kind)
}

pub struct ScopePushPlanInputs<'a> {
    pub scope: &'a CatalogScope,
    pub link: &'a CatalogScopeLink,
    pub storage_kind: StorageKind,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub remote: &'a BTreeMap<CatalogResourceId, CatalogResource>,
    pub observed_head: Option<LibraryHeadCoordinate>,
    pub now: u64,
}

pub fn build_scope_push_plan(
    inputs: ScopePushPlanInputs<'_>,
) -> Result<CatalogScopePushPlan, String> {
    let locals: BTreeMap<&CatalogResourceId, &LocalResourceRecord> =
        scope_local_records(inputs.inventory, inputs.scope.resource_kind).collect();
    let mut resource_ids: BTreeSet<&CatalogResourceId> = locals.keys().copied().collect();
    resource_ids.extend(inputs.remote.keys());

    let mut rows = Vec::new();
    let mut warning_digests = BTreeSet::new();
    for resource_id in resource_ids {
        let local = locals.get(resource_id).copied();
        let remote = inputs.remote.get(resource_id);
        let state = scope_resource_state(
            inputs.link,
            resource_id,
            local.map(|record| record.resource.content_sha256()),
            remote.map(|resource| resource.content_sha256()),
        );
        let presented = match local.map(|record| &record.resource).or(remote) {
            Some(resource) => resource,
            None => continue,
        };
        let selectable = local.is_some()
            && matches!(
                state,
                CatalogLinkState::New | CatalogLinkState::LocalChanged
            );
        let blocked_reason = match state {
            CatalogLinkState::StorageChanged | CatalogLinkState::Diverged => {
                Some("Storage changed; review Pull before pushing".to_string())
            }
            CatalogLinkState::UnreviewedRemote if local.is_some() => {
                Some("Storage changed; review Pull before pushing".to_string())
            }
            CatalogLinkState::UnreviewedRemote | CatalogLinkState::LocalMissing => {
                Some("Only in the storage — Pull installs it".to_string())
            }
            CatalogLinkState::RemoteMissing => {
                Some("The accepted stored copy is missing from the storage".to_string())
            }
            CatalogLinkState::Blocked => Some("This resource is blocked".to_string()),
            CatalogLinkState::New | CatalogLinkState::LocalChanged | CatalogLinkState::InSync => {
                None
            }
        };
        let warnings = local
            .map(|record| record.warnings.clone())
            .unwrap_or_default();
        for warning in &warnings {
            warning_digests.insert(warning.digest.clone());
        }
        rows.push(CatalogScopePushRow {
            row: CatalogPushRow {
                resource_id: (*resource_id).clone(),
                kind: presented.kind_tag(),
                display_name: presented.display_name(),
                provider: presented.provider(),
                scope_label: scope_label(presented),
                relation: relation_for_state(state),
                selectable,
                selected_by_default: selectable,
                held_for_pull: blocked_reason.is_some(),
                removal_candidate: false,
                local_content_sha256: local
                    .map(|record| record.resource.content_sha256().to_string()),
                storage_content_sha256: remote
                    .map(|resource| resource.content_sha256().to_string()),
                blocked_reason,
                warnings,
                detail: selectable.then(|| match inputs.storage_kind {
                    StorageKind::Local => {
                        "Publish this resource to the linked local storage".to_string()
                    }
                    StorageKind::S3 => {
                        "Publish this resource to the linked cloud storage".to_string()
                    }
                }),
            },
            resource: local.map(|record| record.resource.clone()),
        });
    }

    // Keep the review in the same stable order as the scope page. Resource
    // IDs are content-derived, so their BTree order otherwise makes rows
    // appear to move when the user leaves the review after a Push.
    rows.sort_by(|left, right| {
        left.row
            .display_name
            .to_lowercase()
            .cmp(&right.row.display_name.to_lowercase())
            .then_with(|| left.row.resource_id.cmp(&right.row.resource_id))
    });

    let plan = CatalogScopePushPlan {
        schema_version: CATALOG_SCHEMA,
        plan_id: CatalogPlanId::generate()?,
        scope_id: inputs.scope.scope_id.clone(),
        link_id: inputs.link.link_id.clone(),
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        observed_head: inputs.observed_head,
        rows,
        warning_digests: warning_digests.into_iter().collect(),
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

struct PullItemInputs<'a> {
    context: &'a ResolvedCatalogContext,
    inventory: &'a CatalogLocalInventory,
    remote_resource: &'a CatalogResource,
    state: CatalogLinkState,
    requested_copy_name: Option<&'a str>,
    name_taken: &'a dyn Fn(&LocalCatalogTargetId, &str) -> bool,
}

fn build_pull_item(inputs: &PullItemInputs<'_>) -> Result<CatalogPullItem, String> {
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
    let local = inputs
        .inventory
        .resources
        .get(&inputs.remote_resource.resource_id());

    match inputs.remote_resource {
        CatalogResource::PluginInstallIntent(plugin) => {
            build_plugin_item(inputs, &mut item, plugin, local)?;
        }
        CatalogResource::AgentSkill(_) => {
            build_directory_item(inputs, &mut item, local, false)?;
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
                inputs,
                &mut item,
                if cross_format { occupant } else { local },
                cross_format,
            )?;
        }
    }
    Ok(item)
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
    inputs: &PullItemInputs<'_>,
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
                    reason: "the local target could not be observed".to_string(),
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
                copy_source_name(occupant),
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
                reason: "this resource is blocked".to_string(),
            }
        }
        _ => {
            item.disposition = CatalogPullDisposition::Blocked {
                reason: "this state cannot be pulled".to_string(),
            }
        }
    }
    Ok(())
}

fn build_plugin_item(
    inputs: &PullItemInputs<'_>,
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
                reason: "this plugin is blocked".to_string(),
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

/// The seed a generated copy name grows from: the occupant's own directory
/// name, which is a valid component name by construction. Display names are
/// free text — "Mallard Duck-local-copy" is not a directory name.
fn copy_source_name(occupant: &LocalResourceRecord) -> &str {
    match &occupant.target {
        LocalCatalogTargetId::SkillDirectory {
            install_directory, ..
        } => install_directory,
        LocalCatalogTargetId::PetDirectory { pet_id, .. } => pet_id,
        LocalCatalogTargetId::Plugin { plugin_id, .. } => plugin_id,
    }
}

/// Stable per-root key for the per-plan claimed-names set. Two preservation
/// copies in one review collide only when they share a parent directory.
fn copy_root_key(target: &LocalCatalogTargetId) -> String {
    match target {
        LocalCatalogTargetId::SkillDirectory { canonical_root, .. } => canonical_root.clone(),
        LocalCatalogTargetId::PetDirectory { canonical_home, .. } => {
            format!("{}/pets", canonical_home)
        }
        LocalCatalogTargetId::Plugin { .. } => String::new(),
    }
}

pub struct ScopePullPlanInputs<'a> {
    pub scope: &'a CatalogScope,
    pub link: &'a CatalogScopeLink,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub remote: &'a BTreeMap<CatalogResourceId, CatalogResource>,
    pub head: LibraryHeadCoordinate,
    pub claims: &'a [CatalogClaim],
    pub requested_copy_names: &'a BTreeMap<CatalogResourceId, String>,
    pub name_taken: &'a dyn Fn(&LocalCatalogTargetId, &str) -> bool,
    pub now: u64,
}

pub fn build_scope_pull_plan(
    inputs: ScopePullPlanInputs<'_>,
) -> Result<CatalogScopePullPlan, String> {
    // Copy names chosen earlier in this plan are taken for the rest of it,
    // so two same-review preservation copies can never collide.
    let claimed_names: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    let name_taken = |target: &LocalCatalogTargetId, candidate: &str| {
        claimed_names
            .borrow()
            .contains(&(copy_root_key(target), candidate.to_lowercase()))
            || (inputs.name_taken)(target, candidate)
    };

    let mut items = Vec::new();
    for (resource_id, remote_resource) in inputs.remote {
        let local = inputs.inventory.resources.get(resource_id);
        let base_state = scope_resource_state(
            inputs.link,
            resource_id,
            local.map(|record| record.resource.content_sha256()),
            Some(remote_resource.content_sha256()),
        );
        let (target, _) = target_for_resource(inputs.context, remote_resource)?;
        let conflicting_claims = conflicting_claim_pins(
            inputs.claims,
            inputs.inventory,
            &inputs.link.storage_id,
            &target,
            remote_resource.content_sha256(),
        );
        let state =
            if !conflicting_claims.is_empty() && base_state == CatalogLinkState::StorageChanged {
                CatalogLinkState::Diverged
            } else {
                base_state
            };
        let item = build_pull_item(&PullItemInputs {
            context: inputs.context,
            inventory: inputs.inventory,
            remote_resource,
            state,
            requested_copy_name: inputs
                .requested_copy_names
                .get(resource_id)
                .map(String::as_str),
            name_taken: &name_taken,
        })?;
        for action in &item.actions {
            if let Some(pin) = action.kind.preservation() {
                claimed_names
                    .borrow_mut()
                    .insert((copy_root_key(&action.target), pin.copy_name.to_lowercase()));
            }
        }
        items.push(CatalogScopePullItem {
            item,
            resource: Some(remote_resource.clone()),
            conflicting_claims,
        });
    }

    let plan = CatalogScopePullPlan {
        schema_version: CATALOG_SCHEMA,
        plan_id: CatalogPlanId::generate()?,
        scope_id: inputs.scope.scope_id.clone(),
        link_id: inputs.link.link_id.clone(),
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        head: inputs.head,
        items,
        created_at: inputs.now,
        expires_at: inputs
            .now
            .saturating_add(CATALOG_RESOURCE_PLAN_LIFETIME_SECS),
    };
    plan.validate()?;
    Ok(plan)
}

/// One reviewed decision for one Pull item: the item plus the approved
/// action, or `None` for an explicit Skip.
#[derive(Debug)]
pub struct ScopePullSelection<'a> {
    pub item: &'a CatalogScopePullItem,
    pub action: Option<&'a CatalogPullAction>,
}

pub fn validate_scope_pull_selection<'a>(
    plan: &'a CatalogScopePullPlan,
    choices: &BTreeMap<CatalogResourceId, CatalogPlanActionId>,
    destructive_acknowledged: &BTreeSet<CatalogResourceId>,
) -> Result<Vec<ScopePullSelection<'a>>, String> {
    for (resource_id, action_id) in choices {
        let item = plan
            .pull_item(resource_id)
            .ok_or("a selected action does not belong to this Pull review")?;
        let action = item
            .action(action_id)
            .ok_or("a selected action does not belong to this Pull review")?;
        if action.requires_destructive_approval && !destructive_acknowledged.contains(resource_id) {
            return Err("the overwrite requires explicit acknowledgement".to_string());
        }
    }
    Ok(plan
        .items
        .iter()
        .map(|item| ScopePullSelection {
            item,
            action: choices
                .get(&item.item.resource_id)
                .and_then(|action_id| item.action(action_id)),
        })
        .collect())
}

pub struct ScopeForkPlanInputs<'a> {
    pub scope: &'a CatalogScope,
    pub resource_id: &'a CatalogResourceId,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub requested_name: &'a str,
    pub name_taken: &'a dyn Fn(&LocalCatalogTargetId, &str) -> bool,
    pub now: u64,
}

pub fn build_resource_fork_plan(
    inputs: ScopeForkPlanInputs<'_>,
) -> Result<CatalogResourceForkPlan, String> {
    let record = inputs
        .inventory
        .resources
        .get(inputs.resource_id)
        .ok_or("the Fork source is missing locally")?;
    if record.resource.kind_tag() != inputs.scope.resource_kind {
        return Err("the Fork source does not belong to this Catalog row".to_string());
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
        scope_id: inputs.scope.scope_id.clone(),
        profile_pins: inputs.context.profile_pins(),
        skill_root_pins: inputs.context.skill_root_pins(),
        source_kind,
        source_resource_id: (*inputs.resource_id).clone(),
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

#[cfg(test)]
mod tests {
    use super::super::domain::{
        sha256_hex, skill_resource_id, AgentSkillDescriptor, AgentSkillResource, AgentSkillScope,
        CatalogActionOutcome, CatalogFileEntry, CatalogResourceKindTag, CatalogResourceRelation,
        CatalogResourceRepositoryId, CatalogScopeId, LocalCatalogProfileId, ProviderKey,
    };
    use super::super::profiles::{ResolvedCatalogProfile, ResolvedSkillRoot};
    use super::super::skill_tree::{CapturedSkillFile, CapturedSkillTree};
    use super::*;
    use std::path::Path;

    const SKILL_ROOT: &str = "/tmp/planning/codex-home/skills";

    fn context() -> ResolvedCatalogContext {
        ResolvedCatalogContext {
            codex: Some(ResolvedCatalogProfile {
                profile_id: LocalCatalogProfileId::generate().unwrap(),
                provider_key: ProviderKey::Codex,
                revision: 1,
                canonical_home: PathBuf::from("/tmp/planning/codex-home"),
            }),
            claude: None,
            skill_roots: vec![ResolvedSkillRoot {
                scope: AgentSkillScope::CodexHome,
                canonical_root: PathBuf::from(SKILL_ROOT),
            }],
        }
    }

    fn skill_record(
        install_directory: &str,
        declared_name: &str,
        body: &str,
    ) -> LocalResourceRecord {
        let skill_md = format!("---\nname: {}\n---\n{}\n", declared_name, body);
        let files = BTreeMap::from([(
            "SKILL.md".to_string(),
            CapturedSkillFile {
                bytes: skill_md.clone().into_bytes(),
                mode: 0o644,
            },
        )]);
        let entries = BTreeMap::from([(
            "SKILL.md".to_string(),
            CatalogFileEntry {
                sha256: sha256_hex(skill_md.as_bytes()),
                size: skill_md.len() as u64,
                mode: 0o644,
                object_key: format!("_objects/{}", sha256_hex(skill_md.as_bytes())),
            },
        )]);
        let mut resource = AgentSkillResource {
            descriptor: AgentSkillDescriptor {
                scope: AgentSkillScope::CodexHome,
                effective_name: declared_name.to_string(),
                install_directory: install_directory.to_string(),
            },
            content_sha256: String::new(),
            files: entries,
        };
        resource.content_sha256 = resource.compute_content_sha256();
        LocalResourceRecord {
            resource: CatalogResource::AgentSkill(resource),
            target: LocalCatalogTargetId::SkillDirectory {
                scope: AgentSkillScope::CodexHome,
                canonical_root: SKILL_ROOT.to_string(),
                install_directory: install_directory.to_string(),
            },
            target_path: Some(Path::new(SKILL_ROOT).join(install_directory)),
            local_modified_at: Some(1),
            warnings: Vec::new(),
            skill_tree: Some(CapturedSkillTree {
                files,
                declared_name: Some(declared_name.to_string()),
                warnings: Default::default(),
            }),
            pet: None,
        }
    }

    fn inventory(records: Vec<LocalResourceRecord>) -> CatalogLocalInventory {
        CatalogLocalInventory {
            resources: records
                .into_iter()
                .map(|record| (record.resource.resource_id(), record))
                .collect(),
            blocked: Vec::new(),
            plugin_owned_skills: Vec::new(),
            warnings: Vec::new(),
            plugin_inventories: BTreeMap::new(),
        }
    }

    fn scope() -> CatalogScope {
        CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            provider: ProviderKey::Codex,
            profile_id: LocalCatalogProfileId::generate().unwrap(),
            resource_kind: CatalogResourceKindTag::AgentSkill,
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn link(
        scope: &CatalogScope,
        accepted: &[(CatalogResourceId, String)],
    ) -> super::super::domain::CatalogScopeLink {
        super::super::domain::CatalogScopeLink {
            link_id: super::super::domain::CatalogLinkId::generate().unwrap(),
            scope_id: scope.scope_id.clone(),
            storage_id: StorageId::parse("storage-a").unwrap(),
            repository_id: CatalogResourceRepositoryId::for_kind(
                scope.provider,
                scope.resource_kind,
            ),
            accepted_head: None,
            accepted_resources: accepted.iter().cloned().collect(),
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn head(link: &super::super::domain::CatalogScopeLink) -> LibraryHeadCoordinate {
        LibraryHeadCoordinate {
            repository_id: link.repository_id.clone(),
            revision: 3,
            commit_id: super::super::domain::CatalogCommitId::generate().unwrap(),
            manifest_sha256: "d".repeat(64),
            manifest_size: 10,
            storage_identity_sha256: "e".repeat(64),
        }
    }

    #[test]
    fn derive_link_state_covers_the_nine_states() {
        let base = Some("a");
        let table = [
            (None, Some("x"), None, false, CatalogLinkState::New),
            (None, None, None, false, CatalogLinkState::LocalMissing),
            (None, Some("x"), Some("x"), false, CatalogLinkState::InSync),
            (
                None,
                Some("x"),
                Some("y"),
                false,
                CatalogLinkState::UnreviewedRemote,
            ),
            (base, None, Some("x"), true, CatalogLinkState::LocalMissing),
            (base, Some("x"), None, true, CatalogLinkState::RemoteMissing),
            (base, Some("x"), None, false, CatalogLinkState::New),
            (base, Some("x"), Some("x"), true, CatalogLinkState::InSync),
            (
                base,
                Some("a"),
                Some("y"),
                true,
                CatalogLinkState::StorageChanged,
            ),
            (
                base,
                Some("y"),
                Some("a"),
                true,
                CatalogLinkState::LocalChanged,
            ),
            (base, Some("y"), Some("z"), true, CatalogLinkState::Diverged),
        ];
        for (accepted, local, remote, was_accepted, expected) in table {
            assert_eq!(
                derive_link_state(accepted, local, remote, was_accepted),
                expected,
                "({accepted:?}, {local:?}, {remote:?}, {was_accepted})"
            );
        }
    }

    #[test]
    fn a_scope_push_plan_rows_the_union_of_local_and_stored_resources() {
        let scope = scope();
        let context = context();
        let changed = skill_record("review", "review", "Changed locally.");
        let changed_id = changed.resource.resource_id();
        let local_only = skill_record("fresh", "fresh", "Local only.");
        let changed_base = "b".repeat(64);
        let stored_only = skill_record("remote", "remote", "Stored only.");
        let stored_only_resource = stored_only.resource.clone();

        // The stored side: `review` at the accepted base, plus one resource
        // this machine has never seen.
        let mut stored_review = skill_record("review", "review", "Original body.");
        if let CatalogResource::AgentSkill(skill) = &mut stored_review.resource {
            skill.content_sha256 = changed_base.clone();
        }
        let remote = BTreeMap::from([
            (changed_id.clone(), stored_review.resource.clone()),
            (stored_only_resource.resource_id(), stored_only_resource),
        ]);
        let link = link(&scope, &[(changed_id.clone(), changed_base)]);

        let plan = build_scope_push_plan(ScopePushPlanInputs {
            scope: &scope,
            link: &link,
            storage_kind: StorageKind::Local,
            storage_identity_sha256: "c".repeat(64),
            context: &context,
            inventory: &inventory(vec![local_only, changed]),
            remote: &remote,
            observed_head: Some(head(&link)),
            now: 100,
        })
        .unwrap();

        assert_eq!(plan.rows.len(), 3);
        assert_eq!(
            plan.rows
                .iter()
                .map(|row| row.row.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["fresh", "remote", "review"]
        );
        let by_name = |name: &str| plan.rows.iter().find(|row| row.row.display_name == name);
        let fresh = by_name("fresh").unwrap();
        assert_eq!(fresh.row.relation, CatalogResourceRelation::LocalOnly);
        assert!(fresh.row.selectable && fresh.resource.is_some());
        let review = by_name("review").unwrap();
        assert_eq!(review.row.relation, CatalogResourceRelation::LocalChanged);
        assert!(review.row.selectable);
        let remote_row = by_name("remote").unwrap();
        assert_eq!(
            remote_row.row.relation,
            CatalogResourceRelation::StorageOnly
        );
        assert!(!remote_row.row.selectable && remote_row.resource.is_none());
        assert!(remote_row
            .row
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("Pull installs it"));
    }

    #[test]
    fn a_scope_pull_plan_items_every_stored_resource_and_installs_the_missing_one() {
        let scope = scope();
        let context = context();
        let stored = skill_record("review", "review", "Stored body.");
        let remote = BTreeMap::from([(stored.resource.resource_id(), stored.resource.clone())]);
        let link = link(&scope, &[]);
        let never = |_: &LocalCatalogTargetId, _: &str| false;

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &context,
            inventory: &inventory(Vec::new()),
            remote: &remote,
            head: head(&link),
            claims: &[],
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();

        assert_eq!(plan.items.len(), 1);
        let item = &plan.items[0];
        assert!(matches!(
            item.item.disposition,
            CatalogPullDisposition::Executable
        ));
        assert!(matches!(
            item.item.actions[0].kind,
            CatalogPullActionKind::InstallSkill
        ));
        assert_eq!(
            item.item.actions[0].expected_target_state,
            CatalogTargetExpectation::Absent
        );
    }

    #[test]
    fn two_preservation_copies_in_one_review_get_distinct_names() {
        let scope = scope();
        let context = context();
        // Two directory names identical up to the 96-character copy-name stem:
        // their generated copy names would collide without the per-plan
        // claimed-names set.
        let stem = "a".repeat(96);
        let first_name = format!("{stem}b");
        let second_name = format!("{stem}c");
        let first_local = skill_record(&first_name, "review", "Local a.");
        let second_local = skill_record(&second_name, "review", "Local b.");
        let first_stored = skill_record(&first_name, "review", "Stored a.");
        let second_stored = skill_record(&second_name, "review", "Stored b.");
        let remote = BTreeMap::from([
            (
                first_stored.resource.resource_id(),
                first_stored.resource.clone(),
            ),
            (
                second_stored.resource.resource_id(),
                second_stored.resource.clone(),
            ),
        ]);
        let link = link(&scope, &[]);
        let never = |_: &LocalCatalogTargetId, _: &str| false;

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &context,
            inventory: &inventory(vec![first_local, second_local]),
            remote: &remote,
            head: head(&link),
            claims: &[],
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();

        let copy_names: BTreeSet<String> = plan
            .items
            .iter()
            .flat_map(|item| &item.item.actions)
            .filter_map(|action| action.kind.preservation())
            .map(|pin| pin.copy_name.clone())
            .collect();
        assert_eq!(
            copy_names,
            BTreeSet::from([format!("{stem}-local-copy"), format!("{stem}-local-copy-2")])
        );
    }

    #[test]
    fn a_cross_storage_claim_escalates_storage_changed_to_the_preserve_pair() {
        let scope = scope();
        let context = context();
        let base = "b".repeat(64);
        let mut local = skill_record("review", "review", "Base body.");
        if let CatalogResource::AgentSkill(skill) = &mut local.resource {
            skill.content_sha256 = base.clone();
        }
        let stored = skill_record("review", "review", "Newer stored body.");
        let resource_id = stored.resource.resource_id();
        let remote = BTreeMap::from([(resource_id.clone(), stored.resource.clone())]);
        let link = link(&scope, &[(resource_id.clone(), base.clone())]);
        let never = |_: &LocalCatalogTargetId, _: &str| false;
        let claims = vec![CatalogClaim {
            target: local.target.clone(),
            storage_id: StorageId::parse("storage-other").unwrap(),
            resource_id: resource_id.clone(),
            content_sha256: "9".repeat(64),
            outcome: CatalogActionOutcome::Updated,
            recorded_at: 1,
            plan_id: None,
        }];

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &context,
            inventory: &inventory(vec![local]),
            remote: &remote,
            head: head(&link),
            claims: &claims,
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();

        let item = &plan.items[0];
        assert_eq!(item.conflicting_claims.len(), 1);
        assert_eq!(item.item.relation, CatalogResourceRelation::ChangedBoth);
        let kinds: Vec<bool> = item
            .item
            .actions
            .iter()
            .map(|action| action.kind.preservation().is_some())
            .collect();
        assert_eq!(kinds, vec![true, false]);
        assert!(item.item.actions[1].requires_destructive_approval);
    }

    #[test]
    fn pull_selections_validate_choices_and_destructive_acknowledgements() {
        let scope = scope();
        let context = context();
        let base = "b".repeat(64);
        let mut local = skill_record("review", "review", "Base body.");
        if let CatalogResource::AgentSkill(skill) = &mut local.resource {
            skill.content_sha256 = "9".repeat(64);
        }
        let stored = skill_record("review", "review", "Stored body.");
        let resource_id = stored.resource.resource_id();
        let remote = BTreeMap::from([(resource_id.clone(), stored.resource.clone())]);
        let link = link(&scope, &[(resource_id.clone(), base)]);
        let never = |_: &LocalCatalogTargetId, _: &str| false;
        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &context,
            inventory: &inventory(vec![local]),
            remote: &remote,
            head: head(&link),
            claims: &[],
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();
        let item = &plan.items[0];
        let overwrite = item
            .item
            .actions
            .iter()
            .find(|action| action.requires_destructive_approval)
            .unwrap();

        // No choices: every item is an explicit Skip.
        let skipped =
            validate_scope_pull_selection(&plan, &BTreeMap::new(), &BTreeSet::new()).unwrap();
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].action.is_none());

        // A destructive choice without its acknowledgement is refused.
        let choices = BTreeMap::from([(resource_id.clone(), overwrite.action_id.clone())]);
        assert!(
            validate_scope_pull_selection(&plan, &choices, &BTreeSet::new())
                .unwrap_err()
                .contains("explicit acknowledgement")
        );
        let approved =
            validate_scope_pull_selection(&plan, &choices, &BTreeSet::from([resource_id.clone()]))
                .unwrap();
        assert_eq!(approved[0].action.unwrap().action_id, overwrite.action_id);

        // A foreign action id is refused outright.
        let foreign = BTreeMap::from([(
            skill_resource_id(AgentSkillScope::CodexHome, "other", "other"),
            overwrite.action_id.clone(),
        )]);
        assert!(
            validate_scope_pull_selection(&plan, &foreign, &BTreeSet::new())
                .unwrap_err()
                .contains("does not belong")
        );
    }

    #[test]
    fn cross_storage_claim_pins_exclude_the_current_storage_and_matching_content() {
        let target = LocalCatalogTargetId::SkillDirectory {
            scope: AgentSkillScope::CodexHome,
            canonical_root: SKILL_ROOT.to_string(),
            install_directory: "review".to_string(),
        };
        let claim = |storage: &str, digest: char| CatalogClaim {
            target: target.clone(),
            storage_id: StorageId::parse(storage).unwrap(),
            resource_id: skill_resource_id(AgentSkillScope::CodexHome, "review", "review"),
            content_sha256: digest.to_string().repeat(64),
            outcome: CatalogActionOutcome::Updated,
            recorded_at: 1,
            plan_id: None,
        };
        let claims = vec![
            claim("storage-current", 'b'),
            claim("storage-a", 'a'),
            claim("storage-b", 'b'),
        ];
        let pins = claim_pins_for_target(
            &claims,
            &StorageId::parse("storage-current").unwrap(),
            &target,
            &"b".repeat(64),
        );
        assert_eq!(
            pins,
            vec![CatalogClaimPin {
                storage_id: StorageId::parse("storage-a").unwrap(),
                content_sha256: "a".repeat(64),
            }]
        );

        // Stably sorted regardless of claim order.
        let pins = claim_pins_for_target(
            &[claim("storage-b", 'x'), claim("storage-a", 'y')],
            &StorageId::parse("storage-current").unwrap(),
            &target,
            &"c".repeat(64),
        );
        assert_eq!(
            pins.iter()
                .map(|pin| pin.storage_id.as_str())
                .collect::<Vec<_>>(),
            vec!["storage-a", "storage-b"]
        );
    }

    #[test]
    fn a_fork_plan_is_scope_keyed_and_kind_guarded() {
        let scope = scope();
        let context = context();
        let record = skill_record("review", "review", "Fork me.");
        let resource_id = record.resource.resource_id();
        let never = |_: &LocalCatalogTargetId, _: &str| false;
        let plan = build_resource_fork_plan(ScopeForkPlanInputs {
            scope: &scope,
            resource_id: &resource_id,
            context: &context,
            inventory: &inventory(vec![record]),
            requested_name: "review-fork",
            name_taken: &never,
            now: 100,
        })
        .unwrap();
        assert_eq!(plan.scope_id, scope.scope_id);
        assert_eq!(plan.copy_name, "review-fork");
        assert_eq!(
            plan.copy_resource_id,
            skill_resource_id(AgentSkillScope::CodexHome, "review-fork", "review-fork")
        );

        // A pets scope cannot fork a skill even if the id resolves.
        let mut pets_scope = scope.clone();
        pets_scope.resource_kind = CatalogResourceKindTag::CodexPetV1;
        let record = skill_record("review", "review", "Fork me.");
        let error = build_resource_fork_plan(ScopeForkPlanInputs {
            scope: &pets_scope,
            resource_id: &resource_id,
            context: &context,
            inventory: &inventory(vec![record]),
            requested_name: "review-fork",
            name_taken: &never,
            now: 100,
        })
        .unwrap_err();
        assert!(
            error.contains("does not belong to this Catalog row"),
            "{error}"
        );
    }
}

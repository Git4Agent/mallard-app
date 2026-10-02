//! Scope-level reconciliation and immutable review construction.
//!
//! Every Push and Pull plan addresses exactly one scope link — one storage,
//! one coordinate repository — and carries one row or item per resource. The
//! three-way comparison itself stays per resource: `derive_link_state` runs
//! over each resource's accepted base, local capture, and stored copy.
//! Nothing here names a provider, kind, or format: every format-specific
//! question goes to the scope's registered codec and adapter.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mallard_domain::StorageId;

use super::domain::{
    validate_component_name, CatalogClaim, CatalogClaimPin, CatalogLinkState, CatalogPlanActionId,
    CatalogPlanId, CatalogPreservationPin, CatalogPullAction, CatalogPullActionKind,
    CatalogPullDisposition, CatalogPullItem, CatalogPushRow, CatalogResourceEnvelope,
    CatalogResourceForkPlan, CatalogResourceId, CatalogResourceRelation, CatalogScope,
    CatalogScopeLink, CatalogScopePullItem, CatalogScopePullPlan, CatalogScopePushPlan,
    CatalogScopePushRow, CatalogTargetConfig, CatalogTargetExpectation, CatalogTargetIdentity,
    LibraryHeadCoordinate, NativePluginPlan, PluginApplyMode, MAX_GENERATED_NAME_ATTEMPTS,
};
use super::inventory::{CatalogLocalInventory, LocalResourceRecord};
use super::profiles::ResolvedCatalogContext;
use super::registry::{CaptureSupport, CatalogCapability};

pub const CATALOG_RESOURCE_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

pub fn relation_for_state(state: CatalogLinkState) -> CatalogResourceRelation {
    use CatalogResourceRelation as Relation;
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
        (None, None, None) | (None, None, Some(_)) | (Some(_), None, Some(_)) => {
            CatalogLinkState::LocalMissing
        }
        (None, Some(local), Some(remote)) => {
            if local == remote {
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

/// The machine-local target one stored resource lands on under this profile:
/// the capability's root whose class the resource's identity names (or its
/// only root), and the member the codec derives from the identity. Directory
/// resources also get their target path; provider-inventory resources have
/// none.
pub fn target_for_resource(
    context: &ResolvedCatalogContext,
    capability: &CatalogCapability,
    resource: &CatalogResourceEnvelope,
) -> Result<(CatalogTargetConfig, Option<PathBuf>), String> {
    let codec = capability.codec.as_ref();
    let targets = context.targets_for(capability);
    let target = match codec.root_class(resource)? {
        Some(class) => targets
            .iter()
            .find(|target| target.root_class == class)
            .copied()
            .ok_or_else(|| format!("no {} target is active for this profile", class))?,
        None => targets.first().copied().ok_or_else(|| {
            format!(
                "no {} target is active for this profile",
                capability.descriptor.label
            )
        })?,
    };
    let member = codec.member_name(resource)?;
    let config = CatalogTargetConfig::new(
        capability.provider().clone(),
        target.target.clone(),
        &CatalogTargetIdentity {
            canonical_root: target.canonical_root.to_string_lossy().to_string(),
            root_class: target.root_class.clone(),
            member: member.clone(),
        },
    )?;
    let path = match capability.descriptor.capture {
        CaptureSupport::DirectoryTree => Some(target.canonical_root.join(&member)),
        CaptureSupport::ProviderInventory => None,
    };
    Ok((config, path))
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

/// The identity and content the reviewed copy of one local record would
/// have, through the codec of the record's own coordinate.
pub fn transformed_copy_identity(
    record: &LocalResourceRecord,
    copy_name: &str,
) -> Result<(CatalogResourceId, String), String> {
    let captured = record
        .captured
        .as_ref()
        .ok_or("the copy source has no captured payload")?;
    let renamed = record.capability.codec.rename(captured, copy_name)?;
    let envelope = renamed.to_envelope(&record.resource.coordinate());
    Ok((envelope.resource_id, envelope.content_sha256))
}

/// Cross-storage claim pins for one target: every other storage that
/// materialized this target at a different content digest.
pub fn claim_pins_for_target(
    claims: &[CatalogClaim],
    current_storage_id: &StorageId,
    target: &CatalogTargetConfig,
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
    target: &CatalogTargetConfig,
    incoming_content_sha256: &str,
) -> Vec<CatalogClaimPin> {
    if inventory.occupant(target).is_none() {
        return Vec::new();
    }
    claim_pins_for_target(claims, storage_id, target, incoming_content_sha256)
}

pub struct ScopePushPlanInputs<'a> {
    pub scope: &'a CatalogScope,
    pub capability: &'a CatalogCapability,
    pub link: &'a CatalogScopeLink,
    /// The storage occupies a machine-local folder (adapter-declared).
    pub storage_is_local: bool,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub remote: &'a BTreeMap<CatalogResourceId, CatalogResourceEnvelope>,
    pub observed_head: Option<LibraryHeadCoordinate>,
    /// Per item, on a storage that holds one item per repository.
    pub observed_items: BTreeMap<CatalogResourceId, LibraryHeadCoordinate>,
    /// Items that cannot push yet, with why: several repositories hold
    /// them and none is chosen.
    pub blocked: BTreeMap<CatalogResourceId, String>,
    pub now: u64,
}

pub fn build_scope_push_plan(
    inputs: ScopePushPlanInputs<'_>,
) -> Result<CatalogScopePushPlan, String> {
    let codec = inputs.capability.codec.as_ref();
    let mut resource_ids: BTreeSet<&CatalogResourceId> =
        inputs.inventory.resources.keys().collect();
    resource_ids.extend(inputs.remote.keys());

    let mut rows = Vec::new();
    let mut warning_digests = BTreeSet::new();
    for resource_id in resource_ids {
        let local = inputs.inventory.resources.get(resource_id);
        let remote = inputs.remote.get(resource_id);
        let state = scope_resource_state(
            inputs.link,
            resource_id,
            local.map(|record| record.resource.content_sha256.as_str()),
            remote.map(|resource| resource.content_sha256.as_str()),
        );
        let Some(presented) = local.map(|record| &record.resource).or(remote) else {
            continue;
        };
        let blocked = inputs.blocked.get(resource_id);
        let selectable = local.is_some()
            && blocked.is_none()
            && matches!(
                state,
                CatalogLinkState::New | CatalogLinkState::LocalChanged
            );
        let held_reason = match state {
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
        let held_for_pull = blocked.is_none() && held_reason.is_some();
        let blocked_reason = blocked.cloned().or(held_reason);
        let warnings = local
            .map(|record| record.warnings.clone())
            .unwrap_or_default();
        for warning in &warnings {
            warning_digests.insert(warning.digest.clone());
        }
        rows.push(CatalogScopePushRow {
            row: CatalogPushRow {
                resource_id: (*resource_id).clone(),
                coordinate: inputs.capability.coordinate(),
                labels: inputs.capability.labels(),
                display_name: codec.display_name(presented)?,
                scope_label: codec.resource_label(presented),
                relation: relation_for_state(state),
                selectable,
                selected_by_default: selectable,
                held_for_pull,
                local_content_sha256: local.map(|record| record.resource.content_sha256.clone()),
                storage_content_sha256: remote.map(|resource| resource.content_sha256.clone()),
                blocked_reason,
                warnings,
                detail: selectable.then(|| {
                    if inputs.storage_is_local {
                        "Publish this resource to the linked local storage".to_string()
                    } else {
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
        plan_id: CatalogPlanId::generate()?,
        scope_id: inputs.scope.scope_id.clone(),
        link_id: inputs.link.link_id.clone(),
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        target_pins: inputs.context.target_pins(),
        observed_head: inputs.observed_head,
        observed_items: inputs.observed_items,
        destinations: BTreeMap::new(),
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

/// The root a target's copies land beside. Two preservation copies in one
/// review collide only when they share a parent directory.
fn copy_root(target: &CatalogTargetConfig) -> Result<String, String> {
    Ok(target.decode()?.canonical_root)
}

fn preservation_pin(
    occupant: &LocalResourceRecord,
    storage_digest: &str,
    requested_copy_name: Option<&str>,
    name_taken: &dyn Fn(&CatalogTargetConfig, &str) -> bool,
) -> Result<CatalogPreservationPin, String> {
    let identity = occupant.target.decode()?;
    let taken = |candidate: &str| name_taken(&occupant.target, candidate);
    let copy_name = match requested_copy_name {
        Some(requested) => {
            validate_component_name("preservation copy name", requested, 128)?;
            if taken(requested) {
                return Err(format!(
                    "the copy name '{}' is already in use; choose another",
                    requested
                ));
            }
            requested.to_string()
        }
        // The seed is the occupant's own member name, a valid component by
        // construction; display names are free text.
        None => generate_copy_name(&identity.member, &taken)?,
    };
    let (copy_resource_id, transformed_content_sha256) =
        transformed_copy_identity(occupant, &copy_name)?;
    Ok(CatalogPreservationPin {
        copy_target_path: Path::new(&identity.canonical_root)
            .join(&copy_name)
            .to_string_lossy()
            .to_string(),
        copy_name,
        copy_resource_id,
        transformed_content_sha256,
        original_content_sha256: occupant.resource.content_sha256.clone(),
        storage_content_sha256: storage_digest.to_string(),
        reusable_witness: false,
    })
}

struct PullItemInputs<'a> {
    capability: &'a CatalogCapability,
    context: &'a ResolvedCatalogContext,
    inventory: &'a CatalogLocalInventory,
    remote_resource: &'a CatalogResourceEnvelope,
    state: CatalogLinkState,
    requested_copy_name: Option<&'a str>,
    name_taken: &'a dyn Fn(&CatalogTargetConfig, &str) -> bool,
}

fn build_pull_item(inputs: &PullItemInputs<'_>) -> Result<CatalogPullItem, String> {
    let codec = inputs.capability.codec.as_ref();
    let remote = inputs.remote_resource;
    let mut item = CatalogPullItem {
        resource_id: remote.resource_id.clone(),
        coordinate: inputs.capability.coordinate(),
        labels: inputs.capability.labels(),
        display_name: codec.display_name(remote)?,
        scope_label: codec.resource_label(remote),
        relation: relation_for_state(inputs.state),
        disposition: CatalogPullDisposition::KeptLocal,
        actions: Vec::new(),
        default_action_id: None,
        detail: None,
    };
    let (target, target_path) = target_for_resource(inputs.context, inputs.capability, remote)?;
    match inputs.capability.descriptor.capture {
        CaptureSupport::ProviderInventory => build_native_item(inputs, &mut item, &target)?,
        CaptureSupport::DirectoryTree => {
            let target_path = target_path.ok_or("directory resource has no target path")?;
            build_directory_item(inputs, &mut item, &target, &target_path)?;
        }
    }
    Ok(item)
}

#[allow(clippy::too_many_arguments)]
fn push_action(
    item: &mut CatalogPullItem,
    kind: CatalogPullActionKind,
    destructive: bool,
    target: &CatalogTargetConfig,
    target_path: Option<&Path>,
    native: Option<NativePluginPlan>,
    expected: CatalogTargetExpectation,
    incoming_digest: &str,
) -> Result<CatalogPlanActionId, String> {
    let action_id = CatalogPlanActionId::generate()?;
    item.actions.push(CatalogPullAction {
        action_id: action_id.clone(),
        resource_id: item.resource_id.clone(),
        kind,
        requires_destructive_approval: destructive,
        target: target.clone(),
        target_path: target_path.map(|path| path.to_string_lossy().to_string()),
        native,
        expected_target_state: expected,
        incoming_content_sha256: incoming_digest.to_string(),
    });
    Ok(action_id)
}

fn build_directory_item(
    inputs: &PullItemInputs<'_>,
    item: &mut CatalogPullItem,
    target: &CatalogTargetConfig,
    target_path: &Path,
) -> Result<(), String> {
    let remote = inputs.remote_resource;
    let incoming_digest = remote.content_sha256.as_str();
    // Whatever sits at the target path is the occupant: this resource's own
    // local capture, or a different resource (another identity, or a sibling
    // version of the format) that happens to hold the same directory.
    let occupant = inputs.inventory.occupant(target);
    let local_digest = occupant.map(|record| record.resource.content_sha256.as_str());
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
            let action_id = push_action(
                item,
                CatalogPullActionKind::Install,
                false,
                target,
                Some(target_path),
                None,
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
            let action_id = push_action(
                item,
                CatalogPullActionKind::Update,
                false,
                target,
                Some(target_path),
                None,
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
            if occupant.is_some() =>
        {
            let occupant = occupant.ok_or("the conflicting local resource disappeared")?;
            let local_digest = occupant.resource.content_sha256.clone();
            item.disposition = CatalogPullDisposition::Executable;
            let pin = preservation_pin(
                occupant,
                incoming_digest,
                inputs.requested_copy_name,
                inputs.name_taken,
            )?;
            item.detail = Some(format!(
                "Keep the local version as '{}' and overwrite with the stored version",
                pin.copy_name
            ));
            let action_id = push_action(
                item,
                CatalogPullActionKind::PreserveAsCopyAndOverwrite { copy: pin },
                false,
                target,
                Some(target_path),
                None,
                CatalogTargetExpectation::Sha256 {
                    sha256: local_digest.clone(),
                },
                incoming_digest,
            )?;
            push_action(
                item,
                CatalogPullActionKind::OverwriteWithStorage,
                true,
                target,
                Some(target_path),
                None,
                CatalogTargetExpectation::Sha256 {
                    sha256: local_digest,
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

fn build_native_item(
    inputs: &PullItemInputs<'_>,
    item: &mut CatalogPullItem,
    target: &CatalogTargetConfig,
) -> Result<(), String> {
    let remote = inputs.remote_resource;
    let local = inputs.inventory.resources.get(&remote.resource_id);
    let locally_present = local.is_some();
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
                reason: "this resource is blocked".to_string(),
            };
            return Ok(());
        }
        _ => {}
    }
    let native_inventory = inputs
        .inventory
        .native
        .as_ref()
        .ok_or("the provider inventory was not observed")?;
    let mode = if locally_present {
        PluginApplyMode::OverwriteExisting
    } else {
        PluginApplyMode::InstallMissing
    };
    let native = match inputs
        .capability
        .adapter
        .plan_native_apply(remote, native_inventory, mode)
    {
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
        CatalogPullActionKind::OverwriteWithStorage
    } else if locally_present {
        CatalogPullActionKind::Update
    } else {
        CatalogPullActionKind::Install
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
        target,
        None,
        Some(native),
        local.map_or(CatalogTargetExpectation::Absent, |record| {
            CatalogTargetExpectation::Sha256 {
                sha256: record.resource.content_sha256.clone(),
            }
        }),
        &remote.content_sha256,
    )?;
    item.default_action_id = Some(action_id);
    Ok(())
}

pub struct ScopePullPlanInputs<'a> {
    pub scope: &'a CatalogScope,
    pub capability: &'a CatalogCapability,
    pub link: &'a CatalogScopeLink,
    pub storage_identity_sha256: String,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub remote: &'a BTreeMap<CatalogResourceId, CatalogResourceEnvelope>,
    pub head: Option<LibraryHeadCoordinate>,
    /// Per item, on a storage that holds one item per repository.
    pub item_heads: BTreeMap<CatalogResourceId, LibraryHeadCoordinate>,
    pub claims: &'a [CatalogClaim],
    pub requested_copy_names: &'a BTreeMap<CatalogResourceId, String>,
    pub name_taken: &'a dyn Fn(&CatalogTargetConfig, &str) -> bool,
    pub now: u64,
}

pub fn build_scope_pull_plan(
    inputs: ScopePullPlanInputs<'_>,
) -> Result<CatalogScopePullPlan, String> {
    // Copy names chosen earlier in this plan are taken for the rest of it,
    // so two same-review preservation copies can never collide.
    let claimed_names: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    let name_taken = |target: &CatalogTargetConfig, candidate: &str| {
        copy_root(target).is_ok_and(|root| {
            claimed_names
                .borrow()
                .contains(&(root, candidate.to_lowercase()))
        }) || (inputs.name_taken)(target, candidate)
    };

    let mut items = Vec::new();
    for (resource_id, remote_resource) in inputs.remote {
        let local = inputs.inventory.resources.get(resource_id);
        let base_state = scope_resource_state(
            inputs.link,
            resource_id,
            local.map(|record| record.resource.content_sha256.as_str()),
            Some(remote_resource.content_sha256.as_str()),
        );
        let (target, _) = target_for_resource(inputs.context, inputs.capability, remote_resource)?;
        let conflicting_claims = conflicting_claim_pins(
            inputs.claims,
            inputs.inventory,
            &inputs.link.storage_id,
            &target,
            &remote_resource.content_sha256,
        );
        let state =
            if !conflicting_claims.is_empty() && base_state == CatalogLinkState::StorageChanged {
                CatalogLinkState::Diverged
            } else {
                base_state
            };
        let item = build_pull_item(&PullItemInputs {
            capability: inputs.capability,
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
                    .insert((copy_root(&action.target)?, pin.copy_name.to_lowercase()));
            }
        }
        items.push(CatalogScopePullItem {
            item,
            resource: Some(remote_resource.clone()),
            conflicting_claims,
        });
    }

    let plan = CatalogScopePullPlan {
        plan_id: CatalogPlanId::generate()?,
        scope_id: inputs.scope.scope_id.clone(),
        link_id: inputs.link.link_id.clone(),
        link_revision: inputs.link.revision,
        storage_id: inputs.link.storage_id.clone(),
        repository_id: inputs.link.repository_id.clone(),
        storage_identity_sha256: inputs.storage_identity_sha256,
        profile_pins: inputs.context.profile_pins(),
        target_pins: inputs.context.target_pins(),
        head: inputs.head,
        item_heads: inputs.item_heads,
        sources: BTreeMap::new(),
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
    pub capability: &'a CatalogCapability,
    pub resource_id: &'a CatalogResourceId,
    pub context: &'a ResolvedCatalogContext,
    pub inventory: &'a CatalogLocalInventory,
    pub requested_name: &'a str,
    pub name_taken: &'a dyn Fn(&CatalogTargetConfig, &str) -> bool,
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
    if record.resource.coordinate() != inputs.capability.coordinate() {
        return Err("the Fork source does not belong to this Catalog row".to_string());
    }
    validate_component_name("Fork copy name", inputs.requested_name, 128)?;
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
    let identity = record.target.decode()?;
    let plan = CatalogResourceForkPlan {
        plan_id: CatalogPlanId::generate()?,
        scope_id: inputs.scope.scope_id.clone(),
        profile_pins: inputs.context.profile_pins(),
        target_pins: inputs.context.target_pins(),
        source_coordinate: inputs.capability.coordinate(),
        source_resource_id: (*inputs.resource_id).clone(),
        source_display_name: inputs.capability.codec.display_name(&record.resource)?,
        source_target: record.target.clone(),
        source_path: source_path.to_string_lossy().to_string(),
        source_content_sha256: record.resource.content_sha256.clone(),
        copy_name: inputs.requested_name.to_string(),
        copy_resource_id,
        copy_target_path: Path::new(&identity.canonical_root)
            .join(inputs.requested_name)
            .to_string_lossy()
            .to_string(),
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
    use super::super::adapters::ResolvedCatalogTarget;
    use super::super::domain::{
        CatalogActionOutcome, CatalogCommitId, CatalogCoordinate, CatalogLinkId,
        CatalogResourceRepositoryId, CatalogScopeId, CatalogTargetKey, LocalCatalogProfileId,
        ProviderKey,
    };
    use super::super::profiles::ResolvedCatalogProfile;
    use super::super::registry::CatalogRegistry;
    use super::*;
    use std::fs;
    use std::sync::Arc;

    struct Fixture {
        _temp: tempfile::TempDir,
        registry: Arc<CatalogRegistry>,
        capability: Arc<CatalogCapability>,
        context: ResolvedCatalogContext,
        skills_root: PathBuf,
        /// A second tree the "stored" side captures from.
        stored_root: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let skills_root = home.join("skills");
        let stored_root = temp.path().join("stored");
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&stored_root).unwrap();
        let registry = CatalogRegistry::production();
        let capability = registry
            .capability(
                &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap(),
            )
            .unwrap();
        let context = ResolvedCatalogContext {
            profile: ResolvedCatalogProfile {
                profile_id: LocalCatalogProfileId::generate().unwrap(),
                provider: ProviderKey::parse("codex").unwrap(),
                revision: 1,
                canonical_home: home,
            },
            targets: vec![ResolvedCatalogTarget {
                target: CatalogTargetKey::parse("provider-home.skills").unwrap(),
                canonical_root: skills_root.clone(),
                root_class: "provider-home".to_string(),
                summary: "Codex home".to_string(),
                exclude_provider_owned: true,
            }],
        };
        Fixture {
            _temp: temp,
            registry,
            capability,
            context,
            skills_root,
            stored_root,
        }
    }

    fn write_skill(root: &Path, install_directory: &str, declared_name: &str, body: &str) {
        let dir = root.join(install_directory);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {}\n---\n{}\n", declared_name, body),
        )
        .unwrap();
    }

    /// A local record captured from the fixture's skills root.
    fn skill_record(
        fixture: &Fixture,
        install_directory: &str,
        declared_name: &str,
        body: &str,
    ) -> LocalResourceRecord {
        write_skill(&fixture.skills_root, install_directory, declared_name, body);
        let path = fixture.skills_root.join(install_directory);
        let captured = fixture
            .capability
            .codec
            .capture_directory(&path, "provider-home")
            .unwrap();
        let resource = captured.to_envelope(&fixture.capability.coordinate());
        LocalResourceRecord {
            target: CatalogTargetConfig::new(
                ProviderKey::parse("codex").unwrap(),
                CatalogTargetKey::parse("provider-home.skills").unwrap(),
                &CatalogTargetIdentity {
                    canonical_root: fixture.skills_root.to_string_lossy().to_string(),
                    root_class: "provider-home".to_string(),
                    member: install_directory.to_string(),
                },
            )
            .unwrap(),
            resource,
            capability: fixture.capability.clone(),
            target_path: Some(path),
            local_modified_at: Some(1),
            warnings: Vec::new(),
            captured: Some(captured),
        }
    }

    /// A stored envelope for the same identity, captured from a separate tree.
    fn stored_skill(
        fixture: &Fixture,
        install_directory: &str,
        declared_name: &str,
        body: &str,
    ) -> CatalogResourceEnvelope {
        write_skill(&fixture.stored_root, install_directory, declared_name, body);
        fixture
            .capability
            .codec
            .capture_directory(
                &fixture.stored_root.join(install_directory),
                "provider-home",
            )
            .unwrap()
            .to_envelope(&fixture.capability.coordinate())
    }

    fn inventory(records: Vec<LocalResourceRecord>) -> CatalogLocalInventory {
        CatalogLocalInventory {
            resources: records
                .into_iter()
                .map(|record| (record.resource.resource_id.clone(), record))
                .collect(),
            siblings: BTreeMap::new(),
            blocked: Vec::new(),
            warnings: Vec::new(),
            native: None,
        }
    }

    fn scope(fixture: &Fixture) -> CatalogScope {
        let coordinate = fixture.capability.coordinate();
        CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            provider: coordinate.provider,
            profile_id: fixture.context.profile.profile_id.clone(),
            resource_kind: coordinate.resource_kind,
            format: coordinate.format,
            format_version: coordinate.format_version,
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn link(
        fixture: &Fixture,
        scope: &CatalogScope,
        accepted: &[(CatalogResourceId, String)],
    ) -> CatalogScopeLink {
        CatalogScopeLink {
            link_id: CatalogLinkId::generate().unwrap(),
            scope_id: scope.scope_id.clone(),
            storage_id: StorageId::parse("storage-a").unwrap(),
            repository_id: fixture.capability.repository_id(),
            accepted_head: None,
            accepted_resources: accepted.iter().cloned().collect(),
            accepted_items: BTreeMap::new(),
            cloud_repositories: BTreeMap::new(),
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn head(link: &CatalogScopeLink) -> LibraryHeadCoordinate {
        LibraryHeadCoordinate {
            repository_id: link.repository_id.clone(),
            revision: 3,
            commit_id: CatalogCommitId::generate().unwrap(),
            manifest_sha256: "d".repeat(64),
            manifest_size: 10,
            storage_identity_sha256: "e".repeat(64),
        }
    }

    fn never(_: &CatalogTargetConfig, _: &str) -> bool {
        false
    }

    #[test]
    fn derive_link_state_covers_the_nine_states() {
        let base = Some("a");
        let table = [
            (None, Some("x"), None, false, CatalogLinkState::New),
            (None, None, None, false, CatalogLinkState::LocalMissing),
            (None, None, Some("x"), false, CatalogLinkState::LocalMissing),
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
        let fixture = fixture();
        let scope = scope(&fixture);
        let changed = skill_record(&fixture, "review", "review", "Changed locally.");
        let changed_id = changed.resource.resource_id.clone();
        let local_only = skill_record(&fixture, "fresh", "fresh", "Local only.");
        let stored_review = stored_skill(&fixture, "review", "review", "Original body.");
        let changed_base = stored_review.content_sha256.clone();
        let stored_only = stored_skill(&fixture, "remote", "remote", "Stored only.");
        assert_eq!(stored_review.resource_id, changed_id);

        let remote = BTreeMap::from([
            (changed_id.clone(), stored_review),
            (stored_only.resource_id.clone(), stored_only),
        ]);
        let link = link(&fixture, &scope, &[(changed_id.clone(), changed_base)]);

        let plan = build_scope_push_plan(ScopePushPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_is_local: true,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(vec![local_only, changed]),
            remote: &remote,
            observed_head: Some(head(&link)),
            observed_items: BTreeMap::new(),
            blocked: BTreeMap::new(),
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
        assert_eq!(fresh.row.labels.kind_label, "Skills");
        assert_eq!(fresh.row.scope_label.as_deref(), Some("Provider home"));
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
        assert_eq!(plan.target_pins.len(), 1);
    }

    #[test]
    fn a_scope_pull_plan_items_every_stored_resource_and_installs_the_missing_one() {
        let fixture = fixture();
        let scope = scope(&fixture);
        let stored = stored_skill(&fixture, "review", "review", "Stored body.");
        let remote = BTreeMap::from([(stored.resource_id.clone(), stored.clone())]);
        let link = link(&fixture, &scope, &[]);

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(Vec::new()),
            remote: &remote,
            head: Some(head(&link)),
            item_heads: BTreeMap::new(),
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
        let action = &item.item.actions[0];
        assert!(matches!(action.kind, CatalogPullActionKind::Install));
        assert_eq!(
            action.expected_target_state,
            CatalogTargetExpectation::Absent
        );
        assert!(action.native.is_none());
        assert_eq!(
            action.target_path.as_deref(),
            Some(
                fixture
                    .skills_root
                    .join("review")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(action.target.decode().unwrap().member, "review");
    }

    #[test]
    fn two_preservation_copies_in_one_review_get_distinct_names() {
        let fixture = fixture();
        let scope = scope(&fixture);
        // Two directory names identical up to the 96-character copy-name stem:
        // their generated copy names would collide without the per-plan
        // claimed-names set.
        let stem = "a".repeat(96);
        let first_name = format!("{stem}b");
        let second_name = format!("{stem}c");
        let first_local = skill_record(&fixture, &first_name, "review-one", "Local a.");
        let second_local = skill_record(&fixture, &second_name, "review-two", "Local b.");
        let first_stored = stored_skill(&fixture, &first_name, "review-one", "Stored a.");
        let second_stored = stored_skill(&fixture, &second_name, "review-two", "Stored b.");
        let remote = BTreeMap::from([
            (first_stored.resource_id.clone(), first_stored),
            (second_stored.resource_id.clone(), second_stored),
        ]);
        let link = link(&fixture, &scope, &[]);

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(vec![first_local, second_local]),
            remote: &remote,
            head: Some(head(&link)),
            item_heads: BTreeMap::new(),
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
        let fixture = fixture();
        let scope = scope(&fixture);
        let local = skill_record(&fixture, "review", "review", "Base body.");
        let base = local.resource.content_sha256.clone();
        let stored = stored_skill(&fixture, "review", "review", "Newer stored body.");
        let resource_id = stored.resource_id.clone();
        let remote = BTreeMap::from([(resource_id.clone(), stored)]);
        let link = link(&fixture, &scope, &[(resource_id.clone(), base)]);
        let claims = vec![CatalogClaim {
            target: local.target.clone(),
            coordinate: fixture.capability.coordinate(),
            storage_id: StorageId::parse("storage-other").unwrap(),
            resource_id: resource_id.clone(),
            content_sha256: "9".repeat(64),
            outcome: CatalogActionOutcome::Updated,
            recorded_at: 1,
            plan_id: None,
        }];

        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(vec![local]),
            remote: &remote,
            head: Some(head(&link)),
            item_heads: BTreeMap::new(),
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
        let pin = item.item.actions[0].kind.preservation().unwrap();
        assert_eq!(pin.copy_name, "review-local-copy");
        assert!(pin.copy_target_path.ends_with("skills/review-local-copy"));
        // The copy keeps its content transform reproducible: renaming the
        // same capture again yields the same identity and digest.
        let inventory = inventory(vec![skill_record(
            &fixture,
            "review",
            "review",
            "Base body.",
        )]);
        let (copy_id, transformed) = transformed_copy_identity(
            inventory.resources.values().next().unwrap(),
            "review-local-copy",
        )
        .unwrap();
        assert_eq!(copy_id, pin.copy_resource_id);
        assert_eq!(transformed, pin.transformed_content_sha256);
        assert_ne!(copy_id, resource_id);
    }

    #[test]
    fn a_foreign_occupant_at_the_target_is_preserved_not_clobbered() {
        let fixture = fixture();
        let scope = scope(&fixture);
        // The same directory holds a skill declaring another name: a
        // different resource identity occupies the stored resource's target.
        let occupant = skill_record(&fixture, "review", "review-renamed", "Local body.");
        let stored = stored_skill(&fixture, "review", "review", "Stored body.");
        assert_ne!(occupant.resource.resource_id, stored.resource_id);
        let remote = BTreeMap::from([(stored.resource_id.clone(), stored)]);
        let link = link(&fixture, &scope, &[]);
        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(vec![occupant]),
            remote: &remote,
            head: Some(head(&link)),
            item_heads: BTreeMap::new(),
            claims: &[],
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();
        let item = &plan.items[0];
        assert_eq!(item.item.relation, CatalogResourceRelation::StorageOnly);
        assert!(item.item.actions[0].kind.preservation().is_some());
        assert!(item.item.actions[1].kind.is_destructive_overwrite());
    }

    #[test]
    fn pull_selections_validate_choices_and_destructive_acknowledgements() {
        let fixture = fixture();
        let scope = scope(&fixture);
        let local = skill_record(&fixture, "review", "review", "Local body.");
        let stored = stored_skill(&fixture, "review", "review", "Stored body.");
        let resource_id = stored.resource_id.clone();
        let remote = BTreeMap::from([(resource_id.clone(), stored)]);
        let link = link(&fixture, &scope, &[(resource_id.clone(), "b".repeat(64))]);
        let plan = build_scope_pull_plan(ScopePullPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            link: &link,
            storage_identity_sha256: "c".repeat(64),
            context: &fixture.context,
            inventory: &inventory(vec![local]),
            remote: &remote,
            head: Some(head(&link)),
            item_heads: BTreeMap::new(),
            claims: &[],
            requested_copy_names: &BTreeMap::new(),
            name_taken: &never,
            now: 100,
        })
        .unwrap();
        let item = &plan.items[0];
        assert_eq!(item.item.relation, CatalogResourceRelation::ChangedBoth);
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
            stored_skill(&fixture, "other", "other", "x").resource_id,
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
        let fixture = fixture();
        let record = skill_record(&fixture, "review", "review", "body");
        let target = record.target.clone();
        let claim = |storage: &str, digest: char| CatalogClaim {
            target: target.clone(),
            coordinate: fixture.capability.coordinate(),
            storage_id: StorageId::parse(storage).unwrap(),
            resource_id: record.resource.resource_id.clone(),
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

        // A vacant target carries no conflict pins at all.
        assert!(conflicting_claim_pins(
            &claims,
            &inventory(Vec::new()),
            &StorageId::parse("storage-current").unwrap(),
            &target,
            &"c".repeat(64)
        )
        .is_empty());
    }

    #[test]
    fn a_fork_plan_is_scope_keyed_and_coordinate_guarded() {
        let fixture = fixture();
        let scope = scope(&fixture);
        let record = skill_record(&fixture, "review", "review", "Fork me.");
        let resource_id = record.resource.resource_id.clone();
        let plan = build_resource_fork_plan(ScopeForkPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            resource_id: &resource_id,
            context: &fixture.context,
            inventory: &inventory(vec![record]),
            requested_name: "review-fork",
            name_taken: &never,
            now: 100,
        })
        .unwrap();
        assert_eq!(plan.scope_id, scope.scope_id);
        assert_eq!(plan.copy_name, "review-fork");
        assert_eq!(plan.source_coordinate, fixture.capability.coordinate());
        assert!(plan.copy_target_path.ends_with("skills/review-fork"));
        assert_ne!(plan.copy_resource_id, resource_id);

        // A row of another coordinate cannot fork this record even if the
        // id resolves in the inventory it was handed.
        let pets = fixture
            .registry
            .capability(
                &CatalogCoordinate::new("codex", "desktop_pet", "openai.codex-pet", 1).unwrap(),
            )
            .unwrap();
        let record = skill_record(&fixture, "review", "review", "Fork me.");
        let error = build_resource_fork_plan(ScopeForkPlanInputs {
            scope: &scope,
            capability: &pets,
            resource_id: &resource_id,
            context: &fixture.context,
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

        // A taken name is refused before any transform runs.
        let record = skill_record(&fixture, "review", "review", "Fork me.");
        let taken = |_: &CatalogTargetConfig, _: &str| true;
        assert!(build_resource_fork_plan(ScopeForkPlanInputs {
            scope: &scope,
            capability: &fixture.capability,
            resource_id: &resource_id,
            context: &fixture.context,
            inventory: &inventory(vec![record]),
            requested_name: "review-fork",
            name_taken: &taken,
            now: 100,
        })
        .unwrap_err()
        .contains("already in use"));
    }

    #[test]
    fn targets_resolve_by_root_class_and_fail_when_the_root_is_absent() {
        let fixture = fixture();
        let stored = stored_skill(&fixture, "review", "review", "body");
        let (target, path) =
            target_for_resource(&fixture.context, &fixture.capability, &stored).unwrap();
        assert_eq!(target.target.as_str(), "provider-home.skills");
        assert_eq!(path.unwrap(), fixture.skills_root.join("review"));

        // A shared-user skill needs the shared root, which this profile does
        // not resolve.
        let shared = fixture
            .capability
            .codec
            .capture_directory(&fixture.stored_root.join("review"), "shared-user")
            .unwrap()
            .to_envelope(&fixture.capability.coordinate());
        let error =
            target_for_resource(&fixture.context, &fixture.capability, &shared).unwrap_err();
        assert!(error.contains("no shared-user target is active"), "{error}");
        assert_eq!(
            CatalogResourceRepositoryId::for_coordinate(&fixture.capability.coordinate()),
            fixture.capability.repository_id()
        );
    }
}

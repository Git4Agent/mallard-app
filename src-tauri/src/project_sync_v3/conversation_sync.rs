//! Pure conversation-variant reconciliation and reviewed-plan pinning.
//!
//! Filesystem, Tauri, and object-store code call these functions rather than
//! reimplementing relation-to-action policy. This keeps Push, Pull, recovery,
//! and the integration harness on one state machine.

use std::collections::{BTreeMap, BTreeSet};

use super::domain::{
    classify_conversation_relation, ActionId, CanonicalConversationMessage, ConflictGroupId,
    ConversationBindings, ConversationGroup, ConversationPlanAction, ConversationPlanActionType,
    ConversationPlanDirection, ConversationReasonCode, ConversationRelation, HostId,
    LocalConversationBinding, LocalConversationPreference, LocalProjectId, LocalProviderProfileId,
    Provider, ProviderStateExpectation, ReplicaId, ResourceId, TargetExpectation, VariantId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingResolution {
    Bound,
    UniqueCompatibleAncestor,
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationCapabilities {
    pub binding: BindingResolution,
    pub local_safely_capturable: bool,
    pub cloud_safely_readable: bool,
    pub destination_codec_resumable: bool,
    pub provider_quiescent: bool,
    /// True only when raw bytes differ solely through fields the verified
    /// codec declares structural (thread ID and mapped cwd).
    pub structural_rewrite_only: bool,
}

impl Default for ConversationCapabilities {
    fn default() -> Self {
        Self {
            binding: BindingResolution::Bound,
            local_safely_capturable: true,
            cloud_safely_readable: true,
            destination_codec_resumable: true,
            provider_quiescent: true,
            structural_rewrite_only: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationDecision {
    pub action: ConversationPlanActionType,
    pub reason_code: ConversationReasonCode,
}

/// Map a proven relation to one typed default action. Explicit promotions,
/// replacement, and tombstones are never synthesized here; callers add those
/// only from reviewed user intent.
pub fn default_conversation_decision(
    direction: ConversationPlanDirection,
    relation: ConversationRelation,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    use ConversationPlanActionType as Action;
    use ConversationReasonCode as Reason;
    use ConversationRelation as Relation;

    if relation == Relation::Identical && capabilities.structural_rewrite_only {
        return ConversationDecision {
            action: Action::NoOp,
            reason_code: Reason::StructuralRewriteOnly,
        };
    }

    match direction {
        ConversationPlanDirection::Push => match relation {
            Relation::Identical => decision(Action::NoOp, Reason::Identical),
            Relation::CloudPrefixOfLocal => {
                if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Missing | BindingResolution::Ambiguous
                ) {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                } else {
                    decision(Action::FastForwardCloud, Reason::CloudPrefixOfLocal)
                }
            }
            Relation::LocalPrefixOfCloud => {
                decision(Action::RequirePull, Reason::LocalPrefixOfCloud)
            }
            Relation::Diverged => {
                if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Bound | BindingResolution::UniqueCompatibleAncestor
                ) {
                    decision(Action::AddAlternate, Reason::Diverged)
                } else {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                }
            }
            Relation::Unknown => {
                if capabilities.local_safely_capturable
                    && capabilities.binding == BindingResolution::Bound
                {
                    decision(Action::AddAlternate, Reason::UnknownLineage)
                } else {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else if !capabilities.local_safely_capturable {
                            Reason::CodecUnsupported
                        } else {
                            Reason::MissingBinding
                        },
                    )
                }
            }
            Relation::LocalMissing => decision(Action::NoOp, Reason::LocalMissing),
            Relation::CloudMissing => {
                if capabilities.local_safely_capturable {
                    decision(Action::AddAlternate, Reason::CloudMissing)
                } else {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                }
            }
            Relation::BothMissing => decision(Action::NoOp, Reason::BothMissing),
        },
        ConversationPlanDirection::Pull => match relation {
            Relation::Identical => decision(Action::NoOp, Reason::Identical),
            Relation::CloudPrefixOfLocal => decision(Action::KeepLocal, Reason::CloudPrefixOfLocal),
            Relation::LocalPrefixOfCloud => {
                if !capabilities.cloud_safely_readable || !capabilities.destination_codec_resumable
                {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                } else if !capabilities.provider_quiescent {
                    decision(Action::StoreOnly, Reason::ProviderNotQuiescent)
                } else {
                    decision(Action::FastForwardLocal, Reason::LocalPrefixOfCloud)
                }
            }
            Relation::Diverged => {
                if capabilities.cloud_safely_readable && capabilities.destination_codec_resumable {
                    decision(Action::AddAlternate, Reason::Diverged)
                } else {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                }
            }
            Relation::Unknown => decision(Action::StoreOnly, Reason::UnknownLineage),
            Relation::LocalMissing => {
                if capabilities.cloud_safely_readable && capabilities.destination_codec_resumable {
                    decision(Action::AddAlternate, Reason::LocalMissing)
                } else {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                }
            }
            Relation::CloudMissing => decision(Action::KeepLocal, Reason::CloudMissing),
            Relation::BothMissing => decision(Action::NoOp, Reason::BothMissing),
        },
    }
}

fn decision(
    action: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
) -> ConversationDecision {
    ConversationDecision {
        action,
        reason_code,
    }
}

/// Immutable inputs used to build one reviewed conversation action. Keeping
/// these values together makes it difficult for Push, Pull, or a test facade
/// to omit one of the concurrency pins.
#[derive(Clone, Debug)]
pub struct ConversationPlanActionRequest<'a> {
    pub action_id: ActionId,
    pub direction: ConversationPlanDirection,
    pub local_chain: Option<&'a [CanonicalConversationMessage]>,
    pub cloud_chain: Option<&'a [CanonicalConversationMessage]>,
    pub capabilities: ConversationCapabilities,
    pub provider: Provider,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub conflict_group_id: ConflictGroupId,
    pub cloud_variant_id: Option<VariantId>,
    pub local_variant_id: Option<VariantId>,
    pub result_local_variant_id: Option<VariantId>,
    pub source_resource_id: Option<ResourceId>,
    pub proposed_provider_thread_id: Option<String>,
    pub required_codec: String,
    pub pinned_generation: u64,
    pub pinned_commit_id: String,
    pub pinned_manifest_sha256: String,
    pub pinned_lineage_digest: Option<String>,
    pub pinned_semantic_digest: Option<String>,
    pub expected_local_target: TargetExpectation,
    pub expected_provider_state: Vec<ProviderStateExpectation>,
    pub expected_profile_revision: u64,
    pub expected_project_binding_revision: u64,
    pub expected_host_identity_revision: u64,
    pub expected_bindings_document_revision: u64,
    pub expected_binding_revision: Option<u64>,
    pub expected_preference_revision: Option<u64>,
    pub expires_at: u64,
}

/// Build the exact typed action that is serialized into a reviewed plan.
/// This function is pure: ID reservations and target expectations are copied
/// into the plan, but provider, binding, and cloud state are never changed.
pub fn build_conversation_plan_action(
    request: ConversationPlanActionRequest<'_>,
) -> Result<ConversationPlanAction, String> {
    let relation = classify_conversation_relation(request.local_chain, request.cloud_chain);
    let decision = default_conversation_decision(request.direction, relation, request.capabilities);
    let action = ConversationPlanAction {
        action_id: request.action_id,
        direction: request.direction,
        action: decision.action,
        reason_code: decision.reason_code,
        relation,
        provider: request.provider,
        host_id: request.host_id,
        replica_id: request.replica_id,
        local_project_id: request.local_project_id,
        provider_profile_id: request.provider_profile_id,
        conflict_group_id: request.conflict_group_id,
        cloud_variant_id: request.cloud_variant_id,
        local_variant_id: request.local_variant_id,
        result_local_variant_id: request.result_local_variant_id,
        source_resource_id: request.source_resource_id,
        proposed_provider_thread_id: request.proposed_provider_thread_id,
        required_codec: request.required_codec,
        pinned_generation: request.pinned_generation,
        pinned_commit_id: request.pinned_commit_id,
        pinned_manifest_sha256: request.pinned_manifest_sha256,
        pinned_lineage_digest: request.pinned_lineage_digest,
        pinned_semantic_digest: request.pinned_semantic_digest,
        expected_local_target: request.expected_local_target,
        expected_provider_state: request.expected_provider_state,
        expected_profile_revision: request.expected_profile_revision,
        expected_project_binding_revision: request.expected_project_binding_revision,
        expected_host_identity_revision: request.expected_host_identity_revision,
        expected_bindings_document_revision: request.expected_bindings_document_revision,
        expected_binding_revision: request.expected_binding_revision,
        expected_preference_revision: request.expected_preference_revision,
        expires_at: request.expires_at,
    };
    action.validate()?;
    Ok(action)
}

#[derive(Clone, Debug)]
pub struct BindingRecoveryCandidate<'a> {
    pub variant_id: VariantId,
    pub chain: Option<&'a [CanonicalConversationMessage]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingRecoveryResult {
    Unique(VariantId),
    Missing,
    Ambiguous,
    Unknown,
}

/// Recover a missing binding only from one unique longest compatible cloud
/// ancestor. Names, titles, and provider IDs are intentionally absent from
/// this API and therefore cannot influence the result.
pub fn recover_unique_compatible_binding(
    local_chain: Option<&[CanonicalConversationMessage]>,
    candidates: &[BindingRecoveryCandidate<'_>],
) -> BindingRecoveryResult {
    let Some(local_chain) = local_chain else {
        return BindingRecoveryResult::Unknown;
    };
    if candidates.iter().any(|candidate| candidate.chain.is_none()) {
        return BindingRecoveryResult::Unknown;
    }
    let compatible = candidates
        .iter()
        .filter_map(|candidate| {
            let chain = candidate.chain?;
            matches!(
                classify_conversation_relation(Some(local_chain), Some(chain)),
                ConversationRelation::Identical | ConversationRelation::CloudPrefixOfLocal
            )
            .then_some((chain.len(), candidate.variant_id.clone()))
        })
        .collect::<Vec<_>>();
    let Some(longest) = compatible.iter().map(|(length, _)| *length).max() else {
        return BindingRecoveryResult::Missing;
    };
    let mut longest_matches = compatible
        .into_iter()
        .filter(|(length, _)| *length == longest)
        .map(|(_, variant_id)| variant_id);
    let Some(result) = longest_matches.next() else {
        return BindingRecoveryResult::Missing;
    };
    if longest_matches.next().is_some() {
        BindingRecoveryResult::Ambiguous
    } else {
        BindingRecoveryResult::Unique(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TombstoneDecision {
    RejectUnapproved,
    RejectPrimaryWithoutReplacement,
    RejectStaleDigest,
    PreserveModifiedInSameGroup,
    TombstoneVariant,
    TombstoneGroup,
}

/// Decide a reviewed deletion without treating absence as intent. A modified
/// local copy always survives; deleting the last branch uses a group
/// tombstone, and deleting the primary requires a replacement in the same
/// reviewed publication. A modified branch wins a group-tombstone race and
/// resurrects the same conflict-group identity so bindings remain stable.
pub fn review_conversation_tombstone(
    group: &ConversationGroup,
    variant_id: &VariantId,
    reviewed_tree_digest: &str,
    current_tree_digest: &str,
    approved: bool,
    replacement_primary: Option<&VariantId>,
) -> TombstoneDecision {
    if !approved {
        return TombstoneDecision::RejectUnapproved;
    }
    if reviewed_tree_digest != current_tree_digest {
        return TombstoneDecision::PreserveModifiedInSameGroup;
    }
    if !group.variants.contains_key(variant_id) {
        return TombstoneDecision::RejectStaleDigest;
    }
    if group.variants.len() == 1 {
        return TombstoneDecision::TombstoneGroup;
    }
    if group.primary_variant_id == *variant_id
        && replacement_primary.is_none_or(|replacement| {
            replacement == variant_id || !group.variants.contains_key(replacement)
        })
    {
        return TombstoneDecision::RejectPrimaryWithoutReplacement;
    }
    TombstoneDecision::TombstoneVariant
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationStalePin {
    Expired,
    Host,
    Replica,
    LocalProject,
    ProviderProfile,
    HostIdentityRevision,
    Generation,
    Commit,
    Manifest,
    LineageDigest,
    SemanticDigest,
    LocalTarget,
    ProviderState,
    ProfileRevision,
    ProjectBindingRevision,
    BindingsDocumentRevision,
    BindingRevision,
    PreferenceRevision,
    Codec,
    ProviderQuiescence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationApplyContext {
    pub now: u64,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub host_identity_revision: u64,
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub lineage_digest: Option<String>,
    pub semantic_digest: Option<String>,
    pub local_target: TargetExpectation,
    pub provider_state: BTreeMap<String, TargetExpectation>,
    pub profile_revision: u64,
    pub project_binding_revision: u64,
    pub bindings_document_revision: u64,
    pub binding_revision: Option<u64>,
    pub preference_revision: Option<u64>,
    pub codec: String,
    pub provider_quiescent: bool,
}

/// Revalidate every reviewed pin before a caller writes a journal record or
/// mutates provider/cloud state. The stable comparison order gives callers a
/// deterministic, typed rejection reason while still rejecting all changes.
pub fn validate_conversation_action_pins(
    action: &ConversationPlanAction,
    current: &ConversationApplyContext,
) -> Result<(), ConversationStalePin> {
    if current.now > action.expires_at {
        return Err(ConversationStalePin::Expired);
    }
    if current.host_id != action.host_id {
        return Err(ConversationStalePin::Host);
    }
    if current.replica_id != action.replica_id {
        return Err(ConversationStalePin::Replica);
    }
    if current.local_project_id != action.local_project_id {
        return Err(ConversationStalePin::LocalProject);
    }
    if current.provider_profile_id != action.provider_profile_id {
        return Err(ConversationStalePin::ProviderProfile);
    }
    if current.host_identity_revision != action.expected_host_identity_revision {
        return Err(ConversationStalePin::HostIdentityRevision);
    }
    if current.generation != action.pinned_generation {
        return Err(ConversationStalePin::Generation);
    }
    if current.commit_id != action.pinned_commit_id {
        return Err(ConversationStalePin::Commit);
    }
    if current.manifest_sha256 != action.pinned_manifest_sha256 {
        return Err(ConversationStalePin::Manifest);
    }
    if current.lineage_digest != action.pinned_lineage_digest {
        return Err(ConversationStalePin::LineageDigest);
    }
    if current.semantic_digest != action.pinned_semantic_digest {
        return Err(ConversationStalePin::SemanticDigest);
    }
    if current.local_target != action.expected_local_target {
        return Err(ConversationStalePin::LocalTarget);
    }
    let planned_provider_state = action
        .expected_provider_state
        .iter()
        .map(|target| (target.target_path.clone(), target.expectation.clone()))
        .collect::<BTreeMap<_, _>>();
    if current.provider_state != planned_provider_state {
        return Err(ConversationStalePin::ProviderState);
    }
    if current.profile_revision != action.expected_profile_revision {
        return Err(ConversationStalePin::ProfileRevision);
    }
    if current.project_binding_revision != action.expected_project_binding_revision {
        return Err(ConversationStalePin::ProjectBindingRevision);
    }
    if current.bindings_document_revision != action.expected_bindings_document_revision {
        return Err(ConversationStalePin::BindingsDocumentRevision);
    }
    if current.binding_revision != action.expected_binding_revision {
        return Err(ConversationStalePin::BindingRevision);
    }
    if current.preference_revision != action.expected_preference_revision {
        return Err(ConversationStalePin::PreferenceRevision);
    }
    if current.codec != action.required_codec {
        return Err(ConversationStalePin::Codec);
    }
    if matches!(
        action.action,
        ConversationPlanActionType::FastForwardLocal | ConversationPlanActionType::ReplaceLocal
    ) && !current.provider_quiescent
    {
        return Err(ConversationStalePin::ProviderQuiescence);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationBindingScope {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub conflict_group_id: super::domain::ConflictGroupId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationRekeyResult {
    pub binding: LocalConversationBinding,
    pub preference: Option<LocalConversationPreference>,
}

/// Perform the machine-local half of a same-ID divergence. Callers invoke
/// this inside `V3Repository::mutate_conversation_bindings`, making the binding
/// rekey and preference-pointer move one atomic document replacement. The
/// provider thread ID and transcript are deliberately untouched.
pub fn rekey_same_id_divergence(
    document: &mut ConversationBindings,
    scope: &ConversationBindingScope,
    old_variant_id: &VariantId,
    result_variant_id: &VariantId,
    expected_document_revision: u64,
    expected_binding_revision: u64,
    expected_preference_revision: Option<u64>,
    updated_at: u64,
) -> Result<ConversationRekeyResult, String> {
    if document.revision != expected_document_revision {
        return Err("conversation bindings changed after review".to_string());
    }
    if old_variant_id == result_variant_id {
        return Err("same-id divergence must allocate a distinct variant id".to_string());
    }
    if document.bindings.iter().any(|binding| {
        binding.host_id == scope.host_id
            && binding.replica_id == scope.replica_id
            && binding.local_project_id == scope.local_project_id
            && binding.provider_profile_id == scope.provider_profile_id
            && binding.conflict_group_id == scope.conflict_group_id
            && binding.variant_id == *result_variant_id
    }) {
        return Err("same-id divergence result variant is already bound".to_string());
    }

    let binding_index = document
        .bindings
        .iter()
        .position(|binding| binding_matches_scope(binding, scope, old_variant_id))
        .ok_or_else(|| "same-id divergence binding disappeared".to_string())?;
    if document.bindings[binding_index].revision != expected_binding_revision {
        return Err("conversation binding changed after review".to_string());
    }
    let preference_index = document
        .preferences
        .iter()
        .position(|preference| preference_matches_scope(preference, scope));
    match (preference_index, expected_preference_revision) {
        (Some(index), Some(expected_revision))
            if document.preferences[index].revision != expected_revision =>
        {
            return Err("conversation preference changed after review".to_string());
        }
        (Some(_), None) => {
            return Err("conversation preference appeared after review".to_string());
        }
        (None, Some(_)) => {
            return Err("conversation preference disappeared after review".to_string());
        }
        _ => {}
    }

    let binding = &mut document.bindings[binding_index];
    binding.variant_id = result_variant_id.clone();
    binding.revision = binding.revision.saturating_add(1);
    binding.updated_at = updated_at;
    let binding_after = binding.clone();

    let preference_after = preference_index.map(|index| {
        let preference = &mut document.preferences[index];
        if preference.preferred_variant_id == *old_variant_id {
            preference.preferred_variant_id = result_variant_id.clone();
            preference.revision = preference.revision.saturating_add(1);
            preference.updated_at = updated_at;
        }
        preference.clone()
    });
    Ok(ConversationRekeyResult {
        binding: binding_after,
        preference: preference_after,
    })
}

fn binding_matches_scope(
    binding: &LocalConversationBinding,
    scope: &ConversationBindingScope,
    variant_id: &VariantId,
) -> bool {
    binding.host_id == scope.host_id
        && binding.replica_id == scope.replica_id
        && binding.local_project_id == scope.local_project_id
        && binding.provider_profile_id == scope.provider_profile_id
        && binding.conflict_group_id == scope.conflict_group_id
        && binding.variant_id == *variant_id
}

fn preference_matches_scope(
    preference: &LocalConversationPreference,
    scope: &ConversationBindingScope,
) -> bool {
    preference.host_id == scope.host_id
        && preference.replica_id == scope.replica_id
        && preference.local_project_id == scope.local_project_id
        && preference.provider_profile_id == scope.provider_profile_id
        && preference.conflict_group_id == scope.conflict_group_id
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderIdRequest {
    pub variant_id: VariantId,
    pub source_hint: Option<String>,
    pub primary: bool,
}

/// Reserve provider IDs in deterministic plan order: primary first, then
/// sorted variant ID. Existing IDs always win collisions. Generated values
/// come from an injected allocator so plans persist one chosen value and do
/// not depend on recalculation during retries.
pub fn reserve_provider_thread_ids(
    requests: impl IntoIterator<Item = ProviderIdRequest>,
    occupied: &BTreeSet<String>,
    mut allocate: impl FnMut() -> Result<String, String>,
) -> Result<BTreeMap<VariantId, String>, String> {
    let mut requests = requests.into_iter().collect::<Vec<_>>();
    requests.sort_by(|left, right| {
        right
            .primary
            .cmp(&left.primary)
            .then_with(|| left.variant_id.cmp(&right.variant_id))
    });
    let mut reserved = occupied.clone();
    let mut result = BTreeMap::new();
    for request in requests {
        let chosen = match request.source_hint {
            Some(hint) if valid_provider_thread_id(&hint) && !reserved.contains(&hint) => hint,
            _ => loop {
                let candidate = allocate()?;
                if !valid_provider_thread_id(&candidate) {
                    return Err("provider ID allocator returned an invalid value".to_string());
                }
                if !reserved.contains(&candidate) {
                    break candidate;
                }
            },
        };
        reserved.insert(chosen.clone());
        result.insert(request.variant_id, chosen);
    }
    Ok(result)
}

fn valid_provider_thread_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_policy_exposes_both_push_prefix_outcomes() {
        let capabilities = ConversationCapabilities::default();
        assert_eq!(
            default_conversation_decision(
                ConversationPlanDirection::Push,
                ConversationRelation::CloudPrefixOfLocal,
                capabilities,
            )
            .action,
            ConversationPlanActionType::FastForwardCloud
        );
        assert_eq!(
            default_conversation_decision(
                ConversationPlanDirection::Push,
                ConversationRelation::LocalPrefixOfCloud,
                capabilities,
            )
            .action,
            ConversationPlanActionType::RequirePull
        );
    }

    #[test]
    fn unknown_policy_never_guesses_a_missing_binding() {
        let bound = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Unknown,
            ConversationCapabilities::default(),
        );
        assert_eq!(bound.action, ConversationPlanActionType::AddAlternate);

        let missing = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Unknown,
            ConversationCapabilities {
                binding: BindingResolution::Missing,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(missing.action, ConversationPlanActionType::Blocked);
        assert_eq!(missing.reason_code, ConversationReasonCode::MissingBinding);

        let pull = default_conversation_decision(
            ConversationPlanDirection::Pull,
            ConversationRelation::Unknown,
            ConversationCapabilities::default(),
        );
        assert_eq!(pull.action, ConversationPlanActionType::StoreOnly);
    }

    #[test]
    fn provider_id_reservation_is_primary_first_and_injective() {
        let requests = vec![
            ProviderIdRequest {
                variant_id: VariantId::parse("variant-z").unwrap(),
                source_hint: Some("shared-source".to_string()),
                primary: false,
            },
            ProviderIdRequest {
                variant_id: VariantId::parse("variant-a").unwrap(),
                source_hint: Some("shared-source".to_string()),
                primary: true,
            },
        ];
        let mut sequence = ["generated-1", "generated-2"].into_iter();
        let reserved = reserve_provider_thread_ids(requests, &BTreeSet::new(), || {
            Ok(sequence.next().unwrap().to_string())
        })
        .unwrap();
        assert_eq!(
            reserved[&VariantId::parse("variant-a").unwrap()],
            "shared-source"
        );
        assert_eq!(
            reserved[&VariantId::parse("variant-z").unwrap()],
            "generated-1"
        );
    }
}

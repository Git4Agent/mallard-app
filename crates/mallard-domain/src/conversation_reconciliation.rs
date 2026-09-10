//! Canonical conversation relation-to-review policy.
//!
//! Relation classification, presentation, and executable planning all ask
//! this one domain value. Infrastructure may gather evidence and reserve
//! provider identities, but it does not own a second action table.

use crate::{
    ConversationBlocker, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReviewAction, SyncCurrency, TailClassification,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingResolution {
    Bound,
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationOperationState {
    Ready,
    Pending,
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationCapabilities {
    pub export_settings_changed: bool,
    /// True only after validated parent/child pins and confirmed native absence.
    pub missing_child_recovery: bool,
    pub binding: BindingResolution,
    pub local_safely_capturable: bool,
    /// The cloud lineage is readable and the destination codec can resume it.
    /// Those facts come from the same provider proof and are one capability.
    pub cloud_materializable: bool,
    pub operation_state: ConversationOperationState,
    /// False only for a first-contact adoption whose proof has not yet been
    /// written as a durable accepted checkpoint.
    pub acceptance_durably_pinned: bool,
}

impl Default for ConversationCapabilities {
    fn default() -> Self {
        Self {
            export_settings_changed: false,
            missing_child_recovery: false,
            binding: BindingResolution::Bound,
            local_safely_capturable: true,
            cloud_materializable: true,
            operation_state: ConversationOperationState::Ready,
            acceptance_durably_pinned: true,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationDecision {
    pub action: ConversationReviewAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<ConversationBlocker>,
    pub selectable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassifiedThread {
    pub relation: TailClassification,
    pub capabilities: ConversationCapabilities,
}

impl ClassifiedThread {
    pub fn new(relation: TailClassification, capabilities: ConversationCapabilities) -> Self {
        Self {
            relation,
            capabilities,
        }
    }

    pub fn decision(self, direction: ConversationPlanDirection) -> ConversationDecision {
        default_conversation_decision(direction, self.relation, self.capabilities)
    }

    pub fn planned_action(
        self,
        direction: ConversationPlanDirection,
    ) -> Option<ConversationPlanActionType> {
        self.decision(direction).action.planned_action()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewPurpose {
    Observe,
    PushPlan,
    PullPlan,
}

impl ReviewPurpose {
    pub fn builds_push_plan(self) -> bool {
        self == Self::PushPlan
    }

    pub fn builds_pull_plan(self) -> bool {
        self == Self::PullPlan
    }
}

impl ConversationReviewAction {
    pub fn planned_action(self) -> Option<ConversationPlanActionType> {
        match self {
            Self::Publish => Some(ConversationPlanActionType::Publish),
            Self::CreateCloud => Some(ConversationPlanActionType::CreateCloud),
            Self::AdoptCloudMetadata => Some(ConversationPlanActionType::AdoptCloudMetadata),
            Self::MaterializeCloud => Some(ConversationPlanActionType::MaterializeCloud),
            Self::KeepBoth => Some(ConversationPlanActionType::KeepBoth),
            Self::CreateLocal => Some(ConversationPlanActionType::CreateLocal),
            Self::NoChanges
            | Self::RequirePullMetadata
            | Self::RequirePull
            | Self::NotPushable
            | Self::ReportPushEligible
            | Self::ReportLocalOnly
            | Self::NotFound
            | Self::Blocked => None,
        }
    }
}

impl ConversationPlanActionType {
    pub(crate) fn matches_relation(self, relation: TailClassification) -> bool {
        match self {
            Self::Publish => relation == TailClassification::LocalOutstanding,
            Self::CreateCloud => relation == TailClassification::MissingCloud,
            Self::AdoptCloudMetadata => matches!(relation, TailClassification::Sync(_)),
            Self::MaterializeCloud => relation == TailClassification::CloudOutstanding,
            Self::KeepBoth => relation == TailClassification::Divergent,
            Self::CreateLocal => relation == TailClassification::MissingLocal,
        }
    }
}

pub fn default_conversation_decision(
    direction: ConversationPlanDirection,
    relation: TailClassification,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    use ConversationBlocker as Blocker;
    use ConversationReviewAction as Action;
    use TailClassification as Relation;

    if capabilities.operation_state == ConversationOperationState::Pending {
        return blocked(Blocker::Busy);
    }
    if capabilities.operation_state == ConversationOperationState::Detached
        && relation == Relation::Unknown
    {
        return blocked(Blocker::DetachedRequiresPull);
    }

    let binding_block = || match capabilities.binding {
        BindingResolution::Ambiguous => Some(Blocker::AmbiguousBinding),
        BindingResolution::Missing => Some(Blocker::MissingBinding),
        BindingResolution::Bound => None,
    };

    match direction {
        ConversationPlanDirection::Push => match relation {
            Relation::Sync(SyncCurrency::Current) if !capabilities.export_settings_changed => {
                decision(Action::NoChanges)
            }
            Relation::Sync(SyncCurrency::Advanced) => decision(Action::RequirePullMetadata),
            Relation::LocalOutstanding | Relation::Sync(SyncCurrency::Current) => {
                if let Some(blocker) = binding_block() {
                    blocked(blocker)
                } else if !capabilities.local_safely_capturable {
                    blocked(Blocker::CodecUnsupported)
                } else {
                    decision(Action::Publish)
                }
            }
            Relation::CloudOutstanding | Relation::Divergent => decision(Action::RequirePull),
            Relation::MissingCloud => {
                if capabilities.binding == BindingResolution::Ambiguous {
                    blocked(Blocker::AmbiguousBinding)
                } else if !capabilities.local_safely_capturable {
                    blocked(Blocker::CodecUnsupported)
                } else {
                    decision(Action::CreateCloud)
                }
            }
            Relation::MissingLocal | Relation::MissingBoth => decision(Action::NotPushable),
            Relation::Unknown => blocked(Blocker::UnknownEvidence),
        },
        ConversationPlanDirection::Pull => match relation {
            Relation::Sync(SyncCurrency::Current) if capabilities.acceptance_durably_pinned => {
                decision(Action::NoChanges)
            }
            Relation::Sync(_) => decision(Action::AdoptCloudMetadata),
            Relation::LocalOutstanding => decision(Action::ReportPushEligible),
            Relation::CloudOutstanding => {
                materializing_decision(Action::MaterializeCloud, capabilities)
            }
            Relation::Divergent => {
                if !capabilities.local_safely_capturable {
                    blocked(Blocker::CodecUnsupported)
                } else {
                    materializing_decision(Action::KeepBoth, capabilities)
                }
            }
            Relation::Unknown
                if capabilities.missing_child_recovery && capabilities.local_safely_capturable =>
            {
                materializing_decision(Action::KeepBoth, capabilities)
            }
            Relation::MissingLocal => materializing_decision(Action::CreateLocal, capabilities),
            Relation::MissingCloud => decision(Action::ReportLocalOnly),
            Relation::MissingBoth => decision(Action::NotFound),
            Relation::Unknown => blocked(Blocker::UnknownEvidence),
        },
    }
}

fn materializing_decision(
    action: ConversationReviewAction,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    if !capabilities.cloud_materializable {
        blocked(ConversationBlocker::CodecUnsupported)
    } else if capabilities.binding == BindingResolution::Ambiguous {
        blocked(ConversationBlocker::AmbiguousBinding)
    } else if capabilities.binding == BindingResolution::Missing
        && action != ConversationReviewAction::CreateLocal
    {
        blocked(ConversationBlocker::MissingBinding)
    } else {
        decision(action)
    }
}

fn decision(action: ConversationReviewAction) -> ConversationDecision {
    ConversationDecision {
        selectable: action.planned_action().is_some(),
        action,
        blocker: None,
        blocked_reason: None,
    }
}

/// A blocked decision with the blocker's standard reason text, for
/// review-stage overrides computed outside the relation matrix (e.g. the
/// redaction ruleset gate, which needs the cloud row's stamp).
pub fn blocked_conversation_decision(blocker: ConversationBlocker) -> ConversationDecision {
    blocked(blocker)
}

fn blocked(blocker: ConversationBlocker) -> ConversationDecision {
    ConversationDecision {
        action: ConversationReviewAction::Blocked,
        blocker: Some(blocker),
        selectable: false,
        blocked_reason: Some(blocked_reason(blocker).to_string()),
    }
}

fn blocked_reason(blocker: ConversationBlocker) -> &'static str {
    match blocker {
        ConversationBlocker::CodecUnsupported => {
            "This conversation cannot be transformed safely by the active codec."
        }
        ConversationBlocker::AmbiguousBinding => {
            "More than one local-to-cloud conversation binding is compatible."
        }
        ConversationBlocker::MissingBinding => {
            "No verified local-to-cloud conversation binding exists."
        }
        ConversationBlocker::Busy => {
            "Another sync operation still owns this conversation. Wait for it to finish."
        }
        ConversationBlocker::DetachedRequiresPull => {
            "This conversation's local mapping was replaced. Pull to re-establish it."
        }
        ConversationBlocker::UnknownEvidence => {
            "This conversation's local and stored state could not be proven safely."
        }
        ConversationBlocker::RedactionRulesetMismatch => {
            "This conversation's stored redaction ruleset does not match this app version. \
             Update Mallard everywhere before syncing it."
        }
        ConversationBlocker::PathMapCodecMismatch => {
            "This conversation's stored path encoding does not match this app version. \
             Update Mallard everywhere before syncing it."
        }
        ConversationBlocker::ObservationFailed => {
            "This conversation's local session could not be read for review."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_six_review_outcomes_can_become_plan_actions() {
        use ConversationReviewAction as Action;
        let mutations = [
            Action::Publish,
            Action::CreateCloud,
            Action::AdoptCloudMetadata,
            Action::MaterializeCloud,
            Action::KeepBoth,
            Action::CreateLocal,
        ];
        assert!(mutations
            .into_iter()
            .all(|action| action.planned_action().is_some()));
        assert!(decision(Action::RequirePull)
            .action
            .planned_action()
            .is_none());
        assert!(blocked(ConversationBlocker::Busy)
            .action
            .planned_action()
            .is_none());
    }
}

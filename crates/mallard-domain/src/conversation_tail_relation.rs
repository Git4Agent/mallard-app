//! The sole conversation relation implementation.
//!
//! `compare_conversation_tail` is the only place in the workspace that
//! compares accepted/local/cloud tail values.  Refresh, Push, and Pull map
//! the returned relation to their own actions; no command or provider module
//! may own a second truth table or refine the result.
//!
//! The reading is **pin-relative**: wire and local bytes differ under the
//! path-mapped wire encoding, so the cloud side is compared only to the
//! accepted *wire* pin and the local side only to the accepted *local* pin.
//! All tail comparisons are field-wise including the codec label, so a
//! cross-form comparison can never accidentally succeed.  There is
//! deliberately no containment refinement: a canonical-prefix relationship
//! between two unequal tails still classifies `Divergent`, and the accepted
//! residual is an occasional extra keep-both.

use serde::{Deserialize, Serialize};

use crate::conversation_tail::PortableConversationTail;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncCurrency {
    /// Both sides equal their accepted pins.
    Current,
    /// Local and cloud agree on one same-form value newer than the accepted
    /// pins (reachable only for single-form providers whose local and cloud
    /// evidence share a codec; the dual-form codex labels never compare
    /// equal across the columns).
    Advanced,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TailClassification {
    Sync(SyncCurrency),
    LocalOutstanding,
    CloudOutstanding,
    Divergent,
    MissingCloud,
    MissingLocal,
    MissingBoth,
    Unknown,
}

/// One side's evidence: a complete validated five-field value, a confirmed
/// absence, or a typed invalidity.  Absence must be proven (no thread entry,
/// no binding, no pin); anything unproven or unreadable is `Invalid`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TailSideEvidence {
    Present(PortableConversationTail),
    ConfirmedAbsent,
    Invalid,
}

impl TailSideEvidence {
    fn value(&self) -> Option<&PortableConversationTail> {
        match self {
            Self::Present(tail) => Some(tail),
            _ => None,
        }
    }

    fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid)
    }
}

/// Why raw tail evidence could not be promoted into classifier input. Failed
/// proofs are data, not caller-controlled Booleans: callers keep this blocker
/// beside the resulting `Unknown` presentation instead of manufacturing a
/// partially verified evidence value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailEvidenceBlocker {
    InvalidSide,
    InvalidTail,
    IncompleteAcceptance,
    UnacceptedOverlap,
    ExpectedSideMissing,
    LayoutMismatch,
    AcceptanceCoordinateMismatch,
}

/// Evidence that has passed every prerequisite for semantic comparison.
/// Construction is the proof boundary; `compare_conversation_tail` therefore
/// accepts no independent proof flags and cannot combine values whose
/// provenance was not verified together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedTailEvidence {
    /// The accepted cloud version's wire-form tail (`P_wire`).
    accepted: TailSideEvidence,
    /// The accepted version's local-form twin (`P_local`), pinned atomically
    /// with `accepted` at the same acceptance event.
    accepted_local: TailSideEvidence,
    /// Local observation (local form).
    local: TailSideEvidence,
    /// Cloud manifest value (wire form).
    cloud: TailSideEvidence,
}

impl VerifiedTailEvidence {
    pub fn new(
        accepted: TailSideEvidence,
        accepted_local: TailSideEvidence,
        local: TailSideEvidence,
        cloud: TailSideEvidence,
    ) -> Result<Self, TailEvidenceBlocker> {
        if accepted.is_invalid()
            || accepted_local.is_invalid()
            || local.is_invalid()
            || cloud.is_invalid()
        {
            return Err(TailEvidenceBlocker::InvalidSide);
        }
        for tail in [&accepted, &accepted_local, &local, &cloud]
            .into_iter()
            .filter_map(TailSideEvidence::value)
        {
            tail.validate()
                .map_err(|_| TailEvidenceBlocker::InvalidTail)?;
        }
        if accepted.value().is_some() != accepted_local.value().is_some() {
            return Err(TailEvidenceBlocker::IncompleteAcceptance);
        }
        match (accepted.value(), local.value(), cloud.value()) {
            (None, Some(_), Some(_)) => return Err(TailEvidenceBlocker::UnacceptedOverlap),
            (Some(_), None, _) | (Some(_), _, None) => {
                return Err(TailEvidenceBlocker::ExpectedSideMissing)
            }
            _ => {}
        }
        if let (Some(accepted), Some(accepted_local), Some(local), Some(cloud)) = (
            accepted.value(),
            accepted_local.value(),
            local.value(),
            cloud.value(),
        ) {
            if !accepted.layout().matches(cloud) || !accepted_local.layout().matches(local) {
                return Err(TailEvidenceBlocker::LayoutMismatch);
            }
            if accepted.partition_kind != accepted_local.partition_kind
                || accepted.start_record_count != accepted_local.start_record_count
                || accepted.accepted_end_record_count != accepted_local.accepted_end_record_count
            {
                return Err(TailEvidenceBlocker::AcceptanceCoordinateMismatch);
            }
        }
        Ok(Self {
            accepted,
            accepted_local,
            local,
            cloud,
        })
    }
}

/// Classify one thread from evidence whose provenance and shape have already
/// passed `VerifiedTailEvidence::new`.
pub fn compare_conversation_tail(evidence: &VerifiedTailEvidence) -> TailClassification {
    let accepted = evidence.accepted.value();
    let accepted_local = evidence.accepted_local.value();
    let local = evidence.local.value();
    let cloud = evidence.cloud.value();

    match (accepted, local, cloud) {
        (None, Some(_), None) => return TailClassification::MissingCloud,
        (None, None, Some(_)) => return TailClassification::MissingLocal,
        (None, None, None) => return TailClassification::MissingBoth,
        (Some(_), Some(_), Some(_)) => {}
        _ => unreachable!("verified evidence exhausts ambiguous absence shapes"),
    }
    let (accepted, accepted_local, local, cloud) = match (accepted, accepted_local, local, cloud) {
        (Some(accepted), Some(accepted_local), Some(local), Some(cloud)) => {
            (accepted, accepted_local, local, cloud)
        }
        _ => unreachable!("verified evidence pairs accepted tails"),
    };

    let local_current = local == accepted_local;
    let cloud_current = cloud == accepted;
    if local_current && cloud_current {
        return TailClassification::Sync(SyncCurrency::Current);
    }
    // Same-form agreement on a newer value (single-form providers only:
    // field-wise equality includes the codec, so the dual-form codex labels
    // make this row unreachable across the wire/local columns).
    if local == cloud {
        return TailClassification::Sync(SyncCurrency::Advanced);
    }
    if cloud_current && local.accepted_end_record_count > accepted_local.accepted_end_record_count {
        return TailClassification::LocalOutstanding;
    }
    if local_current && cloud.accepted_end_record_count > accepted.accepted_end_record_count {
        return TailClassification::CloudOutstanding;
    }
    // Everything else — either side rewritten at or below its pinned end,
    // or both sides moved — is a pinned divergence.
    TailClassification::Divergent
}

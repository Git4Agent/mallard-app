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
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Evidence for one classification: the accepted wire pin `P_wire`, its
/// local-form twin `P_local`, local observation `L`, cloud `C`, and the
/// caller-verified gates.  Each gate is `true` when its proof succeeded or
/// is vacuous (for example the witness gates without a pin); any failed or
/// ambiguous proof must be passed as `false`.
#[derive(Clone, Debug)]
pub struct TailComparisonEvidence {
    /// The accepted cloud version's wire-form tail (`P_wire`).
    pub accepted: TailSideEvidence,
    /// The accepted version's local-form twin (`P_local`), pinned atomically
    /// with `accepted` at the same acceptance event.
    pub accepted_local: TailSideEvidence,
    /// Local observation (local form).
    pub local: TailSideEvidence,
    /// Cloud manifest value (wire form).
    pub cloud: TailSideEvidence,
    /// The local value was derived from the bound root under its binding
    /// layout — cursor continuity from `P_local`'s accumulator state plus
    /// the probe window is the standing prefix proof.
    pub local_provenance_verified: bool,
    /// The accepted pin's immutable commit/manifest witness re-verified.
    pub accepted_witness_verified: bool,
    /// The current cloud head is the accepted witness or a verified
    /// descendant with an unmoved start.
    pub cloud_ancestry_verified: bool,
    /// Exact ScopeBase/HEAD coordinates matched the reviewed snapshot.
    pub coordinates_verified: bool,
}

/// Classify one thread from complete five-field values and validated gates.
/// A value match can never override a failed gate.
pub fn compare_conversation_tail(evidence: TailComparisonEvidence) -> TailClassification {
    if !evidence.local_provenance_verified
        || !evidence.accepted_witness_verified
        || !evidence.cloud_ancestry_verified
        || !evidence.coordinates_verified
    {
        return TailClassification::Unknown;
    }
    if evidence.accepted.is_invalid()
        || evidence.accepted_local.is_invalid()
        || evidence.local.is_invalid()
        || evidence.cloud.is_invalid()
    {
        return TailClassification::Unknown;
    }
    for tail in [
        &evidence.accepted,
        &evidence.accepted_local,
        &evidence.local,
        &evidence.cloud,
    ]
    .into_iter()
    .filter_map(TailSideEvidence::value)
    {
        if tail.validate().is_err() {
            return TailClassification::Unknown;
        }
    }

    // The dual pins are recorded atomically; a half-present pair is
    // corrupted acceptance evidence, never a directional absence.
    if evidence.accepted.value().is_some() != evidence.accepted_local.value().is_some() {
        return TailClassification::Unknown;
    }
    let accepted = evidence.accepted.value();
    let accepted_local = evidence.accepted_local.value();
    let local = evidence.local.value();
    let cloud = evidence.cloud.value();

    // Directional absence is decided first.  Pull does not bootstrap by
    // comparing graphs: both sides present without a pin is Unknown (first
    // contact re-establishes acceptance through the encode-compare adoption
    // path before classification), and an expected side missing after a pin
    // exists is Unknown.
    match (accepted, local, cloud) {
        (None, Some(_), None) => return TailClassification::MissingCloud,
        (None, None, Some(_)) => return TailClassification::MissingLocal,
        (None, None, None) => return TailClassification::MissingBoth,
        (None, Some(_), Some(_)) => return TailClassification::Unknown,
        (Some(_), None, _) | (Some(_), _, None) => return TailClassification::Unknown,
        (Some(_), Some(_), Some(_)) => {}
    }
    let (accepted, accepted_local, local, cloud) = match (accepted, accepted_local, local, cloud) {
        (Some(accepted), Some(accepted_local), Some(local), Some(cloud)) => {
            (accepted, accepted_local, local, cloud)
        }
        _ => unreachable!("absence and pairing handled above"),
    };

    // Column layout guards: the wire column shares one codec, partition,
    // and start; the local column likewise; the two pins agree on the
    // form-independent partition and coordinates.  A moved start makes
    // ancestry Unknown.
    if !accepted.layout().matches(cloud) || !accepted_local.layout().matches(local) {
        return TailClassification::Unknown;
    }
    if accepted.partition_kind != accepted_local.partition_kind
        || accepted.start_record_count != accepted_local.start_record_count
        || accepted.accepted_end_record_count != accepted_local.accepted_end_record_count
    {
        return TailClassification::Unknown;
    }

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
    if cloud_current
        && local.accepted_end_record_count > accepted_local.accepted_end_record_count
    {
        return TailClassification::LocalOutstanding;
    }
    if local_current && cloud.accepted_end_record_count > accepted.accepted_end_record_count {
        return TailClassification::CloudOutstanding;
    }
    // Everything else — either side rewritten at or below its pinned end,
    // or both sides moved — is a pinned divergence.
    TailClassification::Divergent
}

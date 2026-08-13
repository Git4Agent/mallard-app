//! The sole conversation relation implementation.
//!
//! `compare_conversation_tail` is the only place in the workspace that
//! compares accepted/local/cloud tail values.  Refresh, Push, and Pull map
//! the returned relation to their own actions; no command or provider module
//! may own a second truth table or refine the result.  There is deliberately
//! no containment refinement: a canonical-prefix relationship between two
//! unequal tails still classifies `Divergent`, and the accepted residual is
//! an occasional extra keep-both.

use serde::{Deserialize, Serialize};

use crate::conversation_tail::PortableConversationTail;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncCurrency {
    /// The accepted pin equals the shared current value.
    Current,
    /// Local and cloud agree on a value newer than the accepted pin.
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

/// Evidence for one classification: the accepted pin `P`, local `L`, cloud
/// `C`, and the caller-verified gates.  Each gate is `true` when its proof
/// succeeded or is vacuous (for example the witness gates without a pin);
/// any failed or ambiguous proof must be passed as `false`.
#[derive(Clone, Debug)]
pub struct TailComparisonEvidence {
    pub accepted: TailSideEvidence,
    pub local: TailSideEvidence,
    pub cloud: TailSideEvidence,
    /// The local value was derived from the bound root under its binding
    /// layout.
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
    if evidence.accepted.is_invalid() || evidence.local.is_invalid() || evidence.cloud.is_invalid()
    {
        return TailClassification::Unknown;
    }
    for tail in [&evidence.accepted, &evidence.local, &evidence.cloud]
        .into_iter()
        .filter_map(TailSideEvidence::value)
    {
        if tail.validate().is_err() {
            return TailClassification::Unknown;
        }
    }

    let accepted = evidence.accepted.value();
    let local = evidence.local.value();
    let cloud = evidence.cloud.value();

    // Directional absence is decided first.  Pull does not bootstrap by
    // comparing graphs: both sides present without a pin is Unknown, and an
    // expected side missing after a pin exists is Unknown.
    match (accepted, local, cloud) {
        (None, Some(_), None) => return TailClassification::MissingCloud,
        (None, None, Some(_)) => return TailClassification::MissingLocal,
        (None, None, None) => return TailClassification::MissingBoth,
        (None, Some(_), Some(_)) => return TailClassification::Unknown,
        (Some(_), None, _) | (Some(_), _, None) => return TailClassification::Unknown,
        (Some(_), Some(_), Some(_)) => {}
    }
    let (accepted, local, cloud) = match (accepted, local, cloud) {
        (Some(accepted), Some(local), Some(cloud)) => (accepted, local, cloud),
        _ => unreachable!("absence handled above"),
    };

    // Tail equality is meaningful only under a compatible codec, partition,
    // and start; a moved start makes ancestry Unknown.
    let layout = accepted.layout();
    if !layout.matches(local) || !layout.matches(cloud) {
        return TailClassification::Unknown;
    }

    if local == cloud {
        return TailClassification::Sync(if accepted == cloud {
            SyncCurrency::Current
        } else {
            SyncCurrency::Advanced
        });
    }
    if cloud == accepted && local != accepted {
        return TailClassification::LocalOutstanding;
    }
    if local == accepted && cloud != accepted {
        return TailClassification::CloudOutstanding;
    }
    TailClassification::Divergent
}

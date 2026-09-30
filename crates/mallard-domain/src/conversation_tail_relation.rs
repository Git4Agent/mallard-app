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
//!
//! A pair without accepted pins has nothing to be read relative to. The
//! caller proves an equal or an earlier stored copy from content before it
//! builds the evidence (first-contact adoption) and passes that proof as the
//! accepted pins. A pair that arrives here without them is one that proof
//! could not decide: the stored copy is longer or different. It classifies
//! `Divergent`, so nothing is replaced and Pull keeps both copies.

use serde::{Deserialize, Serialize};

use crate::conversation_tail::PortableConversationTail;
use crate::{SubagentContentState, SubagentTailPins};

/// Adapt compact child evidence to the same proof boundary and comparator.
pub fn compare_subagent_tail(
    pins: &SubagentTailPins,
) -> Result<TailClassification, TailEvidenceBlocker> {
    let side = |state: Option<&SubagentContentState>, wire| match state {
        Some(state) => TailSideEvidence::Present(if wire {
            state.normalized_tail()
        } else {
            state.local_tail()
        }),
        None => TailSideEvidence::ConfirmedAbsent,
    };
    let evidence = VerifiedTailEvidence::new(
        side(pins.accepted.as_ref().map(|pair| &pair.normalized), true),
        side(pins.accepted.as_ref().map(|pair| &pair.local), false),
        side(pins.local.as_ref(), false),
        side(pins.cloud.as_ref(), true),
    )?;
    Ok(compare_conversation_tail(&evidence))
}

/// Fold already-classified rollouts without comparing their content again.
///
/// What this computer changed stands against what the storage changed, and
/// the session is `Divergent`. A child that only this computer has is such a
/// change, unless `local_only_children_kept`: the Pull that installs the
/// stored version then updates the session where it is and leaves that child
/// in its place. The session is behind the storage, and ahead of it after
/// the Pull. A main transcript or a child that was continued here is
/// replaced by the Pull, so it stands against the storage all the same.
pub fn fold_conversation_relations(
    main: TailClassification,
    children: impl IntoIterator<Item = TailClassification>,
    local_only_children_kept: bool,
) -> TailClassification {
    use TailClassification::*;
    let mut local = false;
    let mut local_only_child = false;
    let mut cloud = false;
    let mut divergent = false;
    let mut advanced = false;
    let children = children.into_iter().map(|relation| (relation, true));
    for (relation, is_child) in std::iter::once((main, false)).chain(children) {
        match relation {
            Unknown => return Unknown,
            Divergent => divergent = true,
            MissingCloud if is_child => local_only_child = true,
            LocalOutstanding | MissingCloud => local = true,
            CloudOutstanding | MissingLocal => cloud = true,
            Sync(SyncCurrency::Advanced) => advanced = true,
            Sync(SyncCurrency::Current) | MissingBoth => {}
        }
    }
    // The Pull keeps a child in its place only beside a main transcript
    // that is on both sides.
    let kept = local_only_children_kept && matches!(main, Sync(_) | CloudOutstanding);
    local |= local_only_child && !(cloud && kept);
    if divergent || (local && cloud) {
        Divergent
    } else if matches!(main, MissingCloud | MissingLocal | MissingBoth) {
        main
    } else if local {
        LocalOutstanding
    } else if cloud {
        CloudOutstanding
    } else {
        Sync(if advanced {
            SyncCurrency::Advanced
        } else {
            SyncCurrency::Current
        })
    }
}

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
        if let (Some(_), None, _) | (Some(_), _, None) =
            (accepted.value(), local.value(), cloud.value())
        {
            return Err(TailEvidenceBlocker::ExpectedSideMissing);
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
        // Both copies exist and no accepted pins relate them.
        (None, Some(_), Some(_)) => return TailClassification::Divergent,
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

#[cfg(test)]
mod owned_child_tests {
    use super::*;
    use crate::AcceptedSubagentState;

    fn state(count: u64, byte: &str) -> SubagentContentState {
        SubagentContentState {
            message_count: count,
            sha256: byte.repeat(64),
        }
    }

    #[test]
    fn children_use_the_existing_evidence_and_content_rules() {
        let local = state(2, "a");
        let wire = state(2, "b");
        let accepted = AcceptedSubagentState {
            local: local.clone(),
            normalized: wire.clone(),
        };
        let pins = SubagentTailPins {
            accepted: Some(accepted),
            local: Some(local.clone()),
            cloud: Some(wire.clone()),
        };
        assert_eq!(
            compare_subagent_tail(&pins),
            Ok(TailClassification::Sync(SyncCurrency::Current))
        );
        for (local, cloud, expected) in [
            (
                state(3, "c"),
                wire.clone(),
                TailClassification::LocalOutstanding,
            ),
            (
                local.clone(),
                state(3, "d"),
                TailClassification::CloudOutstanding,
            ),
            (state(2, "c"), wire.clone(), TailClassification::Divergent),
            (state(1, "c"), wire.clone(), TailClassification::Divergent),
            (state(3, "c"), state(3, "d"), TailClassification::Divergent),
        ] {
            assert_eq!(
                compare_subagent_tail(&SubagentTailPins {
                    local: Some(local),
                    cloud: Some(cloud),
                    ..pins.clone()
                }),
                Ok(expected)
            );
        }
        assert_eq!(
            compare_subagent_tail(&SubagentTailPins {
                local: None,
                ..pins.clone()
            }),
            Err(TailEvidenceBlocker::ExpectedSideMissing)
        );
        // A child pair without accepted pins: nothing relates the two
        // copies, so the conversation keeps both.
        assert_eq!(
            compare_subagent_tail(&SubagentTailPins {
                accepted: None,
                ..pins.clone()
            }),
            Ok(TailClassification::Divergent)
        );
        assert_eq!(
            compare_subagent_tail(&SubagentTailPins {
                accepted: None,
                cloud: None,
                ..pins.clone()
            }),
            Ok(TailClassification::MissingCloud)
        );
        assert_eq!(
            compare_subagent_tail(&SubagentTailPins {
                accepted: None,
                local: None,
                ..pins
            }),
            Ok(TailClassification::MissingLocal)
        );
    }

    #[test]
    fn fold_counts_new_children_and_mixed_sides_without_hiding_invalid_evidence() {
        use TailClassification::*;
        let synced = Sync(SyncCurrency::Current);
        // No session here has a child of this computer alone beside a
        // change in the storage, so what a Pull keeps decides nothing.
        for kept in [false, true] {
            let fold = |main, children: &[TailClassification]| {
                fold_conversation_relations(main, children.iter().copied(), kept)
            };
            assert_eq!(fold(synced, &[MissingCloud]), LocalOutstanding);
            assert_eq!(fold(synced, &[MissingLocal]), CloudOutstanding);
            assert_eq!(
                fold(synced, &[LocalOutstanding, CloudOutstanding]),
                Divergent
            );
            assert_eq!(fold(LocalOutstanding, &[CloudOutstanding]), Divergent);
            assert_eq!(fold(synced, &[Divergent]), Divergent);
            assert_eq!(fold(Divergent, &[Unknown]), Unknown);
            assert_eq!(fold(MissingCloud, &[MissingCloud]), MissingCloud);
            assert_eq!(fold(MissingLocal, &[MissingLocal]), MissingLocal);
            assert_eq!(fold(synced, &[synced]), synced);
        }
    }

    #[test]
    fn a_child_of_this_computer_alone_stands_against_the_storage_unless_a_pull_keeps_it() {
        use TailClassification::*;
        let synced = Sync(SyncCurrency::Current);
        let advanced = Sync(SyncCurrency::Advanced);
        let fold = |main, children: &[TailClassification], kept| {
            fold_conversation_relations(main, children.iter().copied(), kept)
        };
        // The storage has a child more, a longer child, or a longer main
        // transcript. This computer has a child that is not stored.
        for (main, children) in [
            (synced, vec![MissingCloud, MissingLocal]),
            (synced, vec![MissingLocal, MissingCloud, MissingCloud]),
            (synced, vec![MissingCloud, CloudOutstanding]),
            (advanced, vec![MissingCloud, MissingLocal]),
            (CloudOutstanding, vec![MissingCloud]),
            (CloudOutstanding, vec![synced, MissingCloud, MissingLocal]),
        ] {
            assert_eq!(
                fold(main, &children, false),
                Divergent,
                "{main:?} {children:?}"
            );
            assert_eq!(
                fold(main, &children, true),
                CloudOutstanding,
                "{main:?} {children:?}"
            );
        }
        for kept in [false, true] {
            // After that Pull the child is what the session is ahead by.
            assert_eq!(
                fold(synced, &[synced, MissingCloud], kept),
                LocalOutstanding
            );
            // The Pull replaces a main transcript and a child that were
            // continued here, and it keeps no child beside a main
            // transcript that is on one side only.
            for (main, children) in [
                (LocalOutstanding, vec![MissingLocal]),
                (LocalOutstanding, vec![MissingCloud, MissingLocal]),
                (synced, vec![LocalOutstanding, MissingLocal]),
                (
                    synced,
                    vec![LocalOutstanding, MissingCloud, CloudOutstanding],
                ),
                (CloudOutstanding, vec![LocalOutstanding, MissingCloud]),
                (synced, vec![Divergent, MissingCloud, MissingLocal]),
                (Divergent, vec![MissingCloud, MissingLocal]),
                (MissingCloud, vec![MissingCloud, MissingLocal]),
                (MissingLocal, vec![MissingCloud, MissingLocal]),
                (MissingBoth, vec![MissingCloud, MissingLocal]),
            ] {
                assert_eq!(
                    fold(main, &children, kept),
                    Divergent,
                    "{main:?} {children:?} {kept}"
                );
            }
            assert_eq!(
                fold(CloudOutstanding, &[MissingCloud, Unknown], kept),
                Unknown
            );
        }
    }
}

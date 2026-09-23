//! Publication records: names, tokens, head selection, and ancestry.
//!
//! A stream is one bundle or one Library repository. It publishes by writing
//! immutable records, and a record's name carries its graph facts, so the
//! current publication of a stream is selected from the names of one listing
//! without reading a record. This module is pure: it builds and parses names
//! and selects over them. Listing, reading, and writing live with the stores.

use std::collections::{HashMap, HashSet};

use crate::validate_sha256;

/// Ten zero-padded digits hold a record's depth in its name.
pub const MAX_RECORD_REVISION: u64 = 9_999_999_999;
/// Thirteen zero-padded digits hold a record's creation time in its name.
pub const MAX_RECORD_CREATED_AT_MS: u64 = 9_999_999_999_999;

const COMMIT_PREFIX: &str = "commit-";
const COMMIT_HEX: usize = 24;
const ROOT: &str = "root";
const SUFFIX: &str = "json";

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_commit_id(value: &str) -> bool {
    value
        .strip_prefix(COMMIT_PREFIX)
        .is_some_and(|hex| lower_hex(hex, COMMIT_HEX))
}

/// A publication's full identity in its stream: the commit id plus the
/// SHA-256 of the record's exact bytes. The commit id alone is unique in
/// normal operation; the digest makes two records that share an id two
/// separate publications instead of a conflict.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationToken {
    commit_id: String,
    record_sha256: String,
}

impl PublicationToken {
    pub fn new(
        commit_id: impl Into<String>,
        record_sha256: impl Into<String>,
    ) -> Result<Self, String> {
        let commit_id = commit_id.into();
        let record_sha256 = record_sha256.into();
        if !valid_commit_id(&commit_id) {
            return Err(format!(
                "publication commit id must be 'commit-' plus {} lowercase hexadecimal characters: '{}'",
                COMMIT_HEX,
                commit_id.escape_debug()
            ));
        }
        validate_sha256("publication record", &record_sha256)?;
        Ok(Self {
            commit_id,
            record_sha256,
        })
    }

    pub fn commit_id(&self) -> &str {
        &self.commit_id
    }

    pub fn record_sha256(&self) -> &str {
        &self.record_sha256
    }

    fn parse(value: &str) -> Option<Self> {
        let hex = value.strip_prefix(COMMIT_PREFIX)?;
        let (commit_hex, record_sha256) = hex.split_once('-')?;
        if !lower_hex(commit_hex, COMMIT_HEX) || !lower_hex(record_sha256, 64) {
            return None;
        }
        Some(Self {
            commit_id: format!("{COMMIT_PREFIX}{commit_hex}"),
            record_sha256: record_sha256.to_string(),
        })
    }
}

impl std::fmt::Display for PublicationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}-{}", self.commit_id, self.record_sha256)
    }
}

/// The graph facts of one record, as its object name states them:
///
/// ```text
/// <revision:010>.<commit_id>-<record_sha256>.<parent token | root>.<created_at_ms:013>.json
/// ```
///
/// Names are lowercase `[a-z0-9.-]`, at most 223 bytes, and sort by depth.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordName {
    revision: u64,
    token: PublicationToken,
    parent: Option<PublicationToken>,
    created_at_ms: u64,
}

impl RecordName {
    /// A first publication has no parent and revision 1; every other record
    /// names its parent and sits one deeper than it.
    pub fn new(
        revision: u64,
        token: PublicationToken,
        parent: Option<PublicationToken>,
        created_at_ms: u64,
    ) -> Result<Self, String> {
        if revision == 0 || revision > MAX_RECORD_REVISION {
            return Err(format!("publication revision {revision} is out of range"));
        }
        if created_at_ms > MAX_RECORD_CREATED_AT_MS {
            return Err(format!(
                "publication timestamp {created_at_ms} is out of range"
            ));
        }
        if (revision == 1) != parent.is_none() {
            return Err(
                "a first publication has revision 1 and no parent; every other names its parent"
                    .to_string(),
            );
        }
        if parent.as_ref() == Some(&token) {
            return Err("a publication cannot be its own parent".to_string());
        }
        Ok(Self {
            revision,
            token,
            parent,
            created_at_ms,
        })
    }

    /// Parse one listed object name. `None` means the name is not a record:
    /// callers ignore it and report it, and it is never selected.
    pub fn parse(file_name: &str) -> Option<Self> {
        let mut parts = file_name.split('.');
        let revision = parts.next()?;
        let token = parts.next()?;
        let parent = parts.next()?;
        let created_at_ms = parts.next()?;
        if parts.next()? != SUFFIX || parts.next().is_some() {
            return None;
        }
        let digits = |value: &str, length: usize| {
            (value.len() == length && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.parse::<u64>().ok())
                .flatten()
        };
        let parent = if parent == ROOT {
            None
        } else {
            Some(PublicationToken::parse(parent)?)
        };
        Self::new(
            digits(revision, 10)?,
            PublicationToken::parse(token)?,
            parent,
            digits(created_at_ms, 13)?,
        )
        .ok()
    }

    pub fn file_name(&self) -> String {
        let parent = self
            .parent
            .as_ref()
            .map_or_else(|| ROOT.to_string(), PublicationToken::to_string);
        format!(
            "{:010}.{}.{}.{:013}.{}",
            self.revision, self.token, parent, self.created_at_ms, SUFFIX
        )
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn token(&self) -> &PublicationToken {
        &self.token
    }

    pub fn commit_id(&self) -> &str {
        self.token.commit_id()
    }

    pub fn parent(&self) -> Option<&PublicationToken> {
        self.parent.as_ref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Siblings are ranked by creation time, then commit id, then record
    /// digest; the smallest rank is selected.
    fn rank(&self) -> (u64, &str, &str) {
        (
            self.created_at_ms,
            self.token.commit_id(),
            self.token.record_sha256(),
        )
    }
}

/// The selected chain of one stream, from its first publication to the
/// selected one. Every listed record that is not on it is kept and not
/// selected; nothing marks, moves, or deletes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mainline(Vec<RecordName>);

impl Mainline {
    /// Select from the names of one listing. Starting at the virtual root,
    /// the earliest-ranked child is taken until a publication has none.
    /// Listing order and duplicate entries do not matter, a record whose
    /// parent is not listed is never reached, and neither a later timestamp
    /// on a descendant nor a longer branch rescues a losing sibling.
    pub fn select(names: impl IntoIterator<Item = RecordName>) -> Self {
        let unique = names.into_iter().collect::<HashSet<_>>();
        let mut children = HashMap::<Option<&PublicationToken>, Vec<&RecordName>>::new();
        for name in &unique {
            children.entry(name.parent()).or_default().push(name);
        }
        let mut mainline = Vec::<RecordName>::new();
        loop {
            let depth = mainline.len() as u64 + 1;
            let parent = mainline.last().map(|name| name.token().clone());
            let Some(next) = children.get(&parent.as_ref()).and_then(|siblings| {
                siblings
                    .iter()
                    .filter(|name| name.revision() == depth)
                    .min_by(|left, right| left.rank().cmp(&right.rank()))
            }) else {
                break;
            };
            mainline.push((*next).clone());
        }
        Self(mainline)
    }

    /// The current publication of the stream, or none while it lists empty.
    pub fn selected(&self) -> Option<&RecordName> {
        self.0.last()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn names(&self) -> &[RecordName] {
        &self.0
    }

    /// Whether the publication `commit_id` at depth `revision` is the
    /// selected publication or one of its ancestors. Parent edges are
    /// hash-linked tokens, so the answer needs no record read.
    pub fn contains(&self, revision: u64, commit_id: &str) -> bool {
        revision
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.0.get(index))
            .is_some_and(|name| name.commit_id() == commit_id)
    }

    /// Whether exactly this publication is on the selected chain.
    pub fn contains_token(&self, token: &PublicationToken) -> bool {
        self.0.iter().any(|name| name.token() == token)
    }

    /// Whether the publication `commit_id` is on the selected chain, at any
    /// depth. A session acceptance records the commit it was accepted in and
    /// no revision, so it is looked up by identity alone.
    pub fn contains_commit(&self, commit_id: &str) -> bool {
        self.0.iter().any(|name| name.commit_id() == commit_id)
    }

    /// The listed record of the publication `commit_id` that this chain
    /// left, when it is one of `unselected`: the records of the same listing
    /// that this chain does not hold. It is outranked when walking its
    /// parent names over listed records reaches this chain, where another
    /// record on the same parent ranked first, or the virtual root while
    /// another first publication is selected. Ranks never change and records
    /// are deleted only by a clear, so the verdict is final; a listing that
    /// lags can hide records, and cannot produce it. A publication that is
    /// selected, that is not listed, or that no listed record connects to
    /// this chain is not outranked: a stream that was cleared and pushed
    /// again proves nothing about it.
    pub fn outranked<'u>(
        &self,
        unselected: &'u [RecordName],
        commit_id: &str,
    ) -> Option<&'u RecordName> {
        if self.contains_commit(commit_id) {
            return None;
        }
        let record = unselected
            .iter()
            .filter(|name| name.commit_id() == commit_id)
            .min_by(|left, right| left.rank().cmp(&right.rank()))?;
        let mut current = record;
        loop {
            let Some(parent) = current.parent() else {
                // A first publication that is not selected: another one is.
                return (!self.is_empty()).then_some(record);
            };
            if let Some(shared) = self.0.iter().find(|name| name.token() == parent) {
                // The selected chain goes on from `shared` through a sibling.
                let sibling_selected = self.contains_depth(shared.revision() + 1);
                return (current.revision() == shared.revision() + 1 && sibling_selected)
                    .then_some(record);
            }
            let next = unselected.iter().find(|name| name.token() == parent)?;
            if current.revision() != next.revision() + 1 {
                return None;
            }
            current = next;
        }
    }

    fn contains_depth(&self, revision: u64) -> bool {
        usize::try_from(revision).is_ok_and(|depth| depth >= 1 && depth <= self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: u8, record: u8) -> PublicationToken {
        PublicationToken::new(
            format!("commit-{}", format!("{id:02x}").repeat(12)),
            format!("{record:02x}").repeat(32),
        )
        .unwrap()
    }

    fn first(id: u8, created_at_ms: u64) -> RecordName {
        RecordName::new(1, token(id, id), None, created_at_ms).unwrap()
    }

    fn child(parent: &RecordName, id: u8, created_at_ms: u64) -> RecordName {
        RecordName::new(
            parent.revision() + 1,
            token(id, id),
            Some(parent.token().clone()),
            created_at_ms,
        )
        .unwrap()
    }

    fn selected_ids(names: &[RecordName]) -> Vec<String> {
        Mainline::select(names.iter().cloned())
            .names()
            .iter()
            .map(|name| name.commit_id().to_string())
            .collect()
    }

    #[test]
    fn names_round_trip_sort_by_depth_and_stay_within_the_component_limit() {
        let root = first(1, 1_789_635_601_000);
        let next = child(&root, 2, 1_789_635_602_000);
        assert_eq!(
            root.file_name(),
            format!(
                "0000000001.commit-{}-{}.root.1789635601000.json",
                "01".repeat(12),
                "01".repeat(32)
            )
        );
        for name in [&root, &next] {
            let file_name = name.file_name();
            assert_eq!(RecordName::parse(&file_name).as_ref(), Some(name));
            assert!(file_name
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-')));
        }
        assert_eq!(next.file_name().len(), 223);
        assert!(root.file_name() < next.file_name());
        let deep = RecordName::new(
            MAX_RECORD_REVISION,
            token(3, 3),
            Some(token(2, 2)),
            MAX_RECORD_CREATED_AT_MS,
        )
        .unwrap();
        assert_eq!(deep.file_name().len(), 223);
        assert!(next.file_name() < deep.file_name());
    }

    #[test]
    fn names_outside_the_grammar_are_not_records() {
        let valid = child(&first(1, 5), 2, 6).file_name();
        assert!(RecordName::parse(&valid).is_some());
        for foreign in [
            ".DS_Store".to_string(),
            "_head.json".to_string(),
            valid.replace(".json", " (conflicted copy).json"),
            valid.replace(".json", ".tmp"),
            valid.to_uppercase(),
            // Unpadded depth, a parent on a first publication, and no parent
            // on a later one are not names a conforming writer builds.
            valid.trim_start_matches('0').to_string(),
            valid.replacen("0000000002", "0000000001", 1),
            first(1, 5)
                .file_name()
                .replacen("0000000001", "0000000002", 1),
            valid.replacen("0000000002", "0000000000", 1),
            format!("{valid}.json"),
        ] {
            assert_eq!(RecordName::parse(&foreign), None, "{foreign}");
        }
        assert!(RecordName::new(2, token(1, 1), Some(token(1, 1)), 5).is_err());
        assert!(PublicationToken::new("commit-short", "a".repeat(64)).is_err());
        assert!(
            PublicationToken::new(format!("commit-{}", "a".repeat(24)), "A".repeat(64)).is_err()
        );
    }

    #[test]
    fn every_listing_order_and_duplicate_entries_select_the_same_mainline() {
        let root = first(1, 10);
        let middle = child(&root, 2, 20);
        let tip = child(&middle, 3, 30);
        let loser = child(&root, 4, 25);
        let names = [root, middle, tip, loser];
        let expected = selected_ids(&names);
        assert_eq!(expected.len(), 3);
        let mut order = [0_usize, 1, 2, 3];
        // Heap's algorithm over the four names.
        fn permute(
            size: usize,
            order: &mut [usize; 4],
            names: &[RecordName; 4],
            expected: &[String],
        ) {
            if size == 1 {
                let mut listed = order.iter().map(|&i| names[i].clone()).collect::<Vec<_>>();
                listed.push(names[order[0]].clone());
                assert_eq!(selected_ids(&listed), expected);
                return;
            }
            for index in 0..size {
                permute(size - 1, order, names, expected);
                let swap = if size.is_multiple_of(2) { index } else { 0 };
                order.swap(swap, size - 1);
            }
        }
        permute(4, &mut order, &names, &expected);
    }

    #[test]
    fn the_earliest_sibling_wins_and_ties_fall_to_the_commit_id_then_the_digest() {
        let root = first(1, 10);
        let early = child(&root, 9, 20);
        let late = child(&root, 2, 21);
        assert_eq!(
            Mainline::select([root.clone(), late.clone(), early.clone()]).selected(),
            Some(&early)
        );

        let low_id = child(&root, 3, 50);
        let high_id = child(&root, 7, 50);
        assert_eq!(
            Mainline::select([root.clone(), high_id, low_id.clone()]).selected(),
            Some(&low_id)
        );

        // The same commit id with different bytes is two publications with
        // two names, ranked normally; nothing blocks the stream.
        let low_digest = RecordName::new(2, token(5, 1), Some(root.token().clone()), 50).unwrap();
        let high_digest = RecordName::new(2, token(5, 8), Some(root.token().clone()), 50).unwrap();
        let mainline = Mainline::select([root.clone(), high_digest.clone(), low_digest.clone()]);
        assert_eq!(mainline.selected(), Some(&low_digest));
        assert!(mainline.contains(2, low_digest.commit_id()));
        assert!(mainline.contains_token(low_digest.token()));
        assert!(!mainline.contains_token(high_digest.token()));

        // Two first publications are ordinary siblings of the virtual root.
        let other_root = first(6, 9);
        assert_eq!(
            Mainline::select([root, other_root.clone()]).selected(),
            Some(&other_root)
        );
    }

    #[test]
    fn a_later_descendant_or_a_longer_branch_does_not_rescue_a_losing_fork() {
        let root = first(1, 10);
        let winner = child(&root, 2, 20);
        let loser = child(&root, 3, 21);
        let loser_child = child(&loser, 4, 5);
        let loser_grandchild = child(&loser_child, 5, 6);
        let mainline = Mainline::select([
            root.clone(),
            loser.clone(),
            loser_grandchild.clone(),
            winner.clone(),
            loser_child,
        ]);
        assert_eq!(mainline.names(), [root, winner]);
        // A sibling branch with a greater revision is never an ancestor.
        assert!(!mainline.contains(2, loser.commit_id()));
        assert!(!mainline.contains(4, loser_grandchild.commit_id()));

        // A child with a smaller timestamp than its parent stays its child.
        let root = first(1, 100);
        let backdated = child(&root, 2, 1);
        assert_eq!(
            Mainline::select([backdated.clone(), root]).selected(),
            Some(&backdated)
        );
    }

    #[test]
    fn a_record_whose_parent_is_not_listed_connects_only_when_the_parent_appears() {
        let root = first(1, 10);
        let middle = child(&root, 2, 20);
        let tip = child(&middle, 3, 30);
        let without_parent = Mainline::select([root.clone(), tip.clone()]);
        assert_eq!(without_parent.selected(), Some(&root));
        assert!(!without_parent.contains(3, tip.commit_id()));

        let connected = Mainline::select([tip.clone(), root.clone(), middle.clone()]);
        assert_eq!(connected.selected(), Some(&tip));
        assert!(connected.contains(1, root.commit_id()));
        assert!(connected.contains(2, middle.commit_id()));
        assert!(connected.contains(3, tip.commit_id()));
        assert!(!connected.contains(0, root.commit_id()));
        assert!(!connected.contains(4, tip.commit_id()));
        assert!(!connected.contains(2, tip.commit_id()));

        // An empty stream, and one holding only disconnected records,
        // selects nothing.
        assert!(Mainline::select([]).is_empty());
        assert!(Mainline::select([middle, tip]).selected().is_none());
    }

    #[test]
    fn an_outranked_publication_is_one_the_chain_left_through_a_sibling() {
        let root = first(1, 10);
        let middle = child(&root, 2, 20);
        let winner = child(&middle, 3, 30);
        let tip = child(&winner, 4, 40);
        let loser = child(&middle, 5, 31);
        let built_on_loser = child(&loser, 6, 5);
        let names = [&root, &middle, &winner, &tip, &loser, &built_on_loser];
        let mainline = Mainline::select(names.into_iter().cloned());
        let unselected = [loser.clone(), built_on_loser.clone()];
        // The sibling that ranked second, and everything built on it, left
        // the chain at the parent both siblings name. The verdict names the
        // record it was asked about.
        assert_eq!(
            mainline.outranked(&unselected, loser.commit_id()),
            Some(&loser)
        );
        assert_eq!(
            mainline.outranked(&unselected, built_on_loser.commit_id()),
            Some(&built_on_loser)
        );
        // Selected publications and their ancestors are not outranked, and
        // neither is a publication the listing does not hold.
        for selected in [&root, &middle, &winner, &tip] {
            assert_eq!(
                mainline.outranked(&unselected, selected.commit_id()),
                None
            );
        }
        assert_eq!(
            mainline.outranked(&unselected, first(9, 1).commit_id()),
            None
        );

        // A first publication that ranked second left the chain at the
        // virtual root, together with what was built on it.
        let other_root = first(7, 11);
        let on_other_root = child(&other_root, 8, 12);
        let unselected = [other_root.clone(), on_other_root.clone()];
        for name in [&other_root, &on_other_root] {
            assert_eq!(
                mainline.outranked(&unselected, name.commit_id()),
                Some(name)
            );
        }
    }

    #[test]
    fn a_record_the_listing_cannot_connect_to_the_chain_is_not_outranked() {
        // A stream that was cleared and pushed again: the first records of
        // the old history are gone, and what an interrupted clear left behind
        // names parents the listing no longer holds.
        let old_root = first(1, 10);
        let old_middle = child(&old_root, 2, 20);
        let old_tip = child(&old_middle, 3, 30);
        let new_root = first(4, 100);
        let mainline = Mainline::select([new_root, old_tip.clone()]);
        assert_eq!(mainline.names().len(), 1);
        assert_eq!(
            mainline.outranked(std::slice::from_ref(&old_tip), old_tip.commit_id()),
            None
        );
        // A record at the wrong depth for its parent was never a candidate.
        let root = first(1, 10);
        let winner = child(&root, 2, 20);
        let skipped = RecordName::new(3, token(3, 3), Some(root.token().clone()), 5).unwrap();
        let mainline = Mainline::select([root, winner, skipped.clone()]);
        assert_eq!(
            mainline.outranked(std::slice::from_ref(&skipped), skipped.commit_id()),
            None
        );
        // A selected commit id is never outranked, even when another record
        // carries the same id with different bytes.
        let root = first(1, 10);
        let low_digest = RecordName::new(2, token(5, 1), Some(root.token().clone()), 50).unwrap();
        let high_digest = RecordName::new(2, token(5, 8), Some(root.token().clone()), 50).unwrap();
        let mainline = Mainline::select([root, low_digest.clone(), high_digest.clone()]);
        assert_eq!(mainline.selected(), Some(&low_digest));
        assert_eq!(
            mainline.outranked(std::slice::from_ref(&high_digest), high_digest.commit_id()),
            None
        );
        // Nothing is selected: nothing can have outranked anything.
        assert_eq!(
            Mainline::default().outranked(std::slice::from_ref(&old_tip), old_tip.commit_id()),
            None
        );
    }

    #[test]
    fn a_record_at_the_wrong_depth_for_its_parent_is_never_selected() {
        let root = first(1, 10);
        let skipped = RecordName::new(3, token(2, 2), Some(root.token().clone()), 5).unwrap();
        let honest = child(&root, 3, 50);
        assert_eq!(
            Mainline::select([root, skipped, honest.clone()]).selected(),
            Some(&honest)
        );
    }
}

//! Git presentation captured with a publication. It never restores Git state.

use crate::{ConversationThread, ResourceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_GIT_HISTORY_COMMITS: usize = 10_000;
pub const MAX_GIT_SUBJECT_BYTES: usize = 512;
const MAX_GIT_TIME: u64 = 253_402_300_799;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitHistoryCommit {
    pub sha: String,
    pub committed_at: u64,
    pub subject: String,
    pub session_refs: Vec<ResourceId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitHistorySnapshot {
    pub branch: String,
    pub head_sha: Option<String>,
    /// Newest first, in first-parent order. Commit clocks need not be monotonic.
    pub commits: Vec<GitHistoryCommit>,
    pub uncommitted_session_refs: Vec<ResourceId>,
    /// Sessions too old to place when the first-parent history is truncated.
    pub outside_history_session_refs: Vec<ResourceId>,
    pub truncated: bool,
}

/// What a Push adds to the stored history: the leading commits the stored
/// snapshot lacks, and whether its head left this first-parent rail (amend,
/// rebase, reset). Presentation only; it never gates a Push.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitHistoryOutgoing {
    pub new_commits: usize,
    pub rewritten: bool,
}

/// The closest successor by commit time. Equal times choose the older commit
/// in the first-parent rail, as in the desktop Activity view.
pub fn closest_commit_index(committed_at: &[u64], placed_at: u64) -> Option<usize> {
    committed_at
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, time)| **time >= placed_at)
        .min_by_key(|(_, time)| **time)
        .map(|(index, _)| index)
}

impl GitHistorySnapshot {
    /// Compare with the history storage holds. `None` when the two share no
    /// commit — a first history, or an unrelated branch — since every commit
    /// would read as new and the marks would say nothing.
    pub fn outgoing_since(&self, stored: &GitHistorySnapshot) -> Option<GitHistoryOutgoing> {
        let stored_shas: BTreeSet<&str> = stored
            .commits
            .iter()
            .map(|commit| commit.sha.as_str())
            .collect();
        let new_commits = self
            .commits
            .iter()
            .position(|commit| stored_shas.contains(commit.sha.as_str()))?;
        let rewritten = stored
            .head_sha
            .as_deref()
            .is_some_and(|head| !self.commits.iter().any(|commit| commit.sha == head));
        Some(GitHistoryOutgoing {
            new_commits,
            rewritten,
        })
    }

    /// Rebuild all references from the final manifest, including retained
    /// conversations. A failed or omitted capture cannot add a reference.
    pub fn map_sessions<'a>(
        &mut self,
        conversations: impl Iterator<Item = &'a ConversationThread>,
    ) {
        for commit in &mut self.commits {
            commit.session_refs.clear();
        }
        self.uncommitted_session_refs.clear();
        self.outside_history_session_refs.clear();
        let times: Vec<_> = self
            .commits
            .iter()
            .map(|commit| commit.committed_at)
            .collect();
        let oldest_time = times.iter().min().copied();
        let mut sessions: Vec<_> = conversations.collect();
        sessions.sort_by(|left, right| {
            right
                .presentation
                .ended_at
                .unwrap_or(right.updated_at)
                .cmp(&left.presentation.ended_at.unwrap_or(left.updated_at))
                .then_with(|| {
                    left.presentation
                        .started_at
                        .unwrap_or(left.created_at)
                        .cmp(&right.presentation.started_at.unwrap_or(right.created_at))
                })
                .then_with(|| left.resource_id.cmp(&right.resource_id))
        });
        for session in sessions {
            let placed_at = session
                .presentation
                .last_prompt_at
                .or(session.presentation.ended_at)
                .unwrap_or(session.updated_at);
            if self.truncated && oldest_time.is_some_and(|time| placed_at < time) {
                self.outside_history_session_refs
                    .push(session.resource_id.clone());
            } else if let Some(index) = closest_commit_index(&times, placed_at) {
                self.commits[index]
                    .session_refs
                    .push(session.resource_id.clone());
            } else {
                self.uncommitted_session_refs
                    .push(session.resource_id.clone());
            }
        }
    }

    pub fn validate(&self, conversations: &BTreeSet<ResourceId>) -> Result<(), String> {
        if conversations.len() > crate::MAX_RESOURCES {
            return Err("Git history exceeds 20,000 session references".into());
        }
        if self.branch.is_empty()
            || self.branch.len() > 256
            || self.branch.chars().any(char::is_control)
        {
            return Err("Git history branch is invalid".into());
        }
        if self.commits.len() > MAX_GIT_HISTORY_COMMITS {
            return Err("Git history exceeds 10,000 commits".into());
        }
        if self.head_sha.as_deref() != self.commits.first().map(|commit| commit.sha.as_str()) {
            return Err("Git history head does not match its first commit".into());
        }
        if self.truncated && self.commits.is_empty() {
            return Err("Truncated Git history must contain commits".into());
        }
        if !self.truncated && !self.outside_history_session_refs.is_empty() {
            return Err("Complete Git history cannot have sessions outside the history".into());
        }
        let mut shas = BTreeSet::new();
        let mut refs = BTreeSet::new();
        for commit in &self.commits {
            if !matches!(commit.sha.len(), 40 | 64)
                || !commit
                    .sha
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || !shas.insert(&commit.sha)
                || commit.committed_at > MAX_GIT_TIME
                || commit.subject.len() > MAX_GIT_SUBJECT_BYTES
                || commit.subject.chars().any(char::is_control)
            {
                return Err("Git history commit is invalid".into());
            }
        }
        for resource_id in self
            .commits
            .iter()
            .flat_map(|commit| &commit.session_refs)
            .chain(&self.uncommitted_session_refs)
            .chain(&self.outside_history_session_refs)
        {
            if !conversations.contains(resource_id) || !refs.insert(resource_id.clone()) {
                return Err("Git history session reference is missing or repeated".into());
            }
        }
        if refs != *conversations {
            return Err("Git history must reference every published session once".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successor_placement_handles_ties_skew_and_no_successor() {
        let times = [300, 200, 200, 250, 100];
        assert_eq!(closest_commit_index(&times, 200), Some(2));
        assert_eq!(closest_commit_index(&times, 210), Some(3));
        assert_eq!(closest_commit_index(&times, 301), None);
        assert_eq!(closest_commit_index(&[], 1), None);
    }

    #[test]
    fn snapshot_rejects_missing_duplicate_and_unknown_sessions() {
        let a = ResourceId::parse("conversation-a").unwrap();
        let b = ResourceId::parse("conversation-b").unwrap();
        let sessions = BTreeSet::from([a.clone(), b.clone()]);
        let mut history = GitHistorySnapshot {
            branch: "main".into(),
            head_sha: Some("a".repeat(40)),
            truncated: false,
            commits: vec![GitHistoryCommit {
                sha: "a".repeat(40),
                committed_at: 100,
                subject: "Save work".into(),
                session_refs: vec![a.clone()],
            }],
            uncommitted_session_refs: vec![b.clone()],
            outside_history_session_refs: vec![],
        };
        history.validate(&sessions).unwrap();
        history.uncommitted_session_refs.clear();
        assert!(history.validate(&sessions).is_err());
        history.uncommitted_session_refs = vec![a, b];
        assert!(history.validate(&sessions).is_err());
        assert!(history.validate(&BTreeSet::new()).is_err());
    }

    #[test]
    fn outgoing_counts_new_commits_and_flags_rewrites() {
        let rail = |branch: &str, shas: &[char]| GitHistorySnapshot {
            branch: branch.into(),
            head_sha: shas.first().map(|sha| sha.to_string().repeat(40)),
            truncated: false,
            commits: shas
                .iter()
                .enumerate()
                .map(|(index, sha)| GitHistoryCommit {
                    sha: sha.to_string().repeat(40),
                    committed_at: 100 - index as u64,
                    subject: "Save work".into(),
                    session_refs: vec![],
                })
                .collect(),
            uncommitted_session_refs: vec![],
            outside_history_session_refs: vec![],
        };
        let outgoing = |new_commits, rewritten| {
            Some(GitHistoryOutgoing {
                new_commits,
                rewritten,
            })
        };
        let stored = rail("main", &['c', 'b', 'a']);
        // Fast-forward: two commits on top of the stored head.
        assert_eq!(
            rail("main", &['e', 'd', 'c', 'b', 'a']).outgoing_since(&stored),
            outgoing(2, false)
        );
        assert_eq!(stored.outgoing_since(&stored), outgoing(0, false));
        // Another branch at the same head adds nothing.
        assert_eq!(
            rail("pelican-art", &['c', 'b', 'a']).outgoing_since(&stored),
            outgoing(0, false)
        );
        // Amend: the new head replaces the stored one on the shared parent.
        assert_eq!(
            rail("main", &['x', 'b', 'a']).outgoing_since(&stored),
            outgoing(1, true)
        );
        // Reset back: nothing new, but the stored head is gone.
        assert_eq!(
            rail("main", &['b', 'a']).outgoing_since(&stored),
            outgoing(0, true)
        );
        // No shared commit: unrelated history, an unborn branch on either side.
        assert_eq!(rail("main", &['z', 'y']).outgoing_since(&stored), None);
        assert_eq!(rail("main", &[]).outgoing_since(&stored), None);
        assert_eq!(stored.outgoing_since(&rail("main", &[])), None);
    }
}

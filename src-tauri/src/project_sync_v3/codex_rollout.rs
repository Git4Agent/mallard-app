//! Shared classification of Codex rollout `session_meta` payloads.
//!
//! Provider capture and the local History page must agree on which rollouts
//! are user-visible root tasks: a surface that hides a rollout another
//! surface uploads would leak internal subagent transcripts into bundles.
//! This module is the single, filesystem-free definition of that boundary.
//!
//! Codex subagent rollouts mark themselves in two ways on the `session_meta`
//! payload:
//!
//! - `source` is polymorphic: a plain string such as `"vscode"` on normal
//!   root sessions, and an object such as
//!   `{"subagent": {"thread_spawn": {"parent_thread_id": ...}}}` or
//!   `{"subagent": {"other": "guardian"}}` on subagent rollouts.
//! - `thread_source` is the string `"subagent"`.
//!
//! Genuine spawned descendants carry both markers at once, so the `source`
//! variant is classified first and `thread_source` acts only as a
//! fail-closed fallback for shapes this module does not recognize. Top-level
//! provenance fields (`session_id`, `parent_thread_id`, `forked_from_id`)
//! never participate: ordinary non-subagent forks carry them too, and they
//! must not turn a root into a spawned agent.

use serde_json::Value;

/// Parent thread IDs obey the same bounds as validated session IDs.
const MAX_THREAD_ID_CHARS: usize = 512;

/// `source.subagent.other` values upstream documents as internal machinery
/// that does not participate in the spawn-edge lifecycle.
const RECOGNIZED_INTERNAL_SOURCES: [&str; 3] = ["guardian", "review", "compact"];

/// Role of one Codex rollout, derived from its `session_meta` metadata.
/// Only `Root` is eligible for capture; every other class is excluded from
/// sync fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexRolloutClass {
    /// A user-visible root task.
    Root,
    /// A genuine `source.subagent.thread_spawn` descendant of a root task.
    ThreadSpawn,
    /// Recognized internal machinery (guardian, review, compact).
    InternalSubagent,
    /// An unrecognized or malformed subagent marker.
    UnknownSubagent,
}

impl CodexRolloutClass {
    pub(crate) fn is_root(self) -> bool {
        matches!(self, CodexRolloutClass::Root)
    }
}

/// Classification of one rollout together with the validated parent thread
/// ID retained for genuine spawned descendants. The parent is present only
/// when `class` is `ThreadSpawn`; roots never expose one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexRolloutClassification {
    pub(crate) class: CodexRolloutClass,
    pub(crate) parent_thread_id: Option<String>,
}

impl CodexRolloutClassification {
    pub(crate) fn root() -> Self {
        Self {
            class: CodexRolloutClass::Root,
            parent_thread_id: None,
        }
    }

    fn internal() -> Self {
        Self {
            class: CodexRolloutClass::InternalSubagent,
            parent_thread_id: None,
        }
    }

    fn unknown() -> Self {
        Self {
            class: CodexRolloutClass::UnknownSubagent,
            parent_thread_id: None,
        }
    }

    pub(crate) fn is_root(&self) -> bool {
        self.class.is_root()
    }

    /// Fold another `session_meta` record's classification into this one.
    ///
    /// Resumed rollouts can append additional `session_meta` records, and a
    /// rollout is a root only when no record carries a subagent marker.
    /// Records that agree keep their class; disagreeing subagent classes or
    /// conflicting parent IDs fail closed to `UnknownSubagent`.
    pub(crate) fn absorb(&mut self, other: CodexRolloutClassification) {
        if other.is_root() {
            return;
        }
        if self.is_root() {
            *self = other;
            return;
        }
        if *self != other {
            *self = Self::unknown();
        }
    }
}

/// Classify one `session_meta` payload. Malformed subagent metadata never
/// falls back to `Root`.
pub(crate) fn classify_session_meta(payload: &Value) -> CodexRolloutClassification {
    // `Value::get` returns `None` for the plain-string `source` of a normal
    // root session, so only object sources can carry the subagent marker.
    if let Some(subagent) = payload
        .get("source")
        .and_then(|source| source.get("subagent"))
    {
        return classify_subagent_source(subagent);
    }
    if payload.get("thread_source").and_then(Value::as_str) == Some("subagent") {
        return CodexRolloutClassification::unknown();
    }
    CodexRolloutClassification::root()
}

fn classify_subagent_source(subagent: &Value) -> CodexRolloutClassification {
    let Some(variants) = subagent.as_object() else {
        return CodexRolloutClassification::unknown();
    };
    if let Some(spawn) = variants.get("thread_spawn") {
        let parent = spawn
            .get("parent_thread_id")
            .and_then(Value::as_str)
            .filter(|id| is_valid_thread_id(id));
        return match parent {
            Some(parent) => CodexRolloutClassification {
                class: CodexRolloutClass::ThreadSpawn,
                parent_thread_id: Some(parent.to_string()),
            },
            None => CodexRolloutClassification::unknown(),
        };
    }
    if let Some(kind) = variants.get("other").and_then(Value::as_str) {
        if RECOGNIZED_INTERNAL_SOURCES.contains(&kind) {
            return CodexRolloutClassification::internal();
        }
        return CodexRolloutClassification::unknown();
    }
    CodexRolloutClassification::unknown()
}

fn is_valid_thread_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_THREAD_ID_CHARS && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn classify(payload: Value) -> CodexRolloutClassification {
        classify_session_meta(&payload)
    }

    #[test]
    fn plain_string_source_is_root() {
        let result = classify(json!({
            "id": "root-thread",
            "cwd": "/tmp/project",
            "source": "vscode"
        }));
        assert_eq!(result.class, CodexRolloutClass::Root);
        assert_eq!(result.parent_thread_id, None);
    }

    #[test]
    fn missing_source_is_root() {
        let result = classify(json!({"id": "root-thread", "cwd": "/tmp/project"}));
        assert!(result.is_root());
    }

    #[test]
    fn object_source_without_subagent_marker_is_root() {
        let result = classify(json!({"id": "t", "source": {"vscode": {}}}));
        assert!(result.is_root());
    }

    #[test]
    fn fork_provenance_fields_do_not_make_a_root_a_spawned_agent() {
        let result = classify(json!({
            "id": "forked-thread",
            "session_id": "origin-thread",
            "parent_thread_id": "origin-thread",
            "forked_from_id": "origin-thread",
            "source": "vscode"
        }));
        assert_eq!(result.class, CodexRolloutClass::Root);
        assert_eq!(result.parent_thread_id, None);
    }

    #[test]
    fn real_shaped_thread_spawn_child_retains_embedded_parent() {
        // Mirrors a live Codex 0.146 spawned child: both markers set, the
        // parent ID present at the top level and inside the spawn source.
        let result = classify(json!({
            "id": "child-thread",
            "session_id": "parent-thread",
            "parent_thread_id": "parent-thread",
            "forked_from_id": "parent-thread",
            "thread_source": "subagent",
            "source": {"subagent": {"thread_spawn": {
                "parent_thread_id": "parent-thread",
                "depth": 1,
                "agent_path": "/root/agent",
                "agent_nickname": "Aquinas",
                "agent_role": null
            }}}
        }));
        assert_eq!(result.class, CodexRolloutClass::ThreadSpawn);
        assert_eq!(result.parent_thread_id.as_deref(), Some("parent-thread"));
    }

    #[test]
    fn recognized_internal_sources_are_internal_subagents() {
        for kind in ["guardian", "review", "compact"] {
            let result = classify(json!({
                "id": "internal-thread",
                "thread_source": "subagent",
                "source": {"subagent": {"other": kind}}
            }));
            assert_eq!(result.class, CodexRolloutClass::InternalSubagent);
            assert_eq!(result.parent_thread_id, None);
        }
    }

    #[test]
    fn unrecognized_subagent_shapes_are_unknown_subagents() {
        for source in [
            json!({"subagent": {"other": "mystery"}}),
            json!({"subagent": {"other": 7}}),
            json!({"subagent": {"novel_variant": {}}}),
            json!({"subagent": {}}),
            json!({"subagent": "guardian"}),
        ] {
            let result = classify(json!({"id": "t", "source": source}));
            assert_eq!(result.class, CodexRolloutClass::UnknownSubagent);
            assert_eq!(result.parent_thread_id, None);
        }
    }

    #[test]
    fn thread_source_marker_alone_cannot_fall_back_to_root() {
        let result = classify(json!({"id": "t", "thread_source": "subagent", "source": "vscode"}));
        assert_eq!(result.class, CodexRolloutClass::UnknownSubagent);
    }

    #[test]
    fn malformed_thread_spawn_parents_never_classify_as_root_or_spawn() {
        let oversized = "x".repeat(MAX_THREAD_ID_CHARS + 1);
        for spawn in [
            json!({}),
            json!({"parent_thread_id": ""}),
            json!({"parent_thread_id": "bad\u{0007}id"}),
            json!({"parent_thread_id": oversized}),
            json!({"parent_thread_id": 42}),
            json!("parent-thread"),
        ] {
            let result = classify(json!({
                "id": "t",
                "source": {"subagent": {"thread_spawn": spawn}}
            }));
            assert_eq!(result.class, CodexRolloutClass::UnknownSubagent);
            assert_eq!(result.parent_thread_id, None);
        }
    }

    fn spawned(parent: &str) -> CodexRolloutClassification {
        CodexRolloutClassification {
            class: CodexRolloutClass::ThreadSpawn,
            parent_thread_id: Some(parent.to_string()),
        }
    }

    #[test]
    fn absorb_keeps_agreeing_records_and_fails_closed_on_conflicts() {
        let mut root = CodexRolloutClassification::root();
        root.absorb(CodexRolloutClassification::root());
        assert!(root.is_root());

        let mut becomes_spawn = CodexRolloutClassification::root();
        becomes_spawn.absorb(spawned("parent"));
        assert_eq!(becomes_spawn, spawned("parent"));

        let mut stays_spawn = spawned("parent");
        stays_spawn.absorb(CodexRolloutClassification::root());
        stays_spawn.absorb(spawned("parent"));
        assert_eq!(stays_spawn, spawned("parent"));

        let mut conflicting_parents = spawned("parent-a");
        conflicting_parents.absorb(spawned("parent-b"));
        assert_eq!(
            conflicting_parents.class,
            CodexRolloutClass::UnknownSubagent
        );
        assert_eq!(conflicting_parents.parent_thread_id, None);

        let mut conflicting_classes = spawned("parent");
        conflicting_classes.absorb(CodexRolloutClassification::internal());
        assert_eq!(
            conflicting_classes.class,
            CodexRolloutClass::UnknownSubagent
        );

        let mut agreeing_internal = CodexRolloutClassification::internal();
        agreeing_internal.absorb(CodexRolloutClassification::internal());
        assert_eq!(agreeing_internal.class, CodexRolloutClass::InternalSubagent);
    }
}

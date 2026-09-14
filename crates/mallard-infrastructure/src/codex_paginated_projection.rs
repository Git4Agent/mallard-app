//! Codex paginated item projection for standalone rollout reconstruction.
//!
//! The Codex 0.147 paginated projector records a turn row only from turn
//! lifecycle events and a display item row only from an
//! `event_msg`/`item_completed` record. Legacy display events and ordinary
//! `response_item` records are context, not display state. This module owns
//! the frozen 0.147 adapter that makes a flattened standalone rollout
//! displayable: it classifies every `event_msg` payload, converts recognized
//! legacy display completions into current `TurnItem` projections, attaches
//! out-of-turn completions to synthesized implicit `rollout-N` turns exactly
//! as Codex's own canonicalizer does, and stays a fixpoint on its
//! own output so rebuilds are byte-identical.
//!
//! Original non-metadata records always pass through unchanged, in order.
//! The only inserted records are `item_completed` projections and implicit
//! turn lifecycle records; the only mutated field of an original record is
//! the `thread_id` inside a current `item_completed` envelope, which is
//! normalized to the output root identity.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use chrono::DateTime;
use serde_json::{Map, Value};

/// Upper bound for provider turn and item identifier strings.
const MAX_IDENTIFIER_BYTES: usize = 256;

/// The complete Codex 0.147 `TurnItem` kind set. A preserved current
/// `item_completed` record must carry one of these; Mallard does not
/// deep-validate payload fields it merely round-trips.
const ITEM_KINDS: [&str; 18] = [
    "AgentMessage",
    "CollabAgentToolCall",
    "CommandExecution",
    "ContextCompaction",
    "DynamicToolCall",
    "EnteredReviewMode",
    "ExitedReviewMode",
    "Extension",
    "FileChange",
    "HookPrompt",
    "ImageGeneration",
    "ImageView",
    "McpToolCall",
    "Plan",
    "Reasoning",
    "SubAgentActivity",
    "UserMessage",
    "WebSearch",
];

/// `event_msg` payload types that are context, never display completions.
/// Frozen to the 0.147 persistence surface plus every type observed in real
/// rollouts. `view_image_tool_call` is deliberately context-only: the 0.147
/// canonical mapping produces no item for it.
const CONTEXT_ONLY_EVENTS: [&str; 8] = [
    "error",
    "item_started",
    "thread_goal_updated",
    "thread_name_updated",
    "thread_rolled_back",
    "thread_settings_applied",
    "token_count",
    "view_image_tool_call",
];

/// Complete a flattened standalone record sequence with the current-format
/// projection records paginated Codex history requires. `records` holds the
/// ordered non-metadata records with top-level ordinals already removed;
/// `output_root_thread_id` is the final provider identity of the standalone
/// root. Original records come back unchanged and in order, with projection
/// and implicit-turn records inserted at the positions the frozen adapter
/// selects. `check_inserted` checks each new record before it enters the
/// output. Preserved records and adopted projections do not invoke it.
pub fn complete_paginated_item_projection(
    records: Vec<Value>,
    output_root_thread_id: &str,
    check_inserted: &mut dyn FnMut(&Value) -> Result<(), String>,
) -> Result<Vec<Value>, String> {
    Ok(
        project_with_sources(records.iter(), output_root_thread_id, check_inserted)?
            .into_iter()
            .map(|record| record.value.into_owned())
            .collect(),
    )
}

pub(crate) struct ProjectedRecord<'r> {
    pub value: Cow<'r, Value>,
    pub source_index: Option<usize>,
    pub derived: bool,
}

pub(crate) fn project_with_sources<'r>(
    records: impl IntoIterator<Item = &'r Value>,
    output_root_thread_id: &str,
    check_inserted: &mut dyn FnMut(&Value) -> Result<(), String>,
) -> Result<Vec<ProjectedRecord<'r>>, String> {
    let records: Vec<_> = records.into_iter().collect();
    let mut projector = Projector::new(&records, output_root_thread_id, check_inserted)?;
    for index in 0..records.len() {
        projector.current_source = index;
        projector.process(&records, index)?;
    }
    projector.finish()
}

/// Prove the projection pass is a fixpoint on `records`: running it again
/// must add nothing and change nothing. This is the projection verifier —
/// the same state machine that produced the output, not a second
/// implementation of its rules.
pub fn verify_projection_fixpoint(
    records: &[Value],
    output_root_thread_id: &str,
) -> Result<(), String> {
    let reprojected = project_with_sources(records.iter(), output_root_thread_id, &mut |_| Ok(()))?;
    if reprojected.len() != records.len() {
        return Err(format!(
            "paginated projection is not a fixpoint: {} records reprojected to {}",
            records.len(),
            reprojected.len()
        ));
    }
    for (index, (before, after)) in records.iter().zip(&reprojected).enumerate() {
        if before != after.value.as_ref() {
            return Err(format!(
                "paginated projection is not a fixpoint at record {index}"
            ));
        }
    }
    Ok(())
}

struct ActiveTurn {
    id: String,
    explicit: bool,
    saw_user: bool,
}

struct ReasoningRun {
    turn_id: String,
    summary_text: Vec<String>,
    raw_content: Vec<String>,
    /// Position in the output vector of the run's last reasoning record.
    last_output_index: usize,
    /// Source index of the run's last reasoning record, for adjacency
    /// adoption of an already-materialized run projection.
    last_source_index: usize,
    last_timestamp: String,
}

struct Projector<'a, 'r> {
    output_root_thread_id: &'a str,
    check_inserted: &'a mut dyn FnMut(&Value) -> Result<(), String>,
    output: Vec<Cow<'r, Value>>,
    output_sources: Vec<Option<usize>>,
    redundant_sources: BTreeSet<usize>,
    current_source: usize,
    active_turn: Option<ActiveTurn>,
    known_turn_ids: BTreeSet<String>,
    run: Option<ReasoningRun>,
    used_item_ids: BTreeSet<String>,
    next_generated_item_index: u64,
    /// Pre-parsed hook prompts keyed by source index of their response item.
    hook_prompts: BTreeMap<usize, HookPromptSource>,
    /// Source index of a projection record the previous completion adopted.
    /// It replays as pure output: preserved without touching machine state,
    /// exactly as when it was first emitted.
    adopted_candidate_index: Option<usize>,
    last_timestamp: Option<String>,
}

impl<'a, 'r> Projector<'a, 'r> {
    fn new(
        records: &[&'r Value],
        output_root_thread_id: &'a str,
        check_inserted: &'a mut dyn FnMut(&Value) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut used_item_ids = BTreeSet::new();
        let mut hook_prompts = BTreeMap::new();
        for (index, record) in records.iter().enumerate() {
            let Some(outer) = record.get("type").and_then(Value::as_str) else {
                continue;
            };
            match outer {
                "event_msg" => {
                    let Some(payload) = record.get("payload").and_then(Value::as_object) else {
                        continue;
                    };
                    match payload.get("type").and_then(Value::as_str) {
                        Some("item_completed") => {
                            if let Some(id) = payload
                                .get("item")
                                .and_then(Value::as_object)
                                .and_then(|item| item.get("id"))
                                .and_then(Value::as_str)
                            {
                                used_item_ids.insert(id.to_string());
                            }
                        }
                        Some(
                            "exec_command_end"
                            | "patch_apply_end"
                            | "mcp_tool_call_end"
                            | "web_search_end"
                            | "image_generation_end"
                            | "dynamic_tool_call_response",
                        ) => {
                            if let Some(id) = payload.get("call_id").and_then(Value::as_str) {
                                used_item_ids.insert(id.to_string());
                            }
                        }
                        Some("sub_agent_activity") => {
                            if let Some(id) = payload.get("event_id").and_then(Value::as_str) {
                                used_item_ids.insert(id.to_string());
                            }
                        }
                        Some("entered_review_mode" | "exited_review_mode") => {
                            if let Some(id) = payload.get("item_id").and_then(Value::as_str) {
                                used_item_ids.insert(id.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                "response_item" => {
                    if let Some(hook) = parse_hook_prompt_response_item(record) {
                        if let Some(id) = hook.item_id.as_deref() {
                            used_item_ids.insert(id.to_string());
                        }
                        hook_prompts.insert(index, hook);
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            output_root_thread_id,
            check_inserted,
            output: Vec::with_capacity(records.len()),
            output_sources: Vec::with_capacity(records.len()),
            redundant_sources: BTreeSet::new(),
            current_source: 0,
            active_turn: None,
            known_turn_ids: BTreeSet::new(),
            run: None,
            used_item_ids,
            next_generated_item_index: 1,
            hook_prompts,
            adopted_candidate_index: None,
            last_timestamp: None,
        })
    }

    fn process(&mut self, records: &[&'r Value], index: usize) -> Result<(), String> {
        let record = records[index];
        if let Some(timestamp) = record.get("timestamp").and_then(Value::as_str) {
            self.last_timestamp = Some(timestamp.to_string());
        }
        let outer = record
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex rollout record has no type".to_string())?
            .to_string();
        if outer != "event_msg" {
            if outer == "response_item" {
                if let Some(hook) = self.hook_prompts.get(&index).cloned() {
                    let timestamp = record_timestamp(record, "hook prompt response item")?;
                    self.flush_reasoning()?;
                    self.ensure_turn(&timestamp, index)?;
                    // A synthetic start must precede its source message.
                    // Otherwise reprojection opens a second implicit turn.
                    self.push_source(Cow::Borrowed(record));
                    let generated = hook.item_id.is_none();
                    let item = hook_prompt_item(hook.item_id, &hook.fragments);
                    self.adopt_or_emit(
                        records,
                        index,
                        &timestamp,
                        None,
                        item,
                        IdIsGenerated(generated),
                    )?;
                    return Ok(());
                }
            }
            self.push_source(Cow::Borrowed(record));
            return Ok(());
        }
        let payload = record
            .get("payload")
            .and_then(Value::as_object)
            .ok_or_else(|| "Codex event record has no payload object".to_string())?;
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex event payload has no type".to_string())?
            .to_string();
        match event_type.as_str() {
            "task_started" | "turn_started" => {
                self.flush_reasoning()?;
                let timestamp = record_timestamp(record, &event_type)?;
                let turn_id = required_identifier(payload, "turn_id", &event_type)?;
                self.finish_implicit_turn(&timestamp)?;
                self.known_turn_ids.insert(turn_id.clone());
                // A start while an explicit turn is still open replaces it
                // silently, matching the 0.147 canonicalizer. Crash-shaped
                // rollouts are ordinary inputs.
                self.active_turn = Some(ActiveTurn {
                    id: turn_id,
                    explicit: true,
                    saw_user: false,
                });
                self.push_source(Cow::Borrowed(record));
            }
            "task_complete" | "turn_complete" => {
                self.flush_reasoning()?;
                let turn_id = required_identifier(payload, "turn_id", &event_type)?;
                if self
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| turn.id == turn_id)
                {
                    self.active_turn = None;
                }
                self.push_source(Cow::Borrowed(record));
            }
            "turn_aborted" | "task_aborted" => {
                self.flush_reasoning()?;
                match payload.get("turn_id").and_then(Value::as_str) {
                    Some(turn_id) => {
                        if self
                            .active_turn
                            .as_ref()
                            .is_some_and(|turn| turn.id == turn_id)
                        {
                            self.active_turn = None;
                        }
                    }
                    // Upstream backfills a missing abort turn ID from the
                    // active turn and then clears it; Mallard clears the
                    // association without mutating the original record.
                    None => self.active_turn = None,
                }
                self.push_source(Cow::Borrowed(record));
            }
            "item_completed" => {
                let normalized = self.normalize_current_item_record(record)?;
                if self.adopted_candidate_index.take() == Some(index) {
                    self.redundant_sources.insert(index);
                    self.push_source(Cow::Owned(normalized));
                    return Ok(());
                }
                if self.adopt_reasoning_projection(&normalized, index)? {
                    self.redundant_sources.insert(index);
                    self.push_source(Cow::Owned(normalized));
                    return Ok(());
                }
                self.flush_reasoning()?;
                self.push_source(Cow::Owned(normalized));
            }
            "user_message" => {
                self.flush_reasoning()?;
                let timestamp = record_timestamp(record, &event_type)?;
                if self
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| !turn.explicit && turn.saw_user)
                {
                    self.finish_implicit_turn(&timestamp)?;
                }
                self.ensure_turn(&timestamp, index)?;
                if let Some(turn) = self.active_turn.as_mut() {
                    turn.saw_user = true;
                }
                let item_body = user_message_item_body(payload)?;
                self.push_source(Cow::Borrowed(record));
                self.adopt_or_emit(
                    records,
                    index,
                    &timestamp,
                    None,
                    item_body,
                    IdIsGenerated(true),
                )?;
            }
            "agent_message" => {
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Codex agent_message event has no message".to_string())?;
                if message.is_empty() {
                    // Upstream skips empty agent messages without touching
                    // reasoning state; the original record still travels.
                    self.push_source(Cow::Borrowed(record));
                    return Ok(());
                }
                self.flush_reasoning()?;
                let timestamp = record_timestamp(record, &event_type)?;
                self.ensure_turn(&timestamp, index)?;
                let item_body = agent_message_item_body(payload, message);
                self.push_source(Cow::Borrowed(record));
                self.adopt_or_emit(
                    records,
                    index,
                    &timestamp,
                    None,
                    item_body,
                    IdIsGenerated(true),
                )?;
            }
            "agent_reasoning" | "agent_reasoning_raw_content" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let text = payload
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("Codex {event_type} event has no text"))?
                    .to_string();
                if text.is_empty() {
                    self.push_source(Cow::Borrowed(record));
                    return Ok(());
                }
                self.ensure_turn(&timestamp, index)?;
                self.push_source(Cow::Borrowed(record));
                let turn_id = self
                    .active_turn
                    .as_ref()
                    .map(|turn| turn.id.clone())
                    .ok_or_else(|| "Codex reasoning event has no active turn".to_string())?;
                let last_output_index = self.output.len() - 1;
                match self.run.as_mut() {
                    Some(run) => {
                        if event_type == "agent_reasoning" {
                            run.summary_text.push(text);
                        } else {
                            run.raw_content.push(text);
                        }
                        run.last_output_index = last_output_index;
                        run.last_source_index = index;
                        run.last_timestamp = timestamp;
                    }
                    None => {
                        let (summary_text, raw_content) = if event_type == "agent_reasoning" {
                            (vec![text], Vec::new())
                        } else {
                            (Vec::new(), vec![text])
                        };
                        self.run = Some(ReasoningRun {
                            turn_id,
                            summary_text,
                            raw_content,
                            last_output_index,
                            last_source_index: index,
                            last_timestamp: timestamp,
                        });
                    }
                }
            }
            "exec_command_end" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let hint = Some(required_identifier(payload, "turn_id", &event_type)?);
                let item_body = exec_command_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, hint, item_body, false,
                )?;
            }
            "patch_apply_end" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let hint = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string);
                let item_body = patch_apply_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, hint, item_body, false,
                )?;
            }
            "mcp_tool_call_end" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let item_body = mcp_tool_call_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, None, item_body, false,
                )?;
            }
            "web_search_end" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let item_body = web_search_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, None, item_body, false,
                )?;
            }
            "image_generation_end" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let item_body = image_generation_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, None, item_body, false,
                )?;
            }
            "dynamic_tool_call_response" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let hint = Some(required_identifier(payload, "turn_id", &event_type)?);
                let item_body = dynamic_tool_call_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, hint, item_body, false,
                )?;
            }
            "context_compacted" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let item_body = ItemBody {
                    kind: "ContextCompaction",
                    explicit_id: None,
                    fields: Map::new(),
                };
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, None, item_body, true,
                )?;
            }
            "entered_review_mode" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let hint = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string);
                let item_body = entered_review_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, hint, item_body, true,
                )?;
            }
            "exited_review_mode" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let hint = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string);
                let item_body = exited_review_item_body(payload);
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, hint, item_body, true,
                )?;
            }
            "sub_agent_activity" => {
                let timestamp = record_timestamp(record, &event_type)?;
                let item_body = sub_agent_activity_item_body(payload)?;
                self.complete_with_turn_hint(
                    records, index, record, &timestamp, None, item_body, false,
                )?;
            }
            other if CONTEXT_ONLY_EVENTS.contains(&other) => {
                self.push_source(Cow::Borrowed(record));
            }
            other => {
                return Err(format!(
                    "Codex adapter does not recognize event '{other}'; update the frozen 0.147 \
                     projection mapping before publishing this conversation"
                ));
            }
        }
        Ok(())
    }

    /// Attribute one non-reasoning display completion to a turn using the
    /// 0.147 canonicalizer's dispatch, then preserve the original and adopt
    /// or insert its projection. `reasoning_survives_hint_mismatch` is the
    /// upstream rule: a completion attributed to a different explicit turn
    /// does not end the active reasoning run.
    #[allow(clippy::too_many_arguments)]
    fn complete_with_turn_hint(
        &mut self,
        records: &[&'r Value],
        index: usize,
        record: &'r Value,
        timestamp: &str,
        hint: Option<String>,
        item_body: ItemBody,
        generated_id: bool,
    ) -> Result<(), String> {
        let target = match hint {
            Some(hint_id)
                if self
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| turn.id != hint_id) =>
            {
                // Reasoning survives: the completion belongs to another turn.
                Some(hint_id)
            }
            Some(hint_id) => {
                self.flush_reasoning()?;
                if self.active_turn.is_none() && !self.known_turn_ids.contains(&hint_id) {
                    self.start_implicit_turn(hint_id.clone(), timestamp)?;
                }
                Some(hint_id)
            }
            None => {
                self.flush_reasoning()?;
                self.ensure_turn(timestamp, index)?;
                None
            }
        };
        let turn_id = match target {
            Some(turn_id) => turn_id,
            None => self
                .active_turn
                .as_ref()
                .map(|turn| turn.id.clone())
                .ok_or_else(|| "Codex display completion has no active turn".to_string())?,
        };
        self.push_source(Cow::Borrowed(record));
        self.adopt_or_emit(
            records,
            index,
            timestamp,
            Some(turn_id),
            item_body,
            IdIsGenerated(generated_id),
        )
    }

    /// Adopt the immediately following record when it is this completion's
    /// exact existing projection; otherwise insert a fresh projection. An
    /// unrelated adjacent record simply means the completion is not covered.
    fn adopt_or_emit(
        &mut self,
        records: &[&'r Value],
        index: usize,
        timestamp: &str,
        turn_id: Option<String>,
        item_body: ItemBody,
        generated: IdIsGenerated,
    ) -> Result<(), String> {
        let turn_id = match turn_id {
            Some(turn_id) => turn_id,
            None => self
                .active_turn
                .as_ref()
                .map(|turn| turn.id.clone())
                .ok_or_else(|| "Codex display completion has no active turn".to_string())?,
        };
        let completed_at_ms = rfc3339_millis(timestamp)?;
        if let Some(candidate) = records.get(index + 1) {
            if let Some(adopted_id) = self.adoption_candidate_id(
                candidate,
                timestamp,
                &turn_id,
                &item_body,
                completed_at_ms,
                generated,
            ) {
                self.used_item_ids.insert(adopted_id);
                self.adopted_candidate_index = Some(index + 1);
                return Ok(());
            }
        }
        let id = match &item_body.explicit_id {
            Some(id) => id.clone(),
            None => self.allocate_item_id(),
        };
        let projection =
            self.projection_record(timestamp, &turn_id, &item_body, &id, completed_at_ms);
        self.insert_record(self.output.len(), projection)
    }

    /// When `candidate` equals the exact projection this completion needs —
    /// modulo the normalized `thread_id` and, for generated-ID items, the ID
    /// itself — return the ID to adopt.
    fn adoption_candidate_id(
        &self,
        candidate: &Value,
        timestamp: &str,
        turn_id: &str,
        item_body: &ItemBody,
        completed_at_ms: i64,
        generated: IdIsGenerated,
    ) -> Option<String> {
        let payload = candidate
            .get("payload")
            .and_then(Value::as_object)
            .filter(|payload| {
                payload.get("type").and_then(Value::as_str) == Some("item_completed")
            })?;
        if candidate.get("type").and_then(Value::as_str) != Some("event_msg") {
            return None;
        }
        let candidate_item_id = payload
            .get("item")
            .and_then(Value::as_object)
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= MAX_IDENTIFIER_BYTES)?
            .to_string();
        let expected_id = match (&item_body.explicit_id, generated.0) {
            (Some(explicit), _) => explicit.clone(),
            (None, true) => candidate_item_id.clone(),
            (None, false) => return None,
        };
        let expected =
            self.projection_record(timestamp, turn_id, item_body, &expected_id, completed_at_ms);
        let mut normalized = candidate.clone();
        if let Some(record) = normalized.as_object_mut() {
            record.remove("ordinal");
        }
        if let Some(payload) = normalized.get_mut("payload").and_then(Value::as_object_mut) {
            payload.insert(
                "thread_id".to_string(),
                Value::String(self.output_root_thread_id.to_string()),
            );
        }
        (normalized == expected).then_some(candidate_item_id)
    }

    fn projection_record(
        &self,
        timestamp: &str,
        turn_id: &str,
        item_body: &ItemBody,
        id: &str,
        completed_at_ms: i64,
    ) -> Value {
        let mut item = Map::new();
        item.insert(
            "type".to_string(),
            Value::String(item_body.kind.to_string()),
        );
        item.insert("id".to_string(), Value::String(id.to_string()));
        for (key, value) in &item_body.fields {
            item.insert(key.clone(), value.clone());
        }
        let mut payload = Map::new();
        payload.insert(
            "type".to_string(),
            Value::String("item_completed".to_string()),
        );
        payload.insert(
            "thread_id".to_string(),
            Value::String(self.output_root_thread_id.to_string()),
        );
        payload.insert("turn_id".to_string(), Value::String(turn_id.to_string()));
        payload.insert("item".to_string(), Value::Object(item));
        payload.insert("completed_at_ms".to_string(), Value::from(completed_at_ms));
        let mut record = Map::new();
        record.insert(
            "timestamp".to_string(),
            Value::String(timestamp.to_string()),
        );
        record.insert("type".to_string(), Value::String("event_msg".to_string()));
        record.insert("payload".to_string(), Value::Object(payload));
        Value::Object(record)
    }

    /// Envelope validation and identity normalization for a preserved
    /// current `item_completed` record. Payload fields Mallard round-trips
    /// are not deep-validated; an unknown item kind fails closed.
    fn normalize_current_item_record(&mut self, record: &Value) -> Result<Value, String> {
        let mut normalized = record.clone();
        let payload = normalized
            .get_mut("payload")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Codex item_completed record has no payload".to_string())?;
        let turn_id = payload
            .get("turn_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex item_completed record has no turn_id".to_string())?;
        if turn_id.is_empty() || turn_id.len() > MAX_IDENTIFIER_BYTES {
            return Err("Codex item_completed record has an invalid turn_id".to_string());
        }
        let item = payload
            .get("item")
            .and_then(Value::as_object)
            .ok_or_else(|| "Codex item_completed record has no item".to_string())?;
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex item_completed item has no type".to_string())?;
        if !ITEM_KINDS.contains(&kind) {
            return Err(format!(
                "Codex adapter does not recognize item kind '{kind}'; update the frozen 0.147 \
                 projection mapping before publishing this conversation"
            ));
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex item_completed item has no id".to_string())?;
        if id.is_empty() || id.len() > MAX_IDENTIFIER_BYTES {
            return Err("Codex item_completed item has an invalid id".to_string());
        }
        self.used_item_ids.insert(id.to_string());
        payload.insert(
            "thread_id".to_string(),
            Value::String(self.output_root_thread_id.to_string()),
        );
        Ok(normalized)
    }

    /// Adopt a current `Reasoning` projection that directly follows the
    /// active run's last reasoning record and carries exactly the run's
    /// accumulated snapshot.
    fn adopt_reasoning_projection(
        &mut self,
        normalized: &Value,
        index: usize,
    ) -> Result<bool, String> {
        let Some(run) = self.run.as_ref() else {
            return Ok(false);
        };
        if index != run.last_source_index + 1 {
            return Ok(false);
        }
        let Some(payload) = normalized.get("payload").and_then(Value::as_object) else {
            return Ok(false);
        };
        let Some(item) = payload.get("item").and_then(Value::as_object) else {
            return Ok(false);
        };
        if item.get("type").and_then(Value::as_str) != Some("Reasoning") {
            return Ok(false);
        }
        let Some(candidate_id) = item.get("id").and_then(Value::as_str) else {
            return Ok(false);
        };
        let item_body = reasoning_item_body(run);
        let completed_at_ms = rfc3339_millis(&run.last_timestamp)?;
        let expected = self.projection_record(
            &run.last_timestamp,
            &run.turn_id,
            &item_body,
            candidate_id,
            completed_at_ms,
        );
        let mut candidate = normalized.clone();
        if let Some(record) = candidate.as_object_mut() {
            record.remove("ordinal");
        }
        if candidate == expected {
            self.used_item_ids.insert(candidate_id.to_string());
            self.run = None;
            return Ok(true);
        }
        Ok(false)
    }

    /// Emit the active reasoning run's one cumulative projection immediately
    /// after the run's last reasoning record.
    fn flush_reasoning(&mut self) -> Result<(), String> {
        let Some(run) = self.run.take() else {
            return Ok(());
        };
        let id = self.allocate_item_id();
        let item_body = reasoning_item_body(&run);
        let completed_at_ms = rfc3339_millis(&run.last_timestamp)?;
        let projection = self.projection_record(
            &run.last_timestamp,
            &run.turn_id,
            &item_body,
            &id,
            completed_at_ms,
        );
        self.insert_record(run.last_output_index + 1, projection)
    }

    fn push_source(&mut self, record: Cow<'r, Value>) {
        self.output.push(record);
        self.output_sources.push(Some(self.current_source));
    }

    fn insert_record(&mut self, index: usize, record: Value) -> Result<(), String> {
        (self.check_inserted)(&record)
            .map_err(|error| format!("inserted record {}: {error}", index + 1))?;
        self.output.insert(index, Cow::Owned(record));
        self.output_sources.insert(index, None);
        Ok(())
    }

    fn ensure_turn(&mut self, timestamp: &str, source_index: usize) -> Result<(), String> {
        if self.active_turn.is_some() {
            return Ok(());
        }
        self.start_implicit_turn(format!("rollout-{source_index}"), timestamp)
    }

    fn start_implicit_turn(&mut self, turn_id: String, timestamp: &str) -> Result<(), String> {
        let started_at = rfc3339_seconds(timestamp)?;
        let mut payload = Map::new();
        payload.insert(
            "type".to_string(),
            Value::String("task_started".to_string()),
        );
        payload.insert("turn_id".to_string(), Value::String(turn_id.clone()));
        payload.insert("started_at".to_string(), Value::from(started_at));
        self.insert_record(self.output.len(), event_record(timestamp, payload))?;
        self.known_turn_ids.insert(turn_id.clone());
        self.active_turn = Some(ActiveTurn {
            id: turn_id,
            explicit: false,
            saw_user: false,
        });
        Ok(())
    }

    fn finish_implicit_turn(&mut self, timestamp: &str) -> Result<(), String> {
        let Some(turn) = self.active_turn.as_ref() else {
            return Ok(());
        };
        if turn.explicit {
            return Ok(());
        }
        let turn_id = turn.id.clone();
        self.active_turn = None;
        let completed_at = rfc3339_seconds(timestamp)?;
        let mut payload = Map::new();
        payload.insert(
            "type".to_string(),
            Value::String("task_complete".to_string()),
        );
        payload.insert("turn_id".to_string(), Value::String(turn_id));
        payload.insert("completed_at".to_string(), Value::from(completed_at));
        self.insert_record(self.output.len(), event_record(timestamp, payload))
    }

    fn allocate_item_id(&mut self) -> String {
        loop {
            let candidate = format!("item-{}", self.next_generated_item_index);
            self.next_generated_item_index += 1;
            if !self.used_item_ids.contains(&candidate) {
                self.used_item_ids.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn finish(mut self) -> Result<Vec<ProjectedRecord<'r>>, String> {
        self.flush_reasoning()?;
        if self.active_turn.as_ref().is_some_and(|turn| !turn.explicit) {
            let timestamp = self.last_timestamp.clone().ok_or_else(|| {
                "Codex rollout has an implicit turn but no timestamps".to_string()
            })?;
            self.finish_implicit_turn(&timestamp)?;
        }
        Ok(self
            .output
            .into_iter()
            .zip(self.output_sources)
            .map(|(value, source_index)| {
                let derived =
                    source_index.is_none_or(|index| self.redundant_sources.contains(&index));
                ProjectedRecord {
                    value,
                    source_index,
                    derived,
                }
            })
            .collect())
    }
}

/// Marker for whether a completion's projection uses a generated `item-N`
/// identifier, which adoption may take from the adjacent existing record.
#[derive(Clone, Copy)]
struct IdIsGenerated(bool);

/// One converted item: its `TurnItem` kind tag, an explicit source identity
/// when the contract preserves one, and the remaining payload fields.
struct ItemBody {
    kind: &'static str,
    explicit_id: Option<String>,
    fields: Map<String, Value>,
}

fn event_record(timestamp: &str, payload: Map<String, Value>) -> Value {
    let mut record = Map::new();
    record.insert(
        "timestamp".to_string(),
        Value::String(timestamp.to_string()),
    );
    record.insert("type".to_string(), Value::String("event_msg".to_string()));
    record.insert("payload".to_string(), Value::Object(payload));
    Value::Object(record)
}

fn record_timestamp(record: &Value, event_type: &str) -> Result<String, String> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|timestamp| !timestamp.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "Codex {event_type} record has no timestamp; wall-clock time is not a substitute"
            )
        })
}

fn rfc3339_millis(timestamp: &str) -> Result<i64, String> {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| parsed.timestamp_millis())
        .map_err(|_| format!("Codex record timestamp '{timestamp}' is not RFC 3339"))
}

fn rfc3339_seconds(timestamp: &str) -> Result<i64, String> {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| parsed.timestamp())
        .map_err(|_| format!("Codex record timestamp '{timestamp}' is not RFC 3339"))
}

fn required_identifier(
    payload: &Map<String, Value>,
    key: &str,
    event_type: &str,
) -> Result<String, String> {
    let value = payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Codex {event_type} event has no {key}"))?;
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES {
        return Err(format!("Codex {event_type} event has an invalid {key}"));
    }
    Ok(value.to_string())
}

fn copy_field(fields: &mut Map<String, Value>, payload: &Map<String, Value>, key: &str) {
    if let Some(value) = payload.get(key) {
        if !value.is_null() {
            fields.insert(key.to_string(), value.clone());
        }
    }
}

fn required_field(
    payload: &Map<String, Value>,
    key: &str,
    event_type: &str,
) -> Result<Value, String> {
    payload
        .get(key)
        .cloned()
        .ok_or_else(|| format!("Codex {event_type} event has no {key}"))
}

fn required_string(
    payload: &Map<String, Value>,
    key: &str,
    event_type: &str,
) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Codex {event_type} event has no {key}"))
}

/// `UserMessageEvent -> UserMessageItem` per the 0.147 canonical mapping:
/// text plus remote/local image and audio attachments, in that order.
fn user_message_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let message = required_string(payload, "message", "user_message")?;
    let mut content = Vec::new();
    if !message.trim().is_empty() {
        let mut text = Map::new();
        text.insert("type".to_string(), Value::String("text".to_string()));
        text.insert("text".to_string(), Value::String(message));
        text.insert(
            "text_elements".to_string(),
            payload
                .get("text_elements")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
        content.push(Value::Object(text));
    }
    let image_details = payload
        .get("image_details")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(images) = payload.get("images").and_then(Value::as_array) {
        for (index, image_url) in images.iter().enumerate() {
            let mut image = Map::new();
            image.insert("type".to_string(), Value::String("image".to_string()));
            image.insert("image_url".to_string(), image_url.clone());
            if let Some(detail) = image_details.get(index).filter(|detail| !detail.is_null()) {
                image.insert("detail".to_string(), detail.clone());
            }
            content.push(Value::Object(image));
        }
    }
    let local_image_details = payload
        .get("local_image_details")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(local_images) = payload.get("local_images").and_then(Value::as_array) {
        for (index, path) in local_images.iter().enumerate() {
            let mut image = Map::new();
            image.insert("type".to_string(), Value::String("local_image".to_string()));
            image.insert("path".to_string(), path.clone());
            if let Some(detail) = local_image_details
                .get(index)
                .filter(|detail| !detail.is_null())
            {
                image.insert("detail".to_string(), detail.clone());
            }
            content.push(Value::Object(image));
        }
    }
    if let Some(audio) = payload.get("audio").and_then(Value::as_array) {
        for audio_url in audio {
            let mut entry = Map::new();
            entry.insert("type".to_string(), Value::String("audio".to_string()));
            entry.insert("audio_url".to_string(), audio_url.clone());
            content.push(Value::Object(entry));
        }
    }
    if let Some(local_audio) = payload.get("local_audio").and_then(Value::as_array) {
        for path in local_audio {
            let mut entry = Map::new();
            entry.insert("type".to_string(), Value::String("local_audio".to_string()));
            entry.insert("path".to_string(), path.clone());
            content.push(Value::Object(entry));
        }
    }
    let mut fields = Map::new();
    copy_field(&mut fields, payload, "client_id");
    fields.insert("content".to_string(), Value::Array(content));
    Ok(ItemBody {
        kind: "UserMessage",
        explicit_id: None,
        fields,
    })
}

fn agent_message_item_body(payload: &Map<String, Value>, message: &str) -> ItemBody {
    let mut text = Map::new();
    text.insert("type".to_string(), Value::String("Text".to_string()));
    text.insert("text".to_string(), Value::String(message.to_string()));
    let mut fields = Map::new();
    fields.insert(
        "content".to_string(),
        Value::Array(vec![Value::Object(text)]),
    );
    copy_field(&mut fields, payload, "phase");
    copy_field(&mut fields, payload, "memory_citation");
    ItemBody {
        kind: "AgentMessage",
        explicit_id: None,
        fields,
    }
}

fn reasoning_item_body(run: &ReasoningRun) -> ItemBody {
    let mut fields = Map::new();
    fields.insert(
        "summary_text".to_string(),
        Value::Array(
            run.summary_text
                .iter()
                .map(|text| Value::String(text.clone()))
                .collect(),
        ),
    );
    fields.insert(
        "raw_content".to_string(),
        Value::Array(
            run.raw_content
                .iter()
                .map(|text| Value::String(text.clone()))
                .collect(),
        ),
    );
    ItemBody {
        kind: "Reasoning",
        explicit_id: None,
        fields,
    }
}

/// `ExecCommandEndEvent -> CommandExecutionItem`. A plain legacy `cwd` path
/// becomes its `file:` URI, mirroring the upstream line parser; every real
/// pre-projection rollout on record carries the plain form.
fn exec_command_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "exec_command_end")?;
    let mut fields = Map::new();
    copy_field(&mut fields, payload, "plugin_id");
    copy_field(&mut fields, payload, "script_path");
    copy_field(&mut fields, payload, "process_id");
    fields.insert(
        "command".to_string(),
        required_field(payload, "command", "exec_command_end")?,
    );
    let cwd = required_string(payload, "cwd", "exec_command_end")?;
    fields.insert("cwd".to_string(), Value::String(path_to_file_uri(&cwd)?));
    fields.insert(
        "parsed_cmd".to_string(),
        required_field(payload, "parsed_cmd", "exec_command_end")?,
    );
    fields.insert(
        "source".to_string(),
        payload
            .get("source")
            .cloned()
            .unwrap_or_else(|| Value::String("agent".to_string())),
    );
    copy_field(&mut fields, payload, "interaction_input");
    let status = required_string(payload, "status", "exec_command_end")?;
    if !matches!(status.as_str(), "completed" | "failed" | "declined") {
        return Err(format!(
            "Codex exec_command_end event has unsupported status '{status}'"
        ));
    }
    fields.insert("status".to_string(), Value::String(status));
    for key in ["stdout", "stderr", "aggregated_output"] {
        let text = payload.get(key).and_then(Value::as_str).unwrap_or_default();
        if !text.is_empty() {
            fields.insert(key.to_string(), Value::String(text.to_string()));
        }
    }
    fields.insert(
        "exit_code".to_string(),
        required_field(payload, "exit_code", "exec_command_end")?,
    );
    fields.insert(
        "duration".to_string(),
        required_field(payload, "duration", "exec_command_end")?,
    );
    let formatted = required_string(payload, "formatted_output", "exec_command_end")?;
    if !formatted.is_empty() {
        fields.insert("formatted_output".to_string(), Value::String(formatted));
    }
    Ok(ItemBody {
        kind: "CommandExecution",
        explicit_id: Some(call_id),
        fields,
    })
}

fn patch_apply_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "patch_apply_end")?;
    let mut fields = Map::new();
    fields.insert(
        "changes".to_string(),
        payload
            .get("changes")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new())),
    );
    fields.insert(
        "status".to_string(),
        Value::String(required_string(payload, "status", "patch_apply_end")?),
    );
    for key in ["stdout", "stderr"] {
        let text = payload.get(key).and_then(Value::as_str).unwrap_or_default();
        if !text.is_empty() {
            fields.insert(key.to_string(), Value::String(text.to_string()));
        }
    }
    Ok(ItemBody {
        kind: "FileChange",
        explicit_id: Some(call_id),
        fields,
    })
}

fn mcp_tool_call_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "mcp_tool_call_end")?;
    let invocation = payload
        .get("invocation")
        .and_then(Value::as_object)
        .ok_or_else(|| "Codex mcp_tool_call_end event has no invocation".to_string())?;
    let mut fields = Map::new();
    fields.insert(
        "server".to_string(),
        Value::String(required_string(invocation, "server", "mcp_tool_call_end")?),
    );
    fields.insert(
        "tool".to_string(),
        Value::String(required_string(invocation, "tool", "mcp_tool_call_end")?),
    );
    fields.insert(
        "arguments".to_string(),
        invocation
            .get("arguments")
            .cloned()
            .filter(|value| !value.is_null())
            .unwrap_or(Value::Null),
    );
    // The item envelope is camelCase; the legacy event was snake_case.
    for (source, target) in [
        ("connector_id", "connectorId"),
        ("mcp_app_resource_uri", "mcpAppResourceUri"),
        ("link_id", "linkId"),
        ("app_name", "appName"),
        ("action_name", "actionName"),
        ("plugin_id", "pluginId"),
        ("read_only_hint", "readOnlyHint"),
    ] {
        if let Some(value) = payload.get(source) {
            if !value.is_null() {
                fields.insert(target.to_string(), value.clone());
            }
        }
    }
    let result = required_field(payload, "result", "mcp_tool_call_end")?;
    let result_object = result
        .as_object()
        .ok_or_else(|| "Codex mcp_tool_call_end event has a malformed result".to_string())?;
    if let Some(ok) = result_object.get("Ok") {
        let is_error = ok.get("isError").and_then(Value::as_bool).unwrap_or(false);
        fields.insert(
            "status".to_string(),
            Value::String(if is_error { "failed" } else { "completed" }.to_string()),
        );
        fields.insert("result".to_string(), ok.clone());
    } else if let Some(error) = result_object.get("Err") {
        let message = error
            .as_str()
            .ok_or_else(|| "Codex mcp_tool_call_end event has a malformed error".to_string())?;
        fields.insert("status".to_string(), Value::String("failed".to_string()));
        let mut error_object = Map::new();
        error_object.insert("message".to_string(), Value::String(message.to_string()));
        fields.insert("error".to_string(), Value::Object(error_object));
    } else {
        return Err("Codex mcp_tool_call_end event has a malformed result".to_string());
    }
    fields.insert(
        "duration".to_string(),
        required_field(payload, "duration", "mcp_tool_call_end")?,
    );
    Ok(ItemBody {
        kind: "McpToolCall",
        explicit_id: Some(call_id),
        fields,
    })
}

fn web_search_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "web_search_end")?;
    let mut fields = Map::new();
    fields.insert(
        "query".to_string(),
        Value::String(required_string(payload, "query", "web_search_end")?),
    );
    fields.insert(
        "action".to_string(),
        required_field(payload, "action", "web_search_end")?,
    );
    copy_field(&mut fields, payload, "results");
    Ok(ItemBody {
        kind: "WebSearch",
        explicit_id: Some(call_id),
        fields,
    })
}

fn image_generation_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "image_generation_end")?;
    let mut fields = Map::new();
    fields.insert(
        "status".to_string(),
        Value::String(required_string(payload, "status", "image_generation_end")?),
    );
    copy_field(&mut fields, payload, "revised_prompt");
    fields.insert(
        "result".to_string(),
        Value::String(required_string(payload, "result", "image_generation_end")?),
    );
    copy_field(&mut fields, payload, "saved_path");
    Ok(ItemBody {
        kind: "ImageGeneration",
        explicit_id: Some(call_id),
        fields,
    })
}

fn dynamic_tool_call_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let call_id = required_identifier(payload, "call_id", "dynamic_tool_call_response")?;
    let success = payload
        .get("success")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Codex dynamic_tool_call_response event has no success".to_string())?;
    let mut fields = Map::new();
    copy_field(&mut fields, payload, "namespace");
    fields.insert(
        "tool".to_string(),
        Value::String(required_string(
            payload,
            "tool",
            "dynamic_tool_call_response",
        )?),
    );
    fields.insert(
        "arguments".to_string(),
        required_field(payload, "arguments", "dynamic_tool_call_response")?,
    );
    fields.insert(
        "status".to_string(),
        Value::String(if success { "completed" } else { "failed" }.to_string()),
    );
    fields.insert(
        "content_items".to_string(),
        required_field(payload, "content_items", "dynamic_tool_call_response")?,
    );
    fields.insert("success".to_string(), Value::Bool(success));
    copy_field(&mut fields, payload, "error");
    fields.insert(
        "duration".to_string(),
        required_field(payload, "duration", "dynamic_tool_call_response")?,
    );
    Ok(ItemBody {
        kind: "DynamicToolCall",
        explicit_id: Some(call_id),
        fields,
    })
}

fn entered_review_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let target = match payload.get("target") {
        Some(target) if !target.is_null() => target.clone(),
        // The upstream line parser synthesizes a custom target from bare
        // `prompt` records before conversion.
        _ => match payload.get("prompt").and_then(Value::as_str) {
            Some(prompt) => {
                let mut target = Map::new();
                target.insert("type".to_string(), Value::String("custom".to_string()));
                target.insert(
                    "instructions".to_string(),
                    Value::String(prompt.to_string()),
                );
                Value::Object(target)
            }
            None => {
                return Err("Codex entered_review_mode event has no target".to_string());
            }
        },
    };
    let hint = payload
        .get("user_facing_hint")
        .and_then(Value::as_str)
        .filter(|hint| !hint.is_empty())
        .unwrap_or("Review requested.");
    let mut fields = Map::new();
    fields.insert("target".to_string(), target);
    fields.insert(
        "user_facing_hint".to_string(),
        Value::String(hint.to_string()),
    );
    Ok(ItemBody {
        kind: "EnteredReviewMode",
        explicit_id: review_item_id(payload),
        fields,
    })
}

fn exited_review_item_body(payload: &Map<String, Value>) -> ItemBody {
    let mut fields = Map::new();
    fields.insert(
        "review_output".to_string(),
        payload.get("review_output").cloned().unwrap_or(Value::Null),
    );
    ItemBody {
        kind: "ExitedReviewMode",
        explicit_id: review_item_id(payload),
        fields,
    }
}

fn review_item_id(payload: &Map<String, Value>) -> Option<String> {
    payload
        .get("item_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= MAX_IDENTIFIER_BYTES)
        .map(str::to_string)
}

fn sub_agent_activity_item_body(payload: &Map<String, Value>) -> Result<ItemBody, String> {
    let event_id = required_identifier(payload, "event_id", "sub_agent_activity")?;
    let mut fields = Map::new();
    fields.insert(
        "kind".to_string(),
        required_field(payload, "kind", "sub_agent_activity")?,
    );
    fields.insert(
        "agent_thread_id".to_string(),
        required_field(payload, "agent_thread_id", "sub_agent_activity")?,
    );
    fields.insert(
        "agent_path".to_string(),
        required_field(payload, "agent_path", "sub_agent_activity")?,
    );
    Ok(ItemBody {
        kind: "SubAgentActivity",
        explicit_id: Some(event_id),
        fields,
    })
}

fn hook_prompt_item(id: Option<String>, fragments: &[(String, String)]) -> ItemBody {
    let mut fields = Map::new();
    fields.insert(
        "fragments".to_string(),
        Value::Array(
            fragments
                .iter()
                .map(|(text, hook_run_id)| {
                    let mut fragment = Map::new();
                    fragment.insert("text".to_string(), Value::String(text.clone()));
                    fragment.insert("hookRunId".to_string(), Value::String(hook_run_id.clone()));
                    Value::Object(fragment)
                })
                .collect(),
        ),
    );
    ItemBody {
        kind: "HookPrompt",
        explicit_id: id,
        fields,
    }
}

/// A hook-prompt user message response item: the response item's own ID when
/// it has one, and the `(text, hook_run_id)` fragments it carries.
#[derive(Clone)]
struct HookPromptSource {
    item_id: Option<String>,
    fragments: Vec<(String, String)>,
}

/// Recognize a hook-prompt user message response item: every content entry
/// is an `input_text` whose text is one serialized `<hook_prompt>` fragment.
fn parse_hook_prompt_response_item(record: &Value) -> Option<HookPromptSource> {
    let payload = record.get("payload").and_then(Value::as_object)?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let content = payload.get("content").and_then(Value::as_array)?;
    let mut fragments = Vec::new();
    for entry in content {
        let entry = entry.as_object()?;
        if entry.get("type").and_then(Value::as_str) != Some("input_text") {
            return None;
        }
        let text = entry.get("text").and_then(Value::as_str)?;
        fragments.push(parse_hook_prompt_fragment(text)?);
    }
    if fragments.is_empty() {
        return None;
    }
    let item_id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= MAX_IDENTIFIER_BYTES)
        .map(str::to_string);
    Some(HookPromptSource { item_id, fragments })
}

/// Parse one `<hook_prompt hook_run_id="...">text</hook_prompt>` fragment,
/// unescaping the XML entity forms the upstream serializer emits.
fn parse_hook_prompt_fragment(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix("<hook_prompt")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("hook_run_id=\"")?;
    let quote = rest.find('"')?;
    let hook_run_id = xml_unescape(&rest[..quote])?;
    if hook_run_id.trim().is_empty() {
        return None;
    }
    let rest = rest[quote + 1..].trim_start();
    let rest = rest.strip_prefix('>')?;
    let body = rest.strip_suffix("</hook_prompt>")?;
    Some((xml_unescape(body)?, hook_run_id))
}

fn xml_unescape(text: &str) -> Option<String> {
    if text.contains('<') {
        return None;
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('&') {
        output.push_str(&rest[..position]);
        rest = &rest[position + 1..];
        let end = rest.find(';')?;
        let entity = &rest[..end];
        match entity {
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "amp" => output.push('&'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            _ => {
                let code = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))?;
                output.push(char::from_u32(code)?);
            }
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    Some(output)
}

/// Encode an absolute POSIX path as the `file:` URI the current
/// `CommandExecutionItem` contract stores. Already-encoded URIs pass
/// through; a relative or non-POSIX path fails closed.
fn path_to_file_uri(path: &str) -> Result<String, String> {
    if path.starts_with("file:") {
        return Ok(path.to_string());
    }
    if !path.starts_with('/') {
        return Err(format!(
            "Codex exec_command_end cwd '{path}' is not an absolute path or file URI"
        ));
    }
    let mut uri = String::with_capacity(path.len() + 8);
    uri.push_str("file://");
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char);
            }
            other => {
                uri.push('%');
                uri.push_str(&format!("{other:02X}"));
            }
        }
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OUT: &str = "output-thread";

    fn project(records: Vec<Value>) -> Vec<Value> {
        complete_paginated_item_projection(records, OUT, &mut |_| Ok(())).unwrap()
    }

    #[test]
    fn inserted_checks_cover_emitters_and_skip_adopted_records() {
        let mut redactor = crate::credential_redaction::shared().begin_run();
        let source = vec![
            reasoning("2026-08-12T10:01:00Z", "Check the result."),
            agent("The result is ready."),
        ]
        .into_iter()
        .map(|record| redactor.redact_record_gated(&record).unwrap().0)
        .collect::<Vec<_>>();
        let mut inserted = Vec::new();
        let output = complete_paginated_item_projection(source.clone(), OUT, &mut |record| {
            redactor.verify_wire_value(record)?;
            inserted.push(record.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(inserted.len(), 4);
        assert_eq!(payload_type(&inserted[0]), "task_started");
        assert_eq!(inserted[1]["payload"]["item"]["type"], "Reasoning");
        assert_eq!(inserted[2]["payload"]["item"]["type"], "AgentMessage");
        assert_eq!(payload_type(&inserted[3]), "task_complete");
        for record in &inserted {
            assert!(output.contains(record));
        }

        // Each emission site must propagate a failed construction check.
        for reject_at in 0..inserted.len() {
            let mut calls = 0;
            let error = complete_paginated_item_projection(source.clone(), OUT, &mut |_| {
                let reject = calls == reject_at;
                calls += 1;
                if reject {
                    Err("rejected fixture".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert!(error.contains("rejected fixture"), "{error}");
            assert_eq!(calls, reject_at + 1);
        }

        // Existing projections are adopted. Their excluded thread_id may
        // change without another credential check or a change to their text.
        let new_root = format!("sk-proj-{}", "a".repeat(24));
        let adopted = complete_paginated_item_projection(output.clone(), &new_root, &mut |_| {
            Err("an adopted record must not invoke the insertion check".to_string())
        })
        .unwrap();
        assert_eq!(adopted.len(), output.len());
        for (before, after) in output.iter().zip(&adopted) {
            let mut expected = before.clone();
            if payload_type(before) == "item_completed" {
                expected["payload"]["thread_id"] = json!(new_root);
            }
            assert_eq!(&expected, after);
            redactor.verify_wire_value(after).unwrap();
        }
        verify_projection_fixpoint(&adopted, &new_root).unwrap();
    }

    fn project_err(records: Vec<Value>) -> String {
        complete_paginated_item_projection(records, OUT, &mut |_| Ok(())).unwrap_err()
    }

    fn event(timestamp: &str, payload: Value) -> Value {
        json!({"timestamp": timestamp, "type": "event_msg", "payload": payload})
    }

    fn start(turn: &str) -> Value {
        event(
            "2026-08-12T10:00:00Z",
            json!({"type": "task_started", "turn_id": turn}),
        )
    }

    fn complete(turn: &str) -> Value {
        event(
            "2026-08-12T10:05:00Z",
            json!({"type": "task_complete", "turn_id": turn, "last_agent_message": null}),
        )
    }

    fn user(text: &str) -> Value {
        event(
            "2026-08-12T10:01:00Z",
            json!({"type": "user_message", "message": text, "images": null,
                   "local_images": [], "text_elements": []}),
        )
    }

    fn agent(text: &str) -> Value {
        event(
            "2026-08-12T10:02:00Z",
            json!({"type": "agent_message", "message": text, "phase": "final_answer",
                   "memory_citation": null}),
        )
    }

    fn reasoning(timestamp: &str, text: &str) -> Value {
        event(timestamp, json!({"type": "agent_reasoning", "text": text}))
    }

    fn payload_type(record: &Value) -> &str {
        record["payload"]["type"].as_str().unwrap_or_default()
    }

    fn items(records: &[Value]) -> Vec<&Value> {
        records
            .iter()
            .filter(|record| payload_type(record) == "item_completed")
            .collect()
    }

    #[test]
    fn user_and_agent_messages_project_inside_their_turn() {
        let source = vec![
            start("turn-1"),
            user("Hello"),
            agent("Hi!"),
            complete("turn-1"),
        ];
        let output = project(source.clone());
        assert_eq!(
            output.iter().map(payload_type).collect::<Vec<_>>(),
            vec![
                "task_started",
                "user_message",
                "item_completed",
                "agent_message",
                "item_completed",
                "task_complete",
            ]
        );
        // Originals are preserved verbatim, in order.
        assert_eq!(output[0], source[0]);
        assert_eq!(output[1], source[1]);
        assert_eq!(output[3], source[2]);
        assert_eq!(output[5], source[3]);
        let projected = items(&output);
        assert_eq!(
            projected[0]["payload"],
            json!({
                "type": "item_completed",
                "thread_id": OUT,
                "turn_id": "turn-1",
                "item": {"type": "UserMessage", "id": "item-1",
                          "content": [{"type": "text", "text": "Hello", "text_elements": []}]},
                "completed_at_ms": 1786528860000_i64,
            })
        );
        assert_eq!(
            projected[1]["payload"]["item"],
            json!({"type": "AgentMessage", "id": "item-2",
                   "content": [{"type": "Text", "text": "Hi!"}],
                   "phase": "final_answer"})
        );
        assert_eq!(projected[1]["timestamp"], json!("2026-08-12T10:02:00Z"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn user_message_attachments_remain_structurally_identical() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:01:00Z",
                json!({"type": "user_message", "message": "look",
                       "text_elements": [{"byte_range": {"start": 0, "end": 4}}],
                       "images": ["data:image/png;base64,AA=="],
                       "image_details": ["high"],
                       "local_images": ["/tmp/a.png"],
                       "local_image_details": [null],
                       "audio": ["data:audio/mp3;base64,BB=="],
                       "local_audio": ["/tmp/b.wav"],
                       "client_id": "client-7"}),
            ),
        ];
        let output = project(source);
        let item = &items(&output)[0]["payload"]["item"];
        assert_eq!(
            *item,
            json!({"type": "UserMessage", "id": "item-1", "client_id": "client-7",
            "content": [
                {"type": "text", "text": "look",
                 "text_elements": [{"byte_range": {"start": 0, "end": 4}}]},
                {"type": "image", "image_url": "data:image/png;base64,AA==", "detail": "high"},
                {"type": "local_image", "path": "/tmp/a.png"},
                {"type": "audio", "audio_url": "data:audio/mp3;base64,BB=="},
                {"type": "local_audio", "path": "/tmp/b.wav"},
            ]})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn empty_agent_message_is_preserved_without_an_item() {
        let output = project(vec![start("turn-1"), agent(""), agent("real")]);
        assert_eq!(items(&output).len(), 1);
        assert_eq!(items(&output)[0]["payload"]["item"]["id"], json!("item-1"));
    }

    #[test]
    fn reasoning_run_emits_one_cumulative_item_after_its_last_record() {
        let source = vec![
            start("turn-1"),
            reasoning("2026-08-12T10:01:00Z", "first"),
            event(
                "2026-08-12T10:01:10Z",
                json!({"type": "agent_reasoning_raw_content", "text": "raw"}),
            ),
            event(
                "2026-08-12T10:01:20Z",
                json!({"type": "token_count", "total": 5}),
            ),
            reasoning("2026-08-12T10:01:30Z", "second"),
            complete("turn-1"),
        ];
        let output = project(source);
        assert_eq!(
            output.iter().map(payload_type).collect::<Vec<_>>(),
            vec![
                "task_started",
                "agent_reasoning",
                "agent_reasoning_raw_content",
                "token_count",
                "agent_reasoning",
                "item_completed",
                "task_complete",
            ]
        );
        assert_eq!(
            output[5]["payload"],
            json!({"type": "item_completed", "thread_id": OUT, "turn_id": "turn-1",
                   "item": {"type": "Reasoning", "id": "item-1",
                             "summary_text": ["first", "second"], "raw_content": ["raw"]},
                   "completed_at_ms": 1786528890000_i64})
        );
        assert_eq!(output[5]["timestamp"], json!("2026-08-12T10:01:30Z"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn user_message_splits_reasoning_and_allocates_in_upstream_order() {
        let output = project(vec![
            start("turn-1"),
            reasoning("2026-08-12T10:01:00Z", "think"),
            user("next question"),
            reasoning("2026-08-12T10:03:00Z", "again"),
            complete("turn-1"),
        ]);
        let projected = items(&output);
        assert_eq!(projected[0]["payload"]["item"]["type"], json!("Reasoning"));
        assert_eq!(projected[0]["payload"]["item"]["id"], json!("item-1"));
        assert_eq!(
            projected[1]["payload"]["item"]["type"],
            json!("UserMessage")
        );
        assert_eq!(projected[1]["payload"]["item"]["id"], json!("item-2"));
        assert_eq!(projected[2]["payload"]["item"]["type"], json!("Reasoning"));
        assert_eq!(projected[2]["payload"]["item"]["id"], json!("item-3"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn exec_command_end_maps_call_id_file_uri_and_omits_empty_output() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:02:00Z",
                json!({"type": "exec_command_end", "call_id": "call-9", "turn_id": "turn-1",
                       "command": ["rg", "main"], "cwd": "/tmp/pro ject",
                       "parsed_cmd": [{"type": "search"}], "source": "agent",
                       "status": "completed", "stdout": "hit", "stderr": "",
                       "aggregated_output": "hit", "exit_code": 0,
                       "duration": {"secs": 1, "nanos": 5}, "formatted_output": ""}),
            ),
        ];
        let output = project(source);
        assert_eq!(
            items(&output)[0]["payload"]["item"],
            json!({"type": "CommandExecution", "id": "call-9",
                   "command": ["rg", "main"], "cwd": "file:///tmp/pro%20ject",
                   "parsed_cmd": [{"type": "search"}], "source": "agent",
                   "status": "completed", "stdout": "hit", "aggregated_output": "hit",
                   "exit_code": 0, "duration": {"secs": 1, "nanos": 5}})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn patch_and_review_and_subagent_and_compaction_map_to_their_items() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:02:00Z",
                json!({"type": "patch_apply_end", "call_id": "patch-1", "turn_id": "turn-1",
                       "stdout": "ok", "stderr": "", "success": true,
                       "changes": {"/a.rs": {"type": "update"}}, "status": "completed"}),
            ),
            event(
                "2026-08-12T10:02:10Z",
                json!({"type": "entered_review_mode", "prompt": "check the diff"}),
            ),
            event(
                "2026-08-12T10:02:20Z",
                json!({"type": "exited_review_mode", "review_output": null}),
            ),
            event(
                "2026-08-12T10:02:30Z",
                json!({"type": "sub_agent_activity", "event_id": "sub-1",
                       "occurred_at_ms": 5, "kind": "spawned",
                       "agent_thread_id": "child-1", "agent_path": ["p"]}),
            ),
            event("2026-08-12T10:02:40Z", json!({"type": "context_compacted"})),
        ];
        let output = project(source);
        let projected = items(&output);
        assert_eq!(
            projected[0]["payload"]["item"],
            json!({"type": "FileChange", "id": "patch-1",
                   "changes": {"/a.rs": {"type": "update"}},
                   "status": "completed", "stdout": "ok"})
        );
        assert_eq!(
            projected[1]["payload"]["item"],
            json!({"type": "EnteredReviewMode", "id": "item-1",
                   "target": {"type": "custom", "instructions": "check the diff"},
                   "user_facing_hint": "Review requested."})
        );
        assert_eq!(
            projected[2]["payload"]["item"],
            json!({"type": "ExitedReviewMode", "id": "item-2", "review_output": null})
        );
        assert_eq!(
            projected[3]["payload"]["item"],
            json!({"type": "SubAgentActivity", "id": "sub-1", "kind": "spawned",
                   "agent_thread_id": "child-1", "agent_path": ["p"]})
        );
        assert_eq!(
            projected[4]["payload"]["item"],
            json!({"type": "ContextCompaction", "id": "item-3"})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn mcp_web_image_and_dynamic_tools_map_status_and_results() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:02:00Z",
                json!({"type": "mcp_tool_call_end", "call_id": "mcp-1",
                       "invocation": {"server": "fs", "tool": "read", "arguments": {"p": 1}},
                       "connector_id": "conn", "duration": {"secs": 0, "nanos": 1},
                       "result": {"Ok": {"content": [], "isError": false}}}),
            ),
            event(
                "2026-08-12T10:02:10Z",
                json!({"type": "mcp_tool_call_end", "call_id": "mcp-2",
                       "invocation": {"server": "fs", "tool": "read", "arguments": null},
                       "duration": {"secs": 0, "nanos": 1},
                       "result": {"Err": "boom"}}),
            ),
            event(
                "2026-08-12T10:02:20Z",
                json!({"type": "web_search_end", "call_id": "web-1", "query": "rust",
                       "action": {"type": "search"}, "results": [{"u": 1}]}),
            ),
            event(
                "2026-08-12T10:02:30Z",
                json!({"type": "image_generation_end", "call_id": "img-1",
                       "status": "completed", "revised_prompt": null, "result": "done",
                       "transparent_background": true, "saved_path": "/tmp/i.png"}),
            ),
            event(
                "2026-08-12T10:02:40Z",
                json!({"type": "dynamic_tool_call_response", "call_id": "dyn-1",
                       "turn_id": "turn-1", "namespace": null, "tool": "lint",
                       "arguments": {"a": 1}, "content_items": [], "success": false,
                       "error": "no", "duration": {"secs": 2, "nanos": 0}}),
            ),
        ];
        let output = project(source);
        let projected = items(&output);
        assert_eq!(
            projected[0]["payload"]["item"],
            json!({"type": "McpToolCall", "id": "mcp-1", "server": "fs", "tool": "read",
                   "arguments": {"p": 1}, "connectorId": "conn", "status": "completed",
                   "result": {"content": [], "isError": false},
                   "duration": {"secs": 0, "nanos": 1}})
        );
        assert_eq!(
            projected[1]["payload"]["item"],
            json!({"type": "McpToolCall", "id": "mcp-2", "server": "fs", "tool": "read",
                   "arguments": null, "status": "failed", "error": {"message": "boom"},
                   "duration": {"secs": 0, "nanos": 1}})
        );
        assert_eq!(
            projected[2]["payload"]["item"],
            json!({"type": "WebSearch", "id": "web-1", "query": "rust",
                   "action": {"type": "search"}, "results": [{"u": 1}]})
        );
        assert_eq!(
            projected[3]["payload"]["item"],
            json!({"type": "ImageGeneration", "id": "img-1", "status": "completed",
                   "result": "done", "saved_path": "/tmp/i.png"})
        );
        assert_eq!(
            projected[4]["payload"]["item"],
            json!({"type": "DynamicToolCall", "id": "dyn-1", "tool": "lint",
                   "arguments": {"a": 1}, "status": "failed", "content_items": [],
                   "success": false, "error": "no", "duration": {"secs": 2, "nanos": 0}})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn image_generation_projection_preserves_certified_base64() {
        let result = "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx=";
        let redactor = crate::credential_redaction::shared();
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:02:30Z",
                json!({"type": "image_generation_end", "call_id": "img-1",
                       "status": "completed", "revised_prompt": null, "result": result,
                       "saved_path": "/tmp/i.png"}),
            ),
        ]
        .into_iter()
        .map(|record| redactor.redact_record_gated(&record).unwrap().0)
        .collect();

        let output = project(source);
        assert_eq!(items(&output)[0]["payload"]["item"]["result"], result);
        for record in &output {
            redactor.verify_wire_value(record).unwrap();
        }
    }

    #[test]
    fn hook_prompt_response_item_projects_and_plain_messages_do_not() {
        let hook = json!({"timestamp": "2026-08-12T10:01:00Z", "type": "response_item",
            "payload": {"type": "message", "id": "msg_1", "role": "user",
                        "content": [{"type": "input_text",
                                     "text": "<hook_prompt hook_run_id=\"run-1\">Retry &amp; win.</hook_prompt>"}]}});
        let plain = json!({"timestamp": "2026-08-12T10:02:00Z", "type": "response_item",
            "payload": {"type": "message", "id": "msg_2", "role": "user",
                        "content": [{"type": "input_text", "text": "just text"}]}});
        let output = project(vec![start("turn-1"), hook, plain]);
        let projected = items(&output);
        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0]["payload"]["item"],
            json!({"type": "HookPrompt", "id": "msg_1",
                   "fragments": [{"text": "Retry & win.", "hookRunId": "run-1"}]})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn out_of_turn_completions_get_an_implicit_rollout_turn() {
        let output = project(vec![
            reasoning("2026-08-12T10:00:00Z", "early"),
            user("hello"),
            start("turn-1"),
            user("inside"),
        ]);
        let types: Vec<_> = output.iter().map(payload_type).collect();
        assert_eq!(
            types,
            vec![
                "task_started", // synthesized rollout-0
                "agent_reasoning",
                "item_completed", // one cumulative reasoning item
                "user_message",
                "item_completed",
                "task_complete", // explicit start closes the implicit turn first
                "task_started",  // original turn-1
                "user_message",
                "item_completed",
            ]
        );
        assert_eq!(output[0]["payload"]["turn_id"], json!("rollout-0"));
        assert_eq!(output[2]["payload"]["turn_id"], json!("rollout-0"));
        assert_eq!(output[4]["payload"]["turn_id"], json!("rollout-0"));
        assert_eq!(output[5]["payload"]["turn_id"], json!("rollout-0"));
        assert_eq!(output[8]["payload"]["turn_id"], json!("turn-1"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn implicit_turn_lifecycle_matches_upstream_shapes() {
        let output = project(vec![reasoning("2026-08-12T10:00:00Z", "early")]);
        assert_eq!(
            output[0],
            json!({"timestamp": "2026-08-12T10:00:00Z", "type": "event_msg",
                   "payload": {"type": "task_started", "turn_id": "rollout-0",
                                "started_at": 1786528800_i64}})
        );
        assert_eq!(payload_type(&output[1]), "agent_reasoning");
        assert_eq!(payload_type(&output[2]), "item_completed");
        assert_eq!(output[2]["payload"]["turn_id"], json!("rollout-0"));
        assert_eq!(
            output[3],
            json!({"timestamp": "2026-08-12T10:00:00Z", "type": "event_msg",
                   "payload": {"type": "task_complete", "turn_id": "rollout-0",
                                "completed_at": 1786528800_i64}})
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn consecutive_out_of_turn_user_messages_split_implicit_turns() {
        let output = project(vec![user("one"), user("two")]);
        let starts: Vec<_> = output
            .iter()
            .filter(|record| payload_type(record) == "task_started")
            .map(|record| record["payload"]["turn_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(starts, vec!["rollout-0", "rollout-1"]);
        let completes = output
            .iter()
            .filter(|record| payload_type(record) == "task_complete")
            .count();
        assert_eq!(completes, 2);
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn nested_start_replaces_the_open_turn_and_closes_nothing() {
        let source = vec![start("turn-1"), user("hi"), start("turn-2"), user("again")];
        let output = project(source.clone());
        assert_eq!(
            output.iter().map(payload_type).collect::<Vec<_>>(),
            vec![
                "task_started",
                "user_message",
                "item_completed",
                "task_started",
                "user_message",
                "item_completed",
            ]
        );
        assert_eq!(output[5]["payload"]["turn_id"], json!("turn-2"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn close_without_open_passes_through_and_aborts_clear_the_turn() {
        let source = vec![
            complete("turn-9"),
            start("turn-1"),
            event(
                "2026-08-12T10:03:00Z",
                json!({"type": "turn_aborted", "turn_id": null, "reason": "interrupted"}),
            ),
            user("after abort"),
        ];
        let output = project(source.clone());
        assert_eq!(output[0], source[0]);
        assert_eq!(output[2], source[2]);
        // The abort cleared the active turn, so the user message opens an
        // implicit turn.
        let implicit_start = output
            .iter()
            .find(|record| {
                payload_type(record) == "task_started"
                    && record["payload"]["turn_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("rollout-"))
            })
            .unwrap();
        assert_eq!(implicit_start["payload"]["turn_id"], json!("rollout-3"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn turn_hint_dispatch_matches_upstream() {
        // A hinted completion for another turn keeps the reasoning run open
        // and attributes the item to the hint turn.
        let source = vec![
            start("turn-1"),
            complete("turn-1"),
            start("turn-2"),
            reasoning("2026-08-12T10:02:00Z", "thinking"),
            event(
                "2026-08-12T10:02:30Z",
                json!({"type": "exec_command_end", "call_id": "late-1", "turn_id": "turn-1",
                       "command": ["ls"], "cwd": "/tmp", "parsed_cmd": [],
                       "status": "completed", "stdout": "", "stderr": "",
                       "aggregated_output": "", "exit_code": 0,
                       "duration": {"secs": 0, "nanos": 0}, "formatted_output": ""}),
            ),
            reasoning("2026-08-12T10:02:40Z", "more"),
            complete("turn-2"),
        ];
        let output = project(source);
        let projected = items(&output);
        assert_eq!(projected[0]["payload"]["turn_id"], json!("turn-1"));
        assert_eq!(projected[0]["payload"]["item"]["id"], json!("late-1"));
        // The reasoning run survived the hint mismatch: one cumulative item.
        assert_eq!(projected[1]["payload"]["turn_id"], json!("turn-2"));
        assert_eq!(
            projected[1]["payload"]["item"]["summary_text"],
            json!(["thinking", "more"])
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn hint_for_unknown_turn_without_active_becomes_an_implicit_turn() {
        let output = project(vec![event(
            "2026-08-12T10:02:00Z",
            json!({"type": "patch_apply_end", "call_id": "p-1", "turn_id": "ghost-turn",
                   "stdout": "", "stderr": "", "success": true, "changes": {},
                   "status": "completed"}),
        )]);
        assert_eq!(payload_type(&output[0]), "task_started");
        assert_eq!(output[0]["payload"]["turn_id"], json!("ghost-turn"));
        assert_eq!(output[2]["payload"]["turn_id"], json!("ghost-turn"));
        assert_eq!(payload_type(&output[3]), "task_complete");
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn existing_current_records_are_kept_normalized_not_duplicated() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:01:00Z",
                json!({"type": "item_completed", "thread_id": "old-thread", "turn_id": "turn-1",
                       "item": {"type": "Plan", "id": "plan-1", "text": "step"},
                       "completed_at_ms": 5}),
            ),
            user("hello"),
        ];
        let output = project(source);
        let projected = items(&output);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0]["payload"]["thread_id"], json!(OUT));
        assert_eq!(projected[0]["payload"]["item"]["id"], json!("plan-1"));
        assert_eq!(
            projected[1]["payload"]["item"]["type"],
            json!("UserMessage")
        );
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn plan_item_adjacent_to_a_legacy_completion_is_not_ambiguous() {
        let source = vec![
            start("turn-1"),
            user("hello"),
            event(
                "2026-08-12T10:01:30Z",
                json!({"type": "item_completed", "thread_id": OUT, "turn_id": "turn-1",
                       "item": {"type": "Plan", "id": "plan-1", "text": "step"}}),
            ),
        ];
        let output = project(source);
        let projected = items(&output);
        assert_eq!(projected.len(), 2);
        assert_eq!(
            projected[0]["payload"]["item"]["type"],
            json!("UserMessage")
        );
        assert_eq!(projected[1]["payload"]["item"]["type"], json!("Plan"));
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn existing_item_ids_force_the_allocator_to_skip() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:01:00Z",
                json!({"type": "item_completed", "thread_id": OUT, "turn_id": "turn-1",
                       "item": {"type": "Plan", "id": "item-1", "text": "step"}}),
            ),
            user("hello"),
        ];
        let output = project(source);
        assert_eq!(items(&output)[1]["payload"]["item"]["id"], json!("item-2"));
    }

    #[test]
    fn reprojection_of_reidentified_output_is_byte_stable() {
        let source = vec![
            reasoning("2026-08-12T10:00:00Z", "early"),
            start("turn-1"),
            user("hello"),
            agent("hi"),
            event(
                "2026-08-12T10:02:30Z",
                json!({"type": "exec_command_end", "call_id": "c-1", "turn_id": "turn-1",
                       "command": ["ls"], "cwd": "/tmp", "parsed_cmd": [],
                       "status": "completed", "stdout": "x", "stderr": "",
                       "aggregated_output": "", "exit_code": 0,
                       "duration": {"secs": 0, "nanos": 0}, "formatted_output": ""}),
            ),
            complete("turn-1"),
        ];
        let output =
            complete_paginated_item_projection(source, "fresh-root", &mut |_| Ok(())).unwrap();
        verify_projection_fixpoint(&output, "fresh-root").unwrap();
        let reprojected =
            complete_paginated_item_projection(output.clone(), "fresh-root", &mut |_| Ok(()))
                .unwrap();
        assert_eq!(output, reprojected);
        for record in items(&output) {
            assert_eq!(record["payload"]["thread_id"], json!("fresh-root"));
        }
    }

    #[test]
    fn unknown_event_and_item_kinds_and_bad_timestamps_fail_closed() {
        assert!(project_err(vec![event(
            "2026-08-12T10:00:00Z",
            json!({"type": "mystery_event"}),
        )])
        .contains("mystery_event"));
        assert!(project_err(vec![
            start("turn-1"),
            event(
                "2026-08-12T10:01:00Z",
                json!({"type": "item_completed", "thread_id": OUT, "turn_id": "turn-1",
                       "item": {"type": "FutureKind", "id": "x"}}),
            ),
        ])
        .contains("FutureKind"));
        assert!(project_err(vec![
            start("turn-1"),
            json!({"type": "event_msg",
                   "payload": {"type": "user_message", "message": "hi",
                                "local_images": [], "text_elements": []}}),
        ])
        .contains("timestamp"));
        assert!(project_err(vec![event(
            "2026-08-12T10:00:00Z",
            json!({"type": "task_started", "turn_id": ""}),
        )])
        .contains("turn_id"));
    }

    #[test]
    fn context_events_and_unknown_outer_types_pass_through() {
        let source = vec![
            json!({"timestamp": "2026-08-12T10:00:00Z", "type": "world_state", "payload": {"x": 1}}),
            event(
                "2026-08-12T10:00:01Z",
                json!({"type": "thread_rolled_back", "num_turns": 1}),
            ),
            event(
                "2026-08-12T10:00:02Z",
                json!({"type": "view_image_tool_call",
                                                   "call_id": "v-1", "path": "/tmp/i.png"}),
            ),
            json!({"timestamp": "2026-08-12T10:00:03Z", "type": "future_record", "payload": {}}),
        ];
        let output = project(source.clone());
        assert_eq!(output, source);
    }

    #[test]
    fn current_only_paginated_rollouts_remain_current_only() {
        let source = vec![
            start("turn-1"),
            event(
                "2026-08-12T10:01:00Z",
                json!({"type": "item_completed", "thread_id": OUT, "turn_id": "turn-1",
                       "item": {"type": "UserMessage", "id": "u-1", "content": []},
                       "completed_at_ms": 7}),
            ),
            complete("turn-1"),
        ];
        let output = project(source.clone());
        assert_eq!(output, source);
        verify_projection_fixpoint(&output, OUT).unwrap();
    }

    #[test]
    fn hook_fragment_parser_matches_the_wire_format() {
        assert_eq!(
            parse_hook_prompt_fragment(
                "  <hook_prompt hook_run_id=\"r&amp;1\">a &lt;b&gt; &#x41;</hook_prompt>  "
            ),
            Some(("a <b> A".to_string(), "r&1".to_string()))
        );
        assert_eq!(
            parse_hook_prompt_fragment("<hook_prompt hook_run_id=\"\">x</hook_prompt>"),
            None
        );
        assert_eq!(parse_hook_prompt_fragment("plain text"), None);
        assert_eq!(
            parse_hook_prompt_fragment("<hook_prompt hook_run_id=\"r\"><x/></hook_prompt>"),
            None
        );
    }

    #[test]
    fn file_uri_encoding_is_conservative_and_pass_through() {
        assert_eq!(path_to_file_uri("/tmp/a b").unwrap(), "file:///tmp/a%20b");
        assert_eq!(path_to_file_uri("file:///tmp/x").unwrap(), "file:///tmp/x");
        assert!(path_to_file_uri("relative/path").is_err());
    }
}

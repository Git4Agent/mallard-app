# Plan: Store-only archival for legacy id-less rollouts

Status: Deferred (2026-07-28), not scheduled. Decision of record: legacy
rollouts stay `Blocked` for Push — see PLAN_LEGACY_ROLLOUT_BLOCKED_THREADS.md
T3 and the Push/Pull rules in THREAD_VARIANT_SYNC_REQUIREMENTS.md. This doc
preserves the verified diagnosis and design sketch so the work can start
later without re-deriving either.

## Context (verified 2026-07-28)

- Codex CLI began stamping durable message ids
  (`payload.id = "msg_<uuidv7>"` on `response_item` records) around 0.146.0.
  Rollouts written by ≤0.141 have ids on zero durable records; 0.142–0.145
  are partial; both fail lineage proof.
- `analyze_codex_transcript` returns `Unknown(missing_message_id)` on the
  first id-less durable record, so `local_safely_capturable` is false and
  the push planner's `CloudMissing` arm yields `Blocked / CodecUnsupported`
  (conversation_sync.rs:143-149). Such sessions can never be backed up.
- The codec anticipated archival: "bounded reason codes so capture can
  safely offer StoredOnly" (conversation_codec.rs:102-104), and the pull
  direction already degrades to `StoreOnly` on codec failure. Only the push
  arm refuses.
- Scale on the diagnosed machine: 15 of 29 captured danci_nextjs
  conversations are permanently unsyncable under today's rules.

## Design sketch (validated against the current planner)

- Push arm change (conversation_sync.rs, `CloudMissing` + not
  `local_safely_capturable`): offer a selectable, **not**
  selected-by-default `StoreOnly` action instead of `Blocked`.
- Upload raw transcript bytes as a non-resumable thread: no conversation
  binding, no materialization, `lineage_codec` recorded with the codec's
  unknown-reason code so readers know why no chain exists.
- Re-push no-ops on unchanged bytes via the existing
  `expected_local_target` sha256 pin.
- UI: the row keeps its "can't sync (older Codex format)" fine print and
  gains an explicit opt-in "Archive to storage" affordance. Never included
  in select-all defaults.

## Open questions (settle before implementing)

- Archive growth policy: 0.142–0.145 sessions can still be resumed and
  grow. Ancestry against the stored archive is unprovable, so the options
  are replace-the-archive (destructive; requires explicit approval) or
  store-a-second-copy. Pick one; do not diff.
- Whether the bounded transcript limit (`MAX_CONVERSATION_FILE_BYTES`) is
  also the archive cap, and what the UI says when a legacy file exceeds it.

## Sequencing

- After the Push→`publish_schema6` wiring from
  PLAN_REMOVE_BACKWARD_COMPAT.md Tier 2 — stored-only threads and
  Requirement placeholders are schema-6 concepts.
- Independent of T1/T2 in PLAN_LEGACY_ROLLOUT_BLOCKED_THREADS.md (card
  fold and `ThreadVariantReview` removal); those can land first.

## Non-goals

- No legacy codec that reconstructs lineage by content or position
  matching — repeated identical messages and provider-side rewrites make
  any such alignment a guess, and a wrong guess corrupts a resumable
  transcript. `Unknown` remains the honest verdict for resume-grade sync.
- No relaxation of the id+fingerprint prefix proof for resumable
  fast-forwards.

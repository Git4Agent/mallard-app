# Plan: Legacy id-less rollouts — history-page noise and archival gap

Status: Done (2026-07-28). T1 + T2 landed as one change; T3 was decided
with no work. Diagnosed on a live project (danci_nextjs bound to
"Local storage 1"): the embedded history page showed a wall of "Blocked"
conversation cards for every pre-0.146 Codex session. Nothing here was a
planner bug — the codec was refusing exactly as designed — but three
follow-ups fell out of the diagnosis.

Line numbers are as of the uncommitted working tree on top of commit ab8b7d0.
The working checklist across this plan and its sibling docs is TODO.md.

## Diagnosis (verified, keep for context)

- Codex CLI started stamping durable ids (`payload.id = "msg_<uuidv7>"` on
  `response_item` records) around **0.146.0**. Verified against real rollouts
  in `~/.codex/sessions`: every file written by ≤ 0.141 has ids on **zero**
  durable records, 0.142–0.145 files are partial (some records stamped, some
  not — still failing), every 0.146.0-alpha.3.1 file has ids on all records.
- `analyze_codex_transcript` returns `Unknown { MissingMessageId }` on the
  first id-less durable record (conversation_codec.rs:143-150), so
  `lineage.is_complete()` is false → `local_safely_capturable: false`
  (commands.rs:4184, 4196).
- Push planner, relation `CloudMissing` + not capturable →
  `Blocked / CodecUnsupported` (conversation_sync.rs:143-149). The two UI
  strings on each card are that reason's label and `blocked_reason`.
- The history page renders these as `ThreadVariantReview` cards even in plain
  browse mode: every comparison entry gets
  `cloud_thread_id: Some(thread_id)` (commands.rs:4260), the page renders
  whenever `threadEntries.length > 0` (ProjectChatHistoryPage.tsx:713), and
  the component's only filter is `!!entry.cloud_thread_id`
  (ThreadVariantReview.tsx:88) with direction defaulting to `"push"`.
- Unbound entries default to
  `materialization_state = StoredOnly` (commands.rs:4240), so a purely local,
  never-stored thread renders the contradictory badge pair
  "Not yet published" + "Stored only".
- Verified against the real home (2026-07-28, `discover_full` +
  `capture_from_discovery` on `~/.codex` with the danci_nextjs root): all 14
  sessions written by 0.146.0-alpha.3.1 capture `Complete`; the 15 pre-0.146
  sessions capture `Unknown(missing_message_id)`. A healthy recent thread
  ("Test task", `019fa733…`, `Complete`, decision `CreateCloud`) renders the
  same "Not yet published" + "Stored only" badge pair as the blocked legacy
  rows — only the fine-print reason line differs — so healthy and permanently
  unsyncable threads are visually indistinguishable at a glance. T1 + T2
  together are the fix.

## Todo

### T1 — retire `ThreadVariantReview` as a separate list (user-confirmed
### direction, 2026-07-28: the existing thread cards are the review surface)

The review list is redundant even inside the Push dialog: a thread in the
30-day window renders twice with two checkboxes bound to the same
`resource_id` (ThreadVariantReview at ProjectChatHistoryPage.tsx:714 plus the
`renderThread` cards below it). The regular cards already show title, times,
`ThreadSyncIndicator`, a selection-mode review-state label, and open actions.

- [x] Fold the only unique content into the existing cards in selection
      mode: the review-state label already covered the action; the fold adds
      the "Review required" badge for `requires_explicit_approval` decisions
      and, for blocked rows, the skills-style red `octagon-x` marker in the
      checkbox gutter whose tooltip carries a plain-language reason
      (`syncHoldDetail`) — no extra visible text. Engine strings ("active
      codec", "provider representation") no longer render.
- [x] Give threads outside the 30-day history window a home in selection
      mode (collapsed "Older activity" group) — previously the review list
      was the only surface where they appeared at all.
- [x] Collapse non-actionable rows (`blocked`, `no_op`) behind a count in
      selection mode (nested inside "Older activity"; expandable, so they
      stay discoverable); render nothing extra in browse mode.
- [x] Delete `ThreadVariantReview` and its styles — done in the same change
      as T2's entry-level `materialization_state` / `resumable` plumbing
      removal (its only consumer died here). The stored-only card filter is
      now presence-based (`!local_present && storage_present`) instead of
      keying on `cloud_thread_id`, which had quietly become always-set and
      left storage-only threads with no card outside the review list.
- [x] Acceptance: browse mode shows history only (no decision cards); in
      Push/Pull mode each thread appears exactly once with one checkbox;
      out-of-window and blocked threads remain discoverable. Covered by the
      rewritten `tests/frontend/thread-variants.integration.test.tsx`.

### T2 — delete the entry-level `materialization_state` plumbing together
### with `ThreadVariantReview` (same change as T1's delete step)

User-confirmed direction (2026-07-28): the only consumers of
`ThreadSyncEntry.materialization_state` and `entry.resumable` are inside
`ThreadVariantReview` (badge at ThreadVariantReview.tsx:96-100, Resume gate
at :174; the ProjectSidebar "resumable setup draft" string is unrelated).
There is no reason to patch the `.unwrap_or(StoredOnly)` default or the
badge label as an interim step — the legacy plumbing is removed outright in
the same change that deletes the component.

- [x] In T1's "delete `ThreadVariantReview`" change, also removed
      `materialization_state` and `resumable` from `ThreadSyncEntry`:
      the struct, the comparison loop (including the
      `.unwrap_or(StoredOnly)` default and the materialization-derived
      `resumable` expression), the capture-unavailable builder, and the
      `ThreadSyncEntry` interface plus the `ConversationMaterializationState`
      alias in types.ts. No serde/back-compat shims — clean deletion.
- [x] `LocalConversationBinding.materialization_state` (domain.rs) is NOT
      legacy — the apply/verify engine runs on it
      (conversation_materialization.rs). It stayed; only the UI-payload
      copy died. (`MaterializationRecord` on `get_project` is the bundle
      apply-receipt surface, a different concept — untouched.)
- [x] The folded cards (T1) express openability presence-based: the Open
      in Codex / Terminal actions render for every local history card and
      gate only on a valid provider thread id. Codec verdicts never gate
      Open — blocked means "can't sync", never "can't open"; legacy
      pre-0.146 sessions stay openable. Compact rows speak location +
      actions ("On this Mac", "In <storage>", "Pull to open").
- [x] Updated entry-level expectations in `sync_tests/thread_variants`
      (two entry-level assertion blocks in continuation.rs; every other
      hit was binding- or journal-level and kept the field).
- [x] Acceptance: `ThreadSyncEntry` carries no materialization vocabulary;
      grepping src/ for "Stored only", "Resumable", "Not materialized"
      finds nothing (the lowercase "resumable setup draft" strings are the
      unrelated project-setup surface); every row answers open/push/pull
      at a glance; blocked legacy threads remain openable; genuine
      storage-only rows offer "Pull to open". The review-state labels for
      `blocked` and `store_only` now read "Can't sync" and "Backup only"
      (also in RestorePlanView's fallback list).

### T3 — legacy id-less rollouts stay Blocked (decided 2026-07-28)

Decision, confirmed by the user: keep today's behavior. A push of a
conversation whose lineage the active codec cannot prove remains
`Blocked / CodecUnsupported` (conversation_sync.rs:143-149) — no store-only
archival, no special-casing. The T1/T2 card work keeps these rows honest
("can't sync" fine print, still openable) but adds no new sync affordance.

- [x] Decide whether legacy sessions should be archivable → **no, for now**.
- [x] Record the decision here and in THREAD_VARIANT_SYNC_REQUIREMENTS.md
      (Push/Pull rules).

Future support is tracked separately in PLAN_LEGACY_ROLLOUT_ARCHIVAL.md
(status: deferred, not scheduled). The design notes that previously lived in
this section — the opt-in `StoreOnly` push arm, byte-hash no-op on re-push,
the archive-growth policy question, and the `publish_schema6` sequencing —
moved there so the work can start without re-deriving the diagnosis.

## Non-goals

- No legacy codec that reconstructs lineage for id-less rollouts by content
  or position matching — repeated identical messages and provider-side
  rewrites (compaction, `forked_from_id` forks) make any such alignment a
  guess, and a wrong guess corrupts a resumable transcript. `Unknown` stays
  the honest answer for resume-grade sync.
- No relaxation of the id+fingerprint prefix proof for resumable
  fast-forwards.

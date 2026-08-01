# TODO — thread sync UI and legacy rollouts

Combined checklist from the 2026-07-28 history-page diagnosis. Rationale,
file:line citations, and acceptance criteria live in the linked plans; this
file is the working index — check items off here as they land.

## Now — one combined change

From [PLAN_LEGACY_ROLLOUT_BLOCKED_THREADS.md](PLAN_LEGACY_ROLLOUT_BLOCKED_THREADS.md)
(T1 + T2, one commit: the plumbing's only consumer dies with the component).
**Done 2026-07-28** — landed as one working-tree change; verified by
`cargo test` (293), the frontend integration suite (101, incl. the rewritten
`thread-variants.integration.test.tsx`), and the acceptance greps.

- [x] Fold decision label + reason line + "Review required" badge into the
      existing thread cards in selection mode (T1)
- [x] Give out-of-window threads a collapsed "Older activity" group in
      selection mode (T1)
- [x] Collapse non-actionable rows (`blocked`, `no_op`) behind a count in
      selection mode; render no decision cards in browse mode (T1)
- [x] Delete `ThreadVariantReview` + styles (T1) and, in the same change,
      remove `materialization_state` / `resumable` from `ThreadSyncEntry` —
      struct, both construction sites, types.ts (T2)
- [x] Openability is presence-based: `can_open = local_present &&
      provider_thread_id.is_some()`; codec verdicts never gate Open (T2)
- [x] Rows speak location + actions only ("On this Mac", "In <storage>",
      "Pull to open"); no internal state names render (T2)
- [x] Update entry-level `sync_tests/thread_variants` expectations;
      binding-level assertions keep the field (T2)
- [x] Acceptance sweep: browse mode shows history only; each thread once,
      one checkbox; grep src/ for "Stored only" / "Resumable" /
      "Not materialized" finds nothing; blocked legacy threads openable

## Decided — no work

- [x] Legacy id-less rollouts stay `Blocked` for Push (T3, 2026-07-28) —
      rule recorded in
      [THREAD_VARIANT_SYNC_REQUIREMENTS.md](THREAD_VARIANT_SYNC_REQUIREMENTS.md)
      (Push/Pull rules)

## Deferred — future work

- [ ] **Sync subagent task** — Codex subagent and guardian rollouts are
      deliberately excluded from discovery, Push, comparison, and Pull for
      now. Supporting them safely requires an explicit parent/child task
      model, grouped (not standalone) review UI, atomic parent-plus-child
      selection, and complete ID remapping for `session_id`,
      `parent_thread_id`, nested
      `source.subagent.thread_spawn.parent_thread_id`, and the restored Codex
      agent graph/state. The review must also disclose that child JSONL can
      contain inherited context and tool output. Do not present or restore
      these rollouts as independent top-level conversations.
- [ ] Store-only archival for legacy id-less rollouts —
      [PLAN_LEGACY_ROLLOUT_ARCHIVAL.md](PLAN_LEGACY_ROLLOUT_ARCHIVAL.md)
      (not scheduled; sequence after the Push→`publish_schema6` wiring from
      [PLAN_REMOVE_BACKWARD_COMPAT.md](PLAN_REMOVE_BACKWARD_COMPAT.md)
      Tier 2)
- [ ] **Row-7 ancestry walk for retained-files force-resets** — the
      remote-history-continuity probe misreads a rewound-and-regrown
      repository as `Advanced` when the reset kept old files (`rsync`
      without `--delete` over the storage dir, or an S3 rollback of
      `_head`'s latest version only) and the new lineage overtook the
      base's generation. Degrades safely (pull is offered; pins and
      apply-time verification hold) but the diagnosis is wrong: no
      replaced-history warning, and purged content can be innocently
      re-pushed. Fix: row 7 requires *ancestry* via
      `CommitRecord.previous_commit_id`, not *presence*. Worked example in
      [DESIGN_REMOTE_HISTORY_CONTINUITY.md](DESIGN_REMOTE_HISTORY_CONTINUITY.md)
      (*Considered and deferred*)

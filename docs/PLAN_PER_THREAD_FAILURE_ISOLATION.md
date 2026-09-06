# Per-Thread Failure Isolation for Push and Pull

- Status: Implemented 2026-09-04 (see §10 for the deltas from the design)
- Date: 2026-09-04
- Scope: Project Sync conversation actions in Push and Pull
- Baseline: branch `extensible-providers`, `HEAD` a5a47a1 + working tree.
  Line numbers below are from `src-tauri/src/project_sync/service.rs` in
  this tree and will drift.

## 1. Decision

A failure whose inputs belong to one conversation fails that conversation
only. Every other selected conversation in the same Apply proceeds to
publication (Push) or materialization (Pull). The result names every failed
conversation with its message.

A failure whose inputs are shared still stops the whole Apply, and a stopped
Push publishes nothing.

The classification rule is mechanical:

| Class | Inputs | Blast radius |
| --- | --- | --- |
| Thread-caused | one thread's bytes, pins, mapping, stamps, or sidecars | that thread |
| Operation-caused | storage transport, the head compare-and-swap, config and binding loads, revisions moving mid-apply, provider availability, batch invariants | the whole Apply |

Pull already has this shape for its prepare and apply steps
(`PreparedPullConversationFailure`, `failed_conversation_receipt`,
"Threads settle independently" at 10842). Push has none of it. This plan gives
Push the same shape and closes the three whole-Pull holes that remain.

## 2. Where the blast radius is today

### 2.1 Push (`push_bundle_reviewed_with_progress`)

| Stage | Function | One bad thread today |
| --- | --- | --- |
| capture selected payloads | `provider_capture::capture_from_discovery_with_workers` | Phase 2 worker `Err` sets the fast-fail flag; the merge returns the earliest `Err`; whole Push stops |
| apply-time review | `thread_review(ReviewPurpose::PushPlan)` → `provider_thread_review` | a per-thread `Err` (`take_observation_outcome(..)?` at 4994, "reviewed without captured bytes" at 5260) becomes a provider `Err`; 8827 turns the first `provider_errors` entry into a whole-Push `Err` |
| build publish request | `build_publish_request` loop 7607–8180 | every `?` in the loop body returns before publication; this is the reported case (7787) |
| publish | `StorageEngine::publish_with_progress` | one request, one CAS; all-or-nothing by design |
| install | `apply_conversation_cloud_publications_locked` | per-operation snapshot install after the CAS; the documented crash trade-off |
| result | `ProjectOperationResult` | `success` is always `true`; `results` lists every manifest resource as `synced`/`unchanged`; there is no failure channel |

Thread-caused errors in the build loop, by arm:

- shared: source no longer capturable, lost transcript, lost sidecar; cloud
  mapping changed after review; already mapped; minted id collision; lost
  descriptor; checkpoint advanced after review; `redaction_ruleset_mismatch`;
  `path_map_codec_mismatch`; provider session id missing; sidecar
  certification.
- Codex: path-map root grammar; no provider home; source resolution; bounded
  read; root seeding; changed after review; prefix witness mismatch;
  standalone encoding and redaction.
- Claude: path-map root grammar; record boundary; changed after review;
  mapper construction; UTF-8; per-record encoding; bounded limit after path
  encoding; redaction; per-record wire self-check; certification;
  `redaction_gate_failed` (boundary, layout, re-analysis).

Two checks in the loop are batch invariants, not thread failures: an action
that targets an unbound provider scope (7609) and a cloud id resolved twice
in one batch (7679).

The loop also mutates the scope before its last fallible step: the thread row
and transcript are inserted at 7951–7981, and sidecar certification can still
fail at 8001. Today that is harmless because any `Err` abandons the whole
request. Once the loop continues past a failure it is not harmless, so the
refactor in §4.3 reorders build and commit.

One accounting hole is independent of errors: the frontend derives the
"pushed" rows of the outcome from the Compare-time comparison
(`buildPushOutcome`, `syncOutcome.ts:85`). A thread the apply-time review
holds (blocked, no action built) is reported as synced. §4.4 closes it.

### 2.2 Pull (`apply_bundle_restore_with_progress`)

| Stage | Function | One bad thread today |
| --- | --- | --- |
| build apply plan | `build_restore_plan_for_apply` | `capture_from_discovery(local selection)?` (9200) and `thread_review(PullPlan)?` plus the `provider_errors` abort (9224) stop the whole Pull |
| align selections | `align_pull_apply_selections` | an approved row whose apply-time action is missing returns "conversation '…' changed after comparison" (10681); whole Pull stops |
| prepare | `prepare_pull_conversation_actions` | `bytes.prefetch(..)?` stops the whole Pull (deliberate: DESIGN_FAST_PUSH_PULL "Prefetch partial failure"); per-action `prepare_one` is isolated |
| apply | `apply_prepared_pull_conversation` | isolated; failure receipt per thread |
| metadata adoption | `adopt_cloud_metadata_actions` | one transaction; a failure writes the same receipt on every metadata action (documented) |
| files | `apply_restore_plan_lazy` | per-action receipts |
| result | `RestoreResult` | `success` = no `Failed`/`Blocked` receipt; `failed_actions`; "Applied N actions; M require attention" |

Against `HEAD`: the two `provider_errors.into_iter().next()` abort sites are
new in the working tree; the per-thread prepare/apply isolation is in `HEAD`.

## 3. Principles

- **P1 Classification.** Apply the table in §1 to every `?` on the path. No
  new category.
- **P2 Same shape as Pull.** Push reports failures the way Pull already does:
  one row per failed action with its message, `success = false` when any
  thread failed, a count in the message. No new persistence.
- **P3 Build, then commit.** In Push, every fallible step for a thread
  finishes before any shared publish state is touched. A failed thread leaves
  its scope exactly as the previous manifest had it.
- **P4 Identity travels with the failure.** Each failure carries the review's
  capture resource id (`source_resource_id` / `conversation_action_row_key`),
  so the UI shows the title the review showed.
- **P5 Fail closed per thread.** No check becomes a warning. Every check that
  fails today still fails; only its blast radius changes.
- **P6 Stateless.** No retry, no rounds, no partial-progress state, per
  PLAN_COMPARE_ONLY_APPLY_TIME_CAPTURE. The outcome says "not pushed" and the
  next Push starts from a new Compare.

## 4. Design by stage

### 4.1 Capture (shared by Push and Pull)

`provider_capture.rs`:

- `CapturedResources` gains `failed_conversations: BTreeMap<String, String>`
  (capture resource id → message).
- Phase 2: a worker `Err` for a resource whose kind primitive is
  `Conversation` no longer sets the fast-fail flag. Phase 3 merge inserts it
  into `failed_conversations` and skips the unit. Any other kind (provider
  state, tools, project content) keeps today's behavior: fast-fail and whole
  capture `Err`. Those resources are the bundle's requirements; a Push cannot
  proceed without them.
- Phase 1 preflight stays whole-capture. It is metadata-only and every error
  there is a cross-resource invariant: case-fold collisions, logical-path
  ownership, the aggregate size cap.
- Merge order and the earliest-failure rule are unchanged for the fatal
  kinds.

Thread-caused errors this covers: `fs::read` failure, sidecar redaction
failure, `detect_secret_material` hard error in a non-opaque file.

### 4.2 Apply-time review (`provider_thread_review`)

The review is one function for Compare, Push apply, and Pull apply, so a
per-thread failure has to become a blocked row rather than a provider error.

Phase 1 (capture failures):

- At 4433, where the local row looks up its captured bytes, a resource in
  `failed_conversations` yields `LocalThreadReview { captured: None,
  capture_failure: Some(message) }`.
- The row's push and pull decisions are blocked with a new
  `ConversationBlocker::CaptureFailed` and `blocked_reason = message`. No
  action is built. The 5260 invariant ("reviewed without captured bytes")
  stays as the guard for rows with no recorded failure.

Phase 2 (observation failures):

- The observation pool (4935–5000) drops its fast-fail flag. `Err` outcomes
  are collected per thread instead of propagated by `?` at 4994.
- A thread with an observation error is blocked with
  `ConversationBlocker::ObservationFailed` and the error as
  `blocked_reason`. For Codex, a thread whose `depends_on` names a failed
  thread is blocked with "depends on <thread>, whose observation failed".
- Provider-level `Err`s stay provider-level: binding changed (4387), two
  local sessions resolving to one logical thread (4519), a cloud thread
  claimed twice (4753), session index expectation, adapter or home missing
  in `thread_review`.

Phase 2 also changes Compare's face: a provider with one unreadable rollout
shows its rows with one blocked row instead of disappearing behind a
`provider_errors` banner. That is the intended behavior, and it is why Phase
2 is separated: it touches the review that every screen reads.

### 4.3 Push build (`build_publish_request`)

Split the loop body:

```text
build_conversation_publication(ctx, action) -> Result<ConversationPublication, String>
commit_publication(scope, publication)                 // infallible
```

`ConversationPublication` holds everything the loop currently writes into
the scope after its checks: the resolved cloud id, the thread row,
descriptor, logical path, the published transcript bytes or the retained
entry, the certified sidecar files, the checkpoint, and the
`ConversationApplyOperation`. The builder reads `scope.conversations` for
`previous_thread`, consumes the captured bytes, uses the shared
`redaction_run`, and mints the cloud id. It writes nothing.

The loop becomes:

```text
for action in actions:
    scope = writable scope for action.provider        // batch invariant: stays fatal
    match build_conversation_publication(..):
        Ok(p)  => duplicate-cloud-id check (batch invariant: stays fatal)
                  commit_publication(scope, p); operations.push; planned_checkpoints.insert
        Err(m) => failed.push(PushThreadFailure { action_id, resource_id, thread_id, provider, message: m })
```

What a failed thread leaves behind:

- Its previous cloud row and files are carried untouched, because every
  scope starts from `previous_scope.conversations` and their retained files
  (7563–7583, 7692).
- Its tombstone, if any, stays.
- Its checkpoint does not move; no operation, no receipt.
- Its captured bytes were taken and are dropped.
- Its capture resource id stays in the recipe stored on the link, and that
  recipe seeds the next review's recommended selection
  (`ProjectSync.tsx` `savedRecipe` → `recommendedPushSelection`), so the
  thread opens ticked next time.

Pushing it later needs nothing from this push: the next Compare classifies
the thread from its unmoved checkpoint against the carried cloud row, offers
the same action, and the next Push publishes it as a new revision. The link's
scope base advanced with this push, so the partial publish itself never
raises a "pull first" gate.

Checkpoint chaining and `validate_cloud_publication_binding_owners` run over
built operations only. `build_publish_request` returns the failures alongside
the request and operations; `publish_reviewed` passes them up.

If no operation was built, `publish_reviewed` takes its existing
`operations.is_empty()` branch: a direct publish, which the engine reports as
`published: false` when no scope content changed, or as a real revision when
generic content (provider state, tools, project files) did.

### 4.4 Accounting rule (Push)

Every selected conversation resource ends in exactly one state after
`publish_reviewed` returns:

| State | Source |
| --- | --- |
| published | has an operation |
| held | selected, no operation, and the apply-time review blocked it with a pull-class blocker (`DetachedRequiresPull`) |
| failed | selected, no operation, any other blocker (`CaptureFailed`, `ObservationFailed`, `UnknownEvidence`, `CodecUnsupported`, `RedactionRulesetMismatch`, …) or a build failure |

The backend computes `selected_conversations − operations' source ids` and
resolves each remainder against `review.entries` by `resource_id`; the entry's
`push_decision` supplies the blocker and reason. This replaces the frontend's
Compare-time derivation and closes the hole in §2.1.

### 4.5 Publish and install

Unchanged. One request, one CAS. Operations exist only for built threads. A
failure after the CAS is the existing "head advanced, local metadata write
failed" trade-off.

### 4.6 Push result and UI

Wire (`ProjectOperationResult`):

- `results` gains one row per non-published selected thread:
  `{ resource_id: <capture resource id>, state: "failed" | "held", message }`.
  Existing rows keep their `synced`/`unchanged` states.
- `success` = no `failed` row. Held rows do not fail the Push, matching the
  outcome's existing held bucket.
- `message`: "Published revision N", "Published revision N — M sessions not
  pushed", or "Nothing published — M sessions failed".
- `resources_changed` counts `synced` rows only.

No new typed list. The capture resource id is the key the frontend already
uses to join outcome rows to the comparison it holds, which is where titles
and thread ids come from.

Frontend (`syncOutcome.ts`, `ProjectSync.tsx`):

- `buildPushOutcome` takes the held and failed ids from `result.results`;
  pushed = selected minus both. Rows: synced, held (with the reason), failed
  (with the message). Title "Pushed X of Y sessions"; tone warning;
  `defaultOpen` when any row failed; "Open Pull review" when any row is held.
- The review closes on any `Ok`, as today. The `Err` path is unchanged,
  including the destination-status probe for a moved head.

Activity log (`commands.rs` 1140–1165): a `push.resource_failed` line at
error level per failed row, carrying the message; `push.completed` keeps
`operation.message`.

`mallard-cli` does not consume either result shape.

### 4.7 Pull

1. **Capture failure.** §4.1 records it; the PullPlan review (§4.2) blocks
   the row; `prepare_pull_conversation_actions` additionally checks
   `failed_conversations` for the action's local resource and fails that
   action with the capture message, so the message survives even when the
   action was built before the check.
2. **Align.** `align_pull_apply_selections` returns the approved conversation
   rows it could not match instead of failing: `(ActionId, ResourceId,
   reason)`, where the reason is the row's apply-time `blocked_reason` when
   the row exists with a blocker, else "changed after comparison". The
   orchestrator writes a failure receipt for each. `failed_conversation_receipt`
   gets a variant that takes the id pair, since there is no action to pass.
   Duplicate and unknown approved ids stay fatal: those are selection
   invariants, not thread state.
3. **Prefetch.** Unchanged and deliberately whole: it is one batched transfer,
   and a transport failure would otherwise print the same error on every row.
   Object-level problems still surface per thread at `read_bounded`.
4. **Provider errors** at 9224: same rule as Push (D2).

## 5. What stays fatal

| Failure | Why it is operation-caused |
| --- | --- |
| storage transport, the CAS, a moved head, a moved scope pin | shared with every thread |
| config and bindings loads; revisions moving mid-apply; host identity; project binding | shared state |
| provider adapter, home, or session index unavailable; duplicate claims | provider or document invariants |
| unbound provider scope; cloud id resolved twice; inconsistent plan pins | batch invariants |
| capture preflight: path collisions, ownership, aggregate cap | cross-resource |
| project-content (files) review verification | separate subsystem, untouched |
| Catalog Push (`StorageCatalogPage`) | separate flow over resources, untouched |
| local install after the CAS | existing crash trade-off |

## 6. Decisions to confirm

- **D1 (recommended): failures travel in `results`.** One representation for
  the UI and the activity log; the frontend already joins by resource id.
  Alternative: a typed `failed_conversations` list mirroring Pull's
  `FailedAction`. Rejected as a second shape for the same fact.
- **D2 (recommended): provider-level review errors keep stopping the whole
  Apply.** Alternative: drop the provider's actions and report all its
  selected threads as failed. Rejected because the provider's scope would
  still publish its non-conversation content while every conversation
  "failed", and the fix (adapter, home, binding) is environmental.
- **D3 (recommended): a Push that publishes nothing returns `Ok` with
  `success = false` and the failed rows.** The review closes and the outcome
  lists the rows, the same face as one failure among a hundred. Alternative:
  return `Err` so the review stays open. Rejected: the review is stale after
  any Apply, and "push again" is the only next step either way.
- **D4 (recommended): two phases, each a clean cutover.** Phase 1 = §4.1,
  §4.3, §4.4, §4.6, §4.7 (items 1, 2); it covers the reported class (build
  failures) and capture failures. Phase 2 = §4.2 observation isolation,
  which also changes Compare's face.

## 7. Performance cost

Reference run: PUSH_SLOWNESS_2026-09-04 (37 threads, 136 MB, 29.3 s wall,
dev profile): capture 2.18 s, build 23.36 s, publish 0.20 s, receipts 1.43 s.

| Path | Today | With isolation |
| --- | --- | --- |
| all threads succeed | full run | full run; no extra I/O, hashing, or redaction |
| one thread fails at build | stops at that thread; the user re-runs after fixing | full run minus that thread's remaining work; the other threads publish |
| all threads fail at build | stops at the first | all build work is spent, nothing is published; bounded by one full push |
| one thread fails at capture | stops at that thread | the other reads continue; same memory ceiling (512 MiB cap) |
| Pull align failure | stops | O(approved) bookkeeping, no new I/O |

Success path: +0. The refactor moves bytes into `ConversationPublication`
instead of cloning them; today each transcript is cloned twice on its way
into `captured_files` (7900, 7973), so the split removes roughly 270 MB of
memcpy in the reference run, well under 0.1 s. Result rows are O(failed).

Failure path: wall time rises from "until the first failure" to "one full
push", which is the work the user would otherwise pay again on the re-run.
Net across the workflow: one run instead of two.

Memory: unchanged. A failed thread's captured bytes are dropped at the point
of failure instead of at abort.

The redaction 3–4× problem in the slowness report is orthogonal. The
build/commit split is the natural seam for that fix later (one certification
per thread), but this plan does not touch it.

## 8. Tests

Rust (`service.rs`):

- Push, two threads, one fails at build, the other publishes: `Ok`,
  `success = false`, one `failed` row with the message, the good thread's
  cloud row at the new revision, the failed thread's checkpoint generation
  unchanged, and a second Push after the fix publishes it. Deterministic
  triggers: a torn Claude tail (record boundary), a non-opaque sidecar with
  a secret (capture stage), an unreadable transcript (capture stage), a
  `redaction_ruleset_mismatch` stamp on one existing cloud row.
- Push, every selected thread fails: `Ok`, `success = false`,
  `published = false`, revision unchanged, "Nothing published".
- Push, batch invariant (unbound provider scope) still returns `Err`.
- Pull, one approved row changed after comparison: a failure receipt for it,
  the others applied, `success = false`.
- `provider_capture`: a conversation worker error lands in
  `failed_conversations` and does not stop the pool; a provider-state worker
  error still fails the capture.

Existing tests: the four Push tests that `unwrap_err` (git head, review
token, machine-local recipe) are operation-level and stay. No existing test
asserts a thread-caused message from a whole-batch `Err`.

Frontend: `syncOutcome` (failed and held rows, title, tone),
`project-sync-cutover.integration` (outcome with failures),
`activity-log.integration` (`push.resource_failed`).

## 9. Files (Phase 1)

- `crates/mallard-infrastructure/src/provider_capture.rs` — `failed_conversations`, pool flag, merge.
- `crates/mallard-domain/src/lib.rs` — `ConversationBlocker::CaptureFailed` (Phase 2 adds `ObservationFailed`).
- `src-tauri/src/project_sync/service.rs` — `build_publish_request` split; `publish_reviewed` returns failures; `push_bundle_reviewed_with_progress` accounting and result; `provider_thread_review` capture-failure row; `align_pull_apply_selections`; `prepare_pull_conversation_actions` capture check; `failed_conversation_receipt` id-pair variant.
- `src-tauri/src/project_sync/commands.rs` — `push.resource_failed`.
- `src/types.ts`, `src/components/project-sync/syncOutcome.ts`, `ProjectSync.tsx` — outcome rows.

## 10. Implementation notes (2026-09-04)

Landed as designed with these deltas:

- **Capture failures never reach the review as a special row.** The
  `CaptureFailed` blocker from §4.2 was not added. `CapturedResources`
  carries `failed_conversations`, the capture pool no longer fast-fails on a
  conversation's error, and the two consumers read the map directly: the
  Push builder (`take_captured_conversation_for_action`) and Pull prepare
  fail that thread with the capture's own message. Cursor providers review
  from disk, so the review needs nothing from the capture to classify.
- **Observation failures (§4.2, Phase 2).** `ConversationBlocker::
  ObservationFailed` exists, the review pool no longer fast-fails, and a
  thread's observation error becomes its own blocked row instead of a
  provider error. Every realistic failure a fixture can produce (unreadable
  source, unresolvable history base, truncated file, over-declared base
  ordinal) already degrades to Invalid evidence under the standing D3a rule
  and reaches the row as `UnknownEvidence`; the `ObservationFailed` arm
  therefore covers the cursor and seeding internal-inconsistency errors
  only. The Codex test pins the per-row rule under the new pool.
- **Accounting (§4.4).** `conversation_push_results` implements the table:
  build failures first, then every selected resource without an action
  resolved against the apply-time review entries. `NoChanges` rows add
  nothing; `RequirePull`, `RequirePullMetadata`, and a
  `DetachedRequiresPull` blocker report as `held`; every other blocked
  decision reports as `failed` with the review's reason. A selected thread
  the apply-time review blocks is now reported, where before it was absent
  from the result and shown as pushed.
- **Pull (§4.7).** `align_pull_apply_selections` returns the approved rows
  it could not match; the orchestrator removes them from the approved set
  and writes a failure receipt per row through
  `failed_conversation_receipt_for`. Prefetch stays whole, as designed.
- **Frontend.** `buildPushOutcome` takes held and failed rows from
  `result.results`, keeps the Compare-time holds for rows the backend did
  not report, and titles the outcome "Pushed X of Y sessions" with a
  "not pushed" stat. The activity log gains `push.resource_failed` (error)
  and `push.resource_held` (warning) lines; `push.completed` logs at
  warning level when the Push did not succeed.

Tests: `push_isolates_a_thread_that_fails_to_build` (torn Claude tail:
builder failure, second Push publishes it, third Push with only an
unpublishable change publishes nothing),
`pull_reports_an_approved_row_the_apply_time_review_no_longer_offers`
(two-machine Claude flow, one approved row diverges before Apply),
`compare_blocks_only_the_thread_whose_rollout_cannot_be_resolved`,
`conversation_capture_failure_is_isolated_per_resource`, and the frontend
`push-outcome.integration.test.tsx`. The existing witness test
(`a_prefix_mutated_paginated_thread_is_blocked_by_the_witness_at_publish`)
was retargeted from a whole-Push error to a reported per-thread failure.

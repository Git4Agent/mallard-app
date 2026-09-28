# Pull carries the same 512 MB limit as Push

- Status: implemented 2026-09-26 (see "What landed")
- Date: 2026-09-26
- Scope: the project Pull review and Pull apply. Rust, the IPC DTOs, the
  frontend review, and the meter's CSS. The Library's own Pull is out of
  scope.

## Lineage

Push has carried one aggregate limit since the 2026-09-01 capture-budget
cutover: `MAX_CAPTURE_BYTES` (512 MiB, printed `512 MB`). After the 2026-09-03
compare-only cutover the limit is enforced twice — the review meter refuses a
selection that does not fit, and Push Apply recomputes the total before it
captures — and the Push review opens with the newest sessions that fit ticked.
Pull got none of this: its review has no meter, no refusal, no size on a row,
and its Apply installs whatever was ticked.

This plan gives Pull the same rule, the same meter, the same refusal and the
same recheck, with the one difference the direction forces: a Pull measures
what it installs from the storage, a Push what it captures from this
computer.

## What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | A Pull review carries no limit and no per-row size. `ConversationReviewRow` has no size field, and `PullReview` has no budget. The only sized rows are file writes, whose `RestoreAction.source_size` feeds the files step's readout and nothing else. | `crates/mallard-domain/src/lib.rs:3608`, `:5240`, `pull_review.rs:11` |
| 2 | The review never refuses on size. `primaryDisabledReason` knows one reason, "Loading storage changes…". The path bar's Pull reads the same reason through `PullReviewPrimary`, so both buttons already share one rule — there is simply no size rule to share. | `RestorePlanView.tsx:273`, `ProjectSync.tsx:3289` |
| 3 | Pull Apply has no aggregate recheck. It caps each conversation on its own (transcript plus child rollouts ≤ 512 MiB, each file ≤ the thread size limit) and nothing across the selection. | `service.rs:11397-11405` |
| 4 | The one aggregate cap a Pull does hit is accidental: capturing the *local* counterparts of the selected conversations goes through `capture_from_discovery`, which refuses past 512 MiB with the raw text `selected resources exceed 536870912 bytes` and fails the whole Apply. Storage-only sessions have no local counterpart, so a Pull of any number of new sessions is never capped at all. | `service.rs:10997`, `provider_capture.rs:709` |
| 5 | ✓ on the sessions step ticks every safe row. With a limit in place it would produce a refused selection the way the Push ✓ did before the newest-first walk. | `RestorePlanView.tsx:317`, `pullReviewModel.ts:278` |
| 6 | The manifest already knows every stored size: each thread lists its files, and every listed file resolves to a `BundleFileEntry` with `size` (validation rejects a manifest where one does not). Nothing has to be downloaded to price a Pull. | `lib.rs:2751`, `:3899`, `:4285` |

## The rule

- **R1. One limit.** Pull uses `MAX_CAPTURE_BYTES`, the constant Push uses.
  There is no second number; the constant's comment gains the sentence that
  it is the most one Apply moves in either direction.
- **R2. What a Pull weighs.** The pull size of a selection is the sum of
  - every selected session's stored bytes: the sizes of every file the
    manifest lists for that thread (transcript, sidecars, child transcripts,
    provider index fragment); and
  - every selected file action's `source_size` (agent setup & tools files,
    project files).
  Nothing is reserved. Unlike a Push, a Pull carries nothing it does not show
  as a row. Local counterparts are not counted: they are already on this
  computer.
- **R3. The review refuses.** When the pull size exceeds the limit, both
  buttons that apply the review — the footer primary and the path bar's Pull
  — are disabled with the same sentence as their title:
  `N over the 512 MB pull limit — exclude sessions or files to pull.` The
  rule lives in `RestorePlanView` and reaches the bar through
  `PullReviewPrimary.disabledReason`, as the loading reason does today. An
  empty selection is still the acknowledge-only apply ("Mark as reviewed").
- **R4. Apply rechecks.** Before Pull Apply writes anything — before the
  metadata adoptions, which are its first write — it sums the approved
  selection from the pinned manifest and refuses the whole Apply with the
  sentence of R3 when the sum exceeds the limit. The version is pinned, so
  the backend's figure equals the review's; the recheck guards a selection
  that did not come from this review. The per-conversation caps of fact 3
  stay.
- **R5. ✓ fits.** On the sessions step, ✓ ticks the safe agent setup & tools
  rows, then the safe sessions in the review's order — newest stored change
  first — until the pull is full, charging what is already ticked on the
  files step. The same walk Push uses (see D3 for which variant). On the
  files step ✓ stays "every safe addition"; the meter refuses if they do not
  fit, as on Push.
- **R6. Every row shows its size.** Sessions in the review list carry the
  `drive` readout Push rows carry, titled "Pull size", read from the
  comparison entry the activity page already holds. The fallback item list
  prints the same number in its copy line. File rows already print theirs.

Sizes are manifest facts, so the number on a row, the number the meter sums
and the number Apply rechecks are the same number read three times.

## What the user sees

Opening a Pull review on a storage with a large backlog: the header carries
the glance `0 B / 512 MB` beside "Pull review · Version N"; the footer carries
the gauge pill against the primary, which reads "Mark as reviewed". Each
session row shows its size next to its metrics. Ticking rows fills the
track in the cloud ink; ✓ ticks the newest sessions that fit in one block at
the top of the list. Past the limit both figures turn to the overshoot, the
track grows the limit tick, the refusal takes its own footer row and the
Pull button refuses with that sentence. After a Pull the activity page shows
the remaining sessions as storage-outstanding and the next review carries
the next block. No rounds, no count of future pulls, nothing stored between
them.

## Where

### Rust

- `crates/mallard-domain/src/lib.rs`
  - `ConversationReviewRow.source_size: Option<u64>` — the storage-side
    bytes a pull of this row installs; `None` for rows whose action writes
    no payload (`adopt_cloud_metadata`, the report rows). The name is the
    one `RestoreAction` already uses for the same fact in the same DTO.
    `#[serde(default, skip_serializing_if)]`, no validation beyond serde.
  - `PullReview.apply_limit_bytes: u64`. `TryFrom<RestorePlan>` becomes
    `PullReview::from_plan(plan, apply_limit_bytes)`; `From<PullReview> for
    RestorePlan` drops it. The domain does not learn the constant; the
    command passes it. `RestorePlan` is unchanged.
- `src-tauri/src/project_sync/service.rs`
  - `ThreadSyncEntry.pull_bytes: Option<u64>` beside `capture_bytes`
    (`:741`): what a pull of this row's stored conversation installs;
    `None` for local-only rows. Set in `thread_review` where the entry is
    assembled (`:6480`, the cloud `thread` is in hand) and on the view-only
    storage rows (`:7607`). One comparison carries both directions' sizes;
    the Push review ignores `pull_bytes` as the Pull review ignores
    `capture_bytes`.
  - `stored_conversation_bytes(&BundleManifest, &ConversationThread) -> u64`:
    the sum of `find_file(path).size` over `thread.files`. Pure, beside the
    helpers `conversation_sidecar_refs` sits with (`:6974`).
  - `plan_bundle_restore_with_progress` (`:10864-10905`): copy
    `entry.pull_bytes` into `ConversationReviewRow.source_size` when the
    row's action installs bytes. The rows built for `ReviewPurpose::PullPlan`
    (`:3480`) set it the same way.
  - `apply_bundle_restore_with_progress` (`:12753`): after
    `split_approved_restore_actions` and the project-file eligibility check,
    before `adopt_cloud_metadata_actions`, one call
    `pull_bytes_of_selection(&snapshot.manifest, &plan, &approved,
    &approved_conversation_actions)` summing approved file actions'
    `source_size` and the materializing conversation actions' threads through
    `stored_conversation_bytes`; over the limit, return
    `Err(pull_limit_sentence(total))`. The sentence is built with
    `provider_capture::format_bytes` so it matches the frontend's.
  - `commands.rs:1487`: `PullReview::from_plan(plan,
    provider_capture::MAX_CAPTURE_BYTES)`.
- Not `session_watch_service.rs`: `pull_bytes` is not a `LocalSessionView`
  field, so the dirty-root refresh has nothing to carry.

### DTOs (`src/types.ts`)

`ThreadSyncEntry.pull_bytes?`, `ConversationReviewRow.source_size?`,
`PullReview.apply_limit_bytes`. IPC-only; no machine-local or bundle schema
changes.

### Frontend

- `src/components/sync-review/ReviewBudgetMeter.tsx` (new): the meter
  Push draws today, moved to the resource-neutral layer because two reviews
  now draw it. `ReviewBudget { total, limit }`,
  `reviewBudgetSentence(budget, verb)`, `ReviewBudgetGlance`,
  `ReviewBudgetGauge`, `ReviewBudgetTrack`, `ReviewBudgetFigure`, all taking
  `{ budget, verb: "push" | "pull", breakdown }`. The verb picks the
  sentence ("push limit … to push" / "pull limit … to pull"), the figure's
  label ("Push size" / "Pull size") and the fill ink. It imports `formatBytes`
  from `src/format.ts` and nothing from either feature. The private copies in
  `PushResourceWorkspace.tsx:429-534` are deleted; `pushBudgetTotal`,
  `pushReviewDisabledReason` and `recommendedPushSelection` stay where they
  are and pass their totals to the shared components.
- `src/components/project-sync/pullReviewModel.ts`
  - `PullReviewItem.pullBytes: number | null`: the item's restore actions'
    `source_size` plus its conversation rows' `source_size`.
  - `pullBudgetTotal(items, selected, limit)` →
    `{ sessions, other, projectFiles, total, limit }`, the Pull twin of
    `pushBudgetTotal`. Breakdown: "Sessions · Setup & tools · Project
    files".
  - `fitPullRecommendation(plan, items, selected, stepItems, limit)`: the
    walk of R5. `recommendedPullSelection` stays as the per-item safety
    predicate the walk admits from.
- `src/components/project-sync/RestorePlanView.tsx`
  - `budget = pullBudgetTotal(items, selected, plan.apply_limit_bytes)`.
  - `primaryDisabledReason`: loading first, then the over-limit sentence.
    Both buttons follow through `onPrimaryChange` as today.
  - Header: the glance between the heading and the selection bar, as in
    Push (`PushResourceWorkspace.tsx:859`). Footer: the refusal row first
    when over, the gauge flush against the primary.
  - ✓ on the sessions step uses the fitted set for `recommendedIds`,
    `pendingRecommendedCount` and `stepMatchesRecommended`; title "Select
    the newest sessions that fit this pull (N)".
  - `ItemList` conversation rows print `formatBytes(item.pullBytes)` after
    the provider name.
- `src/components/project-sync/ProjectActivityPage.tsx`: `SessionMetrics`'s
  `captureBytes` prop becomes `sizeReadout: { bytes, label } | null`; the
  rail row (`:1405`) and the stored-thread row (`:1526`) pass
  `capture_bytes` / "Push size" in push mode and `pull_bytes` / "Pull size"
  in pull mode.
- `src/App.css:7040-7185`: the block is renamed `.sync-review-budget-*`
  (`-glance`, `-track`, `-figure`, `-gauge`, `-fill`, `-sentence`) — the
  push name would be a lie under the Pull footer — and gains one modifier:
  `.sync-review-budget-track.pull > i` and `.sync-review-budget-gauge.pull
  .sync-review-budget-fill` paint `--thread-status-cloud`. The
  `flex-wrap: wrap` that `.sync-push-resource-footer` carries for the
  refusal row moves to `.sync-sync-review-footer`, which both footers wear.
  The comment block's contrast facts stay; the pull ink's are added from the
  table below.

### The pull ink, measured

`--thread-status-cloud` resolves to `--info-text`: `#256c90` on light,
`#8ccff3` on dark. Measured with the css-hygiene gate's arithmetic, the
track composited on `--surface-card`, the gauge wash at the shipped 34%:

| | light | dark | floor |
|---|---|---|---|
| fill on `--loader-track` | 4.00 | 4.76 | 3.0 (1.4.11) |
| fill on `--surface-card` | 5.69 | 8.78 | 3.0 |
| `--text-1` figure on the 34% wash | 6.24 | 4.65 | 4.5 |
| `--text-0` figure on the 34% wash | 8.68 | 6.05 | 4.5 |

For reference the local ink measures 4.31 / 5.04 on the track, which is the
figure the Push meter's comment records. The pull meter can wear the cloud
ink in both themes; nothing in the palette moves.

## Decisions

- **D1. What counts.** Recommendation: R2 — stored bytes of every file the
  manifest lists for the session, plus selected file actions. Alternative:
  transcript only, which under-counts a session with sidecars or children
  and makes the row number disagree with what the Pull transfers. Not
  counting local counterparts is deliberate (fact 4 explains the one edge it
  leaves; see Not doing).
- **D2. The order ✓ walks.** Recommendation: the review's own row order.
  `thread_review` sorts entries newest first by the later of the local and
  stored update times (`service.rs:7618-7632`), and `plan_bundle_restore`
  keeps that order, so the rows arrive as the list shows them; a Rust test
  pins it. Alternative: a `stored_updated_at` field on the row and an
  explicit sort in the frontend — one more DTO field for the same walk.
- **D3. Strict prefix or first-fit.** The 2026-09-01 decision for Push was a
  strict time prefix (stop at the first session that does not fit). The walk
  that shipped in `recommendedPushSelection` (`PushResourceWorkspace.tsx:289`)
  skips any session that does not fit and keeps going — first-fit. The Pull
  walk must use the same rule as Push, so this plan asks which: keep
  first-fit in both, or return Push to the strict prefix in the same change
  (one branch in the walk: a session that could not fit an empty pull is
  skipped, any other miss stops the walk). Recommendation: strict in both,
  as decided.
- **D4. The pull meter's ink.** Resolved by measurement above: the cloud
  ink, so the meter reads as the direction every other status mark reads.

## Not doing

- **Counting local counterparts.** A keep-both row captures its local
  transcript too; a Pull whose selected local sides exceed 512 MiB still
  fails whole on the raw sentence of fact 4. It needs a diverged backlog of
  over half a gigabyte on this computer alone, and a rule that summed both
  sides would show a number no row explains.
- **A banner, chips or holds.** Every row stays selectable; the meter
  refuses. The count and the meter say what this pull carries.
- **A recommendation from the backend.** The frontend holds every input.
- **The Library's Pull.** It has its own review and its own disable rule
  (`StorageCatalogPage.tsx:1407`) and no size on its items.
- **Re-fitting when the selection changes.** ✓ is computed when pressed, as
  Push computes its default at open.

## Tests

Rust (`cargo test --manifest-path src-tauri/Cargo.toml --lib`):

1. `stored_conversation_bytes` sums transcript, sidecar and child entries of
   a hand-built manifest and nothing else.
2. `thread_review` sets `pull_bytes` on every storage-present entry and
   leaves it absent on a local-only one; `capture_bytes` is unchanged.
3. `plan_bundle_restore` rows carry `source_size` exactly when the action
   installs bytes, and the review's rows are in the comparison's newest-first
   order (D2).
4. `pull_bytes_of_selection` over a synthetic manifest: file actions and
   materializing conversation actions are summed, metadata adoptions are
   not, and the sentence names the overshoot and the limit. No
   half-gigabyte fixture: the sum is a pure function and the limit a
   constant.
5. `PullReview::from_plan` round-trips `apply_limit_bytes` and the row size;
   the existing `pull_review.rs` fixtures gain the field.

Frontend (`tests/frontend/pull-review.integration.test.tsx`, plus the
renamed selectors in `push-budget.integration.test.tsx`):

6. The meter is drawn in the header and the footer with the pull verb and
   the three-part breakdown; only the footer instance is a live region.
7. A selection over the limit disables the footer primary with the sentence
   as its title and reports the same `disabledReason` through
   `onPrimaryChange`; "Mark as reviewed" at zero is unchanged.
8. ✓ on the sessions step ticks setup & tools first, then sessions in row
   order until full, charging ticked project files; a session that could
   not fit an empty pull is skipped; the files-step ✓ is unchanged.
9. `pullBudgetTotal` sums sessions, setup & tools and project files, and
   `PullReviewItem.pullBytes` is null for an unsized row.
10. An item-list conversation row prints its size; the activity row in pull
    mode prints "Pull size" and in push mode still "Push size"
    (`project-activity.integration.test.tsx`).
11. `css-hygiene.integration.test.tsx`: one pinned assertion per theme that
    both fills clear 3:1 on `--loader-track`, read through `ruleColour` from
    the two track rules, so a token drift is measured as it drifts.

Gates: `npm run build`, `npm run test:frontend-integration`,
`cargo test --manifest-path src-tauri/Cargo.toml --lib`, clippy with
`-D warnings`, `cargo fmt`. Live check: open a Pull review against a storage
with more than 512 MB of sessions this computer does not have, expect the
gauge to fill, ✓ to tick one block at the top, and the refusal when a row
below it is added by hand; then Pull and confirm the next review carries the
next block.

## Docs

- `PLAN_COMPARE_ONLY_APPLY_TIME_CAPTURE.md` § Pull: Compare returns each
  row's stored size and the aggregate Apply limit; the review meter refuses
  past it; Apply recomputes the selection's size from the pinned manifest
  before its first write.
- "What landed" here on implementation.

## Compatibility

Clean cutover. No migration, no legacy path; the machine-local and bundle
schemas stay at 1; the three DTO fields are IPC-only and the manifest already
holds every size they read. No local or cloud state needs a wipe. The
renamed CSS classes have no consumer outside the two reviews and the tests.

## Checklist

1. Domain: `ConversationReviewRow.source_size`, `PullReview.apply_limit_bytes`
   and `from_plan`; fixture JSON.
2. Service: `stored_conversation_bytes`, `ThreadSyncEntry.pull_bytes` in
   `thread_review`, row sizes in `plan_bundle_restore`, the recheck in
   `apply_bundle_restore`; tests 1–5.
3. `types.ts`.
4. `sync-review/ReviewBudgetMeter.tsx`; Push switched onto it; CSS block
   renamed with the pull modifier; push-budget test selectors.
5. `pullReviewModel.ts`: `pullBytes`, `pullBudgetTotal`,
   `fitPullRecommendation`.
6. `RestorePlanView.tsx`: meter, refusal, ✓; `ProjectActivityPage.tsx`
   readout by mode; tests 6–11.
7. Bars green; live check; docs.

## What landed (2026-09-26)

The owner's decisions: D1 as recommended (R2), D2 the alternative — a
timestamp on the row, keyed on the stored session's last-message time — D3
strict in both, D4 the cloud ink. Deltas from the plan above:

- **Domain.** `ConversationReviewRow.source_size` and
  `ConversationReviewRow.stored_ended_at` (D2). `ConversationThread::last_message_at`
  reads the presentation the way the activity projection reads a storage-only
  row: the end, else the start, never before the start. The size sum is
  `BundleManifest::conversation_bytes(&thread)` beside `find_file`, in the
  domain rather than the service, because it is a manifest fact.
  `PullReview.apply_limit_bytes` with `PullReview::from_plan(plan, limit)`
  replacing `TryFrom<RestorePlan>`; the command passes
  `provider_capture::MAX_CAPTURE_BYTES`, whose comment now names both
  directions.
- **Service.** `ThreadSyncEntry.pull_bytes` is set wherever a comparison row
  has a stored thread (the per-provider review, the view-only rows) and
  absent on capture-blocked candidates. `pull_action_installs_bytes` names
  the three actions that weigh anything; both row constructors (the observe
  review's rows in `plan_bundle_restore_with_progress` and the pull-plan rows
  in `provider_thread_review`) set `source_size` through it and
  `stored_ended_at` from the thread. `pull_bytes_of_selection(manifest, plan,
  approved, (action type, cloud id) pairs)` and `pull_limit_sentence` price
  the approved selection in `apply_bundle_restore_with_progress` after the
  project-file eligibility check and before `adopt_cloud_metadata_actions`;
  the whole Apply returns the sentence as its error past the limit.
- **Frontend.** `src/components/sync-review/ReviewBudgetMeter.tsx` owns the
  meter (`ReviewBudget`, `reviewBudgetSentence`, `ReviewBudgetGlance`,
  `ReviewBudgetGauge`); the track and gauge carry the verb as a class
  (`push` / `pull`). `PushResourceWorkspace` keeps `pushBudgetTotal`,
  `pushReviewDisabledReason` and `pushBudgetBreakdown` and draws the shared
  components; its `recommendedPushSelection` walk is the strict prefix (D3):
  a miss ends the walk unless the session could not fit an empty push, which
  is skipped. `pullReviewModel.ts` gained `PullReviewItem.pullBytes`,
  `PullReviewItem.storedEndedAt`, `pullBudgetTotal` (sessions · setup &
  tools · project files, no reserve) and `fitPullRecommendation` (setup &
  tools first, then sessions newest first by `storedEndedAt`, charging the
  other step's ticks). `RestorePlanView` draws the glance and the gauge,
  refuses through `primaryDisabledReason` (loading first, then the sentence),
  fits ✓ on the sessions step with the title "Select the newest sessions that
  fit this pull (N)", and prints each item-list row's size.
  `ProjectActivityPage`'s `SessionMetrics` takes a `sizeReadout`
  (`reviewSizeReadout(mode, entry)`: "Push size" from `capture_bytes` in push
  mode, "Pull size" from `pull_bytes` in pull mode), on the rail rows and the
  stored-thread rows alike.
- **CSS.** The block is `.sync-review-budget-*`; `.sync-review-budget-track.pull > i`
  and `.sync-review-budget-gauge.pull .sync-review-budget-fill` paint
  `--thread-status-cloud`, declared before the over hatch so the hatch wins
  at equal specificity. `flex-wrap: wrap` moved from `.sync-push-resource-footer`
  into the existing `.sync-sync-review-footer` rule. The specimen
  `src/push-limit-mockup.tsx` was pointed at the renamed classes so its
  shipped frame still renders them.
- **Tests.** Domain: `a_pull_of_a_thread_weighs_every_file_the_manifest_lists_for_it`
  (transcript + sidecar, an unresolved reference counts nothing, the four
  `last_message_at` cases); the `pull_review.rs` round trip pins
  `apply_limit_bytes` out and dropped on the way back. Service: the replica
  test asserts the row's `source_size` equals the manifest sum, the
  comparison's `pull_bytes` the same, `stored_ended_at` equals both the
  thread's `last_message_at` and the activity row's `ended_at`, the review's
  limit, `pull_bytes_of_selection` on the same manifest (a metadata adoption
  weighs nothing), and the sentence. Frontend: push-budget selectors renamed
  plus the strict-prefix cases; pull-review gained the meter render, the
  `pullBudgetTotal` breakdown and sentence, and the fitted ✓ (order by time
  not review order, files charged first, an oversized session skipped, the
  view's ✓ title); project-activity pins "Pull size" and no "Push size" in
  pull mode; css-hygiene measures both fills on `--loader-track` through
  their rules.
- **Gates.** `npm run build` clean; frontend integration 423/423; domain
  101/101; `cargo test --manifest-path src-tauri/Cargo.toml --lib` 398
  passed, 8 ignored. `cargo clippy --all-targets -D warnings` on rustc
  1.96 stops on one pre-existing `nonminimal_bool` in
  `ConversationPlanAction::validate` (`crates/mallard-domain/src/lib.rs`,
  untouched HEAD code); the added lines carry no clippy warning.
  `cargo fmt --check` reports the same 38 files before and after this
  change — pre-existing drift, none of it in the lines added here.
- **Open.** The live check on a real backlog (a storage holding more than
  512 MB of sessions this computer lacks): gauge fills, ✓ ticks one block at
  the top, the refusal appears when a row below it is added by hand, and the
  next review after a Pull carries the next block.

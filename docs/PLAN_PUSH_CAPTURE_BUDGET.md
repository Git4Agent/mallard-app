# Push what fits, say what waits

2026-09-01. Proposal, not implemented. Follows the same-day evaluation of the
"Load earlier does not bypass the review plan" write-up and replaces its
five-step correction. Two decisions were taken the same day and are built in:
the fit is a strict time-ordered prefix (not the shipped greedy loop), and
there is **no "rounds" concept** — Mallard keeps no state between pushes; a
review says what this push carries and that the rest does not fit, and the
user is prompted to push again. The footer limit bar stays, and so does the
commit rail in the review; what goes is the 30-day paging and the separate
"Older activity" box. Compatibility is a clean cutover with schemas at 1
(see 5). The UI renders on the real stylesheet in
`push-budget-mockup.html` (dev server: `/push-budget-mockup.html`, entry
`src/push-budget-mockup.tsx`).

## 1. What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | Pins served from the watcher cache are removed from the selection *before* the budget fit, so their bytes are never charged. Every quiet reopen can admit another full budget on top of everything already pinned, and the pinned pile grows each time. | `service.rs:6718` (reuse) then `:6732` (fit) |
| 2 | The fit charges `source_size`, the transcript file alone. The push preflight sums every file of the resource, sidecars included. | `provider_capture.rs:952` vs `:599-651` |
| 3 | Tools ("Requirements") and project files ride the same push capture but never enter the fit. Project files are scanned after prepare, so no prepare-time fit can reserve for them. | `service.rs:9751` captures the whole `push_recipe` |
| 4 | Budget-held rows are hidden. The review row filter demands a selectable publish decision, so a blocked row never renders; the banner counts rows nobody can see; the `capture_budget` hold copy is unreachable; the comment saying held rows "stay visible and unselectable" is wrong. | `ProjectActivityPage.tsx:1545`, `:1599`, `:1119`; `PushResourceWorkspace.tsx:563` |
| 5 | Nothing says how much this push carries, and after the push nothing prompts the user that sessions were left out. Reaching the rest means remembering to reopen Push review by hand. | `PushResourceWorkspace.tsx:568-575` |
| 6 | The shipped fit is greedy: it walks newest first and keeps admitting smaller rows past a row that did not fit, so a smaller older session can be selectable while bigger newer ones wait. | `service.rs:6468-6501` |
| 7 | Pull prepare reuses pins and captures with no fit at all. | `service.rs:10073-10095` |

The late failure `selected resources exceed 536870912 bytes` is safe, because
capture runs before `publish_reviewed`, but it is a dead end with a raw number.

## 2. Invariants

- **I1.** Every selectable resource in one review fits the push capture together with everything else the push must carry.
- **I2.** Every outstanding local session is on screen in the review, on the commit rail as the activity page draws it, every commit and session at once, newest first: selectable, held for Pull, or not fitting this push. No paging, no separate box for anything.
- **I3.** No rounds, no memory. Nothing about held sessions is stored between pushes; every Push review recomputes from the current state. The UI says "doesn't fit this push, push again", never "round 2 of 4".
- **I4.** The plan stays frozen. A selection change never rebuilds the plan.
- **I5.** The exact push-time size check stays as the race guard.
- **I6.** A push carries the newest prefix of the outstanding sessions that fits; no older session goes before a newer one.

## 3. Backend

### 3.1 Fit first, then reuse pins; stop at the first miss

In the push branch of `build_thread_sync_comparison`:

1. `selection` = base ∪ every selectable local row from the preliminary review (unchanged).
2. `(kept, held) = fit_capture_budget(selection, budget)` over the **full** selection.
3. `(to_capture, served) = reuse_candidate_pins(kept)`.
4. Capture `to_capture`, store its pins, run the review with `served`, hold `held`.

Reordering the two calls is what charges served pins like any other row.

The policy changes with it. `fit_capture_budget_with` today keeps walking past
a row that does not fit and admits smaller older rows behind it. It becomes a
**strict time-ordered prefix**: base first, then newest first, and the walk
**stops at the first row that does not fit**. Everything older waits. The
list is never interleaved and the banner can say "the newest N sessions, back
to <date>". The cost is unused room in a push (88 MB in the specimen);
accepted.

A session that can never go is not a hold, it is unsyncable. `local_session_views`
sets `blocked_reason` on any conversation whose `capture_bytes` exceed
`MAX_CAPTURE_BYTES` ("Too large to sync: 640 MB; one push carries at most
512 MB."), so it rides the existing blocked-candidate path: the activity page
shows it as unavailable with that reason, and the review never lists it
(`service.rs:6898` mints its Unavailable entry, which the review row filter
already drops). The fit applies the same rule against the real budget
(`limit − reserved`) for the sliver between the two thresholds, blocking the
row with the same reason instead of holding it, so the prefix walk can never
stall on a row that would not fit an empty push.

### 3.2 Charge the whole resource

`LocalSessionView` gains `capture_bytes: u64`, the sum of
`symlink_metadata().len()` over the resource's `files` (transcript plus
sidecars). `source_size` stays for the stamp. Cost is one extra stat per
sidecar; sessions carry few. The fit reads `capture_bytes`.

### 3.3 Reserve for requirements; the UI budgets project files

`budget = MAX_CAPTURE_BYTES − reserved`, where `reserved` is the capture size
of every selectable non-conversation inventory resource (tools and other
requirements, an upper bound that is small in practice). Project files cannot
be fitted at prepare: the scan is later and the selection is the user's. They
are budgeted by the footer meter (4.2), with the exact push check behind it.

### 3.4 The DTO

- `ThreadSyncEntry.capture_bytes: Option<u64>` on local rows, filled at the
  three construction sites (`service.rs:5787` local fill, `:6898` blocked
  candidates, `:6958` review rows). Storage-only rows carry `None`.
- `ThreadSyncComparison.capture_budget: Option<CaptureBudget>` on push-plan
  reviews only: `{ limit_bytes, reserved_bytes, admitted_bytes, held_count, held_bytes }`.
  Nothing in it predicts future pushes.
- The `"N conversation(s) exceed this review's … byte capture budget"` warning
  string goes; the typed field replaces it. No hardcoded `512 MB` anywhere in
  the frontend; every figure comes from `limit_bytes` through `formatBytes`.

### 3.5 Pull parity (phase 2)

Apply the same fit in the Pull prepare path over the pull-selectable
local-present candidates that need byte pins. Held rows get
`pull_decision = blocked(CaptureBudget)` and the same DTO field. The Pull
review already renders inert rows in its older fold, so the UI work is the
chip and the copy.

### 3.6 Tests

- Rust `push_review_charges_cached_pins`: seed pins through
  `store_candidate_pins` for candidates worth more than the budget, prepare a
  push review, assert the selectable rows' `capture_bytes` sum to at most
  `limit − reserved` and the rest carry `CaptureBudget`.
- Rust `fit_charges_sidecars`: a session whose sidecar pushes it past the
  remaining budget is held even though its transcript alone would fit.
- Rust `fit_is_a_time_prefix`: the walk stops at the first miss (the existing
  `fit_capture_budget_keeps_base_and_admits_newest_first` encodes the greedy
  slip-in through its `tiny-old` row and flips to held), and a row larger than
  the whole budget is blocked as unsyncable, not held, while the rows after it
  are still considered.
- Rust `oversized_session_is_unsyncable`: a view whose files exceed
  `MAX_CAPTURE_BYTES` carries `blocked_reason`, is absent from the review's
  selectable and held sets, and shows as unavailable in the activity DTO.
- Rust: the Pull twin of the first test (phase 2).
- Frontend: held rows render inert with their chip and hold copy; the meter
  sums sessions, reserved, and project files; the over-limit state disables
  Push with the sentence as its title; the banner and the outcome prompt read
  the DTO.

## 4. UI

### 4.1 Rows that don't fit are visible, inert, and say so

- **The whole timeline, once.** In push review mode the page keeps the commit
  rail exactly as the activity page draws it — Uncommitted on top, then every
  first-parent commit with its sessions under it, the fold controls on the
  spine — but with no window: every commit and every session at once, newest
  first. No 30-day paging, no Load earlier, no "Older activity" fold. The
  review requests the activity with `window_days` spanning back to the oldest
  comparison entry (`build_project_activity` defaults a missing window to
  `DEFAULT_HISTORY_WINDOW_DAYS`, so the span is passed explicitly), one
  request; entries still without an activity row after that render as plain
  entry rows at the foot of the rail, same list, no box. The `sessions` and
  `olderEntries` filters at `ProjectActivityPage.tsx:1545` and `:1599` stop
  dropping `capture_budget` holds; the fold at `:2050` and the review-mode
  `RailFoot` go. Today's `showGitTimeline` already keeps the rail in selection
  mode whenever there are commits, so this is the status quo minus the paging.
- Rows whose `push_decision.blocker` is `capture_budget` render inert in their
  place: no checkbox, the row's real status glyph (local outstanding) with the
  hold copy in its tooltip, and a chip in the existing
  `sync-history-branch-state history-only` grammar that prints the reason
  without hover: "Later". Rows still lead with the icon; the chip is the same
  element the "View only" access note already uses. Because a push is a time
  prefix (I6), the list reads top to bottom as "this push, then everything
  that waits"; the chip never has to number anything.
- Unsyncable sessions (3.1) are not in the review at all; they show on the
  activity page as unavailable with their reason, like any blocked session.
- Review-mode session rows gain a size readout after the duration chip
  (`drive` icon, `formatBytes`). Review only; the standing activity page keeps
  its metric row.
- Delete the stale comment at `PushResourceWorkspace.tsx:563`.

### 4.2 The footer meter

Selection-driven, so it lives in the footer beside the button it can block,
which is the file's own rule for selection-driven state. The footer's sentence
slot carries "This push · 424 MB of 512 MB" and a 4px bar (`role="meter"` with
`aria-valuenow`, `aria-valuemax`, `aria-valuetext`); the tooltip breaks the
total into sessions, requirements, and project files. Over the limit, which
only project files or requirements can cause once 3.1 lands, the bar takes the
danger ink, the sentence becomes "23 MB over the 512 MB push limit — exclude
sessions or files to push", and Push is disabled with that sentence as its
title through `pushDisabledReason`.

### 4.3 The banner

Replaces the capture-budget warning, still plan-derived and standing, no
action:

- title: "Too much to push at once"
- message: "This push carries the newest 6 sessions (421 MB), back to Aug 28.
  The 13 older sessions (1.1 GB) don't fit — push again after this one."

"The newest N, back to <date>" is exactly true under I6, and the date gives
the reader the cut line without counting rows. Severity stays warning: the
push is not the whole truth while sessions are left out, the same reasoning as
"held for Pull".

### 4.4 After the push: the prompt

The storage card's outcome banner (`buildPushOutcome`) gains one sentence when
the pushed review held rows: "13 sessions didn't fit — push again to
continue." and one action, "Push again" (`SyncOutcomeAction` kind
`push_again`), which opens Push review the ordinary way. Nothing is carried
over: the new review recomputes the fit from the current state, so if
sessions were created in the meantime the newest ones go first, as always. No
chaining, no automatic loop, no counter.

### 4.5 Copy

| Where | Today | Proposed |
|---|---|---|
| Row hold tooltip | "Not in this round — the review's 512 MB capture budget filled before this session's turn. Push the selected sessions first, then open Push review again." | "Doesn't fit in this push — the 512 MB limit is full with newer sessions. Push again after this one." |
| Row chip | none (row hidden) | "Later" |
| Activity page, oversized session | none (fails at push) | "Unavailable for sync. Too large to sync: 640 MB; one push carries at most 512 MB." (existing unavailable presentation) |
| Older-activity fold, review mode | "N sessions need no changes or can't sync" | gone; one list |
| Banner | "N conversations wait for the next round · This review's 512 MB capture budget filled before their turn. Push the selected conversations first, then open Push review again." | 4.3 |
| Footer | none | "This push · 424 MB of 512 MB"; over: "23 MB over the 512 MB push limit — exclude sessions or files to push." |
| Push title | none | the footer sentence |
| Outcome banner | none | "13 sessions didn't fit — push again to continue." + "Push again" |
| Late error | "selected resources exceed 536870912 bytes" | unreachable from the UI once 3.1 lands; keep in Rust as the guard, print the limit through the byte formatter |

## 5. Landing order

- **Phase 1, correctness and visibility:** 3.1, 3.2, 3.3, 3.4, the first three
  tests of 3.6; 4.1, 4.2, 4.3, 4.5. Gate: cargo tests and clippy, frontend
  tests and tsc, the CSS hygiene gates.
- **Phase 2:** 4.4 outcome prompt; 3.5 Pull parity.

Each phase is a clean cutover of its own copy and classes; nothing from the
current banner or hold copy survives beside the new text.

### Compatibility

Clean cutover. No migration, no legacy execution path, no feature flag.
Schemas stay at 1: the DTO additions (`capture_bytes`, `capture_budget`) are
IPC-only and nothing persisted changes — `ConversationPushPlan`
(`PUSH_PLAN_SCHEMA`), the observation disk cache, the candidate-pin cache, and
the sync metadata keep their shapes. The greedy loop, the `"… exceed this
review's … byte capture budget"` warning string, the hidden-row filter, and
the current banner and hold copy are deleted, not kept beside the new code.
Before retesting, `~/.mallard` and the cloud storage are wiped; there are no
customers to carry over.

## 8. Implementation checklist

Phase 1, in landing order. Each line names the seam so the diff is
predictable.

Rust:

- `crates/mallard-infrastructure/src/provider_capture.rs`:
  `LocalSessionView.capture_bytes` filled in `local_session_views` from every
  file of the resource (`symlink_metadata`, no-follow; a failed stat blocks
  the view as today); `blocked_reason` set when `capture_bytes >
  MAX_CAPTURE_BYTES` with the "Too large to sync" copy, printed through the
  Rust byte formatter.
- `src-tauri/src/project_sync/service.rs`:
  - `fit_capture_budget_with`: strict prefix; stop at the first miss; a row
    larger than the whole budget is held and skipped; lookup reads
    `capture_bytes`.
  - push branch of `build_thread_sync_comparison`: fit over the full
    selection, then `reuse_candidate_pins` over `kept`, capture `kept −
    served`; compute `reserved` from the non-conversation inventory
    resources; drop the warning `push`; fill `ThreadSyncComparison.capture_budget`.
  - `ThreadSyncEntry.capture_bytes` (three construction sites),
    `ThreadSyncComparison.capture_budget`, new `CaptureBudget` struct
    (`Serialize`).
  - Tests of 3.6, in the existing `#[cfg(test)]` module; flip
    `fit_capture_budget_keeps_base_and_admits_newest_first`.
- `cargo fmt`, `cargo clippy -D warnings`, `cargo test`.

Frontend:

- `src/types.ts`: `ThreadSyncEntry.capture_bytes?`,
  `ThreadSyncComparison.capture_budget?`, `CaptureBudget`.
- `src/components/project-sync/ProjectActivityPage.tsx`: push review mode
  keeps the commit rail and the Uncommitted section but loads the whole span
  in one request (`window_days` reaching the oldest comparison entry) and
  renders no `RailFoot` and no Older-activity fold; entries still without an
  activity row render as `SyncEntryCard` rows at the foot of the rail; the
  `sessions` / `olderEntries` filters admit `capture_budget` holds, which
  render inert with the "Later" chip on `SessionCard` and `SyncEntryCard`;
  size readout in review mode; hold copy in `syncHoldDetail`.
- `src/components/project-sync/PushResourceWorkspace.tsx`:
  `visibleConversationResourceIds` admits the holds for the fallback
  inventory path; banner from `capture_budget`; footer meter component and
  `pushDisabledReason` branch; delete the stale comment and the old banner.
- `src/components/project-sync/syncOutcome.ts` +
  `SyncOutcomeNotice.tsx` + `ProjectSync.tsx`: the "didn't fit" sentence,
  `push_again` action, handler opens Push review.
- CSS: the meter's rules in `App.css` (track, fill, over state), no new
  tokens; run the CSS hygiene gates.
- Tests in `tests/frontend/project-activity.integration.test.tsx` and the
  push-review coverage in `project-files` / `resource-inventory` tests as
  listed in 3.6; `npx tsc --noEmit`.

Phase 2: `syncOutcome.ts` prompt if not landed with phase 1; Pull prepare
fit in `service.rs` (the restore-plan path) with its test.

## 6. Decisions

Decided 2026-09-01: strict time-ordered prefix, no greedy; no rounds and no
state between pushes; the footer limit bar stays; clean cutover with schemas
at 1. D1 banner severity: warning. D2 size readout on review rows: yes.
D3 chip text: "Later". The review shows the whole timeline at once (4.1):
every commit and session on the rail, newest first, no paging and no "Older
activity" box; rows that do not fit are simply not selectable. A session too
large for any push is unsyncable (3.1): unavailable on the activity page,
absent from the review. D4: the commit rail stays in the push review, with
the Uncommitted heading; only the window, the Load earlier control, and the
fold go.

Nothing is open.

## 7. Rejected

- **Raising `MAX_CAPTURE_BYTES` or making it a knob.** The cap is a memory bound; a knob moves the cliff and hides it.
- **Pin-only review capture** (hash without retaining bytes so every candidate is pinned at review). It removes the review-time budget, but the push still captures at most 512 MiB, so the split remains, and a plan pinned against one head cannot survive its own first publish (`service.rs:9212`). A larger change for later.
- **Greedy first-fit** (the shipped loop). It squeezes more bytes into a push, but the list interleaves waiting and selectable rows, an older selectable row under a newer waiting one reads as a bug, and every hold needs a per-row explanation of the room that was left.
- **Rounds** — "round 1 of 4", "Continue · round 2 of 4", "Push all rounds", per-row round numbers. They imply Mallard tracks progress across pushes; it does not and should not. The count of future pushes was an estimate that would drift as sessions arrive. Replaced by 4.3 and 4.4.
- **"Move into this push"** (a preferred-rows list passed to prepare so a waiting row can be pulled forward). Not now: an IPC parameter and a re-prepare flow for a case the time-ordered rule already explains. Revisit only if users ask to pick which session goes first.
- **Implicit unlock on deselect.** Violates I4.
- **A paginated comparison.** I2 needs every outstanding row in the review.

## 9. What landed (2026-09-01, uncommitted)

Both phases, in one cutover. Deltas from the plan above:

- **Backend.** `fit_capture_budget_with` returns a `CaptureFit { kept, held, oversized }`
  and stops at the first miss; `plan_candidate_capture` wraps fit-then-reuse
  so the ordering is a pure function with a test
  (`cached_pins_are_charged_to_the_budget_before_they_are_served`). The
  discovery stats every conversation file once and stores `capture_bytes` on
  the conversation meta; `local_session_views` copies it into the view, so
  the activity page and the review read the same number. A conversation over
  `MAX_CAPTURE_BYTES` gets its `blocked_reason` at discovery ("Too large to
  sync: …"); the sliver between the raw cap and `limit − reserved` is blocked
  by the fit with the new `ConversationBlocker::TooLarge`, which the review
  filter never admits. Pull parity landed in the same change:
  `hold_over_budget_pull_candidates` blanks the held rows' actions in the
  restore plan. `Discovery::capture_bytes_of` measures the requirements for
  `reserved_bytes`. `provider_capture::format_bytes` prints binary units like
  the frontend.
- **DTO.** `ThreadSyncEntry.capture_bytes`, `ThreadSyncComparison.capture_budget`
  (`CaptureBudget { limit_bytes, reserved_bytes, admitted_bytes, held_count, held_bytes }`),
  and `too_large` in the blocker union. The warning string is gone.
- **Frontend.** `isPushReviewRow` / `isCaptureBudgetHold` in
  `pullReviewStatus.ts`; review mode requests `reviewWindowDays(comparison)`
  (span to the oldest entry, ten years without a comparison, never under 30)
  and draws no `RailFoot`; the Older-activity fold and its CSS are deleted and
  entries without an activity row render as plain rows labelled "Sessions
  without an activity row"; held rows keep their state glyph
  (`sessionSyncPresentation` has a `capture_budget` branch), wear "Later",
  and read "Doesn't fit in this push — the 512 MB limit is full with newer
  sessions. Push again after this one."; review rows carry a `drive` size
  readout (`SessionMetrics.captureBytes`); the workspace banner reads the DTO
  ("Too much to push at once … back to <date> … push again after this one");
  the footer meter is `pushBudgetTotal` + `PushBudgetMeter`
  (`.sync-push-budget`, `.sync-push-budget-bar`, `.over`), and Push refuses
  with the over-limit sentence as its title
  (**superseded 2026-09-04**: the meter is now drawn in both of the review's
  pinned zones — `PushBudgetGlance` in the header and `PushBudgetGauge` against
  the button — over `PushBudgetTrack`/`PushBudgetFigure` and
  `.sync-push-budget-glance` / `-track` / `-figure` / `-gauge` / `-fill` /
  `-sentence`; the track is scaled to max(total, limit) with the limit as a
  tick past it, the refusal takes its own footer row, and only the footer
  instance is a live region. `PushBudgetMeter`, `.sync-push-budget` and
  `.sync-push-budget-bar` are deleted. Specimen: `push-limit-mockup.html`); the outcome banner adds a
  "didn't fit — push again to continue" stat and a `push_again` action that
  calls `openPushChooser`.
- **Tests.** Rust: the three fit tests plus `format_bytes`,
  `oversized_capture_reason`, and `capture_bytes_of_files` in
  provider_capture. Frontend: `tests/frontend/push-budget.integration.test.tsx`
  (six tests: held row inert with chip and glyph, hold copy with and without
  the limit, `reviewWindowDays`, banner, meter within and over, `pushBudgetTotal`);
  the paging/fold tests in `project-activity` and `thread-variants` now pin
  the one-list behaviour.
- **Gates.** Frontend integration 279/279 (was 273), `tsc` clean, `vite build`
  clean, `cargo clippy --workspace --all-targets -D warnings` clean,
  `cargo fmt` applied, `cargo test --workspace --all-targets` green: service
  lib 259, infrastructure 376 (3 ignored live), domain 55, application 37,
  cli 19 + 10, catalog e2e 1, aimock e2e 2 + 7 + 8. Two existing tests changed
  with the behaviour: `prepare_push_review_matches_standalone_commands` strips
  the push-only `capture_budget` before comparing against the observe-only
  comparison, and the watcher's dirty-root refresh now carries `capture_bytes`
  along with `source_size`, which is what kept the two probe-window tests'
  "cached and rebaselined rows are identical" assertion true.
- **Live probe** (session e8, real `~/.mallard`, mallard-app → Local
  storage 1 after the wipe): the review opens clean with no comparison error;
  294 entries, all local-only; `capture_budget` = limit 536,870,912, reserved
  362,653, admitted 526,396,096, held 240 rows / 1,939,193,505 bytes; 54
  selectable, 240 held, 0 too large, 54 push actions. Discover 3.55 s,
  capture 12.7 s for the admitted 502 MB, about 16 s in all. The strict prefix
  admits 54 rows where the greedy slip-in admitted 67; the backlog takes about
  four more pushes. Each reopen re-captures the admitted rows (~13 s) until the
  watcher's pins cover them, after which a quiet reopen serves the pins.
- The specimen `push-budget-mockup.html` stays as the reference for the look.

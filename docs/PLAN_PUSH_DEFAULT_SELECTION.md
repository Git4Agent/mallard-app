# Push review opens with the newest sessions that fit

- Status: proposal, not implemented
- Date: 2026-09-04
- Scope: the Push review's default selection. Frontend only.

## Lineage

The 2026-09-01 fit (`PLAN_PUSH_CAPTURE_BUDGET.md`) admitted a newest-first
prefix in the backend and held the rest of the backlog inert. The 2026-09-03
compare-only cutover (`PLAN_COMPARE_ONLY_APPLY_TIME_CAPTURE.md`) deleted that
fit, the holds and the banner: every session is selectable, and the meter is
the only thing that refuses a push. It left the default selection as it was
before the fit existed, "every selectable session", which the meter refuses.

This plan puts the newest-first rule back as the **default selection only**.
Eligibility stays exactly as the cutover defined it.

## What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | `recommendedPushSelection` ticks every selectable conversation that has a comparison entry, so the review opens with the whole backlog selected. On this machine that is 347 sessions, 2,670 MB against 512 MB: Push is refused the moment the review opens. | `PushResourceWorkspace.tsx:258-289`, `ProjectSync.tsx:1765`, `:1806` |
| 2 | Clear is the only way out. It leaves `0/344` with the meter at `389 KB` (the reserved requirements alone): the screenshot. From there the user hand-picks ~70 rows against the meter. | `SyncReviewSelectionBar.tsx`, `PushResourceWorkspace.tsx:809` |
| 3 | ✓ "Use the recommended session selection (347)" restores the refused set. | `PushResourceWorkspace.tsx:863` |
| 4 | Every input for a sized default is already on the DTO: `capture_bytes` per row, `capture_budget { limit_bytes, reserved_bytes }`, `local_updated_at`. Nothing reads them to pick a default. | `types.ts:1416`, `:1422` |

Measured on the real state (probe, 2026-09-04, mallard-app → myconfstorage2,
26 s to prepare):

| | |
|---|---|
| Selectable sessions | 347 · 2,670 MB · median 2.9 MB · largest 70.9 MB |
| Reserved requirements | 389 KB |
| Strict newest-first prefix | **71 rows · 493 MB**, back to 2026-09-02 |
| Greedy first-fit, for the cost only | 76 rows · 511.6 MB |
| Sessions that could not fit an empty push | 0 |
| Backend entry order equals newest-first | yes, on all four links |

The legible rule costs 5 rows and 18.6 MB of room per push on this backlog.
About six pushes clear it. Nothing is stored between them.

## The rule

- **R1.** The review opens with the newest sessions that fit one push ticked,
  in time order, and the walk stops at the first session that does not fit.
  Nothing older is ticked. This is the strict prefix chosen on 2026-09-01;
  the greedy fill stays rejected because it interleaves ticked and unticked
  rows.
- **R2.** The walk uses the meter's own arithmetic. Start from
  `pushBudgetTotal(recommended without sessions)`: reserved requirements plus
  the project files the recommendation already carries (recipe files with a
  size in the inventory the review opened with). Add sessions, newest first,
  while the total stays within `limit_bytes`. A default selection therefore
  always passes the meter, by construction.
- **R3.** A session that could not fit an empty push in this review (its
  bytes alone push the starting total past the limit) is never ticked and
  does not stop the walk. Sessions over 512 MiB are already blocked at
  discovery (`provider_capture.rs:1141`); this covers the sliver under it,
  so the default can never stall at zero behind one row the meter would
  refuse anyway.
- **R4.** Everything stays selectable. The user unticks newer rows and ticks
  older ones freely; the meter refuses what does not fit; ✓ restores the
  default. No row is held, hidden, or chipped.
- **R5.** Without a `capture_budget` on the comparison the recommendation is
  every selectable session, unchanged. Only observe-only comparisons lack the
  budget and they never open a Push review.

Order key: `local_updated_at` descending, ties by `thread_id` ascending. That
is the order the backend already emits; the function sorts explicitly so the
rule does not depend on DTO order.

## What the user sees

Opening Push on this backlog: header `71/347`, glance `493 MB / 512 MB`, the
top 71 rows ticked in one block under their commits, the rows below unticked
and selectable, Push enabled. ✓ is disabled while the selection equals the
default; Clear empties it as today. After the push the activity page shows
the remaining sessions as local-outstanding, and the next Push review ticks
the next newest block. No rounds, no count of future pushes, no memory
between pushes.

## Where

- `src/components/project-sync/PushResourceWorkspace.tsx:258`
  `recommendedPushSelection`: replace the "add every selectable
  conversation" loop with the walk in R1–R3. The signature already accepts
  `projectContentInventory`; it is the fourth argument R2 needs. The walk
  can call `pushBudgetTotal` from the same file.
- `src/components/project-sync/ProjectSync.tsx:1765` `openPushChooser`:
  pass `lockedProjectContentInventory` as the fourth argument so R2 has file
  sizes for non-Git projects. `setPushSelected(new Set(projectDefaults))`
  (`:1806`) and `projectDefaults={pendingPush.projectDefaults}` (`:3213`)
  already carry the result to the opening selection and to ✓.
- `src/components/project-sync/PushResourceWorkspace.tsx:863` the ✓ title
  on the history step reads "Select the newest sessions that fit this push
  (N)". The aria-label stays "Use recommended selection (N)"; two suites
  assert it and it still says what the button does.
- Nothing in Rust, the DTO, or CSS. `CaptureBudget` stays
  `{ limit_bytes, reserved_bytes }` and its comment ("Compare does not admit
  or reject sessions by age or list position") stays true: compare still
  returns every eligible session; the review's opening ticks are a frontend
  recommendation.

## Not doing

- **A banner** ("K older sessions aren't in this push — push again"). It
  belonged to the deleted admission set; with a free selection it would have
  to follow every tick. The header count and the meter already say what this
  push carries.
- **Re-fitting after the project-file scan.** Defaults are computed once at
  open; files the scan adds later can push the total over the limit, and the
  meter refuses as it does today. Re-fitting would untick sessions the user
  has already touched.
- **A backend recommendation flag.** The frontend owns the recommendation
  and holds every input; a flag would be a second copy of the meter's
  arithmetic to keep in step.
- **Greedy fill.** Rejected 2026-09-01; the cost of the strict rule is
  measured above.

## Tests

`tests/frontend/push-budget.integration.test.tsx`, on the existing fixtures
(`fits` 96 MB newest, `waits` 118 MB five days older, limit 512 MB,
reserved 3 MB, `projectFile` 500 MB):

1. Newest first: entries `[fits, waits, older 400 MB]` → the recommendation is
   `{fits, waits}`; `older` is not ticked.
2. Strict, not greedy: entries `[fits, big 450 MB, tiny 10 MB oldest]` →
   `{fits}` only. `tiny` would fit and stays unticked.
3. Unpushable alone is skipped: `[huge 511 MB newest, fits]` → `{fits}`.
4. Project files are charged: recipe selects `projectFile` (500 MB, in the
   locked inventory) → a 96 MB session is not ticked, a 5 MB one is; the
   file stays in the recommendation.
5. Storage-only and unselectable rows never enter the walk; `capture_bytes`
   absent counts as 0, as in the meter.
6. The workspace renders the ✓ title "Select the newest sessions that fit
   this push (1)" on the history step, and `useRecommendedForStep` restores
   the prefix.
7. `recommendedPushSelection` without a budget still returns every
   selectable session: the existing case in
   `resource-inventory.integration.test.tsx:157` (its fixture carries no
   `capture_budget`) pins this unchanged.

The existing "does not apply a newest-first admission policy" and "all
eligible sessions stay selectable" tests stay as they are: they pin
eligibility, which this plan does not touch. Update the file's header
comment to say the default selection is the newest prefix and the meter
still enforces the limit.

Gates: `npm run test:frontend-integration`, `tsc`, `vite build`. No Rust
test changes.

Live check after landing: open Push on mallard-app → myconfstorage2 and
expect the header and glance near `71/347` and `493 MB / 512 MB` (the exact
numbers drift as sessions arrive), the ticked rows in one block at the top.

## Docs

- Amend `PLAN_COMPARE_ONLY_APPLY_TIME_CAPTURE.md` § Selection: eligibility
  unchanged; the review opens with the newest sessions that fit ticked; ✓
  restores that set.
- Add "What landed" to this document on implementation.

## Compatibility

Clean cutover. No migration, no legacy path, no new IPC field, machine-local
and bundle schemas stay at 1. The change is frontend-only, so no local or
cloud state needs a wipe for it; the operator's planned wipe of `~/.mallard`
and the test cloud data is harmless to it.

Peers: `ProjectSync.tsx` and `App.css` were edited by other live sessions
within the hour (notice system, push slowness). Check `ListAgents` and
mtimes before editing, and edit with Read+Edit rather than rewrites.

## Checklist

1. `recommendedPushSelection`: prefix walk over `pushBudgetTotal` (R1–R3,
   R5), explicit sort.
2. `openPushChooser`: pass the locked project-file inventory.
3. ✓ title on the history step.
4. Tests 1–6 in `push-budget.integration.test.tsx`; header comment.
5. Frontend bar green; live check on the real backlog.
6. Docs: compare-only § Selection; "What landed" here.

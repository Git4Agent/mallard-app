# Push and Pull open in place, with one loading caption

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

- Status: implemented 2026-09-26, uncommitted (owner picked direction A with
  the Caption progress style); see "What landed"
- Date: 2026-09-26
- Scope: the wait between clicking Push or Pull and a review you can act on,
  for the project Push and Pull reviews. Frontend only; no Rust, no IPC, no
  schema change. The transfer after the click on Push / Pull / Mark as
  reviewed (uploading, applying) is out of scope.
- Visual contract: `mockups/review-loading-mockup.html?verb=pull&dir=a&progress=caption`
  (and `verb=push`), served by the Vite dev server. The Before frames there
  are the shipped behaviour this plan replaces.

## Lineage

The owner reported that "the pull loading page, push loading page are not
good", with a screenshot of a Pull review mid-load: a green *Done · Pull review
ready* message, 0 B shown twice, a 0/3 count, an empty list and a greyed
*Mark as reviewed*. The specimen drew today's sequence moment by moment and
offered A (the storage card opens in place) and B (Activity stays, the review
arrives whole). The owner picked A, rejected round 1's stage line (a progress
underline with a readout and step dots), and picked the Caption: the loading
caption the app already uses, words only.

## What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | The click changes one button. Push and Pull set `busy` and `runningAction`; the path-bar button spins *Preparing…* over an Activity page that looks usable but is locked. Push's prepare takes ~5 s with no readout; Pull squeezes `7/12 · 6.0 MB…` into the button. There is no Cancel. | `ProjectSync.tsx` `openPushChooser`, `planRestore`; `ProjectLinksWorkspace.tsx` `workspacePullReadout` |
| 2 | The review mounts before its list, and a green *Done · Pull review ready for … — nothing applied yet.* message stacks above it. The meters (0 B twice for Pull) and the count (0/3) describe rows that are not on screen. | `ProjectSync.tsx` `notify(\`Pull review ready…\`)`; `RestorePlanView.tsx` header and footer |
| 3 | **Pull's list goes blank** between the two reads. In selection mode the rows are filtered on the comparison, but the scaffold is drawn only for `(loading && !activity) \|\| (storageLoading && allSessions.length === 0)`. Once `activity` lands and `comparison` has not, the list renders empty — the owner's screenshot. The tools row moves up into the gap and back down when the rows land. | `ProjectActivityPage.tsx:1988`, `reviewSessions` |
| 4 | A failed comparison in a Pull review never recovers: `reviewListReady` requires `comparison !== null`, so the primary stays disabled with *Loading storage changes…* for good. | `ProjectActivityPage.tsx` `reviewListReady`; `RestorePlanView.tsx:293` |
| 5 | Pull's primary is greyed while loading and its reason is only a tooltip. The label reads *Mark as reviewed* over a list that has not arrived. | `RestorePlanView.tsx:289-310` |
| 6 | Push's primary is live over the scaffold: the defaults are ticked on rows the user has not seen. | `pushReviewDisabledReason` |
| 7 | The `rows` scaffold draws a 53 px row; a review's two-line session row is 68 px, so the review list jumps when it lands. And on its first commit the review's list paints nothing at all: every loading flag starts `false`, so the branch falls through to `null` until the load effect runs. | `LoadingScaffold.tsx` `RowBones`; `ProjectActivityPage.tsx:1988-2012` |
| 8 | Pull's *Checking readiness…* line sits inside the collapsed *Agent setup & tools* details, so the summary says *0 selected* while it is still checking. | `RestorePlanView.tsx:558` |

## What the user sees

**Pull**

1. Click Pull. If the plan has not arrived within 150 ms, the storage card
   opens where the review will be, in the review's own frame: header
   *Pull review* and ×, the caption *Reading the latest version from
   Attic R2…* in the search field's slot, placeholder rows under it, and a
   disabled primary *Preparing…*. The path-bar Pull says *Preparing…* too.
   × is **Cancel**.
2. The plan is read. The header gains *Version 12*; the plan's standing
   warning (stored without credential redaction) takes its place; the
   placeholders take the plan's session count; the tools row appears with a
   placeholder where its count goes. Caption *Loading sessions…*, primary
   *Loading sessions…* (both buttons).
3. Sessions are read. Caption *Comparing with this computer…*.
4. The comparison lands. The rows replace the placeholders; the search field
   replaces the caption; the meters, the count, ✓ and trash appear; the
   primary becomes *Mark as reviewed* (or *Pull N*). A visually hidden
   status line says the review is ready. No Done message.

**Push**

1. Click Push. After 150 ms, the card opens with *Push review*, the
   credential-redaction flag, ×=Cancel, the caption *Comparing this computer
   with Attic R2…* and placeholders; primary *Preparing…*.
2. The context arrives: placeholders take the session count; caption
   *Loading sessions…*; primary *Loading sessions…*.
3. Sessions are read: rows, meter, count, and an enabled Push, together.

**Endings.** Whatever the prepare resolves to replaces the card in the same
update, from the card or — under 150 ms — straight from Activity. Every one of
these already exists; only the moment it appears changes.

| Resolution | What replaces the card |
|---|---|
| Review | The review, at step 2 above |
| Push gate: storage has newer changes / review required | The card's `requires_pull` notice with *Open Pull review* |
| Storage unavailable, or any failure to start | The card's `action_failed` notice (*Retry*, or *Reconnect* for a refused credential) |
| Pull: a cleared storage this computer synced with | The card's `nothing_to_pull` note |
| Pull: only a metadata accept is needed | The card closes; the shell's *Storage reviewed … — files unchanged* message |
| Pull: binding not confirmed | The existing binding editor, as today |
| Pull: Mallard Cloud repository missing | The existing address picker dialog, as today |

A review never opens and then closes.

## The rules

- **L1. One wait, one place.** From the click until the review is usable, the
  wait lives in the storage card, in the review's own frame. Nothing about it
  goes to the shell message zone.
- **L2. One 150 ms gate, in ProjectSync.** The card opens only if the prepare
  is still running 150 ms after the click — the same interval
  `LoadingScaffold` uses — so a warm cache or an instant ending never flashes
  a card. That is the only gate on this path: the placeholders inside the
  card and inside the mounted review paint on their first frame. A second
  gate would open an empty card for another 150 ms, and the swap from the
  card to the mounted review would restart it.
- **L3. Words only.** Progress is the shipped scaffold caption,
  `<status-loader> {Label}…`, and nothing else: no numbers, no track, no step
  dots. The words name only what Mallard is actually waiting on:

  | Verb | Waiting on | Caption |
  |---|---|---|
  | Pull | plan (`planRestore`) | Reading the latest version from {storage} |
  | Pull | `activity` channel event | Loading sessions |
  | Pull | `comparison` channel event | Comparing with this computer |
  | Push | `preparePushReview` | Comparing this computer with {storage} |
  | Push | activity page read | Loading sessions |

- **L4. Nothing moves when the list lands.** The caption stands in the search
  field's slot (38 px, 12 px below it). The placeholders are the two-line
  session row at its landed height, and once the plan or context is read
  their count is the number of session rows the review will list. Pull
  also knows which rows exist only in storage (`create_local`): those
  placeholders sit under an *Only in {storage}* heading placeholder, as the
  landed rows do. Commit-timeline headings are not predicted; a Pull whose
  rows land in the timeline grows by those headings. The mounted review
  paints placeholders on its very first render, never null: the list is
  "loading" until its reads have settled, not only while a request flag is
  up.
- **L5. Nothing measures rows you cannot see.** Until the list is ready the
  header shows no meter, count, ✓ or trash, and the footer shows no gauge.
  Once ready, both meters behave exactly as shipped (header glance and
  footer gauge, including 0 B on an empty Pull).
- **L6. One label, both buttons.** The footer primary and the path-bar button
  show the same label and the same disabled reason: *Preparing…*, then
  *Loading sessions…*, then the review's own verb. Pull already shares these
  through `PullReviewPrimary`; Push gains the same path (list readiness moves
  into `pushReviewDisabledReason`, and the bar's Push gets `continueLabel`).
  **Push waits for its list** like Pull does.
- **L7. Cancel while reading.** While preparing, × is Cancel: the pending
  request is superseded, the card closes, the page unlocks at once, and the
  late result is dropped. Cancel is withdrawn when a write starts (Pull's
  metadata auto-accept). Once the review has mounted, × is the review's
  ordinary Close.
- **L8. Ready is announced, not celebrated.** The green *Done · … ready*
  message is removed. Each review carries a visually hidden `role="status"`
  line, present from mount and empty, whose text becomes *Pull review ready —
  nothing applied yet.* / *Push review ready.* when the list lands.

## Where

### Loading scaffold (`src/components/LoadingScaffold.tsx`, `src/App.css`)

- A new `sessions` shape beside `rows` and `detail`: the caption in the
  search field's slot (the field's box: `min-height: 38px`,
  `margin-bottom: 12px`, no rule) and the two-line session row at 68 px,
  with `count` (default 3) and `storageOnly` (how many of them sit under
  the *Only in* heading placeholder, default 0). It keeps the component's
  contract (region role, `{label}…`, aria-hidden bones) but is **ungated**:
  its callers are already behind ProjectSync's gate (L2). The default gate
  stays on `rows` and `detail`, which the Library test pins.
- `rows` stays as it is for Activity, the Library and both project-files
  pages; their own landed geometry is not part of this plan (Activity's Git
  project lands as a commit timeline, which was not measured).

### Session list (`ProjectActivityPage.tsx`)

- Selection mode only (the two reviews) uses `sessions`; Activity keeps
  `rows`.
- In selection mode the scaffold is keyed on settled state, not on request
  flags: shown while `activity === null && loadError === null` (label
  *Loading sessions*) or while `comparison === null && comparisonError ===
  null` (label *Comparing with this computer*). That covers the first
  commit — where every flag is still `false` and today's branch falls
  through to `null` — the gap between the two reads, and a failed read.
  The search field is not rendered while the scaffold is.
- A failed comparison in selection mode is a state of its own, never a blank
  and never an endless load: the page's error box (the same box as a failed
  first load) with *Try again*, which reruns `loadWorkspace`. The review
  reads it as not ready, with the reason *Sessions couldn't be compared with
  this computer.* on both buttons.
- `onReviewListReadyChange` reports `{ ready, failed }` instead of a
  boolean so both reviews can word the reason. `count` and `storageOnly`
  for the scaffold come from the review's props: Pull counts the plan's
  conversation rows and its `create_local` rows, Push the context's
  selectable sessions.

### Pull review (`RestorePlanView.tsx`)

- Header glance, selection bar count/✓/trash and footer gauge render only
  when the list is ready (L5); × stays.
- `primaryDisabledReason` / `primaryLabel`: *Loading sessions…* while not
  ready, the comparison sentence when it failed, then the shipped rules.
- The hidden status line (L8).
- The tools summary reads *Checking…* while `supportLoading`, in place of
  *N selected*; the inner *Checking readiness…* line goes.

### Push review (`PushResourceWorkspace.tsx`)

- Passes `onReviewListReadyChange` to its `ProjectActivityPage` and reports
  readiness up to ProjectSync (`onListReadyChange`), the way
  `deletionApproved` was lifted, because the path bar's Push must see it.
- `pushReviewDisabledReason` gains `listReady` (and the failure) ahead of its
  other reasons; the same meters rule (L5); the hidden status line (L8).

### The preparing card (new `src/components/sync-review/ReviewPreparing.tsx`)

- Resource-neutral, like the rest of `sync-review/`: props `title`,
  `flag?`, `caption`, `primaryIcon`, `onCancel`, `cancelDisabled`. Nothing
  is known about the rows yet, so it draws the default three placeholders.
  It and the mounted review both draw the ungated `sessions` scaffold, so
  the swap from one to the other paints placeholders on its first frame. It draws the review's header (title, meta or
  flag, ×), the `sessions` scaffold, and the footer with the disabled
  *Preparing…* primary, using the reviews' own classes so the swap to the
  mounted review keeps its geometry.

### ProjectSync (`ProjectSync.tsx`)

- A `preparingReview` state `{ kind, projectId, storageId, requestId,
  shown, writing }`. `shown` flips after 150 ms if the prepare is still
  pending. While `shown`, `inlineStorageReview` is the preparing variant:
  content `<ReviewPreparing …/>`, `continueDisabled`, `continueLabel`
  *Preparing…*, `onClose` = Cancel.
- Every ending clears `preparingReview` in the same update that sets the
  review, the storage notice, the shell message or the binding editor, so
  no intermediate frame is ever painted.
- Cancel: Pull already supersedes through `restoreRequest`; Push gets its own
  `pushPrepareRequest` ref (today it only has the global
  `hostWorkflowEpoch`). The awaited prepare races a per-request cancel
  promise, so `onPull` / `onPush` settle on Cancel and `runningAction`
  clears at once; the backend call finishes on its own and its result fails
  the request check.
- **`busy` stops covering read-only prepares.** Today both prepares hold
  `busy` until the backend returns, and `busy` is also the updater's
  no-restart gate — Cancel could not unlock anything while it held. The UI
  lock becomes `busy || preparingReview !== null` wherever `busy` is passed
  down; the updater keeps reading `busy` alone. Pull's auto-accept sets
  `busy` (and `writing`, which disables Cancel) around `acceptPullMetadata`.
- The `notify(\`Pull review ready…\`)` call is removed (L8).

### Path bar (`ProjectLinksWorkspace.tsx`)

- `workspacePullReadout` (the plan-phase `i/N · MB…` in the button) is
  removed; the bar shows the shared label (L6). The `push` and `apply`
  readouts during the transfer stay.
- The bar's Push shows `continueLabel` when a Push review or preparing card
  is open, as Pull already does.

The backend keeps emitting `project-sync-fetch-progress` with phase `plan`:
the same emitter serves `apply` and `push` and writes the activity log's
fetch lines. The frontend simply no longer reads the `plan` phase.

## Decisions

| # | Decision | By |
|---|---|---|
| D1 | Direction A: the card opens in place | Owner, 2026-09-26 |
| D2 | Progress is the Caption: words only, in the search field's slot | Owner, 2026-09-26 |
| D3 | Push's primary waits for the list, like Pull's | Owner, 2026-09-26 |
| D4 | Both meters stay as shipped once the list is ready, 0 B on an empty Pull included | Owner, 2026-09-26 |
| D5 | The plan-phase readout leaves the path-bar button | Follows D2: the caption carries no numbers |

## Not doing

- The transfer phase (Pushing… / Pulling… / Marking as reviewed…). It keeps
  its button readout and its "can't be cancelled" sentence.
- New backend stages. `prepare_push_review_with_phases` already times its
  phases for the log; turning them into caption words would need a new event
  and is a later, separate choice.
- Redrawing the `rows` scaffold for Activity, the Library and the
  project-files pages (the 53 px vs 68 px drift on Activity stays a known
  fact for a later pass).
- Direction B, and the Placeholders-only / In-the-header / Stage-line styles,
  which stay in the specimen as a record.

## Risks to verify

- **A new prepare right after Cancel** runs beside the superseded one. Both
  are reads (plus atomic cache writes); nothing serialises them. Check live
  that Cancel → Pull → Cancel → Pull settles on the last request only and the
  activity log shows no error.
- **Group headings.** A Pull whose rows land in the commit timeline grows by
  those headings when it lands (L4); only the *Only in* heading is predicted.
- **Screen readers.** The scaffold region, the preparing card and the hidden
  ready line must not announce the same thing twice: only the ready line and
  the path-bar status are live.

## Tests

Frontend (`npm run build`, `npm run test:frontend-integration`); no Rust
changes.

- `pull-review.integration.test.tsx`
  - with `activity` landed and `comparison` pending, the list shows the
    scaffold labelled *Comparing with this computer*, never an empty list;
  - a failed comparison shows the error box with *Try again*, and the primary
    reason is the comparison sentence;
  - until ready: no glance, no count, no gauge; primary *Loading sessions…*
    through `PullReviewPrimary`;
  - the tools summary reads *Checking…* while support loads;
  - a review mounted with a plan paints placeholders on its first render
    (before any effect has run), never an empty list.
- `push-budget.integration.test.tsx`: `pushReviewDisabledReason` returns
  *Loading sessions…* before `listReady`, and both buttons read it.
- `project-sync-cutover.integration.test.tsx` (or a new
  `review-preparing.integration.test.tsx`, registered in
  `scripts/run-frontend-integration-tests.mjs`):
  - no card before 150 ms; the card after it;
  - Cancel settles `runningAction`, closes the card, and a late result is
    ignored;
  - each ending replaces the card in one update;
  - no *Pull review ready* shell message; the hidden status line reads it
    once the list lands;
  - `busy` is not set by a read-only prepare; it is set around the Pull
    auto-accept, and Cancel is disabled there.
- `LoadingScaffold`: `sessions` renders on its first render (no gate) with
  the region and `{label}…` contract; `rows` and `detail` keep the 150 ms
  gate the Library test pins. The `sessions` row is 68 px, measured by
  computed style in the specimen harness, not asserted as source text.

Live: `npm run tauri dev`, Pull and Push on a real storage with a cold cache
(to see the card) and a warm one (to see no flash), Cancel mid-prepare, and
a screenshot of each Pull step for the PR.

## Checklist

Layer 1 — inside the review (shippable alone; fixes the screenshot):

1. Ungated `sessions` scaffold shape, used by the two reviews only.
2. Selection-mode scaffold keyed on settled state (first frame, the gap
   between reads, failures); comparison failure state; `{ ready, failed }`
   readiness.
3. Pull: meters/count/✓/trash/gauge held until ready; shared label and
   reason; tools *Checking…*.
4. Push: readiness lifted to ProjectSync; `pushReviewDisabledReason` and the
   bar's `continueLabel`; meters held.
5. The Done message removed; hidden ready lines.

Layer 2 — the card opens in place:

6. `ReviewPreparing` in `sync-review/`.
7. `preparingReview` state, the 150 ms gate, endings in one update.
8. Cancel: request refs, the cancel race, `busy` narrowed to writes, the UI
   lock `busy || preparingReview`.
9. Path bar: plan-phase readout removed; shared label on both verbs.
10. Tests above; live pass with screenshots.

## What landed (2026-09-26)

Both layers, frontend only. `npx tsc --noEmit`, `npm run build` and
`npm run test:frontend-integration` (434/434) are green; no Rust changed.

**Files.** `LoadingScaffold.tsx` (`sessions` shape), `App.css` (its rules,
the removed `.sync-pull-support-loading`, the lone-close rule),
`ProjectActivityPage.tsx`, `RestorePlanView.tsx`, `PushResourceWorkspace.tsx`,
`SyncReviewSelectionBar.tsx` (`listPending`), `ProjectSync.tsx`,
`ProjectLinksWorkspace.tsx`; new `sync-review/ReviewPreparing.tsx` and
`project-sync/reviewPreparing.ts`; tests in `pull-review`,
`project-activity`, `push-budget`, `resource-inventory` and the new
`review-preparing.integration.test.tsx`.

**Deltas from the plan.**

1. **The session row is 63 px, not 68.** Measured on the real
   `RestorePlanView` in a Vite harness; 67/68 came from the specimen's
   hand-drawn rows. The `sessions` row pads 15/17.
2. **The `sessions` scaffold is block flow**, not the scaffold's flex
   column: the caption's 12 px margin must collapse into the storage-only
   group's the way the search field's does, or the list lands 12 px short.
   Measured after both fixes: the review is 515 px from its first frame to
   its landed rows, and the tools row and footer do not move.
3. **`busy` was not narrowed.** Cancel races the awaited call against a
   per-prepare cancel promise, so the prepare returns at once and its own
   `finally` releases `busy` and the path bar's `runningAction`; the late
   answer goes nowhere. `busy` still covers the read-only prepare (the
   updater waits for it, as before). No `pushPrepareRequest` ref was needed.
4. **The prepare lifecycle is a module**, `reviewPreparing.ts`
   (`createReviewPreparing`: the 150 ms gate, the race, `writing`,
   idempotent `finish`, superseding), tested with a fake clock; ProjectSync
   mirrors its events into `preparingReview`.
5. **Compare again uses the card too** (`replanPull`). A plan that fails
   there used to set an error inside a review that had already been
   unmounted; it now lands as the card's *Pull couldn't start* notice.
   Cancelling it releases `busy` in `planRestore`'s cancel branch, since
   closing the review supersedes the request that owned the lock.
6. **The Done receipt became `setNotice(null)`**: an older shell receipt
   would otherwise sit above the new review.
7. **A failed list's reason** on both buttons is *The session list didn't
   load. Try again in the list.* — the list carries the error and its retry.
8. **Placeholders are capped at 12 rows**; a longer list still lands.
9. **Close alone draws no divider** (`:only-child`), in the card and while a
   review waits on its rows.
10. **A prepare started from an open review skips the gate.** Compare again,
    Rescan on the files step, and the Push review's *Open Pull review*
    unmount that review at once; waiting out 150 ms would drop to Activity
    (and start its load) between the review and the card. `begin({
    immediate })` shows the card in the same update.
11. **No agent profile connected reads as a failed list**, not an endless
    load: nothing is ever read, and the list itself says why. The bar
    already refuses Push/Pull there; this covers the other entry points.

**Measured in a Vite harness** (real components, `projectSyncApi` reads
monkeypatched and released by hand; deleted afterwards). Pull: first frame
bones (not null), *Comparing with this computer…* between the reads,
landed rows with meters, 0/3 and *Mark as reviewed*, and the hidden line
announcing *Pull review ready — nothing applied yet.*; 515 px from first
frame to landing. Push: card → first frame → landed all 353 px, header
48 px throughout; both buttons read *Loading sessions…* until the rows
land, then Push enables.

**Known limitation.** Clicking *Project files ›* before the Push list lands
unmounts the list in its loading state, so Push stays *Loading sessions…*
on the files step until the sessions step is shown again. Pull had the same
trap before this change; D3 makes it new for Push.

**Not verified yet.** The live Tauri pass (`npm run tauri dev`), which also
gives the PR its screenshots:

- Pull with a cold cache: the card opens after 150 ms;
- Pull with a warm cache: no card, the review opens directly;
- Cancel mid-prepare, then Pull again: settles on the new request, no error
  in the activity log;
- Compare again from inside a Pull review: the card replaces the review with
  no Activity frame between;
- Push through to an enabled button.

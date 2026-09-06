# One notice system: combine, name the type, stop announcing

2026-08-22. **IMPLEMENTED same day — see §10 for deltas.** Clean cutover:
the old notice classes are deleted, not left beside the new ones. The dead
backend warning channels of §5.2 are the one part NOT done; §10 says why.

Scope came from the session-detail warning stack. A 13-agent audit
(six finders, one adversarial verifier each, one completeness critic;
136 distinct sites) found the same three defects across the whole app,
so the fix is a system, not a page.

## 1. The rules, and where they come from

| | Rule | Source |
|---|---|---|
| **(a)** | Never more than one notification banner in a zone; combine the messages into one. | [GOV.UK notification banner](https://design-system.service.gov.uk/components/notification-banner/) |
| **(b)** | Warning tone is for information that may require action; plain status is *informative*. Name the type in the text so meaning does not rest on colour or icon. Short title, details in the body. | [USWDS alert](https://designsystem.digital.gov/components/alert/) |
| **(c)** | Content already present when the view renders is **not** a status message — it must not be a live region. Standing information is `role="region"` with a name, or no role. | [WCAG 2.2 SC 4.1.3](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html) |
| **(d)** | A collapsing notice needs a real `<button>` with `aria-expanded`. | [W3C APG disclosure](https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/) |

## 2. What the audit found

- **44** `sync-callout` sites in shipped components, plus a dozen more
  notice families that are not callouts at all.
- **82** live-region attributes across **22** component files.
- **Ten distinct visual treatments** for "something is wrong", counted in
  the stylesheets: the callout base (App.css:1388), the outcome banner
  (:6416), the storage-notice band (:6579), the shell banner (:1427), the
  toast (:1478), the path-repair notice with two context restyles
  (:3936, :2605), the path-menu banner (:3322), two different dashed state
  boxes with different numbers (:5457, :8247), the borderless field error
  (:4890) and its catalog twin (:7193).
- **Worst single screen: six simultaneous banners spanning three
  components** — the Pull review on its project-files step
  (ProjectSync.tsx:3500/3513/3549 shell rows, RestorePlanView.tsx:484/498,
  nested ProjectFilesReviewPage.tsx:256), before any per-row chip.

Rule attributions across the 183 raw findings: (a) 70, (b) 81, (c) 71,
(d) 9; 46 sites clean.

### 2.1 The four stacks that render N boxes from a loop

| Site | Source |
|---|---|
| [SessionDetailPage.tsx:426](../src/components/project-sync/session-detail/SessionDetailPage.tsx#L426) | `detail.warnings` — the reference case |
| [ProjectActivityPage.tsx:1519](../src/components/project-sync/ProjectActivityPage.tsx#L1519) | one box per provider whose access ≠ `bound_ready` |
| [ProjectActivityPage.tsx:1544](../src/components/project-sync/ProjectActivityPage.tsx#L1544) | one box per `comparison.provider_errors` entry |
| [StorageCatalogPage.tsx:1668](../src/components/storage-catalog/StorageCatalogPage.tsx#L1668) | one box per recovery message — an exact twin of the reference |
| [StorageCatalogPage.tsx:1491](../src/components/storage-catalog/StorageCatalogPage.tsx#L1491) | one box per claimed item in the Pull review |

### 2.2 The zones where independent boxes pile up

- **Session detail** — up to `1 + 1 + N`: action error
  (SessionDetailHeader.tsx:134), stale banner (SessionDetailPage.tsx:416),
  then the warning stack.
- **Activity page** — six sibling slots in one `<section>`
  (ProjectActivityPage.tsx:1485–1544), two of them maps.
- **App shell** — three rows above every screen
  (ProjectSync.tsx:3500 backend, :3513 generic error, :3549 toast). These
  are genuinely independent, which is why rule (a) cannot be satisfied
  file-by-file: the shell zone and the panel zone must be combined together.
- **Storage settings column** — four error callouts down one column
  (ProjectLinksWorkspace.tsx:1424/1443/1454/1460), with an inconsistent
  missing role on the last.
- **Storage card** — `SiblingRepoNotice` plus one of five mutually
  exclusive `activeNotice` kinds (ProjectLinksWorkspace.tsx:2292–2388).
- **Setup workspace, settings page, bundle dialog** — same shape; the
  bundle dialog always shows at least one amber box
  (BundleConnectionDialog.tsx:134).

### 2.3 The CSS defect behind the ragged pills

`.sync-callout` is written as a *page banner*: `max-width: 1180px;
margin: 0 auto` (App.css:1388). Every call site inside a narrower column
inherits page-centring. In a grid — which `.session-detail-warnings`
(App.css:8283) and `.session-detail-header` (App.css:7298) both are — auto
inline margins disable `justify-self: stretch`, so each box shrink-wraps to
its own text and floats mid-column. That is the ragged-width rendering in
the screenshot.

Two rules already fight this by hand, each with a comment explaining it
must out-specify the theme: `.sync-callout.sync-outcome-notice`
(App.css:6416) and `.sync-callout.sync-storage-notice` (App.css:6579) both
re-declare `margin: 0` and `max-width: none`. That is the same fix written
twice, and the session-detail sites are a third and fourth site needing it.

Also confirmed: `.sync-callout.info` **has no rule anywhere**, yet
StorageCatalogPage.tsx:1274 ships `className="sync-callout info"` — the
"informational" tone renders as the neutral base today.

## 3. The design

### 3.1 Vocabulary

**Severity** is a four-rung ordered ladder — `danger > warning > info >
success`. It is the only tone input. `ActivityLogLevel`
(activity_log.rs:35) already derives `Ord`; copy that ladder rather than
inventing a second one.

**Role is derived, never authored.** The component takes `timing`, not
`role`:

| timing | markup | when |
|---|---|---|
| `standing` | `role="region"` + `aria-label`, or no role when it has a heading | the content is there when the view first renders |
| `progress` | `role="status"` | appears after an action, advisory |
| `blocking` | `role="alert"` | appears after an action and stops the flow |

This is the whole of rule (c), enforced by the type instead of by review.

**The type is named in the text**: `Error ·`, `Warning ·`, `Note ·`,
`Done ·` before the title. Rule (b), and it also fixes the sites where
severity is currently carried by an icon name alone
(ProjectActivityPage.tsx:1519) or by a 5px dot with no matching CSS rule
(SyncReviewTabs.tsx:82).

### 3.2 Two components

```
src/components/notice/Notice.tsx     — one box
src/components/notice/NoticeZone.tsx — N messages, one box
```

`Notice`: severity, timing, title, body, optional action button, optional
`<ul>` of details, optional disclosure. The disclosure copies
[SyncOutcomeNotice.tsx:117](../src/components/sync-review/SyncOutcomeNotice.tsx#L117)
verbatim — a real `<button type="button" aria-expanded aria-controls>`,
already APG-correct. It is the only rule-(d) implementation in the app and
it is right; do not rewrite it, generalise it.

`NoticeZone`: takes `NoticeItem[]`, renders **one** `Notice` at the highest
severity present, with the messages as a `<ul>` and a count in the title.
This is candidate **S** from `warning-options.html`, generalised. Zones
collapse `1 + 1 + N` into `1`.

A **zone** is the smallest DOM region that owns all the notices for one
decision. Shell rows and panel banners are one zone even though three
components render them today — §4 phase 4.

### 3.3 CSS

One recipe, `.notice`, replacing `.sync-callout` and the treatments that
are only "a tinted box": the outcome banner, the storage-notice band, the
shell banner, the path-repair notice and its two restyles, the path-menu
banner. The two dashed state boxes stay (they are empty/loading states, not
notices) but converge on one set of numbers. The borderless field error
stays as its own treatment — a field error is not a page notice — and is
handled in phase 6.

`margin: 0 auto` does not survive. A notice fills its container; page-width
capping belongs to the page container. That deletes both hand-written
overrides and both un-written ones.

**New tokens needed.** RetroTheme.css defines only `--danger-soft` (:171,
light :286) and `--warning-soft` (:172, light :287). An informative rung
needs `--info-soft` and a success rung `--success-soft`, in both palettes,
through the WCAG contrast gate the palette work already uses.

## 4. Landing order

**Phase 0 — foundation, no call sites touched.** Tokens, `Notice`,
`NoticeZone`, `.notice` CSS. Verify by computed style, not by source text:
the CSS gates in `tests/frontend/` assert stylesheet *text* and cannot
catch a cascade bug — RetroTheme.css:2480 re-tints both tone borders from
24%/22% to 45%, and RetroTheme.css:770 out-specifies the shell banner's
border at (0,2,0).

**Phase 1 — session detail.** The three callouts become one zone. This is
the approved S. Reference implementation for everything after it.

**Phase 2 — the other stacks.** Activity page's six slots and two maps;
catalog page's recovery map, claimed-item map and shared `notices`
fragment (StorageCatalogPage.tsx:1272, spliced into three surfaces).

**Phase 3 — review banner zones.** Push (PushResourceWorkspace.tsx:574),
Pull (RestorePlanView.tsx:484), catalog Push (StorageCatalogPage.tsx:1334).
The Push zone's own source comment already states everything in it is
plan-derived — i.e. standing content behind a `role="status"`.

**Phase 4 — the shell/panel zone.** The only structural change: one zone
that owns ProjectSync.tsx:3497–3551 and the panel banners nested under it.
Hardest, and the only phase that can be deferred without leaving the
system half-applied.

**Phase 5 — ARIA sweep.** The non-callout live regions, with `timing`
replacing hand-written roles. Unambiguous rule-(c) violations found:
ProjectLinksWorkspace.tsx:2496 (Suspense fallback whose region and text
mount together), ConversationPathRepairNotice.tsx:55, SyncPathBar.tsx:192
(standing prose, and an invalid ARIA child of `role="menu"`),
ProjectSync.tsx:3380 (an entire screen wrapped in `role="alert"`),
ProjectFilesReviewPage.tsx:221/256, ProjectLinksWorkspace.tsx:1812
(a standing count — its already-correct twin is StorageCatalogPage.tsx:1213).

**Phase 6 — field errors.** Keep the borderless treatment; add the named
type and wire `aria-describedby` from the input. Sites:
ProjectLinksWorkspace.tsx:1323, StorageCatalogPage.tsx:363/1658,
ProjectSetupWorkspace.tsx:900/987, ProjectBindingEditor.tsx:364,
AddCatalogScopeWorkspace.tsx:370. Note that 987 and 370 borrow the *field*
error class for a *page*-level failure — that stops.

**Phase 7 — backend.** §5.

## 5. Backend

### 5.1 Severity on the wire

`SessionDetailWarning` (traits.rs:299) is `{ code, message }` with no
severity. Add `severity`, defaulted to informative, set per code. A
frontend map keyed on warning strings would rot the moment a provider adds
a code.

Classification for the session-detail codes: truncated previews and missing
tool results change what the user can read → **warning**; a rebuilt cache
or an inferred phase is a fact about how the data was produced → **info**.

Reconcile with the frontend-only severity that already exists —
ProjectActivityPage.tsx:453 declares `severity: "warning" | "danger"`,
computed at :500 and consumed at ProjectLinksWorkspace.tsx:1812.

Types that already carry a discriminant and only need mapping:
`ProviderActivityState.access`, `RemoteContinuity`, `ConversationBlocker`
(8 variants, no ordering), `ProjectFileSyncEligibility`,
`CatalogPullDisposition`, `CatalogActionResult`, `CodexAppRefreshOffer`.
`BundleReadinessIssue` carries `severity` as an **untyped String** built
from `if failed { "error" } else { "warning" }` (commands.rs:12916, 12986,
13038, 13290) — that becomes the shared enum.

`CatalogBlocker` (domain.rs:2587) is serde-tagged by code but every Tauri
boundary flattens it to a plain String, so the typing buys nothing today.

### 5.2 Dead channels — delete, per the clean-cutover rule

Verified as never read by any frontend file:

- `ProjectActivity.warnings` (project_activity.rs:161) — **ten** push sites,
  zero readers.
- `ThreadSyncComparison.warnings` (commands.rs:475) — never rendered, and
  commands.rs:7454 clones every `provider_errors` value into it, so each
  provider failure is transmitted twice.
- `ResourceInventory.warnings`, `BundleSnapshotSummary.warnings`,
  `SetupDraftList.warnings`, `CatalogScopeResourcesDto.warnings`.
- `CodexConversationPathAudit.warnings` — the notice renders `blockers[0]`
  only.
- `UnmappedSessionReference.reason` — the frontend reads only `resource_id`.
- `BundleReadinessIssue` entries below `error` — dropped at src/components/project-sync/syncOutcome.ts:191.

Each deletion must check the test halves before removing them: a test named
after a dead field may hold the only cover for a live one.

## 6. Test surface

Four assertions pin markup this change rewrites and **will fail**:

- `tests/frontend/resource-inventory.integration.test.tsx:591` — asserts
  exactly one `sync-sync-review-pull-holds`.
- `:595` — matches `class="sync-callout sync-sync-review-pull-holds"
  role="status"` as a literal string, so even reordering the class list
  breaks it.
- `tests/frontend/project-activity.integration.test.tsx:644` — matches
  `role="status">Comparing threads with…`.
- `:1786` — matches `sync-history-provider-notice`.

CSS source-text gates in `catalog-scope-removal.integration.test.tsx:75-93`
and `project-sync-cutover.integration.test.tsx:536-542` pin
`.sync-backend-banner` and `.sync-notice` rules and must move with the CSS.

## 7. Already correct — do not touch

- **SyncOutcomeNotice.tsx:117** — the disclosure. APG-correct; it is the
  pattern to generalise.
- **ThreadStatusIcon.tsx:36-47** — a deliberate non-colour severity channel
  (filled disc vs outlined ring) with a WCAG 1.4.1 rationale in the comment.
  Prior art for rule (b).
- **ProjectFilesReviewPage.tsx:373** — the one place N messages already
  collapse into a single `<details>` with a counts summary.
- Both modals are native `<dialog>` with `aria-labelledby`.
- **StorageCatalogPage.tsx:1213** — the path-bar chip that already avoids
  the live region its twin still uses.

## 8. Open decisions

1. **Severity source** — Rust field (recommended) or frontend map?
2. **Scope** — phases 0–3 (every stack and review zone), or through
   phase 4 (the shell zone, structural)?
3. **Dead channels** — delete as §5.2, or wire them into the new zones?
4. **Native `confirm()` kind** — SettingsPage.tsx:157/200 and
   ProjectSync.tsx:1198 pass `kind: "warning"`; StorageCatalogPage.tsx:1009
   and ProjectSync.tsx:1594/1636/1734 pass none. Same ladder, second
   vocabulary. Unify or leave.

## 9. Caveats

Every `presentAtLoad` judgement in the audit is a source reading of state
initialisers and effect order, not an observed render; the audit ran no
app, test or browser. Per the repo's standing rule, CSS claims are verified
by **computed style** before they are trusted — the source-text CSS tests
cannot catch a cascade bug, and this plan turns on two cascade facts
(App.css:1388's auto margins in a grid, and RetroTheme's overrides at
:770 and :2480).


## 10. What landed, and how it differs from the plan

Full bar green: 237 frontend integration tests (up from 232 — four are new),
316 `mallard-infrastructure`, 235 `src-tauri` lib, catalog e2e, `clippy
--all-targets -D warnings`, `cargo fmt --check`, and the production build.

**Two components, as planned**, in `src/components/notice/Notice.tsx`.
`.sync-callout` is gone from both stylesheets and from every component.
70 sites converted across 22 files; live-region attributes in
`src/components` fell from 82 to about 20, and every one that remains is a
genuine event.

### Deltas

1. **A fourth timing, `inline`, with no role.** Two shapes need it and
   neither is standing content that wants finding: a screen that *is* the
   failure (heading, prose, box and Retry inside one labelled `<main>` —
   ProjectSync.tsx:3392), and a marker that repeats per row
   (ProjectFilesReviewPage.tsx:392, where a blocked scan draws dozens and a
   landmark each would be worse than none). It is documented as the case to
   justify before reaching for.
2. **`NoticeItem.mark`.** The Push review's held-for-Pull banner led with
   the rows' own divergent status mark, not a warning triangle — that mark
   is what ties the banner to the rows it speaks for. A caller can replace
   the severity glyph, which is safe precisely because the rung is named in
   the text.
3. **Mixed zones label every message with its own rung.** A box wears one
   tint, so an `info` note inside a `danger` zone read as an error. Each
   message now prints `Note ·` / `Warning ·` for itself when the zone is
   mixed; a zone where every message agrees prints it once, in the title.
4. **The shell banner keeps the theme's frame, not the rung's border.** It
   is a panel in the workspace stack, a sibling of the path bar and the
   page, and it carries their outline — so its severity lives in the fill,
   the ink and the named type. Its fills are opaque
   (`.notice.sync-backend-banner.<rung>`), because a translucent tint there
   sits on the frame gutter rather than on paper and cost 1.4 of contrast.
5. **One zone per *scope*, not per screen.** Phase 4 combined the shell's
   own three rows into one zone. It did **not** merge the shell zone with
   the panel zones nested under it: an app-level failure, a review-panel
   gate and a per-file marker are different scopes, and folding them into
   one box would put "the backend is unreachable" in the same sentence as
   "this file needs a Pull". The worst case is now three boxes on one
   screen, down from six.
6. **A palette fix the plan did not anticipate.** Light `--danger-text` was
   picked at 4.9:1 against paper, but a danger notice prints it on
   `--danger-soft` *over* that paper, where it measured 4.24:1 — under the
   4.5 this palette holds itself to, in the rung that matters most and is
   now used in dozens of places. Darkened `#c73d28` → `#be3824`. A new gate,
   "every notice rung reads on its own fill" in
   `tests/frontend/css-hygiene.integration.test.tsx`, composites each rung's
   translucent tint onto `--surface-card` and measures the ink against the
   result, in both themes. It was confirmed to fail on the old value.
7. **`SessionDetailWarning.severity` shipped as planned**, carrying the
   shared `NoticeSeverity` from `mallard-domain` (ordered, quietest first,
   `Ord` derived for the zone's comparison). The provider builders take the
   rung explicitly rather than mapping it from a code string:
   `mark_all_lower_bound` and `mark_tools_lower_bound` are always `Warning`
   because a degraded count means the reader is provably seeing less;
   `message_phase_inferred`, `legacy_message_phase_inferred` and the three
   timeline-cache notes are `Info`.
8. **Specimens.** `warning-options.*` is deleted — the question it explored
   shipped, and it referenced the retired classes. `notice-system.html`
   replaces it: every rung, every count, a narrow column, both palettes.
   `error-banner-live.tsx` was rebuilt on the new zone.

### Not done: §5.2, the dead backend channels

Eight channels are still wired up and still unread. Deleting them is one
open decision, and it is not the obvious call it looked like from the
audit, because they are not all the same thing:

- `ProjectActivity.warnings` carries `Skipped '<file>': <error>` — the
  activity list is **incomplete** and nothing says so. That is information
  the reader wants, and the page now has a zone that can hold it. Wiring it
  in is the better fix than deleting it.
- `CatalogScopeResourcesDto.warnings` is the same shape, one scope up.
- `ThreadSyncComparison.warnings` is genuinely redundant: commands.rs:7454
  clones every `provider_errors` value into it, so each provider failure
  crosses the wire twice. Delete this one.
- `UnmappedSessionReference.reason` explains why a session has no commit;
  the rows are grouped by that fact already, so the sentence adds little.
- The rest (`ResourceInventory`, `BundleSnapshotSummary`, `SetupDraftList`,
  `CodexConversationPathAudit.warnings`) need a per-channel read before
  anyone can say which of the two they are.

Removing `ProjectActivity.warnings` also means unpicking a `&mut
Vec<String>` threaded through eight functions — so "delete" and "wire up"
are comparable amounts of work, and they point in opposite directions. It
wants the call, not a default.


## 11. The review round

A 16-agent adversarial pass (one reviewer per file group, each finding put
through a refutation agent that had to reproduce the consequence from the
source) ran over the finished conversion. **19 findings survived
refutation, and all 19 are fixed.** They fell into four shapes, and every
one of them compiled and passed the suite — none would have been caught by
the gates alone.

**Cascade, 5.** A wrapper class kept on a `Notice` whose CSS is declared
later in App.css beats `.notice` at equal specificity. The per-row "Pull
required" strip became a rounded amber capsule with a dashed top edge
crossing three solid ones; `SiblingRepoNotice` drew a second outline inside
the storage block the full-bleed band exists to avoid; the "Unlink storage"
button lost its red, because the red came from a wrapper div the zone no
longer needs, leaving a destructive control in the same ink as the "Try
again" beside it. Verified fixed by computed style, not by reading the
source: `.notice.sync-project-file-conflict` composites to a transparent,
square, dashed-top strip; `.notice .btn.danger` resolves to `#be3824`.

**Timing, 8.** The recurring error was reading "derived from the plan" or
"written by the load" as "present when the view renders". The activity
page's failed-load screen clears `loadError` on the way into a mount
effect, so it is *never* there at first paint — a region left a
screen-reader user with the polite "Loading…" and then silence, unable to
tell a finished retry from a failed one. The shell's backend banner is
written by the same `refresh()` that "Reload app data" runs. Both review
footers' sentences appear on a click, at the exact moment the button beside
them goes dead. The session-detail zone keyed only on `actionError`,
leaving the snapshot-changed banner — whose Refresh is the only way out —
silent.

**Lost behaviour, 3.** A failed project-bundle listing short-circuited the
terminal branch of the Repositories ternary, which also holds the Catalog
rows, so an unrelated failure rendered a heading over nothing. The log
level slot had no CSS, so the empty track meant to keep the columns
straight collapsed and every marked row shifted 12px. And one I caused: an
over-eager dead-rule sweep merged two selectors and dropped
`.sync-host-recovery` from the empty-state width cap, applying it to every
empty state in the app.

**Severity, 1.** The bundle dialog pushed its unconditional "connecting
changes only Mallard metadata" reassurance at `warning`, so the dialog
printed "Warning ·" over a plain fact every time it opened — and pinned the
box at `warning` so a real mismatch below could not raise it.

## 12. Placement: under the bar, on every screen (2026-09-04)

The shell zone rendered before the page, so on screens that draw their own
path bar — project, catalog, storages, settings, the provider picker — it
sat *above* the bar, while draft and setup screens, where the shell draws
the titlebar itself, had it *below*. The bar is the window's title bar
(Tauri overlay, traffic lights at y 35), so the right place is under it:
Material puts a banner below the top app bar, Fluent's MessageBar sits
below the command bar, Polaris and GOV.UK put a page banner under the page
header. Only site-wide web announcements (the USWDS site alert, the
Atlassian banner) go above a header, and there the header is page content,
not window chrome.

What landed:

- `PathBarBelow` (SyncPathBar.tsx): a context the shell fills with the
  rendered zone. The bar returns its `<header>` and then the slot, so DOM
  order, tab order and paint order all read bar → zone → page. No page
  changed.
- ProjectSync builds `shellNoticeZone` once and draws it in exactly one of
  two places: after its own titlebar when `showsPathBar` is false, or
  through the provider when it is true.
- `.sync-backend-banner` z-index 8 → 5. It now stands under the bar (6), so
  it must stack under it too: the bar's menus live in the bar's stacking
  context, and a higher banner painted over any menu dropped onto it.
  Measured in a specimen built from the real components on the real
  stylesheet: `elementFromPoint` in the overlap hits the menu; bar 8–60,
  zone 68–112, page from 120.
- Trade-off: the zone remounts with the bar when one path-bar screen
  replaces another, so a standing `role="alert"` re-announces on that
  navigation. Accepted over CSS `order`, which would have left the zone
  before the bar in the tab sequence and the stacking unfixed.
- Gate: `tests/frontend/shell-notice-placement.integration.test.tsx` —
  static render order for the bar and two real pages, source order for the
  shell's two branches, and a value-measured z-index comparison.

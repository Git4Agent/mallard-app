# Mallard UI audit — 2026-09-01

Date: 2026-09-01
Audited code: current working tree (dirty), base commit `789eea5`
Application version: `0.1.2`

## Executive result

This audit covers the **uncommitted Codex retheme** sitting on top of `789eea5` — a
1172-line `App.css` rewrite plus the switcher-menu and session-detail punchcard changes.

The frontend suite is **green on this tree: 269/269 tests plus `tsc`**. Every finding below
is therefore something the existing gates do not catch, and that is the audit's main result:
the gates read source text, so they are blind to defects composed *across* rules.

Four things matter most:

1. **`--surface-row-active` is a systematic blind spot.** Three independent findings converge
   on it: two status badges, the danger ink, and session-detail tool names all fall under the
   4.5:1 floor once a row is selected or expanded. The contrast gate never measures that surface
   for badges — though it does for `--text-3` in the same file.
2. **The retheme nests duplicate ink frames.** The new box grammar is applied to both a container
   and its direct child in the Files review and the Older-activity fold, drawing two (sometimes
   three) concentric 2px borders.
3. **The startup failure state is inaccessible.** No live region anywhere on the page, so a
   screen-reader user is never told the app failed to start — and the raw exception
   `Cannot read properties of undefined (reading 'invoke')` is shown as user-facing copy.
4. **New switcher menus have no keyboard contract.** The search field the retheme added cannot
   be reached by keyboard at all, and the panel has no max-height under an `overflow: hidden` root.

## Scope and method

Two independent evidence streams:

**Runtime (main session).** The real `App.css` (1605 rules) rendered in a browser against the
running dev server; computed-style measurement, full token resolution through `var()` chains,
and alpha compositing. Production DOM captured from the one production surface that renders
without the Tauri backend (the startup-failure screen). Findings prefixed `R`.

**Static fan-out.** 15 finder lenses across CSS/cascade, theme parity, keyboard, ARIA, the
uncommitted diff, React correctness, state coverage, layout, copy, destructive safety, prior-audit
regression, dead UI, and test-gate blind spots. Every finding was then put to two adversarial
reviewers — one checking code reality, one checking impact — with a tiebreak on disagreement.

**This run was stopped early by the user.** It produced 329 raw findings, of which 163 distinct
findings entered verification. At the stop: **22 confirmed** (13 distinct after dedupe),
**24 killed**, and **114 caught mid-tiebreak**. No runtime, native-Tauri, VoiceOver,
forced-colors, or text-scaling verification was performed.

### The 114 unresolved findings

All 114 split the *same way*: the code-reality reviewer independently reproduced the defect and
the impact reviewer refuted it on "does this matter" grounds. **Zero** were refuted on the facts.
So the citations are sound; what is unsettled is only whether each is worth fixing. Severity as
claimed: 3 high, 41 medium, 70 low.

Spot-checking the three high ones showed the impact lens doing real work — in each case it found
a later cascade rule that wins, or a code path no user can reach. Treat the 114 as a backlog of
accurate observations of unproven consequence, not as pending defects.

## Runtime findings

#### R1 — Status badge gate measures the wrong row surface (light theme)

`tests/frontend/css-hygiene.integration.test.tsx:98` · contrast

The gate measures the seven `--thread-status-*` badges against `--surface-row-hover`, with a
comment calling it *"the harder of the two surfaces a badge sits on."* That is inverted:
`--surface-row-active` (`#f0e3d5`) is **darker** than `--surface-row-hover` (`#faf2e9`), so for
dark ink the active row is the harder surface.

Measured on the surface the gate skips:

| badge | hover | active |
|---|---|---|
| `divergent` `#765ba7` | 4.95 ok | **4.35 FAIL** |
| `conflict` `#be3824` | 5.01 ok | **4.41 FAIL** |

(synced 5.29, cloud 4.59, local 4.95, pending/unavailable 4.72 pass. Dark theme passes on both,
worst 5.50.)

**Fix.** Add `--surface-row-active` to the badge assertion's surface list — the `--text-3`
assertion in the same file already covers it — then lift the two failing tokens.

#### R2 — `--danger-text` violates the invariant its own comment asserts

`src/App.css:8170` · contrast

The comment states the signal hues hold ">=4.5:1 on every paper surface down to
`--surface-row-active` (#f0e3d5)". Light `--danger-text` `#be3824` measures **4.405** there.
success 4.585, warning 4.562 and info 4.592 all clear it, so danger is the lone exception — and
it is the rung that carries error copy. The gate misses it because it only measures signal inks
on their own `--*-soft` fills over `--surface-card`, never on a row.

Note this token was already darkened `#c73d28` → `#be3824` to fix the notice-fill case. This is
a second, independent failure surface for the same ink.

#### R3 — Startup failure is never announced to assistive technology

Production DOM capture · aria-semantics

The failure screen has `h1`, `main` and `lang="en"` — but the page contains **zero**
`aria-live`, `role="status"` or `role="alert"` elements. The error node is a bare
`<div class="notice-line">`. The state replaces the startup content after mount, so a
screen-reader user gets no announcement and must discover the changed heading manually.

**Fix.** Give the failure notice `role="alert"`, or have the startup sequence write into a
persistent `role="status"` region.

#### R4 — Internal JavaScript exception shown as user-facing copy

Production DOM capture · copy

`.notice-line` renders `Error · Cannot read properties of undefined (reading 'invoke')`
verbatim. It names an implementation symbol and states no cause the user can act on. Flagged in
the 2026-08-30 audit; still live.

#### R6 — LATENT: base `.btn-primary` carries a failing colour, masked by call-site convention

`src/App.css:334` · cascade · **not a live defect**

All 115 `App.css` rules that set both `color` and `background` in one block were measured in
both themes. **113 pass.** Both failures are the same rule: `.btn-primary { color: var(--text-0) }`
= 3.30 light / 3.10 dark. It is overridden at `App.css:8523` by
`.btn.btn-primary, .sync-app .btn-primary { color: var(--button-primary-text) }` = 5.11 / 5.33
(hover 5.99 / 6.97). All 16 production call sites use the doubled class `btn btn-primary` and sit
inside `.sync-app`, so the correct token always wins.

It is a latent trap: one `className="btn-primary"` without `btn`, or a portalled button outside
`.sync-app`, silently drops the label to 3.1:1.

**Fix (cheap).** Change the base rule to `color: var(--button-primary-text)` so the fallback is
safe. No visual change to anything shipping today.

### Verified as fixed — do not re-report

- **Focus indicators** (prior M04): `--focus-ring` measures 12.49 light / 12.51 dark worst-case
  across 11 opaque surfaces, far above the 3.0 non-text floor. 62 `:focus-visible` rules against
  3 `outline: none`, all three benign.
- **Muted text** (prior H03): `--text-2` / `--text-3` clear 4.5 on every declared opaque surface
  (worst 4.72 light, 5.74 dark).

## Findings

### High priority

#### H01 — Files review: the flat row list draws a second full frame flush inside the file tile, doubling the ink edge and clipping its own shadow

`src/App.css:6484` · cascade · confirmed unanimous

```
.sync-project-file-tile > .sync-history-thread-list { gap: 0; }
.sync-project-file-tile .sync-history-thread-card {
  border: 0;
  border-radius: 0;
  background: transparent;
}
```

**Impact.** On the Push and Pull Files review (ProjectFilesReviewPage.tsx:571-576 nests `sync-history-thread-list flat` directly inside `sync-project-file-tile`), the retheme's new `.sync-history-thread-list.flat` box (L9804: 2px ink border, 14px radius, cast shadow) renders inside the tile's identical box. Measured in a browser against the real stylesheet: tile box x=42→852, list box x=44→850 — zero gap, so the two 2px ink lines abut as a 4px double rule down both sides and across the bottom, the two 14px corner arcs cross each other, and the tile's `overflow: hidden` clips the list's `2px 2px 0` cast shadow at the right and bottom. Screenshot-confirmed. The tile-only reset at L6485 covers `.sync-history-thread-card` but not the list itself, and the `gap: 0` at L6484 is now redundant because `.sync-history-thread-list.flat` (L9804, same 0,2,0 specificity, later in source) wins.

**Fix.** In src/App.css add a reset beside the existing card reset, matching the two the author already wrote for the sibling cases at L10009 and L9393: `.sync-project-file-tile > .sync-history-thread-list.flat { gap: 0; border: 0; border-radius: 0; background: transparent; box-shadow: none; }`, and delete the now-dead `.sync-project-file-tile > .sync-history-thread-list { gap: 0; }` at L6484.


### Medium priority

#### M01 — Path-bar switcher search field cannot be reached or held by the keyboard

`src/components/sync-review/SyncPathBar.tsx:328` · keyboard-focus · confirmed tiebreak

```
{searchable && (
        <input
          type="search"
          className="sync-path-menu-search"
          placeholder={`Switch ${label.toLowerCase()}…`}
          aria-label={`Filter ${label.toLowerCase()} list`}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      )}

// and, at the menu container (line 255-261):
        if (event.key === "Tab") {
          onClose(false);
          return;
        }
```

**Impact.** With 6 or more switch targets (7+ projects, or 6+ other storages — `SWITCHER_SEARCH_FLOOR = 6` at line 129, counting only labelled "Switch to" groups, e.g. ProjectLinksWorkspace.tsx:2427-2438), the crumb switcher renders a filter box that no keyboard user can use. Focus enters the menu only through `focusMenuOption`, which selects `[role="menuitem"]` nodes (menuKeyboard.ts:13); the input is not one. Arrow/Home/End never leave that set. The only tabbable node inside the open menu is that input, but the container's Tab handler closes the menu on Tab in either direction, so Shift+Tab from the first list row focuses the input and the very next commit sets `hidden` on its ancestor, dropping focus to `<body>`. A keyboard or screen-reader user with many projects therefore cannot filter the list at all and can lose focus to the top of the document just by pressing Shift+Tab; the Enter-takes-first-match branch at lines 267-272 is unreachable dead code as a result.

**Fix.** In src/components/sync-review/SyncPathBar.tsx, focus the search input first when `searchable` (extend `focusMenuOption`/`focusMenuEdge` to prefer `.sync-path-menu-search` when present), and stop treating Tab inside a searchable switcher as "close": let Tab move between the input and the option list, closing only when focus actually leaves the menu root (a `focusout` check), so the field is reachable and keeps focus while typing.


### Low priority

#### L01 — Escape discards the prepared Push review from anywhere, including inside the Logs drawer

`src/components/project-sync/PushResourceWorkspace.tsx:417` · destructive-safety · confirmed tiebreak

```
useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, onClose]);
```

**Impact.** `onClose` runs `setPendingPush(null)` plus three selection resets (ProjectSync.tsx:3396-3402), throwing away a backend-prepared push and every conversation/file the user ticked. The Logs drawer renders in the same workspace above the review (ProjectSync.tsx:3876) and has no Escape handler of its own, so a user who opens Logs mid-review and presses Escape expecting the drawer to close instead silently loses the whole prepared push while the drawer stays open. LogPanel only stops Escape for two cases — a non-empty search field (LogPanel.tsx:305-312) and an open type filter (LogPanel.tsx:180-192) — so Escape from the Level select, Manage logs, Close, Load older, or the drawer region itself reaches the review. The comment at PushResourceWorkspace.tsx:873 shows the hazard is known but only partially mitigated.

**Fix.** Give the Logs drawer its own Escape handler in src/components/project-sync/ActivityLogDrawer.tsx that closes the drawer and calls `stopPropagation()` while it is open, and scope the review's listener to the review subtree (attach it to the review root element rather than `window`) so Escape only discards a review the user is actually focused in.


#### L02 — Logs drawer resize separator is mouse-only

`src/components/project-sync/ActivityLogDrawer.tsx:191` · keyboard-focus · confirmed tiebreak

```
<div
          className="log-resizer"
          role="separator"
          aria-label="Resize log"
          aria-orientation="horizontal"
          onMouseDown={onStartResize}
          onDoubleClick={onResetHeight}
        />
```

**Impact.** The separator has no `tabIndex`, no `aria-valuemin`/`aria-valuemax`/`aria-valuenow`, and no key handler, so a keyboard user cannot change or reset the log drawer height at all — the drawer is stuck at whatever pixel height was last set with a mouse, and on a short window that can leave only a couple of log lines visible with no way to enlarge it. The same file's sibling control in ProjectSync.tsx:3695-3709 already ships the correct contract (`tabIndex={0}`, all three value attributes, `onKeyDown={resizeSidebarWithKeyboard}`), and ProjectSidebar.tsx:664-681 ships a third, so the pattern exists and this one diverges.

**Fix.** In src/components/project-sync/ActivityLogDrawer.tsx, add `tabIndex={0}`, `aria-valuemin`/`aria-valuemax`/`aria-valuenow` for the drawer height, and an `onKeyDown` that steps the height on ArrowUp/ArrowDown and resets on Home/End, mirroring `resizeSidebarWithKeyboard` in ProjectSync.tsx; switch the pointer handlers to the pointer-event set the sidebar uses.


#### L03 — Agent-setup split menu declares role="menu" with no name, no keyboard contract and no focus move

`src/components/project-sync/ProjectLinksWorkspace.tsx:2044` · aria-semantics · confirmed unanimous · found independently by 2 lenses

```
{agentMenuOpen && (
                            <div className="sync-path-menu end" role="menu">
                              {agentLaunchers
                                .filter(({ key }) => key !== defaultAgentLauncher.key)
                                .map(({ key, launcher }) => (
                                  <button
                                    key={key}
                                    type="button"
                                    className="sync-path-menu-item"
                                    role="menuitem"
```

**Impact.** The split button at line 2032 advertises `aria-haspopup="menu"`, but the popup it opens has no `aria-label` (a screen reader announces an unnamed menu), no ArrowDown/ArrowUp/Home/End handling, and no focus move on open — pressing Enter on the chevron leaves focus on the trigger with nothing announced. `menuKeyboard.ts` exists in the codebase precisely for this and is imported by only two files (SyncPathBar.tsx and StorageCatalogPage.tsx); this menu, on the R2 storage-setup path where the alternative agent launchers live, reimplements nothing at all.

**Fix.** In src/components/project-sync/ProjectLinksWorkspace.tsx, give the menu an `aria-label` ("Agent setup routes"), import `focusMenuEdge`/`moveMenuFocus` from ../sync-review/menuKeyboard, focus the first item on open, add the arrow/Home/End `onKeyDown`, set `tabIndex={-1}` on the items, and restore focus to the chevron on Escape.


#### L04 — Three menus that close by `hidden` drop focus to <body> on Escape

`src/components/storage-catalog/StorageCatalogPage.tsx:955` · keyboard-focus · confirmed unanimous · found independently by 2 lenses

```
const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      setOpenChoices(null);
    };
```

**Impact.** The Catalog local-action menu focuses its first option on open (`ref={(node) => focusMenuEdge(node, "first")}`, line 585), so focus is inside the menu when Escape arrives; closing unmounts it and focus falls to `<body>`, so the user's next Tab restarts at the top of the page instead of on the chevron they opened. The same omission exists in the app-actions menu (ProjectSync.tsx:1318-1322, whose single `role="menuitem"` at line 2784 is tabbable and reachable) and the storage agent-setup menu (ProjectLinksWorkspace.tsx:1278-1281). SyncPathBar.tsx:410-412 and ProjectActivityPage.tsx:878-884 already do this correctly with a stored opener ref, so the pattern is established and these three diverge.

**Fix.** Hold a ref to each trigger button and call `trigger.focus()` after the close in all three Escape handlers — StorageCatalogPage.tsx:955-961, ProjectSync.tsx:1318-1322, ProjectLinksWorkspace.tsx:1278-1281 — matching `closeMenu(true)` in ProjectActivityPage.tsx:163-166.


#### L05 — With one storage configured, 'Add storage…' collapses to an unlabeled '+' glyph and the switcher body is empty

`src/components/sync-review/SyncPathBar.tsx:168` · state-coverage · confirmed unanimous

```
const verbs = switcher && populated[0] && !populated[0].label ? populated[0].items : [];
  const lists = switcher ? populated.filter((group) => !!group.label) : [];
  const tails = switcher
    ? populated.slice(verbs.length > 0 ? 1 : 0).filter((group) => !group.label).flatMap((group) => group.items)
    : [];
```

**Impact.** Roles are assigned by group position, so the same menu item changes shape with the data. The storage editor's crumb builds its 'Switch to' group conditionally (ProjectLinksWorkspace.tsx:1708-1721) and then always appends the unlabelled `add-storage` group. With two or more storages, populated[0] is the labelled list, so `add-storage` lands in `tails` and renders as a text pill reading 'Add storage…'. With exactly one storage — the first-run state — the 'Switch to' group is absent, `populated[0]` is the `add-storage` group, and it is classified as a verb instead: renderPill(item, false) emits only `{item.icon && <Icon …/>}` with no text (line 241-242), so the menu shows the current-storage row and then a footer strip containing a single bare 32px '+' button whose words exist only in `title`/`aria-label`. The list area between them is empty. Verified by replaying the partition over the real caller shapes: one storage yields verbs:["add-storage"], lists:[], tails:[]; three storages yield verbs:[], lists:["Switch to"], tails:["add-storage"].

**Fix.** Classify roles explicitly rather than by position — e.g. add a `role?: "verbs" | "targets" | "more"` field to PathBarMenuGroup and have ProjectLinksWorkspace tag the `add-storage` group as "more" — or, in SyncPathBar.tsx:168, only treat populated[0] as verbs when a labelled group exists later in `populated`.


#### L06 — The switcher panel has no max-height or scroll and is clipped by #root's overflow:hidden

`src/App.css:3061` · layout-responsive · confirmed unanimous · found independently by 2 lenses

```
.sync-path-menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 30;
  min-width: 260px;
  max-width: 380px;
  padding: 8px;
```

**Impact.** No rule in App.css gives `.sync-path-menu` a max-height or `overflow`, and the panel's only clipping ancestor is `html, body, #root { height: 100%; width: 100%; overflow: hidden; }` (App.css:7-11) — nothing between them scrolls. The bar is 3.25rem tall (App.css:2539) and each target row is min-height 34px (App.css:3102), so on the configured minimum window height of 560px (src-tauri/tauri.conf.json:25) about 11 target rows fit; on the 720px default about 16. Past that the list is cut off by the window bottom and the `.sync-path-menu-foot` strip goes with it — meaning 'Storage settings…', 'Unlink…', 'Rename project…', 'Reset local sync state…' and 'Add project…' become entirely unreachable, since they now live only in that footer. Arrow-key focus does not rescue it: focus() finds no scrollable ancestor, so the focused row stays off-screen. The change makes this reachable by design rather than by accident — SWITCHER_SEARCH_FLOOR = 6 (SyncPathBar.tsx:129) exists precisely because lists of this size are expected.

**Fix.** Give `.sync-path-menu` a viewport-relative ceiling and let the target list scroll: `max-height: min(60vh, …)` on the panel with `overflow-y: auto` on the `.sync-path-menu-group` region, keeping `.sync-path-menu-current`, `.sync-path-menu-search` and `.sync-path-menu-foot` pinned outside the scrollport so the verbs stay reachable.


#### L07 — Singular capture-budget banner reads "1 conversation wait for the next round"

`src/components/project-sync/PushResourceWorkspace.tsx:574` · copy · confirmed unanimous

```
title: `${capturePendingCount} conversation${capturePendingCount === 1 ? "" : "s"} wait for the next round`,
```

**Impact.** The noun is pluralised but the verb is not, so a one-row hold prints ungrammatical copy in a standing banner. The sibling banner twelve lines above gets this right (`${heldProjectFileCount === 1 ? "requires" : "require"}`), so the review can show a correct and an incorrect sentence side by side.

**Fix.** `${count} conversation${count === 1 ? " waits" : "s wait"} for the next round`, matching the project-file banner's verb agreement.


#### L08 — Logs drawer resize separator is not focusable and carries no aria-value* state

`src/components/project-sync/ActivityLogDrawer.tsx:193` · aria-semantics · confirmed unanimous

```
<div
          className="log-resizer"
          role="separator"
          aria-label="Resize log"
          aria-orientation="horizontal"
          onMouseDown={onStartResize}
          onDoubleClick={onResetHeight}
        />
```

**Impact.** A separator that is operable must be focusable and expose aria-valuenow/valuemin/valuemax; this one has no tabIndex, no value attributes, and only mouse handlers. A keyboard or switch user cannot change the log drawer height at all, and a screen reader reads "Resize log, separator" with no indication of the current size or that it can be moved. The sidebar separator in the same app (ProjectSync.tsx:3695-3708) already has the full contract — tabIndex={0}, aria-valuemin/max/now, onKeyDown — so the pattern exists and was simply not applied here.

**Fix.** Copy the sidebar separator's contract: tabIndex={0}, aria-valuemin/aria-valuemax/aria-valuenow bound to the drawer height, pointer events instead of onMouseDown, and an onKeyDown handling ArrowUp/ArrowDown/Home/End.


#### L09 — Read-category tool names fall to 3.93:1 on an expanded tool row in the light theme

`src/App.css:7609` · contrast · confirmed unanimous

```
.session-detail-tool-call[open] > summary {
  background: var(--surface-row-active);     /* light #f0e3d5 */
}
...
.session-detail-tool-name.cat-read { color: var(--session-detail-link); }

/* --session-detail-link: color-mix(in srgb, var(--blue) 85%, var(--text-0)) — App.css:6851 */
```

**Impact.** `--session-detail-link` resolves to #2c779c in light. On the closed row's --surface-row (#fdf9f3) it is 4.72:1, but the moment the row is expanded the background becomes --surface-row-active (#f0e3d5) and it drops to 3.93:1 — under the 4.5 floor for text that is 9px (`font-size: var(--type-3xs)`), uppercase and letter-spaced (App.css:7597-7606). The same row's `--session-detail-you` red would be 4.00:1 and `--danger-text` on the failure pill 4.41:1 there. Dark is unaffected (8.8–10.6:1). The mixes at App.css:6850-6852 were tuned against the page paper, not against the darkest row state the page actually paints.

**Fix.** Re-tune the three `--session-detail-*` mixes at src/App.css:6850-6852 against --surface-row-active (#f0e3d5 light) rather than the page paper — raising the --text-0 share for the link mix clears it — or stop darkening the open row and use the ink border alone to mark it open.


#### L10 — Fresh-install main pane tells the user to select a project that cannot exist, and names one provider

`src/components/project-sync/ProjectLinksWorkspace.tsx:2236` · copy · confirmed unanimous · found independently by 2 lenses

```
<h1 id="git-info-heading" className="settings-section-title">Project history</h1>
              <div className="profile-links-subtitle">Select a completed project to view its Codex history.</div>
```

**Impact.** This branch renders whenever no project is selected, including the zero-project fresh install — where the sidebar simultaneously says "Add your first project" (ProjectSidebar.tsx:857). The dominant content area instructs an action that is impossible from that screen and offers no control. "completed" describes no state the UI ever shows (there is no completed/incomplete badge on projects). "Codex history" hard-codes one provider on a screen that also handles Claude — every neighbouring string is provider-neutral ("Git history", "Sessions", `provider_display_name`). Filed previously as M15; unchanged.

**Fix.** Replace the subtitle with a first-use sentence plus a primary action: "Add a project to see its agent sessions and sync them to a storage." with an "Add project" button, and drop "completed" and "Codex".


#### L11 — Agent-launcher split menu declares role="menu" with no arrow keys, no label, no focus move, and stays open when the user tabs past it

`src/components/project-sync/ProjectLinksWorkspace.tsx:2044` · keyboard-focus · confirmed unanimous

```
{agentMenuOpen && (
                            <div className="sync-path-menu end" role="menu">
                              {agentLaunchers
                                .filter(({ key }) => key !== defaultAgentLauncher.key)
                                .map(({ key, launcher }) => (
                                  <button
                                    key={key}
                                    type="button"
                                    className="sync-path-menu-item"
                                    role="menuitem"

// Its only dismissal handlers (lines 1272-1289)
```

**Impact.** Four separate keyboard defects on the R2 storage-setup "Run with <agent>" split button. (1) `role="menu"` with two or more menuitems and no ArrowUp/ArrowDown/Home/End — a screen reader switches to menu navigation and the arrows do nothing, which is precisely what menuKeyboard.ts was written to prevent, and this file never imports it. (2) Opening the menu does not move focus into it, so it appears without any focus cue. (3) There is no focusin dismissal, so tabbing past the last item leaves the popover open and floating over the setup form until a mouse click or Escape. (4) Escape unmounts the menu (`{agentMenuOpen && …}`) without restoring focus to the chevron, dropping focus to <body>. The menu is unlabelled too, so it is announced as a bare "menu".

**Fix.** Import `focusMenuEdge`/`moveMenuFocus` from ../sync-review/menuKeyboard in ProjectLinksWorkspace.tsx and apply the same recipe StorageCatalogPage.tsx:581-592 uses: `aria-label` on the menu div, `tabIndex={-1}` on the items, focus the first item on open, `onKeyDown` delegating to `moveMenuFocus` and closing on Tab, and a ref to the `.sync-r2-split-arrow` trigger that is refocused after `setAgentMenuOpen(false)` in the Escape handler at line 1278.


## Recommended repair order

1. **The `--surface-row-active` cluster** (R1, R2, L09). One surface, four tokens, one gate
   change. Fix the gate first so it fails on the current values, then retint.
2. **Startup accessibility and copy** (R3, R4). Same screen, same component.
3. **Nested ink frames** (H01, and the Older-activity fold it was deduped with). The retheme's
   box grammar needs a rule about which of a container/child pair owns the edge.
4. **Switcher keyboard and overflow** (M01, L05, L06). All in the uncommitted `SyncPathBar`
   work, best fixed before that change is committed.
5. **Menu keyboard contract** (L03, L04, L11). Three menus reimplement what
   `menuKeyboard.ts` already provides; route them through it.
6. **Logs drawer separator** (L02, L08) — the same control, two findings.
7. Copy and pluralisation (L07, L10) — cheap, independent.
8. **R6** — one-line safety change, no visual effect.

## Runtime verification still required

None of this was checked in a real Tauri run: native focus behaviour, VoiceOver output,
forced-colors mode, system text scaling, real window resizing below 800x560, native dialogs,
provider launches, and the production React tree under real IPC. The only production surface
captured live was the startup-failure screen.

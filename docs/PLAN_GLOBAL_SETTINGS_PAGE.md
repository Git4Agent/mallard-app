# PLAN: Global Settings page

Status: IMPLEMENTED (2026-08-21, uncommitted). All stages S1-S5 landed in one
pass; `npx tsc --noEmit`, `npm run build`, and the full frontend suite (219)
are green. Mockup specimen kept in sync at `settings-mockup.html` +
`src/settings-mockup.tsx` — run `npm run dev`, open
`http://localhost:1420/settings-mockup.html`.

Implementation deltas against the sections below:

- **Privacy copy (user-set)**: the redaction row does *not* name the ruleset —
  no "Built into every Push — ruleset mallard.redact.v1." sentence. The row
  is the guarantee sentence plus the "Always on" chip; a test pins the
  ruleset id out of the page (`settings-page.integration.test.tsx`).
- The Logs group absorbed the *whole* dialog, which held more than §3's
  table: stats readout (files · size, oldest entry as its tooltip), **Open
  folder**, and a fourth "Clean up" row carrying **Clean up now** + **Delete
  all** with the dialog's confirm() texts. Policy selects apply on change —
  the dialog's explicit Save existed only to keep cleanup honest against a
  drafted select, and apply-on-change removes that state entirely.
- Two more clock sites existed than §5 listed: `SyncOutcomeNotice.tsx` and
  `ProjectLinksWorkspace.tsx` (absoluteTime). All eight now go through
  `src/clock.ts` `clockOptions()`; `formatSessionDetailTimestamp` (previously
  locale AM/PM) follows the preference too. LogPanel subscribes via
  `usePreference("clock24")` because the drawer can be visible while the
  Settings page flips the preference.
- Group headings reuse the existing `.settings-section-title` class (already
  in RetroTheme's pixel list) instead of a new heading class; only
  `.sync-settings-flag` and `.sync-settings-segment > button` joined the
  single pixel-face rule.
- `useSyncExternalStore` hooks pass a server-snapshot getter — the frontend
  test harness renders with `renderToStaticMarkup`, which requires it.
- Tests updated: cutover's "Dark theme" menu pin → Settings item pin;
  activity-log's push/pull pin now asserts the `getPreference` gate;
  storage-settings' `showsPathBar` pin gained `settingsOpen`; the sidebar
  nav-order test asserts Settings sits between Storages and Logs. New suite:
  `tests/frontend/settings-page.integration.test.tsx` (registered in the
  runner).

One sentence: **a fifth primary-nav destination — Settings, above Logs — that
gathers every app-wide preference into one workspace page** built from the
existing page vocabulary (path bar root crumb, card rows, pixel-face chrome),
absorbing the log-policy dialog, giving theme a real three-state home, and
turning two hardcoded behaviors (open-Logs-on-sync, refresh-on-focus) into
switches without adding a second settings store to the codebase.

## 1. What exists today (verified against source)

- **No general settings surface.** Five localStorage keys are written by four
  different components, each rolling its own read/write pair: `agent-sync-theme`
  (`src/theme.ts:3`), `mallard.project-sidebar-width` / `-visible`
  (`ProjectSync.tsx:163-164`), `mallard.active-storage-by-project`
  (`ProjectSync.tsx:165`), `mallard.sidebar-section-split`
  (`ProjectSidebar.tsx:29`).
- **One real persisted user policy**: activity-log retention/size in
  `~/.mallard/logs/_policy.json` (`activity_log.rs:21-26`), edited through
  `LogManagerDialog.tsx` (a native `<dialog>`; retention 7/30/90/180/365 days,
  cap 25/100/250/500/1000 MB) via `get_activity_log_stats` /
  `update_activity_log_policy` (`src-tauri/src/activity_log.rs:140,146`,
  fronted by `api.ts:88-92`). This dialog is effectively the app's only
  settings screen today.
- **Theme** is binary light/dark, `data-theme` on the root, persisted, default
  dark; there is **no System option** — no `prefers-color-scheme` anywhere in
  the two stylesheets. Toggles: titlebar icon button
  (`ProjectSync.tsx:2609-2619`) and a checked item in the path-bar overflow
  (`ProjectSync.tsx:2584-2590`).
- **Open Logs on Push/Pull is unconditional**: `openActivity()` is the first
  statement of the push path (`ProjectSync.tsx:1938`) and of the pull-apply
  path (`ProjectSync.tsx:2772`).
- **Refresh on window focus is unconditional**: returning after ≥5 s away
  triggers a reconcile (`FOCUS_RECONCILE_MIN_AWAY_MS`, `ProjectSync.tsx:169`,
  used at `:1279`).
- **Credential redaction is always-on by decision** — the compiled-in
  singleton `credential_redaction::shared()` runs unconditionally at the push
  boundary (`provider_capture.rs:469-473`), and
  `PLAN_CREDENTIAL_REDACTION.md` D4 explicitly rules out a toggle ("a toggle
  creates mixed-form clouds and re-introduces the false-divergence surface").
- **Clocks disagree across surfaces**: session detail hardcodes `hourCycle:
  "h23"` deliberately (`sessionDetailModel.ts:263-270`); `LogPanel.tsx:72-76`,
  `StorageCatalogPage.tsx:123-124`, `storage-catalog/model.ts:142-143`, and
  `ProjectActivityPage.tsx:283-289` each do something locale-dependent and
  different.

## 2. Navigation and page shell

- **Sidebar**: fifth `sync-primary-nav` button between Storages and Logs
  (user-specified position), icon `settings` — the pixel sliders glyph already
  in `Icons.tsx`. Not a cog: the cog is the `tool` glyph and already means
  "tool call" in session detail. New props on `ProjectSidebar`
  (`settingsOpen`, `onOpenSettings`); the other items' `active` conditions
  each gain `&& !settingsOpen` (`ProjectSidebar.tsx:408-444`).
- **View state**: `settingsOpen` boolean in `ProjectSync`, shaped exactly like
  `storageIndexOpen` — opening it clears drafts/catalog/storage views and vice
  versa; `showsPathBar` (`ProjectSync.tsx:830-836`) gains it; the workspace
  branch (`ProjectSync.tsx:3395-3431`) renders `<SettingsPage>` ahead of the
  project fallback.
- **Page skeleton**: the `StorageIndexPage` recipe — real `SyncPathBar` with a
  single root crumb `{ id: "settings", label: "Settings", icon: "settings",
  tone: "root" }`, overflow = `appActions`, no action buttons; content in
  `<main className="sync-main sync-project-links-page sync-settings-page">`.
- **App menu**: `appActions` (`ProjectSync.tsx:2579-2603`) gains a
  `Settings` item; the checked `Dark theme` item is **removed** — Appearance
  owns theme now, and the menu's own design comment says surplus chrome there
  was "furniture". The titlebar sun/moon button on non-path-bar screens stays
  (it is the window's control, one keystroke, no menu).

## 3. Page content (as mocked)

Five groups, each a pixel-face heading (`--type-xl`) + one bordered card
(`--border-mid` / `--surface-card`, radius 6 — the `project-storage-group`
recipe) of rows. A row = copy column (sans strong + `--text-2` sentence) and a
control column. New CSS all rides existing tokens; no new colors/sizes/radii.

| Group | Icon | Row | Control | Default |
|---|---|---|---|---|
| Appearance | `sun` | Theme | segmented System / Light / Dark | System |
| Privacy | `lock` | Credential redaction | status chip **Always on** (not a switch — honors redaction D4); copy names ruleset `mallard.redact.v1` | — |
| Sync | `git-compare-arrows` | Open Logs on Push and Pull | switch | on (current behavior) |
| Sync | | Refresh on window focus | switch | on (current behavior) |
| Logs | `activity` | Keep events for | select 7/30/90/180/365 days | 30 days |
| Logs | | Size cap | select 25 MB–1 GB | 100 MB |
| Logs | | Log files | readout `N files · X MB` + **Reveal in Finder** (`~/.mallard/logs` in mono) | — |
| Display | `clock` | 24-hour clock | switch | on (current session-detail behavior) |

New controls, both drawn from existing vocabulary:

- **Switch** — the `--switch-off` / `--switch-thumb` tokens both palettes have
  carried unused; construction matches the app's drawn checkboxes
  (`appearance: none`, thumb on `::before`), checked = `--accent` track +
  `--accent-contrast` thumb (visible in both palettes), stepped 90 ms
  transitions per the skin.
- **Segmented control** — button tokens for the frame, active segment on
  `--surface-row-active` like an active nav row; labels in the pixel face
  (closed vocabulary).

## 4. Persistence

- **UI preferences → localStorage**, via a new `src/preferences.ts`: typed
  keys, defaults, `get`/`set`/`subscribe` in the `theme.ts` shape. `SyncConfig`
  is the wrong home — it is a schema-versioned, revision-checked, watched
  document (`config_watcher.rs:16-17`); putting UI prefs there buys watcher
  churn for nothing. No Rust changes for any v1 preference.
  - Keys: `mallard.theme` (`"system" | "light" | "dark"`),
    `mallard.open-logs-on-sync`, `mallard.refresh-on-focus`,
    `mallard.clock-24h` (booleans as `"on"`/`"off"`).
  - `theme.ts` folds into `preferences.ts` and is deleted (clean cutover).
    The legacy `agent-sync-theme` key is abandoned, not migrated; absence now
    means **System**. Consequence: on first launch after update the app
    follows the OS instead of forced dark, and anyone who wants the old look
    picks Dark once. (Flagged as D3 below — say no and the default stays
    Dark.)
  - `applyTheme` gains the System branch: resolve via
    `matchMedia("(prefers-color-scheme: dark)")`, subscribe to its `change`
    while the choice is System (mockup demonstrates this working).
- **Log policy stays in `~/.mallard/logs/_policy.json`** through the existing
  two Tauri commands; the page's Logs group is a second front end to the same
  policy, plus `get_activity_log_stats` for the readout and the existing
  reveal capability for the folder button.

## 5. Behavior wiring

- `openActivity()` call sites gate on the pref: push (`ProjectSync.tsx:1938`)
  and pull-apply (`ProjectSync.tsx:2772`). The unread badge keeps counting
  either way — with the drawer closed, a suppressed auto-open still shows the
  count on the Logs nav item.
- Focus reconcile (`ProjectSync.tsx:1279`) gates on `refresh-on-focus`.
- Clock: one shared `formatClock` helper reads `clock-24h` and replaces the
  five divergent call sites (`sessionDetailModel.ts:263`, `LogPanel.tsx:72`,
  `StorageCatalogPage.tsx:123`, `storage-catalog/model.ts:142`,
  `ProjectActivityPage.tsx:283`). Default **on** preserves session detail's
  deliberate h23; "off" = system locale everywhere. (This unifies formats; it
  does not reintroduce relative timestamps, which stay refused.)
- **LogManagerDialog is absorbed and deleted** (clean cutover): retention,
  size cap, stats, and the folder reveal all live on the Settings page; the
  log drawer's manage affordance becomes "Log settings" and navigates to
  Settings. The dialog's purge-now action moves onto the Log files row as a
  second button if we want to keep it (decide at implementation; nothing else
  in the dialog is unique).

## 6. Decisions

- **D1 — position**: Settings sits between Storages and Logs (user-specified
  "on top of Logs").
- **D2 — redaction is a status row, not a switch.** Upstream D4 of the
  redaction plan stands; the row states the guarantee and names the ruleset.
  If a toggle is ever wanted, that is a redaction-plan revision first, not a
  settings-page checkbox.
- **D3 — theme default flips to System** (with the `mallard.theme` key
  replacing `agent-sync-theme`, no migration). Alternative: keep Dark as the
  absent-key default and System is merely available.
- **D4 — one store split**: UI prefs in localStorage (`preferences.ts`), disk
  policy stays `_policy.json`. No prefs in `SyncConfig`.
- **D5 — v1 rows are exactly the mocked eight.** Surveyed and deferred:
  preferred terminal (`launch.rs:9-27` hardcodes Terminal.app), GitHub-shaped
  commit URLs (`externalLink.ts:11-14`), default log-type filters
  (`LogPanel.tsx:35,90`), log-drawer height persistence, watch-service
  limits, `MALLARD_SYNC_METRICS` as a diagnostics switch. Rejected outright:
  suppressing destructive confirmations (13 `confirm()` sites are load-bearing
  safety), window-geometry persistence (Tauri plugin territory), per-project
  setting overrides.

## 7. Stages

- **S1 (done)** — this mockup: shell, nav item, page, controls, both palettes.
- **S2** — `preferences.ts` + theme three-state + `SettingsPage.tsx` +
  sidebar/view wiring + appActions change. Page fully works for Appearance
  and Logs (policy round-trips through existing commands).
- **S3** — behavior gates: open-Logs, focus-refresh, `formatClock`
  unification across the five call sites.
- **S4** — absorb + delete `LogManagerDialog`, repoint the drawer's manage
  affordance.
- **S5** — gates: new CSS through the four hygiene gates (tokens only, rem
  scale — already true of the mocked rules); pixel-face classes join
  RetroTheme's single shared `--font-pixel` list (a test counts declarations
  in that file; the mockup carries them inline only because it is not
  RetroTheme.css); frontend integration test for the page (toggle →
  localStorage; push with open-Logs off must not open the drawer); WCAG spot
  checks (switch track/thumb and segment active state are token pairs already
  used at ≥3:1). Full bar green.

## 8. Out of scope

Cross-machine settings sync, per-project overrides, a redaction toggle,
notification preferences (nothing notifies yet), locale/language.

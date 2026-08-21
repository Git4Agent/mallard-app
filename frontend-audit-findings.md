# Mallard Frontend Audit

**Scope:** `src/` (42,752 lines), `tests/frontend/`, `index.html`, `vite.config.ts`, `public/`, and the Tauri surface where it meets the frontend (`src-tauri/tauri.conf.json`, `capabilities/`, DTO contracts).
**Working tree audited:** uncommitted state at `e0d03b6` + local changes. **Date:** 2026-08-21.
**Nature:** audit only — no source file was modified.

**Method.** Twelve parallel auditors swept one dimension each (hooks, async/IPC, DTO contracts, state architecture, render perf, a11y, CSS, security, module boundaries, UX states, tests, session-detail). Every finding was then handed to an adversarial verifier instructed to refute it, defaulting to REFUTED when the failure path could not be traced in the real code. 84 raw findings → **68 survived**, plus 5 from a completeness critic. 16 were killed as wrong or unverifiable. The high-severity items and every claim quoted below were then re-checked by hand against source, including the `tauri-plugin-opener` and `react-dom` crate/package sources.

**Confidence labels.** `CONFIRMED` = the failing path was traced end to end in the code. `PLAUSIBLE` = the code is genuinely wrong but the concrete trigger depends on runtime state (a Windows host, a failing bucket) that could not be exercised by reading.

---

## Status — Waves 1, 3 and 4 implemented (2026-08-21)

**F1–F8 (Wave 1), F18–F35 (Wave 3) and F36–F72 (Wave 4) are fixed in the working
tree**, plus F12 and F13 from Wave 2 (both CSS gates, done alongside the CSS work
they guard). The rest of Wave 2 — the jsdom harness, typechecking `tests/`, and
the command-name gate — remains open.

Verification after the change: `npx tsc --noEmit` clean · `npm run build` clean ·
frontend suite **224/224** · Rust lib suite **213/213** · `cargo clippy -- -D warnings`
clean · `cargo fmt --check` clean. `dist/` fell from 1.6 MB to 1.0 MB and `App.css`
from 8,725 to 8,402 lines.

### Two items deliberately not done, and why

- **F38, and the Rust half of F39.** `get_bundle_status`, `get_bundle_readiness`
  (orphaned by F22) and `anchor_event_id` are all registered backend surface with no
  frontend caller. Removing them means editing `lib.rs` and the frozen command list —
  the exact files an in-flight `SiblingRepoNotice` feature was rewriting throughout
  this session (it added `get_sibling_bundle_status` and
  `list_pending_storage_bundle_snapshots`, which is what put the freeze list out of
  sync mid-session). Changing a frozen contract from two directions at once invites a
  merge mess, so all three should be swept in one deliberate commit. The frontend
  halves are done: nothing in `src/` can reach any of them. `anchorEventId` still
  ships as an explicit `null` so the wire shape is unchanged.
- **F43's mass deletion.** The ratchet moved 435 → 423 and F13 now reads effective
  values, which is the durable half. The 76 layout-critical shadowed pairs were *not*
  bulk-deleted: my extractor does not carry `@media` context, so its list has false
  positives, and a wrong deletion changes layout silently in an app I cannot render
  here. Resolving them wants a visual pass.

### Corrections to the plans as written

Each of these was wrong in the report and fixed during implementation.

- **F2** — the report said to route the failure through `setError`; that setter does not
  exist in `ProjectLinksWorkspace` (`error` is a read-only prop). A dedicated
  `revealError` slot was added and rendered on both surfaces, since the two call sites
  live on different screens.
- **F6** — step 3 ("remove `secret_access_key` from the TS interface") would have broken
  the settings form, which legitimately *writes* the field. The read path is redacted
  and the write path merges blank→stored by id; `storageConfigReady` and `r2FieldsSet`
  now take an explicit `secretStored` argument so no call site can assume it, and a
  server-side `require_s3_storage_secrets` check makes the invariant explicit. Five
  Rust tests cover the round trip.
- **F31** — a `string | null` return conflated "superseded" with "failed", which an
  existing epoch test caught. The contract is now
  `RenameResult = { ok: true } | { ok: false; message: string | null }`.
- **F35** — `onCloseRequested` calls `window.destroy()` itself, and the project's own ACL
  manifest confirms `core:default` grants 28 window permissions **not** including
  `allow-destroy`. Registering it without that permission would have made the window
  unclosable. The permission was added and the listener is registered **only while
  `busy`**.
- **F53** — the report said Rust never emits `selected_by_default`. It does, but on
  `ResourceCandidate`, a different struct; the field the inventory really emits is
  `default_selected`. The dead one was removed and the live one made required.
- **F68** — the suggested `:nth-child(odd)` label rule would have been wrong (the
  Resource ID value is a `<code>`, so the count is off). The grid became a real
  `<dl>` instead, which removes the conflict at the source and took both `!important`s
  with it.
- **F41** — my first deletion script computed each rule's span from the prelude match,
  which starts immediately after the previous `}`; walking back to the line start
  swallowed the preceding rule and would have deleted the live
  `.icon-git-folder-mark`. Caught before it shipped. The corrected pass removed 80
  dead rules, and every one of the 66 classes that lost all styling was verified
  absent from the source.

### Needs a run I could not perform

**F35** (close guard) needs `npm run tauri dev`: start a Pull, click the red close
button, confirm the dialog appears and that both "Keep syncing" and "Quit anyway"
behave. **F70** (CSP) was verified as far as it can be outside Tauri — the real bundle
was served under the real policy and checked in a browser: React mounted, 1,733 CSS
rules applied, Departure Mono loaded, inline element styles applied, a cross-origin
script was blocked, and https images stayed allowed. What that cannot cover is Tauri's
own IPC injection, so it still wants one packaged-build launch.

### Working-tree note

A separate in-flight feature (`SiblingRepoNotice`) was being edited in parallel
throughout. It accounted for 23 Rust failures and the frozen-command-list mismatch
seen mid-session; all resolved as that work landed. One test,
`replaced_remote_history_blocks_push_on_every_surface_and_pull_heals_it`, failed once
in a full run and passed in isolation and on every run since — order-dependent, not a
regression, worth a look on its own.


---

## 1. Verdict

This is a **disciplined codebase with a systemic verification gap**. The things that usually rot in a React app are clean here; the damage is concentrated in places where nothing was ever able to observe a regression.

**Healthy — verified, not assumed:**

| Check | Result |
|---|---|
| `npx tsc --noEmit` | passes, zero errors |
| `any` / `as any` / `@ts-ignore` in `src/` | **0** |
| `dangerouslySetInnerHTML` | **0** |
| Raw `invoke()` outside the two `api.ts` boundaries | **0** — the AGENTS.md rule holds |
| Markdown hardening | `skipHtml` + `urlTransform` + project-path allowlist — genuinely defended |
| CSS tokenisation | 2 literal colours in 8,672 lines of `App.css` outside token blocks |
| `console.*` left in production code | **1**, and it is in a mockup file |
| Module boundaries (`sync-review`, `storage-catalog`) | no violations found |
| `npm run test:frontend-integration` | 219/219 pass |

**Damaged:**

| Area | Finding |
|---|---|
| Regression safety | All 219 tests use `renderToStaticMarkup`. No effect, event, focus or state transition is executed by any test. `tests/` is excluded from `tsconfig.json`. |
| Failure visibility | Two shipped menu items are permanently dead and silent. A failed Pull renders nothing at all. |
| Cross-platform | Every custom agent-profile path is gated on `startsWith("/")`. The repo ships a Windows NSIS installer. |
| Shell size | `ProjectSync.tsx` is 3,585 lines holding **75 `useState` declarations**. `React.memo` appears **0 times** in the app. |
| Bundle | 640 KB of internal design mockups ship inside the released app (41% of `dist/`). |

**Counts:** 70 distinct findings (73 raw, 3 duplicate pairs merged) — 8 high, 23 medium, 39 low. 56 CONFIRMED, 14 PLAUSIBLE.

---

## 2. Root causes

Seventy findings are not seventy problems. They are seven, and fixing the first two collapses most of the rest.

### A — Nothing in CI can observe behaviour *(the meta-cause)*

All 13 suites render through `renderToStaticMarkup` from `react-dom/server`. There is no jsdom, no `createRoot`, no `act`. Effects never run; handlers never fire; focus does not exist. The suite's 219 assertions are first-paint HTML strings and **244 regexes over component source text**. Consequences, all measured:

- The `SyncReviewTabs` keyboard crash (F17) sits in shipped code because no test can press a key.
- `tsconfig.json` `include` is `["src"]`, so `tsc` never sees `tests/` — **62 fixtures already diverge** from `src/types.ts` and from production prop types, silently.
- The CSS gate at `css-hygiene.integration.test.tsx:89` clears a rule when **any** class in the chain is live rather than all, so it reports green with 64 orphan rules.
- A cutover assertion at `project-sync-cutover.integration.test.tsx:149` is green on a declaration `RetroTheme.css` later overrides — it asserts a value that never reaches the screen.
- None of `projectSyncApi`'s 68 command names is gated against the Rust `invoke_handler`. A backend rename ships green and breaks at runtime.

Everything downstream in this report survived because of A.

### B — Silent failure is the default posture

Async failures are dropped rather than surfaced, in a consistent pattern: `void somePromise(...)` with no `.catch`, or a caught error written to a state field that has no render site.

The app-wide `error` state in `ProjectSync.tsx:356` is **written by 23 handlers and rendered on 3 narrow surfaces**. A failed Pull plan renders nothing whatsoever. The Catalog posts a green ✓ notice when every item in a push failed.

### C — One component owns the application

`ProjectSync.tsx`: 3,585 lines, 75 `useState`, 20 `useEffect`, 2 `useMemo`. Three distinct classes of bug trace directly to this:

- **Races** — `loadProjectData` has no per-selection guard, so switching projects fast leaves project A's data under project B's name (F1).
- **State bleed** — no reset on selection change, so the previous project's error banners and stuck spinners carry over (F12), and a wiped storage keeps listing its deleted repositories (F11).
- **Prop-identity churn** — every state change anywhere re-renders everything below, and inline arrow props turn that into refetches (F9) and lost UI state (F10).

### D — Nothing is memoised, and hot rows rebuild `Intl` formatters

`React.memo`: 0 uses. Against 41 `useMemo` and 22 `useCallback`, that means memoisation exists for values but never for components. Costs are measurable: 6 `Intl.DateTimeFormat` constructions per activity row per render (F42), one per log line per render across 500 invisible rows (F15), full markdown re-parse of 150 timeline events on every unrelated state change (F14), and up to 3.2 MB of `<pre>` text mounted for collapsed tool calls (F43).

### E — Cutovers left half-finished

AGENTS.md forbids compatibility layers and demands the old path be deleted in the same change. Eight places kept the wiring after removing the feature: a dead `codex:session-index` special case, an unread required `profileLabel` prop, a registered `get_bundle_status` command with no caller, a live `anchor_event_id` query field nothing can reach, a remote readiness probe on every project switch whose only consumer is a dead field, a 720px sidebar rail that an 800px minimum window makes unreachable, 216 lines of CSS across 64 rules that cannot match any element, and 640 KB of mockups in `public/`.

### F — Duplicated helpers that have already drifted

Three byte formatters (one never emits a `B` unit, so any sub-1.5 KB resource reads "1 KB"), two `errorMessage` implementations (one skips both the R2 translation and the Bundle→Repo wording), two duration formatters (one has no days tier), two copies of `STORAGE_BLOCKING_STATES` that disagree on `"unknown"`, two `expandHomePath`, four clipboard implementations of which one has no fallback.

### G — Platform and trust-boundary assumptions

`startsWith("/")` gates path entry across five sites while `.github/workflows/release.yml` builds and publishes `x86_64-pc-windows-msvc`. `"csp": null` in a webview that renders transcripts pulled from shared S3/R2 — and that same webview receives every R2 secret access key in plaintext, because `StorageConfig.secret_access_key` has no `skip_serializing_if`.

---

## 3. Remediation plan

Four waves. Wave 1 is user-visible breakage; Wave 2 buys the ability to detect regressions and must land before the larger refactors; Waves 3–4 are hygiene.

| Wave | Theme | Items | Est. |
|---|---|---|---|
| **1 — Broken in the shipped app** | Dead features, invisible failures, Windows blocker | F1–F8 | ~2 days |
| **2 — Build the safety net** | DOM harness, typecheck tests, contract gates, fix false-green gates | F22–F27, F49 | ~3 days |
| **3 — State and performance** | Identity churn, resets, memoisation, `Intl` hoisting | F9–F21, F42–F43 | ~3 days |
| **4 — Hygiene** | Finish cutovers, dedupe helpers, a11y, CSS, DTO drift | F28–F41, F44–F73 | ~4 days |

> **A note that applies to every plan below.** Almost every fix plan the auditors produced proposed verifying with a `tests/frontend/` case. **Those cannot work today** — the harness has no DOM, so effects and events never run. Until Wave 2 lands, verification for behavioural fixes is `npm run tauri dev` and manual click-through, or a source-text assertion in the style the suites already use. Do not read a green bar as proof.

---

### Wave 1 — Broken in the shipped app

#### F1 · Rapid project switching shows project A's data under project B's name
`HIGH` · `CONFIRMED` · `src/components/project-sync/ProjectSync.tsx:1132`

`loadProjectData` guards only against a host-identity workflow change, not against its own supersession. Click project A (large Codex history, ~2 s), then immediately B (200 ms): B lands first, A overwrites it. The sidebar highlights B while the workspace renders A's sessions, A's inventory and A's readiness; A's `setError` surfaces under B's name; A's `finally` clears the spinner while B is still resolving. Nothing recovers until the user clicks B again.

**Plan**
1. Add `const projectRequest = useRef(0);` beside `restoreRequest`/`shellRequest` (~line 425).
2. At the top of `loadProjectData`, take `const requestId = ++projectRequest.current;` and define `const stale = () => hostWorkflowEpoch.current !== requestedWorkflowEpoch || projectRequest.current !== requestId;`.
3. Replace the epoch-only checks at every await boundary — lines 1139, 1144, 1156, 1182, 1185, 1193, 1199 — with `stale()`. This mirrors `reconcileExternalChanges` (line 1245), which already does it correctly.
4. Gate the `setLoading(false)` in the `finally` (line 1199) on `!stale()`, or a superseded load clears the live one's spinner.
5. `refresh()` (line 1213) routes through the same counter and needs no change.

**Verify:** `npm run tauri dev`, add an artificial delay to `getInventory` in the Rust command, alternate two projects rapidly, confirm the workspace always matches the sidebar highlight.

#### F2 · "Reveal in Finder" is permanently dead and silent (two call sites)
`HIGH` · `CONFIRMED` · `ProjectLinksWorkspace.tsx:1065` and `:1491`

`src-tauri/capabilities/default.json` grants `opener:allow-open-path` **only** for `$HOME/.mallard/logs`. I read `tauri-plugin-opener-2.5.4/src/commands.rs`: `open_path` ends `if scope.is_path_allowed(...) { ... } else { Err(Error::ForbiddenPath { .. }) }` — the calls are rejected. Both sites are written `void openPath(...)` with no `.catch`, so the rejection is discarded. Clicking "Reveal in Finder" on a project root or a local storage folder does nothing at all: no Finder window, no error, no toast. **This feature has never worked in any build carrying this capability file.**

**Plan**
1. In `ProjectLinksWorkspace.tsx`, change the import on line 12 from `openPath` to `revealItemInDir`. That command is already granted by `opener:default` and — verified in the same plugin source — `reveal_item_in_dir(paths: Vec<PathBuf>)` performs **no scope check at all**, so it works for project roots outside `$HOME` too.
2. Rewrite both call sites to surface failure. **The in-scope setter is `setBundleError` (declared line 513), not `setError`** — `error` is a read-only prop at line 98. Confirm `errorMessage` is imported from `./model` before use:
   ```
   onSelect: () => { void revealItemInDir(path).catch((reason) => setBundleError(errorMessage(reason))); }
   ```
3. Leave `SettingsPage.tsx:159` on `openPath` — opening the *logs folder* is the intended in-scope use and it already catches.
4. Do **not** widen the `allow-open-path` scope instead; that grants the webview the ability to launch arbitrary paths with their default handler.

**Note:** `revealItemInDir` selects the item in its parent rather than opening the directory. For a project root that is arguably the better affordance and it matches the Catalog's existing behaviour, but check the label still reads correctly.

**Verify:** `npm run tauri dev` → open a project → path-bar overflow → "Reveal in Finder" → Finder opens with the folder selected. Repeat on a local-kind storage.

#### F3 · Escape anywhere inside a Push review discards the entire review
`HIGH` · `CONFIRMED` · `PushResourceWorkspace.tsx:394`

`PushResourceWorkspace` listens for Escape on `window`; every nested layer (path-bar menus, the file-search input) listens on `document` and does not stop propagation. Hand-pick 40 of 200 sessions, acknowledge two scan warnings, switch to Project files, type a filter, press Escape to clear it — and the whole review unmounts. `setPendingPush(null)` drops the plan, the selection, the removal set and both acknowledgements. Re-running the entire push scan is the only recovery. Same outcome from dismissing any path-bar menu with Escape.

**Plan**
1. Add `event.stopPropagation()` immediately after the `event.key === "Escape"` branch **wherever the layer actually consumed the key** (i.e. only when a menu was open): `SyncPathBar.tsx:295`, `ProjectSync.tsx:1302`, `ProjectActivityPage.tsx:163` and `:814`, `StorageCatalogPage.tsx:562`, `LogPanel.tsx:130`. Because those listen on `document` and the workspace listens on `window`, stopping at `document` prevents the outer handler.
2. On the search input (`ProjectFilesReviewPage.tsx:239-246`) add `onKeyDown={(e) => { if (e.key === "Escape" && search) { e.stopPropagation(); setSearch(""); } }}` so clearing a filter never escapes to the review.
3. In `PushResourceWorkspace.tsx`, additionally require the review to be the deepest layer before acting — bail when the event originated inside an open popup.
4. Consider whether Escape should close a review with unsaved selection at all; a confirm, or restricting it to the review's own close button, may be the better product answer.

**Verify:** manual only until Wave 2. Build a Push review, select rows, press Escape from each nested surface in turn.

#### F4 · A Pull whose plan fails shows nothing at all
`HIGH` · `CONFIRMED` · `ProjectSync.tsx:2369`

`planRestore` **returns** its failure as a string rather than throwing, so the surrounding `catch` never fires. `beginProjectRestore` acts only on `failure?.includes("does not exist")` — every other failure string is discarded. Rotated R2 credentials, a down network, or corrupt bundle metadata all produce: spinner flickers off, `busy` clears, screen byte-for-byte identical. The user clicks Pull repeatedly with zero feedback.

**Plan**
1. After the existing `if (failure?.includes("does not exist"))` block, add `else if (failure)` calling `setStorageNotice({ projectId, storageId, kind: "action_failed", action: "pull", message: failure })` — the exact shape `openPushChooser` already uses at line 1803. `action_failed` notices render, carry Retry, and route irrecoverable errors to "Reset local sync state".
2. Also cover the `does not exist` branch when `bundles.length === 0`, which currently falls through silently too.
3. **Guard against superseded plans.** `return message` at line 2241 sits *outside* the `restoreRequest.current === requestId` check, and `closePullReview` (line 2518) bumps `restoreRequest` but not `hostWorkflowEpoch`. Move the return inside the guard (returning `null` when superseded), or the new notice can paint an error for a review the user already closed.
4. Drop the now-dead `setError(message)` at line 2239; keep `setRestoreError` for the case where a plan already exists.

**Verify:** point a storage at a bucket with revoked credentials, click Pull, expect a red `action_failed` notice with Retry.

#### F5 · The Windows build cannot save an agent profile
`HIGH` · `PLAUSIBLE` · `ProjectBindingEditor.tsx:101` + 4 more sites

Five sites gate absolute paths on `path.startsWith("/")`: `ProjectBindingEditor.tsx:101`, `ProjectSetupWorkspace.tsx:172` and `:436`, `AddCatalogScopeWorkspace.tsx:203` and `:212`. `C:\Users\me\.codex` never satisfies it, and `expandHomePath` joins with `/` so even the `~/` route fails. On Windows the hint reads "Paths must be absolute — start with / or ~/.", no probe runs, and Save is permanently disabled with no in-UI workaround. `.github/workflows/release.yml` publishes `x86_64-pc-windows-msvc` NSIS installers, so this is a shipping platform. **Second trigger on all platforms:** if `homeDir()` rejects, `homePath` is `null`, `~/.codex` stays unexpanded, and the same permanent disable occurs on macOS.

**Plan**
1. Add one predicate beside `expandHomePath` in `ProjectSetupWorkspace.tsx`:
   ```
   export function isAbsoluteProfilePath(value: string): boolean {
     return value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\");
   }
   ```
   (the last arm covers UNC shares).
2. Replace all five gates with it. `AddCatalogScopeWorkspace` must **not** import from `project-sync` — that boundary is deliberate and documented in its own comment at lines 36-38 — so duplicate the six-line helper there exactly as `expandHomePath` already is.
3. Fix both copies of `expandHomePath` (`ProjectSetupWorkspace.tsx:140-145`, `AddCatalogScopeWorkspace.tsx:28-34`) to strip both separators (`home.replace(/[\\/]+$/, "")`) and join with the separator the home path itself uses.
4. Reword the hint at `ProjectSetupWorkspace.tsx:173` so it does not name only POSIX form.
5. Loosening the gate lets a malformed path reach `probeProviderProfile` — that is correct: the backend probe is the real validator and already returns human-readable refusals.

**Verify:** the only real verification is a Windows run. Failing that, temporarily stub `homeDir()` to return a `C:\...` string in dev and confirm Save enables.

#### F6 · R2 secret access keys are serialised into the webview
`HIGH` · `PLAUSIBLE` · `crates/mallard-domain/src/lib.rs:777` → `src/types.ts:583`

`StorageConfig.secret_access_key` is a plain `String` with `#[serde(default)]` and **no `skip_serializing_if`**, and `get_project_sync_config` (`commands.rs:930`) returns `SyncConfig` straight to the frontend. Every configured R2 secret therefore lives in JS heap and React state from app start, with no user action required. This contradicts the design intent expressed everywhere else: `ConnectR2StorageRequest` is documented as "cannot be debugged or serialized back out", `mallard-cli` and `mallard-application` use `SecretString`, and `ProjectLinksWorkspace.tsx:205-211` deliberately reduces the field to `has_secret_access_key`.

Combined with `"csp": null` (F32) in a webview that renders untrusted pulled transcripts, this is a real defence-in-depth gap rather than a live exploit — the markdown path is genuinely hardened, so it would take a `react-markdown`/`remark` bypass. But that is exactly the scenario a CSP exists for. Any renderer heap snapshot or crash dump also contains all storage secrets in plaintext.

*The input control itself is fine* — `StorageSettings.tsx:389` is `type={secretVisible ? "text" : "password"}` with `autoComplete="new-password"` and a cited NIST rationale. The problem is the read path, not the form.

**Plan**
1. **Backend:** blank `secret_access_key` on every `StorageConfig` before serialising in `get_project_sync_config`.
2. **Preserve on save.** `save_config` is a full-document optimistic save driven by the UI, so blanking the read side would wipe stored secrets on the next save. Change the save path so an empty `secret_access_key` on an *existing* storage id means "keep the stored value", and only a non-empty one replaces it. Add a Rust unit test in `persistence.rs` asserting a blank-secret save preserves the previous value.
3. **Frontend:** remove `secret_access_key` from the `StorageConfig` interface in `src/types.ts` (keep it on the write-side `ConnectR2StorageRequest` at :604) so `tsc` locates every read site. `StorageSettings.tsx:389` becomes a write-only field seeded from `""`; the existing `has_secret_access_key` boolean already covers "is one configured".
4. Drop the `left.secret_access_key === right.secret_access_key` comparison at `ProjectSync.tsx:122` and the pass-through at `:2959`.

**Risk:** this is a Rust + TS change in one commit per the Tauri contract rule, and step 2 is the part that can lose user data if rushed. Write the persistence test first.

#### F7 · 640 KB of design mockups ship inside the released app
`LOW` severity, `CONFIRMED`, **but a 5-minute fix** · `public/session-header-mockup.html` + 2

`public/` is copied verbatim into `dist/` by Vite and `dist/` is `frontendDist` in `tauri.conf.json`. Measured in the current `dist/`: `session-header-mockup.html` 464 K, `storage-menu-mockup.html` 108 K, `storage-menu-a.html` 68 K — **41% of the shipped bundle**, reachable at `mallard.app/session-header-mockup.html` inside the webview.

*For contrast, I verified the `src/` mockups are clean:* the 13 standalone `*-mockup.tsx` entry files and both mockup CSS files do **not** reach the bundle — `grep -c 'history-mockup-' dist/assets/index-*.css` is 0, as is `pp-activity`. Vite has one entry (`index.html`) and tree-shakes the rest. Only the `public/` HTML is a problem.

**Plan:** `git mv` the three files to a new top-level `mockups/` directory alongside the existing root-level `*-mockup.html` specimens (which Vite already leaves alone), or delete them. Update any doc that links them. **Verify:** `npm run build && ls dist` — no `*-mockup*.html`.

#### F8 · `react-icons` is a declared dependency with zero imports
`LOW` · `CONFIRMED` · `package.json`

`grep -rn "react-icons" src` returns nothing; the app uses its own `src/components/Icons.tsx`. **Plan:** `npm uninstall react-icons`, then `npm run build` to confirm.

---

### Wave 2 — Build the safety net

This wave has no user-visible payoff, and it is the highest-value work in the report. Everything in Waves 1, 3 and 4 is a regression risk until it lands.

#### F9 · No test executes any behaviour
`MEDIUM` · `CONFIRMED` · `scripts/run-frontend-integration-tests.mjs:10`

All 13 suites render through `renderToStaticMarkup`. Effects, handlers, focus and scroll never run. Concretely: change `ProjectSync.tsx:547` from `event.key.toLocaleLowerCase() !== "b"` to `!== "B"` and ⌘B silently stops toggling the sidebar — the suite stays green, because the only assertions covering that feature are regexes for `/event\.metaKey/`, `/event\.ctrlKey/` and an `aria-keyshortcuts` string.

**Plan**
1. Add `jsdom` to `devDependencies`. AGENTS.md says prefer existing dependencies — state explicitly that no installed package provides a DOM, so this is a justified addition, not a reflex.
2. Add `tests/frontend/harness.ts` exporting `mountApp(element)`: construct a `JSDOM`, assign `global.window/document/navigator/localStorage/matchMedia/ResizeObserver`, render with `createRoot` inside `act`, return `{ container, unmount }`.
3. Keep the esbuild bundling step in the runner; import the harness first in each new suite.
4. Add `tests/frontend/shell-interaction.integration.test.tsx` covering, at minimum: the ⌘B shortcut, Escape propagation from a nested menu (F3), `SyncReviewTabs` arrow-key focus movement (F17), and one project-switch race (F1).
5. **Do not convert the existing 13 suites.** They are fast (379 ms total) and their static assertions are genuinely useful for markup contracts. Add the DOM layer alongside them.

#### F10 · `tests/` is excluded from typechecking; 62 fixtures already diverge
`HIGH` · `CONFIRMED` · `tsconfig.json:23`

`include` is `["src"]` and `build` is `tsc && vite build`, so no test file has ever been typechecked. Sixty-two fixtures already disagree with `src/types.ts` and with production prop types. The failure mode is precise: add a required DTO field to match a Rust change (exactly what `86a03c1` did with `cloud_thread_id`), update the component, and every suite stays green because the fixtures render `undefined` for it — while production sends it populated and takes a branch no test ever exercised.

**Plan**
1. Add `tsconfig.test.json`: `{ "extends": "./tsconfig.json", "compilerOptions": { "noEmit": true, "types": ["node"] }, "include": ["src", "tests/frontend"] }`, and add `@types/node` (the suites already import `node:test`/`node:fs`).
2. Run it from `scripts/run-frontend-integration-tests.mjs` before the esbuild loop and bail non-zero. **The runner is ESM** (`"type": "module"`), so `require.resolve` throws — use `resolve(projectRoot, "node_modules/typescript/bin/tsc")` or spawn `npx tsc -p tsconfig.test.json`.
3. Fix the 62 errors, highest payoff first: (a) add `loadError`/`onRetryLoad` to `contentDefaults` in `project-activity`; (b) drop `proposed_provider_thread_id`/`keep_both` from `conversationAction()` in `pull-review` and add `cloud_thread_id: null` there and in `conversationRow()`; (c) add the five missing `ThreadSyncEntry` fields to the inline fixtures in `resource-inventory` (there is no `entry()` helper — they are ~11 inline literals).
4. Step 3(b) breaks the four `action.keep_both!.*` assertions at `pull-review` lines 313-336; rewrite them to read `completed.preserved_threads[0]`.

#### F11 · No gate between `projectSyncApi`'s 68 command names and the Rust handler
`MEDIUM` · `CONFIRMED` · `src/components/project-sync/api.ts`

Rename `get_project_activity_workspace` in `lib.rs` and `commands.rs`: `cargo test` passes, `tsc` passes, all 13 suites pass, the app builds and launches — and dies the moment the user opens the activity page.

**Plan:** add one test to `project-sync-cutover.integration.test.tsx` that reads `api.ts`, `storage-catalog/api.ts` and `src-tauri/src/lib.rs` as text, extracts every `invoke("…")` literal from the first two and every identifier in the `invoke_handler![…]` list from the third, and asserts the first set is a subset of the second. This is a source-text assertion, which the current harness handles fine — no DOM needed, so it can land before F9.

#### F12 · The CSS orphan gate reports green with 64 orphan rules
`LOW` · `CONFIRMED` · `tests/frontend/css-hygiene.integration.test.tsx:89`

The check clears a rule when **any** class in the selector chain is live, rather than when all are. **Plan:** split each prelude on `,`; for each selector, strip `:not(...)`, collect its classes, and mark that selector dead if `classes.some(cls => !isLive(cls))`. Report the rule only when *every* comma-separated selector is dead. Expect it to go red immediately — land it together with F35.

#### F13 · A cutover CSS assertion is green on a shadowed declaration
`MEDIUM` · `CONFIRMED` · `tests/frontend/project-sync-cutover.integration.test.tsx:149`

The test asserts a declaration that `RetroTheme.css` later overrides, so it validates a value that never reaches the screen. Change `.sync-sidebar-brand { height }` in `RetroTheme.css` from 56px to 48px — the macOS traffic lights, pinned at `y:29` in `tauri.conf.json`, go off-centre, and the test that exists to prevent exactly that stays green. **422 shadowed (selector, property) pairs** make this unsafe as a general pattern, not just at this line.

**Plan:** replace lines 149-150 with a helper `effectiveDeclaration(selector, property)` that scans `App.css` then `RetroTheme.css` in load order and returns the *last* match, then assert on that. Reuse the `stripComments`/`pairs` logic already in `css-hygiene`. Apply the same helper to every other CSS-value assertion in the suite.

#### F14 · The theme system is entirely untested
`MEDIUM` · `CONFIRMED` · `src/preferences.ts`

Hardcode `resolveTheme` to return `"dark"` and light mode becomes unreachable — Settings still offers Light, clicking it writes `mallard.theme=light`, the app stays dark — and every suite passes.

**Plan:** add a `theme` block to `settings-page.integration.test.tsx` using the F9 harness (or minimally, assign `globalThis.window = { matchMedia, localStorage: memStore }` and `globalThis.document = { documentElement: { dataset: {}, style: {} } }` in `test.before`). Assert: `resolveTheme("light") === "light"`; `resolveTheme("system")` follows `matchMedia`; `applyResolvedTheme` sets both `dataset.theme` and `style.colorScheme`; an absent key resolves to `"system"`; a garbage stored value resolves to `"system"`.

#### F15 · Preference persistence is never round-tripped
`LOW` · `PLAUSIBLE` · `tests/frontend/settings-page.integration.test.tsx:78-102`

No `localStorage` exists in the harness, so `writeKey` silently swallows every write (the `try/catch` in `preferences.ts` is doing its job) and a key collision between two preferences would pass every test. **Plan:** install a minimal in-memory store in `test.before`, remove it in `test.after`, and rewrite the preference tests to assert a real write→read round trip plus key distinctness across all four `mallard.*` keys.

#### F16 · `isIrrecoverableSyncError` gates a destructive button with zero tests
`MEDIUM` · `CONFIRMED` · `src/components/project-sync/syncError.ts:6`

Six substring rules over lowercased English Rust error text decide whether the user is offered "Reset local sync state". **Two of the six match no string the backend produces.** A Rust cleanup that changes `"is stale at {:?}"` to `"is stale at {} digest"` silently turns a genuinely unrecoverable state into an endless Retry loop.

**Plan**
1. Add `tests/frontend/sync-error.integration.test.tsx` asserting `true` for one real message per live rule, copied verbatim from the Rust format strings (`"conversation Push action 'a' is stale at SemanticDigest"`, etc.), and `false` for ordinary transient errors.
2. Delete the two dead rules, or correct them against the current Rust strings — do not leave them as decoration.
3. Better still, replace the string matching with a typed error discriminant from Rust. Substring matching on error prose is a contract that no compiler defends; this belongs in the DTO, not in a regex.

#### F17 · The five largest components are never rendered by any test
`MEDIUM` · `CONFIRMED` · `tests/frontend/project-sync-cutover.integration.test.tsx:133`

244 regexes over component source text stand in for rendering. Rename `completeSetup` and the assertion at lines 56-58 slices from `indexOf(...) === -1`, producing a one-character string and an opaque "input did not match regex" pointing at a rename rather than at any behaviour.

**Plan:** convert file by file, cheapest first, rather than growing the regex layer.
1. `tests/frontend/path-bar.integration.test.tsx` — render `SyncPathBar` (default export, renders fine statically) with a three-crumb trail, an overflow group, and a menu with `disabledReason`. Assert crumb order, `role="menuitem"` vs `menuitemcheckbox`, `aria-disabled` on inert rows, the trail's `aria-label`, and that no `menuitemradio` is emitted.
2. Do **not** delete the corresponding source regexes at `project-sync-cutover:762-768` and `project-activity:253-275` as part of this — they assert cutover invariants (that removed vocabulary stays gone) which a render test does not cover. They are complementary, not interchangeable.

---

### Wave 3 — State and performance

#### F18 · Catalog refetches its whole listing on every unrelated render
`MEDIUM` · `CONFIRMED` · `StorageCatalogPage.tsx:502`

`load` is a `useCallback` with `onScopeGone` in its dep array, and `ProjectSync.tsx:3425` passes a fresh inline arrow every render. Nothing in the tree uses `React.memo`, so every `ProjectSync` render re-creates the callback → re-creates `load` → re-runs the effect → `setLoading(true)` + a fresh `list_scope_resources` round trip. Dragging the sidebar splitter fires dozens of remote listings; the same storm happens per keystroke in the log search box and once per `sync-log` event during a sync. `syncDisabled` (line 990) gates both Push and Pull, so those buttons flicker disabled throughout.

**Plan**
1. Fix it **inside** `StorageCatalogPage` only — a ref for `onScopeGone` works regardless of what the parent passes, whereas hoisting the parent callback holds only until someone passes another inline arrow. Add `const onScopeGoneRef = useRef(onScopeGone); onScopeGoneRef.current = onScopeGone;`, call through the ref, and drop `onScopeGone` from `load`'s deps so its identity depends only on `api`, `chosenStorageId`, `scope.scope.scope_id`.
2. **Decide the real refresh trigger before shipping this.** Today's accidental refetch-per-render is the *only* thing refreshing the Catalog after unrelated app state changes. Once `load` stops re-running, the page holds its listing until Refresh is pressed. Wire an explicit trigger (a sync-completion signal, or the `storages` prop) or you trade a storm for a stale page.
3. Ignore the auditor's suggestion to also ref `notify` — it is not in `load`'s dep array and has no bearing on this bug.

#### F19 · `SyncOutcomeNotice` snaps shut mid-read
`MEDIUM` · `CONFIRMED` · `SyncOutcomeNotice.tsx:77`

The fold-reset key is a `view` object rebuilt inline each render, compared by identity in a render-phase block. After a Push, expand the per-thread rows and start reading: any re-render of the parent — a trailing `sync-log` event, resizing the log drawer, dragging the splitter, the focus reconcile — closes them. Symmetrically, an outcome collapsed by the user re-opens itself.

**Plan:** memoise the view at the source. In `ProjectLinksWorkspace.tsx`, hoist it out of the row-mapping callback: `const outcomeView = useMemo(() => activeNotice?.kind === "outcome" ? syncOutcomeView(activeNotice.outcome) : null, [activeNotice]);`.
**Do not use the `key` approach** the original plan offered as an alternative — `key` only controls remounting, and with a stable key the component is not remounted, so the render-phase block at lines 76-80 still runs every parent render and the fold still closes. It is a non-fix.

#### F20 · A wiped storage keeps listing its deleted repositories
`MEDIUM` · `CONFIRMED` · `ProjectSync.tsx:2990`

"Clear storage data…" → confirm → the toast reports N objects deleted, and the Repositories card below still lists all three with revisions and timestamps. Only pressing Refresh corrects it.

**Plan:** after `clearStorageData`'s `await refresh(workflowEpoch)` and epoch guard, force the child to reload its bundle list. The straightforward route is a re-issued open request, but note that `ProjectLinksWorkspace`'s effect (line 660) short-circuits `loadStorageBundles` when `editingStorage?.storageId === storage.id`, so that guard (line 687) must also change. Prefer the smaller shape: extract the reload into a named helper in `ProjectLinksWorkspace` and expose it, rather than adding a `reloadBundles?: boolean` flag to the request union at `ProjectSync.tsx:348-353` — a per-call flag with two behaviours is the kind of indirection AGENTS.md rules out.

#### F21 · The previous project's errors and spinners follow you to the next project
`MEDIUM` · `CONFIRMED` · `ProjectActivityPage.tsx:1907`

Project A is bound to an unreachable R2 storage and shows "Comparison with Backup failed — …". Click project B, which has no confirmed binding: B renders A's storage error directly above "No agent profile is connected." Switching mid-flight also strands loading flags on.

**Plan:** in the reset effect at line 1907, clear the full per-project slate before dispatching the load — add `setComparisonError(null); setLoadError(null); setActionError(null); setLoading(false); setComparisonLoading(false);` **and `setLoadingMore(false)`**, which is stranded by exactly the same mechanism (set at 1836, cleared only in the guarded `finally` at 1855) and which the original plan missed. The `if (!hasBoundProvider) return;` guards at 1834/1869 can stay.

*Alternative considered and not recommended:* `key={activeProject.local_project_id}` at `ProjectLinksWorkspace.tsx:2228` remounts the page so no state can survive. It is a true cutover and simpler, but it discards mid-flight requests and resets scroll and expansion state on every switch. Take it only if that UX is acceptable.

#### F22 · Every project switch pays a remote round trip for a dead field
`MEDIUM` · `CONFIRMED` · `ProjectSync.tsx:1177`

`loadProjectData` awaits a remote bundle inspect for `readiness` before `setLoading(false)`, and its only consumer is a field nothing renders. On a slow or unreachable bucket that is seconds of avoidable latency on every project click, repeated after every push.

**Plan** (clean cutover per AGENTS.md)
1. Delete `readiness_state` from `LocalProjectSummary` (`src/types.ts:315`) and from the `projects` memo (`ProjectSync.tsx:797`).
2. Delete the `getReadiness` call and its try/catch from `loadProjectData` (1176-1191) and `refreshAfterPush` (1922-1930).
3. **Also remove `readiness?.state` from the dependency array at line 800**, which the original plan missed.
4. Keep the `readiness` *state* itself — `planRestore`/`applyReview`/`runOutcomeAction` genuinely populate it for `RestorePlanView` (lines 2220, 2273, 2797).
5. If `get_bundle_readiness` then has no frontend caller, remove it from `api.ts:273` and the Tauri registry in the same commit.

#### F23 · Session timeline re-parses every markdown body on every render
`MEDIUM` · `CONFIRMED` · `SessionDetailEvent.tsx:111`

`ReactMarkdown` is given a fresh `remarkPlugins` array and a fresh `components` object each render, and nothing is memoised. Load 150 events (three clicks of "Load 50 more", a normal Claude Code session) then expand one tool call: `handleToolExpandedChange` → `setExpandedToolIds`, then `requestToolDetail` → `setToolDetails` twice — three renders, each re-parsing all 150 bodies. At ~3.8 ms per 2 KB body that is ~570 ms of blocked main thread per click.

**Plan**
1. Hoist `const REMARK_PLUGINS = [remarkGfm];` to module scope.
2. Build the `components` object with `useMemo` keyed on `[onOpenProjectPath, onLoadProjectImage, projectImages, projectRoot]`.
3. `const MemoMarkdown = memo(SessionDetailMarkdown)` and use it at both call sites (lines 285, 423); `export default memo(SessionDetailEvent)`.
4. **Memoisation only bites if the props are stable.** `SessionDetailPage.tsx:415-425` passes **six** fresh arrows to the timeline, not the two the original plan named — `onRetry`, `onLoadMore`, `onOpenProjectPath`, `onLoadProjectImage`, plus `handleToolExpandedChange` and `requestToolDetail`. Wrap all six in `useCallback` or the `memo` is dead weight.

#### F24 · `LogPanel` builds an `Intl` formatter per line, per render, while invisible
`MEDIUM` · `CONFIRMED` · `LogPanel.tsx:110-113`

With the drawer closed and 500 retained lines, each `sync-log` event re-renders all 500 invisible rows at ~15 ms of `Intl` construction each — roughly 750 ms of main thread spent formatting timestamps nobody can see, while the Push progress UI tries to animate. The backend emits dozens per run.

**Plan**
1. The whole cheap win is one line: `const formatter = useMemo(() => new Intl.DateTimeFormat(undefined, clockOptions(true)), [clock24]);` and `formatter.format(ts)` at line 292 — `Intl.DateTimeFormat.prototype.format` accepts a number, so the `new Date(...)` allocation goes too. Keep the `clock24` dep to preserve the deliberate live-update behaviour documented at lines 106-109.
2. Extract `const LogRow = memo(function LogRow({ line, formatter }) {…})` so an incoming line reconciles one new row instead of 500.
3. Consider not mounting the list while the drawer is closed — but check `ProjectSync.tsx:3530` and `App.css:492` first, since the drawer may rely on the mounted list for its open animation and for `pinToTail`.

#### F25 · Activity rows build six `Intl` formatters each, per render
`LOW` · `CONFIRMED` · `ProjectActivityPage.tsx:283-295` and `:577-584`

200 session cards × 0.37 ms on every commit-collapse chevron and every review checkbox.

**Plan:** hoist both formatters to module scope behind a small cache keyed on `getPreference("clock24")` so a Settings change still re-formats — `let cachedClock24, cachedDate, cachedTime;` plus a `formatters()` helper that rebuilds only when the flag changes. Then `memo` the row and stabilise the arrow props at the call site (line 1982).

#### F26 · Collapsed tool calls mount up to 3.2 MB of hidden `<pre>` text
`LOW` · `CONFIRMED` · `SessionDetailToolCall.tsx:206-213`

Both payload bodies render into the DOM regardless of `expanded`. A shell-heavy session at 200 tool calls carries ~3,200 invisible elements and has run `readableToolPayload` 400 times.

**Plan:** guard the `<div className="session-detail-tool-body">` contents on `expanded`. The `<details>`/`<summary>` structure and the `onToggle` handler at line 159 stay untouched, so keyboard and native disclosure behaviour is unchanged. Note React re-renders on toggle anyway, so the `useMemo` at line 56 should move inside the guarded branch too.

#### F27 · Directory descendant counts are recomputed per row, per render
`LOW` · `PLAUSIBLE` · `ProjectFilesReviewPage.tsx:263`

Each directory row scans every row to count descendants, inside the row map — O(n²) where n is the project-file count (bounded in `crates/mallard-infrastructure/src/provider_capture.rs:20`).

**Plan:** one `useMemo` keyed on `[rows]` that walks `rows` once, incrementing a counter for each ancestor directory path (reuse the `parentPath` helper at line 51). O(n·depth) instead of O(n²).

#### F28 · Push failures show raw S3 strings where the storage page shows human copy
`MEDIUM` · `CONFIRMED` · `ProjectSync.tsx:2037`

An expired R2 token yields "Couldn't connect to Cloudflare R2. Check the S3 API URL, bucket name, Account ID, and credentials…" on the storage page, and `S3 conditional put 'bundles/9f3c…': dispatch failure: io error: …` on Push two seconds later. Same root cause, contradictory guidance, and the user never learns the fix.

**Plan:** in `openPushChooser`'s catch (line 1802), `publishProject`'s catch (line 2027) and `planRestore` (line 2235), replace `errorMessage(reason)` with `storageAccessErrorMessage(config.storages.find(s => s.id === storageId) ?? null, reason)` — already exported from `model.ts`, and `storageId` is in scope at all three. Keep `errorMessage` for non-storage failures. Verified safe against `isIrrecoverableSyncError`: none of its six substrings collides with the translated copy.

#### F29 · A green success notice for a total failure
`MEDIUM` · `CONFIRMED` · `StorageCatalogPage.tsx:602`, `:667`, `:706`

Push 3 skills, all 3 rejected by the bucket: the shell shows a green check-circle "Pushed 0 skills to storage." above a red list of the three failures. The top-line summary is the affordance people read.

**Plan:** gate each announcement on the outcome in `runPush` (lines 630-632), `runPull` (700-704) and `unlinkStorage` (729-731): when `failures.length > 0`, notify `Pushed ${pushed} of ${selected.length} … — ${failures.length} failed.` with an attention tone. **The `notify` prop at `StorageCatalogPage.tsx:449` is message-only** while `ProjectSync`'s own `notify` takes a tone, so the tone parameter must be threaded through — and because `storage-catalog` must not import `project-sync`, declare the tone union locally in `storage-catalog`.

#### F30 · The app-wide `error` is written by 23 handlers and rendered on 3 surfaces
`MEDIUM` · `CONFIRMED` · `ProjectSync.tsx:356`

Confirm "Remove project", the Rust command fails, `setError` fires, `busy` clears — the row is still there and nothing on screen changed. Same shape for most of the other 22 writers. (This is the same root cause as F4, counted once at the state level.)

**Plan:** render the state unconditionally. Directly under the existing `{backendError && (…)}` block at line 3380 — inside `.sync-workspace`, above `{notice && …}`, a position outside every early return except the host-identity screen — add a sibling `{error && (<div className="sync-callout error" role="alert">…<button onClick={() => setError(null)} aria-label="Dismiss">…)}`. That one surface covers all 23 writers. Then audit which writers should instead route to a *local* error slot (F31) and demote them.

#### F31 · Unrelated failures render inside the rename form
`LOW` · `CONFIRMED` · `ProjectLinksWorkspace.tsx:1860`

Open "Rename project…", leave it open, run "Reset local sync state…" from the same menu, and its failure renders as a `role="alert"` line inside the rename form — attributing a reset failure to the rename.

**Plan:** give the rename form its own error slot. `onRenameProject` already returns `Promise<boolean>` (`ProjectSync.tsx:2427`); change it to `{ ok: true } | { ok: false; message: string }`, stop calling the shared `setError` there, and add a local `const [renameError, setRenameError] = useState<string | null>(null)`.

#### F32 · `activeStorageId` is not validated against the current links
`LOW` · `PLAUSIBLE` · `ProjectSync.tsx:854`

The remembered storage id is used without checking it is still linked, so Push/Pull and the comparison list can target different storages.

**Plan:** apply the validation `loadProjectData` already performs — fall back to the first linked storage when `projectLinks.some(l => l.storage_id === remembered)` is false.

#### F33 · A transient failure wipes every project's Git classification
`LOW` · `PLAUSIBLE` · `ProjectSync.tsx:1827-1831`

`refreshRepositoryKinds` clears the whole map on failure, silently, after which the Push review mislabels a Git repo's files as pushable project content.

**Plan:** on failure, keep the previous map (do nothing) and report through `setBackendError`/`setError` rather than degrading data silently.

#### F34 · The R2 jurisdiction ref survives a storage switch
`LOW` · `CONFIRMED` · `StorageSettings.tsx:183`

Open an EU-jurisdiction R2 storage's settings, then "Add storage" → R2 without leaving the page: the fresh draft inherits `jurisdiction.current === "eu"`.

**Plan:** add `key={storageDraft.id}` to `<StorageEditor>` at `ProjectLinksWorkspace.tsx:1227` so all per-storage editor state resets on a switch. This also fixes the sibling `s3ApiUrlDraft`/`editingS3ApiUrl` refs at `StorageSettings.tsx:179-181`, which carry across storages by the same mechanism.

#### F35 · Nothing guards the window close button during an in-flight Pull
`MEDIUM` · `PLAUSIBLE` · `RestorePlanView.tsx:594`

The UI says "A running pull finishes or fails whole — it can't be cancelled midway", and then the red close button kills the process mid-write.

**Plan:** in `ProjectSync.tsx`, add an effect keyed on the same `busy` flag the footers use, registering `getCurrentWindow().onCloseRequested` from `@tauri-apps/api/window`: when busy, `event.preventDefault()` and `await confirm(…)` before allowing the close. Read `busy` through a ref so the listener does not churn. Verify the `core:window` permission covers the close-requested hook in `capabilities/default.json`.

---

### Wave 4 — Hygiene

#### Finish the abandoned cutovers

| # | Finding | Location | Plan |
|---|---|---|---|
| F36 | Push review special-cases `"codex:session-index"`, a resource id the backend no longer produces. A developer reading lines 274-299 believes there is a live rule to preserve. | `PushResourceWorkspace.tsx:274` | Delete line 274 and 290-299; drop the `resource.resource_id === sessionIndex?.resource_id` guard at 276 so the loop covers all conversations; delete line 405 and inline `conversations` at its use sites. |
| F37 | `RestorePlanView` requires a `profileLabel` prop it never reads — the "Install into &lt;profile&gt;" line it fed was deleted in `f461ed7`, the wiring was not. | `RestorePlanView.tsx:38` | Decide which half was intended. Showing it is likely right (it names the write destination on an approval screen): re-add it to the destructuring and render it near the existing `storageName`. Otherwise delete the prop and every pass-through. |
| F38 | `get_bundle_status` is registered in `lib.rs:123` with no frontend caller, and its orphaned DTO in `types.ts` carries a compat union that no longer matches the Rust struct. | `src/types.ts:621` | Clean cutover in one commit touching Rust and TS together: confirm with `grep -rn "get_bundle_status" src-tauri crates src` (current hits are only definition, registration, and a private helper), then remove the command, its registration, `ResourceStatusReport` and `BundleResourceStatus`. |
| F39 | `anchor_event_id` and its focus hooks are live and unreachable — checkpoint-jump was removed, the contract was not. | `api.ts:362`, `types.ts:1176` | Drop the field from `ProjectSessionTimelineQuery`, the `anchorEventId` key in `api.ts`, `ProjectSessionTimelineRequest` (`session_detail.rs:141`) and `SessionTimelineQuery`, plus the secondary `tabIndex` leftover at `SessionDetailEvent.tsx:384`. |
| F40 | Three live branches and a CSS comment describe a 720px collapsed icon rail that no longer exists and that the 800px `minWidth` makes unreachable. | `ProjectSync.tsx:566` | Delete `if (window.innerWidth <= 720) return;` at 565-568 so `fitSidebarToWindow` always clamps; collapse 186-189 to `Number.isFinite(value) ? clampSidebarWidth(…) : DEFAULT_PROJECT_SIDEBAR_WIDTH`; remove the stale CSS comment. |
| F41 | 216 lines of CSS across **64 rules** can never match any element — whole removed features still styled. My independent scan: **113 of 718 class names** in `App.css` + `RetroTheme.css` never appear as a token in any `src/**/*.ts(x)`; subtract ~10 built by template composition (`log-${level}`, `cat-${kind}`, `quality-${q}`) for the true figure. | `src/App.css:245` and clusters | Land F12 first so the gate goes red and stays red, then delete the clusters. Largest: `storage-link-*` (14), `sync-project-*` (8), `sync-capability-*` (6), `sync-readiness-*` (3). **Exclude anything reachable via `${}` composition** — verify each candidate against the 102 dynamic `className={\`…\`}` sites before deleting. |
| F42 | `.sync-sidebar` is declared twice, 1,400 lines apart; the first block's two-layer gradient is dead. | `src/App.css:927` | Delete the dead `background` at 927-930 and the duplicated `width`/`min-width`, then merge what remains of 2358-2362 into the block at 921 so the selector is declared once. |
| F43 | `App.css` and `RetroTheme.css` redeclare **422 (selector, property) pairs**, ~30 of them layout-critical geometry — so `App.css` edits silently do nothing and the false-green in F13 is systemic. | `src/App.css:2418` | Do not attempt all 422. Write the parser once (both sheets → `(selector, property, value)` triples), filter to the layout property set, and resolve only those ~30 by deleting the shadowed declaration. Then keep the parser as the F13 test helper so new shadowing is caught. |

#### Collapse the duplicated helpers

| # | Finding | Location | Plan |
|---|---|---|---|
| F44 | Three byte formatters; the Library's never emits a `B` unit, so any sub-1.5 KB resource reads "1 KB". | `StorageCatalogPage.tsx:116` | One `formatBytes` in a top-level module both features may import — `src/clock.ts` is the existing precedent for exactly this, or add `src/format.ts` beside it. Use one base consistently (binary, matching the two `project-sync` copies) and emit `B` below 1 KB. |
| F45 | Two `errorMessage` implementations; the Library's skips both the R2 translation and the Bundle→Repo wording pass, so it shows raw AWS SDK text where the Project page shows actionable guidance. | `project-sync/model.ts:137` | Create `src/storageError.ts` at top level (peer of `clock.ts`/`terminology.ts`, so both features import it without breaching the `storage-catalog → project-sync` ban) and move `errorMessage` + `storageAccessErrorMessage` there verbatim, keeping the `userFacingRepoTerms` pass. Note the translation currently reaches exactly one site (`ProjectLinksWorkspace.tsx:634`). |
| F46 | Two duration formatters; the activity-page one has no days tier and skips the seconds/ms normalisation the detail page documents as necessary. | `sessionDetailModel.ts:286` | Move `toMilliseconds` (line 236) and `formatSessionDuration` (line 286) into `src/clock.ts` — already the sanctioned cross-boundary time module — and delete the `ProjectActivityPage.tsx:302` copy. |
| F47 | Two copies of `STORAGE_BLOCKING_STATES` disagree on `"unknown"`, making an unknown project file un-deselectable while it blocks Push. | `ProjectFilesReviewPage.tsx:99` | One exported constant in `project-sync/model.ts`, imported by both `PushResourceWorkspace` and `ProjectFilesReviewPage`. Then decide the single policy for `"unknown"` — keep it out of the blocking set, so an unresolvable file can be deselected and the push proceeds. |
| F48 | Four clipboard implementations; the Library's is the only one with no fallback, so its "Paths and digests" copy silently does nothing where the identical Project popover works. | `StorageCatalogPage.tsx:237` | Add `src/components/sync-review/clipboard.ts` (the shared layer both sides already import) exporting `writeClipboardText(value, fallbackRoot?)`, taking the `ProjectActivityPage.tsx:706-735` version verbatim as the most complete. Point all four sites at it. |
| F49 | Two `expandHomePath` copies, both joining with `/` only. | `ProjectSetupWorkspace.tsx:140`, `AddCatalogScopeWorkspace.tsx:28` | Covered by F5 step 3. The duplication itself is deliberate and documented — keep it, fix both. |
| F50 | A bare `toLocaleString()` bypasses the 24-hour clock preference. | `ProjectFilesReviewPage.tsx:312` | Import `clockOptions` and spread it: `toLocaleString(undefined, { month: "short", day: "numeric", year: "numeric", ...clockOptions() })`. |

#### DTO contract drift

All CONFIRMED against the Rust source. None is currently breaking, and each is a loaded gun for the next developer who trusts the type.

| # | Finding | Location | Plan |
|---|---|---|---|
| F51 | `TailClassification` spells the Rust variant `"divergent"` as `"diverged"`. | `types.ts:1206` | Prefer deletion — `TailClassification` and `SyncCurrency` (1200-1210) mirror a domain type no command returns, and AGENTS.md forbids speculative surface. If kept, correct the literal against `conversation_tail_relation.rs:39`. |
| F52 | `ProjectStorageLink` omits the required, deliberately non-defaulted Rust field `conversation_checkpoints`, so any code rebuilding a link object typechecks and then drops durable state. | `types.ts:213` | Add `conversation_checkpoints: Record<string, Record<string, unknown>>;` — the payload is machine-local and never rendered, so `unknown` is honest; do not invent a shape. Comment that it must pass through untouched. |
| F53 | Two inventory fields declared in TS that the Rust serializer never emits (`candidates`, `selected_by_default`), so a component tested against locally-synthesised descriptors ships broken against real ones. | `types.ts:168` and `:148` | Delete `candidates`; make `resources` non-optional to match `commands.rs:336`; simplify `model.ts:79-81` to `return inventory?.resources ?? [];`; delete `selected_by_default` and its assignment. |
| F54 | `keepsLocalConversation`'s second branch can never be true (the backend never emits a Manual action carrying digests), and it plus `restoreActionCopy` hard-code the provider list against an explicitly extensible adapter architecture. | `pullReviewModel.ts:294` | Delete the second disjunct (294-302), leaving the `conversationReviewRows` check — `report_push_eligible`/`report_local_only` are the backend's real signal. Replace the hard-coded prefixes at 144-155 with a provider-neutral parse of the resource id. |

#### Accessibility

The a11y baseline is better than typical — I found only one `onClick` on a non-interactive element in production code, and it correctly carries `role="button"`, `tabIndex={0}`, `aria-expanded` and a key handler. These are the real gaps.

| # | Finding | Location | Plan |
|---|---|---|---|
| F55 | **Arrow-key focus in `SyncReviewTabs` throws.** `selectFromKeyboard` reads `event.currentTarget` inside a `requestAnimationFrame`. Verified in the installed `react-dom@19.2.7` (`react-dom-client.production.js:12299`): `event.currentTarget = null` runs immediately after the listener returns. So the rAF callback throws `TypeError`, focus never moves, and the strip cannot advance past the second tab. **This is the only deferred `currentTarget` read in the codebase** — the other 12 sites are all synchronous and fine. | `SyncReviewTabs.tsx:41` | Capture synchronously: `const strip = event.currentTarget.parentElement;` right after `event.preventDefault()`, then `strip?.querySelector<HTMLButtonElement>(…)?.focus()` inside the rAF. **Do not use a `useRef` instead** — the component early-returns at line 32 (`if (tabs.length < 2) return null;`) before any hook would run, so adding a ref below it breaks the rules of hooks. |
| F56 | "Include all" / "Clear" in a resource group header cannot be activated by keyboard and folds the group instead — the wrapper's `onKeyDown` swallows Enter/Space before the inner button sees them. | `ResourceInventory.tsx:162` | Replace the fake-button wrapper with real elements: make the fold a real `<button aria-expanded>` wrapping only icon/copy/count, and move "Include all" out to be its sibling inside the section header row. Drop `role="button"`, `tabIndex={0}`, the `onKeyDown`, and the `stopPropagation` span entirely. Adjust `.sync-resource-group-header` (`App.css:1303`) and its `:focus-visible` rule (1318). |
| F57 | `role="tree"`/`role="treeitem"` with no focusable items, no arrow-key navigation, and focusable controls nested inside every treeitem. VoiceOver announces "tree, 120 items" and then cannot move. | `ProjectFilesReviewPage.tsx:254`, `:269` | Cheapest and honest: drop the ARIA tree. `role="tree"` → `role="list"` with `aria-label="Project files"`, `role="treeitem"` → `role="listitem"`, delete `aria-level`, keep `aria-expanded` on the expand button only. Implementing the full tree contract is the alternative, and it is a much larger job for a review surface that Tab already traverses correctly. |
| F58 | The catalog row's local-action popup declares `role="menu"` but implements none of the menu keyboard contract — no focus move on open, no arrow keys, no focus restoration on Escape. | `StorageCatalogPage.tsx:326` | The contract is already implemented correctly in `SyncPathBar.tsx` (`PathMenu`'s `onKeyDown`, lines 154-194, plus `menuOptions`/`focusMenuOption`). Extract it into a shared export from that file and reuse it here; give the trigger at line 311 an `id` and `aria-controls`. |
| F59 | Focusable tooltip spans with no accessible name; the attention chip announces a bare number as a live region. | `StorageCatalogPage.tsx:406`, `:1003` | Add `aria-label={rowStamp(row)?.detail}` alongside the existing `data-tooltip`, matching `ProjectActivityPage.tsx:581`, and give the `<time>` a real `dateTime`. Give the chip `aria-label={\`${counts.attention} needing attention\`}`. |

#### UX state completeness

| # | Finding | Location | Plan |
|---|---|---|---|
| F60 | Catalog scope page claims "none stored yet" when the fetch that would have listed them **failed**. | `StorageCatalogPage.tsx:1466` | Add an explicit failure branch before the empty branch: `error && !page` → a `role="alert"` block with the message and a Retry button. Distinguishing empty from failed is the whole point. |
| F61 | Per-item failure lists are `join("\n")`-ed into a box whose CSS collapses newlines, producing one run-on sentence. | `StorageCatalogPage.tsx:632` | Stop pushing a list through a string: add `const [failureRows, setFailureRows] = useState<string[]>([])` and render it as a real `<ul>`. |
| F62 | Settings shows **fabricated** log-policy values with no retry when the stats fetch fails. | `SettingsPage.tsx:254` | Extract the fetch into a `loadStats` `useCallback` with an explicit `logsError` render branch and a Retry button; never render defaults as though they were read values. |
| F63 | Session timeline keeps a stale cursor after a filter change, so "Load 50 more" fires a cursor/filter mismatch and discards the correct page. | `SessionDetailPage.tsx:206` | Add `if (!append) setNextCursor(null);` immediately after `setTimelineLoading(true)`, so a fresh query can never leave a cursor from the previous filter set reachable. Also disable "Load 50 more" while a fresh load is in flight. |
| F64 | Toggling a filter leaves the previous filter's rows on screen with a readout that contradicts them. | `SessionDetailTimeline.tsx:59` | Either drop `&& events.length === 0` from the loading condition and add an `aria-busy` dimmed wrapper so rows do not vanish, or clear the list in the filter effect. Pick one; the current state is neither. |
| F65 | An empty session is reported as a filter problem and points at a toolbar that has no controls. | `SessionDetailTimeline.tsx:76` | Split the empty state in two: pass `hasFilterToggles` down from `SessionDetailPage.tsx:405` and only mention filters when there are filters to relax. |
| F66 | `activeStorageSettingsId` is never cleared on unmount, so the sidebar highlights three active items at once and "Projects" can never render as inactive. | `ProjectLinksWorkspace.tsx:560-562` | Make the reporting effect symmetric: `return () => onStorageEditorChange?.(null);`. This mirrors the pattern already used for `onHeaderModelChange` (`ProjectActivityPage.tsx:1967`). |
| F67 | External markdown links silently do nothing when the opener rejects — the file's own `openExternalUrl` fallback is bypassed by a local duplicate. | `SessionDetailEvent.tsx:40-43` | Delete the local `openMarkdownLink` and the now-unused `openUrl` import; keep the safety gate at the call site (`if (isSafeExternalMarkdownUrl(safeHref)) void openExternalUrl(safeHref);`) so the shared `window.open` fallback applies. |
| F68 | Resource detail grid: label styling leaks onto value cells, boxing every value to 85px and greying it to caption colour, destroying the label/value distinction the grid exists to create. | `App.css:1464` | Narrow line 1464 to `.sync-resource-detail > div:not(.sync-resource-detail-grid) > span`, then delete both `!important`s on 1466 and raise that rule's specificity to match. **Do not** restore the caption colour with `:nth-child(odd)` — it works today only by accident (the Resource ID value is a `<code>`, not a `<span>`, so the count is off). Wrap labels in `<dt>`/`<dd>` in `ResourceInventory.tsx:100-108` instead. |
| F69 | Session-detail header token-quality tint never renders — the `quality-*` class is applied without its required companion class. | `SessionDetailHeader.tsx:100` | `className={\`session-detail-quality quality-${detail.tokens.quality}\`}` — reuses the existing styled vocabulary rather than adding a rule. |

#### Configuration

| # | Finding | Location | Plan |
|---|---|---|---|
| F70 | **No CSP.** `"csp": null` in a webview that renders transcripts pulled from another machine's shared storage and holds plaintext R2 secrets (F6). | `src-tauri/tauri.conf.json:28` | Set an explicit policy. Start from `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' asset: data: blob: https:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; frame-src 'none'; object-src 'none'; base-uri 'none'`. `style-src 'unsafe-inline'` is required by React's inline styles; `img-src https:` is required by the external-image path in `SessionDetailMarkdown`. Test both `npm run tauri dev` and a packaged build — the dev server and the `asset:` protocol have different needs. |
| F71 | The `$HOME/**` scope on `opener:allow-reveal-item-in-dir` is **inert** — verified in `tauri-plugin-opener-2.5.4/src/commands.rs:71`, `reveal_item_in_dir(paths: Vec<PathBuf>)` performs no scope check. The file implies a boundary that does not exist. | `capabilities/default.json:24` | Replace the scoped object at 24-29 with the plain string `"opener:allow-reveal-item-in-dir"` — or delete it, since `opener:default` (line 11) already grants the command. Make the file state what is actually true. |
| F72 | `vite.config.ts` sets no `build.target`, no explicit `sourcemap` policy, and no `minify` choice for the packaged build. | `vite.config.ts` | Low priority, but pin `build.target` to the webview baseline the Tauri version guarantees, and set `sourcemap: false` for release so the packaged bundle does not carry them. |

---

## 4. What was checked and found clean

Recording the negative results so the next audit does not re-litigate them.

- **XSS surface.** No `dangerouslySetInnerHTML` anywhere. `SessionDetailMarkdown` uses `skipHtml`, a `urlTransform`, a project-root descendant allowlist for images and links, `rel="noopener noreferrer"` on external anchors, and routes clicks through the opener plugin so the webview never navigates itself away from the SPA. This is a genuinely well-defended renderer.
- **IPC boundary.** Every `invoke()` in `src/` is inside `project-sync/api.ts` or `storage-catalog/api.ts`. Zero violations of the AGENTS.md rule.
- **Module boundaries.** `sync-review/` imports no feature DTOs and calls no Tauri command. `storage-catalog/` imports nothing from `project-sync/` — including a deliberately duplicated `expandHomePath` with a comment explaining the trade.
- **Type discipline.** Zero `any`, zero `as any`, zero `@ts-ignore`/`@ts-expect-error`. The only two `as unknown as` casts are in a mockup file. `npx tsc --noEmit` is clean under `strict` + `noUnusedLocals` + `noUnusedParameters` + `noFallthroughCasesInSwitch`.
- **Secret input handling.** The R2 secret field is `type="password"` with a deliberate reveal toggle, `autoComplete="new-password"`, and a cited NIST SP 800-63B rationale. The *form* is right; only the read path (F6) is wrong.
- **`localStorage` robustness.** `preferences.ts` wraps every read and write in `try/catch` with a hardcoded fallback, so a locked-down webview or a corrupt value degrades to defaults instead of crashing at boot.
- **Clock correctness.** `src/clock.ts` uses `hourCycle: "h23"` rather than `hour12: false`, with a comment explaining that the latter selects h24 on some ICU builds and renders midnight as `24:06`. That is the correct call and worth preserving.
- **CSS tokenisation.** 2 literal colours in 8,672 lines of `App.css` outside token definitions, one of which is a modal scrim. `RetroTheme.css` defines the palette on `:root`, `:root[data-theme="dark"]` and `:root[data-theme="light"]`, and `preferences.ts` paints `data-theme` before React mounts so light mode never flashes dark.
- **Code splitting.** `SessionDetailPage` is `lazy()`-loaded behind a `Suspense` boundary — a 178 KB chunk kept out of the initial 545 KB bundle.
- **Mockup `.tsx` files.** The 13 standalone entry files and both mockup stylesheets in `src/` do **not** reach the production bundle (verified empirically against `dist/`). They are typechecked by `tsc`, which is a mild drift risk, but they cost nothing at runtime.
- **`onClick` on non-interactive elements.** One in production code, and it is correctly implemented.
- **`console.*` in production code.** One, in a mockup file.

---

## 5. Full index

| ID | Sev | Conf | Finding | Location |
|---|---|---|---|---|
| F1 | High | Conf | Project-switch race shows A's data under B's name | `ProjectSync.tsx:1132` |
| F2 | High | Conf | "Reveal in Finder" permanently dead and silent (×2) | `ProjectLinksWorkspace.tsx:1065`, `:1491` |
| F3 | High | Conf | Escape anywhere discards the entire Push review | `PushResourceWorkspace.tsx:394` |
| F4 | High | Conf | Failed Pull plan renders nothing at all | `ProjectSync.tsx:2369` |
| F5 | High | Plaus | Windows build cannot save an agent profile (×5 sites) | `ProjectBindingEditor.tsx:101` |
| F6 | High | Plaus | R2 secret keys serialised into the webview | `mallard-domain/src/lib.rs:777` |
| F7 | Low | Conf | 640 KB of mockups ship in the released app | `public/*.html` |
| F8 | Low | Conf | `react-icons` declared, never imported | `package.json` |
| F9 | Med | Conf | No test executes any behaviour | `run-frontend-integration-tests.mjs:10` |
| F10 | High | Conf | `tests/` untypechecked; 62 fixtures diverge | `tsconfig.json:23` |
| F11 | Med | Conf | No gate on 68 command names vs the Rust handler | `project-sync/api.ts` |
| F12 | Low | Conf | CSS orphan gate green with 64 orphan rules | `css-hygiene.integration.test.tsx:89` |
| F13 | Med | Conf | Cutover CSS assertion green on a shadowed declaration | `project-sync-cutover.integration.test.tsx:149` |
| F14 | Med | Conf | Theme system entirely untested | `preferences.ts` |
| F15 | Low | Plaus | Preference persistence never round-tripped | `settings-page.integration.test.tsx:78` |
| F16 | Med | Conf | `isIrrecoverableSyncError` untested; 2 of 6 rules dead | `syncError.ts:6` |
| F17 | Med | Conf | 5 largest components never rendered; 244 source regexes | `project-sync-cutover.integration.test.tsx:133` |
| F18 | Med | Conf | Catalog refetches on every unrelated render | `StorageCatalogPage.tsx:502` |
| F19 | Med | Conf | `SyncOutcomeNotice` snaps shut mid-read | `SyncOutcomeNotice.tsx:77` |
| F20 | Med | Conf | Wiped storage keeps listing deleted repositories | `ProjectSync.tsx:2990` |
| F21 | Med | Conf | Previous project's errors and spinners carry over | `ProjectActivityPage.tsx:1907` |
| F22 | Med | Conf | Remote readiness round trip for a dead field | `ProjectSync.tsx:1177` |
| F23 | Med | Conf | Markdown re-parsed for all events on every render | `SessionDetailEvent.tsx:111` |
| F24 | Med | Conf | `Intl` formatter per log line, per render, while hidden | `LogPanel.tsx:110` |
| F25 | Low | Conf | Six `Intl` formatters per activity row per render | `ProjectActivityPage.tsx:283` |
| F26 | Low | Conf | Up to 3.2 MB of hidden `<pre>` in collapsed tool calls | `SessionDetailToolCall.tsx:206` |
| F27 | Low | Plaus | O(n²) descendant counting inside the row map | `ProjectFilesReviewPage.tsx:263` |
| F28 | Med | Conf | Raw S3 strings on Push, human copy on the storage page | `ProjectSync.tsx:2037` |
| F29 | Med | Conf | Green success notice when every item failed | `StorageCatalogPage.tsx:602` |
| F30 | Med | Conf | `error` written by 23 handlers, rendered on 3 surfaces | `ProjectSync.tsx:356` |
| F31 | Low | Conf | Unrelated failures render inside the rename form | `ProjectLinksWorkspace.tsx:1860` |
| F32 | Low | Plaus | `activeStorageId` not validated against current links | `ProjectSync.tsx:854` |
| F33 | Low | Plaus | Transient failure wipes all Git classification | `ProjectSync.tsx:1827` |
| F34 | Low | Conf | R2 jurisdiction ref survives a storage switch | `StorageSettings.tsx:183` |
| F35 | Med | Plaus | Window close unguarded during an in-flight Pull | `RestorePlanView.tsx:594` |
| F36–F43 | Low | Conf | Eight unfinished cutovers (dead ids, props, commands, CSS) | see table |
| F44–F50 | Low | Mixed | Seven duplicated helpers, already drifted | see table |
| F51–F54 | Low | Conf | Four DTO contract drifts | `src/types.ts` |
| F55–F59 | Low–Med | Conf | Five accessibility defects | see table |
| F60–F69 | Low–Med | Mixed | Ten UX-state and CSS defects | see table |
| F70–F72 | Low | Plaus | No CSP, inert scope entry, unpinned build target | `tauri.conf.json`, `capabilities/`, `vite.config.ts` |

---

## 6. Appendix — evidence

Reproducible commands behind the quantitative claims.

```
npx tsc --noEmit                                          # 0 errors
npm run test:frontend-integration                          # 219/219 in 379ms
grep -rn "invoke(" src --include='*.ts' --include='*.tsx' | grep -v api.ts   # 0
grep -rn "dangerouslySetInnerHTML\|@ts-ignore\|as any" src                   # 0
grep -rc "= useState" src/components/project-sync/ProjectSync.tsx            # 75
grep -rn "memo(" src/components | wc -l                                      # 0
grep -rn "renderToStaticMarkup" tests/frontend | wc -l                       # 127
ls -la dist/assets && du -sh dist/*mockup*.html                              # 640K shipped
grep -c "currentTarget = null" node_modules/react-dom/cjs/react-dom-client.production.js
```

Full machine-readable results, including the 16 refuted findings and every verifier's reasoning, are in the workflow journal at
`~/.claude/projects/-Users-hequ-Desktop-project-mallard-app/…/subagents/workflows/wf_0c66661b-759/journal.jsonl`.

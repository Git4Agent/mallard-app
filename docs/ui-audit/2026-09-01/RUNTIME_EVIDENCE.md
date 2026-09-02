# Runtime-verified findings (main session, computed-style evidence)

Base: dirty working tree on 789eea5. Real App.css (1605 rules) rendered at
http://localhost:1420 via the running Vite dev server. Baloo 2 face loads (status "loaded").
Baseline: 269/269 frontend tests + tsc green — every finding below passes the existing gates.

## R1 — Status badges fall under the project's own 4.5 floor on an active row (light theme)
tests/frontend/css-hygiene.integration.test.tsx:98 "every status badge reads on the paper
of its own theme" measures the seven `--thread-status-*` badges against `--surface-row-hover`,
with the comment calling it "the harder of the two surfaces a badge sits on".
That rationale is inverted in light theme: `--surface-row-active` (#f0e3d5) is DARKER than
`--surface-row-hover` (#faf2e9), so the active row is the harder surface for dark ink.
Measured on the surface the gate skips (light theme):
  divergent  #765ba7  hover=4.95 ok   active=4.35  FAIL
  conflict   #be3824  hover=5.01 ok   active=4.41  FAIL
  (synced 5.29, cloud 4.59, local 4.95, pending 4.72, unavailable 4.72 pass)
Dark theme passes on both surfaces (worst 5.50).
Sharpener: the SAME test file, at its third assertion (line ~120), tests `--text-3` against
`--surface-row-active` explicitly — so the active row is known to matter; the badge
assertion just omits it.
Impact: on a selected/active row in light theme, the "divergent" and "conflict" status
badges sit below the contrast bar this project enforces everywhere else.
Fix: add `--surface-row-active` to the surface list in the badge test, then lift
`--thread-status-divergent` and `--thread-status-conflict` until they clear 4.5 there.

## R2 — `--danger-text` violates the invariant its own comment asserts
src/App.css:8170 comment: signal hues hold ">=4.5:1 on every paper surface down to
--surface-row-active (#f0e3d5)".
Measured #be3824 on #f0e3d5 = 4.405  (independently recomputed, not estimated).
The other three clear it: success 4.585, warning 4.562, info 4.592.
The hygiene gate misses this because it only measures signal inks on their own
`--*-soft` fills composited over `--surface-card`, never on an active row.
Fix: darken `--danger-text` slightly in the light palette, or correct the comment's claim.

## Verified-as-FIXED (do not re-report)
- M04 focus indicators: `--focus-ring` now measures 12.49 (light) / 12.51 (dark) worst-case
  against 11 opaque surfaces — far above the 3.0 non-text floor. 62 `:focus-visible` rules
  vs 3 `outline: none`, and all three of those are non-interactive readouts or carry an
  explicit `:focus-visible` replacement (App.css:6383+1).
- H03 muted text: `--text-2`/`--text-3` (#6f6159 light / #a59d8f dark) now clear 4.5 on every
  declared opaque surface (worst 4.72 light, 5.74 dark). NOTE: an earlier in-session reading
  of 3.98 came from a MOCKUP-local band colour (.spec-title on #dfd1c4), not a production
  surface — that reading does not stand.

## Still open from the prior audit, confirmed live
- Vite-only startup leaks a raw JS error string to the user:
  rendered text = "Error · Cannot read properties of undefined (reading 'invoke')"
  (captured from the real app at http://localhost:1420/).

## Scope limits
Specimens (activity/session-detail/menu mockups) load the real App.css, so TOKEN and CASCADE
findings from them transfer; their own layout does not. The production React tree cannot
render in a browser (Tauri `invoke` absent), so no production-DOM layout/keyboard evidence
was collected here.

## R3 — Startup failure is never announced to assistive tech (production DOM)
Captured from the real production React tree at http://localhost:1420/ (the one production
surface that renders without the Tauri backend).
DOM audit of the failure state:
  headings   ["H1: Host identity unavailable"]        <- good
  landmarks  ["main"]                                  <- good
  html lang  "en"                                      <- good
  aria-live / role=status / role=alert elements: []    <- NONE
  error node: <div class="notice-line"> role=null aria-live=null
              "Error - Cannot read properties of undefined (reading 'invoke')"
The failure replaces the startup content after mount. With no live region anywhere on the
page, a screen-reader user gets no announcement that the app failed to start; they must
discover the changed heading manually.
Fix: give the failure notice role="alert" (or a role="status" live region that the startup
sequence writes into), so the transition from "loading" to "failed" is spoken.

## R4 — Internal JS message shown as user-facing error copy (production DOM, still open)
Same capture. The `.notice-line` renders the raw exception text
"Cannot read properties of undefined (reading 'invoke')" verbatim.
This is an internal property-access failure, not a cause a user can act on, and it names an
implementation symbol. Prior audit flagged this shape; it is still live.
Fix: map transport-level failures to a stated cause and next step; keep the raw string for
the Logs drawer only.

## R5 — "Retry" button has no accessible description of what it retries
Same capture: the only focusable element is <button>Retry</button> with aria-label=null.
Name is fine in isolation but the button is the sole control on a dead-end screen.
Low severity; noted for completeness.

## R6 — LATENT (not a live defect): base .btn-primary carries a failing colour, masked by call-site convention
Swept all 115 App.css rules that set BOTH `color` and `background` in one block and measured each
self-contained pair in both themes. Only 2 of 115 fell under 4.5:1, and both are the same rule:

  src/App.css:334   .btn-primary { background: var(--button-primary-bg); color: var(--text-0); }
      light  #1e1817 on #8a6347 = 3.30      dark  #f6f1e6 on #ad805a = 3.10

That rule is overridden at src/App.css:8523 by the RetroTheme layer:
  .btn.btn-primary, .sync-app .btn-primary { color: var(--button-primary-text); }
      light  #fffaf4 on #8a6347 = 5.11 ok   dark  #161210 on #ad805a = 5.33 ok
      hover: light 5.99 ok, dark 6.97 ok

VERDICT: NOT a live defect. All 16 production call sites use the doubled class
`className="btn btn-primary ..."` (specificity 0,2,0 beats 0,1,0) and the whole tree is inside
`.sync-app` (ProjectSync.tsx:3634), so the correct token always wins.
It is a LATENT trap: one `className="btn-primary"` without `btn`, or a portalled button outside
`.sync-app` that does not double the class, silently drops the label to 3.1:1. This is the same
"`.btn` variants need doubled class vs RetroTheme" hazard already recorded in memory.
Fix (cheap): change the base rule at App.css:334 to `color: var(--button-primary-text)` so the
fallback is safe rather than failing. No visual change to anything shipping today.

POSITIVE SIGNAL: 113/115 self-contained colour+background pairs clear 4.5:1 in both themes.
The retheme's own pairings are clean; the defects found (R1, R2) are in pairings that are
composed across rules, which is exactly where the source-text gates cannot see.

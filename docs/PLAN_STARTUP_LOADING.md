# The startup screen: the mark bobs, six cells count the real stages

- Status: planned 2026-10-03; the owner picked direction A ("Go with A").
  Not implemented.
- Date: 2026-10-03
- Scope: `StartupLoadingScreen`, the screen between launch and the
  workspace. Frontend only: no Rust, no IPC, no schema change. The
  host-identity failure screen (`sync-host-recovery`) and the in-page loading
  scaffolds are out of scope.
- Visual contract: `mockups/startup-loading-mockup.html?dir=a` (Vite dev server).
  Try `moment=live` with each `boot`, and both themes. Its Before frame
  renders the shipped component this plan replaces.

## Lineage

The owner asked for a better loading screen with animation, as a plan and a
mock first. The specimen offered A (the mark bobs, six stage cells), B (the
duck paddles a route of markers), C (the welcome ride plate) and D (a stage
checklist). The owner picked A.

## What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | **The progress fill is never drawn.** The retro layer gives the 2 px track a 1 px border. Under the global `border-box`, the fill `<span>` (height 100%) and its glint therefore measure 0 px high. What shows is an empty outline. Measured by computed style in the specimen. | `App.css` `.sync-startup-progress-track` (base ~1128, retro ~13024) |
| 2 | The widths it would fill to are invented: 12 / 28 / 46 / 64 / 80 / 96 %. Mallard measures no fractional progress, only which stage it is in. | `StartupLoadingScreen.tsx` `STARTUP_STAGES[].progress` |
| 3 | The content rises 5 px on a 360 ms cubic-bezier. That is a sub-pixel transform on crisp pixel art, the WebKit layer flicker that App.css's motion note warns about. | `.sync-startup-content` `sync-startup-rise` |
| 4 | The only motion is a 7 px pulse dot. During a slow `repair` stage nothing on screen reads as working. | `.sync-startup-mark::after` |
| 5 | There is no gate. A warm boot flashes the screen for a few hundred ms before the shell. | `ProjectSync.tsx` `if (!hostIdentityShellReady)` |
| 6 | The mark is an `<img>` that a filter inverts in dark mode, so its cells cannot be animated or themed. | `.sync-startup-mark img`, dark invert list (~10519) |

## What the user sees

1. Launch. For the first 150 ms there is only the window background (the
   same retro grid as today). A boot that finishes inside 150 ms goes
   straight to the workspace and never shows the screen.
2. After 150 ms the content appears at once, with no entrance motion:
   - the Mallard duck at 4 px a cell (80 px), sitting on a pixel waterline;
   - **Mallard** below it in the display face (Baloo 2 heavy, `--type-3xl`);
   - six square cells, one per real startup stage;
   - the status line, e.g. *Finding projects…*.
3. While it waits:
   - the duck bobs down one cell and back on the 0.9 s app clock;
   - every other beat its head dips forward one cell;
   - the waterline drifts left one cell per beat, with a sparser ripple row
     below it;
   - done cells are filled, the current cell blinks on the same clock, and
     pending cells are hollow.
4. Each stage change fills its cell and rewrites the status line. A slow
   `repair` keeps the duck bobbing on cell 3 with *Finishing interrupted
   work…*, so it never looks frozen.
5. When the shell is ready it replaces the screen in one update, as today.

With reduced motion, the duck sits up on still water and the current cell is
filled. Cells and status still update.

## The rules

- **R1 Honest progress.** Progress is the stage index and nothing else.
  - The cell count is `STARTUP_STAGES.length`; there is no percentage and no
    creeping fill.
  - The cells carry today's progressbar contract: `role="progressbar"`,
    `aria-label="Mallard startup progress"`, `aria-valuemin={1}`,
    `aria-valuemax={STARTUP_STAGES.length}`, `aria-valuenow={index + 1}`,
    and `aria-valuetext` = the status line.
- **R2 One live region.** The status line stays the only `role="status"`
  (polite, atomic). The duck scene is `aria-hidden`. `<main>` keeps
  `aria-labelledby="sync-startup-title"` and
  `aria-describedby="sync-startup-status"`.
- **R3 Pixel-safe motion.**
  - Every startup animation uses `steps()` and moves by whole cells.
  - Only SVG-internal transforms (1 user unit = 1 cell = 4 px) and
    paint-only properties (`background-color`) animate.
  - Nothing animates `transform` or `opacity` on an HTML box, and nothing
    eases.
  - `sync-startup-rise` is deleted.
- **R4 One clock.** Beats run on the app's metronome: 0.9 s for the bob, the
  water and the blink; 1.8 s for the head dip and the ripple. This is the
  same clock as `.status-loader`, `icon-spin` and `loading-bone-glint`.
- **R5 The rest pose is the base style.** No startup animation sets a fill
  mode. The global reduced-motion block (0.01 ms, one iteration) therefore
  ends every loop on its un-animated style, which is authored as the rest
  pose: duck up, head home, water still, current cell filled. No extra
  reduced-motion rule.
- **R6 The 150 ms gate is `hidden`, not unmount.**
  - The content `<section>` renders from the first commit with `hidden`, and
    a 150 ms timer clears it.
  - It must not become `visibility`: WebKit skips overlap testing when a box
    merely turns visible.
  - `.sync-startup-content` sets `display: flex`, so it also needs
    `.sync-startup-content[hidden] { display: none; }`. That pairing is
    load-bearing.
  - Animations in a `display: none` subtree do not run, so the bob starts at
    its first frame when the content appears.
- **R7 The duck is the logo.**
  - The scene draws the logo's own cells, split into head (rows 3–9) and
    body (rows 10–16) so the head can dip.
  - The union must equal the `d` of `public/mallard-logo-duck.svg`. A test
    reads both `d` strings from the rendered markup and pins that, so the
    component exports nothing extra.
  - The other mark sites keep their `<img>`.
- **R8 Hand-off is a cut.** The shell replaces the screen in one update. App
  rule: states land at once.
- **R9 Whole-pixel centring.** `<main>` centres the content, so a fractional
  content height puts the crisp scene on a fractional pixel. The inherited
  `body` line-height (1.35) would make the status line 16.2 px. At the default
  16 px root, every child of the content box therefore has a whole height:
  - scene 80, plus a 12 px margin;
  - h1 26 (`--type-3xl` at `line-height: 1`), plus a 20 px margin to the
    cells;
  - cells 10, plus a 12 px margin;
  - status 18 (`--type-sm` 12 px at `line-height: 1.5`).

  The content is 178 × 292, both even. An even window dimension then lands on
  whole pixels. An odd one lands on .5, which is one device pixel on a 2×
  display. On a 1× display at an odd window size the scene can sit on a half
  pixel. If that shows, snap the top margin with CSS `round()`; never use a
  transform.

## Where

### `src/components/project-sync/StartupLoadingScreen.tsx`

- `STARTUP_STAGES` loses `progress`, keeping `{ id, status }`. Delete
  `progressStyle` and the `CSSProperties` import.
- Add module constants `DUCK_HEAD` and `DUCK_BODY` (not exported): the
  logo's rects as `[x, y, w, h]`.
  - Head: `[13,3,3,1] [12,4,5,2] [12,6,7,1] [12,7,3,3]`
  - Body: `[7,10,9,1] [2,11,15,1] [1,12,16,2] [2,14,15,1] [3,15,13,1] [5,16,10,1]`
  - Add a `cells()` helper that writes them as `M x y h w v h h -w Z`.
- Add the gate: `const [pastGate, setPastGate] = useState(false)` plus an
  effect that runs `window.setTimeout(() => setPastGate(true), 150)` and
  clears the timer on cleanup. This mirrors `LoadingScaffold`; there is no
  shared hook for two call sites.
- New markup, inside the unchanged `<main>` and drag region:
  ```tsx
  <section className="sync-startup-content" hidden={!pastGate}>
    <svg className="sync-startup-scene" viewBox="0 0 30 20" width={120} height={80}
         shapeRendering="crispEdges" aria-hidden="true">
      <g transform="translate(5 0)">
        <g className="sync-startup-duck">
          <path className="sync-startup-duck-head" d={cells(DUCK_HEAD)} />
          <path d={cells(DUCK_BODY)} />
        </g>
      </g>
      <rect className="sync-startup-under" x={0} y={17} width={30} height={1} />
      <g className="sync-startup-water">{/* 10 rects: x = i*4, y 16, 3×1 */}</g>
      <g className="sync-startup-ripple">{/* 6 rects: x = 2 + i*7, y 18, 2×1 */}</g>
    </svg>
    <h1 id="sync-startup-title">Mallard</h1>
    <div className="sync-startup-cells" role="progressbar" …R1…>
      {STARTUP_STAGES.map((s, i) => <i key={s.id} className={i < index ? "done" : i === index ? "now" : undefined} />)}
    </div>
    <p id="sync-startup-status" className="sync-startup-status" role="status" aria-live="polite" aria-atomic="true">…</p>
  </section>
  ```
- The CSS `transform` on the inner `<g>` overrides an SVG `transform`
  attribute on the same element. That is why the 5-cell offset sits on an
  outer `<g>`.
- The `under` rect is paper below the waterline. It swallows the duck's
  bottom row on the down beat, so the duck sinks instead of poking out under
  the line.

### `src/App.css`

**Base block (~1059–1180). Replace:**
- `.sync-startup-content`
  - Keep `width: min(292px, calc(100vw - 64px))` (even, so centring lands
    on whole pixels) and the flex column.
  - Drop the animation.
  - Add the `[hidden]` pairing (R6).
- `.sync-startup-content > h1`
  - `margin: 0`, `color: var(--text-0)`, `font-family: var(--font-display)`,
    `font-size: var(--type-3xl)`, `font-weight: var(--weight-heavy)`,
    `line-height: 1`, `letter-spacing: 0`.
- `.sync-startup-scene`
  - `display: block`, `margin-bottom: 12px`.
  - Paths `fill: var(--text-0)`.
  - `.sync-startup-under` `fill: var(--bg-0)`.
  - Water rects `fill: var(--text-2)`.
  - Ripple rects `fill: color-mix(in srgb, var(--text-2) 55%, transparent)`.
- `.sync-startup-cells`
  - `display: flex`, `gap: 6px`, `margin-top: 20px`.
  - `> i`: 10×10, `box-sizing: border-box`,
    `border: 2px solid var(--text-2)`.
  - `.done` and `.now`: `border-color` and `background: var(--text-0)`.
  - `.now` adds the blink.
- `.sync-startup-status`
  - `margin-top: 12px`, `color: var(--text-2)`, `font-size: var(--type-sm)`,
    `line-height: 1.5` (R9), and the existing nowrap/ellipsis.

**Cascade (checked 2026-10-03):** no later App.css rule matches
`.sync-startup-content > h1` or `.sync-startup-status`. The only one was the
retro layer's `.sync-startup-brand h1`, which this plan deletes, and no
app-voice list names a startup selector. The new rules go in the base block
only. The retro layer keeps just the shared grid background.

**Keyframes, all with `steps(1, end)` unless noted.** Every animation is
`infinite` with no fill mode, so the base stays the rest pose (R5).
- `sync-startup-bob` (0.9 s): 0% `translate(0,0)`; 50% and 100%
  `translate(0,1px)`.
- `sync-startup-dip` (1.8 s): 0% `(0,0)`; 75% `(1px,1px)`; 87.5% and 100%
  `(0,0)`.
- `sync-startup-water` (0.9 s, `steps(4, end)`): to `translate(-4px,0)`.
- `sync-startup-ripple` (1.8 s, `steps(7, end)`): to `translate(-7px,0)`.
- `sync-startup-blink` (0.9 s): 0% `background-color: var(--text-0)`; 50%
  and 100% `transparent`.

**Delete:**
- `.sync-startup-brand`, `.sync-startup-mark` (+ `::after`, `img`),
  `.sync-startup-progress`, `-track`, `> span`, `::after`, `> p`.
- Keyframes `sync-startup-rise`, `-pulse`, `-progress-glint`.
- The `.sync-startup-mark img` size rule (~10514). Its comment ("Every mark
  is the same 20-unit vector…") moves to the dark invert list, and
  `.sync-startup-mark img` leaves that list.
- In the retro layer (~13008–13040): the mark, brand and progress-track
  rules. `.sync-startup-screen, .sync-host-recovery` (the grid background)
  stays.

**Comment fix:** the loading-scaffold comment (~162) says the shimmer "rides
the startup glint's 1.8s clock". That clock is gone. It becomes "the app's
1.8s clock".

### `src/components/project-sync/ProjectSync.tsx`

No change. `setStartupStage` and `<StartupLoadingScreen stage={startupStage} />`
stay as they are.

## Decisions

- **No minimum hold.** A's first frame (duck and name) reads at once, and a
  hold would delay every warm start just to show the screen. The specimen's
  *Gate + 600 ms hold* stays as the reference if this is ever revisited.
- **No pre-mount paint.** `index.html` paints nothing before the bundle's
  CSS. A white frame before mount is possible but unverified (see Risks). The
  CSP allows inline `<style>` but not script, so an explicit theme choice
  could not be honoured there anyway.
- **One component, no shared gate hook.** The second 150 ms gate mirrors
  `LoadingScaffold`'s four lines. Extracting a hook for two sites adds
  indirection without removing any.
- **The other logo sites keep `<img>`**: sidebar, landing picker, path bar.
  Only the startup screen animates the mark.

## Not doing

- B's route, C's ride plate, D's checklist.
- Elapsed-time or "this can take a while" copy for a slow `repair`. The
  existing stage line already says what is happening.
- Any change to the shell's load order or stage boundaries.
- The welcome plate's white GIF corners in dark mode. That is a separate
  task: clip `.projects-welcome-art`.

## Risks to verify

1. **WebKit paints the SVG transforms without layer flicker.** SVG-internal
   transforms are not composited in the current engine. Confirm in the live
   Tauri window: watch the duck for half-pixel shimmer, in both themes.
2. **Centring at non-default text sizes.** R9 holds at the default 16 px
   root. A larger OS text size scales the rem type, so the h1 and status
   heights can go fractional. Check one larger text size in the harness. If
   the scene lands off-pixel, snap with `round()` there too.
3. **The pre-mount white frame.** Launch the packaged app in dark mode and
   look for a white flash before the grid background. If it exists, it is a
   separate fix.
4. **Real boot times.** No warm or cold boot has been measured; the
   specimen's 0.3 / 1.2 / 7 s are assumptions. A holds up at any length, but
   the gate's value depends on how often warm boots beat 150 ms.

## Tests

Update `tests/frontend/project-sync-cutover.integration.test.tsx`. Every
assertion below is either rendered markup or computed from the source.

**Retarget "the startup screen communicates minimal, staged loading
progress"** (render `stage="projects"`). Assert:
- `<h1 id="sync-startup-title">Mallard</h1>`;
- *Finding projects…*;
- `role="progressbar"`, `aria-valuenow="4"`, `aria-valuemax="6"`,
  `aria-valuemin="1"`;
- exactly six cells: three `class="done"`, one `class="now"`;
- no `--sync-startup-progress` and no `%`;
- the content section carries `hidden=""` on first render.

**CSS** (scope the slice from `.sync-startup-screen {` to
`.sync-host-recovery {`, plus the keyframes):
- every `animation:` declaration names `steps(`;
- no `cubic-bezier`, `ease` or `opacity` animation;
- no `fill-mode`, `both` or `forwards`;
- `.sync-startup-content[hidden] { display: none; }` exists;
- keyframes `sync-startup-rise|pulse|progress-glint` are gone everywhere in
  App.css;
- `sync-startup-bob` exists.

**New "the startup duck is the logo's own cells"**: read
`public/mallard-logo-duck.svg` and extract its path `d`. From the rendered
markup, take the two `<path d>` strings inside `.sync-startup-scene` (head,
then body), concatenate them, and assert the result equals the logo's `d`.

**Whole-pixel pins (R9)**: in the CSS, `.sync-startup-content > h1` has
`line-height: 1` and `.sync-startup-status` has `line-height: 1.5`.

**Gate source pin**: `setTimeout(() => setPastGate(true), 150)`, with the
timer cleared on cleanup.

**Run:** `npm run build`, `npm run test:frontend-integration`.

**Visual check:**
- Shoot the real component in WebKit with the rig: a scratch harness that
  mounts `StartupLoadingScreen` with the stage stepping, light and dark, at
  2×. Compare against `mockups/startup-loading-mockup.html?dir=a`.
- In the same harness, check by computed style (the live Tauri window cannot
  be captured, so these numbers are the proof):
  - the content box is 178 × 292;
  - the scene's rect `left` and `top` have a fractional part of 0 at an even
    window size, and 0 or .5 at an odd one;
  - the real h1 resolves to Baloo 2, weight 800, 26 px.
- Then the owner's live Tauri cold and warm start (Risks 1–3).

## Checklist

- [ ] `StartupLoadingScreen.tsx`: stages without `progress`, duck cells,
      gate, new markup
- [ ] `App.css`: new base rules and keyframes, `[hidden]` pairing, deletions
      in base / mark size / dark list / retro layer, scaffold comment
- [ ] Cutover test retargeted, CSS motion pins, logo-cells test, gate pin
- [ ] `npm run build` + `npm run test:frontend-integration` green
- [ ] Computed-style check: content 178 × 292, scene offsets whole or .5,
      h1 Baloo 2 / 800 / 26 px
- [ ] WebKit rig shots, light and dark, against the specimen's A
- [ ] Owner's live pass: cold start, warm start, dark mode, reduced motion

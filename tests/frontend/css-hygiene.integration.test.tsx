import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appCss = readFileSync("src/App.css", "utf8");

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/* The seven status badges were one hex apiece serving both palettes, and three
   of them failed outright on cream — a colour picked against near-black cannot
   also read against paper. They resolve per theme now, and this gate is what
   stops the next edit reintroducing a value that only works on one of them.
   It measures the resolved colour, not the source text: the badges reach their
   hue through two or three var() hops, so a gate that read declarations would
   pass a token whose target had drifted. */

/** Every custom property declared in a block whose selector matches. */
function tokensFor(theme: "light" | "dark"): Map<string, string> {
  const css = stripComments(appCss);
  const tokens = new Map<string, string>();
  // Blocks apply in source order; a later block overrides an earlier one, and
  // the dark palette rides the bare :root that also carries the shared tokens.
  for (const block of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = block[1].trim();
    const applies =
      /(^|,)\s*:root\s*(,|$)/.test(selector) ||
      selector.includes(`:root[data-theme="${theme}"]`);
    if (!applies) continue;
    for (const decl of block[2].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) {
      tokens.set(decl[1], decl[2].trim());
    }
  }
  return tokens;
}

function resolve(name: string, tokens: Map<string, string>, seen = new Set<string>()): string {
  assert.ok(!seen.has(name), `${name} resolves in a cycle`);
  seen.add(name);
  const value = tokens.get(name);
  assert.ok(value, `${name} is not declared in this theme`);
  const ref = value.match(/^var\(\s*(--[a-z0-9-]+)\s*\)$/);
  return ref ? resolve(ref[1], tokens, seen) : value;
}

function contrast(a: string, b: string): number {
  const channel = (v: number) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  const luminance = (hex: string) => {
    const m = hex.trim().match(/^#([0-9a-f]{6})$/i);
    assert.ok(m, `${hex} is not a plain 6-digit hex; the gate cannot measure it`);
    const [r, g, b] = [0, 2, 4].map((i) => channel(parseInt(m![1].slice(i, i + 2), 16) / 255));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** Lay a translucent fill over a surface and return the flat colour that
 *  results — what the eye actually reads the ink against. */
function composite(fill: string, surface: string): string {
  const parts = fill.match(/[\d.]+/g)?.map(Number) ?? [];
  const [fr, fg, fb, fa = 1] = parts;
  const under = surface.startsWith("#")
    ? [1, 3, 5].map((i) => parseInt(surface.slice(i, i + 2), 16))
    : (surface.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0]);
  const mix = (f: number, u: number) =>
    Math.round(f * fa + u * (1 - fa)).toString(16).padStart(2, "0");
  return `#${mix(fr, under[0])}${mix(fg, under[1])}${mix(fb, under[2])}`;
}

/** The first rule for `selector` in source order, declarations only. */
function ruleBody(selector: string): string {
  const css = stripComments(appCss);
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(`(?:^|[}\\n])\\s*${escaped}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `no rule for ${selector}`);
  return match![1];
}

test("the primary button's base ink reads on the primary fill without the doubled class", () => {
  // Latent, not shipping: every button on screen carries the doubled-class
  // override, which masked the base rule falling to 2.41:1 in dark when the
  // palette changed. The base rule is the fallback the next component gets.
  const ink = ruleBody(".btn-primary").match(/(?:^|;)\s*color\s*:\s*var\((--[a-z0-9-]+)\)/)?.[1];
  assert.ok(ink, ".btn-primary sets its ink from a token");
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    const measured = contrast(resolve(ink!, tokens), resolve("--button-primary-bg", tokens));
    assert.ok(measured >= 4.5, `${ink} on --button-primary-bg is ${measured.toFixed(2)}:1 on ${theme} — needs 4.5`);
  }
});

test("the focusable status indicator and metric chips wear an opaque focus ring", () => {
  // These spans turn the default outline off, and the app's opaque ring covers
  // only form controls, so they drew a translucent accent wash instead — which
  // measured 2.05-2.86:1 composited on the rows it sits on. A focus indicator
  // needs 3:1, on every row surface, in both themes. The 09-01 audit measured
  // the token instead of the drawn ring, which is how this was closed unfixed.
  for (const selector of [".sync-thread-sync-indicator:focus-visible", ".sync-history-thread-metric:focus-visible"]) {
    const body = ruleBody(selector);
    assert.doesNotMatch(body, /box-shadow/, `${selector} must not draw a translucent ring`);
    const ring = body.match(/outline\s*:\s*2px solid var\((--[a-z0-9-]+)\)/)?.[1];
    assert.ok(ring, `${selector} draws a 2px solid outline from a token`);
    for (const theme of ["light", "dark"] as const) {
      const tokens = tokensFor(theme);
      const colour = resolve(ring!, tokens);
      for (const surfaceToken of ["--surface-row", "--surface-row-hover", "--surface-row-active"]) {
        const measured = contrast(colour, resolve(surfaceToken, tokens));
        assert.ok(
          measured >= 3,
          `${selector} ring ${colour} is ${measured.toFixed(2)}:1 against ${surfaceToken} on ${theme} — needs 3`,
        );
      }
    }
  }
});

test("every notice rung reads on its own fill", () => {
  // A notice prints its ink on its own tint, not on bare paper, and the tint is
  // translucent — so the ratio that matters is against the composite. Light
  // `--danger-text` was picked against paper at 4.9 and measured 4.37 here,
  // which is what this gate exists to catch. The rungs carry the meaning
  // *twice* (the type name is printed in the text) but the text still has to be
  // readable, so this is the 4.5 text floor, not the 3:1 non-text one.
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    const paper = resolve("--surface-card", tokens);
    for (const [ink, fill] of [
      ["--danger-text", "--danger-soft"],
      ["--warning-text", "--warning-soft"],
      ["--info-text", "--info-soft"],
      ["--success-text", "--success-soft"],
    ]) {
      const ground = composite(resolve(fill, tokens), paper);
      const measured = contrast(resolve(ink, tokens), ground);
      assert.ok(
        measured >= 4.5,
        `${ink} on ${fill} is ${measured.toFixed(2)}:1 on ${theme} — needs 4.5`,
      );
    }
  }
});

test("every status badge reads on the paper of its own theme", () => {
  // The seven that carry a verdict. Measured against both row states a badge
  // sits on: the hovered row and the selected or expanded row. On cream the
  // active row is the darker of the two, and it is the one this gate once
  // skipped — `divergent` and `conflict` passed the hovered row at 4.95 and
  // 5.01 while measuring 4.35 and 4.41 on the active one.
  const signals = [
    "synced", "cloud", "local", "divergent", "pending", "conflict", "unavailable",
  ].map((name) => `--thread-status-${name}`);

  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    for (const surfaceToken of ["--surface-row-hover", "--surface-row-active"]) {
      const surface = resolve(surfaceToken, tokens);
      for (const token of signals) {
        const colour = resolve(token, tokens);
        const measured = contrast(colour, surface);
        // 4.5, not the 3:1 WCAG 2.2 sets for non-text: the badge rim is a 1px
        // hairline at the 18px the activity list draws, and the floor assumes a
        // solid shape. Amber `pending` once measured 2.22 here.
        assert.ok(
          measured >= 4.5,
          `${token} is ${colour} on ${theme}, ${measured.toFixed(2)}:1 against ${surfaceToken} ${surface} — needs 4.5`,
        );
      }
    }
    // Tertiary text remains quiet, but it is still normal-size copy throughout
    // the app and must clear the text contrast floor on every opaque surface.
    const neutral = resolve("--text-3", tokens);
    for (const surfaceToken of [
      "--bg-0",
      "--bg-2",
      "--bg-3",
      "--bg-4",
      "--button-bg",
      "--button-bg-hover",
      "--button-bg-active",
      "--surface-card",
      "--surface-row",
      "--surface-row-hover",
      "--surface-row-active",
      "--surface-popover",
      "--surface-footer",
    ]) {
      const opaqueSurface = resolve(surfaceToken, tokens);
      const measured = contrast(neutral, opaqueSurface);
      assert.ok(
        measured >= 4.5,
        `--text-3 is ${measured.toFixed(2)}:1 on ${surfaceToken} in ${theme}`,
      );
    }
    // What marks a thing as chosen — the option row, the setup pill, the
    // setup card — is a translucent accent laid over the paper it sits on,
    // and the quiet line under a chosen name is read against that composite
    // rather than against either token. Only the papers a chosen control
    // actually lands on: the option list's row, a card, and the setup page
    // itself, where the pills and the kind cards sit directly on the shell.
    for (const surfaceToken of ["--surface-row", "--surface-card", "--app-shell"]) {
      const chosen = composite(resolve("--accent-soft", tokens), resolve(surfaceToken, tokens));
      const measured = contrast(neutral, chosen);
      assert.ok(
        measured >= 4.5,
        `--text-3 is ${measured.toFixed(2)}:1 on --accent-soft over ${surfaceToken} in ${theme}`,
      );
    }
  }
});

/* The papers a sentence can land on: the page, a card, a popover, a footer and
   a row in every state. On cream the active row is the darkest of them; in the
   dark palette the order flips and the lightest is the hardest, so the whole
   list is measured rather than one chosen end. Buttons are not paper — their
   labels have recipes of their own. */
const PAPER_SURFACES = [
  "--bg-0",
  "--bg-2",
  "--bg-3",
  "--bg-4",
  "--surface-card",
  "--surface-row",
  "--surface-row-hover",
  "--surface-row-active",
  "--surface-popover",
  "--surface-footer",
];

test("every signal ink reads on every paper down to the active row", () => {
  // The four prose variants of the signal hues carry error, warning, note and
  // success sentences, and a sentence can land on any paper. The light palette
  // says so beside them — "≥4.5:1 on every paper surface down to
  // --surface-row-active" — and light --danger-text measured 4.41 there while
  // clearing its own notice fill, which the fill gate above could not see.
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    for (const ink of ["--danger-text", "--warning-text", "--info-text", "--success-text"]) {
      const colour = resolve(ink, tokens);
      for (const surfaceToken of PAPER_SURFACES) {
        const measured = contrast(colour, resolve(surfaceToken, tokens));
        assert.ok(
          measured >= 4.5,
          `${ink} is ${colour} on ${theme}, ${measured.toFixed(2)}:1 against ${surfaceToken} — needs 4.5`,
        );
      }
    }
  }
});

/** Every custom property declared in a block whose selector is exactly this
 *  one — page-scoped tokens the palette blocks do not carry. */
function scopedTokens(selector: string): Map<string, string> {
  const css = stripComments(appCss);
  const tokens = new Map<string, string>();
  for (const block of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (block[1].trim() !== selector) continue;
    for (const decl of block[2].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) {
      tokens.set(decl[1], decl[2].trim());
    }
  }
  return tokens;
}

/** `color-mix(in srgb, var(--a) P%, var(--b))` as the browser paints it: the
 *  first colour at P, the second at the rest, interpolated per sRGB channel. */
function mixSrgb(expression: string, tokens: Map<string, string>): string {
  const m = expression.match(
    /^color-mix\(in srgb, var\((--[a-z0-9-]+)\) ([\d.]+)%, var\((--[a-z0-9-]+)\)\)$/,
  );
  assert.ok(m, `${expression} is not a two-token srgb color-mix the gate can evaluate`);
  const share = Number(m[2]) / 100;
  const [a, b] = [resolve(m[1], tokens), resolve(m[3], tokens)];
  const channel = (i: number) => {
    const mixed = parseInt(a.slice(i, i + 2), 16) * share + parseInt(b.slice(i, i + 2), 16) * (1 - share);
    return Math.round(mixed).toString(16).padStart(2, "0");
  };
  return `#${channel(1)}${channel(3)}${channel(5)}`;
}

test("the session-detail inks read on an expanded tool row", () => {
  // The page mixes body colour into three signal hues so they survive as text,
  // and prints them on the darkest row state it paints — the expanded tool
  // row's --surface-row-active — not only on the page. Tuned to the page, the
  // link mix once measured 3.94 on the open row, under a 9px uppercase tool
  // name. Evaluated by value: a palette that moves --blue or --text-0 moves
  // the mix with it, and this catches the move.
  const scoped = scopedTokens(".session-detail-page");
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    for (const ink of ["--session-detail-amber", "--session-detail-link", "--session-detail-you", "--session-detail-agent", "--session-detail-live"]) {
      const expression = scoped.get(ink);
      assert.ok(expression, `${ink} is not declared on .session-detail-page`);
      const colour = mixSrgb(expression, tokens);
      for (const surfaceToken of ["--surface-row", "--surface-row-active"]) {
        const measured = contrast(colour, resolve(surfaceToken, tokens));
        assert.ok(
          measured >= 4.5,
          `${ink} resolves to ${colour} on ${theme}, ${measured.toFixed(2)}:1 against ${surfaceToken} — needs 4.5`,
        );
      }
    }
  }
});

/* The grounds a glyph is actually drawn on: the gutter behind the sidebar, a
   card, and a row in the two states that are darker than a card on cream.
   Fewer than PAPER_SURFACES because a mark is placed, not flowed — nothing
   paints a status glyph on a footer or a bare --bg-3. */
const GRAPHIC_SURFACES = [
  "--app-shell",
  "--surface-card",
  "--surface-row-hover",
  "--surface-row-active",
];

/* Properties that put a token in front of the eye directly, rather than
   mixing it into an edge or a wash. */
const PAINTS_DIRECTLY = /^(color|background|background-color|fill|stroke)$/;

test("every graphic ink reads on the ground it is drawn on", () => {
  // The gates above all measure text. Nothing measured a mark until this one,
  // and three tokens were painted as marks: the success tick on an outcome
  // notice, the failed-pull icon, the git-folder mark. 1.4.11 holds a glyph to
  // 3:1, not the 4.5 a sentence carries, because a shape is read by its
  // silhouette — but 3:1 is a floor, not a formality, and light --green sat at
  // 2.85 on the active row while clearing the page at 3.5.
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    for (const ink of ["--green", "--blue", "--red"]) {
      const colour = resolve(ink, tokens);
      for (const surfaceToken of GRAPHIC_SURFACES) {
        const measured = contrast(colour, resolve(surfaceToken, tokens));
        assert.ok(
          measured >= 3,
          `${ink} is ${colour} on ${theme}, ${measured.toFixed(2)}:1 against ${surfaceToken} — needs 3`,
        );
      }
    }
  }
});

/** The colour a rule with exactly this selector gives the property, resolved
 *  through the theme's tokens — the rule is read, so a rule that drifts to
 *  another token is measured as it drifted. */
function ruleColour(selector: string, property: string, tokens: Map<string, string>): string {
  for (const block of stripComments(appCss).matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (block[1].trim() !== selector) continue;
    const decl = block[2].match(new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;]+);`));
    if (!decl) continue;
    const ref = decl[1].match(/var\(\s*(--[a-z0-9-]+)\s*\)/);
    return ref ? resolve(ref[1], tokens) : decl[1].trim();
  }
  assert.fail(`no rule "${selector}" declares ${property}`);
}

test("the switch shows which way it is set", () => {
  // Fill means On. Off is hollow, the card showing through an ink edge, and
  // On is filled, so a lone switch reads without a neighbour to compare with.
  // The old Off was a filled mid-brown bar 3.6:1 off the card: both states
  // read as set and only the thumb's end told them apart. Every layer is
  // measured against the one it sits on (1.4.11), and the word beside the
  // switch is text (1.4.3). The Theme picker's chosen segment is the same
  // question, and was 1.19:1 as honey paper on cream.
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    const card = resolve("--surface-card", tokens);
    const offTrack = ruleColour(".sync-settings-switch", "background", tokens);
    const edge = ruleColour(".sync-settings-switch", "border", tokens);
    const offThumb = ruleColour(".sync-settings-switch::before", "background", tokens);
    const onTrack = ruleColour(".sync-settings-switch:checked", "background", tokens);
    const onThumb = ruleColour(".sync-settings-switch:checked::before", "background", tokens);
    const offWord = ruleColour(".sync-settings-state", "color", tokens);
    const onWord = ruleColour(".sync-settings-toggle:has(:checked) .sync-settings-state", "color", tokens);
    const segment = ruleColour(".sync-settings-segment", "background", tokens);
    const chosen = ruleColour(".sync-settings-segment > button.active", "background", tokens);
    const chosenInk = ruleColour(".sync-settings-segment > button.active", "color", tokens);

    const hollow = contrast(offTrack, card);
    assert.ok(hollow < 1.5, `Off track ${offTrack} is ${hollow.toFixed(2)}:1 on the card on ${theme} — must read hollow (<1.5)`);
    const floors: [string, string, string, number][] = [
      ["the switch edge", edge, card, 3],
      ["the Off thumb", offThumb, offTrack, 3],
      ["the On fill", onTrack, card, 3],
      ["the On thumb", onThumb, onTrack, 3],
      ["the Off word", offWord, card, 4.5],
      ["the On word", onWord, card, 4.5],
      ["the chosen segment", chosen, segment, 3],
      ["the chosen segment's word", chosenInk, chosen, 4.5],
    ];
    for (const [what, ink, ground, floor] of floors) {
      const measured = contrast(ink, ground);
      assert.ok(measured >= floor, `${what} ${ink} on ${ground} is ${measured.toFixed(2)}:1 on ${theme} — needs ${floor}`);
    }
  }
});

test("no glyph is painted in a hue that misses the non-text floor", () => {
  // --amber clears 3:1 on nothing this app paints in the light theme: 2.37 on
  // a card, 2.00 on the active row. Five rules made it a glyph anyway — the
  // sidebar storage warning, the path-menu banner, the profile state, the
  // review tab's dot, the outcome notice — and it was the least readable mark
  // in the theme. A warning mark uses --warning-text now, like the sentence
  // beside it. Mixes stay legal: --yellow at 45% inside a border-color is an
  // edge, and the notice's meaning is in the words it frames.
  const offenders: string[] = [];
  for (const block of stripComments(appCss).matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    for (const decl of block[2].matchAll(/([a-z-]+)\s*:\s*var\(\s*(--[a-z0-9-]+)\s*\)\s*(?:;|$)/g)) {
      if (!PAINTS_DIRECTLY.test(decl[1])) continue;
      if (decl[2] !== "--yellow" && decl[2] !== "--amber") continue;
      offenders.push(`${block[1].trim().split("\n").pop()!.trim()} { ${decl[1]}: var(${decl[2]}) }`);
    }
  }
  assert.deepEqual(
    offenders,
    [],
    `a bare amber is painted as a glyph here — use --warning-text:\n${offenders.join("\n")}`,
  );
});

test("the review meter fill reads on the track it runs along", () => {
  // The meter is a graphic that carries meaning — how much of the limit is
  // spent — so its fill holds 3:1 (1.4.11) on the empty track under it, in
  // both themes. --loader-track is translucent, so it is measured composited
  // on the header paper the track sits on. Read through the rule, so a token
  // drift is measured as it drifted: the pull fill measured 4.00 and 4.76.
  // Only the Pull header draws a track.
  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    const track = composite(resolve("--loader-track", tokens), resolve("--surface-card", tokens));
    const selector = ".sync-review-budget-track > i";
    const fill = ruleColour(selector, "background", tokens);
    const measured = contrast(fill, track);
    assert.ok(
      measured >= 3,
      `${selector} is ${fill} on ${theme}, ${measured.toFixed(2)}:1 against the track ${track} — needs 3`,
    );
  }
});

test("no container query styles its own container", () => {
  // An element is never its own query container, so a rule for the
  // container's own selector inside its `@container` block never applies. The
  // path bar's 640px rung set `gap: 6px` on `.sync-pathbar` that way, and the
  // 6px it counted on at the 580px floor was never given back.
  const css = stripComments(appCss);
  const offenders: string[] = [];
  for (const declared of css.matchAll(/([^{}]+)\{[^{}]*?\bcontainer:\s*([a-z0-9-]+)\s*\//g)) {
    const own = new Set(declared[1].split(",").map((selector) => selector.trim()));
    const name = declared[2];
    for (const query of css.matchAll(new RegExp(`@container\\s+${name}\\s*\\([^)]*\\)\\s*\\{`, "g"))) {
      let depth = 1;
      let end = query.index! + query[0].length;
      while (depth > 0 && end < css.length) {
        if (css[end] === "{") depth += 1;
        else if (css[end] === "}") depth -= 1;
        end += 1;
      }
      const body = css.slice(query.index! + query[0].length, end - 1);
      for (const rule of body.matchAll(/([^{}]+)\{/g)) {
        for (const selector of rule[1].split(",").map((part) => part.trim())) {
          if (own.has(selector)) offenders.push(`@container ${name} styles ${selector}`);
        }
      }
    }
  }
  assert.deepEqual(offenders, [], "a container cannot match its own query");
});

test("a tab strip's rules reach only its own tabs", () => {
  // The cloud picker's Newest/Most stars pill rides inside the review strip.
  // `.sync-sync-review-tabs button { border-radius: 3px }` came later than the
  // pill's own radius at the same specificity and squared its segments.
  const css = stripComments(appCss);
  const offenders: string[] = [];
  for (const rule of css.matchAll(/([^{}]+)\{/g)) {
    for (const selector of rule[1].split(",").map((part) => part.trim())) {
      if (/\.(sync-project-tabs|sync-sync-review-tabs|repo-scope-tabs)(:[a-z-]+)?\s+(?!>)[^\s>+~]*\bbutton\b/.test(selector)) {
        offenders.push(selector);
      }
    }
  }
  assert.deepEqual(offenders, [], "a strip's rule must use `> button` so it skips controls nested in the strip");
});

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
  // The seven that carry a verdict. Measured against the *hovered* row, the
  // harder of the two surfaces a badge sits on.
  const signals = [
    "synced", "cloud", "local", "divergent", "pending", "conflict", "unavailable",
  ].map((name) => `--thread-status-${name}`);

  for (const theme of ["light", "dark"] as const) {
    const tokens = tokensFor(theme);
    const surface = resolve("--surface-row-hover", tokens);
    for (const token of signals) {
      const colour = resolve(token, tokens);
      const measured = contrast(colour, surface);
      // 4.5, not the 3:1 WCAG 2.2 sets for non-text: the badge rim is a 1px
      // hairline at the 18px the activity list draws, and the floor assumes a
      // solid shape. Amber `pending` once measured 2.22 here.
      assert.ok(
        measured >= 4.5,
        `${token} is ${colour} on ${theme}, ${measured.toFixed(2)}:1 against ${surface} — needs 4.5`,
      );
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
  }
});

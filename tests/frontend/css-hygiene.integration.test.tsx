import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";

/* App.css earned three standing defects during its growth: raw font literals
   beside an unused type scale, rules for class vocabularies no component
   renders, and breakpoints below the window floor. Each was cleared in one
   mechanical pass; these gates keep them cleared. */

const appCss = readFileSync("src/App.css", "utf8");
const retroCss = readFileSync("src/RetroTheme.css", "utf8");

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

test("App.css draws type from the scale, not from literals", () => {
  const css = stripComments(appCss);
  // The scale lives in RetroTheme.css (--type-*, --weight-*). Prose-relative
  // em and inherit are fine; a px literal reintroduces an off-scale step.
  assert.deepEqual(css.match(/font-size:\s*[0-9.]+px/g) ?? [], []);
  assert.deepEqual(css.match(/font:\s*[0-9.]+px/g) ?? [], []);
  assert.deepEqual(css.match(/font-weight:\s*\d/g) ?? [], []);
});

test("no media query can hide below the window floor", () => {
  // tauri.conf.json pins minWidth, so narrower max-width blocks can never
  // fire — they read as coverage while being dead code.
  const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const floor: number = conf.app.windows[0].minWidth;
  assert.equal(floor, 800);
  for (const css of [appCss, retroCss]) {
    for (const match of css.matchAll(/@media[^{]*\(max-width:\s*(\d+)px\)/g)) {
      assert.ok(
        Number(match[1]) >= floor,
        `@media (max-width: ${match[1]}px) sits below the ${floor}px window floor`,
      );
    }
  }
});

function sourceVocabulary(): { words: Set<string>; prefixes: string[] } {
  const words = new Set<string>();
  const prefixes = new Set<string>();
  const roots: string[] = ["src"];
  const files: string[] = ["index.html"];
  while (roots.length > 0) {
    const dir = roots.pop() as string;
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) {
        roots.push(path);
      } else if (/\.(tsx?|html)$/.test(entry)) {
        files.push(path);
      }
    }
  }
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/[A-Za-z][A-Za-z0-9_-]*/g)) {
      words.add(match[0]);
    }
    // `foo-${kind}` and "foo-" + kind construct class names at runtime; any
    // class sharing the prefix must be treated as live.
    for (const match of text.matchAll(/([A-Za-z][A-Za-z0-9-]*-)\$\{/g)) {
      prefixes.add(match[1]);
    }
    for (const match of text.matchAll(/["']([A-Za-z][A-Za-z0-9-]*-)["']\s*\+/g)) {
      prefixes.add(match[1]);
    }
  }
  return { words, prefixes: [...prefixes] };
}

test("every class App.css styles is rendered by some component", () => {
  const { words, prefixes } = sourceVocabulary();
  const isLive = (cls: string): boolean =>
    words.has(cls) || prefixes.some((p) => cls.startsWith(p));

  // Per selector, and a selector is dead when ANY class in its chain is: a
  // descendant like `.gone .live` can never match, however live `.live` is.
  // Judging the whole prelude by "every class is dead" passed 80 such rules —
  // whole removed features still carrying their styling.
  const selectorDead = (selector: string): boolean => {
    const withoutNot = selector.replace(/:not\([^)]*\)/g, "");
    const classes = [...withoutNot.matchAll(/\.([A-Za-z_][A-Za-z0-9_-]*)/g)].map((m) => m[1]);
    return classes.length > 0 && classes.some((cls) => !isLive(cls));
  };

  const css = stripComments(appCss);
  const orphanRules: string[] = [];
  for (const match of css.matchAll(/([^{}]+)\{[^{}]*\}/g)) {
    const prelude = match[1].replace(/\s+/g, " ").trim();
    if (prelude.startsWith("@")) continue;
    const selectors = prelude.split(",").map((s) => s.trim()).filter(Boolean);
    // A grouped rule survives while any one of its selectors can still match.
    if (selectors.length > 0 && selectors.every(selectorDead)) {
      orphanRules.push(prelude);
    }
  }
  assert.deepEqual(orphanRules, []);
});

test("cross-file selector collisions can only fall", () => {
  // App.css loads before RetroTheme.css (src/App.tsx), so on a specificity
  // tie the later file wins and the earlier declaration is dead weight. 435
  // (selector, property) pairs were declared in both files when this gate
  // landed; resolving them is incremental work — adding new ones is not.
  // Removing 80 orphan rules took it to 423, and the ratchet moved with it.
  const pairs = (css: string): Set<string> => {
    const out = new Set<string>();
    for (const match of stripComments(css).matchAll(/([^{}@]+)\{([^{}]*)\}/g)) {
      const selectors = match[1].split(",").map((s) => s.replace(/\s+/g, " ").trim());
      const properties = match[2]
        .split(";")
        .filter((d) => d.includes(":"))
        .map((d) => d.split(":")[0].trim());
      for (const selector of selectors) {
        for (const property of properties) {
          out.add(`${selector} :: ${property}`);
        }
      }
    }
    return out;
  };
  const appPairs = pairs(appCss);
  const shared = [...pairs(retroCss)].filter((pair) => appPairs.has(pair));
  assert.ok(
    shared.length <= 423,
    `${shared.length} selector/property pairs are declared in both stylesheets (ratchet: 423)`,
  );
});

/* The seven status badges were one hex apiece serving both palettes, and three
   of them failed outright on cream — a colour picked against near-black cannot
   also read against paper. They resolve per theme now, and this gate is what
   stops the next edit reintroducing a value that only works on one of them.
   It measures the resolved colour, not the source text: the badges reach their
   hue through two or three var() hops, so a gate that read declarations would
   pass a token whose target had drifted. */

/** Every custom property declared in a block whose selector matches. */
function tokensFor(theme: "light" | "dark"): Map<string, string> {
  const css = stripComments(retroCss);
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
    // The three no-verdict badges stay quiet on purpose, but still have to
    // clear the bar that actually governs non-text graphics.
    const neutral = resolve("--text-3", tokens);
    assert.ok(
      contrast(neutral, surface) >= 3,
      `--text-3 is ${contrast(neutral, surface).toFixed(2)}:1 on ${theme}`,
    );
  }
});

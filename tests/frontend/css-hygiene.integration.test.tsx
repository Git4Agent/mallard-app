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

  const css = stripComments(appCss);
  const orphanRules: string[] = [];
  for (const match of css.matchAll(/([^{}]+)\{[^{}]*\}/g)) {
    const prelude = match[1].replace(/\s+/g, " ").trim();
    if (prelude.startsWith("@")) continue;
    // :not(...) contents are exclusions, not existence requirements.
    const withoutNot = prelude.replace(/:not\([^)]*\)/g, "");
    const classes = [...withoutNot.matchAll(/\.([A-Za-z_][A-Za-z0-9_-]*)/g)].map((m) => m[1]);
    if (classes.length > 0 && classes.every((cls) => !isLive(cls))) {
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
    shared.length <= 435,
    `${shared.length} selector/property pairs are declared in both stylesheets (ratchet: 435)`,
  );
});

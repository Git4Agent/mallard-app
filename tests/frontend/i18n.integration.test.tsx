import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdirSync, statSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { formatDuration, formatFullDateTime, formatRelativeTime } from "../../src/clock";
import { formatCompactNumber, formatNumber, joinAnd } from "../../src/format";
import { i18n, OFFERED_LANGUAGES, resolveLanguage, settleValueSpacing, uiLocale, useTranslation } from "../../src/i18n";
import { en } from "../../src/i18n/en";
import { zhCN } from "../../src/i18n/zh-CN";
import { zhBackend } from "../../src/i18n/zh-CN/backend";

/* Every source file but the catalogs themselves: copy lives in `src/i18n`. */
const TRANSLATED_SOURCES = readdirSync("src")
  .filter((entry) => entry !== "i18n")
  .map((entry) => `src/${entry}`)
  .filter((path) => statSync(path).isDirectory() || /\.tsx?$/.test(path));

type Tree = { readonly [key: string]: string | Tree };

function flatten(tree: Tree, prefix = ""): Map<string, string> {
  const out = new Map<string, string>();
  for (const [key, value] of Object.entries(tree)) {
    if (typeof value === "string") out.set(prefix + key, value);
    else for (const [inner, text] of flatten(value, `${prefix}${key}.`)) out.set(inner, text);
  }
  return out;
}

/* The set of values a sentence uses: Chinese may say one twice or once where
   English says it once or twice, but never drop or invent one. */
const placeholders = (text: string) => [...new Set([...text.matchAll(/\{\{\s*([\w.]+)[^}]*\}\}/g)].map((match) => match[1]))].sort();

/** Runs `fn` with the interface in `language`, then puts English back. */
function inLanguage<T>(language: string, fn: () => T): T {
  void i18n.changeLanguage(language);
  try {
    return fn();
  } finally {
    void i18n.changeLanguage("en");
  }
}

test("every namespace has a Chinese catalog with the same keys, plural forms and placeholders", () => {
  assert.deepEqual(Object.keys(zhCN).sort(), Object.keys(en).sort());
  for (const namespace of Object.keys(en) as Array<keyof typeof en>) {
    const source = flatten(en[namespace] as Tree);
    const translated = flatten(zhCN[namespace] as Tree);
    const expected = [...source.keys()].filter((key) => {
      const base = key.match(/^(.*)_one$/)?.[1];
      return !(base && source.has(`${base}_other`));
    });
    assert.deepEqual([...translated.keys()].sort(), expected.sort(), `${namespace}: keys`);
    for (const key of expected) {
      assert.deepEqual(placeholders(translated.get(key)!), placeholders(source.get(key)!), `${namespace}:${key} placeholders`);
      assert.notEqual(translated.get(key)!.trim(), "", `${namespace}:${key} is empty`);
    }
  }
});

test("Chinese copy keeps the owner's terms: 上传/下载, Version and Skill in English", () => {
  const catalogs: Array<[string, Tree]> = [...Object.entries(zhCN) as Array<[string, Tree]>, ["backend", zhBackend as Tree]];
  for (const [namespace, catalog] of catalogs) {
    for (const [key, text] of flatten(catalog)) {
      assert.doesNotMatch(text, /推送|拉取/, `${namespace}:${key} — Push is 上传, Pull is 下载`);
      assert.doesNotMatch(text, /技能/, `${namespace}:${key} — Skill stays English`);
      assert.doesNotMatch(text, /版本/, `${namespace}:${key} — Version stays English`);
      // The owner, 2026-10-08: "固定 is not good" — another storage's claim is 关联.
      assert.doesNotMatch(text, /固定/, `${namespace}:${key} — say 关联, not 固定`);
      // The owner, 2026-10-08: "上传审阅 -> 上传检查" — a review is a 检查.
      assert.doesNotMatch(text, /\u5ba1\u9605/, `${namespace}:${key} — a review is 检查`);
      assert.doesNotMatch(text, /[㐀-鿿](Skill|Version)|(Skill|Version)[㐀-鿿]/, `${namespace}:${key} — a space between Han and Skill/Version`);
    }
  }
});

test("System resolves to English or Simplified Chinese; Traditional follows English; a pick wins", () => {
  const offered = OFFERED_LANGUAGES.includes("zh-CN");
  assert.equal(resolveLanguage("system", ["zh-CN", "en-US"]), offered ? "zh-CN" : "en");
  assert.equal(resolveLanguage("system", ["zh-Hans-CN"]), offered ? "zh-CN" : "en");
  assert.equal(resolveLanguage("system", ["fr-FR", "zh-SG"]), offered ? "zh-CN" : "en");
  assert.equal(resolveLanguage("system", ["zh-TW", "zh-CN"]), "en");
  assert.equal(resolveLanguage("system", ["zh-Hant-HK"]), "en");
  assert.equal(resolveLanguage("system", ["en-GB", "zh-CN"]), "en");
  assert.equal(resolveLanguage("system", ["de-DE"]), "en");
  assert.equal(resolveLanguage("system", []), "en");
  assert.equal(resolveLanguage("zh-CN", ["en-US"]), "zh-CN");
  assert.equal(resolveLanguage("en", ["zh-CN"]), "en");
});

test("English formatting is what it always was; Chinese formats with zh-CN", () => {
  assert.equal(uiLocale(), undefined);
  assert.equal(formatDuration(0, 3_900), "1h 5m");
  assert.equal(formatDuration(0, 45), "45s");
  assert.equal(formatDuration(0, 2 * 86_400 + 3 * 3_600), "2d 3h");
  assert.equal(formatCompactNumber(9_432), "9,432");
  assert.equal(formatCompactNumber(412_880), "413K");
  assert.equal(formatCompactNumber(1_500_000), "1.50M");
  assert.equal(joinAnd(["a", "b", "c"]), "a and b and c");
  assert.equal(formatRelativeTime(Date.now() - 5 * 60_000), "5m ago");
  assert.match(formatFullDateTime(1_786_000_000), / at /);

  inLanguage("zh-CN", () => {
    assert.equal(uiLocale(), "zh-CN");
    assert.equal(formatDuration(0, 3_900), "1小时5分钟");
    assert.equal(formatDuration(0, 45), "45秒");
    assert.equal(formatCompactNumber(412_880), "41.3万");
    assert.equal(formatCompactNumber(126_300), "12.6万");
    assert.equal(formatNumber(9_432), "9,432");
    assert.equal(joinAnd(["甲", "乙", "丙"]), "甲、乙和丙");
    assert.equal(joinAnd(["3 个会话", "2 个文件"]), "3 个会话和 2 个文件");
    assert.equal(joinAnd(["Codex", "Claude"]), "Codex 和 Claude");
    assert.equal(formatRelativeTime(Date.now() - 5 * 60_000), "5分钟前");
    assert.match(formatFullDateTime(1_786_000_000), /^2026年8月\d+日 \d{2}:\d{2}$/);
  });
});

test("a language switch re-renders translated components in place", () => {
  function Probe() {
    const { t } = useTranslation(["common", "format"]);
    return <span>{t("push")} · {t("pull")} · {t("format:justNow")}</span>;
  }
  assert.equal(renderToStaticMarkup(<Probe />), "<span>Push · Pull · just now</span>");
  inLanguage("zh-CN", () => assert.equal(renderToStaticMarkup(<Probe />), "<span>上传 · 下载 · 刚刚</span>"));
  assert.equal(renderToStaticMarkup(<Probe />), "<span>Push · Pull · just now</span>");
});

test("translated sources carry no English copy outside the catalogs", () => {
  const run = spawnSync(process.execPath, ["scripts/i18n-guard.mjs", ...TRANSLATED_SOURCES], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stdout);
});

test("no decision reads the interface's own English back", async () => {
  const { readFileSync } = await import("node:fs");
  const read = (path: string) => readFileSync(path, "utf8");
  const library = read("src/components/storage-catalog/StorageCatalogPage.tsx");
  assert.doesNotMatch(library, /startsWith\("(Keep local as|Overwrite|Install)"\)/);
  assert.doesNotMatch(library, /word === "Push"/);
  for (const path of ["src/components/project-sync/pullReviewModel.ts", "src/components/project-sync/RestorePlanView.tsx"]) {
    assert.doesNotMatch(read(path), /itemKindLabel\(item\) [!=]== "Conversation"/, path);
  }
  const shell = read("src/components/project-sync/ProjectSync.tsx");
  assert.doesNotMatch(shell, /workspaceTitle === "Projects"/);
  assert.doesNotMatch(shell, /includes\(CODEX_DELETE_CLOSE_NOTICE\)/);
  assert.doesNotMatch(read("src/components/app-link/appLink.ts"), /listingPage\(link\) === "Catalog"/);
});

test("a value set into Chinese takes a space against Latin and none against Han", () => {
  const value = (text: string) => `\uE000${text}\uE001`;
  assert.equal(settleValueSpacing(`返回 ${value("项目")}`), "返回项目");
  assert.equal(settleValueSpacing(`返回${value("Mallard")}`), "返回 Mallard");
  assert.equal(settleValueSpacing(`从 ${value("野外笔记")} 下载`), "从野外笔记下载");
  assert.equal(settleValueSpacing(`从${value("R2 mallard-otter")}下载`), "从 R2 mallard-otter 下载");
  assert.equal(settleValueSpacing(`“${value("tidewatch")}”已关联`), "“tidewatch”已关联");
  assert.equal(settleValueSpacing("没有标记的文字 stays"), "没有标记的文字 stays");
  // A value that starts with a digit keeps the space in front and loses a
  // stray one behind when both sides are Han.
  const counted = (text: string) => `\uE002${text}\uE003`;
  assert.equal(settleValueSpacing(`将 ${counted("3 个较新的会话")} 上传到 ${value("Mallard Cloud")}`), "将 3 个较新的会话上传到 Mallard Cloud");
  assert.equal(settleValueSpacing(`${counted("5")}分钟前`), "5分钟前");
  assert.equal(settleValueSpacing(`${counted("1.06M")} Token`), "1.06M Token");
  inLanguage("zh-CN", () => {
    // Digits are not marked: the catalog's own spacing stands.
    assert.equal(i18n.t("format:minutesAgo", { value: 5 }), "5分钟前");
    assert.equal(i18n.t("common:version", { version: "3" }), "Version 3");
  });
  // English is never marked.
  assert.equal(i18n.t("common:version", { version: "版本" }), "Version 版本");
});

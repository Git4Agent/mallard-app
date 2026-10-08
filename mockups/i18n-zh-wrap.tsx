import { installChineseShim, CJK_FONT_CSS } from "./i18n-zh-shim";
import { inUi, startsInUi } from "./i18n-ui-lookup";

/**
 * Any live harness in Chinese (2026-10-07). Loads `mockups/<h>.tsx` — the
 * harness mounts the shipped shell on its own stand-in backend — with the
 * specimen-only shim from i18n-zh-shim.ts installed first, then replays
 * `steps`: clicks by the English a control had before the shim, so one
 * script reaches a state in either language.
 *
 *   ?h=<harness>  ?lang=zh|en  ?font=cjk|default  ?theme=light|dark
 *   ?steps=Label|Label|wait:600|select:<value>|css:<selector>|starts:<label prefix>
 *   (plus the harness's own knobs)
 */

const query = new URLSearchParams(window.location.search);
const harness = query.get("h") ?? "session-pull-live";
const zh = query.get("lang") !== "en";
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

try {
  window.localStorage.setItem("mallard.theme", query.get("theme") === "dark" ? "dark" : "light");
} catch { /* the default theme stays */ }

async function until<T>(find: () => T | null | undefined, timeout = 5000): Promise<T> {
  const started = Date.now();
  for (;;) {
    const found = find();
    if (found) return found;
    if (Date.now() - started > timeout) throw new Error("element never appeared");
    await wait(60);
  }
}

/* A control by the English it had before the shim, its own text, or its label. */
function byEnglish(text: string): HTMLElement | undefined {
  const names = inUi(text);
  const hit = [...document.querySelectorAll<HTMLElement>("body *")].find((element) =>
    !element.closest(".harness-controls, .harness-log")
    && (element.getAttribute("data-i18n-en") === text
      || element.getAttribute("data-i18n-en-aria-label") === text
      || element.getAttribute("data-i18n-en-title") === text
      || names.includes(element.getAttribute("aria-label") ?? "")
      || ((element.children.length === 0 || element.matches("button, [role=radio], [role=tab], [role=menuitem], a"))
        && names.includes(element.textContent?.trim() ?? ""))));
  return (hit?.closest("button, [role=radio], [role=tab], [role=menuitem], [role=option], a") as HTMLElement | null) ?? hit;
}

/* A React-controlled select takes a value through the native setter. */
function choose(value: string) {
  const select = [...document.querySelectorAll("select")].find((candidate) => [...candidate.options].some((option) => option.value === value));
  if (!select) throw new Error(`no select offers ${value}`);
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

async function replay(steps: string[]) {
  for (const step of steps) {
    if (step.startsWith("wait:")) { await wait(Number(step.slice(5))); continue; }
    if (step.startsWith("select:")) { choose(step.slice(7)); await wait(300); continue; }
    if (step.startsWith("css:")) {
      (await until(() => document.querySelector<HTMLElement>(step.slice(4)))).click();
      await wait(450);
      continue;
    }
    if (step.startsWith("starts:")) {
      const prefix = step.slice(7);
      (await until(() => [...document.querySelectorAll<HTMLElement>("button, a, [role=tab], [role=radio]")].find((element) =>
        startsInUi((element.getAttribute("data-i18n-en-aria-label") ?? element.getAttribute("aria-label") ?? element.getAttribute("data-i18n-en") ?? element.textContent ?? "").trim(), prefix)
        || startsInUi(element.textContent?.trim() ?? "", prefix)))).click();
      await wait(450);
      continue;
    }
    (await until(() => byEnglish(step))).click();
    await wait(450);
  }
}

/* The harness page's own styles (its frame and shell overrides) come along. */
async function harnessStyles() {
  const html = await (await fetch(`/mockups/${harness}.html`)).text();
  const page = new DOMParser().parseFromString(html, "text/html");
  page.querySelectorAll("style, link[rel=stylesheet]").forEach((node) => document.head.append(node));
}

const modules = import.meta.glob("./*-live.tsx");

async function main() {
  if (zh) {
    document.documentElement.lang = "zh-CN";
    installChineseShim(document.body);
  }
  if (query.get("font") !== "default") {
    const style = document.createElement("style");
    style.textContent = CJK_FONT_CSS;
    document.head.append(style);
  }
  await harnessStyles();
  const load = modules[`./${harness}.tsx`];
  if (!load) throw new Error(`no harness ${harness}`);
  await load();
  await wait(900);
  const steps = (query.get("steps") ?? "").split("|").filter(Boolean);
  await replay(steps);
  await wait(300);
  document.documentElement.dataset.ready = "1";
}

void main().catch((error) => { document.documentElement.dataset.ready = `failed: ${String(error)}`; });

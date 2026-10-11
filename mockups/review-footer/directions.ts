// @ts-nocheck
/*
 * The review footer specimen's patches, applied to the real app once it has
 * settled (live.tsx). FIX_CSS is the proposed bottom-gutter fix; each
 * direction is a stylesheet plus the DOM it adds to the live review.
 *
 * Pull has no waiting-file count today. The rule shown here: every file row
 * not ticked (folders excluded), worded like the row's own chip — adds read
 * "N new", replaces "N changed".
 */
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import Icon from "../../src/components/Icons";

/* One bottom gutter for every review body: the last tile's 4px shadow plus
   8px of air, inside the scroller so a scrolled list keeps it too. The
   push-only 2px, the <=620px zero and the review Files page's own 22px go
   (the project page's Files view keeps its padding). */
export const FIX_CSS = `
.sync-sync-review-scroll.sync-sync-review-scroll { padding-bottom: 12px !important; }
.sync-sync-review-content.sync-sync-review-content { padding-bottom: 0 !important; }
.sync-sync-review-scroll > .sync-project-files-page.sync-project-files-page { padding-bottom: 0 !important; }
`;

type Side = "push" | "pull";

const WORDS = {
  "zh-CN": {
    newCount: (n: number) => `新增 ${n}`,
    waiting: (side: Side, n: number) => `${n} 个新项目文件不在此次${side === "push" ? "上传" : "下载"}中`,
    review: "查看文件",
    anyway: (side: Side) => (side === "push" ? "仍然上传" : "仍然下载"),
    info: "提示 ·",
    scoped: (side: Side, n: number) => `${side === "push" ? "上传" : "下载"} ${n} 个会话`,
  },
  en: {
    newCount: (n: number) => `${n} new`,
    waiting: (side: Side, n: number) => `${n} new project file is not in this ${side}`,
    review: "Review files",
    anyway: (side: Side) => (side === "push" ? "Push anyway" : "Pull anyway"),
    info: "Info ·",
    scoped: (side: Side, n: number) => `${side === "push" ? "Push" : "Pull"} ${n} session${n === 1 ? "" : "s"}`,
  },
};

/* The app's own glyphs. react-dom/client only: the server renderer is not
   in the dev server's prebundled deps, and a new one reloads every page. */
function icon(name: string, size = 14, className?: string): string {
  const host = document.createElement("div");
  const root = createRoot(host);
  flushSync(() => root.render(createElement(Icon, { name, size, className })));
  const markup = host.innerHTML;
  root.unmount();
  return markup;
}

function html(markup: string): Element {
  const template = document.createElement("template");
  template.innerHTML = markup.trim();
  return template.content.firstElementChild as Element;
}

function addStyle(css: string) {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
}

const footer = () => document.querySelector(".sync-inline-action-review .sync-sync-review-footer") as HTMLElement;
/* Tagged on first sight: a direction may restyle it out of .btn-ghost. */
function stepButton(): HTMLElement {
  const tagged = footer().querySelector(":scope > [data-dir-step]");
  if (tagged) return tagged as HTMLElement;
  const button = footer().querySelector(":scope > .btn.btn-ghost") as HTMLElement;
  button.dataset.dirStep = "";
  return button;
}
const applyButton = () => footer().querySelector(":scope > .sync-pull-apply-button") as HTMLElement;
const filesTab = () => document.querySelector(".sync-sync-review-tabs > [data-sync-review-tab='project_files']") as HTMLElement;
const scroller = () => document.querySelector(".sync-inline-action-review .sync-sync-review-scroll") as HTMLElement;

/* The count the step button carries. Push already prints it; Pull gets one
   under the rule above (the fixture's one file is an add). */
function chip(): HTMLElement | null {
  return stepButton()?.querySelector(".sync-path-count") ?? null;
}

function ensureChip(words, waiting: number) {
  if (chip()) return chip();
  const button = stepButton();
  const chevron = button.querySelector("svg:last-of-type");
  const count = html(`<span class="sync-path-count">${words.newCount(waiting)}</span>`);
  button.insertBefore(count, chevron);
  return count as HTMLElement;
}

const dropChip = () => chip()?.remove();

/* The direction's own ink: Push is local-outstanding, Pull cloud — the gauge
   fill's 34% wash, laid over the card surface. */
const CHIP_CSS = `
.sync-sync-review-footer > .btn .sync-path-count.dir-tint,
.sync-sync-review-tabs .dir-tint {
  background: color-mix(in srgb, var(--dir-ink) 34%, var(--surface-card));
  color: var(--text-0);
}
.dir-push { --dir-ink: var(--thread-status-local); }
.dir-pull { --dir-ink: var(--thread-status-cloud); }
`;

/* A · Box: the step button takes the box grammar (edge + cast shadow), its
   count in the direction's tint. */
function boxStep(words, side: Side, waiting: number) {
  addStyle(CHIP_CSS);
  const button = stepButton();
  button.classList.remove("btn-ghost");
  button.classList.add(`dir-${side}`);
  ensureChip(words, waiting).classList.add("dir-tint");
}

/* F · the apply verb names what travels. */
function scopedVerb(words, side: Side, sessions: number) {
  const button = applyButton();
  const label = [...button.childNodes].find((node) => node.nodeType === Node.TEXT_NODE && node.textContent.trim());
  if (label) label.textContent = words.scoped(side, sessions);
}

export function applyDirection(key: string, side: Side, language: string) {
  const words = WORDS[language] ?? WORDS.en;
  const waiting = 1;
  const sessions = 1;
  document.documentElement.classList.add(`dir-${key}`);

  switch (key) {
    /* Before + fix: the footer as shipped. Pull shows no count. */
    case "fix":
      return;

    case "A":
      return boxStep(words, side, waiting);

    /* B · Tab badge: the count moves to the Files tab, beside its selected
       count; the footer's step button is plain. */
    case "B": {
      addStyle(CHIP_CSS + `
.sync-sync-review-tabs .dir-badge {
  min-width: 18px; height: 18px; margin-left: 2px; padding: 0 6px;
  display: inline-grid; place-items: center;
  border: 1.5px solid var(--dir-ink); border-radius: 999px;
  font-size: var(--type-2xs); font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums; line-height: 1; white-space: nowrap;
}`);
      dropChip();
      const tab = filesTab();
      tab.classList.add(`dir-${side}`);
      tab.querySelector("small").after(html(`<span class="dir-badge dir-tint">${words.newCount(waiting)}</span>`));
      return;
    }

    /* C · Inline notice: one info box in the review's banner zone, above the
       list, with the way to the files. The step button is plain. */
    case "C": {
      addStyle(`
.dir-C .sync-sync-review-scroll > .dir-banners { margin: 8px 0 0; }
.dir-C .sync-sync-review-scroll > .dir-banners .notice { margin: 0; }
.dir-C .sync-sync-review-scroll > .dir-banners + .sync-history-selection-review { padding-top: 4px; }
.dir-C .dir-banners .notice > .btn { min-height: 28px; padding: 0 10px; font-size: var(--type-xs); }`);
      dropChip();
      const banner = html(`
<div class="sync-review-banners dir-banners">
  <div class="notice info">
    ${icon("info", 13, "notice-mark")}
    <div class="notice-body"><p class="notice-line"><strong class="notice-type">${words.info}</strong> ${words.waiting(side, waiting)}</p></div>
    <button type="button" class="btn">${words.review}${icon("chevron-right", 13)}</button>
  </div>
</div>`);
      const list = scroller();
      list.insertBefore(banner, list.firstElementChild);
      return;
    }

    /* D · Footer note: the footer's own sentence row above the cluster — the
       row the over-limit refusal already uses — with
       the way to the files at its end. The step button is plain. */
    case "D": {
      addStyle(`
.dir-D .sync-review-footer-note.dir-note { display: flex; justify-content: flex-end; align-items: center; gap: 6px; color: var(--text-1); }
.dir-D .sync-review-footer-note.dir-note > svg { flex: 0 0 auto; }
.dir-D .sync-review-footer-note.dir-note > svg { color: var(--dir-ink); }
.dir-D .dir-note .btn-link { padding: 0; font-size: inherit; font-weight: var(--weight-semibold); color: var(--text-0); text-decoration: underline; text-underline-offset: 2px; }
.dir-push { --dir-ink: var(--thread-status-local); }
.dir-pull { --dir-ink: var(--thread-status-cloud); }`);
      dropChip();
      const note = html(`
<p class="sync-review-footer-note dir-note dir-${side}">
  ${icon("info", 13)}
  <span>${words.waiting(side, waiting)}</span>
  <span aria-hidden="true">·</span>
  <button type="button" class="btn-link">${words.review}</button>
</p>`);
      footer().prepend(note);
      return;
    }

    /* E · Attention pill: the step button wears the blocker pill's shape in
       the info ink — a status you can press. */
    case "E": {
      addStyle(`
.sync-sync-review-footer > .dir-pill {
  height: 34px; flex: 0 0 auto; display: inline-flex; align-items: center; gap: 6px;
  padding: 0 10px 0 13px; border: var(--box-border); border-color: var(--info-text); border-radius: 999px;
  background: linear-gradient(var(--info-soft), var(--info-soft)), var(--surface-card);
  color: var(--info-text); font-size: var(--type-xs); font-weight: var(--weight-semibold); white-space: nowrap; box-shadow: none;
}
.sync-sync-review-footer > .dir-pill .sync-path-count { background: var(--info-text); color: var(--surface-card); }`);
      const button = stepButton();
      ensureChip(words, waiting);
      button.className = "dir-pill";
      return;
    }

    /* F · A, and the apply verb names what travels (GitHub Desktop's
       "Commit N files"): what stays behind is the step button's count. */
    case "F":
      boxStep(words, side, waiting);
      scopedVerb(words, side, sessions);
      return;

    /* G · Confirm on press: the review as it stands after Push is pressed
       with new files waiting — an anchored popover over the button. */
    case "G": {
      addStyle(`
.dir-G .sync-sync-review-footer { position: relative; }
.dir-G .dir-confirm {
  position: absolute; right: 20px; bottom: calc(100% + 10px); z-index: 5; width: 290px;
  padding: 12px 14px; display: grid; gap: 10px;
  border: var(--box-border); border-radius: 14px; background: var(--surface-card); box-shadow: var(--shadow-card);
  color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-semibold);
}
.dir-G .dir-confirm > div { display: flex; justify-content: flex-end; gap: 8px; }
.dir-G .dir-confirm .btn { min-height: 30px; padding: 0 12px; font-size: var(--type-xs); }`);
      if (side === "pull") ensureChip(words, waiting);
      const confirm = html(`
<div class="dir-confirm" role="dialog">
  <span>${words.waiting(side, waiting)}</span>
  <div>
    <button type="button" class="btn">${words.review}</button>
    <button type="button" class="btn btn-primary">${words.anyway(side)}</button>
  </div>
</div>`);
      footer().append(confirm);
      return;
    }
  }
}

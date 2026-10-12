// @ts-nocheck
/*
 * Cloud picker refresh specimen: the connect dialog's picker built from the
 * real parts (RepositoryLookupField, ListingRows, the strip's markup) with
 * canned rows, one dialog per direction and state.
 *
 *   /mockups/cloud-refresh/index.html?theme=light|dark&lang=en|zh&only=A,B
 *
 * The refresh buttons work: a click spins ~1.4 s, the rows stay, then one
 * row arrives at the top and the tab count grows.
 */
import { useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import "../../src/App.css";
import { applyThemeChoice } from "../../src/preferences";
import { applyUiLanguage, t } from "../../src/i18n";
import Icon from "../../src/components/Icons";
import RepositoryLookupField from "../../src/components/managed-cloud/RepositoryLookupField";
import { ListingRows } from "../../src/components/managed-cloud/CloudScopes";

const params = new URLSearchParams(window.location.search);
const theme = params.get("theme") === "dark" ? "dark" : "light";
const lang = params.get("lang") === "en" ? "en" : "zh";
const only = params.get("only")?.split(",") ?? null;
try {
  window.localStorage.setItem("mallard.theme", theme);
  window.localStorage.setItem("mallard.language", lang === "zh" ? "zh-CN" : "en");
} catch { /* the page still renders */ }
applyThemeChoice(theme);
applyUiLanguage();
document.documentElement.dataset.theme = theme;

/* Words the catalogs do not have yet (the proposal's own). */
const W = lang === "zh"
  ? { refresh: "刷新", refreshing: "正在刷新…", updated: (ago) => `${ago}更新`, justNow: "刚刚", ago: "2 分钟前", newCount: (n) => `${n} 个新项目`, show: "显示", connect: "连接", project: "mallard-app" }
  : { refresh: "Refresh", refreshing: "Refreshing…", updated: (ago) => `Updated ${ago}`, justNow: "just now", ago: "2 min ago", newCount: (n) => `${n} new`, show: "Show", connect: "Connect", project: "mallard-app" };

/* The proposed App.css lines, one block per direction. */
const CSS = {
  A: `
[data-dir="A"] .repo-scope-tabs > .repo-scope-refresh { margin-left: auto; margin-bottom: 7px; width: 28px; min-height: 28px; padding: 0; }
[data-dir="A"] .repo-scope-tabs > .repo-scope-sort + .repo-scope-refresh { margin-left: 8px; }`,
  B: `
[data-dir="B"] .repo-scope-tabs > .repo-scope-refresh { margin-left: auto; margin-bottom: 7px; min-height: 28px; padding: 0 8px; font-size: var(--type-2xs); }
[data-dir="B"] .repo-scope-tabs > .repo-scope-sort + .repo-scope-refresh { margin-left: 8px; }`,
  C: `
[data-dir="C"] .repo-connect .sync-folder-field > div > .btn.repo-refresh { min-width: 0; width: 34px; padding: 0; }`,
  D: `
[data-dir="D"] .repo-list-stamp { display: flex; align-items: center; gap: 6px; margin-top: 8px; color: var(--text-3); font-size: var(--type-2xs); }
[data-dir="D"] .repo-list-stamp .status-loader { width: 12px; height: 12px; }`,
  E: `
[data-dir="E"] .sync-modal-header > .btn.repo-refresh { width: 32px; padding: 0; }`,
  F: `
[data-dir="F"] .repo-new-pill { display: flex; justify-content: center; margin: -2px 0 8px; }
[data-dir="F"] .repo-new-pill > .btn { min-height: 26px; padding: 0 10px; font-size: var(--type-2xs); border-radius: var(--r-pill); }`,
};

const now = Math.floor(Date.now() / 1000);
const H = 3600;
const row = (owner, name, ago, extra = {}) => ({
  repository_id: `${owner}/${name}`, owner_login: owner, name, address: `${owner}/${name}`, display_name: name,
  website_url: `https://mallard.example/${owner}/${name}`, repository_class: "project", catalog: null,
  visibility: "public", role: null, github_url: null, description: null, star_count: 0, viewer_starred: false,
  revision: 1, updated_at: now - ago, ...extra,
});
const PUBLIC = [
  row("FuyaoLi2017", "fuyao-non-git-test", 3 * H),
  row("Rika321", "win_mallard", 3 * H),
  row("Rika321", "win_pelican", 3 * H),
  row("rikaqu0223-arch", "mallard-with-pelican", 19 * H, { github_url: "https://github.com/rikaqu0223-arch/mallard-with-pelican" }),
  row("rikaqu0223-arch", "测试文件", 4 * 24 * H),
  row("rikaqu0223-arch", "mallardNonGit", 4 * 24 * H),
];
const YOURS = [
  row("rikaqu0223-arch", "mallard-with-pelican", 19 * H, { role: "owner", github_url: "https://github.com/rikaqu0223-arch/mallard-with-pelican", visibility: "public" }),
  row("rikaqu0223-arch", "测试文件", 4 * 24 * H, { role: "owner" }),
  row("rikaqu0223-arch", "mallardNonGit", 4 * 24 * H, { role: "owner" }),
];
const ARRIVAL = row("Rika321", "pelican-docs", 40, { description: lang === "zh" ? "刚刚推送的项目" : "Pushed a moment ago" });

const scope = (id) => t(`projectSetup:scope.${id}`);

/** The strip, as CloudScopeTabs draws it, plus the direction's control. */
function Strip({ tab, counts, control }: { tab: string; counts: Record<string, number | null>; control?: ReactNode }) {
  const tabs = [
    { id: "yours", icon: "user" }, { id: "starred", icon: "star" }, { id: "public", icon: "cloud" },
  ];
  return (
    <div className="sync-project-tabs sync-sync-review-tabs repo-scope-tabs" role="tablist" aria-label={t("cloud:scopes.tabs")}>
      {tabs.map((item) => {
        const current = item.id === tab;
        const count = counts[item.id];
        return (
          <button key={item.id} type="button" role="tab" aria-selected={current} tabIndex={current ? 0 : -1}
            className={current ? "active" : undefined} data-scope={item.id}>
            <Icon name={item.icon} size={14} />
            {scope(item.id)}
            {count !== null && count > 0 && <small>{count}</small>}
          </button>
        );
      })}
      {tab === "public" && (
        <div className="sync-segmented repo-scope-sort" role="radiogroup" aria-label={t("cloud:scopes.sort")}>
          <button type="button" role="radio" aria-checked="true" className="active">{t("cloud:scopes.newest")}</button>
          <button type="button" role="radio" aria-checked="false">{t("cloud:scopes.mostStars")}</button>
        </div>
      )}
      {control}
    </div>
  );
}

/** The refresh button in its two shapes. */
function RefreshButton({ labeled, busy, onClick, className }: { labeled: boolean; busy: boolean; onClick: () => void; className: string }) {
  const name = busy ? W.refreshing : W.refresh;
  return (
    <button type="button" className={className} onClick={onClick} disabled={busy} aria-busy={busy || undefined}
      aria-label={labeled ? undefined : name} title={labeled ? undefined : name}>
      <Icon name="refresh" size={14} className={busy ? "icon-spin" : undefined} />
      {labeled && (busy ? W.refreshing : W.refresh)}
    </button>
  );
}

function Picker({ dir, tab, startBusy = false, pill = false }: { dir: string; tab: string; startBusy?: boolean; pill?: boolean }) {
  const [busy, setBusy] = useState(startBusy);
  const [arrived, setArrived] = useState(false);
  const [stamp, setStamp] = useState(W.ago);
  const refresh = () => {
    if (busy) return;
    setBusy(true);
    window.setTimeout(() => { setBusy(false); setArrived(true); setStamp(W.justNow); }, 1400);
  };
  const rows = (tab === "public" ? PUBLIC : YOURS);
  const shown = arrived ? [ARRIVAL, ...rows] : rows;
  const counts = { yours: tab === "yours" ? shown.length : 3, starred: 3, public: null };

  const inStrip = dir === "A" || dir === "B";
  const stripControl = inStrip
    ? <RefreshButton labeled={dir === "B"} busy={busy} onClick={refresh} className="btn btn-ghost repo-scope-refresh" />
    : undefined;

  return (
    <dialog open data-dir={dir} className="sync-modal sync-bundle-connection-dialog" aria-labelledby={`title-${dir}-${tab}`}>
      <header className="sync-modal-header">
        <div><h1 id={`title-${dir}-${tab}`}>{t("reviews:connect.cloudLinkTitle", { project: W.project })}</h1></div>
        {dir === "E" && <RefreshButton labeled={false} busy={busy} onClick={refresh} className="btn btn-ghost repo-refresh" />}
        <button type="button" className="btn btn-ghost" aria-label={t("reviews:connect.close")}><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body repo-connect">
        <div className="sync-bundle-context">
          <Icon name="storage-mallard" size={17} />
          <span><strong>Mallard Cloud · rikaqu0223-arch</strong><small>mallard.example</small></span>
        </div>
        {dir === "C" ? (
          <label className="sync-folder-field compact">
            <span>{t("cloud:lookup.label")}</span>
            <div>
              <input value="" readOnly spellCheck={false} placeholder={t("cloud:lookup.placeholder")} />
              <button type="button" className="btn repo-verb" disabled><Icon name="search" size={12} />{t("cloud:lookup.lookUp")}</button>
              <RefreshButton labeled={false} busy={busy} onClick={refresh} className="btn repo-refresh" />
            </div>
          </label>
        ) : (
          <RepositoryLookupField value="" onChange={() => undefined} onSubmit={() => undefined} disabled={false} looking={false} joining={false} />
        )}
        <Strip tab={tab} counts={counts} control={stripControl} />
        {dir === "F" && pill && !arrived && (
          <div className="repo-new-pill">
            <button type="button" className="btn btn-ghost" onClick={refresh} disabled={busy}>
              <Icon name="chevron-up" size={12} /> {W.newCount(1)} · {W.show}
            </button>
          </div>
        )}
        <ListingRows rows={shown} skin="choice" selectedId={null} resolvingId={null} disabled={false}
          ariaLabel={t("projectSetup:picker.public")} markFor={() => "folder"} onPick={() => undefined} />
        {dir === "D" && (
          <div className="repo-list-stamp" role="status">
            {busy ? <><span className="status-loader" /> {W.refreshing}</> : <>
              <Icon name="clock" size={12} /> {W.updated(stamp)} · <button type="button" className="btn-link" onClick={refresh}>{W.refresh}</button>
            </>}
          </div>
        )}
        {dir !== "D" && <span className="sync-visually-hidden" role="status">{busy ? W.refreshing : arrived ? W.updated(W.justNow) : ""}</span>}
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn">{t("common:cancel")}</button>
          <button type="button" className="btn btn-primary" disabled>{W.connect}</button>
        </div>
      </footer>
    </dialog>
  );
}

const link = (name, url) => <a href={url} target="_blank" rel="noreferrer">{name}</a>;

const ROWS = [
  { key: "before", title: "Before", frames: [{ tab: "public", caption: "Public, as shipped" }] },
  { key: "A", pick: true, title: "A · Strip end, icon",
    seen: <>{link("GitHub Desktop clone dialog", "https://github.com/desktop/desktop/blob/development/app/src/ui/clone-repository/cloneable-repository-filter-list.tsx")} (icon after the filter, tooltip = name, spins, list stays) · {link("Cloudscape loading & refreshing", "https://cloudscape.design/patterns/general/loading-and-refreshing/")} (header actions, keep data on screen) · {link("Primer icon button", "https://primer.style/product/components/icon-button/guidelines/")} / {link("M3 icon buttons", "https://m3.material.io/components/icon-buttons/accessibility")} (tooltip names the action)</>,
    against: "Icon-only; sits beside the sort pills.",
    frames: [{ tab: "yours", caption: "Yours" }, { tab: "public", caption: "Public, after the sort" }, { tab: "public", busy: true, caption: "Refreshing · rows stay" }] },
  { key: "B", title: "B · Strip end, labeled",
    seen: <>Mallard Files page “Rescan” · {link("Fluent command bar", "https://rdrr.io/github/Appsilon/shiny.fluent/man/CommandBar.html")} (icon + label; icon-only only for the most recognizable) · {link("Carbon button", "https://carbondesignsystem.com/components/button/usage/")} (icon-only sparingly)</>,
    against: "Three controls on one strip.",
    frames: [{ tab: "yours", caption: "Yours" }, { tab: "public", caption: "Public" }, { tab: "public", busy: true, caption: "Refreshing" }] },
  { key: "C", title: "C · Field row, after the verb",
    seen: <>{link("GitHub Desktop", "https://github.com/desktop/desktop/blob/development/app/src/ui/clone-repository/cloneable-repository-filter-list.tsx")} (post-filter slot) · {link("Cloudscape", "https://cloudscape.design/patterns/general/loading-and-refreshing/")} (next to the control it refreshes)</>,
    against: "Reads as part of search; verb is dim until a link.",
    frames: [{ tab: "public", caption: "Public" }, { tab: "public", busy: true, caption: "Refreshing" }] },
  { key: "D", title: "D · Stamp under the list",
    seen: <>{link("Cloudscape", "https://cloudscape.design/patterns/general/loading-and-refreshing/")} (last-updated time, announced) · {link("WCAG 4.1.3", "https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html")} (status text, no focus move) · macOS Mail “Updated …” line</>,
    against: "Below the fold on a long list; needs a clock.",
    frames: [{ tab: "public", caption: "Public" }, { tab: "public", busy: true, caption: "Refreshing" }] },
  { key: "E", title: "E · Dialog header, by the close",
    seen: <>{link("Cloudscape header actions", "https://cloudscape.design/patterns/general/loading-and-refreshing/")} · macOS toolbars</>,
    against: "Far from the list; reads as window chrome.",
    frames: [{ tab: "public", caption: "Public" }, { tab: "public", busy: true, caption: "Refreshing" }] },
  { key: "F", title: "F · Poll, then “N new” pill",
    seen: <>X / Instagram timelines (“Show new posts”) · {link("Cloudscape auto-refresh", "https://cloudscape.design/patterns/general/loading-and-refreshing/")} (lists others add to)</>,
    against: "Polling a short-lived dialog; no manual control.",
    frames: [{ tab: "public", pill: true, caption: "A row arrived while open" }] },
];

function Page() {
  const rows = only ? ROWS.filter((r) => only.includes(r.key)) : ROWS;
  const href = (next) => { const p = new URLSearchParams(params); Object.entries(next).forEach(([k, v]) => p.set(k, v)); return `?${p}`; };
  return (
    <>
      <style>{Object.values(CSS).join("\n")}</style>
      <div className="spec-head">
        <h1>Cloud picker · Refresh</h1>
        <div className="toggles">
          <a href={href({ theme: "light" })} aria-current={theme === "light"}>light</a>
          <a href={href({ theme: "dark" })} aria-current={theme === "dark"}>dark</a>
        </div>
        <div className="toggles">
          <a href={href({ lang: "zh" })} aria-current={lang === "zh"}>中文</a>
          <a href={href({ lang: "en" })} aria-current={lang === "en"}>en</a>
        </div>
      </div>
      <p className="rule">Real picker parts, canned rows, dialog at its 680px. Every button works: rows stay while it spins, then one row arrives and the count grows.</p>
      <p className="rule">All directions: one Refresh reloads Yours + Starred now and Public on its next open; selection and filter words stay; a failure shows the existing notice. No web/cloud change.</p>
      {rows.map((r) => (
        <section key={r.key} className="row">
          <h3>{r.title}{r.pick && <span className="pick">recommended</span>}</h3>
          {r.seen && <p>seen in: {r.seen}</p>}
          {r.against && <p><span className="against">against:</span> {r.against}</p>}
          <div className="frames">
            {r.frames.map((f, i) => (
              <figure key={i} className="frame">
                <figcaption><b>{f.caption}</b></figcaption>
                <Picker dir={r.key} tab={f.tab} startBusy={!!f.busy} pill={!!f.pill} />
              </figure>
            ))}
          </div>
        </section>
      ))}
    </>
  );
}

createRoot(document.getElementById("root")).render(<Page />);

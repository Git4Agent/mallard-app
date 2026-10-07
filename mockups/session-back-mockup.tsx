import { StrictMode, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import LoadingScaffold from "../src/components/LoadingScaffold";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import SessionDetailBack, { type SessionBackTarget } from "../src/components/project-sync/session-detail/SessionDetailBack";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../src/components/sync-review/SyncPathBar";
import "../src/App.css";

/**
 * Mockup: a return button on the session page.
 *
 * Today the only way back to the project's activity is the project crumb's
 * label, which looks the same as it does on the activity page, where it opens
 * a menu. Nothing on the session page reads as "back".
 *
 *   Before  the project crumb's label is the only way back
 *   A       a back button leads the path bar, ahead of the trail
 *   B       a back button leads the page's fact strip (picked 2026-10-02 and
 *           shipped: this frame mounts the shipped `SessionDetailBack`)
 *   C       no new control: the project crumb is drawn as the back button
 *
 * `npm run dev`, then /mockups/session-back-mockup.html
 * (`?page=session|agent|opening|failed&project=git|plain&width=wide|default|floor&sidebar=shown|hidden&pay=off|on&theme=light|dark&only=before|a|b|c&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "before" | "a" | "b" | "c";
type Page = "session" | "agent" | "opening" | "failed";
type Width = "wide" | "default" | "floor";
interface Knobs {
  page: Page;
  project: "git" | "plain";
  width: Width;
  sidebar: "shown" | "hidden";
  /** A only: what the bar gives up near its floor to pay for the button. */
  pay: "off" | "on";
  theme: "light" | "dark";
  only: Dir | "all";
  /** Draws the frames without the specimen's own title and controls. */
  shot: "off" | "on";
}

const DIRS: readonly Dir[] = ["before", "a", "b", "c"];
const PAGES: readonly Page[] = ["session", "agent", "opening", "failed"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    page: pick("page", PAGES, "session"),
    project: pick("project", ["git", "plain"] as const, "git"),
    width: pick("width", ["wide", "default", "floor"] as const, "wide"),
    sidebar: pick("sidebar", ["shown", "hidden"] as const, "shown"),
    pay: pick("pay", ["off", "on"] as const, "on"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    shot: pick("shot", ["off", "on"] as const, "off"),
  };
}

const PAGE_LABEL: Record<Page, string> = {
  session: "Session",
  agent: "Agent’s page",
  opening: "Opening",
  failed: "Failed to open",
};

const DIR_LABEL: Record<Dir, string> = {
  before: "Before",
  a: "A · leads the bar",
  b: "B · leads the page (shipped)",
  c: "C · the crumb, drawn as a button",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>The project crumb’s label is the way back. It looks the same as on the activity page, where it opens the project menu.</>,
  a: <>One round button ahead of the trail, in the refresh button’s box. It stays while the page scrolls and shows in every state of the page. It takes 36 px from the trail.</>,
  b: <>A labelled button ahead of the fact chips. It names where it goes and costs the bar nothing. It scrolls away with the page, and the opening and failed states need their own copy.</>,
  c: <>No new control. In a session the project crumb’s label takes a chip’s edge and a back chevron in place of its folder. Free in width; the folder mark is gone while a session is open.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const STORAGE = "Mallard Cloud team-space";
const PROJECT = "mallard-app";
const TITLE = "Run E2E tests without xlarge";
const AGENT_TITLE = "Map the e2e suites and their sizes";
const PROVIDER = "codex";
const PROVIDER_NAME = "Codex";

const noop = () => undefined;

const CLOUD: ManagedCloudStatus = {
  repository_id: "repo_team_mallard-app",
  state: "ready",
  link: {
    visibility: "private",
    can_pull: true,
    can_push: true,
    repository_id: "repo_team_mallard-app",
    website_url: "https://cloud.example/team/mallard-app",
    owner_login: "team",
    name: "mallard-app",
    release: { kind: "private" },
    repository_revision: 4,
    can_request_release: true,
    can_publish_version: true,
    can_discard_draft: true,
    role: "owner",
  },
  publication: { draft: "pushed", latest_version: null, next_version: 1, draft_ahead: null, public_version: null, review_version: null },
  message: null,
};

const STORAGE_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: noop }] },
];
const PROJECT_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop }] },
];
const APP_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "settings", label: "Settings…", icon: "settings", onSelect: noop }] },
];

/** Where the button lands, in the trail's own words. */
function backTarget(knobs: Knobs): SessionBackTarget {
  if (knobs.page === "agent") return { kind: "session", label: TITLE, onActivate: noop };
  return { kind: "section", label: knobs.project === "git" ? "Git history" : "Sessions", onActivate: noop };
}

function leafMark(active: boolean) {
  return (
    <span className={`sync-path-leaf-mark${active ? " active" : ""}`}>
      <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} />
      {active && <span className="sync-visually-hidden">Active</span>}
    </span>
  );
}

function crumbsOf(dir: Dir, knobs: Knobs): PathBarCrumb[] {
  const folder = knobs.project === "git" ? "git-folder" : "folder";
  const crumbs: PathBarCrumb[] = [
    {
      id: "storage",
      label: STORAGE,
      icon: "storage-mallard",
      tone: "root",
      menu: STORAGE_MENU,
      menuHeader: { label: STORAGE, detail: "team/mallard-app", icon: "storage-mallard" },
      menuLabel: "Storage",
    },
    {
      id: "project",
      label: PROJECT,
      /* C: the mark says what a click on the label does. */
      icon: dir === "c" ? "chevron-left" : folder,
      title: dir === "c" ? `Back to ${knobs.project === "git" ? "Git history" : "Sessions"}` : undefined,
      onActivate: noop,
      menu: PROJECT_MENU,
      menuHeader: { label: PROJECT, detail: "~/project/mallard-app", detailIsPath: true, icon: folder },
      menuLabel: "Project",
    },
  ];
  if (knobs.page === "agent") {
    crumbs.push({ id: "session-parent", label: TITLE, title: `Back to ${TITLE}`, leading: leafMark(false), onActivate: noop });
    crumbs.push({ id: "session", label: AGENT_TITLE, leading: leafMark(false) });
  } else if (knobs.page === "session") {
    crumbs.push({ id: "session", label: TITLE, leading: leafMark(true) });
  } else {
    crumbs.push({ id: "session", label: knobs.page === "opening" ? "Opening session…" : "Session unavailable" });
  }
  return crumbs;
}

/* ——— the bar's action cluster, as the workspace mounts it ——————————————— */

function PathActions({ page }: { page: Page }) {
  return (
    <>
      <span className="sync-path-attention danger" tabIndex={0} data-tooltip="1 session can’t be compared" aria-label="1 thread needs attention">
        <Icon name="octagon-x" size={13} />
        <b>1</b>
      </span>
      <ManagedCloudChip
        status={CLOUD}
        storageName={STORAGE}
        busy={false}
        refreshing={false}
        onOpenUrl={noop}
        onPublishVersion={noop}
        onDiscardDraft={noop}
        onRetry={noop}
        onStorageSettings={noop}
      />
      {(page === "session" || page === "agent") && (
        <button type="button" className="sync-path-icon-action" title="Refresh session" aria-label="Refresh session">
          <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
        </button>
      )}
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync">
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
        </button>
        <button type="button" className="sync-path-sync" disabled={page !== "session"}>
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {page === "session" && <span className="sync-path-count" aria-hidden="true">1</span>}
        </button>
      </div>
    </>
  );
}

/* ——— A: the button ahead of the trail ————————————————————————————————————
   The shipped bar has no slot before its list, so the mock seats the button
   there through a portal; the plan adds the slot to `SyncPathBar`. */

function PathBar({ dir, knobs }: { dir: Dir; knobs: Knobs }) {
  const host = useRef<HTMLDivElement>(null);
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (dir !== "a") return;
    const header = host.current?.querySelector<HTMLElement>("header.sync-pathbar");
    if (!header) return;
    const node = document.createElement("span");
    node.className = "mk-back-slot";
    header.prepend(node);
    setSlot(node);
    return () => {
      node.remove();
      setSlot(null);
    };
  }, [dir]);
  const target = backTarget(knobs).label;
  return (
    <div ref={host} className="mk-bar-host">
      <SyncPathBar
        crumbs={crumbsOf(dir, knobs)}
        actions={<PathActions page={knobs.page} />}
        overflow={APP_MENU}
        overflowLabel="App actions"
        ariaLabel={`${PROJECT} path`}
      />
      {slot && createPortal(
        <button type="button" className="sync-path-icon-action mk-bar-back" title={`Back to ${target}`} aria-label={`Back to ${target}`}>
          <Icon name="chevron-left" size={16} />
        </button>,
        slot,
      )}
    </div>
  );
}

/* ——— the session page: its fact strip, the SHOW bar, the first turn ——————— */

function SessionPage({ dir, knobs }: { dir: Dir; knobs: Knobs }) {
  if (knobs.page === "opening") {
    return (
      <section className="session-detail-page session-detail-entry-state" aria-label="Loading session detail">
        {dir === "b" && <SessionDetailBack target={backTarget(knobs)} />}
        <LoadingScaffold shape="detail" label="Loading session detail" />
      </section>
    );
  }
  if (knobs.page === "failed") {
    return (
      <section className="session-detail-page session-detail-entry-state" aria-label="Session detail unavailable">
        {dir === "b" && <SessionDetailBack target={backTarget(knobs)} />}
        <div className="session-detail-state error">
          <Icon name="alert-triangle" size={24} />
          <strong>Session detail unavailable</strong>
          <span>The session file moved or was deleted since the list was read.</span>
          <button type="button" className="btn btn-ghost">Try again</button>
        </div>
      </section>
    );
  }
  const agent = knobs.page === "agent";
  const toggle = (label: string, count: string, on: boolean) => (
    <button type="button" className={`session-detail-filter-toggle${on ? " is-on" : ""}`} aria-pressed={on}>
      <span className="session-detail-filter-box" aria-hidden="true" />
      <span className="session-detail-filter-name">{label}</span>
      <span className="session-detail-filter-count quality-exact">{count}</span>
    </button>
  );
  const chips = (
    <div className="session-detail-chips" aria-label="Session facts">
      <span className="session-detail-chip"><Icon name="git-branch" size={12} /><code>main</code></span>
      <span className="session-detail-chip"><Icon name="git-commit" size={12} /><code>8577306</code></span>
      <span className="session-detail-chip"><time>Oct 2, 2026 · 21:43</time></span>
      <span className="session-detail-chip">
        <Icon name={agent ? "clock" : "activity"} size={12} /><b>{agent ? "4m" : "running"}</b>
      </span>
      <span className="session-detail-chip"><code>{agent ? "312K" : "2.67M"}</code> tokens</span>
      {agent && <span className="session-detail-chip"><Icon name="git-fork" size={12} /><code>explorer</code></span>}
    </div>
  );
  return (
    <section className="session-detail-page" aria-label={agent ? AGENT_TITLE : TITLE}>
      <header className="session-detail-header">
        <div className="session-detail-strip">
          {dir === "b"
            ? <div className="session-detail-lead"><SessionDetailBack target={backTarget(knobs)} />{chips}</div>
            : chips}
          <div className="session-detail-header-actions" aria-label="Session actions">
            <button type="button" className="btn btn-ghost session-detail-launch" aria-label={`Open in ${PROVIDER_NAME}`}>
              <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} size={12} />
            </button>
            <button type="button" className="btn btn-ghost session-detail-launch" aria-label="Open in Terminal">
              <Icon name="terminal" size={12} />
            </button>
          </div>
        </div>
      </header>
      <div className="session-detail-layout">
        <div className="session-detail-toolbar">
          <span className="session-detail-toolbar-label">Show</span>
          <div className="session-detail-toolbar-toggles" role="group" aria-label="Show">
            {toggle("Prompts", "1", true)}
            {toggle("Steps", "13", false)}
            {toggle("Tool calls", "31", false)}
          </div>
          <output className="session-detail-toolbar-readout">1 of 45+ events</output>
        </div>
        <div className="session-detail-days">
          <div className="session-detail-day">
            <h2 className="session-detail-day-divider"><span>Oct 2, 2026</span></h2>
            <ol className="session-detail-event-list">
              <li>
                <article className="session-detail-event event-prompt">
                  <span className="session-detail-event-node" role="img" aria-label="You · 21:43"><Icon name="user" size={12} /></span>
                  <div className="session-detail-event-body">
                    <div className="session-detail-card session-detail-card-prompt">
                      <div className="session-detail-card-head"><time>21:43</time><span>31 calls</span></div>
                      <div className="session-detail-event-copy">
                        <p>{agent
                          ? "List every e2e suite under tests/ with its size class, and say which ones the xlarge tag covers."
                          : "help me run all the e2e tests exclude the xlarge one"}</p>
                      </div>
                    </div>
                  </div>
                </article>
              </li>
            </ol>
          </div>
        </div>
      </div>
    </section>
  );
}

/* ——— measuring the trail ——————————————————————————————————————————————
   The action cluster never shrinks, so what the trail cannot hold paints
   under it. Spare width is found the way the bar's budget was measured
   before: grow a spacer in the cluster until a crumb passes the list's edge. */

function trailOverflow(header: HTMLElement): number {
  const list = header.querySelector<HTMLElement>("ol.sync-path");
  const actions = header.querySelector<HTMLElement>(".sync-path-actions");
  if (!list || !actions) return 0;
  const limit = list.getBoundingClientRect().right;
  let worst = -Infinity;
  list.querySelectorAll<HTMLElement>("*").forEach((node) => {
    if (node.closest(".sync-path-menu")) return;
    const rect = node.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) return;
    worst = Math.max(worst, rect.right - limit);
  });
  const edge = header.getBoundingClientRect().right - parseFloat(getComputedStyle(header).paddingRight);
  return Math.max(worst, actions.getBoundingClientRect().right - edge);
}

/** Spare trail width in px; negative is how far the trail runs under the actions. */
function trailSlack(header: HTMLElement): number {
  const actions = header.querySelector<HTMLElement>(".sync-path-actions");
  if (!actions) return 0;
  const over = trailOverflow(header);
  if (over > 0.5) return -Math.round(over);
  const spacer = document.createElement("span");
  spacer.style.cssText = "flex:0 0 auto;height:1px;margin-left:-6px;";
  actions.prepend(spacer);
  let low = 0;
  let high = 600;
  while (high - low > 1) {
    const mid = Math.floor((low + high) / 2);
    spacer.style.width = `${mid}px`;
    if (trailOverflow(header) > 0.5) high = mid;
    else low = mid;
  }
  spacer.remove();
  return low;
}

function measureAll(): Record<string, number> {
  const out: Record<string, number> = {};
  document.querySelectorAll<HTMLElement>("[data-dir]").forEach((frame) => {
    const header = frame.querySelector<HTMLElement>("header.sync-pathbar");
    if (header && frame.dataset.dir) out[frame.dataset.dir] = trailSlack(header);
  });
  return out;
}

(window as unknown as { mkMeasure: typeof measureAll }).mkMeasure = measureAll;

/* ——— the page ——————————————————————————————————————————————————— */

/** The bar is the main pane: the screenshot's window, a 1100px window beside
 *  a 318px sidebar, and the 580px floor. With the sidebar hidden the pane is
 *  the window, whose own floor is 800px. */
function paneWidth(knobs: Knobs): number {
  if (knobs.sidebar === "hidden") return knobs.width === "wide" ? 1100 : knobs.width === "default" ? 900 : 800;
  return knobs.width === "wide" ? 990 : knobs.width === "default" ? 782 : 580;
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [slack, setSlack] = useState<Record<string, number>>({});
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ ...next });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const width = paneWidth(knobs);
  const shown = knobs.only === "all" ? DIRS : [knobs.only];

  useEffect(() => {
    let live = true;
    void document.fonts.ready.then(() => window.setTimeout(() => {
      if (live) setSlack(measureAll());
    }, 60));
    return () => { live = false; };
  }, [knobs]);

  return (
    <div
      className={`spec${knobs.pay === "on" ? " mk-pay" : ""}${knobs.sidebar === "hidden" ? " mk-sidebar-hidden" : ""}`}
      data-page={knobs.page}
    >
      {knobs.shot === "off" && (
        <>
          <h1 className="spec-title">A return button on the session page</h1>
          <p className="spec-sub">
            Every frame is the main pane at the chosen width, drawn with the shipped path bar, Cloud chip and session
            page classes on the shipped CSS. Only the classes prefixed <code>mk-</code> are new. The figure under each
            bar is measured: the width the trail still has to spare, or how far it runs under the buttons.
          </p>

          <div className="spec-controls">
            <div className="spec-group"><span>Page</span>{PAGES.map((id) => seg("page", id, PAGE_LABEL[id]))}</div>
            <div className="spec-group"><span>Project</span>{seg("project", "git", "Git")}{seg("project", "plain", "Not Git")}</div>
            <div className="spec-group"><span>Pane</span>{seg("width", "wide", "Screenshot")}{seg("width", "default", "1100 px window")}{seg("width", "floor", "Minimum")}</div>
            <div className="spec-group"><span>Sidebar</span>{seg("sidebar", "shown", "Shown")}{seg("sidebar", "hidden", "Hidden")}</div>
            <div className="spec-group"><span>A’s 36 px</span>{seg("pay", "on", "The trail pays")}{seg("pay", "off", "Nothing pays")}</div>
            <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
          </div>
        </>
      )}

      <div className="spec-grid">
        {shown.map((dir) => {
          const spare = slack[dir];
          return (
            <div key={dir} data-dir={dir} style={{ width }}>
              <h2 className="spec-col-title">{DIR_LABEL[dir]}</h2>
              <p className="spec-col-note">{DIR_NOTE[dir]}</p>
              <div className="spec-frame">
                {knobs.sidebar === "hidden" && <span className="mk-lights" aria-hidden="true"><i /><i /><i /></span>}
                <PathBar dir={dir} knobs={knobs} />
                <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
                  <SessionPage dir={dir} knobs={knobs} />
                </main>
              </div>
              {spare !== undefined && (
                <p className={`spec-measure${spare < 0 ? " over" : ""}`}>
                  {width} px bar · {spare < 0 ? `trail runs ${-spare} px under the buttons` : `trail has ${spare} px to spare`}
                </p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);

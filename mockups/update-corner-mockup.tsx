import { StrictMode, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import { NoticeZone } from "../src/components/notice/Notice";
import { buildUpdateNotice, type UpdatePhase, type UpdateSummary } from "./update-corner-before";
import type { LocalProjectSummary, StorageSummary } from "../src/types";
import "../src/App.css";

/**
 * Mockup: the update corner.
 *
 * Today an available update is one item in the shell's notice zone (Release
 * notes · Later · Update and restart), the install is a native confirm plus a
 * modal progress dialog, and a manual check that fails says so in a danger
 * notice.
 *
 *   A · Quiet download   ChatGPT / Slack / VS Code: a found release downloads
 *                        and verifies in the background; only then does an
 *                        Update pill appear in the sidebar's version mark.
 *                        Clicking it installs and restarts.
 *   B · Pill on found    The pill appears as soon as a release is found; a
 *                        click downloads (progress in the pill), then restarts.
 *
 * Either way no failure is ever drawn: a failed check, download, signature,
 * or install returns the corner to the version mark, and the next check tries
 * again.
 *
 * Real ProjectSidebar, NoticeZone, buildUpdateNotice and App.css; the pill is
 * portalled into the real footer. `npm run dev`, then /mockups/update-corner-mockup.html
 * (`?dir=before|a|b&state=…&theme=dark&sidebar=open|narrow|hidden`).
 */

const CURRENT = "0.1.4";
const NEXT = "0.1.5";
const SUMMARY: UpdateSummary = {
  version: NEXT,
  currentVersion: CURRENT,
  notes: `https://github.com/Git4Agent/mallard-app/releases/tag/v${NEXT}`,
  date: "2026-09-25T09:00:00.000Z",
};

type Direction = "before" | "a" | "b";
type State = "idle" | "checking" | "found" | "downloading" | "ready" | "restarting" | "failed" | "installed";
type SidebarMode = "open" | "narrow" | "hidden";
type Receipt = "chip" | "none";
type SettingsState = "idle" | "checking" | "latest" | "found" | "ready" | "failed";

const STATES: Array<[State, string]> = [
  ["idle", "Idle"],
  ["checking", "Checking"],
  ["found", `Found ${NEXT}`],
  ["downloading", "Downloading"],
  ["ready", "Ready"],
  ["restarting", "Restarting"],
  ["failed", "Failed"],
  ["installed", "After restart"],
];

interface Knobs {
  dir: Direction;
  state: State;
  theme: "light" | "dark";
  sidebar: SidebarMode;
  receipt: Receipt;
  busy: boolean;
  settings: SettingsState;
}

function readKnobs(): Knobs {
  const q = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = q.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick("dir", ["before", "a", "b"] as const, "a"),
    state: pick("state", STATES.map(([id]) => id), "ready"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    sidebar: pick("sidebar", ["open", "narrow", "hidden"] as const, "open"),
    receipt: pick("receipt", ["chip", "none"] as const, "chip"),
    busy: q.get("busy") === "1",
    settings: pick("settings", ["idle", "checking", "latest", "found", "ready", "failed"] as const, "idle"),
  };
}

/* ——— fixtures —————————————————————————————————————————————————— */

const project = (id: string, name: string, storages: string[]): LocalProjectSummary => ({
  local_project_id: id, bundle_id: `b-${id}`, display_name: name, revision: 1,
  project_root: `/Users/you/code/${name}`, canonical_project_root: `/Users/you/code/${name}`,
  profile_ids: { codex: "p-codex", claude: "p-claude" }, linked_storage_ids: storages, is_git_repository: true,
});
const PROJECTS = [project("app", "mallard-app", ["s-r2"]), project("blog", "blog", ["s-r2"]), project("api", "api", [])];
const STORAGES: StorageSummary[] = [
  { storage_id: "s-r2", name: "Team R2", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [], settings: {}, secrets: [] },
];

/* ——— the proposed corner ————————————————————————————————————— */

/** What the corner shows for a state, or null for the plain version mark. */
function cornerFor(dir: Direction, state: State, receipt: Receipt): "update" | "progress" | "restarting" | "receipt" | null {
  if (dir === "before") return null;
  if (state === "restarting") return "restarting";
  if (state === "installed") return receipt === "chip" ? "receipt" : null;
  if (dir === "a") return state === "ready" ? "update" : null;
  if (state === "found") return "update";
  if (state === "downloading") return "progress";
  return null;
}

function UpdatePill({ kind, dir, busy, progress, onClick }: {
  kind: "update" | "progress" | "restarting" | "receipt";
  dir: Direction;
  busy: boolean;
  progress: number;
  onClick: () => void;
}) {
  if (kind === "receipt") {
    return (
      <button type="button" className="sync-update-pill receipt" onClick={onClick} title={`See what changed in Mallard ${NEXT}`}>
        <span>What’s new</span>
      </button>
    );
  }
  if (kind === "restarting") {
    return (
      <span className="sync-update-pill" role="status">
        <Icon name="refresh" size={13} className="sync-update-spin" />
        <span>Restarting</span>
      </span>
    );
  }
  if (kind === "progress") {
    return (
      <span className="sync-update-pill progress" role="status" aria-label={`Downloading Mallard ${NEXT}, ${progress}%`}>
        <span className="sync-update-fill" style={{ width: `${progress}%` }} />
        <Icon name="download" size={13} />
        <span className="sync-update-num">{progress}%</span>
      </span>
    );
  }
  const title = busy
    ? `Mallard ${NEXT} is ${dir === "a" ? "ready" : "available"}. Finish the Push, then restart to update.`
    : dir === "a"
      ? `Restart Mallard to install ${NEXT}`
      : `Download Mallard ${NEXT} and restart`;
  return (
    <button type="button" className="sync-update-pill" onClick={onClick} disabled={busy} title={title} aria-label={title}>
      <Icon name={dir === "a" ? "refresh" : "download"} size={13} />
      <span>Update</span>
    </button>
  );
}

/* ——— the live frame ———————————————————————————————————————————— */

function Frame({ knobs, setState }: { knobs: Knobs; setState: (state: State) => void }) {
  const frameRef = useRef<HTMLDivElement>(null);
  const [footer, setFooter] = useState<Element | null>(null);
  const [progress, setProgress] = useState(0);
  const [failNext, setFailNext] = useState(false);
  const timers = useRef<number[]>([]);
  const animating = useRef(false);

  useLayoutEffect(() => {
    setFooter(frameRef.current?.querySelector(".sync-sidebar-product-mark") ?? null);
  }, []);

  const clear = () => {
    timers.current.forEach((id) => window.clearTimeout(id));
    timers.current = [];
  };
  useEffect(() => clear, []);
  /** Steps through states, the way the updater would over a few seconds. */
  const play = (steps: Array<[State, number]>) => {
    clear();
    let at = 0;
    for (const [state, wait] of steps) {
      at += wait;
      timers.current.push(window.setTimeout(() => setState(state), at));
    }
  };
  const downloadThen = (after: Array<[State, number]>) => {
    clear();
    animating.current = true;
    setState("downloading");
    setProgress(0);
    for (let step = 1; step <= 10; step += 1) {
      timers.current.push(window.setTimeout(() => setProgress(step * 10), step * 220));
    }
    let at = 2300;
    for (const [state, wait] of after) {
      at += wait;
      timers.current.push(window.setTimeout(() => {
        animating.current = false;
        setState(state);
      }, at));
    }
  };

  /* Picked from the State bar rather than played: a mid-download pill. */
  useEffect(() => {
    if (knobs.state === "downloading" && !animating.current) setProgress(42);
  }, [knobs.state]);

  const install = () => {
    if (knobs.busy) return;
    if (knobs.dir === "a") {
      play(failNext ? [["restarting", 0], ["idle", 1400]] : [["restarting", 0], ["installed", 1600]]);
    } else {
      downloadThen(failNext ? [["idle", 0]] : [["restarting", 0], ["installed", 1600]]);
    }
  };

  const corner = cornerFor(knobs.dir, knobs.state, knobs.receipt);
  const pill = corner && (
    <UpdatePill
      kind={corner}
      dir={knobs.dir}
      busy={knobs.busy && corner === "update"}
      progress={progress}
      onClick={corner === "receipt" ? () => setState("idle") : install}
    />
  );

  /* Before: today's shell notice and modal, from the shipped builder. */
  const beforePhase: UpdatePhase = ({
    idle: "idle", checking: "checking", found: "available", downloading: "downloading", ready: "installing",
    restarting: "installing", failed: "error", installed: "idle",
  } as const)[knobs.state];
  const beforeNotice = knobs.dir === "before"
    ? buildUpdateNotice(
      {
        phase: beforePhase,
        summary: knobs.state === "checking" || knobs.state === "failed" || knobs.state === "idle" || knobs.state === "installed" ? null : SUMMARY,
        error: knobs.state === "failed" ? "error sending request for url (https://github.com/Git4Agent/mallard-app/releases/latest/download/latest.json)" : null,
        completed: knobs.state === "installed" ? SUMMARY : null,
        busy: knobs.busy,
      },
      {
        onInstall: () => downloadThen([["restarting", 0], ["installed", 1600]]),
        onLater: () => setState("idle"),
        onRetry: () => play([["checking", 0], ["found", 1200]]),
        onDismiss: () => setState("idle"),
        onOpenReleaseNotes: () => undefined,
      },
    )
    : null;
  const beforeModal = knobs.dir === "before" && (knobs.state === "downloading" || knobs.state === "ready" || knobs.state === "restarting");
  const installing = knobs.state !== "downloading";

  const showDot = knobs.sidebar === "hidden" && (corner === "update" || corner === "progress");
  const width = knobs.sidebar === "narrow" ? 220 : 298;

  return (
    <>
      <div className="uc-row">
        <div className="uc-group">
          <span>Play</span>
          <button type="button" className="uc-seg play" onClick={() => play(knobs.dir === "a"
            ? [["checking", 0], ["found", 1100], ["downloading", 500], ["ready", 2400]]
            : [["checking", 0], ["found", 1100]])}
          >
            Scheduled check finds {NEXT}
          </button>
          <button type="button" className="uc-seg play" onClick={() => play([["checking", 0], ["failed", 1100], ["idle", 900]])}>
            Scheduled check fails
          </button>
          <label className="uc-check"><input type="checkbox" checked={failNext} onChange={(event) => setFailNext(event.target.checked)} /> Next download/install fails</label>
        </div>
      </div>
      <div className="uc-frame" ref={frameRef}>
        <div
          className={`sync-app${knobs.sidebar === "hidden" ? " sidebar-hidden" : ""}`}
          style={{ ["--sync-sidebar-width" as string]: `${width}px` }}
        >
          <button
            type="button"
            className="sync-sidebar-toggle"
            title={showDot ? `Show sidebar — Mallard ${NEXT} is ready` : "Toggle sidebar ⌘B"}
            aria-label={showDot ? `Show sidebar, Mallard ${NEXT} is ready` : knobs.sidebar === "hidden" ? "Show sidebar" : "Hide sidebar"}
          >
            <Icon name={knobs.sidebar === "hidden" ? "sidebar-expand" : "sidebar-collapse"} size={15} />
            {showDot && <span className="sync-update-dot" />}
          </button>
          <ProjectSidebar
            projects={PROJECTS}
            drafts={[]}
            activeDraftId={null}
            storages={STORAGES}
            storageDrafts={[]}
            storageProviders={[]}
            storageUsage={{}}
            activeProjectId="app"
            activeStorageId={null}
            catalogScopes={[]}
            activeCatalogScopeId={null}
            addingProject={false}
            addingCatalogScope={false}
            addingStorage={false}
            catalogIndexOpen={false}
            storageIndexOpen={false}
            settingsOpen={false}
            loading={false}
            busy={false}
            activityOpen={false}
            unreadLogs={0}
            onOpenProjects={() => undefined}
            onSelectProject={() => undefined}
            onRemoveProject={() => undefined}
            onSelectDraft={() => undefined}
            onDiscardDraft={() => undefined}
            onToggleActivity={() => undefined}
            onAddProject={() => undefined}
            onOpenStorage={() => undefined}
            onOpenStorageDraft={() => undefined}
            onDiscardStorageDraft={() => undefined}
            onRemoveStorage={() => undefined}
            onAddStorage={() => undefined}
            onOpenCatalogIndex={() => undefined}
            onOpenCatalogScope={() => undefined}
            onAddCatalogScope={() => undefined}
            onRemoveCatalogScope={() => undefined}
            onOpenStorageIndex={() => undefined}
            onOpenSettings={() => undefined}
          />
          <div className="sync-workspace">
            <header className="sync-titlebar">
              <div className="sync-titlebar-context"><Icon name="folder" size={16} /><strong>mallard-app</strong></div>
            </header>
            {beforeNotice && <NoticeZone items={[beforeNotice]} subject="messages from Mallard" className="sync-backend-banner" timing="progress" />}
            <main className="sync-main">
              <div className="uc-placeholder">
                <strong>{knobs.busy ? "Pushing 14 sessions to Team R2…" : "mallard-app"}</strong>
                {knobs.busy
                  ? "A tracked operation is running. The updater never restarts over it."
                  : "The project workspace — not part of this specimen."}
              </div>
            </main>
          </div>
          {beforeModal && (
            <>
              <div className="uc-backdrop" />
              <dialog open className="sync-modal app-update-dialog" aria-labelledby="uc-update-title">
                <header className="sync-modal-header">
                  <div>
                    <h1 id="uc-update-title">{installing ? "Installing update" : `Downloading Mallard ${NEXT}`}</h1>
                    <p>{installing ? "Mallard restarts when the update is in place." : "Keep Mallard open while the signed release downloads."}</p>
                  </div>
                </header>
                <div className="sync-modal-body">
                  <progress className="app-update-progress" aria-label="Update progress" max={100} value={installing ? undefined : progress} />
                  <p className="app-update-progress-label">{installing ? "Verifying the signature and replacing the application." : `${progress}% downloaded`}</p>
                </div>
              </dialog>
            </>
          )}
        </div>
        {footer && pill && createPortal(pill, footer)}
      </div>
    </>
  );
}

function caption(dir: Direction, state: State, busy: boolean): ReactNode {
  if (dir === "before") {
    switch (state) {
      case "idle": return <>Nothing drawn. The footer is only the version mark.</>;
      case "checking": return <>Shown <b>only for a manual check</b> (Settings › Check for updates). A scheduled check is silent.</>;
      case "found": return <>The one shell notice: <b>Release notes · Later · Update and restart</b>. <i>Later</i> hides this version for 24 h.{busy ? " While busy, the install button is disabled." : ""}</>;
      case "downloading": return <>After a native <code>confirm()</code>: a modal that owns the window until relaunch (5 min download timeout).</>;
      case "ready":
      case "restarting": return <>The modal switches to <i>Installing update</i>, then <code>relaunch()</code>.</>;
      case "failed": return <>A <b>manual</b> check or an install failure shows this danger notice today. A <b>scheduled</b> failure is already silent.</>;
      case "installed": return <>The first launch on the new version shows a one-time receipt (localStorage pending-changelog key).</>;
    }
  }
  const silent = <>The corner stays the plain version mark.</>;
  switch (state) {
    case "idle": return silent;
    case "checking": return <>{silent} No spinner — a check is not news.</>;
    case "found": return dir === "a"
      ? <>{silent} The release is found; the download starts on its own, in the background.</>
      : <>The pill appears as soon as the manifest names a newer version. Nothing is downloaded yet.</>;
    case "downloading": return dir === "a"
      ? <>{silent} Tens of MB arrive in the background; the signature is verified before anything is shown.</>
      : <>After the click: the pill fills with the download. No modal; the window stays usable.</>;
    case "ready": return dir === "a"
      ? <>Bytes downloaded and signature verified. <b>Update</b> now only runs <code>install()</code> + <code>relaunch()</code>.{busy ? " While a Push runs, the pill stays but is disabled; its title says why." : ""}</>
      : <>B has no ready state: the download is already the user’s click, so it installs straight away (Restarting).</>;
    case "restarting": return <>The click is the confirmation — no native <code>confirm()</code>, no modal. On Windows the installer exits the app itself.</>;
    case "failed": return <>{silent} A failed check, background download, or signature check returns here with <b>nothing drawn</b>; the next scheduled check retries. (A failure <i>after</i> the click is an open question below.)</>;
    case "installed": return <>Open question: a quiet <b>What’s new</b> chip for this launch only, or nothing at all (toggle <i>Receipt</i>).</>;
  }
}

/* ——— the static gallery ———————————————————————————————————————— */

function Sample({ width, children }: { width: number; children?: ReactNode }) {
  return (
    <div className="uc-widths">
      <small>{width}px sidebar</small>
      <div className="uc-foot-sample" style={{ width }}>
        <footer className="sync-sidebar-product-mark">
          <img src="/mallard-logo-duck.svg" alt="" />
          <span>v{CURRENT}</span>
          {children}
        </footer>
      </div>
    </div>
  );
}

function Gallery({ busy }: { busy: boolean }) {
  const cells: Array<{ title: string; dir: Direction; kind: "update" | "progress" | "restarting" | "receipt" | null; busy?: boolean; note: string }> = [
    { title: "Idle, checking, downloading (A), failed", dir: "a", kind: null, note: "Unchanged from today." },
    { title: "A · Ready", dir: "a", kind: "update", note: `Title: “Restart Mallard to install ${NEXT}”.` },
    { title: "A · Ready, Push running", dir: "a", kind: "update", busy: true, note: "Disabled; the title says to finish the Push first." },
    { title: "B · Found", dir: "b", kind: "update", note: `Title: “Download Mallard ${NEXT} and restart”.` },
    { title: "B · Downloading", dir: "b", kind: "progress", note: "The pill fills; the window stays usable." },
    { title: "Restarting (A and B)", dir: "a", kind: "restarting", note: "A status, not a button, while install() and relaunch() run." },
    { title: "After restart · receipt chip", dir: "a", kind: "receipt", note: "This launch only; opens the release page." },
  ];
  return (
    <div className="uc-gallery">
      {cells.map((cell) => (
        <div className="uc-cell" key={cell.title}>
          <h3>{cell.title}</h3>
          {[298, 220].map((width) => (
            <Sample width={width} key={width}>
              {cell.kind && (
                <UpdatePill kind={cell.kind} dir={cell.dir} busy={(cell.busy ?? false) || (busy && cell.kind === "update")} progress={42} onClick={() => undefined} />
              )}
            </Sample>
          ))}
          <p>{cell.note}</p>
        </div>
      ))}
    </div>
  );
}

/* ——— Settings ——————————————————————————————————————————————————— */

function Row({ label, children, control }: { label: string; children: ReactNode; control: ReactNode }) {
  return (
    <div className="sync-settings-row">
      <div className="sync-settings-copy"><strong>{label}</strong><p>{children}</p></div>
      <div className="sync-settings-control">{control}</div>
    </div>
  );
}

function SettingsBefore({ state }: { state: SettingsState }) {
  const notice = buildUpdateNotice(
    {
      phase: ({ idle: "idle", checking: "checking", latest: "latest", found: "available", ready: "available", failed: "error" } as const)[state],
      summary: state === "found" || state === "ready" ? SUMMARY : null,
      error: state === "failed" ? "error sending request for url (…/latest.json)" : null,
      completed: null,
      busy: false,
    },
    { onInstall: () => undefined, onLater: () => undefined, onRetry: () => undefined, onDismiss: () => undefined, onOpenReleaseNotes: () => undefined },
  );
  return (
    <>
      {notice ? <NoticeZone items={[notice]} subject="messages from Mallard" timing="progress" /> : <p className="uc-lede">(shell notice zone — empty)</p>}
      <div className="sync-settings-card" style={{ marginTop: 8 }}>
        <Row
          label="Automatic update checks"
          control={<label className="sync-settings-toggle"><input type="checkbox" role="switch" className="sync-settings-switch" defaultChecked aria-label="Automatic update checks" /><span className="sync-settings-state" aria-hidden="true">On</span></label>}
        >
          Check GitHub for new releases at startup and every 6 hours. Sends nothing about you or your projects.
        </Row>
        <Row label="App version" control={<button type="button" className="btn">Check for updates</button>}>
          Mallard {CURRENT}. Updates are signed GitHub releases.
        </Row>
      </div>
    </>
  );
}

function SettingsAfter({ state, busy }: { state: SettingsState; busy: boolean }) {
  const copy: Record<SettingsState, ReactNode> = {
    idle: <>Mallard {CURRENT}. <span className="sync-update-when">Checked 12 minutes ago.</span></>,
    checking: <>Mallard {CURRENT}. <span className="sync-update-when">Checked 12 minutes ago.</span></>,
    latest: <>Mallard {CURRENT} is the newest release. <span className="sync-update-when">Checked just now.</span></>,
    found: <>Mallard {NEXT} is downloading. It installs when you restart.</>,
    ready: <>Mallard {NEXT} is ready to install. <span className="sync-update-when">You have {CURRENT}.</span></>,
    failed: <>Mallard {CURRENT}. <span className="sync-update-when">Checked 12 minutes ago.</span></>,
  };
  const control = state === "ready"
    ? (
      <button type="button" className="btn btn-primary" disabled={busy} title={busy ? "Finish the Push first" : undefined}>
        <Icon name="refresh" size={14} /> Restart to update
      </button>
    )
    : (
      <button type="button" className="btn" disabled={state === "checking" || state === "found"}>
        {state === "checking" && <Icon name="refresh" size={14} className="sync-update-spin" />}
        {state === "checking" ? "Checking…" : "Check for updates"}
      </button>
    );
  return (
    <>
      <p className="uc-lede">(shell notice zone — never used by the updater)</p>
      <div className="sync-settings-card" style={{ marginTop: 8 }}>
        <Row
          label="Automatic updates"
          control={<label className="sync-settings-toggle"><input type="checkbox" role="switch" className="sync-settings-switch" defaultChecked aria-label="Automatic updates" /><span className="sync-settings-state" aria-hidden="true">On</span></label>}
        >
          Download new releases from GitHub in the background. They install when you restart Mallard. Sends nothing about you or your projects.
        </Row>
        <Row label="App version" control={control}>{copy[state]}</Row>
      </div>
    </>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

function Seg<T extends string>({ label, value, options, onPick }: { label: string; value: T; options: Array<[T, string]>; onPick: (value: T) => void }) {
  return (
    <div className="uc-group">
      <span>{label}</span>
      {options.map(([id, text]) => (
        <button type="button" key={id} className="uc-seg" aria-pressed={value === id} onClick={() => onPick(id)}>{text}</button>
      ))}
    </div>
  );
}

function Page() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => setKnobs((current) => ({ ...current, [key]: value }));

  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
    const q = new URLSearchParams();
    q.set("dir", knobs.dir);
    q.set("state", knobs.state);
    if (knobs.theme === "dark") q.set("theme", "dark");
    if (knobs.sidebar !== "open") q.set("sidebar", knobs.sidebar);
    if (knobs.receipt !== "chip") q.set("receipt", knobs.receipt);
    if (knobs.busy) q.set("busy", "1");
    if (knobs.settings !== "idle") q.set("settings", knobs.settings);
    window.history.replaceState(null, "", `?${q}`);
  }, [knobs]);

  return (
    <div className="uc-spec">
      <div>
        <h1 className="uc-title">Update corner — directions</h1>
        <p className="uc-lede">
          The sidebar’s bottom-left version mark becomes the only place an update shows up, the way ChatGPT’s desktop app puts
          an <b>Update</b> button beside the profile. No failure is ever drawn. Real <code>ProjectSidebar</code>,{" "}
          <code>NoticeZone</code>, <code>buildUpdateNotice</code> and <code>App.css</code>; the pill is portalled into the real footer.
        </p>
      </div>

      <div className="uc-controls">
        <div className="uc-row">
          <Seg label="Direction" value={knobs.dir} options={[["before", "Before (today)"], ["a", "A · Quiet download"], ["b", "B · Pill on found"]]} onPick={(value) => set("dir", value)} />
          <Seg label="Theme" value={knobs.theme} options={[["light", "Light"], ["dark", "Dark"]]} onPick={(value) => set("theme", value)} />
        </div>
        <div className="uc-row">
          <Seg label="State" value={knobs.state} options={STATES} onPick={(value) => set("state", value)} />
        </div>
        <div className="uc-row">
          <Seg label="Sidebar" value={knobs.sidebar} options={[["open", "298px"], ["narrow", "220px (min)"], ["hidden", "Hidden"]]} onPick={(value) => set("sidebar", value)} />
          <Seg label="Receipt" value={knobs.receipt} options={[["chip", "What’s new chip"], ["none", "None"]]} onPick={(value) => set("receipt", value)} />
          <label className="uc-check"><input type="checkbox" checked={knobs.busy} onChange={(event) => set("busy", event.target.checked)} /> Push running</label>
        </div>
      </div>

      <p className="uc-caption">{caption(knobs.dir, knobs.state, knobs.busy)}</p>
      <Frame knobs={knobs} setState={(state) => set("state", state)} />

      <h2 className="uc-h2">Every corner state, at the default and the minimum sidebar width</h2>
      <Gallery busy={knobs.busy} />

      <h2 className="uc-h2">Settings › Privacy — the manual check</h2>
      <div className="uc-row">
        <Seg
          label="Check"
          value={knobs.settings}
          options={[["idle", "Idle"], ["checking", "Checking"], ["latest", "Up to date"], ["found", `Found ${NEXT}`], ["ready", "Ready"], ["failed", "Failed"]]}
          onPick={(value) => set("settings", value)}
        />
      </div>
      <div className="uc-two">
        <div><h3>Before</h3><SettingsBefore state={knobs.settings} /></div>
        <div><h3>After</h3><SettingsAfter state={knobs.settings} busy={knobs.busy} /></div>
      </div>
      <p className="uc-caption">
        A failed manual check leaves the row exactly as it was: <i>Checked 12 minutes ago</i> only advances on a check that
        reached GitHub, so a stale time is the whole signal. A successful check that finds a release downloads it (even with
        automatic updates off — the user asked) and lands in the same Ready state as the corner.
      </p>

      <h2 className="uc-h2">What the cutover would remove</h2>
      <ul className="uc-notes">
        <li><code>buildUpdateNotice</code> and the updater’s item in the shell notice zone (all five notices).</li>
        <li><code>UpdateProgressDialog</code> and its CSS (<code>app-update-dialog</code>, <code>app-update-progress*</code>).</li>
        <li>The native <code>confirm()</code> before install — the pill click is the consent; the busy gate is the safety.</li>
        <li><i>Later</i> and its 24 h defer key (<code>mallard.update.deferred</code>) — a pill that waits quietly needs no snooze.</li>
        <li><code>describeUpdateError</code> — nothing renders an error any more. (Failures could still go to the activity log.)</li>
      </ul>
      <h2 className="uc-h2">Open questions</h2>
      <ul className="uc-notes">
        <li><b>A or B?</b> A (quiet download) is what ChatGPT does and never shows progress or a failure window; it costs a background download of every release.</li>
        <li><b>Receipt after restart:</b> the quiet <i>What’s new</i> chip for one launch, or nothing?</li>
        <li><b>Toggle label:</b> with A, the switch also downloads in the background — rename “Automatic update checks” to “Automatic updates”?</li>
        <li><b>Quit without clicking (A):</b> the plugin keeps downloaded bytes in the running process only (<code>Update.downloadedBytes</code>). Install on Quit (Mallard’s own Quit item in <code>lib.rs</code>, the way ChatGPT does), or accept a re-download next launch?</li>
        <li><b>Failure after the click:</b> the user asked for silent <i>detection</i>. If <code>install()</code> fails after they pressed Update, should the pill vanish (as mocked), or come back as Update so they can try again?</li>
      </ul>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Page /></StrictMode>);

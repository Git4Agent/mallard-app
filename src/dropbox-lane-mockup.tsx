import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import { CandidateGroups, CandidateRow, candidateSelectable, DiscoveryEmpty, ScanIncompleteNotice } from "./components/project-sync/StorageDiscovery";
import { Step, type StepView } from "./components/project-sync/StorageSetupStep";
import type { StorageDiscoveryResult } from "./types";
import "./App.css";

/**
 * The Dropbox setup lane, drawn beside the shipped R2 lane's grammar. Before
 * reproduces `DropboxSetupLane` as it ships; A, B and C bring it to the R2
 * lane in three steps — the same controls, the same skeleton, the same
 * voice. Every panel draws the same frozen scene, so only the lane differs.
 * `npm run dev`, then /dropbox-lane-mockup.html (`?theme=dark`,
 * `?scene=idle|browser|account|naming|folders|failed|ready`,
 * `?only=before|A|B|C`).
 */

/* ——— frozen data ————————————————————————————————————— */

type Scene = "idle" | "browser" | "account" | "naming" | "folders" | "failed" | "ready";
const SCENES: [Scene, string][] = [
  ["idle", "Before start"], ["browser", "Browser consent"], ["account", "Confirm account"], ["naming", "Naming the folder"],
  ["folders", "Use existing · list"], ["failed", "Consent denied"], ["ready", "Ready for review"],
];
type Level = "before" | "A" | "B" | "C";

const ACCOUNT = { name: "He Qu", email: "quhe1996@gmail.com" };
const LINK = "https://www.dropbox.com/oauth2/authorize?client_id=abc123xyz&response_type=code&token_access_type=offline&redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&state=x9f2";
const FOLDER = "Mallard fox";
const NAMESPACE = "1234567890";
const DESCRIPTOR = { icon: "cloud" };
const STORAGES = [{ storage_id: "storage-1", name: "Team notes" }];

const SCAN: StorageDiscoveryResult = {
  scan_id: "scan-1",
  scope: "quhe1996@gmail.com · shared folders and team space",
  status: "partial",
  issues: ["1 shared folder could not be read (Legal archive: not mounted)."],
  candidates: [
    { candidate_id: "dbx-1", name: "Mallard otter", identity: "ns 1234567890", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Access", value: "Owner" }], cautions: [] },
    { candidate_id: "dbx-2", name: "Mallard team notes", identity: "ns 2233445566", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Access", value: "Viewer" }], cautions: ["Viewer — Push will be refused"], connected_storage_id: "storage-1" },
    { candidate_id: "dbx-3", name: "Legal archive", identity: "ns 9988776655", marker: "unreadable", marker_detail: "not mounted", facts: [{ label: "Access", value: "Viewer" }], cautions: ["Not in your Dropbox"] },
    { candidate_id: "dbx-4", name: "Acme team space", identity: "ns 5544332211", marker: "missing", facts: [{ label: "Team", value: "space" }, { label: "Access", value: "Editor" }], cautions: [] },
    { candidate_id: "dbx-5", name: "Family photos", identity: "ns 3344556677", marker: "missing", facts: [{ label: "Access", value: "Owner" }], cautions: [] },
    { candidate_id: "dbx-6", name: "Design handoff", identity: "ns 4455667788", marker: "missing", facts: [{ label: "Access", value: "Editor" }], cautions: [] },
  ],
};

const modeOf = (scene: Scene): "create" | "existing" => scene === "folders" ? "existing" : "create";
const noop = () => undefined;

/* ——— the lane as it ships (DropboxSetupLane, frozen per scene) ———————— */

function BeforeLane({ scene }: { scene: Scene }) {
  const mode = modeOf(scene);
  const button = (label: string) => <button type="button" className="btn storage-setup-guide-button">{label}</button>;
  if (scene === "idle") return <>
    <span className="storage-setup-guide-copy">Sign in using a browser on this computer. Dropbox permission covers your account, beyond the selected folder.</span>
    {button(mode === "existing" ? "Find in my account" : "Connect Dropbox")}
  </>;
  const phase = { browser: "login", account: "account", naming: "folder", folders: "pick", failed: "login", ready: "review" }[scene];
  const index = ["idle", "login", "account", "folder", "find", "pick", "prepare", "review"].indexOf(phase);
  const error = scene === "failed" ? "Dropbox access was not approved." : null;
  const running = scene === "browser";
  const status = (first: number, last = first): StepView["status"] => index > last ? "done" : index < first ? "todo" : error ? "failed" : "doing";
  const signedIn = index > 1;
  const rows: StepView[] = [
    { key: "login", title: "Sign in", status: status(1),
      fact: scene === "browser" ? <>Complete Dropbox consent · 2:41</> : signedIn ? "Browser consent received" : error ? "Authorization did not finish" : "Preparing authorization…",
      controls: scene === "browser" ? <>
        <input aria-label="Dropbox authorization link" readOnly value={LINK} className="dropbox-authorization-link" />
        {button("Copy link")}{button("Open browser")}
      </> : undefined },
    { key: "account", title: "Account", status: status(2),
      fact: signedIn ? <>{ACCOUNT.name} · {ACCOUNT.email}</> : "Confirm before listing or creating folders",
      controls: scene === "account" ? <>{button("Continue")}{button("Switch account")}</> : undefined },
    { key: "folder", title: mode === "existing" ? "Folders" : "Folder", status: status(3, 5),
      fact: scene === "naming" ? <input aria-label="Dropbox folder name" defaultValue={FOLDER} />
        : scene === "ready" ? FOLDER : "Choose a folder after sign-in",
      controls: scene === "naming" ? <>{button("Suggest another")}{button("Create folder")}</> : scene === "folders" ? button("Scan again") : undefined,
      body: scene === "folders" ? <div className="r2-step-list">
        <ScanIncompleteNotice issues={SCAN.issues} />
        <CandidateGroups result={SCAN} unitPlural="folders" choice={false} row={(candidate) => <CandidateRow
          key={candidate.candidate_id} descriptor={DESCRIPTOR} candidate={candidate} disabled={false}
          connectedName={STORAGES.find((s) => s.storage_id === candidate.connected_storage_id)?.name ?? null}
          verb={candidateSelectable(candidate) ? button("Use folder") : undefined} onOpenStorage={noop} />} />
      </div> : undefined },
    { key: "connection", title: "Connection", status: status(6), fact: index > 6 ? "Namespace and refresh credential prepared" : "Verify the folder and token refresh; fill the draft" },
    { key: "review", title: "Review", status: scene === "ready" ? "done" : "todo", fact: "Test connection, then Finish setup" },
  ];
  return <>
    <ol className="r2-steps">{rows.map((step) => <Step key={step.key} step={step} />)}</ol>
    {error && <span className="storage-setup-guide-error" role="alert">{error}</span>}
    {running ? button("Cancel") : <>
      {error && button("Try again")}
      {phase !== "account" && phase !== "review" && button("Switch account")}
    </>}
    <span className="storage-setup-guide-copy">Dropbox permission covers your account, beyond this folder.</span>
  </>;
}

/* ——— the lane in the R2 lane's grammar, at three depths ———————————— */

const chip = (label: string, value: string) => <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>;

function AfterLane({ scene, level }: { scene: Scene; level: Exclude<Level, "before"> }) {
  const mode = modeOf(scene);
  const skeleton = level !== "A";
  const voice = level === "C";
  const button = (label: ReactNode, tone: "plain" | "primary" | "ghost" = "plain", inert = false) => (
    <button type="button" className={`btn storage-setup-guide-button${tone === "primary" ? " btn-primary" : tone === "ghost" ? " btn-ghost" : ""}`} disabled={inert}>{label}</button>
  );
  const resetButton = (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" title="Discards this setup draft and all of its progress, then opens a fresh one. A folder already created in Dropbox is kept.">
      <Icon name="refresh" size={12} /> Reset setup
    </button>
  );
  const changeLogin = skeleton ? button(<><Icon name="user" size={12} /> Change login</>) : button("Switch account");

  if (scene === "idle") return <>
    <span className="storage-setup-guide-copy">
      {voice
        ? (mode === "existing" ? "Lists your shared folders read-only, then keeps a renewing connection to the one you pick. Consent happens in a browser you choose."
          : "Creates a shared folder and keeps a renewing connection to it. Sign-in and consent happen in a browser you choose. Dropbox consent covers your whole account, not only this folder.")
        : "Sign in using a browser on this computer. Dropbox permission covers your account, beyond the selected folder."}
    </span>
    <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta">
      <Icon name="link" size={13} /> {mode === "existing" ? "Find in my Dropbox" : "Connect Dropbox"}
    </button>
  </>;

  if (scene === "ready") return <>
    <span className="r2-ready" role="status">
      <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
      <span className="r2-facts">
        {chip("Folder", FOLDER)}
        {chip("Namespace", NAMESPACE)}
        {chip("Account", ACCOUNT.name)}
        {chip("Access", voice ? "account-wide · renews" : "renews automatically")}
      </span>
    </span>
    {resetButton}
  </>;

  const failed = scene === "failed";
  const signedIn = scene === "account" || scene === "naming" || scene === "folders";
  const browserControls = <>
    {button(<><Icon name="copy" size={12} /> Copy authorization link</>, "primary")}
    {button(<><Icon name="external-link" size={12} /> Open browser</>)}
    {button("Cancel", "ghost")}
  </>;
  const nameField = (
    <span className="r2-name-field">
      <input className="r2-name-input" defaultValue={FOLDER} aria-label="Folder name" spellCheck={false} />
      {button(<><Icon name="refresh" size={12} /> Suggest another</>)}
    </span>
  );
  const signIn: StepView = {
    key: "login", title: "Sign in", status: failed ? "failed" : scene === "browser" ? "doing" : "done",
    fact: failed ? (skeleton ? "Dropbox access was not approved" : "Authorization did not finish")
      : scene === "browser" ? <>{voice ? "Approve access in a browser on this computer" : "Complete Dropbox consent"} · <span className="r2-countdown">2:41</span></>
      : voice ? <>Approved · covers your whole Dropbox account</> : "Browser consent received",
    controls: failed ? <>{button("Try again", "primary")}{changeLogin}</>
      : scene === "browser" ? browserControls
      : skeleton && scene !== "account" ? changeLogin : undefined,
  };
  const account: StepView = {
    key: "account", title: "Account", status: failed ? "todo" : scene === "browser" ? "todo" : scene === "account" ? "doing" : "done",
    fact: signedIn ? <>{ACCOUNT.name} · <code>{ACCOUNT.email}</code></> : voice ? "Confirmed before anything is created or listed" : "Confirm before listing or creating folders",
    controls: scene === "account" ? <>{button("Continue", "primary")}{changeLogin}</> : undefined,
  };
  const folder: StepView = mode === "existing" ? {
    key: "folders", title: "Folders", status: "doing",
    fact: voice ? <>{SCAN.candidates.length} checked · read-only</> : "Listing accessible folders and reading markers · read-only",
    controls: button(<><Icon name="refresh" size={12} /> Scan again</>),
    body: <div className="r2-step-list">
      <ScanIncompleteNotice issues={SCAN.issues} />
      <CandidateGroups result={SCAN} unitPlural="shared folders" choice={false} row={(candidate) => <CandidateRow
        key={candidate.candidate_id} descriptor={DESCRIPTOR} candidate={candidate} disabled={false}
        connectedName={STORAGES.find((s) => s.storage_id === candidate.connected_storage_id)?.name ?? null}
        verb={candidateSelectable(candidate) ? button(voice ? "Use & connect" : "Use folder", "primary") : undefined} onOpenStorage={noop} />} />
      {!SCAN.candidates.some(candidateSelectable) && <DiscoveryEmpty place="your Dropbox" unit="shared folder" sentence="Nothing was changed." disabled={false} onCreateInstead={noop} />}
    </div>,
  } : {
    key: "folder", title: "Folder", status: scene === "naming" ? "doing" : "todo",
    fact: scene === "naming" ? <>{nameField}<span>· proposed by Mallard</span></>
      : voice ? "A new shared folder · a proposed name you can change" : "Choose a folder after sign-in",
    controls: scene === "naming" ? button(skeleton ? "Create & connect" : "Create folder", "primary") : undefined,
  };
  const access: StepView = skeleton
    ? { key: "access", title: "Access", status: "todo", fact: voice ? "Refresh credential · renews automatically · your whole account" : "A refresh credential for this account · renews automatically" }
    : { key: "connection", title: "Connection", status: "todo", fact: "Verify the folder and token refresh; fill the draft" };
  const review: StepView = { key: "review", title: "Review", status: "todo", fact: "Test connection, then Finish setup" };
  return <>
    {resetButton}
    <ol className="r2-steps">{[signIn, account, folder, access, review].map((step) => <Step key={step.key} step={step} />)}</ol>
    {!skeleton && failed && <span className="storage-setup-guide-error" role="alert">Dropbox access was not approved.</span>}
    {!skeleton && <span className="storage-setup-guide-copy">Dropbox permission covers your account, beyond this folder.</span>}
  </>;
}

/* ——— the card around the lane, in the app's classes ————————————— */

function Field({ label, value, placeholder, secret }: { label: string; value?: string; placeholder?: string; secret?: boolean }) {
  return (
    <label>
      <span className="form-label-row"><span className="sync-field-name">{label}</span><button type="button" className="field-help" aria-label={label}>?</button></span>
      {secret ? (
        <span className="sync-secret-input">
          <input type="password" value={value ?? ""} placeholder={placeholder} readOnly autoComplete="new-password" />
          <button type="button" className="sync-secret-reveal">Show</button>
        </span>
      ) : <input type="text" value={value ?? ""} placeholder={placeholder} readOnly />}
    </label>
  );
}

function Card({ scene, level }: { scene: Scene; level: Level }) {
  const mode = modeOf(scene);
  const ready = scene === "ready";
  const door = (icon: IconName, checked: boolean, title: string, detail: string) => (
    <div className="storage-mode-option">
      <button type="button" className="storage-mode-door" role="radio" aria-checked={checked}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
    </div>
  );
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <article className="sync-storage-card">
          <header>
            <span className="sync-storage-icon"><Icon name="cloud" size={18} /></span>
            <input value={ready ? FOLDER : "Dropbox storage 1"} readOnly aria-label="Storage name" />
            <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
              {["Cloudflare R2", "Local folder", "Dropbox"].map((label) => (
                <button key={label} type="button" role="radio" aria-checked={label === "Dropbox"} className={label === "Dropbox" ? "active" : undefined}>{label}</button>
              ))}
            </div>
          </header>
          {!ready && (
            <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing storage">
              {door("plus", mode === "create", "Create new", "A new Mallard shared folder in your Dropbox, created after browser consent.")}
              {door("search", mode === "existing", "Use existing", "A shared folder Mallard already uses, found in your Dropbox — or a namespace you paste in.")}
            </div>
          )}
          <div className="storage-setup-route-row agent r2-lane" role="note">
            <Icon name={level === "before" ? "terminal" : "link"} size={13} />
            <span className="storage-setup-route-label">Dropbox setup</span>
            {level === "before" ? <BeforeLane scene={scene} /> : <AfterLane scene={scene} level={level} />}
          </div>
          {!ready && (
            <div className="storage-setup-route-row" role="note">
              <Icon name="info" size={13} />
              <span className="storage-setup-route-label">Manual setup</span>
              <span className="storage-setup-guide-copy">Use your own connection credentials.</span>
              <button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>
              <button type="button" className="btn storage-manual-toggle" aria-label="Enter by hand"><Icon name="chevron-down" size={12} /></button>
            </div>
          )}
          {ready && (
            <div className="sync-storage-fields storage-provider-fields" aria-label="Dropbox connection details">
              <Field label="Namespace ID" value={NAMESPACE} placeholder="Shared or team namespace ID" />
              <Field label="Access token" secret value="sl.u.AFm9-…" placeholder="User-linked Dropbox access token" />
              <Field label="Refresh token" secret value="rt.9Kq2…" placeholder="Dropbox refresh token" />
            </div>
          )}
        </article>
      </div>
      <footer className="sync-storage-setup-footer">
        <span className="sync-storage-editor-progress" aria-live="polite">{ready ? "3 of 3 set" : "0 of 3 set"}</span>
        <div>
          <button type="button" className="btn btn-ghost">Discard draft…</button>
          <button type="button" className="btn btn-ghost">Cancel</button>
          <button type="button" className="btn" disabled={!ready}>Test connection</button>
          <button type="button" className="btn btn-primary" disabled={!ready}>Finish setup</button>
        </div>
      </footer>
    </div>
  );
}

/* ——— the page ———————————————————————————————————————— */

interface Direction { key: Level; title: string; blurb: string; changes: [string, string][] }
const DIRECTIONS: Direction[] = [
  {
    key: "before", title: "Before · as shipped",
    blurb: "The Dropbox lane borrows the R2 step list but none of its controls: a plain call-to-action under a terminal icon, the raw authorization link in a text box, buttons without tone or marks, a bare name field, the error and the verbs at the lane's tail with a warning sentence after them, and no ready line — at review the six steps stay on screen.",
    changes: [],
  },
  {
    key: "A", title: "A · Same controls",
    blurb: "The Dropbox steps and sentences stay; every control becomes the R2 lane's. The accent call-to-action with a link mark, Reset setup at the lane's head, the browser line with the countdown and Copy authorization link · Open browser · Cancel (the link itself is no longer printed), Continue in the primary tone, the mono name field with Suggest another and a note, the folder list's verb in the primary tone, and the ready line with its fact chips.",
    changes: [
      ["Call-to-action", "storage-setup-agent-cta with a link mark; the lane's row mark is a link, not a terminal"],
      ["Browser line", "r2-countdown; Copy authorization link (primary, copy mark) · Open browser (external-link mark) · Cancel (ghost) on the line; no link text box"],
      ["Name field", "r2-name-field + r2-name-input, Suggest another with the refresh mark, '· proposed by Mallard' note"],
      ["Ready", "r2-ready sentence with Folder · Namespace · Account · Access chips, Reset setup beside it"],
      ["Kept", "Five Dropbox steps, their facts, Switch account, the tail error and the permission sentence"],
    ],
  },
  {
    key: "B", title: "B · Same skeleton",
    blurb: "A, plus the lane's structure follows R2: a failure is said on its own line in the danger colour with Try again beside it, not in a span under the list; Switch account becomes Change login with the user mark on the Sign in and Account lines; Connection becomes Access and says what it keeps; the tail sentence goes — the account-wide permission is stated once, in the hint before start.",
    changes: [
      ["Failure", "the failed line carries the reason and Try again · Change login; storage-setup-guide-error is gone"],
      ["Change login", "the R2 verb and mark, on the Sign in line once signed in and on the Account line"],
      ["Access", "replaces Connection: 'A refresh credential for this account · renews automatically'"],
      ["Tail", "nothing after the list; the permission sentence lives in the idle hint"],
      ["Create & connect", "the naming line's primary verb, parallel to Create & authorize"],
    ],
  },
  {
    key: "C", title: "C · Same voice",
    blurb: "B, plus the sentences are written the way the R2 lane's are, so a user who has set up both reads one product: the hint before start says what the lane does and where consent happens; the todo facts say what each step will settle; the Sign in line, once done, says what was approved and how far it reaches; the list's verb is Use & connect.",
    changes: [
      ["Idle hint", "'Creates a shared folder and keeps a renewing connection to it. Sign-in and consent happen in a browser you choose. Dropbox consent covers your whole account, not only this folder.'"],
      ["Sign in", "doing: 'Approve access in a browser on this computer · 2:41'; done: 'Approved · covers your whole Dropbox account'"],
      ["Folder", "todo: 'A new shared folder · a proposed name you can change'"],
      ["Access", "'Refresh credential · renews automatically · your whole account'; ready chip 'account-wide · renews'"],
      ["Use & connect", "the folder row's verb, parallel to Use & authorize"],
    ],
  },
];

const PARAMS = new URLSearchParams(location.search);

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [scene, setScene] = useState<Scene>(() => (SCENES.some(([key]) => key === PARAMS.get("scene")) ? PARAMS.get("scene") as Scene : "browser"));
  const only = PARAMS.get("only") as Level | null;
  const shown = only ? DIRECTIONS.filter((d) => d.key === only) : DIRECTIONS;
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        {!only && (
          <header className="mockup-head">
            <div>
              <h1>Dropbox setup · one lane with R2</h1>
              <p>
                The Dropbox lane beside the shipped R2 lane's grammar, at three depths: the same controls, the
                same skeleton, the same voice. The scene knob puts every panel in the same state so they compare.
              </p>
            </div>
            <button type="button" className="btn btn-ghost" onClick={() => { const next = !dark; setDark(next); document.documentElement.setAttribute("data-theme", next ? "dark" : "light"); }}>
              <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
            </button>
          </header>
        )}
        {!only && (
          <div className="mock-controls">
            <div className="mock-toggle">
              <span className="mock-aside">Scene</span>
              <div className="sync-segmented" role="radiogroup" aria-label="Scene">
                {SCENES.map(([key, text]) => (
                  <button key={key} type="button" role="radio" aria-checked={scene === key} className={scene === key ? "active" : undefined} onClick={() => setScene(key)}>{text}</button>
                ))}
              </div>
            </div>
          </div>
        )}
        {shown.map((dir) => (
          <section key={dir.key} className="mockup-panel" id={dir.key}>
            <h3>{dir.title}</h3>
            <p className="mock-blurb">{dir.blurb}</p>
            <div className="mockup-stage"><Card scene={scene} level={dir.key} /></div>
            {dir.changes.length > 0 && <dl className="mock-spec">{dir.changes.map(([term, detail]) => <div key={term}><dt>{term}</dt><dd>{detail}</dd></div>)}</dl>}
          </section>
        ))}
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
body { background: var(--app-shell); }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 30px; padding-top: 18px; border-top: 1px solid var(--border); }
.mockup-panel:first-of-type { margin-top: 12px; border-top: 0; }
.mockup-panel > h3 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mock-blurb { max-width: 96ch; margin: 0 0 12px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; gap: 10px 22px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-spec { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 8px 18px; margin: 10px 0 0; }
.mock-spec > div { display: grid; gap: 2px; }
.mock-spec dt { color: var(--text-3); font-size: var(--type-2xs); font-weight: var(--weight-semibold); letter-spacing: 0.6px; text-transform: uppercase; }
.mock-spec dd { margin: 0; color: var(--text-1); font-size: var(--type-xs); line-height: 1.5; }
.r2-lane > .storage-setup-guide-copy:empty { display: none; }
`;

document.documentElement.setAttribute("data-theme", PARAMS.get("theme") === "dark" ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);

import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import "../src/App.css";

/**
 * Typography specimen for the R2 setup page. The shipped lane (A1) is drawn
 * with the app's own classes in four voices — Before (as shipped) and three
 * directions — over the same frozen scenarios, so the only thing that
 * differs between panels is the type. `npm run dev`, then
 * /mockups/r2-type-mockup.html (`?theme=dark`, `?scene=start|midrun|browser|failed|ready`,
 * `?only=before|A|B|C` renders one panel).
 */

/* ——— frozen scenarios ————————————————————————————————— */

type StepStatus = "todo" | "doing" | "done" | "failed" | "stopped";
interface StepView { key: string; title: string; status: StepStatus; fact?: ReactNode; controls?: ReactNode }
type Scene = "start" | "midrun" | "browser" | "failed" | "ready";
const SCENES: [Scene, string][] = [["start", "Checking"], ["midrun", "Naming the bucket"], ["browser", "Browser sign-in"], ["failed", "Wrangler missing"], ["ready", "Ready for review"]];

const ACCOUNT = { name: "He Qu", id: "023e105f4ecef8ad9ca31a8372d0c353" };
const shortId = (id: string) => `${id.slice(0, 8)}…${id.slice(-8)}`;
const KEY_ID = "4a2b8c1d0e9f7a6b5c4d3e2f1a0b91ef";
const SECRET = "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0";
const BUCKET = "mallard-fox";

const small = (label: ReactNode, tone: "plain" | "primary" | "ghost" = "plain", inert = false) => (
  <button type="button" className={`btn storage-setup-guide-button${tone === "primary" ? " btn-primary" : tone === "ghost" ? " btn-ghost" : ""}`} disabled={inert}>{label}</button>
);

function Segments() {
  return (
    <div className="sync-segmented r2-segmented" role="radiogroup" aria-label="Jurisdiction">
      {["Default", "EU", "US", "FedRAMP"].map((option, index) => (
        <button key={option} type="button" role="radio" aria-checked={index === 0} className={index === 0 ? "active" : undefined}>{option}</button>
      ))}
    </div>
  );
}

const nameField = (
  <span className="r2-name-field">
    <input className="r2-name-input" defaultValue={BUCKET} aria-label="Bucket name" spellCheck={false} />
    {small(<><Icon name="refresh" size={12} /> Suggest another</>)}
  </span>
);

const browserFact = (budget: string) => <>Sign in to Cloudflare in a browser on this computer · <span className="r2-countdown">{budget}</span></>;
const browserControls = (
  <>
    {small(<><Icon name="copy" size={12} /> Copy login link</>, "primary")}
    {small(<><Icon name="external-link" size={12} /> Open browser</>)}
    {small("Cancel", "ghost")}
  </>
);

/* The shipped fact strings, per state. */
const TODO = {
  signIn: "Uses the Wrangler session; asks for a browser sign-in if there is none",
  account: "Confirmed before anything is created or listed",
  bucket: "A new bucket · a proposed name you can change",
  key: "Object Read & Write · this bucket only",
  review: "Test connection, then Finish setup",
};

function stepsFor(scene: Scene): StepView[] {
  const todo = (key: string, title: string, fact: ReactNode): StepView => ({ key, title, status: "todo", fact });
  switch (scene) {
    case "start":
      return [
        { key: "wrangler", title: "Wrangler", status: "doing", fact: "Checking the install…", controls: small("Cancel", "ghost") },
        todo("sign-in", "Sign in", TODO.signIn), todo("account", "Account", TODO.account), todo("bucket", "Bucket", TODO.bucket),
        todo("key", "Authorize key", TODO.key), todo("review", "Review", TODO.review),
      ];
    case "midrun":
      return [
        { key: "wrangler", title: "Wrangler", status: "done", fact: <>4.125 · <code>/opt/homebrew/bin/wrangler</code></> },
        { key: "sign-in", title: "Sign in", status: "done", fact: "Wrangler session on this Mac", controls: small(<><Icon name="user" size={12} /> Change login</>) },
        { key: "account", title: "Account", status: "done", fact: <>{ACCOUNT.name} · <code>{shortId(ACCOUNT.id)}</code></> },
        { key: "bucket", title: "Bucket", status: "doing", fact: <>{nameField}<span>· proposed by Mallard</span></>, controls: <><Segments />{small("Create & authorize", "primary")}</> },
        todo("key", "Authorize key", TODO.key), todo("review", "Review", TODO.review),
      ];
    case "browser":
      return [
        { key: "wrangler", title: "Wrangler", status: "done", fact: <>4.125 · <code>/opt/homebrew/bin/wrangler</code></> },
        { key: "sign-in", title: "Sign in", status: "doing", fact: browserFact("2:41"), controls: browserControls },
        todo("account", "Account", TODO.account), todo("bucket", "Bucket", TODO.bucket),
        todo("key", "Authorize key", TODO.key), todo("review", "Review", TODO.review),
      ];
    case "failed":
      return [
        { key: "wrangler", title: "Wrangler", status: "failed", fact: <>Not on PATH · <code>npm install --global wrangler@latest</code></>, controls: <>{small("Copy command")}{small("Copy agent message")}{small("Try again", "primary")}</> },
        todo("sign-in", "Sign in", TODO.signIn), todo("account", "Account", TODO.account), todo("bucket", "Bucket", TODO.bucket),
        todo("key", "Authorize key", TODO.key), todo("review", "Review", TODO.review),
      ];
    case "ready":
      return [];
  }
}

/* ——— card pieces drawn with the app's classes ————————————————— */

function Step({ step, index }: { step: StepView; index: number }) {
  const mark = step.status === "done" ? <Icon name="check-circle" size={12} />
    : step.status === "doing" ? <span className="status-loader" />
    : step.status === "failed" ? <Icon name="alert-triangle" size={12} />
    : step.status === "stopped" ? <Icon name="ban" size={12} />
    : <span className="r2-step-dot" />;
  return (
    <li className="r2-step" data-status={step.status}>
      <span className="r2-step-mark" data-n={index + 1}>{mark}</span>
      <div className="r2-step-body">
        <span className="r2-step-title">{step.title}</span>
        {step.fact && <span className="r2-step-fact">{step.fact}</span>}
        {step.controls && <span className="r2-step-controls">{step.controls}</span>}
      </div>
    </li>
  );
}

const chip = (label: string, value: string) => <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>;

function CardHeader({ name }: { name: string }) {
  return (
    <header>
      <span className="sync-storage-icon"><Icon name="cloud" size={18} /></span>
      <input value={name} readOnly aria-label="Storage name" />
      <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
        {["Cloudflare R2", "Local folder", "Dropbox"].map((label, index) => (
          <button key={label} type="button" role="radio" aria-checked={index === 0} className={index === 0 ? "active" : undefined}>{label}</button>
        ))}
      </div>
    </header>
  );
}

function Doors() {
  const door = (icon: IconName, checked: boolean, title: string, detail: string) => (
    <button type="button" className="storage-mode-door" role="radio" aria-checked={checked}>
      <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
      <span className="storage-mode-door-detail">{detail}</span>
    </button>
  );
  return (
    <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing storage">
      {door("plus", true, "Create new", "A new bucket in your Cloudflare account, created with Wrangler or by hand.")}
      {door("search", false, "Use existing", "A bucket Mallard already uses, found in your account — or a connection you paste in.")}
    </div>
  );
}

function ManualRow() {
  return (
    <div className="storage-setup-route-row" role="note">
      <Icon name="info" size={13} />
      <span className="storage-setup-route-label">Manual setup</span>
      <span className="storage-setup-guide-copy">Enter an existing R2 connection in the fields below.</span>
      <button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>
      <button type="button" className="storage-manual-toggle" aria-label="Hide fields"><Icon name="chevron-up" size={12} /></button>
    </div>
  );
}

function Field({ label, value, placeholder, wide, secret }: { label: string; value?: string; placeholder?: string; wide?: boolean; secret?: boolean }) {
  return (
    <label className={wide ? "storage-provider-url-field" : undefined}>
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

function Fields({ filled }: { filled: boolean }) {
  return (
    <div className="sync-storage-fields storage-provider-fields" aria-label="Cloudflare R2 connection details">
      <Field label="S3 API URL" wide value={filled ? `https://${ACCOUNT.id}.r2.cloudflarestorage.com/${BUCKET}` : undefined} placeholder="Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent" />
      <Field label="Bucket name" value={filled ? BUCKET : undefined} placeholder="Example: codex-sync-backups" />
      <Field label="Account ID" value={filled ? ACCOUNT.id : undefined} placeholder="Example: 023e105f4ecef8ad9ca31a8372d0c353" />
      <Field label="Access Key ID" value={filled ? KEY_ID : undefined} placeholder="Example: 4a2b…91ef" />
      <Field label="Secret Access Key" secret value={filled ? SECRET : undefined} placeholder="Example: a1B2…x9Y0" />
    </div>
  );
}

function Card({ scene }: { scene: Scene }) {
  const ready = scene === "ready";
  const steps = stepsFor(scene);
  const resetVerb = (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" disabled={scene === "start"}>
      <Icon name="refresh" size={12} /> Reset setup
    </button>
  );
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <article className="sync-storage-card">
          <CardHeader name={ready ? BUCKET : "R2 storage 1"} />
          {!ready && <Doors />}
          <div className="storage-setup-route-row agent r2-lane" role="note">
            <Icon name="terminal" size={13} />
            <span className="storage-setup-route-label">Wrangler setup</span>
            {ready ? (
              <>
                <span className="r2-ready" role="status">
                  <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
                  <span className="r2-facts">{chip("Bucket", BUCKET)}{chip("Jurisdiction", "Default")}{chip("Account", ACCOUNT.name)}{chip("Key", "this bucket only")}</span>
                </span>
                {resetVerb}
              </>
            ) : (
              <>
                <span className="storage-setup-guide-copy" />
                {resetVerb}
                <ol className="r2-steps">{steps.map((step, index) => <Step key={step.key} step={step} index={index} />)}</ol>
              </>
            )}
          </div>
          {!ready && <ManualRow />}
          <Fields filled={ready} />
        </article>
      </div>
      <footer className="sync-storage-setup-footer">
        <span className="sync-storage-editor-progress" aria-live="polite">{ready ? "5 of 5 set" : "0 of 5 set"}</span>
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

/* ——— the directions ————————————————————————————————— */

type Dir = "before" | "A" | "B" | "C";
interface Direction { key: Dir; title: string; blurb: string; spec: [string, string][] }

const DIRECTIONS: Direction[] = [
  {
    key: "before", title: "Before · as it was",
    blurb: "Every word in the lane is 11px; titles and facts differ only by weight, section labels are 9px capitals, and the form's placeholders below are the largest type on the page.",
    spec: [["Section label", "9px caps · tracked 1px · text-3"], ["Step title", "11px · semibold · text-1"], ["Step fact", "11px · text-2 · code 10px"], ["Field name", "9px caps · text-2"], ["Lane hint", "11px · text-2"]],
  },
  {
    key: "A", title: "A · One size up, same voice",
    blurb: "Nothing changes shape. The lane moves from 11 to 12px, titles to 13px in the darkest ink, section labels and field names from 9 to 10px, rows get two more pixels of air. The cheapest fix: the page reads as it does now, only legibly.",
    spec: [["Section label", "10px caps · tracked 0.8px · text-2"], ["Step title", "13px · semibold · text-0 (todo: text-3, medium)"], ["Step fact", "12px · text-2 · code 11px"], ["Field name", "10px caps · text-2"], ["Lane hint", "12px · text-2 · rows 20px line, 8px pad"]],
  },
  {
    key: "B", title: "B · Titles in the display face — shipped",
    blurb: "Section labels and step titles take Baloo 2 — the face the doors, buttons and headings already speak in — in sentence case, and the facts stay in the text face. Title and fact are now two voices, not two weights, so the eye lands on the step names first. Field names drop the capitals for the same reason.",
    spec: [["Section label", "Baloo 2 · 14px · bold · text-0 · sentence case"], ["Step title", "Baloo 2 · 14px · bold · text-0 (todo: text-3, semibold)"], ["Step fact", "12px text face · text-2 · code 11px"], ["Field name", "12px · semibold · text-1 · sentence case"], ["Lane hint", "12px · text-2 · rows 20px line, 8px pad"]],
  },
  {
    key: "C", title: "C · Numbered, two lines",
    blurb: "Each step is a numbered row with its title on the first line and its fact under it, so 'which step' and 'what is happening' never share a line. The controls sit at the title's end. Steps not yet reached show their number in place of the empty dot, done ones keep the check. Tallest of the three; clearest reading order.",
    spec: [["Section label", "10px caps · tracked 0.8px · text-2"], ["Step number", "Baloo 2 · 10px · bold in an 18px ring (todo only)"], ["Step title", "13px · semibold · text-0, own line"], ["Step fact", "12px · text-2, second line · code 11px"], ["Field name", "11px · semibold · text-1 · sentence case"]],
  },
];

/* Everything a cutover would lift into App.css for the chosen direction.
   Every other class on this page is the app's own. */
const DIRECTION_CSS = `
/* A and C were drawn over the old base; each restates the face and case B moved. */
.type-dir[data-type="A"] .storage-setup-route-label, .type-dir[data-type="C"] .storage-setup-route-label { font-family: inherit; font-weight: var(--weight-semibold); text-transform: uppercase; }
.type-dir[data-type="A"] .r2-step-title, .type-dir[data-type="C"] .r2-step-title { font-family: inherit; font-weight: var(--weight-semibold); }
.type-dir[data-type="A"] .sync-storage-card .sync-field-name { font-weight: var(--weight-medium); text-transform: uppercase; }
/* ——— A ——— */
.type-dir[data-type="A"] .storage-setup-route-row { font-size: var(--type-sm); }
.type-dir[data-type="A"] .storage-setup-route-label { color: var(--text-2); font-size: var(--type-2xs); letter-spacing: 0.8px; }
.type-dir[data-type="A"] .r2-step { padding: 8px 0; font-size: var(--type-sm); }
.type-dir[data-type="A"] .r2-step-mark { height: 20px; }
.type-dir[data-type="A"] .r2-step-body { line-height: 20px; }
.type-dir[data-type="A"] .r2-step-title { color: var(--text-0); font-size: var(--type-md); }
.type-dir[data-type="A"] .r2-step[data-status="todo"] .r2-step-title { color: var(--text-3); }
.type-dir[data-type="A"] .r2-step-fact code { font-size: var(--type-xs); }
.type-dir[data-type="A"] .r2-name-input, .type-dir[data-type="A"] .r2-countdown { font-size: var(--type-xs); }
.type-dir[data-type="A"] .sync-storage-card .sync-field-name { font-size: var(--type-2xs); letter-spacing: 0.8px; }

/* ——— Before: the rules as they stood before B shipped (2026-09-11) ——— */
.type-dir[data-type="before"] .storage-setup-route-row { font-size: var(--type-xs); }
.type-dir[data-type="before"] .storage-setup-route-label { color: var(--text-3); font-family: inherit; font-size: var(--type-3xs); font-weight: var(--weight-semibold); letter-spacing: 1px; text-transform: uppercase; }
.type-dir[data-type="before"] .r2-step { padding: 6px 0; font-size: var(--type-xs); }
.type-dir[data-type="before"] .r2-step-mark { height: 18px; }
.type-dir[data-type="before"] .r2-step-body { line-height: 18px; }
.type-dir[data-type="before"] .r2-step-title { color: var(--text-1); font-family: inherit; font-size: inherit; font-weight: var(--weight-semibold); }
.type-dir[data-type="before"] .r2-step[data-status="todo"] .r2-step-title { color: var(--text-3); font-weight: var(--weight-medium); }
.type-dir[data-type="before"] .r2-step-fact code { font-size: var(--type-2xs); }
.type-dir[data-type="before"] .r2-name-input, .type-dir[data-type="before"] .r2-countdown { font-size: var(--type-2xs); }
.type-dir[data-type="before"] .sync-storage-card .sync-field-name { color: var(--text-2); font-size: var(--type-3xs); font-weight: var(--weight-medium); letter-spacing: 1px; text-transform: uppercase; }

/* ——— B: SHIPPED into App.css — the base rules now are B, nothing to add ——— */
/* ——— C ——— */
.type-dir[data-type="C"] .storage-setup-route-row { font-size: var(--type-sm); }
.type-dir[data-type="C"] .storage-setup-route-label { color: var(--text-2); font-size: var(--type-2xs); letter-spacing: 0.8px; }
.type-dir[data-type="C"] .r2-step { grid-template-columns: 20px minmax(0, 1fr); column-gap: 10px; padding: 9px 0; font-size: var(--type-sm); }
.type-dir[data-type="C"] .r2-step-mark { width: 20px; height: 20px; }
.type-dir[data-type="C"] .r2-step[data-status="todo"] .r2-step-dot { display: none; }
.type-dir[data-type="C"] .r2-step[data-status="todo"] .r2-step-mark::before {
  content: attr(data-n);
  display: grid; place-items: center; width: 18px; height: 18px;
  border: 1.5px solid var(--border-strong); border-radius: 50%;
  color: var(--text-3); font-family: var(--font-display); font-size: var(--type-2xs); font-weight: var(--weight-bold); line-height: 1;
}
.type-dir[data-type="C"] .r2-step-body { display: grid; grid-template-columns: minmax(0, 1fr) auto; column-gap: 12px; row-gap: 4px; align-items: center; line-height: 20px; }
.type-dir[data-type="C"] .r2-step-title { grid-column: 1; color: var(--text-0); font-size: var(--type-md); }
.type-dir[data-type="C"] .r2-step[data-status="todo"] .r2-step-title { color: var(--text-3); }
.type-dir[data-type="C"] .r2-step-controls { grid-column: 2; grid-row: 1; margin-left: 0; justify-content: flex-end; }
.type-dir[data-type="C"] .r2-step-fact { grid-column: 1 / -1; grid-row: 2; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; }
.type-dir[data-type="C"] .r2-step-fact code { font-size: var(--type-xs); }
.type-dir[data-type="C"] .r2-name-input, .type-dir[data-type="C"] .r2-countdown { font-size: var(--type-xs); }
.type-dir[data-type="C"] .sync-storage-card .sync-field-name { color: var(--text-1); font-size: var(--type-xs); font-weight: var(--weight-semibold); letter-spacing: 0; text-transform: none; }
`;

/* ——— the page ———————————————————————————————————————— */

const PARAMS = new URLSearchParams(location.search);

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [scene, setScene] = useState<Scene>(() => (SCENES.some(([key]) => key === PARAMS.get("scene")) ? PARAMS.get("scene") as Scene : "midrun"));
  const only = PARAMS.get("only") as Dir | null;
  const shown = only ? DIRECTIONS.filter((d) => d.key === only) : DIRECTIONS;
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        {!only && (
          <header className="mockup-head">
            <div>
              <h1>R2 setup · type directions</h1>
              <p>
                The same lane, in four voices. Only the type changes between panels: sizes, faces, case and
                the step's line structure. The scene knob puts every panel in the same state so they compare.
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
            <div className="mockup-stage type-dir" data-type={dir.key}><Card scene={scene} /></div>
            <dl className="mock-spec">{dir.spec.map(([term, detail]) => <div key={term}><dt>{term}</dt><dd>{detail}</dd></div>)}</dl>
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
.mock-blurb { max-width: 92ch; margin: 0 0 12px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; gap: 10px 22px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-spec { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 6px 18px; margin: 10px 0 0; }
.mock-spec > div { display: grid; gap: 1px; }
.mock-spec dt { color: var(--text-3); font-size: var(--type-2xs); font-weight: var(--weight-semibold); letter-spacing: 0.6px; text-transform: uppercase; }
.mock-spec dd { margin: 0; color: var(--text-1); font-family: var(--font-mono); font-size: var(--type-2xs); line-height: 1.5; }
.r2-lane > .storage-setup-guide-copy:empty { display: none; }
`;

document.documentElement.setAttribute("data-theme", PARAMS.get("theme") === "dark" ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + DIRECTION_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);

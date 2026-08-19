import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * The machine-local binding dialog, redrawn in the setup page's language.
 *
 * What the shipped ProjectBindingEditor now does, drawn here specimen by
 * specimen:
 * - The body is the setup page's bands — "Project" and "Agents" as
 *   hairline-ruled sections with the pixel-face headings — instead of a
 *   loose stack of cards.
 * - An active binding is a fact sheet: read-only mono folder, each agent
 *   card stating its bound profile's name and path, one Done button.
 * - An unbound or unconfirmed project keeps the editor: a picker-only
 *   Choose folder button, and the setup page's profile row, class for
 *   class — profile select with a "Custom path…" option, mono path field,
 *   Browse, and the judged-as-typed hint line under it.
 * - The explanations are down to one sentence, under the Project heading.
 *   The header paragraph, the folder-field caption, the per-card captions
 *   and the footer sentence are gone.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const HOME = "/Users/hequ";

type ProviderKey = "claude" | "codex";

interface ProfileOption {
  id: string;
  name: string;
  path: string;
  unavailable?: boolean;
}

interface ProviderSpec {
  key: ProviderKey;
  name: string;
  profileLabel: string;
  childDir: string;
  options: ProfileOption[];
}

const PROVIDERS: ProviderSpec[] = [
  {
    key: "claude",
    name: "Claude",
    profileLabel: ".claude profile",
    childDir: ".claude",
    options: [
      { id: "cl-default", name: "Default Claude", path: "~/.claude" },
      { id: "cl-myconf1", name: "myconf1 · Claude", path: "~/Desktop/project/myconf1" },
      { id: "cl-old", name: "Old laptop import", path: "/Volumes/Archive/claude", unavailable: true },
    ],
  },
  {
    key: "codex",
    name: "Codex",
    profileLabel: ".codex profile",
    childDir: ".codex",
    options: [
      { id: "cx-default", name: "Default Codex", path: "~/.codex" },
      { id: "cx-myconf2", name: "myconf2 · Codex", path: "~/Desktop/project/myconf2" },
    ],
  },
];

type Selection =
  | { kind: "existing"; profileId: string }
  | { kind: "pending"; path: string }
  | null;

type DialogState = Record<ProviderKey, { enabled: boolean; selection: Selection }>;

/* ── the judged-as-typed hint, faked the way the probe answers ───────────── */

interface PathHint {
  tone: "mute" | "ok" | "warn" | "bad";
  message: string;
  path?: string;
}

function expandHome(value: string): string {
  const trimmed = value.trim();
  if (!trimmed.startsWith("~")) return trimmed;
  if (trimmed === "~") return HOME;
  if (trimmed.startsWith("~/")) return `${HOME}${trimmed.slice(1)}`;
  return trimmed;
}

/** The shipped profilePathHint's states, answered from canned data so every
 *  tone can be reached by typing: `myconf1` is already registered, `myconf2`
 *  holds a child dir, any path under /Users exists, anything else does not. */
function pathHint(provider: ProviderSpec, raw: string): PathHint {
  const expanded = expandHome(raw);
  if (!expanded) {
    return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder." };
  }
  if (!expanded.startsWith("/")) {
    return { tone: "mute", message: "Paths must be absolute — start with / or ~/." };
  }
  const registered = provider.options.find((option) => expandHome(option.path) === expanded);
  if (registered) {
    return { tone: "ok", message: `Already set up as “${registered.name}” — setup will reuse it.` };
  }
  if (expanded.endsWith(`/${provider.childDir}`)) {
    return { tone: "ok", message: `${provider.name} profile ·`, path: expanded };
  }
  if (expanded.startsWith(`${HOME}/`)) {
    return { tone: "ok", message: `Found ${provider.childDir} inside — using`, path: `${expanded}/${provider.childDir}` };
  }
  return { tone: "bad", message: "No folder at this path yet." };
}

/* ── the dialog ──────────────────────────────────────────────────────────── */

function profileRowComplete(provider: ProviderSpec, selection: Selection): boolean {
  if (!selection) return false;
  if (selection.kind === "existing") return !!selection.profileId;
  return pathHint(provider, selection.path).tone === "ok";
}

function BindingDialog({ initial, locked = false }: { initial: DialogState; locked?: boolean }) {
  const [state, setState] = useState<DialogState>(initial);

  const patch = (key: ProviderKey, next: { enabled: boolean; selection: Selection }) =>
    setState((current) => ({ ...current, [key]: next }));

  const enabledProviders = PROVIDERS.filter((provider) => state[provider.key].enabled);
  const incomplete = enabledProviders.filter(
    (provider) => !profileRowComplete(provider, state[provider.key].selection),
  );
  const errorMessage = enabledProviders.length === 0
    ? "Turn on at least one agent to continue."
    : incomplete.length > 0 && incomplete.some((provider) => {
      const selection = state[provider.key].selection;
      return !selection || (selection.kind === "pending" && !selection.path.trim());
    })
      ? `Choose a profile for ${incomplete.map((provider) => provider.name).join(", ")}.`
      : null;
  const canSave = enabledProviders.length > 0 && incomplete.length === 0;

  const profileRow = (provider: ProviderSpec) => {
    const { selection } = state[provider.key];
    const value = selection?.kind === "existing"
      ? selection.profileId
      : selection?.kind === "pending" ? "__pending" : "";
    const path = selection?.kind === "pending" ? selection.path : "";
    const hint = selection?.kind === "pending" ? pathHint(provider, path) : null;
    const rowIncomplete = incomplete.some((candidate) => candidate.key === provider.key);
    return (
      <div className={`sync-agent-profile-row${selection?.kind === "pending" ? " has-custom-path" : ""}${rowIncomplete && errorMessage ? " has-error" : ""}`}>
        <span className="sync-select-shell">
          <select
            value={value}
            aria-label={`${provider.name} profile`}
            onChange={(event) => {
              const next = event.target.value;
              patch(provider.key, {
                enabled: true,
                selection: next === "__pending"
                  ? { kind: "pending", path: "" }
                  : next ? { kind: "existing", profileId: next } : null,
              });
            }}
          >
            <option value="">Choose profile</option>
            {provider.options.map((option) => (
              <option key={option.id} value={option.id} disabled={option.unavailable}>
                {option.name}{option.unavailable ? " (unavailable)" : ""}
              </option>
            ))}
            <option value="__pending">Custom path…</option>
          </select>
        </span>
        {selection?.kind === "pending" && (
          <input
            value={path}
            aria-label={`${provider.name} custom profile path`}
            aria-invalid={hint?.tone === "bad" || undefined}
            placeholder={`${provider.name} profile path`}
            spellCheck={false}
            onChange={(event) => patch(provider.key, {
              enabled: true,
              selection: { kind: "pending", path: event.target.value },
            })}
          />
        )}
        <button type="button" className="btn">
          <Icon name="folder" size={13} /> Browse
        </button>
        {hint && (
          <p className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">
            {hint.message}
            {hint.path && <code>{hint.path}</code>}
          </p>
        )}
      </div>
    );
  };

  return (
    <dialog open className="sync-modal sync-binding-dialog" aria-labelledby="mock-binding-title">
      <header className="sync-modal-header">
        <div>
          <span className="sync-eyebrow">Machine-local binding</span>
          <h1 id="mock-binding-title">Project setup for HorrorGame (myconf2)</h1>
        </div>
        <button type="button" className="btn btn-ghost" aria-label="Close binding editor">
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body">
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div>
              <strong>Project</strong>
              <span>Only this machine changes — repo identity and cloud paths stay the same.</span>
            </div>
          </div>
          <div className="sync-field-grid">
            <label>
              <span>Folder</span>
              {locked ? (
                <input value="~/Desktop/project/demo/HorrorGame" readOnly title="/Users/hequ/Desktop/project/demo/HorrorGame" />
              ) : (
                <div className="sync-profile-select-row">
                  <input value="~/Desktop/project/demo/HorrorGame" readOnly title="/Users/hequ/Desktop/project/demo/HorrorGame" />
                  <button type="button" className="btn">Choose folder</button>
                </div>
              )}
            </label>
          </div>
          <dl className="sync-fact-grid compact">
            <div><dt>Local project</dt><dd><code>project-05cd532f890708470946bfc4</code></dd></div>
            <div><dt>Repo</dt><dd><code>31c31d5ef7bb4f5e3de797b4c7d219ed</code></dd></div>
          </dl>
        </section>
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Agents</strong></div>
          </div>
          {locked ? (
            <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
              {PROVIDERS.flatMap((provider) => {
                const { enabled, selection } = state[provider.key];
                if (!enabled || selection?.kind !== "existing") return [];
                const profile = provider.options.find((option) => option.id === selection.profileId);
                if (!profile) return [];
                return [(
                  <div key={provider.key} className="sync-provider-card enabled">
                    <div className="sync-provider-name">
                      <strong>{provider.name}</strong>
                      <span>{provider.profileLabel}</span>
                    </div>
                    <small title={expandHome(profile.path)}>
                      {profile.name} · <code>{profile.path}</code>{profile.unavailable ? " · Unavailable" : ""}
                    </small>
                  </div>
                )];
              })}
            </div>
          ) : (
            <>
              <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
                {PROVIDERS.map((provider) => {
                  const { enabled } = state[provider.key];
                  return (
                    <div key={provider.key} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
                      <label className="sync-provider-toggle">
                        <input
                          type="checkbox"
                          checked={enabled}
                          disabled={enabled && enabledProviders.length === 1}
                          aria-label={`Use ${provider.name} in this project`}
                          onChange={() => patch(provider.key, {
                            enabled: !enabled,
                            selection: enabled ? null : state[provider.key].selection,
                          })}
                        />
                        <strong>{provider.name}</strong>
                        <span>{provider.profileLabel}</span>
                      </label>
                      {enabled && (
                        <div className="sync-agent-profile-list">
                          {profileRow(provider)}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
              {errorMessage && (
                <div className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {errorMessage}</div>
              )}
            </>
          )}
        </section>
      </div>
      <footer className="sync-modal-footer">
        <div>
          {locked ? (
            <button type="button" className="btn">Done</button>
          ) : (
            <>
              <button type="button" className="btn">Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!canSave}>Save project setup</button>
            </>
          )}
        </div>
      </footer>
    </dialog>
  );
}

/* ── specimen states ─────────────────────────────────────────────────────── */

const BOUND: DialogState = {
  claude: { enabled: true, selection: { kind: "existing", profileId: "cl-myconf1" } },
  codex: { enabled: true, selection: { kind: "existing", profileId: "cx-myconf2" } },
};

const CUSTOM_PATH: DialogState = {
  claude: { enabled: true, selection: { kind: "pending", path: "~/Desktop/project/myconf2" } },
  codex: { enabled: true, selection: { kind: "existing", profileId: "cx-myconf2" } },
};

const REFUSAL: DialogState = {
  claude: { enabled: true, selection: { kind: "pending", path: "/Volumes/T7/claude-home" } },
  codex: { enabled: false, selection: null },
};

/* ── the page around them ────────────────────────────────────────────────── */

function Panel({ title, standard, note, children }: {
  title: string;
  standard: string;
  note: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p className="mockup-standard">{standard}</p>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(false);
  const toggleTheme = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
  };
  return (
    <div className="app">
      <style>{SHELL_CSS}</style>
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Project binding dialog · redesign</h1>
            <p>
              The machine-local binding dialog rebuilt in the setup page&rsquo;s language, with
              one rule deciding what it is: an <em>active</em> binding renders as a fact sheet —
              read-only folder, each agent&rsquo;s profile shown as its name and path, one Done —
              while an unbound or unconfirmed project keeps the editor, with the setup
              page&rsquo;s profile row: select, <code>Custom path…</code>, Browse, and the hint
              judged as you type. One sentence of reassurance survives, under the Project
              heading. The editor specimens are interactive: toggle agents, switch a select to
              <code> Custom path…</code>, and type to watch the hint and the save gate.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn" onClick={toggleTheme}>
              <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
            </button>
          </div>
        </header>

        <Panel
          title="The fact sheet"
          standard="An active binding — how the dialog opens on a set-up project"
          note={<>Nothing here is editable: the folder is read-only mono with the full path in
            its tooltip, and each agent card states the profile it is bound to — its registered
            name, then its path in the mono face, with an <code>· Unavailable</code> or
            <code> · Read only</code> marker when the folder has gone bad. No checkboxes, no
            selects, no Browse; the footer is one Done. The identity pair keeps its fact grid,
            inside the Project band.</>}
        >
          <BindingDialog initial={BOUND} locked />
        </Panel>

        <Panel
          title="First bind on this machine"
          standard="No active binding yet — the editor, with a typed custom path"
          note={<>The restore flow opens this same dialog to <em>choose</em> a folder and
            profiles, so an unbound project keeps the editor: the folder gains a picker-only
            Choose&nbsp;folder button, and each agent row is the setup page&rsquo;s —
            Claude&rsquo;s select on <code>Custom path…</code> with the typed folder judged as
            it is typed, answering here in the success tone with the resolved path, which opens
            the save gate.</>}
        >
          <BindingDialog initial={CUSTOM_PATH} />
        </Panel>

        <Panel
          title="The refusals"
          standard="A missing folder, and the last agent pinned on"
          note={<>The typed path points at a volume that is not there, so the hint answers in
            the danger tone and the row&rsquo;s select borders red. Codex is off — its card is
            just the unchecked toggle, no caption — which pins Claude&rsquo;s checkbox: the last
            enabled agent cannot be switched off, same rule as the setup page. Clear the path to
            see the <em>Choose a profile</em> sentence instead.</>}
        >
          <BindingDialog initial={REFUSAL} />
        </Panel>
      </div>
    </div>
  );
}

/* ── no proposed CSS: every rule this page needs shipped in App.css and
      RetroTheme.css, so the specimens render through the app's own
      stylesheets — if the dialog draws wrong here, the shipped CSS is
      wrong. ── */

const SHELL_CSS = `
/* The app is a fixed shell — html, body, #root and .app are all height-locked
   and overflow: hidden, because in Tauri only the inner panes scroll. A
   mockup is a document, so it has to hand the wheel back to the page. */
html, body, #root { height: auto; min-height: 100%; overflow: visible; }
.app { height: auto; min-height: 100vh; overflow: visible; }

/* width:100% is load-bearing — .app is a column flex container, and an auto
   cross-axis margin without it lets the shell shrink to its content. */
.mockup-shell { width: 100%; max-width: 1100px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 78ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-controls { display: inline-flex; gap: 8px; flex: 0 0 auto; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > .mockup-standard { margin: 0 0 8px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); letter-spacing: .04em; text-transform: uppercase; }
.mockup-panel > p { max-width: 84ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }

/* The stage plays backdrop: the dialog sits on the shell tint the way it
   sits on the blurred app, and the modal's own max-height is released so
   the document can scroll it in full. */
.mockup-stage {
  display: flex;
  justify-content: center;
  padding: 36px 24px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--app-shell);
}
.mockup-stage > .sync-modal { position: static; max-height: none; }
body { background: var(--app-shell); }
`;

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);

import { StrictMode, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import "../src/App.css";
import "./message-model-mockup.css";

// Visual: Mallard's warm paper, ink borders, and restrained message stamps.
// Content: short messages with model and effort. Source details stay on demand.
// Interaction: short scene reveal, quiet button feedback, native popovers.
// Standalone fixture. No Tauri commands, provider data, or persistence.

type Provider = "codex" | "claude";
type Scene = "change" | "missing" | "mixed";
type Evidence = "recorded" | "inferred" | "unknown" | "mixed";
type Model = { name: string; effort: string };
interface Message {
  id: number;
  role: "user" | "assistant";
  time: string;
  model: string | null;
  effort: string | null;
  evidence: Evidence;
  body: ReactNode;
  choices?: Model[];
}

const models: Record<Provider, [Model, Model]> = {
  codex: [{ name: "gpt-5.6-luna", effort: "Low" }, { name: "gpt-5.6-sol", effort: "High" }],
  claude: [{ name: "claude-opus-4-6", effort: "Low" }, { name: "claude-sonnet-4-6", effort: "High" }],
};
const names: Record<Provider, string> = { codex: "Codex", claude: "Claude" };

function messagesFor(provider: Provider, scene: Scene): Message[] {
  const [first, second] = models[provider];
  const inputEvidence = provider === "claude" ? "inferred" : "recorded";
  const rows: Message[] = [
    {
      id: 1, role: "user", time: "10:02:14", model: first.name, effort: first.effort,
      evidence: inputEvidence,
      body: <p>Check upload failures.</p>,
    },
    {
      id: 2, role: "assistant", time: "10:02:38", model: first.name, effort: first.effort,
      evidence: "recorded",
      body: <p>A timeout can occur after the file is stored.</p>,
    },
    {
      id: 3, role: "user", time: "10:04:06", model: second.name, effort: second.effort,
      evidence: inputEvidence,
      body: <p>Add a test for a lost response.</p>,
    },
    {
      id: 4, role: "assistant", time: "10:04:54", model: second.name, effort: second.effort,
      evidence: "recorded",
      body: <p>Added. All tests passed.</p>,
    },
  ];
  if (scene === "missing") {
    return [
      { ...rows[0], effort: null },
      { ...rows[1], effort: null },
      { ...rows[2], model: null, effort: null, evidence: "unknown", body: <p>Also check a file mismatch.</p> },
    ];
  }
  if (scene === "mixed") {
    return [
      { ...rows[0], model: null, effort: null, evidence: "mixed", choices: [first, second] },
      rows[1],
      { ...rows[3], time: "10:03:26" },
    ];
  }
  return rows;
}

function ModelStamp({ message, provider }: { message: Message; provider: Provider }) {
  const id = useId();
  const panel = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const mixed = message.evidence === "mixed";
  const model = mixed ? "Mixed" : message.model ?? "Unknown";
  const effort = message.effort ?? "Unknown";
  const source = message.evidence === "unknown" ? "Not recorded"
    : message.evidence === "mixed" || message.evidence === "inferred" ? "Linked replies"
      : provider === "codex" ? "Turn settings" : "Reply record";

  return (
    <>
      <button
        type="button"
        className={`model-stamp${!message.effort && !mixed ? " has-unknown" : ""}`}
        popoverTarget={id}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Model details for message ${message.id}: ${model}, ${mixed ? "Mixed" : effort} thinking`}
        title={message.role === "user" ? "Reply model and thinking" : "Model and thinking"}
        onClick={(event) => {
          const bounds = event.currentTarget.getBoundingClientRect();
          const width = Math.min(280, window.innerWidth - 24);
          if (panel.current) {
            panel.current.style.left = `${Math.max(12, Math.min(bounds.right - width, window.innerWidth - width - 12))}px`;
            panel.current.style.top = `${Math.max(12, Math.min(bounds.bottom + 8, window.innerHeight - 210))}px`;
          }
        }}
      >
        <span className="model-stamp-name">{model}</span>
        {!mixed && (message.model || message.effort) && <><span className="model-stamp-dot" aria-hidden="true">·</span><span className="model-stamp-effort">{effort}</span></>}
      </button>
      <div
        ref={panel} id={id} popover="auto" role="dialog" aria-labelledby={`${id}-title`}
        className="model-popover"
        onToggle={(event) => setOpen(event.currentTarget.matches(":popover-open"))}
      >
        <div className="model-popover-head">
          <strong id={`${id}-title`}>{message.role === "user" ? "Reply settings" : "Model settings"}</strong>
          <button type="button" popoverTarget={id} popoverTargetAction="hide" aria-label="Close model details"><Icon name="x" size={15} /></button>
        </div>
        {mixed ? (
          <ul className="model-choices">
            {message.choices?.map((choice) => <li key={choice.name}><code>{choice.name}</code><span>{choice.effort} thinking</span></li>)}
          </ul>
        ) : (
          <dl className="model-facts">
            <div><dt>Model</dt><dd><code>{message.model ?? "Unknown"}</code></dd></div>
            <div><dt>Thinking</dt><dd>{message.effort ?? "Unknown"}</dd></div>
          </dl>
        )}
        <div className="model-source"><span>Source</span><span>{source}</span></div>
      </div>
    </>
  );
}

function Mockup() {
  const query = new URLSearchParams(window.location.search);
  const [provider, setProvider] = useState<Provider>(query.get("provider") === "claude" ? "claude" : "codex");
  const scene: Scene = query.get("scene") === "missing" ? "missing" : query.get("scene") === "mixed" && provider === "claude" ? "mixed" : "change";
  const theme = query.get("theme") === "dark" ? "dark" : "light";
  const rows = messagesFor(provider, scene);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    const parameters = new URLSearchParams({ provider, scene, theme });
    window.history.replaceState(null, "", `?${parameters}`);
  }, [provider, scene, theme]);

  return (
    <div className="model-mock">
      <header className="model-preview-bar">
        <div className="model-preview-title"><h1>Upload failures</h1><span>Mockup</span></div>
        <div className="model-provider-picker" role="group" aria-label="Provider">
          {(["codex", "claude"] as const).map((key) => (
            <button key={key} type="button" aria-pressed={provider === key} onClick={() => setProvider(key)}>
              <ProviderMark provider={key} displayName={names[key]} size={14} />{names[key]}
            </button>
          ))}
        </div>
      </header>
      <main className="session-detail-page model-session" aria-label={`${names[provider]} session mockup`}>
        <ol className="session-detail-event-list model-message-list" key={`${provider}-${scene}`}>
          {rows.map((message) => (
            <li key={message.id}>
              <article className={`session-detail-event event-${message.role === "user" ? "prompt" : "response"}`} aria-label={`${message.role === "user" ? "Your input" : `${names[provider]} reply`}, message ${message.id}`}>
                <span className="session-detail-event-node" title={message.role === "user" ? "You" : names[provider]}>
                  {message.role === "user" ? <Icon name="user" size={12} /> : <ProviderMark provider={provider} displayName={names[provider]} size={12} />}
                </span>
                <div className="session-detail-card">
                  <div className="session-detail-card-head model-message-head">
                    <time className="model-time">{message.time}</time>
                    <ModelStamp message={message} provider={provider} />
                  </div>
                  <div className="session-detail-event-copy markdown-copy">{message.body}</div>
                </div>
              </article>
            </li>
          ))}
        </ol>
      </main>
    </div>
  );
}

document.documentElement.dataset.theme = new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light";
createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);

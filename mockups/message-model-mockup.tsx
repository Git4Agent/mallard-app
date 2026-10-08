import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import type { SessionMessageSettings, SessionTimelineEvent } from "../src/types";
import Icon from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SessionDetailTimeline from "../src/components/project-sync/session-detail/SessionDetailTimeline";
import { SessionModelChip } from "../src/components/project-sync/session-detail/SessionModel";
import "../src/App.css";
import "./message-model-mockup.css";

/*
 * Message model and thinking (2026-10-07, direction D). What the session
 * started with is a chip among the header's facts; the transcript notes a
 * change of model or thinking level before the message it applies from. The
 * chip and the notes are the shipped SessionModelChip and SessionModelNote on
 * the shipped SessionDetailTimeline.
 *   ?provider=codex|claude   ?scene=change|missing|two   ?theme=light|dark
 * The provider and scene switches belong to the mockup. Static sample data;
 * nothing calls Tauri.
 */

type Provider = "codex" | "claude";
type Scene = "change" | "missing" | "two";

const MODELS: Record<Provider, [string, string]> = {
  codex: ["gpt-5.6-luna", "gpt-5.6-sol"],
  claude: ["claude-opus-4-6", "claude-sonnet-4-6"],
};
const NAMES: Record<Provider, string> = { codex: "Codex", claude: "Claude" };
const SCENES: Record<Scene, string> = { change: "Model change", missing: "Missing fields", two: "Two replies, two levels" };
const START = Math.floor(new Date(2026, 9, 7, 10, 2).getTime() / 1000);

function sample(provider: Provider, scene: Scene): { start: SessionMessageSettings | null; events: SessionTimelineEvent[] } {
  const [first, second] = MODELS[provider];
  const input: SessionMessageSettings["source"] = provider === "claude" ? "linked" : "turn";
  const reply: SessionMessageSettings["source"] = provider === "claude" ? "reply" : "turn";
  const at = (model: string | null, effort: string | null, source: SessionMessageSettings["source"]): SessionMessageSettings =>
    ({ model, effort, source });
  const prompt = (id: number, seconds: number, text: string, settings: SessionMessageSettings, calls: number): SessionTimelineEvent => ({
    event_id: `p${id}`, ordinal: id, timestamp: START + seconds, kind: "prompt", quality: "exact", turn_id: `t${id}`, text, settings,
    turn_tool_summary: { turn_id: `t${id}`, tool_calls: { value: calls, quality: "exact" } },
  });
  const response = (id: number, seconds: number, text: string, settings: SessionMessageSettings, turn: number): SessionTimelineEvent => ({
    event_id: `r${id}`, ordinal: id, timestamp: START + seconds, kind: "response", quality: "exact", turn_id: `t${turn}`, text, settings,
  });
  if (scene === "missing") {
    // Recorded from the second turn on: the chip says Unknown, the first note says where it began.
    return {
      start: at(null, null, "none"),
      events: [
        prompt(1, 0, "Check upload failures.", at(null, null, "none"), 2),
        response(2, 24, "A timeout can occur after the file is stored.", at(null, null, "none"), 1),
        prompt(3, 120, "Add a test for a lost response.", at(second, null, input), 1),
        response(4, 168, "Added. All tests passed.", at(second, null, reply), 3),
      ],
    };
  }
  if (scene === "two") {
    return {
      start: at(first, "low", input),
      events: [
        prompt(1, 0, "Check upload failures.", at(first, "low", input), 2),
        response(2, 24, "A timeout can occur after the file is stored.", at(first, "low", reply), 1),
        prompt(3, 120, "Add a test for a lost response.", at(second, "low", input), 3),
        response(4, 150, "Added.", at(second, "low", reply), 3),
        response(5, 192, "All tests passed.", at(second, "xhigh", reply), 3),
      ],
    };
  }
  return {
    start: at(first, "low", input),
    events: [
      prompt(1, 0, "Check upload failures.", at(first, "low", input), 2),
      response(2, 24, "A timeout can occur after the file is stored.", at(first, "low", reply), 1),
      prompt(3, 120, "Add a test for a lost response.", at(second, "high", input), 1),
      response(4, 168, "Added. All tests passed.", at(second, "high", reply), 3),
    ],
  };
}

function Picker<T extends string>({ label, value, options, onChange, mark }: {
  label: string;
  value: T;
  options: Record<T, string>;
  onChange: (next: T) => void;
  mark?: (key: T) => React.ReactNode;
}) {
  return (
    <div className="model-picker" role="group" aria-label={label}>
      {(Object.keys(options) as T[]).map((key) => (
        <button key={key} type="button" aria-pressed={value === key} onClick={() => onChange(key)}>
          {mark?.(key)}{options[key]}
        </button>
      ))}
    </div>
  );
}

function Mockup() {
  const query = new URLSearchParams(window.location.search);
  const [provider, setProvider] = useState<Provider>(query.get("provider") === "claude" ? "claude" : "codex");
  const [scene, setScene] = useState<Scene>(query.get("scene") === "missing" ? "missing" : query.get("scene") === "two" ? "two" : "change");
  const theme = query.get("theme") === "dark" ? "dark" : "light";
  const { start, events } = sample(provider, scene);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    window.history.replaceState(null, "", `?${new URLSearchParams({ provider, scene, theme })}`);
  }, [provider, scene, theme]);

  return (
    <div className="model-mock">
      <header className="model-preview-bar">
        <div className="model-preview-title"><h1>Upload failures</h1><span>Mockup</span></div>
        <Picker label="Provider" value={provider} options={NAMES} onChange={setProvider} mark={(key) => <ProviderMark provider={key} displayName={NAMES[key]} size={14} />} />
        <Picker label="Scene" value={scene} options={SCENES} onChange={setScene} />
      </header>
      <main className="session-detail-page model-session" aria-label={`${NAMES[provider]} session mockup`}>
        {/* The header's facts, as SessionDetailHeader draws them, with the model chip among them. */}
        <div className="session-detail-header">
          <div className="session-detail-strip">
            <div className="session-detail-chips" aria-label="Session facts">
              <span className="session-detail-chip"><Icon name="git-branch" size={12} />main</span>
              <span className="session-detail-chip"><Icon name="clock" size={12} />Oct 7, 2026 · 10:02</span>
              <span className="session-detail-chip"><code>4.8M</code> tokens</span>
              {start && <SessionModelChip settings={start} />}
            </div>
          </div>
        </div>
        <SessionDetailTimeline
          key={`${provider}-${scene}`}
          events={events}
          provider={provider}
          providerDisplayName={NAMES[provider]}
          loading={false}
          loadingMore={false}
          nextCursor={null}
          error={null}
          expandedToolIds={new Set()}
          toolDetails={{}}
          onRetry={() => undefined}
          onLoadMore={() => undefined}
          onToolExpandedChange={() => undefined}
          onOpenProjectPath={() => undefined}
          projectMedia={{}}
          onLoadProjectMedia={() => undefined}
          projectRoot="/Users/example/project"
          hasFilterToggles
          startSettings={start}
        />
      </main>
    </div>
  );
}

document.documentElement.dataset.theme = new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light";
createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);

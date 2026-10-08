import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import SessionDetailHeader from "../src/components/project-sync/session-detail/SessionDetailHeader";
import type { SessionPushGate } from "../src/components/project-sync/session-detail/sessionDetailModel";
import type { ProjectSessionDetail, SessionCount, SessionDetailWarning } from "../src/types";
import "../src/App.css";

/**
 * Live check: the shipped `SessionDetailHeader` with its Push-hold chip and
 * its transcript-notes chip, mounted client-side so the notes panel opens
 * through `PathBarPopover`'s own off-bar branch (TopLayer, window placement,
 * outside click, Escape, scroll). docs/PLAN_SESSION_DETAIL_NOTICES.md.
 *
 * `npm run dev`, then /mockups/session-notices-live.html
 * (`?theme=light|dark&width=990|782|580`).
 */

const START = 1_790_881_800;
const exact = (value: number): SessionCount => ({ value, quality: "exact" });

const base: ProjectSessionDetail = {
  resource_id: "claude:conversation:session-1",
  provider: "claude",
  provider_display_name: "Claude",
  provider_session_id: "session-1",
  title: "Run the e2e suites without xlarge",
  summary: null,
  started_at: START,
  ended_at: START + 10_080,
  is_active: false,
  branch: "HEAD",
  recorded_sha: null,
  tokens: exact(4_800_000),
  event_counts: {
    prompts: exact(9),
    responses: exact(12),
    intermediate_steps: exact(41),
    checkpoints: exact(0),
    tool_calls: { value: 31, quality: "lower_bound" },
    file_edits: exact(3),
    shell_calls: exact(20),
    reads: exact(6),
    other_tools: exact(2),
    agent_calls: exact(0),
  },
  actions: [
    { action_id: "claude.open_in_app", kind: "open_in_app", label: "Open in Claude", enabled: true },
    { action_id: "claude.open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
  ],
  snapshot_id: "snapshot-1",
  warnings: [],
};

const INCOMPLETE: SessionDetailWarning = {
  code: "incomplete_tool_call",
  severity: "info",
  message: "Tool calls marked incomplete never recorded a result. They were most likely interrupted.",
};
const FOUR: SessionDetailWarning[] = [
  INCOMPLETE,
  { code: "message_preview_truncated", severity: "warning", message: "Messages over 1 MiB are cut short here. The session file keeps the full text." },
  { code: "message_phase_inferred", severity: "info", message: "Some replies had no end marker, so Mallard guessed which are final replies and which are steps." },
  { code: "malformed_record", severity: "warning", message: "Some lines of the session file couldn't be read and are left out." },
];

interface Frame {
  id: string;
  title: string;
  detail: ProjectSessionDetail;
  gate: SessionPushGate | null;
  notes: SessionDetailWarning[];
}

const FRAMES: Frame[] = [
  { id: "screenshot", title: "Up to date, one interrupted call", detail: base, gate: null, notes: [INCOMPLETE] },
  {
    id: "pull",
    title: "Pull first, four notes",
    detail: base,
    gate: { tone: "info", label: "Pull first", detail: "Storage has changes. Review Pull before pushing this session." },
    notes: FOUR,
  },
  {
    id: "agent",
    title: "Agent’s page, comparison failed",
    detail: {
      ...base,
      agent: {
        parent_resource_id: "claude:conversation:session-0",
        parent_title: "Plan the release",
        preview: {
          label: "Map the e2e suites",
          role: "explorer",
          model: "claude-sonnet-5-5",
          mode: "async",
          status: "completed",
          brief_truncated: false,
          report_truncated: false,
        },
      },
    },
    gate: { tone: "danger", label: "Can't compare", detail: "Mallard couldn't read Mallard Cloud team-space: the request timed out." },
    notes: FOUR.slice(0, 2),
  },
];

function FrameView({ frame }: { frame: Frame }) {
  const [toolCalls, setToolCalls] = useState(false);
  return (
    <section className="session-detail-page" data-frame={frame.id} data-tool-calls={toolCalls ? "on" : "off"}>
      <SessionDetailHeader
        source={{ kind: "local" }}
        detail={frame.detail}
        back={frame.id === "agent"
          ? { voice: "name", label: "Plan the release", onActivate: () => undefined }
          : { voice: "app", label: "Git history", onActivate: () => undefined }}
        headingRef={{ current: null }}
        actionBusyId={null}
        onAction={() => undefined}
        pushGate={frame.gate}
        notes={frame.notes}
        onShowToolCalls={toolCalls ? null : () => setToolCalls(true)}
      />
      <p className="live-readout">Tool calls {toolCalls ? "shown" : "hidden"}</p>
    </section>
  );
}

const query = new URLSearchParams(window.location.search);
document.documentElement.dataset.theme = query.get("theme") === "dark" ? "dark" : "light";
const width = Number(query.get("width")) || 990;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <div className="live" style={{ width }}>
      {FRAMES.map((frame) => (
        <div key={frame.id} className="live-frame">
          <h2>{frame.title}</h2>
          <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
            <FrameView frame={frame} />
          </main>
        </div>
      ))}
    </div>
  </StrictMode>,
);

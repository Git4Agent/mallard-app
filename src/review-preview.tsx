import { createRoot } from "react-dom/client";
import "./App.css";
import "./RetroTheme.css";
import PushResourceWorkspace from "./components/project-sync/PushResourceWorkspace";
import type { ProjectResourceDescriptor } from "./types";

const resources: ProjectResourceDescriptor[] = Array.from({ length: 4 }, (_, index) => ({
  resource_id: `codex:conversation:${index + 1}`,
  kind: "codex_conversation",
  provider: "codex",
  scope: "provider_state",
  display_name: `Conversation ${index + 1}`,
  provenance: { kind: "conversation" },
  apply_policy: "safe_file",
  codec_version: 1,
  metadata: {},
  category: "conversations",
  logical_paths: [`sessions/${index + 1}.jsonl`],
  default_selected: true,
  blocked_reason: null,
}));
const selected = new Set(resources.map((resource) => resource.resource_id));

document.documentElement.dataset.theme = "light";

createRoot(document.getElementById("root")!).render(
  <div className="sync-app">
    <main className="sync-main sync-sync-review-page">
      <section style={{ width: "100%", padding: "32px clamp(24px, 5vw, 72px)" }}>
        <PushResourceWorkspace
          resources={resources}
          selected={selected}
          projectDefaults={selected}
          busy={false}
          error={null}
          storageName="Local storage 1"
          destinationStatus={{
            state: "requires_pull",
            reason: "remote_advanced",
            remote_generation: 5,
            base_generation: 4,
          }}
          threadComparison={{
            project_id: "project-1",
            storage_id: "storage-1",
            storage_name: "Local storage 1",
            compared_at: 1,
            entries: [],
            counts: {
              synced: 0,
              local: 0,
              storage: 5,
              diverged: 0,
              unavailable: 0,
              unknown: 0,
            },
            warnings: [],
            remote_continuity: {
              state: "advanced",
              base_generation: 4,
              remote_generation: 5,
            },
          }}
          initialStep="review"
          onToggle={() => undefined}
          onUseProjectDefaults={() => undefined}
          onClear={() => undefined}
          onClose={() => undefined}
          onPush={() => undefined}
          onPull={() => undefined}
        />
      </section>
    </main>
  </div>,
);

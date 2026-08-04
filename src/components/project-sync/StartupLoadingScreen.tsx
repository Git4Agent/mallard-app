import type { CSSProperties } from "react";

export type StartupStage =
  | "installation"
  | "settings"
  | "repair"
  | "projects"
  | "providers"
  | "workspace";

const STARTUP_STAGES: ReadonlyArray<{
  id: StartupStage;
  status: string;
  progress: number;
}> = [
  { id: "installation", status: "Checking installation…", progress: 12 },
  { id: "settings", status: "Loading settings…", progress: 28 },
  // Recovery rescans provider history and can take a while after an
  // interrupted sync, so it says so instead of stalling a later stage.
  { id: "repair", status: "Finishing interrupted work…", progress: 46 },
  { id: "projects", status: "Finding projects…", progress: 64 },
  { id: "providers", status: "Checking profiles…", progress: 80 },
  { id: "workspace", status: "Opening workspace…", progress: 96 },
];

interface Props {
  stage: StartupStage;
}

export default function StartupLoadingScreen({ stage }: Props) {
  const stageIndex = Math.max(
    0,
    STARTUP_STAGES.findIndex((candidate) => candidate.id === stage),
  );
  const currentStage = STARTUP_STAGES[stageIndex];
  const progressStyle = {
    "--sync-startup-progress": `${currentStage.progress}%`,
  } as CSSProperties;

  return (
    <main
      className="sync-startup-screen"
      aria-labelledby="sync-startup-title"
      aria-describedby="sync-startup-status"
    >
      <div className="sync-startup-drag-region" data-tauri-drag-region />

      <section className="sync-startup-content">
        <div className="sync-startup-brand">
          <div className="sync-startup-mark" aria-hidden="true">
            <img src="/mallard-logo-duck.svg" alt="" />
          </div>
          <h1 id="sync-startup-title">Mallard</h1>
        </div>

        <div className="sync-startup-progress">
          <div
            className="sync-startup-progress-track"
            role="progressbar"
            aria-label="Mallard startup progress"
            aria-valuemin={1}
            aria-valuemax={STARTUP_STAGES.length}
            aria-valuenow={stageIndex + 1}
            aria-valuetext={currentStage.status}
            style={progressStyle}
          >
            <span />
          </div>
          <p id="sync-startup-status" role="status" aria-live="polite" aria-atomic="true">
            {currentStage.status}
          </p>
        </div>
      </section>
    </main>
  );
}

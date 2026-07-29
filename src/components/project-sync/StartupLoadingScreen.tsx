import type { CSSProperties } from "react";

export type StartupStage =
  | "installation"
  | "settings"
  | "projects"
  | "providers"
  | "workspace";

const STARTUP_STAGES: ReadonlyArray<{
  id: StartupStage;
  status: string;
  progress: number;
}> = [
  { id: "installation", status: "Checking installation…", progress: 14 },
  { id: "settings", status: "Loading settings…", progress: 34 },
  { id: "projects", status: "Finding projects…", progress: 56 },
  { id: "providers", status: "Checking profiles…", progress: 78 },
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
            <img src="/mallard-logo.svg" alt="" />
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

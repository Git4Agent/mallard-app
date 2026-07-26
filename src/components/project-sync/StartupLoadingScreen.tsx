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
    "--v3-startup-progress": `${currentStage.progress}%`,
  } as CSSProperties;

  return (
    <main
      className="v3-startup-screen"
      aria-labelledby="v3-startup-title"
      aria-describedby="v3-startup-status"
    >
      <div className="v3-startup-drag-region" data-tauri-drag-region />

      <section className="v3-startup-content">
        <div className="v3-startup-brand">
          <div className="v3-startup-mark" aria-hidden="true">
            <img src="/mallard-logo.svg" alt="" />
          </div>
          <h1 id="v3-startup-title">Mallard</h1>
        </div>

        <div className="v3-startup-progress">
          <div
            className="v3-startup-progress-track"
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
          <p id="v3-startup-status" role="status" aria-live="polite" aria-atomic="true">
            {currentStage.status}
          </p>
        </div>
      </section>
    </main>
  );
}

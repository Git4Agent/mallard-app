import type { CSSProperties } from "react";
import { useTranslation } from "../../i18n";

export type StartupStage =
  | "installation"
  | "settings"
  | "repair"
  | "projects"
  | "providers"
  | "workspace";

/* Each stage's status line is `shell:startup.<id>`. Recovery rescans
   provider history and can take a while after an interrupted sync, so it
   says so instead of stalling a later stage. */
const STARTUP_STAGES: ReadonlyArray<{
  id: StartupStage;
  progress: number;
}> = [
  { id: "installation", progress: 12 },
  { id: "settings", progress: 28 },
  { id: "repair", progress: 46 },
  { id: "projects", progress: 64 },
  { id: "providers", progress: 80 },
  { id: "workspace", progress: 96 },
];

interface Props {
  stage: StartupStage;
}

export default function StartupLoadingScreen({ stage }: Props) {
  const { t } = useTranslation("shell");
  const stageIndex = Math.max(
    0,
    STARTUP_STAGES.findIndex((candidate) => candidate.id === stage),
  );
  const currentStage = STARTUP_STAGES[stageIndex];
  const status = t(`startup.${currentStage.id}`);
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
            aria-label={t("startup.progress")}
            aria-valuemin={1}
            aria-valuemax={STARTUP_STAGES.length}
            aria-valuenow={stageIndex + 1}
            aria-valuetext={status}
            style={progressStyle}
          >
            <span />
          </div>
          <p id="sync-startup-status" role="status" aria-live="polite" aria-atomic="true">
            {status}
          </p>
        </div>
      </section>
    </main>
  );
}

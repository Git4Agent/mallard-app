import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import Icon from "../Icons";

export type SyncReviewStep = "history" | "skills" | "plugins" | "project_files" | "review";

export type SyncReviewTabCounts = Record<SyncReviewStep, number>;

export function syncReviewSteps(includeProjectFiles: boolean): SyncReviewStep[] {
  return includeProjectFiles
    ? ["history", "skills", "plugins", "project_files", "review"]
    : ["history", "skills", "plugins", "review"];
}

interface Props {
  activeStep: SyncReviewStep;
  counts: SyncReviewTabCounts;
  steps?: readonly SyncReviewStep[];
  warningSteps?: ReadonlySet<SyncReviewStep>;
  disabled?: boolean;
  onChange: (step: SyncReviewStep) => void;
}

const STEP_DEFINITIONS: Record<SyncReviewStep, {
  id: SyncReviewStep;
  label: string;
  icon: "git-branch" | "folder" | "link" | "file" | "check-circle";
}> = {
  history: { id: "history", label: "Git & sessions", icon: "git-branch" },
  skills: { id: "skills", label: "Skills", icon: "folder" },
  plugins: { id: "plugins", label: "Plugins", icon: "link" },
  project_files: { id: "project_files", label: "Project files", icon: "file" },
  review: { id: "review", label: "Review", icon: "check-circle" },
};

export default function SyncReviewTabs({
  activeStep,
  counts,
  steps = syncReviewSteps(false),
  warningSteps = new Set(),
  disabled = false,
  onChange,
}: Props) {
  const selectFromKeyboard = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    step: SyncReviewStep,
  ) => {
    event.preventDefault();
    onChange(step);
    window.requestAnimationFrame(() => {
      event.currentTarget.parentElement
        ?.querySelector<HTMLButtonElement>(`[data-sync-review-tab="${step}"]`)
        ?.focus();
    });
  };
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const currentStep = event.currentTarget.dataset.syncReviewTab as SyncReviewStep;
    const currentIndex = steps.indexOf(currentStep);
    if (currentIndex < 0) return;
    if (event.key === "Home") return selectFromKeyboard(event, steps[0]);
    if (event.key === "End") return selectFromKeyboard(event, steps[steps.length - 1]);
    if (event.key === "ArrowLeft") {
      return selectFromKeyboard(event, steps[(currentIndex - 1 + steps.length) % steps.length]);
    }
    if (event.key === "ArrowRight") {
      selectFromKeyboard(event, steps[(currentIndex + 1) % steps.length]);
    }
  };

  return (
    <div className="sync-project-tabs sync-sync-review-tabs" role="tablist" aria-label="Sync review steps">
      {steps.map((stepId) => {
        const step = STEP_DEFINITIONS[stepId];
        const active = activeStep === stepId;
        const warning = warningSteps.has(stepId);
        const count = counts[stepId];
        return (
          <button
            key={step.id}
            type="button"
            id={`sync-review-${step.id}-tab`}
            data-sync-review-tab={step.id}
            role="tab"
            aria-selected={active}
            aria-controls={`sync-review-${step.id}-panel`}
            aria-label={`${step.label}${step.id === "review" ? "" : `, ${count} selected`}${warning ? ", needs attention" : ""}`}
            tabIndex={active ? 0 : -1}
            className={`${active ? "active" : ""}${warning ? " warning" : ""}`}
            disabled={disabled}
            onClick={() => onChange(step.id)}
            onKeyDown={handleKeyDown}
          >
            <Icon name={step.icon} size={14} />
            <span>{step.label}</span>
            {step.id !== "review" && <small>{count}</small>}
            {warning && <span className="sync-sync-review-warning-dot" aria-hidden="true" />}
          </button>
        );
      })}
    </div>
  );
}

import type { KeyboardEvent } from "react";
import Icon from "../Icons";

export type ProjectView = "sessions" | "files";

/** The panel both faces render into, which every tab of the switch controls. */
export const PROJECT_VIEW_PANEL_ID = "project-history-panel";

export const projectViewTabId = (view: ProjectView) => `project-view-${view}-tab`;

/** The two faces of a non-Git project, in the order the switch shows them. */
const PROJECT_VIEWS = [
  { id: "sessions", label: "Sessions", icon: "message" },
  { id: "files", label: "Files", icon: "file" },
] as const;

/**
 * The Sessions/Files switch of a non-Git project: the app's segmented pill,
 * hugging its two tabs at the start of the page's search row, as a real
 * tablist with the arrow-key contract. Each face renders it through its own
 * `viewSwitch` slot, so choosing the other face replaces the pill along with
 * the page — focus that was on it follows to the new pill by id.
 */
export default function ProjectViewSwitch({
  view,
  onChange,
}: {
  view: ProjectView;
  onChange: (view: ProjectView) => void;
}) {
  const select = (next: ProjectView, keepFocus: boolean) => {
    onChange(next);
    if (!keepFocus) return;
    window.requestAnimationFrame(() => {
      document.getElementById(projectViewTabId(next))?.focus();
    });
  };
  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const index = PROJECT_VIEWS.findIndex((candidate) => candidate.id === view);
    const next = event.key === "ArrowRight" || event.key === "ArrowDown"
      ? PROJECT_VIEWS[(index + 1) % PROJECT_VIEWS.length]
      : event.key === "ArrowLeft" || event.key === "ArrowUp"
        ? PROJECT_VIEWS[(index - 1 + PROJECT_VIEWS.length) % PROJECT_VIEWS.length]
        : event.key === "Home"
          ? PROJECT_VIEWS[0]
          : event.key === "End"
            ? PROJECT_VIEWS[PROJECT_VIEWS.length - 1]
            : null;
    if (!next) return;
    event.preventDefault();
    select(next.id, true);
  };

  return (
    <div className="sync-segmented sync-project-view-tabs" role="tablist" aria-label="Project view">
      {PROJECT_VIEWS.map((candidate) => {
        const active = view === candidate.id;
        return (
          <button
            key={candidate.id}
            type="button"
            id={projectViewTabId(candidate.id)}
            role="tab"
            aria-selected={active}
            aria-controls={PROJECT_VIEW_PANEL_ID}
            tabIndex={active ? 0 : -1}
            className={active ? "active" : undefined}
            onClick={(event) => select(candidate.id, event.currentTarget === document.activeElement)}
            onKeyDown={handleKeyDown}
          >
            <Icon name={candidate.icon} size={13} />
            <span>{candidate.label}</span>
          </button>
        );
      })}
    </div>
  );
}

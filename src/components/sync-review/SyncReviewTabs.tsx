import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import Icon, { type IconName } from "../Icons";

/**
 * One resource-neutral review tab. Features supply their own vocabulary —
 * the strip owns only presentation, counts, warning dots, and the
 * arrow/Home/End keyboard contract.
 */
export interface SyncReviewTabDefinition {
  id: string;
  label: string;
  icon: IconName;
  count: number;
  warning?: boolean;
}

interface Props {
  tabs: readonly SyncReviewTabDefinition[];
  activeId: string;
  disabled?: boolean;
  onChange: (id: string) => void;
}

export default function SyncReviewTabs({
  tabs,
  activeId,
  disabled = false,
  onChange,
}: Props) {
  // A single tab is not a choice; the strip only renders when there are tabs
  // to switch between.
  if (tabs.length < 2) return null;

  const selectFromKeyboard = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    id: string,
  ) => {
    // Capture the strip synchronously. React clears `currentTarget` after the
    // handler, but the animation-frame focus move needs a stable element.
    const strip = event.currentTarget.parentElement;
    event.preventDefault();
    onChange(id);
    window.requestAnimationFrame(() => {
      strip
        ?.querySelector<HTMLButtonElement>(`[data-sync-review-tab="${id}"]`)
        ?.focus();
    });
  };
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const currentId = event.currentTarget.dataset.syncReviewTab;
    const currentIndex = tabs.findIndex((tab) => tab.id === currentId);
    if (currentIndex < 0) return;
    if (event.key === "Home") return selectFromKeyboard(event, tabs[0].id);
    if (event.key === "End") return selectFromKeyboard(event, tabs[tabs.length - 1].id);
    if (event.key === "ArrowLeft") {
      return selectFromKeyboard(event, tabs[(currentIndex - 1 + tabs.length) % tabs.length].id);
    }
    if (event.key === "ArrowRight") {
      selectFromKeyboard(event, tabs[(currentIndex + 1) % tabs.length].id);
    }
  };

  return (
    <div className="sync-project-tabs sync-sync-review-tabs" role="tablist" aria-label="Sync review steps">
      {tabs.map((tab) => {
        const active = activeId === tab.id;
        const warning = tab.warning ?? false;
        return (
          <button
            key={tab.id}
            type="button"
            id={`sync-review-${tab.id}-tab`}
            data-sync-review-tab={tab.id}
            role="tab"
            aria-selected={active}
            aria-controls={`sync-review-${tab.id}-panel`}
            aria-label={`${tab.label}, ${tab.count} selected${warning ? ", needs attention" : ""}`}
            tabIndex={active ? 0 : -1}
            className={`${active ? "active" : ""}${warning ? " warning" : ""}`}
            disabled={disabled}
            onClick={() => onChange(tab.id)}
            onKeyDown={handleKeyDown}
          >
            <Icon name={tab.icon} size={14} />
            <span>{tab.label}</span>
            <small>{tab.count}</small>
            {warning && <span className="sync-sync-review-warning-dot" aria-hidden="true" />}
          </button>
        );
      })}
    </div>
  );
}

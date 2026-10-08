import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { useTranslation } from "../../i18n";

/**
 * One resource-neutral scope: a part of a list that a status sets apart.
 * Features supply their own vocabulary and marks — the bar owns only the
 * strip, the counts, and the radio group's keyboard contract.
 */
export interface StatusScopeDefinition<Id extends string> {
  id: Id;
  label: string;
  /** How many rows of the list fall in this scope. */
  count: number;
  /** The glyph those rows wear. Decorative: the label names the scope. */
  mark: ReactNode;
}

interface Props<Id extends string> {
  /** Names the group: "Show sessions by status". */
  label: string;
  /** What the count counts, in the count's number: "session", "sessions";
   *  in Chinese it opens on its measure word, 个会话. */
  noun: (count: number) => string;
  /** Every row of the list: the count on All. */
  total: number;
  scopes: readonly StatusScopeDefinition<Id>[];
  /** The scope in force; null is All. */
  active: Id | null;
  onChange: (scope: Id | null) => void;
}

/**
 * A scope bar: the list as a whole, or one of its parts. One scope is in
 * force at a time and All is the way back, so there is nothing to clear.
 */
export default function StatusScopeBar<Id extends string>({
  label, noun, total, scopes, active, onChange,
}: Props<Id>) {
  const { t } = useTranslation("syncReview");
  const ids: readonly (Id | null)[] = [null, ...scopes.map((scope) => scope.id)];

  /* A radio group moves its selection with focus. */
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
      : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1
        : 0;
    const index = event.key === "Home" ? 0
      : event.key === "End" ? ids.length - 1
        : step === 0 ? -1
          : (ids.indexOf(active) + step + ids.length) % ids.length;
    if (index < 0) return;
    // Capture the strip synchronously. React clears `currentTarget` after the
    // handler, but the animation-frame focus move needs a stable element.
    const strip = event.currentTarget.parentElement;
    event.preventDefault();
    onChange(ids[index]);
    window.requestAnimationFrame(() => {
      strip?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus();
    });
  };

  const radio = (id: Id | null, name: string, count: number, mark: ReactNode) => {
    const checked = active === id;
    return (
      <button
        key={id ?? "all"}
        type="button"
        role="radio"
        aria-checked={checked}
        aria-label={t("scopes.option", { name, count, noun: noun(count) })}
        tabIndex={checked ? 0 : -1}
        className={checked ? "active" : undefined}
        onClick={() => onChange(id)}
        onKeyDown={handleKeyDown}
      >
        {mark}
        <span>{name}</span>
        <small>{count}</small>
      </button>
    );
  };

  return (
    <div className="sync-project-tabs sync-status-scopes" role="radiogroup" aria-label={label}>
      {radio(null, t("scopes.all"), total, null)}
      {scopes.map((scope) => radio(scope.id, scope.label, scope.count, scope.mark))}
    </div>
  );
}

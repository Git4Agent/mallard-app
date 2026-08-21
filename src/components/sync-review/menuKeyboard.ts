/**
 * The keyboard half of the `role="menu"` contract, shared by every popup that
 * declares one. Declaring the role without this is a promise the widget does
 * not keep: a screen reader switches to menu navigation and the arrow keys
 * then do nothing (or scroll the page).
 *
 * Lives in `sync-review/` because that is the resource-neutral layer both
 * feature modules already import; it touches no DTO and no Tauri command.
 */

const MENU_OPTION_SELECTOR = '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]';

export function menuOptions(root: HTMLElement): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>(MENU_OPTION_SELECTOR));
}

/**
 * Moves focus for ArrowUp/ArrowDown/Home/End, wrapping at both ends. Returns
 * true when the key was consumed, so the caller can decide what its own
 * remaining keys mean.
 */
export function moveMenuFocus(root: HTMLElement, key: string): boolean {
  const options = menuOptions(root);
  if (options.length === 0) return false;
  const index = options.indexOf(document.activeElement as HTMLButtonElement);
  let next: number | null = null;
  if (key === "ArrowDown") next = index < 0 ? 0 : (index + 1) % options.length;
  if (key === "ArrowUp") next = index < 0 ? options.length - 1 : (index - 1 + options.length) % options.length;
  if (key === "Home") next = 0;
  if (key === "End") next = options.length - 1;
  if (next === null) return false;
  options[next]?.focus();
  return true;
}

/** Focus the first or last option once the menu has been painted. */
export function focusMenuEdge(root: HTMLElement | null, position: "first" | "last" = "first") {
  if (!root) return;
  const options = menuOptions(root);
  if (options.length === 0) return;
  (position === "first" ? options[0] : options[options.length - 1])?.focus();
}

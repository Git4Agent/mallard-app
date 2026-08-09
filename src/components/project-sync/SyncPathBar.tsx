import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import Icon, { type IconName } from "../Icons";

/**
 * The workspace header is one row: a trail that says where you are, and the
 * actions that operate on it.
 *
 *   Local storage 1 ⌄ / HorrorGame ⌄ / Git history / <session title>
 *
 * Storage leads because it is the outermost scope — the thing Push and Pull
 * act on. The last crumb is where you are, so it is stated rather than
 * offered; drilling into a session appends a crumb and turns the section
 * behind it into the way back.
 *
 * The bar owns no domain state. Crumbs, menus and the action buttons are
 * handed in, so every disabled-reason and busy-label still lives with the
 * code that knows it.
 */

export interface PathBarMenuItem {
  id: string;
  label: string;
  /** Second line — a path or a qualifier. Never a sentence. */
  detail?: string;
  icon?: IconName;
  /** Renders as a radio item; the trail always has exactly one active. */
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  onSelect: () => void;
}

export interface PathBarMenuGroup {
  label?: string;
  items: PathBarMenuItem[];
}

export interface PathBarCrumb {
  id: string;
  label: string;
  icon?: IconName;
  /** `root` is the outermost scope; `section` speaks in the app's own voice. */
  tone?: "root" | "section";
  warning?: boolean;
  title?: string;
  ariaLabel?: string;
  menu?: PathBarMenuGroup[];
  menuLabel?: string;
  /** A crumb that navigates instead of opening a menu — the way back. */
  onActivate?: () => void;
  disabled?: boolean;
}

interface Props {
  crumbs: PathBarCrumb[];
  overflow?: PathBarMenuGroup[];
  overflowLabel?: string;
  overflowDisabled?: boolean;
  /** Pull and Push: rendered by the caller, which owns their state. */
  actions?: ReactNode;
  /**
   * Theme toggle and app menu. The bar doubles as the window titlebar in the
   * project workspace, so it carries the window's own controls at the far end.
   */
  windowActions?: ReactNode;
  ariaLabel?: string;
}

function hasItems(groups: PathBarMenuGroup[] | undefined): groups is PathBarMenuGroup[] {
  return !!groups?.some((group) => group.items.length > 0);
}

function PathBarMenu({
  id,
  groups,
  label,
  align,
  open,
  registerRef,
  onClose,
}: {
  id: string;
  groups: PathBarMenuGroup[];
  label: string;
  align: "start" | "end";
  open: boolean;
  registerRef: (node: HTMLDivElement | null) => void;
  onClose: () => void;
}) {
  return (
    <div
      ref={registerRef}
      id={id}
      className={`sync-path-menu${align === "end" ? " end" : ""}`}
      role="menu"
      aria-label={label}
      hidden={!open}
      onKeyDown={(event) => {
        const options = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(
          '[role="menuitem"]:not(:disabled), [role="menuitemradio"]:not(:disabled)',
        ));
        if (options.length === 0) return;
        const index = options.indexOf(document.activeElement as HTMLButtonElement);
        let next: number | null = null;
        if (event.key === "ArrowDown") next = index < 0 ? 0 : (index + 1) % options.length;
        if (event.key === "ArrowUp") next = index < 0 ? options.length - 1 : (index - 1 + options.length) % options.length;
        if (event.key === "Home") next = 0;
        if (event.key === "End") next = options.length - 1;
        if (next === null) return;
        event.preventDefault();
        options[next]?.focus();
      }}
    >
      {groups.filter((group) => group.items.length > 0).map((group, groupIndex) => (
        <div key={group.label ?? `group-${groupIndex}`} className="sync-path-menu-group">
          {groupIndex > 0 && <div className="sync-path-menu-separator" role="separator" />}
          {group.label && <div className="sync-path-menu-label">{group.label}</div>}
          {group.items.map((item) => (
            <button
              key={item.id}
              type="button"
              role={item.checked === undefined ? "menuitem" : "menuitemradio"}
              aria-checked={item.checked === undefined ? undefined : item.checked}
              className={`sync-path-menu-item${item.danger ? " danger" : ""}`}
              disabled={item.disabled}
              title={item.title}
              onClick={() => {
                onClose();
                item.onSelect();
              }}
            >
              {item.icon && <Icon name={item.icon} size={14} />}
              {item.detail ? (
                <span className="sync-path-menu-copy">
                  <span>{item.label}</span>
                  <small>{item.detail}</small>
                </span>
              ) : (
                <span className="sync-path-menu-copy">
                  <span>{item.label}</span>
                </span>
              )}
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

export default function SyncPathBar({
  crumbs,
  overflow,
  overflowLabel = "More actions",
  overflowDisabled,
  actions,
  windowActions,
  ariaLabel = "Workspace path",
}: Props) {
  const baseId = useId().replace(/[^a-zA-Z0-9_-]/g, "-");
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const menuRefs = useRef(new Map<string, HTMLDivElement | null>());

  // A crumb that stops carrying a menu — the section, once you drill past it —
  // must not leave its menu pinned open behind the new leaf.
  const openStillExists = openMenu === "overflow"
    ? hasItems(overflow)
    : crumbs.some((crumb) => crumb.id === openMenu && hasItems(crumb.menu));
  useEffect(() => {
    if (openMenu && !openStillExists) setOpenMenu(null);
  }, [openMenu, openStillExists]);

  useEffect(() => {
    if (!openMenu) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && barRef.current?.contains(target)) return;
      setOpenMenu(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setOpenMenu(null);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [openMenu]);

  const focusMenuOption = (key: string, position: "first" | "last") => {
    window.requestAnimationFrame(() => {
      const options = Array.from(menuRefs.current.get(key)?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not(:disabled), [role="menuitemradio"]:not(:disabled)',
      ) ?? []);
      if (options.length === 0) return;
      if (position === "first") options[0]?.focus();
      else options[options.length - 1]?.focus();
    });
  };

  const registerMenuRef = (key: string) => (node: HTMLDivElement | null) => {
    if (node) menuRefs.current.set(key, node);
    else menuRefs.current.delete(key);
  };

  const overflowOpen = openMenu === "overflow";
  const overflowMenuId = `${baseId}-overflow`;

  return (
    // The bar is the window's drag handle here; its children are buttons and
    // do not carry the attribute, so they stay clickable.
    <header ref={barRef} className="sync-pathbar sync-titlebar" data-tauri-drag-region>
      <ol className="sync-path" aria-label={ariaLabel} data-tauri-drag-region>
        {crumbs.map((crumb, index) => {
          const leaf = index === crumbs.length - 1;
          const menu = hasItems(crumb.menu) ? crumb.menu : null;
          const interactive = !!menu || !!crumb.onActivate;
          const open = openMenu === crumb.id;
          const menuId = `${baseId}-${crumb.id}`;
          const className = [
            "sync-path-crumb",
            crumb.tone === "root" ? "root" : null,
            crumb.tone === "section" ? "section" : null,
            leaf ? "leaf" : null,
            crumb.warning ? "warning" : null,
          ].filter(Boolean).join(" ");

          return (
            <li key={crumb.id} className={leaf ? "sync-path-item leaf-item" : "sync-path-item"} data-tauri-drag-region>
              {index > 0 && <span className="sync-path-separator" aria-hidden="true" data-tauri-drag-region>/</span>}
              {/* An ancestor that can do both goes to its page on the label and
                  opens its switcher on the chevron — one crumb, two targets,
                  which is how a breadcrumb ancestor is expected to behave. */}
              {interactive && menu && crumb.onActivate ? (
                <span className={`${className} split`}>
                  <button
                    type="button"
                    className="sync-path-crumb-link"
                    disabled={crumb.disabled}
                    title={crumb.title}
                    aria-label={crumb.ariaLabel}
                    onClick={() => {
                      setOpenMenu(null);
                      crumb.onActivate?.();
                    }}
                  >
                    {crumb.icon && <Icon name={crumb.icon} size={14} />}
                    <span>{crumb.label}</span>
                  </button>
                  <button
                    type="button"
                    className="sync-path-crumb-toggle"
                    disabled={crumb.disabled}
                    title={crumb.menuLabel ?? "Switch"}
                    aria-label={crumb.menuLabel ?? "Switch"}
                    aria-haspopup="menu"
                    aria-expanded={open}
                    aria-controls={open ? menuId : undefined}
                    onClick={() => setOpenMenu((current) => (current === crumb.id ? null : crumb.id))}
                    onKeyDown={(event) => {
                      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                      event.preventDefault();
                      setOpenMenu(crumb.id);
                      focusMenuOption(crumb.id, event.key === "ArrowDown" ? "first" : "last");
                    }}
                  >
                    <Icon name="chevron-down" size={11} className="sync-path-chevron" />
                  </button>
                </span>
              ) : interactive ? (
                <button
                  type="button"
                  className={className}
                  disabled={crumb.disabled}
                  title={crumb.title}
                  aria-label={crumb.ariaLabel}
                  aria-haspopup={menu ? "menu" : undefined}
                  aria-expanded={menu ? open : undefined}
                  aria-controls={menu && open ? menuId : undefined}
                  onClick={() => {
                    if (!menu) {
                      setOpenMenu(null);
                      crumb.onActivate?.();
                      return;
                    }
                    setOpenMenu((current) => (current === crumb.id ? null : crumb.id));
                  }}
                  onKeyDown={(event) => {
                    if (!menu) return;
                    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                    event.preventDefault();
                    setOpenMenu(crumb.id);
                    focusMenuOption(crumb.id, event.key === "ArrowDown" ? "first" : "last");
                  }}
                >
                  {crumb.icon && <Icon name={crumb.icon} size={14} />}
                  <span>{crumb.label}</span>
                  {menu && <Icon name="chevron-down" size={11} className="sync-path-chevron" />}
                </button>
              ) : (
                <span
                  className={className}
                  title={crumb.title}
                  aria-label={crumb.ariaLabel}
                  aria-current={leaf ? "page" : undefined}
                  data-tauri-drag-region
                >
                  {crumb.icon && <Icon name={crumb.icon} size={14} />}
                  <span>{crumb.label}</span>
                </span>
              )}
              {menu && (
                <PathBarMenu
                  id={menuId}
                  groups={menu}
                  label={crumb.menuLabel ?? crumb.label}
                  align="start"
                  open={open}
                  registerRef={registerMenuRef(crumb.id)}
                  onClose={() => setOpenMenu(null)}
                />
              )}
            </li>
          );
        })}
      </ol>

      <div className="sync-path-actions" data-tauri-drag-region>
        {hasItems(overflow) && (
          <div className={`sync-path-overflow${overflowOpen ? " open" : ""}`}>
            <button
              type="button"
              className="sync-path-more"
              disabled={overflowDisabled}
              title={overflowLabel}
              aria-label={overflowLabel}
              aria-haspopup="menu"
              aria-expanded={overflowOpen}
              aria-controls={overflowOpen ? overflowMenuId : undefined}
              onClick={() => setOpenMenu((current) => (current === "overflow" ? null : "overflow"))}
              onKeyDown={(event) => {
                if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                event.preventDefault();
                setOpenMenu("overflow");
                focusMenuOption("overflow", event.key === "ArrowDown" ? "first" : "last");
              }}
            >
              <Icon name="more" size={16} />
            </button>
            <PathBarMenu
              id={overflowMenuId}
              groups={overflow}
              label={overflowLabel}
              align="end"
              open={overflowOpen}
              registerRef={registerMenuRef("overflow")}
              onClose={() => setOpenMenu(null)}
            />
          </div>
        )}
        {actions && (
          <>
            {hasItems(overflow) && <span className="sync-path-divider" aria-hidden="true" data-tauri-drag-region />}
            {actions}
          </>
        )}
        {windowActions && (
          <>
            <span className="sync-path-divider" aria-hidden="true" data-tauri-drag-region />
            {windowActions}
          </>
        )}
      </div>
    </header>
  );
}

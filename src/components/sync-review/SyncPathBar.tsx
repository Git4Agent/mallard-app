import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import Icon, { type IconName } from "../Icons";
import { menuOptions, moveMenuFocus } from "./menuKeyboard";

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
 * Each crumb's menu speaks for exactly one scope: a header block naming the
 * object the menu acts on, that object's verbs beneath it, a Switch-to list,
 * and the ways to get more. The current object is the header, not a checked
 * row — position says "this is the one you're on" — so no radio state exists
 * anywhere in the bar.
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
  /** Paths keep the mono face; any other detail reads as prose. */
  detailIsPath?: boolean;
  icon?: IconName;
  /** Renders as a checkable item with a trailing tick — state, not selection. */
  checked?: boolean;
  danger?: boolean;
  /**
   * Inert rather than removed: rendered with `aria-disabled` so the row stays
   * focusable and its reason reachable by keyboard, and the menu never
   * changes shape for a cause the user cannot see.
   */
  disabled?: boolean;
  /** Why the item is inert right now — rendered as its second line. */
  disabledReason?: string;
  title?: string;
  onSelect: () => void;
}

/** The object a crumb's menu acts on: context at the top, not a command. */
export interface PathBarMenuHeader {
  label: string;
  detail?: string;
  detailIsPath?: boolean;
  icon?: IconName;
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
  /** Names the object the menu's verbs act on; drawn above the first group. */
  menuHeader?: PathBarMenuHeader;
  /**
   * One sentence explaining why the whole menu is inert — said once, first,
   * instead of a tooltip repeated across every greyed row.
   */
  menuBanner?: string;
  menuLabel?: string;
  /** A crumb that navigates instead of opening a menu — the way back. */
  onActivate?: () => void;
  disabled?: boolean;
  /**
   * Identity rendered ahead of the label — the provider's mark on a session
   * leaf. Richer than `icon` (it may carry a fallback monogram or a presence
   * dot), and supplied by the caller so the bar stays domain-free.
   */
  leading?: ReactNode;
  /**
   * A control that narrows this crumb rather than descending from it — the
   * branch a Git history is showing. It rides inside the crumb's own item with
   * no separator before it, so it reads as a qualifier on the name beside it
   * and not as another step down the path. Drilling into a session appends a
   * leaf after it and the qualifier stays where it belongs.
   */
  scope?: ReactNode;
}

interface Props {
  crumbs: PathBarCrumb[];
  overflow?: PathBarMenuGroup[];
  overflowLabel?: string;
  overflowDisabled?: boolean;
  /** Opens an existing menu in response to an action outside the path bar. */
  openMenuRequest?: { menuId: string; requestId: number };
  /** Pull and Push: rendered by the caller, which owns their state. */
  actions?: ReactNode;
  ariaLabel?: string;
}

// Inert rows stay in the roving order on purpose: their reason line is part
// of what the menu has to say, and a keyboard user has to be able to reach it.

function hasItems(groups: PathBarMenuGroup[] | undefined): groups is PathBarMenuGroup[] {
  return !!groups?.some((group) => group.items.length > 0);
}

function PathBarMenu({
  id,
  groups,
  header,
  banner,
  label,
  align,
  open,
  registerRef,
  onClose,
}: {
  id: string;
  groups: PathBarMenuGroup[];
  header?: PathBarMenuHeader;
  banner?: string;
  label: string;
  align: "start" | "end";
  open: boolean;
  registerRef: (node: HTMLDivElement | null) => void;
  /** `returnFocus` hands focus back to the button that opened the menu. */
  onClose: (returnFocus: boolean) => void;
}) {
  // Type-ahead per the menu pattern: printable characters accumulate for
  // 500ms and focus the next label with that prefix.
  const typeahead = useRef({ buffer: "", at: 0 });

  return (
    <div
      ref={registerRef}
      id={id}
      className={`sync-path-menu${align === "end" ? " end" : ""}`}
      role="menu"
      aria-label={label}
      hidden={!open}
      onKeyDown={(event) => {
        // Tab is not menu navigation: the menu steps aside and the bar's own
        // tab order continues wherever the browser takes it.
        if (event.key === "Tab") {
          onClose(false);
          return;
        }
        const options = menuOptions(event.currentTarget);
        if (options.length === 0) return;
        const index = options.indexOf(document.activeElement as HTMLButtonElement);
        // Arrows, Home and End are the shared menu contract.
        if (moveMenuFocus(event.currentTarget, event.key)) {
          event.preventDefault();
          return;
        }
        // Typeahead is this bar's own addition; its buffer lives here.
        // Space is activation, not a prefix; modifiers mean a shortcut.
        if (event.key.length !== 1 || event.key === " " || event.metaKey || event.ctrlKey || event.altKey) return;
        const now = Date.now();
        const stale = now - typeahead.current.at > 500;
        const buffer = (stale ? "" : typeahead.current.buffer) + event.key.toLowerCase();
        typeahead.current = { buffer, at: now };
        // A repeated first letter cycles matches; a longer prefix narrows
        // from the focused row, so typing "te" stays on "Team bucket".
        const from = buffer.length === 1 ? index + 1 : Math.max(index, 0);
        const labels = options.map((option) => (
          option.querySelector(".sync-path-menu-copy > span")?.textContent?.toLowerCase() ?? ""
        ));
        for (let step = 0; step < options.length; step += 1) {
          const candidate = (from + step) % options.length;
          if (labels[candidate].startsWith(buffer)) {
            event.preventDefault();
            options[candidate]?.focus();
            return;
          }
        }
      }}
    >
      {banner && (
        <div className="sync-path-menu-banner" role="status">
          <Icon name="refresh" size={14} />
          <span>{banner}</span>
        </div>
      )}
      {header && (
        <>
          <div className="sync-path-menu-current">
            {header.icon && <Icon name={header.icon} size={14} />}
            <span className="sync-path-menu-copy">
              <span>{header.label}</span>
              {header.detail && (
                <small className={header.detailIsPath ? undefined : "phrase"}>{header.detail}</small>
              )}
            </span>
          </div>
          <div className="sync-path-menu-separator" role="separator" />
        </>
      )}
      {groups.filter((group) => group.items.length > 0).map((group, groupIndex) => (
        <div key={group.label ?? `group-${groupIndex}`} className="sync-path-menu-group">
          {groupIndex > 0 && <div className="sync-path-menu-separator" role="separator" />}
          {group.label && <div className="sync-path-menu-label">{group.label}</div>}
          {group.items.map((item) => {
            const checkable = item.checked !== undefined;
            const inert = !!item.disabled;
            // A disabled row's second line is its reason; an able row's is its
            // data. Reasons are sentences-in-miniature, so they read as prose.
            const detail = (inert && item.disabledReason) ? item.disabledReason : item.detail;
            const detailIsPath = (inert && item.disabledReason) ? false : item.detailIsPath;
            return (
              <button
                key={item.id}
                type="button"
                role={checkable ? "menuitemcheckbox" : "menuitem"}
                aria-checked={checkable ? item.checked : undefined}
                className={`sync-path-menu-item${item.danger ? " danger" : ""}`}
                aria-disabled={inert || undefined}
                tabIndex={-1}
                title={item.title}
                onClick={() => {
                  if (inert) return;
                  onClose(true);
                  item.onSelect();
                }}
              >
                {item.icon && <Icon name={item.icon} size={14} />}
                <span className="sync-path-menu-copy">
                  <span>{item.label}</span>
                  {detail && (
                    <small className={detailIsPath ? undefined : "phrase"}>{detail}</small>
                  )}
                </span>
                {checkable && item.checked && (
                  <Icon name="check" size={13} className="sync-path-menu-check" />
                )}
              </button>
            );
          })}
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
  openMenuRequest,
  actions,
  ariaLabel = "Workspace path",
}: Props) {
  const baseId = useId().replace(/[^a-zA-Z0-9_-]/g, "-");
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const menuRefs = useRef(new Map<string, HTMLDivElement | null>());
  // The button each menu opened from, so Esc and activation can hand focus
  // back instead of dropping it on <body>.
  const openerRefs = useRef(new Map<string, HTMLButtonElement | null>());

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
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      const opener = openerRefs.current.get(openMenu);
      setOpenMenu(null);
      opener?.focus();
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
      const root = menuRefs.current.get(key);
      const options = root ? menuOptions(root) : [];
      if (options.length === 0) return;
      if (position === "first") options[0]?.focus();
      else options[options.length - 1]?.focus();
    });
  };

  useEffect(() => {
    if (!openMenuRequest) return;
    const exists = crumbs.some((crumb) => (
      crumb.id === openMenuRequest.menuId && hasItems(crumb.menu)
    ));
    if (!exists) return;
    setOpenMenu(openMenuRequest.menuId);
    focusMenuOption(openMenuRequest.menuId, "first");
  }, [openMenuRequest?.menuId, openMenuRequest?.requestId]);

  const registerMenuRef = (key: string) => (node: HTMLDivElement | null) => {
    if (node) menuRefs.current.set(key, node);
    else menuRefs.current.delete(key);
  };

  const registerOpenerRef = (key: string) => (node: HTMLButtonElement | null) => {
    if (node) openerRefs.current.set(key, node);
    else openerRefs.current.delete(key);
  };

  const closeMenu = (key: string) => (returnFocus: boolean) => {
    setOpenMenu(null);
    if (returnFocus) openerRefs.current.get(key)?.focus();
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
                    {crumb.leading}
                    {crumb.icon && <Icon name={crumb.icon} size={14} />}
                    <span>{crumb.label}</span>
                  </button>
                  <button
                    ref={registerOpenerRef(crumb.id)}
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
                      if (event.key === "Tab") {
                        setOpenMenu(null);
                        return;
                      }
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
                  ref={menu ? registerOpenerRef(crumb.id) : undefined}
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
                    if (event.key === "Tab") {
                      setOpenMenu(null);
                      return;
                    }
                    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                    event.preventDefault();
                    setOpenMenu(crumb.id);
                    focusMenuOption(crumb.id, event.key === "ArrowDown" ? "first" : "last");
                  }}
                >
                  {crumb.leading}
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
                  {crumb.leading}
                  {crumb.icon && <Icon name={crumb.icon} size={14} />}
                  <span>{crumb.label}</span>
                </span>
              )}
              {crumb.scope}
              {menu && (
                <PathBarMenu
                  id={menuId}
                  groups={menu}
                  header={crumb.menuHeader}
                  banner={crumb.menuBanner}
                  label={crumb.menuLabel ?? crumb.label}
                  align="start"
                  open={open}
                  registerRef={registerMenuRef(crumb.id)}
                  onClose={closeMenu(crumb.id)}
                />
              )}
            </li>
          );
        })}
      </ol>

      {/* Actions first, overflow last: the two verbs are what the bar is for,
          and the menu is where everything that is not a verb went. */}
      <div className="sync-path-actions" data-tauri-drag-region>
        {actions}
        {hasItems(overflow) && (
          <>
            {actions && <span className="sync-path-divider" aria-hidden="true" data-tauri-drag-region />}
            <div className={`sync-path-overflow${overflowOpen ? " open" : ""}`}>
              <button
                ref={registerOpenerRef("overflow")}
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
                  if (event.key === "Tab") {
                    setOpenMenu(null);
                    return;
                  }
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
                onClose={closeMenu("overflow")}
              />
            </div>
          </>
        )}
      </div>
    </header>
  );
}

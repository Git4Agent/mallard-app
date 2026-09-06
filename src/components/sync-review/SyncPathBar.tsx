import { createContext, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
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

/**
 * A crumb menu with a header is a switcher, and its groups play roles by
 * position: the first unlabelled group is the object's verbs, every labelled
 * group is a list of things to switch to, and the unlabelled groups after
 * them are the ways to get more. The switcher draws those roles in the
 * shape the rest of the app already uses — the current object as a lifted
 * row, the verbs as the row-end icon cluster in a footer strip, the ways to
 * get more as text pills beside them — so the list itself is only the things
 * you can switch to. A headerless menu (the ⋯) stays a plain list.
 */
const SWITCHER_SEARCH_FLOOR = 6;

function verbTitle(item: PathBarMenuItem): string {
  if (item.title) return item.title;
  return item.detail ? `${item.label} · ${item.detail}` : item.label;
}

export function PathBarMenu({
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
  const [query, setQuery] = useState("");
  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  const switcher = !!header;
  const populated = groups.filter((group) => group.items.length > 0);
  const verbs = switcher && populated[0] && !populated[0].label ? populated[0].items : [];
  const lists = switcher ? populated.filter((group) => !!group.label) : [];
  const tails = switcher
    ? populated.slice(verbs.length > 0 ? 1 : 0).filter((group) => !group.label).flatMap((group) => group.items)
    : [];
  const targetCount = lists.reduce((count, group) => count + group.items.length, 0);
  const searchable = switcher && targetCount >= SWITCHER_SEARCH_FLOOR;
  const needle = query.trim().toLowerCase();
  const matches = (item: PathBarMenuItem) => (
    !needle || `${item.label} ${item.detail ?? ""}`.toLowerCase().includes(needle)
  );
  const caption = verbs
    .filter((item) => item.detail)
    .map((item) => `${item.label.replace(/…$/, "")} · ${item.detail}`)
    .join(" · ");

  const renderItem = (item: PathBarMenuItem, target: boolean) => {
    const checkable = item.checked !== undefined;
    const inert = !!item.disabled;
    // A disabled row's second line is its reason; an able row's is its
    // data. Reasons are sentences-in-miniature, so they read as prose. A
    // switch target keeps its path in the tooltip and stays one line high.
    const reason = inert && item.disabledReason ? item.disabledReason : undefined;
    const detail = reason ?? (target && item.detailIsPath ? undefined : item.detail);
    const detailIsPath = reason ? false : item.detailIsPath;
    return (
      <button
        key={item.id}
        type="button"
        role={checkable ? "menuitemcheckbox" : "menuitem"}
        aria-checked={checkable ? item.checked : undefined}
        className={`sync-path-menu-item${target ? " target" : ""}${item.danger ? " danger" : ""}`}
        aria-disabled={inert || undefined}
        tabIndex={-1}
        title={item.title ?? (target && item.detailIsPath ? item.detail : undefined)}
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
  };

  const renderPill = (item: PathBarMenuItem, tail: boolean) => {
    const inert = !!item.disabled;
    return (
      <button
        key={item.id}
        type="button"
        role="menuitem"
        className={`${tail ? "sync-path-menu-tail" : "sync-path-menu-verb"}${item.danger ? " danger" : ""}`}
        aria-disabled={inert || undefined}
        aria-label={tail ? undefined : item.label}
        tabIndex={-1}
        title={inert && item.disabledReason ? item.disabledReason : verbTitle(item)}
        onClick={() => {
          if (inert) return;
          onClose(true);
          item.onSelect();
        }}
      >
        {item.icon && <Icon name={item.icon} size={14} />}
        {tail && <span>{item.label}</span>}
      </button>
    );
  };

  return (
    <div
      ref={registerRef}
      id={id}
      className={`sync-path-menu${switcher ? " switcher" : ""}${align === "end" ? " end" : ""}`}
      role="menu"
      aria-label={label}
      hidden={!open}
      onBlur={(event) => {
        // Focus that lands outside the menu closes it — a Tab that walked
        // out, a click elsewhere. A blur to nowhere is the menu hiding
        // itself, not a departure, and must not be answered.
        const next = event.relatedTarget;
        if (next instanceof Node && !event.currentTarget.contains(next)) onClose(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Tab") {
          // Inside a searchable switcher Tab walks between the filter and
          // the list. Otherwise Tab is not menu navigation: the menu steps
          // aside and the bar's own tab order continues wherever the browser
          // takes it.
          const search = event.currentTarget.querySelector<HTMLInputElement>(".sync-path-menu-search");
          const tabOptions = search ? menuOptions(event.currentTarget) : [];
          if (search && !event.shiftKey && event.target === search && tabOptions.length > 0) {
            event.preventDefault();
            tabOptions[0]?.focus();
            return;
          }
          if (search && event.shiftKey && event.target !== search) {
            event.preventDefault();
            search.focus();
            return;
          }
          onClose(false);
          return;
        }
        const options = menuOptions(event.currentTarget);
        if (options.length === 0) return;
        const typing = event.target instanceof HTMLInputElement;
        // The search field keeps its own Home/End and its characters; the
        // arrows step from it into the list, and Enter takes the first match.
        if (typing) {
          if (event.key === "Enter") {
            event.preventDefault();
            options.find((option) => option.classList.contains("target"))?.click();
            return;
          }
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
        }
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
          option.querySelector(".sync-path-menu-copy > span")?.textContent?.toLowerCase()
            ?? option.getAttribute("aria-label")?.toLowerCase()
            ?? ""
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
      {/* No live region here. The banner is prose the menu is built with — it
          is in the DOM before the menu opens and never changes while it is
          open — so it is not a status message (WCAG 2.2 SC 4.1.3). A
          `role="menu"` admits only the menuitem family. */}
      {banner && (
        <div className="sync-path-menu-banner">
          <Icon name="refresh" size={14} />
          <span>{banner}</span>
        </div>
      )}
      {header && (
        <div className="sync-path-menu-current" aria-current="true">
          {header.icon && <Icon name={header.icon} size={14} />}
          <span className="sync-path-menu-copy">
            <span>{header.label}</span>
            {header.detail && (
              <small className={header.detailIsPath ? undefined : "phrase"}>{header.detail}</small>
            )}
          </span>
          <Icon name="check" size={13} className="sync-path-menu-check" />
        </div>
      )}
      {searchable && (
        <input
          type="search"
          className="sync-path-menu-search"
          placeholder={`Switch ${label.toLowerCase()}…`}
          aria-label={`Filter ${label.toLowerCase()} list`}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      )}
      {switcher ? (
        <>
          {/* One scrollport for every target list, so a long one scrolls
              under the current row and the filter while the footer verbs stay
              in reach; the panel's ceiling is the room left below the bar. */}
          <div className="sync-path-menu-scroll">
            {lists.map((group) => {
              const shown = group.items.filter(matches);
              return (
                <div key={group.label} className="sync-path-menu-group" aria-label={group.label}>
                  {shown.map((item) => renderItem(item, true))}
                  {shown.length === 0 && <p className="sync-path-menu-empty">No match.</p>}
                </div>
              );
            })}
          </div>
          {(verbs.length > 0 || tails.length > 0) && (
            <div className="sync-path-menu-foot">
              {caption && <span className="sync-path-menu-cap">{caption}</span>}
              {verbs.length > 0 && (
                <div className="sync-path-menu-verbs">{verbs.map((item) => renderPill(item, false))}</div>
              )}
              {tails.map((item) => renderPill(item, true))}
            </div>
          )}
        </>
      ) : (
        populated.map((group, groupIndex) => (
          <div key={group.label ?? `group-${groupIndex}`} className="sync-path-menu-group">
            {groupIndex > 0 && <div className="sync-path-menu-separator" role="separator" />}
            {group.label && <div className="sync-path-menu-label">{group.label}</div>}
            {group.items.map((item) => renderItem(item, false))}
          </div>
        ))
      )}
    </div>
  );
}

/* The row under the bar. The shell's notice zone is one box for the whole
 * workspace, but the bar belongs to the page that renders it, so the shell
 * cannot put the zone between the bar and the page on its own. It hands the
 * zone down through this context and the bar renders it right after its own
 * row — where you are, then what happened — in the DOM as well as on screen,
 * so the tab sequence reads in the same order the eye does. A screen with no
 * path bar renders the zone under the shell's own titlebar instead. */
export const PathBarBelow = createContext<ReactNode>(null);

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
      if (!root) return;
      // A searchable switcher opens on its filter: with that many targets,
      // narrowing them is the first thing a keyboard user wants to do.
      const search = position === "first" ? root.querySelector<HTMLInputElement>(".sync-path-menu-search") : null;
      if (search) {
        search.focus();
        return;
      }
      const options = menuOptions(root);
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
  const below = useContext(PathBarBelow);

  return (
    <>
    {/* The bar is the window's drag handle here; its children are buttons and
        do not carry the attribute, so they stay clickable. */}
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
                // Where you are is a property of the position, not of what the
                // crumb can do: a leaf that opens a menu is still the page.
                <span className={`${className} split`} aria-current={leaf ? "page" : undefined}>
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
                  aria-current={leaf ? "page" : undefined}
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
    {below}
    </>
  );
}

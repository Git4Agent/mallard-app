import { useEffect, useId, useRef, useState } from "react";
import Icon from "../Icons";
import { PathBarMenu, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import { providerMarkIcon } from "../sync-review/ProviderMark";
import { focusMenuEdge } from "../sync-review/menuKeyboard";
import type { NewSessionSurface, ProviderActivityState, ProviderKey } from "../../types";
import { t, useTranslation } from "../../i18n";

/** The agents a new session can start with: the ones this project binds. A
 *  stored-only provider is listed by the activity too, but nothing local
 *  reads it. */
export function newSessionProviders(providers: readonly ProviderActivityState[]): ProviderActivityState[] {
  return providers.filter((provider) => provider.access === "bound_ready" || provider.access === "bound_unavailable");
}

/** One labelled group per bound agent: its desktop app, then Terminal. An
 *  agent whose profile can't be read keeps one inert row that says so once,
 *  rather than two rows repeating it. */
export function newSessionMenuGroups(
  providers: readonly ProviderActivityState[],
  onStart: (provider: ProviderKey, surface: NewSessionSurface) => void,
): PathBarMenuGroup[] {
  return providers.map((provider) => {
    const mark = providerMarkIcon(provider.provider) ?? "message";
    if (provider.access !== "bound_ready") {
      return {
        label: provider.display_name,
        items: [{
          id: `${provider.provider}-unavailable`,
          label: provider.display_name,
          icon: mark,
          disabled: true,
          disabledReason: t("projects:newSession.unreadable"),
          onSelect: () => undefined,
        }],
      };
    }
    return {
      label: provider.display_name,
      items: [
        {
          id: `${provider.provider}-app`,
          label: t("projects:newSession.app", { agent: provider.display_name }),
          icon: mark,
          onSelect: () => onStart(provider.provider, "app"),
        },
        {
          id: `${provider.provider}-terminal`,
          label: t("projects:newSession.terminal"),
          icon: "terminal" as const,
          onSelect: () => onStart(provider.provider, "terminal"),
        },
      ],
    };
  });
}

/**
 * "+ New session ▾": a menu button that starts a new session of one of the
 * project's agents in its checkout, in the agent's app or in Terminal. It
 * closes the Sessions search row, and an empty list carries it in its box.
 * The menu is the path bar's own, so rows, groups and keys match every other
 * menu in the window.
 */
export default function NewSessionMenu({
  providers,
  busy,
  primary = false,
  align = "end",
  onStart,
}: {
  providers: readonly ProviderActivityState[];
  /** A start is running: the button spins and takes no second press. */
  busy: boolean;
  primary?: boolean;
  align?: "start" | "end";
  onStart: (provider: ProviderKey, surface: NewSessionSurface) => void;
}) {
  const { t } = useTranslation("projects");
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  /** The row focus lands on when the menu opens: ArrowUp opens on the last. */
  const openEdge = useRef<"first" | "last">("first");
  const menuId = `new-session-${useId().replace(/:/g, "")}`;

  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) triggerRef.current?.focus();
  };

  useEffect(() => {
    if (busy) setOpen(false);
  }, [busy]);

  useEffect(() => {
    if (!open) return;
    const frame = window.requestAnimationFrame(() => {
      focusMenuEdge(panelRef.current, openEdge.current);
      openEdge.current = "first";
    });
    const handlePointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) setOpen(false);
    };
    const handleFocusIn = (event: FocusEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      close(true);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("focusin", handleFocusIn);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("focusin", handleFocusIn);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="sync-new-session">
      <button
        ref={triggerRef}
        type="button"
        className={primary ? "btn btn-primary" : "btn"}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        aria-label={t("newSession.label")}
        title={t("newSession.title")}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          // The menu-button contract: the arrows open the menu on its first
          // or last row.
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          openEdge.current = event.key === "ArrowDown" ? "first" : "last";
          setOpen(true);
        }}
      >
        {busy ? <Icon name="refresh" size={13} className="icon-spin" /> : <Icon name="plus" size={13} />}
        <span className="sync-new-session-label">{t("newSession.label")}</span>
        <Icon name="chevron-down" size={13} />
      </button>
      <PathBarMenu
        id={menuId}
        groups={newSessionMenuGroups(providers, onStart)}
        label={t("newSession.label")}
        align={align}
        open={open}
        registerRef={(node) => { panelRef.current = node; }}
        onClose={close}
      />
    </div>
  );
}

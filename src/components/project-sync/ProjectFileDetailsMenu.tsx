import { useEffect, useId, useRef, useState } from "react";
import Icon from "../Icons";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";

interface Props {
  relativePath: string;
  projectRoot?: string | null;
  entryTypeLabel: string;
  mode?: string | null;
  modified?: string | null;
  preservedCopyPath?: string | null;
  blockedReason?: string | null;
}

/**
 * The project-file counterpart to a session's debug-info menu. The panel is
 * portalled onto the page layer, so file metadata never expands or clips its
 * row in either the activity list or a Push/Pull review.
 */
export default function ProjectFileDetailsMenu({
  relativePath,
  projectRoot = null,
  entryTypeLabel,
  mode = null,
  modified = null,
  preservedCopyPath = null,
  blockedReason = null,
}: Props) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const menuId = `project-file-details-${useId().replace(/:/g, "")}`;
  const closeMenu = (restoreFocus = false) => {
    setOpen(false);
    setCopied(null);
    setCopyError(null);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  };
  const placement = useDebugPopoverPlacement(rootRef, panelRef, open, () => closeMenu());
  /* A dialog that closes when focus leaves it is unreachable by keyboard
     unless opening it also moves focus in. Once the panel has been placed,
     the first Copy button takes focus; Escape hands it back to the trigger. */
  const focusedForOpen = useRef(false);
  useEffect(() => {
    if (!open) {
      focusedForOpen.current = false;
      return;
    }
    if (!placement || focusedForOpen.current) return;
    focusedForOpen.current = true;
    const frame = window.requestAnimationFrame(() => {
      panelRef.current?.querySelector<HTMLElement>("button")?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [open, placement]);
  const pathSeparator = projectRoot?.includes("\\") && !projectRoot.includes("/") ? "\\" : "/";
  const absolutePath = projectRoot
    ? `${projectRoot.replace(/[\\/]+$/, "")}${pathSeparator}${relativePath.replace(/[\\/]/g, pathSeparator)}`
    : relativePath;
  const values = [
    { key: "path", label: "Path", value: absolutePath },
    { key: "type", label: "Type", value: entryTypeLabel },
    ...(mode ? [{ key: "mode", label: "Mode", value: mode }] : []),
    ...(modified ? [{ key: "modified", label: "Modified", value: modified }] : []),
    ...(preservedCopyPath ? [{ key: "preserved-copy", label: "Keeps copy", value: preservedCopyPath }] : []),
    ...(blockedReason ? [{ key: "blocked", label: "Blocked", value: blockedReason }] : []),
  ];
  const copiedValue = values.find((value) => value.key === copied);
  const copiedLabel = copied === "all"
    ? "File info copied."
    : copiedValue
      ? `${copiedValue.label} copied.`
      : "";

  const copyValue = async (key: string, value: string) => {
    try {
      await writeClipboardText(value, rootRef.current);
      setCopyError(null);
      setCopied(key);
    } catch {
      setCopied(null);
      setCopyError("Could not copy.");
    }
  };

  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null) => target instanceof Node
      && Boolean(rootRef.current?.contains(target) || panelRef.current?.contains(target));
    const onPointerDown = (event: PointerEvent) => {
      if (!inside(event.target)) closeMenu();
    };
    const onFocusIn = (event: FocusEvent) => {
      if (!inside(event.target)) closeMenu();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(null), 2_000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  return (
    <div ref={rootRef} className="sync-history-debug-menu">
      <button
        ref={triggerRef}
        type="button"
        className="btn btn-ghost sync-history-debug-trigger"
        title="File info"
        aria-label={`Show file info for ${relativePath}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => open ? closeMenu() : setOpen(true)}
      >
        <Icon name="info" size={14} />
      </button>
      <TopLayer>
        <div
          ref={panelRef}
          id={menuId}
          className="sync-history-debug-popover"
          style={placement
            ? {
              top: `${placement.top}px`,
              left: `${placement.left}px`,
              width: `${placement.width}px`,
              maxHeight: `${placement.maxHeight}px`,
            }
            : { visibility: "hidden" }}
          role="dialog"
          aria-label={`File info for ${relativePath}`}
          hidden={!open}
        >
          <header>
            <strong>File info</strong>
          </header>
          <div className="sync-history-debug-values">
            {/* No aria-label: it would replace the row's content as the
                accessible name, and the value is the one thing worth hearing.
                Named from content instead, so the path or digest is announced
                and stays navigable with a reading cursor. */}
            {values.map((value) => (
              <button
                key={value.key}
                type="button"
                className="sync-history-debug-copy"
                title={`Copy ${value.label.toLowerCase()}`}
                onClick={() => void copyValue(value.key, value.value)}
              >
                <span>{value.label}</span>
                <code>{value.value}</code>
                <small className={copied === value.key ? "copied" : undefined}>
                  {copied === value.key ? "Copied" : "Copy"}
                </small>
              </button>
            ))}
          </div>
          <footer>
            <span
              className={`sync-history-debug-status${copyError ? " error" : copied ? " success" : ""}`}
              role="status"
              aria-live="polite"
            >
              {copyError ?? copiedLabel}
            </span>
            <button
              type="button"
              className="btn btn-ghost sync-history-debug-copy-all"
              onClick={() => void copyValue(
                "all",
                values.map((value) => `${value.label}: ${value.value}`).join("\n"),
              )}
            >
              {copied === "all" ? "Copied all" : "Copy all"}
            </button>
          </footer>
        </div>
      </TopLayer>
    </div>
  );
}

import { useEffect, useId, useRef, useState } from "react";
import type { ParseKeys } from "i18next";
import { useTranslation } from "../../i18n";
import SessionRowIcon from "./SessionRowIcon";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";

type ProjectsKey = ParseKeys<"projects">;

/** One copyable field: its name, its button's title and its receipt. */
interface Field {
  key: string;
  label: ProjectsKey;
  copy: ProjectsKey;
  copied: ProjectsKey;
  value: string;
}

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
  const { t } = useTranslation(["projects", "common"]);
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
  const values: Field[] = [
    { key: "path", label: "fileDetails.path", copy: "fileDetails.copyPath", copied: "fileDetails.pathCopied", value: absolutePath },
    { key: "type", label: "fileDetails.type", copy: "fileDetails.copyType", copied: "fileDetails.typeCopied", value: entryTypeLabel },
    ...(mode ? [{ key: "mode", label: "fileDetails.mode", copy: "fileDetails.copyMode", copied: "fileDetails.modeCopied", value: mode } as const] : []),
    ...(modified ? [{ key: "modified", label: "fileDetails.modified", copy: "fileDetails.copyModified", copied: "fileDetails.modifiedCopied", value: modified } as const] : []),
    ...(preservedCopyPath ? [{ key: "preserved-copy", label: "fileDetails.keepsCopy", copy: "fileDetails.copyKeepsCopy", copied: "fileDetails.keepsCopyCopied", value: preservedCopyPath } as const] : []),
    ...(blockedReason ? [{ key: "blocked", label: "fileDetails.blocked", copy: "fileDetails.copyBlocked", copied: "fileDetails.blockedCopied", value: blockedReason } as const] : []),
  ];
  const copiedValue = values.find((value) => value.key === copied);
  const copiedLabel = copied === "all"
    ? t("fileDetails.infoCopied")
    : copiedValue
      ? t(copiedValue.copied)
      : "";

  const copyValue = async (key: string, value: string) => {
    try {
      await writeClipboardText(value, rootRef.current);
      setCopyError(null);
      setCopied(key);
    } catch {
      setCopied(null);
      setCopyError(t("fileDetails.copyFailed"));
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
        title={t("fileDetails.title")}
        aria-label={t("fileDetails.show", { path: relativePath })}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => open ? closeMenu() : setOpen(true)}
      >
        <SessionRowIcon name="info" />
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
          aria-label={t("fileDetails.dialog", { path: relativePath })}
          hidden={!open}
        >
          <header>
            <strong>{t("fileDetails.title")}</strong>
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
                title={t(value.copy)}
                onClick={() => void copyValue(value.key, value.value)}
              >
                <span>{t(value.label)}</span>
                <code>{value.value}</code>
                <small className={copied === value.key ? "copied" : undefined}>
                  {copied === value.key ? t("common:copied") : t("common:copy")}
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
                values.map((value) => t("fileDetails.line", { label: t(value.label), value: value.value })).join("\n"),
              )}
            >
              {copied === "all" ? t("copiedAll") : t("copyAll")}
            </button>
          </footer>
        </div>
      </TopLayer>
    </div>
  );
}

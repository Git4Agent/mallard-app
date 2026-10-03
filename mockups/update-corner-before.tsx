import type { ReactNode } from "react";
import type { NoticeItem } from "../src/components/notice/Notice";

/* Frozen copy of the shell notice the updater drew before the update corner
 * (AppUpdater.tsx at 8a3a80d), so the specimen's Before panel stays true
 * after the shipped code moved on. Specimen-only; nothing imports it. */

const RELEASES_PAGE_URL = "https://github.com/Git4Agent/mallard-app/releases";
function releasePageUrl(version: string): string {
  return `${RELEASES_PAGE_URL}/tag/v${version}`;
}

export interface UpdateSummary {
  version: string;
  currentVersion: string;
  notes: string;
  date: string | null;
}

export type UpdatePhase =
  | "idle"
  | "checking"
  | "latest"
  | "available"
  | "downloading"
  | "installing"
  | "error";

function releaseNotePreview(notes: string): string | null {
  const trimmed = notes.trim();
  if (!trimmed || /^https?:\/\/\S+$/.test(trimmed)) return null;
  const firstContentLine = trimmed
    .split(/\r?\n/)
    .map((line) => line.replace(/^\s*([#>*-]|\d+\.)+\s*/, "").trim())
    .find(Boolean);
  return firstContentLine || null;
}

export interface UpdateNoticeState {
  phase: UpdatePhase;
  summary: UpdateSummary | null;
  error: string | null;
  /** A restart just landed this version; say so once. */
  completed: UpdateSummary | null;
  busy: boolean;
}

export interface UpdateNoticeHandlers {
  onInstall: () => void;
  onLater: () => void;
  onRetry: () => void;
  onDismiss: () => void;
  onOpenReleaseNotes: (version: string) => void;
}

function releaseNotesButton(version: string, onOpen: (version: string) => void): ReactNode {
  return (
    <button type="button" className="btn" onClick={() => onOpen(version)} title={releasePageUrl(version)}>
      Release notes
    </button>
  );
}

/** The one shell-zone item the updater contributes, or null when it has
 * nothing to say. Pure, so the copy and the busy gate are testable. */
export function buildUpdateNotice(state: UpdateNoticeState, handlers: UpdateNoticeHandlers): NoticeItem | null {
  const { phase, summary, error, completed, busy } = state;

  if (summary && (phase === "available" || (phase === "error" && error))) {
    if (phase === "error") {
      return {
        key: "app-update",
        severity: "danger",
        title: `Mallard ${summary.version} did not install.`,
        message: error,
        action: (
          <>
            <button type="button" className="btn" onClick={handlers.onRetry}>Try again</button>
            <button type="button" className="btn" onClick={handlers.onLater}>Later</button>
          </>
        ),
      };
    }
    const preview = releaseNotePreview(summary.notes);
    return {
      key: "app-update",
      severity: "info",
      title: `Mallard ${summary.version} is available.`,
      message: busy
        ? "Finish the current operation before restarting."
        : preview ?? `You have ${summary.currentVersion}. Updating downloads the signed release and restarts Mallard.`,
      action: (
        <>
          {releaseNotesButton(summary.version, handlers.onOpenReleaseNotes)}
          <button type="button" className="btn" onClick={handlers.onLater}>Later</button>
          <button type="button" className="btn btn-primary" onClick={handlers.onInstall} disabled={busy}>
            Update and restart
          </button>
        </>
      ),
    };
  }

  if (phase === "checking") {
    return {
      key: "app-update",
      severity: "info",
      title: "Checking for updates.",
      message: "Looking for a newer signed release on GitHub.",
    };
  }

  if (phase === "latest") {
    return {
      key: "app-update",
      severity: "success",
      title: "Mallard is up to date.",
      message: "You already have the newest published version.",
      onDismiss: handlers.onDismiss,
    };
  }

  if (phase === "error" && error) {
    return {
      key: "app-update",
      severity: "danger",
      title: "Could not check for updates.",
      message: error,
      action: <button type="button" className="btn" onClick={handlers.onRetry}>Try again</button>,
      onDismiss: handlers.onDismiss,
    };
  }

  if (completed) {
    return {
      key: "app-update",
      severity: "success",
      title: `Mallard ${completed.version} installed.`,
      message: "The update finished and Mallard restarted on the new version.",
      action: releaseNotesButton(completed.version, handlers.onOpenReleaseNotes),
      onDismiss: handlers.onDismiss,
    };
  }

  return null;
}

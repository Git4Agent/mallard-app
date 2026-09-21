import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import Icon from "./Icons";
import type { NoticeItem } from "./notice/Notice";
import { openExternalUrl } from "./project-sync/externalLink";

/* In-app updates from GitHub Releases.
 *
 * The Tauri updater reads the signed `latest.json` that the release workflow
 * attaches to a PUBLISHED release (`docs/DESIGN_APP_UPDATES.md`). This module
 * owns the client side: when to ask, what one notice says, and the install.
 * It renders nothing of its own — the available update is one item in the
 * shell's single notice zone, so it obeys the one-box rule like every other
 * message — and it never restarts over a Push, Pull, or apply in flight.
 */

export const RELEASES_PAGE_URL = "https://github.com/Git4Agent/mallard-app/releases";
export const APP_UPDATE_CHECK_EVENT = "mallard:check-for-updates";

const UPDATE_DEFERRED_KEY = "mallard.update.deferred";
const PENDING_CHANGELOG_KEY = "mallard.update.pending-changelog";
const LAST_SEEN_CHANGELOG_KEY = "mallard.update.last-seen-changelog";
/** The first check waits for the startup load to settle. */
export const AUTO_CHECK_DELAY_MS = 8_000;
export const AUTO_CHECK_INTERVAL_HOURS = 6;
export const AUTO_CHECK_INTERVAL_MS = AUTO_CHECK_INTERVAL_HOURS * 60 * 60 * 1_000;
/** "Later" hides that version, and only that version, for a day. */
export const DEFER_UPDATE_MS = 24 * 60 * 60 * 1_000;
const LATEST_NOTICE_MS = 4_000;
const CHECK_TIMEOUT_MS = 30_000;
const DOWNLOAD_TIMEOUT_MS = 5 * 60_000;

export interface UpdateSummary {
  version: string;
  currentVersion: string;
  notes: string;
  date: string | null;
}

interface DeferredUpdate {
  version: string;
  until: number;
}

export type UpdatePhase =
  | "idle"
  | "checking"
  | "latest"
  | "available"
  | "downloading"
  | "installing"
  | "error";

/** Asks the updater to check now, from anywhere in the app (Settings). */
export function requestAppUpdateCheck(target: EventTarget = window): void {
  target.dispatchEvent(new Event(APP_UPDATE_CHECK_EVENT));
}

function readJson<T>(key: string): T | null {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : null;
  } catch {
    return null;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Update persistence is helpful but must never prevent installation.
  }
}

function removeStoredValue(key: string): void {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // Storage may be unavailable in hardened webviews.
  }
}

export function shouldDeferUpdate(
  deferred: DeferredUpdate | null,
  version: string,
  now = Date.now(),
): boolean {
  return deferred?.version === version && deferred.until > now;
}

/** A scheduled check runs only when the user allows it and one is due; a
 * manual check ignores both. */
export function shouldRunScheduledCheck(
  enabled: boolean,
  lastCheckedAt: number,
  now = Date.now(),
  interval = AUTO_CHECK_INTERVAL_MS,
): boolean {
  return enabled && now - lastCheckedAt >= interval;
}

export function downloadPercentage(downloaded: number, total: number): number | null {
  if (!Number.isFinite(downloaded) || !Number.isFinite(total) || total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((downloaded / total) * 100)));
}

/** The release page for a version, where the full notes live. */
export function releasePageUrl(version: string): string {
  return `${RELEASES_PAGE_URL}/tag/v${version}`;
}

/** The manifest's `notes` is a link to the release page, so a bare URL is
 * not worth repeating in the sentence; anything else is quoted, first line. */
export function releaseNotePreview(notes: string): string | null {
  const trimmed = notes.trim();
  if (!trimmed || /^https?:\/\/\S+$/.test(trimmed)) return null;
  const firstContentLine = trimmed
    .split(/\r?\n/)
    .map((line) => line.replace(/^\s*([#>*-]|\d+\.)+\s*/, "").trim())
    .find(Boolean);
  return firstContentLine || null;
}

export function describeUpdateError(reason: unknown): string {
  if (reason instanceof Error && reason.message.trim()) return reason.message.trim();
  if (typeof reason === "string" && reason.trim()) return reason.trim();
  try {
    const serialized = JSON.stringify(reason);
    if (serialized && serialized !== "{}") return serialized;
  } catch {
    // Host-owned errors may not be serializable.
  }
  return "Mallard could not complete the update. Check your connection and try again.";
}

/** The running app's version, for Settings; `supported` is false under plain Vite. */
export function useInstalledVersion(): { version: string | null; supported: boolean } {
  const [version, setVersion] = useState<string | null>(null);
  const [supported, setSupported] = useState(false);
  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    setSupported(true);
    void getVersion()
      .then((value) => {
        if (active) setVersion(value);
      })
      .catch(() => {
        if (active) setVersion(null);
      });
    return () => {
      active = false;
    };
  }, []);
  return { version, supported };
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

function dismissButton(onDismiss: () => void): ReactNode {
  return (
    <button type="button" className="sync-notice-dismiss" onClick={onDismiss} title="Dismiss" aria-label="Dismiss update notice">
      <Icon name="x" size={12} />
    </button>
  );
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
      action: dismissButton(handlers.onDismiss),
    };
  }

  if (phase === "error" && error) {
    return {
      key: "app-update",
      severity: "danger",
      title: "Could not check for updates.",
      message: error,
      action: (
        <>
          <button type="button" className="btn" onClick={handlers.onRetry}>Try again</button>
          {dismissButton(handlers.onDismiss)}
        </>
      ),
    };
  }

  if (completed) {
    return {
      key: "app-update",
      severity: "success",
      title: `Mallard ${completed.version} installed.`,
      message: "The update finished and Mallard restarted on the new version.",
      action: (
        <>
          {releaseNotesButton(completed.version, handlers.onOpenReleaseNotes)}
          {dismissButton(handlers.onDismiss)}
        </>
      ),
    };
  }

  return null;
}

interface UpdateProgressProps {
  phase: "downloading" | "installing";
  downloaded: number;
  total: number;
  version: string;
}

/** The download owns the window: a native modal, no close, until the app
 * restarts or the install reports its failure back into the notice. */
export function UpdateProgressDialog({ phase, downloaded, total, version }: UpdateProgressProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const percentage = downloadPercentage(downloaded, total);
  const installing = phase === "installing";
  useEffect(() => {
    const element = dialogRef.current;
    if (!element || element.open) return;
    element.showModal();
  }, []);
  return (
    <dialog
      ref={dialogRef}
      className="sync-modal app-update-dialog"
      aria-labelledby="app-update-title"
      onCancel={(event) => event.preventDefault()}
    >
      <header className="sync-modal-header">
        <div>
          <h1 id="app-update-title">{installing ? "Installing update" : `Downloading Mallard ${version}`}</h1>
          <p>{installing ? "Mallard restarts when the update is in place." : "Keep Mallard open while the signed release downloads."}</p>
        </div>
      </header>
      <div className="sync-modal-body">
        <progress
          className="app-update-progress"
          aria-label="Update progress"
          max={100}
          value={installing ? undefined : percentage ?? undefined}
        />
        <p className="app-update-progress-label">
          {installing ? "Verifying the signature and replacing the application." : percentage === null ? "Preparing download." : `${percentage}% downloaded`}
        </p>
      </div>
    </dialog>
  );
}

interface UseAppUpdatesOptions {
  /** A tracked operation is running; the install button waits for it. */
  busy: boolean;
  /** Reads the user's choice for scheduled checks at the moment one is due,
   * so a toggle in Settings needs no event to take effect. */
  autoCheckEnabled: () => Promise<boolean>;
}

export interface AppUpdatesController {
  notice: NoticeItem | null;
  progress: UpdateProgressProps | null;
}

/** Owns the update state machine for the shell. Inert outside Tauri. */
export function useAppUpdates({ busy, autoCheckEnabled }: UseAppUpdatesOptions): AppUpdatesController {
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [summary, setSummary] = useState<UpdateSummary | null>(null);
  const [completed, setCompleted] = useState<UpdateSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(0);
  const updateRef = useRef<Update | null>(null);
  const mountedRef = useRef(true);
  const checkingRef = useRef(false);
  const installingRef = useRef(false);
  const lastCheckRef = useRef(0);
  const latestNoticeTimerRef = useRef<number | null>(null);
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const autoCheckEnabledRef = useRef(autoCheckEnabled);
  autoCheckEnabledRef.current = autoCheckEnabled;

  const clearLatestNoticeTimer = useCallback(() => {
    if (latestNoticeTimerRef.current !== null) {
      window.clearTimeout(latestNoticeTimerRef.current);
      latestNoticeTimerRef.current = null;
    }
  }, []);

  const closeUpdateHandle = useCallback(async () => {
    const current = updateRef.current;
    updateRef.current = null;
    if (current) {
      try {
        await current.close();
      } catch {
        // The updater resource may already be closed after an install failure.
      }
    }
  }, []);

  const checkForUpdates = useCallback(async (interactive: boolean) => {
    if (!isTauri() || checkingRef.current || installingRef.current) return;
    if (!interactive) {
      let enabled = true;
      try {
        enabled = await autoCheckEnabledRef.current();
      } catch {
        // An unreadable settings file must not silently switch checks off or
        // on; the default applies, as it does for every other setting.
      }
      if (!shouldRunScheduledCheck(enabled, lastCheckRef.current)) return;
    }
    checkingRef.current = true;
    lastCheckRef.current = Date.now();
    clearLatestNoticeTimer();
    if (interactive) {
      setError(null);
      setCompleted(null);
      setPhase("checking");
    }
    try {
      const update = await check({ timeout: CHECK_TIMEOUT_MS });
      if (!mountedRef.current) {
        await update?.close();
        return;
      }
      if (!update) {
        await closeUpdateHandle();
        setSummary(null);
        setError(null);
        if (interactive) {
          setPhase("latest");
          latestNoticeTimerRef.current = window.setTimeout(() => {
            latestNoticeTimerRef.current = null;
            if (mountedRef.current) setPhase((current) => (current === "latest" ? "idle" : current));
          }, LATEST_NOTICE_MS);
        } else {
          setPhase("idle");
        }
        return;
      }

      const nextSummary: UpdateSummary = {
        version: update.version,
        currentVersion: update.currentVersion,
        notes: update.body ?? "",
        date: update.date ?? null,
      };
      if (!interactive && shouldDeferUpdate(readJson<DeferredUpdate>(UPDATE_DEFERRED_KEY), nextSummary.version)) {
        await update.close();
        return;
      }

      await closeUpdateHandle();
      updateRef.current = update;
      setSummary(nextSummary);
      setError(null);
      setCompleted(null);
      setPhase("available");
    } catch (reason) {
      // A scheduled check that fails stays quiet: the network is not news.
      if (interactive && mountedRef.current) {
        setError(describeUpdateError(reason));
        setPhase("error");
      }
    } finally {
      checkingRef.current = false;
    }
  }, [clearLatestNoticeTimer, closeUpdateHandle]);

  useEffect(() => {
    mountedRef.current = true;
    if (!isTauri()) {
      return () => {
        mountedRef.current = false;
      };
    }

    void getVersion()
      .then((currentVersion) => {
        if (!mountedRef.current) return;
        const pending = readJson<UpdateSummary>(PENDING_CHANGELOG_KEY);
        let lastSeen: string | null = null;
        try {
          lastSeen = window.localStorage.getItem(LAST_SEEN_CHANGELOG_KEY);
        } catch {
          // Keep the receipt visible if storage is unavailable.
        }
        if (pending?.version === currentVersion && lastSeen !== currentVersion) {
          setCompleted(pending);
        } else if (pending && pending.version !== currentVersion) {
          removeStoredValue(PENDING_CHANGELOG_KEY);
        }
      })
      .catch(() => {
        // App metadata failures must not affect startup.
      });

    const initialTimer = window.setTimeout(() => void checkForUpdates(false), AUTO_CHECK_DELAY_MS);
    const interval = window.setInterval(() => void checkForUpdates(false), AUTO_CHECK_INTERVAL_MS);
    const handleFocus = () => void checkForUpdates(false);
    const handleManualCheck = () => void checkForUpdates(true);
    window.addEventListener("focus", handleFocus);
    window.addEventListener(APP_UPDATE_CHECK_EVENT, handleManualCheck);

    return () => {
      mountedRef.current = false;
      window.clearTimeout(initialTimer);
      window.clearInterval(interval);
      clearLatestNoticeTimer();
      window.removeEventListener("focus", handleFocus);
      window.removeEventListener(APP_UPDATE_CHECK_EVENT, handleManualCheck);
      void closeUpdateHandle();
    };
  }, [checkForUpdates, clearLatestNoticeTimer, closeUpdateHandle]);

  const onLater = useCallback(() => {
    if (summary) {
      writeJson(UPDATE_DEFERRED_KEY, { version: summary.version, until: Date.now() + DEFER_UPDATE_MS });
    }
    void closeUpdateHandle();
    setSummary(null);
    setError(null);
    setPhase("idle");
  }, [closeUpdateHandle, summary]);

  const onInstall = useCallback(async () => {
    const update = updateRef.current;
    if (!update || !summary || busyRef.current || installingRef.current) return;
    const approved = await confirm(
      `Install Mallard ${summary.version} now?\n\nMallard will restart. Finish anything in progress before continuing.`,
      { title: "Install update" },
    );
    if (!approved || busyRef.current) return;
    installingRef.current = true;
    setError(null);
    setDownloaded(0);
    setTotal(0);
    setPhase("downloading");
    writeJson(PENDING_CHANGELOG_KEY, summary);

    let received = 0;
    let expected = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          expected = event.data.contentLength ?? 0;
          setTotal(expected);
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          setDownloaded(received);
        } else if (event.event === "Finished") {
          if (expected > 0) setDownloaded(expected);
          setPhase("installing");
        }
      }, { timeout: DOWNLOAD_TIMEOUT_MS });
      setPhase("installing");
      await relaunch();
    } catch (reason) {
      installingRef.current = false;
      removeStoredValue(PENDING_CHANGELOG_KEY);
      await closeUpdateHandle();
      setError(describeUpdateError(reason));
      setPhase("error");
    }
  }, [closeUpdateHandle, summary]);

  const onRetry = useCallback(() => {
    setError(null);
    setPhase("idle");
    void closeUpdateHandle().then(() => checkForUpdates(true));
  }, [checkForUpdates, closeUpdateHandle]);

  const onDismiss = useCallback(() => {
    clearLatestNoticeTimer();
    if (completed) {
      try {
        window.localStorage.setItem(LAST_SEEN_CHANGELOG_KEY, completed.version);
      } catch {
        // Dismissing must always work.
      }
      removeStoredValue(PENDING_CHANGELOG_KEY);
      setCompleted(null);
    }
    setError(null);
    setPhase("idle");
  }, [clearLatestNoticeTimer, completed]);

  const onOpenReleaseNotes = useCallback((version: string) => {
    void openExternalUrl(releasePageUrl(version));
  }, []);

  const notice = buildUpdateNotice(
    { phase, summary, error, completed, busy },
    { onInstall: () => void onInstall(), onLater, onRetry, onDismiss, onOpenReleaseNotes },
  );
  const progress = summary && (phase === "downloading" || phase === "installing")
    ? { phase, downloaded, total, version: summary.version }
    : null;

  return { notice, progress };
}

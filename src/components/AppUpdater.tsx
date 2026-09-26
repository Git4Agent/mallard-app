import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import packageMetadata from "../../package.json";
import Icon from "./Icons";
import { projectSyncApi } from "./project-sync/api";
import { openExternalUrl } from "./project-sync/externalLink";

/* In-app updates from GitHub Releases.
 *
 * The backend (`app_update.rs`) owns the release: a check that finds a newer
 * published version downloads and verifies it, and the package installs
 * when the user restarts with the sidebar's Update button or quits
 * (`docs/DESIGN_APP_UPDATES.md`). This module decides when to ask and draws
 * the one place an update shows: a pill in the sidebar's version mark. It
 * never draws a failure — a check, download, or install that fails leaves
 * the corner as it was, and the next check tries again — and it never
 * restarts over a Push, Pull, or apply in flight.
 */

export const RELEASES_PAGE_URL = "https://github.com/Git4Agent/mallard-app/releases";

const LAST_RUN_VERSION_KEY = "mallard.update.last-run-version";
/** The first check waits for the startup load to settle. */
export const AUTO_CHECK_DELAY_MS = 8_000;
export const AUTO_CHECK_INTERVAL_HOURS = 6;
export const AUTO_CHECK_INTERVAL_MS = AUTO_CHECK_INTERVAL_HOURS * 60 * 60 * 1_000;

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

/** The release page for a version, where the full notes live. */
export function releasePageUrl(version: string): string {
  return `${RELEASES_PAGE_URL}/tag/v${version}`;
}

/** The version this launch arrived at, when the previous launch ran another
 * one; null on a first launch or an ordinary one. */
export function launchReceipt(previousVersion: string | null, currentVersion: string): string | null {
  return previousVersion && previousVersion !== currentVersion ? currentVersion : null;
}

function readLastRunVersion(): string | null {
  try {
    return window.localStorage.getItem(LAST_RUN_VERSION_KEY);
  } catch {
    return null;
  }
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

export interface AppUpdates {
  /** Downloaded and verified; installs on restart or quit. */
  readyVersion: string | null;
  /** A check (and its download) is in flight, scheduled or manual. */
  checking: boolean;
  restarting: boolean;
  /** A tracked operation is running; the restart waits for it. */
  operationRunning: boolean;
  /** When a check last reached GitHub. A failed check leaves it alone. */
  lastCheckedAt: number | null;
  /** The version this launch was updated to, until its notes are opened. */
  receiptVersion: string | null;
  checkNow: () => void;
  restart: () => void;
  openReceipt: () => void;
}

interface UseAppUpdatesOptions {
  /** A tracked operation is running; the restart waits for it. */
  busy: boolean;
  /** Reads the user's choice for scheduled checks at the moment one is due,
   * so a toggle in Settings needs no event to take effect. */
  autoCheckEnabled: () => Promise<boolean>;
}

/** Owns update scheduling for the shell. Inert outside Tauri. */
export function useAppUpdates({ busy, autoCheckEnabled }: UseAppUpdatesOptions): AppUpdates {
  const [readyVersion, setReadyVersion] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [lastCheckedAt, setLastCheckedAt] = useState<number | null>(null);
  // Read in the initializer and written in an effect, so a double-invoked
  // mount still compares against the previous launch.
  const [receiptVersion, setReceiptVersion] = useState<string | null>(
    () => (isTauri() ? launchReceipt(readLastRunVersion(), packageMetadata.version) : null),
  );
  const mountedRef = useRef(true);
  const checkingRef = useRef(false);
  const restartingRef = useRef(false);
  const lastAttemptRef = useRef(0);
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const autoCheckEnabledRef = useRef(autoCheckEnabled);
  autoCheckEnabledRef.current = autoCheckEnabled;

  const check = useCallback(async (manual: boolean) => {
    if (!isTauri() || checkingRef.current || restartingRef.current) return;
    if (!manual) {
      let enabled = true;
      try {
        enabled = await autoCheckEnabledRef.current();
      } catch {
        // An unreadable settings file must not silently switch checks off or
        // on; the default applies, as it does for every other setting.
      }
      if (!shouldRunScheduledCheck(enabled, lastAttemptRef.current)) return;
    }
    checkingRef.current = true;
    lastAttemptRef.current = Date.now();
    setChecking(true);
    try {
      const status = await projectSyncApi.prepareAppUpdate();
      if (!mountedRef.current) return;
      setReadyVersion(status.ready_version);
      setLastCheckedAt(Date.now());
    } catch {
      // Never news: an offline check, a failed download, or a signature that
      // does not verify leaves the corner as it was.
    } finally {
      checkingRef.current = false;
      if (mountedRef.current) setChecking(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    if (!isTauri()) {
      return () => {
        mountedRef.current = false;
      };
    }
    try {
      window.localStorage.setItem(LAST_RUN_VERSION_KEY, packageMetadata.version);
    } catch {
      // Storage may be unavailable in hardened webviews; the receipt is a courtesy.
    }
    const initialTimer = window.setTimeout(() => void check(false), AUTO_CHECK_DELAY_MS);
    const interval = window.setInterval(() => void check(false), AUTO_CHECK_INTERVAL_MS);
    const handleFocus = () => void check(false);
    window.addEventListener("focus", handleFocus);
    return () => {
      mountedRef.current = false;
      window.clearTimeout(initialTimer);
      window.clearInterval(interval);
      window.removeEventListener("focus", handleFocus);
    };
  }, [check]);

  const restart = useCallback(async () => {
    if (!readyVersion || busyRef.current || restartingRef.current) return;
    restartingRef.current = true;
    setRestarting(true);
    try {
      // On success the process restarts into the new version.
      await projectSyncApi.restartToUpdate();
    } catch {
      // The backend dropped the package; the next check downloads it again.
      restartingRef.current = false;
      if (mountedRef.current) {
        setRestarting(false);
        setReadyVersion(null);
      }
    }
  }, [readyVersion]);

  const openReceipt = useCallback(() => {
    if (!receiptVersion) return;
    void openExternalUrl(releasePageUrl(receiptVersion));
    setReceiptVersion(null);
  }, [receiptVersion]);

  return {
    readyVersion,
    checking,
    restarting,
    operationRunning: busy,
    lastCheckedAt,
    receiptVersion,
    checkNow: () => void check(true),
    restart: () => void restart(),
    openReceipt,
  };
}

/** The sidebar version mark's one action: Update when a release is ready,
 * Restarting while it installs, What's new on the first launch after an
 * update, and nothing otherwise. */
export function UpdateCorner({ updates }: { updates: AppUpdates }) {
  if (updates.restarting) {
    return (
      <span className="sync-update-pill" role="status">
        <span className="status-loader" aria-hidden="true" />
        <span>Restarting</span>
      </span>
    );
  }
  if (updates.readyVersion) {
    const title = updates.operationRunning
      ? `Mallard ${updates.readyVersion} is ready. Finish the current operation, then restart to update.`
      : `Restart Mallard to install ${updates.readyVersion}`;
    return (
      <button
        type="button"
        className="sync-update-pill"
        onClick={updates.restart}
        disabled={updates.operationRunning}
        title={title}
        aria-label={title}
      >
        <Icon name="refresh" size={13} />
        <span>Update</span>
      </button>
    );
  }
  if (updates.receiptVersion) {
    return (
      <button
        type="button"
        className="sync-update-pill receipt"
        onClick={updates.openReceipt}
        title={`See what changed in Mallard ${updates.receiptVersion}`}
      >
        What’s new
      </button>
    );
  }
  return null;
}

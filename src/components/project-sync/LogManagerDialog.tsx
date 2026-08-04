import { useEffect, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ActivityLogPolicy, ActivityLogStats } from "../../types";
import Icon from "../Icons";
import { projectSyncApi } from "./api";
import { errorMessage } from "./model";

interface Props {
  onClose: () => void;
  onLogsChanged: () => void;
}

const RETENTION_OPTIONS = [7, 30, 90, 180, 365];
const SIZE_OPTIONS_MB = [25, 100, 250, 500, 1_000];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatOldest(timestamp?: number | null): string {
  if (!timestamp) return "No retained entries";
  return new Date(timestamp).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export default function LogManagerDialog({ onClose, onLogsChanged }: Props) {
  const [stats, setStats] = useState<ActivityLogStats | null>(null);
  const [policy, setPolicy] = useState<ActivityLogPolicy | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  useEffect(() => {
    let active = true;
    void projectSyncApi.getActivityLogStats()
      .then((next) => {
        if (!active) return;
        setStats(next);
        setPolicy(next.policy);
      })
      .catch((reason) => active && setError(errorMessage(reason)))
      .finally(() => active && setLoading(false));
    return () => { active = false; };
  }, []);

  const savePolicy = async (): Promise<ActivityLogStats> => {
    if (!policy) throw new Error("Log policy is unavailable");
    const next = await projectSyncApi.updateActivityLogPolicy(policy);
    setStats(next);
    setPolicy(next.policy);
    return next;
  };

  const handleSave = async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await savePolicy();
      setNotice("Retention settings saved");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleCleanup = async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await savePolicy();
      const result = await projectSyncApi.cleanupActivityLogs({});
      setStats(result.stats);
      setPolicy(result.stats.policy);
      setNotice(
        result.removed_files
          ? `Removed ${result.removed_files} log file${result.removed_files === 1 ? "" : "s"} · ${formatBytes(result.reclaimed_bytes)} reclaimed`
          : "No old logs needed cleanup",
      );
      onLogsChanged();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleDeleteAll = async () => {
    const approved = await confirm(
      "Delete all retained activity logs?\n\nMallard configuration, backups, local files, and cloud generations will not be changed.",
      { title: "Delete activity logs", kind: "warning", okLabel: "Delete logs", cancelLabel: "Cancel" },
    );
    if (!approved) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await projectSyncApi.cleanupActivityLogs({ delete_all: true });
      setStats(result.stats);
      setPolicy(result.stats.policy);
      setNotice("Retained activity logs deleted");
      onLogsChanged();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleOpenFolder = async () => {
    setError(null);
    try {
      await openPath(await projectSyncApi.getActivityLogFolder());
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <div
      className="sync-modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <section
        className="sync-modal sync-log-manager-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="sync-log-manager-title"
      >
        <header className="sync-modal-header">
          <div>
            <span className="sync-eyebrow">Local activity</span>
            <h1 id="sync-log-manager-title">Manage logs</h1>
            <p>Logs stay on this machine in <code>~/.mallard/logs</code>.</p>
          </div>
          <button type="button" className="btn btn-ghost" onClick={onClose} disabled={busy} aria-label="Close log manager">
            <Icon name="x" size={17} />
          </button>
        </header>

        <div className="sync-modal-body sync-log-manager-body">
          {loading ? (
            <div className="sync-log-manager-loading"><span className="status-loader" /> Loading log storage…</div>
          ) : policy && stats ? (
            <>
              <dl className="sync-log-storage-summary">
                <div><dt>Storage used</dt><dd>{formatBytes(stats.total_bytes)}</dd></div>
                <div><dt>Log files</dt><dd>{stats.file_count}</dd></div>
                <div><dt>Oldest entry</dt><dd>{formatOldest(stats.oldest_ts)}</dd></div>
              </dl>

              <div className="sync-log-policy-fields">
                <label>
                  <span>Keep logs for</span>
                  <select
                    value={policy.retention_days}
                    disabled={busy}
                    onChange={(event) => setPolicy({ ...policy, retention_days: Number(event.target.value) })}
                  >
                    {!RETENTION_OPTIONS.includes(policy.retention_days) && (
                      <option value={policy.retention_days}>{policy.retention_days} days</option>
                    )}
                    {RETENTION_OPTIONS.map((days) => <option key={days} value={days}>{days} days</option>)}
                  </select>
                </label>
                <label>
                  <span>Maximum storage</span>
                  <select
                    value={Math.round(policy.max_total_bytes / (1024 * 1024))}
                    disabled={busy}
                    onChange={(event) => setPolicy({
                      ...policy,
                      max_total_bytes: Number(event.target.value) * 1024 * 1024,
                    })}
                  >
                    {!SIZE_OPTIONS_MB.includes(Math.round(policy.max_total_bytes / (1024 * 1024))) && (
                      <option value={Math.round(policy.max_total_bytes / (1024 * 1024))}>
                        {formatBytes(policy.max_total_bytes)}
                      </option>
                    )}
                    {SIZE_OPTIONS_MB.map((size) => <option key={size} value={size}>{size.toLocaleString()} MB</option>)}
                  </select>
                </label>
              </div>

              <div className="sync-log-cleanup-actions">
                <button type="button" className="btn" onClick={() => void handleOpenFolder()} disabled={busy}>
                  <Icon name="folder" size={14} /> Open logs folder
                </button>
                <button type="button" className="btn" onClick={() => void handleCleanup()} disabled={busy}>
                  <Icon name="trash" size={14} /> Clean up now
                </button>
                <button type="button" className="btn btn-ghost danger" onClick={() => void handleDeleteAll()} disabled={busy || stats.file_count === 0}>
                  Delete all logs
                </button>
              </div>

              {error && <div className="sync-callout error" role="alert"><Icon name="alert-triangle" size={15} /> {error}</div>}
              {notice && <div className="sync-log-manager-notice" role="status"><Icon name="check-circle" size={14} /> {notice}</div>}
            </>
          ) : error ? (
            <div className="sync-callout error" role="alert"><Icon name="alert-triangle" size={15} /> {error}</div>
          ) : null}
        </div>

        <footer className="sync-modal-footer">
          <span>Cleanup never removes sync state, backups, or cloud data.</span>
          <div>
            <button type="button" className="btn" onClick={onClose} disabled={busy}>Close</button>
            <button type="button" className="btn btn-primary" onClick={() => void handleSave()} disabled={busy || !policy}>
              {busy ? "Working…" : "Save settings"}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );
}

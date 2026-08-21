import { useEffect, useRef, useState } from "react";
import type { BundleSnapshotSummary, StorageConfig } from "../../types";
import Icon from "../Icons";
import { formatRelativeTime } from "./model";

interface Props {
  projectName: string;
  currentBundleId: string;
  projectFingerprint?: string | null;
  storage: StorageConfig;
  matches: BundleSnapshotSummary[];
  reason: "link" | "missing" | "sibling";
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onUseExisting: (match: BundleSnapshotSummary) => void;
  onKeepCurrent: () => void;
}

export default function BundleConnectionDialog({
  projectName,
  currentBundleId,
  projectFingerprint,
  storage,
  matches,
  reason,
  busy,
  error,
  onCancel,
  onUseExisting,
  onKeepCurrent,
}: Props) {
  const [selectedId, setSelectedId] = useState("");

  useEffect(() => setSelectedId(""), [matches]);

  // Native <dialog> + showModal() supplies focus trap, Escape, inert
  // background, and focus restoration.
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  const selected = matches.find((match) => match.bundle_id === selectedId) ?? null;
  const missing = reason === "missing";
  const sibling = reason === "sibling";
  const selectedFingerprintDiffers = !!projectFingerprint
    && !!selected?.repository_fingerprint
    && projectFingerprint !== selected.repository_fingerprint;
  const selectedFingerprintUnknown = !!projectFingerprint
    && !!selected
    && !selected.repository_fingerprint;

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-bundle-connection-dialog"
      aria-labelledby="sync-bundle-connection-title"
      onCancel={(event) => { if (busy) event.preventDefault(); }}
      onClose={onCancel}
    >
        <header className="sync-modal-header">
          <div>
            <span className="sync-eyebrow">Repo identity</span>
            <h1 id="sync-bundle-connection-title">
              {sibling
                ? "Connect to this repository's repo"
                : missing
                  ? "Choose the remote repo"
                  : "Choose a repo from this storage"}
            </h1>
            <p>
              {sibling
                ? `${storage.name} already has this repository in another repo. Connecting adopts it, so pushes and pulls from every machine converge; the current empty registration is abandoned.`
                : missing
                  ? `${projectName} has a local-only identity. Pull from an existing repo, or publish this one separately.`
                  : `${storage.name} contains the repos below. Git matches are recommended, but every repo remains available.`}
            </p>
          </div>
          <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy} aria-label="Close repo chooser">
            <Icon name="x" size={17} />
          </button>
        </header>

        <div className="sync-modal-body">
          <div className="sync-bundle-context">
            <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={17} />
            <span><strong>{storage.name}</strong><small>Current local repo {currentBundleId.slice(0, 12)}…</small></span>
          </div>

          <div className="sync-bundle-choice-list" role="radiogroup" aria-label="Available remote repos">
            {matches.map((match) => {
              const active = selectedId === match.bundle_id;
              const recommended = !!projectFingerprint
                && match.repository_fingerprint === projectFingerprint;
              return (
                <button
                  key={match.bundle_id}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  className={`sync-bundle-choice${active ? " active" : ""}`}
                  onClick={() => setSelectedId(match.bundle_id)}
                  disabled={busy}
                >
                  <span className="sync-bundle-radio"><span /></span>
                  <span className="sync-bundle-choice-copy">
                    <strong>{match.display_name}{recommended && <small className="sync-bundle-recommended">Recommended</small>}</strong>
                    <span>{match.bundle_id}</span>
                  </span>
                  <span className="sync-bundle-choice-meta">
                    <strong>rev {match.revision ?? "—"}</strong>
                    <span>{match.resource_count ?? match.resources?.length ?? 0} resources</span>
                    <small>{formatRelativeTime(match.updated_at)}</small>
                  </span>
                </button>
              );
            })}
          </div>

          {selectedFingerprintDiffers && (
            <div className="sync-callout warning">
              <Icon name="alert-triangle" size={15} />
              <span>This repo has a different Git fingerprint. Connecting will adopt its repository identity.</span>
            </div>
          )}
          {selectedFingerprintUnknown && (
            <div className="sync-callout warning">
              <Icon name="alert-triangle" size={15} />
              <span>This repo has no Git fingerprint, so Mallard cannot verify that it belongs to this checkout.</span>
            </div>
          )}

          <div className="sync-callout warning">
            <Icon name="alert-triangle" size={15} />
            <span>Connecting changes only Mallard metadata. It does not move or delete the checkout or provider profile.</span>
          </div>
          {error && <div className="sync-callout error"><Icon name="alert-triangle" size={15} /> {error}</div>}
        </div>

        <footer className="sync-modal-footer">
          <button type="button" className="btn btn-ghost" onClick={onKeepCurrent} disabled={busy}>
            {sibling ? "Not now" : missing ? "Review separate Push" : "Create separate repo"}
          </button>
          <div>
            <button type="button" className="btn" onClick={onCancel} disabled={busy}>Cancel</button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy || !selected}
              onClick={() => selected && onUseExisting(selected)}
            >
              {busy ? "Connecting…" : "Connect and review Pull"}
            </button>
          </div>
        </footer>
    </dialog>
  );
}

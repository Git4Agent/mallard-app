import { useEffect, useRef, useState } from "react";
import type { BundleSnapshotSummary, ManagedCloudRepository, StorageSummary } from "../../types";
import Icon, { storageIconName } from "../Icons";
import Notice, { NoticeZone, type NoticeItem } from "../notice/Notice";
import { formatRelativeTime } from "../../clock";
import RepositoryAddressPicker, {
  isManagedCloudStorage,
  managedCloudAccount,
  useManagedCloudRepositories,
  type RepositorySelection,
} from "./RepositoryAddressPicker";

/** What Connect acts on: the remote bundle, and whether the caller has
 *  seen and accepted that its Git remote is not this checkout's. */
export interface BundleConnectionChoice {
  bundleId: string;
  allowRepositoryMismatch: boolean;
}

interface Props {
  projectName: string;
  currentBundleId: string;
  projectFingerprint?: string | null;
  storage: StorageSummary;
  /** The storage's bundles, for a storage that is not Mallard Cloud; a
   *  Mallard Cloud storage lists and resolves its repositories itself. */
  matches: BundleSnapshotSummary[];
  reason: "link" | "missing" | "sibling";
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onUseExisting: (choice: BundleConnectionChoice) => void;
  onKeepCurrent: () => void;
}

export default function BundleConnectionDialog(props: Props) {
  const { busy, onCancel } = props;
  // Native <dialog> + showModal() supplies focus trap, Escape, inert
  // background, and focus restoration.
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);
  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-bundle-connection-dialog"
      aria-labelledby="sync-bundle-connection-title"
      onCancel={(event) => { if (busy) event.preventDefault(); }}
      onClose={onCancel}
    >
      {isManagedCloudStorage(props.storage) ? <ManagedCloudBody {...props} /> : <BundleListBody {...props} />}
    </dialog>
  );
}

/** Mallard Cloud: the repository is chosen by its website address, or from
 *  the rows the account belongs to; New repository is one of the rows when
 *  the project is being linked for the first time. */
function ManagedCloudBody({
  projectName,
  projectFingerprint,
  storage,
  reason,
  busy,
  error,
  onCancel,
  onUseExisting,
  onKeepCurrent,
}: Props) {
  const localFingerprint = projectFingerprint ?? null;
  const listing = useManagedCloudRepositories(storage.storage_id, localFingerprint);
  const [selection, setSelection] = useState<RepositorySelection | null>(null);
  const [chosen, setChosen] = useState<ManagedCloudRepository | null>(null);
  const account = managedCloudAccount(storage);
  const missing = reason === "missing";
  const sibling = reason === "sibling";
  const offerNew = reason === "link";

  const existing = selection?.kind === "existing" ? selection : null;
  const acknowledgementPending = !!existing
    && (existing.repository_fingerprint ?? null) !== localFingerprint
    && !existing.mismatch_acknowledged;
  const contributor = !!existing && chosen?.role === null;
  const verb = selection?.kind === "new" ? "Create and connect" : "Connect";
  const footerNote = contributor
    ? "Anyone signed in can push. Review changes with Pull."
    : "Review changes with Pull.";
  const connect = () => {
    if (!selection) return;
    if (selection.kind === "new") {
      onKeepCurrent();
      return;
    }
    onUseExisting({
      bundleId: selection.bundle_id,
      allowRepositoryMismatch: (selection.repository_fingerprint ?? null) !== localFingerprint,
    });
  };

  return (
    <>
      <header className="sync-modal-header">
        <div>
          <h1 id="sync-bundle-connection-title">
            {sibling
              ? "Connect to existing repository"
              : missing
                ? "Choose a repository"
                : `Connect ${projectName}`}
          </h1>
          {(sibling || missing) && (
            <p>
              {sibling
                ? "Use the existing repository for Push and Pull. Stop using the empty registration."
                : `${projectName} is local only. Choose a cloud repository.`}
            </p>
          )}
        </div>
        <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy} aria-label="Close repo chooser">
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body repo-connect">
        <div className="sync-bundle-context">
          <Icon name={storageIconName(storage.provider, storage.icon)} size={17} />
          <span><strong>Mallard Cloud · {account.login}</strong><small>{account.host}</small></span>
        </div>
        <RepositoryAddressPicker
          storage={storage}
          projectName={projectName}
          localFingerprint={localFingerprint}
          listing={listing}
          selection={selection}
          rows="choice"
          offerNew={offerNew}
          disabled={busy}
          onSelect={(next, repository) => { setSelection(next); setChosen(repository); }}
        />
        {error && <Notice severity="danger" timing="blocking">{error}</Notice>}
      </div>

      <footer className="sync-modal-footer">
        {offerNew
          ? existing && <span>{footerNote}</span>
          : (
            <button type="button" className="btn btn-ghost" onClick={onKeepCurrent} disabled={busy}>
              {sibling ? "Not now" : "Review separate Push"}
            </button>
          )}
        <div>
          <button type="button" className="btn" onClick={onCancel} disabled={busy}>Cancel</button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !selection || acknowledgementPending}
            onClick={connect}
          >
            {busy ? "Connecting…" : verb}
          </button>
        </div>
      </footer>
    </>
  );
}

/** Every other storage: the bundles it holds, one row each. */
function BundleListBody({
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

  const selected = matches.find((match) => match.bundle_id === selectedId) ?? null;
  const missing = reason === "missing";
  const sibling = reason === "sibling";
  const selectedFingerprintDiffers = !!projectFingerprint
    && !!selected?.repository_fingerprint
    && projectFingerprint !== selected.repository_fingerprint;
  const selectedFingerprintUnknown = !!projectFingerprint
    && !!selected
    && !selected.repository_fingerprint;

  /* One box under the list takes the loudest connection notice.
     All four are standing content — the reassurance is always true, the two
     fingerprint lines are read off the selected match, and the failure comes
     in as a prop — so the box is a named region, never a live one (WCAG 2.2
     SC 4.1.3). */
  const notices: NoticeItem[] = [];
  if (selectedFingerprintDiffers) {
    notices.push({
      key: "fingerprint-differs",
      severity: "warning",
      message: "This repo has a different Git fingerprint. Connecting will adopt its repository identity.",
    });
  }
  if (selectedFingerprintUnknown) {
    notices.push({
      key: "fingerprint-unknown",
      severity: "warning",
      message: "This repo has no Git fingerprint, so Mallard cannot verify that it belongs to this checkout.",
    });
  }
  // Pull remains a separate user action after connecting.
  notices.push({
    key: "metadata-only",
    severity: "info",
    message: "Choose Pull when you are ready to review stored changes.",
  });

  return (
    <>
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
                ? `${projectName} has a local-only identity. Connect to an existing repo, or publish this one separately.`
                : `${storage.name} contains the repos below. Git matches are recommended, but every repo remains available.`}
          </p>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy} aria-label="Close repo chooser">
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body">
        <div className="sync-bundle-context">
          <Icon name={storageIconName(storage.provider, storage.icon)} size={17} />
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
                  <strong>Version {match.revision ?? "—"}</strong>
                  <span>{match.resource_count ?? 0} resources</span>
                  <small>{formatRelativeTime(match.updated_at)}</small>
                </span>
              </button>
            );
          })}
        </div>

        <NoticeZone
          items={notices}
          subject="notes about connecting this repo"
          timing="standing"
          label="Notes about connecting this repo"
        />

        {/* The connect failure is its own box, not a rung of the standing
            one: it is raised by this dialog's own primary button and it is
            why the dialog is still open, so it has to announce. Appending it
            to a region would have added a silent bullet under the reassurance
            the reader had already read. */}
        {error && <Notice severity="danger" timing="blocking">{error}</Notice>}
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
            onClick={() => selected && onUseExisting({
              bundleId: selected.bundle_id,
              allowRepositoryMismatch: !!projectFingerprint
                && projectFingerprint !== (selected.repository_fingerprint ?? null),
            })}
          >
            {busy ? "Connecting…" : "Connect"}
          </button>
        </div>
      </footer>
    </>
  );
}

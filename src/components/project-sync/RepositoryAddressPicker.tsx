import { useEffect, useRef, useState, type ReactNode } from "react";
import type {
  ManagedCloudRepository,
  ManagedCloudRepositoryResolution,
  StorageSummary,
} from "../../types";
import Icon, { type IconName } from "../Icons";
import Notice, { Acknowledgement, FieldMessage } from "../notice/Notice";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { openExternalUrl } from "./externalLink";
import { formatRelativeTime, repositoryFingerprintsRequireAcknowledgement } from "./model";

/**
 * Deciding which Mallard Cloud repository a project connects to, by the
 * repository's website address. One part, two hosts: the body of the
 * connect dialog for an existing project, and the Repository band of
 * project setup once a Mallard Cloud storage is chosen.
 *
 * Top to bottom: the address field in the Folder field's shape, with the
 * lookup verb beside it; what the lookup answered, in the storage
 * discovery's own rows (a found row, the empty state, a field error); the
 * repositories the account belongs to, as the host's option rows, named by
 * address; and New repository first among them, saying the address it will
 * create from the project name. Picking a row fills the field; resolving an
 * address selects the repository; entering a private repository's
 * invitation code joins it and selects it. Nothing here prints a bundle id.
 */

export const MANAGED_CLOUD_PROVIDER = "managed_cloud";

export function isManagedCloudStorage(storage: Pick<StorageSummary, "provider"> | null | undefined): boolean {
  return storage?.provider === MANAGED_CLOUD_PROVIDER;
}

/** The choice the picker reports: the draft's own shape for an existing
 *  repository (its address as the display name), or a new one. */
export type RepositorySelection =
  | { kind: "new" }
  | {
    kind: "existing";
    bundle_id: string;
    display_name: string;
    repository_fingerprint: string | null;
    mismatch_acknowledged: boolean;
  };

export interface ManagedCloudAccount {
  login: string;
  origin: string;
  /** The origin without its scheme: what the website's address bar shows. */
  host: string;
}

/** The account and service a Mallard Cloud storage was signed in to, from
 *  its saved settings (the summary fields carry the same two values). */
export function managedCloudAccount(storage: Pick<StorageSummary, "settings" | "summary">): ManagedCloudAccount {
  const read = (key: string): string => {
    const setting = storage.settings?.[key];
    if (typeof setting === "string" && setting) return setting;
    return storage.summary.find((field) => field.field_id === key)?.value ?? "";
  };
  const origin = read("service_origin").replace(/\/+$/, "");
  return {
    login: read("account_login"),
    origin,
    host: origin.replace(/^[a-z]+:\/\//i, ""),
  };
}

const REPOSITORY_NAME_MAX = 100;

/** Preserve valid names exactly; clean up other project titles using the
 *  service's rules. */
export function normalizeRepositoryName(requested: string): string {
  const trimmed = requested.trim();
  if (/^[A-Za-z0-9._-]{1,100}$/.test(trimmed) && trimmed !== "." && trimmed !== "..") return trimmed;
  let name = trimmed.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/-{2,}/g, "-").replace(/^-+|-+$/g, "");
  if (name.length > REPOSITORY_NAME_MAX) name = name.slice(0, REPOSITORY_NAME_MAX).replace(/-+$/g, "");
  return /^[A-Za-z0-9._-]{1,100}$/.test(name) && name !== "." && name !== ".." ? name : "repository";
}

/** The address a new repository would get: the normalized project name,
 *  suffixed `-2`, `-3`, … while the account already owns the name. The
 *  service stays the authority; this is the preview on the New row. */
export function predictRepositoryAddress(
  login: string,
  requestedName: string,
  repositories: ReadonlyArray<Pick<ManagedCloudRepository, "owner_login" | "name">>,
): { address: string; name: string; base: string; suffixed: boolean } {
  const base = normalizeRepositoryName(requestedName);
  const taken = new Set(
    repositories
      .filter((repository) => repository.owner_login.toLowerCase() === login.toLowerCase())
      .map((repository) => repository.name.toLowerCase()),
  );
  let name = base;
  for (let n = 2; taken.has(name.toLowerCase()); n++) {
    const suffix = `-${n}`;
    name = `${base.slice(0, REPOSITORY_NAME_MAX - suffix.length).replace(/-+$/g, "")}${suffix}`;
  }
  return { address: `${login}/${name}`, name, base, suffixed: name !== base };
}

export type RepositoryListing =
  | { state: "loading" }
  | { state: "ready"; repositories: ManagedCloudRepository[] }
  | { state: "error"; message: string };

/** The account's project repositories on one Mallard Cloud storage, read
 *  once per storage and local fingerprint. */
export function useManagedCloudRepositories(
  storageId: string | null,
  localFingerprint: string | null,
  isActive: () => boolean = () => true,
): RepositoryListing {
  const [listing, setListing] = useState<RepositoryListing>({ state: "loading" });
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  useEffect(() => {
    let cancelled = false;
    if (!storageId) {
      setListing({ state: "ready", repositories: [] });
      return () => { cancelled = true; };
    }
    setListing({ state: "loading" });
    void projectSyncApi.listManagedCloudRepositories(storageId, localFingerprint)
      .then((repositories) => {
        if (!cancelled && isActiveRef.current()) setListing({ state: "ready", repositories });
      })
      .catch((reason) => {
        if (!cancelled && isActiveRef.current()) setListing({ state: "error", message: errorMessage(reason) });
      });
    return () => { cancelled = true; };
  }, [storageId, localFingerprint]);
  return listing;
}

export function selectionFor(repository: ManagedCloudRepository): RepositorySelection {
  return {
    kind: "existing",
    bundle_id: repository.bundle_id ?? "",
    display_name: repository.address,
    repository_fingerprint: repository.repository_fingerprint,
    mismatch_acknowledged: false,
  };
}

const capitalize = (value: string) => (value ? value[0].toUpperCase() + value.slice(1) : value);

/** The class mark every list of repositories wears. */
export function repositoryMark(repository: Pick<ManagedCloudRepository, "repository_class">): IconName {
  return repository.repository_class === "git_project" ? "git-folder" : "folder";
}

/** What the found row says about the repository the address named. A
 *  public repository takes pushes from anyone signed in, so a
 *  non-collaborator's row is a plain finding, not a caution. */
export function repositoryVerdict(
  repository: Pick<ManagedCloudRepository, "role" | "git_remote">,
): { tone: "ok" | "warn"; icon: IconName; text: string } {
  const text = repository.role === null
    ? "Public · anyone signed in can push"
    : repository.role === "owner"
      ? `Yours · ${capitalize(repository.role)}`
      : `Collaborator · ${capitalize(repository.role)}`;
  return repository.git_remote === "matches"
    ? { tone: "ok", icon: "check-circle", text }
    : { tone: "warn", icon: "alert-triangle", text };
}

/** The Git remote chip's word, against this checkout. */
export function gitRemoteWord(repository: Pick<ManagedCloudRepository, "git_remote" | "repository_fingerprint">, localFingerprint: string | null): string {
  switch (repository.git_remote) {
    case "matches": return localFingerprint ? "this checkout's" : "none, like this folder";
    case "differs": return repository.repository_fingerprint ? "differs" : "none";
    case "unknown": return "unverified";
  }
}

/** The cautions under the found row: what connecting would not give. */
export function repositoryCautions(repository: Pick<ManagedCloudRepository, "git_remote">): string[] {
  const cautions: string[] = [];
  if (repository.git_remote === "differs") cautions.push("Different Git remote than this checkout");
  if (repository.git_remote === "unknown") cautions.push("Git remote could not be verified");
  return cautions;
}

type Lookup =
  | { kind: "idle" }
  | { kind: "resolving"; address: string }
  | { kind: "resolved"; resolution: ManagedCloudRepositoryResolution }
  | { kind: "failed"; message: string };

type Join =
  | { kind: "idle" }
  | { kind: "joining" }
  | { kind: "failed"; message: string };

/** The service's code shape: 20 letters in four groups; separators and case are forgiven. */
export function isInvitationCodeShaped(code: string): boolean {
  return code.replace(/[\s-]/g, "").length === 20;
}

interface Props {
  storage: StorageSummary;
  /** The requested name for a new repository: the project's name. */
  projectName: string;
  localFingerprint: string | null;
  listing: RepositoryListing;
  selection: RepositorySelection | null;
  /** The rows' shape: the dialog's or the setup page's. */
  rows: "choice" | "match-option";
  /** Whether New repository is offered first. */
  offerNew: boolean;
  disabled: boolean;
  describedBy?: string;
  /** `null` when a lookup of a new address found no repository to choose:
   *  the field and the rows are one choice, so an existing selection does
   *  not outlive an address that names nothing. */
  onSelect: (selection: RepositorySelection | null, repository: ManagedCloudRepository | null) => void;
}

export default function RepositoryAddressPicker({
  storage,
  projectName,
  localFingerprint,
  listing,
  selection,
  rows,
  offerNew,
  disabled,
  describedBy,
  onSelect,
}: Props) {
  const account = managedCloudAccount(storage);
  const [address, setAddress] = useState("");
  const [lookup, setLookup] = useState<Lookup>({ kind: "idle" });
  const [code, setCode] = useState("");
  const [join, setJoin] = useState<Join>({ kind: "idle" });
  const requestRef = useRef(0);
  useEffect(() => () => { requestRef.current += 1; }, []);

  const repositories = listing.state === "ready" ? listing.repositories : [];
  const resolved = lookup.kind === "resolved"
    && (lookup.resolution.kind === "collaborator" || lookup.resolution.kind === "public")
    ? lookup.resolution.repository
    : null;
  const chosen = selection?.kind === "existing"
    ? repositories.find((repository) => repository.bundle_id === selection.bundle_id)
      ?? (resolved?.bundle_id === selection.bundle_id ? resolved : null)
    : null;

  const runLookup = async () => {
    const typed = address.trim();
    if (!typed || disabled) return;
    const requestId = ++requestRef.current;
    setLookup({ kind: "resolving", address: typed });
    try {
      const resolution = await projectSyncApi.resolveManagedCloudRepository(
        storage.storage_id,
        typed,
        localFingerprint,
      );
      if (requestId !== requestRef.current) return;
      setLookup({ kind: "resolved", resolution });
      if ((resolution.kind === "collaborator" || resolution.kind === "public") && resolution.repository.bundle_id) {
        // The same repository again keeps its acknowledgement.
        if (selection?.kind === "existing" && selection.bundle_id === resolution.repository.bundle_id) return;
        onSelect(selectionFor(resolution.repository), resolution.repository);
      } else if (selection?.kind === "existing") {
        onSelect(null, null);
      }
    } catch (reason) {
      if (requestId !== requestRef.current) return;
      setLookup({ kind: "failed", message: errorMessage(reason) });
      if (selection?.kind === "existing") onSelect(null, null);
    }
  };

  /** Enter an invitation code: the account joins the private repository it
   *  belongs to, and the found row shows it as a collaborator's, selected. */
  const runJoin = async () => {
    const typed = code.trim();
    if (!typed || disabled) return;
    const requestId = ++requestRef.current;
    setJoin({ kind: "joining" });
    try {
      const resolution = await projectSyncApi.acceptManagedCloudInvitation(storage.storage_id, typed, localFingerprint);
      if (requestId !== requestRef.current) return;
      setJoin({ kind: "idle" });
      setCode("");
      setLookup({ kind: "resolved", resolution });
      if ((resolution.kind === "collaborator" || resolution.kind === "public") && resolution.repository.bundle_id) {
        setAddress(resolution.repository.address);
        onSelect(selectionFor(resolution.repository), resolution.repository);
      } else if (selection?.kind === "existing") {
        onSelect(null, null);
      }
    } catch (reason) {
      if (requestId !== requestRef.current) return;
      setJoin({ kind: "failed", message: errorMessage(reason) });
    }
  };

  const chooseRow = (repository: ManagedCloudRepository) => {
    requestRef.current += 1;
    setAddress(repository.address);
    setLookup({ kind: "idle" });
    onSelect(selectionFor(repository), repository);
  };

  const chooseNew = () => {
    requestRef.current += 1;
    setLookup({ kind: "idle" });
    onSelect({ kind: "new" }, null);
  };

  const invalidAddress = lookup.kind === "resolved"
    && (lookup.resolution.kind === "wrong_service" || lookup.resolution.kind === "github" || lookup.resolution.kind === "invalid" || lookup.resolution.kind === "not_a_project");
  const looking = lookup.kind === "resolving";
  const joining = join.kind === "joining";

  const fieldError = (text: ReactNode) => <FieldMessage blocking>{text}</FieldMessage>;

  let answer: ReactNode = null;
  if (lookup.kind === "resolving") {
    answer = (
      <small className="sync-setup-inline-status" role="status">
        <span className="status-loader" /> Looking up {lookup.address} on {account.host}…
      </small>
    );
  } else if (lookup.kind === "failed") {
    answer = fieldError(lookup.message);
  } else if (lookup.kind === "resolved") {
    const { resolution } = lookup;
    switch (resolution.kind) {
      case "wrong_service":
        answer = fieldError(<>That address is on {resolution.host}; this storage is {storage.name} at {resolution.expected}.</>);
        break;
      case "github":
        answer = fieldError("That’s a GitHub address. Paste the Mallard Cloud address from the repository’s page.");
        break;
      case "invalid":
        answer = fieldError(<>Paste the address from the repository’s page on {account.host}, or type owner/name.</>);
        break;
      case "not_a_project":
        answer = fieldError(<>{resolution.repository.address} is a skill, plugin intent, or pet, not a project.</>);
        break;
      case "not_found":
        answer = (
          <div className="r2-step-list">
            <div className="storage-found-empty" role="status">
              <Icon name="search" size={24} />
              <strong>Nothing at {resolution.address} that you can see.</strong>
              <span>Check the spelling. For a private repository, enter an invitation code below. Ask the owner or an admin for the code.</span>
              {offerNew && (
                <span className="storage-found-empty-verbs">
                  <button type="button" className="btn btn-primary" onClick={chooseNew} disabled={disabled}>
                    Create a new repository instead
                  </button>
                </span>
              )}
            </div>
          </div>
        );
        break;
      case "collaborator":
      case "public": {
        const repository = resolution.repository;
        const verdict = repositoryVerdict(repository);
        const selected = selection?.kind === "existing" && selection.bundle_id === repository.bundle_id;
        answer = (
          <div className="r2-step-list">
            <div className="storage-found-list" aria-label="Repository at this address">
              <div className={`storage-found-row with-verb${selected ? " selected" : ""}`}>
                <span className="sync-storage-repository-icon"><Icon name={repositoryMark(repository)} size={14} /></span>
                <span className="storage-found-text">
                  <span className="storage-found-name">
                    {repository.address}
                    <code>{repository.repository_class === "git_project" ? "git project" : "project"}</code>
                  </span>
                  <span className={`storage-found-verdict ${verdict.tone}`}>
                    <Icon name={verdict.icon} size={12} /> {verdict.text}
                  </span>
                </span>
                <span className="storage-found-facts">
                  <span className="sync-storage-repo-chip"><span>Visibility</span><b>{capitalize(repository.visibility)}</b></span>
                  <span className="sync-storage-repo-chip"><span>Git remote</span><b>{gitRemoteWord(repository, localFingerprint)}</b></span>
                  {repositoryCautions(repository).map((caution) => (
                    <span key={caution} className="storage-found-caution"><Icon name="alert-triangle" size={12} /> {caution}</span>
                  ))}
                </span>
                <span className="storage-found-verb">
                  <button type="button" className="btn-link" onClick={() => void openExternalUrl(repository.website_url)}>
                    Open on website <Icon name="external-link" size={11} />
                  </button>
                  {selected ? (
                    <span className="storage-found-connected"><Icon name="check" size={12} /> Selected</span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={disabled || !repository.bundle_id}
                      onClick={() => onSelect(selectionFor(repository), repository)}
                    >
                      Use this
                    </button>
                  )}
                </span>
              </div>
            </div>
          </div>
        );
        break;
      }
    }
  }

  const needsAcknowledgement = selection?.kind === "existing"
    && repositoryFingerprintsRequireAcknowledgement(selection.repository_fingerprint, localFingerprint);
  const acknowledgement = needsAcknowledgement && selection?.kind === "existing" && (
    <Acknowledgement
      checked={selection.mismatch_acknowledged}
      disabled={disabled}
      onChange={(checked) => onSelect({ ...selection, mismatch_acknowledged: checked }, chosen)}
    >
      {chosen?.git_remote === "unknown" || (!selection.repository_fingerprint && !!localFingerprint)
        ? "This repository could not be verified for this project. Connect it anyway, then review its stored changes from Activity."
        : "This repository was pushed from a different Git remote than this checkout’s. Connect it anyway, then review its stored changes from Activity."}
    </Acknowledgement>
  );

  const codeField = (
    <label className="sync-folder-field compact">
      <span>Invitation code (optional)</span>
      <div>
        <input
          value={code}
          disabled={disabled || joining}
          spellCheck={false}
          autoComplete="off"
          placeholder="XXXXX-XXXXX-XXXXX-XXXXX"
          aria-invalid={join.kind === "failed" || undefined}
          onChange={(event) => setCode(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void runJoin();
            }
          }}
        />
        <button type="button" className="btn" disabled={disabled || joining || !isInvitationCodeShaped(code)} onClick={() => void runJoin()}>
          <Icon name="lock" size={12} /> {joining ? "Joining…" : "Join repository"}
        </button>
      </div>
      <small>Paste the code from the repository owner or an admin, then click Join repository. No repository address is needed.</small>
      {join.kind === "failed" && fieldError(join.message)}
    </label>
  );

  const rowClass = rows === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option";
  const copyClass = rows === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy";
  const metaClass = rows === "choice" ? "sync-bundle-choice-meta" : "sync-bundle-match-meta";
  const listClass = rows === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  const prediction = predictRepositoryAddress(account.login, projectName, repositories);
  const newSelected = selection?.kind === "new";

  return (
    <>
      <label className="sync-folder-field compact">
        <span>Repository address</span>
        <div>
          <input
            value={address}
            disabled={disabled}
            spellCheck={false}
            placeholder={`${account.host}/owner/name`}
            aria-invalid={invalidAddress || undefined}
            aria-describedby={describedBy}
            onChange={(event) => setAddress(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void runLookup();
              }
            }}
          />
          <button type="button" className="btn" disabled={disabled || looking || !address.trim()} onClick={() => void runLookup()}>
            <Icon name="search" size={12} /> {looking ? "Looking up…" : "Look up"}
          </button>
        </div>
        <small>Paste the page address from {account.host}, or enter owner/name.</small>
      </label>

      {answer}
      {codeField}
      {acknowledgement}

      {listing.state === "loading" ? (
        <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Checking repositories you belong to…</small>
      ) : listing.state === "error" ? (
        <Notice severity="danger" timing="blocking">{listing.message}</Notice>
      ) : (
        <div className={`${listClass} repo-rows`} role="radiogroup" aria-label="Repositories you belong to" aria-describedby={describedBy}>
          {offerNew && (
            <button
              type="button"
              role="radio"
              aria-checked={newSelected}
              className={`${rowClass} separate${newSelected ? " active" : ""}`}
              onClick={chooseNew}
              disabled={disabled}
            >
              <span className="sync-bundle-radio"><span /></span>
              {rows === "match-option" && <span className="setup-choice-mark" aria-hidden="true"><Icon name="git-fork" size={24} /></span>}
              <span className={copyClass}>
                <strong>New repository</strong>
                <span className="repo-new-address">{prediction.address}</span>
                {prediction.suffixed && <span>{prediction.base} is already used in this account.</span>}
              </span>
              <span className={metaClass}><small>Private</small></span>
            </button>
          )}
          {repositories.map((repository) => {
            const active = selection?.kind === "existing" && selection.bundle_id === repository.bundle_id;
            const gitMatch = repository.git_remote === "matches" && !!repository.repository_fingerprint;
            return (
              <button
                key={repository.repository_id}
                type="button"
                role="radio"
                aria-checked={active}
                className={`${rowClass}${active ? " active" : ""}`}
                onClick={() => chooseRow(repository)}
                disabled={disabled || !repository.bundle_id}
              >
                <span className="sync-bundle-radio"><span /></span>
                {rows === "match-option" && (
                  <span className="setup-choice-mark" aria-hidden="true"><Icon name={repositoryMark(repository)} size={24} /></span>
                )}
                <span className={copyClass}>
                  <strong>{repository.address}{gitMatch && <small className="sync-bundle-recommended">Git match</small>}</strong>
                  <span>{capitalize(repository.visibility)} · {capitalize(repository.role ?? "collaborator")}</span>
                </span>
                {rows === "choice" ? (
                  <span className={metaClass}>
                    <strong>Version {repository.revision}</strong>
                    <span>{repository.resources} resources</span>
                    <small>{repository.registration_only ? "not pushed yet" : formatRelativeTime(repository.updated_at)}</small>
                  </span>
                ) : (
                  <span className={metaClass}>{repository.registration_only ? "not pushed yet" : formatRelativeTime(repository.updated_at)}</span>
                )}
              </button>
            );
          })}
          {!offerNew && repositories.length === 0 && (
            <small className="sync-setup-hint">You belong to no repositories on {storage.name} yet. Paste an address above.</small>
          )}
        </div>
      )}
    </>
  );
}

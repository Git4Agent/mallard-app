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
 * repository's website address. One part, three hosts: the body of the
 * connect dialog for an existing project, the Repository band of project
 * setup once a Mallard Cloud storage is chosen, and — in `clone` mode, for
 * a project with no folder yet — New project's From Mallard Cloud.
 *
 * Top to bottom: one field in the Folder field's shape that takes an
 * address or a private repository's invitation code, its verb beside it
 * (Look up, or Join once the text is a code); what that answered, in the
 * storage discovery's own rows (a found row, the empty state, a field
 * error), or one line when the repository is already a row; then "Your
 * repositories": the account's repositories as the host's option rows,
 * named by address, New repository first when offered, or one quiet line
 * when there are none. Picking a row fills the field; resolving an address
 * selects the repository; joining with a code selects the repository it
 * belongs to. Nothing here prints a bundle id.
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

/** A new project's Git chip, before there is a checkout: the GitHub address
 *  it clones, or why it starts as an empty folder. None without Git. */
export function cloneSourceWord(repository: Pick<ManagedCloudRepository, "repository_class" | "github_url">): string | null {
  if (repository.repository_class !== "git_project") return null;
  return repository.github_url ? repository.github_url.replace(/^https:\/\//, "") : "not on GitHub";
}

/** The account's standing in one word. */
export function roleWord(repository: Pick<ManagedCloudRepository, "role">): string {
  return repository.role === null ? "Public" : capitalize(repository.role);
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

/** An invitation code rather than an address: no `/` or `.` (an address
 *  always has a `/`), and 20 characters once spaces and hyphens go. The
 *  alphabet is the service's to check, so a mistyped code gets the code's
 *  error rather than the address's. */
export function isInvitationCodeShaped(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed || /[/.]/.test(trimmed)) return false;
  return trimmed.replace(/[\s-]+/g, "").length === 20;
}

/** Whether a repository the field named is already one of the rows: then
 *  the row is the selection, and the field only points at it. */
export function inListing(
  repository: Pick<ManagedCloudRepository, "bundle_id">,
  repositories: ReadonlyArray<Pick<ManagedCloudRepository, "bundle_id">>,
): boolean {
  return !!repository.bundle_id && repositories.some((row) => row.bundle_id === repository.bundle_id);
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
  /** `connect` compares with the checkout a project already has; `clone`
   *  names one for a project that has no folder yet: no remote comparison,
   *  the Git chip says what gets cloned, and fewer words. */
  mode?: "connect" | "clone";
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
  mode = "connect",
  disabled,
  describedBy,
  onSelect,
}: Props) {
  const cloning = mode === "clone";
  const account = managedCloudAccount(storage);
  /** An address or an invitation code; the verb follows what it holds. */
  const [entry, setEntry] = useState("");
  const [lookup, setLookup] = useState<Lookup>({ kind: "idle" });
  const [join, setJoin] = useState<Join>({ kind: "idle" });
  /** The repository a code joined here: a row from then on, before the
   *  listing is read again. */
  const [joined, setJoined] = useState<ManagedCloudRepository | null>(null);
  const requestRef = useRef(0);
  useEffect(() => () => { requestRef.current += 1; }, []);

  const listed = listing.state === "ready" ? listing.repositories : [];
  const repositories = joined && !inListing(joined, listed) ? [joined, ...listed] : listed;
  const resolved = lookup.kind === "resolved"
    && (lookup.resolution.kind === "collaborator" || lookup.resolution.kind === "public")
    ? lookup.resolution.repository
    : null;
  const chosen = selection?.kind === "existing"
    ? repositories.find((repository) => repository.bundle_id === selection.bundle_id)
      ?? (resolved?.bundle_id === selection.bundle_id ? resolved : null)
    : null;

  const runLookup = async () => {
    const typed = entry.trim();
    if (!typed || disabled) return;
    const requestId = ++requestRef.current;
    setJoin({ kind: "idle" });
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

  /* Another account on the same page keeps the typed address and asks
     again with that account: an answer belongs to the account it came from.
     A code is not entered again: joining is the person's own click. */
  const storageId = storage.storage_id;
  const answeredFor = useRef(storageId);
  useEffect(() => {
    if (answeredFor.current === storageId) return;
    answeredFor.current = storageId;
    requestRef.current += 1;
    setJoin({ kind: "idle" });
    setJoined(null);
    setLookup({ kind: "idle" });
    if (entry.trim() && !isInvitationCodeShaped(entry)) void runLookup();
  }, [storageId]);

  /** Enter an invitation code: the account joins the private repository it
   *  belongs to; the field shows its address and its row is selected. */
  const runJoin = async () => {
    const typed = entry.trim();
    if (!typed || disabled) return;
    const requestId = ++requestRef.current;
    setJoin({ kind: "joining" });
    try {
      const resolution = await projectSyncApi.acceptManagedCloudInvitation(storage.storage_id, typed, localFingerprint);
      if (requestId !== requestRef.current) return;
      setJoin({ kind: "idle" });
      setLookup({ kind: "resolved", resolution });
      if ((resolution.kind === "collaborator" || resolution.kind === "public") && resolution.repository.bundle_id) {
        if (resolution.kind === "collaborator") setJoined(resolution.repository);
        setEntry(resolution.repository.address);
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
    setEntry(repository.address);
    setJoin({ kind: "idle" });
    setLookup({ kind: "idle" });
    onSelect(selectionFor(repository), repository);
  };

  const chooseNew = () => {
    requestRef.current += 1;
    setJoin({ kind: "idle" });
    setLookup({ kind: "idle" });
    onSelect({ kind: "new" }, null);
  };

  const code = isInvitationCodeShaped(entry);
  const submit = () => void (code ? runJoin() : runLookup());
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
        answer = fieldError(cloning
          ? <>That address is on {resolution.host}, not {resolution.expected}.</>
          : <>That address is on {resolution.host}; this storage is {storage.name} at {resolution.expected}.</>);
        break;
      case "github":
        answer = fieldError(cloning
          ? "That’s a GitHub address. Paste the Mallard Cloud one."
          : "That’s a GitHub address. Paste the Mallard Cloud address from the repository’s page.");
        break;
      case "invalid":
        answer = fieldError(cloning
          ? "Paste a page address, owner/name, or an invitation code."
          : <>Paste the address from the repository’s page on {account.host}, type owner/name, or paste an invitation code.</>);
        break;
      case "not_a_project":
        answer = fieldError(cloning
          ? <>{resolution.repository.address} isn’t a project.</>
          : <>{resolution.repository.address} is a skill, plugin intent, or pet, not a project.</>);
        break;
      case "not_found":
        answer = (
          <div className="r2-step-list">
            <div className="storage-found-empty" role="status">
              <Icon name="search" size={24} />
              <strong>{cloning ? `${account.login} can’t see ${resolution.address}.` : `Nothing at ${resolution.address} that you can see.`}</strong>
              <span>
                {cloning
                  ? "Check the spelling, try another account, or paste its invitation code instead."
                  : "Check the spelling. For a private repository, paste its invitation code instead. The owner or an admin has it."}
              </span>
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
        // One of the rows already: the row is the selection; say so once.
        if (inListing(repository, repositories)) {
          answer = (
            <small className="repo-answer-line" role="status">
              <Icon name="check-circle" size={13} /> In your repositories. Selected below.
            </small>
          );
          break;
        }
        // With no checkout yet there is no remote to compare: the role alone.
        const verdict = cloning
          ? { tone: "ok" as const, icon: "check-circle" as const, text: roleWord(repository) }
          : repositoryVerdict(repository);
        const source = cloning ? cloneSourceWord(repository) : null;
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
                  {cloning ? (
                    source && <span className="sync-storage-repo-chip"><span>Git</span><b>{source}</b></span>
                  ) : (
                    <>
                      <span className="sync-storage-repo-chip"><span>Git remote</span><b>{gitRemoteWord(repository, localFingerprint)}</b></span>
                      {repositoryCautions(repository).map((caution) => (
                        <span key={caution} className="storage-found-caution"><Icon name="alert-triangle" size={12} /> {caution}</span>
                      ))}
                    </>
                  )}
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

  const needsAcknowledgement = !cloning
    && selection?.kind === "existing"
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

  const rowClass = rows === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option";
  const copyClass = rows === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy";
  const metaClass = rows === "choice" ? "sync-bundle-choice-meta" : "sync-bundle-match-meta";
  const listClass = rows === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  const prediction = predictRepositoryAddress(account.login, projectName, repositories);
  const newSelected = selection?.kind === "new";

  return (
    <>
      <label className="sync-folder-field compact">
        <span>Address or invitation code</span>
        <div>
          <input
            value={entry}
            disabled={disabled || joining}
            spellCheck={false}
            autoComplete="off"
            placeholder={`${account.host}/owner/name, or XXXXX-XXXXX-XXXXX-XXXXX`}
            aria-invalid={invalidAddress || join.kind === "failed" || undefined}
            aria-describedby={describedBy}
            onChange={(event) => {
              setEntry(event.target.value);
              if (join.kind === "failed") setJoin({ kind: "idle" });
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submit();
              }
            }}
          />
          <button type="button" className="btn repo-verb" disabled={disabled || looking || joining || !entry.trim()} onClick={submit}>
            <Icon name={code ? "user" : "search"} size={12} />
            {joining ? "Joining…" : looking ? "Looking up…" : code ? "Join" : "Look up"}
          </button>
        </div>
        {code ? (
          <small>An invitation code. Join adds you to its private repository.</small>
        ) : !cloning && (
          <small>Paste the page address from {account.host} or owner/name, or a code from the repository’s owner or an admin.</small>
        )}
        {join.kind === "failed" && fieldError(join.message)}
      </label>

      {answer}
      {acknowledgement}

      <div className="repo-list-head">
        Your repositories{listing.state === "ready" && repositories.length > 0 && <small>{repositories.length}</small>}
      </div>
      {listing.state === "loading" ? (
        <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Checking repositories you belong to…</small>
      ) : listing.state === "error" ? (
        <Notice severity="danger" timing="blocking">{listing.message}</Notice>
      ) : !offerNew && repositories.length === 0 ? (
        /* No rows, no box: an empty bordered list reads as a text field. */
        <p className="repo-list-empty">None yet. Repositories you create or join appear here.</p>
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
            const gitMatch = !cloning && repository.git_remote === "matches" && !!repository.repository_fingerprint;
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
                  <span>{cloning ? roleWord(repository) : `${capitalize(repository.visibility)} · ${capitalize(repository.role ?? "collaborator")}`}</span>
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
        </div>
      )}
    </>
  );
}

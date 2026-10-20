import RepositoryLookupField, { entryKind, isInvitationCodeShaped } from "../managed-cloud/RepositoryLookupField";
import { nameRefusal, predictRepositoryAddress } from "../managed-cloud/repositoryAddress";
import { cloudRoleWord, cloudVisibilityWord } from "../managed-cloud/managedCloudLink";
import {
  CloudScopeTabs,
  ListingFoot,
  ListingRows,
  ListingStatus,
  RepoRowCopy,
  RepoRowMeta,
  RepoRowTile,
  ScopeEmpty,
  matchesFilter,
  rowAge,
  useCloudListings,
  type ScopeId,
  type ScopeTab,
} from "../managed-cloud/CloudScopes";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ManagedCloudListingRow } from "../managed-cloud/types";
import type { ParseKeys } from "i18next";
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
import { repositoryFingerprintsRequireAcknowledgement } from "./model";
import { t as translate, useTranslation } from "../../i18n";

/**
 * Deciding which Mallard Cloud repository a project connects to, by the
 * repository's website address. One part, three hosts: the body of the
 * connect dialog for an existing project, the Repository band of project
 * setup once a Mallard Cloud storage is chosen, and — in `clone` mode, for
 * a project with no folder yet — New project's From Mallard Cloud.
 *
 * Top to bottom: one field in the Folder field's shape. Words in it filter
 * the rows; an address, the page's link, or a private repository's
 * invitation code lights its verb (Look up, or Join). Under the field,
 * what a lookup answered — the storage discovery's found row for an
 * address beyond the rows, the empty state, a field error, or one line
 * when the repository is one of the rows. Then three scopes as tabs:
 * Yours (the account's repositories, New repository first when offered),
 * Starred (the website's Starred list), Public (the released projects,
 * newest first or by stars, a page at a time). Picking a Yours row
 * selects it; picking a Starred or Public row resolves its address, as a
 * typed address does, so the Git remote is compared the same way. Nothing
 * here prints a bundle id.
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

export type RepositoryListing =
  | { state: "loading" }
  | { state: "ready"; repositories: ManagedCloudRepository[]; refreshing: boolean }
  | { state: "error"; message: string };

/** The account's project repositories on one Mallard Cloud storage, read
 *  once per storage and local fingerprint, and again on `refresh`, which
 *  keeps the rows on screen until the answer lands. */
export function useManagedCloudRepositories(
  storageId: string | null,
  localFingerprint: string | null,
  isActive: () => boolean = () => true,
): { listing: RepositoryListing; refresh: () => void } {
  const [listing, setListing] = useState<RepositoryListing>({ state: "loading" });
  const [generation, setGeneration] = useState(0);
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  const lastSource = useRef<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    const source = `${storageId ?? ""}\n${localFingerprint ?? ""}`;
    const sameSource = lastSource.current === source;
    lastSource.current = source;
    if (!storageId) {
      setListing({ state: "ready", repositories: [], refreshing: false });
      return () => { cancelled = true; };
    }
    // A refresh keeps the rows; a new storage or fingerprint starts over.
    setListing((current) => (sameSource && current.state === "ready" ? { ...current, refreshing: true } : { state: "loading" }));
    void projectSyncApi.listManagedCloudRepositories(storageId, localFingerprint)
      .then((repositories) => {
        if (!cancelled && isActiveRef.current()) setListing({ state: "ready", repositories, refreshing: false });
      })
      .catch((reason) => {
        if (!cancelled && isActiveRef.current()) setListing({ state: "error", message: errorMessage(reason) });
      });
    return () => { cancelled = true; };
  }, [storageId, localFingerprint, generation]);
  const refresh = () => setGeneration((current) => current + 1);
  return { listing, refresh };
}

export function selectionFor(repository: ManagedCloudRepository): RepositorySelection {
  return {
    kind: "existing",
    bundle_id: repository.bundle_id ?? "",
    display_name: repository.display_name,
    repository_fingerprint: repository.repository_fingerprint,
    mismatch_acknowledged: false,
  };
}

/** The class mark every list of repositories wears. */
export function repositoryMark(repository: Pick<ManagedCloudRepository, "repository_class">): IconName {
  return repository.repository_class === "git_project" ? "git-folder" : "folder";
}

/** The class in two words, after the name on the setup page. */
function classWord(repositoryClass: string): string {
  return repositoryClass === "git_project" ? translate("projectSetup:picker.classGit") : translate("projectSetup:picker.classProject");
}

/** What the found row says about the repository the address named. A
 *  public repository takes pushes from anyone signed in, so a
 *  non-collaborator's row is a plain finding, not a caution. */
export function repositoryVerdict(
  repository: Pick<ManagedCloudRepository, "role" | "git_remote">,
): { tone: "ok" | "warn"; icon: IconName; text: string } {
  const text = repository.role === null
    ? translate("projectSetup:picker.verdictPublic")
    : repository.role === "owner"
      ? translate("projectSetup:picker.verdictYours", { role: cloudRoleWord(repository.role) })
      : translate("projectSetup:picker.verdictCollaborator", { role: cloudRoleWord(repository.role) });
  return repository.git_remote === "matches"
    ? { tone: "ok", icon: "check-circle", text }
    : { tone: "warn", icon: "alert-triangle", text };
}

/** The Git remote chip's word, against this checkout. */
export function gitRemoteWord(repository: Pick<ManagedCloudRepository, "git_remote" | "repository_fingerprint">, localFingerprint: string | null): string {
  switch (repository.git_remote) {
    case "matches": return localFingerprint ? translate("projectSetup:picker.remoteThis") : translate("projectSetup:picker.remoteNoneLikeFolder");
    case "differs": return repository.repository_fingerprint ? translate("projectSetup:picker.remoteDiffers") : translate("projectSetup:picker.remoteNone");
    case "unknown": return translate("projectSetup:picker.remoteUnverified");
  }
}

/** The cautions under the found row: what connecting would not give. */
export function repositoryCautions(repository: Pick<ManagedCloudRepository, "git_remote">): string[] {
  const cautions: string[] = [];
  if (repository.git_remote === "differs") cautions.push(translate("projectSetup:picker.cautionDiffers"));
  if (repository.git_remote === "unknown") cautions.push(translate("projectSetup:picker.cautionUnknown"));
  return cautions;
}

/** A new project's Git chip, before there is a checkout: the GitHub address
 *  it clones, or why it starts as an empty folder. None without Git. */
export function cloneSourceWord(repository: Pick<ManagedCloudRepository, "repository_class" | "github_url">): string | null {
  if (repository.repository_class !== "git_project") return null;
  return repository.github_url ? repository.github_url.replace(/^https:\/\//, "") : translate("projectSetup:picker.githubMissing");
}

/** The account's standing in one word. */
export function roleWord(repository: Pick<ManagedCloudRepository, "role">): string {
  return repository.role === null ? translate("cloud:scopes.public") : cloudRoleWord(repository.role);
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

/** Whether a repository the field named is already one of the rows: then
 *  the row is the selection, and the field only points at it. */
export function inListing(
  repository: Pick<ManagedCloudRepository, "bundle_id">,
  repositories: ReadonlyArray<Pick<ManagedCloudRepository, "bundle_id">>,
): boolean {
  return !!repository.bundle_id && repositories.some((row) => row.bundle_id === repository.bundle_id);
}

const SCOPE_LABEL: Record<ScopeId, ParseKeys<"projectSetup">> = {
  yours: "scope.yours", shared: "scope.shared", starred: "scope.starred", public: "scope.public",
};

interface Props {
  storage: StorageSummary;
  /** The requested name for a new repository: the project's name. */
  projectName: string;
  localFingerprint: string | null;
  listing: RepositoryListing;
  /** Reads the account's rows again; Starred and Public follow. */
  onRefresh: () => void;
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
  /** An address the page was opened for (a link from the website): the
   *  field starts holding it, and it is looked up as a pasted one is. */
  initialAddress?: string | null;
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
  onRefresh,
  selection,
  rows,
  offerNew,
  mode = "connect",
  disabled,
  describedBy,
  initialAddress = null,
  onSelect,
}: Props) {
  const { t } = useTranslation(["projectSetup", "cloud"]);
  const cloning = mode === "clone";
  const skin = rows === "choice" ? "choice" : "match";
  const account = managedCloudAccount(storage);
  /** Words to filter by, an address or a link to look up, or an
   *  invitation code; the verb follows what it holds. */
  const [entry, setEntry] = useState(initialAddress ?? "");
  const [lookup, setLookup] = useState<Lookup>({ kind: "idle" });
  const [join, setJoin] = useState<Join>({ kind: "idle" });
  /** The repository a code joined here: a row from then on, before the
   *  listing is read again. */
  const [joined, setJoined] = useState<ManagedCloudRepository | null>(null);
  /** The repository the selection names, so a listed row can wear it. */
  const [chosen, setChosen] = useState<ManagedCloudRepository | null>(null);
  const [tab, setTab] = useState<ScopeId>("yours");
  /** A Starred or Public row whose address is being resolved. */
  const [resolvingRow, setResolvingRow] = useState<string | null>(null);
  const requestRef = useRef(0);
  useEffect(() => () => { requestRef.current += 1; }, []);

  const storageId = storage.storage_id;
  const listings = useCloudListings({
    key: storageId,
    starred: () => projectSyncApi.listManagedCloudStarred(storageId, null),
    publicPage: (sort, cursor) => projectSyncApi.listManagedCloudPublic(storageId, sort, cursor),
    publicOpen: tab === "public",
  });

  const listed = listing.state === "ready" ? listing.repositories : [];
  const repositories = joined && !inListing(joined, listed) ? [joined, ...listed] : listed;
  const kind = entryKind(entry);
  const needle = kind === "filter" ? entry.trim() : "";
  const filtering = needle.length > 0;
  const fits = (row: { address: string; description: string | null; display_name: string }) => matchesFilter(needle, row.address, row.description, row.display_name);
  const yoursRows = repositories.filter(fits);
  const starredAll = listings.starred.state === "ready" ? listings.starred.rows : [];
  const publicAll = listings.public.state === "ready" ? listings.public.rows : [];
  const starredRows = starredAll.filter(fits);
  const publicRows = publicAll.filter(fits);

  /** The scope whose rows hold the repository, if one does. */
  const scopeHolding = (repositoryId: string): ScopeId | null => (
    repositories.some((row) => row.repository_id === repositoryId) ? "yours"
      : starredAll.some((row) => row.repository_id === repositoryId) ? "starred"
        : publicAll.some((row) => row.repository_id === repositoryId) ? "public"
          : null
  );

  const select = (next: RepositorySelection | null, repository: ManagedCloudRepository | null) => {
    setChosen(repository);
    onSelect(next, repository);
  };

  const runLookupFor = async (typed: string) => {
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
        const holder = scopeHolding(resolution.repository.repository_id);
        if (holder) setTab(holder);
        // The same repository again keeps its acknowledgement.
        if (selection?.kind === "existing" && selection.bundle_id === resolution.repository.bundle_id) {
          setChosen(resolution.repository);
          return;
        }
        select(selectionFor(resolution.repository), resolution.repository);
      } else if (selection?.kind === "existing") {
        select(null, null);
      }
    } catch (reason) {
      if (requestId !== requestRef.current) return;
      setLookup({ kind: "failed", message: errorMessage(reason) });
      if (selection?.kind === "existing") select(null, null);
    } finally {
      if (requestId === requestRef.current) setResolvingRow(null);
    }
  };
  const runLookup = () => runLookupFor(entry.trim());

  /* Another account on the same page keeps the typed address and asks
     again with that account: an answer belongs to the account it came from.
     A code is not entered again: joining is the person's own click. */
  const answeredFor = useRef(storageId);
  useEffect(() => {
    if (answeredFor.current === storageId) return;
    answeredFor.current = storageId;
    requestRef.current += 1;
    setJoin({ kind: "idle" });
    setJoined(null);
    setLookup({ kind: "idle" });
    setResolvingRow(null);
    if (entryKind(entry) === "address") void runLookup();
  }, [storageId]);

  /* The address the page was opened for is looked up once, with the
     account the picker opens on. Selecting it is still all that happens. */
  useEffect(() => {
    if (initialAddress) void runLookupFor(initialAddress);
  }, []);

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
        setTab("yours");
        select(selectionFor(resolution.repository), resolution.repository);
      } else if (selection?.kind === "existing") {
        select(null, null);
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
    setResolvingRow(null);
    select(selectionFor(repository), repository);
  };

  /** A Starred or Public row: one of the account's own when it is, else
   *  its address resolved, so the remote is compared as for a typed one. */
  const pickListed = (row: ManagedCloudListingRow) => {
    const twin = repositories.find((candidate) => candidate.repository_id === row.repository_id);
    if (twin) {
      chooseRow(twin);
      return;
    }
    setEntry(row.address);
    setResolvingRow(row.repository_id);
    void runLookupFor(row.address);
  };

  const chooseNew = () => {
    requestRef.current += 1;
    setJoin({ kind: "idle" });
    setLookup({ kind: "idle" });
    select({ kind: "new" }, null);
  };

  const changeEntry = (value: string) => {
    setEntry(value);
    if (join.kind === "failed") setJoin({ kind: "idle" });
    // Words that filter clear an answer about an address; the selection stays.
    const next = entryKind(value);
    if ((next === "filter" || next === "idle") && lookup.kind !== "idle") setLookup({ kind: "idle" });
  };

  const code = isInvitationCodeShaped(entry);
  const submit = () => void (code ? runJoin() : runLookup());
  const invalidAddress = lookup.kind === "resolved"
    && (lookup.resolution.kind === "wrong_service" || lookup.resolution.kind === "github" || lookup.resolution.kind === "invalid" || lookup.resolution.kind === "not_a_project");
  const looking = lookup.kind === "resolving";
  const joining = join.kind === "joining";

  const fieldError = (text: ReactNode) => <FieldMessage blocking>{text}</FieldMessage>;

  let answer: ReactNode = null;
  if (lookup.kind === "resolving" && !resolvingRow) {
    answer = (
      <small className="sync-setup-inline-status" role="status">
        <span className="status-loader" /> {t("picker.lookingUp", { address: lookup.address, host: account.host })}
      </small>
    );
  } else if (lookup.kind === "failed") {
    answer = fieldError(lookup.message);
  } else if (lookup.kind === "resolved") {
    const { resolution } = lookup;
    switch (resolution.kind) {
      case "wrong_service":
        answer = fieldError(cloning
          ? t("picker.wrongServiceClone", { host: resolution.host, expected: resolution.expected })
          : t("picker.wrongService", { host: resolution.host, storage: storage.name, expected: resolution.expected }));
        break;
      case "github":
        answer = fieldError(cloning ? t("picker.githubClone") : t("picker.github"));
        break;
      case "invalid":
        answer = fieldError(cloning ? t("picker.invalidClone") : t("picker.invalid", { host: account.host }));
        break;
      case "not_a_project":
        answer = fieldError(cloning
          ? t("picker.notProjectClone", { address: resolution.repository.address })
          : t("picker.notProject", { address: resolution.repository.address }));
        break;
      case "not_found":
        answer = (
          <div className="r2-step-list">
            <div className="storage-found-empty" role="status">
              <Icon name="search" size={24} />
              <strong>
                {cloning
                  ? t("picker.cantSee", { login: account.login, address: resolution.address })
                  : t("picker.nothingAt", { address: resolution.address })}
              </strong>
              <span>{cloning ? t("picker.checkSpellingClone") : t("picker.checkSpelling")}</span>
              {offerNew && (
                <span className="storage-found-empty-verbs">
                  <button type="button" className="btn btn-primary" onClick={chooseNew} disabled={disabled}>
                    {t("picker.createNew")}
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
        const holder = scopeHolding(repository.repository_id);
        if (holder) {
          answer = (
            <small className="repo-answer-line" role="status">
              <Icon name="check-circle" size={13} /> {t("picker.inScope", { scope: t(SCOPE_LABEL[holder]) })}
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
            <div className="storage-found-list" aria-label={t("picker.atAddress")}>
              <div className={`storage-found-row with-verb repo-answer-row${selected ? " selected" : ""}`}>
                <span className="sync-storage-repository-icon"><Icon name={repositoryMark(repository)} size={14} /></span>
                <span className="storage-found-text">
                  <span className="storage-found-name">
                    {repository.address}
                    <code>{classWord(repository.repository_class)}</code>
                  </span>
                  <span className={`storage-found-verdict ${verdict.tone}`}>
                    <Icon name={verdict.icon} size={12} /> {verdict.text}
                  </span>
                </span>
                <span className="storage-found-facts">
                  <span className="sync-storage-repo-chip"><span>{t("picker.visibility")}</span><b>{cloudVisibilityWord(repository.visibility)}</b></span>
                  {cloning ? (
                    source && <span className="sync-storage-repo-chip"><span>Git</span><b>{source}</b></span>
                  ) : (
                    <>
                      <span className="sync-storage-repo-chip"><span>{t("picker.gitRemote")}</span><b>{gitRemoteWord(repository, localFingerprint)}</b></span>
                      {repositoryCautions(repository).map((caution, index) => (
                        <span key={index} className="storage-found-caution"><Icon name="alert-triangle" size={12} /> {caution}</span>
                      ))}
                    </>
                  )}
                </span>
                <span className="storage-found-verb">
                  <button type="button" className="btn-link" onClick={() => void openExternalUrl(repository.website_url)}>
                    {t("picker.openOnWebsite")} <Icon name="external-link" size={11} />
                  </button>
                  {selected ? (
                    <span className="storage-found-connected"><Icon name="check" size={12} /> {t("picker.selected")}</span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={disabled || !repository.bundle_id}
                      onClick={() => select(selectionFor(repository), repository)}
                    >
                      {t("picker.useThis")}
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
        ? t("shared.unverified")
        : t("picker.differentRemote")}
    </Acknowledgement>
  );

  const rowClass = rows === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option";
  const copyClass = rows === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy";
  const metaClass = rows === "choice" ? "sync-bundle-choice-meta" : "sync-bundle-match-meta";
  const listClass = rows === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  const prediction = predictRepositoryAddress(account.login, projectName, repositories);
  const newSelected = selection?.kind === "new";
  const offeringNew = offerNew && !filtering;

  const ownRow = (repository: ManagedCloudRepository) => {
    const active = selection?.kind === "existing" && selection.bundle_id === repository.bundle_id;
    const gitMatch = !cloning && repository.git_remote === "matches" && !!repository.repository_fingerprint;
    const role = cloning ? roleWord(repository) : `${cloudVisibilityWord(repository.visibility)} · ${cloudRoleWord(repository.role ?? "collaborator")}`;
    return (
      <button
        key={repository.repository_id}
        type="button"
        role="radio"
        aria-checked={active}
        className={`${rowClass} repo-row${active ? " active" : ""}${repository.viewer_starred ? " starred" : ""}`}
        onClick={() => chooseRow(repository)}
        disabled={disabled || !repository.bundle_id}
      >
        <span className="sync-bundle-radio"><span /></span>
        <RepoRowTile mark={repositoryMark(repository)} pet={false} skin={skin} />
        <RepoRowCopy
          address={repository.address}
          displayName={repository.display_name}
          github={repository.github_url}
          kind={skin === "match" ? classWord(repository.repository_class) : undefined}
          badge={gitMatch ? <small className="sync-bundle-recommended">{t("setup.gitMatch")}</small> : undefined}
          blurb={repository.description ?? (skin === "match" ? role : null)}
          skin={skin}
        />
        <RepoRowMeta role={role} stars={repository.star_count} starred={repository.viewer_starred} age={rowAge(repository)} skin={skin} />
      </button>
    );
  };

  const tabs: ScopeTab[] = [
    { id: "yours", label: t(SCOPE_LABEL.yours), icon: "user", count: listing.state === "ready" ? yoursRows.length : null },
    { id: "starred", label: t(SCOPE_LABEL.starred), icon: "star", count: listings.starred.state === "ready" ? starredRows.length : null },
    { id: "public", label: t(SCOPE_LABEL.public), icon: "cloud", count: filtering && listings.public.state === "ready" ? publicRows.length : null },
  ];
  const elsewhere = (except: ScopeId) => tabs
    .filter((candidate) => candidate.id !== except && (candidate.count ?? 0) > 0)
    .map((candidate) => ({ id: candidate.id, label: candidate.label, count: candidate.count ?? 0 }));
  const noMatch = (where: ScopeId) => t("picker.noMatch", { needle, scope: t(SCOPE_LABEL[where]) });
  const selectedListedId = selection?.kind === "existing" && chosen && chosen.bundle_id === selection.bundle_id ? chosen.repository_id : null;
  const listedMark = (row: ManagedCloudListingRow): IconName => repositoryMark(row);
  const listedKind = skin === "match" ? (row: ManagedCloudListingRow) => classWord(row.repository_class) : undefined;

  let panel: ReactNode;
  if (tab === "yours") {
    if (listing.state === "loading") {
      panel = <ListingStatus listing={{ state: "loading" }} />;
    } else if (listing.state === "error") {
      panel = <Notice severity="danger" timing="blocking">{listing.message}</Notice>;
    } else if (yoursRows.length === 0 && !offeringNew) {
      panel = (
        <ScopeEmpty skin={skin} icon={filtering ? "search" : "user"} words={filtering ? noMatch("yours") : t("picker.noRepositories")}
          elsewhere={filtering ? elsewhere("yours") : undefined} onJump={setTab}
          onBrowsePublic={filtering ? undefined : () => setTab("public")} disabled={disabled} />
      );
    } else {
      panel = (
        <div className={`${listClass} repo-rows`} role="radiogroup" aria-label={t("picker.yours")} aria-describedby={describedBy}>
          {offeringNew && (
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
                <strong>{t("shared.newRepository")}</strong>
                {prediction.kind === "address" ? (
                  <>
                    <span className="repo-new-address">{prediction.address}</span>
                    {prediction.suffixed && <span>{t("picker.alreadyUsed", { name: prediction.base })}</span>}
                  </>
                ) : (
                  <span className="repo-new-refusal">{nameRefusal("project")}</span>
                )}
              </span>
              <span className={metaClass}><small>{t("cloud:visibility.private")}</small></span>
            </button>
          )}
          {yoursRows.map(ownRow)}
        </div>
      );
    }
  } else if (tab === "starred") {
    if (listings.starred.state !== "ready") {
      panel = <ListingStatus listing={listings.starred} />;
    } else if (starredRows.length === 0) {
      panel = (
        <ScopeEmpty skin={skin} icon={filtering ? "search" : "star"} words={filtering ? noMatch("starred") : t("picker.nothingStarred")}
          elsewhere={filtering ? elsewhere("starred") : undefined} onJump={setTab}
          onBrowsePublic={filtering ? undefined : () => setTab("public")} disabled={disabled} />
      );
    } else {
      panel = (
        <ListingRows rows={starredRows} skin={skin} selectedId={selectedListedId} resolvingId={resolvingRow} disabled={disabled}
          ariaLabel={t("picker.starred")} markFor={listedMark} kindFor={listedKind} onPick={pickListed} />
      );
    }
  } else if (listings.public.state !== "ready") {
    panel = <ListingStatus listing={listings.public} onRetry={listings.retryPublic} />;
  } else if (publicRows.length === 0) {
    panel = (
      <ScopeEmpty skin={skin} icon={filtering ? "search" : "cloud"} words={filtering ? noMatch("public") : t("picker.noPublic")}
        elsewhere={filtering ? elsewhere("public") : undefined} onJump={setTab} disabled={disabled} />
    );
  } else {
    panel = (
      <>
        <ListingRows rows={publicRows} skin={skin} selectedId={selectedListedId} resolvingId={resolvingRow} disabled={disabled}
          ariaLabel={t("picker.public")} markFor={listedMark} kindFor={listedKind} onPick={pickListed} />
        {!filtering && <ListingFoot listing={listings.public} onMore={listings.loadMore} disabled={disabled} />}
      </>
    );
  }

  return (
    <>
      <RepositoryLookupField value={entry} onChange={changeEntry} onSubmit={submit} disabled={disabled} looking={looking} joining={joining}
        invalid={invalidAddress || join.kind === "failed"} describedBy={describedBy}
        error={join.kind === "failed" ? fieldError(join.message) : undefined} />

      {answer}
      {acknowledgement}

      <CloudScopeTabs tabs={tabs} active={tab} disabled={disabled} onChange={setTab}
        sort={listings.sort} onSort={listings.changeSort}
        onRefresh={() => { onRefresh(); listings.refresh(); }}
        refreshing={(listing.state === "ready" && listing.refreshing) || listings.refreshing} />
      {panel}
    </>
  );
}

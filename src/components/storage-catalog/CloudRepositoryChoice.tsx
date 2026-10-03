import { useEffect, useRef, useState, type ReactNode } from "react";
import Icon, { type IconName } from "../Icons";
import RepositoryLookupField, { entryKind, isInvitationCodeShaped } from "../managed-cloud/RepositoryLookupField";
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
import { predictRepositoryAddress } from "../managed-cloud/repositoryAddress";
import Notice, { FieldMessage } from "../notice/Notice";
import { formatRelativeTime } from "../../clock";
import { errorMessage } from "../../storageError";
import type { ManagedCloudListingRow } from "../managed-cloud/types";
import { storageCatalogApi } from "./api";
import { catalogCapabilityFor, catalogPresentation, sameCatalogCoordinate } from "./model";
import type {
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogCloudResolution,
  CatalogCoordinate,
} from "./types";

/**
 * Choosing a Catalog item's Mallard Cloud repository: the project's picker
 * with fewer words. One field filters the rows, looks up an address or a
 * pasted link, or joins with an invitation code; what it answered is one
 * line or the found row. For an item's own chooser the list is the
 * repositories that hold it, a new repository first when the item may
 * have one. For Add — the row page's, or Add Catalog resource's door —
 * three scopes as tabs: Shared with you (the repositories the account
 * belongs to whose item no row lists), Starred, and Public, a page at a
 * time; a Starred or Public row is picked by resolving its address, so
 * the chooser's checks run as for a typed one. The host owns the verb.
 */

/** Who chooses. */
export type CloudChoiceTarget =
  /** An item on its page: Connect, or a choice between several. */
  | {
    kind: "item";
    coordinate: CatalogCoordinate;
    /** The kind's singular noun, lowercased: "pet v2", "skill". */
    noun: string;
    resourceId: string;
    name: string;
    offerNew: boolean;
  }
  /** The row page's Add: someone else's repository of this row's kind. */
  | {
    kind: "add";
    coordinate: CatalogCoordinate;
    noun: string;
    /** "Pets v2 · Codex home", for "already has". */
    rowLabel: string;
    /** The items this row lists already. */
    listed: ReadonlySet<string>;
  }
  /** Add Catalog resource's door: any kind; the repository brings its own. */
  | { kind: "door" };

export type CloudSelection = "new" | CatalogCloudRepository | null;

type Lookup =
  | { kind: "idle" }
  | { kind: "resolving"; address: string }
  | { kind: "found"; repository: CatalogCloudRepository; joined: boolean }
  | { kind: "not_found"; address: string }
  | { kind: "error"; message: string };

type Listing =
  | { state: "loading" }
  | { state: "ready"; rows: CatalogCloudRepository[] }
  | { state: "error"; message: string };

const capitalize = (value: string) => (value ? `${value[0].toUpperCase()}${value.slice(1)}` : value);

/** A repository's standing in a word or two: its role, and Public when
 *  anyone signed in can push to it. */
export function cloudRoleWord(repository: Pick<CatalogCloudRepository, "role" | "visibility">): string {
  if (repository.role === null) return "Public · Contributor";
  return `${repository.visibility === "public" ? "Public · " : ""}${capitalize(repository.role)}`;
}

/** Why a repository does not fit the chooser, in one sentence; `null` when
 *  it does. The backend checks the item again when it connects. */
export function cloudRepositoryMisfit(
  repository: CatalogCloudRepository,
  target: CloudChoiceTarget,
  capabilities: readonly CatalogCapabilityDescriptor[],
): string | null {
  const own = repository.role === "owner";
  if (target.kind === "door") {
    if (!repository.available) return `${repository.address} is a ${repository.kind_label} repository this app can't open.`;
    return own ? `${repository.address} is yours — already listed.` : null;
  }
  if (!sameCatalogCoordinate(repository, target.coordinate)) {
    const theirs = catalogCapabilityFor(capabilities, repository);
    if (repository.resource_kind !== target.coordinate.resource_kind) {
      const noun = (theirs?.singular_label ?? repository.kind_label).toLowerCase();
      return `${repository.address} is a ${noun}, not a ${target.noun}.`;
    }
    if (repository.provider !== target.coordinate.provider) {
      return `${repository.address} is for ${repository.provider_label}.`;
    }
    return `${repository.address} is a ${theirs?.label ?? repository.format_label} repository.`;
  }
  if (target.kind === "item") {
    return repository.resource_id === target.resourceId
      ? null
      : `${repository.address} holds ${repository.display_name}, not ${target.name}.`;
  }
  if (own) return `${repository.address} is yours — already listed.`;
  if (target.listed.has(repository.resource_id)) return `${target.rowLabel} already has ${repository.display_name}.`;
  return null;
}

/** What an answer that is not a repository says. */
function resolutionMessage(resolution: Exclude<CatalogCloudResolution, { kind: "found" } | { kind: "not_found" }>, target: CloudChoiceTarget): string {
  switch (resolution.kind) {
    case "wrong_service":
      return `That’s ${resolution.host}, not this account.`;
    case "github":
      return "That’s a GitHub address.";
    case "invalid":
      return "Paste a link, owner/name, or an invite code.";
    case "not_catalog":
      return `${resolution.address} is a project, not a ${target.kind === "door" ? "Catalog resource" : target.noun}.`;
  }
}

const SCOPE_LABEL: Record<ScopeId, string> = { yours: "Yours", shared: "Shared with you", starred: "Starred", public: "Public" };
const EMPTY_PAGE = { rows: [], next_cursor: null };

export function useCloudRepositoryChoice({ storageId, scopeId, accountLogin, target, working, capabilities = [], list: read, listKey }: {
  storageId: string;
  /** The Catalog row whose coordinate the Starred and Public listings are
   *  narrowed to; null at the door, which takes every kind. */
  scopeId: string | null;
  accountLogin: string;
  target: CloudChoiceTarget;
  /** The host's verb is running: the field and the rows hold still. */
  working: boolean;
  /** For the marks and words of other kinds. */
  capabilities?: readonly CatalogCapabilityDescriptor[];
  /** Reads the rows, once per `listKey`: the item's repositories, or Shared
   *  with you. */
  list: () => Promise<CatalogCloudRepository[]>;
  listKey: string;
}) {
  const adding = target.kind !== "item";
  const [listing, setListing] = useState<Listing>({ state: "loading" });
  const [entry, setEntry] = useState("");
  const [lookup, setLookup] = useState<Lookup>({ kind: "idle" });
  const [joining, setJoining] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);
  const [selection, setSelection] = useState<CloudSelection>(target.kind === "item" && target.offerNew ? "new" : null);
  const [tab, setTab] = useState<ScopeId>("shared");
  /** A Starred or Public row whose address is being resolved. */
  const [resolvingRow, setResolvingRow] = useState<string | null>(null);
  /** A late answer for a closed chooser, or for an earlier entry, is
   *  dropped: every request carries the number it was sent under. */
  const sequence = useRef(0);
  useEffect(() => () => { sequence.current += 1; }, []);

  const readRef = useRef(read);
  readRef.current = read;
  useEffect(() => {
    let current = true;
    setListing({ state: "loading" });
    void readRef.current()
      .then((rows) => { if (current) setListing({ state: "ready", rows }); })
      .catch((reason) => { if (current) setListing({ state: "error", message: errorMessage(reason) }); });
    return () => { current = false; };
  }, [listKey]);

  /* Starred and Public are Add's: an item's own chooser lists only the
     repositories that hold it. */
  const listings = useCloudListings({
    key: `${storageId}:${scopeId ?? "any"}:${adding ? "add" : "item"}`,
    starred: () => (adding ? storageCatalogApi.listCloudStarred(scopeId, storageId, null) : Promise.resolve(EMPTY_PAGE)),
    publicPage: (sort, cursor) => storageCatalogApi.listCloudPublic(scopeId, storageId, sort, cursor),
    publicOpen: adding && tab === "public",
  });

  const rowsAll = listing.state === "ready" ? listing.rows : [];
  const listed = rowsAll.filter((row) => cloudRepositoryMisfit(row, target, capabilities) === null);
  const joined = lookup.kind === "found" && lookup.joined && !listed.some((row) => row.repository_id === lookup.repository.repository_id)
    ? [lookup.repository]
    : [];
  const rows = [...joined, ...listed];
  const ready = listing.state === "ready";
  const offerNew = target.kind === "item" && target.offerNew && !rows.some((row) => row.role === "owner");
  const prediction = target.kind === "item" ? predictRepositoryAddress(accountLogin, target.name, rowsAll) : null;
  const chosen = selection === "new" ? null : selection;

  const kind = entryKind(entry);
  const needle = kind === "filter" ? entry.trim() : "";
  const filtering = needle.length > 0;
  const fits = (address: string, description: string | null) => matchesFilter(needle, address, description);
  const shownRows = rows.filter((row) => fits(row.address, row.description));
  const starredAll = listings.starred.state === "ready" ? listings.starred.rows : [];
  const publicAll = listings.public.state === "ready" ? listings.public.rows : [];
  const starredRows = starredAll.filter((row) => fits(row.address, row.description));
  const publicRows = publicAll.filter((row) => fits(row.address, row.description));

  /** The scope whose rows hold the repository, if one does. */
  const scopeHolding = (repositoryId: string): ScopeId | null => (
    rows.some((row) => row.repository_id === repositoryId) ? "shared"
      : starredAll.some((row) => row.repository_id === repositoryId) ? "starred"
        : publicAll.some((row) => row.repository_id === repositoryId) ? "public"
          : null
  );

  const answer = (resolution: CatalogCloudResolution, joinedHere: boolean): Lookup => {
    if (resolution.kind === "found") {
      const misfit = cloudRepositoryMisfit(resolution.repository, target, capabilities);
      return misfit ? { kind: "error", message: misfit } : { kind: "found", repository: resolution.repository, joined: joinedHere };
    }
    if (resolution.kind === "not_found") return { kind: "not_found", address: resolution.address };
    return { kind: "error", message: resolutionMessage(resolution, target) };
  };

  const submit = async (address?: string) => {
    const typed = (address ?? entry).trim();
    if (!typed || working || joining || lookup.kind === "resolving") return;
    const request = ++sequence.current;
    setJoinError(null);
    if (isInvitationCodeShaped(typed)) {
      setJoining(true);
      try {
        const resolution = await storageCatalogApi.joinCloudRepository(storageId, typed);
        if (request !== sequence.current) return;
        const next = answer(resolution, true);
        setLookup(next);
        if (next.kind === "found") {
          setEntry(next.repository.address);
          setSelection(next.repository);
          setTab("shared");
        }
      } catch (reason) {
        if (request === sequence.current) setJoinError(errorMessage(reason));
      } finally {
        if (request === sequence.current) setJoining(false);
      }
      return;
    }
    setLookup({ kind: "resolving", address: typed });
    try {
      const resolution = await storageCatalogApi.resolveCloudRepository(storageId, typed);
      if (request !== sequence.current) return;
      const next = answer(resolution, false);
      setLookup(next);
      if (next.kind === "found") {
        setSelection(next.repository);
        const holder = scopeHolding(next.repository.repository_id);
        if (holder) setTab(holder);
      } else if (selection !== "new") setSelection(null);
    } catch (reason) {
      if (request !== sequence.current) return;
      setLookup({ kind: "error", message: errorMessage(reason) });
      if (selection !== "new") setSelection(null);
    } finally {
      if (request === sequence.current) setResolvingRow(null);
    }
  };

  const choose = (next: CloudSelection, fill?: string) => {
    sequence.current += 1;
    setJoining(false);
    setJoinError(null);
    setLookup({ kind: "idle" });
    setResolvingRow(null);
    if (fill !== undefined) setEntry(fill);
    setSelection(next);
  };

  /** A Starred or Public row: one of the rows when it is, else its
   *  address resolved, so the misfit check runs as for a typed one. */
  const pickListed = (row: ManagedCloudListingRow) => {
    const twin = rows.find((candidate) => candidate.repository_id === row.repository_id);
    if (twin) {
      choose(twin, twin.address);
      return;
    }
    setEntry(row.address);
    setResolvingRow(row.repository_id);
    void submit(row.address);
  };

  const changeEntry = (value: string) => {
    setEntry(value);
    setJoinError(null);
    // Words that filter clear an answer about an address; the selection stays.
    const next = entryKind(value);
    if ((next === "filter" || next === "idle") && lookup.kind !== "idle" && lookup.kind !== "resolving") setLookup({ kind: "idle" });
  };

  const markFor = (repository: CatalogCloudRepository) =>
    catalogPresentation(catalogCapabilityFor(capabilities, repository)).icon;
  const kindFor = (repository: CatalogCloudRepository) =>
    catalogCapabilityFor(capabilities, repository)?.singular_label ?? repository.kind_label;
  const listedCapability = (row: ManagedCloudListingRow) => (row.catalog ? catalogCapabilityFor(capabilities, row.catalog) : null);
  const listedMark = (row: ManagedCloudListingRow): IconName => catalogPresentation(listedCapability(row)).icon;
  const listedKind = (row: ManagedCloudListingRow) => listedCapability(row)?.singular_label ?? row.catalog?.resource_kind ?? null;
  const door = target.kind === "door";
  const skin = door ? "match" : "choice";

  const field = (
    <RepositoryLookupField value={entry} labelHidden
      onChange={changeEntry}
      onSubmit={() => void submit()} disabled={working}
      looking={lookup.kind === "resolving"} joining={joining}
      invalid={lookup.kind === "error" || joinError !== null}
      error={joinError ? <FieldMessage blocking>{joinError}</FieldMessage> : undefined} />
  );

  let found: ReactNode = null;
  if (lookup.kind === "resolving") {
    /* The verb's own busy label is the progress; this only announces it. */
    found = <span className="sync-visually-hidden" role="status">Looking up {lookup.address}…</span>;
  } else if (lookup.kind === "error") {
    found = <FieldMessage blocking>{lookup.message}</FieldMessage>;
  } else if (lookup.kind === "not_found") {
    found = (
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="search" size={24} />
          <strong>Nothing at {lookup.address}.</strong>
          <span>Check the spelling, or paste the link from its page.</span>
          {offerNew && (
            <span className="storage-found-empty-verbs">
              <button type="button" className="btn btn-primary" disabled={working} onClick={() => choose("new")}>Use a new repository</button>
            </span>
          )}
        </div>
      </div>
    );
  } else if (lookup.kind === "found" && scopeHolding(lookup.repository.repository_id)) {
    const holder = scopeHolding(lookup.repository.repository_id)!;
    found = (
      <small className="repo-answer-line" role="status">
        <Icon name="check-circle" size={13} /> {lookup.joined ? `Joined ${lookup.repository.address}. Selected below.` : `${adding ? `In ${SCOPE_LABEL[holder]}. ` : ""}Selected below.`}
      </small>
    );
  } else if (lookup.kind === "found") {
    const repository = lookup.repository;
    const selected = chosen?.repository_id === repository.repository_id;
    found = (
      <div className="r2-step-list">
        <div className="storage-found-list" aria-label="Repository at this address">
          <div className={`storage-found-row with-verb${selected ? " selected" : ""}`}>
            <span className="sync-storage-repository-icon"><Icon name={markFor(repository)} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">{repository.address}</span>
              <span className="storage-found-verdict ok">
                <Icon name="check-circle" size={12} /> {cloudRoleWord(repository)}{repository.registration_only ? "" : ` · v${repository.revision}`}
              </span>
            </span>
            <span className="storage-found-verb">
              {selected
                ? <span className="storage-found-connected"><Icon name="check" size={12} /> Selected</span>
                : <button type="button" className="btn btn-primary" disabled={working} onClick={() => setSelection(repository)}>Use this</button>}
            </span>
          </div>
        </div>
      </div>
    );
  }

  /** A row of the account's own: the shared anatomy, the pet's tile. */
  const ownRow = (row: CatalogCloudRepository) => {
    const active = chosen?.repository_id === row.repository_id;
    const role = cloudRoleWord(row);
    return (
      <button key={row.repository_id} type="button" role="radio" aria-checked={active}
        className={`${skin === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option"} repo-row${active ? " active" : ""}${row.viewer_starred ? " starred" : ""}`}
        disabled={working} onClick={() => choose(row, row.address)}>
        <span className="sync-bundle-radio"><span /></span>
        <RepoRowTile mark={markFor(row)} pet={row.resource_kind === "desktop_pet"} skin={skin} />
        <RepoRowCopy address={row.address} kind={door ? kindFor(row) : undefined} skin={skin}
          blurb={row.description ?? (skin === "match" ? role : null)} />
        <RepoRowMeta role={role} stars={row.star_count} starred={row.viewer_starred} age={rowAge(row)} skin={skin} />
      </button>
    );
  };

  let choices: ReactNode;
  if (!adding) {
    /* An item's own chooser: the repositories that hold it, a new one first. */
    choices = listing.state === "loading" ? (
      <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Loading repositories…</small>
    ) : listing.state === "error" ? (
      <Notice severity="danger" timing="blocking">{listing.message}</Notice>
    ) : !offerNew && shownRows.length === 0 ? (
      <p className="repo-list-empty">{filtering ? `No match for “${needle}”.` : "No repositories yet."}</p>
    ) : (
      <div className="sync-bundle-choice-list repo-rows" role="radiogroup" aria-label={`Repositories for ${target.name}`}>
        {offerNew && prediction && !filtering && (
          <button type="button" role="radio" aria-checked={selection === "new"}
            className={`sync-bundle-choice separate${selection === "new" ? " active" : ""}`} disabled={working}
            onClick={() => choose("new")}>
            <span className="sync-bundle-radio"><span /></span>
            <span className="sync-bundle-choice-copy">
              <strong>{prediction.address}<small className="sync-bundle-recommended">New</small></strong>
              {prediction.suffixed && <span>{prediction.base} is taken.</span>}
            </span>
            <span className="sync-bundle-choice-meta"><small>Private</small></span>
          </button>
        )}
        {shownRows.map((row) => {
          const active = chosen?.repository_id === row.repository_id;
          return (
            <button key={row.repository_id} type="button" role="radio" aria-checked={active}
              className={`sync-bundle-choice${active ? " active" : ""}`} disabled={working}
              onClick={() => choose(row, row.address)}>
              <span className="sync-bundle-radio"><span /></span>
              <span className="sync-bundle-choice-copy">
                <strong>{row.address}</strong>
                <span>{cloudRoleWord(row)}</span>
              </span>
              <span className="sync-bundle-choice-meta">
                {!row.registration_only && <strong>v{row.revision}</strong>}
                <small>{row.registration_only ? "not pushed yet" : formatRelativeTime(row.updated_at)}</small>
              </span>
            </button>
          );
        })}
      </div>
    );
  } else {
    const tabs: ScopeTab[] = [
      { id: "shared", label: "Shared with you", icon: "cloud-user", count: ready ? shownRows.length : null },
      { id: "starred", label: "Starred", icon: "star", count: listings.starred.state === "ready" ? starredRows.length : null },
      { id: "public", label: "Public", icon: "cloud", count: filtering && listings.public.state === "ready" ? publicRows.length : null },
    ];
    const elsewhere = (except: ScopeId) => tabs
      .filter((candidate) => candidate.id !== except && (candidate.count ?? 0) > 0)
      .map((candidate) => ({ id: candidate.id, label: candidate.label, count: candidate.count ?? 0 }));
    const noMatch = (where: ScopeId) => `No match for “${needle}” in ${SCOPE_LABEL[where]}.`;
    const noun = target.kind === "add" ? target.noun : "Catalog resource";
    const selectedListedId = chosen?.repository_id ?? null;
    let panel: ReactNode;
    if (tab === "shared") {
      if (listing.state === "loading") {
        panel = <ListingStatus listing={{ state: "loading" }} />;
      } else if (listing.state === "error") {
        panel = <Notice severity="danger" timing="blocking">{listing.message}</Notice>;
      } else if (shownRows.length === 0) {
        panel = (
          <ScopeEmpty skin={skin} icon={filtering ? "search" : "cloud-user"} words={filtering ? noMatch("shared") : "Nothing shared with you yet."}
            elsewhere={filtering ? elsewhere("shared") : undefined} onJump={setTab}
            onBrowsePublic={filtering ? undefined : () => setTab("public")} disabled={working} />
        );
      } else {
        panel = (
          <div className={`${skin === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options"} repo-rows`} role="radiogroup" aria-label="Shared with you">
            {shownRows.map(ownRow)}
          </div>
        );
      }
    } else if (tab === "starred") {
      if (listings.starred.state !== "ready") {
        panel = <ListingStatus listing={listings.starred} />;
      } else if (starredRows.length === 0) {
        panel = (
          <ScopeEmpty skin={skin} icon={filtering ? "search" : "star"} words={filtering ? noMatch("starred") : "Nothing starred yet."}
            elsewhere={filtering ? elsewhere("starred") : undefined} onJump={setTab}
            onBrowsePublic={filtering ? undefined : () => setTab("public")} disabled={working} />
        );
      } else {
        panel = (
          <ListingRows rows={starredRows} skin={skin} selectedId={selectedListedId} resolvingId={resolvingRow} disabled={working}
            ariaLabel="Starred" markFor={listedMark} kindFor={door ? listedKind : undefined} onPick={pickListed} />
        );
      }
    } else if (listings.public.state !== "ready") {
      panel = <ListingStatus listing={listings.public} onRetry={listings.retryPublic} />;
    } else if (publicRows.length === 0) {
      panel = (
        <ScopeEmpty skin={skin} icon={filtering ? "search" : "cloud"} words={filtering ? noMatch("public") : `No public ${noun}s yet.`}
          elsewhere={filtering ? elsewhere("public") : undefined} onJump={setTab} disabled={working} />
      );
    } else {
      panel = (
        <>
          <ListingRows rows={publicRows} skin={skin} selectedId={selectedListedId} resolvingId={resolvingRow} disabled={working}
            ariaLabel="Public" markFor={listedMark} kindFor={door ? listedKind : undefined} onPick={pickListed} />
          {!filtering && <ListingFoot listing={listings.public} onMore={listings.loadMore} disabled={working} />}
        </>
      );
    }
    choices = (
      <>
        <CloudScopeTabs tabs={tabs} active={tab} disabled={working} onChange={setTab} sort={listings.sort} onSort={listings.changeSort} />
        {panel}
      </>
    );
  }

  return { selection, chosen, ready, rows, field, found, list: choices };
}

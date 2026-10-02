import { useEffect, useRef, useState, type ReactNode } from "react";
import Icon from "../Icons";
import RepositoryLookupField, { isInvitationCodeShaped } from "../managed-cloud/RepositoryLookupField";
import { predictRepositoryAddress } from "../managed-cloud/repositoryAddress";
import Notice, { FieldMessage } from "../notice/Notice";
import { formatRelativeTime } from "../../clock";
import { errorMessage } from "../../storageError";
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
 * with fewer words. One field takes an address or an invitation code (Look
 * up, or Join); what it answered is one line; then the list, with no
 * heading: a new repository first when the item may have one, then the
 * repositories the account belongs to. The connect dialog and Add Catalog
 * resource's From Mallard Cloud door share it; the host owns the verb.
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
      return "Type owner/name, or paste an invite code.";
    case "not_catalog":
      return `${resolution.address} is a project, not a ${target.kind === "door" ? "Catalog resource" : target.noun}.`;
  }
}

export function useCloudRepositoryChoice({ storageId, accountLogin, target, working, capabilities = [], list: read, listKey }: {
  storageId: string;
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

  const answer = (resolution: CatalogCloudResolution, joinedHere: boolean): Lookup => {
    if (resolution.kind === "found") {
      const misfit = cloudRepositoryMisfit(resolution.repository, target, capabilities);
      return misfit ? { kind: "error", message: misfit } : { kind: "found", repository: resolution.repository, joined: joinedHere };
    }
    if (resolution.kind === "not_found") return { kind: "not_found", address: resolution.address };
    return { kind: "error", message: resolutionMessage(resolution, target) };
  };

  const submit = async () => {
    const typed = entry.trim();
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
      if (next.kind === "found") setSelection(next.repository);
      else if (selection !== "new") setSelection(null);
    } catch (reason) {
      if (request !== sequence.current) return;
      setLookup({ kind: "error", message: errorMessage(reason) });
      if (selection !== "new") setSelection(null);
    }
  };

  const choose = (next: CloudSelection, fill?: string) => {
    sequence.current += 1;
    setJoining(false);
    setJoinError(null);
    setLookup({ kind: "idle" });
    if (fill !== undefined) setEntry(fill);
    setSelection(next);
  };

  const markFor = (repository: CatalogCloudRepository) =>
    catalogPresentation(catalogCapabilityFor(capabilities, repository)).icon;
  const kindFor = (repository: CatalogCloudRepository) =>
    catalogCapabilityFor(capabilities, repository)?.singular_label ?? repository.kind_label;

  const field = (
    <RepositoryLookupField value={entry} labelHidden
      onChange={(value) => { setEntry(value); setJoinError(null); }}
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
          <span>Check the spelling, or paste an invite code.</span>
          {offerNew && (
            <span className="storage-found-empty-verbs">
              <button type="button" className="btn btn-primary" disabled={working} onClick={() => choose("new")}>Use a new repository</button>
            </span>
          )}
        </div>
      </div>
    );
  } else if (lookup.kind === "found" && rows.some((row) => row.repository_id === lookup.repository.repository_id)) {
    found = (
      <small className="repo-answer-line" role="status">
        <Icon name="check-circle" size={13} /> {lookup.joined ? `Joined ${lookup.repository.address}. Selected below.` : "Selected below."}
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

  const door = target.kind === "door";
  const choices = (
    <>
      {adding && <div className="repo-list-head">Shared with you{ready && rows.length > 0 && <small>{rows.length}</small>}</div>}
      {listing.state === "loading" ? (
        <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Loading repositories…</small>
      ) : listing.state === "error" ? (
        <Notice severity="danger" timing="blocking">{listing.message}</Notice>
      ) : !offerNew && rows.length === 0 ? (
        <p className="repo-list-empty">{adding ? "Nothing shared with you yet." : "No repositories yet."}</p>
      ) : door ? (
        /* New project's rows: square radios and a mark, here the kind's. */
        <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Shared with you">
          {rows.map((row) => {
            const active = chosen?.repository_id === row.repository_id;
            return (
              <button key={row.repository_id} type="button" role="radio" aria-checked={active}
                className={`sync-bundle-match-option${active ? " active" : ""}`} disabled={working}
                onClick={() => choose(row, row.address)}>
                <span className="sync-bundle-radio"><span /></span>
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={markFor(row)} size={24} /></span>
                <span className="sync-bundle-match-copy">
                  <strong>{row.address}</strong>
                  <span>{kindFor(row)} · {cloudRoleWord(row)}</span>
                </span>
                <span className="sync-bundle-match-meta">{row.registration_only ? "not pushed yet" : formatRelativeTime(row.updated_at)}</span>
              </button>
            );
          })}
        </div>
      ) : (
        <div className="sync-bundle-choice-list repo-rows" role="radiogroup"
          aria-label={target.kind === "item" ? `Repositories for ${target.name}` : "Shared with you"}>
          {offerNew && prediction && (
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
          {rows.map((row) => {
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
      )}
    </>
  );

  return { selection, chosen, ready, rows, field, found, list: choices };
}

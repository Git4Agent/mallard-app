import { useEffect, useRef, useState, type ReactNode } from "react";
import { nameKey } from "./repositoryAddress";
import { cloudRoleWord } from "./managedCloudLink";
import type { ManagedCloudListingPage, ManagedCloudListingRow, ManagedCloudPublicSort } from "./types";
import { formatRelativeTime } from "../../clock";
import { errorMessage } from "../../storageError";
import Icon, { type IconName } from "../Icons";
import Notice from "../notice/Notice";
import { t as translate, useTranslation } from "../../i18n";

/**
 * The parts both Mallard Cloud pickers share once they look beyond the
 * repositories the account belongs to: the scope tabs (the review strip's
 * tabs at the dialog's measure), the Starred and Public listings with
 * their pages and sort, the rows those listings draw, the empty box and
 * the Show-more foot. The hosts keep their own selection: a listed row is
 * picked by resolving its address, as a typed address is.
 */

export type ScopeId = "yours" | "shared" | "starred" | "public";

export interface ScopeTab {
  id: ScopeId;
  label: string;
  icon: IconName;
  /** Null when there is no number to say: Public, before its pages are read. */
  count: number | null;
}

/** Whether the filter's words are in the address or the description. */
/** Whether a row answers the filter: its address, its display name or its
 *  description holds the words, compared by key (NFC, no case), so `大`
 *  finds 大黄 in any spelling. */
export function matchesFilter(needle: string, address: string, description: string | null | undefined, displayName?: string | null): boolean {
  const wanted = nameKey(needle);
  if (!wanted) return true;
  return nameKey(address).includes(wanted) || nameKey(displayName ?? "").includes(wanted) || nameKey(description ?? "").includes(wanted);
}

/** The account's standing on a listed repository, in a word or two. */
export function listingRoleWord(row: Pick<ManagedCloudListingRow, "role" | "visibility">): string {
  if (row.role === null) return translate("cloud:scopes.public");
  const role = cloudRoleWord(row.role);
  return row.visibility === "public" ? translate("cloud:scopes.publicRole", { role }) : role;
}

export function CloudScopeTabs({ tabs, active, disabled, onChange, sort, onSort }: {
  tabs: readonly ScopeTab[];
  active: ScopeId;
  disabled?: boolean;
  onChange: (id: ScopeId) => void;
  /** Public's order, drawn at the strip's end while Public is open. */
  sort?: ManagedCloudPublicSort;
  onSort?: (sort: ManagedCloudPublicSort) => void;
}) {
  const { t } = useTranslation("cloud");
  return (
    <div className="sync-project-tabs sync-sync-review-tabs repo-scope-tabs" role="tablist" aria-label={t("scopes.tabs")}>
      {tabs.map((tab) => {
        const current = tab.id === active;
        return (
          <button key={tab.id} type="button" role="tab" aria-selected={current} tabIndex={current ? 0 : -1}
            className={current ? "active" : undefined} disabled={disabled}
            onClick={() => onChange(tab.id)}
            onKeyDown={(event) => {
              const index = tabs.findIndex((candidate) => candidate.id === tab.id);
              const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
              if (!step) return;
              event.preventDefault();
              const next = tabs[(index + step + tabs.length) % tabs.length];
              onChange(next.id);
              (event.currentTarget.parentElement?.querySelector(`[data-scope="${next.id}"]`) as HTMLButtonElement | null)?.focus();
            }}
            data-scope={tab.id}>
            <Icon name={tab.icon} size={14} />
            {tab.label}
            {tab.count !== null && tab.count > 0 && <small>{tab.count}</small>}
          </button>
        );
      })}
      {active === "public" && sort && onSort && (
        <div className="sync-segmented repo-scope-sort" role="radiogroup" aria-label={t("scopes.sort")}>
          <button type="button" role="radio" aria-checked={sort === "newest"} className={sort === "newest" ? "active" : undefined} disabled={disabled} onClick={() => onSort("newest")}>{t("scopes.newest")}</button>
          <button type="button" role="radio" aria-checked={sort === "stars"} className={sort === "stars" ? "active" : undefined} disabled={disabled} onClick={() => onSort("stars")}>{t("scopes.mostStars")}</button>
        </div>
      )}
    </div>
  );
}

export type Listing =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready"; rows: ManagedCloudListingRow[]; next: string | null; loadingMore: boolean }
  | { state: "error"; message: string };

const NOT_LOADED: Listing = { state: "idle" };

/** Starred is read when the picker opens; Public when its tab first opens,
 *  and again when its sort changes. Every answer carries the request it
 *  was sent under, so a late one for an earlier key is dropped. */
export function useCloudListings({ key, starred, publicPage, publicOpen }: {
  key: string;
  starred: () => Promise<ManagedCloudListingPage>;
  publicPage: (sort: ManagedCloudPublicSort, cursor: string | null) => Promise<ManagedCloudListingPage>;
  publicOpen: boolean;
}) {
  const [starredListing, setStarred] = useState<Listing>(NOT_LOADED);
  const [publicListing, setPublic] = useState<Listing>(NOT_LOADED);
  const [sort, setSort] = useState<ManagedCloudPublicSort>("newest");
  const sequence = useRef(0);
  const starredRef = useRef(starred);
  starredRef.current = starred;
  const publicRef = useRef(publicPage);
  publicRef.current = publicPage;

  useEffect(() => {
    const request = ++sequence.current;
    setStarred({ state: "loading" });
    setPublic(NOT_LOADED);
    void starredRef.current()
      .then((page) => { if (request === sequence.current) setStarred({ state: "ready", rows: page.rows, next: page.next_cursor, loadingMore: false }); })
      .catch((reason) => { if (request === sequence.current) setStarred({ state: "error", message: errorMessage(reason) }); });
    return () => { sequence.current += 1; };
  }, [key]);

  const publicWanted = publicOpen && publicListing.state === "idle";
  useEffect(() => {
    if (!publicWanted) return;
    const request = sequence.current;
    setPublic({ state: "loading" });
    void publicRef.current(sort, null)
      .then((page) => { if (request === sequence.current) setPublic({ state: "ready", rows: page.rows, next: page.next_cursor, loadingMore: false }); })
      .catch((reason) => { if (request === sequence.current) setPublic({ state: "error", message: errorMessage(reason) }); });
  }, [publicWanted, sort]);

  const changeSort = (next: ManagedCloudPublicSort) => {
    if (next === sort) return;
    setSort(next);
    setPublic(NOT_LOADED);
  };

  const loadMore = () => {
    if (publicListing.state !== "ready" || !publicListing.next || publicListing.loadingMore) return;
    const request = sequence.current;
    const cursor = publicListing.next;
    setPublic({ ...publicListing, loadingMore: true });
    void publicRef.current(sort, cursor)
      .then((page) => {
        if (request !== sequence.current) return;
        setPublic((current) => (current.state === "ready"
          ? { state: "ready", rows: [...current.rows, ...page.rows], next: page.next_cursor, loadingMore: false }
          : current));
      })
      .catch((reason) => { if (request === sequence.current) setPublic({ state: "error", message: errorMessage(reason) }); });
  };

  const retryPublic = () => setPublic(NOT_LOADED);

  return { starred: starredListing, public: publicListing, sort, changeSort, loadMore, retryPublic };
}

/* ——— the row's parts: one anatomy for a full repository and a listed one ——— */

/** The tile: the Catalog card's ink-boxed preview for a pet (its image comes
 *  in a later layer), the found row's grey tile for every other kind; on
 *  the setup page, the bare 24px mark setup's rows wear. */
export function RepoRowTile({ mark, pet, skin }: { mark: IconName; pet: boolean; skin: "choice" | "match" }) {
  if (pet) {
    return <span className="catalog-pet-preview repo-row-tile" aria-hidden="true"><Icon name={mark} size={16} /></span>;
  }
  if (skin === "match") {
    return <span className="setup-choice-mark" aria-hidden="true"><Icon name={mark} size={24} /></span>;
  }
  return <span className="sync-storage-repository-icon" aria-hidden="true"><Icon name={mark} size={14} /></span>;
}

/** A row's words: the display name leads (the project's title as it was
 *  pushed, a pet's name), the address sits beneath it with the description
 *  or the role word; with no display name the address leads, as a row that
 *  only knows its address does. */
export function RepoRowCopy({ address, displayName, github, kind, badge, blurb, skin }: {
  address: string;
  /** The display name of the head the account may read, when the row knows it. */
  displayName?: string | null;
  github?: string | null;
  /** The kind, printed after the name on the setup page (`git project`). */
  kind?: string | null;
  badge?: ReactNode;
  /** The second line: the description, else the role word. */
  blurb: ReactNode;
  skin: "choice" | "match";
}) {
  const { t } = useTranslation("cloud");
  return (
    <span className={skin === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy"}>
      <strong>
        <span>{displayName || address}</span>
        {badge}
        {github && <Icon name="git-branch" size={12} aria-label={t("scopes.onGitHub")} />}
        {kind && <code>{kind}</code>}
      </strong>
      <span className="repo-row-blurb">
        {displayName ? <span className="repo-row-address">{address}</span> : null}
        {displayName && blurb ? <i className="repo-row-sep" aria-hidden="true">·</i> : null}
        {blurb}
      </span>
    </span>
  );
}

export function RepoRowMeta({ role, stars, starred, age, skin }: {
  role: string;
  stars: number;
  starred: boolean;
  age: string;
  skin: "choice" | "match";
}) {
  if (skin === "match") {
    return <span className="sync-bundle-match-meta">{age}</span>;
  }
  return (
    <span className="sync-bundle-choice-meta">
      <strong>{role}</strong>
      <small>
        {stars > 0 && <><Icon name={starred ? "star-filled" : "star"} size={11} /> {stars} · </>}
        {age}
      </small>
    </span>
  );
}

export const rowAge = (row: { registration_only?: boolean; updated_at: number }) =>
  (row.registration_only ? translate("cloud:scopes.notPushedYet") : formatRelativeTime(row.updated_at));

/** The Starred and Public rows: the same anatomy as the account's own,
 *  picked by address. */
export function ListingRows({ rows, skin, selectedId, resolvingId, disabled, ariaLabel, markFor, kindFor, onPick }: {
  rows: ManagedCloudListingRow[];
  skin: "choice" | "match";
  selectedId: string | null;
  /** The row whose address is being resolved. */
  resolvingId: string | null;
  disabled: boolean;
  ariaLabel: string;
  markFor: (row: ManagedCloudListingRow) => IconName;
  /** The kind word after the name, where the host prints one. */
  kindFor?: (row: ManagedCloudListingRow) => string | null;
  onPick: (row: ManagedCloudListingRow) => void;
}) {
  const { t } = useTranslation("cloud");
  const listClass = skin === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  const rowClass = skin === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option";
  return (
    <div className={`${listClass} repo-rows`} role="radiogroup" aria-label={ariaLabel}>
      {rows.map((row) => {
        const active = selectedId === row.repository_id;
        const resolving = resolvingId === row.repository_id;
        const role = listingRoleWord(row);
        return (
          <button key={row.repository_id} type="button" role="radio" aria-checked={active} aria-busy={resolving || undefined}
            className={`${rowClass} repo-row${active ? " active" : ""}${row.viewer_starred ? " starred" : ""}`}
            disabled={disabled} onClick={() => onPick(row)}>
            <span className="sync-bundle-radio"><span /></span>
            <RepoRowTile mark={markFor(row)} pet={row.catalog?.resource_kind === "desktop_pet"} skin={skin} />
            <RepoRowCopy address={row.address} displayName={row.display_name} github={row.github_url} kind={kindFor?.(row)} skin={skin}
              blurb={row.description ?? (skin === "match" ? role : null)}
              badge={resolving ? <small className="sync-bundle-recommended">{t("scopes.lookingUp")}</small> : undefined} />
            <RepoRowMeta role={role} stars={row.star_count} starred={row.viewer_starred} age={rowAge(row)} skin={skin} />
          </button>
        );
      })}
    </div>
  );
}

/** A tab with nothing in it says what it would hold and offers the other
 *  way in; a filter with no match here names where the matches are. */
export function ScopeEmpty({ skin, icon, words, elsewhere, onJump, onBrowsePublic, disabled }: {
  skin: "choice" | "match";
  icon: IconName;
  words: string;
  /** Where a filter's matches are, each a way over. */
  elsewhere?: Array<{ id: ScopeId; label: string; count: number }>;
  onJump?: (id: ScopeId) => void;
  onBrowsePublic?: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation("cloud");
  const listClass = skin === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  return (
    <div className={listClass}>
      <div className="storage-found-empty repo-scope-empty" role="status">
        <Icon name={icon} size={24} />
        <strong>{words}</strong>
        {elsewhere && elsewhere.length > 0 && (
          <span>
            {elsewhere.map((scope, index) => (
              <span key={scope.id}>
                {index > 0 && " · "}
                <button type="button" className="btn-link" disabled={disabled} onClick={() => onJump?.(scope.id)}>{t("scopes.countIn", { count: scope.count, scope: scope.label })}</button>
              </span>
            ))}
          </span>
        )}
        {onBrowsePublic && (
          <span className="storage-found-empty-verbs">
            <button type="button" className="btn" disabled={disabled} onClick={onBrowsePublic}><Icon name="cloud" size={13} /> {t("scopes.browsePublic")}</button>
          </span>
        )}
      </div>
    </div>
  );
}

/** The next page: the Activity page's Load-earlier foot, under the list. */
export function ListingFoot({ listing, onMore, disabled }: { listing: Listing; onMore: () => void; disabled?: boolean }) {
  const { t } = useTranslation(["cloud", "common"]);
  if (listing.state !== "ready" || !listing.next) return null;
  return (
    <div className="sync-history-rail-foot no-rail repo-list-foot">
      <span className="sync-history-rail-foot-node" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
      <button type="button" className="sync-history-rail-step" disabled={disabled || listing.loadingMore} aria-busy={listing.loadingMore || undefined} onClick={onMore}>
        <strong>{listing.loadingMore ? t("common:loading") : t("scopes.showMore")}</strong>
      </button>
    </div>
  );
}

/** A listing that is not ready yet: the shipped inline status, or the
 *  blocking notice with its one retry. */
export function ListingStatus({ listing, onRetry }: { listing: Listing; onRetry?: () => void }) {
  const { t } = useTranslation("common");
  if (listing.state === "loading" || listing.state === "idle") {
    return <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> {t("loading")}</small>;
  }
  if (listing.state === "error") {
    return (
      <Notice severity="danger" timing="blocking">
        {listing.message}
        {onRetry && <> <button type="button" className="btn-link" onClick={onRetry}>{t("tryAgain")}</button></>}
      </Notice>
    );
  }
  return null;
}

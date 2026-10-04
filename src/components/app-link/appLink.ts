import type {
  AppLinkAccount,
  AppLinkAnswer,
  AppLinkArrival,
  AppLinkRepository,
  LocalProjectSummary,
  ProjectStorageLink,
} from "../../types";
import type { IconName } from "../Icons";
import { catalogCapabilityFor, catalogChoiceKey, catalogPresentation, catalogScopeLabel, sameCatalogCoordinate } from "../storage-catalog/model";
import type { CatalogCapabilityDescriptor, CatalogCloudPlace, CatalogScopeView } from "../storage-catalog/types";

/**
 * Open in Mallard: where a `mallard://` link lands
 * (docs/PLAN_DEEP_LINK_DESTINATIONS.md, PLAN_DEEP_LINK_SHARE_YOURS.md,
 * PLAN_DEEP_LINK_SESSION_PULL.md).
 *
 * A repository's link: the service said what each account on the link's host
 * sees at the address; this joins that with what is on this computer and
 * names one destination, which the shell follows when the person presses
 * the sheet's verb. A session's link is its repository's with one session
 * named: the same join, and a verb that opens that session's Pull review. A
 * Catalog item's page is its repository's, so its link pulls the item. The
 * listing's link has nothing to ask and nothing to choose between: with an
 * account on its host it goes straight to Projects or the Catalog. Either
 * way this decides and nothing more.
 */

/** A link that goes nowhere: why, and the one way forward when there is one. */
export interface LinkStop {
  kind: "stop";
  severity: "info" | "warning" | "danger";
  message: string;
  /** `set_up`: Mallard Cloud's sign-in, then the link again. `retry`: ask again. */
  action: "set_up" | "retry" | null;
}

/** What the service found, as the sheet's row says it. */
export interface LinkFound {
  address: string;
  mark: IconName;
  /** The account's standing, in the pickers' words. */
  standing: string;
  /** What the link names beyond the repository, when it names more. */
  sub: string | null;
  /** Where it stands on this computer. */
  here: string;
}

export type LinkDestination =
  /** The project that repository is connected to, on that storage. */
  | { kind: "project"; projectId: string; storageId: string }
  /** That project's Pull review of one session, named by its stored row's id. */
  | { kind: "pull_session"; projectId: string; storageId: string; resourceId: string }
  /** New project › From Mallard Cloud. No account ticked when several see it. */
  | { kind: "new_project"; storageId: string | null; address: string }
  /** The item's page in the Catalog row that holds it, its Pull review open. */
  | { kind: "catalog_item"; scopeId: string; storageId: string; resourceId: string }
  /** Add Catalog resource › From Mallard Cloud; Add then opens the item's Pull review. */
  | { kind: "add_catalog_item"; storageId: string; address: string }
  /** Add Catalog resource, its Kind chosen: the account's own item, and no
   *  row of its kind. The row, linked to that account, then opens on the
   *  item's Pull review. */
  | { kind: "add_catalog_row"; choiceKey: string; storageId: string; resourceId: string }
  /** The Projects page: where a project is picked to push. */
  | { kind: "projects" }
  /** The Catalog: where a skill, plugin or pet row is picked to push. */
  | { kind: "catalog" };

export interface LinkLanding {
  kind: "landing";
  found: LinkFound;
  verb: string;
  destination: LinkDestination;
}

export type LinkVerdict = LinkStop | LinkLanding;

/** A link with nothing to say first: it goes straight to its page. */
export interface LinkDirect {
  kind: "direct";
  destination: LinkDestination;
}

/** What this computer holds, for the join. */
export interface LinkContext {
  links: readonly Pick<ProjectStorageLink, "local_project_id" | "storage_id" | "bundle_id" | "managed_cloud_repository_id">[];
  /** In the project list's order: two projects on one repository open the first. */
  projects: readonly Pick<LocalProjectSummary, "local_project_id" | "display_name" | "local_alias">[];
  scopes: readonly Pick<CatalogScopeView, "scope" | "label" | "profile">[];
  capabilities: readonly CatalogCapabilityDescriptor[];
  /** Per storage that sees a Catalog item: the row that saved it, if one did. */
  places: Readonly<Record<string, CatalogCloudPlace | null>>;
}

export const INVALID_LINK: LinkStop = {
  kind: "stop",
  severity: "warning",
  message: "Mallard can’t open that link.",
  action: null,
};

/**
 * The host a Mallard Cloud storage signs in to, spelled as the backend's
 * `origin_host` spells the host of a link: lower case, `www.` dropped, the
 * port kept when the origin names one. `null` for what is not an origin.
 */
export function serviceHost(origin: string): string | null {
  try {
    return new URL(origin).host.toLowerCase().replace(/^www\./, "") || null;
  } catch {
    return null;
  }
}

type ListingLink = Extract<AppLinkArrival, { kind: "listing" }>;

/** The page the listing's link opens, by name: projects are pushed from
 *  Projects; skills, plugins and pets from the Catalog. */
export function listingPage(link: Pick<ListingLink, "listing_kind">): "Projects" | "Catalog" {
  return link.listing_kind === null || link.listing_kind === "project" ? "Projects" : "Catalog";
}

/**
 * Share yours: where the listing's link goes. It is decided on this
 * computer, from the storages it has: with none on the link's host there is
 * nothing to share to yet, and the sheet says so; otherwise it goes straight
 * to its page. `cloudHosts` are `serviceHost` of each Mallard Cloud storage.
 */
export function listingVerdict(link: ListingLink, cloudHosts: readonly string[]): LinkStop | LinkDirect {
  if (!cloudHosts.includes(link.host)) {
    return link.can_set_up
      ? { kind: "stop", severity: "info", message: "Sign in to Mallard Cloud to share.", action: "set_up" }
      : { kind: "stop", severity: "warning", message: `No account here for ${link.host}.`, action: null };
  }
  return { kind: "direct", destination: { kind: listingPage(link) === "Catalog" ? "catalog" : "projects" } };
}

const capitalize = (value: string) => (value ? `${value[0].toUpperCase()}${value.slice(1)}` : value);

/** The account's standing on what it found, as the pickers word a role. */
function standing(repository: Pick<AppLinkRepository, "role" | "visibility">): string {
  if (repository.role === null) return "Public";
  return `${repository.visibility === "public" ? "Public · " : ""}${capitalize(repository.role)}`;
}

function projectMark(repository: Pick<AppLinkRepository, "repository_class">): IconName {
  return repository.repository_class === "git_project" ? "git-folder" : "folder";
}

type Seer = AppLinkAccount & { sight: { kind: "found"; repository: AppLinkRepository } };
const sees = (account: AppLinkAccount): account is Seer => account.sight.kind === "found";

/** The storages whose accounts see a Catalog item at the address: the ones
 *  whose saved rows are worth asking for before `linkVerdict` runs. */
export function catalogSeers(answer: AppLinkAnswer): Array<{ storageId: string; repositoryId: string }> {
  return answer.accounts.filter(sees)
    .filter((account) => account.sight.repository.repository_class === "catalog")
    .map((account) => ({ storageId: account.storage_id, repositoryId: account.sight.repository.repository_id }));
}

/** The sheet's line for a link that names one session of its repository. */
export const ONE_SESSION = "One session of this repository";

/**
 * `session` is the stored row's id a session link named; null for a
 * repository's own link.
 */
export function linkVerdict(answer: AppLinkAnswer, context: LinkContext, session: string | null = null): LinkVerdict {
  const { accounts, address } = answer;
  if (accounts.length === 0) {
    return answer.can_set_up
      ? { kind: "stop", severity: "info", message: `Sign in to Mallard Cloud to open ${address}.`, action: "set_up" }
      : { kind: "stop", severity: "warning", message: `No account here for ${answer.host}.`, action: null };
  }
  const seers = accounts.filter(sees);
  if (seers.length === 0) {
    const failed = accounts.find((account) => account.sight.kind === "failed");
    if (failed && failed.sight.kind === "failed") {
      return { kind: "stop", severity: "danger", message: failed.sight.message, action: "retry" };
    }
    return {
      kind: "stop",
      severity: "warning",
      message: accounts.length === 1 ? `${accounts[0].account_login} can’t see ${address}.` : `No account here can see ${address}.`,
      action: null,
    };
  }
  const repository = seers[0].sight.repository;

  if (repository.repository_class !== "catalog") {
    if (!repository.bundle_id) return INVALID_LINK;
    // Any account on the host may be the one the project is connected through.
    const onHost = new Set(accounts.map((account) => account.storage_id));
    const connected = context.links.filter((link) => onHost.has(link.storage_id) && (
      link.managed_cloud_repository_id === repository.repository_id || link.bundle_id === repository.bundle_id
    ));
    const project = context.projects.find((candidate) => (
      connected.some((link) => link.local_project_id === candidate.local_project_id)
    ));
    const mark = projectMark(repository);
    const sub = session ? ONE_SESSION : null;
    if (project) {
      const link = connected.find((candidate) => candidate.local_project_id === project.local_project_id)!;
      const here = `On this computer: ${project.local_alias || project.display_name}`;
      return {
        kind: "landing",
        found: { address: repository.address, mark, standing: standing(repository), sub, here },
        ...(session
          ? { verb: "Pull session…", destination: { kind: "pull_session", projectId: project.local_project_id, storageId: link.storage_id, resourceId: session } }
          : { verb: "Open project", destination: { kind: "project", projectId: project.local_project_id, storageId: link.storage_id } }),
      };
    }
    return {
      kind: "landing",
      // The session is not carried through setup: the link is used again.
      found: { address: repository.address, mark, standing: standing(repository), sub, here: session ? "Not on this computer. Set up the project, then use the link again." : "Not on this computer." },
      verb: "Set up project…",
      // Several accounts see it: none is ticked, as New project does.
      destination: { kind: "new_project", storageId: seers.length === 1 ? seers[0].storage_id : null, address: repository.address },
    };
  }
  // A Catalog item has no session pages: the website writes no such link.
  if (session) return INVALID_LINK;

  const coordinate = repository.catalog;
  const capability = coordinate ? catalogCapabilityFor(context.capabilities, coordinate) : null;
  if (!coordinate || !capability) {
    return { kind: "stop", severity: "warning", message: `${repository.address} is a kind this app can’t open.`, action: null };
  }
  const mark = catalogPresentation(capability).icon;
  // The item's page is its repository's, so its link pulls the item.
  const pull = `Pull ${capability.singular_label.toLowerCase()}…`;
  const rowLabel = (scopeId: string) => {
    const row = context.scopes.find((scope) => scope.scope.scope_id === scopeId);
    return row ? catalogScopeLabel(row) : capability.label;
  };
  const saved = seers.map((seer) => context.places[seer.storage_id] ?? null).find((place) => place !== null) ?? null;
  if (saved) {
    return {
      kind: "landing",
      found: { address: repository.address, mark, standing: standing(repository), sub: null, here: `In your Catalog: ${rowLabel(saved.scope_id)}` },
      verb: pull,
      destination: { kind: "catalog_item", scopeId: saved.scope_id, storageId: saved.storage_id, resourceId: saved.resource_id },
    };
  }
  // The account's own item is listed by every row of its kind; Add from
  // Mallard Cloud is for someone else's.
  const owner = seers.find((seer) => seer.sight.repository.role === "owner");
  if (owner) {
    const row = context.scopes.find((scope) => sameCatalogCoordinate(scope.scope, coordinate));
    if (row) {
      return {
        kind: "landing",
        found: { address: repository.address, mark, standing: standing(owner.sight.repository), sub: null, here: `In your Catalog: ${catalogScopeLabel(row)}` },
        verb: pull,
        destination: { kind: "catalog_item", scopeId: row.scope.scope_id, storageId: owner.storage_id, resourceId: coordinate.resource_id },
      };
    }
    return {
      kind: "landing",
      found: { address: repository.address, mark, standing: standing(owner.sight.repository), sub: null, here: `No ${capability.label} row on this computer.` },
      verb: "Add to Catalog…",
      destination: { kind: "add_catalog_row", choiceKey: catalogChoiceKey(capability), storageId: owner.storage_id, resourceId: coordinate.resource_id },
    };
  }
  return {
    kind: "landing",
    found: { address: repository.address, mark, standing: standing(repository), sub: null, here: "Not in your Catalog." },
    verb: "Add to Catalog…",
    destination: { kind: "add_catalog_item", storageId: seers[0].storage_id, address: repository.address },
  };
}

/**
 * What makes a link wait under the bar instead of opening its sheet or its
 * page: an operation is running, a Push or Pull review is open, or a dialog is. A
 * link is never allowed to interrupt one; setup pages do not hold it, since
 * nothing on them is written until their own verb.
 */
export function linkWaits(shell: { busy: boolean; reviewOpen: boolean; dialogOpen: boolean }): boolean {
  return shell.busy || shell.reviewOpen || shell.dialogOpen;
}

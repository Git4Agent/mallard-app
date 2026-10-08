import type {
  AppLinkAccount,
  AppLinkAnswer,
  AppLinkArrival,
  AppLinkRepository,
  LocalProjectSummary,
  ProjectStorageLink,
} from "../../types";
import { t } from "../../i18n";
import type { IconName } from "../Icons";
import { capabilityLabel, catalogCapabilityFor, catalogChoiceKey, catalogPresentation, catalogScopeLabel, sameCatalogCoordinate } from "../storage-catalog/model";
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
  /** Why, in a few words: the sheet's title. */
  title: string;
  icon: IconName;
  /** Why, as a sentence: the title's description. */
  message: string;
  /** `set_up`: Mallard Cloud's sign-in, then the link again. `retry`: ask again. */
  action: "set_up" | "retry" | null;
}

/** One thing the service said about what it found: a glyph and a word. */
export interface LinkFact {
  icon: IconName;
  word: string;
}

/** Where what was found stands on this computer. */
export interface LinkHere {
  place: "computer" | "catalog";
  /** It is here: `word` is its name here. */
  present: boolean;
  /** Its name here, or why it is not, in a few words. */
  word: string;
  /** The same, as a sentence. */
  sentence: string;
}

/** What the service found, as the sheet says it. */
export interface LinkFound {
  address: string;
  mark: IconName;
  /** Visibility, the account's role, and the one session a session's link names. */
  facts: LinkFact[];
  here: LinkHere;
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
  /** `capability` words the row in the interface language; without it the
   *  row reads as the backend labelled it. */
  scopes: readonly (Pick<CatalogScopeView, "scope" | "label" | "profile"> & Partial<Pick<CatalogScopeView, "capability">>)[];
  capabilities: readonly CatalogCapabilityDescriptor[];
  /** Per storage that sees a Catalog item: the row that saved it, if one did. */
  places: Readonly<Record<string, CatalogCloudPlace | null>>;
}

/** A link that cannot be read, worded in the language of the moment it is said. */
export function invalidLink(): LinkStop {
  return {
    kind: "stop",
    severity: "warning",
    title: t("shell:link.brokenTitle"),
    icon: "link-off",
    message: t("shell:link.broken"),
    action: null,
  };
}

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

/** Which page a listing link opens: projects are pushed from Projects;
 *  skills, plugins and pets from the Catalog. */
export function listingDestination(link: Pick<ListingLink, "listing_kind">): "projects" | "catalog" {
  return link.listing_kind === null || link.listing_kind === "project" ? "projects" : "catalog";
}

/** That page's name, as the waiting notice says it. */
export function listingPage(link: Pick<ListingLink, "listing_kind">): string {
  return t(listingDestination(link) === "catalog" ? "shell:roots.catalog" : "shell:roots.projects");
}

/**
 * Share yours: where the listing's link goes. It is decided on this
 * computer, from the storages it has: with none on the link's host there is
 * nothing to share to yet, and the sheet says so; otherwise it goes straight
 * to its page. `cloudHosts` are `serviceHost` of each Mallard Cloud storage.
 */
export function listingVerdict(link: ListingLink, cloudHosts: readonly string[]): LinkStop | LinkDirect {
  if (!cloudHosts.includes(link.host)) {
    return link.can_set_up ? signIn(t("shell:link.signInShare")) : noAccount(link.host);
  }
  return { kind: "direct", destination: { kind: listingDestination(link) } };
}

/** The service could not be asked, or could not answer: say why, and ask again. */
export const unanswered = (message: string): LinkStop => (
  { kind: "stop", severity: "danger", title: t("shell:link.unansweredTitle"), icon: "cloud-x", message, action: "retry" }
);
const signIn = (message: string): LinkStop => (
  { kind: "stop", severity: "info", title: t("shell:link.signInTitle"), icon: "user", message, action: "set_up" }
);
const noAccount = (host: string): LinkStop => (
  { kind: "stop", severity: "warning", title: t("shell:link.noAccountTitle", { host }), icon: "user", message: t("shell:link.noAccount", { host }), action: null }
);

const capitalize = (value: string) => (value ? `${value[0].toUpperCase()}${value.slice(1)}` : value);

/** The account's role in a word: the service's three roles by key, any
 *  other as the service spelled it. */
function roleWord(role: string): string {
  switch (role) {
    case "owner": return t("shell:link.roleOwner");
    case "admin": return t("shell:link.roleAdmin");
    case "member": return t("shell:link.roleMember");
    default: return capitalize(role);
  }
}

/** What the service said about it: who can see it, the account's role, and the session a session's link names. */
function facts(repository: Pick<AppLinkRepository, "role" | "visibility">, session: string | null = null): LinkFact[] {
  const shown: LinkFact[] = [repository.visibility === "public"
    ? { icon: "cloud", word: t("shell:link.public") }
    : { icon: "lock", word: t("shell:link.private") }];
  if (repository.role !== null) shown.push({ icon: "user", word: roleWord(repository.role) });
  if (session) shown.push({ icon: "message", word: t("shell:link.sessions", { count: 1 }) });
  return shown;
}

const present = (place: LinkHere["place"], name: string): LinkHere => ({
  place, present: true, word: name, sentence: t(place === "catalog" ? "shell:link.inCatalog" : "shell:link.onComputer", { name }),
});
const absent = (place: LinkHere["place"], word: string, sentence: string): LinkHere => ({ place, present: false, word, sentence });

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

/**
 * `session` is the stored row's id a session link named; null for a
 * repository's own link.
 */
export function linkVerdict(answer: AppLinkAnswer, context: LinkContext, session: string | null = null): LinkVerdict {
  const { accounts, address } = answer;
  if (accounts.length === 0) {
    return answer.can_set_up ? signIn(t("shell:link.signInOpen", { address })) : noAccount(answer.host);
  }
  const seers = accounts.filter(sees);
  if (seers.length === 0) {
    const failed = accounts.find((account) => account.sight.kind === "failed");
    if (failed && failed.sight.kind === "failed") {
      return unanswered(failed.sight.message);
    }
    return {
      kind: "stop",
      severity: "warning",
      title: t("shell:link.noAccessTitle"),
      icon: "lock",
      message: accounts.length === 1
        ? t("shell:link.noAccessOne", { login: accounts[0].account_login, address })
        : t("shell:link.noAccessAny", { address }),
      action: null,
    };
  }
  const repository = seers[0].sight.repository;

  if (repository.repository_class !== "catalog") {
    if (!repository.bundle_id) return invalidLink();
    // Any account on the host may be the one the project is connected through.
    const onHost = new Set(accounts.map((account) => account.storage_id));
    const connected = context.links.filter((link) => onHost.has(link.storage_id) && (
      link.managed_cloud_repository_id === repository.repository_id || link.bundle_id === repository.bundle_id
    ));
    const project = context.projects.find((candidate) => (
      connected.some((link) => link.local_project_id === candidate.local_project_id)
    ));
    const mark = projectMark(repository);
    const shown = facts(repository, session);
    if (project) {
      const link = connected.find((candidate) => candidate.local_project_id === project.local_project_id)!;
      return {
        kind: "landing",
        found: { address: repository.address, mark, facts: shown, here: present("computer", project.local_alias || project.display_name) },
        ...(session
          ? { verb: t("shell:link.pullSession"), destination: { kind: "pull_session", projectId: project.local_project_id, storageId: link.storage_id, resourceId: session } }
          : { verb: t("shell:link.openProject"), destination: { kind: "project", projectId: project.local_project_id, storageId: link.storage_id } }),
      };
    }
    return {
      kind: "landing",
      // The session is not carried through setup: the link is used again.
      found: {
        address: repository.address,
        mark,
        facts: shown,
        here: absent("computer", t("shell:link.notHere"), t(session ? "shell:link.notOnComputerSetUp" : "shell:link.notOnComputer")),
      },
      verb: t("shell:link.setUpProject"),
      // Several accounts see it: none is ticked, as New project does.
      destination: { kind: "new_project", storageId: seers.length === 1 ? seers[0].storage_id : null, address: repository.address },
    };
  }
  // A Catalog item has no session pages: the website writes no such link.
  if (session) return invalidLink();

  const coordinate = repository.catalog;
  const capability = coordinate ? catalogCapabilityFor(context.capabilities, coordinate) : null;
  if (!coordinate || !capability) {
    return {
      kind: "stop",
      severity: "warning",
      title: t("shell:link.unknownKindTitle"),
      icon: "ban",
      message: t("shell:link.unknownKind", { address: repository.address }),
      action: null,
    };
  }
  const mark = catalogPresentation(capability).icon;
  // The item's page is its repository's, so its link pulls the item.
  const pull = pullVerb(capability);
  const rowLabel = (scopeId: string) => {
    const row = context.scopes.find((scope) => scope.scope.scope_id === scopeId);
    return row ? catalogScopeLabel(row) : capabilityLabel(capability);
  };
  const saved = seers.map((seer) => context.places[seer.storage_id] ?? null).find((place) => place !== null) ?? null;
  if (saved) {
    return {
      kind: "landing",
      found: { address: repository.address, mark, facts: facts(repository), here: present("catalog", rowLabel(saved.scope_id)) },
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
        found: { address: repository.address, mark, facts: facts(owner.sight.repository), here: present("catalog", catalogScopeLabel(row)) },
        verb: pull,
        destination: { kind: "catalog_item", scopeId: row.scope.scope_id, storageId: owner.storage_id, resourceId: coordinate.resource_id },
      };
    }
    return {
      kind: "landing",
      found: {
        address: repository.address,
        mark,
        facts: facts(owner.sight.repository),
        here: absent("catalog", t("shell:link.noRow", { label: capabilityLabel(capability) }), t("shell:link.noRowHere", { label: capabilityLabel(capability) })),
      },
      verb: t("shell:link.addToCatalog"),
      destination: { kind: "add_catalog_row", choiceKey: catalogChoiceKey(capability), storageId: owner.storage_id, resourceId: coordinate.resource_id },
    };
  }
  return {
    kind: "landing",
    found: { address: repository.address, mark, facts: facts(repository), here: absent("catalog", t("shell:link.notInCatalog"), t("shell:link.notInYourCatalog")) },
    verb: t("shell:link.addToCatalog"),
    destination: { kind: "add_catalog_item", storageId: seers[0].storage_id, address: repository.address },
  };
}

/** The verb that pulls a Catalog item, one sentence per kind the registry
 *  ships, since a lower-cased English noun is not a word in every language.
 *  A kind this build does not know is named by the registry's own label. */
function pullVerb(capability: Pick<CatalogCapabilityDescriptor, "resource_kind" | "format_version" | "singular_label">): string {
  switch (capability.resource_kind) {
    case "agent_skill": return t("shell:link.pullSkill");
    case "plugin_install_intent": return t("shell:link.pullPlugin");
    case "desktop_pet": return t("shell:link.pullPet", { version: capability.format_version });
    default: return t("shell:link.pullItem", { kind: capability.singular_label.toLowerCase() });
  }
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

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import Icon from "../Icons";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { storageCatalogApi, type StorageCatalogApi } from "./api";
import {
  actionLabel,
  catalogKindMeta,
  catalogScopeLabel,
  errorMessage,
  linkStateLabel,
  rowLocation,
  rowPullable,
  rowPushable,
  rowStamp,
  rowStatus,
  rowStatusLabel,
} from "./model";
import type {
  CatalogLinkInspection,
  CatalogProviderKey,
  CatalogResourcePullPlan,
  CatalogResourceView,
  CatalogScopeResources,
  CatalogScopeRow,
  CatalogScopeView,
} from "./types";

/**
 * One Catalog row's page, drawn the way a project's is: the storage chosen in
 * the path bar, one flat list of resources each wearing the status badge a
 * thread wears, and Push and Pull as page verbs with a review — rather than a
 * per-resource drill-down with a storage picker on every row.
 *
 * The storage in the bar is the page's outer scope. Every resource of this
 * kind is listed against it, whether it is registered here, only on this Mac,
 * or only in that storage; the badge says which. Linking is not a step the
 * user takes — Push and Pull create the pairing they need, which is what
 * makes this page read like the project one.
 */

export interface CatalogStorageOption {
  id: string;
  name: string;
  kind: "local" | "s3";
}

type Mode =
  | { kind: "list" }
  | { kind: "push" }
  | { kind: "pull" }
  | { kind: "detail"; registrationId: string };

/** One reviewed row in the Pull review: the pairing it needed, and the exact
 * local actions the backend planned for it. */
interface PullEntry {
  row: CatalogScopeRow;
  linkId: string;
  plan: CatalogResourcePullPlan;
}

function providerLabel(provider: CatalogProviderKey): string {
  return provider === "codex" ? "Codex" : "Claude";
}

function resourceKindLabel(resource: CatalogResourceView): string {
  if (resource.registration.resource_kind === "codex_pet_v1") return "Codex pet v1";
  if (resource.registration.resource_kind === "codex_pet_v2") return "Codex pet v2";
  if (resource.registration.resource_kind === "plugin_install_intent") return "Plugin intent";
  return resource.scope_label ? `${resource.scope_label} skill` : "Skill";
}

/** The verb a local action reduces to, for the row's trailing readout. The
 * whole sentence stays in the choice list; the row gets the word. */
function actionVerb(label: string): string {
  if (label.startsWith("Keep local as")) return "Keep copy";
  if (label.startsWith("Overwrite")) return "Overwrite";
  if (label.startsWith("Install")) return "Install";
  return "Update";
}

/** The indicator's colour class, keyed by badge — the project's own pairing. */
const TONE: Record<string, string> = {
  synced: "synced",
  "local-outstanding": "local",
  "cloud-outstanding": "storage",
  divergent: "diverged",
  pending: "pending",
  conflict: "conflict",
  unavailable: "unavailable",
  untracked: "untracked",
  checking: "checking",
  unknown: "unknown",
};

/** What the Pull review hangs off a row: the chosen action, and the way to
 * change it when the plan offers more than one. */
interface RowAction {
  chosen: string;
  options: Array<{ id: string; label: string }>;
  expanded: boolean;
  destructive: boolean;
  onToggle: () => void;
  onChoose: (actionId: string) => void;
}

/** One line: what state it is in, what it is called, where it lives, when it
 * last moved. Everything else — revisions, other storage links — is a tooltip
 * or the detail page behind the chevron. */
function ResourceRow({
  row,
  storageName,
  selectable,
  selected,
  onToggle,
  action,
  failure,
  onOpen,
}: {
  row: CatalogScopeRow;
  storageName: string | null;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  action?: RowAction;
  failure?: string;
  onOpen?: () => void;
}) {
  const status = rowStatus(row);
  const location = rowLocation(row);
  const stamp = rowStamp(row);
  const badgeLabel = [
    rowStatusLabel(row, storageName),
    row.other_storage_names.length > 0 && `Also linked to ${row.other_storage_names.join(", ")}.`,
    failure,
  ].filter(Boolean).join(" ");
  return (
    <article className={`catalog-row${selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      {selectable && (
        <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${row.display_name}`}>
          <input
            type="checkbox"
            checked={!!selected}
            aria-label={`${selected ? "Exclude" : "Include"} ${row.display_name}`}
            onChange={() => onToggle?.()}
          />
        </label>
      )}
      <span
        className={`sync-thread-sync-indicator ${TONE[status] ?? "unknown"}`}
        data-tooltip={badgeLabel}
        aria-label={badgeLabel}
        tabIndex={0}
      >
        <ThreadStatusIcon status={status} size={18} />
      </span>
      {onOpen ? (
        <button type="button" className="catalog-row-name" onClick={onOpen}>{row.display_name}</button>
      ) : (
        <span className="catalog-row-name">{row.display_name}</span>
      )}
      {action && (action.options.length > 1 ? (
        <span className="catalog-row-action-shell">
          <button
            type="button"
            className={`catalog-row-action is-choice${action.destructive ? " destructive" : ""}`}
            aria-expanded={action.expanded}
            aria-haspopup="menu"
            title={`${action.chosen} — choose a different local action`}
            onClick={action.onToggle}
          >
            {actionVerb(action.chosen)}
            <Icon name={action.expanded ? "chevron-up" : "chevron-down"} size={10} />
          </button>
          {action.expanded && (
            /* The app's own popover, item markup and all: a checkable list
               with the current choice ticked. */
            <div className="sync-path-menu end" role="menu" aria-label={`Local action for ${row.display_name}`}>
              <div className="sync-path-menu-group">
                {action.options.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={action.chosen === option.label}
                    className={`sync-path-menu-item${option.label.startsWith("Overwrite") ? " danger" : ""}`}
                    onClick={() => action.onChoose(option.id)}
                  >
                    <span className="sync-path-menu-copy">
                      <span>{actionVerb(option.label)}</span>
                      <small className="phrase">{option.label}</small>
                    </span>
                    {action.chosen === option.label && (
                      <Icon name="check" size={13} className="sync-path-menu-check" />
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}
        </span>
      ) : (
        <span className="catalog-row-action" title={action.chosen}>{actionVerb(action.chosen)}</span>
      ))}
      <span
        className="catalog-row-place"
        data-tooltip={location?.label}
        aria-label={location?.label}
        tabIndex={location ? 0 : undefined}
      >
        {location && <Icon name={location.icon} size={13} />}
      </span>
      <span
        className="catalog-row-stamp"
        data-tooltip={stamp?.detail}
        aria-label={stamp?.detail}
        tabIndex={stamp ? 0 : undefined}
      >
        {stamp?.short ?? ""}
      </span>
      {onOpen && (
        <button
          type="button"
          className="btn btn-ghost catalog-row-open"
          title="Storage links, fork, unlink"
          aria-label={`Open ${row.display_name}`}
          onClick={onOpen}
        >
          <Icon name="chevron-right" size={14} />
        </button>
      )}
    </article>
  );
}

export default function StorageCatalogPage({
  scope,
  scopes,
  storages,
  appActions,
  onOpenScope,
  onOpenStorages,
  onOpenStorageSettings,
  onRemoveScope,
  onScopeGone,
  api = storageCatalogApi,
}: {
  scope: CatalogScopeView;
  /** The other Catalog rows, for this row's Switch-to menu. */
  scopes: CatalogScopeView[];
  storages: CatalogStorageOption[];
  /** Theme and app-wide commands: this bar is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  onOpenStorages: () => void;
  onOpenStorageSettings: (storageId: string) => void;
  onRemoveScope: () => void;
  /** The row disappeared underneath the page (deleted elsewhere). */
  onScopeGone: () => void;
  api?: StorageCatalogApi;
}) {
  const label = catalogScopeLabel(scope.scope, scope.profile);
  const kind = catalogKindMeta(scope.scope.resource_kind);
  const [page, setPage] = useState<CatalogScopeResources | null>(null);
  const [chosenStorageId, setChosenStorageId] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [inspectingLinks, setInspectingLinks] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<string | null>(null);

  const [pushSelected, setPushSelected] = useState<Set<string>>(new Set());
  const [pushAcks, setPushAcks] = useState<Set<string>>(new Set());

  const [pullEntries, setPullEntries] = useState<PullEntry[]>([]);
  const [pullBlocked, setPullBlocked] = useState<Record<string, string>>({});
  const [pullSelected, setPullSelected] = useState<Set<string>>(new Set());
  const [pullChoice, setPullChoice] = useState<Record<string, string>>({});
  const [pullAcks, setPullAcks] = useState<Set<string>>(new Set());
  const [openChoices, setOpenChoices] = useState<string | null>(null);
  const pullListRef = useRef<HTMLDivElement | null>(null);

  const [detail, setDetail] = useState<CatalogResourceView | null>(null);
  const [recovery, setRecovery] = useState<string[]>([]);
  const [linkInspections, setLinkInspections] = useState<Record<string, CatalogLinkInspection>>({});
  const [forkName, setForkName] = useState("");
  const epochRef = useRef(0);
  const inspectionEpochRef = useRef(0);

  const load = useCallback(async () => {
    const epoch = ++epochRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await api.listScopeResources(scope.scope.scope_id, chosenStorageId);
      if (epoch !== epochRef.current) return;
      setPage(next);
    } catch (reason) {
      if (epoch !== epochRef.current) return;
      if (errorMessage(reason).startsWith("unknown catalog scope")) {
        onScopeGone();
        return;
      }
      setError(errorMessage(reason));
    } finally {
      if (epoch === epochRef.current) setLoading(false);
    }
  }, [api, chosenStorageId, onScopeGone, scope.scope.scope_id]);

  useEffect(() => {
    void load();
    return () => {
      epochRef.current += 1;
    };
  }, [load]);

  useEffect(() => {
    setMode({ kind: "list" });
    setChosenStorageId(null);
    setOutcome(null);
  }, [scope.scope.scope_id]);

  const rows = useMemo(() => page?.rows ?? [], [page]);
  const storageName = page?.storage_name ?? null;
  const storageId = page?.storage_id ?? null;
  const pushRows = useMemo(() => rows.filter(rowPushable), [rows]);
  const pullRows = useMemo(() => rows.filter(rowPullable), [rows]);
  /* Local changes that Push cannot take yet: the storage moved too, so Pull
     has to settle them first. Counted here rather than written as a number,
     so the banner and the list can never disagree. */
  const heldForPull = useMemo(
    () => rows.filter((row) => row.local_present
      && (row.state === "diverged" || row.state === "unreviewed_remote")),
    [rows],
  );

  /* The bar's three numbers are read off the same rows the list draws — the
     project derives its own header tally the same way. */
  const counts = useMemo(() => rows.reduce((totals, row) => {
    const status = rowStatus(row);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 }), [rows]);

  const attentionDetail = useMemo(() => ([
    ["divergent", "changed on both sides"],
    ["pending", "waiting to sync"],
    ["unavailable", "can't be synced"],
  ] as Array<[string, string]>)
    .map(([status, phrase]) => {
      const count = rows.filter((row) => rowStatus(row) === status).length;
      const noun = count === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
      return count === 0 ? null : `${count} ${noun} ${phrase}`;
    })
    .filter((phrase): phrase is string => phrase !== null)
    .join(", "), [kind, rows]);

  /* Outside-click and Escape dismiss the action menu, the same contract the
     path bar's own menus keep. */
  useEffect(() => {
    if (!openChoices) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && pullListRef.current?.contains(target)) return;
      setOpenChoices(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpenChoices(null);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [openChoices]);

  /** Every row Push and Pull act on needs a pairing with this storage.
   * Linking is configuration only, no provider content changed, so creating
   * it as part of the verb the user pressed is safe — and it is why nothing
   * on this page asks the user to link by hand. */
  const ensureLink = useCallback(async (row: CatalogScopeRow): Promise<string> => {
    if (row.link_id) return row.link_id;
    if (!storageId) throw new Error("choose a storage first");
    const result = await api.linkScopeResource({
      scope_id: scope.scope.scope_id,
      storage_id: storageId,
      local_resource_id: row.repository_id ? null : row.resource_id ?? null,
      repository_id: row.repository_id ?? null,
    });
    return result.link_id;
  }, [api, scope.scope.scope_id, storageId]);

  const openPush = () => {
    setPushSelected(new Set(pushRows.map((row) => row.row_id)));
    setPushAcks(new Set());
    setError(null);
    setOutcome(null);
    setMode({ kind: "push" });
  };

  const runPush = async () => {
    const selected = pushRows.filter((row) => pushSelected.has(row.row_id));
    setBusy(true);
    setError(null);
    const failures: string[] = [];
    let pushed = 0;
    try {
      for (const [index, row] of selected.entries()) {
        setProgress(`Pushing ${row.display_name} (${index + 1} of ${selected.length})…`);
        try {
          const linkId = await ensureLink(row);
          const plan = await api.preparePush(linkId);
          if (!plan.row.selectable) {
            throw new Error(plan.row.blocked_reason ?? "nothing safe to push");
          }
          const result = await api.publish(linkId, plan.plan_id, true, [...pushAcks]);
          if (result.published) pushed += 1;
        } catch (reason) {
          failures.push(`${row.display_name}: ${errorMessage(reason)}`);
        }
      }
    } finally {
      setProgress(null);
      setBusy(false);
    }
    setMode({ kind: "list" });
    await load();
    const noun = pushed === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
    setOutcome(`Pushed ${pushed} ${noun} to ${storageName ?? "storage"}.`);
    if (failures.length > 0) setError(failures.join("\n"));
  };

  /** Opening the Pull review plans every row it could act on. Rows the
   * storage holds but this row has never paired with are linked first — the
   * same "link for review" step this page has always taken, configuration
   * only, no provider content changed. */
  const openPull = async () => {
    setBusy(true);
    setError(null);
    setOutcome(null);
    setMode({ kind: "pull" });
    const entries: PullEntry[] = [];
    const blocked: Record<string, string> = {};
    try {
      for (const [index, row] of pullRows.entries()) {
        setProgress(`Reviewing ${row.display_name} (${index + 1} of ${pullRows.length})…`);
        try {
          const linkId = await ensureLink(row);
          const plan = await api.preparePull(linkId, null);
          entries.push({ row, linkId, plan });
        } catch (reason) {
          blocked[row.row_id] = errorMessage(reason);
        }
      }
    } finally {
      setProgress(null);
      setBusy(false);
    }
    setPullEntries(entries);
    setPullBlocked(blocked);
    setPullSelected(new Set(entries
      .filter((entry) => entry.plan.item.default_action_id)
      .map((entry) => entry.row.row_id)));
    setPullChoice(Object.fromEntries(entries
      .filter((entry) => entry.plan.item.default_action_id)
      .map((entry) => [entry.row.row_id, entry.plan.item.default_action_id as string])));
    setPullAcks(new Set());
  };

  const runPull = async () => {
    const selected = pullEntries.filter((entry) => pullSelected.has(entry.row.row_id));
    setBusy(true);
    setError(null);
    const failures: string[] = [];
    let applied = 0;
    let unaccepted = 0;
    try {
      for (const [index, entry] of selected.entries()) {
        setProgress(`Pulling ${entry.row.display_name} (${index + 1} of ${selected.length})…`);
        try {
          const result = await api.applyPull(
            entry.linkId,
            entry.plan.plan_id,
            pullChoice[entry.row.row_id] ?? null,
            pullAcks.has(entry.row.row_id),
          );
          applied += 1;
          if (!result.remote_accepted) unaccepted += 1;
        } catch (reason) {
          failures.push(`${entry.row.display_name}: ${errorMessage(reason)}`);
        }
      }
    } finally {
      setProgress(null);
      setBusy(false);
    }
    setMode({ kind: "list" });
    await load();
    const noun = applied === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
    setOutcome(unaccepted > 0
      ? `Pulled ${applied} ${noun}. Pull did not advance the comparison base for ${unaccepted} of them.`
      : `Pulled ${applied} ${noun} from ${storageName ?? "storage"}.`);
    if (failures.length > 0) setError(failures.join("\n"));
  };

  const unlinkStorage = async () => {
    if (!storageId) return;
    const linked = rows.filter((row) => row.link_id && row.registration_id);
    const approved = await confirm(
      `Unlink “${storageName}” from ${label}?\n\nPush and Pull stop for all ${linked.length} paired ${kind.plural.toLowerCase()}. Content already stored there and local content are left untouched, and you can link again later.`,
      { title: "Unlink storage" },
    );
    if (!approved) return;
    setBusy(true);
    setError(null);
    const failures: string[] = [];
    try {
      for (const row of linked) {
        try {
          await api.unlink(row.registration_id!, row.link_id!, row.link_revision ?? 1);
        } catch (reason) {
          failures.push(`${row.display_name}: ${errorMessage(reason)}`);
        }
      }
    } finally {
      setBusy(false);
    }
    await load();
    setOutcome("Storage links removed. Local and stored content were left untouched.");
    if (failures.length > 0) setError(failures.join("\n"));
  };

  // ---- the per-resource detail behind a row -------------------------------

  useEffect(() => {
    inspectionEpochRef.current += 1;
    setInspectingLinks(false);
    setLinkInspections({});
    if (mode.kind !== "detail") {
      setDetail(null);
      setRecovery([]);
      return;
    }
    let cancelled = false;
    void api.getResource(mode.registrationId).then((resource) => {
      if (!cancelled) setDetail(resource);
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    // Anything a half-finished operation left behind is the resource's own
    // news, so it is reported where the resource is.
    void api.getStatus(mode.registrationId).then((status) => {
      if (!cancelled) setRecovery(status.recovery_messages);
    }).catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [api, mode]);

  const inspectLinks = useCallback(async (registrationId: string) => {
    const epoch = ++inspectionEpochRef.current;
    setInspectingLinks(true);
    setError(null);
    try {
      const inspections = await api.inspectLinks(registrationId);
      if (epoch !== inspectionEpochRef.current) return;
      setLinkInspections(Object.fromEntries(
        inspections.map((inspection) => [inspection.link_id, inspection]),
      ));
    } catch (reason) {
      if (epoch === inspectionEpochRef.current) setError(errorMessage(reason));
    } finally {
      if (epoch === inspectionEpochRef.current) setInspectingLinks(false);
    }
  }, [api]);

  useEffect(() => {
    if (!detail) return;
    void inspectLinks(detail.registration.registration_id);
  }, [detail, inspectLinks]);

  const runFork = async (resource: CatalogResourceView) => {
    setBusy(true);
    setError(null);
    try {
      const plan = await api.prepareFork(resource.registration.registration_id, forkName.trim());
      const result = await api.applyFork(resource.registration.registration_id, plan.plan_id);
      setForkName("");
      setOutcome(result.message ?? `Created ${result.display_name}`);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const unlinkOne = async (resource: CatalogResourceView, linkId: string, revision: number) => {
    // Unlink sits beside Refresh, so a misclick must not tear down the
    // pairing unannounced — the same contract project-scoped unlink keeps.
    const name = resource.links.find((candidate) => candidate.link.link_id === linkId)?.storage_name ?? "this storage";
    const approved = await confirm(
      `Unlink “${name}” from “${resource.registration.display_name}”?\n\nPush and Pull will stop for this pairing. Content already stored there and local content will not be deleted, and you can link the storage again later.`,
      { title: "Unlink storage" },
    );
    if (!approved) return;
    setBusy(true);
    setError(null);
    try {
      await api.unlink(resource.registration.registration_id, linkId, revision);
      setOutcome("Storage link removed. Local and remote content were left untouched.");
      setMode({ kind: "list" });
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const removeResource = async (resource: CatalogResourceView) => {
    const approved = await confirm(
      `Remove the registration for “${resource.registration.display_name}”?\n\nThe Library stops tracking this resource. Provider content on disk is left untouched, and you can register it again later.`,
      { title: "Remove registration" },
    );
    if (!approved) return;
    setBusy(true);
    setError(null);
    try {
      await api.remove(resource.registration.registration_id, resource.registration.revision);
      setOutcome("Registration removed. Provider content was left untouched.");
      setMode({ kind: "list" });
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  // ---- the bar -----------------------------------------------------------

  const crumbs: PathBarCrumb[] = [
    {
      id: "storages",
      label: "Storages",
      icon: "drive",
      tone: "section",
      onActivate: onOpenStorages,
    },
    {
      id: "storage",
      label: storageName ?? "No storage",
      icon: "cloud",
      tone: "root",
      warning: !storageId,
      menuLabel: storageId ? "Switch storage" : "Choose storage",
      menuHeader: { label: storageName ?? "No storage linked", icon: "cloud" },
      menuBanner: storageId
        ? undefined
        : `Add a storage to compare these ${kind.plural.toLowerCase()} and turn on Push and Pull.`,
      menu: [
        {
          items: [
            {
              id: "settings",
              label: "Storage settings",
              icon: "settings",
              disabled: !storageId,
              onSelect: () => storageId && onOpenStorageSettings(storageId),
            },
            {
              id: "unlink",
              label: `Unlink from these ${kind.plural.toLowerCase()}`,
              icon: "link-off",
              danger: true,
              disabled: !storageId || !rows.some((row) => row.link_id),
              disabledReason: "Nothing here is paired with it yet",
              onSelect: () => void unlinkStorage(),
            },
          ],
        },
        {
          label: "Switch to",
          items: storages
            .filter((storage) => storage.id !== storageId)
            .map((storage) => ({
              id: storage.id,
              label: storage.name,
              icon: storage.kind === "local" ? ("drive" as const) : ("cloud" as const),
              onSelect: () => {
                setChosenStorageId(storage.id);
                setMode({ kind: "list" });
              },
            })),
        },
      ],
    },
    {
      id: "scope",
      label,
      icon: kind.icon,
      title: `${label} · reads ${scope.profile.configured_home}`,
      menuLabel: "Catalog row actions",
      menuHeader: {
        label,
        detail: scope.profile.configured_home,
        detailIsPath: true,
        icon: kind.icon,
      },
      onActivate: mode.kind === "detail" ? () => setMode({ kind: "list" }) : undefined,
      menu: mode.kind === "detail" ? undefined : [
        {
          items: [
            { id: "rescan", label: "Rescan this provider home", icon: "refresh", onSelect: () => void load() },
            { id: "remove", label: "Remove this Catalog row", icon: "trash", danger: true, onSelect: onRemoveScope },
          ],
        },
        {
          label: "Switch to",
          items: scopes
            .filter((candidate) => candidate.scope.scope_id !== scope.scope.scope_id)
            .map((candidate) => ({
              id: candidate.scope.scope_id,
              label: catalogScopeLabel(candidate.scope, candidate.profile),
              icon: catalogKindMeta(candidate.scope.resource_kind).icon,
              onSelect: () => onOpenScope(candidate.scope.scope_id),
            })),
        },
      ],
    },
  ];
  if (mode.kind === "detail" && detail) {
    crumbs.push({
      id: "resource",
      label: detail.registration.display_name,
      icon: "file",
      title: `${providerLabel(detail.registration.provider)} · ${resourceKindLabel(detail)}`,
    });
  }

  const syncDisabled = busy || !storageId || loading;
  const syncDisabledReason = storageId
    ? undefined
    : `Add a storage to these ${kind.plural.toLowerCase()} before pushing or pulling`;
  const bar = (
    <SyncPathBar
      crumbs={crumbs}
      overflow={appActions}
      overflowLabel="App actions"
      ariaLabel={`${label} path`}
      actions={(
        <>
          {counts.attention > 0 && (
            <span className="sync-path-attention warning" tabIndex={0} role="status" data-tooltip={attentionDetail}>
              <Icon name="alert-triangle" size={13} /><b>{counts.attention}</b>
            </span>
          )}
          <button
            type="button"
            className="sync-path-icon-action"
            title="Refresh comparison"
            aria-label="Refresh comparison"
            disabled={busy || loading}
            onClick={() => void load()}
          >
            <span aria-hidden="true" className={`icon-spin-frame${loading ? " spinning" : ""}`}>
              <Icon name="refresh" size={16} />
            </span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button
              type="button"
              className={`sync-path-sync${mode.kind === "pull" ? " active" : ""}`}
              aria-expanded={mode.kind === "pull"}
              disabled={syncDisabled || pullRows.length === 0}
              title={syncDisabledReason}
              onClick={() => void openPull()}
            >
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              {counts.pull > 0 && mode.kind !== "pull" && (
                <span className="sync-path-count" aria-hidden="true">{counts.pull}</span>
              )}
            </button>
            <button
              type="button"
              className={`sync-path-sync${mode.kind === "push" ? " active" : ""}`}
              aria-expanded={mode.kind === "push"}
              disabled={syncDisabled || pushRows.length === 0}
              title={syncDisabledReason}
              onClick={openPush}
            >
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              {counts.push > 0 && mode.kind !== "push" && (
                <span className="sync-path-count" aria-hidden="true">{counts.push}</span>
              )}
            </button>
          </div>
        </>
      )}
    />
  );

  const notices = (
    <>
      {progress && <div className="sync-callout info" role="status">{progress}</div>}
      {error && <div className="sync-callout error" role="alert">{error}</div>}
      {outcome && (
        <div className="sync-callout success" role="status">
          <Icon name="check-circle" size={14} /> {outcome}
          <button type="button" className="btn btn-ghost" onClick={() => setOutcome(null)}>Dismiss</button>
        </div>
      )}
    </>
  );

  // ---- Push review -------------------------------------------------------

  if (mode.kind === "push") {
    const warnings = pushRows
      .filter((row) => pushSelected.has(row.row_id))
      .flatMap((row) => row.warnings.map((warning) => ({ row, warning })));
    const unacknowledged = warnings.some(({ warning }) => !pushAcks.has(warning.digest));
    return (
      <>
        {bar}
        <main className="sync-main sync-project-links-page sync-sync-review-page">
          <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="catalog-push-title">
            <header className="sync-inline-action-header sync-sync-review-header">
              <div className="sync-sync-review-title"><h2 id="catalog-push-title">Push review</h2></div>
              <SyncReviewSelectionBar
                count={pushSelected.size}
                total={pushRows.length}
                busy={busy}
                recommendedTitle={`Use the recommended selection (${pushRows.length})`}
                recommendedAriaLabel={`Use recommended selection (${pushRows.length})`}
                recommendedDisabled={pushSelected.size === pushRows.length}
                onRecommended={() => setPushSelected(new Set(pushRows.map((row) => row.row_id)))}
                clearDisabled={pushSelected.size === 0}
                onClear={() => setPushSelected(new Set())}
                close={{ ariaLabel: "Close Push review", onClose: () => setMode({ kind: "list" }) }}
              />
            </header>
            <div className="sync-inline-action-content sync-sync-review-content">
              <div className="sync-inline-action-scroll sync-sync-review-scroll">
                {notices}
                {heldForPull.length > 0 && (
                  <div className="sync-review-banners">
                    <div className="sync-callout sync-sync-review-pull-holds" role="status">
                      <ThreadStatusIcon status="divergent" size={16} />
                      <span>
                        <strong>
                          {`${heldForPull.length} ${heldForPull.length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase()} held for Pull`}
                        </strong>
                        {`${storageName ?? "Storage"} has newer versions of them, so their local changes cannot push yet. Selected ${kind.plural.toLowerCase()} still push.`}
                      </span>
                      <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void openPull()}>
                        Open Pull review
                      </button>
                    </div>
                  </div>
                )}
                <div className="catalog-row-list">
                  {pushRows.map((row) => (
                    <div key={row.row_id} className="catalog-review-row">
                      <ResourceRow
                        row={row}
                        storageName={storageName}
                        selectable
                        selected={pushSelected.has(row.row_id)}
                        onToggle={() => setPushSelected((current) => {
                          const next = new Set(current);
                          if (next.has(row.row_id)) next.delete(row.row_id);
                          else next.add(row.row_id);
                          return next;
                        })}
                      />
                      {/* Under the row that raised it, the way the Pull
                          review's overwrite acknowledgement sits under its
                          own row: a warning collected at the foot of the list
                          makes you count rows to find out who it is about. */}
                      {pushSelected.has(row.row_id) && row.warnings.map((warning) => (
                        <label key={warning.digest} className="catalog-row-ack">
                          <input
                            type="checkbox"
                            checked={pushAcks.has(warning.digest)}
                            onChange={(event) => setPushAcks((current) => {
                              const next = new Set(current);
                              if (event.target.checked) next.add(warning.digest);
                              else next.delete(warning.digest);
                              return next;
                            })}
                          />
                          <span>{warning.message}</span>
                        </label>
                      ))}
                    </div>
                  ))}
                  {pushRows.length === 0 && (
                    <div className="sync-history-state">{`Nothing on this Mac is ahead of ${storageName ?? "storage"}.`}</div>
                  )}
                </div>
              </div>
            </div>
            <footer className="sync-inline-action-footer sync-sync-review-footer">
              {unacknowledged && <span role="status"><small>Acknowledge the warnings above before pushing.</small></span>}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={busy || pushSelected.size === 0 || unacknowledged}
                onClick={() => void runPush()}
              >
                <ThreadStatusIcon status="local-outstanding" size={16} />
                Push
              </button>
            </footer>
          </section>
        </main>
      </>
    );
  }

  // ---- Pull review -------------------------------------------------------

  if (mode.kind === "pull") {
    const chosenLabel = (entry: PullEntry) => {
      const actionId = pullChoice[entry.row.row_id];
      const action = entry.plan.item.actions.find((candidate) => candidate.action_id === actionId);
      return action ? actionLabel(action) : "Skip";
    };
    const needsAck = (entry: PullEntry) => {
      const actionId = pullChoice[entry.row.row_id];
      return entry.plan.item.actions
        .find((candidate) => candidate.action_id === actionId)?.requires_destructive_approval ?? false;
    };
    const claimed = pullEntries.filter((entry) => entry.plan.conflicting_claims.length > 0);
    const blocked = pullEntries.some((entry) => pullSelected.has(entry.row.row_id)
      && needsAck(entry) && !pullAcks.has(entry.row.row_id));
    return (
      <>
        {bar}
        <main className="sync-main sync-project-links-page sync-sync-review-page">
          <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="catalog-pull-title">
            <header className="sync-inline-action-header sync-sync-review-header">
              <div className="sync-sync-review-title"><h2 id="catalog-pull-title">Pull review</h2></div>
              <SyncReviewSelectionBar
                count={pullSelected.size}
                total={pullEntries.length}
                busy={busy}
                recommendedTitle={`Use the recommended local actions (${pullEntries.length})`}
                recommendedAriaLabel={`Use recommended selection (${pullEntries.length})`}
                recommendedDisabled={busy}
                onRecommended={() => {
                  setPullSelected(new Set(pullEntries
                    .filter((entry) => entry.plan.item.default_action_id)
                    .map((entry) => entry.row.row_id)));
                  setPullChoice(Object.fromEntries(pullEntries
                    .filter((entry) => entry.plan.item.default_action_id)
                    .map((entry) => [entry.row.row_id, entry.plan.item.default_action_id as string])));
                  setPullAcks(new Set());
                }}
                clearDisabled={pullSelected.size === 0}
                onClear={() => setPullSelected(new Set())}
                close={{ ariaLabel: "Close Pull review", onClose: () => setMode({ kind: "list" }) }}
              />
            </header>
            <div className="sync-inline-action-content sync-sync-review-content">
              <div className="sync-inline-action-scroll sync-sync-review-scroll">
                {notices}
                {claimed.map((entry) => (
                  <div key={entry.row.row_id} className="sync-review-banners">
                    <div className="sync-callout warning" role="status">
                      <Icon name="alert-triangle" size={16} />
                      <span>
                        <strong>
                          {`${entry.row.display_name} is claimed by ${entry.plan.conflicting_claims
                            .map((claim) => storages.find((storage) => storage.id === claim.storage_id)?.name ?? claim.storage_id)
                            .join(", ")}`}
                        </strong>
                        A different version of this local target is pinned there. Preserve a copy, acknowledge overwrite, or Skip.
                      </span>
                    </div>
                  </div>
                ))}
                <div className="catalog-row-list" ref={pullListRef}>
                  {pullEntries.map((entry) => {
                    const included = pullSelected.has(entry.row.row_id);
                    const expanded = openChoices === entry.row.row_id;
                    const chosen = chosenLabel(entry);
                    return (
                      <div key={entry.row.row_id} className="catalog-review-row">
                        <ResourceRow
                          row={entry.row}
                          storageName={storageName}
                          selectable
                          selected={included}
                          onToggle={() => setPullSelected((current) => {
                            const next = new Set(current);
                            if (next.has(entry.row.row_id)) next.delete(entry.row.row_id);
                            else next.add(entry.row.row_id);
                            return next;
                          })}
                          action={included ? {
                            chosen,
                            options: entry.plan.item.actions.map((action) => ({
                              id: action.action_id,
                              label: actionLabel(action),
                            })),
                            expanded,
                            destructive: needsAck(entry),
                            onToggle: () => setOpenChoices(expanded ? null : entry.row.row_id),
                            onChoose: (actionId) => {
                              setPullChoice((current) => ({ ...current, [entry.row.row_id]: actionId }));
                              setPullAcks((current) => {
                                const next = new Set(current);
                                next.delete(entry.row.row_id);
                                return next;
                              });
                              setOpenChoices(null);
                            },
                          } : undefined}
                        />
                        {included && needsAck(entry) && (
                          /* Sits under the row it belongs to, not in the
                             footer: the footer says why the button is off,
                             this says which row turned it off. */
                          <label className="catalog-row-ack">
                            <input
                              type="checkbox"
                              checked={pullAcks.has(entry.row.row_id)}
                              onChange={(event) => setPullAcks((current) => {
                                const next = new Set(current);
                                if (event.target.checked) next.add(entry.row.row_id);
                                else next.delete(entry.row.row_id);
                                return next;
                              })}
                            />
                            <span>{`I understand this replaces the current ${entry.row.display_name} on this Mac.`}</span>
                          </label>
                        )}
                      </div>
                    );
                  })}
                  {Object.entries(pullBlocked).map(([rowId, reason]) => (
                    <div key={rowId} className="sync-callout warning" role="status">
                      {`${rows.find((row) => row.row_id === rowId)?.display_name ?? rowId}: ${reason}`}
                    </div>
                  ))}
                  {pullEntries.length === 0 && !busy && (
                    <div className="sync-history-state">{`Nothing in ${storageName ?? "storage"} is ahead of this Mac.`}</div>
                  )}
                </div>
              </div>
            </div>
            <footer className="sync-inline-action-footer sync-sync-review-footer">
              {blocked && <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={busy || pullSelected.size === 0 || blocked}
                onClick={() => void runPull()}
              >
                <ThreadStatusIcon status="cloud-outstanding" size={16} />
                Pull
              </button>
            </footer>
          </section>
        </main>
      </>
    );
  }

  // ---- the detail behind a row -------------------------------------------

  if (mode.kind === "detail") {
    const forkable = detail && detail.registration.resource_kind !== "plugin_install_intent";
    const detailBusy = busy || inspectingLinks;
    return (
      <>
        {bar}
        <main className="sync-main sync-project-links-page catalog-category-page">
          <section className="profile-links-section" aria-label={detail?.registration.display_name ?? "Resource"}>
            {notices}
            {recovery.map((message) => (
              <div key={message} className="sync-callout warning" role="status">{message}</div>
            ))}
            {!detail ? (
              <div className="profile-links-empty">Loading…</div>
            ) : (
              <>
                {/* No heading: the trail's last crumb already names this
                    resource, and carries the provider and kind in its own
                    title. The bar owns the title on every screen. */}
                {forkable && (
                  <form
                    className="catalog-inline-form catalog-fork-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void runFork(detail);
                    }}
                  >
                    <label>
                      <span>Fork local resource</span>
                      <input
                        placeholder={`${detail.registration.display_name}-local-copy`}
                        value={forkName}
                        onChange={(event) => setForkName(event.target.value)}
                      />
                    </label>
                    <button type="submit" className="btn btn-ghost" disabled={detailBusy || forkName.trim() === ""}>
                      <Icon name="git-fork" size={13} /> Fork
                    </button>
                  </form>
                )}
                <div className="catalog-link-grid">
                  {detail.links.map(({ link, storage_name: name }) => {
                    const inspection = linkInspections[link.link_id];
                    const state = inspection?.state;
                    return (
                      <article key={link.link_id} className="catalog-link-card">
                        <div className="catalog-link-card-heading">
                          <div>
                            <h2>{name}</h2>
                            <span className={`catalog-state-badge ${state ?? "loading"}`}>
                              {state ? linkStateLabel(state) : "Checking…"}
                            </span>
                          </div>
                          <button
                            type="button"
                            className="sync-history-icon-action"
                            title="Refresh local and stored resource"
                            aria-label={`Refresh ${name}`}
                            disabled={detailBusy}
                            onClick={() => void inspectLinks(detail.registration.registration_id)}
                          >
                            <Icon name="refresh" size={14} />
                          </button>
                        </div>
                        {inspection?.blocker && <div className="sync-callout error">{inspection.blocker}</div>}
                        <div className="catalog-link-meta">
                          <span>{`Repository ${link.repository_id.slice(0, 20)}…`}</span>
                          {inspection?.remote_revision != null && <span>{`Revision ${inspection.remote_revision}`}</span>}
                          {!inspection?.local_present && <span>missing locally</span>}
                        </div>
                        <div className="catalog-link-actions">
                          <button
                            type="button"
                            className="btn btn-ghost danger"
                            disabled={detailBusy}
                            onClick={() => void unlinkOne(detail, link.link_id, link.revision)}
                          >
                            Unlink
                          </button>
                        </div>
                      </article>
                    );
                  })}
                  {detail.links.length === 0 && (
                    <div className="profile-links-empty">
                      This resource has no storage links. Push adds it to the storage in the bar.
                    </div>
                  )}
                </div>
                {detail.links.length === 0 && (
                  <button
                    type="button"
                    className="btn btn-ghost danger"
                    disabled={detailBusy}
                    onClick={() => void removeResource(detail)}
                  >
                    <Icon name="trash" size={13} /> Remove registration
                  </button>
                )}
              </>
            )}
          </section>
        </main>
      </>
    );
  }

  // ---- the list ----------------------------------------------------------

  return (
    <>
      {bar}
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        {notices}
        {loading && !page ? (
          <div className="sync-history-state">{`Loading ${label}…`}</div>
        ) : rows.length === 0 ? (
          /* Its own container: wrapping one sentence in the list card drew a
             second border around it. Nor does it name the home or the
             storage — the crumb bar above is already saying both. */
          <div className="sync-history-state">
            {`No ${kind.label.toLowerCase()} on this Mac, and none stored yet.`}
          </div>
        ) : (
          <section className="catalog-list-section">
            <div className="catalog-list-heading">
              {/* The last crumb already says "Skills · conf2", so the section
                  keeps its accessible name and shows the one thing the crumb
                  cannot: how many. */}
              <h2 className="sync-visually-hidden">{label}</h2>
              <span className="catalog-list-count">
                {`${rows.length} ${rows.length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase()}`}
              </span>
            </div>
            <div className="catalog-row-list">
              {rows.map((row) => (
                <ResourceRow
                  key={row.row_id}
                  row={row}
                  storageName={storageName}
                  onOpen={row.registration_id
                    ? () => setMode({ kind: "detail", registrationId: row.registration_id! })
                    : undefined}
                />
              ))}
            </div>
          </section>
        )}
      </main>
    </>
  );
}

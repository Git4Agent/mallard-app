import { t } from "../../i18n";
import { backendText } from "../../i18n/backend";
import type { SyncOutcomeView, OutcomeAction } from "../sync-review/outcome";
import type { CatalogNouns } from "./model";
import type {
  CatalogActionResult,
  CatalogRemovalResult,
  CatalogScopePullOutcome,
  CatalogScopePushOutcome,
  CatalogSyncProgress,
} from "./types";

/* The backend's progress lines and fixed result messages are English; each
   carries a code, and the name it was built from, to translate by. */

/** A Push or Pull progress line: "Pushing tidal-notes… (1/3)". */
export function catalogProgressText(event: CatalogSyncProgress): string {
  const line = event.stage
    ? backendText(`catalogProgress.${event.stage}`, event.message, { name: event.name ?? "" })
    : event.message;
  return `${line} (${event.completed}/${event.total})`;
}

/** An action result's message: a fixed one translated by its code, an
 *  error or a planning detail as the backend wrote it. */
export function catalogActionMessage(row: Pick<CatalogActionResult, "message" | "message_code" | "message_name">): string | null | undefined {
  return row.message && row.message_code
    ? backendText(`catalogOutcome.${row.message_code}`, row.message, { name: row.message_name ?? "" })
    : row.message;
}

/** A finished removal's sentence; a failed one's message is an error. */
export function catalogRemovalMessage(result: CatalogRemovalResult): string {
  return result.removed && result.display_name != null
    ? backendText("catalogOutcome.removed", result.message, { name: result.display_name })
    : result.message;
}

export interface CatalogOutcomeAction extends OutcomeAction {
  kind: "push" | "pull";
  resourceIds: string[];
}
export function pushOutcome(result: CatalogScopePushOutcome): SyncOutcomeView<CatalogOutcomeAction> {
  const remaining = result.results.filter((row) => row.state === "failed" || row.state === "not_attempted");
  const completed = result.results.length - remaining.length;
  const attention = remaining.length > 0 || !!result.stopped;
  return {
    title: attention ? t("library:outcome.pushedItems", { done: completed, total: result.results.length }) : t("library:outcome.pushComplete"),
    tone: attention ? "warning" : "success",
    message: result.stopped ? t("library:outcome.publicationsKept") : undefined,
    stats: [
      { icon: "upload", value: String(completed), label: t("library:outcome.completed") },
      ...(result.revision == null ? [] : [{ icon: "git-branch" as const, label: "Version", value: String(result.revision), lead: true }]),
    ],
    rows: result.results.map((row) => ({ key: row.resource_id, title: row.display_name,
      status: row.state === "failed" ? "failed" : row.state === "not_attempted" ? "not_attempted" : "synced", reason: row.message })),
    actions: attention ? [{ kind: "push", label: remaining.length ? t("library:outcome.reviewRemaining") : t("library:verb.compareAgain"), resourceIds: remaining.map((row) => row.resource_id) }] : [],
    defaultOpen: attention, technical: result.stopped ? [{ label: t("library:outcome.pushStopped"), value: result.stopped }] : [],
  };
}
export function pullOutcome(result: CatalogScopePullOutcome): SyncOutcomeView<CatalogOutcomeAction> {
  const failed = result.results.filter((row) => ["failed", "partial", "manual", "skipped"].includes(row.outcome));
  const applied = result.results.filter((row) => ["installed", "updated", "preserved_and_updated"].includes(row.outcome));
  const attention = failed.length > 0 || !!result.stopped || !result.remote_accepted;
  return {
    title: attention ? t("library:outcome.pullNeedsAttention") : t("library:outcome.pullComplete"), tone: attention ? "warning" : "success",
    message: attention ? t("library:outcome.changesKept") : undefined,
    stats: [{ icon: "download", value: String(applied.length), label: t("library:outcome.applied") }],
    rows: result.results.map((row) => ({ key: row.resource_id, title: row.display_name,
      // i18n-ignore: backend text
      status: row.message === "Not pulled" ? "not_pulled" : row.outcome === "failed" || row.outcome === "partial" ? "failed" : row.outcome === "skipped" || row.outcome === "manual" ? "skipped" : row.outcome === "preserved_and_updated" || row.outcome === "kept_local" ? "preserved" : "synced",
      reason: catalogActionMessage(row) })),
    actions: attention ? [{ kind: "pull", label: failed.length ? t("library:outcome.reviewRemaining") : t("library:verb.compareAgain"), resourceIds: failed.map((row) => row.resource_id) }] : [],
    defaultOpen: attention, technical: result.stopped ? [{ label: t("library:outcome.pullStopped"), value: result.stopped }] : [],
  };
}

/** What a Mallard Cloud outcome names, as a project's names its sessions:
 *  the kind's nouns, each item's repository on line two where a session
 *  prints its thread id, and when it happened. */
export interface CatalogCloudOutcomeWords {
  nouns: CatalogNouns;
  addressOf: (resourceId: string) => string | null | undefined;
  at: number;
}

/** A Mallard Cloud Push in the project's words: the count names what moved,
 *  one repository's version leads, and every row names its repository. */
export function cloudPushOutcome(result: CatalogScopePushOutcome, words: CatalogCloudOutcomeWords): SyncOutcomeView<CatalogOutcomeAction> {
  const remaining = result.results.filter((row) => row.state === "failed" || row.state === "not_attempted");
  const pushed = result.results.length - remaining.length;
  const attention = remaining.length > 0 || !!result.stopped;
  const version = result.results.length === 1 ? result.results[0].revision ?? null : null;
  return {
    title: attention
      ? words.nouns.say((noun) => t("library:outcome.pushedOf", { done: pushed, count: result.results.length, noun }), result.results.length)
      : t("library:outcome.pushComplete"),
    tone: attention ? "warning" : "success",
    message: result.stopped ? t("library:outcome.publicationsKept") : undefined,
    stats: [
      { icon: "upload", value: String(pushed), label: attention ? t("library:outcome.pushed") : words.nouns.counted(pushed) },
      ...(remaining.length > 0 ? [{ icon: "octagon-x" as const, value: String(remaining.length), label: t("library:outcome.notPushed") }] : []),
      ...(version == null ? [] : [{ icon: "git-branch" as const, label: "Version", value: String(version), lead: true }]),
    ],
    rows: result.results.map((row) => {
      const done = row.state === "published" || row.state === "unchanged";
      return {
        key: row.resource_id,
        title: row.display_name,
        status: done ? "synced" as const : row.state === "failed" ? "failed" as const : "not_attempted" as const,
        reason: row.message,
        threadId: words.addressOf(row.resource_id) ?? null,
        updatedAt: done ? words.at : null,
      };
    }),
    actions: attention ? [{ kind: "push", label: remaining.length ? t("library:outcome.reviewRemaining") : t("library:verb.compareAgain"), resourceIds: remaining.map((row) => row.resource_id) }] : [],
    defaultOpen: attention,
    technical: result.stopped ? [{ label: t("library:outcome.pushStopped"), value: result.stopped }] : [],
  };
}

/** A Mallard Cloud Pull in the project's words: what was restored, what
 *  was kept independently (the local copy, under its new name), what still
 *  needs attention, and one repository's version. */
export function cloudPullOutcome(result: CatalogScopePullOutcome, words: CatalogCloudOutcomeWords & {
  /** The name a keep-local action gave the local copy. */
  keptAs: (resourceId: string) => string | null | undefined;
  /** The version one item's Pull read; null for a batch. */
  version: number | null;
}): SyncOutcomeView<CatalogOutcomeAction> {
  // i18n-ignore: backend text
  const notPulled = result.results.filter((row) => row.message === "Not pulled");
  // i18n-ignore: backend text
  const failed = result.results.filter((row) => row.message !== "Not pulled" && (row.outcome === "failed" || row.outcome === "partial"));
  // i18n-ignore: backend text
  const skipped = result.results.filter((row) => row.message !== "Not pulled" && (row.outcome === "skipped" || row.outcome === "manual"));
  const restored = result.results.filter((row) => row.outcome === "installed" || row.outcome === "updated");
  const kept = result.results.filter((row) => row.outcome === "preserved_and_updated");
  const unfinished = [...failed, ...skipped, ...notPulled];
  const problems = failed.length + skipped.length;
  const attention = unfinished.length > 0 || !!result.stopped || !result.remote_accepted;
  return {
    title: result.stopped
      ? t("library:outcome.pullStopped")
      : problems > 0
        ? words.nouns.say((noun) => t("library:outcome.needAttention", { count: problems, noun }), problems)
        : attention ? t("library:outcome.pullNeedsAttention") : t("library:outcome.pullComplete"),
    tone: attention ? "warning" : "success",
    message: result.stopped ?? (attention ? t("library:outcome.changesKept") : undefined),
    stats: [
      { icon: "download", value: String(restored.length), label: t("library:outcome.restored") },
      ...(kept.length > 0 ? [{ icon: "git-fork" as const, value: String(kept.length), label: t("library:outcome.keptIndependently") }] : []),
      ...(failed.length > 0 ? [{ icon: "octagon-x" as const, value: String(failed.length), label: t("library:outcome.failed") }] : []),
      ...(skipped.length > 0 ? [{ icon: "git-compare-arrows" as const, value: String(skipped.length), label: t("library:outcome.skipped") }] : []),
      ...(notPulled.length > 0 ? [{ icon: "git-compare-arrows" as const, value: String(notPulled.length), label: t("library:outcome.notPulled") }] : []),
      ...(words.version == null ? [] : [{ icon: "git-branch" as const, label: "Version", value: String(words.version), lead: true }]),
    ],
    rows: [
      ...restored.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "synced" as const, threadId: words.addressOf(row.resource_id) ?? null,
      })),
      ...kept.map((row) => {
        const copy = words.keptAs(row.resource_id) ?? row.display_name;
        return {
          key: `${row.resource_id}:kept`, title: copy, status: "preserved" as const, threadId: copy,
          reason: words.nouns.say((noun) => t("library:outcome.keptAs", { name: row.display_name, noun }), 1),
        };
      }),
      ...failed.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "failed" as const, reason: row.message, threadId: words.addressOf(row.resource_id) ?? null,
      })),
      ...skipped.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "skipped" as const, reason: catalogActionMessage(row), threadId: words.addressOf(row.resource_id) ?? null,
      })),
      ...notPulled.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "not_pulled" as const, threadId: words.addressOf(row.resource_id) ?? null,
      })),
    ],
    actions: attention
      ? [{ kind: "pull", label: unfinished.length ? t("library:outcome.reviewRemaining") : t("library:verb.compareAgain"), resourceIds: unfinished.map((row) => row.resource_id) }]
      : [],
    defaultOpen: attention,
    technical: result.stopped ? [{ label: t("library:outcome.pullStopped"), value: result.stopped }] : [],
  };
}

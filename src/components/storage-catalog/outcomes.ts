import type { SyncOutcomeView, OutcomeAction } from "../sync-review/outcome";
import type { CatalogScopePullOutcome, CatalogScopePushOutcome } from "./types";

export interface CatalogOutcomeAction extends OutcomeAction {
  kind: "push" | "pull";
  resourceIds: string[];
}
export function pushOutcome(result: CatalogScopePushOutcome): SyncOutcomeView<CatalogOutcomeAction> {
  const remaining = result.results.filter((row) => row.state === "failed" || row.state === "not_attempted");
  const completed = result.results.length - remaining.length;
  const attention = remaining.length > 0 || !!result.stopped;
  return {
    title: attention ? `Pushed ${completed} of ${result.results.length} items` : "Push complete",
    tone: attention ? "warning" : "success",
    message: result.stopped ? "Completed publications were kept. Compare again before retrying." : undefined,
    stats: [
      { icon: "upload", value: String(completed), label: "completed" },
      ...(result.revision == null ? [] : [{ icon: "git-branch" as const, label: "Version", value: String(result.revision), lead: true }]),
    ],
    rows: result.results.map((row) => ({ key: row.resource_id, title: row.display_name,
      status: row.state === "failed" ? "failed" : row.state === "not_attempted" ? "not_attempted" : "synced", reason: row.message })),
    actions: attention ? [{ kind: "push", label: remaining.length ? "Review remaining items" : "Compare again", resourceIds: remaining.map((row) => row.resource_id) }] : [],
    defaultOpen: attention, technical: result.stopped ? [{ label: "Push stopped", value: result.stopped }] : [],
  };
}
export function pullOutcome(result: CatalogScopePullOutcome): SyncOutcomeView<CatalogOutcomeAction> {
  const failed = result.results.filter((row) => ["failed", "partial", "manual", "skipped"].includes(row.outcome));
  const applied = result.results.filter((row) => ["installed", "updated", "preserved_and_updated"].includes(row.outcome));
  const attention = failed.length > 0 || !!result.stopped || !result.remote_accepted;
  return {
    title: attention ? "Pull needs attention" : "Pull complete", tone: attention ? "warning" : "success",
    message: attention ? "Some items still need review. Completed changes were kept." : undefined,
    stats: [{ icon: "download", value: String(applied.length), label: "applied" }],
    rows: result.results.map((row) => ({ key: row.resource_id, title: row.display_name,
      status: row.message === "Not pulled" ? "not_pulled" : row.outcome === "failed" || row.outcome === "partial" ? "failed" : row.outcome === "skipped" || row.outcome === "manual" ? "skipped" : row.outcome === "preserved_and_updated" || row.outcome === "kept_local" ? "preserved" : "synced",
      reason: row.message })),
    actions: attention ? [{ kind: "pull", label: failed.length ? "Review remaining items" : "Compare again", resourceIds: failed.map((row) => row.resource_id) }] : [],
    defaultOpen: attention, technical: result.stopped ? [{ label: "Pull stopped", value: result.stopped }] : [],
  };
}

/** What a Mallard Cloud outcome names, as a project's names its sessions:
 *  the kind's noun, each item's repository on line two where a session
 *  prints its thread id, and when it happened. */
export interface CatalogCloudOutcomeWords {
  noun: (count: number) => string;
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
    title: attention ? `Pushed ${pushed} of ${result.results.length} ${words.noun(result.results.length)}` : "Push complete",
    tone: attention ? "warning" : "success",
    message: result.stopped ? "Completed publications were kept. Compare again before retrying." : undefined,
    stats: [
      { icon: "upload", value: String(pushed), label: attention ? "pushed" : words.noun(pushed) },
      ...(remaining.length > 0 ? [{ icon: "octagon-x" as const, value: String(remaining.length), label: "not pushed" }] : []),
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
    actions: attention ? [{ kind: "push", label: remaining.length ? "Review remaining items" : "Compare again", resourceIds: remaining.map((row) => row.resource_id) }] : [],
    defaultOpen: attention,
    technical: result.stopped ? [{ label: "Push stopped", value: result.stopped }] : [],
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
  const notPulled = result.results.filter((row) => row.message === "Not pulled");
  const failed = result.results.filter((row) => row.message !== "Not pulled" && (row.outcome === "failed" || row.outcome === "partial"));
  const skipped = result.results.filter((row) => row.message !== "Not pulled" && (row.outcome === "skipped" || row.outcome === "manual"));
  const restored = result.results.filter((row) => row.outcome === "installed" || row.outcome === "updated");
  const kept = result.results.filter((row) => row.outcome === "preserved_and_updated");
  const unfinished = [...failed, ...skipped, ...notPulled];
  const problems = failed.length + skipped.length;
  const attention = unfinished.length > 0 || !!result.stopped || !result.remote_accepted;
  return {
    title: result.stopped
      ? "Pull stopped"
      : problems > 0
        ? `${problems} ${words.noun(problems)} ${problems === 1 ? "needs" : "need"} attention`
        : attention ? "Pull needs attention" : "Pull complete",
    tone: attention ? "warning" : "success",
    message: result.stopped ?? (attention ? "Some items still need review. Completed changes were kept." : undefined),
    stats: [
      { icon: "download", value: String(restored.length), label: "restored" },
      ...(kept.length > 0 ? [{ icon: "git-fork" as const, value: String(kept.length), label: "kept independently" }] : []),
      ...(failed.length > 0 ? [{ icon: "octagon-x" as const, value: String(failed.length), label: "failed" }] : []),
      ...(skipped.length > 0 ? [{ icon: "git-compare-arrows" as const, value: String(skipped.length), label: "skipped" }] : []),
      ...(notPulled.length > 0 ? [{ icon: "git-compare-arrows" as const, value: String(notPulled.length), label: "not pulled" }] : []),
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
          reason: `The stored ${row.display_name} was restored, and the local copy was kept as an independent ${words.noun(1)}.`,
        };
      }),
      ...failed.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "failed" as const, reason: row.message, threadId: words.addressOf(row.resource_id) ?? null,
      })),
      ...skipped.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "skipped" as const, reason: row.message, threadId: words.addressOf(row.resource_id) ?? null,
      })),
      ...notPulled.map((row) => ({
        key: row.resource_id, title: row.display_name, status: "not_pulled" as const, threadId: words.addressOf(row.resource_id) ?? null,
      })),
    ],
    actions: attention
      ? [{ kind: "pull", label: unfinished.length ? "Review remaining items" : "Compare again", resourceIds: unfinished.map((row) => row.resource_id) }]
      : [],
    defaultOpen: attention,
    technical: result.stopped ? [{ label: "Pull stopped", value: result.stopped }] : [],
  };
}

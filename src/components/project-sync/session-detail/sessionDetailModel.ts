import type {
  ProjectSessionTimelinePage,
  SessionCheckpointJump,
  SessionCount,
  SessionEventCounts,
  SessionTimelineEvent,
  SessionTimelineEventKind,
  SessionToolCategory,
} from "../../../types";

export type ActiveProjectSubview =
  | { kind: "overview" }
  | {
    kind: "session_detail";
    localProjectId: string;
    resourceId: string;
    occurrenceKey: string;
    initialCheckpointId?: string;
  };

export interface OpenSessionDetailRequest {
  resourceId: string;
  occurrenceKey: string;
  initialCheckpointId?: string;
}

export interface SessionDetailFilters {
  prompts: boolean;
  responses: boolean;
  intermediateSteps: boolean;
  checkpoints: boolean;
  toolCalls: boolean;
  toolCategories: Record<SessionToolCategory, boolean>;
}

export const SESSION_TOOL_CATEGORIES: readonly SessionToolCategory[] = [
  "file_edit",
  "shell",
  "read",
  "other",
];

export const SESSION_TOOL_CATEGORY_LABELS: Record<SessionToolCategory, string> = {
  file_edit: "File edits",
  shell: "Bash / Shell",
  read: "Read",
  other: "Other",
};

export const SESSION_EVENT_KIND_LABELS: Record<SessionTimelineEventKind, string> = {
  prompt: "Prompt",
  response: "Response",
  intermediate_step: "Intermediate step",
  checkpoint: "Checkpoint",
  tool_call: "Tool call",
};

export function sessionCountSupported(count: SessionCount): boolean {
  return count.quality !== "unsupported";
}

export function createDefaultSessionDetailFilters(
  counts: SessionEventCounts,
): SessionDetailFilters {
  return {
    prompts: sessionCountSupported(counts.prompts),
    responses: sessionCountSupported(counts.responses),
    intermediateSteps: false,
    checkpoints: sessionCountSupported(counts.checkpoints),
    toolCalls: false,
    toolCategories: {
      file_edit: sessionCountSupported(counts.file_edits),
      shell: sessionCountSupported(counts.shell_calls),
      read: sessionCountSupported(counts.reads),
      other: sessionCountSupported(counts.other_tools),
    },
  };
}

export function includedSessionEventKinds(
  filters: SessionDetailFilters,
): SessionTimelineEventKind[] {
  const kinds: SessionTimelineEventKind[] = [];
  if (filters.prompts) kinds.push("prompt");
  if (filters.responses) kinds.push("response");
  if (filters.intermediateSteps) kinds.push("intermediate_step");
  if (filters.checkpoints) kinds.push("checkpoint");
  if (filters.toolCalls) kinds.push("tool_call");
  return kinds;
}

export function includedSessionToolCategories(
  filters: SessionDetailFilters,
): SessionToolCategory[] {
  return SESSION_TOOL_CATEGORIES.filter((category) => filters.toolCategories[category]);
}

export function sessionDetailFilterFingerprint(filters: SessionDetailFilters): string {
  return JSON.stringify([
    includedSessionEventKinds(filters),
    includedSessionToolCategories(filters),
  ]);
}

export function sessionToolParentState(
  filters: SessionDetailFilters,
  counts: SessionEventCounts,
): { checked: boolean; indeterminate: boolean; disabled: boolean } {
  const supported = SESSION_TOOL_CATEGORIES.filter((category) => (
    sessionCountSupported(sessionToolCategoryCount(counts, category))
  ));
  const selected = supported.filter((category) => filters.toolCategories[category]).length;
  return {
    checked: filters.toolCalls && selected > 0 && selected === supported.length,
    indeterminate: filters.toolCalls && selected > 0 && selected < supported.length,
    disabled: !sessionCountSupported(counts.tool_calls),
  };
}

export function sessionToolCategoryCount(
  counts: SessionEventCounts,
  category: SessionToolCategory,
): SessionCount {
  switch (category) {
    case "file_edit": return counts.file_edits;
    case "shell": return counts.shell_calls;
    case "read": return counts.reads;
    case "other": return counts.other_tools;
  }
}

export function toggleSessionToolParent(
  filters: SessionDetailFilters,
  counts: SessionEventCounts,
): SessionDetailFilters {
  if (!sessionCountSupported(counts.tool_calls)) return filters;
  if (filters.toolCalls) return { ...filters, toolCalls: false };

  const hasSelectedSupportedCategory = SESSION_TOOL_CATEGORIES.some((category) => (
    sessionCountSupported(sessionToolCategoryCount(counts, category))
    && filters.toolCategories[category]
  ));
  return {
    ...filters,
    toolCalls: true,
    toolCategories: hasSelectedSupportedCategory
      ? filters.toolCategories
      : Object.fromEntries(SESSION_TOOL_CATEGORIES.map((category) => [
        category,
        sessionCountSupported(sessionToolCategoryCount(counts, category)),
      ])) as Record<SessionToolCategory, boolean>,
  };
}

export function formatSessionCount(count: SessionCount): string {
  if (count.quality === "unsupported" || count.value == null) return "Unavailable";
  const value = new Intl.NumberFormat().format(count.value);
  return count.quality === "lower_bound" ? `${value}+` : value;
}

export function sessionCountQualityLabel(count: SessionCount): string | null {
  switch (count.quality) {
    case "exact": return null;
    case "lower_bound": return "At least";
    case "inferred": return "Inferred";
    case "unsupported": return "Unavailable";
  }
}

export function mergeSessionTimelinePages(
  previous: SessionTimelineEvent[],
  page: ProjectSessionTimelinePage,
  replace = false,
): SessionTimelineEvent[] {
  const byId = new Map<string, SessionTimelineEvent>();
  if (!replace) previous.forEach((event) => byId.set(event.event_id, event));
  page.events.forEach((event) => byId.set(event.event_id, event));
  return [...byId.values()].sort((left, right) => (
    left.ordinal - right.ordinal || left.event_id.localeCompare(right.event_id)
  ));
}

export function eventAnchorId(eventId: string): string {
  return `session-event-${eventId.replace(/[^a-z0-9_-]/gi, "-")}`;
}

export function checkpointAnchorEventId(
  checkpoint: SessionCheckpointJump,
): string {
  return checkpoint.event_id;
}

export function findCheckpointJump(
  checkpoints: SessionCheckpointJump[],
  identifier: string,
): SessionCheckpointJump | null {
  return checkpoints.find((checkpoint) => (
    checkpoint.event_id === identifier
    || checkpoint.checkpoint.sha === identifier
    || checkpoint.checkpoint.short_sha === identifier
    || checkpoint.checkpoint.sha.startsWith(identifier)
  )) ?? null;
}

export function formatSessionDetailTimestamp(value?: number | null): string {
  if (value == null) return "Time unavailable";
  const milliseconds = value > 10_000_000_000 ? value : value * 1_000;
  return new Date(milliseconds).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function sessionDetailDateTime(value?: number | null): string | undefined {
  if (value == null) return undefined;
  const milliseconds = value > 10_000_000_000 ? value : value * 1_000;
  return new Date(milliseconds).toISOString();
}

const EXTERNAL_MARKDOWN_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

export function safeMarkdownUrl(url: string): string {
  if (url.startsWith("#")) return url;
  try {
    const parsed = new URL(url);
    return EXTERNAL_MARKDOWN_PROTOCOLS.has(parsed.protocol) ? parsed.toString() : "";
  } catch {
    return "";
  }
}

export function isSafeExternalMarkdownUrl(url: string): boolean {
  if (url.startsWith("#")) return false;
  return safeMarkdownUrl(url) !== "";
}

export interface SessionDetailFailure {
  code: string | null;
  message: string;
}

export function normalizeSessionDetailFailure(reason: unknown): SessionDetailFailure {
  if (typeof reason === "object" && reason !== null) {
    const candidate = reason as { code?: unknown; message?: unknown };
    if (typeof candidate.message === "string") {
      return {
        code: typeof candidate.code === "string" ? candidate.code : null,
        message: candidate.message,
      };
    }
  }
  if (typeof reason === "string") {
    try {
      const parsed = JSON.parse(reason) as { code?: unknown; message?: unknown };
      if (typeof parsed.message === "string") {
        return {
          code: typeof parsed.code === "string" ? parsed.code : null,
          message: parsed.message,
        };
      }
    } catch {
      // Tauri commonly rejects with a plain string. Preserve it verbatim.
    }
    return {
      code: /snapshot[ _-]changed/i.test(reason) ? "snapshot_changed" : null,
      message: reason,
    };
  }
  return { code: null, message: "Session details could not be loaded." };
}

import type {
  ProjectSessionDetail,
  ProjectSessionTimelinePage,
  SessionAgentStatus,
  SessionCount,
  SessionEventCounts,
  SessionTimelineEvent,
  SessionTimelineEventKind,
  SessionToolCallPreview,
  SessionToolCategory,
  ThreadSyncEntry,
} from "../../../types";
import { isPushReviewDecision } from "../../sync-review/pullReviewStatus";
import { formatCompactNumber, formatNumber } from "../../../format";
import { t, uiLocale } from "../../../i18n";
import { syncDecisionReason, syncStatusDetail } from "../syncDetailText";
import type { PushSessionFacts } from "../ProjectActivityPage";
import { isSafeExternalMarkdownUrl, safeMarkdownUrl } from "../../markdown/markdownUrl";
import { resolveProjectPath } from "../../markdown/projectPath";
import {
  clockOptions,
  formatDateTime,
  formatDuration,
  toEpochMilliseconds,
} from "../../../clock";

export type ActiveProjectSubview =
  | { kind: "overview" }
  | {
    kind: "session_detail";
    localProjectId: string;
    resourceId: string;
    occurrenceKey: string;
    /** Set for a row that is only in a storage: the page reads that
     *  storage's copy, and its Pull is the Pull of this one session. */
    storageId?: string;
  };

export interface OpenSessionDetailRequest {
  resourceId: string;
  occurrenceKey: string;
  /** The storage a stored row is read from; absent for a local row. */
  storageId?: string;
}

/** Where a session page reads from: this computer's provider home, or the
 *  storage a stored row sits under. */
export type SessionDetailSource =
  | { kind: "local" }
  | { kind: "stored"; storageId: string; storageName: string };

export function sessionPushDisabledReason(entry: ThreadSyncEntry | undefined): string | null {
  if (!entry) return t("session:push.notIndependent");
  if (!entry.local_present) return t("session:push.noLocal");
  if (isPushReviewDecision(entry.push_decision)) return null;
  switch (entry.push_decision.action) {
    case "no_changes": return t("session:push.upToDate");
    case "require_pull": return t("session:push.requirePull");
    case "require_pull_metadata": return t("session:push.requirePullMetadata");
    default: {
      // A reason the service words itself is translated by its code, a
      // blocker's own sentence by the blocker; an error reads as written.
      const reason = syncDecisionReason(entry.push_decision);
      if (reason != null) return reason;
      return syncStatusDetail(entry) ?? t("session:push.unavailable");
    }
  }
}

/** A hold on Push that the session page says, as a chip under Push: storage
 *  the user must pull first, or a comparison that failed. Up to date,
 *  checking and ready are not holds the page repeats — Push's own state,
 *  tooltip and count already say them. */
export interface SessionPushGate {
  tone: "info" | "danger";
  label: string;
  /** The full sentence: the chip's tooltip and its accessible text. */
  detail: string;
}

export function sessionPushGate(
  entry: ThreadSyncEntry | undefined,
  comparisonError: string | null,
): SessionPushGate | null {
  if (comparisonError) return { tone: "danger", label: t("session:push.cantCompare"), detail: comparisonError };
  if (!entry?.local_present) return null;
  const { action, blocker } = entry.push_decision;
  if (action !== "require_pull" && action !== "require_pull_metadata" && blocker !== "detached_requires_pull") {
    return null;
  }
  const detail = sessionPushDisabledReason(entry);
  return detail ? { tone: "info", label: t("session:push.pullFirst"), detail } : null;
}

/** What the session's page reports for the row of its own Push review: the
 *  page's counts, in the fields the row prints. A count the transcript does
 *  not give prints as zero, and a lower bound marks the readouts partial. */
export function sessionPushFacts(detail: ProjectSessionDetail): PushSessionFacts {
  const { prompts, responses, tool_calls: toolCalls } = detail.event_counts;
  const startedAt = detail.started_at ?? detail.ended_at ?? 0;
  return {
    title: detail.title || t("session:untitled"),
    metrics: {
      started_at: startedAt,
      ended_at: Math.max(detail.ended_at ?? startedAt, startedAt),
      user_round_count: prompts.value ?? 0,
      agent_message_count: responses.value ?? 0,
      tool_call_count: toolCalls.value ?? 0,
      total_tokens: detail.tokens.value ?? null,
      metrics_complete: [detail.tokens, prompts, responses, toolCalls]
        .every((count) => count.quality !== "lower_bound"),
    },
  };
}

export interface SessionDetailFilters {
  prompts: boolean;
  responses: boolean;
  intermediateSteps: boolean;
  checkpoints: boolean;
  toolCalls: boolean;
  agents: boolean;
}

export type SessionFilterKey = keyof SessionDetailFilters;

/** The timeline query requires a category list, so every request names all
 * supported categories. */
export const ALL_SESSION_TOOL_CATEGORIES: readonly SessionToolCategory[] = [
  "file_edit",
  "shell",
  "read",
  "other",
];

/**
 * Who spoke, not what the record calls it. A response is labelled with the
 * provider's own name, so this entry is only the fallback for a provider that
 * reports none.
 */
export function sessionEventKindLabel(kind: SessionTimelineEventKind): string {
  switch (kind) {
    case "prompt": return t("session:speaker.prompt");
    case "response": return t("session:speaker.response");
    case "intermediate_step": return t("session:speaker.intermediateStep");
    case "checkpoint": return t("session:speaker.checkpoint");
    case "tool_call": return t("session:speaker.toolCall");
    case "agent_call": return t("session:speaker.agentCall");
  }
}

/** What the row prints for a run that is not simply done. */
export function sessionAgentStatusLabel(status: SessionAgentStatus): string {
  return t(`session:agentStatus.${status}`);
}

function sessionCountSupported(count: SessionCount): boolean {
  return count.quality !== "unsupported";
}

/** A supported count that is provably zero. Unknown (`null`) is not empty. */
function sessionCountEmpty(count: SessionCount): boolean {
  return sessionCountSupported(count) && count.value === 0;
}

/**
 * Show prompts, replies (including Claude widgets), and supported checkpoints by default.
 * Steps, tool calls, and agents require selection.
 */
export function createDefaultSessionDetailFilters(
  counts: SessionEventCounts,
): SessionDetailFilters {
  return {
    prompts: sessionCountSupported(counts.prompts),
    responses: sessionCountSupported(counts.responses),
    intermediateSteps: false,
    checkpoints: sessionCountSupported(counts.checkpoints),
    toolCalls: false,
    agents: false,
  };
}

function sessionFilterCount(
  counts: SessionEventCounts,
  key: SessionFilterKey,
): SessionCount {
  switch (key) {
    case "prompts": return counts.prompts;
    case "responses": return counts.responses;
    case "intermediateSteps": return counts.intermediate_steps;
    case "checkpoints": return counts.checkpoints;
    case "toolCalls": return counts.tool_calls;
    case "agents": return counts.agent_calls;
  }
}

export interface SessionFilterToggle {
  key: SessionFilterKey;
  label: string;
  count: SessionCount;
}

const SESSION_FILTER_ORDER: readonly SessionFilterKey[] = [
  "prompts",
  "responses",
  "intermediateSteps",
  "checkpoints",
  "toolCalls",
  "agents",
];

/**
 * A toggle appears only when this session actually holds that kind. A control
 * for an empty set is a control that can only disappoint — the previous panel
 * rendered a disabled checkpoint picker and three zero-count categories on a
 * session that had neither.
 */
export function sessionDetailFilterToggles(
  counts: SessionEventCounts,
): SessionFilterToggle[] {
  return SESSION_FILTER_ORDER
    .map((key) => ({ key, label: t(`session:filter.${key}`), count: sessionFilterCount(counts, key) }))
    .filter((toggle) => sessionCountSupported(toggle.count) && !sessionCountEmpty(toggle.count));
}

export interface SessionEventCoverage {
  total: number | null;
  approximate: boolean;
}

/**
 * Every event this session holds across all kinds. `approximate` is set when
 * any contributing count is a lower bound, so the readout can say `31+`
 * rather than claim precision the transcript cannot support.
 */
export function sessionEventCoverage(counts: SessionEventCounts): SessionEventCoverage {
  let total = 0;
  let approximate = false;
  for (const key of SESSION_FILTER_ORDER) {
    const count = sessionFilterCount(counts, key);
    if (!sessionCountSupported(count)) continue;
    if (count.value == null) return { total: null, approximate: false };
    total += count.value;
    if (count.quality !== "exact") approximate = true;
  }
  return { total, approximate };
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
  if (filters.agents) kinds.push("agent_call");
  return kinds;
}

export function sessionDetailFilterFingerprint(filters: SessionDetailFilters): string {
  return JSON.stringify(includedSessionEventKinds(filters));
}

export function formatSessionCount(count: SessionCount): string {
  if (count.quality === "unsupported" || count.value == null) return t("session:count.unavailable");
  const value = formatNumber(count.value);
  return count.quality === "lower_bound" ? `${value}+` : value;
}

/**
 * Header figures are read at a glance and sit in a row that must not reflow, so
 * they are abbreviated; the exact value stays in the element's title.
 */
export function formatCompactCount(count: SessionCount): string {
  if (count.quality === "unsupported" || count.value == null) return t("session:count.unavailable");
  const compact = formatCompactNumber(count.value);
  return count.quality === "lower_bound" ? `${compact}+` : compact;
}

export function sessionCountQualityLabel(count: SessionCount): string | null {
  switch (count.quality) {
    case "exact": return null;
    case "lower_bound": return t("session:count.atLeast");
    case "inferred": return t("session:count.inferred");
    case "unsupported": return t("session:count.unavailable");
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

/** Providers record either seconds or milliseconds; both reach this page. */
const toMilliseconds = toEpochMilliseconds;

export function formatSessionDetailTimestamp(value?: number | null): string {
  if (value == null) return t("session:time.unavailable");
  return formatDateTime(toMilliseconds(value));
}

export function sessionDetailDateTime(value?: number | null): string | undefined {
  if (value == null) return undefined;
  return new Date(toMilliseconds(value)).toISOString();
}

/**
 * Clock time alone; the day it belongs to is stated once, by the divider.
 * 24-hour by default — this is a fixed-width column read down the page, and a
 * locale AM/PM adds three characters of chrome to every row for a distinction
 * the column already makes by sorting. The global clock preference supplies
 * the cycle for every timestamp in the app.
 */
export function formatSessionDetailClock(value?: number | null): string {
  if (value == null) return "--:--";
  return new Date(toMilliseconds(value)).toLocaleTimeString(uiLocale(), clockOptions());
}

function sessionDetailDayKey(value?: number | null): string | null {
  if (value == null) return null;
  const date = new Date(toMilliseconds(value));
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

function formatSessionDetailDay(value?: number | null): string {
  if (value == null) return t("session:time.undated");
  return new Date(toMilliseconds(value)).toLocaleDateString(uiLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** Gaps have their own thresholds and prefix, so they keep their own shape —
 *  only the unit normalisation is shared. */
function elapsedSeconds(from: number, to: number): number {
  return Math.max(0, Math.round((toMilliseconds(to) - toMilliseconds(from)) / 1_000));
}

export function formatSessionDuration(
  started?: number | null,
  ended?: number | null,
): string | null {
  if (started == null || ended == null) return null;
  return formatDuration(started, ended);
}

export interface SessionTurnSpan {
  started: number;
  ended: number;
}

/**
 * First and last loaded timestamps per turn, for the prompt card's duration
 * readout. Spans grow as pages load; a turn whose only loaded event is the
 * prompt has no extent yet and yields no duration — measured, never promised.
 */
export function sessionTurnSpans(
  events: SessionTimelineEvent[],
): Map<string, SessionTurnSpan> {
  const spans = new Map<string, SessionTurnSpan>();
  for (const event of events) {
    if (event.turn_id == null || event.timestamp == null) continue;
    const span = spans.get(event.turn_id);
    if (!span) {
      spans.set(event.turn_id, { started: event.timestamp, ended: event.timestamp });
      continue;
    }
    span.started = Math.min(span.started, event.timestamp);
    span.ended = Math.max(span.ended, event.timestamp);
  }
  return spans;
}

export function sessionTurnDurationLabel(span: SessionTurnSpan | undefined): string | null {
  if (!span || span.ended <= span.started) return null;
  return formatSessionDuration(span.started, span.ended);
}

/**
 * When the session began. One fact, because the strip states one fact per chip
 * — the length is the chip beside this one, and the end is the two of them
 * added, which is arithmetic nobody asked the header to show.
 *
 * The year always prints. It costs six characters on a chip that is read once,
 * deliberately, on a page you opened on purpose — and the alternative is a
 * stamp whose shape depends on the calendar, where `Aug 18` is ambiguous.
 *
 * Nothing is abbreviated here, so this chip carries no tooltip: there is no
 * fuller value to put in one.
 */
export function formatSessionStart(value?: number | null): string | null {
  if (value == null) return null;
  return `${formatSessionDetailDay(value)} · ${formatSessionDetailClock(value)}`;
}

/** Tooltip detail for duration and end time. */
export function formatSessionLengthDetail(
  started?: number | null,
  ended?: number | null,
): string | null {
  const length = formatSessionDuration(started, ended);
  if (length == null || ended == null) return null;
  const sameDay = sessionDetailDayKey(started) === sessionDetailDayKey(ended);
  const end = sameDay
    ? formatSessionDetailClock(ended)
    : `${formatSessionDetailDay(ended)} · ${formatSessionDetailClock(ended)}`;
  return t("session:time.lengthDetail", { length, end });
}

/**
 * The wait between one turn and the next. Rendered beside the clock so a stall
 * — the four minutes an agent spent retrying — is visible without arithmetic.
 * Gaps under a minute are noise and return null.
 */
export function formatSessionGap(
  previous?: number | null,
  current?: number | null,
): string | null {
  if (previous == null || current == null) return null;
  const seconds = elapsedSeconds(previous, current);
  if (seconds < 60) return null;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("session:time.gapMinutes", { value: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("session:time.gapHours", { value: hours });
  return t("session:time.gapDays", { value: Math.round(hours / 24) });
}

export interface SessionDayGroup {
  key: string;
  label: string;
  events: SessionTimelineEvent[];
}

/**
 * The date belongs to the day, not to every line inside it. Undated events join
 * the group they arrived in rather than opening one of their own.
 */
export function groupSessionEventsByDay(
  events: SessionTimelineEvent[],
): SessionDayGroup[] {
  const groups: SessionDayGroup[] = [];
  for (const event of events) {
    const key = sessionDetailDayKey(event.timestamp);
    const current = groups[groups.length - 1];
    if (!current || (key !== null && key !== current.key)) {
      groups.push({
        key: key ?? current?.key ?? "undated",
        label: key === null && !current
          ? formatSessionDetailDay(null)
          : key === null
            ? current.label
            : formatSessionDetailDay(event.timestamp),
        events: [event],
      });
      continue;
    }
    current.events.push(event);
  }
  return groups;
}

/**
 * Providers hand us the call as the agent wrote it, which for Codex means the
 * whole JS wrapper: `const r = await tools.exec_command({cmd:"…","workdir":…})`.
 * Printed verbatim, all 24 rows open with the same 34 characters and the part
 * that tells them apart is exactly what the truncation eats.
 *
 * The wrapper is parsed once and read twice: the *function* names what the call
 * is (`exec_command` → Bash, `read_file` → Read), and the primary *argument* is
 * the command line the row shows. Anything unrecognised is returned untouched,
 * and the full original stays in the row's tooltip and the Arguments pane.
 */
const WRAPPER_CALL = /^(?:const\s+[\w$]+\s*=\s*)?(?:await\s+)?(?:tools\s*\.\s*)?([A-Za-z_$][\w$]*)\s*\(\s*(\{[\s\S]*)/;

/** Checked in order: the first one present is what the call is *about*. */
const PRIMARY_ARGUMENT_KEYS = [
  "cmd",
  "command",
  "file_path",
  "path",
  "prompt",
  "query",
  "pattern",
  "url",
];

function decodeJsonFragment(value: string): string {
  try {
    return JSON.parse(`"${value}"`) as string;
  } catch {
    return value;
  }
}

/** The wrapper's arguments are JS, not JSON: keys may be bare and strings may
 *  be single-quoted or template literals, so this reads all three quote forms. */
function wrapperArgumentValue(argsLiteral: string, key: string): string | null {
  const pattern = new RegExp(`["']?${key}["']?\\s*:\\s*(?:"((?:[^"\\\\]|\\\\.)*)"|'((?:[^'\\\\]|\\\\.)*)'|\`((?:[^\`\\\\]|\\\\.)*)\`)`);
  const match = pattern.exec(argsLiteral);
  if (!match) return null;
  if (match[1] != null) return decodeJsonFragment(match[1]);
  const value = match[2] ?? match[3];
  return value.replace(/\\(['"`\\nt])/g, (_, escaped: string) => (
    escaped === "n" ? "\n" : escaped === "t" ? "\t" : escaped
  ));
}

/**
 * Rows end early rather than running to the chevron: a hard cap, like the
 * reference viewer's, keeps the right side of the list quiet so the eye scans
 * the aligned left edge. The full text stays in the row's tooltip and the
 * Arguments pane. Counted in code points so a CJK or emoji command cannot be
 * split mid-character.
 */
const TOOL_COMMAND_MAX_LENGTH = 64;

function capToolCommand(value: string): string {
  // One line only — a template-literal command can span several, and nowrap
  // would smear them into one with the breaks swallowed.
  const newline = value.indexOf("\n");
  const line = newline < 0 ? value : value.slice(0, newline).trimEnd();
  const truncated = newline >= 0;
  const points = [...line];
  if (points.length <= TOOL_COMMAND_MAX_LENGTH) return truncated ? `${line}…` : line;
  return points.slice(0, TOOL_COMMAND_MAX_LENGTH).join("").trimEnd() + "…";
}

export function toolCommandLabel(summary: string): string {
  const trimmed = summary.trim();
  if (!trimmed) return trimmed;

  const wrapper = WRAPPER_CALL.exec(trimmed);
  if (wrapper) {
    for (const key of PRIMARY_ARGUMENT_KEYS) {
      const value = wrapperArgumentValue(wrapper[2], key);
      if (value?.trim()) return capToolCommand(value.trim());
    }
    // A wrapper with no primary argument still sheds the harness noise: show
    // the call, not the `const r = await tools.` boilerplate around it.
    return capToolCommand(trimmed
      .replace(/^(?:const\s+[\w$]+\s*=\s*)?(?:await\s+)?(?:tools\s*\.\s*)?/, "")
      .trim());
  }

  if (trimmed.startsWith("{")) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      if (typeof parsed === "object" && parsed !== null) {
        const fields = parsed as Record<string, unknown>;
        for (const key of PRIMARY_ARGUMENT_KEYS) {
          const value = fields[key];
          if (typeof value === "string" && value.trim()) return capToolCommand(value.trim());
        }
      }
    } catch {
      // Not JSON after all; fall through to the first line.
    }
  }

  return capToolCommand(trimmed);
}

export interface SessionToolDescriptor {
  label: string;
  category: SessionToolCategory;
}

/**
 * What the row calls the tool. Providers that already record readable names
 * (`Bash`, `Grep`) pass straight through; machine names — the recorded `exec`,
 * or the function inside the JS wrapper — normalise to the action they perform,
 * and carry their category with them so the tint always matches the label.
 * Codex projects nearly everything as category `other`, which is exactly why
 * the wrapper is the better witness when it is present.
 */
const TOOL_NAME_LABELS: Record<string, SessionToolDescriptor> = {
  exec: { label: "Bash", category: "shell" },
  exec_command: { label: "Bash", category: "shell" },
  run_command: { label: "Bash", category: "shell" },
  run_terminal_cmd: { label: "Bash", category: "shell" },
  local_shell: { label: "Bash", category: "shell" },
  shell: { label: "Bash", category: "shell" },
  bash: { label: "Bash", category: "shell" },
  read: { label: "Read", category: "read" },
  read_file: { label: "Read", category: "read" },
  open_file: { label: "Read", category: "read" },
  cat: { label: "Read", category: "read" },
  view: { label: "Read", category: "read" },
  edit: { label: "Edit", category: "file_edit" },
  edit_file: { label: "Edit", category: "file_edit" },
  apply_patch: { label: "Edit", category: "file_edit" },
  write_file: { label: "Edit", category: "file_edit" },
  create_file: { label: "Edit", category: "file_edit" },
  update_file: { label: "Edit", category: "file_edit" },
  str_replace_editor: { label: "Edit", category: "file_edit" },
};

export function sessionToolDescriptor(
  tool: Pick<SessionToolCallPreview, "native_name" | "summary" | "category">,
): SessionToolDescriptor {
  const wrapper = WRAPPER_CALL.exec(tool.summary.trim());
  const name = wrapper?.[1] ?? tool.native_name;
  const hit = TOOL_NAME_LABELS[name.toLowerCase()];
  if (hit) return hit;
  return { label: name, category: tool.category };
}

export interface SessionPayloadView {
  /** What the pane shows by default. */
  text: string;
  /** The stored string, reachable through the raw toggle. */
  raw: string;
  /** Whether the two differ, i.e. whether the raw toggle has a job. */
  transformed: boolean;
}

function unescapePayload(value: string): string {
  const replacements: Record<string, string> = {
    n: "\n",
    t: "\t",
    r: "",
    '"': '"',
    "\\": "\\",
  };
  return value.replace(/\\([ntr"\\])/g, (match, character: string) => (
    replacements[character] ?? match
  ));
}

function isTextPart(value: unknown): value is { text: string } {
  return typeof value === "object"
    && value !== null
    && typeof (value as { text?: unknown }).text === "string";
}

/**
 * Tool results arrive as a stored transport string: usually a JSON envelope
 * whose payload is prose with the newlines escaped. Printing it verbatim — the
 * page's previous behaviour — produced a wall of `\n` that no one can read.
 * This lifts the text out; the raw toggle keeps the original one click away, so
 * nothing is hidden, only unwrapped.
 */
export function readableToolPayload(value: string): SessionPayloadView {
  const raw = value;
  try {
    const parsed: unknown = JSON.parse(value.trim());
    if (Array.isArray(parsed) && parsed.length > 0 && parsed.every(isTextPart)) {
      const text = parsed.map((part) => part.text).join("\n\n").trim();
      return { text, raw, transformed: text !== raw };
    }
    if (typeof parsed === "string") {
      return { text: parsed, raw, transformed: parsed !== raw };
    }
    if (typeof parsed === "object" && parsed !== null) {
      const text = JSON.stringify(parsed, null, 2);
      return { text, raw, transformed: text !== raw };
    }
  } catch {
    // Not JSON. Fall through to plain unescaping.
  }
  const text = unescapePayload(value);
  return { text, raw, transformed: text !== raw };
}

export function projectMarkdownDisplayPath(projectRoot: string, relativePath: string): string {
  if (!projectRoot) return relativePath;
  const separator = projectRoot.includes("\\") && !projectRoot.includes("/") ? "\\" : "/";
  const root = projectRoot.replace(/[\\/]+$/, "");
  const path = relativePath.replace(/\//g, separator);
  return root ? `${root}${separator}${path}` : `${separator}${path}`;
}

/** Resolve a transcript destination from its message's cwd. Only parent
 * steps that stay inside the project are allowed. Encode once for the shared
 * Markdown URL sanitizer, which returns the decoded project-relative path. */
export function sessionMarkdownUrl(url: string, workingDirectory: string | null): string {
  if (url.startsWith("#") || isSafeExternalMarkdownUrl(url)) return safeMarkdownUrl(url);
  let path: string;
  try {
    path = decodeURIComponent(url);
  } catch {
    return "";
  }
  return resolveProjectPath(path, workingDirectory)?.split("/").map(encodeURIComponent).join("/") ?? "";
}

/**
 * One project picture's, track's or clip's settled outcome in the page-level
 * cache: a ready `data:` URI (a picture) or native media URL (a track or clip),
 * or a terminal failure in words for the chip (rejected format, size cap,
 * missing file). An absent entry means the load has not settled yet.
 */
export type ProjectMarkdownMediaState =
  | { status: "loaded"; src: string }
  | { status: "failed"; message: string };

const MARKDOWN_AUDIO_EXTENSIONS = new Set(["wav", "mp3", "m4a", "aac", "ogg", "oga", "opus", "flac"]);
const MARKDOWN_VIDEO_EXTENSIONS = new Set(["mp4", "m4v", "mov", "webm", "mkv", "ogv"]);

export type MarkdownMediaKind = "image" | "audio" | "video";

function markdownExtension(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** A project path a session plays in place: a track or a clip, by extension. */
export function markdownPlayerKind(path: string): "audio" | "video" | null {
  const extension = markdownExtension(path);
  return MARKDOWN_AUDIO_EXTENSIONS.has(extension) ? "audio" : MARKDOWN_VIDEO_EXTENSIONS.has(extension) ? "video" : null;
}

/** Why a picture does not show or a track or a clip does not play, in the file's own words. */
export function markdownMediaFailureMessage(kind: MarkdownMediaKind, path: string, failure: SessionDetailFailure): string {
  const format = markdownExtension(path).toUpperCase();
  switch (failure.code) {
    case "not_found":
      return kind === "image"
        ? t("session:media.notFoundImage")
        : kind === "audio" ? t("session:media.notFoundAudio") : t("session:media.notFoundVideo");
    case "response_too_large":
      return kind === "image"
        ? t("session:media.tooLargeToShow")
        : kind === "video" ? t("session:media.videoTooLarge") : failure.message;
    case "invalid_request":
      if (kind === "image") return format ? t("session:media.cantShow", { format }) : t("session:media.cantShowFile");
      return format ? t("session:media.cantPlay", { format }) : t("session:media.cantPlayFile");
    default: return failure.message;
  }
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
  return { code: null, message: t("session:loadFailed") };
}

import type {
  ProjectSessionTimelinePage,
  SessionCount,
  SessionEventCounts,
  SessionTimelineEvent,
  SessionTimelineEventKind,
  SessionToolCallPreview,
  SessionToolCategory,
} from "../../../types";
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
  };

export interface OpenSessionDetailRequest {
  resourceId: string;
  occurrenceKey: string;
}

export interface SessionDetailFilters {
  prompts: boolean;
  responses: boolean;
  intermediateSteps: boolean;
  checkpoints: boolean;
  toolCalls: boolean;
}

export type SessionFilterKey = keyof SessionDetailFilters;

/**
 * The timeline query still takes a category list, so the page names every
 * category on every request. Sorting by category was removed with the
 * inspector: three of the four buckets read zero on a typical session and the
 * only populated one is `other`, the fallback — four rows of chrome that
 * partitioned nothing.
 */
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
export const SESSION_EVENT_KIND_LABELS: Record<SessionTimelineEventKind, string> = {
  prompt: "You",
  response: "Response",
  intermediate_step: "Step",
  checkpoint: "Checkpoint",
  tool_call: "Tool call",
};

const SESSION_FILTER_LABELS: Record<SessionFilterKey, string> = {
  prompts: "Prompts",
  responses: "Replies",
  intermediateSteps: "Steps",
  checkpoints: "Checkpoints",
  toolCalls: "Tool calls",
};

function sessionCountSupported(count: SessionCount): boolean {
  return count.quality !== "unsupported";
}

/** A supported count that is provably zero. Unknown (`null`) is not empty. */
function sessionCountEmpty(count: SessionCount): boolean {
  return sessionCountSupported(count) && count.value === 0;
}

/**
 * Tool calls default on: the punchcard rows are the transcript's account of the
 * work, and the per-turn disclosure that used to reveal them lazily is gone.
 * SHOW › Tool calls remains the way to hide them.
 */
export function createDefaultSessionDetailFilters(
  counts: SessionEventCounts,
): SessionDetailFilters {
  return {
    prompts: sessionCountSupported(counts.prompts),
    responses: sessionCountSupported(counts.responses),
    intermediateSteps: false,
    checkpoints: sessionCountSupported(counts.checkpoints),
    toolCalls: sessionCountSupported(counts.tool_calls),
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
    .map((key) => ({ key, label: SESSION_FILTER_LABELS[key], count: sessionFilterCount(counts, key) }))
    .filter((toggle) => sessionCountSupported(toggle.count) && !sessionCountEmpty(toggle.count));
}

export interface SessionEventCoverage {
  total: number | null;
  approximate: boolean;
}

/**
 * Every event this session holds, across all kinds — the denominator the page
 * never used to state. `approximate` is set when any contributing count is a
 * lower bound, so the readout can say `31+` rather than claim precision the
 * transcript cannot support.
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
  return kinds;
}

export function sessionDetailFilterFingerprint(filters: SessionDetailFilters): string {
  return JSON.stringify(includedSessionEventKinds(filters));
}

export function formatSessionCount(count: SessionCount): string {
  if (count.quality === "unsupported" || count.value == null) return "Unavailable";
  const value = new Intl.NumberFormat().format(count.value);
  return count.quality === "lower_bound" ? `${value}+` : value;
}

/**
 * The one compaction rule for counts of tokens and turns, everywhere they
 * appear: exact below 10K, whole K to 1M, hundredths of M above. The activity
 * row a session is launched from and the page it opens must agree on the
 * number — two thresholds made 9,432 read "9.4K" in one and "9,432" in the
 * other.
 */
export function formatCompactNumber(value: number): string {
  return value >= 1_000_000
    ? `${(value / 1_000_000).toFixed(2)}M`
    : value >= 10_000
      ? `${Math.round(value / 1_000)}K`
      : new Intl.NumberFormat().format(value);
}

/**
 * Header figures are read at a glance and sit in a row that must not reflow, so
 * they are abbreviated; the exact value stays in the element's title.
 */
export function formatCompactCount(count: SessionCount): string {
  if (count.quality === "unsupported" || count.value == null) return "Unavailable";
  const compact = formatCompactNumber(count.value);
  return count.quality === "lower_bound" ? `${compact}+` : compact;
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

/** Providers record either seconds or milliseconds; both reach this page. */
const toMilliseconds = toEpochMilliseconds;

export function formatSessionDetailTimestamp(value?: number | null): string {
  if (value == null) return "Time unavailable";
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
 * the column already makes by sorting. The cycle itself now comes from the
 * global clock preference, shared with every other timestamp in the app.
 */
export function formatSessionDetailClock(value?: number | null): string {
  if (value == null) return "--:--";
  return new Date(toMilliseconds(value)).toLocaleTimeString(undefined, clockOptions());
}

function sessionDetailDayKey(value?: number | null): string | null {
  if (value == null) return null;
  const date = new Date(toMilliseconds(value));
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

function formatSessionDetailDay(value?: number | null): string {
  if (value == null) return "Undated";
  return new Date(toMilliseconds(value)).toLocaleDateString(undefined, {
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
 * stamp whose shape depends on the calendar, where `Aug 18` silently means a
 * different thing in January. The range this replaced went the other way and
 * stripped the year unconditionally, so a 2024 session and a 2026 session came
 * out as the very same string.
 *
 * Nothing is abbreviated here, so this chip carries no tooltip: there is no
 * fuller value to put in one.
 */
export function formatSessionStart(value?: number | null): string | null {
  if (value == null) return null;
  return `${formatSessionDetailDay(value)} · ${formatSessionDetailClock(value)}`;
}

/**
 * The length chip hovered. The end time is off the chip faces now, so this is
 * where it lives — dropped from the strip, not dropped from the page. An
 * active session has no end to name, only a distance travelled so far.
 */
export function formatSessionLengthDetail(
  started?: number | null,
  ended?: number | null,
  isActive = false,
): string | null {
  const length = formatSessionDuration(started, ended);
  if (isActive) return length ? `Still running · ${length} so far` : "Still running";
  if (length == null || ended == null) return null;
  const sameDay = sessionDetailDayKey(started) === sessionDetailDayKey(ended);
  const end = sameDay
    ? formatSessionDetailClock(ended)
    : `${formatSessionDetailDay(ended)} · ${formatSessionDetailClock(ended)}`;
  return `Ran ${length}, ending ${end}`;
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
  if (minutes < 60) return `+${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `+${hours}h`;
  return `+${Math.round(hours / 24)}d`;
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
 *  be single-quoted or template literals, so this reads all three quote forms.
 *  (Backticks are why some rows used to show nothing but `exec_command({`.) */
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

const EXTERNAL_MARKDOWN_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

const MARKDOWN_URL_SCHEME = /^[a-z][a-z\d+.-]*:/i;

/**
 * Session text crosses the backend boundary with the registered project root
 * rewritten to `.`. Treat only clean relative descendants as project paths:
 * absolute paths, URL schemes, and parent traversal stay plain text.
 */
export function safeProjectMarkdownPath(url: string): string | null {
  if (!url || url.startsWith("#") || url.startsWith("/") || url.startsWith("\\")
      || MARKDOWN_URL_SCHEME.test(url)) {
    return null;
  }
  let decoded: string;
  try {
    decoded = decodeURIComponent(url).replace(/\\/g, "/");
  } catch {
    return null;
  }
  if (decoded.startsWith("/") || MARKDOWN_URL_SCHEME.test(decoded)) return null;
  const parts: string[] = [];
  for (const part of decoded.split("/")) {
    if (!part || part === ".") continue;
    if (part === ".." || part.includes("\0")) return null;
    parts.push(part);
  }
  return parts.length > 0 ? parts.join("/") : null;
}

export function projectMarkdownDisplayPath(projectRoot: string, relativePath: string): string {
  if (!projectRoot) return relativePath;
  const separator = projectRoot.includes("\\") && !projectRoot.includes("/") ? "\\" : "/";
  const root = projectRoot.replace(/[\\/]+$/, "");
  const path = relativePath.replace(/\//g, separator);
  return root ? `${root}${separator}${path}` : `${separator}${path}`;
}

export function safeMarkdownUrl(url: string): string {
  if (url.startsWith("#")) return url;
  try {
    const parsed = new URL(url);
    if (EXTERNAL_MARKDOWN_PROTOCOLS.has(parsed.protocol)) return parsed.toString();
  } catch {
    // Relative Markdown destinations are handled as project paths below.
  }
  return safeProjectMarkdownPath(url) ?? "";
}

export function isSafeExternalMarkdownUrl(url: string): boolean {
  if (url.startsWith("#")) return false;
  try {
    return EXTERNAL_MARKDOWN_PROTOCOLS.has(new URL(url).protocol);
  } catch {
    return false;
  }
}

/**
 * One project image's settled embed outcome in the page-level cache: a
 * ready `data:` URI, or a terminal failure (rejected format, size cap,
 * missing file). An absent entry means the load has not settled yet.
 */
export type ProjectMarkdownImageState =
  | { status: "loaded"; src: string }
  | { status: "failed"; message: string };

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

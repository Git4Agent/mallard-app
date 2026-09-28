import { useEffect, useRef, useState } from "react";
import type { LocalProjectSummary, ProjectFolderProbe } from "../../types";
import { expandHomePath, isAbsoluteProfilePath } from "../../profilePath";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { projectLabel } from "./model";

/** What the Folder field holds, before the machine is asked anything. */
export type FolderInput =
  | { kind: "empty" }
  | { kind: "search"; query: string }
  | { kind: "path"; path: string };

/**
 * Read the field the way people fill it: a terminal copy arrives quoted, a
 * file-manager copy as a `file://` URL, and `~` means the home folder.
 * Whatever is still not an absolute path is a search of the suggestions.
 */
export function readFolderInput(raw: string, home: string | null): FolderInput {
  let text = raw.trim();
  const quoted = /^(['"])(.*)\1$/.exec(text);
  if (quoted) text = quoted[2].trim();
  if (!text) return { kind: "empty" };
  if (/^file:\/\//i.test(text)) {
    try {
      const pathname = decodeURIComponent(new URL(text).pathname);
      // file:///C:/work arrives as /C:/work.
      text = /^\/[A-Za-z]:[\\/]/.test(pathname) ? pathname.slice(1) : pathname;
    } catch {
      // Not a URL after all: judge it as typed.
    }
  }
  const expanded = expandHomePath(text, home);
  if (!isAbsoluteProfilePath(expanded)) return { kind: "search", query: text };
  return { kind: "path", path: withoutTrailingSeparator(expanded) };
}

/** A path under the home folder as `~/…`, the spelling the field accepts
 *  back; anything else as it is. */
export function compactFolderPath(path: string, home: string | null): string {
  if (!home) return path;
  if (path === home) return "~";
  const rest = path.startsWith(`${home}/`) || path.startsWith(`${home}\\`)
    ? path.slice(home.length + 1)
    : null;
  return rest === null ? path : `~/${rest}`;
}

/** `/work/app/` is `/work/app`; a root keeps its separator. */
function withoutTrailingSeparator(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  if (!trimmed) return path.slice(0, 1);
  if (/^[A-Za-z]:$/.test(trimmed)) return `${trimmed}${path.charAt(2)}`;
  return trimmed;
}

export type FolderProbeState =
  | { status: "idle" }
  | { status: "checking"; path: string }
  | { status: "done"; path: string; probe: ProjectFolderProbe }
  | { status: "error"; path: string; message: string };

/* Shorter than an autosave: the field is judged while it is typed, and the
   probe only reads. */
const FOLDER_PROBE_DELAY_MS = 320;

/** The typed folder's verdict, asked a beat after typing stops. */
export function useProjectFolderProbe(
  path: string | null,
  isActive: () => boolean = () => true,
): FolderProbeState {
  const [state, setState] = useState<FolderProbeState>({ status: "idle" });
  const requestRef = useRef(0);
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  useEffect(() => {
    const requestId = ++requestRef.current;
    if (!path) {
      setState({ status: "idle" });
      return;
    }
    setState({ status: "checking", path });
    const timer = window.setTimeout(() => {
      void projectSyncApi.probeProjectFolder(path).then(
        (probe) => {
          if (requestRef.current === requestId && isActiveRef.current()) {
            setState({ status: "done", path, probe });
          }
        },
        (reason) => {
          if (requestRef.current === requestId && isActiveRef.current()) {
            setState({ status: "error", path, message: errorMessage(reason) });
          }
        },
      );
    }, FOLDER_PROBE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [path]);
  return state;
}

/** The probe for exactly what the field holds now, or nothing yet. */
export function settledProbe(input: FolderInput, state: FolderProbeState): ProjectFolderProbe | null {
  return input.kind === "path" && state.status === "done" && state.path === input.path
    ? state.probe
    : null;
}

/** What a way on does, where it is more than new text for the field: set
 *  this folder up with another agent profile, connect it as it is, or open
 *  the project already there. The field's owner carries it out. */
export type FolderRun = "another-profile" | "use-as-is" | "open-project";

/** A way on: new text for the field, or something its owner runs. */
export type FolderAction = { label: string; value: string } | { label: string; run: FolderRun };

export interface FolderVerdict {
  tone: "mute" | "ok" | "warn" | "bad";
  message: string;
  /** Printed in the mono face after the message. */
  path?: string;
  /** Quiet ways on, as link-styled buttons after the message. */
  actions?: FolderAction[];
}

/**
 * The line under the Folder field for every state it has. `draftId` is the
 * draft the field belongs to, once there is one: its own folder is simply a
 * folder, and another draft's is not somewhere it can move.
 */
export function folderVerdict(
  input: FolderInput,
  state: FolderProbeState,
  context: {
    compact: (path: string) => string;
    projects: LocalProjectSummary[];
    draftId?: string | null;
  },
): FolderVerdict | null {
  // An empty field needs no sentence: the placeholder and Browse say it.
  if (input.kind === "empty") return null;
  if (input.kind === "search") {
    return context.draftId === undefined
      ? null
      : { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter." };
  }
  if (state.status === "error" && state.path === input.path) {
    return { tone: "bad", message: state.message };
  }
  const probe = settledProbe(input, state);
  if (!probe) return { tone: "mute", message: "Checking", path: context.compact(input.path) };
  switch (probe.kind) {
    case "invalid":
      return { tone: "mute", message: "Write the whole path, without . or .. parts." };
    case "missing":
      return { tone: "bad", message: "No folder at this path yet." };
    case "unreadable":
      return { tone: "warn", message: "This folder can’t be read. Check its permissions." };
    case "not_a_directory":
      return probe.parent
        ? {
          tone: "bad",
          message: "This is a file.",
          actions: [{ label: `Use ${context.compact(probe.parent)}`, value: context.compact(probe.parent) }],
        }
        : { tone: "bad", message: "This is a file." };
    case "mallard_data":
      return probe.contains
        ? { tone: "bad", message: "This folder holds Mallard’s own data. Choose a folder inside it." }
        : { tone: "bad", message: "This is Mallard’s own data folder." };
    case "folder": {
      const otherDraft = probe.existing_draft_id && probe.existing_draft_id !== context.draftId;
      if (otherDraft) {
        return context.draftId === undefined
          ? { tone: "ok", message: "Setup already started for this folder." }
          : { tone: "warn", message: "Another setup draft already uses this folder." };
      }
      const project = context.projects.find((candidate) => (
        probe.existing_project_ids.includes(candidate.local_project_id)
      ));
      if (project) {
        const name = projectLabel(project);
        return context.draftId === undefined
          ? {
            tone: "ok",
            message: `Already added as “${name}”.`,
            actions: [{ label: "Set it up with another agent profile", run: "another-profile" }],
          }
          : { tone: "ok", message: `“${name}” uses this folder too, with its own agent profiles.` };
      }
      const kind = probe.is_git_repository ? "Git repository" : "Folder · not a Git repository";
      const resolved = context.compact(probe.canonical_path);
      return resolved === context.compact(input.path)
        ? { tone: "ok", message: kind }
        : { tone: "ok", message: `${kind} · resolves to`, path: resolved };
    }
  }
}

/**
 * A folder inside an app's own data or a temporary directory — Claude's
 * scratch workspaces live under Application Support. Sessions ran there,
 * but it is seldom a project, so the list leaves it out until the field
 * searches for it or names its path.
 */
export function isAppScratchFolder(path: string): boolean {
  const normalized = path.replace(/\\/g, "/");
  return /\/Library\/Application Support\//.test(normalized)
    || /\/AppData\//i.test(normalized)
    || /\/\.local\/share\//.test(normalized)
    || /^\/(?:private\/)?(?:tmp|var\/folders)\//.test(normalized);
}

let recentDayFormatter: Intl.DateTimeFormat | null = null;
let recentYearFormatter: Intl.DateTimeFormat | null = null;

/** When a folder was last used, at a glance: `just now`, `12m ago`, `3h ago`,
 *  then the day — with the year only when it is not this one. */
export function formatRecentStamp(seconds: number, now = Date.now()): string {
  const elapsed = Math.max(0, now - seconds * 1_000);
  if (elapsed < 60_000) return "just now";
  if (elapsed < 3_600_000) return `${Math.floor(elapsed / 60_000)}m ago`;
  if (elapsed < 86_400_000) return `${Math.floor(elapsed / 3_600_000)}h ago`;
  const when = new Date(seconds * 1_000);
  if (when.getFullYear() === new Date(now).getFullYear()) {
    recentDayFormatter ??= new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
    return recentDayFormatter.format(when);
  }
  recentYearFormatter ??= new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
  return recentYearFormatter.format(when);
}

/** What committing a probed folder does from the New project step. */
export type FolderChoice =
  | { kind: "setup"; path: string; resumes: boolean }
  | { kind: "open"; projectId: string; name: string };

export function folderChoice(
  probe: ProjectFolderProbe | null,
  path: string,
  projects: LocalProjectSummary[],
): FolderChoice | null {
  if (probe?.kind !== "folder") return null;
  if (probe.existing_draft_id) return { kind: "setup", path, resumes: true };
  const project = projects.find((candidate) => (
    probe.existing_project_ids.includes(candidate.local_project_id)
  ));
  return project
    ? { kind: "open", projectId: project.local_project_id, name: projectLabel(project) }
    : { kind: "setup", path, resumes: false };
}

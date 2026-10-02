/**
 * Resolve a typed `~` before sending a profile path to the backend. The home
 * path supplies the host separator, so this works for both POSIX and Windows
 * paths.
 */
export function expandHomePath(value: string, home: string | null): string {
  const trimmed = value.trim();
  if (!home || !trimmed.startsWith("~")) return trimmed;
  if (trimmed === "~") return home;
  if (trimmed.startsWith("~/")) {
    const separator = home.includes("\\") ? "\\" : "/";
    return `${home.replace(/[\\/]+$/, "")}${separator}${trimmed.slice(2)}`;
  }
  return trimmed;
}

/** A POSIX root, Windows drive letter, or UNC share. */
export function isAbsoluteProfilePath(value: string): boolean {
  return value.startsWith("/")
    || /^[A-Za-z]:[\\/]/.test(value)
    || value.startsWith("\\\\");
}

/** What a probe says about a typed profile path, in the fields both setup
 *  pages read; each API's own probe carries more. */
export interface ProbedProfilePath {
  canonical_path: string;
  readable: boolean;
  /** The project's probe found the provider's default directory one level
   *  down and used it. */
  detected_child?: boolean;
  existing_profile_id?: string | null;
}

export type ProfilePathProbeState<P extends ProbedProfilePath = ProbedProfilePath> =
  | { status: "checking" }
  | { status: "ok"; probe: P }
  | { status: "error"; message: string };

export interface ProfilePathHint {
  tone: "mute" | "ok" | "warn" | "bad";
  message: string;
  /** Rendered in the mono face when present — it is a machine identifier. */
  path?: string;
}

/**
 * Probe refusals are written for people ("cannot be a symlink; choose the
 * resolved directory"), so they are shown as they arrive — minus the internal
 * call prefix. The one exception is the miss that happens on every keystroke
 * of a path being typed, which deserves a sentence rather than an errno.
 */
function readableProbeError(message: string): string {
  if (/no such file or directory|cannot find the path|os error 2/i.test(message)) {
    return "No folder at this path yet.";
  }
  return message.replace(/^inspect (?:provider|catalog) profile '[^']*':\s*/, "");
}

/** What the line under a custom path field says, for every state it has —
 *  on project setup and on Add Catalog resource alike. */
export function profilePathHint(
  state: ProfilePathProbeState | undefined,
  rawPath: string,
  home: string | null,
  descriptor: { display_name: string; default_profile_directory?: string | null },
  profiles: ReadonlyArray<{ profile_id: string; display_name: string }>,
): ProfilePathHint {
  const expanded = expandHomePath(rawPath, home);
  if (!expanded) {
    return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder." };
  }
  if (!isAbsoluteProfilePath(expanded)) {
    return { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter." };
  }
  if (!state || state.status === "checking") {
    return { tone: "mute", message: "Checking", path: expanded };
  }
  if (state.status === "error") {
    return { tone: "bad", message: readableProbeError(state.message) };
  }
  const { probe } = state;
  const registered = probe.existing_profile_id
    ? profiles.find((profile) => profile.profile_id === probe.existing_profile_id) ?? null
    : null;
  if (registered) {
    return { tone: "ok", message: `Already set up as “${registered.display_name}” — setup will reuse it.` };
  }
  if (!probe.readable) {
    return { tone: "warn", message: "This folder can’t be read. Check its permissions.", path: probe.canonical_path };
  }
  if (probe.detected_child && descriptor.default_profile_directory) {
    return {
      tone: "ok",
      message: `Found ${descriptor.default_profile_directory} inside — using`,
      path: probe.canonical_path,
    };
  }
  return { tone: "ok", message: `${descriptor.display_name} profile ·`, path: probe.canonical_path };
}

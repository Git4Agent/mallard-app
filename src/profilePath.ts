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

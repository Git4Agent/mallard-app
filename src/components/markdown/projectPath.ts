/** Resolve a decoded file path from a project-relative message directory.
 * The caller owns URL decoding/encoding and portable-root references. This
 * helper never touches a filesystem and never permits climbing above root.
 * Kept identical in the website by its scripts/check-project-path.mjs. */
export function resolveProjectPath(path: string, workingDirectory: string | null): string | null {
  if (workingDirectory === null) return null;
  path = path.replace(/\\/g, "/");
  const unsafe = (value: string) => value.startsWith("/") || /^[a-z][a-z\d+.-]*:/i.test(value) || /[\u0000-\u001f\u007f]/.test(value);
  if (!path || unsafe(path) || unsafe(workingDirectory) || workingDirectory.includes("\\")) return null;
  const parts = workingDirectory.split("/").filter((part) => part && part !== ".");
  if (parts.includes("..")) return null;
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length === 0) return null;
      parts.pop();
    } else parts.push(part);
  }
  return parts.length ? parts.join("/") : null;
}

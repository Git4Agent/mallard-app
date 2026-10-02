/**
 * The address Mallard Cloud gives a new repository, as the desktop previews
 * it before the request: a project's New repository row and a Catalog item's
 * both name it the same way. The service stays the authority.
 */

const REPOSITORY_NAME_MAX = 100;

/** Preserve valid names exactly; clean up other titles using the service's
 *  rules. */
export function normalizeRepositoryName(requested: string): string {
  const trimmed = requested.trim();
  if (/^[A-Za-z0-9._-]{1,100}$/.test(trimmed) && trimmed !== "." && trimmed !== "..") return trimmed;
  let name = trimmed.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/-{2,}/g, "-").replace(/^-+|-+$/g, "");
  if (name.length > REPOSITORY_NAME_MAX) name = name.slice(0, REPOSITORY_NAME_MAX).replace(/-+$/g, "");
  return /^[A-Za-z0-9._-]{1,100}$/.test(name) && name !== "." && name !== ".." ? name : "repository";
}

/** The address a new repository would get: the normalized name, suffixed
 *  `-2`, `-3`, … while the account already owns the name. This is the
 *  preview on a New row. */
export function predictRepositoryAddress(
  login: string,
  requestedName: string,
  repositories: ReadonlyArray<{ owner_login: string; name: string }>,
): { address: string; name: string; base: string; suffixed: boolean } {
  const base = normalizeRepositoryName(requestedName);
  const taken = new Set(
    repositories
      .filter((repository) => repository.owner_login.toLowerCase() === login.toLowerCase())
      .map((repository) => repository.name.toLowerCase()),
  );
  let name = base;
  for (let n = 2; taken.has(name.toLowerCase()); n++) {
    const suffix = `-${n}`;
    name = `${base.slice(0, REPOSITORY_NAME_MAX - suffix.length).replace(/-+$/g, "")}${suffix}`;
  }
  return { address: `${login}/${name}`, name, base, suffixed: name !== base };
}

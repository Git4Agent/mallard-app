/**
 * The address Mallard Cloud gives a new repository, as the desktop previews
 * it before the request: a project's New repository row and a Catalog item's
 * both name it the same way. The service stays the authority; this mirrors
 * its `validate/names.ts` and the desktop's `managed_cloud/names.rs`: a name
 * is any script (letters, marks and digits plus `._-`), stored in NFC, and
 * compared by `nameKey`.
 */

export const REPOSITORY_NAME_MAX = 100;

/** The `u` flag makes the quantifier count code points. */
const NAME_SCALARS = /^[\p{L}\p{M}\p{N}._-]{1,100}$/u;
const OTHER_RUNS = /[^\p{L}\p{M}\p{N}._-]+/gu;
const LEADING_MARKS = /^\p{M}+/u;

/** Trimmed and NFC-normalized: the form the service stores. */
export function canonicalName(value: string): string {
  return value.trim().normalize("NFC");
}

export function isRepositoryName(value: string): boolean {
  return NAME_SCALARS.test(value) && !LEADING_MARKS.test(value) && value !== "." && value !== "..";
}

/** The key two spellings of one name share: NFC, then Unicode default case mapping. */
export function nameKey(value: string): string {
  return canonicalName(value).toLowerCase();
}

function cut(value: string, max: number): string {
  return [...value].slice(0, max).join("");
}

/** A valid title is kept exactly; otherwise runs of other characters become
 *  one hyphen, hyphens at the ends go, and the result is cut to the limit.
 *  A title with nothing usable derives no name: the service refuses it. */
export function deriveRepositoryName(requested: string): string | null {
  const title = canonicalName(requested);
  if (isRepositoryName(title)) return title;
  const cleaned = title.replace(OTHER_RUNS, "-").replace(/-{2,}/g, "-").replace(/^-+|-+$/g, "").replace(LEADING_MARKS, "");
  const name = cut(cleaned, REPOSITORY_NAME_MAX).replace(/-+$/g, "");
  return isRepositoryName(name) ? name : null;
}

/** The one sentence a New row prints for a title the service would refuse. */
export function nameRefusal(subject: "project" | "item"): string {
  return `Name needs a letter or digit; rename the ${subject} first.`;
}

export type AddressPrediction =
  | { kind: "address"; address: string; name: string; base: string; suffixed: boolean }
  | { kind: "refused" };

/** The address a new repository would get: the derived name, suffixed
 *  `-2`, `-3`, … while the account already owns the key. This is the
 *  preview on a New row; a title with nothing usable is a refusal. */
export function predictRepositoryAddress(
  login: string,
  requestedName: string,
  repositories: ReadonlyArray<{ owner_login: string; name: string }>,
): AddressPrediction {
  const base = deriveRepositoryName(requestedName);
  if (base === null) return { kind: "refused" };
  const taken = new Set(
    repositories
      .filter((repository) => repository.owner_login.toLowerCase() === login.toLowerCase())
      .map((repository) => nameKey(repository.name)),
  );
  let name = base;
  for (let n = 2; taken.has(nameKey(name)); n++) {
    const suffix = `-${n}`;
    name = `${cut(base, REPOSITORY_NAME_MAX - suffix.length).replace(/-+$/g, "")}${suffix}`;
  }
  return { kind: "address", address: `${login}/${name}`, name, base, suffixed: name !== base };
}

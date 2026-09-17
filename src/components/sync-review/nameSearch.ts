/** Literal, case-insensitive words. Every word must occur in the name. */
export function nameMatchesSearch(name: string, query: string): boolean {
  const normalized = name.toLowerCase();
  return query.toLowerCase().trim().split(/\s+/u).every((word) => normalized.includes(word));
}

export interface NameSearchResults {
  query: string;
  resourceIds: ReadonlySet<string>;
}

/** Replace only this search's selection; retain selections outside its scope. */
export function replaceSearchSelection(
  selected: ReadonlySet<string>,
  matchingIds: Iterable<string>,
  recommendedIds: ReadonlySet<string> = new Set(),
): Set<string> {
  const next = new Set(selected);
  for (const id of matchingIds) {
    if (recommendedIds.has(id)) next.add(id);
    else next.delete(id);
  }
  return next;
}

import type { CatalogCoordinate } from "./types";

/**
 * Resource kinds the app does not show for now: Codex pets, both formats
 * (v1 and v2). The backend still registers them; `storageCatalogApi` drops
 * them where they arrive, so no door, row, card or chooser names one, and a
 * pet's link answers as a kind this app can't open. Nothing on this computer
 * or in a storage changes — a pet row added earlier, and its links, are back
 * when the kind leaves this set.
 */
const SWITCHED_OFF_KINDS: ReadonlySet<string> = new Set(["desktop_pet"]);

export function isSwitchedOff(coordinate: Pick<CatalogCoordinate, "resource_kind">): boolean {
  return SWITCHED_OFF_KINDS.has(coordinate.resource_kind);
}

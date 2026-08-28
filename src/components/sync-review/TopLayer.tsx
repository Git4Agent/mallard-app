import { createPortal } from "react-dom";
import type { ReactNode } from "react";

/**
 * Renders its children on `document.body` rather than in place.
 *
 * For panels that must not be bounded by the list they belong to: on body
 * nothing in the page can clip them — not a card's `overflow`, not a stacking
 * context, not a scrollport's edge.
 *
 * Without a document there is nothing to portal into and no clipping to escape,
 * so the children render where they stand. That keeps the markup identical for
 * anything rendering this tree outside a browser.
 */
export default function TopLayer({ children }: { children: ReactNode }) {
  if (typeof document === "undefined") return <>{children}</>;
  return createPortal(children, document.body);
}

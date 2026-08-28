import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";

/* Matches the gap the panel used to keep from its row when it was anchored
   inside the list. */
const POPOVER_GAP = 6;
/* The panel never touches the window edge. */
const POPOVER_EDGE = 8;
/* Indent from the row's left edge, so the panel still reads as belonging to
   the row it was opened from rather than floating free of it. */
const POPOVER_INSET = 12;
const POPOVER_WIDTH = 380;
const POPOVER_MIN_WIDTH = 260;

export interface DebugPopoverPlacement {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
}

/**
 * Positions a `.sync-history-debug-popover` against the window.
 *
 * The panel is rendered in a portal on `document.body`, so nothing in the list
 * can bound it: not the row's card, not the commit rail's stacking context, and
 * not `.sync-main`'s scrollport. Anchored inside the list it was capped at the
 * room the scroller had left, which near the top or bottom of a long list meant
 * a panel too short to read; against the window there is almost always room for
 * all of it.
 *
 * A fixed panel does not travel with its row, so scrolling or resizing dismisses
 * it rather than leaving it stranded beside the wrong session.
 *
 * Layout, not effect: the panel is measured and placed before anything paints,
 * so it never appears at the origin for a frame on its way to the anchor.
 */
export function useDebugPopoverPlacement(
  rootRef: RefObject<HTMLElement | null>,
  panelRef: RefObject<HTMLElement | null>,
  open: boolean,
  onDismiss: () => void,
): DebugPopoverPlacement | null {
  const [placement, setPlacement] = useState<DebugPopoverPlacement | null>(null);
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    const panel = panelRef.current;
    // The row, not the trigger: anchoring to the icon puts the panel wherever
    // that icon happens to land, which is how it used to end up off the card.
    const anchorEl = rootRef.current?.closest<HTMLElement>(".sync-history-thread-card")
      ?? rootRef.current;
    if (!panel || !anchorEl) return;
    const anchor = anchorEl.getBoundingClientRect();

    const width = Math.max(
      POPOVER_MIN_WIDTH,
      Math.min(POPOVER_WIDTH, anchor.width - POPOVER_INSET * 2),
    );
    // Measure at the width it will be drawn at, uncapped, before it paints —
    // the value rows wrap, so height is meaningless at any other width.
    panel.style.width = `${width}px`;
    panel.style.maxHeight = "none";
    // Ceiling of the fractional height, not offsetHeight's rounded integer: a
    // panel measuring 263.4 capped at 263 is a hair short of its own content,
    // and the value list grows a scrollbar it does not need.
    const needed = Math.ceil(panel.getBoundingClientRect().height);

    const roomBelow = window.innerHeight - anchor.bottom - POPOVER_GAP - POPOVER_EDGE;
    const roomAbove = anchor.top - POPOVER_GAP - POPOVER_EDGE;
    // Below unless below cannot hold it and above holds more.
    const dropUp = needed > roomBelow && roomAbove > roomBelow;
    const maxHeight = Math.max(0, Math.min(needed, dropUp ? roomAbove : roomBelow));

    setPlacement({
      top: dropUp ? anchor.top - POPOVER_GAP - maxHeight : anchor.bottom + POPOVER_GAP,
      left: Math.max(
        POPOVER_EDGE,
        Math.min(anchor.left + POPOVER_INSET, window.innerWidth - width - POPOVER_EDGE),
      ),
      width,
      maxHeight,
    });
  }, [rootRef, panelRef, open]);

  useEffect(() => {
    if (!open) return;
    const dismiss = () => dismissRef.current();
    // Capture, so a scroll inside .sync-main counts and not only the window's.
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [open]);

  return placement;
}

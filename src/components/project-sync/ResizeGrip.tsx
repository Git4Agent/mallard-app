/**
 * The one mark the sidebar's drag seam wears: a short vertical pill centred
 * on the seam, the way a split-view handle is drawn. The seam itself paints
 * nothing at rest; the pill appears while the seam is hovered and goes to ink
 * while it is held or keyboard-focused (App.css draws the states).
 */
export default function ResizeGrip() {
  return <span className="sync-resize-grip" aria-hidden="true" />;
}

import type { SVGProps } from "react";

/* The first seven answer "how does this stand against storage?". The last three
   answer "there is no answer yet" — the state a project with no storage, a
   comparison in flight, and a failed comparison are all in. Without them the
   row had nothing to draw and drew nothing, which is what collapsed the icon
   slot and shifted every title on the page. */
export type ThreadStatusIconName =
  | "synced"
  | "cloud-outstanding"
  | "local-outstanding"
  | "divergent"
  | "pending"
  | "conflict"
  | "unavailable"
  | "untracked"
  | "checking"
  | "unknown";

interface ThreadStatusIconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  status: ThreadStatusIconName;
  size?: number;
}

// Ring badge and glyphs share `currentColor`, so each status reads as a single
// outlined mark in its own colour. Geometry is drawn on the 24 px pixel grid.
// Outer radius 11 on the 24px grid: it steps down in 6px runs, which is what
// reads as a circle. A wider radius flattens the cardinals and looks octagonal.
const badgePath = "M9 1h6v1h3v1h1v1h1v1h1v1h1v3h1v6h-1v3h-1v1h-1v1h-1v1h-1v1h-3v1h-6v-1h-3v-1h-1v-1h-1v-1h-1v-1h-1v-3h-1v-6h1v-3h1v-1h1v-1h1v-1h1v-1h3ZM11 2v1h-3v1h-2v1h-1v1h-1v2h-1v3h-1v2h1v3h1v2h1v1h1v1h2v1h3v1h2v-1h3v-1h2v-1h1v-1h1v-2h1v-3h1v-2h-1v-3h-1v-2h-1v-1h-1v-1h-2v-1h-3v-1Z";
// The badge is a ring: an outer boundary followed by an inner one wound the
// other way. Everything before that second subpath is the outer boundary on its
// own — a solid disc, which is what the filled variant paints. Sliced rather
// than written out a second time so the two cannot drift apart.
const discPath = badgePath.slice(0, badgePath.indexOf("ZM") + 1);

/* The two states a button cannot resolve. They paint as a solid disc with the
   glyph knocked out, so they are found without being looked for — and found in
   greyscale, which is what keeps them legible to the readers hue does not reach
   (WCAG 1.4.1: colour must not be the only carrier). Fill as the severity
   channel is Material's filled/outlined axis and SF Symbols' `.fill` variants,
   not a private invention.

   The knockout uses fill-rule="evenodd" over one path holding the disc and the
   glyph together, which needs no mask and so no generated id. It carries one
   constraint: within a filled glyph no two subpaths may overlap, or the shared
   area is crossed an even number of times and fills back in. Both glyphs below
   are drawn as disjoint rectangles for that reason. */
const filledStatuses: ReadonlySet<ThreadStatusIconName> = new Set(["conflict", "divergent"]);

const glyphPaths: Record<ThreadStatusIconName, readonly string[]> = {
  synced: ["M16 9h2v2h-1v1h-1v2h-1v1h-1v1h-1v1h-2v-1h-1v-1h-1v-1h-1v-2h2v1h1v1h2v-1h1v-2h1v-1h1Z"],
  // 1px outline to match the reference art. It needs room to read — below
  // about 16px the rim starts dropping pixels and breaks up.
  "cloud-outstanding": [
    "M9 7h5v1h-5ZM9 8v2h-1v-2ZM14 8h1v2h-1ZM8 10v1h-2v-1ZM15 10h2v1h-2ZM6 11v3h-1v-3ZM17 11h1v5h-12v-2h1v1h10Z",
  ],
  "local-outstanding": ["M11 8h3v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v7h-2v-6h-1v1h-2v-2h1v-1h1v-1h1Z"],
  // One trunk becoming two matches the standard version-control branch mark.
  // Nothing is thinner than three units, and no two rectangles touch, so the
  // filled 18px glyph remains legible.
  divergent: [
    "M6 7h3v1h-3ZM7 8h3v1h-3ZM8 9h3v1h-3ZM9 10h3v1h-3ZM15 7h3v1h-3ZM14 8h3v1h-3ZM13 9h3v1h-3ZM12 10h3v1h-3ZM10 11h4v1h-4ZM10 12h4v6h-4Z",
  ],
  pending: ["M11 7h2v5h4v2h-6Z"],
  conflict: ["M11 6h3v9h-3ZM11 16h3v3h-3Z"],
  unavailable: ["M7 11h10v3h-10Z"],
  // An empty box: no verdict has been recorded, as against `unavailable`'s
  // minus bar, which is the verdict "this one cannot be synced".
  untracked: ["M8 8h8v8H8ZM10 14h4v-4h-4Z"],
  // Three dots, faded in turn by CSS. Separate paths so they can blink
  // independently; a single pulsing glyph read as an error, not as work.
  checking: ["M6 11h3v3H6Z", "M11 11h3v3h-3Z", "M16 11h3v3h-3Z"],
  unknown: ["M9 6h6v2H9ZM13 8h2v3h-2ZM11 11h4v2h-4ZM11 13h2v2h-2ZM11 16h2v2h-2Z"],
};

export default function ThreadStatusIcon({
  status,
  size = 24,
  className,
  ...props
}: ThreadStatusIconProps) {
  const classes = ["thread-status-icon", className].filter(Boolean).join(" ");
  const paths = glyphPaths[status];
  return (
    <svg
      aria-hidden="true"
      className={classes}
      data-thread-status={status}
      fill="none"
      focusable="false"
      height={size}
      shapeRendering={size % 24 === 0 ? "crispEdges" : "geometricPrecision"}
      viewBox="0 0 24 24"
      width={size}
      {...props}
    >
      {filledStatuses.has(status) ? (
        <path
          className="thread-status-icon-badge"
          d={`${discPath}${paths.join("")}`}
          fill="currentColor"
          fillRule="evenodd"
        />
      ) : (
        <>
          <path className="thread-status-icon-badge" fill="currentColor" d={badgePath} />
          <g className="thread-status-icon-glyph">
            {status === "checking" ? (
              <g className="thread-status-icon-dots">
                {paths.map((d) => (
                  <path key={d} fill="currentColor" d={d} />
                ))}
              </g>
            ) : (
              paths.map((d) => <path key={d} fill="currentColor" d={d} />)
            )}
          </g>
        </>
      )}
    </svg>
  );
}

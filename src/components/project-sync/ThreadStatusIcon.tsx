import type { SVGProps } from "react";

export type ThreadStatusIconName =
  | "synced"
  | "cloud-outstanding"
  | "local-outstanding"
  | "divergent"
  | "divergent-forked"
  | "pending"
  | "conflict"
  | "unavailable";

interface ThreadStatusIconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  status: ThreadStatusIconName;
  size?: number;
}

// Ring badge and glyphs share `currentColor`, so each status reads as a single
// outlined mark in its own colour. Geometry is drawn on the 24 px pixel grid.
// Outer radius 11 on the 24px grid: it steps down in 6px runs, which is what
// reads as a circle. A wider radius flattens the cardinals and looks octagonal.
const badgePath = "M9 1h6v1h3v1h1v1h1v1h1v1h1v3h1v6h-1v3h-1v1h-1v1h-1v1h-1v1h-3v1h-6v-1h-3v-1h-1v-1h-1v-1h-1v-1h-1v-3h-1v-6h1v-3h1v-1h1v-1h1v-1h1v-1h3ZM11 2v1h-3v1h-2v1h-1v1h-1v2h-1v3h-1v2h1v3h1v2h1v1h1v1h2v1h3v1h2v-1h3v-1h2v-1h1v-1h1v-2h1v-3h1v-2h-1v-3h-1v-2h-1v-1h-1v-1h-2v-1h-3v-1Z";
// Synced and forked share the check; the status colour is what tells them apart.
const checkPath = "M16 9h2v2h-1v1h-1v2h-1v1h-1v1h-1v1h-2v-1h-1v-1h-1v-1h-1v-2h2v1h1v1h2v-1h1v-2h1v-1h1Z";

function StatusGlyph({ status }: { status: ThreadStatusIconName }) {
  switch (status) {
    case "synced":
      return <path fill="currentColor" d={checkPath} />;
    // 1px outline to match the reference art. It needs room to read — below
    // about 16px the rim starts dropping pixels and breaks up.
    case "cloud-outstanding":
      return (
        <path
          fill="currentColor"
          d="M9 7h5v1h-5ZM9 8v2h-1v-2ZM14 8h1v2h-1ZM8 10v1h-2v-1ZM15 10h2v1h-2ZM6 11v3h-1v-3ZM17 11h1v5h-12v-2h1v1h10Z"
        />
      );
    case "local-outstanding":
      return (
        <path
          fill="currentColor"
          d="M11 8h3v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v7h-2v-6h-1v1h-2v-2h1v-1h1v-1h1Z"
        />
      );
    case "divergent":
      return (
        <path
          fill="currentColor"
          d="M6 6h4v5h1v1h1v2h1v-1h1v-2h1v-1h-1v-4h4v4h-1v2h-1v2h-1v1h-1v1h-1v2h-2v-3h-1v-2h-1v-1h-1v-2h-2ZM7 7v2h2v-2ZM15 7v2h2v-2Z"
        />
      );
    case "divergent-forked":
      return <path fill="currentColor" d={checkPath} />;
    case "pending":
      return <path fill="currentColor" d="M11 7h2v5h4v2h-6Z" />;
    case "conflict":
      return <path fill="currentColor" d="M11 6h3v9h-3ZM11 16h3v3h-3Z" />;
    case "unavailable":
      return <path fill="currentColor" d="M7 11h10v3h-10Z" />;
  }
}

export default function ThreadStatusIcon({
  status,
  size = 24,
  className,
  ...props
}: ThreadStatusIconProps) {
  const classes = ["thread-status-icon", className].filter(Boolean).join(" ");
  return (
    <svg
      aria-hidden="true"
      className={classes}
      data-thread-status={status}
      fill="none"
      focusable="false"
      height={size}
      shapeRendering="crispEdges"
      viewBox="0 0 24 24"
      width={size}
      {...props}
    >
      <path className="thread-status-icon-badge" fill="currentColor" d={badgePath} />
      <g className="thread-status-icon-glyph">
        <StatusGlyph status={status} />
      </g>
    </svg>
  );
}

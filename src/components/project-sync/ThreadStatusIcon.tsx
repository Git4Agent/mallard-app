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

const badgePath = "M8 2H16V3H18V4H19V5H20V6H21V8H22V16H21V18H20V19H19V20H18V21H16V22H8V21H6V20H5V19H4V18H3V16H2V8H3V6H4V5H5V4H6V3H8Z";
const glyphColor = "var(--thread-status-glyph, #fffaf1)";

function StatusGlyph({ status }: { status: ThreadStatusIconName }) {
  switch (status) {
    case "synced":
      return (
        <path
          fill={glyphColor}
          d="M18 7H19V9H18V10H17V11H16V12H15V14H14V15H13V16H12V17H11V18H10V17H9V16H8V15H7V14H6V12H8V13H9V14H10V15H11V16H12V15H13V14H14V13H15V11H16V10H17V8H18Z"
        />
      );
    case "cloud-outstanding":
      return (
        <>
          <path fill={glyphColor} d="M11 6H14V13H18L12 18L6 13H11Z" />
          <path fill={glyphColor} d="M7 19H18V21H7Z" />
        </>
      );
    case "local-outstanding":
      return (
        <>
          <path fill={glyphColor} d="M11 18H14V11H18L12 6L6 11H11Z" />
          <path fill={glyphColor} d="M7 19H18V21H7Z" />
        </>
      );
    case "divergent":
      return (
        <>
          <path fill={glyphColor} d="M6 12L10 8V11H15V13H10V16Z" />
          <path fill={glyphColor} d="M19 12L15 8V11H10V13H15V16Z" />
        </>
      );
    case "divergent-forked":
      return (
        <path fill={glyphColor} d="M6 6H9V9H10V10H11V12H14V10H15V9H16V6H19V9H18V10H17V12H15V14H14V20H11V14H10V12H8V10H7V9H6Z" />
      );
    case "pending":
      return <path fill={glyphColor} d="M11 6H14V12H18V15H11Z" />;
    case "conflict":
      return <path fill={glyphColor} d="M10 5H14V14H10ZM10 17H14V21H10Z" />;
    case "unavailable":
      return (
        <path
          fill="none"
          stroke={glyphColor}
          strokeLinecap="square"
          strokeWidth="2"
          d="M7 18L18 7"
        />
      );
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

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

const badgePath = "M6 3H18V4H20V6H21V8H22V16H21V18H20V20H18V21H6V20H4V18H3V16H2V8H3V6H4V4H6Z";
const glyphColor = "var(--thread-status-glyph, #fffaf1)";

function StatusGlyph({ status }: { status: ThreadStatusIconName }) {
  switch (status) {
    case "synced":
      return (
        <path
          fill={glyphColor}
          d="M5 12H8V15H10V13H12V11H14V9H16V7H19V10H17V12H15V14H13V16H11V18H8V16H7V15H5Z"
        />
      );
    case "cloud-outstanding":
      return (
        <>
          <path
            fill="none"
            stroke={glyphColor}
            strokeWidth="2"
            strokeLinejoin="miter"
            d="M6 12V10H8V8H10V7H14V8H16V10H18V12H19V14H5V12Z"
          />
          <path fill={glyphColor} d="M11 14H13V17H15L12 20L9 17H11Z" />
        </>
      );
    case "local-outstanding":
      return (
        <>
          <path fill={glyphColor} d="M11 17H13V9H16L12 5L8 9H11Z" />
          <path fill={glyphColor} d="M7 17H9V19H15V17H17V20H7Z" />
        </>
      );
    case "divergent":
      return (
        <>
          <path fill={glyphColor} d="M5 7H14V5L19 8L14 11V9H5Z" />
          <path fill={glyphColor} d="M19 15H10V13L5 16L10 19V17H19Z" />
        </>
      );
    case "divergent-forked":
      return (
        <>
          <path fill={glyphColor} d="M5 4H9V8H8V10H16V8H15V4H19V8H18V11H17V12H8V17H9V21H5V17H6V8H5Z" />
          <path fill={glyphColor} d="M13 15H15V17H16V16H18V18H17V19H15V18H14V17H13Z" />
        </>
      );
    case "pending":
      return (
        <>
          <path
            fill="none"
            stroke={glyphColor}
            strokeWidth="2"
            strokeLinejoin="miter"
            d="M8 4H16V5H19V8H20V16H19V19H16V20H8V19H5V16H4V8H5V5H8Z"
          />
          <path fill={glyphColor} d="M11 7H13V12H17V14H11Z" />
        </>
      );
    case "conflict":
      return <path fill={glyphColor} d="M10 5H14V14H10ZM10 17H14V21H10Z" />;
    case "unavailable":
      return <path fill={glyphColor} d="M6 11H18V14H6Z" />;
  }
}

export default function ThreadStatusIcon({
  status,
  size = 16,
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

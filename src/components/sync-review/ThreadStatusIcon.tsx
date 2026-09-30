import {
  IconCircle,
  IconCircleCheck,
  IconCircleMinus,
  IconClock,
  IconCloudDownload,
  IconDots,
  IconExclamationCircleFilled,
  IconGitMerge,
  IconHelpCircle,
  IconSquareRounded,
  IconUpload,
  type TablerIcon,
} from "@tabler/icons-react";
import type { HTMLAttributes } from "react";

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

interface ThreadStatusIconProps extends Omit<HTMLAttributes<HTMLSpanElement>, "children"> {
  status: ThreadStatusIconName;
  size?: number;
}

const PRIMARY_ICONS = {
  synced: IconCircleCheck,
  pending: IconClock,
  conflict: IconExclamationCircleFilled,
  unavailable: IconCircleMinus,
  unknown: IconHelpCircle,
} satisfies Partial<Record<ThreadStatusIconName, TablerIcon>>;

type CompositeStatus = Exclude<ThreadStatusIconName, keyof typeof PRIMARY_ICONS>;

const COMPOSITE_GLYPHS: Record<CompositeStatus, TablerIcon> = {
  "cloud-outstanding": IconCloudDownload,
  "local-outstanding": IconUpload,
  divergent: IconGitMerge,
  untracked: IconSquareRounded,
  checking: IconDots,
};

export default function ThreadStatusIcon({
  status,
  size = 24,
  className,
  style,
  ...props
}: ThreadStatusIconProps) {
  const classes = ["thread-status-icon", className].filter(Boolean).join(" ");
  const PrimaryIcon = (PRIMARY_ICONS as Partial<Record<ThreadStatusIconName, TablerIcon>>)[status];
  const CompositeGlyph = PrimaryIcon ? null : COMPOSITE_GLYPHS[status as CompositeStatus];
  const glyphSize = Math.max(8, Math.round(size * 0.56));

  return (
    <span
      {...props}
      aria-hidden="true"
      className={classes}
      data-thread-status={status}
      style={{ width: size, height: size, ...style }}
    >
      {PrimaryIcon ? (
        <PrimaryIcon
          className="thread-status-icon-primary"
          size={size}
          stroke={2}
        />
      ) : (
        <>
          <IconCircle
            className="thread-status-icon-badge"
            size={size}
            stroke={2}
          />
          {CompositeGlyph && (
            <CompositeGlyph
              className={`thread-status-icon-glyph${status === "checking" ? " thread-status-icon-dots" : ""}`}
              size={glyphSize}
              stroke={2}
            />
          )}
        </>
      )}
    </span>
  );
}

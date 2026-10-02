import type { ReactNode } from "react";
import Icon, { type IconName } from "../Icons";

/**
 * The framed group every project list sorts its rows into — a folder on the
 * Files face and in the Files reviews, the session list on the Sessions face —
 * so the two faces read as one page. A header strip names the group; the rows
 * below are two-line cards that join borders inside the frame. The
 * storage-only group is the cloud-tinted tile.
 */
export default function ListTile({
  icon, label, facts, cloud = false, leading, trailing, children,
}: {
  icon: IconName;
  /** Chrome in the display face; a `<span>` inside it is the user's data
   *  (a path, a storage name) and reads in the normal face. */
  label: ReactNode;
  facts?: string;
  cloud?: boolean;
  /** Controls the strip carries around its label — a review's checkbox and
   *  selection count. */
  leading?: ReactNode;
  trailing?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className={`sync-list-tile${cloud ? " cloud" : ""}`}>
      <div className="sync-list-tile-head">
        {leading}
        <Icon name={icon} size={14} />
        <strong>{label}</strong>
        {facts && <small>{facts}</small>}
        {trailing}
      </div>
      <div className="sync-history-thread-list flat">{children}</div>
    </div>
  );
}

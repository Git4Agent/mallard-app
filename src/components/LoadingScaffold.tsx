import { useEffect, useState } from "react";

/**
 * The loading recipe's T1 tier draws a page's first layout in bones, so
 * arrival changes pixels,
 * never geometry. Two shapes cover the first-load pages — `rows` (the
 * thread-card list family Activity and the Library both land) and
 * `detail` (session detail).
 *
 * The container is a quiet named region: first paint announces nothing
 * (the refresh paths that need politeness carry their own visually hidden
 * lines), and every bone is aria-hidden. The caption is the one line a
 * sighted reader gets, and it names the thing being loaded — never a bare
 * "Loading…".
 *
 * Nothing renders for the first 150ms: the observation and timeline disk
 * caches make warm loads near-instant, and the gate keeps them flash-free
 * while cold loads still get bones well before they read as a stall.
 *
 * `sessions` is the one ungated shape: the Push and Pull reviews draw it,
 * and they are already behind the 150ms gate their host applies before the
 * review's card opens. A second gate would open that card empty, and the
 * swap from the preparing card to the mounted review would restart it.
 */

export function LoadingBone({
  width,
  height = 12,
  className,
}: {
  width: number | string;
  height?: number;
  className?: string;
}) {
  return (
    <i
      className={`loading-bone${className ? ` ${className}` : ""}`}
      aria-hidden="true"
      style={{ display: "block", width, height }}
    />
  );
}

/* The thread-card anatomy both landed lists share (Activity sessions and
   the Library's catalog rows are the same `sync-history-thread-card`
   family): hairline-separated flat rows, a topline with the badge and name
   on the left and the action run on the right, and a wrapped meta-chip
   line below. */
function RowBones() {
  return (
    <div className="loading-scaffold-list">
      {[62, 44, 55].map((width) => (
        <div key={width} className="loading-scaffold-row">
          <div className="loading-scaffold-topline">
            <div className="loading-scaffold-title">
              <LoadingBone width={12} height={12} className="round quiet" />
              <LoadingBone width={`${width}%`} height={13} />
            </div>
            <div className="loading-scaffold-actions">
              <LoadingBone width={16} height={16} className="quiet" />
              <LoadingBone width={16} height={16} className="quiet" />
              <LoadingBone width={16} height={16} className="quiet" />
            </div>
          </div>
          <div className="loading-scaffold-chips">
            <LoadingBone width={54} height={9} className="pill quiet" />
            <LoadingBone width={38} height={9} className="pill quiet" />
            <LoadingBone width={46} height={9} className="pill quiet" />
          </div>
        </div>
      ))}
    </div>
  );
}

/* A review's session list: the two-line row at its landed height, `count`
   of them. The first `storageOnly` sit under an "Only in …" heading, the
   group a Pull lands its storage-only sessions in. */
const SESSION_TITLE_WIDTHS = [62, 44, 55, 48, 58];

function SessionRowBones({ count, from }: { count: number; from: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, index) => from + index).map((position) => (
        <div key={position} className="loading-scaffold-row">
          <div className="loading-scaffold-topline">
            <div className="loading-scaffold-title">
              <LoadingBone width={12} height={12} className="round quiet" />
              <LoadingBone width={`${SESSION_TITLE_WIDTHS[position % SESSION_TITLE_WIDTHS.length]}%`} height={13} />
            </div>
            <div className="loading-scaffold-actions">
              <LoadingBone width={16} height={16} className="quiet" />
              <LoadingBone width={16} height={16} className="quiet" />
            </div>
          </div>
          <div className="loading-scaffold-chips">
            <LoadingBone width={54} height={9} className="pill quiet" />
            <LoadingBone width={38} height={9} className="pill quiet" />
            <LoadingBone width={46} height={9} className="pill quiet" />
          </div>
        </div>
      ))}
    </>
  );
}

function SessionBones({ count, storageOnly }: { count: number; storageOnly: number }) {
  const grouped = Math.min(storageOnly, count);
  const loose = count - grouped;
  return (
    <>
      {grouped > 0 && (
        <section className="sync-history-storage-only">
          <div className="sync-history-section-heading">
            <LoadingBone width={150} height={12} className="pill quiet" />
            <LoadingBone width={30} height={12} className="pill quiet" />
          </div>
          <div className="sync-history-thread-list flat loading-scaffold-sessions">
            <SessionRowBones count={grouped} from={0} />
          </div>
        </section>
      )}
      {loose > 0 && (
        <div className="loading-scaffold-list loading-scaffold-sessions">
          <SessionRowBones count={loose} from={grouped} />
        </div>
      )}
    </>
  );
}

function DetailBones() {
  // The punchcard band shimmers as one surface — a fixed tile lattice, not
  // sixty spinners — so it reads as a region the way the landed band does.
  const tiles = Array.from({ length: 54 }, (_, index) => (index * 7) % 10);
  return (
    <>
      <div className="loading-scaffold-chips">
        <LoadingBone width={74} height={16} className="pill" />
        <LoadingBone width={58} height={16} className="pill" />
        <LoadingBone width={90} height={16} className="pill" />
        <LoadingBone width={48} height={16} className="pill" />
      </div>
      <div className="loading-scaffold-band loading-bone">
        {tiles.map((step, index) => (
          <i key={index} style={{ opacity: 0.25 + step * 0.06 }} />
        ))}
      </div>
      <div className="loading-scaffold-panes">
        {[0, 1].map((pane) => (
          <div key={pane} className="loading-scaffold-pane">
            <LoadingBone width={64} height={9} className="pill quiet" />
            <LoadingBone width="92%" height={11} />
            <LoadingBone width="78%" height={11} />
            <LoadingBone width="84%" height={11} />
          </div>
        ))}
      </div>
    </>
  );
}

/** Placeholders past this many rows sit below any window a review opens in;
 *  a longer list still lands, it just is not drawn in bone first. */
const MAX_SESSION_BONES = 12;

export default function LoadingScaffold({
  shape,
  label,
  count = 3,
  storageOnly = 0,
}: {
  shape: "rows" | "detail" | "sessions";
  /** The concrete noun phrase, without the ellipsis: "Loading project activity". */
  label: string;
  /** `sessions` only: how many rows the list will land, once known. */
  count?: number;
  /** `sessions` only: how many of those land under "Only in …". */
  storageOnly?: number;
}) {
  const gated = shape !== "sessions";
  const [pastGate, setPastGate] = useState(!gated);
  useEffect(() => {
    if (!gated) return;
    const timer = window.setTimeout(() => setPastGate(true), 150);
    return () => window.clearTimeout(timer);
  }, [gated]);
  if (!pastGate) return null;
  const rows = Math.max(1, Math.min(count, MAX_SESSION_BONES));
  return (
    <div
      className={`loading-scaffold${shape === "sessions" ? " sessions" : ""}`}
      role="region"
      aria-label={label}
    >
      <div className="loading-scaffold-caption">
        <span className="status-loader" /> {label}…
      </div>
      <div className="loading-scaffold-bones" aria-hidden="true">
        {shape === "rows" && <RowBones />}
        {shape === "detail" && <DetailBones />}
        {shape === "sessions" && <SessionBones count={rows} storageOnly={Math.min(storageOnly, rows)} />}
      </div>
    </div>
  );
}

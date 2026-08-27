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

const SHAPES = {
  rows: RowBones,
  detail: DetailBones,
} as const;

export default function LoadingScaffold({
  shape,
  label,
}: {
  shape: keyof typeof SHAPES;
  /** The concrete noun phrase, without the ellipsis: "Loading project activity". */
  label: string;
}) {
  const [pastGate, setPastGate] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setPastGate(true), 150);
    return () => window.clearTimeout(timer);
  }, []);
  if (!pastGate) return null;
  const Bones = SHAPES[shape];
  return (
    <div
      className="loading-scaffold"
      role="region"
      aria-label={label}
    >
      <div className="loading-scaffold-caption">
        <span className="status-loader" /> {label}…
      </div>
      <div className="loading-scaffold-bones" aria-hidden="true">
        <Bones />
      </div>
    </div>
  );
}

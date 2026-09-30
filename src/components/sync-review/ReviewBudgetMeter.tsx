import { formatBytes } from "../../format";

/** What a review's selection would move against the most one apply may. */
export interface ReviewBudget {
  total: number;
  limit: number;
}

/** The direction a review moves bytes in. It picks the sentence, the meter's
 *  label and the fill ink: the local-outstanding ink for what leaves this
 *  computer, the cloud-outstanding ink for what arrives. */
export type ReviewBudgetVerb = "push" | "pull";

/** What the meter says in one line: the size while it fits, the overshoot and
 *  the way out past the limit. The over sentence is also the apply button's
 *  title in both reviews, so the refusal and its reason are worded once. */
export function reviewBudgetSentence(budget: ReviewBudget, verb: ReviewBudgetVerb): string {
  return budget.total > budget.limit
    ? `${formatBytes(budget.total - budget.limit)} over the ${formatBytes(budget.limit)} ${verb} limit — exclude sessions or files to ${verb}.`
    : `This ${verb} · ${formatBytes(budget.total)} of ${formatBytes(budget.limit)}`;
}

function sizeLabel(verb: ReviewBudgetVerb): string {
  return verb === "push" ? "Push size" : "Pull size";
}

/** `424 MB / 512 MB` while it fits, the overshoot alone once it does not —
 *  `short` drops the trailing "over 512 MB" where the sentence beside it has
 *  already named the limit. Carries the meter semantics for both instances. */
function ReviewBudgetFigure({ budget, verb, short = false, label }: {
  budget: ReviewBudget;
  verb: ReviewBudgetVerb;
  short?: boolean;
  label?: string;
}) {
  const over = budget.total - budget.limit;
  return (
    <span
      className={`sync-review-budget-figure${over > 0 ? " over" : ""}`}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={budget.limit}
      aria-valuenow={Math.min(budget.total, budget.limit)}
      aria-valuetext={reviewBudgetSentence(budget, verb)}
    >
      {over > 0
        ? <><b>+{formatBytes(over)}</b>{short ? "" : ` over ${formatBytes(budget.limit)}`}</>
        : <><b>{formatBytes(budget.total)}</b> / {formatBytes(budget.limit)}</>}
    </span>
  );
}

/** The bar under the header's figure. Scaled to max(total, limit): while the
 *  selection fits, the track's right end IS the limit; past it the limit
 *  becomes a tick the fill visibly runs beyond, so the overshoot reads as a
 *  distance and not only as a number.
 *
 *  One fill, not a segment per source. Sessions, requirements and project
 *  files were drawn in three tones first, and the palette cannot carry them at
 *  6px: --text-3 against --thread-status-local measures 1.05:1 on cream, so
 *  the requirements segment simply disappeared into the sessions one. The
 *  breakdown stays where it has always been — the title on both instances.
 *  Decoration: the figure beside it is the meter. */
function ReviewBudgetTrack({ budget }: { budget: ReviewBudget }) {
  const scale = Math.max(budget.total, budget.limit);
  const over = budget.total > budget.limit;
  return (
    <span className={`sync-review-budget-track${over ? " over" : ""}`} aria-hidden="true">
      <i style={{ width: `${(budget.total / scale) * 100}%` }} />
      {over && <b style={{ left: `${(budget.limit / scale) * 100}%` }} />}
    </span>
  );
}

/** The header's glance: the same measurement as the footer gauge, in the one
 *  other zone of the review that does not scroll away, so what this apply
 *  carries is on screen at the top of a long list as well as at the button.
 *  It is not a live region — the footer's is, and two would read every tick
 *  out twice. `breakdown` is the title on both instances: the sources the
 *  total is made of, worded by the review that knows them. */
export function ReviewBudgetGlance({ budget, verb, breakdown }: {
  budget: ReviewBudget;
  verb: ReviewBudgetVerb;
  breakdown: string;
}) {
  return (
    <div className="sync-review-budget-glance" title={breakdown}>
      {/* Pull only. The Push header also carries the redaction flag and the
          Select pill, and at the default window it has no room for the bar;
          the track is decoration and the footer gauge fills the same way. */}
      {verb === "pull" && <ReviewBudgetTrack budget={budget} />}
      <ReviewBudgetFigure budget={budget} verb={verb} short label={sizeLabel(verb)} />
    </div>
  );
}

/** The footer gauge: a pill the height of the button that fills left to right
 *  behind its own figure, set flush against the apply button so the quantity
 *  and the act that spends it read as one object. Live with the selection,
 *  which is why it sits beside the button and not in the banner zone above
 *  the list. */
export function ReviewBudgetGauge({ budget, verb, breakdown }: {
  budget: ReviewBudget;
  verb: ReviewBudgetVerb;
  breakdown: string;
}) {
  const over = budget.total > budget.limit;
  return (
    <div
      className={`sync-review-budget-gauge ${verb}${over ? " over" : ""}`}
      role="status"
      title={breakdown}
    >
      {/* Past the limit there is nothing left to fill, and painting the danger
          ink as a wash under its own figure took that figure to 3.4:1 on the
          light theme. The plate and the ring carry the state instead; how far
          over it went is the header track's job. */}
      {!over && (
        <span
          className="sync-review-budget-fill"
          style={{ width: `${(budget.total / budget.limit) * 100}%` }}
          aria-hidden="true"
        />
      )}
      <ReviewBudgetFigure budget={budget} verb={verb} short />
    </div>
  );
}

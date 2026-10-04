import { useRef, type ReactNode } from "react";
import Icon from "../Icons";

export default function NameSearchField({
  query, onChange, label, matches, total, noun, narrowed = false, hiddenSelected = 0, disabled = false, leading, trailing, count = true,
}: {
  query: string;
  onChange: (query: string) => void;
  label: string;
  matches: number;
  total: number;
  /** What the count counts, in the count's number: "1 file", "8 files". */
  noun: (count: number) => string;
  /** The list is narrowed by something other than the query — a status
   *  scope — so the count reads "3 of 12" with the field empty. */
  narrowed?: boolean;
  hiddenSelected?: number;
  disabled?: boolean;
  /** A control that leads the field's row — the project's view switch —
   *  beside the field, outside its rule. */
  leading?: ReactNode;
  /** A control that closes the field's row — a rescan, a verb — beside the
   *  field, outside its rule. */
  trailing?: ReactNode;
  /** False keeps the count off the page; what the filter kept still
   *  reaches a screen reader. */
  count?: boolean;
}) {
  const input = useRef<HTMLInputElement>(null);
  const filtering = narrowed || query.trim().length > 0;
  const field = (
    <div className="sync-name-search">
      <div className="sync-name-search-input">
        <Icon name="search" size={14} />
        <input
          ref={input}
          type="search"
          value={query}
          placeholder={`${label}…`}
          aria-label={label}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Escape" || !query) return;
            event.preventDefault();
            event.stopPropagation();
            onChange("");
          }}
        />
        {query && (
          <button type="button" className="sync-name-search-clear" aria-label="Clear search" title="Clear search"
            disabled={disabled} onClick={() => { onChange(""); input.current?.focus(); }}>
            <Icon name="x" size={13} />
          </button>
        )}
      </div>
      {count ? (
        <div className="sync-name-search-results" aria-live="polite" aria-atomic="true">
          {/* The count names what it counts, so its text is its whole
              announcement: "8 files", or "2 of 8 files" while filtering. */}
          <span className="sync-name-search-count">
            {filtering
              ? <><b>{matches}</b><span> of {total} {noun(total)}</span></>
              : <>{total} {noun(total)}</>}
          </span>
          {hiddenSelected > 0 && (
            <span className="sync-name-search-hidden">{hiddenSelected} selected hidden by search</span>
          )}
        </div>
      ) : (
        /* Off the page, not off the air: a filter that hides rows still
           says how many it kept. */
        <span className="sync-visually-hidden" role="status">
          {filtering ? `${matches} of ${total} ${noun(total)}` : ""}
        </span>
      )}
    </div>
  );
  /* The rule is the field's own: a control riding beside it sits on the
     row, not under the line. */
  if (!leading && !trailing) return field;
  return (
    <div className="sync-list-head">
      {leading}
      {field}
      {trailing}
    </div>
  );
}

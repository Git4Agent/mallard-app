import { useRef } from "react";
import Icon from "../Icons";

export default function NameSearchField({
  query, onChange, label, matches, total, hiddenSelected = 0, disabled = false,
}: {
  query: string;
  onChange: (query: string) => void;
  label: string;
  matches: number;
  total: number;
  hiddenSelected?: number;
  disabled?: boolean;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
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
      <div className="sync-name-search-results" aria-live="polite" aria-atomic="true">
        <span className="sync-name-search-count" aria-label={`${matches} of ${total} matches`}>
          {query.trim() ? <><b>{matches}</b><span> / {total}</span></> : <>{total} items</>}
        </span>
        {hiddenSelected > 0 && (
          <span className="sync-name-search-hidden">{hiddenSelected} selected hidden by search</span>
        )}
      </div>
    </div>
  );
}

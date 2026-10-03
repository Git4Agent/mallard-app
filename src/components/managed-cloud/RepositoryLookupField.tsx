import type { ReactNode } from "react";
import Icon from "../Icons";

export function isInvitationCodeShaped(text: string): boolean {
  const trimmed = text.trim();
  return !!trimmed && !/[/.]/.test(trimmed) && trimmed.replace(/[\s-]+/g, "").length === 20;
}

/** `owner/name`, `host/owner/name`, or a page link: something a lookup
 *  can answer. Anything else the field holds is a filter. */
export function isAddressShaped(text: string): boolean {
  const trimmed = text.trim().replace(/^https?:\/\//i, "");
  return /^([a-z0-9.-]+\/)?[a-z0-9._-]+\/[a-z0-9._-]+\/?$/i.test(trimmed) || /^[a-z0-9.-]+\.[a-z]{2,}\//i.test(trimmed);
}

/** What the one field holds: nothing, words that filter the rows, an
 *  address or link to look up, or an invitation code to join with. */
export type EntryKind = "idle" | "filter" | "address" | "code";

export function entryKind(text: string): EntryKind {
  if (!text.trim()) return "idle";
  if (isInvitationCodeShaped(text)) return "code";
  if (isAddressShaped(text)) return "address";
  return "filter";
}

export default function RepositoryLookupField({ value, onChange, onSubmit, disabled, looking, joining, invalid, describedBy, error, labelHidden }: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled: boolean;
  looking: boolean;
  joining: boolean;
  invalid?: boolean;
  describedBy?: string;
  error?: ReactNode;
  /** The placeholder already says it: the label stays for screen readers. */
  labelHidden?: boolean;
}) {
  const kind = entryKind(value);
  const code = kind === "code";
  /* Words filter the rows as they are typed; the verb lights only for
     what it can answer: an address, a link, or a code. */
  const actionable = kind === "address" || kind === "code";
  return (
    <label className="sync-folder-field compact">
      <span className={labelHidden ? "sync-visually-hidden" : undefined}>Search, address or invite code</span>
      <div>
        <input value={value} disabled={disabled || joining} spellCheck={false} autoComplete="off"
          placeholder="Search, or paste a link, owner/name or an invite code" aria-invalid={invalid || undefined} aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && actionable) { event.preventDefault(); onSubmit(); } }} />
        <button type="button" className="btn repo-verb" disabled={disabled || looking || joining || !actionable} onClick={onSubmit}>
          <Icon name={code ? "user" : "search"} size={12} />
          {joining ? "Joining…" : looking ? "Looking up…" : code ? "Join" : "Look up"}
        </button>
      </div>
      {code && <small>Join to become a member.</small>}
      {error}
    </label>
  );
}

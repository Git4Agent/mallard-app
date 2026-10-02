import type { ReactNode } from "react";
import Icon from "../Icons";

export function isInvitationCodeShaped(text: string): boolean {
  const trimmed = text.trim();
  return !!trimmed && !/[/.]/.test(trimmed) && trimmed.replace(/[\s-]+/g, "").length === 20;
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
  const code = isInvitationCodeShaped(value);
  return (
    <label className="sync-folder-field compact">
      <span className={labelHidden ? "sync-visually-hidden" : undefined}>Address or invite code</span>
      <div>
        <input value={value} disabled={disabled || joining} spellCheck={false} autoComplete="off"
          placeholder="owner/name or invite code" aria-invalid={invalid || undefined} aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); onSubmit(); } }} />
        <button type="button" className="btn repo-verb" disabled={disabled || looking || joining || !value.trim()} onClick={onSubmit}>
          <Icon name={code ? "user" : "search"} size={12} />
          {joining ? "Joining…" : looking ? "Looking up…" : code ? "Join" : "Look up"}
        </button>
      </div>
      {code && <small>Join to become a member.</small>}
      {error}
    </label>
  );
}

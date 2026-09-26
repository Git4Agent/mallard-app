import { useId, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import Icon from "../Icons";
import type { FolderVerdict } from "./projectFolder";

/**
 * A project folder as a field: typed, pasted, or picked with Browse. It
 * never commits as it is typed — every prefix of a real path is a folder
 * too — so its owner decides what Enter and a Browse result do.
 */
export default function ProjectFolderField({
  value,
  verdict,
  disabled,
  autoFocus,
  labelHidden,
  placeholder,
  describedBy,
  onChange,
  onEnter,
  onBrowsed,
  onAnotherProfile,
}: {
  value: string;
  verdict: FolderVerdict | null;
  disabled: boolean;
  autoFocus?: boolean;
  /** For a band already headed Folder: the label stays for screen readers. */
  labelHidden?: boolean;
  placeholder: string;
  /** Another message about the field, such as a section's refusal. */
  describedBy?: string;
  onChange: (value: string) => void;
  onEnter: () => void;
  onBrowsed: (path: string) => void;
  onAnotherProfile?: () => void;
}) {
  const id = useId();
  const hintId = `${id}-hint`;
  const [browseFailed, setBrowseFailed] = useState(false);
  const browse = async () => {
    setBrowseFailed(false);
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) onBrowsed(picked);
    } catch {
      // Browse is the second way in; the field still works without it.
      setBrowseFailed(true);
    }
  };
  const line: FolderVerdict | null = browseFailed
    ? { tone: "warn", message: "The folder dialog didn’t open. Type or paste the path instead." }
    : verdict;
  const action = line?.action;
  return (
    <div className="sync-project-folder-field">
      <label htmlFor={id} className={labelHidden ? "sync-visually-hidden" : undefined}>Folder</label>
      <div className="sync-profile-select-row">
        <input
          id={id}
          value={value}
          disabled={disabled}
          autoFocus={autoFocus}
          placeholder={placeholder}
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          aria-invalid={line?.tone === "bad" || undefined}
          aria-describedby={[describedBy, line ? hintId : null].filter(Boolean).join(" ") || undefined}
          onChange={(event) => {
            setBrowseFailed(false);
            onChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            onEnter();
          }}
        />
        <button type="button" className="btn" onClick={() => void browse()} disabled={disabled}>
          <Icon name="folder" size={13} /> Browse…
        </button>
      </div>
      {line && (
        /* Polite: it changes on a debounce while the field keeps focus. */
        <p id={hintId} className={`sync-setup-path-hint ${line.tone}`} aria-live="polite">
          {line.message}
          {line.path && <code>{line.path}</code>}
          {action && (
            <button
              type="button"
              className="btn-link"
              disabled={disabled}
              onClick={() => ("value" in action ? onChange(action.value) : onAnotherProfile?.())}
            >
              {action.label}
            </button>
          )}
        </p>
      )}
    </div>
  );
}

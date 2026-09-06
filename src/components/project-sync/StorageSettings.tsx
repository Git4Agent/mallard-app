import { open } from "@tauri-apps/plugin-dialog";
import { useState, type ReactNode } from "react";
import type {
  SecretOperation,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSettings,
  StorageSetupDraft,
  StorageSummary,
} from "../../types";
import Icon, { iconNameFromHint } from "../Icons";
import { openExternalUrl } from "./externalLink";
import { compactProjectPath } from "./model";

/**
 * The storage form, drawn from a provider descriptor: every field, secret,
 * and assistant it shows was declared by the backend adapter. Nothing here
 * knows a provider by name.
 *
 * The form's own state is small on purpose. Settings are the descriptor's
 * fields as strings; secret values stay only in the open editor. The backend
 * reports a stored secret as a `present` flag.
 */
export interface StorageEditorForm {
  name: string;
  settings: Record<string, string>;
  /** Secret text in the field: typed this session, or revealed by the exact
   *  fetch of a new draft. It stays for the life of the open editor so its
   *  password mask and Show control do not change after a save. */
  typedSecrets: Record<string, string>;
  /** Secret values that the open editor knows are already stored. A field
   *  still equal to this local baseline does not travel again. */
  persistedSecrets?: Record<string, string>;
}

/** Which declared secrets the backend draft currently holds. */
export type SecretPresence = Record<string, boolean>;

export function findStorageProvider(
  providers: readonly StorageProviderDescriptor[],
  provider: StorageProviderKey,
  configVersion: number,
): StorageProviderDescriptor | null {
  return providers.find((candidate) => (
    candidate.key === provider && candidate.config_version === configVersion
  )) ?? null;
}

/** Settings cross IPC as a bounded JSON object; the form edits text. */
export function settingString(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

export function formFromDraft(
  descriptor: Pick<StorageProviderDescriptor, "fields">,
  draft: Pick<StorageSetupDraft, "name" | "settings"> & Partial<Pick<StorageSetupDraft, "secrets">>,
): StorageEditorForm {
  // A new draft's exact fetch carries its own secret values; they sit in the
  // field for review, masked, with Show to reveal — as a typed value would.
  const persisted = Object.fromEntries((draft.secrets ?? [])
    .filter((secret) => typeof secret.value === "string" && secret.value.length > 0)
    .map((secret) => [secret.secret_id, secret.value as string]));
  return {
    name: draft.name,
    settings: Object.fromEntries(descriptor.fields.map((field) => [
      field.field_id,
      settingString(draft.settings[field.field_id]),
    ])),
    typedSecrets: { ...persisted },
    ...(Object.keys(persisted).length > 0 ? { persistedSecrets: persisted } : {}),
  };
}

export function secretPresenceOf(draft: Pick<StorageSetupDraft, "secrets">): SecretPresence {
  return Object.fromEntries(draft.secrets.map((secret) => [secret.secret_id, secret.present]));
}

/** The settings object a draft update carries: the form's declared fields
 *  over whatever else the adapter keeps in the draft, which the form never
 *  shows and must not drop. */
export function draftSettingsFromForm(
  descriptor: Pick<StorageProviderDescriptor, "fields">,
  form: Pick<StorageEditorForm, "settings">,
  base: StorageSettings,
): StorageSettings {
  return {
    ...base,
    ...Object.fromEntries(descriptor.fields.map((field) => [
      field.field_id,
      (form.settings[field.field_id] ?? "").trim(),
    ])),
  };
}

/** Only the secrets the user touched travel; a saved one is `replace`, a
 *  missing one is `set`, and everything untyped stays as the backend has it. */
export function secretOperationsFromForm(
  form: Pick<StorageEditorForm, "typedSecrets" | "persistedSecrets">,
  presence: SecretPresence,
): Record<string, SecretOperation> {
  const operations: Record<string, SecretOperation> = {};
  for (const [secretId, value] of Object.entries(form.typedSecrets)) {
    // A persisted value still in the field is already stored.
    if (form.persistedSecrets?.[secretId] === value) continue;
    if (!value) continue;
    operations[secretId] = presence[secretId] ? { op: "replace", value } : { op: "set", value };
  }
  return operations;
}

function fieldSet(form: Pick<StorageEditorForm, "settings">, fieldId: string): boolean {
  return (form.settings[fieldId] ?? "").trim().length > 0;
}

function secretSet(
  form: Pick<StorageEditorForm, "typedSecrets">,
  presence: SecretPresence,
  secretId: string,
): boolean {
  return !!presence[secretId] || (form.typedSecrets[secretId] ?? "").length > 0;
}

/** Test and Finish open once the name, every required field, and every
 *  required secret (stored or typed) are in. The backend validates again. */
export function storageDraftReady(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets">,
  form: StorageEditorForm,
  presence: SecretPresence,
): boolean {
  return form.name.trim().length > 0
    && descriptor.fields.every((field) => !field.required || fieldSet(form, field.field_id))
    && descriptor.secrets.every((secret) => !secret.required || secretSet(form, presence, secret.secret_id));
}

/** The tally under the form: required fields plus required secrets, and how
 *  many of them are set. */
export function fieldsSet(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets">,
  form: StorageEditorForm,
  presence: SecretPresence,
): { set: number; total: number } {
  const requiredFields = descriptor.fields.filter((field) => field.required);
  const requiredSecrets = descriptor.secrets.filter((secret) => secret.required);
  return {
    set: requiredFields.filter((field) => fieldSet(form, field.field_id)).length
      + requiredSecrets.filter((secret) => secretSet(form, presence, secret.secret_id)).length,
    total: requiredFields.length + requiredSecrets.length,
  };
}

/** The one-line "where is it" every listing shows: the first path-shaped
 *  fact, else the first fact, else the provider's own label. */
export function storageSubtitle(storage: Pick<StorageSummary, "summary" | "provider_label">): string {
  const path = storage.summary.find((field) => field.is_path);
  if (path) return compactProjectPath(path.value);
  return storage.summary[0]?.value || storage.provider_label;
}

// A path subtitle keeps the mono face; any other fact reads as prose. The
// menu says which one it is handing over so the stylesheet reads a class.
export function storageSubtitleIsPath(storage: Pick<StorageSummary, "summary">): boolean {
  return storage.summary.some((field) => field.is_path);
}

/** The `?` beside a field: opens the declared documentation when the
 *  descriptor names one, otherwise it only carries the tooltip. Always a
 *  button so the mark keeps its 18px circle. */
function HelpMark({ text, url }: { text: string; url?: string | null }) {
  return (
    <button
      type="button"
      className="field-help"
      onClick={url ? () => void openExternalUrl(url) : undefined}
      title={text}
      aria-label={text}
    >
      ?
    </button>
  );
}

/** The name the descriptor's suggestion rule proposes for these settings,
 *  or null when the rule is absent, unmatched, or invalid. */
export function suggestStorageName(
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion">,
  settings: Record<string, string>,
): string | null {
  const rule = descriptor.name_suggestion;
  if (!rule) return null;
  try {
    const matched = (settings[rule.source] ?? "").match(new RegExp(rule.pattern));
    const suggested = matched?.[rule.group]?.trim() ?? "";
    return suggested.length > 0 ? suggested : null;
  } catch {
    return null;
  }
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Whether the name is still Mallard's — empty, the numbered default a new
 *  draft opens with, or the suggestion its current settings produce — so a
 *  later suggestion may replace it. A name the user typed is never touched. */
export function storageNameIsAuto(
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "default_name">,
  form: Pick<StorageEditorForm, "name" | "settings">,
): boolean {
  const name = form.name.trim();
  if (name.length === 0) return true;
  const stem = descriptor.default_name?.trim();
  if (stem && new RegExp(`^${escapeRegExp(stem)} \\d+$`).test(name)) return true;
  return name === suggestStorageName(descriptor, form.settings);
}

/** Apply every declared derivation whose source is `changedFieldId`: a match
 *  fills the derived field from its capture group, no match clears it. An
 *  invalid pattern leaves the field alone. */
export function applyFieldDerivations(
  descriptor: Pick<StorageProviderDescriptor, "fields">,
  settings: Record<string, string>,
  changedFieldId: string,
): Record<string, string> {
  const next = { ...settings };
  for (const field of descriptor.fields) {
    const rule = field.derive;
    if (!rule || rule.source !== changedFieldId) continue;
    let matched: RegExpMatchArray | null;
    try {
      matched = (settings[changedFieldId] ?? "").trim().match(new RegExp(rule.pattern));
    } catch {
      continue;
    }
    next[field.field_id] = matched?.[rule.group] ?? "";
  }
  return next;
}

export function StorageEditor({
  descriptor,
  providers,
  form,
  secretPresence,
  disabled,
  editsExisting,
  onChange,
  onSwitchProvider,
  onRemove,
  primaryAction,
  agentSlot,
}: {
  descriptor: StorageProviderDescriptor;
  /** Every provider this build offers, in the backend's order: the header's
   *  type switch lists them. Defaults to the draft's own. */
  providers?: readonly StorageProviderDescriptor[];
  form: StorageEditorForm;
  secretPresence: SecretPresence;
  disabled: boolean;
  /** The draft edits a storage that already exists: the name stays in the
   *  page heading and the card opens on the configured guide row. */
  editsExisting: boolean;
  onChange: (form: StorageEditorForm) => void;
  /** Switching type restarts the draft on the chosen provider; absent (and
   *  for an existing storage) the switch is shown locked. */
  onSwitchProvider?: (provider: StorageProviderDescriptor) => void;
  /** The trash at the header's end, for a storage that already exists. */
  onRemove?: () => void;
  /** An existing storage's save sits in the card's own footer. */
  primaryAction?: ReactNode;
  /** The workspace owns the assistant hand-off and its lifecycle; the editor
   *  only seats it, in the row the descriptor's assistant labels. */
  agentSlot?: ReactNode;
}) {
  const [secretVisible, setSecretVisible] = useState<Record<string, boolean>>({});
  const setName = (name: string) => onChange({ ...form, name });
  const setSetting = (fieldId: string, value: string) => {
    const settings = applyFieldDerivations(descriptor, { ...form.settings, [fieldId]: value }, fieldId);
    // The name follows the settings only while it is still Mallard's own;
    // a typed name stays, and the field stays editable either way.
    const suggested = storageNameIsAuto(descriptor, form) ? suggestStorageName(descriptor, settings) : null;
    onChange({ ...form, settings, name: suggested ?? form.name });
  };
  const setSecret = (secretId: string, value: string) => onChange({
    ...form,
    typedSecrets: { ...form.typedSecrets, [secretId]: value },
  });
  const chooseFolder = async (fieldId: string) => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) setSetting(fieldId, picked);
  };

  const folderFields = descriptor.fields.filter((field) => field.kind === "directory");
  const gridFields = descriptor.fields.filter((field) => field.kind !== "directory");
  const assistant = descriptor.setup_assistant ?? null;
  const credentialed = descriptor.secrets.length > 0 || assistant !== null;

  return (
    <article className="sync-storage-card">
      <header>
        <span className="sync-storage-icon"><Icon name={iconNameFromHint(descriptor.icon, "drive")} size={18} /></span>
        {editsExisting ? (
          <h2 className="sync-storage-editor-title">Storage settings</h2>
        ) : (
          <input
            value={form.name}
            onChange={(event) => setName(event.target.value)}
            disabled={disabled}
            aria-label="Storage name"
            placeholder="Storage name"
          />
        )}
        {/* The type switch lists every provider this build offers. A new
            draft restarts on the chosen one; an existing storage keeps its
            type, so the switch only states it. */}
        <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
          {(providers ?? [descriptor]).map((provider) => {
            const current = provider.key === descriptor.key && provider.config_version === descriptor.config_version;
            return (
              <button
                key={`${provider.key}@${provider.config_version}`}
                type="button"
                role="radio"
                aria-checked={current}
                className={current ? "active" : undefined}
                onClick={() => { if (!current) onSwitchProvider?.(provider); }}
                disabled={disabled || editsExisting || !onSwitchProvider}
              >
                {provider.display_name}
              </button>
            );
          })}
        </div>
        {onRemove && (
          <button type="button" className="btn btn-ghost danger" onClick={onRemove} disabled={disabled} title="Remove storage" aria-label="Remove storage"><Icon name="trash" size={14} /></button>
        )}
      </header>

      {folderFields.map((field) => (
        <label key={field.field_id} className="sync-folder-field compact">
          <span className="sync-field-name">{field.label}</span>
          <div>
            <input
              type="text"
              value={form.settings[field.field_id] ?? ""}
              onChange={(event) => setSetting(field.field_id, event.target.value)}
              placeholder={field.placeholder ?? undefined}
              disabled={disabled}
              required={field.required}
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
            <button type="button" className="btn" onClick={() => void chooseFolder(field.field_id)} disabled={disabled}>
              <Icon name="folder" size={13} /> Browse…
            </button>
          </div>
          {field.help && <small>{field.help}</small>}
        </label>
      ))}

      {!editsExisting && assistant && (
        <>
          {/* The two routes are named lanes, not one run-on sentence: the
              agent lane carries the hand-off cluster and its lifecycle, the
              manual lane introduces the fields it sits above. Both take their
              words from the descriptor, never from a provider name. */}
          <div className="storage-setup-route-row agent" role="note">
            <Icon name="terminal" size={13} />
            <span className="storage-setup-route-label">{assistant.label}</span>
            <span className="storage-setup-guide-copy">{assistant.description}</span>
            {agentSlot}
          </div>
          <div className="storage-setup-route-row" role="note">
            <Icon name="info" size={13} />
            <span className="storage-setup-route-label">Manual setup</span>
            <span className="storage-setup-guide-copy">
              {descriptor.manual_setup_hint ?? `Enter an existing ${descriptor.display_name} connection in the fields below.`}
            </span>
            {descriptor.setup_guide_url && (
              <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>
                Setup guide <Icon name="external-link" size={11} />
              </button>
            )}
          </div>
        </>
      )}
      {editsExisting && credentialed && (
        <div className="storage-setup-guide" role="note">
          <Icon name="check-circle" size={13} />
          <span className="storage-setup-guide-copy">Configured. You can review or update the connection values below.</span>
          {descriptor.setup_guide_url && (
            <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>Setup guide</button>
          )}
          {agentSlot}
        </div>
      )}

      {(gridFields.length > 0 || descriptor.secrets.length > 0) && (
        <div className="sync-storage-fields storage-provider-fields" aria-label={`${descriptor.display_name} connection details`}>
          {gridFields.map((field) => (
            <label key={field.field_id} className={field.kind === "url" ? "storage-provider-url-field" : undefined}>
              <span className="form-label-row">
                <span className="sync-field-name">{field.label}</span>
                {(field.help || field.help_url) && (
                  <HelpMark text={field.help ?? field.label} url={field.help_url} />
                )}
              </span>
              <input
                type={field.kind === "url" ? "url" : "text"}
                value={form.settings[field.field_id] ?? ""}
                onChange={(event) => setSetting(field.field_id, event.target.value)}
                placeholder={field.placeholder ?? undefined}
                disabled={disabled}
                required={field.required}
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
              />
            </label>
          ))}
          {descriptor.secrets.map((secret) => {
            const present = !!secretPresence[secret.secret_id];
            const visible = !!secretVisible[secret.secret_id];
            return (
              <label key={secret.secret_id}>
                <span className="form-label-row">
                  <span className="sync-field-name">{secret.label}</span>
                  {(secret.help || secret.help_url) && (
                    <HelpMark text={secret.help ?? secret.label} url={secret.help_url} />
                  )}
                </span>
                {/* Backend updates return presence only. A value known by the
                    open editor remains here for stable masking and review;
                    otherwise the field accepts a replacement. */}
                <span className="sync-secret-input">
                  <input
                    type={visible ? "text" : "password"}
                    value={form.typedSecrets[secret.secret_id] ?? ""}
                    onChange={(event) => setSecret(secret.secret_id, event.target.value)}
                    placeholder={present ? "Stored — type to replace" : (secret.placeholder ?? undefined)}
                    disabled={disabled}
                    required={secret.required && !present}
                    autoComplete="new-password"
                  />
                  <button
                    type="button"
                    className="sync-secret-reveal"
                    onClick={() => setSecretVisible((current) => ({ ...current, [secret.secret_id]: !visible }))}
                    aria-pressed={visible}
                    disabled={disabled}
                  >
                    {visible ? "Hide" : "Show"}
                  </button>
                </span>
              </label>
            );
          })}
        </div>
      )}
      {primaryAction && (
        <footer className="sync-storage-editor-footer">
          {descriptor.secrets.length > 0 && (
            <span className="sync-storage-editor-progress" aria-live="polite">
              {fieldsSet(descriptor, form, secretPresence).set} of {fieldsSet(descriptor, form, secretPresence).total} set
            </span>
          )}
          {primaryAction}
        </footer>
      )}
    </article>
  );
}

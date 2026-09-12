import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import type {
  SecretOperation,
  StorageConnectionHealth,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSettings,
  StorageSetupDraft,
  StorageSummary,
} from "../../types";
import Icon, { iconNameFromHint, type IconName } from "../Icons";
import { openExternalUrl } from "./externalLink";
import { compactProjectPath, formatRelativeTime } from "./model";
import { rowReceipt, useSettingsReceipts } from "./settingsReceipts";

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

export function hasManagedCredentials(
  descriptor: Pick<StorageProviderDescriptor, "managed_secret_id">,
  presence: SecretPresence,
): boolean {
  const key = descriptor.managed_secret_id;
  return !!key && !!presence[key];
}

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

function activeSecrets(
  descriptor: Pick<StorageProviderDescriptor, "secrets" | "managed_secret_id">,
  presence: SecretPresence,
) {
  const managedId = descriptor.managed_secret_id;
  return descriptor.secrets.filter((secret) => hasManagedCredentials(descriptor, presence)
    ? secret.secret_id === managedId
    : secret.secret_id !== managedId);
}

/** Test and Finish open once the name, every required field, and every
 *  required secret (stored or typed) are in. The backend validates again. */
export function storageDraftReady(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets" | "managed_secret_id">,
  form: StorageEditorForm,
  presence: SecretPresence,
): boolean {
  return form.name.trim().length > 0
    && descriptor.fields.every((field) => !field.required || fieldSet(form, field.field_id))
    && activeSecrets(descriptor, presence).every((secret) => !secret.required || secretSet(form, presence, secret.secret_id));
}

/** The tally under the form: required fields plus required secrets, and how
 *  many of them are set. */
export function fieldsSet(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets" | "managed_secret_id">,
  form: StorageEditorForm,
  presence: SecretPresence,
): { set: number; total: number } {
  const requiredFields = descriptor.fields.filter((field) => field.required);
  const requiredSecrets = activeSecrets(descriptor, presence).filter((secret) => secret.required);
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

/** A recorded state only a new credential or consent recovers. The backend
 *  refuses every request to such a storage until then, so the UI leads to
 *  its settings rather than offering a retry. */
export function connectionSuspends(health: StorageConnectionHealth | null | undefined): boolean {
  return health?.state === "reconnect_required"
    || health?.state === "token_expired"
    || health?.state === "permission_denied";
}

/** The one-line issue a listing shows for a suspended storage, naming the
 *  provider by its descriptor label; nothing for any other state. */
export function storageConnectionIssue(
  storage: Pick<StorageSummary, "provider_label" | "connection">,
): string | null {
  switch (storage.connection?.state) {
    case "reconnect_required":
    case "token_expired":
      return `${storage.provider_label} needs reconnection`;
    case "permission_denied":
      return `${storage.provider_label} denied access`;
    default:
      return null;
  }
}

/** The message with its evidence, the way the backend prints it into a
 *  failure: route, status, fixed error identifier, request ID. */
export function connectionDetail(health: StorageConnectionHealth): string {
  const evidence = health.evidence;
  if (!evidence) return health.message;
  const facts = [
    evidence.endpoint,
    typeof evidence.http_status === "number" ? `HTTP ${evidence.http_status}` : "no response",
    evidence.error_tag ?? null,
    evidence.request_id ? `request ${evidence.request_id}` : null,
  ].filter((fact): fact is string => !!fact);
  return `${health.message} (${facts.join(", ")})`;
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
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "local_setup">,
  settings: Record<string, string>,
): string | null {
  const rule = descriptor.name_suggestion;
  if (!rule) return null;
  try {
    const matched = (settings[rule.source] ?? "").match(new RegExp(rule.pattern));
    const suggested = matched?.[rule.group]?.trim() ?? "";
    return suggested.length > 0 ? descriptor.local_setup === "wrangler" ? `R2 ${suggested}` : suggested : null;
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
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "default_name" | "local_setup">,
  form: Pick<StorageEditorForm, "name" | "settings">,
): boolean {
  const name = form.name.trim();
  if (name.length === 0) return true;
  const stem = descriptor.default_name?.trim();
  if (stem && (name === stem || new RegExp(`^${escapeRegExp(stem)} \\d+$`).test(name))) return true;
  return name === suggestStorageName(descriptor, form.settings);
}

/** Use the same naming rule for typed settings and a prepared Wrangler draft. */
export function storageNameForSettings(
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "default_name" | "local_setup">,
  form: Pick<StorageEditorForm, "name" | "settings">,
  settings: Record<string, string>,
): string {
  if (!storageNameIsAuto(descriptor, form)) return form.name;
  // Clearing a bucket keeps the name automatic while its replacement is typed.
  return suggestStorageName(descriptor, settings)
    ?? (descriptor.local_setup === "wrangler" ? descriptor.default_name : null)
    ?? form.name;
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

/** A folder as two people would type it: trailing separators dropped, the
 *  root kept. Case and symlinks are the backend's call — its canonical check
 *  still runs on Test and Save — so the page only catches what it can see. */
function comparableDirectory(value: string): string {
  const trimmed = value.trim();
  const stripped = trimmed.replace(/[\/\\]+$/, "");
  return stripped === "" && trimmed !== "" ? trimmed[0] : stripped;
}

/** The connected storage that already occupies a folder the form names, if
 *  any: same provider, same directory field, same folder as typed. The
 *  storage being edited is not a conflict with itself. Only directory fields
 *  are compared — a folder is the one destination the page can read off a
 *  storage's summary facts. */
export function findDestinationConflict(
  descriptor: Pick<StorageProviderDescriptor, "key" | "fields">,
  form: Pick<StorageEditorForm, "settings">,
  storages: readonly StorageSummary[],
  excludeStorageId: string | null,
): StorageSummary | null {
  for (const field of descriptor.fields) {
    if (field.kind !== "directory") continue;
    const typed = comparableDirectory(settingString(form.settings[field.field_id]));
    if (!typed) continue;
    const owner = storages.find((storage) => (
      storage.storage_id !== excludeStorageId
      && storage.provider === descriptor.key
      && storage.summary.some((fact) => (
        fact.field_id === field.field_id && fact.is_path && comparableDirectory(fact.value) === typed
      ))
    ));
    if (owner) return owner;
  }
  return null;
}

/** The New-storage question: a fresh bucket or folder, or one Mallard
 *  already writes to. Only asked on a provider that can look in its account. */
export type StorageSetupMode = "create" | "existing";

function ModeDoor({ icon, title, detail, help, checked, disabled, onChoose }: {
  icon: IconName;
  title: string;
  detail: string;
  help?: { label: string; url: string } | null;
  checked: boolean;
  disabled: boolean;
  onChoose: () => void;
}) {
  return (
    <div className={`storage-mode-option${help ? " has-help" : ""}`}>
      <button type="button" className="storage-mode-door" role="radio" aria-checked={checked} disabled={disabled} onClick={onChoose}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
      {help && (
        <a
          className="btn-link storage-mode-help"
          href={help.url}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => { event.preventDefault(); void openExternalUrl(help.url); }}
        >
          {help.label} <Icon name="external-link" size={11} />
        </a>
      )}
    </div>
  );
}

/** The New-storage form. A settled storage never opens here: its page is
 *  `StorageLockedForm`, and a credential is replaced through
 *  `StorageReconnectBand`. */
export function StorageEditor({
  descriptor,
  providers,
  form,
  secretPresence,
  disabled,
  onChange,
  onSwitchProvider,
  setupSlot,
  destinationConflict,
  setupMode,
  discoverySlot,
  manualEntry,
  setupDone = false,
}: {
  descriptor: StorageProviderDescriptor;
  /** Every provider this build offers, in the backend's order: the header's
   *  type switch lists them. Defaults to the draft's own. */
  providers?: readonly StorageProviderDescriptor[];
  form: StorageEditorForm;
  secretPresence: SecretPresence;
  disabled: boolean;
  onChange: (form: StorageEditorForm) => void;
  /** Switching type restarts the draft on the chosen provider; absent the
   *  switch is shown locked. */
  onSwitchProvider?: (provider: StorageProviderDescriptor) => void;
  /** The workspace owns setup and its lifecycle. */
  setupSlot?: ReactNode;
  /** The connected storage that already occupies the folder the form names.
   *  The editor states it under the folder field with a door to that
   *  storage; the workspace, which owns the verbs, disables Test and Save
   *  while it stands. */
  destinationConflict?: { storage: StorageSummary; onOpen: () => void } | null;
  /** The New-storage question for a provider that can look in its account:
   *  the door that stands chosen, or null while it is still being asked.
   *  The doors stay as a switch for as long as the workspace passes this. */
  setupMode?: { mode: StorageSetupMode | null; onChoose: (mode: StorageSetupMode) => void };
  /** The Use-existing lane, what it found, and the by-hand line: the
   *  workspace owns the look and seats it here, where the lanes would be. */
  discoverySlot?: ReactNode;
  /** Expands manual fields for providers without a Wrangler setup lane. */
  manualEntry?: boolean;
  /** The setup lane reached the final connection review. */
  setupDone?: boolean;
}) {
  const fieldIdPrefix = useId();
  const [secretVisible, setSecretVisible] = useState<Record<string, boolean>>({});
  const [manualExpanded, setManualExpanded] = useState(false);
  const setName = (name: string) => onChange({ ...form, name });
  const setSetting = (fieldId: string, value: string) => {
    const settings = applyFieldDerivations(descriptor, { ...form.settings, [fieldId]: value }, fieldId);
    // The name follows the settings only while it is still Mallard's own;
    // a typed name stays, and the field stays editable either way.
    onChange({ ...form, settings, name: storageNameForSettings(descriptor, form, settings) });
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
  const managed = hasManagedCredentials(descriptor, secretPresence);
  /* A new draft on a provider that can look in its account is asked Create
     new or Use existing before it shows a lane. The doors are the editor's;
     what each door leads to is the workspace's. A provider that cannot look
     is past the question. */
  const doors = setupMode && descriptor.discovery
    ? { ...setupMode, discovery: descriptor.discovery }
    : null;
  const usingExisting = doors?.mode === "existing";
  const showLanes = doors === null || doors.mode === "create";
  /* Direct setup is available for both creation and discovery. */
  const wranglerLane = !!descriptor.local_setup && !managed;
  // Keep review available when a prepared draft is reopened or edited.
  const wranglerReviewReady = setupDone || descriptor.secrets.some((secret) => secretPresence[secret.secret_id]);
  const showFields = wranglerLane
    ? wranglerReviewReady || manualExpanded
    : showLanes || (usingExisting && !!manualEntry);
  return (
    <article className="sync-storage-card">
      <header>
        <span className="sync-storage-icon"><Icon name={iconNameFromHint(descriptor.icon, "drive")} size={18} /></span>
        <input
          value={form.name}
          onChange={(event) => setName(event.target.value)}
          disabled={disabled}
          aria-label="Storage name"
          placeholder="Storage name"
        />
        {/* The type switch lists every provider this build offers; a draft
            restarts on the chosen one. */}
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
                disabled={disabled || !onSwitchProvider}
              >
                {provider.display_name}
              </button>
            );
          })}
        </div>
      </header>

      {doors && (
        <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing storage">
          <ModeDoor
            icon="plus"
            title="Create new"
            detail={doors.discovery.create_hint}
            checked={doors.mode === "create"}
            disabled={disabled}
            onChoose={() => doors.onChoose("create")}
          />
          <ModeDoor
            icon="search"
            title="Use existing"
            detail={doors.discovery.existing_hint}
            help={doors.discovery.existing_help}
            checked={doors.mode === "existing"}
            disabled={disabled}
            onChoose={() => doors.onChoose("existing")}
          />
        </div>
      )}

      {showFields && folderFields.map((field) => (
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
      {showFields && destinationConflict && folderFields.length > 0 && (
        /* One folder is one storage. Said the moment the folder settles, in
           the box grammar every other refusal wears, with the only two ways
           on: the storage that owns the folder, or a different folder. */
        <div className="notice danger storage-destination-conflict" role="alert">
          <Icon name="alert-triangle" size={13} className="notice-mark" />
          <div className="notice-body">
            <p className="notice-line">
              <strong className="notice-type">Error ·</strong>{" "}
              <strong className="notice-lead">
                This folder is already connected as {destinationConflict.storage.name}.
              </strong>{" "}
              A folder can be one storage. Open that storage, or choose another folder for this one.
            </p>
          </div>
          <div className="storage-destination-conflict-verbs">
            <button type="button" className="btn btn-primary" onClick={destinationConflict.onOpen} disabled={disabled}>
              Open {destinationConflict.storage.name}
            </button>
            <button type="button" className="btn" onClick={() => void chooseFolder(folderFields[0].field_id)} disabled={disabled}>
              <Icon name="folder" size={13} /> Choose another…
            </button>
          </div>
        </div>
      )}

      {wranglerLane && (showLanes || usingExisting) && (
        <div className="storage-setup-route-row agent r2-lane" role="note">
          <Icon name={descriptor.local_setup === "dropbox" ? "link" : "terminal"} size={13} />
          <span className="storage-setup-route-label">{descriptor.local_setup === "dropbox" ? "Dropbox setup" : "Wrangler setup"}</span>
          {setupSlot}
        </div>
      )}
      {usingExisting && discoverySlot}
      {(showLanes || usingExisting && wranglerLane) && !managed && (wranglerLane || !setupDone) && (wranglerLane || descriptor.manual_setup_hint || descriptor.setup_guide_url) && (
        <>

          <div className="storage-setup-route-row" role="note">
            <Icon name="info" size={13} />
            <span className="storage-setup-route-label">Manual setup</span>
            <span className="storage-setup-guide-copy">
              {wranglerLane && !showFields
                ? "Use your own connection credentials."
                : descriptor.manual_setup_hint ?? `Enter an existing ${descriptor.display_name} connection in the fields below.`}
            </span>
            {descriptor.setup_guide_url && (
              <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>
                Setup guide <Icon name="external-link" size={11} />
              </button>
            )}
            {wranglerLane && !wranglerReviewReady && (
              <button
                type="button"
                className="btn storage-manual-toggle"
                aria-expanded={manualExpanded}
                aria-label={manualExpanded ? "Hide fields" : "Enter by hand"}
                title={manualExpanded ? "Hide fields" : "Enter by hand"}
                disabled={disabled}
                onClick={() => setManualExpanded((expanded) => !expanded)}
              >
                <Icon name={manualExpanded ? "chevron-up" : "chevron-down"} size={12} />
              </button>
            )}
          </div>
        </>
      )}
      {managed && (
        /* A stored credential by itself proves nothing: "Connected" waits
           for an observed success, which only the settled storage's page
           can report. */
        <div className={`storage-setup-guide${descriptor.local_setup ? " r2-lane" : ""}`} role="note">
          <Icon name="check-circle" size={13} />
          <span className="storage-setup-guide-copy">Access renews automatically. Test the connection to verify it.</span>
          {descriptor.setup_guide_url && (
            <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>Setup guide</button>
          )}
          {setupSlot}
        </div>
      )}

      {showFields && (gridFields.length > 0 || descriptor.secrets.length > 0) && (
        <div className="sync-storage-fields storage-provider-fields" aria-label={`${descriptor.display_name} connection details`}>
          {gridFields.map((field) => (
            <div key={field.field_id} className={`storage-provider-field${field.kind === "url" ? " storage-provider-url-field" : ""}`}>
              <span className="form-label-row">
                <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-field-${field.field_id}`}>{field.label}</label>
                {(field.help || field.help_url) && (
                  <HelpMark text={field.help ?? field.label} url={field.help_url} />
                )}
              </span>
              <input
                type={field.kind === "url" ? "url" : "text"}
                id={`${fieldIdPrefix}-field-${field.field_id}`}
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
            </div>
          ))}
          {activeSecrets(descriptor, secretPresence).map((secret) => {
            const present = !!secretPresence[secret.secret_id];
            const visible = !!secretVisible[secret.secret_id];
            return (
              <div key={secret.secret_id} className="storage-provider-field">
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-secret-${secret.secret_id}`}>{secret.label}</label>
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
                    id={`${fieldIdPrefix}-secret-${secret.secret_id}`}
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
              </div>
            );
          })}
        </div>
      )}
    </article>
  );
}

/** How long a revealed secret stays on screen before the field masks
 *  itself again. */
export const SECRET_REVEAL_MS = 30_000;
const SECRET_MASK = "••••••••••••••••••••••••";
const COPIED_MS = 1_500;

type Revealed = { value: string; hidesAt: number };

/**
 * A settled storage's page: the setup form with every field read-only,
 * because a destination is fixed for the life of a storage. Two verbs
 * remain — the name, which saves itself, and Test connection — plus the
 * one editable moment, replacing a refused credential, which the workspace
 * seats here as `reconnect.band`.
 *
 * A saved secret is not in the page. Show asks the backend for that one
 * value and the field masks itself again after SECRET_REVEAL_MS, on Hide,
 * or when the page goes; Copy fetches, writes the clipboard, and drops it.
 */
export function StorageLockedForm({
  storage,
  descriptor,
  disabled,
  onRename,
  focusNameSeq = 0,
  onRevealSecret,
  onTest,
  testing,
  verifiedAt,
  reconnect,
}: {
  storage: StorageSummary;
  descriptor: StorageProviderDescriptor;
  disabled: boolean;
  /** Save the name (Save or Enter); resolves false when refused (the shell
   *  has shown why) so the receipt can say so. */
  onRename: (name: string) => Promise<boolean>;
  /** Bumped by the ⋯ menu's Rename to put the caret in the name field. */
  focusNameSeq?: number;
  onRevealSecret: (secretId: string) => Promise<string>;
  onTest: () => void;
  testing: boolean;
  /** When this visit last saw the connection verified; null until then. */
  verifiedAt: number | null;
  reconnect: {
    open: boolean;
    onOpen: () => void;
    /** The secrets-only editor, seated under the guide row while open. */
    band: ReactNode;
  };
}) {
  const fieldIdPrefix = useId();
  const [name, setName] = useState(storage.name);
  const savedName = storage.name;
  useEffect(() => { setName(savedName); }, [savedName]);
  const receipts = useSettingsReceipts();
  const nameRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!focusNameSeq) return;
    nameRef.current?.focus();
    nameRef.current?.select();
  }, [focusNameSeq]);
  const [savingName, setSavingName] = useState(false);
  const nameDirty = name.trim() !== savedName;
  const commitName = async () => {
    const next = name.trim();
    if (!next || next === savedName || savingName) return;
    setSavingName(true);
    receipts.begin("name", ["name"]);
    const ok = await onRename(next);
    receipts.settle("name", ok, ok ? `Storage renamed to ${next}` : null);
    setSavingName(false);
  };
  const cancelName = () => setName(savedName);
  const nameReceipt = rowReceipt(receipts.state, "name");

  const [revealed, setRevealed] = useState<Record<string, Revealed>>({});
  const [revealing, setRevealing] = useState<Record<string, boolean>>({});
  const [revealError, setRevealError] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<Record<string, boolean>>({});
  const [now, setNow] = useState(() => Date.now());
  const timers = useRef(new Set<number>());
  useEffect(() => {
    const standing = timers.current;
    return () => { for (const timer of standing) window.clearTimeout(timer); };
  }, []);
  const later = (ms: number, run: () => void) => {
    const timer = window.setTimeout(() => { timers.current.delete(timer); run(); }, ms);
    timers.current.add(timer);
  };
  // One clock for every countdown; it runs only while something is shown.
  const anyRevealed = Object.keys(revealed).length > 0;
  useEffect(() => {
    if (!anyRevealed) return;
    const tick = window.setInterval(() => {
      const at = Date.now();
      setNow(at);
      setRevealed((current) => {
        const kept = Object.fromEntries(Object.entries(current).filter(([, entry]) => entry.hidesAt > at));
        return Object.keys(kept).length === Object.keys(current).length ? current : kept;
      });
    }, 1000);
    return () => window.clearInterval(tick);
  }, [anyRevealed]);
  const conceal = (secretId: string) => setRevealed((current) => {
    const { [secretId]: _gone, ...rest } = current;
    return rest;
  });
  const reveal = async (secretId: string) => {
    if (revealing[secretId]) return;
    setRevealing((current) => ({ ...current, [secretId]: true }));
    setRevealError((current) => { const { [secretId]: _gone, ...rest } = current; return rest; });
    try {
      const value = await onRevealSecret(secretId);
      setNow(Date.now());
      setRevealed((current) => ({ ...current, [secretId]: { value, hidesAt: Date.now() + SECRET_REVEAL_MS } }));
    } catch (reason) {
      setRevealError((current) => ({ ...current, [secretId]: reason instanceof Error ? reason.message : String(reason) }));
    } finally {
      setRevealing((current) => { const { [secretId]: _gone, ...rest } = current; return rest; });
    }
  };
  const copy = async (secretId: string) => {
    if (revealing[secretId]) return;
    setRevealing((current) => ({ ...current, [secretId]: true }));
    setRevealError((current) => { const { [secretId]: _gone, ...rest } = current; return rest; });
    try {
      const value = revealed[secretId]?.value ?? await onRevealSecret(secretId);
      await navigator.clipboard.writeText(value);
      setCopied((current) => ({ ...current, [secretId]: true }));
      later(COPIED_MS, () => setCopied((current) => { const { [secretId]: _gone, ...rest } = current; return rest; }));
    } catch (reason) {
      setRevealError((current) => ({ ...current, [secretId]: reason instanceof Error ? reason.message : String(reason) }));
    } finally {
      setRevealing((current) => { const { [secretId]: _gone, ...rest } = current; return rest; });
    }
  };

  const settingString = (fieldId: string) => {
    const value = storage.settings[fieldId];
    return typeof value === "string" ? value : value == null ? "" : String(value);
  };
  const presence = Object.fromEntries(storage.secrets.map((secret) => [secret.secret_id, secret.present]));
  const folderFields = descriptor.fields.filter((field) => field.kind === "directory");
  const gridFields = descriptor.fields.filter((field) => field.kind !== "directory");
  const managed = hasManagedCredentials(descriptor, presence);
  const suspended = connectionSuspends(storage.connection);
  const lock = <span className="sync-storage-lock" title="Fixed for this storage"><Icon name="lock" size={11} /></span>;
  /* "Connected" is said only after this visit saw a probe succeed; a stored
     credential proves nothing, and a refused one says so in the row. */
  const guideCopy = verifiedAt !== null
    ? `Connected · verified ${formatRelativeTime(Math.floor(verifiedAt / 1000))}.${managed ? " Access renews automatically." : ""}`
    : `Configured. Test the connection to verify it.${managed ? " Access renews automatically." : ""}`;
  const showGuide = suspended || descriptor.secrets.length > 0 || !!descriptor.local_setup;

  return (
    <article className="sync-storage-card sync-storage-locked">
      <header>
        <span className="sync-storage-icon"><Icon name={iconNameFromHint(descriptor.icon, "drive")} size={18} /></span>
        {/* The name saves the way a project rename does: Save and Cancel
            appear beside the field as soon as it differs from what is
            saved; Enter saves, Escape cancels; the receipt says it landed. */}
        <form
          className="sync-storage-name"
          onSubmit={(event) => { event.preventDefault(); void commitName(); }}
        >
          <input
            ref={nameRef}
            value={name}
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") { cancelName(); event.currentTarget.blur(); }
            }}
            disabled={disabled || savingName}
            aria-label="Storage name"
            placeholder="Storage name"
          />
          {nameDirty && (
            <span className="sync-storage-name-verbs">
              <button type="submit" className="btn btn-primary" disabled={disabled || savingName || !name.trim()}>
                {savingName ? "Saving…" : "Save"}
              </button>
              <button type="button" className="btn btn-ghost" disabled={disabled || savingName} onClick={cancelName}>
                Cancel
              </button>
            </span>
          )}
          {nameReceipt && !nameDirty && (
            <span className={`sync-storage-name-stamp ${nameReceipt.kind}${nameReceipt.kind === "saved" && nameReceipt.fading ? " dim" : ""}`} role="status">
              {nameReceipt.kind === "saving"
                ? <><Icon name="refresh" size={11} className="icon-spin" /> Saving…</>
                : nameReceipt.kind === "failed"
                  ? <><Icon name="alert-triangle" size={11} /> Not saved</>
                  : <><Icon name="check" size={11} /> Saved</>}
            </span>
          )}
        </form>
        <span className="sync-storage-editor-provider">{descriptor.display_name}</span>
      </header>

      {showGuide && (
        <div className="storage-setup-guide" role={suspended ? "alert" : "note"}>
          <Icon name={suspended ? "alert-triangle" : verifiedAt !== null ? "check-circle" : "info"} size={13} />
          <span className={suspended ? "storage-setup-guide-error" : "storage-setup-guide-copy"}>
            {suspended ? storage.connection!.message : guideCopy}
          </span>
          {descriptor.setup_guide_url && (
            <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>Setup guide</button>
          )}
          {suspended && !reconnect.open && (
            <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={reconnect.onOpen}>
              <Icon name="refresh" size={12} /> Reconnect…
            </button>
          )}
        </div>
      )}
      {reconnect.open && reconnect.band}

      {folderFields.map((field) => (
        <label key={field.field_id} className="sync-folder-field compact">
          <span className="sync-field-name">{field.label} {lock}</span>
          <div>
            <input type="text" value={settingString(field.field_id)} readOnly spellCheck={false} />
          </div>
          {field.help && <small>{field.help}</small>}
        </label>
      ))}

      {(gridFields.length > 0 || descriptor.secrets.length > 0) && (
        <div className="sync-storage-fields storage-provider-fields" aria-label={`${descriptor.display_name} connection details`}>
          {gridFields.map((field) => (
            <div key={field.field_id} className={`storage-provider-field${field.kind === "url" ? " storage-provider-url-field" : ""}`}>
              <span className="form-label-row">
                <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-field-${field.field_id}`}>{field.label}</label>
                {lock}
              </span>
              <input
                type={field.kind === "url" ? "url" : "text"}
                id={`${fieldIdPrefix}-field-${field.field_id}`}
                value={settingString(field.field_id)}
                readOnly
                spellCheck={false}
              />
              {field.help && <small className="sync-storage-field-help">{field.help}</small>}
            </div>
          ))}
          {descriptor.secrets.map((secret) => {
            const present = !!presence[secret.secret_id];
            const shown = revealed[secret.secret_id];
            const busy = !!revealing[secret.secret_id];
            const error = revealError[secret.secret_id];
            const secondsLeft = shown ? Math.max(0, Math.ceil((shown.hidesAt - now) / 1000)) : 0;
            const inputId = `${fieldIdPrefix}-secret-${secret.secret_id}`;
            return (
              <div key={secret.secret_id} className="storage-provider-field">
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={inputId}>{secret.label}</label>
                  {suspended && present && (
                    <span className="sync-storage-refused-chip"><Icon name="alert-triangle" size={10} /> Refused</span>
                  )}
                  {lock}
                  {copied[secret.secret_id]
                    ? <span className="sync-secret-cap copied" role="status"><Icon name="check" size={11} /> Copied</span>
                    : busy
                      ? <span className="sync-secret-cap"><span className="status-loader" /> Reading…</span>
                      : shown
                        ? <span className="sync-secret-cap"><Icon name="clock" size={11} /> Hides in {secondsLeft} s</span>
                        : null}
                </span>
                <span className={`sync-secret-input${present ? " has-copy" : ""}`}>
                  <input
                    type={shown ? "text" : "password"}
                    id={inputId}
                    value={shown ? shown.value : present ? SECRET_MASK : ""}
                    placeholder={present ? undefined : "Not stored"}
                    readOnly
                    spellCheck={false}
                    autoComplete="off"
                  />
                  {present && (
                    <span className="sync-secret-verbs">
                      <button
                        type="button"
                        className="sync-secret-copy"
                        onClick={() => void copy(secret.secret_id)}
                        disabled={disabled || busy}
                        title={`Copy ${secret.label}`}
                        aria-label={`Copy ${secret.label}`}
                      >
                        <Icon name="copy" size={13} />
                      </button>
                      <button
                        type="button"
                        className="sync-secret-reveal"
                        onClick={() => { if (shown) conceal(secret.secret_id); else void reveal(secret.secret_id); }}
                        aria-pressed={!!shown}
                        disabled={disabled || busy}
                      >
                        {shown ? "Hide" : "Show"}
                      </button>
                    </span>
                  )}
                </span>
                {/* A refused read is said under the field, where there is
                    room for a sentence; the label row keeps its one word. */}
                {error
                  ? <small className="sync-storage-field-help error" role="alert"><Icon name="alert-triangle" size={11} /> {error}</small>
                  : secret.help && <small className="sync-storage-field-help">{secret.help}</small>}
              </div>
            );
          })}
        </div>
      )}

      <footer className="sync-storage-editor-footer">
        <span className="sync-storage-fixed">
          <Icon name="lock" size={12} />
          This storage’s destination and credentials are fixed. To sync somewhere else, add a new storage.
        </span>
        {verifiedAt !== null && !testing && (
          <span className="storage-setup-verified" role="status">
            <Icon name="check-circle" size={13} /> Connection verified
          </span>
        )}
        <button
          type="button"
          className="btn"
          disabled={disabled || testing || suspended || reconnect.open}
          onClick={onTest}
        >
          {testing ? "Testing…" : "Test connection"}
        </button>
      </footer>
    </article>
  );
}

/**
 * The one editable band on a settled storage: the credential, and nothing
 * else. The workspace opens an edit draft for it (the backend refuses any
 * settings change on that draft), seats the provider's lane, and owns the
 * verbs — Cancel, Test connection, Save credential.
 */
export function StorageReconnectBand({
  descriptor,
  form,
  secretPresence,
  disabled,
  connection,
  onChange,
  setupSlot,
  verbs,
}: {
  descriptor: StorageProviderDescriptor;
  form: StorageEditorForm;
  secretPresence: SecretPresence;
  disabled: boolean;
  connection?: StorageConnectionHealth | null;
  onChange: (form: StorageEditorForm) => void;
  setupSlot?: ReactNode;
  verbs: ReactNode;
}) {
  const fieldIdPrefix = useId();
  const [secretVisible, setSecretVisible] = useState<Record<string, boolean>>({});
  const setSecret = (secretId: string, value: string) => onChange({
    ...form,
    typedSecrets: { ...form.typedSecrets, [secretId]: value },
  });
  const replacementPlaceholder = connection?.state === "token_expired"
    ? "Expired — paste a replacement"
    : `Rejected by ${descriptor.display_name} — paste a replacement`;
  return (
    <div className="sync-storage-reconnect" role="region" aria-label={`Reconnect ${descriptor.display_name}`}>
      {setupSlot && (
        <div className="storage-setup-route-row agent r2-lane" role="note">
          <Icon name={descriptor.local_setup === "dropbox" ? "link" : "terminal"} size={13} />
          <span className="storage-setup-route-label">{descriptor.local_setup === "dropbox" ? "Dropbox sign-in" : "Wrangler setup"}</span>
          {setupSlot}
        </div>
      )}
      {activeSecrets(descriptor, secretPresence).map((secret) => {
        const present = !!secretPresence[secret.secret_id];
        const visible = !!secretVisible[secret.secret_id];
        return (
          <div key={secret.secret_id} className="storage-provider-field">
            <span className="form-label-row">
              <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-secret-${secret.secret_id}`}>{secret.label}</label>
              {(secret.help || secret.help_url) && <HelpMark text={secret.help ?? secret.label} url={secret.help_url} />}
            </span>
            <span className="sync-secret-input">
              <input
                type={visible ? "text" : "password"}
                id={`${fieldIdPrefix}-secret-${secret.secret_id}`}
                value={form.typedSecrets[secret.secret_id] ?? ""}
                onChange={(event) => setSecret(secret.secret_id, event.target.value)}
                placeholder={present && connectionSuspends(connection) ? replacementPlaceholder : (secret.placeholder ?? undefined)}
                disabled={disabled}
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
          </div>
        );
      })}
      <span className="sync-storage-fixed">
        <Icon name="info" size={12} />
        Same destination — only the credential changes.
      </span>
      <div className="sync-storage-reconnect-verbs">{verbs}</div>
    </div>
  );
}

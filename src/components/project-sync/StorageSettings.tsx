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
import Icon, { storageIconName, type IconName } from "../Icons";
import { openExternalUrl } from "./externalLink";
import { compactProjectPath } from "./model";
import { formatRelativeTime } from "../../clock";
import { rowReceipt, useSettingsReceipts } from "./settingsReceipts";
import { writeClipboardText, writeClipboardTextOf } from "../sync-review/clipboard";

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
  // A provider whose only secret is the managed one (Mallard Cloud) has no
  // manual alternative: the lane must produce it before the draft is ready.
  if (descriptor.secrets.every((secret) => secret.secret_id === managedId)) return descriptor.secrets;
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

/* ── Copy ──
   Every value a storage form shows can be copied from its own box, and the
   whole connection at once by Copy all, as `Label: value` lines in the
   form's order. A copy of a value the page holds touches nothing else; a
   saved secret is fetched by that click, never before. */

/** One line Copy all may write: a value the page holds, or a saved secret
 *  the click fetches. A secret the storage does not hold is left out. */
export type StorageCopyLine =
  | { label: string; value: string }
  | { label: string; secretId: string; present: boolean };

export function storageCopyText(lines: readonly { label: string; value: string }[]): string {
  return lines.map((line) => `${line.label}: ${line.value}`).join("\n");
}

/** What a settled storage's card shows that the fields grid draws: the
 *  adapter's facts beyond its typed fields, less a path (the folder field's
 *  own) and what the lead band already states. */
function extraFactsOf(
  descriptor: Pick<StorageProviderDescriptor, "fields">,
  storage: Pick<StorageSummary, "summary">,
  leadFacts: readonly string[],
) {
  return storage.summary.filter((fact) => (
    !fact.is_path
    && !descriptor.fields.some((field) => field.field_id === fact.field_id)
    && !leadFacts.includes(fact.field_id)
  ));
}

/** A settled storage's Copy all, in the card's order: folder, fields,
 *  facts, then the secrets. */
export function lockedCopyLines(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets">,
  storage: Pick<StorageSummary, "settings" | "summary" | "secrets">,
  leadFacts: readonly string[] = [],
): StorageCopyLine[] {
  const value = (field: { label: string; field_id: string }) => ({ label: field.label, value: settingString(storage.settings[field.field_id]) });
  return [
    ...descriptor.fields.filter((field) => field.kind === "directory").map(value),
    ...descriptor.fields.filter((field) => field.kind !== "directory").map(value),
    ...extraFactsOf(descriptor, storage, leadFacts).map((fact) => ({ label: fact.label, value: fact.value })),
    ...descriptor.secrets.map((secret) => ({
      label: secret.label,
      secretId: secret.secret_id,
      present: storage.secrets.some((held) => held.secret_id === secret.secret_id && held.present),
    })),
  ];
}

/** A New-storage form's Copy all: what its boxes hold, typed or filled by a
 *  setup lane. An empty box, or a stored secret the form never saw, is not
 *  a line. */
export function editorCopyLines(
  descriptor: Pick<StorageProviderDescriptor, "fields" | "secrets">,
  form: Pick<StorageEditorForm, "settings" | "typedSecrets">,
): { label: string; value: string }[] {
  return [
    ...descriptor.fields.map((field) => ({ label: field.label, value: form.settings[field.field_id] ?? "" })),
    ...descriptor.secrets.map((secret) => ({ label: secret.label, value: form.typedSecrets[secret.secret_id] ?? "" })),
  ].filter((line) => line.value.trim().length > 0);
}

type CopyReceipt = { ok: boolean; text: string };

/** Copy receipts by key, each standing for COPIED_MS. */
function useCopyReceipts() {
  const [receipts, setReceipts] = useState<Record<string, CopyReceipt>>({});
  const timers = useRef(new Map<string, number>());
  useEffect(() => {
    const standing = timers.current;
    return () => { for (const timer of standing.values()) window.clearTimeout(timer); };
  }, []);
  const settle = (key: string, ok: boolean, text = ok ? "Copied" : "Not copied") => {
    window.clearTimeout(timers.current.get(key));
    setReceipts((current) => ({ ...current, [key]: { ok, text } }));
    timers.current.set(key, window.setTimeout(() => {
      timers.current.delete(key);
      setReceipts((current) => { const { [key]: _gone, ...rest } = current; return rest; });
    }, COPIED_MS));
  };
  /** Writes a value the page holds and says so beside its label. */
  const copyValue = (key: string, value: string) => {
    writeClipboardText(value).then(() => settle(key, true), () => settle(key, false));
  };
  return { receipts, settle, copyValue };
}

/** A box's own Copy, at its right edge. */
function CopyButton({ label, disabled, receipt, onCopy }: {
  label: string;
  disabled: boolean;
  receipt?: CopyReceipt;
  onCopy: () => void;
}) {
  return (
    <button
      type="button"
      className="sync-secret-copy"
      onClick={onCopy}
      disabled={disabled}
      title={`Copy ${label}`}
      aria-label={`Copy ${label}`}
    >
      <Icon name={receipt?.ok ? "check" : "copy"} size={13} />
    </button>
  );
}

/** The receipt in a field's label row. */
function CopyCap({ receipt }: { receipt?: CopyReceipt }) {
  if (!receipt) return null;
  return (
    <span className={`sync-secret-cap ${receipt.ok ? "copied" : "failed"}`} role="status">
      <Icon name={receipt.ok ? "check" : "alert-triangle"} size={11} /> {receipt.text}
    </span>
  );
}

/** Copy all fields: the form's whole connection, as `Label: value` lines.
 *  Named for what it copies so it never reads as the guide beside it. */
function CopyAllButton({ disabled, busy, receipt, onCopy }: {
  disabled: boolean;
  busy: boolean;
  receipt?: CopyReceipt;
  onCopy: () => void;
}) {
  return (
    <button
      type="button"
      className={`btn storage-setup-guide-button storage-copy-all${receipt ? (receipt.ok ? " copied" : " failed") : ""}`}
      disabled={disabled || busy}
      onClick={onCopy}
    >
      <Icon name={receipt ? (receipt.ok ? "check" : "alert-triangle") : "clipboard-list"} size={12} />
      {busy ? "Copying…" : receipt ? receipt.text : "Copy all fields"}
    </button>
  );
}

/** The lane row's glyph and label per direct-setup route. The Dropbox,
 *  Feishu and Mallard Cloud routes are browser sign-ins; Wrangler is a
 *  terminal. */
export function setupRouteIcon(route: StorageProviderDescriptor["local_setup"]): "cloud" | "link" | "terminal" {
  return route === "managed_cloud" ? "cloud" : route === "dropbox" || route === "feishu" ? "link" : "terminal";
}
export function setupRouteLabel(route: StorageProviderDescriptor["local_setup"], verb: "setup" | "sign-in"): string {
  if (route === "managed_cloud") return `Mallard Cloud ${verb}`;
  if (route === "dropbox") return `Dropbox ${verb}`;
  if (route === "feishu") return `Feishu ${verb}`;
  return "Wrangler automatic setup";
}

/** A draft's settings as the naming rule reads them: every value as text,
 *  including the ones the adapter keeps that no field shows, such as a
 *  Dropbox folder's name or a Mallard Cloud login. */
export function settingStrings(settings: StorageSettings): Record<string, string> {
  return Object.fromEntries(Object.entries(settings).map(([key, value]) => [key, settingString(value)]));
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
    return suggested.length > 0 ? rule.template.replace("{1}", suggested) : null;
  } catch {
    return null;
  }
}

/** The smallest free `name`, `name 2`, `name 3`, … among the taken ones. */
export function uniqueStorageName(name: string, taken: readonly string[]): string {
  let candidate = name;
  for (let n = 2; taken.includes(candidate); n += 1) candidate = `${name} ${n}`;
  return candidate;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Whether the name is still Mallard's — empty, the placeholder a new draft
 *  opens with, the stem alone, or the suggestion these settings produce with
 *  or without its uniqueness number — so a later suggestion may replace it.
 *  A name the user typed is never touched. */
export function storageNameIsAuto(
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "default_name">,
  form: Pick<StorageEditorForm, "name" | "settings">,
  settings: Record<string, string> = form.settings,
): boolean {
  const name = form.name.trim();
  if (name.length === 0) return true;
  const stem = descriptor.default_name?.trim();
  if (stem && (name === stem || new RegExp(`^${escapeRegExp(stem)} draft \\d+$`).test(name))) return true;
  const suggested = suggestStorageName(descriptor, settings);
  return suggested !== null && (name === suggested || new RegExp(`^${escapeRegExp(suggested)} \\d+$`).test(name));
}

/** The name for `settings`: the suggestion made unique among `taken`, or
 *  the stem while the source is cleared — unless the user typed a name. */
export function storageNameForSettings(
  descriptor: Pick<StorageProviderDescriptor, "name_suggestion" | "default_name">,
  form: Pick<StorageEditorForm, "name" | "settings">,
  settings: Record<string, string>,
  taken: readonly string[] = [],
): string {
  if (!storageNameIsAuto(descriptor, form)) return form.name;
  const suggested = suggestStorageName(descriptor, settings);
  return suggested ? uniqueStorageName(suggested, taken) : descriptor.default_name ?? form.name;
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

/** One of two ways in, as a radio door: New storage's Create new / Use
 *  existing, and New project's On this computer / From Mallard Cloud. */
export function ModeDoor({ icon, title, detail, help, checked, disabled, onChoose }: {
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
  takenNames = [],
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
  /** Names the configured storages already use: a suggested name that
   *  collides takes the next free number. */
  takenNames?: readonly string[];
}) {
  const fieldIdPrefix = useId();
  const [secretVisible, setSecretVisible] = useState<Record<string, boolean>>({});
  const [manualExpanded, setManualExpanded] = useState(descriptor.local_setup === "wrangler");
  const copies = useCopyReceipts();
  const copyLines = editorCopyLines(descriptor, form);
  const copyAll = () => {
    writeClipboardText(storageCopyText(copyLines)).then(() => copies.settle("all", true), () => copies.settle("all", false));
  };
  const setName = (name: string) => onChange({ ...form, name });
  const setSetting = (fieldId: string, value: string) => {
    const settings = applyFieldDerivations(descriptor, { ...form.settings, [fieldId]: value }, fieldId);
    // The name follows the settings only while it is still Mallard's own;
    // a typed name stays, and the field stays editable either way.
    onChange({ ...form, settings, name: storageNameForSettings(descriptor, form, settings, takenNames) });
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
  /* A new draft on a provider whose lane can reach an existing destination
     is asked Create new or Use existing before it shows a lane. The doors
     are the editor's; what each door leads to is the workspace's. Any
     other provider is past the question. */
  const doors = setupMode && descriptor.setup_doors
    ? { ...setupMode, sentences: descriptor.setup_doors }
    : null;
  const usingExisting = doors?.mode === "existing";
  const showLanes = doors === null || doors.mode === "create";
  /* Direct setup is available for both creation and discovery. */
  const wranglerLane = !!descriptor.local_setup && !managed;
  // Keep review available when a prepared draft is reopened or edited: a
  // stored secret says the lane ran; a provider with no secret says so
  // through its required fields.
  const wranglerReviewReady = setupDone
    || descriptor.secrets.some((secret) => secretPresence[secret.secret_id])
    || (descriptor.secrets.length === 0 && descriptor.fields.length > 0
      && descriptor.fields.every((field) => !field.required || (form.settings[field.field_id] ?? "").trim().length > 0));
  const showFields = wranglerLane
    ? wranglerReviewReady || (manualExpanded && (showLanes || usingExisting))
    : showLanes || (usingExisting && !!manualEntry);
  return (
    <article className="sync-storage-card">
      <header>
        <span className="sync-storage-icon"><Icon name={storageIconName(descriptor.key, descriptor.icon)} size={18} /></span>
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
            detail={doors.sentences.create_hint}
            checked={doors.mode === "create"}
            disabled={disabled}
            onChoose={() => doors.onChoose("create")}
          />
          <ModeDoor
            icon="search"
            title="Use existing"
            detail={doors.sentences.existing_hint}
            help={doors.sentences.existing_help}
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
          <Icon name={setupRouteIcon(descriptor.local_setup)} size={13} />
          <span className="storage-setup-route-label">{setupRouteLabel(descriptor.local_setup, "setup")}</span>
          {setupSlot}
        </div>
      )}
      {usingExisting && discoverySlot}
      {/* Beside a lane, Manual setup is offered only by a provider that can
          be entered by hand; one whose lane is the only way in (a sign-in
          only Mallard can mint) declares no manual hint and gets no row. */}
      {(showLanes || usingExisting && wranglerLane) && !managed && (wranglerLane || !setupDone) && !!(wranglerLane ? descriptor.manual_setup_hint : descriptor.manual_setup_hint || descriptor.setup_guide_url) && (
        <>

          <div className="storage-setup-route-row" role="note">
            <Icon name="info" size={13} />
            <span className="storage-setup-route-label">Manual setup</span>
            <span className="storage-setup-guide-copy">
              {wranglerLane && !showFields
                ? "Use your own connection credentials."
                : descriptor.local_setup === "wrangler" ? null
                : descriptor.manual_setup_hint ?? `Enter an existing ${descriptor.display_name} connection in the fields below.`}
            </span>
            {descriptor.setup_guide_url && (
              <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>
                Setup guide <Icon name="external-link" size={11} />
              </button>
            )}
            {showFields && descriptor.key === "r2" && (
              <CopyAllButton disabled={copyLines.length === 0} busy={false} receipt={copies.receipts.all} onCopy={copyAll} />
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
          {gridFields.map((field) => {
            const value = form.settings[field.field_id] ?? "";
            const receipt = copies.receipts[`field:${field.field_id}`];
            return (
              <div key={field.field_id} className={`storage-provider-field${field.kind === "url" ? " storage-provider-url-field" : ""}`}>
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-field-${field.field_id}`}>{field.label}</label>
                  {(field.help || field.help_url) && (
                    <HelpMark text={field.help ?? field.label} url={field.help_url} />
                  )}
                  <CopyCap receipt={receipt} />
                </span>
                <span className="sync-secret-input has-copy-one">
                  <input
                    type={field.kind === "url" ? "url" : "text"}
                    id={`${fieldIdPrefix}-field-${field.field_id}`}
                    value={value}
                    onChange={(event) => setSetting(field.field_id, event.target.value)}
                    placeholder={field.placeholder ?? undefined}
                    disabled={disabled}
                    required={field.required}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoCorrect="off"
                    autoComplete="off"
                  />
                  <span className="sync-secret-verbs">
                    <CopyButton label={field.label} disabled={!value.trim()} receipt={receipt} onCopy={() => copies.copyValue(`field:${field.field_id}`, value)} />
                  </span>
                </span>
              </div>
            );
          })}
          {activeSecrets(descriptor, secretPresence).map((secret) => {
            const present = !!secretPresence[secret.secret_id];
            const visible = !!secretVisible[secret.secret_id];
            const value = form.typedSecrets[secret.secret_id] ?? "";
            const receipt = copies.receipts[`secret:${secret.secret_id}`];
            return (
              <div key={secret.secret_id} className="storage-provider-field">
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-secret-${secret.secret_id}`}>{secret.label}</label>
                  {(secret.help || secret.help_url) && (
                    <HelpMark text={secret.help ?? secret.label} url={secret.help_url} />
                  )}
                  <CopyCap receipt={receipt} />
                </span>
                {/* Backend updates return presence only. A value known by the
                    open editor remains here for stable masking and review,
                    and Copy takes it; otherwise the field accepts a
                    replacement and there is nothing to copy. */}
                <span className="sync-secret-input has-copy">
                  <input
                    type={visible ? "text" : "password"}
                    id={`${fieldIdPrefix}-secret-${secret.secret_id}`}
                    value={value}
                    onChange={(event) => setSecret(secret.secret_id, event.target.value)}
                    placeholder={present ? "Stored — type to replace" : (secret.placeholder ?? undefined)}
                    disabled={disabled}
                    required={secret.required && !present}
                    autoComplete="new-password"
                  />
                  <span className="sync-secret-verbs">
                    {!(descriptor.key === "dropbox" && secret.secret_id === "refresh_token") && (
                      <CopyButton label={secret.label} disabled={!value} receipt={receipt} onCopy={() => copies.copyValue(`secret:${secret.secret_id}`, value)} />
                    )}
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
 * seats here as `reconnect.band`. A provider whose facts deserve more than a
 * read-only field gets a `lead` band from the workspace, under those two.
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
  lead,
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
  /** The provider's own band, and the summary facts it already states: the
   *  fields grid leaves those out rather than saying them twice. */
  lead?: {
    band: ReactNode;
    facts: readonly string[];
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
  const copies = useCopyReceipts();
  const [copyingAll, setCopyingAll] = useState(false);
  const [now, setNow] = useState(() => Date.now());
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
  /* The clipboard write starts inside the click and takes the secret as it
     arrives (see writeClipboardTextOf). */
  const copy = async (secretId: string) => {
    if (revealing[secretId]) return;
    setRevealing((current) => ({ ...current, [secretId]: true }));
    setRevealError((current) => { const { [secretId]: _gone, ...rest } = current; return rest; });
    const shown = revealed[secretId];
    try {
      await writeClipboardTextOf(shown ? Promise.resolve(shown.value) : onRevealSecret(secretId));
      copies.settle(`secret:${secretId}`, true);
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
  /* What the adapter reports about the storage beyond its typed fields: a
     folder's name, the signed-in account, the date a sign-in runs out.
     Read-only facts in the fields' grammar; a path fact is the folder
     field's own, and a fact the lead band states is the band's. */
  const extraFacts = extraFactsOf(descriptor, storage, lead?.facts ?? []);
  /* Copy all fetches each saved secret in turn on the click, and writes
     nothing unless every fetch landed. The write starts inside the click
     with the text still pending. */
  const copyAll = async () => {
    if (copyingAll) return;
    setCopyingAll(true);
    let missing = 0;
    const text = (async () => {
      const lines: { label: string; value: string }[] = [];
      for (const line of lockedCopyLines(descriptor, storage, lead?.facts ?? [])) {
        if (!("secretId" in line)) {
          if (line.value) lines.push(line);
        } else if (!line.present) {
          missing += 1;
        } else {
          lines.push({ label: line.label, value: revealed[line.secretId]?.value ?? await onRevealSecret(line.secretId) });
        }
      }
      return storageCopyText(lines);
    })();
    try {
      await writeClipboardTextOf(text);
      copies.settle("all", true, missing ? `Copied · ${missing} not stored` : "Copied");
    } catch {
      copies.settle("all", false);
    } finally {
      setCopyingAll(false);
    }
  };
  /** A read-only box with its own Copy at the right edge. */
  const valueBox = (key: string, label: string, value: string, input: ReactNode) => (
    <span className="sync-secret-input has-copy-one">
      {input}
      <span className="sync-secret-verbs">
        <CopyButton label={label} disabled={!value} receipt={copies.receipts[key]} onCopy={() => copies.copyValue(key, value)} />
      </span>
    </span>
  );
  const managed = hasManagedCredentials(descriptor, presence);
  const suspended = connectionSuspends(storage.connection);
  /* "Connected" is said only after this visit saw a probe succeed; a stored
     credential proves nothing, and a refused one says so in the row. */
  const guideCopy = verifiedAt !== null
    ? `Connected · verified ${formatRelativeTime(Math.floor(verifiedAt / 1000))}.${managed ? " Access renews automatically." : ""}`
    : `Not tested yet.${managed ? " Access renews automatically." : ""}`;
  const showGuide = suspended || descriptor.secrets.length > 0 || !!descriptor.local_setup;

  return (
    <article className="sync-storage-card sync-storage-locked">
      <header>
        <span className="sync-storage-icon"><Icon name={storageIconName(descriptor.key, descriptor.icon)} size={18} /></span>
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
            <button type="button" className="btn-link" onClick={() => void openExternalUrl(descriptor.setup_guide_url!)}>
              Setup guide <Icon name="external-link" size={11} />
            </button>
          )}
          {descriptor.key === "r2" && (
            <CopyAllButton disabled={disabled} busy={copyingAll} receipt={copies.receipts.all} onCopy={() => void copyAll()} />
          )}
          {suspended && !reconnect.open && (
            <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={reconnect.onOpen}>
              <Icon name="refresh" size={12} /> Reconnect…
            </button>
          )}
        </div>
      )}
      {reconnect.open && reconnect.band}
      {lead?.band}

      {folderFields.map((field) => {
        const key = `field:${field.field_id}`;
        const inputId = `${fieldIdPrefix}-field-${field.field_id}`;
        return (
          <div key={field.field_id} className="sync-folder-field compact">
            <span className="form-label-row">
              <label className="sync-field-name" htmlFor={inputId}>{field.label}</label>
              {(field.help || field.help_url) && <HelpMark text={field.help ?? field.label} url={field.help_url} />}
              <CopyCap receipt={copies.receipts[key]} />
            </span>
            {valueBox(key, field.label, settingString(field.field_id), (
              <input type="text" id={inputId} value={settingString(field.field_id)} readOnly spellCheck={false} />
            ))}
          </div>
        );
      })}

      {(gridFields.length > 0 || extraFacts.length > 0 || descriptor.secrets.length > 0) && (
        <div className="sync-storage-fields storage-provider-fields" aria-label={`${descriptor.display_name} connection details`}>
          {gridFields.map((field) => {
            const key = `field:${field.field_id}`;
            const value = settingString(field.field_id);
            return (
              <div key={field.field_id} className={`storage-provider-field${field.kind === "url" ? " storage-provider-url-field" : ""}`}>
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-field-${field.field_id}`}>{field.label}</label>
                  {(field.help || field.help_url) && <HelpMark text={field.help ?? field.label} url={field.help_url} />}
                  <CopyCap receipt={copies.receipts[key]} />
                </span>
                {valueBox(key, field.label, value, (
                  <input
                    type={field.kind === "url" ? "url" : "text"}
                    id={`${fieldIdPrefix}-field-${field.field_id}`}
                    value={value}
                    readOnly
                    spellCheck={false}
                  />
                ))}
              </div>
            );
          })}
          {extraFacts.map((fact) => {
            const key = `fact:${fact.field_id}`;
            return (
              <div key={fact.field_id} className="storage-provider-field storage-provider-fact">
                <span className="form-label-row">
                  <label className="sync-field-name" htmlFor={`${fieldIdPrefix}-fact-${fact.field_id}`}>{fact.label}</label>
                  <CopyCap receipt={copies.receipts[key]} />
                </span>
                {valueBox(key, fact.label, fact.value, (
                  <input type="text" id={`${fieldIdPrefix}-fact-${fact.field_id}`} value={fact.value} readOnly spellCheck={false} />
                ))}
              </div>
            );
          })}
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
                  {(secret.help || secret.help_url) && <HelpMark text={secret.help ?? secret.label} url={secret.help_url} />}
                  {suspended && present && (
                    <span className="sync-storage-refused-chip"><Icon name="alert-triangle" size={10} /> Refused</span>
                  )}
                  {copies.receipts[`secret:${secret.secret_id}`]
                    ? <CopyCap receipt={copies.receipts[`secret:${secret.secret_id}`]} />
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
                      {!(descriptor.key === "dropbox" && secret.secret_id === "refresh_token") && (
                        <CopyButton
                          label={secret.label}
                          disabled={disabled || busy}
                          receipt={copies.receipts[`secret:${secret.secret_id}`]}
                          onCopy={() => void copy(secret.secret_id)}
                        />
                      )}
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
                {error && <small className="sync-storage-field-help error" role="alert"><Icon name="alert-triangle" size={11} /> {error}</small>}
              </div>
            );
          })}
        </div>
      )}

      <footer className="sync-storage-editor-footer">
        <span className="sync-storage-fixed">
          <Icon name="lock" size={12} />
          Fixed after setup. To sync elsewhere, add a new storage.
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
          <Icon name={setupRouteIcon(descriptor.local_setup)} size={13} />
          <span className="storage-setup-route-label">{setupRouteLabel(descriptor.local_setup, "sign-in")}</span>
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

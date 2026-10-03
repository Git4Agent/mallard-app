import { StrictMode, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { storageIconName } from "../src/components/Icons";
import { R2SetupLane, idleR2State, type R2SetupFlow } from "../src/components/project-sync/R2Setup";
import {
  ModeDoor,
  StorageEditor,
  StorageLockedForm,
  applyFieldDerivations,
  setupRouteIcon,
  setupRouteLabel,
  type StorageEditorForm,
} from "../src/components/project-sync/StorageSettings";
import type { StorageProviderDescriptor, StorageSummary } from "../src/types";
import "../src/App.css";

/**
 * Owner picked A on 2026-10-03 with the ? marks and "Example:" kept, and
 * it shipped as "Copy all fields" with a clipboard-list mark, so Current
 * now draws A itself; the proposal column is the record.
 *
 * Mockup: the settled storage's card (R2, Dropbox, Feishu), with fewer words
 * and a Copy on every value plus one Copy all.
 *
 *   Current  the real StorageLockedForm, as it ships today
 *   A        Quiet form: the same read-only boxes; the five help lines become
 *            the `?` marks the New-storage form already shows; no lock per
 *            label; Copy inside every box; Copy all in the status row
 *   B        Details list: label · value · verbs rows, no boxes (the facts
 *            sheet passed on 2026-09-12, shown again for comparison)
 *
 * Both keep the footer as approved ("Fixed after setup. To sync elsewhere,
 * add a new storage."), the field order, and the S3 endpoint row. The card
 * is shared by every provider; the mock shows R2, Dropbox and Feishu.
 *
 * Page "Manual setup" applies A's grammar to the New-storage R2 form: the
 * real StorageEditor + R2 lane as Current, beside a rebuilt proposal with
 * Copy in every box, Copy all on the Manual setup row, and placeholders
 * without the "Example:" prefix.
 *
 * `npm run dev`, then /mockups/r2-details-mockup.html
 * (`?page=card|setup&dir=a|b&status=untested|verified&fill=empty|filled&theme=light|dark&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "a" | "b";
type Status = "untested" | "verified";
type Page = "card" | "setup";
type Fill = "empty" | "filled";
interface Knobs { page: Page; dir: Dir; status: Status; fill: Fill; theme: "light" | "dark"; shot: "on" | "off" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = query.get(key);
    return allowed.includes(value as T) ? (value as T) : fallback;
  };
  return {
    page: pick("page", ["card", "setup"] as const, "card"),
    dir: pick("dir", ["a", "b"] as const, "a"),
    fill: pick("fill", ["empty", "filled"] as const, "filled"),
    status: pick("status", ["untested", "verified"] as const, "untested"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    shot: pick("shot", ["on", "off"] as const, "off"),
  };
}

/* ——— the R2 descriptor and a settled storage, as the backend sends them ——— */

// crates/mallard-infrastructure/src/storage_providers/r2.rs
const R2_SETUP_DOC = "https://developers.cloudflare.com/r2/get-started/s3/";
const R2_CREDENTIALS_DOC = "https://developers.cloudflare.com/r2/api/tokens/";
const R2_API_URL_PATTERN = "^https://([0-9a-fA-F]{32})(?:\\.(eu|us|fedramp))?\\.r2\\.cloudflarestorage\\.com/([^/?#%]+)$";
const R2: StorageProviderDescriptor = {
  key: "r2", config_version: 1, display_name: "Cloudflare R2", description: "Store snapshots in your R2 bucket.",
  display_rank: 10, icon: "cloud", clear_mode: "unconditional", local_setup: "wrangler",
  setup_guide_url: R2_SETUP_DOC, manual_setup_hint: "Enter an existing R2 connection in the fields below.",
  default_name: "R2 storage",
  fields: [
    { field_id: "api_url", label: "S3 API URL", kind: "url", required: true, placeholder: "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent", help: "How to find the Cloudflare R2 S3 API URL", help_url: R2_SETUP_DOC },
    { field_id: "bucket", label: "Bucket name", kind: "text", required: true, placeholder: "Example: codex-sync-backups", help: "How to create a Cloudflare R2 bucket", help_url: "https://developers.cloudflare.com/r2/buckets/create-buckets/", derive: { source: "api_url", pattern: R2_API_URL_PATTERN, group: 3 } },
    { field_id: "account_id", label: "Account ID", kind: "text", required: true, placeholder: "Example: 023e105f4ecef8ad9ca31a8372d0c353", help: "Where to find your Cloudflare Account ID", help_url: "https://developers.cloudflare.com/fundamentals/account/find-account-and-zone-ids/", derive: { source: "api_url", pattern: R2_API_URL_PATTERN, group: 1 } },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true, placeholder: "Example: 4a2b…91ef", help: "How to create an R2 Access Key ID", help_url: R2_CREDENTIALS_DOC },
  ],
  secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, placeholder: "Example: a1B2…x9Y0", help: "How to create an R2 Secret Access Key", help_url: R2_CREDENTIALS_DOC }],
  managed_secret_id: null,
  setup_doors: {
    create_hint: "A new bucket in your Cloudflare account.",
    existing_hint: "A bucket Mallard already uses.",
    existing_help: { label: "Set up team access", url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/" },
  },
};

// Made-up values.
const ACCOUNT = "023e105f4ecef8ad9ca31a8372d0c353";
const ENDPOINT = `https://${ACCOUNT}.r2.cloudflarestorage.com`;
const SECRET = "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0u1V2w3X4";
const STORAGE: StorageSummary = {
  storage_id: "storage-r2", name: "R2 mallard-open", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true,
  // r2.rs `summary()`: Bucket, Account ID, S3 endpoint, Access Key ID.
  summary: [
    { field_id: "bucket", label: "Bucket", value: "mallard-open", is_path: false },
    { field_id: "account_id", label: "Account ID", value: ACCOUNT, is_path: false },
    { field_id: "endpoint", label: "S3 endpoint", value: ENDPOINT, is_path: false },
    { field_id: "access_key_id", label: "Access Key ID", value: "4a2b9c1d7e8f0a3b5c6d7e8f9a0b1c2d", is_path: false },
  ],
  settings: { api_url: `${ENDPOINT}/mallard-open`, bucket: "mallard-open", account_id: ACCOUNT, access_key_id: "4a2b9c1d7e8f0a3b5c6d7e8f9a0b1c2d" },
  secrets: [{ secret_id: "secret_access_key", present: true }],
};
// crates/mallard-infrastructure/src/storage_providers/dropbox.rs
const DROPBOX_GUIDE = "https://www.dropbox.com/developers/reference/auth-types";
const DROPBOX: StorageProviderDescriptor = {
  key: "dropbox", config_version: 1, display_name: "Dropbox", description: "A Dropbox folder. Up to 150 MiB per object.",
  display_rank: 20, icon: "cloud", clear_mode: "revision_checked", local_setup: "dropbox", setup_guide_url: DROPBOX_GUIDE,
  fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, help: "Use the same namespace ID on each machine. This value cannot change after setup.", help_url: DROPBOX_GUIDE }],
  secrets: [
    { secret_id: "access_token", label: "Access token", required: true, help: "For manual setup, supply account_info.read, files.metadata.read, files.content.read, and files.content.write. Replace this token when it expires. Browser setup renews access automatically.", help_url: DROPBOX_GUIDE },
    { secret_id: "refresh_token", label: "Refresh token", required: true, help: "Browser setup saves this token to renew access automatically. Show it to review the value, or enter a replacement issued for this Mallard app and Dropbox account.", help_url: DROPBOX_GUIDE },
  ],
  managed_secret_id: "refresh_token",
};
const DROPBOX_STORAGE: StorageSummary = {
  storage_id: "storage-dbx", name: "Dropbox Mallard", provider: "dropbox", config_version: 1, provider_label: "Dropbox", icon: "cloud", available: true,
  summary: [
    { field_id: "folder_name", label: "Folder", value: "Mallard", is_path: false },
    { field_id: "namespace_id", label: "Namespace ID", value: "4823017", is_path: false },
  ],
  settings: { namespace_id: "4823017" },
  secrets: [{ secret_id: "access_token", present: true }, { secret_id: "refresh_token", present: true }],
};
// crates/mallard-infrastructure/src/storage_providers/feishu.rs
const FEISHU_GUIDE = "https://github.com/Git4Agent/mallard-app/blob/main/docs/FEISHU_APP_SETUP_GUIDE.md";
const FEISHU: StorageProviderDescriptor = {
  key: "feishu", config_version: 1, display_name: "Feishu", description: "", display_rank: 25, icon: "cloud",
  clear_mode: "unavailable", local_setup: "feishu", setup_guide_url: FEISHU_GUIDE,
  fields: [{ field_id: "folder_token", label: "Folder token", kind: "text", required: true, help: "The Drive folder this storage lives in. Fixed after setup; share it in Feishu with each user who connects.", help_url: FEISHU_GUIDE }],
  secrets: [],
  managed_secret_id: null,
};
const FEISHU_STORAGE: StorageSummary = {
  storage_id: "storage-feishu", name: "Feishu Team", provider: "feishu", config_version: 1, provider_label: "Feishu", icon: "cloud", available: true,
  // feishu.rs `summary()`: Folder, Folder token, Account, Sign in again by.
  summary: [
    { field_id: "folder_name", label: "Folder", value: "Team", is_path: false },
    { field_id: "folder_token", label: "Folder token", value: "fldcnQx8Yz2Lm4Np6Rt", is_path: false },
    { field_id: "account_name", label: "Account", value: "Mallard bot", is_path: false },
    { field_id: "reauthorize_by", label: "Sign in again by", value: "2026-12-30", is_path: false },
  ],
  settings: { folder_token: "fldcnQx8Yz2Lm4Np6Rt" },
  secrets: [],
};
const PROVIDERS = [[R2, STORAGE], [DROPBOX, DROPBOX_STORAGE], [FEISHU, FEISHU_STORAGE]] as const;

const SECRETS: Record<string, string> = {
  secret_access_key: SECRET,
  access_token: "sl.u.AFyQ3nR8kZt0b2LxPq9WvJm7Hc4YeTd1oSaG6NiUf5XrBwK2lM8pDzE0vC3hQ7yJ",
  refresh_token: "rt_9XkQ2mPz7LbT4vWn8HcJ3sYd6FgR1aE5uKoI0NxZ",
};

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
/** Stand-in for `reveal_storage_secret`: fetched per click, logged as "Secret revealed". */
const revealSecret = async (secretId: string) => { await wait(220); return SECRETS[secretId] ?? ""; };

/* ——— what the card shows, in card order ————————————————————————————— */

interface Row {
  id: string;
  label: string;
  /** Plain values; null for a secret, which is fetched on Copy or Show. */
  value: string | null;
  secret: boolean;
  wide: boolean;
  help?: string | null;
  helpUrl?: string | null;
}

/** Declared fields, then the adapter's extra facts, then secrets: the order
 *  the card draws today. */
function cardRows(descriptor: StorageProviderDescriptor, storage: StorageSummary): Row[] {
  const setting = (id: string) => {
    const value = storage.settings[id];
    return typeof value === "string" ? value : value == null ? "" : String(value);
  };
  return [
    ...descriptor.fields.map((field): Row => ({ id: field.field_id, label: field.label, value: setting(field.field_id), secret: false, wide: field.kind === "url", help: field.help, helpUrl: field.help_url })),
    ...storage.summary
      .filter((fact) => !fact.is_path && !descriptor.fields.some((field) => field.field_id === fact.field_id))
      .map((fact): Row => ({ id: fact.field_id, label: fact.label, value: fact.value, secret: false, wide: false })),
    ...descriptor.secrets.map((secret): Row => ({ id: secret.secret_id, label: secret.label, value: null, secret: true, wide: false, help: secret.help, helpUrl: secret.help_url })),
  ];
}

/* ——— the proposals —————————————————————————————————————————————————— */

const COPIED_MS = 1400;

function HelpMark({ row }: { row: Row }) {
  if (!row.help && !row.helpUrl) return null;
  const text = row.help ?? row.label;
  return (
    <button type="button" className="field-help" title={text} aria-label={text} onClick={row.helpUrl ? () => window.open(row.helpUrl!, "_blank") : undefined}>?</button>
  );
}

function ProposedCard({ descriptor, storage, dir, status, onClipboard }: {
  descriptor: StorageProviderDescriptor;
  storage: StorageSummary;
  dir: Dir;
  status: Status;
  onClipboard: (text: string, what: string) => void;
}) {
  const rows = cardRows(descriptor, storage);
  const managed = !!descriptor.managed_secret_id;
  const [shownById, setShownById] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const flash = (id: string) => {
    setCopied(id);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(null), COPIED_MS);
  };
  const valueOf = async (row: Row) => row.secret ? (shownById[row.id] ?? await revealSecret(row.id)) : row.value ?? "";
  const copyOne = async (row: Row) => {
    setBusy(row.id);
    const value = await valueOf(row);
    setBusy(null);
    onClipboard(value, `${descriptor.display_name} · ${row.label}`);
    flash(row.id);
  };
  const copyAll = async () => {
    setBusy("all");
    const lines: string[] = [];
    for (const row of rows) lines.push(`${row.label}: ${await valueOf(row)}`);
    setBusy(null);
    onClipboard(lines.join("\n"), `${descriptor.display_name} · Copy all`);
    flash("all");
  };
  const toggleShow = async (row: Row) => {
    if (shownById[row.id]) { setShownById(({ [row.id]: _gone, ...rest }) => rest); return; }
    setBusy(row.id);
    const value = await revealSecret(row.id);
    setShownById((current) => ({ ...current, [row.id]: value }));
    setBusy(null);
  };
  const cap = (row: Row) => copied === row.id
    ? <span className="sync-secret-cap copied" role="status"><Icon name="check" size={11} /> Copied</span>
    : busy === row.id ? <span className="sync-secret-cap"><span className="status-loader" /> Reading…</span> : null;

  const statusRow = (
    <div className="storage-setup-guide" role="note">
      <Icon name={status === "verified" ? "check-circle" : "info"} size={13} />
      <span className="storage-setup-guide-copy">{status === "verified" ? "Connected · verified 3m ago." : "Not tested yet."}{managed ? " Access renews automatically." : ""}</span>
      <button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>
      <button type="button" className={`btn storage-setup-guide-button storage-copy-all${copied === "all" ? " copied" : ""}`} disabled={busy === "all"} onClick={() => void copyAll()}>
        <Icon name={copied === "all" ? "check" : "copy"} size={12} />
        {busy === "all" ? "Copying…" : copied === "all" ? "Copied" : "Copy all"}
      </button>
    </div>
  );

  const body = dir === "a" ? (
    <div className="sync-storage-fields storage-provider-fields" aria-label={`${descriptor.display_name} connection details`}>
      {rows.map((row) => {
        const shown = shownById[row.id];
        return (
        <div key={row.id} className={`storage-provider-field${row.wide ? " storage-provider-url-field" : ""}`}>
          <span className="form-label-row">
            <label className="sync-field-name" htmlFor={`mk-${descriptor.key}-${row.id}`}>{row.label}</label>
            <HelpMark row={row} />
            {cap(row)}
          </span>
          <span className={`sync-secret-input ${row.secret ? "has-copy" : "has-copy-one"}`}>
            <input
              type={row.secret && !shown ? "password" : "text"}
              id={`mk-${descriptor.key}-${row.id}`}
              value={row.secret ? shown ?? "••••••••••••••••••••••••" : row.value ?? ""}
              readOnly
              spellCheck={false}
            />
            <span className="sync-secret-verbs">
              <button type="button" className="sync-secret-copy" title={`Copy ${row.label}`} aria-label={`Copy ${row.label}`} onClick={() => void copyOne(row)}>
                <Icon name={copied === row.id ? "check" : "copy"} size={13} />
              </button>
              {row.secret && (
                <button type="button" className="sync-secret-reveal" aria-pressed={!!shown} onClick={() => void toggleShow(row)}>{shown ? "Hide" : "Show"}</button>
              )}
            </span>
          </span>
        </div>
        );
      })}
    </div>
  ) : (
    <dl className="storage-details-list" aria-label={`${descriptor.display_name} connection details`}>
      {rows.map((row) => {
        const shown = shownById[row.id];
        return (
        <div key={row.id} className="storage-details-row">
          <dt>{row.label}<HelpMark row={row} /></dt>
          <dd title={row.secret ? undefined : row.value ?? undefined}>{row.secret ? shown ?? "••••••••••••••••••••••••" : row.value}</dd>
          <span className="storage-details-verbs">
            {row.secret && <button type="button" aria-pressed={!!shown} onClick={() => void toggleShow(row)}>{busy === row.id ? "…" : shown ? "Hide" : "Show"}</button>}
            <button type="button" className={copied === row.id ? "copied" : undefined} title={`Copy ${row.label}`} aria-label={`Copy ${row.label}`} onClick={() => void copyOne(row)}>
              <Icon name={copied === row.id ? "check" : "copy"} size={13} />
            </button>
          </span>
        </div>
        );
      })}
    </dl>
  );

  return (
    <article className="sync-storage-card sync-storage-locked">
      <header>
        <span className="sync-storage-icon"><Icon name={storageIconName(descriptor.key, descriptor.icon)} size={18} /></span>
        <form className="sync-storage-name" onSubmit={(event) => event.preventDefault()}>
          <input defaultValue={storage.name} aria-label="Storage name" placeholder="Storage name" />
        </form>
        <span className="sync-storage-editor-provider">{descriptor.display_name}</span>
      </header>
      {statusRow}
      {body}
      <footer className="sync-storage-editor-footer">
        <span className="sync-storage-fixed">
          <Icon name="lock" size={12} />
          Fixed after setup. To sync elsewhere, add a new storage.
        </span>
        {status === "verified" && (
          <span className="storage-setup-verified" role="status"><Icon name="check-circle" size={13} /> Connection verified</span>
        )}
        <button type="button" className="btn">Test connection</button>
      </footer>
    </article>
  );
}

/* ——— the New-storage R2 form (manual setup) ————————————————————————— */

const noop = () => undefined;
const SETUP_PROVIDERS = [R2, DROPBOX, FEISHU];
const SECRET_ID = "secret_access_key";

/** Empty: typing by hand. Filled: the lane prepared the draft, so the form
 *  holds every value, the secret included (a revealed draft secret). */
function setupForm(fill: Fill): StorageEditorForm {
  return fill === "filled"
    ? { name: STORAGE.name, settings: { ...STORAGE.settings } as Record<string, string>, typedSecrets: { [SECRET_ID]: SECRET } }
    : { name: "R2 storage", settings: { api_url: "", bucket: "", account_id: "", access_key_id: "" }, typedSecrets: {} };
}
function laneFlow(fill: Fill): R2SetupFlow {
  const state = fill === "filled"
    ? { ...idleR2State(), phase: "review" as const, account: { id: ACCOUNT, name: "Personal" }, bucket: "mallard-open", name: "mallard-open" }
    : idleR2State();
  return {
    state, isRunning: () => false, start: noop, changeLogin: noop, continueWith: noop, setJurisdiction: noop, setName: noop,
    suggestAnother: noop, createAndAuthorize: noop, useBucket: noop, cancel: async () => undefined, retry: noop, scanAgain: noop, dismiss: noop,
  };
}
const lane = (fill: Fill) => (
  <R2SetupLane mode="create" flow={laneFlow(fill)} disabled={false} descriptor={R2} storages={[]} onOpenStorage={noop} onCreateInstead={noop} onReset={noop} />
);

function SetupCurrent({ fill }: { fill: Fill }) {
  const [form, setForm] = useState(() => setupForm(fill));
  return (
    <StorageEditor
      descriptor={R2}
      providers={SETUP_PROVIDERS}
      form={form}
      secretPresence={fill === "filled" ? { [SECRET_ID]: true } : {}}
      disabled={false}
      onChange={setForm}
      onSwitchProvider={noop}
      setupSlot={lane(fill)}
      setupMode={{ mode: "create", onChoose: noop }}
      setupDone={fill === "filled"}
    />
  );
}

function SetupProposed({ fill, onClipboard }: { fill: Fill; onClipboard: (text: string, what: string) => void }) {
  const [form, setForm] = useState(() => setupForm(fill));
  const [visible, setVisible] = useState(false);
  const [expanded, setExpanded] = useState(true);
  const [copied, setCopied] = useState<string | null>(null);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const flash = (id: string) => {
    setCopied(id);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(null), COPIED_MS);
  };
  const rows: (Row & { placeholder: string; text: string })[] = [
    ...R2.fields.map((field) => ({
      id: field.field_id, label: field.label, value: null, secret: false, wide: field.kind === "url", help: field.help, helpUrl: field.help_url,
      placeholder: (field.placeholder ?? "").replace(/^Example:\s*/, ""), text: form.settings[field.field_id] ?? "",
    })),
    ...R2.secrets.map((secret) => ({
      id: secret.secret_id, label: secret.label, value: null, secret: true, wide: false, help: secret.help, helpUrl: secret.help_url,
      placeholder: (secret.placeholder ?? "").replace(/^Example:\s*/, ""), text: form.typedSecrets[secret.secret_id] ?? "",
    })),
  ];
  const filled = rows.filter((row) => row.text.trim());
  const copyOne = (row: (typeof rows)[number]) => { onClipboard(row.text, `Manual setup · ${row.label}`); flash(row.id); };
  const copyAll = () => {
    onClipboard(filled.map((row) => `${row.label}: ${row.text}`).join("\n"), "Manual setup · Copy all");
    flash("all");
  };
  const edit = (row: (typeof rows)[number], value: string) => setForm(row.secret
    ? { ...form, typedSecrets: { ...form.typedSecrets, [row.id]: value } }
    : { ...form, settings: applyFieldDerivations(R2, { ...form.settings, [row.id]: value }, row.id) });
  return (
    <article className="sync-storage-card">
      <header>
        <span className="sync-storage-icon"><Icon name={storageIconName(R2.key, R2.icon)} size={18} /></span>
        <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} aria-label="Storage name" placeholder="Storage name" />
        <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
          {SETUP_PROVIDERS.map((provider) => (
            <button key={provider.key} type="button" role="radio" aria-checked={provider.key === "r2"} className={provider.key === "r2" ? "active" : undefined}>{provider.display_name}</button>
          ))}
        </div>
      </header>
      <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing storage">
        <ModeDoor icon="plus" title="Create new" detail={R2.setup_doors!.create_hint} checked disabled={false} onChoose={noop} />
        <ModeDoor icon="search" title="Use existing" detail={R2.setup_doors!.existing_hint} help={R2.setup_doors!.existing_help} checked={false} disabled={false} onChoose={noop} />
      </div>
      <div className="storage-setup-route-row agent r2-lane" role="note">
        <Icon name={setupRouteIcon(R2.local_setup)} size={13} />
        <span className="storage-setup-route-label">{setupRouteLabel(R2.local_setup, "setup")}</span>
        {lane(fill)}
      </div>
      <div className="storage-setup-route-row" role="note">
        <Icon name="info" size={13} />
        <span className="storage-setup-route-label">Manual setup</span>
        <span className="storage-setup-guide-copy" />
        <button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>
        <button type="button" className={`btn storage-setup-guide-button storage-copy-all${copied === "all" ? " copied" : ""}`} disabled={filled.length === 0} onClick={copyAll}>
          <Icon name={copied === "all" ? "check" : "copy"} size={12} />
          {copied === "all" ? "Copied" : "Copy all"}
        </button>
        {fill === "empty" && (
          <button type="button" className="btn storage-manual-toggle" aria-expanded={expanded} aria-label={expanded ? "Hide fields" : "Enter by hand"} onClick={() => setExpanded(!expanded)}>
            <Icon name={expanded ? "chevron-up" : "chevron-down"} size={12} />
          </button>
        )}
      </div>
      {expanded && (
        <div className="sync-storage-fields storage-provider-fields" aria-label="Cloudflare R2 connection details">
          {rows.map((row) => (
            <div key={row.id} className={`storage-provider-field${row.wide ? " storage-provider-url-field" : ""}`}>
              <span className="form-label-row">
                <label className="sync-field-name" htmlFor={`mk-setup-${row.id}`}>{row.label}</label>
                <HelpMark row={row} />
                {copied === row.id && <span className="sync-secret-cap copied" role="status"><Icon name="check" size={11} /> Copied</span>}
              </span>
              <span className={`sync-secret-input ${row.secret ? "has-copy" : "has-copy-one"}`}>
                <input
                  type={row.secret && !visible ? "password" : row.wide ? "url" : "text"}
                  id={`mk-setup-${row.id}`}
                  value={row.text}
                  onChange={(event) => edit(row, event.target.value)}
                  placeholder={row.placeholder}
                  spellCheck={false}
                  autoComplete={row.secret ? "new-password" : "off"}
                />
                <span className="sync-secret-verbs">
                  <button type="button" className="sync-secret-copy" title={`Copy ${row.label}`} aria-label={`Copy ${row.label}`} disabled={!row.text} onClick={() => copyOne(row)}>
                    <Icon name={copied === row.id ? "check" : "copy"} size={13} />
                  </button>
                  {row.secret && <button type="button" className="sync-secret-reveal" aria-pressed={visible} onClick={() => setVisible(!visible)}>{visible ? "Hide" : "Show"}</button>}
                </span>
              </span>
            </div>
          ))}
        </div>
      )}
    </article>
  );
}

/* ——— page ————————————————————————————————————————————————————————— */

const DIRECTIONS: Record<Dir, { title: string; tag: string; note: string }> = {
  a: {
    title: "A · Quiet form",
    tag: "recommended",
    note: "The boxes you picked in September stay. Every help line becomes a ? mark beside its label (same mark, same doc as the New-storage form), which also tucks Dropbox's and Feishu's long sentences into the tooltip. The per-label locks go; the footer already says it. Every box gets Copy at its right edge.",
  },
  b: {
    title: "B · Details list",
    tag: "passed on 2026-09-12",
    note: "Label, value and verbs per row, no boxes. The quietest, but it is the facts sheet you passed on when the locked form was chosen; here for comparison.",
  },
};

function App() {
  const [knobs, setKnobs] = useState(readKnobs);
  const [clip, setClip] = useState<{ text: string; what: string } | null>(null);
  const [measure, setMeasure] = useState("");
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams({ ...next })}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string): ReactNode => (
    <button type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const onClipboard = (text: string, what: string) => {
    setClip({ text, what });
    void navigator.clipboard?.writeText(text).catch(() => undefined);
  };
  const proposalRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = proposalRef.current;
    const copy = root?.querySelector<HTMLElement>(".sync-secret-copy, .storage-details-verbs > button");
    const all = root?.querySelector<HTMLElement>(".storage-copy-all");
    if (!copy || !all) return;
    const c = getComputedStyle(copy);
    const a = getComputedStyle(all);
    setMeasure(`copy ${Math.round(copy.getBoundingClientRect().width)}×${Math.round(copy.getBoundingClientRect().height)} ${c.color} · copy all ${a.fontSize} ${a.color} ${a.backgroundColor}`);
  }, [knobs]);
  const direction = DIRECTIONS[knobs.dir];
  if (knobs.page === "setup") return (
    <div className={`spec${knobs.shot === "on" ? " spec-shot" : ""}`}>
      <div className="spec-title">New R2 storage, manual setup — copy everything</div>
      <p className="spec-sub">
        The New-storage form for Cloudflare R2. A’s grammar carried over: Copy in every box (the secret too), Copy all on the
        Manual setup row, and shorter placeholders. Type in the proposal’s S3 API URL: bucket and account still fill themselves.
      </p>
      <div className="spec-controls">
        <div className="spec-group"><span>Page</span>{seg("page", "card", "Storage card")}{seg("page", "setup", "Manual setup")}</div>
        <div className="spec-group"><span>Fields</span>{seg("fill", "filled", "Filled by Wrangler")}{seg("fill", "empty", "Empty")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <div className="spec-panel">
        <strong>What changes</strong>
        <ul>
          <li><b>Copy.</b> Every box gets Copy at its right edge, disabled while empty; the secret keeps Show beside it. What is copied is what the form holds — typed, or filled by the Wrangler lane — so nothing is fetched and nothing is logged.</li>
          <li><b>Copy all.</b> On the Manual setup row beside Setup guide: the filled boxes as <code>Label: value</code> lines, secret included; disabled until one box holds something.</li>
          <li><b>Words (open question).</b> The proposal shows placeholders without “Example:”. Recommended: keep the prefix. Bucket and Account ID fill themselves, so a bare 32-hex example in an empty box reads as already filled, and the prefix was a keep on 2026-08-22.</li>
          <li><b>Unchanged.</b> Doors, the Wrangler lane, ? marks, the S3 API URL filling bucket and account, the footer (tally, Test connection, Finish setup).</li>
        </ul>
      </div>
      <div className="spec-cols">
        <div className="spec-col">
          <h3 className="spec-col-title">Proposal<em>A’s grammar</em></h3>
          <div className="spec-frame"><SetupProposed key={knobs.fill} fill={knobs.fill} onClipboard={onClipboard} /></div>
          <h3 className="spec-col-title">Clipboard{clip && <em>{clip.what}</em>}</h3>
          <pre className="spec-clip">{clip?.text ?? ""}</pre>
        </div>
        <div className="spec-col">
          <h3 className="spec-col-title">Current<em>ships</em></h3>
          <p className="spec-col-note">The real StorageEditor and R2 lane, as the New-storage page draws them.</p>
          <div className="spec-frame"><SetupCurrent key={knobs.fill} fill={knobs.fill} /></div>
        </div>
      </div>
    </div>
  );
  return (
    <div className={`spec${knobs.shot === "on" ? " spec-shot" : ""}`}>
      <div className="spec-title">Storage card — fewer words, copy everything</div>
      <p className="spec-sub">
        The configured storage’s card (your first screenshot), for R2, Dropbox and Feishu. Today R2 prints five help lines,
        five locks and two sentences, Dropbox three paragraphs; only secrets can be copied. The proposal keeps every value and
        every doc link, says less, and puts Copy on each value plus one Copy all.
      </p>
      <div className="spec-controls">
        <div className="spec-group"><span>Page</span>{seg("page", "card", "Storage card")}{seg("page", "setup", "Manual setup")}</div>
        <div className="spec-group"><span>Direction</span>{seg("dir", "a", "A · Quiet form")}{seg("dir", "b", "B · Details list")}</div>
        <div className="spec-group"><span>Status</span>{seg("status", "untested", "Not tested")}{seg("status", "verified", "Verified")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <div className="spec-panel">
        <strong>What changes</strong>
        <ul>
          <li><b>Words.</b> {direction.note}</li>
          <li><b>Status row.</b> “Configured. Test the connection to verify it.” becomes “Not tested yet.”; Setup guide stays; Copy all joins it.</li>
          <li><b>Copy all.</b> One plain-text block, <code>Label: value</code> per line in card order (R2’s endpoint, Dropbox’s and Feishu’s folder and account facts included). Secrets are fetched on that click like today’s secret Copy (each logged “Secret revealed”; Dropbox copies both tokens), never on load. A secret the storage does not hold is left out and the receipt says so.</li>
          <li><b>Unchanged.</b> Field order, footer sentence, Test connection, S3 endpoint row, Show/Hide with the 30 s auto-hide, the Reconnect band.</li>
        </ul>
      </div>
      <div className="spec-cols">
        <div className="spec-col">
          <h3 className="spec-col-title">{direction.title}<em>{direction.tag}</em></h3>
          <div className="spec-frame" ref={proposalRef}>
            <div className="sync-storage-details-panel"><ProposedCard key={`r2-${knobs.dir}`} descriptor={R2} storage={STORAGE} dir={knobs.dir} status={knobs.status} onClipboard={onClipboard} /></div>
          </div>
          {PROVIDERS.slice(1).map(([descriptor, storage]) => (
            <div key={descriptor.key} className="spec-frame">
              <div className="sync-storage-details-panel"><ProposedCard key={`${descriptor.key}-${knobs.dir}`} descriptor={descriptor} storage={storage} dir={knobs.dir} status={knobs.status} onClipboard={onClipboard} /></div>
            </div>
          ))}
          <h3 className="spec-col-title">Clipboard{clip && <em>{clip.what}</em>}</h3>
          <pre className="spec-clip">{clip?.text ?? ""}</pre>
          {measure && <p className="spec-measure">{measure}</p>}
        </div>
        <div className="spec-col">
          <h3 className="spec-col-title">Current<em>ships</em></h3>
          <p className="spec-col-note">The real StorageLockedForm with the descriptors and summaries the backend sends.</p>
          {PROVIDERS.map(([descriptor, storage]) => (
            <div key={descriptor.key} className="spec-frame">
              <div className="sync-storage-details-panel">
                <StorageLockedForm
                  storage={storage}
                  descriptor={descriptor}
                  disabled={false}
                  onRename={async () => true}
                  onRevealSecret={revealSecret}
                  onTest={() => undefined}
                  testing={false}
                  verifiedAt={knobs.status === "verified" ? Date.now() - 180_000 : null}
                  reconnect={{ open: false, onOpen: () => undefined, band: null }}
                />
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><App /></StrictMode>);

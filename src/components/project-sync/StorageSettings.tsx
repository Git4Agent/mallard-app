import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { StorageConfig } from "../../types";
import Icon from "../Icons";

const R2_SETUP_DOC = "https://developers.cloudflare.com/r2/get-started/s3/";
const R2_BUCKET_DOC = "https://developers.cloudflare.com/r2/buckets/create-buckets/";
const R2_CREDENTIALS_DOC = "https://developers.cloudflare.com/r2/api/tokens/";
const CLOUDFLARE_ACCOUNT_ID_DOC = "https://developers.cloudflare.com/fundamentals/account/find-account-and-zone-ids/";

export type R2Jurisdiction = "default" | "eu" | "fedramp";

const R2_ACCOUNT_ID_PATTERN = /^[0-9a-f]{32}$/i;
const R2_BUCKET_PATTERN = /^[a-z0-9](?:[a-z0-9-]{1,61}[a-z0-9])$/;
const R2_ENDPOINT_PATTERN = /^https:\/\/([0-9a-f]{32})(?:\.(eu|fedramp))?\.r2\.cloudflarestorage\.com$/i;
const R2_S3_API_URL_PATTERN = /^https:\/\/([0-9a-f]{32})(?:\.(eu|fedramp))?\.r2\.cloudflarestorage\.com\/([^/?#%]+)$/i;

async function openHelp(url: string) {
  try {
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

function HelpButton({ url, label }: { url: string; label: string }) {
  return (
    <button
      type="button"
      className="field-help"
      onClick={() => void openHelp(url)}
      title={label}
      aria-label={label}
    >
      ?
    </button>
  );
}

export function isCloudflareR2AccountId(value: string): boolean {
  return R2_ACCOUNT_ID_PATTERN.test(value.trim());
}

export function isCloudflareR2BucketName(value: string): boolean {
  return R2_BUCKET_PATTERN.test(value);
}

export function r2EndpointForAccount(
  accountId: string,
  jurisdiction: R2Jurisdiction = "default",
): string {
  const account = accountId.trim().toLowerCase();
  if (!isCloudflareR2AccountId(account)) return "";
  const jurisdictionLabel = jurisdiction === "default" ? "" : `.${jurisdiction}`;
  return `https://${account}${jurisdictionLabel}.r2.cloudflarestorage.com`;
}

export function parseR2Endpoint(value: string): {
  accountId: string;
  jurisdiction: R2Jurisdiction;
} | null {
  const match = R2_ENDPOINT_PATTERN.exec(value);
  if (!match) return null;
  return {
    accountId: match[1].toLowerCase(),
    jurisdiction: (match[2]?.toLowerCase() as Exclude<R2Jurisdiction, "default"> | undefined) ?? "default",
  };
}

export function r2S3ApiUrl(
  accountId: string,
  bucket: string,
  storedEndpoint?: string,
): string {
  if (!isCloudflareR2BucketName(bucket)) return "";
  const account = accountId.trim().toLowerCase();
  const endpoint = storedEndpoint === undefined
    ? r2EndpointForAccount(account)
    : (() => {
        const parsed = parseR2Endpoint(storedEndpoint);
        return parsed?.accountId === account
          ? r2EndpointForAccount(parsed.accountId, parsed.jurisdiction)
          : "";
      })();
  return endpoint ? `${endpoint}/${bucket}` : "";
}

export function parseR2S3ApiUrl(value: string): {
  accountId: string;
  bucket: string;
  jurisdiction: R2Jurisdiction;
} | null {
  const match = R2_S3_API_URL_PATTERN.exec(value);
  if (!match) return null;
  const bucket = match[3];
  if (!isCloudflareR2BucketName(bucket)) return null;
  return {
    accountId: match[1].toLowerCase(),
    bucket,
    jurisdiction: (match[2]?.toLowerCase() as Exclude<R2Jurisdiction, "default"> | undefined) ?? "default",
  };
}

export function storageConfigReady(storage: StorageConfig): boolean {
  if (!storage.name?.trim()) return false;
  if (storage.kind === "local") return !!storage.local_dir?.trim();
  const accountId = storage.account_id?.trim().toLowerCase() ?? "";
  const endpoint = parseR2Endpoint(storage.s3_endpoint ?? "");
  return isCloudflareR2BucketName(storage.bucket ?? "")
    && isCloudflareR2AccountId(accountId)
    && endpoint?.accountId === accountId
    && !!storage.access_key_id?.trim()
    && !!storage.secret_access_key?.trim();
}

export function newStorage(kind: "s3" | "local", index: number): StorageConfig {
  return {
    id: `storage-${crypto.randomUUID()}`,
    name: kind === "local" ? `Local storage ${index}` : `R2 storage ${index}`,
    kind,
    bucket: "",
    access_key_id: "",
    secret_access_key: "",
    account_id: "",
    s3_endpoint: "",
    region: kind === "s3" ? "auto" : "",
    local_dir: "",
    included_default_exclusions: [],
  };
}

export function storageNameForKind(name: string, kind: "s3" | "local"): string {
  const defaultName = /^(?:Local storage|R2 storage) (\d+)$/.exec(name);
  if (!defaultName) return name;
  return `${kind === "local" ? "Local storage" : "R2 storage"} ${defaultName[1]}`;
}

export function StorageEditor({
  storage,
  disabled,
  onChange,
  onRemove,
  primaryAction,
  showNameField = true,
}: {
  storage: StorageConfig;
  disabled: boolean;
  onChange: (storage: StorageConfig) => void;
  onRemove?: () => void;
  primaryAction?: ReactNode;
  showNameField?: boolean;
}) {
  const set = (patch: Partial<StorageConfig>) => onChange({ ...storage, ...patch });
  const derivedS3ApiUrl = r2S3ApiUrl(
    storage.account_id ?? "",
    storage.bucket ?? "",
    storage.s3_endpoint ?? "",
  );
  const [s3ApiUrlDraft, setS3ApiUrlDraft] = useState(derivedS3ApiUrl);
  const [secretVisible, setSecretVisible] = useState(false);
  const editingS3ApiUrl = useRef(false);
  const storedEndpoint = parseR2Endpoint(storage.s3_endpoint ?? "");
  const jurisdiction = useRef<R2Jurisdiction>(storedEndpoint?.jurisdiction ?? "default");
  const parsedS3ApiUrl = parseR2S3ApiUrl(s3ApiUrlDraft);
  useEffect(() => {
    if (!editingS3ApiUrl.current) setS3ApiUrlDraft(derivedS3ApiUrl);
  }, [derivedS3ApiUrl]);
  useEffect(() => {
    const parsed = parseR2Endpoint(storage.s3_endpoint ?? "");
    if (parsed) jurisdiction.current = parsed.jurisdiction;
  }, [storage.s3_endpoint]);
  const setAccountId = (accountId: string) => set({
    account_id: accountId,
    s3_endpoint: r2EndpointForAccount(accountId, jurisdiction.current),
    region: "auto",
  });
  const setS3ApiUrl = (value: string) => {
    setS3ApiUrlDraft(value);
    const parsed = parseR2S3ApiUrl(value);
    if (parsed) jurisdiction.current = parsed.jurisdiction;
    set(parsed ? {
      account_id: parsed.accountId,
      bucket: parsed.bucket,
      s3_endpoint: r2EndpointForAccount(parsed.accountId, parsed.jurisdiction),
      region: "auto",
    } : {
      account_id: "",
      bucket: "",
      s3_endpoint: "",
      region: "auto",
    });
  };
  const chooseFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) set({ local_dir: picked });
  };
  // Counts the five rendered fields, not the four stored ones: the URL flips to
  // set exactly when account and bucket both exist, so pasting it moves the
  // counter by three — which is the coupling, stated honestly.
  const r2FieldsSet = [
    derivedS3ApiUrl,
    storage.bucket,
    storage.account_id,
    storage.access_key_id,
    storage.secret_access_key,
  ].filter((value) => !!value?.trim()).length;

  return (
    <article className="sync-storage-card">
      <header>
        <span className="sync-storage-icon"><Icon name={storage.kind === "local" ? "drive" : "cloud"} size={18} /></span>
        {showNameField ? (
          <input value={storage.name} onChange={(event) => set({ name: event.target.value })} disabled={disabled} aria-label="Storage name" />
        ) : (
          <h2 className="sync-storage-editor-title">Storage settings</h2>
        )}
        <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
          <button
            type="button"
            role="radio"
            aria-checked={storage.kind !== "local"}
            className={storage.kind !== "local" ? "active" : undefined}
            onClick={() => set({
              name: storageNameForKind(storage.name, "s3"),
              kind: "s3",
              region: "auto",
              s3_endpoint: storage.s3_endpoint
                || r2EndpointForAccount(storage.account_id ?? "", jurisdiction.current),
            })}
            disabled={disabled}
          >
            Cloudflare R2
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={storage.kind === "local"}
            className={storage.kind === "local" ? "active" : undefined}
            onClick={() => set({
              name: storageNameForKind(storage.name, "local"),
              kind: "local",
            })}
            disabled={disabled}
          >
            Local folder
          </button>
        </div>
        {onRemove && (
          <button type="button" className="btn btn-ghost danger" onClick={onRemove} disabled={disabled} title="Remove storage" aria-label="Remove storage"><Icon name="trash" size={14} /></button>
        )}
      </header>
      {storage.kind === "local" ? (
        <label className="sync-folder-field compact">
          <span className="sync-field-name">Location path</span>
          <div>
            <input
              type="text"
              value={storage.local_dir ?? ""}
              onChange={(event) => set({ local_dir: event.target.value })}
              placeholder="e.g. /Volumes/backup/mallard-storage"
              disabled={disabled}
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
            <button type="button" className="btn" onClick={() => void chooseFolder()} disabled={disabled}>
              <Icon name="folder" size={13} /> Browse…
            </button>
          </div>
        </label>
      ) : (
        <>
          {/* One line instead of a heading, a sentence, a numbered list, and a
              warning: every doc the list linked is still one click away behind
              the field it explains, and the only unrecoverable fact — the
              secret is shown once — survives as the line's tail. */}
          <div className="sync-r2-guide" role="note">
            <Icon name="info" size={13} />
            <span className="sync-r2-guide-copy">
              Needs one bucket and an <strong>Object Read &amp; Write</strong> token scoped to that bucket.
            </span>
            <button type="button" className="btn-link" onClick={() => void openHelp(R2_SETUP_DOC)}>Setup guide</button>
            <span className="sync-r2-guide-warning">Secret shown once</span>
          </div>

          <div className="sync-storage-fields sync-r2-fields">
            <label className="sync-r2-url-field">
              <span className="form-label-row">
                <span className="sync-field-name">S3 API URL</span>
                <HelpButton url={R2_SETUP_DOC} label="How to find the Cloudflare R2 S3 API URL" />
              </span>
              <input
                type="url"
                value={s3ApiUrlDraft}
                onFocus={() => { editingS3ApiUrl.current = true; }}
                onChange={(event) => setS3ApiUrl(event.target.value)}
                onBlur={() => {
                  editingS3ApiUrl.current = false;
                  if (parsedS3ApiUrl) {
                    setS3ApiUrlDraft(r2S3ApiUrl(
                      parsedS3ApiUrl.accountId,
                      parsedS3ApiUrl.bucket,
                      r2EndpointForAccount(parsedS3ApiUrl.accountId, parsedS3ApiUrl.jurisdiction),
                    ));
                  }
                }}
                placeholder="Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent"
                disabled={disabled}
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
                aria-invalid={s3ApiUrlDraft.trim() && !parsedS3ApiUrl ? true : undefined}
              />
            </label>
            <label>
              <span className="form-label-row">
                <span className="sync-field-name">Bucket name</span>
                <HelpButton url={R2_BUCKET_DOC} label="How to create a Cloudflare R2 bucket" />
              </span>
              <input
                value={storage.bucket ?? ""}
                onChange={(event) => set({ bucket: event.target.value })}
                placeholder="Example: codex-sync-backups"
                disabled={disabled}
                required
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
              />
            </label>
            <label>
              <span className="form-label-row">
                <span className="sync-field-name">Account ID</span>
                <HelpButton url={CLOUDFLARE_ACCOUNT_ID_DOC} label="Where to find your Cloudflare Account ID" />
              </span>
              <input
                value={storage.account_id ?? ""}
                onChange={(event) => setAccountId(event.target.value)}
                placeholder="Example: 023e105f4ecef8ad9ca31a8372d0c353"
                disabled={disabled}
                required
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
              />
            </label>
            <label>
              <span className="form-label-row">
                <span className="sync-field-name">Access Key ID</span>
                <HelpButton url={R2_CREDENTIALS_DOC} label="How to create an R2 Access Key ID" />
              </span>
              <input
                value={storage.access_key_id ?? ""}
                onChange={(event) => set({ access_key_id: event.target.value })}
                placeholder="Example: 4a2b…91ef"
                disabled={disabled}
                required
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
              />
            </label>
            <label>
              <span className="form-label-row">
                <span className="sync-field-name">Secret Access Key</span>
                <HelpButton url={R2_CREDENTIALS_DOC} label="How to create an R2 Secret Access Key" />
              </span>
              {/* NIST SP 800-63B §5.1.1.2: the secret can be shown while
                  entering it. A word, not an eye glyph — the icon alone does
                  not say which state it puts you in. */}
              <span className="sync-secret-input">
                <input
                  type={secretVisible ? "text" : "password"}
                  value={storage.secret_access_key ?? ""}
                  onChange={(event) => set({ secret_access_key: event.target.value })}
                  placeholder="Example: a1B2…x9Y0"
                  disabled={disabled}
                  required
                  autoComplete="new-password"
                />
                <button
                  type="button"
                  className="sync-secret-reveal"
                  onClick={() => setSecretVisible((visible) => !visible)}
                  aria-pressed={secretVisible}
                  disabled={disabled}
                >
                  {secretVisible ? "Hide" : "Show"}
                </button>
              </span>
            </label>
          </div>
        </>
      )}
      {(primaryAction || storage.kind !== "local") && (
        <footer className="sync-storage-editor-footer">
          {storage.kind !== "local" && (
            <span className="sync-storage-editor-progress" aria-live="polite">
              {r2FieldsSet} of 5 set
            </span>
          )}
          {primaryAction}
        </footer>
      )}
    </article>
  );
}

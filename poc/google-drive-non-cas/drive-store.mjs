// Standalone capability probe. The application does not import this module.
import { createHash, randomBytes } from "node:crypto";
import { credential, json, PocError, request, requireValue } from "../storage-discovery/http.mjs";

export { PocError };
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const MARKER_KEY = ".mallard/_storage.json";
export const MARKER = Buffer.from('{"format":"mallard-append-storage","layout_version":1}');
export const MAX_OBJECT_BYTES = 4 * 1024 * 1024;
export const MAX_RECORD_BYTES = 64 * 1024;
const API = "https://www.googleapis.com/drive/v3/";
const UPLOAD = "https://www.googleapis.com/upload/drive/v3/files";
const MIME = "application/octet-stream";
const FOLDER = "application/vnd.google-apps.folder";
const DESCRIPTION = "mallard-object-key:";
const FORMAT = "append-v1";
const FILE_FIELDS = "id,name,description,mimeType,parents,properties,trashed,size";

export function driveId(value) {
  requireValue(typeof value === "string" && /^[A-Za-z0-9_-]{1,200}$/u.test(value), "invalid_drive_id");
  return value;
}

function pathParts(value) {
  requireValue(typeof value === "string" && value.isWellFormed() && Buffer.byteLength(value) <= 4096
    && !/[\\:\p{Cc}]/u.test(value), "invalid_key");
  const parts = value.split("/");
  // Current engine keys need at most seven parts. Bound the property count.
  requireValue(parts.length <= 25 && parts.every((part) => part && part !== "."
    && part !== ".." && Buffer.byteLength(part) <= 255), "invalid_key");
  return parts;
}

export function validateKey(key) {
  const parts = pathParts(key);
  requireValue(key === MARKER_KEY || (parts.length >= 5 && parts[0] === ".mallard"
    && parts[1] === "v1" && ["repositories", "library"].includes(parts[2])), "invalid_key");
  return key;
}

function validatePrefix(prefix) {
  const parts = pathParts(prefix);
  requireValue(parts.length >= 3 && parts[0] === ".mallard" && parts[1] === "v1"
    && ["repositories", "library"].includes(parts[2]), "invalid_prefix");
  return parts;
}

export function metadataFor(key, rootId, id) {
  validateKey(key);
  const parts = key.split("/");
  const properties = { mallard: FORMAT, key: sha256(key) };
  for (let depth = 3; depth < parts.length; depth += 1) {
    properties[`p${depth}`] = sha256(parts.slice(0, depth).join("/"));
  }
  return { id: driveId(id), name: parts.at(-1), description: DESCRIPTION + key,
    mimeType: MIME, parents: [driveId(rootId)], properties };
}

export class DriveStore {
  #token;
  constructor({ token, rootId, sharedDriveId, space = "drive", fetchImpl = fetch,
    pageSize = 1000, maxPages = 1000, maxItems = 100_000, maxCopies = 32 }) {
    this.#token = credential(token);
    this.rootId = driveId(rootId);
    this.sharedDriveId = sharedDriveId === undefined ? undefined : driveId(sharedDriveId);
    requireValue(["drive", "appDataFolder"].includes(space)
      && !(space === "appDataFolder" && sharedDriveId), "invalid_space");
    requireValue(Number.isInteger(pageSize) && pageSize >= 1 && pageSize <= 1000
      && [maxPages, maxItems, maxCopies].every((n) => Number.isSafeInteger(n) && n > 0), "invalid_limits");
    Object.assign(this, { space, fetchImpl, pageSize, maxPages, maxItems, maxCopies });
  }

  async call(url, { method = "GET", body, headers = {}, maxBytes = 8 * 1024 * 1024 } = {}) {
    // URLs are made only from fixed Google endpoints and validated IDs.
    const parsed = new URL(url);
    requireValue(parsed.origin === "https://www.googleapis.com" && !parsed.username
      && !parsed.password && !parsed.hash, "invalid_api_origin");
    const result = await request(parsed, { method, body,
      headers: { ...headers, Authorization: `Bearer ${this.#token}` } },
    { fetchImpl: this.fetchImpl, maxBytes });
    if (!result.ok) {
      // No provider response body, token, URL, or account detail escapes.
      throw new PocError(`drive_http_${result.status}`, result.status);
    }
    return result.bytes;
  }

  async allocateId() {
    const url = new URL("files/generateIds", API);
    url.search = new URLSearchParams({ count: "1", space: this.space, type: "files" }).toString();
    const value = json(await this.call(url));
    requireValue(Array.isArray(value.ids) && value.ids.length === 1, "invalid_generated_id");
    return driveId(value.ids[0]);
  }

  async validateRoot() {
    const url = new URL(`files/${this.rootId}`, API);
    url.search = new URLSearchParams({ fields: "id,mimeType,trashed,driveId,capabilities(canAddChildren)",
      supportsAllDrives: "true" }).toString();
    const value = json(await this.call(url));
    requireValue((value.id === this.rootId || ["root", "appDataFolder"].includes(this.rootId))
      && value.mimeType === FOLDER && !value.trashed
      && value.capabilities?.canAddChildren === true
      && value.driveId === this.sharedDriveId, "root_not_writable_or_wrong_drive");
    this.rootId = driveId(value.id);
  }

  async createProbeFolder(name) {
    requireValue(/^mallard-drive-poc-[a-f0-9]{24}$/u.test(name), "invalid_probe_name");
    const id = await this.allocateId();
    const url = new URL("files", API);
    url.search = new URLSearchParams({ supportsAllDrives: "true", fields: "id" }).toString();
    // One attempt. A failed response is ambiguous; the live report retains name.
    const value = json(await this.call(url, { method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id, name, parents: [this.rootId], mimeType: FOLDER }) }));
    requireValue(value.id === id, "invalid_create_response");
    return id;
  }

  decode(file) {
    requireValue(file && typeof file === "object" && !Array.isArray(file), "invalid_file");
    if (file.properties?.mallard !== FORMAT) return null;
    requireValue(typeof file.description === "string" && file.description.startsWith(DESCRIPTION), "invalid_key_metadata");
    const key = validateKey(file.description.slice(DESCRIPTION.length));
    const expected = metadataFor(key, this.rootId, driveId(file.id));
    requireValue(file.mimeType === MIME && file.trashed !== true
      && Array.isArray(file.parents) && file.parents.length === 1 && file.parents[0] === this.rootId
      && file.name === expected.name && Object.entries(expected.properties)
        .every(([name, value]) => file.properties[name] === value), "invalid_key_metadata");
    return key;
  }

  async query(property, value) {
    requireValue(/^(key|p[0-9]+)$/u.test(property) && /^[a-f0-9]{64}$/u.test(value), "invalid_query");
    const q = `'${this.rootId}' in parents and trashed = false`
      + ` and properties has { key='mallard' and value='${FORMAT}' }`
      + ` and properties has { key='${property}' and value='${value}' }`;
    const files = new Map();
    const cursors = new Set();
    let cursor;
    let listed = 0;
    for (let page = 0; page < this.maxPages; page += 1) {
      const params = { q, spaces: this.space, corpora: this.sharedDriveId ? "drive" : "user",
        pageSize: String(this.pageSize), supportsAllDrives: "true",
        fields: `nextPageToken,incompleteSearch,files(${FILE_FIELDS})` };
      if (this.sharedDriveId) Object.assign(params,
        { driveId: this.sharedDriveId, includeItemsFromAllDrives: "true" });
      if (cursor !== undefined) params.pageToken = cursor;
      const url = new URL("files", API);
      url.search = new URLSearchParams(params).toString();
      const result = json(await this.call(url));
      requireValue(result.incompleteSearch === undefined || result.incompleteSearch === false, "incomplete_search");
      requireValue(Array.isArray(result.files), "invalid_listing");
      listed += result.files.length;
      requireValue(listed <= this.maxItems, "listing_limit");
      for (const file of result.files) {
        const key = this.decode(file);
        if (key === null) continue;
        requireValue(file.properties[property] === value, "query_scope_mismatch");
        const previous = files.get(file.id);
        requireValue(!previous || JSON.stringify(previous) === JSON.stringify(file), "listing_changed");
        files.set(file.id, file);
      }
      if (result.nextPageToken === undefined) return [...files.values()];
      requireValue(typeof result.nextPageToken === "string" && result.nextPageToken.length > 0
        && result.nextPageToken.length <= 4096 && !cursors.has(result.nextPageToken), "invalid_page_token");
      cursor = result.nextPageToken;
      cursors.add(cursor);
    }
    throw new PocError("listing_page_limit");
  }

  async copies(key) {
    validateKey(key);
    const files = await this.query("key", sha256(key));
    requireValue(files.every((file) => this.decode(file) === key), "key_hash_conflict");
    requireValue(files.length <= this.maxCopies, "duplicate_limit");
    return files.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  }

  async listKeys(prefix) {
    const parts = validatePrefix(prefix);
    const files = await this.query(`p${parts.length}`, sha256(prefix));
    const keys = files.map((file) => this.decode(file));
    requireValue(keys.every((key) => key.startsWith(`${prefix}/`)), "prefix_mismatch");
    return [...new Set(keys)].sort();
  }

  async get(key, { maxBytes = MAX_OBJECT_BYTES, expected } = {}) {
    requireValue(Number.isSafeInteger(maxBytes) && maxBytes >= 0 && maxBytes <= MAX_OBJECT_BYTES, "invalid_read_limit");
    const files = await this.copies(key);
    let bytes;
    for (const file of files) {
      const url = new URL(`files/${driveId(file.id)}`, API);
      url.search = new URLSearchParams({ alt: "media", supportsAllDrives: "true" }).toString();
      const current = await this.call(url, { maxBytes });
      requireValue(bytes === undefined || current.equals(bytes), "duplicate_content_conflict");
      bytes = current;
    }
    if (bytes !== undefined && expected) {
      requireValue(bytes.length === expected.size && sha256(bytes) === expected.sha256, "digest_mismatch");
    }
    return bytes ?? null;
  }

  async prepare(key, input) {
    validateKey(key);
    const bytes = Buffer.from(input);
    requireValue(bytes.length <= MAX_OBJECT_BYTES, "upload_limit");
    const id = await this.allocateId();
    return { key, bytes, id, metadata: metadataFor(key, this.rootId, id) };
  }

  async writeOnce(prepared) {
    // This method never retries, queries, or updates a file. Record retry belongs
    // to the publication layer, after another complete stream listing.
    requireValue(prepared.bytes.length <= MAX_OBJECT_BYTES
      && JSON.stringify(prepared.metadata) === JSON.stringify(metadataFor(prepared.key, this.rootId, prepared.id)), "invalid_prepared_upload");
    let boundary;
    do { boundary = `mallard_${randomBytes(24).toString("hex")}`; }
    while (prepared.bytes.includes(boundary));
    const body = Buffer.concat([
      Buffer.from(`--${boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n${JSON.stringify(prepared.metadata)}\r\n`),
      Buffer.from(`--${boundary}\r\nContent-Type: ${MIME}\r\n\r\n`), prepared.bytes,
      Buffer.from(`\r\n--${boundary}--\r\n`),
    ]);
    const url = new URL(UPLOAD);
    url.search = new URLSearchParams({ uploadType: "multipart", supportsAllDrives: "true", fields: "id" }).toString();
    const result = json(await this.call(url, { method: "POST", body,
      headers: { "Content-Type": `multipart/related; boundary=${boundary}` } }));
    requireValue(result.id === prepared.id, "invalid_create_response");
    return "written";
  }

  async putImmutable(key, bytes) {
    validateKey(key);
    const input = Buffer.from(bytes);
    requireValue(input.length <= MAX_OBJECT_BYTES, "upload_limit");
    const existing = await this.get(key);
    if (existing !== null) {
      requireValue(existing.equals(input), "immutable_content_conflict");
      return "already_present";
    }
    return this.writeOnce(await this.prepare(key, input));
  }
}

import { createHash } from "node:crypto";
import { credential, json, PocError, request, requireValue, responseError } from "./http.mjs";

export const GRAPH = "https://graph.microsoft.com/v1.0/";
export const CHUNK_BYTES = 320 * 1024;
const ITEM_FIELDS = "id,name,eTag,size,file,folder,remoteItem,parentReference";
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function identifier(value) {
  requireValue(typeof value === "string" && /^[A-Za-z0-9!_,.-]{1,512}$/u.test(value)
    && value !== "." && value !== "..", "invalid_item_identity");
  return value;
}

function parseUrl(value, code) {
  try { return new URL(value); }
  catch { throw new PocError(code); }
}

export function sharingUrl(value) {
  requireValue(typeof value === "string" && value.length <= 8192, "invalid_sharing_url");
  const url = parseUrl(value, "invalid_sharing_url");
  requireValue(url.protocol === "https:" && url.hostname.endsWith(".sharepoint.com")
    && !url.port && !url.username && !url.password && !url.hash, "invalid_sharing_url");
  return url.href;
}

function capabilityUrl(value) {
  requireValue(typeof value === "string" && value.length <= 16384, "invalid_transfer_url");
  const url = parseUrl(value, "invalid_transfer_url");
  requireValue(url.protocol === "https:" && !url.port && !url.username && !url.password && !url.hash
    && [".sharepoint.com", ".1drv.com"].some((suffix) => url.hostname.endsWith(suffix)),
  "invalid_transfer_url");
  return url;
}

export function folderIdentity(item) {
  // A shortcut is resolved only here. Files inside the probe cannot be shortcuts.
  const target = item?.remoteItem ?? item;
  requireValue(target?.folder && !target.file && !target.remoteItem, "shared_root_is_not_folder");
  return { driveId: identifier(target.parentReference?.driveId), id: identifier(target.id) };
}

function itemPath(root, id = root.id) {
  return `drives/${encodeURIComponent(identifier(root.driveId))}/items/${encodeURIComponent(identifier(id))}`;
}

function probeName(name) {
  requireValue(/^mallard-ms-poc-[0-9a-f]{24}-(a|b|claim)\.bin$/u.test(name), "invalid_probe_name");
  return name;
}

export function fileIdentity(item, root, name, size) {
  requireValue(item?.name === name && item.file && !item.folder && !item.remoteItem
    && Number.isSafeInteger(item.size) && item.size >= 0 && (size === undefined || item.size === size)
    && typeof item.eTag === "string" && item.eTag.length > 0 && item.eTag.length <= 2048
    && item.parentReference?.id === root.id && item.parentReference?.driveId === root.driveId,
  "invalid_probe_file");
  return { id: identifier(item.id), name, etag: item.eTag, size: item.size };
}

export class GraphClient {
  constructor(token, options = {}) {
    this.token = credential(token);
    this.options = options;
  }

  async call(path, init = {}) {
    const url = new URL(path, GRAPH);
    requireValue(url.origin === "https://graph.microsoft.com" && url.pathname.startsWith("/v1.0/")
      && !url.username && !url.password && !url.hash, "invalid_graph_url");
    const response = await request(url, { ...init, method: init.method ?? "GET",
      headers: { ...init.headers, Authorization: `Bearer ${this.token}` } }, this.options);
    if (!response.ok) throw responseError(response);
    return json(response.bytes);
  }

  async resolveRoot(link) {
    const encoded = `u!${Buffer.from(sharingUrl(link)).toString("base64url")}`;
    // No Prefer: redeemSharingLink: the operator must grant both users access first.
    const shared = await this.call(`shares/${encoded}/driveItem?$select=${ITEM_FIELDS}`);
    const root = folderIdentity(shared);
    const direct = folderIdentity(await this.call(`${itemPath(root)}?$select=${ITEM_FIELDS}`));
    requireValue(direct.driveId === root.driveId && direct.id === root.id, "root_changed");
    const drive = await this.call(`drives/${encodeURIComponent(root.driveId)}?$select=id,driveType`);
    requireValue(drive.id === root.driveId && ["business", "documentLibrary"].includes(drive.driveType),
      "unsupported_drive_type");
    return { ...root, driveType: drive.driveType };
  }

  async list(root) {
    const first = new URL(`${itemPath(root)}/children?$top=1&$select=${ITEM_FIELDS}`, GRAPH);
    let url = first;
    const pages = new Set();
    const ids = new Set();
    const names = new Set();
    const items = [];
    while (url) {
      requireValue(url.origin === first.origin && url.pathname === first.pathname
        && !url.username && !url.password && !url.hash && !pages.has(url.href), "invalid_next_link");
      requireValue(pages.size < (this.options.maxPages ?? 100), "listing_limit");
      pages.add(url.href);
      const page = await this.call(url.href);
      requireValue(Array.isArray(page.value), "invalid_listing");
      for (const item of page.value) {
        const id = identifier(item?.id);
        requireValue(typeof item.name === "string" && item.name.length <= 1024, "invalid_listing");
        const name = item.name.toLowerCase();
        requireValue(!ids.has(id) && !names.has(name), "duplicate_listing_item");
        requireValue(items.length < (this.options.maxItems ?? 1000), "listing_limit");
        ids.add(id);
        names.add(name);
        items.push(item);
      }
      const next = page["@odata.nextLink"];
      requireValue(next === undefined || typeof next === "string" && next.length > 0 && next.length <= 16384,
        "invalid_next_link");
      url = next === undefined ? null : parseUrl(next, "invalid_next_link");
    }
    return { items, pages: pages.size };
  }

  async getFile(root, name) {
    probeName(name);
    try { return await this.call(`${itemPath(root)}:/${name}`); }
    catch (error) {
      if (error instanceof PocError && error.status === 404) return null;
      throw error;
    }
  }

  async startUpload(root, name) {
    probeName(name);
    const result = await this.call(`${itemPath(root)}:/${name}:/createUploadSession`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ item: { name, "@microsoft.graph.conflictBehavior": "fail" }, deferCommit: false }),
    });
    return capabilityUrl(result.uploadUrl);
  }

  async uploadChunk(url, bytes, start = 0, end = bytes.length) {
    requireValue(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= 2 * CHUNK_BYTES
      && Number.isInteger(start) && Number.isInteger(end) && start >= 0 && start < end
      && end <= bytes.length && start % CHUNK_BYTES === 0
      && (end === bytes.length || (end - start) % CHUNK_BYTES === 0), "invalid_upload_range");
    const response = await request(capabilityUrl(url.href ?? url), {
      method: "PUT", headers: { "Content-Type": "application/octet-stream",
        "Content-Length": String(end - start), "Content-Range": `bytes ${start}-${end - 1}/${bytes.length}` },
      body: bytes.subarray(start, end),
    }, this.options);
    if (!response.ok) throw responseError(response);
    const result = json(response.bytes);
    if (end < bytes.length) {
      requireValue(response.status === 202 && Array.isArray(result.nextExpectedRanges)
        && result.nextExpectedRanges.length === 1 && result.nextExpectedRanges[0] === `${end}-`,
      "unexpected_upload_progress");
    } else {
      requireValue(response.status === 200 || response.status === 201, "upload_not_committed");
    }
    return result;
  }

  async create(root, name, bytes) {
    const upload = await this.startUpload(root, name);
    return fileIdentity(await this.uploadChunk(upload, bytes), root, name, bytes.length);
  }

  async verify(root, expected, bytes) {
    const before = await this.call(itemPath(root, expected.id));
    const actual = fileIdentity(before, root, expected.name, bytes.length);
    requireValue(actual.id === expected.id && actual.etag === expected.etag, "file_changed");
    const download = await request(capabilityUrl(before["@microsoft.graph.downloadUrl"]),
      { method: "GET" }, { ...this.options, maxBytes: bytes.length });
    if (!download.ok) throw responseError(download);
    requireValue(download.bytes.length === bytes.length && sha256(download.bytes) === sha256(bytes),
      "digest_mismatch");
    const after = fileIdentity(await this.call(itemPath(root, expected.id)), root, expected.name, bytes.length);
    requireValue(after.id === expected.id && after.etag === expected.etag, "file_changed");
  }
}

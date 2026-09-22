import { createHash } from "node:crypto";

export const API_ROOT = "https://open.feishu.cn/open-apis/drive/v1";
export const SMALL_FILE_BYTES = 20 * 1024 * 1024;
export const MAX_FILE_BYTES = 100 * 1024 * 1024;
export const MAX_FOLDER_ENTRIES = 1500;
const MAX_JSON_BYTES = 1024 * 1024;
const MAX_PAGES = 1500;
const MAX_TREE_FOLDERS = 1024;

export class PocError extends Error {
  constructor(code) {
    super(code);
    this.name = "PocError";
    this.code = code;
  }
}

export function requireValue(condition, code) {
  if (!condition) throw new PocError(code);
}

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function expected(bytes) {
  return { size: bytes.length, sha256: sha256(bytes) };
}

function token(value) {
  requireValue(typeof value === "string" && /^[a-zA-Z0-9_-]{1,256}$/u.test(value), "invalid_token");
  return value;
}

function fileName(value) {
  requireValue(typeof value === "string" && /^[a-z0-9_.-]{1,250}$/u.test(value)
    && value !== "." && value !== "..", "invalid_name");
  return value;
}

export function keyParts(key) {
  requireValue(typeof key === "string" && key.length <= 4096, "invalid_key");
  const parts = key.split("/");
  for (const part of parts) fileName(part);
  requireValue(key === ".mallard/_storage.json" || (
    parts[0] === ".mallard" && parts[1] === "v1"
    && ["repositories", "library"].includes(parts[2]) && parts.length >= 5
  ), "key_outside_mallard");
  return parts;
}

export async function readBounded(response, maximum) {
  const length = response.headers.get("content-length");
  if (length !== null && (!/^\d+$/u.test(length) || Number(length) > maximum)) {
    await response.body?.cancel().catch(() => {});
    throw new PocError("response_too_large");
  }
  const chunks = [];
  let size = 0;
  if (response.body) {
    const reader = response.body.getReader();
    try {
      for (;;) {
        const next = await reader.read();
        if (next.done) break;
        size += next.value.length;
        requireValue(size <= maximum, "response_too_large");
        chunks.push(Buffer.from(next.value));
      }
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
  }
  return Buffer.concat(chunks, size);
}

// Share one gate across both probe clients. The live probe sends no parallel
// requests. This gate does not coordinate independent machines.
export class RequestGate {
  constructor(intervalMs = 250) {
    this.intervalMs = intervalMs;
    this.tail = Promise.resolve();
    this.nextAt = 0;
  }

  run(action) {
    const pending = this.tail.then(async () => {
      const delay = this.nextAt - Date.now();
      if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
      try {
        return await action();
      } finally {
        this.nextAt = Date.now() + this.intervalMs;
      }
    });
    this.tail = pending.catch(() => {});
    return pending;
  }
}

export class FeishuDrive {
  #accessToken;

  constructor({ accessToken, fetchImpl = fetch, gate = new RequestGate(), pageSize = 200 }) {
    requireValue(typeof accessToken === "string" && /^[\x21-\x7e]{1,16384}$/u.test(accessToken),
      "invalid_access_token");
    requireValue(Number.isInteger(pageSize) && pageSize > 0 && pageSize <= 200, "invalid_page_size");
    this.#accessToken = accessToken;
    this.fetchImpl = fetchImpl;
    this.gate = gate;
    this.pageSize = pageSize;
  }

  async request(path, { method = "GET", body, binaryLimit } = {}) {
    requireValue(path.startsWith("/files") && !path.includes(".."), "invalid_api_path");
    return this.gate.run(async () => {
      try {
        const headers = { Authorization: `Bearer ${this.#accessToken}` };
        if (!(body instanceof FormData)) headers["Content-Type"] = "application/json; charset=utf-8";
        const response = await this.fetchImpl(`${API_ROOT}${path}`, {
          method, headers, body: body instanceof FormData ? body : body === undefined ? undefined : JSON.stringify(body),
          redirect: "error", signal: AbortSignal.timeout(30_000),
        });
        if (response.status !== 200) {
          await response.body?.cancel().catch(() => {});
          throw new PocError(`http_${response.status}`);
        }
        if (binaryLimit !== undefined) return await readBounded(response, binaryLimit);
        const bytes = await readBounded(response, MAX_JSON_BYTES);
        let envelope;
        try { envelope = JSON.parse(bytes.toString("utf8")); } catch { throw new PocError("invalid_json"); }
        requireValue(envelope && Number.isInteger(envelope.code), "invalid_envelope");
        requireValue(envelope.code === 0, `feishu_${envelope.code}`);
        requireValue(envelope.data && typeof envelope.data === "object" && !Array.isArray(envelope.data),
          "invalid_data");
        return envelope.data;
      } catch (error) {
        // Do not log provider bodies, URLs, file tokens, credentials, or raw
        // transport exceptions. In particular, never retry a record upload.
        if (error instanceof PocError) throw error;
        throw new PocError("transport_failure");
      }
    });
  }

  async listFolder(folderToken) {
    token(folderToken);
    const entries = new Map();
    const cursors = new Set();
    let cursor;
    for (let page = 0; page < MAX_PAGES; page += 1) {
      const query = new URLSearchParams({
        folder_token: folderToken, page_size: String(this.pageSize),
        order_by: "CreatedTime", direction: "ASC",
      });
      if (cursor !== undefined) query.set("page_token", cursor);
      const data = await this.request(`/files?${query}`);
      requireValue(Array.isArray(data.files) && typeof data.has_more === "boolean"
        && data.files.length <= this.pageSize, "invalid_listing");
      for (const entry of data.files) {
        requireValue(entry !== null && typeof entry === "object", "invalid_listing_entry");
        token(entry.token);
        requireValue(typeof entry.name === "string" && entry.name.length <= 1024
          && typeof entry.type === "string" && entry.parent_token === folderToken, "invalid_listing_entry");
        const previous = entries.get(entry.token);
        requireValue(!previous || (previous.name === entry.name && previous.type === entry.type),
          "listing_entry_changed");
        entries.set(entry.token, entry);
        requireValue(entries.size <= MAX_FOLDER_ENTRIES, "folder_entry_limit");
      }
      if (!data.has_more) return [...entries.values()];
      requireValue(typeof data.next_page_token === "string" && data.next_page_token.length > 0
        && data.next_page_token.length <= 4096 && !cursors.has(data.next_page_token), "invalid_cursor");
      cursor = data.next_page_token;
      cursors.add(cursor);
    }
    throw new PocError("listing_page_limit");
  }

  async createFolder(parent, name) {
    const data = await this.request("/files/create_folder", {
      method: "POST", body: { name: fileName(name), folder_token: token(parent) },
    });
    return token(data.token);
  }

  async upload(parent, name, bytes, { multipart = bytes.length > SMALL_FILE_BYTES } = {}) {
    token(parent);
    fileName(name);
    requireValue(Buffer.isBuffer(bytes) && bytes.length <= MAX_FILE_BYTES, "file_size_limit");
    if (!multipart) {
      requireValue(bytes.length <= SMALL_FILE_BYTES, "small_file_size_limit");
      const form = new FormData();
      form.set("file_name", name);
      form.set("parent_type", "explorer");
      form.set("parent_node", parent);
      form.set("size", String(bytes.length));
      form.set("file", new Blob([bytes]), name);
      const data = await this.request("/files/upload_all", { method: "POST", body: form });
      return token(data.file_token);
    }
    const plan = await this.request("/files/upload_prepare", {
      method: "POST", body: { file_name: name, parent_type: "explorer", parent_node: parent, size: bytes.length },
    });
    token(plan.upload_id);
    requireValue(Number.isInteger(plan.block_size) && plan.block_size > 0
      && plan.block_size <= SMALL_FILE_BYTES && Number.isInteger(plan.block_num)
      && plan.block_num > 0 && plan.block_num <= 1024
      && plan.block_num === Math.ceil(bytes.length / plan.block_size), "invalid_multipart_plan");
    for (let seq = 0; seq < plan.block_num; seq += 1) {
      const part = bytes.subarray(seq * plan.block_size, (seq + 1) * plan.block_size);
      const form = new FormData();
      form.set("upload_id", plan.upload_id);
      form.set("seq", String(seq));
      form.set("size", String(part.length));
      form.set("file", new Blob([part]), "part");
      await this.request("/files/upload_part", { method: "POST", body: form });
    }
    const data = await this.request("/files/upload_finish", {
      method: "POST", body: { upload_id: plan.upload_id, block_num: plan.block_num },
    });
    return token(data.file_token);
  }

  async download(fileToken, expectation) {
    requireValue(Number.isInteger(expectation.size) && expectation.size >= 0
      && expectation.size <= MAX_FILE_BYTES && /^[a-f0-9]{64}$/u.test(expectation.sha256), "invalid_expectation");
    const bytes = await this.request(`/files/${token(fileToken)}/download`, { binaryLimit: expectation.size });
    requireValue(bytes.length === expectation.size && sha256(bytes) === expectation.sha256, "integrity_mismatch");
    return bytes;
  }
}

// Small, unsharded PoC only. It preserves logical keys as folder components.
// Duplicate folders fail closed. No first-result path resolution is allowed.
export class FeishuStore {
  constructor(drive, rootFolderToken) {
    this.drive = drive;
    this.root = token(rootFolderToken);
  }

  async folder(parts, create = false) {
    let parent = this.root;
    requireValue(parts.length <= 12, "folder_depth_limit");
    for (const component of parts) {
      fileName(component);
      let entries = await this.drive.listFolder(parent);
      let matches = entries.filter((entry) => entry.name === component);
      if (matches.length === 0 && create) {
        const created = await this.drive.createFolder(parent, component);
        entries = await this.drive.listFolder(parent);
        matches = entries.filter((entry) => entry.name === component);
        requireValue(matches.some((entry) => entry.token === created), "folder_visibility_gap");
      }
      if (matches.length === 0) return null;
      requireValue(matches.length === 1 && matches[0].type === "folder", "ambiguous_folder");
      parent = matches[0].token;
    }
    return parent;
  }

  async location(key, create = false) {
    const parts = keyParts(key);
    return { parent: await this.folder(parts.slice(0, -1), create), name: parts.at(-1) };
  }

  async matches(parent, name) {
    const matches = (await this.drive.listFolder(parent)).filter((entry) => entry.name === name);
    requireValue(matches.every((entry) => entry.type === "file"), "wrong_object_type");
    requireValue(matches.length <= 16, "duplicate_file_limit");
    return matches;
  }

  async get(key, expectation) {
    const { parent, name } = await this.location(key);
    if (parent === null) return null;
    const matches = await this.matches(parent, name);
    if (matches.length === 0) return null;
    let bytes;
    // Verify every duplicate. Do not select a good copy and hide corruption.
    for (const entry of matches) bytes = await this.drive.download(entry.token, expectation);
    return bytes;
  }

  async put(key, bytes, options) {
    const { parent, name } = await this.location(key, true);
    const matches = await this.matches(parent, name);
    const expectation = expected(bytes);
    if (matches.length > 0) {
      for (const entry of matches) await this.drive.download(entry.token, expectation);
      return "already_present";
    }
    const written = await this.drive.upload(parent, name, bytes, options);
    await this.drive.download(written, expectation);
    const after = await this.matches(parent, name);
    requireValue(after.some((entry) => entry.token === written), "file_visibility_gap");
    for (const entry of after) await this.drive.download(entry.token, expectation);
    return "written";
  }

  async listKeys(prefix) {
    // Prefixes may stop at repositories, _commits, or one stream.
    requireValue(typeof prefix === "string" && /^\.mallard\/v1\/(repositories|library)(\/|$)/u.test(prefix),
      "invalid_prefix");
    const parts = prefix.split("/");
    parts.forEach(fileName);
    const root = await this.folder(parts);
    if (root === null) return [];
    const queue = [{ folder: root, key: prefix, depth: parts.length }];
    const visited = new Set();
    const keys = new Set();
    for (let index = 0; index < queue.length; index += 1) {
      const next = queue[index];
      requireValue(!visited.has(next.folder) && visited.size < MAX_TREE_FOLDERS && next.depth <= 12,
        "folder_traversal_limit");
      visited.add(next.folder);
      const entries = await this.drive.listFolder(next.folder);
      const directories = new Map();
      for (const entry of entries) {
        const key = `${next.key}/${entry.name}`;
        try { fileName(entry.name); } catch { continue; }
        if (entry.type === "folder") {
          requireValue(!directories.has(entry.name)
            && !entries.some((other) => other.name === entry.name && other.type !== "folder"), "ambiguous_folder");
          directories.set(entry.name, entry.token);
          queue.push({ folder: entry.token, key, depth: next.depth + 1 });
          requireValue(queue.length <= MAX_TREE_FOLDERS, "folder_traversal_limit");
        } else if (entry.type === "file") {
          try { keyParts(key); } catch { continue; }
          keys.add(key);
          requireValue(keys.size <= 100_000, "tree_entry_limit");
        }
      }
    }
    return [...keys].sort();
  }
}

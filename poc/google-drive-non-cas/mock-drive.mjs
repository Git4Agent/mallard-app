// In-process HTTP mock. It parses actual Drive URLs and multipart bodies.
// It has no network, OAuth, permission inheritance, or consistency SLA.
import { sha256 } from "./drive-store.mjs";

const reply = (value, status = 200) => new Response(JSON.stringify(value), {
  status, headers: { "Content-Type": "application/json" },
});

export class MockDrive {
  constructor() {
    this.files = new Map();
    this.requests = [];
    this.nextId = 0;
    this.hidden = new Set();
    this.incomplete = false;
    this.emptyFirstPage = false;
    this.repeatCursor = false;
    this.rejectedCursor = false;
    this.loseNextWrite = false;
    this.failNextWrite = false;
    this.failLists = false;
    this.afterUpload = null;
    this.fetch = this.fetch.bind(this);
  }

  add(prepared) {
    const metadata = structuredClone(prepared.metadata);
    this.files.set(prepared.id, { metadata: { ...metadata, trashed: false,
      size: String(prepared.bytes.length) }, bytes: Buffer.from(prepared.bytes) });
  }

  async fetch(input, init = {}) {
    const url = new URL(input);
    const method = init.method ?? "GET";
    const headers = new Headers(init.headers);
    const token = headers.get("Authorization");
    // Store only a fixed mock actor label, never caller-supplied credentials.
    const actor = token === "Bearer fixture-b" ? "b" : "a";
    this.requests.push({ method, path: url.pathname, params: Object.fromEntries(url.searchParams), actor });
    if (!token || init.redirect !== "error") return reply({}, 401);
    if (headers.has("If-Match") || headers.has("If-None-Match") || method === "PATCH" || method === "DELETE") return reply({}, 400);
    if (url.pathname === "/drive/v3/files/generateIds") {
      return reply({ ids: [`generated_${++this.nextId}`], space: url.searchParams.get("space") });
    }
    if (url.pathname === "/upload/drive/v3/files" && method === "POST") {
      if (url.searchParams.get("uploadType") !== "multipart") return reply({}, 400);
      const boundary = /boundary=([a-z0-9_]+)/u.exec(headers.get("Content-Type"))?.[1];
      if (!boundary) return reply({}, 400);
      const body = Buffer.from(init.body);
      const headerEnd = body.indexOf("\r\n\r\n");
      const metadataEnd = body.indexOf(`\r\n--${boundary}`, headerEnd + 4);
      const metadata = JSON.parse(body.subarray(headerEnd + 4, metadataEnd));
      const dataStart = body.indexOf("\r\n\r\n", metadataEnd + 4) + 4;
      const dataEnd = body.lastIndexOf(`\r\n--${boundary}--\r\n`);
      if (dataEnd < dataStart) return reply({}, 400);
      if (this.failNextWrite) { this.failNextWrite = false; return reply({}, 503); }
      if (this.files.has(metadata.id)) return reply({}, 409);
      this.add({ id: metadata.id, metadata, bytes: body.subarray(dataStart, dataEnd) });
      await this.afterUpload?.(metadata);
      if (this.loseNextWrite) { this.loseNextWrite = false; throw new Error("simulated lost reply"); }
      return reply({ id: metadata.id });
    }
    if (url.pathname === "/drive/v3/files" && method === "POST") {
      const metadata = JSON.parse(init.body);
      if (this.files.has(metadata.id)) return reply({}, 409);
      this.files.set(metadata.id, { metadata, bytes: Buffer.alloc(0) });
      return reply({ id: metadata.id });
    }
    if (url.pathname === "/drive/v3/files" && method === "GET") {
      if (this.failLists) return reply({}, 503);
      if (this.rejectedCursor && url.searchParams.has("pageToken")) return reply({}, 400);
      const query = url.searchParams.get("q");
      const root = /^'([A-Za-z0-9_-]+)' in parents and trashed = false/u.exec(query)?.[1];
      const filters = [...query.matchAll(/properties has \{ key='([^']+)' and value='([^']+)' \}/gu)];
      if (!root || filters.length !== 2) return reply({}, 400);
      let files = [...this.files.values()].map((file) => file.metadata)
        .filter((f) => f.parents?.includes(root) && !f.trashed && !this.hidden.has(`${actor}:${f.id}`)
          && filters.every(([, key, value]) => f.properties?.[key] === value));
      // Deliberately do not sort by name. Callers must not trust API order.
      files = files.reverse();
      if (this.emptyFirstPage && !url.searchParams.has("pageToken")) {
        return reply({ files: [], nextPageToken: "offset_0", incompleteSearch: this.incomplete });
      }
      const cursor = url.searchParams.get("pageToken");
      const offset = cursor ? Number(cursor.slice(7)) : 0;
      if (!Number.isSafeInteger(offset)) return reply({}, 400);
      const size = Number(url.searchParams.get("pageSize"));
      const end = Math.min(offset + size, files.length);
      return reply({ files: files.slice(offset, end), incompleteSearch: this.incomplete,
        ...(this.repeatCursor ? { nextPageToken: "offset_0" }
          : end < files.length ? { nextPageToken: `offset_${end}` } : {}) });
    }
    const match = /^\/drive\/v3\/files\/([A-Za-z0-9_-]+)$/u.exec(url.pathname);
    if (match && method === "GET") {
      const file = this.files.get(match[1]);
      if (!file) return reply({}, 404);
      if (url.searchParams.get("alt") === "media") return new Response(file.bytes);
      return reply({ ...file.metadata, sha256Checksum: sha256(file.bytes),
        capabilities: { canAddChildren: true } });
    }
    return reply({}, 404);
  }
}

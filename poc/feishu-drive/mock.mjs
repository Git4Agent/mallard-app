import assert from "node:assert/strict";
import { API_ROOT, MAX_FOLDER_ENTRIES, SMALL_FILE_BYTES } from "./drive.mjs";

const ok = (data = {}) => Response.json({ code: 0, msg: "success", data });
export const apiError = (code, status = 200) => Response.json({ code, msg: "fixture_error" }, { status });

// An in-memory HTTP fixture, not a Feishu emulator or evidence about its
// consistency. It deliberately permits duplicates and delayed listings.
export class MockDrive {
  constructor() {
    this.entries = new Map();
    this.sessions = new Map();
    this.requests = [];
    this.hidden = new Set();
    this.sequence = 0;
    this.hook = null;
    this.afterUpload = null;
    this.add("", "test-root", "folder", undefined, "root");
  }

  add(parent, name, type, bytes, forcedToken) {
    const token = forcedToken ?? `node${++this.sequence}`;
    this.entries.set(token, { token, parent_token: parent, name, type, bytes });
    return token;
  }

  fetch = async (url, options) => {
    assert.ok(url.startsWith(`${API_ROOT}/files`));
    assert.equal(options.redirect, "error");
    assert.ok(options.signal instanceof AbortSignal);
    assert.ok(options.headers.Authorization.startsWith("Bearer fixture-"));
    assert.equal(options.headers["If-Match"], undefined);
    assert.equal(options.headers["If-None-Match"], undefined);
    // Round-trip native multipart serialization; inspect decoded wire fields.
    const request = new Request(url, options);
    const parsed = new URL(url);
    const path = parsed.pathname.slice("/open-apis/drive/v1".length);
    const call = { path, method: options.method, query: parsed.searchParams };
    this.requests.push(call);
    if (this.hook) {
      const response = await this.hook(call, request);
      if (response) return response;
    }
    if (path === "/files" && options.method === "GET") {
      assert.ok(parsed.searchParams.get("folder_token"));
      assert.equal(parsed.searchParams.get("order_by"), "CreatedTime");
      assert.equal(parsed.searchParams.get("direction"), "ASC");
      const parent = parsed.searchParams.get("folder_token");
      if (!this.entries.has(parent)) return apiError(1061003, 404);
      const all = [...this.entries.values()].filter((entry) => entry.parent_token === parent && !this.hidden.has(entry.token));
      const start = Number(parsed.searchParams.get("page_token") ?? 0);
      const size = Number(parsed.searchParams.get("page_size"));
      const hasMore = start + size < all.length;
      return ok({ files: all.slice(start, start + size).map(({ bytes: _, ...entry }) => entry),
        has_more: hasMore, ...(hasMore ? { next_page_token: String(start + size) } : {}) });
    }
    if (path === "/files/create_folder") {
      assert.equal(options.method, "POST");
      const body = await request.json();
      assert.deepEqual(Object.keys(body).sort(), ["folder_token", "name"]);
      const error = this.checkParent(body.folder_token);
      if (error) return error;
      const token = this.add(body.folder_token, body.name, "folder");
      return ok({ token, url: "https://example.invalid/not-followed" });
    }
    if (path === "/files/upload_all") {
      assert.equal(options.method, "POST");
      const form = await request.formData();
      assert.deepEqual([...form.keys()].sort(), ["file", "file_name", "parent_node", "parent_type", "size"]);
      assert.equal(form.get("parent_type"), "explorer");
      const bytes = Buffer.from(await form.get("file").arrayBuffer());
      assert.equal(bytes.length, Number(form.get("size")));
      assert.ok(bytes.length <= SMALL_FILE_BYTES);
      const error = this.checkParent(form.get("parent_node"));
      if (error) return error;
      const token = this.add(form.get("parent_node"), form.get("file_name"), "file", bytes);
      if (this.afterUpload) await this.afterUpload(token, this.entries.get(token));
      return ok({ file_token: token });
    }
    if (path === "/files/upload_prepare") {
      const body = await request.json();
      assert.equal(body.parent_type, "explorer");
      const error = this.checkParent(body.parent_node);
      if (error) return error;
      const uploadId = `upload${++this.sequence}`;
      this.sessions.set(uploadId, { ...body, parts: [] });
      return ok({ upload_id: uploadId, block_size: 4 * 1024 * 1024, block_num: Math.ceil(body.size / (4 * 1024 * 1024)) });
    }
    if (path === "/files/upload_part") {
      const form = await request.formData();
      const session = this.sessions.get(form.get("upload_id"));
      assert.ok(session);
      assert.equal(Number(form.get("seq")), session.parts.length);
      const bytes = Buffer.from(await form.get("file").arrayBuffer());
      assert.equal(bytes.length, Number(form.get("size")));
      assert.ok(bytes.length > 0 && bytes.length <= 4 * 1024 * 1024);
      session.parts.push(bytes);
      return ok();
    }
    if (path === "/files/upload_finish") {
      const body = await request.json();
      const session = this.sessions.get(body.upload_id);
      assert.ok(session);
      assert.equal(body.block_num, session.parts.length);
      const bytes = Buffer.concat(session.parts);
      assert.equal(bytes.length, session.size);
      const token = this.add(session.parent_node, session.file_name, "file", bytes);
      if (this.afterUpload) await this.afterUpload(token, this.entries.get(token));
      return ok({ file_token: token });
    }
    const download = /^\/files\/([^/]+)\/download$/u.exec(path);
    if (download && options.method === "GET") {
      const entry = this.entries.get(download[1]);
      if (!entry) return apiError(1061003, 404);
      assert.equal(entry.type, "file");
      return new Response(entry.bytes, { headers: { "Content-Type": "application/octet-stream" } });
    }
    throw new Error("unexpected_fixture_request");
  };

  checkParent(parent) {
    if (this.entries.get(parent)?.type !== "folder") return apiError(1061003, 404);
    if ([...this.entries.values()].filter((entry) => entry.parent_token === parent).length >= MAX_FOLDER_ENTRIES) {
      return apiError(1062507, 400);
    }
    return null;
  }
}

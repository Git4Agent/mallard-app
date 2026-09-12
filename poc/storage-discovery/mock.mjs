import assert from "node:assert/strict";
import { discoverDropbox, discoverR2, publicReport } from "./discovery.mjs";
import { MARKER_KEY } from "./http.mjs";

export const ACCOUNT = "0123456789abcdef0123456789abcdef";
export const TOKEN = "poc-secret-canary";
export const MARKER = { format: "mallard-storage", layout_version: 1 };
export const jsonResponse = (value, status = 200) => new Response(JSON.stringify(value), { status });

export function createMock() {
  const calls = [];
  const fetchImpl = async (input, init) => {
    const url = new URL(input);
    calls.push({ url: url.toString(), method: init.method, body: init.body });
    assert.equal(init.redirect, "error");
    assert.equal(init.headers.Authorization, `Bearer ${TOKEN}`);
    if (url.origin === "https://api.cloudflare.com") {
      assert.equal(init.method, "GET");
      assert.equal(init.headers["cf-r2-jurisdiction"], "default");
      const base = `/client/v4/accounts/${ACCOUNT}/r2/buckets`;
      if (url.pathname === base) {
        const second = url.searchParams.get("cursor") === "r2-next";
        return jsonResponse({ success: true, result: { buckets: (second
          ? ["mallard-empty", "restricted-store"] : ["renamed-backup"]).map((name) => ({ name })) },
        ...(second ? {} : { result_info: { cursor: "r2-next" } }) });
      }
      if (url.pathname === `${base}/renamed-backup/objects/${MARKER_KEY}`) return jsonResponse(MARKER);
      if (url.pathname === `${base}/mallard-empty/objects/${MARKER_KEY}`) return jsonResponse({}, 404);
      if (url.pathname === `${base}/restricted-store/objects/${MARKER_KEY}`) return jsonResponse({ error: TOKEN }, 403);
    }
    if (url.origin === "https://api.dropboxapi.com") {
      assert.equal(init.method, "POST");
      const folder = (id, name, access, mounted = true) => ({ shared_folder_id: id, name,
        access_type: { ".tag": access }, ...(mounted ? { path_lower: `/private/${name}` } : {}) });
      if (url.pathname === "/2/users/get_current_account") return jsonResponse({
        root_info: { ".tag": "user", home_namespace_id: "home-1", root_namespace_id: "home-1" } });
      if (url.pathname === "/2/sharing/list_folders") return jsonResponse({
        entries: [folder("share-1", "Renamed archive", "owner")], cursor: "dbx-next" });
      if (url.pathname === "/2/sharing/list_folders/continue") return jsonResponse({
        entries: [folder("share-2", "Mallard empty", "editor")] });
      if (url.pathname === "/2/sharing/list_mountable_folders") return jsonResponse({
        entries: [folder("share-1", "Renamed archive", "owner"), folder("share-3", "Unmounted archive", "viewer", false)] });
    }
    if (url.origin === "https://content.dropboxapi.com" && url.pathname === "/2/files/download") {
      assert.equal(init.method, "POST");
      assert.deepEqual(JSON.parse(init.headers["Dropbox-API-Arg"]), { path: `/${MARKER_KEY}` });
      const root = JSON.parse(init.headers["Dropbox-API-Path-Root"]);
      assert.equal(root[".tag"], "namespace_id");
      if (root.namespace_id === "share-1" || root.namespace_id === "share-3") return jsonResponse(MARKER);
      if (root.namespace_id === "share-2") return jsonResponse({
        error: { ".tag": "path", path: { ".tag": "not_found" } } }, 409);
    }
    throw new Error("Mock rejected an unexpected route or mutation.");
  };
  return { fetchImpl, calls };
}

export async function runMock() {
  const mock = createMock();
  const r2 = await discoverR2({ accountId: ACCOUNT, token: TOKEN, fetchImpl: mock.fetchImpl });
  const dropbox = await discoverDropbox({ token: TOKEN, fetchImpl: mock.fetchImpl });
  assert.deepEqual(r2.candidates.map((candidate) => candidate.state), ["mallard_storage", "marker_missing", "unreadable"]);
  assert.deepEqual(dropbox.candidates.map((candidate) => candidate.state), ["mallard_storage", "marker_missing", "mallard_storage"]);
  assert.equal(dropbox.candidates[2].mounted, false);
  return { mode: "mock", verdict: "POC_PASS", reports: [publicReport(r2, "mock"), publicReport(dropbox, "mock")] };
}

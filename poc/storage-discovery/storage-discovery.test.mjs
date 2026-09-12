import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { authorizeDropbox, DISCOVERY_SCOPES, newDropboxAuthorization, parseWranglerAuth, validateDiscoveryToken } from "./auth.mjs";
import { discoverDropbox, discoverR2, publicReport } from "./discovery.mjs";
import { MARKER_KEY, MAX_MARKER_BYTES, markerState, request } from "./http.mjs";
import { ACCOUNT, TOKEN, MARKER, createMock, jsonResponse, runMock } from "./mock.mjs";

test("mock proves discovery, pagination, deduplication, and denied marker access", async () => {
  const result = await runMock();
  assert.equal(result.verdict, "POC_PASS");
  assert.equal(result.reports[0].verdict, "SCAN_PARTIAL");
  assert.equal(result.reports[0].counts.mallard_storage, 1);
  assert.equal(result.reports[1].verdict, "SCAN_COMPLETE");
  assert.equal(result.reports[1].counts.mallard_storage, 2);
});

test("marker constants match the current Rust contract", async () => {
  const marker = await readFile(new URL("../../crates/mallard-infrastructure/src/storage_core/marker.rs", import.meta.url), "utf8");
  const core = await readFile(new URL("../../crates/mallard-infrastructure/src/storage_core/mod.rs", import.meta.url), "utf8");
  assert.match(marker, /STORAGE_MARKER_FORMAT: &str = "mallard-storage"/u);
  assert.match(marker, /STORAGE_LAYOUT_VERSION: u32 = 1/u);
  assert.match(core, /MAX_STORAGE_MARKER_BYTES: u64 = 4 \* 1024/u);
  assert.ok(core.includes(`"${MARKER_KEY}"`));
  assert.equal(MAX_MARKER_BYTES, 4096);
});

for (const [name, bytes, state] of [
  ["valid", JSON.stringify(MARKER), "mallard_storage"],
  ["unknown fields accepted by Rust", JSON.stringify({ ...MARKER, extra: true }), "mallard_storage"],
  ["future layout", JSON.stringify({ ...MARKER, layout_version: 2 }), "unsupported_layout"],
  ["wrong format", JSON.stringify({ format: "other", layout_version: 1 }), "invalid_marker"],
  ["missing version", JSON.stringify({ format: "mallard-storage" }), "invalid_marker"],
  ["string version", JSON.stringify({ ...MARKER, layout_version: "1" }), "invalid_marker"],
  ["negative version", JSON.stringify({ ...MARKER, layout_version: -1 }), "invalid_marker"],
  ["out of range version", JSON.stringify({ ...MARKER, layout_version: 2 ** 32 }), "invalid_marker"],
  ["bad JSON", "{", "invalid_marker"],
  ["large marker", " ".repeat(MAX_MARKER_BYTES + 1), "invalid_marker"],
]) {
  test(`marker: ${name}`, () => assert.equal(markerState(Buffer.from(bytes)), state));
}

test("R2 scans renamed storage and does not accept a name match without a marker", async () => {
  const mock = createMock();
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN, fetchImpl: mock.fetchImpl });
  assert.equal(result.candidates[0].name, "renamed-backup");
  assert.equal(result.candidates[0].state, "mallard_storage");
  assert.equal(result.candidates[1].name, "mallard-empty");
  assert.equal(result.candidates[1].state, "marker_missing");
  assert.ok(mock.calls.every((call) => call.method === "GET"));
  assert.ok(mock.calls.every((call) => !new URL(call.url).searchParams.has("name_contains")));
});

test("R2 preserves EU jurisdiction in each request and selected destination", async () => {
  const calls = [];
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN, jurisdiction: "eu",
    fetchImpl: async (url, init) => {
      calls.push(String(url));
      assert.equal(init.headers["cf-r2-jurisdiction"], "eu");
      return String(url).includes("/objects/") ? jsonResponse(MARKER)
        : jsonResponse({ success: true, result: { buckets: [{ name: "team-store", jurisdiction: "eu" }] } });
    } });
  assert.equal(calls.length, 2);
  assert.equal(result.candidates[0].destination.api_url, `https://${ACCOUNT}.eu.r2.cloudflarestorage.com/team-store`);
});

test("R2 reports current app limit for US jurisdiction", async () => {
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN, jurisdiction: "us",
    fetchImpl: async (url) => String(url).includes("/objects/") ? jsonResponse(MARKER)
      : jsonResponse({ success: true, result: { buckets: [{ name: "team-store", jurisdiction: "us" }] } }) });
  assert.equal(result.candidates[0].state, "mallard_storage");
  assert.equal(result.candidates[0].supportedByApp, false);
});

test("R2 rejects a mismatched jurisdiction and unsafe bucket path", async () => {
  for (const bucket of [{ name: "team-store", jurisdiction: "eu" }, { name: "../other/objects" }]) {
    let calls = 0;
    const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN,
      fetchImpl: async () => { calls++; return jsonResponse({ success: true, result: { buckets: [bucket] } }); } });
    assert.equal(calls, 1);
    assert.equal(result.enumerationComplete, false);
    assert.equal(result.candidates.length, 0);
  }
});

test("R2 stops on a repeated cursor and preserves earlier matches", async () => {
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN,
    fetchImpl: async (url) => String(url).includes("/objects/") ? jsonResponse(MARKER)
      : jsonResponse({ success: true, result: { buckets: [{ name: "team-store" }] }, result_info: { cursor: "repeat" } }) });
  assert.equal(result.candidates.length, 1);
  assert.equal(result.candidates[0].state, "mallard_storage");
  assert.equal(result.issues[0].code, "repeated_cursor");
  assert.equal(publicReport(result, "mock").verdict, "SCAN_PARTIAL");
});

test("R2 limits candidates and pages without declaring a complete scan", async () => {
  for (const limits of [{ maxCandidates: 1 }, { maxPages: 1 }]) {
    const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN, ...limits,
      fetchImpl: createMock().fetchImpl });
    assert.equal(result.enumerationComplete, false);
    assert.equal(result.candidates[0].state, "mallard_storage");
    assert.match(result.issues[0].code, /limit_reached$/u);
  }
});

test("R2 authorization failure is not an empty successful scan", async () => {
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN,
    fetchImpl: async () => jsonResponse({ message: TOKEN }, 403) });
  assert.equal(result.issues[0].code, "access_denied");
  assert.equal(publicReport(result, "mock").verdict, "SCAN_PARTIAL");
});

test("R2 distinguishes oversized markers from oversized server errors", async () => {
  for (const [status, state] of [[200, "invalid_marker"], [503, "unreadable"]]) {
    const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN,
      fetchImpl: async (url) => String(url).includes("/objects/")
        ? new Response(" ".repeat(17 * 1024), { status })
        : jsonResponse({ success: true, result: { buckets: [{ name: "team-store" }] } }) });
    assert.equal(result.candidates[0].state, state);
  }
});

test("R2 rate limits stop enumeration without an automatic retry", async () => {
  let calls = 0;
  const result = await discoverR2({ accountId: ACCOUNT, token: TOKEN,
    fetchImpl: async () => { calls++; return jsonResponse({}, 429); } });
  assert.equal(calls, 1);
  assert.equal(result.issues[0].code, "rate_limited");
  assert.equal(publicReport(result, "mock").verdict, "SCAN_PARTIAL");
});

test("Dropbox deduplicates namespaces and reads unmounted shares without mounting", async () => {
  const mock = createMock();
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: mock.fetchImpl });
  assert.equal(result.candidates.length, 3);
  assert.equal(result.candidates[2].access, "viewer");
  assert.equal(result.candidates[2].mounted, false);
  assert.equal(mock.calls.filter((call) => call.url.includes("/files/download")).length, 3);
  assert.ok(mock.calls.every((call) => !/mount_folder|create_folder|upload|delete|share_folder/u.test(call.url)));
});

test("Dropbox missing sharing.read is reported as incomplete", async () => {
  const mock = createMock();
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: (url, init) =>
    String(url).includes("/sharing/") ? jsonResponse({ error: { ".tag": "missing_scope", required_scope: "sharing.read" } }, 401)
      : mock.fetchImpl(url, init) });
  assert.equal(result.enumerationComplete, false);
  assert.equal(result.issues[0].code, "missing_scope");
});

test("Dropbox probes a team root but never a personal root", async () => {
  const mock = createMock();
  const roots = [];
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: (url, init) => {
    if (String(url).endsWith("users/get_current_account")) return jsonResponse({
      root_info: { ".tag": "team", home_namespace_id: "home-1", root_namespace_id: "team-1" } });
    if (String(url).endsWith("files/download")) {
      roots.push(JSON.parse(init.headers["Dropbox-API-Path-Root"]).namespace_id);
      return jsonResponse(MARKER);
    }
    return mock.fetchImpl(url, init);
  } });
  assert.equal(result.candidates[0].kind, "team_root");
  assert.ok(roots.includes("team-1"));
  assert.ok(!roots.includes("home-1"));
});

test("Dropbox rejects a listed personal namespace before reading its marker", async () => {
  const mock = createMock();
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: (url, init) =>
    String(url).includes("/sharing/") ? jsonResponse({ entries: [{ name: "Private home", shared_folder_id: "home-1" }] })
      : mock.fetchImpl(url, init) });
  assert.equal(result.candidates.length, 0);
  assert.equal(result.issues[0].code, "personal_namespace_rejected");
});

test("Dropbox continues mountable-folder pagination", async () => {
  const mock = createMock();
  let continued = false;
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: (url, init) => {
    if (String(url).endsWith("sharing/list_mountable_folders")) return jsonResponse({ entries: [], cursor: "mount-next" });
    if (String(url).endsWith("sharing/list_mountable_folders/continue")) {
      continued = true;
      assert.deepEqual(JSON.parse(init.body), { cursor: "mount-next" });
      return jsonResponse({ entries: [] });
    }
    return mock.fetchImpl(url, init);
  } });
  assert.ok(continued);
  assert.equal(result.enumerationComplete, true);
});

test("Dropbox distinguishes a missing marker from a different 409 failure", async () => {
  const mock = createMock();
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: (url, init) =>
    String(url).endsWith("files/download") ? jsonResponse({ error: { ".tag": "path", path: { ".tag": "restricted_content" } } }, 409)
      : mock.fetchImpl(url, init) });
  assert.ok(result.candidates.every((candidate) => candidate.state === "unreadable"));
  assert.equal(publicReport(result, "mock").counts.marker_missing, 0);
});

test("report excludes credentials, account IDs, namespace IDs, names, and paths", async () => {
  const mock = createMock();
  const result = await discoverDropbox({ token: TOKEN, fetchImpl: mock.fetchImpl });
  const output = JSON.stringify(publicReport(result, "mock"));
  for (const privateValue of [TOKEN, ACCOUNT, "share-1", "Renamed archive", "/private/"]) assert.ok(!output.includes(privateValue));
  assert.equal(JSON.parse(output).remoteWrites, 0);
  assert.equal(JSON.parse(output).localConfigWrites, 0);
});

test("transport bounds a streamed body even when no content length is given", async () => {
  await assert.rejects(request("https://api.cloudflare.com/", { method: "GET" }, {
    maxBytes: 4, fetchImpl: async () => new Response(new ReadableStream({ start(controller) {
      controller.enqueue(new Uint8Array(3)); controller.enqueue(new Uint8Array(3)); controller.close();
    } })),
  }), (error) => error.code === "response_too_large");
});

test("transport timeout includes body reads and redacts the error", async () => {
  const keepAlive = setInterval(() => {}, 100);
  try {
    await assert.rejects(request("https://api.cloudflare.com/", { method: "GET" }, {
      timeoutMs: 10, fetchImpl: async (_url, init) => new Response(new ReadableStream({ start(controller) {
        init.signal.addEventListener("abort", () => controller.error(new Error(TOKEN)), { once: true });
      } })),
    }), (error) => error.code === "request_timeout" && !String(error).includes(TOKEN));
  } finally { clearInterval(keepAlive); }
});

test("transport rejects redirects and redacts network errors", async () => {
  await assert.rejects(request("https://api.cloudflare.com/", { method: "GET" }, {
    fetchImpl: async (_url, init) => { assert.equal(init.redirect, "error"); throw new Error(TOKEN); },
  }), (error) => error.code === "transport_failed" && !String(error).includes(TOKEN));
});

test("Wrangler selects only a verified account and an OAuth token", () => {
  const account = { id: ACCOUNT };
  const whoami = { loggedIn: true, accounts: [account] };
  assert.deepEqual(parseWranglerAuth(whoami, { type: "oauth", token: TOKEN }), { accountId: ACCOUNT, token: TOKEN });
  assert.throws(() => parseWranglerAuth(whoami, { type: "api_token", token: TOKEN }));
  assert.throws(() => parseWranglerAuth({ ...whoami, accounts: [account, { id: "f".repeat(32) }] }, { type: "oauth", token: TOKEN }));
  assert.throws(() => parseWranglerAuth(whoami, { type: "oauth", token: TOKEN }, "f".repeat(32)));
});

test("Dropbox PKCE requests short-lived read scopes and rejects callback replay", () => {
  const flow = newDropboxAuthorization();
  const url = new URL(flow.url);
  assert.equal(url.searchParams.get("scope"), DISCOVERY_SCOPES.join(" "));
  assert.equal(url.searchParams.get("token_access_type"), "online");
  assert.equal(url.searchParams.get("code_challenge_method"), "S256");
  const callback = new URL(url.searchParams.get("redirect_uri"));
  callback.search = new URLSearchParams({ code: TOKEN, state: "wrong" }).toString();
  assert.throws(() => flow.consume(callback), (error) => error.code === "invalid_callback_state");
  callback.searchParams.set("state", url.searchParams.get("state"));
  const grant = flow.consume(callback);
  assert.equal(createHash("sha256").update(grant.verifier).digest("base64url"), url.searchParams.get("code_challenge"));
  assert.throws(() => flow.consume(callback), (error) => error.code === "callback_already_used");
});

test("Dropbox accepts all four read scopes and rejects added write scopes", () => {
  const grant = { access_token: TOKEN, token_type: "bearer", expires_in: 14400 };
  assert.equal(validateDiscoveryToken({ ...grant, scope: DISCOVERY_SCOPES.join("  ") }), TOKEN);
  assert.equal(validateDiscoveryToken(grant), TOKEN);
  assert.throws(() => validateDiscoveryToken({ ...grant, scope: `${DISCOVERY_SCOPES.join(" ")} sharing.write` }),
    (error) => error.code === "unexpected_token_scope" && error.scopeEvidence.writeScopePresent);
  assert.throws(() => validateDiscoveryToken({ ...grant, scope: "account_info.read" }),
    (error) => error.scopeEvidence.missing.includes("sharing.read"));
  assert.throws(() => validateDiscoveryToken({ ...grant, scope: [TOKEN] }),
    (error) => error.scopeEvidence.format === "array" && !JSON.stringify(error.scopeEvidence).includes(TOKEN));
  assert.throws(() => validateDiscoveryToken({ ...grant, refresh_token: TOKEN }));
});

test("real loopback callback completes a mock read-only token exchange", async () => {
  let authorization;
  const token = await authorizeDropbox({ port: 0, timeoutMs: 1000,
    openBrowserImpl: async (input) => {
      authorization = new URL(input);
      const callback = new URL(authorization.searchParams.get("redirect_uri"));
      callback.search = new URLSearchParams({ state: authorization.searchParams.get("state"), code: "fixture-code" }).toString();
      const response = await fetch(callback);
      assert.equal(response.status, 200);
    }, fetchImpl: async (url, init) => {
      assert.equal(url, "https://api.dropboxapi.com/oauth2/token");
      const params = new URLSearchParams(init.body);
      assert.equal(params.get("code"), "fixture-code");
      assert.equal(params.get("grant_type"), "authorization_code");
      assert.equal(createHash("sha256").update(params.get("code_verifier")).digest("base64url"), authorization.searchParams.get("code_challenge"));
      assert.ok(!params.has("client_secret"));
      return jsonResponse({ access_token: TOKEN, token_type: "bearer", expires_in: 14400, scope: DISCOVERY_SCOPES.join(" ") });
    } });
  assert.equal(token, TOKEN);
});

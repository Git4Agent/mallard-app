import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createServer, request as httpRequest } from "node:http";
import { test } from "node:test";
import { MockDropbox } from "./mock-dropbox.mjs";
import { authorizeUser, CALLBACK_URI, inspectCreatedFolder, newAuthorization, PocError, runSetupPoc, STORAGE_SCOPES } from "./setup-poc.mjs";

async function start(t, faults = {}) {
  const mock = await MockDropbox.start(faults);
  t.after(() => mock.close());
  return mock;
}

function callback(flow, change = () => {}) {
  const initial = new URL(flow.url);
  const url = new URL(initial.searchParams.get("redirect_uri"));
  url.searchParams.set("state", initial.searchParams.get("state"));
  url.searchParams.set("code", "synthetic-code");
  change(url);
  return url.toString();
}

test("PKCE uses S256, offline scope, fresh state, and one callback", () => {
  const flow = newAuthorization("mockApp", CALLBACK_URI);
  const initial = new URL(flow.url);
  const accepted = flow.consume(callback(flow));
  assert.equal(initial.searchParams.get("code_challenge_method"), "S256");
  assert.equal(initial.searchParams.get("code_challenge"), createHash("sha256").update(accepted.verifier).digest("base64url"));
  assert.equal(initial.searchParams.get("token_access_type"), "offline");
  assert.equal(initial.searchParams.has("client_secret"), false);
  assert.notEqual(new URL(newAuthorization("mockApp", CALLBACK_URI).url).searchParams.get("state"), initial.searchParams.get("state"));
  assert.throws(() => flow.consume(callback(flow)), { code: "callback_already_used" });
});

for (const [name, mutate] of [
  ["wrong state", (url) => url.searchParams.set("state", "wrong")],
  ["duplicate state", (url) => url.searchParams.append("state", "wrong")],
  ["duplicate code", (url) => url.searchParams.append("code", "wrong")],
  ["code with error", (url) => url.searchParams.set("error", "access_denied")],
  ["wrong host", (url) => { url.hostname = "localhost"; }],
  ["wrong path", (url) => { url.pathname = "/wrong"; }],
]) {
  test(`callback rejects ${name} without consuming valid consent`, () => {
    const flow = newAuthorization("mockApp", CALLBACK_URI);
    assert.throws(() => flow.consume(callback(flow, mutate)), PocError);
    assert.equal(flow.consume(callback(flow)).code, "synthetic-code");
  });
}

test("loopback callback rejects Host injection and then accepts valid consent", async () => {
  const grant = await authorizeUser({ appKey: "mockApp", port: 0, timeoutMs: 1000,
    openBrowser: async (url) => {
      const auth = new URL(url);
      const target = new URL(auth.searchParams.get("redirect_uri"));
      target.searchParams.set("state", auth.searchParams.get("state"));
      target.searchParams.set("code", "synthetic-code");
      // Fetch normalizes Host. Use HTTP directly to exercise an actual bad Host.
      const wrongStatus = await new Promise((resolve, reject) => {
        const request = httpRequest(target, { headers: { Host: "attacker.invalid" } }, (response) => {
          response.resume();
          response.on("end", () => resolve(response.statusCode));
        });
        request.on("error", reject);
        request.end();
      });
      assert.equal(wrongStatus, 400);
      const valid = await fetch(target);
      assert.equal(valid.status, 200);
      assert.equal(valid.headers.get("cache-control"), "no-store");
      assert.equal((await valid.text()).includes("synthetic-code"), false);
    } });
  assert.equal(grant.code, "synthetic-code");
});

test("callback timeout closes the listener", async () => {
  let uri;
  await assert.rejects(authorizeUser({ appKey: "mockApp", port: 0, timeoutMs: 20,
    openBrowser: async (url) => { uri = new URL(url).searchParams.get("redirect_uri"); } }), { code: "authorization_timeout" });
  await assert.rejects(fetch(uri));
});

test("occupied callback port fails before browser launch", async (t) => {
  const occupied = createServer();
  await new Promise((resolve) => occupied.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => occupied.close(resolve)));
  let opened = false;
  await assert.rejects(authorizeUser({ appKey: "mockApp", port: occupied.address().port,
    openBrowser: async () => { opened = true; } }), { code: "callback_listener_unavailable" });
  assert.equal(opened, false);
});

test("browser launch failure closes the listener and returns fixed text", async () => {
  let uri;
  await assert.rejects(authorizeUser({ appKey: "mockApp", port: 0,
    openBrowser: async (url) => { uri = new URL(url).searchParams.get("redirect_uri"); throw new PocError("browser_open_failed"); } }),
  { code: "browser_open_failed" });
  await assert.rejects(fetch(uri));
});

test("HTTP POC creates and shares the exact ID, refreshes, and prepares schema 1 in memory", async (t) => {
  const mock = await start(t, { wrongStateFirst: true });
  const result = await runSetupPoc(mock.options());
  const fixture = JSON.parse(await readFile(new URL("./mock-provider-config.json", import.meta.url), "utf8"));
  assert.deepEqual(result.preparedConfig, fixture);
  assert.equal(result.oauth.refresh_token, mock.refreshToken);
  assert.equal(result.report.verdict, "POC_PASS");
  assert.equal(result.report.productionGate, "INCOMPLETE");
  assert.equal(result.report.storageCommitted, false);
  assert.equal(mock.invalidStateStatus, 400);
  const shared = mock.calls.find((call) => call.path === "/2/sharing/share_folder");
  assert.equal(shared.body.path, mock.folder.id);
  assert.equal(mock.calls.find((call) => call.path === "/2/files/create_folder_v2").body.autorename, false);
  const listing = mock.calls.find((call) => call.path === "/2/files/list_folder");
  assert.deepEqual(listing.root, { ".tag": "namespace_id", namespace_id: mock.namespaceId });
  assert.equal(listing.authorization, `Bearer ${mock.accessToken}`);
  const refresh = mock.calls.find((call) => call.body?.grant_type === "refresh_token");
  assert.equal(refresh.body.scope, STORAGE_SCOPES.join(" "));
  assert.equal(Object.hasOwn(refresh.body, "client_secret"), false);
  assert.equal(mock.calls.some((call) => /add_.*member|shared_link|delete|unshare|upload/u.test(call.path)), false);
  for (const value of [mock.code, mock.setupToken, mock.accessToken, mock.refreshToken, mock.namespaceId, mock.accountId, mock.folder.id]) {
    assert.equal(JSON.stringify(result.report).includes(value), false);
    assert.equal(mock.callbackText.includes(value), false);
  }
});

test("team member creates a child share in home but never stores in the home namespace", async (t) => {
  const mock = await start(t, { team: true });
  const result = await runSetupPoc(mock.options());
  assert.equal(result.preparedConfig.settings.namespace_id, mock.namespaceId);
  assert.notEqual(result.preparedConfig.settings.namespace_id, "mock-team-root");
  assert.notEqual(result.preparedConfig.settings.namespace_id, "mock-home");
});

test("folder inspection reads only the exact POC target and returns no private metadata", async (t) => {
  const mock = await start(t);
  const runId = "a".repeat(24);
  mock.folder = { id: "id:mock-folder", path_lower: `/mallard poc ${runId}`, name: `Mallard POC ${runId}` };
  mock.shared = true;
  const report = await inspectCreatedFolder(mock.options({ runId }));
  assert.equal(report.namespaceReadable, true);
  assert.equal(report.pathAndIdAgree, true);
  assert.equal(report.idUnchangedAfterRefresh, true);
  assert.equal(report.mutationRequests, 0);
  assert.equal(mock.calls.some((call) => /create|share_folder|delete|add_folder_member/u.test(call.path)), false);
  for (const value of [mock.accountId, mock.namespaceId, mock.folder.id, mock.accessToken, mock.refreshToken]) {
    assert.equal(JSON.stringify(report).includes(value), false);
  }
});

test("asynchronous share polls until the namespace is returned", async (t) => {
  const mock = await start(t, { async: true });
  assert.equal((await runSetupPoc(mock.options())).report.namespaceResolved, true);
  assert.equal(mock.polls, 2);
  assert.equal(mock.calls.filter((call) => call.path === "/2/sharing/share_folder").length, 1);
});

test("sharing can change the folder ID while the acknowledged namespace stays exact", async (t) => {
  const mock = await start(t, { changedFolderId: true });
  const result = await runSetupPoc(mock.options());
  assert.equal(result.report.folderIdChangedOnShare, true);
  assert.equal(result.preparedConfig.settings.namespace_id, mock.namespaceId);
  const metadata = mock.calls.find((call) => call.path === "/2/files/get_metadata");
  assert.equal(metadata.body.path.toLowerCase(), mock.folder.path_lower);
  assert.equal(mock.calls.filter((call) => call.path === "/2/sharing/share_folder").length, 1);
});

test("missing namespace or replacement identity fails without adopting the folder", async (t) => {
  const delayed = await start(t, { metadataDelay: 10 });
  await assert.rejects(runSetupPoc(delayed.options()), { code: "namespace_identity_mismatch" });
  assert.equal(delayed.metadataReads, 1);
  const replaced = await start(t, { changedFolderId: true, wrongNamespace: true });
  await assert.rejects(runSetupPoc(replaced.options()), (error) => {
    assert.equal(error.metadataChecks.namespaceMatches, false);
    return error.code === "namespace_identity_mismatch";
  });
  assert.equal(replaced.metadataReads, 1);
});

test("shared namespace must remain mounted at the exact requested path", async (t) => {
  const mock = await start(t, { wrongFolderPath: true });
  await assert.rejects(runSetupPoc(mock.options()), (error) => {
    assert.equal(error.metadataChecks.folderPathMatches, false);
    return error.code === "namespace_identity_mismatch";
  });
});

test("refresh response may omit unchanged requested scope", async (t) => {
  const mock = await start(t, { omitRefreshScope: true });
  assert.equal((await runSetupPoc(mock.options())).report.refreshVerified, true);
});

for (const [name, faults, code, remote] of [
  ["consent denied", { denied: true }, "authorization_denied", false],
  ["PKCE mismatch", { wrongVerifier: true }, "dropbox_request_rejected", false],
  ["refresh token absent", { missingRefresh: true }, "invalid_credential_response", false],
  ["sharing permission absent", { missingScope: true }, "required_scope_missing", false],
  ["account mismatch", { wrongAccount: true }, "account_mismatch", false],
  ["existing folder", { collision: true }, "dropbox_request_rejected", true],
  ["home namespace", { homeNamespace: true }, "invalid_shared_namespace", true],
  ["viewer access", { viewer: true }, "invalid_shared_namespace", true],
  ["refresh revoked", { revokedRefresh: true }, "dropbox_request_rejected", true],
  ["refresh account switched", { switchedRefreshAccount: true }, "account_mismatch", true],
  ["namespace mismatch", { wrongNamespace: true }, "namespace_identity_mismatch", true],
  ["asynchronous share fails", { async: true, asyncFailed: true }, "folder_share_incomplete", true],
  ["asynchronous share never finishes", { async: true, neverComplete: true }, "folder_share_incomplete", true],
  ["create response lost", { lostCreateResponse: true }, "dropbox_transport_failed", true],
  ["share response lost", { lostShareResponse: true }, "dropbox_transport_failed", true],
  ["redirect", { redirect: true }, "dropbox_transport_failed", false],
  ["rate limit", { status: 429, statusRoute: "/2/users/get_current_account" }, "dropbox_rate_limited", false],
  ["app permission denied", { status: 403, statusRoute: "/2/sharing/share_folder" }, "dropbox_request_rejected", true],
]) {
  test(`HTTP POC stops safely: ${name}`, async (t) => {
    const mock = await start(t, faults);
    await assert.rejects(runSetupPoc(mock.options()), (error) => {
      assert.equal(error.code, code);
      assert.equal(error.remoteFolderMayExist, remote);
      assert.equal(String(error).includes("canary"), false);
      if (faults.missingScope) {
        assert.deepEqual(error.missingScopes, ["sharing.write"]);
        assert.equal(error.scopeFormat, "string");
      }
      return true;
    });
    assert.equal(mock.calls.filter((call) => call.path === "/2/files/create_folder_v2").length <= 1, true);
    assert.equal(mock.calls.filter((call) => call.path === "/2/sharing/share_folder").length <= 1, true);
    assert.equal(mock.calls.some((call) => /delete|add_folder_member|must-not-follow/u.test(call.path)), false);
  });
}

test("response body bound rejects oversized JSON before folder creation", async (t) => {
  const mock = await start(t);
  await assert.rejects(runSetupPoc(mock.options({ fetchImpl: async (url, options) => {
    if (url.endsWith("users/get_current_account")) return new Response(JSON.stringify({ padding: "a".repeat(1024 * 1024) }));
    return mock.fetch(url, options);
  } })), { code: "response_too_large" });
  assert.equal(mock.folder, null);
});

test("request deadline stays active while reading a delayed HTTP body", async (t) => {
  const stalled = createServer((_request, response) => {
    response.writeHead(200, { "Content-Type": "application/json" });
    response.write("{");
  });
  await new Promise((resolve) => stalled.listen(0, "127.0.0.1", resolve));
  t.after(async () => {
    stalled.closeAllConnections();
    await new Promise((resolve) => stalled.close(resolve));
  });
  await assert.rejects(runSetupPoc({ appKey: "mockApp", requestTimeoutMs: 30,
    authorize: async () => ({ code: "mock-code", verifier: "mock-verifier", redirectUri: CALLBACK_URI }),
    fetchImpl: (_url, options) => fetch(`http://127.0.0.1:${stalled.address().port}`, options) }), { code: "dropbox_request_timeout" });
});

test("CLI mock prints only the safe result and does not use user homes", async () => {
  const child = spawn(process.execPath, [new URL("./setup-poc.mjs", import.meta.url).pathname, "mock"], {
    env: { ...process.env, HOME: "/nonexistent-poc-home", CODEX_HOME: "/nonexistent-poc-codex", CLAUDE_CONFIG_DIR: "/nonexistent-poc-claude" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (part) => { stdout += part; });
  child.stderr.on("data", (part) => { stderr += part; });
  const code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", resolve); });
  assert.equal(code, 0);
  assert.equal(stderr, "");
  assert.equal(stdout.includes("canary"), false);
  const result = JSON.parse(stdout);
  assert.equal(result.mode, "local_http_mock");
  assert.equal(result.storageCommitted, false);
  assert.equal(result.productionGate, "INCOMPLETE");
});

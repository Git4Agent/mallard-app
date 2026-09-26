import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { request } from "node:http";
import { PassThrough } from "node:stream";
import test from "node:test";
import { readTerminalSecret, startOAuthProbe, USER_SCOPES } from "./oauth-probe.mjs";

const secret = "synthetic-app-secret";
const accessToken = "synthetic-user-token";
const success = () => ({ code: 0, access_token: accessToken, token_type: "Bearer",
  expires_in: 3600, scope: USER_SCOPES.join(" ") });

async function fixture(t, options = {}) {
  const calls = [];
  let completed;
  const done = new Promise((resolve) => { completed = resolve; });
  const probe = await startOAuthProbe({ port: 0, parentFolderToken: "synthetic_folder",
    clientId: "cli_synthetic", clientSecret: secret,
    run: async (token, folder) => { calls.push({ token, folder }); return { verdict: "PROBE_PASS", production_ready: false }; },
    fetchImpl: async () => Response.json(success()), ...options, onResult: completed });
  t.after(async () => { probe.server.closeAllConnections(); await new Promise((resolve) => probe.server.close(resolve)); });
  const page = await fetch(probe.origin);
  assert.equal(page.headers.get("referrer-policy"), "no-referrer");
  assert.equal(page.headers.get("cache-control"), "no-store");
  const html = await page.text();
  assert.ok(!html.includes(secret));
  assert.ok(!html.includes("<form"));
  const post = () => fetch(`${probe.origin}/authorize`, {
    method: "POST", redirect: "manual",
  });
  const begin = async () => {
    const page = await (await fetch(probe.origin)).text();
    const href = page.match(/<a href="([^"]+)">Continue to Feishu/u)?.[1];
    assert.ok(href);
    assert.ok(!page.includes(secret));
    return new URL(href.replaceAll("&amp;", "&"));
  };
  const callback = (params) => fetch(`${probe.origin}/oauth/callback?${params}`, { redirect: "manual" });
  return { ...probe, post, begin, callback, calls, done };
}

test("OAuth listener rejects foreign hosts and has no credential POST endpoint", async (t) => {
  const probe = await fixture(t);
  assert.equal((await probe.post()).status, 405);
  const status = await new Promise((resolve, reject) => {
    const req = request(probe.origin, { headers: { Host: "untrusted.invalid" } }, (res) => {
      res.resume(); res.on("end", () => resolve(res.statusCode));
    });
    req.on("error", reject); req.end();
  });
  assert.equal(status, 403);
  assert.equal(probe.status().state, "authorizing");
});

test("OAuth v3 uses PKCE, exact scopes and one callback; only the user token reaches the probe", async (t) => {
  let authorize;
  let exchanges = 0;
  const probe = await fixture(t, { fetchImpl: async (url, options) => {
    exchanges += 1;
    assert.equal(url, "https://accounts.feishu.cn/oauth/v3/token");
    assert.equal(options.method, "POST");
    assert.equal(options.redirect, "error");
    assert.deepEqual(options.headers, { "Content-Type": "application/x-www-form-urlencoded" });
    assert.equal(options.body.get("client_id"), "cli_synthetic");
    assert.equal(options.body.get("client_secret"), secret);
    assert.equal(options.body.get("code"), "synthetic-code");
    assert.equal(options.body.get("redirect_uri"), `${probe.origin}/oauth/callback`);
    assert.equal(options.body.get("scope"), USER_SCOPES.join(" "));
    assert.equal(createHash("sha256").update(options.body.get("code_verifier")).digest("base64url"),
      authorize.searchParams.get("code_challenge"));
    return Response.json(success());
  } });
  authorize = await probe.begin();
  assert.equal(authorize.origin, "https://accounts.feishu.cn");
  assert.equal(authorize.pathname, "/oauth/v3/authorize");
  assert.equal(authorize.searchParams.get("code_challenge_method"), "S256");
  assert.equal(authorize.searchParams.get("scope"), USER_SCOPES.join(" "));
  assert.ok(!authorize.href.includes(secret));
  assert.equal((await probe.callback(new URLSearchParams({ state: "wrong", code: "synthetic-code" }))).status, 403);
  assert.equal(exchanges, 0);
  const params = new URLSearchParams({ state: authorize.searchParams.get("state"), code: "synthetic-code" });
  const duplicate = new URLSearchParams(params); duplicate.append("code", "another-code");
  assert.equal((await probe.callback(duplicate)).status, 400);
  const callback = await probe.callback(params);
  assert.equal(callback.status, 303);
  assert.equal(callback.headers.get("location"), "/");
  assert.equal((await probe.done).verdict, "PROBE_PASS");
  assert.equal((await probe.callback(params)).status, 409);
  assert.equal((await probe.post()).status, 405);
  assert.equal(exchanges, 1);
  assert.deepEqual(probe.calls, [{ token: accessToken, folder: "synthetic_folder" }]);
  const resultPage = await (await fetch(probe.origin)).text();
  for (const value of [secret, accessToken, "synthetic-code", 'name="app_secret"']) assert.ok(!resultPage.includes(value));
});

test("terminal secret entry does not echo input and restores terminal mode", async () => {
  const input = new PassThrough();
  input.isTTY = true;
  input.isRaw = false;
  input.setRawMode = (value) => { input.isRaw = value; };
  const output = new PassThrough();
  let transcript = "";
  output.on("data", (bytes) => { transcript += bytes.toString(); });
  const reading = readTerminalSecret(input, output);
  input.write(`${secret}\r`);
  assert.equal(await reading, secret);
  assert.ok(transcript.includes("input hidden"));
  assert.ok(!transcript.includes(secret));
  assert.equal(input.isRaw, false);
  input.destroy(); output.destroy();
});

test("terminal secret entry rejects pipes and supports cancellation", async () => {
  await assert.rejects(readTerminalSecret(new PassThrough(), new PassThrough()), /secret_requires_terminal/u);
  const input = new PassThrough();
  input.isTTY = true;
  input.setRawMode = () => {};
  const output = new PassThrough();
  const reading = readTerminalSecret(input, output);
  input.write("\u0003");
  await assert.rejects(reading, /secret_entry_cancelled/u);
  input.destroy(); output.destroy();
});

for (const [name, response, expected] of [
  ["additional scope", { ...success(), scope: `${USER_SCOPES.join(" ")} offline_access` }, "oauth_scope_mismatch"],
  ["missing scope", { ...success(), scope: USER_SCOPES.slice(1).join(" ") }, "oauth_scope_mismatch"],
  ["refresh token", { ...success(), refresh_token: "synthetic-refresh-token" }, "unexpected_refresh_token"],
  ["API failure", { code: 20002, error_description: secret }, "oauth_20002"],
]) {
  test(`OAuth rejects ${name} before Drive access and does not expose raw response`, async (t) => {
    const probe = await fixture(t, { fetchImpl: async () => Response.json(response) });
    const authorize = await probe.begin();
    await probe.callback(new URLSearchParams({ state: authorize.searchParams.get("state"), code: "synthetic-code" }));
    assert.equal((await probe.done).code, expected);
    assert.equal(probe.calls.length, 0);
    const page = await (await fetch(probe.origin)).text();
    for (const value of [secret, accessToken, "synthetic-refresh-token"]) assert.ok(!page.includes(value));
  });
}

test("OAuth denial and expired authorization cannot exchange credentials", async (t) => {
  for (const denied of [false, true]) {
    let clock = 0;
    let exchanges = 0;
    const probe = await fixture(t, { now: () => clock, fetchImpl: async () => { exchanges += 1; } });
    const authorize = await probe.begin();
    if (!denied) clock = 300_001;
    await probe.callback(new URLSearchParams({ state: authorize.searchParams.get("state"),
      ...(denied ? { error: "access_denied" } : { code: "synthetic-code" }) }));
    assert.equal((await probe.done).code, denied ? "authorization_denied" : "authorization_expired");
    assert.equal(exchanges, 0);
    assert.equal(probe.calls.length, 0);
  }
});

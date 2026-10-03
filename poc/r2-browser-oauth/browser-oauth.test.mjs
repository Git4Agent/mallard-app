import assert from "node:assert/strict";
import { request as httpRequest } from "node:http";
import { createHash } from "node:crypto";
import test from "node:test";
import { CALLBACK_PATH, newAuthorization, startPoc } from "./browser-oauth.mjs";
import { startMockProvider } from "./mock-provider.mjs";
import { issuedS3Key, keyPolicy } from "./s3-key-poc.mjs";

async function fixture(t, providerOptions = {}, pocOptions = {}) {
  const provider = await startMockProvider(providerOptions);
  t.after(() => provider.close());
  const poc = await startPoc({ mock: true, port: 0, clientId: provider.clientId,
    scopes: provider.scopes, endpoints: provider.endpoints, ...pocOptions });
  t.after(() => poc.close());
  return { provider, poc };
}

async function signIn(poc, decision = "allow") {
  const page = await (await fetch(poc.startUrl)).text();
  const authorizeUrl = page.match(/href="([^"]+)"/u)[1].replaceAll("&amp;", "&");
  const authPage = await fetch(authorizeUrl);
  assert.equal(authPage.status, 200);
  const consent = await fetch(new URL(`/${decision}`, authorizeUrl), { redirect: "manual" });
  const callback = consent.headers.get("location");
  const resultPage = await fetch(callback);
  return { callback, resultPage };
}

async function confirmAccount(resultPage, selected = "1".padStart(32, "0")) {
  return fetch(resultPage.url, { method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded", Origin: new URL(resultPage.url).origin },
    body: new URLSearchParams({ action: "continue", account_id: selected }) });
}

async function authorize(poc, decision = "allow", selected) {
  const response = await signIn(poc, decision);
  if (new URL(response.resultPage.url).pathname.startsWith("/account/")) {
    response.resultPage = await confirmAccount(response.resultPage, selected);
  }
  return { ...response, result: await poc.done };
}

test("browser redirect, PKCE exchange, and R2 list complete with no CLI or saved connection", async (t) => {
  const { poc, provider } = await fixture(t);
  const { result, resultPage, callback } = await authorize(poc);
  assert.equal(result.status, "POC_PASSED");
  assert.equal(result.evidence, "local_mock");
  assert.equal(result.wrangler_used, false);
  assert.equal(result.credentials_saved, false);
  assert.equal(result.connection_saved, false);
  assert.equal(result.remote_storage_changed, false);
  assert.equal(result.r2_list_passed, true);
  assert.equal(result.bucket_count_first_page, 1);
  assert.equal(result.s3_credentials_received, false);
  assert.equal(result.s3_probe_passed, false);
  assert.deepEqual(provider.calls.filter((call) => call.startsWith("POST")), ["POST /token"]);
  assert.equal(new URL(resultPage.url).pathname, "/result");
  assert.equal(new URL(resultPage.url).search, "");
  const body = await resultPage.text();
  assert.ok(!body.includes("fixture-access-token-canary"));
  assert.ok(!body.includes(new URL(callback).searchParams.get("code")));
  assert.equal((await fetch(callback)).status, 400);
  assert.equal(provider.calls.filter((call) => call === "POST /token").length, 1);
});

test("wrong state, duplicate code, and wrong host cannot consume the pending flow", async (t) => {
  const { poc } = await fixture(t);
  const invalid = new URL(poc.redirectUri);
  invalid.search = "state=wrong&code=fixture";
  assert.equal((await fetch(invalid)).status, 400);
  const wrongHost = await new Promise((resolve, reject) => {
    const req = httpRequest(poc.startUrl, { headers: { Host: "attacker.invalid" } }, (res) => {
      res.resume();
      resolve(res.statusCode);
    });
    req.on("error", reject);
    req.end();
  });
  assert.equal(wrongHost, 400);
  assert.equal((await authorize(poc)).result.status, "POC_PASSED");
  const flow = newAuthorization({ clientId: "fixture", scopes: ["read"],
    redirectUri: "http://127.0.0.1:12345/r2-poc/callback" });
  const state = new URL(flow.url).searchParams.get("state");
  assert.throws(() => flow.consume(`http://127.0.0.1:12345/r2-poc/callback?state=${state}&code=a&code=b`));
  assert.throws(() => flow.consume(`http://localhost:12345/r2-poc/callback?state=${state}&code=a`));
  const grant = flow.consume(`http://127.0.0.1:12345/r2-poc/callback?state=${state}&code=a`);
  assert.equal(createHash("sha256").update(grant.verifier).digest("base64url"),
    new URL(flow.url).searchParams.get("code_challenge"));
  assert.ok(!flow.url.includes(grant.verifier));
  assert.throws(() => flow.consume(`http://127.0.0.1:12345/r2-poc/callback?state=${state}&code=a`));
});

test("denial stops before the token exchange", async (t) => {
  const { poc, provider } = await fixture(t);
  assert.equal((await authorize(poc, "deny")).result.code, "authorization_denied");
  assert.ok(!provider.calls.includes("POST /token"));
});

test("expired and cancelled attempts reject late callbacks", async (t) => {
  const { poc, provider } = await fixture(t, {}, { timeoutMs: 20 });
  const result = await poc.done;
  assert.equal(result.code, "authorization_timeout");
  assert.equal((await fetch(`${poc.redirectUri}?state=late&code=late`)).status, 400);
  assert.equal(provider.calls.length, 0);
  const other = await fixture(t);
  await other.poc.close();
  assert.equal((await other.poc.done).code, "cancelled");
});

test("provider error content is omitted and token exchange is never retried", async (t) => {
  const { poc, provider } = await fixture(t, { tokenStatus: 400 });
  const { result, resultPage } = await authorize(poc);
  assert.equal(result.code, "token_exchange_failed");
  assert.ok(!JSON.stringify(result).includes("fixture-provider-private-canary"));
  assert.ok(!(await resultPage.text()).includes("fixture-provider-private-canary"));
  assert.equal(provider.calls.filter((call) => call === "POST /token").length, 1);
  assert.ok(!provider.calls.some((call) => call.includes("/client/v4/")));
});

test("token redirects, extra scopes, missing scopes, and oversized bodies fail closed", async (t) => {
  for (const options of [
    { tokenRedirect: "http://127.0.0.1:1/token" },
    { tokenScopes: "fixture.account.read fixture.r2.read fixture.r2.write" },
    { tokenScopes: "fixture.account.read" },
    { tokenBytes: 40 * 1024 },
  ]) {
    const { poc, provider } = await fixture(t, options);
    const { result } = await authorize(poc);
    assert.equal(result.status, "POC_STOPPED");
    assert.ok(!provider.calls.some((call) => call.includes("/client/v4/")));
  }
});

test("a valid OAuth token does not imply R2 permission", async (t) => {
  const { poc } = await fixture(t, { bucketStatus: 403 });
  const { result } = await authorize(poc);
  assert.equal(result.code, "r2_permission_denied");
  assert.notEqual(result.r2_list_passed, true);
});

test("one and multiple accounts require explicit confirmation before bucket access", async (t) => {
  for (const accountCount of [1, 2]) {
    const { poc, provider } = await fixture(t, { accountCount });
    const { resultPage } = await signIn(poc);
    assert.ok(new URL(resultPage.url).pathname.startsWith("/account/"));
    const body = await resultPage.text();
    assert.match(body, /Continue/u);
    assert.match(body, /Change login/u);
    assert.ok(!provider.calls.some((call) => call.includes("/r2/buckets")));
    const selected = String(accountCount).padStart(32, "0");
    await confirmAccount(resultPage, selected);
    assert.equal((await poc.done).account_confirmed, true);
    assert.equal(provider.calls.filter((call) => call.endsWith(`/${selected}/r2/buckets`)).length, 1);
    assert.equal((await confirmAccount(resultPage, selected)).status, 400);
  }
});

test("Change login discards the old session before any bucket operation", async (t) => {
  const { poc, provider } = await fixture(t, { accountCount: 2 });
  const first = await signIn(poc);
  const changed = await fetch(first.resultPage.url, { method: "POST", redirect: "manual",
    body: new URLSearchParams({ action: "change-login" }) });
  const authUrl = changed.headers.get("location");
  assert.equal((await fetch(first.callback)).status, 400);
  assert.ok(!provider.calls.some((call) => call.includes("/r2/buckets")));
  await fetch(authUrl);
  const consent = await fetch(new URL("/allow", authUrl), { redirect: "manual" });
  const choice = await fetch(consent.headers.get("location"));
  await confirmAccount(choice, "2".padStart(32, "0"));
  assert.equal((await poc.done).status, "POC_PASSED");
  assert.equal(provider.calls.filter((call) => call === "POST /token").length, 2);
  assert.equal(provider.calls.filter((call) => call.includes("/r2/buckets")).length, 1);
});

test("read failure is terminal and offers no retry, reset, or recovery", async (t) => {
  const { poc, provider } = await fixture(t, { bucketStatus: 503 });
  const choice = await signIn(poc);
  const stopped = await confirmAccount(choice.resultPage);
  assert.equal((await poc.done).status, "POC_STOPPED");
  const body = await stopped.text();
  assert.doesNotMatch(body, /<button|<form|Try again|Reset|Recover|Resume/u);
  assert.equal((await confirmAccount(choice.resultPage)).status, 400);
  assert.equal((await fetch(poc.startUrl)).status, 400);
  assert.equal(provider.calls.filter((call) => call.includes("/r2/buckets")).length, 1);
});

test("a foreign form origin cannot select an account or read buckets", async (t) => {
  const { poc, provider } = await fixture(t);
  const { resultPage } = await signIn(poc);
  await fetch(resultPage.url, { method: "POST", headers: { Origin: "https://attacker.invalid" },
    body: new URLSearchParams({ action: "continue", account_id: "1".padStart(32, "0") }) });
  assert.equal((await poc.done).code, "invalid_form_origin");
  assert.ok(!provider.calls.some((call) => call.includes("/r2/buckets")));
  assert.equal((await confirmAccount(resultPage)).status, 400);
});

test("a bucket cursor is reported as a partial listing", async (t) => {
  const { poc } = await fixture(t, { bucketCursor: "fixture-next-page" });
  assert.equal((await authorize(poc)).result.bucket_list_partial, true);
});

test("live mode requires its own client ID and explicit scopes and pins Cloudflare endpoints", async () => {
  await assert.rejects(startPoc({ clientId: "fixture", scopes: ["read"], port: 0,
    endpoints: { authorization: "http://127.0.0.1:1/auth", token: "http://127.0.0.1:1/token", api: "http://127.0.0.1:1" } }),
  (error) => error.code === "invalid_provider_endpoint");
  assert.throws(() => newAuthorization({ clientId: "fixture", scopes: [],
    redirectUri: `http://127.0.0.1:12345${CALLBACK_PATH}` }));
  assert.throws(() => newAuthorization({ clientId: "fixture", scopes: ["offline_access"],
    redirectUri: `http://127.0.0.1:12345${CALLBACK_PATH}` }));
  assert.throws(() => newAuthorization({ clientId: "fixture", scopes: ["r2:read"],
    redirectUri: `http://127.0.0.1:12345${CALLBACK_PATH}` }));
  assert.throws(() => newAuthorization({ scopes: ["read"],
    redirectUri: `http://127.0.0.1:12345${CALLBACK_PATH}` }));
});

test("S3 key denial is terminal, uses one POST, and creates no bucket", async (t) => {
  const { poc, provider } = await fixture(t, {}, { issueS3Key: true });
  const choice = await signIn(poc);
  assert.match(await choice.resultPage.text(), /Change login/u);
  assert.equal(provider.keyPolicies.length, 0);
  const stopped = await confirmAccount(choice.resultPage);
  const result = await poc.done;
  assert.equal(result.status, "POC_BLOCKED");
  assert.equal(result.code, "account_token_creation_rejected");
  assert.equal(result.token_create_http_status, 403);
  assert.deepEqual(result.provider_error_codes, [10000]);
  assert.equal(result.scope_catalog_complete, true);
  assert.deepEqual(result.token_scope_ids, []);
  assert.equal(result.s3_credentials_received, false);
  assert.equal(result.bucket_create_requests, 0);
  assert.equal(provider.keyPolicies.length, 1);
  assert.ok(!provider.calls.some((call) => call.includes("/r2/buckets")));
  const body = await stopped.text();
  assert.doesNotMatch(body, /private-canary|<button|<form|Try again|Reset|Recover|Resume/u);
  assert.equal((await confirmAccount(choice.resultPage)).status, 400);
  assert.equal(provider.keyPolicies.length, 1);
});

test("issued key uses the created token, with a one-hour policy for one new bucket", async (t) => {
  const { poc, provider } = await fixture(t, { keyStatus: 200 }, { issueS3Key: true });
  const { result, resultPage } = await authorize(poc);
  assert.equal(result.status, "POC_PASSED");
  assert.equal(result.remote_token_created, true);
  assert.equal(result.s3_credentials_received, true);
  assert.equal(result.s3_probe_passed, false);
  assert.equal(provider.keyPolicies.length, 1);
  const policy = provider.keyPolicies[0];
  assert.ok(Date.parse(policy.expires_on) > Date.now() + 59 * 60 * 1000);
  assert.ok(Date.parse(policy.expires_on) <= Date.now() + 60 * 60 * 1000);
  assert.equal(Object.keys(policy.policies[0].resources).length, 1);
  assert.match(Object.keys(policy.policies[0].resources)[0], /_default_mallard-poc-oauth-[a-f0-9]{16}$/u);
  assert.doesNotMatch(await resultPage.text(), /fixture-issued-token-private-canary|fixture-access-token-canary/u);
  const key = issuedS3Key({ ...policy, id: "a".repeat(32), status: "active", value: "fixture-issued-token" }, policy);
  assert.equal(key.accessKeyId, "a".repeat(32));
  assert.equal(key.secretAccessKey, createHash("sha256").update("fixture-issued-token").digest("hex"));
  assert.throws(() => issuedS3Key({ ...policy, id: "a".repeat(32), status: "active",
    value: "fixture-issued-token", policies: [{ ...policy.policies[0], resources: { "*": "*" } }] }, policy));
  assert.throws(() => keyPolicy("1".padStart(32, "0"), "existing-production-bucket"));
});

test("an unreadable create response reports an unknown outcome without another request", async (t) => {
  const { poc, provider } = await fixture(t, { keyMalformed: true }, { issueS3Key: true });
  const { result } = await authorize(poc);
  assert.equal(result.status, "POC_STOPPED");
  assert.equal(result.code, "token_create_outcome_unknown");
  assert.equal(result.remote_token_created, "unknown");
  assert.equal(provider.keyPolicies.length, 1);
});

test("the documented provisioning scope can be tested and its rejection is terminal", async (t) => {
  const { poc, provider } = await fixture(t, {}, { scopes: ["account_api_tokens:create"], issueS3Key: true });
  const page = await (await fetch(poc.startUrl)).text();
  const authorizeUrl = new URL(page.match(/href="([^"]+)"/u)[1].replaceAll("&amp;", "&"));
  assert.equal(authorizeUrl.searchParams.get("scope"), "account_api_tokens:create");
  const callback = new URL(poc.redirectUri);
  callback.search = new URLSearchParams({ state: authorizeUrl.searchParams.get("state"), error: "invalid_scope" });
  await fetch(callback);
  assert.equal((await poc.done).code, "authorization_invalid_scope");
  assert.equal(provider.calls.length, 0);
  assert.equal((await fetch(callback)).status, 400);
});

test("scope catalogue inspection cannot create a token or bucket", async (t) => {
  const { poc, provider } = await fixture(t, {}, { catalogOnly: true });
  const { result, resultPage } = await authorize(poc);
  assert.equal(result.operation, "scope_catalog");
  assert.equal(result.status, "POC_PASSED");
  assert.equal(result.documented_provisioning_scope_available, false);
  assert.equal(result.token_create_requests, 0);
  assert.equal(result.bucket_create_requests, 0);
  assert.equal(provider.keyPolicies.length, 0);
  assert.ok(!provider.calls.some((call) => call.includes("/r2/buckets")));
  assert.match(await resultPage.text(), /No S3 key is requested/u);
});

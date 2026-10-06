import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { request as httpRequest } from "node:http";
import { PublicClientApplication } from "@azure/msal-node";
import { configuration, msalConfiguration, consumeCallback, authorize, validateGrant,
  GraphReader, runProbe, safeAuthError, READ_SCOPES, microsoftNetwork } from "./browser-auth.mjs";

const CLIENT = "11111111-1111-4111-8111-111111111111";
const TENANT = "22222222-2222-4222-8222-222222222222";
const USER = "33333333-3333-4333-8333-333333333333";
const config = configuration({ clientId: CLIENT, tenant: TENANT });
const b64 = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
const ok = (value) => new Response(JSON.stringify(value), { status: 200 });
const expectCode = (code) => (error) => error.code === code;

function postCallback(url, params, host = new URL(url).host) {
  const target = new URL(url);
  target.hostname = "127.0.0.1";
  const body = new URLSearchParams(params).toString();
  return new Promise((resolve, reject) => {
    const req = httpRequest(target, { method: "POST", headers: { Host: host,
      "Content-Type": "application/x-www-form-urlencoded", "Content-Length": Buffer.byteLength(body) } }, (res) => {
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString() }));
    });
    req.on("error", reject);
    req.end(body);
  });
}

function msalFixture({ wrongNonce = false, rejectToken = false } = {}) {
  const calls = [];
  let authUrl;
  let tokenNumber = 0;
  const metadata = {
    authorization_endpoint: `${config.authority}/oauth2/v2.0/authorize`,
    token_endpoint: `${config.authority}/oauth2/v2.0/token`,
    issuer: `${config.authority}/v2.0`,
    end_session_endpoint: `${config.authority}/oauth2/v2.0/logout`,
    jwks_uri: `${config.authority}/discovery/v2.0/keys`,
  };
  const settings = msalConfiguration(config);
  settings.auth.authorityMetadata = JSON.stringify(metadata);
  settings.auth.cloudDiscoveryMetadata = JSON.stringify({ tenant_discovery_endpoint: metadata.issuer,
    metadata: [{ preferred_network: "login.microsoftonline.com", preferred_cache: "login.windows.net",
      aliases: ["login.microsoftonline.com", "login.windows.net", "login.microsoft.com"] }] });
  settings.system.networkClient = {
    async sendGetRequestAsync() { throw new Error("Unexpected network GET in fixture"); },
    async sendPostRequestAsync(url, options) {
      assert.equal(new URL(url).origin + new URL(url).pathname, metadata.token_endpoint);
      const body = new URLSearchParams(options.body);
      calls.push(body);
      assert.equal(body.has("client_secret"), false);
      assert.equal(body.has("client_assertion"), false);
      assert.equal(body.get("client_id"), CLIENT);
      tokenNumber += 1;
      if (rejectToken) return { status: 400, headers: {}, body: {
        error: "invalid_grant", error_description: "SECRET_FIXTURE_MUST_NOT_ESCAPE" } };
      if (tokenNumber === 1) {
        assert.equal(body.get("grant_type"), "authorization_code");
        assert.equal(body.get("code"), "synthetic-code");
        assert.equal(createHash("sha256").update(body.get("code_verifier")).digest("base64url"),
          authUrl.searchParams.get("code_challenge"));
      } else {
        assert.equal(body.get("grant_type"), "refresh_token");
        assert.equal(body.get("refresh_token"), "synthetic-refresh-token");
      }
      const claims = { aud: CLIENT, iss: metadata.issuer, tid: TENANT, oid: USER, sub: USER,
        iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 3600,
        preferred_username: "fixture@example.invalid",
        ...(tokenNumber === 1 ? { nonce: wrongNonce ? "wrong" : authUrl.searchParams.get("nonce") } : {}) };
      return { status: 200, headers: {}, body: {
        token_type: "Bearer", expires_in: 3600, scope: READ_SCOPES.join(" "),
        access_token: `synthetic-opaque-access-${tokenNumber}`, refresh_token: "synthetic-refresh-token",
        id_token: `${b64({ alg: "RS256", typ: "JWT" })}.${b64(claims)}.fixture-signature`,
        client_info: b64({ uid: USER, utid: TENANT }),
      } };
    },
  };
  const pca = new PublicClientApplication(settings);
  return { pca, calls, async openBrowserImpl(value) {
    authUrl = new URL(value);
    assert.equal(authUrl.origin, "https://login.microsoftonline.com");
    assert.equal(authUrl.searchParams.get("response_type"), "code");
    assert.equal(authUrl.searchParams.get("response_mode"), "form_post");
    assert.equal(authUrl.searchParams.get("code_challenge_method"), "S256");
    assert.equal(authUrl.searchParams.has("client_secret"), false);
    const redirect = new URL(authUrl.searchParams.get("redirect_uri"));
    const response = await postCallback(redirect, { code: "synthetic-code", state: authUrl.searchParams.get("state") });
    assert.equal(response.status, 200);
    assert.doesNotMatch(response.body, /synthetic-code|access_token/u);
  } };
}

test("public-client config needs no secret and keeps consent modes explicit", () => {
  assert.deepEqual(config.scopes, READ_SCOPES);
  assert.equal(msalConfiguration(config).auth.clientSecret, undefined);
  assert.equal(msalConfiguration(config).cache, undefined);
  assert.equal(msalConfiguration(config).system.disableInternalRetries, true);
  assert.equal(configuration({ clientId: CLIENT }).tenant, "organizations");
  assert.ok(configuration({ clientId: CLIENT, mode: "sync-consent" }).scopes.includes("Files.ReadWrite.All"));
  for (const options of [{ clientId: "" }, { clientId: CLIENT, tenant: "evil.invalid" },
    { clientId: CLIENT, tenant: "common" }, { clientId: CLIENT, mode: "implicit" }]) {
    assert.throws(() => configuration(options));
  }
});

test("callback rejects wrong state, duplicate fields, and code plus error", () => {
  assert.equal(consumeCallback("state=expected&code=fixture", "expected"), "fixture");
  for (const body of ["state=other&code=fixture", "state=expected&state=expected&code=x",
    "state=expected&code=x&code=y", "state=expected&code=x&error=access_denied", "state=expected"]) {
    assert.throws(() => consumeCallback(body, "expected"));
  }
});

test("provider denial exposes only a fixed error code", () => {
  assert.throws(() => consumeCallback("state=s&error=access_denied&error_description=SECRET", "s"),
    expectCode("access_denied"));
  assert.equal(safeAuthError({ errorCode: "SECRET", message: "SECRET" }).message, "authorization_failed");
});

test("identity transport rejects foreign hosts and bounds response bodies", async () => {
  let calls = 0;
  const network = microsoftNetwork(async (_url, init) => {
    calls += 1;
    assert.equal(init.redirect, "error");
    return ok({ data: "x".repeat(1024 * 1024) });
  });
  await assert.rejects(network.sendPostRequestAsync("https://attacker.invalid/token", {}), expectCode("invalid_identity_url"));
  assert.equal(calls, 0);
  await assert.rejects(network.sendPostRequestAsync(`${config.authority}/oauth2/v2.0/token`, {}), expectCode("response_too_large"));
  assert.equal(calls, 1);
});

test("real MSAL completes loopback PKCE and forced silent renewal with synthetic provider replies", async () => {
  const fixture = msalFixture();
  const session = await authorize(config, fixture);
  assert.equal(session.grant.account.localAccountId, USER);
  const renewed = await fixture.pca.acquireTokenSilent({ account: session.grant.account,
    scopes: config.scopes, forceRefresh: true });
  validateGrant(renewed, config, { ...session.grant.account, tenantId: TENANT });
  assert.equal(fixture.calls.length, 2);
  fixture.pca.clearCache();
  assert.equal((await fixture.pca.getTokenCache().getAllAccounts()).length, 0);
});

test("real MSAL rejects a nonce mismatch", async () => {
  const fixture = msalFixture({ wrongNonce: true });
  await assert.rejects(authorize(config, fixture), expectCode("authorization_failed"));
  assert.equal(fixture.calls.length, 1);
  assert.equal((await fixture.pca.getTokenCache().getAllAccounts()).length, 0);
});

test("failed token exchange is not repeated and does not expose the provider description", async () => {
  const fixture = msalFixture({ rejectToken: true });
  await assert.rejects(authorize(config, fixture), (error) => error.code === "invalid_grant"
    && !String(error).includes("SECRET"));
  assert.equal(fixture.calls.length, 1);
});

test("invalid host and state do not consume the pending callback", async () => {
  const fixture = msalFixture();
  const complete = fixture.openBrowserImpl;
  fixture.openBrowserImpl = async (value) => {
    const url = new URL(value);
    const callback = new URL(url.searchParams.get("redirect_uri"));
    const host = callback.host;
    for (const [requestHost, state] of [["attacker.invalid", url.searchParams.get("state")], [host, "wrong"]]) {
      const response = await postCallback(callback, { state, code: "synthetic-code" }, requestHost);
      assert.equal(response.status, 400);
    }
    await complete(value);
  };
  const session = await authorize(config, fixture);
  assert.equal(fixture.calls.length, 1);
  session.pca.clearCache();
});

test("authorization deadline closes the callback listener without a code exchange", async () => {
  let callback;
  const fixture = msalFixture();
  await assert.rejects(authorize(config, { ...fixture, timeoutMs: 100,
    openBrowserImpl: async (url) => { callback = new URL(url).searchParams.get("redirect_uri"); } }),
  expectCode("authorization_timeout"));
  assert.equal(fixture.calls.length, 0);
  assert.ok(callback);
  await assert.rejects(fetch(callback, { signal: AbortSignal.timeout(1000) }));
});

test("grant validation rejects missing scope, unexpected write scope, and changed tenant", () => {
  const grant = { accessToken: "fixture", expiresOn: new Date(Date.now() + 60000), tenantId: TENANT,
    scopes: [...READ_SCOPES], account: { homeAccountId: "fixture", localAccountId: USER } };
  validateGrant(grant, config);
  assert.throws(() => validateGrant({ ...grant, scopes: ["User.Read"] }, config), expectCode("missing_scope"));
  assert.throws(() => validateGrant({ ...grant, scopes: [...READ_SCOPES, "Files.ReadWrite.All"] }, config), expectCode("unexpected_scope"));
  assert.throws(() => validateGrant({ ...grant, tenantId: CLIENT }, config), expectCode("invalid_account_result"));
  assert.throws(() => validateGrant(grant, config, { ...grant.account, tenantId: CLIENT }), expectCode("account_changed"));
});

test("Graph pagination reads every page and sends only GET requests", async () => {
  const calls = [];
  const reader = new GraphReader("synthetic-token", { fetchImpl: async (url, init) => {
    calls.push(url.href);
    assert.equal(init.method, "GET");
    assert.equal(init.redirect, "error");
    assert.equal(init.headers.Authorization, "Bearer synthetic-token");
    return ok(calls.length === 1 ? { value: [{ id: "one" }],
      "@odata.nextLink": "https://graph.microsoft.com/v1.0/sites?$skiptoken=next" } : { value: [{ id: "two" }] });
  } });
  assert.equal((await reader.list("sites?search=*")).length, 2);
  assert.equal(calls.length, 2);
});

test("Graph refuses foreign next links before sending a bearer", async () => {
  for (const next of ["https://attacker.invalid/v1.0/sites", "https://graph.microsoft.com/v1.0/me",
    "https://user@graph.microsoft.com/v1.0/sites", "not a URL"]) {
    let count = 0;
    const reader = new GraphReader("synthetic-token", { fetchImpl: async () => {
      count += 1; return ok({ value: [], "@odata.nextLink": next });
    } });
    await assert.rejects(reader.list("sites"));
    assert.equal(count, 1);
  }
});

test("Graph rejects duplicate results, cursor loops, and truncated scans", async () => {
  for (const [page, options] of [
    [{ value: [{ id: "same" }, { id: "same" }] }, {}],
    [{ value: [], "@odata.nextLink": "https://graph.microsoft.com/v1.0/sites" }, {}],
    [{ value: [{ id: "one" }, { id: "two" }] }, { maxItems: 1 }],
    [{ value: [], "@odata.nextLink": "https://graph.microsoft.com/v1.0/sites?next=1" }, { maxPages: 1 }],
  ]) {
    const reader = new GraphReader("synthetic-token", { ...options, fetchImpl: async () => ok(page) });
    await assert.rejects(reader.list("sites"));
  }
});

test("Graph errors and oversized bodies stay bounded and sanitized", async () => {
  for (const [status, code] of [[401, "authorization_failed"], [403, "access_denied"], [429, "rate_limited"]]) {
    const reader = new GraphReader("fixture", { fetchImpl: async () => new Response("SECRET", { status }) });
    await assert.rejects(reader.get("me"), expectCode(code));
  }
  const reader = new GraphReader("fixture", { fetchImpl: async () => ok({ value: "x".repeat(1024 * 1024) }) });
  await assert.rejects(reader.get("me"), expectCode("response_too_large"));
});

test("complete read-only probe reports counts and clears real MSAL tokens", async () => {
  const fixture = msalFixture();
  const graphCalls = [];
  const result = await runProbe(config, {
    authorizeImpl: () => authorize(config, fixture),
    graphOptions: { fetchImpl: async (url, init) => {
      assert.equal(init.method, "GET");
      graphCalls.push(url.pathname);
      if (url.pathname.endsWith("/me")) return ok({ id: USER });
      if (url.pathname.endsWith("/drives")) return ok({ value: [{ id: "private-library", driveType: "documentLibrary" }] });
      return ok({ value: [{ id: "private-site", displayName: "SECRET_SITE_NAME" }] });
    } },
  });
  assert.equal(result.site_candidates, 1);
  assert.equal(result.document_libraries, 1);
  assert.equal(result.graph_writes, 0);
  assert.equal(result.storage_qualified, false);
  assert.equal(graphCalls.length, 4);
  assert.equal(fixture.calls.length, 2);
  assert.doesNotMatch(JSON.stringify(result), /private-|SECRET|synthetic|33333333/u);
  assert.equal((await fixture.pca.getTokenCache().getAllAccounts()).length, 0);
});

test("failed discovery clears tokens and preserves access-denied evidence", async () => {
  const fixture = msalFixture();
  await assert.rejects(runProbe(config, {
    authorizeImpl: () => authorize(config, fixture),
    graphOptions: { fetchImpl: async (url) => url.pathname.endsWith("/me")
      ? ok({ id: USER }) : new Response("SECRET", { status: 403 }) },
  }), expectCode("access_denied"));
  assert.equal((await fixture.pca.getTokenCache().getAllAccounts()).length, 0);
});

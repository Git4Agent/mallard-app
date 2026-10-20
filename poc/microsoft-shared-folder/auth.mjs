// Isolated POC. The production app does not import this module.
import { PublicClientApplication, CryptoProvider } from "@azure/msal-node";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { PocError, credential, json, request, requireValue } from "./http.mjs";

export const CALLBACK_PATH = "/microsoft-shared-folder/callback";
export const SCOPES = Object.freeze(["User.Read", "Files.ReadWrite.All"]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const exec = promisify(execFile);

export function configuration({ clientId, tenant }) {
  requireValue(typeof clientId === "string" && UUID.test(clientId), "client_id_required");
  requireValue(typeof tenant === "string" && UUID.test(tenant), "resource_tenant_required");
  return { clientId, tenant, authority: `https://login.microsoftonline.com/${tenant}`,
    scopes: [...SCOPES] };
}

export function msalConfiguration(config) {
  return {
    auth: { clientId: config.clientId, authority: config.authority },
    // No cache plugin: tokens remain in the process, never in a file.
    system: { disableInternalRetries: true, networkClient: microsoftNetwork(),
      loggerOptions: { piiLoggingEnabled: false, loggerCallback: () => {} } },
  };
}

export function microsoftNetwork(fetchImpl) {
  const send = async (target, options, method) => {
    const url = new URL(target);
    requireValue(url.origin === "https://login.microsoftonline.com"
      && !url.username && !url.password && !url.hash, "invalid_identity_url");
    const response = await request(url, { method, headers: options?.headers,
      ...(method === "POST" ? { body: options?.body } : {}) }, { fetchImpl });
    return { status: response.status, headers: {}, body: json(response.bytes) };
  };
  return {
    sendGetRequestAsync: (url, options) => send(url, options, "GET"),
    sendPostRequestAsync: (url, options) => send(url, options, "POST"),
  };
}

export function safeAuthError(error) {
  if (error instanceof PocError) return error;
  const known = new Set(["access_denied", "consent_required", "interaction_required",
    "invalid_client", "invalid_grant", "invalid_scope", "unauthorized_client"]);
  return new PocError(known.has(error?.errorCode) ? error.errorCode : "authorization_failed");
}

export async function openBrowser(url) {
  const parsed = new URL(url);
  requireValue(parsed.origin === "https://login.microsoftonline.com"
    && parsed.pathname.endsWith("/oauth2/v2.0/authorize"), "invalid_authorization_url");
  const command = process.platform === "darwin" ? "/usr/bin/open"
    : process.platform === "linux" ? "xdg-open"
      : process.platform === "win32" ? "rundll32.exe" : null;
  requireValue(command, "browser_platform_unsupported");
  try {
    await exec(command, process.platform === "win32" ? ["url.dll,FileProtocolHandler", url] : [url],
      { timeout: 10_000, maxBuffer: 4096 });
  } catch { throw new PocError("browser_open_failed"); }
}

// Keep the bounded, one-use callback under POC control. MSAL handles PKCE
// generation, authorization URL, token exchange, cache, and token renewal.
export function consumeCallback(body, expectedState) {
  const params = new URLSearchParams(body);
  const actual = Buffer.from(params.get("state") ?? "");
  const expected = Buffer.from(expectedState);
  requireValue(params.getAll("state").length === 1 && actual.length === expected.length
    && timingSafeEqual(actual, expected), "invalid_callback_state");
  requireValue(params.getAll("code").length <= 1 && params.getAll("error").length <= 1
    && params.has("code") !== params.has("error"), "invalid_callback_response");
  if (params.has("error")) throw safeAuthError({ errorCode: params.get("error") });
  return credential(params.get("code"));
}

async function callbackBody(req) {
  let size = 0;
  const parts = [];
  for await (const part of req) {
    size += part.length;
    requireValue(size <= 16 * 1024, "callback_too_large");
    parts.push(part);
  }
  return Buffer.concat(parts).toString("utf8");
}

export async function authorize(config, {
  pca = new PublicClientApplication(msalConfiguration(config)),
  cryptoProvider = new CryptoProvider(), openBrowserImpl = openBrowser,
  timeoutMs = 180_000, port = 0,
} = {}) {
  const state = randomBytes(32).toString("base64url");
  const nonce = randomBytes(32).toString("base64url");
  const { verifier, challenge } = await cryptoProvider.generatePkceCodes();
  let redirectUri;
  let used = false;
  let accept;
  let reject;
  const completion = new Promise((resolve, fail) => { accept = resolve; reject = fail; });
  completion.catch(() => {});
  const server = createServer({ maxHeaderSize: 8192 }, async (req, res) => {
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'");
    if (!redirectUri || req.method !== "POST" || req.url !== CALLBACK_PATH
      || req.headers.host !== new URL(redirectUri).host
      || req.headers["content-type"]?.split(";")[0] !== "application/x-www-form-urlencoded") {
      res.writeHead(400).end("Invalid callback.");
      return;
    }
    if (used) { res.writeHead(409).end("This sign-in response was already received."); return; }
    try {
      const body = await callbackBody(req);
      // A second request may have completed while this body was being read.
      requireValue(!used, "callback_already_used");
      const code = consumeCallback(body, state);
      used = true;
      res.end("Sign-in response received. Return to the Mallard POC for the result.");
      accept(code);
    } catch (error) {
      res.writeHead(400).end("Sign-in response was not accepted.");
      if (error instanceof PocError && !["invalid_callback_state", "invalid_callback_response",
        "callback_too_large", "callback_already_used", "credential_required"].includes(error.code)) {
        used = true;
        reject(error);
      }
    }
  });
  server.headersTimeout = 5000;
  server.requestTimeout = 5000;
  server.timeout = 5000;
  server.maxConnections = 8;
  let timer;
  let code;
  try {
    await new Promise((resolve, fail) => {
      server.once("error", () => fail(new PocError("callback_unavailable")));
      server.listen(port, "127.0.0.1", resolve);
    });
    redirectUri = `http://localhost:${server.address().port}${CALLBACK_PATH}`;
    const pending = new Promise((_, fail) => {
      timer = setTimeout(() => fail(new PocError("authorization_timeout")), timeoutMs);
    });
    code = await Promise.race([pending, (async () => {
      const url = await pca.getAuthCodeUrl({ scopes: config.scopes, redirectUri, state, nonce,
        codeChallenge: challenge, codeChallengeMethod: "S256", responseMode: "form_post",
        prompt: "select_account" });
      if (used) throw new PocError("authorization_timeout");
      await openBrowserImpl(url);
      return await completion;
    })()]);
  } catch (error) { throw safeAuthError(error); }
  finally {
    used = true;
    clearTimeout(timer);
    server.closeAllConnections();
    if (server.listening) await new Promise((resolve) => server.close(resolve));
  }
  try {
    // Exactly one code exchange. No client secret and no manual refresh token.
    const grant = await pca.acquireTokenByCode({ scopes: config.scopes, redirectUri,
      code, codeVerifier: verifier, nonce });
    validateGrant(grant, config);
    return { pca, grant };
  } catch (error) { await pca.clearCache(); throw safeAuthError(error); }
}

function normalizeScope(scope) { return scope.replace(/^https:\/\/graph\.microsoft\.com\//iu, "").toLowerCase(); }

export function validateGrant(grant, config, originalAccount) {
  credential(grant?.accessToken);
  requireValue(grant.account && typeof grant.account.homeAccountId === "string"
    && grant.account.homeAccountId.length > 0 && UUID.test(grant.tenantId)
    && typeof grant.account.localAccountId === "string" && grant.account.localAccountId.length > 0
    && config.tenant.toLowerCase() === grant.tenantId.toLowerCase(),
  "invalid_account_result");
  const actual = new Set((grant.scopes ?? []).map(normalizeScope));
  requireValue(config.scopes.every((scope) => actual.has(normalizeScope(scope))), "missing_scope");
  const allowed = new Set([...config.scopes.map(normalizeScope), "openid", "profile", "email", "offline_access"]);
  requireValue([...actual].every((scope) => allowed.has(scope)), "unexpected_scope");
  requireValue(grant.expiresOn instanceof Date && grant.expiresOn.getTime() > Date.now(), "expired_grant");
  if (originalAccount) requireValue(grant.account.homeAccountId === originalAccount.homeAccountId
    && grant.account.localAccountId === originalAccount.localAccountId
    && grant.tenantId === originalAccount.tenantId, "account_changed");
}

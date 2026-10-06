// Isolated POC. The production app does not import this module.
import { PublicClientApplication, CryptoProvider } from "@azure/msal-node";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify, parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { PocError, credential, json, request, requireValue, responseError } from "../storage-discovery/http.mjs";

export const CALLBACK_PATH = "/sharepoint/callback";
export const GRAPH = "https://graph.microsoft.com/v1.0";
export const READ_SCOPES = Object.freeze(["User.Read", "Sites.Read.All"]);
export const SYNC_SCOPES = Object.freeze([...READ_SCOPES, "Files.ReadWrite.All"]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const exec = promisify(execFile);

export function configuration({ clientId, tenant = "organizations", mode = "read" }) {
  requireValue(typeof clientId === "string" && UUID.test(clientId), "client_id_required");
  requireValue(tenant === "organizations" || UUID.test(tenant), "invalid_tenant");
  requireValue(["read", "sync-consent"].includes(mode), "invalid_permission_mode");
  return {
    clientId, tenant, mode,
    authority: `https://login.microsoftonline.com/${tenant}`,
    scopes: mode === "read" ? [...READ_SCOPES] : [...SYNC_SCOPES],
  };
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
  } catch (error) { pca.clearCache(); throw safeAuthError(error); }
}

function normalizeScope(scope) { return scope.replace(/^https:\/\/graph\.microsoft\.com\//iu, "").toLowerCase(); }

export function validateGrant(grant, config, originalAccount) {
  credential(grant?.accessToken);
  requireValue(grant.account && typeof grant.account.homeAccountId === "string"
    && grant.account.homeAccountId.length > 0 && UUID.test(grant.tenantId)
    && (config.tenant === "organizations" || config.tenant.toLowerCase() === grant.tenantId.toLowerCase()),
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

export class GraphReader {
  constructor(accessToken, { fetchImpl, maxPages = 20, maxItems = 1000 } = {}) {
    this.token = credential(accessToken);
    this.fetchImpl = fetchImpl;
    this.maxPages = maxPages;
    this.maxItems = maxItems;
  }
  async get(path) {
    const url = new URL(path, `${GRAPH}/`);
    requireValue(url.origin === "https://graph.microsoft.com" && url.pathname.startsWith("/v1.0/")
      && !url.username && !url.password && !url.hash, "invalid_graph_url");
    const response = await request(url, { method: "GET", headers: { Authorization: `Bearer ${this.token}` } },
      { fetchImpl: this.fetchImpl });
    if (!response.ok) throw responseError(response);
    return json(response.bytes);
  }
  async list(path) {
    const first = new URL(path, `${GRAPH}/`);
    let url = first;
    const seenPages = new Set();
    const seenItems = new Set();
    const values = [];
    while (url) {
      requireValue(url.origin === first.origin && url.pathname === first.pathname
        && !seenPages.has(url.href), "invalid_next_link");
      requireValue(seenPages.size < this.maxPages, "listing_limit");
      seenPages.add(url.href);
      const page = await this.get(url.href);
      requireValue(Array.isArray(page.value), "invalid_listing");
      for (const item of page.value) {
        requireValue(item && typeof item.id === "string" && item.id.length > 0
          && item.id.length <= 1024 && !seenItems.has(item.id), "invalid_listing");
        requireValue(values.length < this.maxItems, "listing_limit");
        seenItems.add(item.id);
        values.push(item);
      }
      const next = page["@odata.nextLink"];
      requireValue(next === undefined || typeof next === "string" && next.length <= 8192, "invalid_next_link");
      try { url = next === undefined ? null : new URL(next); }
      catch { throw new PocError("invalid_next_link"); }
    }
    return values;
  }
}

export async function runProbe(config, { authorizeImpl = authorize, graphOptions } = {}) {
  let session;
  let graph;
  let renewedGraph;
  try {
    session = await authorizeImpl(config);
    validateGrant(session.grant, config);
    graph = new GraphReader(session.grant.accessToken, graphOptions);
    const identity = await graph.get("me?$select=id");
    requireValue(typeof identity.id === "string" && identity.id.length > 0, "invalid_identity");
    const renewed = await session.pca.acquireTokenSilent({ account: session.grant.account,
      authority: `https://login.microsoftonline.com/${session.grant.tenantId}`,
      scopes: config.scopes, forceRefresh: true });
    validateGrant(renewed, config, { ...session.grant.account, tenantId: session.grant.tenantId });
    renewedGraph = new GraphReader(renewed.accessToken, graphOptions);
    requireValue((await renewedGraph.get("me?$select=id")).id === identity.id, "account_changed");
    // Search results are candidates, never an inventory or permission proof.
    const sites = await renewedGraph.list("sites?search=*&$select=id,displayName");
    const result = { sign_in: "passed", silent_renewal: "passed", permission_mode: config.mode,
      discovery: "search_candidates_only", site_candidates: sites.length,
      library_scan_sites: Math.min(sites.length, 3), document_libraries: 0,
      graph_writes: 0, token_persistence: "none", storage_qualified: false };
    for (const site of sites.slice(0, 3)) {
      const drives = await renewedGraph.list(`sites/${encodeURIComponent(site.id)}/drives?$select=id,driveType`);
      result.document_libraries += drives.filter((drive) => drive.driveType === "documentLibrary").length;
    }
    return result;
  } catch (error) { throw safeAuthError(error); }
  finally {
    if (graph) graph.token = "";
    if (renewedGraph) renewedGraph.token = "";
    if (session) {
      session.grant.accessToken = "";
      // Cache clearing removes local POC tokens. It does not revoke consent.
      await session.pca.clearCache();
    }
  }
}

async function main() {
  const { values } = parseArgs({ options: {
    "client-id": { type: "string" }, tenant: { type: "string", default: "organizations" },
    mode: { type: "string", default: "read" }, help: { type: "boolean", default: false },
  } });
  if (values.help) {
    console.log("Usage: node browser-auth.mjs --client-id PUBLIC_APP_ID [--tenant TENANT_ID] [--mode read|sync-consent]\nRegister http://localhost/sharepoint/callback as a Mobile and desktop redirect.\nDefault: read-only Graph calls. The sync-consent mode requests write permission but sends no writes.\nNo tokens, account IDs, names, or remote URLs are printed or saved.");
    return;
  }
  const config = configuration({ clientId: values["client-id"], tenant: values.tenant, mode: values.mode });
  console.log("Opening Microsoft sign-in. Complete consent in the browser.");
  console.log(JSON.stringify(await runProbe(config), null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(JSON.stringify({ error: safeAuthError(error).code }));
    process.exitCode = 1;
  });
}

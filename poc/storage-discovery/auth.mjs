import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { mkdtemp, rmdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { accountId } from "./discovery.mjs";
import { PocError, credential, json, request, requireValue } from "./http.mjs";

const exec = promisify(execFile);
// Same public client ID and registered callback as the current Rust app.
export const DROPBOX_APP_KEY = "ex00srwhe5f5m5m";
export const CALLBACK_URI = "http://127.0.0.1:53682/dropbox/callback";
export const DISCOVERY_SCOPES = Object.freeze([
  "account_info.read", "files.metadata.read", "files.content.read", "sharing.read",
]);

export function validateDiscoveryToken(value) {
  requireValue(typeof value.token_type === "string" && value.token_type.toLowerCase() === "bearer"
    && Number.isSafeInteger(value.expires_in) && value.expires_in > 0 && value.expires_in <= 86400
    && value.refresh_token === undefined, "invalid_token_response");
  // RFC 6749 permits scope omission when the requested scope did not change.
  const granted = value.scope === undefined ? DISCOVERY_SCOPES
    : typeof value.scope === "string" ? value.scope.trim().split(/\s+/u) : [];
  if (!DISCOVERY_SCOPES.every((scope) => granted.includes(scope))
    || !granted.every((scope) => DISCOVERY_SCOPES.includes(scope))) {
    const error = new PocError("unexpected_token_scope");
    // Only fixed scope names and counts may leave the token boundary.
    error.scopeEvidence = {
      format: Array.isArray(value.scope) ? "array" : typeof value.scope,
      missing: DISCOVERY_SCOPES.filter((scope) => !granted.includes(scope)),
      additionalCount: granted.filter((scope) => !DISCOVERY_SCOPES.includes(scope)).length,
      writeScopePresent: ["files.content.write", "files.metadata.write", "sharing.write"]
        .some((scope) => granted.includes(scope)),
    };
    throw error;
  }
  return credential(value.access_token);
}

export function newDropboxAuthorization(appKey = DROPBOX_APP_KEY, redirectUri = CALLBACK_URI) {
  requireValue(typeof appKey === "string" && /^[a-zA-Z0-9]{1,128}$/u.test(appKey), "invalid_app_key");
  const redirect = new URL(redirectUri);
  requireValue(redirect.protocol === "http:" && redirect.hostname === "127.0.0.1"
    && redirect.pathname === "/dropbox/callback" && !redirect.username && !redirect.password
    && !redirect.search && !redirect.hash, "invalid_callback_uri");
  const verifier = randomBytes(64).toString("base64url");
  const state = randomBytes(32).toString("base64url");
  let consumed = false;
  const url = new URL("https://www.dropbox.com/oauth2/authorize");
  url.search = new URLSearchParams({ client_id: appKey, response_type: "code",
    redirect_uri: redirectUri, token_access_type: "online",
    code_challenge_method: "S256",
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    state, scope: DISCOVERY_SCOPES.join(" ") }).toString();
  return { url: url.toString(), consume(callbackUrl) {
    requireValue(!consumed, "callback_already_used");
    const callback = new URL(callbackUrl);
    const params = callback.searchParams;
    const actual = Buffer.from(params.get("state") ?? "");
    const expected = Buffer.from(state);
    requireValue(callback.origin === redirect.origin && callback.pathname === redirect.pathname
      && !callback.username && !callback.password && !callback.hash
      && params.getAll("state").length === 1 && actual.length === expected.length
      && timingSafeEqual(actual, expected), "invalid_callback_state");
    requireValue(params.getAll("code").length <= 1 && params.getAll("error").length <= 1
      && !(params.has("code") && params.has("error")), "invalid_callback_response");
    if (params.has("error")) {
      consumed = true;
      throw new PocError(params.get("error") === "invalid_scope" ? "dropbox_scope_not_enabled" : "authorization_denied");
    }
    const code = credential(params.get("code"));
    consumed = true;
    return { code, verifier, redirectUri };
  } };
}

async function openBrowser(url) {
  const program = process.platform === "darwin" ? "/usr/bin/open"
    : process.platform === "linux" ? "xdg-open" : undefined;
  requireValue(program, "browser_platform_unsupported");
  try { await exec(program, [url], { timeout: 10_000, maxBuffer: 8192 }); }
  catch { throw new PocError("browser_open_failed"); }
}

export async function authorizeDropbox({ appKey = DROPBOX_APP_KEY, openBrowserImpl = openBrowser,
  port = 53682, timeoutMs = 180_000, fetchImpl } = {}) {
  let flow;
  let redirectUri;
  let accept;
  let reject;
  const completion = new Promise((resolve, fail) => { accept = resolve; reject = fail; });
  completion.catch(() => {});
  const server = createServer({ maxHeaderSize: 8192 }, (req, res) => {
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'");
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    if (!flow || req.method !== "GET" || req.headers.host !== new URL(redirectUri).host
      || !req.url?.startsWith("/dropbox/callback?") || req.url.length > 8192) {
      res.writeHead(400).end("Invalid callback.");
      return;
    }
    try {
      const grant = flow.consume(new URL(req.url, redirectUri).toString());
      res.end("Read access received. Return to Mallard's POC task.");
      accept(grant);
    } catch (error) {
      res.writeHead(400).end("Consent was not accepted. Return to the POC task.");
      if (error instanceof PocError && ["authorization_denied", "dropbox_scope_not_enabled"].includes(error.code)) reject(error);
    }
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.timeout = 5000;
  server.maxConnections = 8;
  let timer;
  let grant;
  try {
    await new Promise((resolve, fail) => {
      server.once("error", () => fail(new PocError("callback_listener_unavailable")));
      server.listen(port, "127.0.0.1", resolve);
    });
    redirectUri = `http://127.0.0.1:${server.address().port}/dropbox/callback`;
    flow = newDropboxAuthorization(appKey, redirectUri);
    timer = setTimeout(() => reject(new PocError("authorization_timeout")), timeoutMs);
    await openBrowserImpl(flow.url);
    grant = await completion;
  } finally {
    clearTimeout(timer);
    server.closeAllConnections();
    if (server.listening) await new Promise((resolve) => server.close(resolve));
  }
  const response = await request("https://api.dropboxapi.com/oauth2/token", {
    method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "authorization_code", client_id: appKey,
      code: grant.code, code_verifier: grant.verifier, redirect_uri: grant.redirectUri }).toString(),
  }, { fetchImpl });
  requireValue(response.ok, "dropbox_token_exchange_failed");
  return validateDiscoveryToken(json(response.bytes));
}

export function parseWranglerAuth(whoami, token, selectedAccount) {
  requireValue(whoami.loggedIn === true && Array.isArray(whoami.accounts)
    && whoami.accounts.length > 0 && whoami.accounts.length <= 64, "wrangler_login_required");
  const accounts = [...new Set(whoami.accounts.map((account) => accountId(account.id)))];
  const selected = selectedAccount ? accountId(selectedAccount) : undefined;
  requireValue(selected ? accounts.includes(selected) : accounts.length === 1, "wrangler_account_selection_required");
  requireValue(token.type === "oauth", "wrangler_oauth_required");
  return { accountId: selected ?? accounts[0], token: credential(token.token) };
}

export async function wranglerAuthorization(selectedAccount) {
  const directory = await mkdtemp(join(tmpdir(), "mallard-storage-discovery-"));
  const env = { ...process.env };
  for (const name of Object.keys(env)) {
    if (/^(CLOUDFLARE_|CF_|WRANGLER_)/u.test(name) || name === "NODE_OPTIONS" || name === "CI") delete env[name];
  }
  Object.assign(env, { NO_COLOR: "1", WRANGLER_SEND_METRICS: "false", WRANGLER_SEND_ERROR_REPORTS: "false",
    WRANGLER_NO_SKILLS_UPDATE_PROMPTS: "true", WRANGLER_WRITE_LOGS: "false", WRANGLER_LOG_SANITIZE: "true" });
  const read = async (args) => {
    try {
      const { stdout } = await exec("wrangler", args, { cwd: directory, env, timeout: 30_000,
        maxBuffer: 256 * 1024, encoding: "buffer" });
      try { return json(stdout); }
      finally { stdout.fill(0); }
    } catch { throw new PocError("wrangler_auth_unavailable"); }
  };
  try {
    const whoami = await read(["whoami", "--json"]);
    requireValue(whoami.loggedIn === true, "wrangler_login_required");
    const token = await read(["auth", "token", "--json"]);
    return parseWranglerAuth(whoami, token, selectedAccount);
  } finally {
    // Remove only the exact empty temporary directory. Never remove recursively.
    await rmdir(directory).catch(() => {});
  }
}

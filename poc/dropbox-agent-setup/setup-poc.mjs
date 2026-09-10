import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";

// Protocol POC only. Nothing in the application imports this module.
export const CALLBACK_URI = "http://127.0.0.1:53682/dropbox/callback";
export const STORAGE_SCOPES = Object.freeze([
  "account_info.read",
  "files.metadata.read",
  "files.content.read",
  "files.content.write",
]);
export const SETUP_SCOPES = Object.freeze([...STORAGE_SCOPES, "sharing.write"]);
const API = "https://api.dropboxapi.com";
const MAX_RESPONSE_BYTES = 1024 * 1024;
const IDENTIFIER = /^[-_a-zA-Z0-9:]{1,256}$/u;

export class PocError extends Error {
  constructor(code) {
    super(code);
    this.code = code;
    this.remoteFolderMayExist = false;
  }
}

function requireValue(condition, code) {
  if (!condition) throw new PocError(code);
}

function secret(value) {
  requireValue(typeof value === "string" && /^[\x21-\x7e]{1,16384}$/u.test(value), "invalid_credential_response");
  return value;
}

function identifier(value) {
  requireValue(typeof value === "string" && IDENTIFIER.test(value), "invalid_metadata_response");
  return value;
}

export function newAuthorization(appKey, redirectUri) {
  requireValue(typeof appKey === "string" && /^[a-zA-Z0-9]{1,128}$/u.test(appKey), "invalid_app_key");
  const redirect = new URL(redirectUri);
  requireValue(redirect.protocol === "http:" && redirect.hostname === "127.0.0.1"
    && redirect.pathname === "/dropbox/callback" && !redirect.username && !redirect.password
    && !redirect.search && !redirect.hash, "invalid_callback_uri");
  const verifier = randomBytes(64).toString("base64url");
  const state = randomBytes(32).toString("base64url");
  let consumed = false;
  const url = new URL("https://www.dropbox.com/oauth2/authorize");
  url.search = new URLSearchParams({
    client_id: appKey,
    response_type: "code",
    redirect_uri: redirectUri,
    token_access_type: "offline",
    code_challenge_method: "S256",
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    state,
    scope: SETUP_SCOPES.join(" "),
  }).toString();
  return {
    url: url.toString(),
    consume(callbackUrl) {
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
        throw new PocError("authorization_denied");
      }
      const code = secret(params.get("code"));
      consumed = true;
      return { code, verifier, redirectUri };
    },
  };
}

async function openSystemBrowser(url) {
  const program = process.platform === "darwin" ? "/usr/bin/open"
    : process.platform === "linux" ? "xdg-open" : null;
  requireValue(program, "live_browser_platform_unsupported");
  await new Promise((resolve, reject) => {
    const child = spawn(program, [url], { stdio: "ignore", shell: false });
    const timer = setTimeout(() => { child.kill(); reject(new PocError("browser_open_failed")); }, 10_000);
    child.once("error", () => { clearTimeout(timer); reject(new PocError("browser_open_failed")); });
    child.once("exit", (code) => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new PocError("browser_open_failed"));
    });
  });
}

export async function authorizeUser({ appKey, openBrowser = openSystemBrowser, port = 53682, timeoutMs = 180_000 }) {
  let flow;
  let redirectUri;
  let accept;
  let reject;
  const completion = new Promise((resolve, fail) => { accept = resolve; reject = fail; });
  // Attach a handler before browser launch; denial can arrive during launch.
  completion.catch(() => {});
  const server = createServer({ maxHeaderSize: 8192 }, (request, response) => {
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("Referrer-Policy", "no-referrer");
    response.setHeader("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'");
    response.setHeader("Content-Type", "text/plain; charset=utf-8");
    if (request.method !== "GET" || request.headers.host !== new URL(redirectUri).host
      || !request.url?.startsWith("/dropbox/callback?") || request.url.length > 8192) {
      response.writeHead(400).end("Invalid callback.");
      return;
    }
    try {
      const credentials = flow.consume(new URL(request.url, redirectUri).toString());
      response.end("Dropbox consent received. Return to the terminal.");
      accept(credentials);
    } catch (error) {
      response.writeHead(400).end("Dropbox consent was not accepted.");
      if (error instanceof PocError && error.code === "authorization_denied") reject(error);
    }
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.timeout = 5000;
  server.maxConnections = 8;
  let timer;
  try {
    await new Promise((resolve, fail) => {
      server.once("error", () => fail(new PocError("callback_listener_unavailable")));
      server.listen(port, "127.0.0.1", resolve);
    });
    redirectUri = `http://127.0.0.1:${server.address().port}/dropbox/callback`;
    flow = newAuthorization(appKey, redirectUri);
    timer = setTimeout(() => reject(new PocError("authorization_timeout")), timeoutMs);
    await openBrowser(flow.url);
    return await completion;
  } finally {
    clearTimeout(timer);
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

class DropboxClient {
  constructor(fetchImpl, requestTimeoutMs) {
    this.fetchImpl = fetchImpl;
    this.requestTimeoutMs = requestTimeoutMs;
  }

  async request(path, body, token, root, form = false) {
    const headers = { "Content-Type": form ? "application/x-www-form-urlencoded" : "application/json" };
    if (token) headers.Authorization = `Bearer ${token}`;
    if (root) headers["Dropbox-API-Path-Root"] = JSON.stringify(root);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.requestTimeoutMs);
    try {
      const response = await this.fetchImpl(`${API}${path}`, {
        method: "POST", headers, redirect: "error", signal: controller.signal,
        body: form ? new URLSearchParams(body).toString() : JSON.stringify(body),
      });
      if (!response.ok) {
        await response.body?.cancel();
        throw new PocError(response.status === 401 ? "dropbox_authorization_failed"
          : response.status === 429 ? "dropbox_rate_limited" : "dropbox_request_rejected");
      }
      requireValue(response.body, "invalid_json_response");
      const reader = response.body.getReader();
      const chunks = [];
      let size = 0;
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > MAX_RESPONSE_BYTES) {
            await reader.cancel();
            throw new PocError("response_too_large");
          }
          chunks.push(value);
        }
      } finally {
        reader.releaseLock();
      }
      let result;
      try { result = JSON.parse(Buffer.concat(chunks).toString("utf8")); }
      catch { throw new PocError("invalid_json_response"); }
      requireValue(result !== null && typeof result === "object" && !Array.isArray(result), "invalid_json_response");
      return result;
    } catch (error) {
      if (error instanceof PocError) throw error;
      throw new PocError(controller.signal.aborted ? "dropbox_request_timeout" : "dropbox_transport_failed");
    } finally {
      clearTimeout(timer);
    }
  }

  rpc(route, body, token, root) { return this.request(`/2/${route}`, body, token, root); }
  token(body) { return this.request("/oauth2/token", body, undefined, undefined, true); }
}

function checkToken(value, scopes, refreshRequired) {
  const accessToken = secret(value.access_token);
  requireValue(value.token_type?.toLowerCase() === "bearer" && Number.isSafeInteger(value.expires_in)
    && value.expires_in > 0 && value.expires_in <= 31_536_000, "invalid_credential_response");
  // OAuth permits omission when the granted scope equals the requested scope.
  const granted = value.scope === undefined ? scopes : typeof value.scope === "string" ? value.scope.split(" ") : [];
  const missingScopes = scopes.filter((scope) => !granted.includes(scope));
  if (missingScopes.length > 0) {
    const error = new PocError("required_scope_missing");
    // These names come from our fixed scope list, never from response text.
    error.missingScopes = missingScopes;
    error.scopeFormat = Array.isArray(value.scope) ? "array" : typeof value.scope;
    throw error;
  }
  if (refreshRequired) secret(value.refresh_token);
  return accessToken;
}

function checkAccount(account, expectedId) {
  requireValue(account.account_id === expectedId && expectedId?.startsWith("dbid:"), "account_mismatch");
  identifier(expectedId);
  const root = account.root_info;
  requireValue(root && ["user", "team"].includes(root[".tag"]), "invalid_account_response");
  identifier(root.home_namespace_id);
  identifier(root.root_namespace_id);
  return root;
}

// Read-only diagnosis of one POC folder. Reports field presence and equality
// only. Never returns provider IDs, paths, credentials, or response bodies.
export async function inspectCreatedFolder({ appKey, runId,
  authorize = () => authorizeUser({ appKey }), fetchImpl = globalThis.fetch }) {
  requireValue(/^[a-f0-9]{24}$/u.test(runId), "invalid_run_id");
  const client = new DropboxClient(fetchImpl, 15_000);
  const { code, verifier, redirectUri } = await authorize();
  const grant = await client.token({ grant_type: "authorization_code", client_id: appKey,
    code: secret(code), code_verifier: secret(verifier), redirect_uri: redirectUri });
  const token = checkToken(grant, SETUP_SCOPES, true);
  const root = checkAccount(await client.rpc("users/get_current_account", null, token), grant.account_id);
  const home = { ".tag": "home" };
  const byPath = await client.rpc("files/get_metadata", { path: `/Mallard POC ${runId}` }, token, home);
  identifier(byPath.id);
  const byId = await client.rpc("files/get_metadata", { path: byPath.id }, token, home);
  const renewed = await client.token({ grant_type: "refresh_token", client_id: appKey,
    refresh_token: grant.refresh_token, scope: STORAGE_SCOPES.join(" ") });
  const renewedToken = checkToken(renewed, STORAGE_SCOPES, false);
  checkAccount(await client.rpc("users/get_current_account", null, renewedToken), grant.account_id);
  const afterRefresh = await client.rpc("files/get_metadata", { path: byPath.id }, renewedToken, home);
  const shape = (metadata) => ({ folderType: metadata[".tag"] === "folder",
    sharingInfoPresent: Boolean(metadata.sharing_info),
    sharedNamespacePresent: Boolean(metadata.sharing_info?.shared_folder_id),
    parentNamespacePresent: Boolean(metadata.sharing_info?.parent_shared_folder_id) });
  const namespaceId = byPath.sharing_info?.shared_folder_id;
  let namespaceReadable = false;
  if (typeof namespaceId === "string" && IDENTIFIER.test(namespaceId)
    && namespaceId !== root.home_namespace_id && namespaceId !== root.root_namespace_id) {
    const listing = await client.rpc("files/list_folder", { path: "", recursive: false, limit: 1 }, renewedToken,
      { ".tag": "namespace_id", namespace_id: namespaceId });
    namespaceReadable = Array.isArray(listing.entries) && typeof listing.has_more === "boolean";
  }
  return { verdict: "INSPECTION_COMPLETE", runId, mutationRequests: 0,
    byPath: shape(byPath), byId: shape(byId), afterRefresh: shape(afterRefresh),
    pathAndIdAgree: byPath.id === byId.id, idUnchangedAfterRefresh: byPath.id === afterRefresh.id,
    namespaceReadable };
}

/** Returns secret-bearing values only to its in-process caller. The CLI prints report only. */
export async function runSetupPoc({
  appKey,
  authorize = () => authorizeUser({ appKey }),
  fetchImpl = globalThis.fetch,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  requestTimeoutMs = 15_000,
  maxPolls = 30,
  runId = randomBytes(12).toString("hex"),
}) {
  requireValue(typeof appKey === "string" && /^[a-zA-Z0-9]{1,128}$/u.test(appKey), "invalid_app_key");
  requireValue(/^[a-f0-9]{24}$/u.test(runId), "invalid_run_id");
  const client = new DropboxClient(fetchImpl, requestTimeoutMs);
  let remoteFolderMayExist = false;
  try {
    const { code, verifier, redirectUri } = await authorize();
    const grant = await client.token({ grant_type: "authorization_code", client_id: appKey,
      code: secret(code), code_verifier: secret(verifier), redirect_uri: redirectUri });
    const setupToken = checkToken(grant, SETUP_SCOPES, true);
    const root = checkAccount(await client.rpc("users/get_current_account", null, setupToken), grant.account_id);
    const homeRoot = { ".tag": "home" };
    const folderPath = `/Mallard POC ${runId}`;

    // Strict creation first. Share the acknowledged ID, never a path that can
    // be replaced between requests. There is no "adopt existing" fallback.
    remoteFolderMayExist = true;
    const created = await client.rpc("files/create_folder_v2", { path: folderPath, autorename: false }, setupToken, homeRoot);
    const folder = created.metadata;
    requireValue(folder?.id?.startsWith("id:") && folder.path_lower === folderPath.toLowerCase()
      && (!folder[".tag"] || folder[".tag"] === "folder") && !folder.sharing_info, "invalid_created_folder");
    identifier(folder.id);
    let shared = await client.rpc("sharing/share_folder", { path: folder.id, force_async: false }, setupToken, homeRoot);
    if (shared[".tag"] === "async_job_id") {
      const jobId = identifier(shared.async_job_id);
      for (let poll = 0; poll < maxPolls; poll += 1) {
        await sleep(1000);
        shared = await client.rpc("sharing/check_share_job_status", { async_job_id: jobId }, setupToken);
        if (shared[".tag"] !== "in_progress") break;
      }
    }
    requireValue(shared[".tag"] === "complete", "folder_share_incomplete");
    const namespaceId = identifier(shared.shared_folder_id);
    requireValue(namespaceId !== root.home_namespace_id && namespaceId !== root.root_namespace_id
      && shared.access_type?.[".tag"] === "owner" && shared.path_lower === folderPath.toLowerCase(), "invalid_shared_namespace");

    // A refresh grant uses the public app key and refresh token. No app
    // secret is shipped. The new access token requests only storage scopes.
    const renewed = await client.token({ grant_type: "refresh_token", client_id: appKey,
      refresh_token: grant.refresh_token, scope: STORAGE_SCOPES.join(" ") });
    const accessToken = checkToken(renewed, STORAGE_SCOPES, false);
    checkAccount(await client.rpc("users/get_current_account", null, accessToken), grant.account_id);
    // Sharing can change the ordinary folder ID. The acknowledged namespace
    // is the storage identity. Check the exact mount path against that
    // namespace; a same-name replacement with another namespace fails.
    const metadata = await client.rpc("files/get_metadata", { path: folderPath }, accessToken, homeRoot);
    const checks = { folderType: metadata[".tag"] === "folder",
      folderPathMatches: metadata.path_lower === folderPath.toLowerCase(),
      sharedNamespacePresent: typeof metadata.sharing_info?.shared_folder_id === "string",
      namespaceMatches: metadata.sharing_info?.shared_folder_id === namespaceId };
    if (!checks.folderType || !checks.folderPathMatches || !checks.namespaceMatches) {
      const error = new PocError("namespace_identity_mismatch");
      error.metadataChecks = checks;
      throw error;
    }
    identifier(metadata.id);
    requireValue(metadata.id.startsWith("id:"), "invalid_metadata_response");
    const listing = await client.rpc("files/list_folder", { path: "", recursive: false, limit: 1 }, accessToken,
      { ".tag": "namespace_id", namespace_id: namespaceId });
    requireValue(Array.isArray(listing.entries) && listing.entries.length <= 1
      && typeof listing.has_more === "boolean" && typeof listing.cursor === "string", "invalid_namespace_listing");

    return {
      report: { verdict: "POC_PASS", productionGate: "INCOMPLETE", schemaChanged: false,
        folderCreated: true, namespaceResolved: true, refreshVerified: true,
        configurationPreparedInMemory: true, storageCommitted: false,
        invitationsSent: false, remoteFolderRetained: true,
        folderIdChangedOnShare: metadata.id !== folder.id, runId },
      // This exact config is accepted by the current Dropbox adapter. The
      // refresh grant is separate POC data, not an added production schema.
      preparedConfig: { version: 1, settings: { namespace_id: namespaceId }, secrets: { access_token: accessToken } },
      oauth: { app_key: appKey, refresh_token: grant.refresh_token,
        expires_at: Date.now() + renewed.expires_in * 1000 },
    };
  } catch (error) {
    const safe = error instanceof PocError ? error : new PocError("poc_failed");
    safe.remoteFolderMayExist = remoteFolderMayExist;
    safe.runId = runId;
    throw safe;
  }
}

export async function executeCli(args = process.argv.slice(2)) {
  if (args.length === 5 && args[0] === "inspect" && args[1] === "--app-key" && args[3] === "--run-id") {
    return inspectCreatedFolder({ appKey: args[2], runId: args[4] });
  }
  if (args.length === 1 && args[0] === "mock") {
    const { MockDropbox } = await import("./mock-dropbox.mjs");
    const mock = await MockDropbox.start();
    try {
      const { report } = await runSetupPoc(mock.options());
      return { ...report, mode: "local_http_mock" };
    } finally { await mock.close(); }
  }
  requireValue(args.length === 3 && args[0] === "live" && args[1] === "--app-key", "usage_mock_or_live_app_key");
  const { report } = await runSetupPoc({ appKey: args[2] });
  return { ...report, mode: "live" };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  executeCli().then((report) => process.stdout.write(`${JSON.stringify(report)}\n`)).catch((error) => {
    process.stdout.write(`${JSON.stringify({ verdict: "POC_FAIL", productionGate: "INCOMPLETE",
      code: error instanceof PocError ? error.code : "poc_failed",
      remoteFolderMayExist: error instanceof PocError && error.remoteFolderMayExist,
      missingScopes: error instanceof PocError ? error.missingScopes : undefined,
      scopeFormat: error instanceof PocError ? error.scopeFormat : undefined,
      metadataChecks: error instanceof PocError ? error.metadataChecks : undefined,
      runId: error instanceof PocError ? error.runId : undefined })}\n`);
    process.exitCode = 1;
  });
}

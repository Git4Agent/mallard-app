// Isolated POC. Production code does not import this module.
import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { PocError, credential, json, request, requireValue } from "../storage-discovery/http.mjs";
import { runS3KeyPoc } from "./s3-key-poc.mjs";

export const CLOUDFLARE = Object.freeze({
  authorization: "https://dash.cloudflare.com/oauth2/auth",
  token: "https://dash.cloudflare.com/oauth2/token",
  api: "https://api.cloudflare.com/client/v4",
});
export const CALLBACK_PATH = "/r2-poc/callback";
export const DEFAULT_PORT = 53684;

const escapeHtml = (text) => String(text).replace(/[&<>"']/gu, (char) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
})[char]);

export function page(title, body) {
  return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${escapeHtml(title)}</title><style>body{font:17px/1.6 system-ui;max-width:740px;margin:70px auto;padding:0 24px;color:#172b35;background:#f6f8f9}h1{font-size:30px}a,button{display:inline-block;color:#fff;background:#16677b;padding:10px 18px;border:0;border-radius:5px;font:inherit;text-decoration:none}pre{white-space:pre-wrap;background:white;border:1px solid #ccd7db;padding:20px}small{color:#52636b}</style><main><small>MALLARD · ISOLATED POC</small><h1>${escapeHtml(title)}</h1>${body}</main></html>`;
}

export function sendPage(response, status, title, body) {
  response.writeHead(status, {
    "Content-Type": "text/html; charset=utf-8",
    "Cache-Control": "no-store",
    // Keep a same-origin form POST's Origin. Callback redirects still strip referrers.
    "Referrer-Policy": "same-origin",
    "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    "X-Content-Type-Options": "nosniff",
  }).end(page(title, body));
}

function validId(value) {
  return typeof value === "string" && /^[a-f0-9]{32}$/u.test(value);
}

export function newAuthorization({ clientId, scopes, redirectUri, authorization = CLOUDFLARE.authorization }) {
  requireValue(typeof clientId === "string" && /^[A-Za-z0-9_-]{1,128}$/u.test(clientId), "client_id_required");
  requireValue(Array.isArray(scopes) && scopes.length > 0 && scopes.length <= 16
    // One documented provisioning scope is allowed for the live availability test.
    // Do not accept arbitrary legacy/first-party colon-delimited scopes.
    && scopes.every((scope) => /^[A-Za-z0-9_.-]{1,128}$/u.test(scope)
      || scope === "account_api_tokens:create")
    && new Set(scopes).size === scopes.length
    && !scopes.some((scope) => ["offline", "offline_access"].includes(scope)), "explicit_scopes_required");
  const redirect = new URL(redirectUri);
  requireValue(redirect.protocol === "http:" && redirect.hostname === "127.0.0.1"
    && redirect.pathname === CALLBACK_PATH && !redirect.search && !redirect.hash
    && !redirect.username && !redirect.password, "invalid_redirect_uri");
  let verifier = randomBytes(32).toString("base64url");
  let state = randomBytes(32).toString("base64url");
  let consumed = false;
  const url = new URL(authorization);
  url.search = new URLSearchParams({ client_id: clientId, redirect_uri: redirectUri,
    response_type: "code", scope: scopes.join(" "), state,
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    code_challenge_method: "S256" }).toString();
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
        throw new PocError(params.get("error") === "invalid_scope" ? "authorization_invalid_scope" : "authorization_denied");
      }
      const code = credential(params.get("code"));
      consumed = true;
      return { code, verifier, redirectUri };
    },
    dispose() { consumed = true; verifier = ""; state = ""; },
  };
}

export async function startR2Session({ grant, clientId, scopes, endpoints = CLOUDFLARE, signal,
  issueS3Key = false, catalogOnly = false }) {
  const exchange = await request(endpoints.token, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "authorization_code", client_id: clientId,
      redirect_uri: grant.redirectUri, code: grant.code, code_verifier: grant.verifier }).toString(),
  }, { signal, maxBytes: 32 * 1024 });
  requireValue(exchange.ok, "token_exchange_failed");
  const token = json(exchange.bytes);
  requireValue(typeof token.token_type === "string" && token.token_type.toLowerCase() === "bearer"
    && Number.isSafeInteger(token.expires_in) && token.expires_in > 0, "invalid_token_response");
  // RFC 6749 allows scope omission if the requested scope did not change.
  const granted = token.scope === undefined ? scopes
    : typeof token.scope === "string" ? token.scope.trim().split(/\s+/u) : [];
  requireValue(scopes.every((scope) => granted.includes(scope))
    && granted.every((scope) => scopes.includes(scope)), "unexpected_token_scope");
  let access = credential(token.access_token);
  async function get(path) {
    const response = await request(`${endpoints.api}${path}`, {
      method: "GET", headers: { Authorization: `Bearer ${access}` },
    }, { signal, maxBytes: 256 * 1024 });
    requireValue(response.ok, response.status === 403 ? "r2_permission_denied" : "cloudflare_read_failed");
    const value = json(response.bytes);
    requireValue(value.success === true, "cloudflare_read_failed");
    return value;
  }
  const accounts = await get("/accounts?per_page=50");
  requireValue(Array.isArray(accounts.result) && accounts.result.length <= 50
    && accounts.result.every((item) => validId(item.id)), "invalid_accounts_response");
  requireValue(!(accounts.result_info?.total_pages > 1), "account_list_truncated");
  const ids = [...new Set(accounts.result.map((item) => item.id))];
  requireValue(ids.length > 0, "account_selection_required");
  let ended = false;
  const dispose = () => { ended = true; access = ""; };
  return {
    accounts: ids,
    dispose,
    async confirmAccount(selected) {
      requireValue(!ended, "attempt_ended");
      ended = true;
      try {
        requireValue(ids.includes(selected), "account_selection_required");
        if (issueS3Key || catalogOnly) {
          return { account_count: ids.length, account_confirmed: true,
            ...await runS3KeyPoc({ accessToken: access, accountId: selected, api: endpoints.api, signal, catalogOnly }) };
        }
        const buckets = await get(`/accounts/${selected}/r2/buckets?per_page=100`);
        requireValue(Array.isArray(buckets.result?.buckets) && buckets.result.buckets.length <= 100,
          "invalid_buckets_response");
        // Counts only: do not publish IDs, tokens, or raw provider responses.
        return {
          account_count: ids.length, account_confirmed: true,
          bucket_count_first_page: buckets.result.buckets.length,
          bucket_list_partial: Boolean(buckets.result_info?.cursor), jurisdiction: "default",
          oauth_token_received: true, r2_list_passed: true,
          s3_credentials_received: false, s3_probe_passed: false,
        };
      } finally { dispose(); }
    },
  };
}

export async function startPoc({ clientId, scopes, port = DEFAULT_PORT, timeoutMs = 180_000,
  accountId, mock = false, endpoints = CLOUDFLARE, issueS3Key = false, catalogOnly = false } = {}) {
  requireValue(!(issueS3Key && catalogOnly), "one_poc_operation_required");
  // A test endpoint cannot be selected by a live CLI flag.
  for (const [key, endpoint] of Object.entries(endpoints)) {
    const url = new URL(endpoint);
    requireValue(mock ? url.protocol === "http:" && url.hostname === "127.0.0.1"
      : endpoint === CLOUDFLARE[key], "invalid_provider_endpoint");
  }
  if (accountId !== undefined) requireValue(validId(accountId), "invalid_account_id");
  const controller = new AbortController();
  let flow;
  let session;
  let origin;
  let result;
  let checking = false;
  let timer;
  let resolveDone;
  const done = new Promise((resolve) => { resolveDone = resolve; });
  const startKey = randomBytes(24).toString("hex");
  const finish = (status, detail = {}) => {
    if (result) return;
    clearTimeout(timer);
    flow?.dispose();
    session?.dispose();
    result = { status, evidence: mock ? "local_mock" : "live_cloudflare",
      wrangler_used: false, credentials_saved: false, connection_saved: false,
      remote_storage_changed: false, ...detail };
    resolveDone(result);
  };
  const server = createServer({ maxHeaderSize: 8192 }, async (req, res) => {
    if (!["GET", "POST"].includes(req.method) || req.headers.host !== new URL(origin).host
      || !req.url?.startsWith("/") || req.url.startsWith("//") || req.url.length > 8192) {
      sendPage(res, 400, "Request rejected", "<p>Use the POC start link.</p>");
      return;
    }
    const url = new URL(req.url, origin);
    const redirectTo = (path) => res.writeHead(303, {
      Location: path, "Cache-Control": "no-store", "Referrer-Policy": "no-referrer",
    }).end();
    if (req.method === "GET" && url.pathname === `/start/${startKey}` && !url.search && !result) {
      sendPage(res, 200, "R2 setup without Wrangler", `<p>${mock ? "Local mock. No Cloudflare account is used." : "Live Cloudflare test. Sign in with the registered client."}</p><p>${catalogOnly ? "This POC reads the OAuth scope catalogue. It does not request a key or create a bucket." : issueS3Key ? "This POC reads the OAuth scope catalogue and sends one S3 key request. The key is restricted to a new test bucket name and expires after one hour. No bucket or Mallard connection is created." : "This POC checks browser authorization and lists R2 buckets. It does not create storage or issue S3 keys."}</p><a href="${escapeHtml(flow.url)}">${mock ? "Start mock sign-in" : "Sign in to Cloudflare"}</a>`);
      return;
    }
    if (req.method === "GET" && url.pathname === "/result" && result) {
      sendPage(res, 200, result.status === "POC_PASSED" ? "Browser POC passed" : "Browser POC stopped",
        `<p>${mock ? "Local mock evidence only." : "Live Cloudflare evidence."} ${catalogOnly ? "Scope catalogue check only. No S3 key is requested." : issueS3Key ? "Automatic S3 key permission check. No bucket or connection is saved." : "S3 setup is a separate check."}</p><pre>${escapeHtml(JSON.stringify(result, null, 2))}</pre>`);
      return;
    }
    if (url.pathname === `/account/${startKey}` && !url.search && session && !result && !checking) {
      if (req.method === "GET") {
        const selected = accountId ?? (session.accounts.length === 1 ? session.accounts[0] : "");
        const options = session.accounts.map((id, index) => `<option value="${id}"${id === selected ? " selected" : ""}>Account ${index + 1} · ${id}</option>`).join("");
        sendPage(res, 200, "Choose the Cloudflare account", `<p>Confirm the account before Mallard ${catalogOnly ? "reads the scope catalogue" : issueS3Key ? "requests a test S3 key" : "reads its buckets"}.</p><form method="post"><label>Cloudflare account <select name="account_id"><option value="">Choose an account</option>${options}</select></label><p><button name="action" value="continue">Continue</button> <button name="action" value="change-login">Change login</button></p></form><p>No retry or recovery is available after an attempt ends.</p>`);
        return;
      }
      checking = true;
      try {
        requireValue(!req.headers.origin || req.headers.origin === origin, "invalid_form_origin");
        const parts = [];
        let length = 0;
        for await (const chunk of req) {
          length += chunk.length;
          requireValue(length <= 4096, "form_too_large");
          parts.push(chunk);
        }
        const form = new URLSearchParams(Buffer.concat(parts).toString());
        requireValue(!result && !controller.signal.aborted, "attempt_ended");
        requireValue(form.getAll("action").length === 1, "invalid_action");
        if (form.get("action") === "change-login") {
          session.dispose();
          session = undefined;
          flow.dispose();
          flow = newAuthorization({ clientId, scopes, redirectUri: `${origin}${CALLBACK_PATH}`,
            authorization: endpoints.authorization });
          checking = false;
          redirectTo(flow.url);
          return;
        }
        requireValue(form.get("action") === "continue" && form.getAll("account_id").length === 1,
          "invalid_action");
        finish("POC_PASSED", await session.confirmAccount(form.get("account_id")));
      } catch (error) {
        finish("POC_STOPPED", { code: error instanceof PocError ? error.code : "poc_failed" });
      }
      redirectTo("/result");
      return;
    }
    if (req.method !== "GET" || url.pathname !== CALLBACK_PATH || result || checking) {
      sendPage(res, 400, "Request rejected", "<p>No active callback is available.</p>");
      return;
    }
    try {
      const grant = flow.consume(url.toString());
      checking = true;
      try {
        const received = await startR2Session({ grant, clientId, scopes, endpoints,
          signal: controller.signal, issueS3Key, catalogOnly });
        if (result) received.dispose();
        else session = received;
      } finally {
        grant.code = "";
        grant.verifier = "";
      }
      checking = false;
    } catch (error) {
      if (!checking && !(error instanceof PocError
        && ["authorization_denied", "authorization_invalid_scope"].includes(error.code))) {
        sendPage(res, 400, "Callback rejected", "<p>The sign-in is still waiting for a valid callback.</p>");
        return;
      }
      finish("POC_STOPPED", { code: error instanceof PocError ? error.code : "poc_failed" });
    }
    redirectTo(result ? "/result" : `/account/${startKey}`);
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.timeout = 45_000;
  server.maxConnections = 8;
  await new Promise((resolve, reject) => {
    server.once("error", () => reject(new PocError("callback_listener_unavailable")));
    server.listen(port, "127.0.0.1", resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  try {
    flow = newAuthorization({ clientId, scopes, redirectUri: `${origin}${CALLBACK_PATH}`,
      authorization: endpoints.authorization });
  } catch (error) {
    await new Promise((resolve) => server.close(resolve));
    throw error;
  }
  timer = setTimeout(() => {
    controller.abort();
    finish("POC_STOPPED", { code: "authorization_timeout" });
  }, timeoutMs);
  return {
    startUrl: `${origin}/start/${startKey}`, redirectUri: `${origin}${CALLBACK_PATH}`, done,
    async close() {
      controller.abort();
      finish("POC_STOPPED", { code: "cancelled" });
      server.closeAllConnections();
      if (server.listening) await new Promise((resolve) => server.close(resolve));
    },
  };
}

async function main() {
  const { values } = parseArgs({ options: {
    mock: { type: "boolean" }, "client-id": { type: "string" }, scopes: { type: "string" },
    "account-id": { type: "string" }, port: { type: "string" }, help: { type: "boolean" },
    "issue-s3-key": { type: "boolean" },
    "scope-catalog": { type: "boolean" },
  } });
  if (values.help) {
    console.log("Local: node poc/r2-browser-oauth/browser-oauth.mjs --mock\nLive: node poc/r2-browser-oauth/browser-oauth.mjs --client-id ID --scopes 'SCOPES_FROM_REGISTERED_CLIENT' [--account-id ID]\nRegister callback: http://127.0.0.1:53684/r2-poc/callback\nDefault: list buckets. --scope-catalog: inspect available scopes only. --issue-s3-key: send one token-creation request for a new test bucket name, expiring after one hour. These two flags are mutually exclusive. No bucket or connection is created. Stop with Ctrl+C.");
    return;
  }
  requireValue(!values.mock || (!values["client-id"] && !values.scopes && !values["account-id"]), "mixed_mock_and_live_options");
  const port = values.port === undefined ? DEFAULT_PORT : Number(values.port);
  requireValue(Number.isInteger(port) && port >= 1024 && port <= 65535, "invalid_port");
  let provider;
  let poc;
  try {
    provider = values.mock ? await (await import("./mock-provider.mjs")).startMockProvider() : undefined;
    poc = await startPoc({ mock: Boolean(values.mock), port,
      clientId: provider?.clientId ?? values["client-id"],
      scopes: provider?.scopes ?? values.scopes?.split(/\s+/u).filter(Boolean),
      endpoints: provider?.endpoints ?? CLOUDFLARE, accountId: values["account-id"],
      issueS3Key: Boolean(values["issue-s3-key"]), catalogOnly: Boolean(values["scope-catalog"]) });
    console.log(JSON.stringify({ status: "OPEN_BROWSER", evidence: values.mock ? "local_mock" : "live_cloudflare",
      url: poc.startUrl, wrangler_used: false }));
    const stop = async () => { await poc.close(); await provider?.close(); };
    process.once("SIGINT", stop);
    process.once("SIGTERM", stop);
    console.log(JSON.stringify(await poc.done));
    // Keep the result visible briefly, then release the listener.
    const cleanup = setTimeout(stop, 60_000);
    cleanup.unref();
  } catch (error) {
    await poc?.close();
    await provider?.close();
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(JSON.stringify({ status: "POC_STOPPED", code: error instanceof PocError ? error.code : "invalid_arguments_or_local_error" }));
    process.exitCode = 1;
  });
}

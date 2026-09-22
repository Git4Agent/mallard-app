import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { Writable } from "node:stream";
import { pathToFileURL } from "node:url";
import { FeishuDrive, PocError, RequestGate, readBounded, requireValue } from "./drive.mjs";
import { runProbe } from "./probe.mjs";
import { buildSelector } from "./selector.mjs";

export const USER_SCOPES = Object.freeze([
  "space:document:retrieve", "space:folder:create", "drive:file:upload",
  "drive:file:download", "auth:user.id:read",
]);
const AUTHORIZE_URL = "https://accounts.feishu.cn/open-apis/authen/v1/authorize";
const TOKEN_URL = "https://accounts.feishu.cn/oauth/v3/token";
const escapeHtml = (value) => value.replaceAll("&", "&amp;").replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;").replaceAll('"', "&quot;");
const secretText = (value) => typeof value === "string" && /^[\x21-\x7e]{1,16384}$/u.test(value);
const equal = (left, right) => typeof left === "string" && Buffer.byteLength(left) === Buffer.byteLength(right)
  && timingSafeEqual(Buffer.from(left), Buffer.from(right));

// A developer-only confidential client. This is not a desktop OAuth design.
export async function startOAuthProbe({ parentFolderToken, clientId, clientSecret, run, port = 54321,
  fetchImpl = fetch, onResult = () => {}, now = Date.now }) {
  requireValue(/^[a-zA-Z0-9_-]{1,256}$/u.test(parentFolderToken), "invalid_parent_folder_token");
  requireValue(/^cli_[a-zA-Z0-9]{1,120}$/u.test(clientId ?? "") && secretText(clientSecret), "invalid_app_credentials");
  requireValue(Number.isInteger(port) && port >= 0 && port <= 65535, "invalid_port");
  const oauthState = randomBytes(32).toString("hex");
  const verifier = randomBytes(48).toString("base64url");
  let origin;
  let phase = "authorizing";
  let credentials = { client_id: clientId, client_secret: clientSecret };
  clientSecret = undefined;
  let authorizationUrl;
  const startedAt = now();
  let result;
  const finish = (value) => { credentials = undefined; result = value; phase = "finished"; onResult(value); };
  const fail = (error) => finish({ verdict: "FAIL", mode: "live", production_ready: false,
    code: error instanceof PocError && /^[a-z0-9_]+$/u.test(error.code) ? error.code : "unexpected_failure" });
  const server = createServer(async (request, response) => {
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("Referrer-Policy", "no-referrer");
    response.setHeader("Content-Security-Policy", "default-src 'none'; form-action 'none'; frame-ancestors 'none'; base-uri 'none'");
    response.setHeader("X-Content-Type-Options", "nosniff");
    const reply = (status, body, type = "text/plain; charset=utf-8") => {
      response.writeHead(status, { "Content-Type": type }); response.end(body);
    };
    const redirect = (location) => { response.writeHead(303, { Location: location }); response.end(); };
    try {
      if (request.headers.host !== new URL(origin).host) return reply(403, "host_rejected");
      if (request.url.length > 20_000) return reply(414, "url_too_large");
      const url = new URL(request.url, origin);
      if (request.method === "GET" && request.url === "/") {
        const body = `${phase === "authorizing" ? `<p><a href="${escapeHtml(authorizationUrl)}">Continue to Feishu</a></p>` : ""}
<p>${phase === "finished" ? "Probe finished." : "Authorization or probe in progress. Refresh for the result."}</p>
<pre>${escapeHtml(JSON.stringify(result ?? { state: phase }, null, 2))}</pre>`;
        return reply(200, `<!doctype html><meta charset="utf-8"><title>Mallard Feishu OAuth probe</title>
<h1>Mallard Feishu OAuth probe</h1><p>One run in the configured test folder. Synthetic data only.
The app secret was read from the terminal. It and the user token stay in memory. They are not saved or logged.
The app secret is sent only to Feishu's OAuth token endpoint. No refresh token is requested.</p>${body}`, "text/html; charset=utf-8");
      }
      if (request.method !== "GET") return reply(405, "method_not_allowed");
      if (request.method === "GET" && url.pathname === "/oauth/callback") {
        if (phase !== "authorizing") return reply(409, "callback_not_pending");
        if (url.searchParams.getAll("state").length !== 1 || !equal(url.searchParams.get("state"), oauthState)) {
          return reply(403, "state_rejected");
        }
        if (now() - startedAt > 5 * 60_000) { fail(new PocError("authorization_expired")); return redirect("/"); }
        if (url.searchParams.has("error")) { fail(new PocError("authorization_denied")); return redirect("/"); }
        if (url.searchParams.getAll("code").length !== 1 || !secretText(url.searchParams.get("code"))) {
          return reply(400, "invalid_authorization_code");
        }
        phase = "exchanging";
        const code = url.searchParams.get("code");
        // Remove the code from the address bar before the long-running probe.
        redirect("/");
        try {
          const tokenResponse = await fetchImpl(TOKEN_URL, { method: "POST", redirect: "error",
            signal: AbortSignal.timeout(30_000), headers: { "Content-Type": "application/x-www-form-urlencoded" },
            body: new URLSearchParams({ ...credentials, grant_type: "authorization_code", code,
              redirect_uri: `${origin}/oauth/callback`, code_verifier: verifier, scope: USER_SCOPES.join(" ") }) });
          credentials = undefined;
          const tokenBytes = await readBounded(tokenResponse, 64 * 1024);
          let tokenResult;
          try { tokenResult = JSON.parse(tokenBytes.toString("utf8")); } finally { tokenBytes.fill(0); }
          requireValue(tokenResult && typeof tokenResult === "object", "invalid_oauth_response");
          if (Number.isSafeInteger(tokenResult.code) && tokenResult.code !== 0) throw new PocError(`oauth_${tokenResult.code}`);
          requireValue(tokenResponse.status === 200 && tokenResult.code === 0, "oauth_exchange_failed");
          requireValue(secretText(tokenResult.access_token) && tokenResult.token_type?.toLowerCase() === "bearer"
            && Number.isFinite(tokenResult.expires_in) && tokenResult.expires_in > 30, "invalid_oauth_response");
          const scopes = typeof tokenResult.scope === "string" ? tokenResult.scope.split(" ").filter(Boolean) : [];
          requireValue(scopes.length === USER_SCOPES.length && new Set(scopes).size === USER_SCOPES.length
            && USER_SCOPES.every((scope) => scopes.includes(scope)), "oauth_scope_mismatch");
          requireValue(!tokenResult.refresh_token, "unexpected_refresh_token");
          phase = "running";
          const probeResult = await run(tokenResult.access_token, parentFolderToken);
          tokenResult = undefined;
          finish({ ...probeResult, oauth: "v3_with_pkce", granted_scopes: scopes });
        } catch (error) { fail(error); }
        return;
      }
      return reply(404, "not_found");
    } catch {
      if (!response.headersSent) reply(400, "request_failed");
      else response.end();
    }
  });
  server.requestTimeout = 15_000;
  server.headersTimeout = 10_000;
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  const authorize = new URL(AUTHORIZE_URL);
  authorize.search = new URLSearchParams({ client_id: clientId,
    response_type: "code", redirect_uri: `${origin}/oauth/callback`,
    scope: USER_SCOPES.join(" "), state: oauthState, code_challenge_method: "S256",
    code_challenge: createHash("sha256").update(verifier).digest("base64url") }).toString();
  authorizationUrl = authorize.href;
  server.once("close", () => { credentials = undefined; });
  return { server, origin, status: () => ({ state: phase, result }) };
}

export async function readTerminalSecret(input = process.stdin, output = process.stderr) {
  requireValue(input.isTTY === true, "secret_requires_terminal");
  output.write("Feishu App Secret (input hidden): ");
  // Readline handles terminal input and paste. Its output never echoes the input.
  const muted = new Writable({ write(_chunk, _encoding, done) { done(); } });
  const prompt = createInterface({ input, output: muted, terminal: true, historySize: 0 });
  try {
    return await new Promise((resolve, reject) => {
      prompt.once("SIGINT", () => reject(new PocError("secret_entry_cancelled")));
      prompt.once("close", () => reject(new PocError("secret_entry_cancelled")));
      prompt.question("", (value) => {
        if (!secretText(value)) reject(new PocError("invalid_app_credentials"));
        else resolve(value);
      });
    });
  } finally { prompt.close(); output.write("\n"); }
}

async function main() {
  const [appFlag, clientId, folderFlag, parentFolderToken, ...extra] = process.argv.slice(2);
  requireValue(appFlag === "--app-id" && folderFlag === "--parent-folder" && !extra.length,
    "usage_--app-id_id_--parent-folder_token");
  const selector = buildSelector();
  const runId = randomBytes(8).toString("hex");
  let requests = 0;
  const probe = await startOAuthProbe({ parentFolderToken, clientId, clientSecret: await readTerminalSecret(),
    run: async (accessToken, folder) => {
      console.log(JSON.stringify({ mode: "live", probe_folder_name: `mallard-feishu-poc-${runId}`,
        data: "synthetic_only", cleanup: "retained_no_delete_api" }));
      const gate = new RequestGate();
      const fetchImpl = async (url, options) => {
        requests += 1;
        if (requests % 25 === 0) console.log(JSON.stringify({ state: "running", http_requests: requests }));
        return fetch(url, options);
      };
      const writer = new FeishuDrive({ accessToken, fetchImpl, gate, pageSize: 2 });
      const reader = new FeishuDrive({ accessToken, fetchImpl, gate, pageSize: 2 });
      return { ...await runProbe({ writer, reader, parentFolderToken: folder, selector, runId }),
        mode: "live", distinct_credentials: false, http_requests: requests };
    }, onResult: (result) => console.log(JSON.stringify({ ...result, http_requests: requests }, null, 2)),
  });
  console.log(JSON.stringify({ state: "ready", url: probe.origin, callback: `${probe.origin}/oauth/callback`, expires_in_minutes: 15 }));
  const close = () => { probe.server.close(); probe.server.closeAllConnections(); process.exit(0); };
  const deadline = setTimeout(close, 15 * 60_000);
  process.once("SIGINT", () => { clearTimeout(deadline); close(); });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(() => { console.error("oauth_probe_start_failed"); process.exitCode = 1; });
}

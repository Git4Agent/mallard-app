import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { FeishuDrive, PocError, RequestGate, requireValue } from "./drive.mjs";
import { runProbe } from "./probe.mjs";
import { buildSelector } from "./selector.mjs";

const escapeHtml = (value) => value.replaceAll("&", "&amp;").replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;").replaceAll('"', "&quot;");

// A single-use loopback form passes a short-lived token to the existing probe.
// The token stays in process memory. No cookies, browser storage, or files.
export async function startBrowserProbe({ parentFolderToken, run, onResult = () => {} }) {
  requireValue(/^[a-zA-Z0-9_-]{1,256}$/u.test(parentFolderToken), "invalid_parent_folder_token");
  const nonce = randomBytes(32).toString("hex");
  let origin;
  let state = "ready";
  let result;
  const server = createServer(async (request, response) => {
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("Content-Security-Policy", "default-src 'none'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'");
    response.setHeader("X-Content-Type-Options", "nosniff");
    const reply = (status, body, type = "text/plain; charset=utf-8") => {
      response.writeHead(status, { "Content-Type": type });
      response.end(body);
    };
    try {
      if (request.headers.host !== new URL(origin).host) return reply(403, "host_rejected");
      if (request.method === "GET" && request.url === "/") {
        const body = state === "ready"
          ? `<form method="post" action="/probe" autocomplete="off">
<input type="hidden" name="nonce" value="${nonce}">
<p><label>Feishu user token <input type="password" name="token" required maxlength="16384" autocomplete="off"></label></p>
<button type="submit">Run synthetic Drive probe</button></form>`
          : `<p>${state === "running" ? "Probe running. Refresh this page for the result." : "Probe finished."}</p>
<pre>${escapeHtml(JSON.stringify(result ?? { state }, null, 2))}</pre>`;
        return reply(200, `<!doctype html><meta charset="utf-8"><title>Mallard Feishu API probe</title>
<h1>Mallard Feishu API probe</h1><p>One run in the configured test folder. Synthetic data only.
The token stays in memory and is not saved or logged. The remote test folder is retained.</p>${body}`, "text/html; charset=utf-8");
      }
      if (request.method !== "POST" || request.url !== "/probe") return reply(404, "not_found");
      if (request.headers.origin !== origin) return reply(403, "origin_rejected");
      if (request.headers["content-type"]?.split(";")[0] !== "application/x-www-form-urlencoded") {
        return reply(415, "content_type_rejected");
      }
      if (state !== "ready") return reply(409, "probe_already_started");
      if (Number(request.headers["content-length"]) > 64 * 1024) return reply(413, "body_too_large");
      const chunks = [];
      let size = 0;
      for await (const chunk of request) {
        size += chunk.length;
        if (size > 64 * 1024) return reply(413, "body_too_large");
        chunks.push(chunk);
      }
      const body = Buffer.concat(chunks);
      const fields = new URLSearchParams(body.toString("utf8"));
      body.fill(0);
      for (const chunk of chunks) chunk.fill(0);
      const suppliedNonce = Buffer.from(fields.get("nonce") ?? "");
      if (suppliedNonce.length !== nonce.length || !timingSafeEqual(suppliedNonce, Buffer.from(nonce))) {
        return reply(403, "nonce_rejected");
      }
      let accessToken = fields.get("token");
      const tokenCount = fields.getAll("token").length;
      fields.delete("token");
      if (tokenCount !== 1 || fields.getAll("nonce").length !== 1 || [...fields.keys()].some((key) => key !== "nonce")
        || typeof accessToken !== "string" || !/^[\x21-\x7e]{1,16384}$/u.test(accessToken)) {
        return reply(400, "invalid_form");
      }
      // Recheck after the asynchronous body read to reject concurrent submits.
      if (state !== "ready") return reply(409, "probe_already_started");
      state = "running";
      Promise.resolve().then(() => run(accessToken, parentFolderToken)).then(
        (value) => { result = value; },
        (error) => { result = { verdict: "FAIL", mode: "live", production_ready: false,
          code: error instanceof PocError && /^[a-z0-9_]+$/u.test(error.code) ? error.code : "unexpected_failure" }; },
      ).finally(() => {
        accessToken = undefined;
        state = "finished";
        onResult(result);
      });
      response.writeHead(303, { Location: "/" });
      response.end();
    } catch {
      if (!response.headersSent) reply(400, "request_failed");
      else response.end();
    }
  });
  server.requestTimeout = 15_000;
  server.headersTimeout = 10_000;
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  return { server, origin, status: () => ({ state, result }) };
}

async function main() {
  const [flag, parentFolderToken, ...extra] = process.argv.slice(2);
  requireValue(flag === "--parent-folder" && !extra.length, "usage_--parent-folder_token");
  const selector = buildSelector();
  let requests = 0;
  const runId = randomBytes(8).toString("hex");
  const probe = await startBrowserProbe({ parentFolderToken,
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
    },
    onResult: (result) => console.log(JSON.stringify({ ...result, http_requests: requests }, null, 2)),
  });
  console.log(JSON.stringify({ state: "ready", url: probe.origin, expires_in_minutes: 15 }));
  const deadline = setTimeout(() => { probe.server.close(); probe.server.closeAllConnections(); process.exit(0); }, 15 * 60_000);
  process.once("SIGINT", () => { clearTimeout(deadline); probe.server.close(); probe.server.closeAllConnections(); process.exit(0); });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(() => { console.error("browser_probe_start_failed"); process.exitCode = 1; });
}

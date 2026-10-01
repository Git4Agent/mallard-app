// Production Cloud assets and a preview built by Rust from a real fixture Push.
// The API is local and read-only. This server does not publish a Cloud release.
import { createServer } from "node:http";
import { readFile, realpath } from "node:fs/promises";
import { resolve, extname } from "node:path";
import { fileURLToPath } from "node:url";
const app = fileURLToPath(new URL("../../", import.meta.url));
const fixtureRoot = await realpath((await readFile(resolve(app, "output/mermaid-flowcharts/fixture-root.txt"), "utf8")).trim());
if (!fixtureRoot.startsWith("/private/tmp/mallard-mermaid-qa-")) throw new Error("Use an isolated fixture root");
const dist = resolve(app, "../mallardInternal/cloudflare/managed-cloud/web/dist");
const fixture = JSON.parse(await readFile(resolve(fixtureRoot, "fixture.json"), "utf8"));
const view = await readFile(resolve(fixtureRoot, "cloud-view.json"));
const manifest = JSON.parse(await readFile(resolve(fixtureRoot, "cloud-manifest.json"), "utf8"));
const hashes = JSON.parse(await readFile(resolve(dist, "mermaid-renderer-hashes.json"), "utf8"));
const childCsp = `default-src 'none'; script-src ${hashes.map(hash => `'sha256-${hash}'`).join(" ")}; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'`;
const pageCsp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";
const owner = { user_id: "fixture-user", github_login: "fixture", avatar_url: "/mallard-logo-duck.svg" };
const head = { scope: "public", revision: 1, commit_id: fixture.commit_id, manifest_sha256: "0".repeat(64), manifest_size: 0, updated_at: 1790812800, kind: "project", bundle_id: manifest.bundle_id, pushed_by: owner };
const repository = { id: "fixture-repository", owner, name: "session-diagrams", repository_class: "non_git_project", bundle_id: manifest.bundle_id, repository_fingerprint: null, github_url: null, description: "Local session display verification", github_description: null, catalog: null, revision: 1, state: "active", visibility: "public", private_revision: 1, public_revision: 1, public_admitted_at: 1790812800, committed_bytes: 0, star_count: 0, member_count: 1, viewer_starred: false, created_at: 1790812800, updated_at: 1790812800, role: null, writer: false, private_head: null, public_head: head, private_summary: null, public_summary: null, open_release_request: null };
createServer(async (request, response) => {
  const url = new URL(request.url, "http://localhost");
  const path = url.pathname;
  response.setHeader("X-Content-Type-Options", "nosniff");
  response.setHeader("Cache-Control", "no-store");
  if (request.method !== "GET") { response.writeHead(405); response.end(); return; }
  if (path.startsWith("/v1/")) {
    response.setHeader("Content-Type", "application/json");
    if (path === "/v1/me") { response.writeHead(401); response.end(JSON.stringify({ error: "Fixture signed-out viewer" })); }
    else if (path === "/v1/repositories/by-name/fixture/session-diagrams") response.end(JSON.stringify(repository));
    else if (path === "/v1/public/repositories/fixture-repository/manifest") response.end(JSON.stringify(manifest));
    else if (path === `/v1/public/repositories/fixture-repository/views/${encodeURIComponent(fixture.resource_id)}` && url.searchParams.get("commit_id") === fixture.commit_id) response.end(view);
    else { response.writeHead(404); response.end(JSON.stringify({ error: "Fixture route absent" })); }
    return;
  }
  const parser = path === "/mermaid-renderer.html";
  response.setHeader("Content-Security-Policy", parser ? childCsp : pageCsp);
  response.setHeader("X-Frame-Options", parser ? "SAMEORIGIN" : "DENY");
  const name = path === "/" || !extname(path) ? "index.html" : path.slice(1);
  if (name.includes("..")) { response.writeHead(400); response.end(); return; }
  response.setHeader("Content-Type", { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".png": "image/png" }[extname(name)] ?? "application/octet-stream");
  try { response.end(await readFile(resolve(dist, name))); } catch { response.writeHead(404); response.end(); }
}).listen(64483, "127.0.0.1", () => console.log(`Cloud session fixture: http://127.0.0.1:64483/fixture/session-diagrams/sessions/${encodeURIComponent(fixture.resource_id)}?commit=${fixture.commit_id}&scope=public`));

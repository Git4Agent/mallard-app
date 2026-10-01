import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { dirname, resolve, extname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/mermaid-flowcharts/web");
let canary = 0;
const localCsp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; media-src 'self' data:; frame-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'";
const cloudCsp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";
const hashes = JSON.parse(await readFile(resolve(root, "mermaid-renderer-hashes.json"), "utf8"));
const childCsp = `default-src 'none'; script-src ${hashes.map((hash) => `'sha256-${hash}'`).join(" ")}; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'`;
createServer(async (request, response) => {
  const path = new URL(request.url, "http://localhost").pathname;
  if (path === "/canary") { canary++; response.end("unexpected"); return; }
  if (path === "/stats") { response.setHeader("Content-Type", "application/json"); response.end(JSON.stringify({ canary })); return; }
  if (!/^\/(?:[\w.-]+\.(?:html|js|css|json|woff2)|fonts\/[\w.-]+\.woff2)$/.test(path)) { response.writeHead(404); response.end(); return; }
  response.setHeader("Content-Security-Policy", path === "/mermaid-renderer.html" ? childCsp : path.startsWith("/cloud") ? cloudCsp : localCsp);
  response.setHeader("X-Frame-Options", path === "/mermaid-renderer.html" ? "SAMEORIGIN" : "DENY");
  response.setHeader("Content-Type", { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".json": "application/json" }[extname(path)]);
  try { response.end(await readFile(resolve(root, path.slice(1)))); } catch { response.writeHead(404); response.end(); }
}).listen(64482, "127.0.0.1", () => console.log("Session renderer verification: http://127.0.0.1:64482/desktop.html and /cloud.html"));

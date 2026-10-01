import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import type { Plugin } from "vite";

const require = createRequire(import.meta.url);
const LIBRARY_SHA256 = "85d76389b789056300eb32e084b2a10c42c6545f08cae28c50382a445f0fef4a";

/** Emit the parser separately; it must not inherit the app's script or network access. */
export function mermaidRenderer(controller: URL): Plugin {
  const libraryPath = require.resolve("@mermaid-js/tiny");
  const library = readFileSync(libraryPath, "utf8");
  if (createHash("sha256").update(library).digest("hex") !== LIBRARY_SHA256) throw new Error("Mermaid package checksum mismatch");
  const scripts = [library, readFileSync(controller, "utf8")].map((script) => script.replace(/<\/script/gi, "<\\/script"));
  const hashes = scripts.map((script) => createHash("sha256").update(script).digest("base64"));
  const policy = `default-src 'none'; script-src ${hashes.map((hash) => `'sha256-${hash}'`).join(" ")}; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'`;
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><title>Flowchart parser</title></head><body><div id="stage"></div>${scripts.map((script) => `<script>${script}</script>`).join("\n")}</body></html>`;
  const embedded = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((match) => match[1]);
  if (embedded.length !== scripts.length || embedded.some((script, index) => script !== scripts[index])) throw new Error("Parser script hash mismatch");
  const assets = [
    { name: "mermaid-renderer.html", source: html, type: "text/html; charset=utf-8" },
    { name: "mermaid-renderer-hashes.json", source: JSON.stringify(hashes), type: "application/json" },
    { name: "mermaid-LICENSE.txt", source: readFileSync(resolve(dirname(libraryPath), "../LICENSE"), "utf8"), type: "text/plain" },
  ];
  return {
    name: "mallard-mermaid-renderer",
    generateBundle() {
      for (const asset of assets) this.emitFile({ type: "asset", fileName: asset.name, source: asset.source });
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = new URL(request.url ?? "/", "http://localhost").pathname;
        const asset = assets.find((entry) => path === `/${entry.name}`);
        if (!asset) { next(); return; }
        response.setHeader("Content-Type", asset.type);
        response.setHeader("Content-Security-Policy", `${policy}; frame-ancestors 'self'`);
        response.setHeader("X-Frame-Options", "SAMEORIGIN");
        response.setHeader("Cache-Control", "no-store");
        response.end(asset.source);
      });
    },
  };
}

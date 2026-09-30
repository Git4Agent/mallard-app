import { createServer } from "node:http";
import { open, readFile } from "node:fs/promises";
import { constants } from "node:fs";

const web = new URL("./web/", import.meta.url);
const args = process.argv.slice(2);
const widgetFileIndex = args.indexOf("--widget-file");
let widgetCode = await readFile(new URL("diagram.svg", web), "utf8");
if (widgetFileIndex >= 0) {
  const file = await open(args[widgetFileIndex + 1], constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > 1024 * 1024) throw new Error("Widget input must be a regular file below 1 MiB.");
    widgetCode = await file.readFile("utf8");
  } finally {
    await file.close();
  }
}

export const childCsp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

function widgetDocument(kind, canaryUrl) {
  const styles = "html{color-scheme:dark;background:#18191a;color:#e6e5e0;font:15px -apple-system,BlinkMacSystemFont,sans-serif}body{margin:0;padding:40px}h1{font-size:20px;font-weight:550;margin:0 0 12px}p{color:#9ea7a2;line-height:1.6}button{font:inherit;border:1px solid #7ec5a8;background:#115942;color:#bfe6d4;border-radius:6px;padding:10px 18px;cursor:pointer}output{display:block;font-size:64px;font-weight:550;margin:24px 0;color:#9bd5bb}code{color:#b6c8be}.sub{font-size:12px;color:#8c9a92}";
  const interactive = `<h1>Session sync counter</h1><p>Each click changes this widget's own state.</p><output id="count" aria-live="polite">0</output><button id="increment">Sync one session</button><p class="sub">Local state inside the frame</p><script>
    let count = 0;
    const button = document.getElementById("increment");
    button.addEventListener("click", () => {
      document.getElementById("count").textContent = String(++count);
      parent.postMessage({ type: "mallard-widget-poc", report: { scriptRan: true, count } }, "*");
    });
    setTimeout(() => button.click(), 50);
  </script>`;
  const isolation = `<h1>Frame isolation</h1><p>The widget script can run. It attempts to access the parent, storage, native commands, and the network.</p><p class="sub">The parent shows the test results below.</p><script>
    (async () => {
      const blocked = (fn) => { try { fn(); return false; } catch { return true; } };
      const report = {
        scriptRan: true,
        parentDomBlocked: blocked(() => parent.document.body),
        storageBlocked: blocked(() => localStorage.setItem("widget-poc", "probe")),
        nativeApiAbsent: typeof window.__TAURI_INTERNALS__ === "undefined" && typeof window.__TAURI__ === "undefined",
        parentNativeApiBlocked: blocked(() => parent.__TAURI_INTERNALS__),
        popupBlocked: window.open("about:blank") === null,
        topNavigationBlocked: blocked(() => { top.location.href = ${JSON.stringify(canaryUrl)}; }),
        fetchBlocked: false,
        svgScriptBlocked: false,
      };
      try { await fetch(${JSON.stringify(canaryUrl)}); } catch { report.fetchBlocked = true; }
      // The image's script would turn its green rectangle red if it ran.
      const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect id="pixel" width="2" height="2" fill="#00ff00"/><script>document.getElementById("pixel").setAttribute("fill","#ff0000")<' + '/script></svg>';
      const image = new Image();
      image.src = "data:image/svg+xml;base64," + btoa(svg);
      await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; });
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 2;
      const context = canvas.getContext("2d");
      context.drawImage(image, 0, 0);
      const pixel = context.getImageData(0, 0, 1, 1).data;
      report.svgScriptBlocked = pixel[0] === 0 && pixel[1] === 255 && pixel[2] === 0;
      // Resource loads also attempt to reach the local request counter.
      const externalImage = new Image();
      externalImage.src = ${JSON.stringify(canaryUrl + "?image")};
      const style = document.createElement("style");
      style.textContent = 'body{background-image:url(' + ${JSON.stringify(canaryUrl + "?css")} + ')}';
      document.head.append(style);
      parent.postMessage({ type: "mallard-widget-poc", report }, "*");
    })();
  </script>`;
  return `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Widget frame</title><style>${styles}</style></head><body>${kind === "interactive" ? interactive : isolation}</body></html>`;
}

let origin;
const counts = { canaryRequests: 0, controlRequests: 0 };
const assets = new Map([
  ["/", ["index.html", "text/html; charset=utf-8"]],
  ["/index.html", ["index.html", "text/html; charset=utf-8"]],
  ["/app.js", ["app.js", "text/javascript; charset=utf-8"]],
  ["/styles.css", ["styles.css", "text/css; charset=utf-8"]],
]);
const server = createServer(async (request, response) => {
  try {
    const path = new URL(request.url, origin).pathname;
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("X-Content-Type-Options", "nosniff");
    response.setHeader("Referrer-Policy", "no-referrer");
    response.setHeader("Permissions-Policy", "camera=(), microphone=(), geolocation=()");
    if (path.startsWith("/widget/")) {
      const kind = path.slice("/widget/".length);
      if (!["interactive", "isolation"].includes(kind)) { response.writeHead(404).end(); return; }
      response.setHeader("Content-Security-Policy", childCsp);
      response.setHeader("Content-Type", "text/html; charset=utf-8");
      response.end(widgetDocument(kind, `${origin}/canary`));
      return;
    }
    if (path === "/canary") counts.canaryRequests += 1;
    if (path === "/control") counts.controlRequests += 1;
    if (["/stats", "/control", "/canary", "/data.json"].includes(path)) {
      response.setHeader("Content-Type", "application/json");
      response.end(JSON.stringify(path === "/data.json" ? { widget_code: widgetCode, server_url: origin, interactive_enabled: true } : counts));
      return;
    }
    const asset = assets.get(path);
    if (!asset) { response.writeHead(404).end(); return; }
    const production = JSON.parse(await readFile(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8")).app.security.csp;
    response.setHeader("Content-Security-Policy", production.replace("frame-src 'none'", `frame-src ${origin}`));
    response.setHeader("Content-Type", asset[1]);
    response.end(await readFile(new URL(asset[0], web)));
  } catch {
    response.writeHead(500).end("POC server error");
  }
});
server.listen(0, "127.0.0.1", () => {
  origin = `http://127.0.0.1:${server.address().port}`;
  console.log(JSON.stringify({ serverUrl: origin }));
});

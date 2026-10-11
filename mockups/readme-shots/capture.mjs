// Captures the README screenshots from the harness through headless Chrome.
//
//   node mockups/readme-shots/capture.mjs [view-theme-lang ...]
//
// Needs the specimen Vite server on :1437 (node_modules/.mockups-vite.config.mjs).
// Each shot runs in a fresh Chrome profile, waits for the harness to mark the
// page ready (<html data-shot-ready="1">), then captures at 2x: 1280x800 for
// the project page, 1280x720 for New project, 1280x640 for the rest (HEIGHTS).
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const BASE = "http://localhost:1437/mockups/readme-shots/shot.html";
const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "out");
mkdirSync(outDir, { recursive: true });
const scratch = process.env.SHOTS_TMP ?? tmpdir();

const all = [];
for (const view of ["project", "push", "pull", "storage", "newproject"]) {
  for (const theme of ["light", "dark"]) {
    for (const lang of ["en", "zh"]) all.push({ view, theme, lang });
  }
}
const wanted = process.argv.slice(2);
const shots = wanted.length
  ? all.filter(({ view, theme, lang }) => wanted.includes(`${view}-${theme}-${lang}`))
  : all;

const HEIGHTS = { project: 800, push: 640, pull: 640, storage: 460, newproject: 720 };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function devtoolsPort(profile) {
  const file = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 200; i += 1) {
    if (existsSync(file)) {
      const [port] = readFileSync(file, "utf8").split("\n");
      if (port) return Number(port);
    }
    await sleep(100);
  }
  throw new Error("Chrome did not report a DevTools port");
}

function connect(url) {
  const socket = new WebSocket(url);
  let nextId = 1;
  const pending = new Map();
  const listeners = [];
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    } else if (message.method) {
      listeners.forEach((listener) => listener(message));
    }
  });
  const opened = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  return {
    opened,
    send(method, params = {}) {
      const id = nextId++;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    on(listener) { listeners.push(listener); },
    close() { socket.close(); },
  };
}

async function shoot({ view, theme, lang }) {
  const name = `${view}-${theme}${lang === "zh" ? "-zh" : ""}.png`;
  const profile = mkdtempSync(join(scratch, "readme-shot-"));
  const chrome = spawn(CHROME, [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--no-first-run",
    "--no-default-browser-check",
    `--user-data-dir=${profile}`,
    "--remote-debugging-port=0",
    `--lang=${lang === "zh" ? "zh-CN" : "en-US"}`,
    "--force-device-scale-factor=2",
    `--window-size=1280,${HEIGHTS[view]}`,
    "about:blank",
  ], { stdio: "ignore" });
  const problems = [];
  try {
    const port = await devtoolsPort(profile);
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    const page = targets.find((target) => target.type === "page");
    const cdp = connect(page.webSocketDebuggerUrl);
    await cdp.opened;
    cdp.on((message) => {
      if (message.method === "Runtime.exceptionThrown") {
        problems.push(`exception: ${message.params.exceptionDetails?.exception?.description ?? message.params.exceptionDetails?.text}`);
      }
      if (message.method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(message.params.type)) {
        problems.push(`${message.params.type}: ${message.params.args.map((arg) => arg.value ?? arg.description ?? "").join(" ").slice(0, 300)}`);
      }
    });
    await cdp.send("Runtime.enable");
    await cdp.send("Page.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280, height: HEIGHTS[view], deviceScaleFactor: 2, mobile: false,
    });
    const url = `${BASE}?view=${view}&theme=${theme}&lang=${lang}`;
    await cdp.send("Page.navigate", { url });
    let state = null;
    for (let i = 0; i < 300; i += 1) {
      const { result } = await cdp.send("Runtime.evaluate", {
        expression: "document.documentElement.dataset.shotReady ?? null",
        returnByValue: true,
      });
      state = result.value;
      if (state) break;
      await sleep(100);
    }
    if (state !== "1") throw new Error(`harness never became ready (${state})`);
    const { result: audit } = await cdp.send("Runtime.evaluate", {
      expression: `(() => {
        const text = document.body.innerText;
        const html = document.body.outerHTML; // head holds only Vite's dev style ids
        return {
          unhandled: [...(window.__readmeShotsUnhandled ?? [])],
          leaks: [...new Set([...html.matchAll(/\\/Users\\/(?!demo\\b)[A-Za-z0-9._-]+/g)].map((m) => m[0]))],
          hequ: /hequ/i.test(html),
          notConnected: /desktop backend|not connected/i.test(text),
          feishu: /feishu|飞书/i.test(html),
          lang: document.documentElement.lang,
          theme: document.documentElement.dataset.theme ?? null,
        };
      })()`,
      returnByValue: true,
    });
    const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(outDir, name), Buffer.from(shot.data, "base64"));
    cdp.close();
    console.log(name, JSON.stringify(audit.value), problems.length ? `\n  ${problems.join("\n  ")}` : "");
  } finally {
    chrome.kill("SIGKILL");
    await sleep(300);
    rmSync(profile, { recursive: true, force: true });
  }
}

for (const shot of shots) {
  await shoot(shot);
}

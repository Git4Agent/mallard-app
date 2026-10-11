// Captures public Mallard Cloud pages for the README's Examples: each page in
// light/dark (prefers-color-scheme) and en/zh (the site picks its language
// from navigator.languages), 1280x760 at 2x, into out/web/.
// Usage: node mockups/readme-shots/capture-web.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PAGES = {
  "mallard-with-pelican": "https://mallard-ai.com/rikaqu0223-arch/mallard-with-pelican",
  mallardNonGit: "https://mallard-ai.com/rikaqu0223-arch/mallardNonGit",
  Maple: "https://mallard-ai.com/rikaqu0223-arch/Maple",
};
const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "out", "web");
mkdirSync(outDir, { recursive: true });
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
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
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
    close() { socket.close(); },
  };
}

async function shoot(name, url, theme, lang) {
  const profile = mkdtempSync(join(tmpdir(), "readme-web-"));
  const chrome = spawn(CHROME, [
    "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
    "--no-default-browser-check", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
    `--lang=${lang === "zh" ? "zh-CN" : "en-US"}`, "--force-device-scale-factor=2",
    "--window-size=1280,760", "about:blank",
  ], { stdio: "ignore" });
  try {
    const port = await devtoolsPort(profile);
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    const cdp = connect(targets.find((target) => target.type === "page").webSocketDebuggerUrl);
    await cdp.opened;
    await cdp.send("Page.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 760, deviceScaleFactor: 2, mobile: false });
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
    await cdp.send("Emulation.setLocaleOverride", { locale: lang === "zh" ? "zh-CN" : "en-US" });
    // The site's saved language choice wins over the browser's (boot-language.js).
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
      source: `try { localStorage.setItem("mallard.language", ${JSON.stringify(lang === "zh" ? "zh-CN" : "en")}); } catch (e) {}`,
    });
    await cdp.send("Page.navigate", { url });
    // The page is ready once its repository heading and About panel are drawn.
    for (let i = 0; i < 100; i += 1) {
      const { result } = await cdp.send("Runtime.evaluate", {
        expression: "!!document.querySelector('main') && /About|关于|简介/.test(document.querySelector('main').innerText)",
        returnByValue: true,
      });
      if (result.value) break;
      await sleep(200);
    }
    await sleep(2500); // fonts, avatars, the pet's first frames
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    const file = join(outDir, `${name}-${theme}${lang === "zh" ? "-zh" : ""}.png`);
    writeFileSync(file, Buffer.from(data, "base64"));
    cdp.close();
    console.log(file);
  } finally {
    chrome.kill("SIGKILL");
    await sleep(300);
    rmSync(profile, { recursive: true, force: true });
  }
}

for (const [name, url] of Object.entries(PAGES)) {
  for (const theme of ["light", "dark"]) {
    for (const lang of ["en", "zh"]) await shoot(name, url, theme, lang);
  }
}

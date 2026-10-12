// Separate Electron fixture. All browser state is temporary. No Codex access.
const { app, BrowserWindow } = require("electron");
const { mkdirSync, renameSync, writeFileSync } = require("node:fs");
const { dirname, join } = require("node:path");

const [reportPath, scenario] = process.argv.slice(2);
if (!reportPath || !["data-only", "exact-id", "ancestor-id", "duplicate-id", "duplicate-project"].includes(scenario)) process.exit(2);
const profile = join(dirname(reportPath), `electron-${scenario}`);
mkdirSync(profile, { recursive: true });
for (const key of ["userData", "sessionData", "logs", "crashDumps"]) app.setPath(key, profile);
app.setName("Mallard Accessibility Fixture");

const targetID = "11111111-1111-4111-8111-111111111111";
const otherID = "22222222-2222-4222-8222-222222222222";
const row = (location, current = false) => {
  const threadID = location === "other-project-row" && scenario !== "duplicate-id" ? otherID : targetID;
  // Positive controls deliberately expose an HTML id. The data-only case models Codex's data attribute.
  const identifier = scenario === "data-only" ? "" : `id="${threadID}"`;
  return `<div role="listitem"><div role="group" aria-label="Fixture row" ${scenario === "ancestor-id" ? identifier : ""}>
  <div role="button" tabindex="0" ${scenario === "ancestor-id" ? "" : identifier}
  aria-label="Fixture chat" data-thread-id="${threadID}"
  data-location="${location}" ${current ? 'aria-current="page"' : ""}>Fixture chat</div></div></div>`;
};
const project = `<div role="list" tabindex="-1" aria-label="Scheduled tasks in Fixture project">
  ${row("project")}${["exact-id", "ancestor-id", "duplicate-id"].includes(scenario) ? row("other-project-row") : ""}</div>`;
const html = `<!doctype html><html><head><meta charset="utf-8"><title>Mallard Accessibility Fixture</title>
  <style>body{font:16px system-ui;padding:24px}[role=button]{padding:12px;margin:8px;border:1px solid #aaa}
  [aria-current=page]{background:#d9eaff}</style></head><body>
  <h2>Recents</h2><div role="list" aria-label="Recents">${row("recents", true)}</div>
  <h2>Fixture project</h2>${project}${scenario === "duplicate-project" ? project : ""}
  <script>
  window.fixtureState = { ready: true, presses: 0, locationId: "chats", recentsPresent: true, events: [], selectedThreadId: null,
    buttonCount: document.querySelectorAll('[role="button"]').length };
  document.addEventListener("click", event => {
    const row = event.target.closest('[role="button"]');
    if (!row) return;
    document.querySelectorAll('[aria-current]').forEach(item => item.removeAttribute('aria-current'));
    row.setAttribute('aria-current', 'page');
    window.fixtureState.presses++;
    window.fixtureState.selectedThreadId = row.dataset.threadId;
    window.fixtureState.locationId = row.dataset.location === 'project'
      ? 'unified-project:fixture-project' : row.dataset.location;
    window.fixtureState.currentLocations = [...document.querySelectorAll('[aria-current="page"]')]
      .map(item => item.dataset.location);
    window.fixtureState.events.push({ location: row.dataset.location, detail: event.detail,
      trusted: event.isTrusted, x: event.clientX, y: event.clientY });
  });
  </script></body></html>`;

app.whenReady().then(async () => {
  app.setAccessibilitySupportEnabled(false);
  const window = new BrowserWindow({ width: 620, height: 440, show: false, title: "Mallard Accessibility Fixture",
    webPreferences: { contextIsolation: true, sandbox: true, nodeIntegration: false } });
  window.webContents.session.webRequest.onBeforeRequest({ urls: ["http://*/*", "https://*/*"] },
    (_details, callback) => callback({ cancel: true }));
  await window.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
  window.showInactive();
  let previous;
  async function report() {
    const state = await window.webContents.executeJavaScript("window.fixtureState");
    const json = JSON.stringify({ ...state, electronVersion: process.versions.electron,
      accessibilityEnabled: app.accessibilitySupportEnabled });
    if (json !== previous) {
      writeFileSync(`${reportPath}.tmp`, json);
      renameSync(`${reportPath}.tmp`, reportPath);
      previous = json;
    }
  }
  await report();
  const timer = setInterval(() => { report().catch(() => app.exit(2)); }, 100);
  window.on("closed", () => clearInterval(timer));
}).catch(() => app.exit(2));
app.on("window-all-closed", () => app.quit());

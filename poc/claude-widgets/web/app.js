/* POC only. These files are not imported by the Mallard application. */
const MAX_WIDGET_BYTES = 1024 * 1024;
const native = typeof window.__TAURI_INTERNALS__?.invoke === "function";
const host = document.getElementById("widget-host");
const source = document.getElementById("source");
let data;
let activeFrame;
let currentKind = "diagram";
let lastReport = {};

function fail(message) {
  const error = document.getElementById("error");
  error.textContent = message;
  error.hidden = false;
}

function svgImage(widgetCode) {
  const bytes = new TextEncoder().encode(widgetCode);
  if (bytes.length > MAX_WIDGET_BYTES) throw new Error("Widget exceeds the preview limit.");
  const documentCode = widgetCode.replace(/<svg\b([^>]*)>/, (root, attributes) => (
    /\bxmlns\s*=/.test(attributes) ? root : `<svg xmlns="http://www.w3.org/2000/svg"${attributes}>`
  ));
  const parsed = new DOMParser().parseFromString(documentCode, "image/svg+xml");
  if (parsed.querySelector("parsererror") || parsed.documentElement.localName !== "svg"
      || parsed.documentElement.namespaceURI !== "http://www.w3.org/2000/svg") {
    throw new Error("The saved code is not a complete SVG document.");
  }
  // Claude supplies these presentation classes outside widget_code. Apply
  // their small, fixed local equivalents inside the SVG image document.
  const style = parsed.createElementNS("http://www.w3.org/2000/svg", "style");
  style.textContent = "text{font-family:-apple-system,BlinkMacSystemFont,sans-serif;fill:#c7c8bf}.th{font-size:16px;font-weight:600}.ts{font-size:13px}.c-teal rect,rect.c-teal{fill:#075e4f;stroke:#68baa2}.c-teal .th{fill:#a1ded0}.c-teal .ts{fill:#70c4ac}.c-gray rect,rect.c-gray{fill:#454744;stroke:#a2a79d}.c-gray .th{fill:#e0dfd7}.box{fill:none;stroke:#404242}";
  parsed.documentElement.prepend(style);
  const encoded = new TextEncoder().encode(new XMLSerializer().serializeToString(parsed));
  const image = document.createElement("img");
  image.alt = "Child transcript locations";
  // SVG image mode disables scripts and external document resources.
  image.src = `data:image/svg+xml;base64,${btoa(Array.from(encoded, (byte) => String.fromCharCode(byte)).join(""))}`;
  image.onload = () => record({ ...lastReport, svgLoaded: image.naturalWidth > 0 });
  image.onerror = () => fail("SVG image failed to load.");
  return image;
}

function record(report) {
  lastReport = report;
  window.pocReport = { runtime: native ? "tauri" : "browser", kind: currentKind, parentNativeControl: window.pocControl?.parentNativeControl ?? null, ...report };
  if (native) {
    void window.__TAURI_INTERNALS__.invoke("poc_report", { report: window.pocReport }).catch(() => fail("Could not save the native test report."));
  }
}

function show(kind) {
  currentKind = kind;
  lastReport = {};
  activeFrame = undefined;
  host.replaceChildren();
  source.hidden = true;
  document.getElementById("source-button").setAttribute("aria-expanded", "false");
  document.getElementById("error").hidden = true;
  document.getElementById("checks-section").hidden = kind !== "isolation";
  for (const name of ["diagram", "interactive", "isolation"]) {
    document.getElementById(`${name}-tab`).setAttribute("aria-pressed", String(name === kind));
  }
  if (kind === "diagram") {
    document.getElementById("widget-title").textContent = "Child transcript locations";
    document.getElementById("description").textContent = "Saved SVG displayed as an image.";
    document.getElementById("policy-label").textContent = "Existing app policy · scripts disabled in SVG image mode";
    source.textContent = data.widget_code;
    host.append(svgImage(data.widget_code));
    return;
  }
  if (!data.interactive_enabled) {
    fail("This run uses the existing app policy. Start the POC with --interactive to test HTML widgets.");
    return;
  }
  document.getElementById("widget-title").textContent = kind === "interactive" ? "Session sync counter" : "Frame isolation";
  document.getElementById("description").textContent = kind === "interactive" ? "HTML and JavaScript running inside a separate frame." : "Script execution and blocked access are checked together.";
  document.getElementById("policy-label").textContent = "POC frame policy · separate origin · no network access";
  source.textContent = "iframe sandbox=\"allow-scripts\"\nNo allow-same-origin, forms, popups, downloads, or top navigation.\nChild CSP: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  const frame = document.createElement("iframe");
  frame.title = kind === "interactive" ? "Interactive sync counter" : "Isolation probes";
  frame.sandbox = "allow-scripts";
  frame.allow = "camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'";
  frame.referrerPolicy = "no-referrer";
  frame.src = `${data.server_url}/widget/${kind}`;
  activeFrame = frame;
  host.append(frame);
}

// The child can report preview state. No message maps to an app command.
window.addEventListener("message", (event) => {
  if (!activeFrame || event.source !== activeFrame.contentWindow || event.origin !== "null") return;
  const message = event.data;
  if (!message || message.type !== "mallard-widget-poc" || typeof message.report !== "object" || !message.report) return;
  const report = message.report;
  if (currentKind === "interactive" && Number.isSafeInteger(report.count)) {
    record({ count: report.count, scriptRan: report.scriptRan === true });
  }
  if (currentKind === "isolation") {
    const keys = ["scriptRan", "parentDomBlocked", "storageBlocked", "nativeApiAbsent", "parentNativeApiBlocked", "fetchBlocked", "popupBlocked", "topNavigationBlocked", "svgScriptBlocked"];
    if (!keys.every((key) => typeof report[key] === "boolean")) return;
    const results = Object.fromEntries(keys.map((key) => [key, report[key]]));
    record(results);
    const labels = ["Widget script runs", "Parent document blocked", "Local storage blocked", "Child has no Tauri API", "Parent Tauri API blocked", "Network fetch blocked", "Popup blocked", "Top navigation blocked", "Script inside SVG image blocked"];
    const checks = document.getElementById("checks");
    checks.replaceChildren();
    keys.forEach((key, index) => {
      const label = document.createElement("dt");
      label.textContent = labels[index];
      const result = document.createElement("dd");
      result.textContent = results[key] ? "Passed" : "Failed";
      if (!results[key]) result.className = "failed";
      checks.append(label, result);
    });
    document.getElementById("check-status").textContent = `${Object.values(results).filter(Boolean).length} / ${keys.length} checks passed`;
  }
});

document.getElementById("source-button").addEventListener("click", () => {
  source.hidden = !source.hidden;
  document.getElementById("source-button").setAttribute("aria-expanded", String(!source.hidden));
});
for (const kind of ["diagram", "interactive", "isolation"]) {
  document.getElementById(`${kind}-tab`).addEventListener("click", () => {
    try { show(kind); } catch (error) { fail(error.message); }
  });
}

async function start() {
  data = native
    ? await window.__TAURI_INTERNALS__.invoke("poc_data")
    : await fetch("/data.json").then((response) => response.json());
  document.getElementById("runtime").textContent = native ? "Tauri · native WebView" : "Browser";
  const parentNativeControl = native ? await window.__TAURI_INTERNALS__.invoke("poc_probe") === "parent-command-ok" : null;
  window.pocControl = { parentNativeControl };
  show(new URLSearchParams(location.search).get("example") || window.POC_EXAMPLE || "diagram");
}
void start().catch((error) => fail(error.message));

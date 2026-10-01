// This trusted controller runs only in the opaque, network-blocked parser frame.
const token = location.hash.slice(1);
const stage = document.getElementById("stage");
const tauriPresent = typeof window.__TAURI_INTERNALS__ !== "undefined" || typeof window.__TAURI__ !== "undefined";
let sequence = 0;
let busy = false;
const palettes = {
  light: { darkMode: false, background: "#fffdf7", primaryColor: "#e8f1fa", primaryTextColor: "#1e405f", primaryBorderColor: "#507ca0", lineColor: "#74624a" },
  dark: { darkMode: true, background: "#2d261b", primaryColor: "#143b56", primaryTextColor: "#d4e9f8", primaryBorderColor: "#7197b3", lineColor: "#c8b998" },
};
const config = {
  startOnLoad: false,
  securityLevel: "strict",
  suppressErrorRendering: true,
  maxTextSize: 32768,
  maxEdges: 128,
  htmlLabels: false,
  fontFamily: "system-ui, sans-serif",
  theme: "base",
  flowchart: { htmlLabels: false, curve: "linear", useMaxWidth: true },
  secure: ["secure", "securityLevel", "startOnLoad", "maxTextSize", "maxEdges", "suppressErrorRendering", "htmlLabels", "flowchart", "fontFamily", "theme", "themeCSS", "themeVariables", "dompurifyConfig", "look", "layout"],
};
function send(type, fields = {}) { parent.postMessage({ type, token, ...fields }, "*"); }

async function render(id, code, theme) {
  busy = true;
  let svg;
  let error;
  try {
    if (tauriPresent || new TextEncoder().encode(code).length > config.maxTextSize) throw new Error("Source or frame boundary");
    const palette = palettes[theme];
    mermaid.initialize({ ...config, themeVariables: {
      ...palette, secondaryColor: palette.primaryColor, tertiaryColor: palette.primaryColor,
      fontSize: "16px", fontFamily: "system-ui, sans-serif",
    } });
    const diagram = await mermaid.mermaidAPI.getDiagramFromText(code);
    if (!/^flowchart/.test(diagram.type)) throw new Error("Diagram type");
    const nodes = diagram.db.getVertices().size;
    if (!Number.isInteger(nodes) || nodes === 0) {
      error = "This flowchart has no nodes. The source is available below.";
      throw new Error("Empty flowchart");
    }
    if (nodes > 128) {
      error = "This flowchart exceeds the 128-node limit. The source is available below.";
      throw new Error("Node limit");
    }
    ({ svg } = await mermaid.render(`mallard-flowchart-${++sequence}`, code, stage));
    if (new TextEncoder().encode(svg).length > 1048576) throw new Error("SVG limit");
  } catch {
    svg = undefined;
    error ??= "This flowchart is invalid or exceeds its limits. The source is available below.";
  } finally {
    stage.replaceChildren();
    busy = false;
  }
  const current = mermaid.mermaidAPI.getConfig();
  send("result", { id, result: {
    svg, error, tauriPresent,
    strict: current.securityLevel === "strict", htmlLabels: current.htmlLabels,
  } });
}
window.addEventListener("message", (event) => {
  if (busy || event.source !== parent || event.data?.type !== "render" || typeof event.data.id !== "string"
      || event.data.id.length > 64 || typeof event.data.code !== "string" || event.data.code.length > 32768
      || (event.data.theme !== "light" && event.data.theme !== "dark")) return;
  void render(event.data.id, event.data.code, event.data.theme);
});
send("ready", { tauriPresent });

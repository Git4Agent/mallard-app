import { mermaidSourceProblem, readMermaidResult, type MermaidResult } from "./mermaidModel";
import type { SessionTheme } from "../sessionTheme";

const MAX_JOBS = 16;
const REPLY_TIMEOUT_MS = 10_000;
const cache = new Map<string, Promise<MermaidResult>>();
let queue: Promise<unknown> = Promise.resolve();
let jobs = 0;
let connection: RendererConnection | null = null;
let pending: { id: string; resolve: (result: MermaidResult) => void; timer: number } | null = null;

interface RendererConnection {
  frame: HTMLIFrameElement;
  ready: Promise<void>;
  dispose: () => void;
}

function connectRenderer(): RendererConnection {
  if (connection) return connection;
  const frame = document.createElement("iframe");
  frame.title = "Flowchart parser";
  frame.className = "mermaid-parser-frame";
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("aria-hidden", "true");
  frame.tabIndex = -1;
  const token = crypto.randomUUID();
  let resolveReady: () => void;
  let rejectReady: (error: Error) => void;
  const ready = new Promise<void>((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  const timeout = window.setTimeout(() => rejectReady(new Error("Parser load timeout")), REPLY_TIMEOUT_MS);
  function receive(event: MessageEvent) {
    if (event.source !== frame.contentWindow || event.origin !== "null" || event.data?.token !== token) return;
    if (event.data.type === "ready") {
      window.clearTimeout(timeout);
      if (event.data.tauriPresent !== false) rejectReady(new Error("Privileged parser frame"));
      else resolveReady();
    } else if (event.data.type === "result" && pending && pending.id === event.data.id) {
      const request = pending;
      pending = null;
      window.clearTimeout(request.timer);
      request.resolve(readMermaidResult(event.data.result));
    }
  }
  const current: RendererConnection = {
    frame,
    ready,
    dispose() {
      window.clearTimeout(timeout);
      window.removeEventListener("message", receive);
      frame.remove();
      if (connection === current) connection = null;
    },
  };
  window.addEventListener("message", receive);
  frame.src = `/mermaid-renderer.html#${token}`;
  document.body.append(frame);
  connection = current;
  return current;
}

async function requestDiagram(code: string, theme: SessionTheme): Promise<MermaidResult> {
  const renderer = connectRenderer();
  try {
    await renderer.ready;
    return await new Promise<MermaidResult>((resolve) => {
      const id = crypto.randomUUID();
      const timer = window.setTimeout(() => {
        pending = null;
        renderer.dispose();
        resolve({ error: "The flowchart parser did not reply. The source is available below." });
      }, REPLY_TIMEOUT_MS);
      pending = { id, resolve, timer };
      // An opaque frame needs '*'. Replies must match the window, origin, token, and ID.
      renderer.frame.contentWindow!.postMessage({ type: "render", id, code, theme }, "*");
    });
  } catch {
    renderer.dispose();
    return { error: "The flowchart parser is unavailable. The source is available below." };
  }
}

export function renderMermaid(code: string, theme: SessionTheme): Promise<MermaidResult> {
  const error = mermaidSourceProblem(code);
  if (error) return Promise.resolve({ error });
  const key = `${theme}\n${code}`;
  const cached = cache.get(key);
  if (cached) return cached;
  if (jobs >= MAX_JOBS) return Promise.resolve({ error: "Too many flowcharts are waiting. The source is available below." });
  jobs++;
  const job = queue.then(() => requestDiagram(code, theme)).finally(() => { jobs--; });
  queue = job.catch(() => undefined);
  if (cache.size >= MAX_JOBS) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(key, job);
  return job;
}

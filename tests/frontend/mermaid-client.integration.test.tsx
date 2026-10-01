import assert from "node:assert/strict";
import test from "node:test";
import { renderMermaid } from "../../src/components/mermaid/mermaidClient";

// Test the real client protocol. No browser, native command, or real home is used.
test("Mermaid accepts only the parser window, opaque origin, token, request ID, and safe flags", async () => {
  const descriptors = ["window", "document"].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const);
  const listeners = new Set<(event: Record<string, unknown>) => void>();
  const timers = new Map<number, () => void>();
  const requests: Array<{ id: string; code: string; theme: string }> = [];
  let timerId = 0;
  let removed = 0;
  const frame = {
    src: "", title: "", className: "", tabIndex: 0,
    attributes: new Map<string, string>(),
    setAttribute(name: string, value: string) { this.attributes.set(name, value); },
    remove() { removed++; },
    contentWindow: { postMessage(request: { id: string; code: string; theme: string }, target: string) {
      assert.equal(target, "*");
      requests.push(request);
    } },
  };
  const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
  const dispatch = (data: Record<string, unknown>, origin = "null", source: unknown = frame.contentWindow) => {
    for (const listener of listeners) listener({ source, origin, data });
  };
  try {
    Object.defineProperty(globalThis, "window", { configurable: true, value: {
      setTimeout(callback: () => void) { timers.set(++timerId, callback); return timerId; },
      clearTimeout(id: number) { timers.delete(id); },
      addEventListener(_name: string, listener: (event: Record<string, unknown>) => void) { listeners.add(listener); },
      removeEventListener(_name: string, listener: (event: Record<string, unknown>) => void) { listeners.delete(listener); },
    } });
    Object.defineProperty(globalThis, "document", { configurable: true, value: {
      createElement(tag: string) { assert.equal(tag, "iframe"); return frame; }, body: { append() {} },
    } });
    assert.match((await renderMermaid("sequenceDiagram", "light")).error!, /Only Mermaid/);
    assert.equal(frame.src, "");
    const code = "flowchart TD\nA-->B";
    const light = renderMermaid(code, "light");
    assert.equal(renderMermaid(code, "light"), light, "same source and theme share one request");
    await flush();
    assert.equal(frame.attributes.get("sandbox"), "allow-scripts");
    assert.equal(frame.attributes.has("allow-same-origin"), false);
    const token = frame.src.split("#")[1];
    dispatch({ type: "ready", token, tauriPresent: false }, "null", {});
    dispatch({ type: "ready", token, tauriPresent: false }, "http://localhost");
    dispatch({ type: "ready", token: "wrong", tauriPresent: false });
    await flush();
    assert.equal(requests.length, 0);
    dispatch({ type: "ready", token, tauriPresent: false });
    await flush();
    assert.equal(requests.length, 1);
    const result = { svg: '<svg xmlns="http://www.w3.org/2000/svg"/>', strict: true, htmlLabels: false, tauriPresent: false };
    dispatch({ type: "result", token, id: "wrong", result });
    dispatch({ type: "result", token: "wrong", id: requests[0].id, result });
    dispatch({ type: "result", token, id: requests[0].id, result }, "https://example.com");
    await flush();
    assert.equal(timers.size, 1, "invalid replies do not complete the request");
    dispatch({ type: "result", token, id: requests[0].id, result });
    assert.deepEqual(await light, { svg: result.svg });
    const dark = renderMermaid(code, "dark");
    await flush();
    assert.equal(requests[1].theme, "dark", "theme changes require a new image");
    dispatch({ type: "result", token, id: requests[1].id, result: { ...result, tauriPresent: true } });
    assert.match((await dark).error!, /safely/);
    const timed = renderMermaid("graph LR\nA-->C", "light");
    await flush();
    const timeout = [...timers.values()][0];
    timeout();
    assert.match((await timed).error!, /did not reply/);
    assert.equal(removed, 1);
    assert.equal(listeners.size, 0);
    const reconnect = renderMermaid("graph LR\nA-->D", "light");
    await flush();
    dispatch({ type: "ready", token: frame.src.split("#")[1], tauriPresent: true });
    assert.match((await reconnect).error!, /unavailable/);
    assert.equal(removed, 2);
  } finally {
    for (const [name, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});

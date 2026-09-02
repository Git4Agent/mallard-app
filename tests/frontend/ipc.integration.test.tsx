import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { DESKTOP_BRIDGE_UNAVAILABLE, invoke } from "../../src/ipc";

/* The startup failure screen prints whatever the first backend call rejects
   with. Outside the desktop shell that used to be the engine's own
   `Cannot read properties of undefined (reading 'invoke')` — an implementation
   symbol, no cause, no next step. The bridge check has to turn that into a
   sentence a person can act on, and every command has to pass through it. */

test("a window without the desktop bridge rejects with a stated cause, not the engine message", async () => {
  // Node has no `window`; `isTauri()` reads `globalThis.isTauri`, so the
  // bridge is absent here exactly as it is in a browser tab.
  await assert.rejects(invoke("get_host_identity_status"), (reason: unknown) => {
    assert.ok(reason instanceof Error);
    assert.equal(reason.message, DESKTOP_BRIDGE_UNAVAILABLE);
    assert.doesNotMatch(reason.message, /invoke|undefined|properties/);
    return true;
  });
  // Cause and next step, in words the screen's Retry button does not contradict.
  assert.match(DESKTOP_BRIDGE_UNAVAILABLE, /desktop app/);
  assert.match(DESKTOP_BRIDGE_UNAVAILABLE, /Launch Mallard/);
});

test("every backend command goes through the guarded door", () => {
  for (const module of [
    "src/components/project-sync/api.ts",
    "src/components/storage-catalog/api.ts",
  ]) {
    const source = readFileSync(module, "utf8");
    assert.doesNotMatch(source, /@tauri-apps\/api\/core/, `${module} imports invoke from the Tauri package directly`);
    assert.match(source, /from "\.\.\/\.\.\/ipc"/, `${module} does not import the guarded invoke`);
  }
});

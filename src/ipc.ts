export { Channel } from "@tauri-apps/api/core";
import { invoke as invokeCommand, isTauri, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";

/**
 * The one door to the backend, a top-level peer of `storageError.ts`.
 *
 * Outside the desktop shell — a browser tab opened on the dev server —
 * `@tauri-apps/api` fails with `Cannot read properties of undefined (reading
 * 'invoke')`, and that engine message reached the startup failure screen as
 * user-facing copy. The bridge is checked here, once, so the failure states its
 * cause and the next step instead of naming an implementation symbol.
 */
export const DESKTOP_BRIDGE_UNAVAILABLE =
  "This window is not connected to Mallard’s desktop backend: it was opened outside the desktop app. Launch Mallard from the desktop app to continue.";

export function invoke<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  if (!isTauri()) return Promise.reject(new Error(DESKTOP_BRIDGE_UNAVAILABLE));
  return invokeCommand<T>(command, args, options);
}

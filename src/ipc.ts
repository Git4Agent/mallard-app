export { Channel, convertFileSrc } from "@tauri-apps/api/core";
import { invoke as invokeCommand, isTauri, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { t } from "./i18n";

/**
 * The one door to the backend, a top-level peer of `storageError.ts`.
 *
 * Outside the desktop shell — a browser tab opened on the dev server —
 * `@tauri-apps/api` fails with `Cannot read properties of undefined (reading
 * 'invoke')`, and that engine message reached the startup failure screen as
 * user-facing copy. The bridge is checked here, once, so the failure states its
 * cause and the next step instead of naming an implementation symbol.
 */
/* A declaration, not a constant: `src/i18n` calls `invoke` while it loads
   (the native menu's language), and this module imports `t` from it, so this
   must exist before either module has finished evaluating. */
export function desktopBridgeUnavailable(): string {
  return t("shell:desktopBridgeUnavailable");
}

export function invoke<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  if (!isTauri()) return Promise.reject(new Error(desktopBridgeUnavailable()));
  return invokeCommand<T>(command, args, options);
}

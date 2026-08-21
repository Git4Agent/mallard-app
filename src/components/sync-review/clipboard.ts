/**
 * The one clipboard path. Four copies of this used to exist and they were not
 * equivalent: the Library's was a bare `navigator.clipboard?.writeText(...)`
 * with no fallback and no error, so on a webview that exposes Clipboard but
 * rejects it, clicking a value copied nothing and said nothing.
 *
 * Lives in `sync-review/` because that is the resource-neutral layer both
 * feature modules already import; it holds no DTOs and calls no Tauri command.
 */
export async function writeClipboardText(
  value: string,
  fallbackRoot: HTMLElement | null = null,
): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Some WebViews expose Clipboard but still reject it. Use the local
      // document fallback while this user gesture is active.
    }
  }
  if (typeof document === "undefined") throw new Error("Clipboard access is unavailable");
  const field = document.createElement("textarea");
  field.value = value;
  field.readOnly = true;
  field.style.position = "fixed";
  field.style.opacity = "0";
  field.style.pointerEvents = "none";
  const previousFocus = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  (fallbackRoot ?? document.body).append(field);
  let copied = false;
  try {
    field.select();
    copied = document.execCommand("copy");
  } finally {
    field.remove();
    previousFocus?.focus();
  }
  if (!copied) throw new Error("Clipboard access is unavailable");
}

/**
 * Resource-neutral clipboard path for both feature modules. Some WebViews
 * expose Clipboard but reject writes, so this uses a document fallback and
 * reports failure. It holds no DTOs and calls no Tauri command.
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

/**
 * Copies text that is still being fetched. WebKit allows a clipboard write
 * only inside the click, so the write starts now with the pending text as
 * the item; elsewhere the text is awaited and written as usual. A rejected
 * fetch writes nothing and rejects with its own reason.
 */
export async function writeClipboardTextOf(
  value: Promise<string>,
  fallbackRoot: HTMLElement | null = null,
): Promise<void> {
  if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({ "text/plain": value.then((text) => new Blob([text], { type: "text/plain" })) }),
      ]);
      return;
    } catch {
      // The fetch failed (awaited below, it rejects again) or this WebView
      // refuses the item; try the plain path with the text in hand.
    }
  }
  await writeClipboardText(await value, fallbackRoot);
}

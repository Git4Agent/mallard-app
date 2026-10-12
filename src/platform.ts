/** The webview runs on macOS. Its window draws the traffic lights over the
 *  page (titleBarStyle Overlay in tauri.conf.json); every other platform keeps
 *  its window controls in a native title bar outside the page. */
export const isMacOS = typeof navigator !== "undefined" && /^Mac/.test(navigator.platform);

/** Names the platform on the root before React mounts, so the stylesheet can
 *  keep a strip clear for the traffic lights on macOS and nowhere else. */
export function applyPlatform(): void {
  if (isMacOS) document.documentElement.dataset.platform = "macos";
}

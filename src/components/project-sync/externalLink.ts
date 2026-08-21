import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Opening a commit on the forge, from the two places that offer it: the
 * activity list's commit heading and the session timeline's checkpoint marker.
 * Both state the same rule — a sha links only where a remote can resolve it.
 */

/** The commit's page on the project's GitHub remote, or null when none is configured. */
export function githubCommitUrl(repositoryUrl: string | null | undefined, sha: string): string | null {
  if (!repositoryUrl) return null;
  return `${repositoryUrl.replace(/\/+$/, "")}/commit/${sha}`;
}

/** The webview never navigates itself: the link opens in the user's browser. */
export async function openExternalUrl(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch {
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener,noreferrer");
  }
}

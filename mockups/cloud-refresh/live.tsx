// @ts-nocheck
/*
 * The real connect dialog on mock IPC, for driving the shipped Refresh:
 *
 *   /mockups/cloud-refresh/live.html?lang=zh|en&theme=light|dark&delay=1200
 *
 * Every listing command answers after `delay` ms; each answer adds one row
 * so a refresh is visible. window.__calls counts the commands.
 */
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import "../../src/App.css";
import { applyThemeChoice } from "../../src/preferences";
import { applyUiLanguage } from "../../src/i18n";
import BundleConnectionDialog from "../../src/components/project-sync/BundleConnectionDialog";

const params = new URLSearchParams(window.location.search);
const theme = params.get("theme") === "dark" ? "dark" : "light";
const lang = params.get("lang") === "en" ? "en" : "zh";
const delay = Number(params.get("delay") ?? 1200);
try {
  window.localStorage.setItem("mallard.theme", theme);
  window.localStorage.setItem("mallard.language", lang === "zh" ? "zh-CN" : "en");
} catch { /* fine */ }
applyThemeChoice(theme);
applyUiLanguage();

const storage = {
  storage_id: "storage-cloud", name: "Mallard Cloud", provider: "managed_cloud", config_version: 1, provider_label: "Mallard Cloud",
  icon: "cloud", available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "rikaqu0223-arch", is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: { service_origin: "https://mallard-ai.com", github_user_id: "123", account_login: "rikaqu0223-arch" },
  secrets: [{ secret_id: "account_token", present: true }],
};
const CHECKOUT = "a".repeat(64);
const now = Math.floor(Date.now() / 1000);
const H = 3600;
const repo = (owner, name, ago, extra = {}) => ({
  repository_id: `${owner}/${name}`, owner_login: owner, name, address: `${owner}/${name}`, display_name: name,
  website_url: `https://mallard-ai.com/${owner}/${name}`, bundle_id: name.replace(/[^a-z0-9]/gi, "").padEnd(32, "0").slice(0, 32),
  repository_class: "git_project", visibility: "public", role: "owner", can_push: true, repository_fingerprint: CHECKOUT,
  git_remote: "matches", github_url: null, revision: 1, updated_at: now - ago, resources: 2, registration_only: false,
  description: null, star_count: 0, viewer_starred: false, ...extra,
});
const listed = (owner, name, ago, extra = {}) => ({
  repository_id: `${owner}/${name}`, owner_login: owner, name, address: `${owner}/${name}`, display_name: name,
  website_url: `https://mallard-ai.com/${owner}/${name}`, repository_class: "project", catalog: null, visibility: "public",
  role: null, github_url: null, description: null, star_count: 0, viewer_starred: false, revision: 1, updated_at: now - ago, ...extra,
});

const calls = { yours: 0, starred: 0, public: 0 };
window.__calls = calls;
const wait = (value) => new Promise((resolve) => setTimeout(() => resolve(value), delay));

window.isTauri = true;
mockIPC((cmd, args) => {
  switch (cmd) {
    case "list_managed_cloud_repositories": {
      calls.yours += 1;
      const rows = [repo("rikaqu0223-arch", "mallard-with-pelican", 19 * H), repo("rikaqu0223-arch", "测试文件", 96 * H), repo("rikaqu0223-arch", "mallardNonGit", 96 * H)];
      for (let i = 1; i < calls.yours; i += 1) rows.unshift(repo("rikaqu0223-arch", `arrived-${i}`, 30 * i));
      return wait(rows);
    }
    case "list_managed_cloud_starred":
      calls.starred += 1;
      return wait({ rows: [listed("Rika321", "win_mallard", 3 * H, { viewer_starred: true, star_count: 1 })], next_cursor: null });
    case "list_managed_cloud_public": {
      calls.public += 1;
      const rows = [listed("FuyaoLi2017", "fuyao-non-git-test", 3 * H), listed("Rika321", "win_mallard", 3 * H), listed("Rika321", "win_pelican", 3 * H)];
      for (let i = 1; i < calls.public; i += 1) rows.unshift(listed("Rika321", `public-arrived-${i}`, 30 * i));
      return wait({ rows, next_cursor: null });
    }
    default:
      console.warn("unhandled", cmd, args);
      return Promise.reject(new Error(`unhandled ${cmd}`));
  }
});

createRoot(document.getElementById("root")).render(
  <BundleConnectionDialog storage={storage} projectName="mallard-app" currentBundleId="" matches={[]} projectFingerprint={CHECKOUT} reason="link"
    busy={false} error={null} onCancel={() => undefined} onUseExisting={() => undefined} onKeepCurrent={() => undefined} />,
);

// @ts-nocheck
/*
 * Review footer specimen: the real Mallard app on mock IPC (copied from
 * mockups/readme-shots), for a non-Git folder "crow" with project files.
 *
 *   /mockups/review-footer/live.html?view=push|pull&theme=light|dark&lang=en|zh
 *                                    &rows=1..3&files=0..3
 *
 * <html data-shot-ready="1"> is set once the review has settled.
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import * as demo from "./fixtures";

const params = new URLSearchParams(window.location.search);
const view = params.get("view") ?? "project";
const theme = params.get("theme") === "dark" ? "dark" : "light";
const language = params.get("lang") === "zh" ? "zh-CN" : "en";
const step = params.get("step") === "setup" ? "setup" : "folder";
const fresh = view === "newproject";

// The app reads both from this origin's localStorage before it mounts. Start
// from a clean slate so a remembered sidebar width or root never carries over.
try {
  for (const key of Object.keys(window.localStorage)) {
    if (key.startsWith("mallard.")) window.localStorage.removeItem(key);
  }
  window.localStorage.setItem("mallard.theme", theme);
  window.localStorage.setItem("mallard.language", language);
  window.localStorage.setItem("mallard.clock-24h", "on");
} catch {
  /* Rendering still works; the app falls back to System. */
}

const unhandled = new Set<string>();

function emitActivity(args) {
  const channel = args?.onProgress;
  const send = (event) => channel?.onmessage?.(event);
  send({ stage: "activity", activity: demo.activity(), storage_pending: true });
  send({ stage: "comparison", comparison: demo.comparison() });
  send({ stage: "activity", activity: demo.activity(), storage_pending: false });
  return null;
}

const handlers: Record<string, (args: any) => unknown> = {
  // Shell start-up.
  get_host_identity_status: () => demo.hostIdentity,
  get_project_sync_config: () => demo.config,
  recover_startup_state: () => null,
  list_local_projects: () => [demo.registration],
  list_setup_drafts: () => ({ drafts: [], warnings: [] }),
  list_storage_setup_drafts: () => ({ drafts: [], warnings: [] }),
  list_project_repository_kinds: () => ({ [demo.PROJECT_ID]: false }),
  list_agent_providers: () => demo.agentProviders,
  list_storage_providers: () => demo.storageProviders,
  list_provider_profiles: () => demo.profiles,
  list_project_bindings: () => [demo.binding],
  get_app_settings: () => demo.appSettings,
  set_ui_language: () => null,
  take_app_link: () => null,
  prepare_app_update: () => ({ current_version: "0.1.8", ready_version: null }),
  "plugin:app|version": () => "0.1.8",
  "plugin:app|name": () => "Mallard",
  "plugin:app|tauri_version": () => "2.8.5",
  "plugin:path|resolve_directory": () => "/Users/demo",

  // Library (empty).
  list_catalog_scopes: () => [],
  list_catalog_capabilities: () => [],
  list_catalog_resource_profiles: () => [],
  list_catalog_storage_repositories: () => [],

  // Logs.
  get_activity_log_stats: () => ({
    total_bytes: 48_213,
    file_count: 3,
    oldest_ts: demo.NOW - 9 * 86_400,
    newest_ts: demo.NOW - 3_600,
    policy: { schema: 1, retention_days: 30, max_total_bytes: 50 * 1024 * 1024 },
  }),
  query_activity_logs: () => ({ entries: [], next_cursor: null }),

  // Project page.
  get_project: () => demo.projectDetail,
  get_bundle_inventory: () => demo.inventory(),
  get_sibling_bundle_status: () => ({ state: "none" }),
  refresh_project_activity: emitActivity,
  get_project_activity: () => demo.activity(),
  watch_project_git: () => null,
  unwatch_project_git: () => null,
  list_remote_bundle_snapshots: () => [],
  inspect_provider_cli: (args) => ({
    provider: args?.provider ?? "codex",
    selected_path: null,
    launcher: `/opt/homebrew/bin/${args?.provider ?? "codex"}`,
    executable: `/opt/homebrew/bin/${args?.provider ?? "codex"}`,
    source: "shell_path",
    error: null,
  }),

  // Push review.
  prepare_push_review: (args) => demo.pushReviewContext(args?.scope ?? { kind: "project" }),
  get_push_destination_status: () => ({ state: "ready", revision: 14 }),
  inspect_project_files: () => demo.projectFiles(),

  // Pull review.
  plan_bundle_restore: (args) => demo.pullReview(args?.scope ?? { kind: "project" }),
  get_restore_readiness: () => demo.readiness,
};

/* Add a project: no project yet, one setup draft once a folder is chosen. */
let draft = null;
if (fresh) {
  Object.assign(handlers, {
    get_project_sync_config: () => demo.freshConfig,
    list_local_projects: () => [],
    list_project_repository_kinds: () => ({}),
    list_project_bindings: () => [],
    list_provider_profiles: () => demo.freshProfiles,
    list_setup_drafts: () => ({ drafts: draft ? [demo.draftSummary(draft)] : [], warnings: [] }),
    list_agent_folders: () => demo.agentFolders,
    probe_project_folder: (args) => demo.probeFolder(args?.projectRoot, draft?.draft_id ?? null),
    create_setup_draft: () => {
      draft = draft ?? demo.setupDraft(0);
      return { draft, resumed: false };
    },
    get_setup_draft: () => draft,
    update_setup_draft: (args) => {
      draft = { ...args.draft, revision: (draft?.revision ?? 0) + 1, updated_at: demo.NOW };
      return draft;
    },
    inspect_setup_draft: () => demo.draftInspection(draft),
    list_managed_cloud_repositories: () => [],
    list_managed_cloud_starred: () => ({ rows: [], next_cursor: null }),
    list_managed_cloud_public: () => ({ rows: [], next_cursor: null }),
  });
}

window.isTauri = true;
mockWindows("main");
mockIPC((command, args) => {
  const handler = handlers[command];
  if (handler) return handler(args);
  if (command.startsWith("plugin:")) return null;
  if (!unhandled.has(command)) {
    unhandled.add(command);
    console.warn(`[review-footer] unhandled command: ${command}`, args);
  }
  return null;
}, { shouldMockEvents: true });
window.__reviewFooterUnhandled = unhandled;

/* Mount exactly as src/main.tsx does, after the mocks exist: i18n reports the
   menu language while it loads, and the shell subscribes to events at once. */
const React = await import("react");
const ReactDOM = await import("react-dom/client");
const { applyUiLanguage } = await import("../../src/i18n");
const { applyThemeChoice, getThemeChoice } = await import("../../src/preferences");
const { default: App } = await import("../../src/App");

applyThemeChoice(getThemeChoice());
applyUiLanguage();
ReactDOM.createRoot(document.getElementById("root")).render(
  React.createElement(React.StrictMode, null, React.createElement(App)),
);

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
// A hidden document runs no animation frames; never wait on one alone.
const frame = () => Promise.race([
  new Promise((resolve) => requestAnimationFrame(() => resolve(null))),
  sleep(60),
]);

async function until(label: string, test: () => unknown, timeoutMs = 15_000) {
  const started = performance.now();
  for (;;) {
    const value = test();
    if (value) return value;
    if (performance.now() - started > timeoutMs) throw new Error(`timed out waiting for ${label}`);
    await sleep(50);
  }
}

const syncButtons = () => [...document.querySelectorAll(".sync-path-action-group .sync-path-sync")];
const pullButton = () => syncButtons()[0] as HTMLButtonElement | undefined;
const pushButton = () => syncButtons()[1] as HTMLButtonElement | undefined;
const busy = () => document.querySelector("[aria-busy='true']");

async function settle() {
  await until("no busy region", () => !busy());
  await document.fonts.ready;
  // Entrance animations (workspace enter, review reveal) run once; wait them out.
  await Promise.race([
    Promise.all(document.getAnimations()
      .filter((animation) => animation.effect?.getTiming().iterations !== Infinity)
      .map((animation) => animation.finished.catch(() => null))),
    sleep(1_500),
  ]);
  document.getAnimations().forEach((animation) => {
    if (animation.effect?.getTiming().iterations !== Infinity) animation.finish();
  });
  await sleep(500);
  await frame();
  await frame();
}

async function projectReady() {
  await until("session cards", () => document.querySelectorAll(".sync-history-thread-card").length >= 1);
  const verb = view === "pull" ? pullButton : pushButton;
  await until(`${view} button enabled`, () => verb() && !verb().disabled);
  await settle();
}

const reviewChoices = () => [...document.querySelectorAll(
  "[role='tabpanel'] .sync-history-thread-card .sync-sync-row-choice input[type='checkbox']",
)] as HTMLInputElement[];

const clickable = (element) => element && !element.disabled ? element : null;

async function storageView() {
  await projectReady();
  // The sidebar's Storages tab, then its + (Add storage).
  const tabs = [...document.querySelectorAll(".sidebar-root-tabs .sidebar-root-tab")];
  (await until("storages tab", () => clickable(tabs[2]))).click();
  await until("storages root", () => tabs[2].getAttribute("aria-current") === "page");
  (await until("add storage", () => clickable(document.querySelector(".sidebar-add-action")))).click();
  await until("provider cards", () => document.querySelectorAll(".landing-picker-grid .landing-picker-card").length >= 4);
  await settle();
}

async function newProjectView() {
  await until("shell", () => document.querySelector(".sidebar-add-action") && !document.querySelector(".sync-startup-screen"));
  (await until("add project", () => clickable(document.querySelector(".sidebar-add-action")))).click();
  const rows = () => [...document.querySelectorAll(".sync-folder-choices .sync-bundle-match-option")];
  await until("agent folders", () => rows().length >= 3);
  const tidewatch = rows().find((row) => row.querySelector("strong")?.textContent?.startsWith("tidewatch") && !row.textContent.includes("tidewatch-"));
  tidewatch.click();
  const use = () => clickable(document.querySelector(".sync-setup-footer .btn-primary"));
  await until("folder chosen", () => tidewatch.getAttribute("aria-checked") === "true" && use());
  if (step === "folder") {
    // The field keeps focus from autoFocus; a screenshot reads cleaner without the caret ring.
    (document.activeElement as HTMLElement | null)?.blur?.();
    await settle();
    return;
  }
  use().click();
  await until("setup page", () => document.querySelector("#sync-setup-title"));
  await until("agent profiles", () => document.querySelectorAll(".sync-provider-card.enabled").length >= 2);
  await until("storage chosen", () => document.querySelector(".setup-pill.active[aria-checked='true']:not(:first-child)"));
  await until("repository section", () => document.querySelector(".repo-connect") && !document.querySelector(".sync-setup-inline-status"));
  await sleep(1_200); // the draft's autosave window, so "Saving…" is not mid-flight
  (document.activeElement as HTMLElement | null)?.blur?.();
  await settle();
}

async function drive() {
  if (view === "storage") return storageView();
  if (view === "newproject") return newProjectView();
  await projectReady();
  if (view === "push") {
    pushButton().click();
    await until("push review", () => document.querySelector("[role='tabpanel'][id^='sync-review-']"));
    await until("ticked push rows", () => reviewChoices().filter((input) => input.checked).length >= 1);
    await settle();
  } else if (view === "pull") {
    pullButton().click();
    await until("pull review", () => document.querySelector("[role='tabpanel'][id^='sync-review-']"));
    await until("pull rows", () => reviewChoices().length >= 1);
    await settle();
    for (const input of reviewChoices()) {
      if (!input.checked && !input.disabled) {
        input.click();
        await sleep(80);
      }
    }
    await until("ticked pull rows", () => reviewChoices().every((input) => input.checked));
    await settle();
  }
}

/* Specimen hooks, applied once the real review has settled:
     &step=files      open the Files tab
     &fix=1           the proposed bottom-gutter fix (directions.ts FIX_CSS)
     &dir=<key>       a visibility direction (directions.ts)
     &id=<frame id>   post the review's box to the parent page for cropping */
async function specimen() {
  const { FIX_CSS, applyDirection } = await import("./directions");
  if (params.get("step") === "files") {
    const tab = [...document.querySelectorAll("[role='tab']")].find((candidate) => candidate.textContent?.match(/项目文件|Project files/));
    (tab as HTMLElement | undefined)?.click();
    await until("files step", () => document.querySelector(".sync-project-files-page"));
    await settle();
  }
  if (params.get("fix") === "1") {
    const style = document.createElement("style");
    style.textContent = FIX_CSS;
    document.head.append(style);
  }
  const dir = params.get("dir");
  if (dir) applyDirection(dir, view === "pull" ? "pull" : "push", language);
  (document.activeElement as HTMLElement | null)?.blur?.();
  await frame();
  await frame();
}

function measure() {
  const review = document.querySelector(".sync-inline-action-review") as HTMLElement | null;
  const scroll = review?.querySelector(".sync-sync-review-scroll") as HTMLElement | null;
  const footer = review?.querySelector(".sync-sync-review-footer") as HTMLElement | null;
  if (!review || !scroll || !footer) return null;
  const shadowed = [...scroll.querySelectorAll("*")].filter((element) => {
    const shadow = getComputedStyle(element).boxShadow;
    return shadow !== "none" && element.getBoundingClientRect().height > 24;
  });
  const lowest = shadowed.reduce((low, element) => (
    !low || element.getBoundingClientRect().bottom > low.getBoundingClientRect().bottom ? element : low
  ), null as Element | null);
  const offsetY = lowest ? Number(getComputedStyle(lowest).boxShadow.match(/(-?[\d.]+)px\s+(-?[\d.]+)px/)?.[2] ?? 0) : 0;
  const box = review.getBoundingClientRect();
  const lastBottom = lowest ? lowest.getBoundingClientRect().bottom : null;
  const footerTop = footer.getBoundingClientRect().top;
  return {
    reviewWidth: Math.round(box.width),
    rect: { x: box.left, y: box.top, width: box.width, height: box.height },
    tileBottom: lastBottom,
    shadowEnd: lastBottom === null ? null : lastBottom + offsetY,
    footerTop,
    clear: lastBottom === null ? null : Math.round((footerTop - (lastBottom + offsetY)) * 10) / 10,
  };
}
window.__reviewFooterMeasure = measure;

drive()
  .then(specimen)
  .then(() => {
    document.documentElement.dataset.shotReady = "1";
    const id = params.get("id");
    if (id && window.parent !== window) window.parent.postMessage({ kind: "review-footer-frame", id, measure: measure() }, "*");
  })
  .catch((error) => {
    document.documentElement.dataset.shotReady = "error";
    console.error("[review-footer]", error);
    const id = params.get("id");
    if (id && window.parent !== window) window.parent.postMessage({ kind: "review-footer-frame", id, error: String(error) }, "*");
  });

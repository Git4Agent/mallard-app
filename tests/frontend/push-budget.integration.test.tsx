import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
  pullRequiredCopy,
  pushBlockerPillLabel,
  pushBlockerSentence,
  pushBudgetTotal,
  pushProjectFileWork,
  pushReviewDisabledReason,
  recommendedPushSelection,
  selectPushProjectFilesWithinBudget,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import { i18n } from "../../src/i18n";

test("a reviewed Git-only change can be pushed without a session selection", () => {
  assert.equal(pushReviewDisabledReason({ selected: new Set(), deletionApproved: true, gitHistoryChanged: true }), null);
  assert.equal(pushReviewDisabledReason({ selected: new Set(), deletionApproved: true, gitHistoryChanged: false }), "Nothing is selected");
  assert.equal(pushReviewDisabledReason({ selected: new Set(), deletionApproved: true, gitHistoryChanged: true, listState: "loading" }), "Loading sessions…");
});
import { recipeWithSelection } from "../../src/components/project-sync/model";
import { sessionPushDisabledReason, sessionPushGate } from "../../src/components/project-sync/session-detail/sessionDetailModel";
import {
  ProjectChatHistoryContent,
  type PushSessionFacts,
} from "../../src/components/project-sync/ProjectActivityPage";
import SessionPushReview from "../../src/components/project-sync/SessionPushReview";
import type {
  InventoryResource,
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectContentInventory,
  ProjectSessionSummary,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../src/types";

/* Automatic selection is the newest sessions that fit, as a strict time
   prefix. Every eligible session remains available for manual selection, and
   only the meter — shared with the Pull review — refuses a selection. */

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
/* 2026-09-01T00:00:00Z. */
const NOW = 1_788_220_800;

const PROJECT_ROOT = "/Users/test/projects/mallard";

const project: LocalProjectSummary = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: "mallard",
  local_alias: "Mallard local",
  repository_fingerprint: "fingerprint",
  project_root: PROJECT_ROOT,
  canonical_project_root: PROJECT_ROOT,
  profile_ids: { codex: "profile-codex" },
  profile_names: ["Default Codex"],
  revision: 1,
};

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: project.local_project_id,
  bundle_id: project.bundle_id,
  project_root: PROJECT_ROOT,
  canonical_project_root: PROJECT_ROOT,
  profile_ids: { codex: "profile-codex" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

const FITS_ID = "019f7798-5437-7632-9dbc-5b589cf68b10";
const WAITS_ID = "019f7798-5437-7632-9dbc-5b589cf68b11";

function entry(threadId: string, overrides: Partial<ThreadSyncEntry> = {}): ThreadSyncEntry {
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: threadId,
    conversation_resource_id: `codex:conversation:${threadId}`,
    resource_id: `codex:session:${threadId}`,
    display_name: threadId,
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    local_updated_at: NOW - 3_600,
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: 96 * MB,
    ...overrides,
  };
}

const fits = entry(FITS_ID, { display_name: "Refine sync review UX", capture_bytes: 96 * MB });
const waits = entry(WAITS_ID, {
  display_name: "Catalog profile launch buttons",
  capture_bytes: 118 * MB,
  local_updated_at: NOW - 5 * 86_400,
});

test("session Push follows the publish decision, including changed export settings", () => {
  assert.equal(sessionPushDisabledReason(fits), null);
  assert.equal(sessionPushDisabledReason(entry("mode-change", {
    state: "synced",
    push_decision: { action: "publish", selectable: true },
  })), null);
  assert.equal(sessionPushDisabledReason(entry("synced", {
    state: "synced",
    push_decision: { action: "no_changes", selectable: false },
  })), "This session is up to date.");
  for (const state of ["storage_ahead", "diverged"] as const) {
    assert.match(sessionPushDisabledReason(entry(state, {
      state,
      push_decision: { action: "require_pull", selectable: false },
    }))!, /Review Pull/);
  }
  assert.match(sessionPushDisabledReason(entry("metadata", {
    state: "synced",
    push_decision: { action: "require_pull_metadata", selectable: false },
  }))!, /metadata in Pull/);
  assert.equal(sessionPushDisabledReason(entry("remote", { local_present: false })), "No local session to push.");
  assert.equal(sessionPushDisabledReason(undefined), "This session cannot be pushed independently.");
  for (const state of ["unknown", "unavailable"] as const) {
    assert.equal(sessionPushDisabledReason(entry(state, {
      state,
      push_decision: { action: "blocked", selectable: false, blocked_reason: "Cannot read this session." },
    })), "Cannot read this session.");
  }
});

// The session page repeats only a hold the user clears. Up to date, ready and
// a hold nobody can clear from here stay Push's own tooltip.
test("a session page names a Push hold only when Pull clears it or the comparison failed", () => {
  const upToDate = entry("synced", { state: "synced", push_decision: { action: "no_changes", selectable: false } });
  assert.equal(sessionPushGate(upToDate, null), null);
  assert.equal(sessionPushGate(fits, null), null);
  assert.equal(sessionPushGate(undefined, null), null);
  assert.equal(sessionPushGate(entry("remote", { local_present: false }), null), null);
  assert.equal(sessionPushGate(entry("unreadable", {
    state: "unknown",
    push_decision: { action: "blocked", selectable: false, blocker: "observation_failed", blocked_reason: "Cannot read this session." },
  }), null), null);

  assert.deepEqual(sessionPushGate(entry("ahead", {
    state: "storage_ahead",
    push_decision: { action: "require_pull", selectable: false },
  }), null), { tone: "info", label: "Pull first", detail: "Storage has changes. Review Pull before pushing this session." });
  assert.equal(sessionPushGate(entry("metadata", {
    state: "synced",
    push_decision: { action: "require_pull_metadata", selectable: false },
  }), null)?.label, "Pull first");
  assert.deepEqual(sessionPushGate(entry("detached", {
    state: "diverged",
    push_decision: { action: "blocked", selectable: false, blocker: "detached_requires_pull", blocked_reason: "Pull this session's storage copy first." },
  }), null), { tone: "info", label: "Pull first", detail: "Pull this session's storage copy first." });

  // A failed comparison outranks the entry it could not produce.
  assert.deepEqual(sessionPushGate(upToDate, "Mallard couldn't read the storage."), {
    tone: "danger",
    label: "Can't compare",
    detail: "Mallard couldn't read the storage.",
  });
});

/* What the session's page reports for its row: 2 prompts, 3 replies, 4 tool
   calls, a total with no input/output split, an hour long. */
const fitsFacts: PushSessionFacts = {
  title: "Refine sync review UX",
  metrics: {
    started_at: NOW - 7_200,
    ended_at: NOW - 3_600,
    user_round_count: 2,
    agent_message_count: 3,
    tool_call_count: 4,
    total_tokens: 1_200,
    metrics_complete: true,
  },
};

function sessionReview(
  target: ThreadSyncEntry,
  overrides: Partial<ComponentProps<typeof SessionPushReview>> = {},
): string {
  return renderToStaticMarkup(
    <SessionPushReview
      credentialRedactionEnabled={true}
      entry={target}
      facts={fitsFacts}
      storageName="Attic R2"
      selected={new Set([target.resource_id])}
      threadComparison={comparison({ capture_budget: { limit_bytes: LIMIT, reserved_bytes: 0 } })}
      busy={false}
      onClose={() => undefined}
      onPush={() => undefined}
      {...overrides}
    />,
  );
}

test("a session's own review draws that session's row and nothing to select", () => {
  const reviewed = comparison({ capture_budget: { limit_bytes: LIMIT, reserved_bytes: 0 } });
  // The scoped inventory holds the target alone, so nothing else can travel.
  const selected = sanitizePushSelection(
    new Set([fits.resource_id, waits.resource_id]),
    [conversationResource(fits)],
    reviewed,
  );
  assert.deepEqual([...selected], [fits.resource_id]);
  assert.equal(pushBudgetTotal(selected, reviewed)?.total, fits.capture_bytes);

  const html = sessionReview(fits);
  assert.match(html, /<h2 id="sync-push-resource-title">Push review<\/h2>/);
  assert.match(html, /Credential redaction: On/);
  assert.match(html, /Refine sync review UX/);
  assert.doesNotMatch(html, /Catalog profile launch buttons|Scan project files|Git history/);
  // Every control that acts on a list is gone: search, count, recommended
  // selection, clear, the group header, the checkbox, and the header glance.
  assert.doesNotMatch(html, /type="checkbox"|type="search"|selected"|Unselect|Clear|Conversations|sync-review-budget-glance/);
  // The row prints what the page reported, then the size the push weighs.
  assert.match(html, /data-tooltip="Prompts · 2\nReplies · 3"[^>]*>[\s\S]*?<b>5<\/b>/);
  assert.match(html, /data-tooltip="Tool calls · 4"/);
  assert.match(html, /data-tooltip="Total tokens · 1,200"/, "a total without its split prints alone");
  assert.doesNotMatch(html, /Not reported/);
  assert.match(html, /data-tooltip="Ran for 1h"/);
  assert.match(html, /data-tooltip="Push size · 96 MB"/);
  // Close and the commit are the two controls left; the size prints once more,
  // at the button.
  assert.match(html, /aria-label="Close push review"/);
  const footer = html.slice(html.indexOf("<footer"));
  assert.match(footer, /sync-review-budget-gauge push/);
  assert.match(footer, /<b>96 MB<\/b> \/ 512 MB/);
  assert.match(footer, /class="btn btn-primary sync-pull-apply-button"(?![^>]*disabled)[^>]*>[\s\S]*?Push<\/button>/);
});

test("a session's own review shows the redaction mode without the footer warning", () => {
  for (const enabled of [true, false]) {
    const html = sessionReview(fits, { credentialRedactionEnabled: enabled });
    assert.match(html, new RegExp(`Credential redaction: ${enabled ? "On" : "Off"}`));
    assert.doesNotMatch(html, /Not redacted:|redaction-off/);
    assert.doesNotMatch(html, /class="notice |sync-review-banners/);
  }
});

test("a session's own review names its row from the comparison when the page had not loaded", () => {
  const html = sessionReview(fits, { facts: null });
  assert.match(html, /<strong>Refine sync review UX<\/strong>/);
  assert.match(html, /data-tooltip="Push size · 96 MB"/);
  assert.doesNotMatch(html, /Prompts ·|Total tokens/);
  assert.match(sessionReview(entry("019f7798-5437-7632-9dbc-5b589cf68b12"), { facts: null }), /<strong>Untitled session<\/strong>/);
});

test("a session's own review says which subagents travel", () => {
  const withAgents = entry(FITS_ID, {
    display_name: "Refine sync review UX",
    capture_bytes: 96 * MB,
    subagent_selection: { wire_children: "on", keys: ["a", "b", "c"], omitted: false, estimated_bytes: 12 * MB },
  });
  assert.match(sessionReview(withAgents), /data-tooltip="Subagents · 3\nPush size · 12 MB"[^>]*>[\s\S]*?<b>3<\/b>/);
  assert.doesNotMatch(sessionReview(fits), /Subagents ·/);
  const omitted = entry(FITS_ID, {
    display_name: "Refine sync review UX",
    subagent_selection: { wire_children: "off", keys: [], omitted: true, estimated_bytes: 0 },
  });
  assert.match(sessionReview(omitted), /aria-label="Child rollouts excluded"/);
  assert.doesNotMatch(sessionReview(omitted), /Subagents ·/);
});

test("a session past the limit is refused in its own words: it has nothing to exclude", () => {
  const huge = entry(FITS_ID, { display_name: "Refine sync review UX", capture_bytes: 540 * MB });
  const reviewed = comparison({ entries: [huge], capture_budget: { limit_bytes: LIMIT, reserved_bytes: 0 } });
  const selected = new Set([huge.resource_id]);
  const refusal = "28 MB over the 512 MB push limit.";
  // The path bar's Push reads the same rule, so both buttons word it alike.
  assert.equal(
    pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true, sessionScope: true }),
    refusal,
  );
  assert.equal(
    pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true }),
    "28 MB over the 512 MB push limit — exclude sessions or files to push.",
  );
  const html = sessionReview(huge, { threadComparison: reviewed });
  const footer = html.slice(html.indexOf("<footer"));
  assert.match(footer, new RegExp(`<p class="sync-review-footer-note danger">${refusal}</p>`));
  assert.match(footer, /sync-review-budget-gauge push over/);
  assert.match(footer, new RegExp(`sync-pull-apply-button" disabled="" title="${refusal}"`));
});

test("a running session push seals its exits and narrates the transfer", () => {
  const html = sessionReview(fits, {
    busy: true,
    credentialRedactionEnabled: false,
    fetchProgress: {
      phase: "push",
      storage_id: "storage-1",
      bundle_id: project.bundle_id,
      index: 3,
      total_objects: 7,
      fetched_bytes: 800_000,
      total_bytes: 96 * MB,
    },
  });
  assert.match(html, /aria-label="Close push review"[^>]*disabled=""|disabled=""[^>]*aria-label="Close push review"/);
  assert.doesNotMatch(html, /A running push finishes or fails whole|Not redacted:/);
  assert.match(html, /<span class="sync-btn-readout">3\/7 · 0\.8 MB…<\/span>/);
});

test("a session's own review marks the commits its Push adds to the Cloud's history", () => {
  const history = {
    branch: "main",
    head_sha: null,
    commits: [],
    uncommitted_session_refs: [],
    outside_history_session_refs: [],
    truncated: false,
  };
  const marked = sessionReview(fits, { gitHistory: history, gitHistoryOutgoing: { new_commits: 3, rewritten: false } });
  // The rail's commit chip, holding its count, right after the title; its
  // words are the chip's sentence with the count and the branch.
  assert.match(
    marked,
    /Refine sync review UX<\/strong><span class="sync-history-branch-state sync-history-outgoing" title="3 commits new since the last Push to Attic R2 · main" aria-label="3 commits new since the last Push to Attic R2 · main" tabindex="0">↑3<\/span>/,
  );
  const rewritten = sessionReview(fits, { gitHistory: history, gitHistoryOutgoing: { new_commits: 1, rewritten: true } });
  assert.match(
    rewritten,
    /sync-history-outgoing history-only" title="1 commit new since the last Push to Attic R2 · main\nHistory was rewritten: Attic R2&#x27;s latest commit is no longer on this branch\."[^>]*>↑1<\/span>/,
  );
  // Nothing added, or no stored history this branch shares: no mark, as on
  // the project review's rail. Other storages carry no history at all.
  assert.doesNotMatch(sessionReview(fits, { gitHistory: history, gitHistoryOutgoing: { new_commits: 0, rewritten: false } }), /sync-history-outgoing/);
  assert.doesNotMatch(sessionReview(fits, { gitHistory: history, gitHistoryOutgoing: null }), /sync-history-outgoing/);
  assert.doesNotMatch(sessionReview(fits), /sync-history-outgoing/);
  // The shell hands the review what the context returned for the session scope.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /<SessionPushReview[\s\S]*?gitHistory=\{pendingPush\.gitHistory\}\s+gitHistoryOutgoing=\{pendingPush\.gitHistoryOutgoing\}/);
  // The pill takes the commit chip's size on the title line too.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-history-commit-meta > \.sync-history-outgoing,\n\.sync-history-thread-title > \.sync-history-outgoing \{/);
});

test("the shell opens a session's own review on that session alone", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  // No budget-fitted default: the one session is the selection, so one past
  // the limit is refused in the review instead of opening unticked.
  assert.match(shell, /setPushSelected\(scope\.kind === "session" \? new Set\(\[scope\.resource_id\]\) : new Set\(projectDefaults\)\)/);
  // The row's review is mounted in the project review's place, on the
  // selection the push carries, and both buttons read the session's rule.
  assert.match(shell, /content: pushSessionEntry && pendingPush\.threadComparison \? \(\s*<SessionPushReview[\s\S]*?selected=\{safePushSelection\}/);
  assert.match(shell, /sessionScope: pushSessionScope !== null,/);
  // The session's page reports the row's readouts with the Push it starts.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /sessionHeader\?\.state === "ready" \? sessionHeader\.pushFacts : undefined/);
});

function comparison(overrides: Partial<ThreadSyncComparison> = {}): ThreadSyncComparison {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Attic R2",
    compared_at: NOW,
    entries: [fits, waits],
    counts: { synced: 0, local: 2, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    capture_budget: {
      limit_bytes: LIMIT,
      reserved_bytes: 3 * MB,
    },
    remote_continuity: { state: "in_sync", revision: 1 },
    ...overrides,
  };
}

function session(threadId: string, title: string, endedAt: number): ProjectSessionSummary {
  return {
    resource_id: `codex:conversation:${threadId}`,
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: threadId,
    logical_thread_id: threadId,
    source: "local_only",
    access: "bound_ready",
    title,
    preserved: null,
    summary: "",
    started_at: endedAt - 1_000,
    ended_at: endedAt,
    working_directory: null,
    recorded_sha: "a".repeat(40),
    is_active: false,
    user_round_count: 2,
    agent_message_count: 3,
    tool_call_count: 4,
    total_tokens: 1_200,
    metrics_complete: true,
    actions: [
      {
        action_id: "codex.open-in-app",
        kind: "open_in_app",
        label: "Open in Codex",
        enabled: true,
        disabled_reason: null,
      },
    ],
  };
}

const activity: ProjectActivity = {
  project_id: project.local_project_id,
  sessions: [
    session(FITS_ID, "Refine sync review UX", NOW - 3_600),
    session(WAITS_ID, "Catalog profile launch buttons", NOW - 5 * 86_400),
  ],
  git: null,
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
  warnings: [],
  window_start: NOW - 400 * 86_400,
  window_end: NOW,
  // The backend still reports what lies before the window; a review ignores it.
  older: { newest_at: NOW - 401 * 86_400, commits: 3, sessions: 2 },
  storage_sync: [],
};

function renderReviewList(selectable: Set<string>) {
  return renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={project}
      binding={binding}
      activity={activity}
      loading={false}
      loadingMore={false}
      loadError={null}
      onRetryLoad={() => undefined}
      actionBusyResourceId={null}
      onShowLess={() => undefined}
      canShowLess={false}
      onLoadMore={() => undefined}
      onSessionAction={() => undefined}
      comparison={comparison()}
      selectionMode="push"
      selectedResourceIds={selectable}
      selectableResourceIds={selectable}
      onToggleResource={() => undefined}
    />,
  );
}

test("all eligible sessions stay selectable regardless of age or list position", () => {
  const html = renderReviewList(new Set([fits.resource_id, waits.resource_id]));

  // Both rows are on screen and selectable in one list.
  assert.match(html, /Refine sync review UX/);
  assert.match(html, /Catalog profile launch buttons/);
  assert.doesNotMatch(html, /Older activity|sync-history-older|Load earlier/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  assert.doesNotMatch(html, />Later<\/span>/);
  assert.equal((html.match(/sync-thread-sync-indicator local"/g) ?? []).length, 2);
  assert.match(html, /data-icon="drive"[^>]*>(?:<[^>]*>)*<\/svg><b>96 MB<\/b>/);
  assert.match(html, /<b>118 MB<\/b>/);
});

function conversationResource(threadEntry: ThreadSyncEntry): InventoryResource {
  return {
    resource_id: threadEntry.resource_id,
    kind: { key: "conversation", version: 1 },
    provider: "codex",
    provider_resource_type: "session",
    display_name: threadEntry.display_name,
    provenance: { kind: "conversation", version: 1, fields: {} },
    apply_policy: { key: "safe_file", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Conversation",
    apply_policy_label: "Safe file",
    selectable: true,
    default_selected: true,
  };
}

const projectFile: InventoryResource = {
  resource_id: "project:file:assets/recording.mov",
  kind: { key: "project_content_file", version: 1 },
  provider: null,
  provider_resource_type: null,
  display_name: "recording.mov",
  provenance: { kind: "project_content", version: 1, fields: {} },
  apply_policy: { key: "explicit_review", version: 1 },
  codec_version: 1,
  attributes: {},
  category: "project_files",
  kind_label: "Project file",
  apply_policy_label: "Explicit review",
  selectable: true,
  logical_paths: ["project/assets/recording.mov"],
  default_selected: true,
};

const projectFiles: ProjectContentInventory = {
  local_project_id: project.local_project_id,
  storage_id: "storage-1",
  project_root: PROJECT_ROOT,
  eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  review_token: "review-token",
  storage_revision: null,
  preference_revision: 0,
  excluded_resource_ids: [],
  entries: [{
    descriptor: projectFile,
    entry_type: "file",
    relative_path: "assets/recording.mov",
    logical_path: "project/assets/recording.mov",
    size: 500 * MB,
    mode: 0o644,
    source_mtime: NOW - 60,
    state: "local_only",
    local_present: true,
    storage_present: false,
    base_present: false,
    selected_in_recipe: false,
    newly_discovered: true,
    undecided: true,
    blocked_reason: null,
    warning_code: null,
    warning_digest: null,
  }],
  ignored_count: 0,
  blocked_count: 0,
  warnings: [],
  scanned_at: NOW,
};

test("Files Select respects the 512 MB total including sessions and reserved content", () => {
  const session = entry("selected", { capture_bytes: 100 * MB });
  const reviewed = comparison({ entries: [session], capture_budget: { limit_bytes: LIMIT, reserved_bytes: 20 * MB } });
  const file = (path: string, size: number) => ({ ...flaggedFile(path, "", "").inventoryEntry, size, undecided: false });
  const first = file("a.md", 200 * MB);
  const tooLarge = file("b.mov", 400 * MB);
  const exactFit = { ...file("c.sh", 192 * MB), warning_code: "executable", warning_digest: "w".repeat(64) };
  const oneByteOver = file("d.md", 1);
  const scanned = { ...projectFiles, entries: [oneByteOver, tooLarge, exactFit, first] };
  const candidates = new Set(scanned.entries.map((row) => row.descriptor.resource_id));
  const selected = selectPushProjectFilesWithinBudget(new Set([session.resource_id]), candidates, reviewed, scanned);

  assert.deepEqual([...selected], [session.resource_id, first.descriptor.resource_id, exactFit.descriptor.resource_id]);
  assert.equal(pushBudgetTotal(selected, reviewed, scanned)?.total, LIMIT);
  assert.equal(selected.has(tooLarge.descriptor.resource_id), false);
  assert.equal(selected.has(oneByteOver.descriptor.resource_id), false);
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, projectContentInventory: scanned, deletionApproved: true }), "c.sh is executable — mark it reviewed or exclude it.");
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, projectContentInventory: scanned, deletionApproved: true, acknowledgedWarningDigests: new Set([exactFit.warning_digest]) }), null);
  assert.deepEqual(selectPushProjectFilesWithinBudget(selected, candidates, reviewed, scanned), selected);

  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined} credentialRedactionEnabled
      resources={[conversationResource(session), ...scanned.entries.map((row) => ({ ...projectFile, ...row.descriptor }))]}
      selected={new Set([session.resource_id])} projectDefaults={new Set()} busy={false}
      threadComparison={reviewed} projectContentInventory={scanned} projectContentScanned showProjectFiles
      activeStep="project_files" onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
  );
  assert.match(html, /title="Select the 2 project files that fit within the 512 MB total push limit"/);
});

test("Files Select reserves unchanged files that will be recaptured and keeps existing choices first", () => {
  const session = entry("selected", { capture_bytes: 100 * MB });
  const reviewed = comparison({ entries: [session], capture_budget: { limit_bytes: LIMIT, reserved_bytes: 20 * MB } });
  const stored = { ...flaggedFile("stored.md", "", "").inventoryEntry, size: 300 * MB, state: "synced" as const, storage_present: true, selected_in_recipe: true };
  const large = { ...flaggedFile("a.mov", "", "").inventoryEntry, size: 100 * MB };
  const chosen = { ...flaggedFile("z.md", "", "").inventoryEntry, size: 92 * MB };
  const scanned = { ...projectFiles, entries: [stored, large, chosen] };
  const candidates = new Set([large.descriptor.resource_id, chosen.descriptor.resource_id]);
  const initial = new Set([session.resource_id, stored.descriptor.resource_id]);
  assert.equal(pushBudgetTotal(initial, reviewed, scanned)?.total, 120 * MB);
  const selected = selectPushProjectFilesWithinBudget(initial, candidates, reviewed, scanned);
  assert.equal(selected.has(large.descriptor.resource_id), false);
  assert.equal(selected.has(chosen.descriptor.resource_id), true);
  assert.equal(pushBudgetTotal(selected, reviewed, scanned)?.total, LIMIT);

  const smaller = { ...large, size: 50 * MB };
  const withChoice = { ...scanned, entries: [stored, smaller, chosen] };
  assert.deepEqual(selectPushProjectFilesWithinBudget(selected, candidates, reviewed, withChoice), selected);
});

test("Files Select adds nothing when sessions and requirements use the whole budget", () => {
  const session = entry("full", { capture_bytes: 500 * MB });
  const reviewed = comparison({ entries: [session], capture_budget: { limit_bytes: LIMIT, reserved_bytes: 12 * MB } });
  const initial = new Set([session.resource_id]);
  const selected = selectPushProjectFilesWithinBudget(initial, new Set([projectFile.resource_id]), reviewed, projectFiles);
  assert.deepEqual(selected, initial);
  assert.equal(pushBudgetTotal(selected, reviewed, projectFiles)?.total, LIMIT);
});

test("automatic selection takes the newest sessions that fit, even when every session was saved", () => {
  const entries = Array.from({ length: 394 }, (_, index) => entry(`session-${index}`, {
    local_updated_at: NOW - index * 60,
    capture_bytes: 8 * MB,
  }));
  const resources = [...entries].reverse().map(conversationResource);
  const reviewed = comparison({ entries: [...entries].reverse() });
  const selected = recommendedPushSelection(new Set(resources.map((resource) => resource.resource_id)), resources, reviewed);

  assert.deepEqual([...selected], entries.slice(0, 63).map((row) => row.resource_id));
  assert.equal(pushBudgetTotal(selected, reviewed)?.total, 507 * MB);
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true }), null);
});

test("the default selection is a strict time prefix: the walk ends at the first session that does not fit", () => {
  // fits 96 MB is newest, big 450 MB is older, tiny 10 MB is oldest. tiny
  // would fit beside fits, and stays unticked: the ticked rows are one block
  // at the top of the list, never interleaved with rows that wait.
  const big = entry("big", { capture_bytes: 450 * MB, local_updated_at: NOW - 2 * 86_400 });
  const tiny = entry("tiny", { capture_bytes: 10 * MB, local_updated_at: NOW - 10 * 86_400 });
  const resources = [tiny, big, fits].map(conversationResource);
  const reviewed = comparison({ entries: [tiny, big, fits] });
  const selected = recommendedPushSelection(
    new Set(resources.map((resource) => resource.resource_id)),
    resources,
    reviewed,
  );
  assert.deepEqual([...selected], [fits.resource_id]);

  // A session that could not fit an empty push is skipped, not the end of
  // the walk: beside 3 MB of requirements, 511 MB can never be pushed, and
  // the 96 MB session behind it is still ticked.
  const huge = entry("huge", { capture_bytes: 511 * MB, local_updated_at: NOW - 60 });
  const withHuge = [huge, fits].map(conversationResource);
  const reviewedHuge = comparison({ entries: [huge, fits] });
  assert.deepEqual(
    [...recommendedPushSelection(new Set(withHuge.map((resource) => resource.resource_id)), withHuge, reviewedHuge)],
    [fits.resource_id],
  );
});

function renderWorkspace(selected: Set<string>, inventory: ProjectContentInventory | null = null) {
  return renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={[conversationResource(fits), conversationResource(waits), ...(inventory ? [projectFile] : [])]}
      selected={selected}
      projectDefaults={new Set([fits.resource_id])}
      busy={false}
      threadComparison={comparison()}
      projectContentInventory={inventory}
      projectContentScanned={inventory != null}
      showProjectFiles={inventory != null}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
}

test("an empty Push review offers a way back without enabling publication", () => {
  const synced = entry("synced", {
    state: "synced",
    storage_present: true,
    push_decision: { action: "no_changes", selectable: false },
  });
  const render = (overrides: Partial<ComponentProps<typeof PushResourceWorkspace>> = {}) => (
    renderToStaticMarkup(
      <PushResourceWorkspace onSessionActionError={() => undefined}
        credentialRedactionEnabled={true}
        project={{ ...project, is_git_repository: true }}
        resources={[conversationResource(synced)]}
        selected={new Set()}
        projectDefaults={new Set()}
        threadComparison={comparison({ entries: [synced] })}
        busy={false}
        onToggle={() => undefined}
        onClose={() => undefined}
        onPush={() => undefined}
        {...overrides}
      />,
    )
  );
  const footer = (html: string) => html.match(/<footer\b[\s\S]*?<\/footer>/)?.[0] ?? "";
  const exitButton = (html: string) => (footer(html).match(/<button\b[^>]*>[\s\S]*?<\/button>/g) ?? [])
    .find((button) => button.includes("sync-review-exit"));
  const ready = render();
  const exit = exitButton(ready);
  assert.ok(exit, "sessions with no delta still have a visible exit");
  // The exit takes the commit's slot: a box button, the footer's only one.
  assert.match(exit, /class="btn sync-review-exit"/);
  assert.match(exit, /title="Close Push review — back to Git history"/);
  assert.match(exit, /<span>Back to Git history<\/span>/);
  assert.doesNotMatch(exit, /disabled=""|btn-ghost|btn-primary/);
  assert.equal(footer(ready).match(/<button\b/g)?.length, 1, "the exit is the empty footer's only button");
  assert.doesNotMatch(ready, /sync-pull-apply-button/, "no dead Push in an empty review");
  assert.doesNotMatch(ready, /sync-review-budget-(glance|gauge)/, "no meter for a push that cannot happen");
  const gitOnly = render({
    gitHistoryChanged: true,
    gitHistory: {
      branch: "main", head_sha: "a".repeat(40), truncated: false,
      commits: [{ sha: "a".repeat(40), committed_at: 100, subject: "Save artwork", session_refs: [] }],
      uncommitted_session_refs: [], outside_history_session_refs: [],
    },
  });
  // The history travels on the session rail; the review grows no band for it.
  assert.doesNotMatch(gitOnly, /<details|Git history: main|Save artwork/);
  assert.equal(exitButton(gitOnly), undefined, "a Git-only change keeps the Push action");
  assert.match(footer(gitOnly), /class="btn btn-primary sync-pull-apply-button"/);
  assert.doesNotMatch(footer(gitOnly), /disabled=""/, "the reviewed Git-only Push is enabled");
  assert.match(exitButton(render({ busy: true })) ?? "", /disabled=""/);
  for (const listState of ["loading", "failed"] as const) {
    assert.equal(exitButton(render({ listState })), undefined, "unsettled reads are not empty reviews");
  }
  const changed = render({
    resources: [conversationResource(fits)],
    threadComparison: comparison({ entries: [fits] }),
  });
  assert.equal(exitButton(changed), undefined, "an unticked change is not an empty review");
  assert.match(changed, /class="btn btn-primary sync-pull-apply-button"[^>]*disabled=""/);
  // A non-Git review closes onto the project, named as its crumb names it.
  const nonGit = { ...project, is_git_repository: false };
  const filesStep = render({ project: nonGit, showProjectFiles: true, activeStep: "project_files" });
  assert.match(exitButton(filesStep) ?? "", /<span>Back to Mallard local<\/span>/);
  assert.doesNotMatch(footer(filesStep), /btn-ghost/, "no step navigator between two empty tabs");
  // A running file scan may still land files: the footer stays the loading one.
  const scanning = render({ project: nonGit, showProjectFiles: true, projectContentLoading: true });
  assert.equal(exitButton(scanning), undefined);
  assert.match(scanning, /sync-pull-apply-button/);
});

test("manual selection can use an older session", () => {
  const html = renderWorkspace(new Set([waits.resource_id]));
  assert.doesNotMatch(html, /Too much to push at once|push again after this one|>Later</i);
  assert.match(html, /Catalog profile launch buttons/);
  assert.match(html, /This push · 121 MB of 512 MB/);
});

test("the meter is drawn in both pinned zones and sums sessions, requirements, and project files", () => {
  const within = renderWorkspace(new Set([fits.resource_id]));
  // The header glance and the footer gauge are the review's two zones that do
  // not scroll away, and they carry the same figure.
  assert.equal(within.match(/class="sync-review-budget-figure"/g)?.length, 2);
  assert.match(within, /class="sync-review-budget-glance" title="Sessions 96 MB · Requirements 3\.0 MB · Project files 0 B"/);
  assert.match(within, /class="sync-review-budget-gauge push"[^>]*role="status"/);
  // Only the footer's announces. Two live regions would read every tick twice.
  assert.equal(within.match(/class="sync-review-budget[^"]*"[^>]*role="status"/g)?.length, 1);
  assert.match(within, /role="meter"[^>]*aria-valuemax="536870912"[^>]*aria-valuenow="103809024"/);
  assert.match(within, /aria-valuetext="This push · 99 MB of 512 MB"/);
  assert.match(within, /<b>99 MB<\/b> \/ 512 MB/);
  // The header shares its row with the redaction flag and the Select pill:
  // the glance is its figure alone, and the footer gauge does the filling.
  assert.doesNotMatch(within, /sync-review-budget-track/);
  assert.doesNotMatch(within, /sync-review-footer-note/);
  const pushWithin = within.slice(within.lastIndexOf("<button", within.indexOf("sync-pull-apply-button")));
  assert.doesNotMatch(pushWithin.slice(0, pushWithin.indexOf(">")), /disabled=""/);

  // A 500 MB project file carries the total past the limit: both figures turn
  // to the overshoot, the gauge turns over, the refusal takes its own row, and
  // Push refuses with the same sentence as its title.
  const over = renderWorkspace(new Set([fits.resource_id, projectFile.resource_id]), projectFiles);
  assert.match(over, /class="sync-review-budget-gauge push over"/);
  assert.doesNotMatch(over, /sync-review-budget-track/);
  assert.equal(over.match(/class="sync-review-budget-figure over"/g)?.length, 2);
  assert.match(over, /<b>\+87 MB<\/b>/);
  assert.equal(
    over.match(/87 MB over the 512 MB push limit — exclude sessions or files to push\./g)?.length,
    // Once as the refusal row, once as the Push button's title, and once in
    // each figure's aria-valuetext.
    4,
  );
  assert.match(over, /class="sync-review-footer-note danger">87 MB over the 512 MB push limit/);
  const pushOver = over.slice(over.lastIndexOf("<button", over.indexOf("sync-pull-apply-button")));
  const pushOverTag = pushOver.slice(0, pushOver.indexOf(">"));
  assert.match(pushOverTag, /disabled=""/);
  assert.match(pushOverTag, /title="87 MB over the 512 MB push limit — exclude sessions or files to push\."/);
});

test("one rule refuses the push, and it is the one both push buttons read", () => {
  // Regression: the path bar's Push publishes the open review, and its own
  // disable rule knew nothing about the capture budget or the project-file
  // deletion approval — so a review the footer refused could still be pushed
  // from the bar above it.
  const overBudget = new Set([fits.resource_id, projectFile.resource_id]);
  assert.equal(
    pushReviewDisabledReason({
      selected: overBudget,
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      deletionApproved: true,
    }),
    "87 MB over the 512 MB push limit — exclude sessions or files to push.",
  );
  // Unticked deletions refuse too, and that state lives with the selection.
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      projectContentRemovals: new Set([projectFile.resource_id]),
      deletionApproved: false,
    }),
    "Approve the selected project file deletions",
  );
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      projectContentRemovals: new Set([projectFile.resource_id]),
      deletionApproved: true,
    }),
    null,
  );
  // An empty review, and one still scanning, refuse as they always did.
  assert.equal(
    pushReviewDisabledReason({ selected: new Set(), deletionApproved: true }),
    "Nothing is selected",
  );
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      deletionApproved: true,
      projectContentLoading: true,
    }),
    "Project files are still being scanned",
  );
  // A Push waits for its rows: the defaults are ticked before the list has
  // landed, and nothing ticked by default may travel unseen. It is the first
  // reason, ahead of even an over-limit selection.
  assert.equal(
    pushReviewDisabledReason({
      selected: overBudget,
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      deletionApproved: true,
      listState: "loading",
    }),
    "Loading sessions…",
  );
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      deletionApproved: true,
      listState: "failed",
    }),
    "The session list didn't load. Try again in the list.",
  );
  // And a review that fits, with nothing held, may push.
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      deletionApproved: true,
    }),
    null,
  );
});

test("the panel's footer button refuses on exactly the shared rule", () => {
  // The footer stopped carrying a rule of its own; this pins that it still
  // refuses where the shared one does, with that reason as its title.
  const over = renderWorkspace(new Set([fits.resource_id, projectFile.resource_id]), projectFiles);
  const reason = pushReviewDisabledReason({
    selected: new Set([fits.resource_id, projectFile.resource_id]),
    threadComparison: comparison(),
    projectContentInventory: projectFiles,
    deletionApproved: true,
  });
  assert.equal(reason, "87 MB over the 512 MB push limit — exclude sessions or files to push.");
  const push = over.slice(over.lastIndexOf("<button", over.indexOf("sync-pull-apply-button")));
  const tag = push.slice(0, push.indexOf(">"));
  assert.match(tag, /disabled=""/);
  assert.match(tag, new RegExp(`title="${reason!.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`));
});

test("pushBudgetTotal is null without a budget and otherwise sums what the selection would push", () => {
  assert.equal(pushBudgetTotal(new Set([fits.resource_id]), comparison({ capture_budget: null })), null);
  const total = pushBudgetTotal(
    new Set([fits.resource_id, waits.resource_id, projectFile.resource_id]),
    comparison(),
    projectFiles,
  );
  assert.deepEqual(total, {
    sessions: (96 + 118) * MB,
    reserved: 3 * MB,
    projectFiles: 500 * MB,
    total: (96 + 118 + 3 + 500) * MB,
    limit: LIMIT,
  });
});

test("five stored 96 MiB sessions do not expand a selected 40 MiB capture", () => {
  const selectedEntry = entry("selected", { capture_bytes: 40 * MB });
  const syncedEntries = Array.from({ length: 5 }, (_, index) => entry(`synced-${index}`, {
    state: "synced",
    storage_present: true,
    push_decision: { action: "no_changes", selectable: false },
  }));
  const resources = [...syncedEntries, selectedEntry].map(conversationResource);
  const reviewed = comparison({
    entries: [...syncedEntries, selectedEntry],
    capture_budget: { limit_bytes: LIMIT, reserved_bytes: 0 },
  });
  const savedRecipe = recipeWithSelection(
    { schema_version: 1, revision: 1, entries: {} },
    resources,
    new Set(syncedEntries.map((row) => row.resource_id)),
  );
  const selected = sanitizePushSelection(new Set([selectedEntry.resource_id]), resources, reviewed);
  const captureRecipe = recipeWithSelection(savedRecipe, resources, selected);

  assert.deepEqual(Object.keys(captureRecipe.entries), [selectedEntry.resource_id]);
  assert.equal(pushBudgetTotal(selected, reviewed)?.total, 40 * MB);
  assert.equal(
    pushBudgetTotal(new Set(Object.keys(captureRecipe.entries)), reviewed)?.total,
    40 * MB,
    "the meter and backend recipe use the same capture set",
  );
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true }), null);
});


test("Push review shows the selected redaction mode without the footer warning", () => {
  for (const enabled of [true, false]) {
    const html = renderToStaticMarkup(
      <PushResourceWorkspace onSessionActionError={() => undefined} credentialRedactionEnabled={enabled} resources={[]} selected={new Set()} projectDefaults={new Set()} busy={false} onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
    );
    assert.match(html, new RegExp(`Credential redaction: ${enabled ? "On" : "Off"}`));
    assert.doesNotMatch(html, /Not redacted:|redaction-off/);
    // The setting is not a message: nothing joins the banner zone over the list.
    assert.doesNotMatch(html, /sync-review-banners/);
  }
});


test("current sessions remain selectable for a reviewed mode change", () => {
  const changed = entry("mode-change", { state: "synced", storage_present: true, status_detail: "Export settings changed", push_decision: { action: "publish", selectable: true } });
  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined} credentialRedactionEnabled={false} resources={[conversationResource(changed)]}
      selected={new Set([changed.resource_id])} projectDefaults={new Set([changed.resource_id])}
      busy={false} threadComparison={comparison({ entries: [changed] })}
      onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
  );
  assert.match(html, /1 of 1 selected/);
  assert.doesNotMatch(html, /Nothing to push/);
});

/* A selected item that stops the push. A scanner finding is cleared by Mark
   reviewed on its card, an item the capture cannot read only by excluding
   it, and the footer used to say "can't be captured — exclude it" for both. */

function flaggedFile(path: string, warningCode: string, digest: string) {
  const descriptor: InventoryResource = {
    ...projectFile,
    resource_id: `project:file:${path}`,
    display_name: path.split("/").pop()!,
    logical_paths: [`project/${path}`],
  };
  const inventoryEntry = {
    ...projectFiles.entries[0],
    descriptor,
    relative_path: path,
    logical_path: `project/${path}`,
    size: 1024,
    mode: 0o755,
    warning_code: warningCode,
    warning_digest: digest,
  };
  return { descriptor, inventoryEntry };
}

test("the blocker sentence words each cause by its fix, naming one item and counting several", () => {
  const run = flaggedFile("scripts/run.sh", "executable", "e".repeat(64));
  const env = flaggedFile(".env.local", "possible_secret", "s".repeat(64));
  const unknown = flaggedFile("notes/plan.md", "", "");
  const inventory: ProjectContentInventory = {
    ...projectFiles,
    entries: [run.inventoryEntry, env.inventoryEntry, { ...unknown.inventoryEntry, state: "unknown", warning_code: null, warning_digest: null }],
  };
  const lost = entry("lost", { display_name: "Fix login redirect", state: "unavailable" });
  const sessions = comparison({ entries: [lost] });
  const flagged = (resourceId: string) => ({ kind: "local" as const, step: "project_files" as const, resourceId, state: "warning_unacknowledged" });

  assert.equal(
    pushBlockerSentence([flagged(run.descriptor.resource_id)], sessions, inventory),
    "run.sh is executable — mark it reviewed or exclude it.",
  );
  assert.equal(
    pushBlockerSentence([flagged(env.descriptor.resource_id)], sessions, inventory),
    ".env.local may hold a secret — mark it reviewed or exclude it.",
  );
  assert.equal(
    pushBlockerSentence([flagged(run.descriptor.resource_id), flagged(env.descriptor.resource_id)], sessions, inventory),
    "2 files need review — mark them reviewed or exclude them.",
  );
  const unreadableFile = { kind: "local" as const, step: "project_files" as const, resourceId: unknown.descriptor.resource_id, state: "unknown" };
  const unreadableSession = { kind: "local" as const, step: "history" as const, resourceId: lost.resource_id, state: "unavailable" };
  assert.equal(pushBlockerSentence([unreadableFile], sessions, inventory), "plan.md can’t be captured — exclude it to push.");
  assert.equal(pushBlockerSentence([unreadableSession], sessions, inventory), "“Fix login redirect” can’t be captured — exclude it to push.");
  assert.equal(
    pushBlockerSentence([unreadableSession, flagged(run.descriptor.resource_id)], sessions, inventory),
    "run.sh is executable — mark it reviewed or exclude it. “Fix login redirect” can’t be captured — exclude it to push.",
  );
  // Storage holds are the banner's, not the footer's.
  const held = { kind: "storage" as const, step: "history" as const, resourceId: lost.resource_id, state: "storage_ahead" };
  assert.equal(pushBlockerSentence([held], sessions, inventory), null);

  // The pill's short form: a count and what it needs.
  assert.equal(pushBlockerPillLabel([flagged(run.descriptor.resource_id)]), "1 needs review");
  assert.equal(pushBlockerPillLabel([flagged(run.descriptor.resource_id), flagged(env.descriptor.resource_id)]), "2 need review");
  assert.equal(pushBlockerPillLabel([unreadableFile]), "1 unavailable");
  assert.equal(pushBlockerPillLabel([unreadableFile, flagged(run.descriptor.resource_id)]), "2 need attention");
  assert.equal(pushBlockerPillLabel([held]), null);
});

test("a flagged file stops Push with a pill at the button, and both Push buttons read its sentence", () => {
  const run = flaggedFile("scripts/run.sh", "executable", "e".repeat(64));
  const inventory: ProjectContentInventory = { ...projectFiles, entries: [run.inventoryEntry] };
  const selected = new Set([fits.resource_id, run.descriptor.resource_id]);
  const sentence = "run.sh is executable — mark it reviewed or exclude it.";
  // The path bar's Push reads the same rule as the footer's.
  assert.equal(
    pushReviewDisabledReason({ selected, threadComparison: comparison(), projectContentInventory: inventory, deletionApproved: true }),
    sentence,
  );

  const render = (acknowledged: ReadonlySet<string>) => renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={[conversationResource(fits), conversationResource(waits), run.descriptor]}
      selected={selected}
      projectDefaults={new Set([fits.resource_id, run.descriptor.resource_id])}
      busy={false}
      threadComparison={comparison()}
      projectContentInventory={inventory}
      projectContentScanned
      acknowledgedWarningDigests={acknowledged}
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  const html = render(new Set());
  const footer = html.slice(html.indexOf("<footer"));
  assert.match(footer, new RegExp(`<button type="button" class="sync-review-blocker-pill" title="${sentence}"><svg[^>]*data-icon="alert-triangle"[\\s\\S]*?</svg>1 needs review</button>`));
  // In the cluster: after the step navigator, before the gauge and Push.
  assert.ok(footer.indexOf("Back to session selection") < footer.indexOf("sync-review-blocker-pill"));
  assert.ok(footer.indexOf("sync-review-blocker-pill") < footer.indexOf("sync-review-budget-gauge"));
  // The whole sentence is the region a screen reader hears, and Push's title.
  assert.match(footer, new RegExp(`<span class="sync-visually-hidden" role="status">${sentence}</span>`));
  assert.match(footer, new RegExp(`sync-pull-apply-button" disabled="" title="${sentence}"`));
  // No footer sentence is a squeezed column any more.
  assert.doesNotMatch(footer, /<small>/);
  assert.doesNotMatch(footer, /can.t be captured/);
  // The counts print only what the commit does, sized to share the row.
  assert.match(footer, /<div class="sync-project-file-commit-impact" aria-label="Selected project file impact"><div class="sync-project-file-impact-counts"><span><strong>1<\/strong> Add<\/span><\/div>/);
  // The card the pill opens carries its id.
  assert.match(html, /data-resource-id="project:file:scripts\/run\.sh"/);

  // Marked reviewed: the pill goes, the region empties, and Push is live.
  const reviewed = render(new Set(["e".repeat(64)]));
  const reviewedFooter = reviewed.slice(reviewed.indexOf("<footer"));
  assert.doesNotMatch(reviewedFooter, /sync-review-blocker-pill/);
  assert.match(reviewedFooter, /<span class="sync-visually-hidden" role="status"><\/span>/);
  const push = reviewedFooter.slice(reviewedFooter.lastIndexOf("<button", reviewedFooter.indexOf("sync-pull-apply-button")));
  assert.doesNotMatch(push.slice(0, push.indexOf(">")), /disabled=""/);
});

test("unchanged warned files do not block a session push without file capture", () => {
  const files = [flaggedFile("run.sh", "executable", "e".repeat(64)), flaggedFile(".env.local", "possible_secret", "s".repeat(64))];
  const inventory: ProjectContentInventory = {
    ...projectFiles,
    entries: files.map(({ inventoryEntry }) => ({
      ...inventoryEntry,
      state: "synced",
      storage_present: true,
      selected_in_recipe: true,
      undecided: false,
    })),
  };
  const selected = new Set([fits.resource_id, ...files.map(({ descriptor }) => descriptor.resource_id)]);
  for (const projectFilesShown of [false, true]) {
    assert.equal(pushProjectFileWork(inventory, selected, new Set(), projectFilesShown), null);
    assert.equal(pushReviewDisabledReason({
      selected, threadComparison: comparison(), projectContentInventory: inventory,
      projectFilesShown, deletionApproved: true,
    }), null);
    const html = renderToStaticMarkup(
      <PushResourceWorkspace onSessionActionError={() => undefined} credentialRedactionEnabled={false}
        resources={[conversationResource(fits), ...files.map(({ descriptor }) => descriptor)]}
        selected={selected} projectDefaults={selected} busy={false} threadComparison={comparison()}
        projectContentInventory={inventory} projectContentScanned projectFilesShown={projectFilesShown}
        showProjectFiles activeStep="project_files" onToggle={() => undefined}
        onReviewProjectContentWarnings={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
    );
    assert.match(html, /aria-label="Project files, 0 selected"/);
    assert.doesNotMatch(html, /sync-review-blocker-pill|Mark reviewed/);
    assert.match(html, /sync-pull-apply-button"(?![^>]*disabled)/);
  }
});

for (const work of ["send", "remember"] as const) {
  test(`${work} file work exposes unchanged warned files until and after approval`, () => {
    const run = flaggedFile("run.sh", "executable", "e".repeat(64));
    const env = flaggedFile(".env.local", "possible_secret", "s".repeat(64));
    const fresh = flaggedFile("notes.md", "", "");
    const inventory: ProjectContentInventory = {
      ...projectFiles,
      entries: [run, env].map(({ inventoryEntry }): ProjectContentInventory["entries"][number] => ({
        ...inventoryEntry,
        state: "synced",
        storage_present: true,
        selected_in_recipe: true,
        undecided: false,
      })).concat([fresh.inventoryEntry]),
    };
    const selected = new Set([fits.resource_id, run.descriptor.resource_id, env.descriptor.resource_id]);
    if (work === "send") selected.add(fresh.descriptor.resource_id);
    assert.equal(pushProjectFileWork(inventory, selected, new Set(), true), work);
    for (const approvedCount of [0, 1, 2]) {
      const acknowledged = new Set([run.inventoryEntry.warning_digest, env.inventoryEntry.warning_digest].slice(0, approvedCount));
      const reason = pushReviewDisabledReason({
        selected, threadComparison: comparison(), projectContentInventory: inventory,
        projectFilesShown: true, acknowledgedWarningDigests: acknowledged, deletionApproved: true,
      });
      assert.equal(reason, approvedCount === 0
        ? "2 files need review — mark them reviewed or exclude them."
        : approvedCount === 1 ? ".env.local may hold a secret — mark it reviewed or exclude it." : null);
      const html = renderToStaticMarkup(
        <PushResourceWorkspace onSessionActionError={() => undefined} credentialRedactionEnabled
          resources={[conversationResource(fits), run.descriptor, env.descriptor, fresh.descriptor]}
          selected={selected} projectDefaults={selected} busy={false} threadComparison={comparison()}
          projectContentInventory={inventory} projectContentScanned projectFilesShown
          acknowledgedWarningDigests={acknowledged} showProjectFiles activeStep="project_files"
          onToggle={() => undefined} onReviewProjectContentWarnings={() => undefined}
          onClose={() => undefined} onPush={() => undefined} />,
      );
      assert.match(html, /data-resource-id="project:file:run\.sh"/);
      assert.match(html, /data-resource-id="project:file:\.env\.local"/);
      assert.equal((html.match(/aria-label="Reviewed: (Executable|Credential-shaped) content/g) ?? []).length, 2);
      // The step's own box follows the rows: unticked, mixed, then done.
      const strip = html.match(/<label class="notice warning notice-ack sync-project-files-review-all[^"]*">[\s\S]*?<\/label>/)?.[0] ?? "";
      assert.match(strip, /2 selected files flagged<\/strong> — 1 executable · 1 credential-shaped/);
      assert.equal(/aria-checked="mixed"/.test(strip), approvedCount === 1);
      assert.equal(/review-all done"/.test(strip), approvedCount === 2);
      if (approvedCount < 2) {
        assert.match(html, /Mark reviewed/);
        assert.match(html, /sync-review-blocker-pill/);
        assert.match(html, /sync-pull-apply-button" disabled=""/);
      } else {
        assert.doesNotMatch(html, /sync-review-blocker-pill/);
        assert.match(html, /sync-pull-apply-button"(?![^>]*disabled)/);
      }
    }
  });
}

test("the blocker pill opens its card: the step switches and the files page scrolls to it and hands it focus", () => {
  const workspace = readFileSync("src/components/project-sync/PushResourceWorkspace.tsx", "utf8");
  assert.match(workspace, /onClick=\{revealFirstBlocker\}/);
  assert.match(workspace, /selectStep\(target\.step\);/);
  assert.match(workspace, /reveal=\{projectFileReveal\}/);
  const page = readFileSync("src/components/project-sync/ProjectFilesReviewPage.tsx", "utf8");
  // A card the search hides, or one past the render limit, is searched for.
  assert.match(page, /if \(!shownCards\.includes\(row\)\) setSearch\(row\.relativePath\);/);
  assert.match(page, /scrollIntoView\(\{ block: "nearest" \}\)/);
  // Focus goes to Mark reviewed first, the row's tick only without one.
  assert.match(page, /querySelector<HTMLInputElement>\(".notice-ack input"\)\s*\?\? card\.querySelector<HTMLInputElement>\("\.sync-sync-row-choice input"\)/);
});

/* The Push review in Chinese: the review's own words translate, and what the
   user named — sessions, files, storages — stays exactly as given. */
test("the Push review header and footer speak Chinese around the user's names", () => {
  const run = flaggedFile("scripts/run.sh", "executable", "e".repeat(64));
  const inventory: ProjectContentInventory = { ...projectFiles, entries: [run.inventoryEntry] };
  void i18n.changeLanguage("zh-CN");
  try {
    const html = renderWorkspace(new Set([fits.resource_id]));
    const header = html.match(/<header\b[\s\S]*?<\/header>/)?.[0] ?? "";
    const footer = html.match(/<footer\b[\s\S]*?<\/footer>/)?.[0] ?? "";
    assert.match(header, /<h2 id="sync-push-resource-title">上传检查<\/h2>/);
    assert.match(header, /<span class="sync-settings-flag">密钥隐藏：开<\/span>/);
    assert.match(header, /aria-label="关闭上传检查"/);
    assert.match(header, /title="会话 96 MB · 依赖 3\.0 MB · 项目文件 0 B"/);
    assert.match(footer, /title="会话 96 MB · 依赖 3\.0 MB · 项目文件 0 B"/);
    assert.match(footer, /<\/span>上传<\/button>/);
    assert.match(html, /<strong>会话<\/strong><span>此项目的代理会话<\/span>/);
    // The session's own title is never translated.
    assert.match(html, /<strong>Refine sync review UX<\/strong>/);
    assert.match(html, /aria-label="选择“Refine sync review UX”"/);
    assert.doesNotMatch(html, />Push review<|Credential redaction|>Push<\/button>/);

    // The footer's refusals, and the blocker that names a file as it is.
    assert.equal(pushReviewDisabledReason({ selected: new Set(), deletionApproved: true }), "未选择任何内容");
    assert.equal(
      pushReviewDisabledReason({ selected: new Set(), deletionApproved: true, listState: "loading" }),
      "正在载入会话…",
    );
    assert.equal(
      pushReviewDisabledReason({ selected: new Set([fits.resource_id, run.descriptor.resource_id]), threadComparison: comparison(), projectContentInventory: inventory, deletionApproved: true }),
      "run.sh 是可执行文件——请标为已检查或将其排除。",
    );
    assert.equal(pushBlockerPillLabel([{ kind: "local", step: "project_files", resourceId: run.descriptor.resource_id, state: "warning_unacknowledged" }]), "1 个需检查");
    // A session is quoted inside its sentence; one the review cannot name
    // reads as "a selected session".
    const lost = entry("lost", { display_name: "修复登录跳转", state: "unavailable" });
    const unreadable = { kind: "local" as const, step: "history" as const, resourceId: lost.resource_id, state: "unavailable" };
    assert.equal(pushBlockerSentence([unreadable], comparison({ entries: [lost] }), inventory), "“修复登录跳转”无法读取——请将其排除后再上传。");
    assert.equal(pushBlockerSentence([unreadable], comparison({ entries: [] }), inventory), "所选会话无法读取——请将其排除后再上传。");

    // The banner a storage that moved on raises: its name stays as given.
    const copy = pullRequiredCopy("remote_advanced", "Backup drive", 7, 5, 2);
    assert.equal(copy.title, "存储中有更新的更改");
    assert.equal(copy.body, "Backup drive 中有 2 个会话较新。请在下载中检查——之后它们仍会暂缓，你的其他会话照常上传。");
    assert.equal(copy.detail, "Version 7 · 已检查 5");
    assert.equal(pullRequiredCopy("remote_without_base", "  ", 7, null).body, "从此连接上传前，请先检查此存储中已有的数据。");
  } finally {
    void i18n.changeLanguage("en");
  }
  // English is back the moment the language is.
  assert.match(renderWorkspace(new Set([fits.resource_id])), /<h2 id="sync-push-resource-title">Push review<\/h2>/);
  assert.equal(
    pullRequiredCopy("remote_advanced", "Backup drive", 7, 5, 1).body,
    "1 conversation is newer in Backup drive. Review it in Pull — it stays held afterwards while your other sessions push.",
  );
  assert.equal(pullRequiredCopy("remote_advanced", null, 7, null).body, "this storage changed after this computer last synced. Review it in Pull before pushing.");
  assert.equal(pullRequiredCopy("remote_advanced", null, 7, null).detail, "Version 7 · review required");
});

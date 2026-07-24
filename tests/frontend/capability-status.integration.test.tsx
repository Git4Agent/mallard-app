import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ProjectWorkspaceTabs } from "../../src/components/project-sync/ProjectLinksWorkspace";
import {
  capabilityStatusPresentation,
  SkillsPluginStatusContent,
} from "../../src/components/project-sync/SkillsPluginStatusPage";
import PushResourceWorkspace, {
  pushReviewBlockers,
  pushReviewBlockingCount,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import type { CapabilityStatusItem, CapabilityStatusReport } from "../../src/types";

function capability(
  kind: "standalone_skill" | "plugin",
  name: string,
  state: string,
): CapabilityStatusItem {
  return {
    resource_id: `codex:${kind}:${name}`,
    kind,
    provider: "codex",
    scope: kind === "plugin" ? "dependency" : "provider_state",
    display_name: name,
    provenance: {},
    apply_policy: kind === "plugin" ? "explicit_install" : "safe_file",
    codec_version: 1,
    metadata: {},
    category: kind === "plugin" ? "plugins" : "skills",
    state,
    local_present: state !== "storage_only",
    storage_present: state !== "local_only" && state !== "not_compared",
    selected_in_recipe: state !== "local_only",
    logical_paths: [],
    provided_skills: [],
  };
}

function report(items: CapabilityStatusItem[]): CapabilityStatusReport {
  return {
    project_id: "project-one",
    project_name: "Project one",
    profiles: [{
      provider: "codex",
      profile_id: "profile-one",
      display_name: "Work Codex",
      path: "/Users/test/.codex",
      shared_project_count: 2,
    }],
    storage_id: "storage-one",
    storage_name: "R2 backup",
    generation: 7,
    base_generation: 6,
    compared_at: 1_753_000_000,
    items,
    warnings: [],
  };
}

function inputTagFor(html: string, accessibleName: string): string {
  const markerIndex = html.indexOf(`aria-label="${accessibleName}"`);
  assert.notEqual(markerIndex, -1, `missing input '${accessibleName}'`);
  const start = html.lastIndexOf("<input", markerIndex);
  const end = html.indexOf(">", markerIndex);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  return html.slice(start, end + 1);
}

test("an unselected blocked skill does not prevent Push", () => {
  const selectedSkill = capability("standalone_skill", "frontend-skill", "local_only");
  const blockedBackup = {
    ...capability("standalone_skill", "_oca-backup-duplicates", "blocked"),
    selected_in_recipe: false,
    storage_present: false,
    blocked_reason: "no SKILL.md declaration; not a recognizable skill",
  };
  const selected = new Set([selectedSkill.resource_id]);
  const capabilityReport = report([selectedSkill, blockedBackup]);

  assert.equal(pushReviewBlockingCount(selected, null, capabilityReport), 0);
  assert.deepEqual(pushReviewBlockers(selected, null, capabilityReport), []);

  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[selectedSkill, blockedBackup]}
      selected={selected}
      projectDefaults={selected}
      busy={false}
      error={null}
      capabilityReport={capabilityReport}
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );

  assert.doesNotMatch(html, /Pull before pushing|storage change.*review|needs attention/);
  const pushText = html.lastIndexOf(">Push</button>");
  assert.notEqual(pushText, -1);
  assert.doesNotMatch(html, /Push \d+ resources?/);
  const pushTagStart = html.lastIndexOf("<button", pushText);
  const pushTagEnd = html.indexOf(">", pushTagStart);
  assert.doesNotMatch(html.slice(pushTagStart, pushTagEnd), /disabled/);
});

test("Push rejects blocked selections while preserving storage conflict review", () => {
  const blockedSkill = {
    ...capability("standalone_skill", "broken-skill", "blocked"),
    storage_present: false,
    blocked_reason: "SKILL.md is unreadable",
  };
  const storageSkill = capability("standalone_skill", "remote-skill", "storage_only");

  assert.deepEqual(
    pushReviewBlockers(new Set([blockedSkill.resource_id]), null, report([blockedSkill])),
    [{
      kind: "local",
      step: "skills",
      resourceId: blockedSkill.resource_id,
      state: "blocked",
    }],
  );
  assert.deepEqual(
    pushReviewBlockers(new Set(), null, report([storageSkill])),
    [{
      kind: "storage",
      step: "skills",
      resourceId: storageSkill.resource_id,
      state: "storage_only",
    }],
  );

  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([blockedSkill.resource_id]),
      [blockedSkill],
      null,
      report([blockedSkill]),
    )],
    [],
  );

  const statusHtml = renderToStaticMarkup(
    <SkillsPluginStatusContent
      view="skills"
      report={report([blockedSkill])}
      loading={false}
      error={null}
      missingProfile={false}
      activeStorageName="R2 backup"
      onRefresh={() => undefined}
      onOpenProjectSettings={() => undefined}
      selectionMode="push"
      selectedResourceIds={new Set([blockedSkill.resource_id])}
      selectableResourceIds={new Set([blockedSkill.resource_id])}
      onToggleResource={() => undefined}
    />,
  );
  const blockedCheckbox = inputTagFor(statusHtml, "Include broken-skill");
  assert.match(blockedCheckbox, /disabled=""/);
  assert.doesNotMatch(blockedCheckbox, /checked=""/);
  assert.match(statusHtml, /v3-history-selection-review/);
  assert.match(statusHtml, /aria-label="Skill status"/);
  assert.doesNotMatch(statusHtml, /v3-history-header/);
  assert.doesNotMatch(statusHtml, /v3-history-storage-lens/);
  assert.doesNotMatch(statusHtml, /v3-history-toolbar-count/);

  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[blockedSkill]}
      selected={new Set([blockedSkill.resource_id])}
      projectDefaults={new Set([blockedSkill.resource_id])}
      busy={false}
      error={null}
      capabilityReport={report([blockedSkill])}
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );
  assert.match(html, />0 included</);
  assert.match(html, /aria-label="Use recommended selection \(0\)"/);
  assert.doesNotMatch(html, /Resolve selected resources|cannot be captured|needs attention/);
  assert.doesNotMatch(html, /Review Pull/);
});

test("project information exposes History, Skills, and Plugins as peer tabs", () => {
  const html = renderToStaticMarkup(
    <ProjectWorkspaceTabs
      activeTab="plugins"
      isGitRepository
      onChange={() => undefined}
    />,
  );

  assert.match(html, /role="tablist" aria-label="Project information"/);
  assert.match(html, />Git &amp; sessions</);
  assert.match(html, />Skills</);
  assert.match(html, />Plugins</);
  assert.match(html, /id="project-plugins-tab"[^>]*aria-selected="true"/);
  assert.match(html, /id="project-skills-tab"[^>]*aria-selected="false"/);
  assert.match(html, /id="project-history-tab"[^>]*aria-selected="false"/);
});

test("capability status uses precise skill and plugin language", () => {
  const skill = capability("standalone_skill", "review", "synced");
  const plugin = capability("plugin", "tools@team", "synced");
  assert.equal(capabilityStatusPresentation(skill, "R2 backup").label, "Up to date");
  assert.equal(capabilityStatusPresentation(plugin, "R2 backup").label, "Backup intent matches");

  plugin.enabled = false;
  assert.equal(capabilityStatusPresentation(plugin, "R2 backup").label, "Disabled");
});

test("capability status uses the quiet desktop sync vocabulary", () => {
  const presentation = (state: string) => (
    capabilityStatusPresentation(capability("standalone_skill", "review", state), "R2 backup")
  );

  assert.deepEqual(
    ["synced", "local_only", "local_ahead", "storage_only", "storage_ahead", "diverged", "not_compared"].map((state) => {
      const { icon, className } = presentation(state);
      return { icon, className };
    }),
    [
      { icon: "badge-check", className: "synced" },
      { icon: "upload", className: "local" },
      { icon: "upload", className: "local" },
      { icon: "download", className: "storage" },
      { icon: "download", className: "storage" },
      { icon: "git-compare-arrows", className: "diverged" },
      { icon: "cloud-off", className: "unknown" },
    ],
  );

  const blocked = capability("standalone_skill", "review", "blocked");
  assert.deepEqual(
    {
      icon: capabilityStatusPresentation(blocked, "R2 backup").icon,
      className: capabilityStatusPresentation(blocked, "R2 backup").className,
    },
    { icon: "octagon-x", className: "blocked" },
  );
});

test("the Skills status view excludes plugins", () => {
  const skill = {
    ...capability("standalone_skill", "security-review", "local_ahead"),
    metadata: { install_dir_name: "security" },
    local_digest: "a".repeat(64),
    storage_digest: "b".repeat(64),
    logical_paths: ["state/codex/skills/security/SKILL.md"],
  };
  const skillsReport = report([skill, capability("plugin", "tools@team", "synced")]);
  skillsReport.warnings = ["One skill could not be inspected"];
  const html = renderToStaticMarkup(
    <SkillsPluginStatusContent
      view="skills"
      report={skillsReport}
      loading={false}
      error={null}
      missingProfile={false}
      activeStorageName="R2 backup"
      onRefresh={() => undefined}
      onOpenProjectSettings={() => undefined}
    />,
  );

  assert.match(html, /Skill status/);
  assert.match(html, /title="1 skills"/);
  assert.match(html, /1 warning/);
  assert.ok(html.indexOf("v3-history-toolbar") < html.indexOf("v3-capability-warnings"));
  assert.ok(html.indexOf("v3-capability-warnings") < html.indexOf("</header>"));
  assert.match(html, /Local changes/);
  assert.doesNotMatch(html, /tools@team/);
  assert.doesNotMatch(html, /Project and global custom skills/);
  assert.doesNotMatch(html, /Work Codex/);
  assert.doesNotMatch(html, /Shared by/);
  assert.doesNotMatch(html, /Updated/);
  assert.doesNotMatch(html, /Generation 7/);
  assert.doesNotMatch(html, /inventory warning/);
});

test("the Plugins status view excludes standalone skills and nests provided skills", () => {
  const plugin = {
    ...capability("plugin", "tools@team", "synced"),
    metadata: { plugin_marketplace: "team", plugin_observed_version: "2.1.0" },
    local_version: "2.1.0",
    storage_version: "2.0.0",
    enabled: true,
    provided_skills: ["security-review", "release-notes"],
  };
  const pluginsReport = report([capability("standalone_skill", "security-review", "synced"), plugin]);
  pluginsReport.warnings = ["One plugin could not be inspected"];
  const html = renderToStaticMarkup(
    <SkillsPluginStatusContent
      view="plugins"
      report={pluginsReport}
      loading={false}
      error={null}
      missingProfile={false}
      activeStorageName="R2 backup"
      onRefresh={() => undefined}
      onOpenProjectSettings={() => undefined}
    />,
  );

  assert.match(html, /Plugin status/);
  assert.match(html, /title="1 plugins"/);
  assert.match(html, /1 warning/);
  assert.ok(html.indexOf("v3-history-toolbar") < html.indexOf("v3-capability-warnings"));
  assert.ok(html.indexOf("v3-capability-warnings") < html.indexOf("</header>"));
  assert.match(html, /Backup intent matches/);
  assert.match(html, /Provides security-review · release-notes/);
  assert.match(html, /In backup selection/);
  assert.doesNotMatch(html, />Install selected</);
  assert.doesNotMatch(html, />security-review<\/strong>/);
  assert.doesNotMatch(html, /Installed plugins and portable installation intent/);
});

test("a missing profile produces a recoverable project-settings state", () => {
  const html = renderToStaticMarkup(
    <SkillsPluginStatusContent
      view="skills"
      report={null}
      loading={false}
      error={null}
      missingProfile
      onRefresh={() => undefined}
      onOpenProjectSettings={() => undefined}
    />,
  );

  assert.match(html, /Choose an agent profile/);
  assert.match(html, /Open Project Settings/);
});

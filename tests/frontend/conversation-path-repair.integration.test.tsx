import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ConversationPathRepairNotice, {
  shouldShowConversationPathRepairNotice,
} from "../../src/components/project-sync/ConversationPathRepairNotice";
import { conversationPathsBlockSync } from "../../src/components/project-sync/ProjectLinksWorkspace";
import type { CodexConversationPathAudit } from "../../src/types";

function audit(patch: Partial<CodexConversationPathAudit> = {}): CodexConversationPathAudit {
  return {
    local_project_id: "project-health",
    profile_id: "profile-default-codex",
    profile_path: "/Users/hequ/.codex",
    project_root: "/Users/hequ/Desktop/project/healthGame",
    assigned_thread_count: 1,
    matching_thread_count: 0,
    issues: [{
      thread_id: "019f7500-0000-7000-8000-000000000001",
      transcript_path: "/Users/hequ/.codex/sessions/rollout.jsonl",
      recorded_cwd: "/Users/hequ/Desktop/project/game3",
      target_cwd: "/Users/hequ/Desktop/project/healthGame",
    }],
    blockers: [],
    warnings: [],
    ready: false,
    can_repair: true,
    ...patch,
  };
}

test("stale assigned conversations expose a scoped repair action", () => {
  const html = renderToStaticMarkup(
    <ConversationPathRepairNotice
      audit={audit()}
      projectName="healthGame (hequ-mac)"
      profileName="Default Codex"
      profilePath="/Users/hequ/.codex"
      busy={false}
      onRepair={() => undefined}
    />,
  );

  assert.match(html, /aria-label="1 conversation path requires repair"/);
  assert.doesNotMatch(html, /healthGame \(hequ-mac\) · Default Codex · ~\/\.codex/);
  assert.doesNotMatch(html, />1 conversation points to/);
  assert.match(html, /conversation-path-repair-button-icon/);
  assert.match(html, /conversation-path-repair-help/);
  assert.match(html, /title="1 conversation points to ~\/Desktop\/project\/game3\. Push and Pull are paused\."/);
  assert.match(html, /Repair 1 conversation path/);
});

test("safe conversations do not render the repair callout", () => {
  const readyAudit = audit({
    assigned_thread_count: 1,
    matching_thread_count: 1,
    issues: [],
    ready: true,
    can_repair: false,
  });
  const html = renderToStaticMarkup(
    <ConversationPathRepairNotice
      audit={readyAudit}
      projectName="healthGame"
      profileName="Default Codex"
      profilePath="/Users/hequ/.codex"
      busy={false}
      onRepair={() => undefined}
    />,
  );

  assert.equal(html, "");
  assert.equal(shouldShowConversationPathRepairNotice(readyAudit), false);
  assert.equal(shouldShowConversationPathRepairNotice(undefined), false);
  assert.equal(shouldShowConversationPathRepairNotice(audit()), true);
  assert.equal(shouldShowConversationPathRepairNotice(readyAudit, "audit failed"), true);
});

test("a grouped configuration can omit the repeated scope label", () => {
  const html = renderToStaticMarkup(
    <ConversationPathRepairNotice
      audit={audit()}
      projectName="healthGame"
      profileName="Default Codex"
      profilePath="/Users/hequ/.codex"
      showScope={false}
      busy={false}
      onRepair={() => undefined}
    />,
  );

  assert.match(html, /aria-label="1 conversation path requires repair"/);
  assert.doesNotMatch(html, /<strong>/);
  assert.doesNotMatch(html, /conversation-path-repair-scope/);
  assert.doesNotMatch(html, /Default Codex/);
});

test("ambiguous ownership blocks sync without offering an unsafe rewrite", () => {
  const html = renderToStaticMarkup(
    <ConversationPathRepairNotice
      audit={audit({
        issues: [],
        blockers: ["Codex thread has two rollout files"],
        can_repair: false,
      })}
      projectName="healthGame"
      profileName="Conf 2"
      profilePath="/Users/hequ/conf2/.codex"
      busy={false}
      onRepair={() => undefined}
    />,
  );

  assert.match(html, /Conversation paths need review/);
  assert.match(html, /Codex thread has two rollout files/);
  assert.doesNotMatch(html, /Repair 0 conversation paths/);
});

test("a stale or unverifiable path audit gates storage sync", () => {
  assert.equal(conversationPathsBlockSync(true, audit(), undefined, false), true);
  assert.equal(conversationPathsBlockSync(true, undefined, "audit failed", false), true);
  assert.equal(conversationPathsBlockSync(true, undefined, undefined, true), true);
  assert.equal(conversationPathsBlockSync(true, audit({
    issues: [],
    matching_thread_count: 1,
    ready: true,
    can_repair: false,
  }), undefined, false), false);
  assert.equal(conversationPathsBlockSync(false, undefined, undefined, true), false);
});

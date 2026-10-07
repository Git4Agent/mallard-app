# Session Detail Push Plan

Status: implemented and verified on October 2, 2026. Step 6 (skip local
Git-history capture; copy the stored history forward) is superseded by
`PLAN_SESSION_PUSH_GIT_HISTORY.md`.

Push from session detail must publish only the open session and its required
transcript files. It must not capture new project Git history. Project Push
continues to use its existing review flow.

This plan interprets the accepted Git choice as excluding local Git-history
capture from a session Push. It preserves project history already in storage.
Removing that stored history would change the latest project view for every
session. That is a separate project action.

**Expected behavior**

The button keeps the label `Push`. Its badge is `1` when the session has a
publishable change. A disabled button has no badge and shows the reason near
the action. The review contains only the target session.

Use the backend decision: `selectable` must be true, and `action` must be
`publish` or `create_cloud`. The displayed status alone is not sufficient.
Existing storage permission, destination, budget, and safety checks also apply.

| Session condition | Behavior |
| --- | --- |
| Local outstanding | Review and push this session. |
| Local session without a stored copy | Review and create its stored copy. |
| Synced with no export change | Disable Push. Show `This session is up to date.` |
| Synced with changed export settings | Allow the reviewed export update. |
| Cloud outstanding or divergent | Disable Push. Show the reason to review Pull. |
| Matching content with newer storage metadata | Require the existing Pull metadata review. |
| No local copy | Disable Push. Show `No local session to push.` |
| Loading, unknown, unavailable, or blocked | Disable Push. Show progress or the specific reason. |

An ineligible session must never cause a project Push or select another
session. Pull remains an explicit user action. A child transcript that is not
an independent Push resource must show that limit. It must not silently select
its parent. A parent uses the existing settings for its owned subagents.

**Implementation sequence**

1. Add a required Push scope to the existing review and publish requests:
   `project`, or `session` with the existing resource ID. Update Rust and
   TypeScript together. Update all callers explicitly. This field describes
   one operation; it is not a saved project preference.

2. Resolve `activeProjectSubview.resourceId` through the comparison's
   `conversation_resource_id`. Pass its capture `resource_id` as the Push
   target. Preserve that target and its display title through preparation,
   review, retry, and publication. A delayed
   response must not replace the state of another session. Keep the existing
   request cancellation and workflow checks.

3. Prepare a review for the target. Resolve its project and provider. Use the
   existing Push decision and review components. Restrict selection controls,
   counts, and size totals to the target and its approved transcript files.
   Show its warnings and owned-subagent choice. Exclude ordinary project files
   and unrelated provider data. A refresh must retain the same scope.

4. Enforce the target in Rust during publication. Reject selections outside
   the target, another project's resource, and project-file removals.
   Recheck eligibility and the existing source, child, redaction, and storage
   evidence. If the session is no longer eligible, stop without publication
   and refresh its status. A session Push must not publish a Git-only or empty
   result.

5. Build the new snapshot from the verified stored snapshot. Start the target
   provider scope from its stored content and apply only the selected session
   actions. Carry other sessions and generic resources forward. Retain other
   provider scopes and project files through the existing publication engine.
   Preserve the saved link recipe and project-file preferences. Update only
   the session acceptance records and scope coordinates required by the
   successful publication. Retained sessions must not gain acceptance merely
   because their stored bytes were carried forward.

6. For a session scope, skip local Git-history capture and its local review
   token. Copy `git_history` from the verified stored snapshot when present.
   Pass that copy to the existing publication engine. If stored history is
   absent, keep it absent. A local branch switch or new commit must not create
   Git work for this operation. Keep the remote publication checks intact.

Keep the current destination and Pull requirements. This plan does not add a
way to bypass them when another session changes in storage. Preserve immutable
publication, digest checks, claim handling, and acceptance only after a
successful publication. Do not add an automatic retry after a storage race.

**Why session references must be updated**

A publication manifest describes the complete stored snapshot, including
retained sessions. It is not a document containing only the changed session.

An optional `GitHistorySnapshot` contains two types of data:

- Git metadata: branch, HEAD, commit IDs, commit times, commit subjects, and
  the history truncation flag.
- Session references: the session IDs grouped under each commit, under
  `uncommitted_session_refs`, or under `outside_history_session_refs`.

The second group is an index for display. When Git history is present, the
validator requires each published session ID to appear exactly once across
these lists. References to missing sessions and duplicate references are
invalid.

For example, storage can contain session A and a commit whose reference list
is `[A]`. A session Push then adds B. The new snapshot contains A and B. If the
index still contains only A, validation fails with
`Git history must reference every published session once`.

The existing `map_sessions` helper clears the reference lists and derives them
from all final stored sessions, including retained sessions. It uses session
times and the stored commit times. It does not read Git, add commits, restore
Git state, or upload the retained session files again. This placement is a
display rule; it is not proof of the session's original branch or commit.

If B fits an existing stored commit, its ID joins that commit's list. If B is
newer than every stored commit, its ID goes in `uncommitted_session_refs` under
the current rule. That does not prove that the local repository has no commit
for B; the stored history may be old. A project Push can refresh that history.
The existing rule also handles sessions older than truncated history.

Complete references are required. Rebuilding every list is the existing
implementation, not a new schema requirement. An update to an existing session
ID might already satisfy the membership check, but its display position can
change. Reusing the helper covers both additions and updates. When Git history
is absent, this step does not run.

Preserve the stored Git metadata. Permit changes to its derived session
reference lists. Do not set history to `None` merely to skip local capture:
that would remove existing project history from the new snapshot. A session
Push can therefore repeat stored history in its new manifest without capturing
new local Git information.

**Code locations and data contract**

| Location | Responsibility |
| --- | --- |
| `src/components/project-sync/ProjectLinksWorkspace.tsx` | Session target, button state, and badge. |
| `src/components/project-sync/ProjectSync.tsx` | Scoped review state and publication flow. |
| `src/components/project-sync/PushResourceWorkspace.tsx` | Review limited to one session. |
| `src/components/project-sync/api.ts` and `src/types.ts` | Updated request contract and existing decision data. |
| `src-tauri/src/project_sync/commands.rs` and `service.rs` | Scope validation, capture, retention, Git policy, and persistence. |
| `crates/mallard-infrastructure/src/bundle_engine.rs` | Existing retained content and final session-reference rebuild. |
| `crates/mallard-domain/src/git_history.rs` | Existing `map_sessions` and completeness validation. |

The work includes frontend and Rust changes. Keep local state and portable
bundle schema 1. Add no persistent metadata fields or migration. Reuse the
existing publication records, receipts, and session acceptance records.
Current Git-history capture applies only to Mallard Cloud. Other storage
providers continue to omit it.

**Acceptance checks**

- With changed sessions A and B, push from A's detail. Prove that only A's
  content changes. Retain B's stored bytes and acceptance state.
- Verify unrelated project files, provider data, saved Push choices, and
  project-file preferences remain unchanged.
- Cover every eligibility state above, including changed export settings.
  Verify both header and review buttons use the same decision.
- Reject a target or selection outside the approved scope. Cover cancellation,
  navigation during preparation, and a state change after review.
- Cover Codex and Claude, required transcript files, and owned-subagent settings.
- With stored Git history, preserve its branch, HEAD, commit metadata, and
  truncation flag. Add a session and validate complete references exactly once.
  Verify retained session payloads are not recaptured.
- Without stored history, publish the session with history still absent.
  Verify a local branch switch or Git-only change does not alter this result.
- When the target cannot publish, create no publication or acceptance update.
  Verify a storage race fails without automatic retry.
- Verify project Push still supports its existing selection and Git-history
  behavior.

Use temporary provider homes and fixture storage. Run focused tests first,
then `npm run build`, `npm run test:frontend-integration`, and
`cargo test --manifest-path src-tauri/Cargo.toml --lib`. Verify the native Tauri
flow with fixture homes and capture a screenshot of the one-session review.
Local and fixture checks do not establish live-provider behavior.

**Verification results**

- `npm run build`: passed.
- `npm run test:frontend-integration`: 527 passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 520 passed,
  15 ignored.
- Native Tauri, temporary homes, and local fixture storage: the project had
  two local sessions. The selected session showed `Push 1`. Its review showed
  its title and one selected resource. The stored manifest contained only
  that session, with no Git history. The other session stayed local. On return
  to the published session, Push was disabled and the page showed
  `This session is up to date.`
- Rust fixture tests cover Codex and Claude, child transcript settings,
  rejected extra selections, repeat publication refusal, retained provider
  memory and project files, saved selections, stored Git commits, and complete
  session references. Cloud tests use a local service fixture.
- Removed the unused `PendingPush.projectName` field. No schema, migration,
  dependency, or publication-engine change was needed.

# Best-effort local session deletion

Status: implemented. Provider and UI checks use disposable homes.

## Behavior

Add one trash icon immediately before the **Session info** icon in each local
Codex or Claude session row. Use the existing small ghost-button style.

- Accessible name: **Delete local session**. Tooltip: **Delete local session and its subagents**.
- Show the icon in normal history rows with a resolved local provider session.
  Hide it for remote-only rows and Push/Pull selection views.
- Clicking opens a native warning with the session title, deletion scope, and
  an irreversible-action notice. **Delete** proceeds; **Cancel** does nothing.
- Disable the clicked row's actions while its request runs.
- Attempt provider deletion once. Then attempt Mallard metadata cleanup,
  even if provider deletion failed.
- Refresh the activity list after both attempts. Show partial or failed
  outcomes through the existing action-error area. Do not hide a surviving
  local file permanently.
- A stored cloud copy can remain visible as a remote-only row.

The target is the selected session and its owned subagents. Parent sessions,
sibling sessions, and independent session forks are outside the target.
Cloud storage, project files, and worktrees are outside the target.

## Provider operations

### Claude

Use the deletion scope established by `POC_CLAUDE_SESSION_DELETE.md`:

1. Attempt to remove the resolved `<session-id>.jsonl` file.
2. Attempt to remove its exact adjacent `<session-id>/` directory recursively.
3. Return any errors from either step. Missing targets count as already absent.

Implement these two filesystem operations in the Rust Claude adapter. The
desktop application does not need a Node runtime or SDK sidecar for this.
Attempt the directory step even if the transcript is absent or its removal
failed. This is a deliberate best-effort improvement over the SDK's early
return behavior.

This removes all files in that session directory, including nested child
rollouts, child metadata, and tool results. It leaves sidecars elsewhere in
the provider home, such as `file-history`, unchanged. Do not call
`claude rm` or `claude project purge`.

Use the local session path and provider profile resolved by Mallard. Do not
derive ownership from inherited message IDs or search arbitrary homes.

### Codex

Use the installed Codex CLI's app-server `thread/delete` request:

1. Resolve the executable with the existing binary resolver.
2. Start a short-lived stdio app-server with `CODEX_HOME` set to the selected
   provider profile.
3. Complete `initialize` / `initialized`, then send one `thread/delete`
   request for the selected native thread ID.
4. Consume the response and notifications. Close the helper when finished.
5. Use a bounded timeout. Return missing-CLI, unsupported-method, timeout,
   and provider errors as failed attempts. Do not retry an uncertain result.

Let Codex select its spawned descendants and remove its own session metadata.
Do not follow parent/history-base links or build a new Mallard descendant
scanner. Do not add a raw-file fallback.

Codex 0.156.0 passed the disposable native deletion test. The selected root
and spawned child were removed. The parent and independent fork remained.
This test does not prove coordination with a running writer.

## Backend and Mallard cleanup

Add one command:

`delete_local_project_session(local_project_id, resource_id)`

The frontend sends identities, not a filesystem path. The backend resolves
the local project, provider profile, native session ID, transcript path,
and current Mallard thread binding before starting.

Return two outcomes in one response:

- Provider deletion: completed, already absent, or failed, with a short message.
- Mallard cleanup: completed, already absent, or failed, with a short message.

Metadata cleanup removes only:

- The selected local conversation binding, including its installed-child map
  and path-origin reference.
- Checkpoints for the selected local thread ID on all affected storage links,
  including accepted child state stored inside those checkpoints.

Owned child state is contained in the root binding and checkpoint. Do not
match other root bindings by child ID: Claude child IDs are scoped to their
parent. Codex rejects duplicate root/child ownership in valid metadata.

Use existing persistence helpers, document validation, and atomic JSON writes.
Keep unrelated bindings, link-wide state, historical receipts, immutable
path-origin files, and cloud data. Invalidate affected activity and detail
caches and reload the list through the existing refresh path.

Metadata cleanup runs after a provider error as a separate best-effort step.
If provider files survive, a later scan can discover and bind them again.
Report this outcome; do not add a persistent hidden-session record.

## Files to change

- `src/components/project-sync/ProjectActivityPage.tsx`: icon, click handler,
  row busy state, outcome display, and refresh.
- `src/components/project-sync/api.ts` and `src/types.ts`: typed command and
  response.
- `src-tauri/src/project_sync/commands.rs`, `service.rs`, and
  `src-tauri/src/lib.rs`: command orchestration and registration. Keep provider
  operations in small provider modules.
- `crates/mallard-infrastructure/src/providers/claude/` and
  `providers/codex/`: the two provider operations.
- `crates/mallard-infrastructure/src/persistence.rs`: targeted binding and
  checkpoint removal. Reuse current session-resolution and cache helpers.
- Existing frontend and Rust test modules: focused coverage.

## Accepted failure behavior

The requested feature is best effort. It adds no file-digest review, writer
lock, active-session check, dependency scan, deletion journal, rollback,
automatic retry, or automatic provider-app shutdown.

Partial deletion, a crash between the two cleanup steps, and files recreated
by active writers are accepted outcomes. Preserve identity/path validation
needed to address the selected session; best effort does not expand its scope.

This feature intentionally overrides the repository's expected-digest and
non-recursive-directory deletion rules for this operation, as requested by
the user. Document that exception and test the resulting scope. Do not change
the safety behavior of Push, Pull, or other deletion operations.

## Acceptance and validation

1. The small trash icon is beside Session info and has an accessible name.
   After confirmation, it starts one request and handles success, partial failure, and failure.
2. Claude deletion removes the selected root and its session directory.
   Parent, sibling, independent fork, and unrelated sidecars remain.
3. Codex's native deletion is checked with temporary fixtures. Parent and
   independent fork preservation are explicit assertions; errors are reported.
4. A provider failure does not prevent the targeted Mallard cleanup attempt.
   Unrelated bindings and checkpoints remain.
5. A metadata failure does not undo provider deletion. The result identifies
   the failed step. A later refresh reflects files that remain or reappear.
6. All tests use temporary homes. No paid model requests are required.

Run `npm run build`, `npm run test:frontend-integration`, and
`cargo test --manifest-path src-tauri/Cargo.toml --lib`. Run the focused
infrastructure tests for the provider and persistence helpers. Verify the icon
and command in the complete Tauri app with fixture homes and capture a screenshot.


## Implementation validation

- Frontend build passed. Frontend integration: 424 passed.
- Tauri library: 399 passed, 8 ignored. The deletion service test also passed
  after the final metadata cleanup change.
- Infrastructure library: 746 passed, 6 ignored on the final run.
  One S3 timing test failed in the first concurrent run, then passed alone
  and in the final full run.
- The explicit native Codex fixture test passed with CLI 0.156.0.
- Native Tauri UI: the icon appeared beside Session info. Deletion removed
  the disposable Claude root, child directory, and binding. The stored copy
  remained visible without a local delete icon. The added warning was checked:
  Cancel kept the fixture file; Delete removed it. Screenshots were captured.
- Tests used disposable homes. No paid model requests were made.


## Project-level deletion

The project menu's former metadata-only reset is now **Delete all local sessions…**.
It uses this native warning, with **Cancel** and **Delete all** buttons:

> Delete all local sessions for “Project name”?
>
> Deletes sessions and subagents from its bound Codex/Claude profiles. Clears all project session bindings and local sync state.
>
> Project files and cloud copies are kept. This cannot be undone.

The command validates the project revision before file deletion. It resolves the
current bound profiles and scans each once, using the exact project-root match.
This includes old and archived sessions, even without Mallard bindings. It does
not use the visible history window, search, or sync selections.

Each discovered root uses the existing provider deletion operation. Errors do
not stop later sessions or the other provider. After these attempts, the existing
project metadata reset runs once. It removes all project session bindings,
checkpoints, scope bases, and saved Pull results. Setup, recipes, storage links,
project files, immutable path-origin files, and cloud data remain.

The response separates completed and failed session deletions, provider scan
errors, and metadata reset errors. The UI refreshes and shows partial failures.
Active writers can recreate files. There is no retry or rollback. The existing
best-effort exception to digest checks and recursive session-directory deletion
also applies to this explicit project-wide action.

Project deletion validation:

- Frontend build passed. Frontend integration: 425 passed.
- Infrastructure library: 746 passed, 6 ignored.
- Tauri library: 400 passed, 9 ignored with one test thread. The first parallel
  run had one Library publication-lock test failure. That test passed alone
  and in the full single-thread run; its code was not changed.
- Both focused project-deletion tests passed, including the explicit native
  Codex test. They cover mixed providers, archived roots, sessions without
  Mallard bindings, an unrelated parent, stale revisions, provider failures,
  and metadata failure.
- Native Tauri UI: Cancel kept the fixture files. Delete all removed the current
  and older Claude sessions plus their child trees, then cleared all project
  bindings. The other project's session, project file, and stored copy remained.
  Confirmation and result screenshots are under `output/session-delete/`.
- All deletion checks used temporary homes and local storage fixtures.

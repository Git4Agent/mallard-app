# Plan: Branch-Neutral Project Sessions

**Status:** implemented and verified
**Date:** 2026-09-04
**Scope:** Project Activity, Push review, Pull review, Codex, and Claude

## 1. Outcome

Make Git branches presentation-only for project sessions.

For one project and one loaded time range, every local Git branch must show
the same set of project sessions. The selected branch controls only the Git
commit rail that Mallard uses to organize those sessions.

For project `P`, time range `W`, and any local branches `A` and `B`:

```text
visible_sessions(P, A, W) = visible_sessions(P, B, W)
                           = project_sessions(P, W)
```

Commit placement can differ between branches:

- use the recorded SHA when that SHA is on the selected branch;
- otherwise use the existing time-based placement on the selected branch;
- if no commit qualifies, show the session as not mapped to that branch; and
- never omit a session because of its provider-recorded branch.

This rule applies equally to:

- local Codex sessions;
- local Claude sessions;
- stored-only Codex sessions;
- stored-only Claude sessions; and
- future providers that use the provider-neutral session contract.

Push and Pull remain project-wide. A Git branch must not affect comparison,
selection, planning, publication, materialization, or apply behavior.

## 2. Terminology

This plan separates three concepts that the code can call a branch or owner.

### 2.1 Git branch

A repository ref such as `main` or `test`. This plan removes Git-branch
ownership of sessions.

### 2.2 Recorded branch

Optional provenance read from a provider transcript:

- Codex reads it from authoritative `session_meta` Git metadata.
- Claude reads it from `gitBranch` records.

Recorded branch remains informational. It can appear in Session Detail and in
portable presentation metadata. It must not control visibility or sync.

### 2.3 Conversation lineage branch

A local or remote side of a diverged conversation, including the copy that
Keep Both preserves. This is not a Git branch. This plan does not remove that
identity or conflict-resolution behavior.

### 2.4 Project ownership

A session belongs to a Mallard project only when its authoritative provider
home canonicalizes to the exact project root. This safety boundary remains.
A subdirectory is a separate project.

## 3. Current behavior

### 3.1 Project membership is branch-neutral

`provider_capture::session_home_is_project_root` admits a session by exact
canonical project-root equality. It does not inspect a Git branch.

This is the correct project boundary and must remain unchanged.

### 3.2 Activity applies a branch ownership gate

`project_activity::map_sessions_to_commits` currently skips a session when its
optional recorded branch differs from the selected branch.

The skipped session is added to neither:

- a commit's `session_refs`; nor
- the `unmapped` result.

The Activity response still contains the session in `sessions`, but the Git
view renders sessions through commit references and unmapped references.
Therefore, the row is invisible.

A session without branch metadata does not enter this gate. Mallard can show
that session on several branches today. This makes current behavior dependent
on provider metadata availability.

### 3.3 Provider branch evidence is inconsistent by format

The Codex summary keeps its first applicable branch from authoritative session
metadata. The Claude summary updates its singular branch when later nonempty
`gitBranch` records appear. Claude also accumulates all observed names in
`recorded_branches`.

These format differences are valid as provenance. They are not a sound basis
for session ownership.

### 3.4 Recorded branches create synthetic branch choices

Activity currently merges local Git refs with branch names found in session
metadata. A recorded branch that no longer exists locally appears as
`History only`.

After this cutover, this behavior has no valid purpose. Every session will be
visible on every usable local branch. A deleted recorded branch would only
produce an empty rail with every session shown as unmapped.

### 3.5 Push and Pull backend planning is already branch-neutral

Push review performs one complete project discovery. It builds its resource
inventory and thread comparison from all bound providers. Neither
`prepare_push_review` nor `get_project_thread_sync_comparison` accepts a Git
branch.

Push Apply captures the explicit conversation resource IDs in the reviewed
recipe. It does not inspect the current Git branch or a recorded branch.

Pull planning reads all applicable conversations from the stored provider
scopes. `plan_bundle_restore` does not accept a Git branch. Pull Apply executes
only explicit approved action IDs.

No backend Push or Pull eligibility rule needs a branch-removal change.

### 3.6 Push and Pull review presentation has an indirect omission

Both review workflows embed `ProjectActivityPage` for the Git-and-sessions
step. The backend comparison is branch-neutral, but the Activity projection
can omit a recorded-branch mismatch from the Git rail.

The current frontend fallback does not repair this omission:

1. The session exists in `ProjectActivity.sessions`.
2. The mapper adds it to neither a commit nor `unmapped`.
3. The frontend marks its comparison entry as matched because the session
   object exists.
4. The fallback list excludes matched entries.
5. The planned session is invisible in the review.

The central Activity fix removes the normal cause. The review fallback must
also use rendered rows as its truth so a future projection defect cannot hide
a planned action.

## 4. Settled decisions

### D1 — A branch selects a commit rail, not a session set

The branch picker remains. Selecting a branch reloads that branch's
first-parent history and remaps the complete project-session set to it.

The picker remains view-only. It does not run `git switch`, update `HEAD`, or
modify project files.

### D2 — Every Activity session has one visible placement per branch

For the selected branch and loaded time range, each session must occur exactly
once in one of these places:

- under one commit;
- under the branch-unmapped group; or
- under the stored-only group when the transcript exists only in storage.

A session must not occur under two commits. It must not disappear between the
session collection and the rendered projection.

### D3 — Mapping remains branch-specific

Visibility is branch-neutral. Commit association is not.

For each selected branch:

1. Load its bounded first-parent history.
2. If the recorded SHA is on that rail, use the existing SHA-aware successor
   rule.
3. If the SHA is absent or is not on that rail, use the existing time-based
   successor rule.
4. If no successor exists, return an unmapped reference.

The same session can therefore map to one commit on `main`, another commit on
`test`, and no commit on a third branch.

### D4 — Recorded branch is provenance only

Keep the singular optional recorded branch in provider summaries and
`SessionPresentation`. Do not pass it into Activity mapping or branch
discovery.

Session Detail may show it as `Recorded branch`. It must not call the value the
session's owner or use it as a visibility gate.

### D5 — Branch choices come only from the local repository

The branch picker lists:

- local `refs/heads`; and
- the current symbolic branch, including an unborn current branch.

Provider metadata cannot add a branch choice. Remove the `History only`
state.

### D6 — All providers use one rule

The core Activity mapper must have no provider-name condition. Remove obsolete
multi-branch accumulation from the shared provider summary contract and from
both current adapters.

Adding a future provider must not require new branch-visibility code.

### D7 — Push comparison and planning stay branch-free

Do not add a branch field to:

- `get_project_thread_sync_comparison`;
- `prepare_push_review`;
- `ThreadSyncComparison`;
- `PushApproval`; or
- the reviewed bundle recipe.

Push eligibility continues to depend on project ownership, provider binding,
thread relation, remote continuity, capture limits, and explicit selection.

### D8 — Pull comparison and planning stay branch-free

Do not add a branch field to:

- `plan_bundle_restore`;
- `RestorePlan`;
- `ConversationReviewRow`; or
- `apply_bundle_restore`.

Pull eligibility continues to depend on project ownership, provider binding,
stored provider scope, thread relation, plan pins, and explicit approval.

### D9 — Reviews must show every actionable conversation

The review UI must derive fallback coverage from rows that it actually
renders. It must not infer coverage from the presence of an unrendered session
object.

Changing the selected Git branch must not change Push or Pull selection.
`Show less` can keep its existing explicit selection-removal behavior because
that operation removes reviewed rows from the loaded time range.

### D10 — Keep actual ownership and safety boundaries

Do not remove:

- exact canonical project-root session ownership;
- local provider profile bindings;
- provider-scope ownership in portable manifests;
- local logical-thread and cloud-thread bindings;
- Keep Both lineage and preserved-copy behavior;
- internal-subagent exclusions;
- capture byte limits;
- credential redaction;
- immutable publication and compare-and-swap checks;
- digest, revision, expiry, and review-token checks; or
- explicit Push and Pull approval.

These rules protect projects and provider state. They are not Git-branch
ownership.

### D11 — Clean cutover with schema 1

Keep all persisted schema numbers at `1`. Do not add a migration, feature
flag, legacy reader, dual execution path, or read-old/write-new mode.

The live test starts after local Mallard state and remote Mallard data are
removed. Automated tests must still use temporary homes and temporary storage.
They must never use the real `~/.mallard`, `~/.codex`, or `~/.claude`.

### D12 — Normal Activity remains time-windowed

`All sessions` means all project sessions in the same loaded Activity range.
Normal Activity keeps its bounded page and `Load earlier` behavior.

Push and Pull review must show every relevant comparison or plan candidate.
It can use the existing expanded review window and the rendered-row fallback.

## 5. Backend implementation

### 5.1 Remove branch from the Activity session contract

In `crates/mallard-infrastructure/src/project_activity.rs`:

- remove `ProjectSessionSummary.branch`;
- stop copying local `SessionFileSummary.branch` into Activity rows;
- stop copying remote `SessionPresentation.branch` into Activity rows; and
- update Rust fixtures that construct `ProjectSessionSummary`.

This makes branch-based Activity ownership impossible at the DTO level.

Keep `ProjectSessionSummary.recorded_sha`. The backend mapper still needs that
evidence for commit placement. The frontend can continue to ignore it.

### 5.2 Make the commit mapper total

Change:

```text
map_sessions_to_commits(sessions, commits, selected_branch)
```

to:

```text
map_sessions_to_commits(sessions, commits)
```

Delete the recorded-branch mismatch gate and its authoritative-routing
comment.

For every input session:

- map it to one successor commit; or
- add one `UnmappedSessionReference`.

Keep stable ordering by session activity and resource ID. Add a debug or test
invariant that the union of mapped and unmapped resource IDs equals the input
resource-ID set.

### 5.3 Simplify branch discovery

Replace:

```text
discover_git_branches_with_recorded_branches(...)
```

with:

```text
discover_git_branches(...)
```

Delete:

- the `recorded_branches` argument;
- the Activity-level scan of session branch values;
- insertion of recorded names into the branch set; and
- validation against synthetic recorded branch choices.

Keep the internal check that prevents `git log` from running against an
unborn ref. Rename `selected_ref_available` to `selected_has_ref` if that makes
the remaining purpose clearer.

### 5.4 Remove branch availability from the live DTO

Delete `GitBranchSummary.available` from:

- the Rust DTO in `project_activity.rs`;
- `GitBranchSummary` in `src/types.ts`; and
- all Rust and frontend fixtures.

This is a live Tauri contract cleanup. It is not a persisted schema change.

### 5.5 Remove obsolete provider multi-branch state

Delete `SessionFileSummary.recorded_branches` from
`crates/mallard-infrastructure/src/providers/traits.rs`.

Delete its construction or mutation from:

- `providers/codex/history.rs`;
- `providers/claude/history.rs`;
- `project_activity.rs` test adapters;
- `session_detail.rs` test adapters; and
- other summary fixtures found by the compiler.

Keep `SessionFileSummary.branch` as optional provenance.

### 5.6 Keep the portable presentation contract

Do not change `SessionPresentation` in
`crates/mallard-domain/src/lib.rs`:

```rust
pub struct SessionPresentation {
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub branch: Option<String>,
    pub recorded_sha: Option<String>,
    pub metrics: SessionMetrics,
}
```

`provider_capture::session_presentation` continues to validate and copy the
optional singular branch and recorded SHA into a published conversation.

The branch field remains schema-1 provenance. It does not enter Push or Pull
decisions.

## 6. Frontend implementation

### 6.1 Simplify the branch picker

In `src/components/project-sync/ProjectActivityPage.tsx`:

- remove `available` checks;
- remove `History only` labels and accessible copy;
- remove the unavailable-branch tooltip;
- retain the selected branch name and `Current` marker; and
- retain keyboard and focus behavior.

### 6.2 Remove the unavailable-branch empty state

Delete the `Branch not checked out here` case. Retain these cases:

- `No commits yet` for an unborn branch; and
- `No commits` for a loaded range with no commits.

### 6.3 Rename the unmapped presentation

Replace the Git-view heading `Uncommitted` with `Not mapped to this branch`.

Rename local variables, test IDs, comments, and CSS names that describe the
group as globally uncommitted when practical. The UI must not imply that a
session has no commit on every branch.

Keep stored-only sessions in their separate storage group.

### 6.4 Fix Push and Pull review coverage

For a Git project, derive `renderedSessionIds` from:

- every visible commit's `session_refs`; and
- every visible `unmapped` reference.

For a non-Git project, derive it from the flat session list.

Use this set when computing `matchedSyncEntries` and `unlistedEntries`.
Therefore, a comparison entry without a rendered Activity row appears in the
plain fallback list.

Do not clear selected resource IDs on a branch change. Keep selection state in
`PushResourceWorkspace` and `RestorePlanView`, where it is independent of the
Activity branch.

### 6.5 Clarify the workspace path

Update the comment in `ProjectLinksWorkspace.tsx` that says the branch scopes
the list. The new statement is:

> The branch selects the Git commit rail. Sessions remain project-wide.

### 6.6 Clarify Session Detail provenance

Keep the current `branch` response field for Session Detail. Change visible or
accessible copy from `Branch` to `Recorded branch`.

Change checkpoint explanations from `session branch` to `provider-recorded
branch`. This work does not add selected-branch context to the Session Detail
API. Detail reports transcript provenance; Activity reports the selected
branch projection.

## 7. Push and Pull planning verification

### 7.1 Push review

Verify that `prepare_push_review_with_progress`:

- discovers all sessions whose authoritative home is the project root;
- includes all bound providers;
- builds one provider-neutral `ThreadSyncComparison`;
- does not read the current Git branch; and
- does not inspect `SessionPresentation.branch` for a decision.

No production behavior change is expected in this function.

### 7.2 Push Apply

Verify that `push_bundle_reviewed_with_progress`:

- derives selected conversations only from the reviewed recipe;
- captures those exact resource IDs;
- calls `thread_review` with those explicit IDs;
- publishes each conversation to its provider scope; and
- carries recorded branch only as `SessionPresentation` metadata.

Do not add a Git branch to the recipe, review pins, scope key, or publication
expectation.

### 7.3 Pull review

Verify that `plan_bundle_restore_with_progress`:

- reads all conversations from applicable stored provider scopes;
- compares them with all project-owned local views;
- emits every non-no-change conversation review row; and
- does not read the current Git branch.

No production behavior change is expected in the planner.

### 7.4 Pull Apply

Verify that `apply_bundle_restore_with_progress`:

- validates the reviewed plan and current remote head;
- transfers selection by resource and typed action;
- rebuilds fresh apply-time pins;
- captures only the explicitly selected local conversations; and
- materializes only explicitly approved actions.

Do not add Git branch checks. Keep provider bindings and all stale-plan
protections.

### 7.5 Preserve provider-scope ownership

`split_recipe_by_scope` and the publish mutation firewall must keep separating
project resources from provider-owned resources. A Codex Push must preserve an
unbound Claude scope, and a Claude Push must preserve an unbound Codex scope.

This ownership is required for mixed-provider repositories. It is unrelated
to Git branches.

## 8. Test plan

### 8.1 Activity mapper unit tests

Add focused tests beside `map_sessions_to_commits`:

1. Map a Codex-origin session and a Claude-origin session onto `main`.
2. Map the same IDs onto `test`.
3. Assert that both result sets contain the same session IDs.
4. Assert that each ID appears exactly once in mapped plus unmapped output.
5. Verify through the provider integration tests that a session with no
   recorded branch remains visible.
6. Assert that a recorded SHA on the selected rail uses SHA-aware placement.
7. Assert that a SHA outside the selected rail uses time-based placement.
8. Assert that a branch with no suitable commit returns every session as
   unmapped instead of dropping it.

After `ProjectSessionSummary.branch` is deleted, these tests prove the rule by
construction: the mapper has no branch ownership input.

### 8.2 Branch discovery tests

Test these cases:

- local `main` and `test` refs appear;
- the current marker follows symbolic `HEAD`;
- an unborn current branch appears without a usable ref;
- a provider-recorded deleted branch does not appear; and
- a request for a non-enumerated branch is rejected.

### 8.3 Provider parser tests

For Codex and Claude:

- keep tests that prove singular recorded-branch extraction;
- remove `recorded_branches` assertions;
- keep recorded-SHA extraction tests; and
- verify that provider summary output remains bounded and sanitized.

### 8.4 Activity integration tests

Use a temporary Git repository with `main` and `test`:

- create a Codex session whose transcript records `main`;
- create a Claude session whose transcript records `test`;
- request Activity for `main` and assert that both IDs render;
- request Activity for `test` and assert that both IDs render;
- verify that placement can differ while visibility does not;
- include a stored-only conversation with a third recorded branch value; and
- verify that the stored-only row renders on both selected branches.

### 8.5 Push planning tests

In `src-tauri/src/project_sync/service.rs`:

- prepare the same Push review with `main` checked out;
- prepare it again with `test` checked out;
- compare the Codex and Claude conversation resource-ID sets;
- compare Push decisions and recommended selections;
- select both providers' conversations;
- publish to temporary storage; and
- verify that the schema-1 manifest contains both provider scopes.

A recorded branch mismatch must not cause `Blocked`, `Unavailable`, or
nonselectable Push state.

### 8.6 Pull planning tests

Use a temporary schema-1 remote manifest with one Codex conversation and one
Claude conversation. Give them different recorded branches.

- plan Pull with `main` checked out;
- plan Pull with `test` checked out;
- compare conversation review resource IDs and actions;
- approve both actions;
- verify that both providers materialize; and
- verify that an unbound provider remains view-only or blocked by its binding,
  not by Git branch.

### 8.7 Frontend review tests

Update `tests/frontend/project-activity.integration.test.tsx` and related Push
and Pull review tests:

- remove the `available` fixture field;
- remove `History only` assertions;
- remove `Branch not checked out here` assertions;
- replace `Uncommitted` assertions with `Not mapped to this branch`;
- show Codex and Claude rows under both `main` and `test`;
- switch branches in Push review and keep both rows visible;
- switch branches in Pull review and keep both rows visible;
- keep checked rows selected after each branch change; and
- force a comparison entry to lack an Activity projection and verify that the
  fallback row still renders.

### 8.8 Session Detail tests

Update detail tests to use `Recorded branch` copy. Keep the branch and SHA
provenance facts. Keep checkpoint quality and bounded Git-history tests.

### 8.9 Schema assertions

Verify that these constants remain `1`:

- `LOCAL_SCHEMA`;
- `MACHINE_PROJECT_SCHEMA`;
- `HOST_IDENTITY_SCHEMA`;
- `CONVERSATION_BINDINGS_SCHEMA`;
- `BUNDLE_SCHEMA`;
- `RESTORE_PLAN_SCHEMA`;
- `SETUP_DRAFT_SCHEMA`;
- `SETUP_TRANSACTION_SCHEMA`;
- `STORAGE_SETUP_DRAFT_SCHEMA`; and
- `PROJECT_SYNC_SCHEMA`.

Do not add migration tests. Add a direct manifest serialization assertion that
`SessionPresentation.branch` remains optional schema-1 provenance.

## 9. File change map

| File | Planned change |
| --- | --- |
| `crates/mallard-infrastructure/src/project_activity.rs` | Remove branch ownership, synthetic branch discovery, Activity branch field, and availability DTO; add total-mapping tests |
| `crates/mallard-infrastructure/src/providers/traits.rs` | Remove `recorded_branches`; keep singular `branch` provenance |
| `crates/mallard-infrastructure/src/providers/codex/history.rs` | Remove `recorded_branches` construction; keep Codex branch extraction |
| `crates/mallard-infrastructure/src/providers/claude/history.rs` | Remove `recorded_branches` accumulation; keep Claude branch extraction |
| `crates/mallard-infrastructure/src/provider_capture.rs` | Keep exact-home ownership and schema-1 presentation capture unchanged |
| `crates/mallard-infrastructure/src/session_detail.rs` | Update provenance wording and fixtures; keep detail branch metadata |
| `crates/mallard-domain/src/lib.rs` | Keep all schema constants and `SessionPresentation` unchanged; add or retain schema assertions |
| `src-tauri/src/project_sync/service.rs` | Add Push/Pull branch-neutral regression tests; do not add branch inputs |
| `src/types.ts` | Remove Activity `branch` and Git branch `available`; keep Session Detail branch |
| `src/components/project-sync/ProjectActivityPage.tsx` | Simplify branch picker, rename unmapped UI, make review fallback depend on rendered rows |
| `src/components/project-sync/ProjectLinksWorkspace.tsx` | State that the branch selects the rail, not the session list |
| `src/components/project-sync/session-detail/SessionDetailHeader.tsx` | Label branch provenance as recorded |
| `tests/frontend/project-activity.integration.test.tsx` | Update branch UI tests and add cross-provider branch-switch coverage |
| `tests/frontend/session-detail.integration.test.tsx` | Update recorded-branch wording |
| `README.md` | Document branch-neutral visibility and branch-neutral Push/Pull planning |

Do not edit historical plans unless an implementation detail in this document
must link to them for context.

## 10. Implementation sequence

1. Add failing Activity mapper and branch discovery tests.
2. Remove `ProjectSessionSummary.branch` and the mapper gate.
3. Remove recorded-session branch synthesis from Git branch discovery.
4. Remove `GitBranchSummary.available` from Rust and TypeScript together.
5. Remove `SessionFileSummary.recorded_branches` from the shared contract and
   both provider adapters.
6. Update the Activity branch picker and branch-unmapped presentation.
7. Fix rendered-row accounting in Push and Pull review.
8. Add Push and Pull branch-neutral backend tests.
9. Add frontend branch-switch and selection-retention tests.
10. Update Session Detail provenance copy and current README behavior.
11. Run the focused suites, then the complete integration suite.

No intermediate commit may leave the Rust and TypeScript Tauri contracts out
of sync.

## 11. Verification commands

Run the smallest relevant tests while editing. Before handoff, run:

```text
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:integration
```

If an S3-compatible transport test requires a loopback listener, request the
required sandbox permission. Do not point any test at real provider homes,
real Mallard state, or real cloud storage.

## 12. Acceptance criteria

The cutover is complete only when all of these statements are true:

1. For the same project and loaded time range, `main` and `test` return the
   same Activity session resource-ID set.
2. Codex and Claude follow the same rule.
3. Stored-only sessions follow the same rule.
4. Every Activity session appears exactly once on each selected branch, either
   under one commit or in an explicit non-commit group.
5. A provider-recorded branch never adds a branch-picker option.
6. A provider-recorded branch never blocks, hides, selects, or deselects a
   Push resource.
7. A provider-recorded branch never blocks, hides, selects, or deselects a
   Pull action.
8. Push comparison IDs, decisions, and recommended selection do not change
   when the checkout changes between local branches.
9. Pull review IDs and actions do not change when the checkout changes between
   local branches.
10. A branch change inside a Push or Pull review keeps the current selection.
11. Every planned conversation is visible in the review, either through the
    Activity projection or the fallback list.
12. Exact project-root ownership still prevents cross-project session capture.
13. Provider bindings still prevent unauthorized local provider mutation.
14. Keep Both and conversation-lineage behavior remain unchanged.
15. All persisted schema constants remain `1`.
16. No migration, compatibility reader, feature flag, or legacy execution path
    exists.

## 13. Non-goals

This change does not:

- make the branch picker change the working tree;
- remove Activity time windows or pagination;
- make sessions visible across different Mallard projects;
- make an unbound provider writable;
- remove provider scopes from the portable manifest;
- change conversation divergence or Keep Both behavior;
- synchronize Git-managed project files through Mallard;
- change the portable schema number or `.mallard/v1` object namespace; or
- migrate existing local or cloud data.

## 14. Implementation record

Completed on 2026-09-04.

- Activity no longer exposes or tests a session branch.
- Each selected local Git branch receives the complete project-session set for
  the loaded time range.
- A session maps by recorded SHA when the SHA is on the selected rail. It
  otherwise maps by time. A session with no successor commit stays visible in
  **Not mapped to this branch**.
- The branch picker lists local Git branches only. Provider metadata cannot add
  a branch.
- Codex and Claude no longer build the unused `recorded_branches` collection.
  Their singular recorded branch remains available as provenance in Session
  Detail and schema-1 portable presentation data.
- Push and Pull planning remain project-wide. A regression test changes the
  checkout between `main` and `test` and proves identical Codex and Claude
  Push comparisons and Pull actions.
- Review row accounting now uses rows rendered by the selected Git projection.
  A planned row that is absent from that projection appears in the fallback
  list and cannot disappear from review.
- Branch changes do not clear a current Push or Pull selection.
- No migration, compatibility reader, feature flag, or legacy branch-ownership
  path was added.
- Local, bundle, and restore-plan schema constants remain `1`.

Verification completed with:

```text
npm run build
npm run test:frontend-integration
cargo test --manifest-path crates/mallard-infrastructure/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:integration
```

All automated checks passed. Tests that explicitly require a real provider
home or a paid/live provider stayed skipped by their existing opt-in gates.

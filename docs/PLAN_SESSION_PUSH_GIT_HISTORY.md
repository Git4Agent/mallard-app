# Session Push Carries Git History

Status: implemented October 6, 2026 (owner picked mark B). Live Tauri pass
against a Mallard Cloud storage not yet run.

Supersedes step 6 of `PLAN_SESSION_DETAIL_PUSH.md` (skip local Git-history
capture; copy the stored history forward).

## Problem

A session Push (Push from a session's own page) never reads local Git. It
copies the Git history storage already holds into the new snapshot and only
rebuilds its session references (`publish_reviewed`, service.rs ~10895;
`bundle_engine.rs` ~1223).

The common sequence is: work in a session, commit, open the session, Push.
The stored history predates that commit, so on Mallard Cloud the session is
placed under **Uncommitted** ("No captured commit follows these sessions'
last prompts"). The desktop's Activity view places the same session on its
commit. The two disagree, and the Cloud's Git history tab stops advancing for
as long as sessions are pushed one at a time. Only a project Push repairs it.

## Decision

On Mallard Cloud, a session Push captures and publishes the current local Git
history exactly as a project Push does. One rule for both scopes:

- the storage must be Mallard Cloud (`capture_binding_git_history` already
  returns `None` for every other provider; unchanged);
- the review carries the history, its review token, and `git_history_outgoing`;
- Push re-captures and refuses when the token no longer matches;
- the published manifest holds the local rail; the engine rebuilds every
  session reference from the final sessions, as today.

What does not change:

- Git history never makes a session Push live. A history-only change stays a
  project Push. The session page's Push reads the session's own decision
  (`sessionPushDisabledReason`), and the backend already refuses a session
  Push whose session produced no action ("This session has no changes to
  push.", service.rs ~11561). Both stay.
- No schema, DTO, or manifest change. Schema 1 throughout.
- Pull never reads history. Unchanged.

### Behaviour

| Case (Git project) | Today | Planned |
| --- | --- | --- |
| Storage is not Mallard Cloud | No history | Unchanged: no history |
| Cloud; stored history older than the session's commit | Session under Uncommitted | Session under its commit; the new commits travel |
| Cloud; no stored history yet | Stays absent | The first history travels with the session |
| Cloud; local branch differs from the stored branch | Stored branch kept | The local rail replaces it (as a project Push) |
| Cloud; local head left the stored rail (amend, rebase, reset) | Stored kept | The local rail replaces it (as a project Push) |
| Cloud; session unchanged, history changed | Push disabled: up to date | Unchanged: disabled; history-only is a project Push |
| Commit between review and Push | Not checked | Refused: `Git history changed after review. Review Push again.` |
| `.git` missing | Session pushes | Refused: `The Git project is unavailable. Restore its repository before Push.` |

The last row is the one new refusal. A project Push to the Cloud already
refuses this way; a session Push to the Cloud joins it.

### Alternatives rejected

- **Keep copying the stored history forward.** The Uncommitted defect above.
- **Advance only.** Take the local history only when it extends the stored
  rail on the same branch; otherwise keep the stored one. A second rule that
  differs from the project Push, invisible to the user, that never publishes
  a first history and never repairs a rewritten rail.
- **Publish only this session's commits.** A manifest holds one history for
  the whole snapshot, and every published session must appear in it exactly
  once (`GitHistorySnapshot::validate`). A slice is not expressible.

## Implementation sequence

Backend, `src-tauri/src/project_sync/service.rs`:

1. `prepare_push_review_with_progress` (~8563): capture for both scopes.
   Replace the `scope == PushScope::Project` guard with an unconditional
   `capture_binding_git_history(...)?`. The token, `git_history_changed`, and
   `git_history_outgoing` compute as today. Add
   `on_phase("capture Git history", …)` so the Activity log times it.
2. `push_bundle_reviewed_with_progress` (~11165): drop
   `git_history_review_token.is_some()` from the session-scope refusal list.
   The token is now expected. Capture at ~11219 for both scopes; the
   `actual_git_token != approval token` refusal stays.
3. `publish_reviewed` (~10887): compute `git_history_review_token` and set
   `request.git_history = git_history` for both scopes. Delete the
   carry-forward branch. `verify_git` already runs the pre-write re-check.
4. `push_bundle_reviewed_with_repository` (~10979, test helper): drop the
   `scope == PushScope::Project` condition so fixtures supply the token for
   a session approval too.
5. `PushApproval::recommended` already copies the review's token. No change.

Frontend: no data-flow change. `PendingPush` already stores `gitHistory`,
`gitHistoryReviewToken`, `gitHistoryChanged`, and `gitHistoryOutgoing` from
the context for both scopes (ProjectSync.tsx ~2126), and `publishPendingPush`
already passes the token to `push_bundle` (~2332). `pushReviewDisabledReason`
with `sessionScope` is unaffected: the session is always selected.

Optional mark in `SessionPushReview` (owner's call; specimen first, per the
house rule). The session review has no commit rail, so the project review's
↑ chip has no commit card to sit on. Candidates:

- **A · None.** History rides along. It is presentation and never gates.
- **B · One pill on the session row.** Reuse
  `.sync-history-branch-state.sync-history-outgoing` after the title: `↑N`
  when `git_history_outgoing.new_commits > 0`, `.history-only` tint when
  `rewritten`; tooltip `N commits new to <storage>'s Git history · <branch>`.
  Risk: on a session row the arrow can read as the session's own count.
- **C · Footer breakdown.** The gauge's breakdown gains `· History ↑N`.
  Tooltip-only; not visible on the row.

Specimen: `mockups/session-push-history-mockup.html` (seven directions, a
standards table). The owner picked **B**.

Docs: add a one-line pointer at the top of `PLAN_SESSION_DETAIL_PUSH.md` to
this plan.

## Tests

Rust, `src-tauri/src/project_sync/session_push_tests.rs` (on the
`StubManagedCloud` fixture via `managed_cloud_gone::with_cloud`):

- Rewrite `session_push_preserves_stored_git_commits_and_maps_the_final_sessions`:
  switch branch and commit **before** the review. Assert the review carries
  `git_history` (4 commits, branch `session-local`), a token, and
  `git_history_outgoing == Some { new_commits: 1, rewritten: false }`. After
  Push, the stored history is the local rail, the restored session (last
  prompt 00:03) sits under `Unrelated local commit` (00:05), not under
  Uncommitted, every session is referenced once, and the manifest validates.
  Other scopes, recipe, and preferences stay unchanged as today.
- Stale token: commit between review and Push; `push_reviewed` fails with
  `Git history changed after review`; storage head, checkpoints, and
  acceptance unchanged.
- History-only still refused: push the session, commit, review it again;
  the entry reads `no_changes`; `push_reviewed` fails with `no changes to
  push`; storage head unchanged.
- `.git` removed: review fails with `Git project is unavailable`; nothing
  published.
- Keep the local-storage test's `review.git_history.is_none()` assertions:
  non-Cloud storages stay history-free.

Frontend, `tests/frontend/push-budget.integration.test.tsx` (mockIPC): the
session review's Push sends `push_bundle` with the review's
`gitHistoryReviewToken` for a `session` scope. Plus the pill's render test if
B ships.

## Safety and schema impact

- No schema, manifest, or DTO change. No migration.
- One check is relaxed: a session approval may carry
  `git_history_review_token`. It is still verified against a fresh capture
  before the write, as a project approval is.
- One check is added: a session Push to Mallard Cloud refuses when the Git
  project is unavailable.
- Immutable publication, claims, digest checks, and acceptance-after-success
  are untouched. Pull is untouched.

## Cost

Review and Push each run one `git log --first-parent -n 10001` plus a few
`rev-parse` calls, read by pinned object id (`capture_git_history`). Expect
tens of milliseconds; a few hundred on a 10,000-commit repository. The new
phase line in the Activity log measures it on the live pass.

## Verification

- `cargo test --manifest-path src-tauri/Cargo.toml --lib` (the included
  `session_push_tests.rs` and `git_history_tests.rs`).
- `npm run build` and `npm run test:frontend-integration`.
- Live Tauri pass against a Mallard Cloud staging storage with temporary
  homes: commit, Push the session from its page, open the Cloud's Git history
  tab and confirm the session sits under that commit. Repeat on a second
  branch to see the rail switch. Record the `capture Git history` phase time.

## What was built

- `service.rs`: review and Push capture the Git history for both scopes
  (`capture Git history` phase in the Activity log); `publish_reviewed`
  publishes the captured history for both scopes, and the carry-forward of
  storage's history is gone; a session approval carries its review token,
  which the Push re-checks as before. The test-only helper supplies the token
  for either scope.
- `SessionPushRow` takes `history` (`SessionPushHistory`: branch and the
  outgoing delta) and draws the rail's ↑ chip after the title, holding the
  count, `.history-only` when rewritten, with `sessionPushHistoryTitle` as
  its tooltip and label. Nothing is drawn when the Push adds no commit or
  storage holds no history this branch shares. `SessionPushReview` takes
  `gitHistory` and `gitHistoryOutgoing`; `ProjectSync` passes them from the
  review context. `App.css` sizes the chip on the title line as on a commit's
  meta line.
- No schema, DTO or manifest change.

## Verification

- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- session_push git_history`:
  7 passed, 1 ignored. The Cloud session tests now assert the local rail is
  published and the session sits under its commit, the first history
  travels to an empty Cloud, a stale token and a missing repository are
  refused, and a history-only change still does not publish from a session.
  Local-folder storages stay history-free.
- `npm run test:frontend-integration`: 595 passed. The new test renders the
  pill, its words, the rewritten tint, and the rule that hides it.
- `npm run build`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 541 passed, 0 failed,
  15 ignored.
- The shipped component drawn in the specimen (Vite, no Tauri): the pill is
  26×21 px at 11 px semibold after the title, the rewritten case takes the
  warning tint and the two-line tooltip, and *Up to date* draws nothing.
- Not run: the live Tauri pass against a Mallard Cloud storage (commit, Push
  the session from its page, confirm the Cloud's Git history tab places it
  under that commit; read the `capture Git history` phase time).

Open: the rewritten state is the warning tint alone, as the rail's chip is
(WCAG 1.4.1); the tooltip carries the words.

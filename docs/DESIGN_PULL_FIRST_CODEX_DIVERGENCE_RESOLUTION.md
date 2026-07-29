# Design: Pull-first Codex divergence resolution

- Status: Approved direction; implementation pending
- Last reviewed: July 29, 2026
- Scope: Codex conversation divergence in `ProjectSync`
- Compatibility: clean cutover; no migration or legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner
  will clear `~/.mallard` and the test cloud buckets before acceptance testing
- Readable companion:
  [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION_READABLE.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION_READABLE.md)

## Decision

For the same Mallard logical conversation, when local and cloud histories have
both advanced from a shared prefix:

1. Push is blocked and directs the user to Pull.
2. Pull offers:
   - **Keep both branches (Recommended)**: create a Codex-native fork that
     preserves the reviewed local branch, then restore the cloud branch into
     the original local Codex thread.
   - **Use cloud version only**: replace the original local thread with the
     cloud branch without preserving the divergent local branch as a
     resumable Codex conversation.
3. The native fork executor uses this order:
   1. app-server `thread/fork` over stdio;
   2. a bounded PTY wrapper around `codex fork`; and
   3. a verified user-performed fork.
4. Every executor feeds the same verification and transaction. A fallback
   never weakens the content, ancestry, or stale-state checks.
5. Mallard gives the preserved local fork a deterministic, human-readable
   name and restores the cloud title onto the original thread when one is
   available. Naming is presentation metadata, never thread identity.
6. After the Pull transaction commits, Mallard may offer an explicit,
   optional Codex App restart. Restart is a presentation refresh, not part of
   transaction correctness.
7. Pull never changes cloud state.

Mallard does not synthesize a Codex fork by copying rollout files, use private
Codex App IPC, or navigate to a fork with a `codex://threads/<id>` deep link.

## Goals

- Preserve both divergent branches by default.
- Keep the original provider thread ID associated with the cloud logical
  thread.
- Use Codex-native fork provenance for the preserved local branch.
- Give the preserved local branch a distinct, predictable Codex title.
- Prevent Push from publishing a non-fast-forward conflict.
- Make fork creation and original-thread replacement crash recoverable.
- Support Codex App, CLI-only installations, macOS, Linux, and Windows through
  explicit capability checks.
- Keep provider IDs, paths, executable details, and recovery state local.

## Non-goals

- Merge divergent conversation messages.
- Automatically choose the destructive cloud-only resolution.
- Guarantee immediate Codex App sidebar placement or project grouping.
- Use titles to identify, match, or recover threads.
- Focus or open the forked thread.
- Support old Mallard plans, journals, or state after the cutover.
- Change bundle schema 1.

## Identity and required result

The implementation must keep Mallard and Codex identities separate:

- `T`: the existing Mallard logical `ThreadId`.
- `t`: the existing Codex provider thread ID bound to `T`.
- `F`: a new Mallard logical `ThreadId` for the preserved local branch.
- `t'`: the Codex-native child created from `t`.

Starting state:

```text
local:  T -> t = (m1, m2, m3)
cloud:  T      = (m1, m2, m4)
```

Recommended Pull result:

```text
local:
  T -> t  = (m1, m2, m4)
  F -> t' = (m1, m2, m3)
  F.forked_from = T

cloud:
  T       = (m1, m2, m4)
```

Cloud-only result:

```text
local:
  T -> t  = (m1, m2, m4)

cloud:
  T       = (m1, m2, m4)
```

The replacement backup under `~/.mallard` remains private recovery material.
It is not inventoried as a Codex conversation.

## Current behavior and cutover

The current implementation maps `Diverged` to `Fork` for Push and Pull.

- Push publishes the local branch under a new logical ID and rekeys the local
  binding.
- Pull rekeys the existing local branch and materializes the cloud branch
  under a newly generated provider ID.

The new behavior removes both paths:

- Push `Diverged` becomes non-selectable `RequirePull`.
- Push `Unknown` becomes `Blocked`.
- Pull `Diverged` becomes `ResolveDivergence`.
- Recommended Pull creates `F -> t'` and keeps `T -> t`.
- Pull no longer generates a new provider ID for the cloud branch when `t`
  already exists.

The fork-safe rollout parser, lineage codec, immutable cloud publication,
target pinning, and binding model from
`DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md` remain in force.

### Clean-cutover assumptions

There are no deployed users or existing Mallard records that this change must
preserve. Before the next manual acceptance run, the owner will clear
`~/.mallard` and the test cloud buckets. Implementation therefore targets only
the new representation and behavior:

- do not migrate old machine-local plans, journals, receipts, or bindings;
- do not decode or recover old conflict-operation shapes;
- do not keep old Push-side fork or Pull rekey/materialization executors;
- do not add compatibility defaults, dual readers, dual writers, feature
  flags, or fallback behavior for the removed design; and
- reject unsupported stale state instead of attempting an in-place upgrade.

The application must not add an automatic wipe command as part of this
feature. Clearing real state is an explicit operator action outside the Pull
transaction. Automated tests continue to use temporary Mallard homes and
disposable cloud fixtures; they must never depend on or modify the real
`~/.mallard`, `~/.codex`, or production cloud storage.

## Directional policy

| Relation | Push | Pull |
| --- | --- | --- |
| `Identical` | `NoOp` | `NoOp` |
| `CloudPrefixOfLocal` | `FastForwardCloud` | `KeepLocal` |
| `LocalPrefixOfCloud` | `RequirePull` | `FastForwardLocal` |
| `Diverged` | `RequirePull` | `ResolveDivergence` |
| `Unknown` | `Blocked` | `StoreOnly` |
| `LocalMissing` | `NoOp` | `CreateLocal` |
| `CloudMissing` | `CreateCloud` | `KeepLocal` |
| `BothMissing` | `NoOp` | `NoOp` |

After recommended Pull, `T` is `Identical` and `F` is `CloudMissing`. A later,
separately reviewed Push may publish `F` through the ordinary `CreateCloud`
path. Pull never publishes it implicitly.

## Pull review contract

`ResolveDivergence` has exactly two resolution options:

| Resolution | Default | Approval | Result |
| --- | --- | --- | --- |
| `PreserveLocalWithNativeFork` | yes | ordinary Apply approval | preserve local as `F -> t'`; restore cloud as `T -> t` |
| `UseCloudOnly` | no | explicit destructive confirmation | restore cloud as `T -> t`; no visible local fork |

The selected resolution belongs to the reviewed plan action. Fork executor
selection does not: stdio, PTY, and manual are ordered implementations of the
same recommended resolution.

The plan also carries the pinned local/cloud titles, generated fork name, and
optional desired original-thread name. These values are review output, not
additional resolution choices.

The Apply request must carry one typed resolution per selected divergence
action. The backend rejects:

- missing or duplicate selections;
- a resolution attached to a non-divergence action;
- a resolution not advertised by the plan;
- cloud-only without explicit destructive approval; and
- any attempt to substitute cloud-only after a fork failure.

The native-fork option is blocked only when none of the three executor paths
can be used safely. A blocked recommended option never preselects cloud-only.

## Pull review UI

The divergent conversation remains one review row:

```text
Conversation conflict

● Keep both branches (Recommended)
  Preserve the current local conversation as a Codex fork, then restore the
  cloud conversation into the original thread.

○ Use cloud version only
  Replace the original local conversation. The divergent local branch will
  not remain as a Codex conversation.
```

The details panel shows the executor order:

```text
Fork method: app-server → PTY helper → manual
Fork name:   Analyze codebase knowledge graph — local before Pull ·
             2026-07-29 18:42Z
Original:    Analyze codebase knowledge graph
```

The names are a read-only preview in the first implementation. Users may
rename either thread in Codex afterward.

Apply progress distinguishes:

- probing app-server;
- creating the fork through app-server;
- trying the PTY helper after a definitive app-server failure;
- waiting for a manual fork;
- verifying the fork;
- applying or deferring the reviewed names;
- restoring the original thread; and
- complete, with the successful method, both provider thread IDs, and naming
  results.

At a manual checkpoint, the UI shows the pinned source thread ID, suggested
fork name, and **Verify fork** and **Cancel** actions. It does not expose the
cloud-only action as a fallback button.

Push renders divergence as non-selectable:

```text
Pull required
Local and stored history have diverged. Pull and choose how to preserve the
local branch before pushing.
```

## Native fork coordinator

### Common contract

Each executor receives:

- the canonical Codex executable;
- the exact reviewed `CODEX_HOME`;
- the source provider thread ID `t`;
- the reviewed project directory;
- the source rollout fingerprint;
- the pinned source-title expectation and desired fork name;
- the bounded set of pre-existing native children of `t`; and
- bounded time and output limits.

Each executor returns either:

- a candidate provider thread ID;
- a definitive no-fork result that permits the next executor;
- an explicit manual-wait state; or
- an ambiguous result that requires recovery.

The coordinator never accepts a title, timestamp, or "most recent thread" as
identity proof.

### Preconditions

Before planning and again before execution, Mallard verifies:

- the local and cloud relation is still `Diverged`;
- the source and cloud transcripts are safely readable and resumable;
- the binding `T -> t` is unique;
- the local explicit title still matches its reviewed expectation;
- the source is not in an active turn;
- the source rollout is stable and matches its pinned digest;
- the original target can still be replaced safely;
- the inventory of existing native children is bounded and complete;
- the selected executable and helper artifacts match their pinned
  fingerprints; and
- no known Codex process is actively mutating `t`.

The current `provider_quiescent: true` placeholder is not sufficient. The
implementation must query provider status where supported, require the user to
finish active App or CLI turns, and recheck the source immediately before and
after fork creation. If the source is active or unstable, Apply stops before
replacement.

### Preference 1: app-server over stdio

Mallard starts the pinned CLI as:

```text
codex app-server --stdio
```

It performs the required JSONL handshake:

1. `initialize` with Mallard client name, title, and version;
2. `initialized`; and
3. `thread/fork`.

The fork request uses the documented minimum:

```json
{
  "method": "thread/fork",
  "id": 2,
  "params": {
    "threadId": "t"
  }
}
```

Mallard does not pass `lastTurnId`, submit a prompt, start a turn, change the
model, or alter sandbox and approval settings. The source must be idle because
Codex documents that forking a mid-turn thread can record an interruption
marker.

The client:

- probes the installed app-server schema and initialization behavior;
- requires `thread/fork` for the executor and records `thread/name/set` as a
  separate presentation capability;
- matches responses by request ID;
- bounds JSONL lines, notifications, stdout, stderr, and total runtime;
- reads the returned thread ID and checks `forkedFromId` when present;
- closes stdin and reaps the Mallard-owned server before provider rewrite; and
- treats any post-request timeout or malformed completion as ambiguous.

App-server is currently an experimental CLI surface, so runtime probing is
required even though it is the preferred machine integration.

### Preference 2: PTY-wrapped `codex fork`

PTY is tried only after app-server has definitively failed without creating a
fork.

The helper drives:

```text
codex fork --no-alt-screen -C <project-directory> <source-thread-id>
```

The PTY contract requires the helper to:

- use the exact pinned CLI and `CODEX_HOME`;
- verify Codex authentication before launch;
- snapshot existing native children before launch;
- suppress and discard TUI transcript output;
- never type or submit a prompt;
- wait for exactly one new persisted native child;
- exit the idle TUI, then kill and reap it only if bounded graceful exit fails;
- emit one bounded machine-readable result record; and
- return ambiguous if it cannot prove whether a fork was created.

`scripts/codex-fork-and-exit` is the current macOS/Linux prototype. Production
packaging must ship a pinned helper artifact rather than depend on a source
checkout. Native Windows support requires a tested ConPTY implementation with
the same contract. Until that exists, PTY is unavailable on native Windows and
the coordinator proceeds to manual fork after a definitive app-server
failure. It must not route a native Windows profile through WSL.

### Preference 3: verified manual fork

Manual fork is the final path:

1. Mallard persists `AwaitingManualFork` before asking the user to act.
2. The UI asks the user to fork `t` in Codex App or CLI without adding a
   message and shows the suggested fork name.
3. The user supplies `t'`, or asks Mallard to perform one bounded child scan.
4. Mallard verifies `t'` through the common verifier.
5. Mallard revalidates every plan pin before continuing.

Canceling the checkpoint leaves `t` unchanged. Mallard does not infer success
from a new title or from whichever conversation is currently focused. If the
user already gave the manual fork a name different from the inherited source
title, Mallard preserves it as an explicit user override.

### Fallback rules

| Previous executor outcome | Next action |
| --- | --- |
| Failed before invocation; no new child | try the next executor |
| Explicit unsupported/rejected response; complete scan proves no new child | try the next executor |
| Request or PTY may have run; result uncertain | stop and recover; do not fall through |
| One exact new child | adopt and verify it |
| One changed child or multiple new children | `RecoveryRequired` |

These rules prevent duplicate forks when Codex created `t'` but Mallard lost
the response.

## Thread naming

### Resulting names

Mallard plans both local names during Pull review:

| Thread | Naming rule |
| --- | --- |
| Preserved local fork `t'` | `<local base title> — local before Pull · <plan time in UTC>` |
| Restored original `t` | cloud `display_title` when non-empty; otherwise retain the pre-Pull title |

Example:

```text
t  = Analyze codebase knowledge graph
t' = Analyze codebase knowledge graph — local before Pull ·
     2026-07-29 18:42Z
```

The fork base title uses the first non-empty value:

1. the explicit local Codex thread name captured at review;
2. the cloud conversation `display_title`;
3. a bounded summary of the first local user message; or
4. `Untitled conversation`.

A provider thread ID is not a usable base title.

The desired fork name is generated once from the plan creation time and stored
in the reviewed plan. Recovery reuses it instead of generating a new timestamp.
Normalization:

- trim leading and trailing whitespace;
- collapse internal whitespace runs to one space;
- reject control characters;
- limit the complete name to 180 Unicode scalar values;
- truncate only the base, adding an ellipsis when needed, so the full suffix
  remains visible; and
- never include a machine name, project path, provider ID, logical ID, storage
  name, or cloud location.

Names need not be unique. Codex and Mallard thread IDs remain authoritative;
titles are never used for binding, matching, candidate discovery, or recovery.

### Applying names

After `t'` passes fork verification, Mallard uses app-server
`thread/name/set`:

```json
{
  "method": "thread/name/set",
  "id": 3,
  "params": {
    "threadId": "t'",
    "name": "Analyze codebase knowledge graph — local before Pull · 2026-07-29 18:42Z"
  }
}
```

Codex supports this operation for a loaded thread or persisted rollout and
emits `thread/name/updated`. `thread/fork` itself may return a null or missing
name, so fork creation is not treated as name application.

- For a stdio fork, Mallard names `t'` on the same app-server connection after
  verification and before closing the connection.
- For a PTY fork, Mallard uses a short-lived app-server naming request after
  the PTY child has exited. It does not add `/rename` keystrokes to the PTY
  automation.
- For a manual fork, an explicit name chosen by the user wins. Otherwise,
  Mallard applies the planned name when `thread/name/set` is available and
  shows the suggested name for manual use when it is not.
- After provider replacement and scan-and-repair, Mallard applies the non-empty
  cloud `display_title` to `t`. When cloud has no title, Mallard does not clear
  or synthesize an original-thread name.
- Cloud-only applies the same original-thread title rule but creates no fork
  name.

Mallard never edits `session_index.jsonl` or Codex SQLite solely to set a name.

### Failure and recovery

Naming is presentation metadata. A naming failure does not invalidate an
exact native fork, roll back a safe replacement, or select cloud-only. The
receipt reports one status for each applicable name:

- `applied`;
- `user_override`;
- `unchanged`;
- `deferred`; or
- `failed`.

For `deferred` or `failed`, the receipt includes the target provider thread ID
and suggested name without exposing paths or conversation content.

The plan and journal pin:

- the local title observed at review;
- the cloud title observed at review;
- the generated fork name;
- the optional desired original-thread name; and
- name attempts and final statuses.

A local title change before fork creation makes the plan stale instead of
overwriting a recent user rename. `thread/name/set` is retried idempotently
only while the current title is still the pinned inherited title, is absent,
or already equals the desired title. A different title discovered after a
manual checkpoint or a previously verified naming step is preserved as a
user override.

## App visibility and optional restart

A Mallard-owned app-server process is separate from a running Codex App
process. The Pull result is therefore correct at the provider level without
assuming that the App immediately refreshes its sidebar or inherits the source
thread's App-specific project grouping.

Mallard does not:

- connect to private App IPC;
- use undocumented shared-daemon switches;
- open `codex://threads/<id>`; or
- move App focus to make the thread visible.

After `LocalCommitted`:

| App state | Behavior |
| --- | --- |
| Not installed | no App action |
| Installed but not running | no restart; allow discovery on next launch |
| Running | offer an explicit optional restart |
| Restart declined or unavailable | keep the successful Pull result and report refresh deferred |

Before restart, the user must confirm that active chats are complete. A
platform restart adapter may gracefully close and relaunch only the exact
installed App. It never force-kills an unknown process and never launches a
thread deep link. If no safe adapter is available, Mallard shows manual
restart instructions.

Restart failure is a presentation warning. It never rolls back or invalidates
the completed Pull. The receipt suggests checking the App's chronological
conversation view when the fork is not shown under the expected project.

## Recommended Pull transaction

The cloud replacement is staged before fork creation, but `t` remains
untouched until `t'` is verified.

```text
1. Revalidate the reviewed plan and all local/cloud pins.
2. Persist the operation journal.
3. Stage the cloud transcript rewritten for provider ID t and local cwd.
4. Snapshot the existing native children of t.
5. Create t' through the ordered fork coordinator.
6. Parse and verify t' against the reviewed local branch.
7. Apply or defer the planned name for t'.
8. Atomically add binding F -> t', with F.forked_from = T.
9. Back up t and atomically replace it with the staged cloud branch.
10. Verify t, then refresh binding T -> t with the cloud pins.
11. Request a bounded Codex inventory scan-and-repair.
12. Apply or defer the cloud display title for t.
13. Persist the receipt and LocalCommitted state.
14. Optionally offer an App restart.
```

Cloud is not written during any step.

### Fork verification

Before replacing `t`, Mallard proves:

- `t'` is valid, different from `t`, and not already bound;
- the result is a new child not present in the pre-fork inventory;
- the result path is a canonical regular file inside the selected
  `CODEX_HOME/sessions` or `CODEX_HOME/archived_sessions` tree;
- its authoritative provider identity is `t'`;
- its immediate `forked_from_id` is `t`;
- its canonical message IDs, order, lineage digest, and semantic digest equal
  the reviewed local branch;
- no user message, model turn, or interruption marker was added;
- `t` still matches its reviewed digest; and
- the provider index delta contains no unrelated removal or rewrite.

When an executor returns `forkedFromId`, that value must also equal `t`.
Rollout structure remains the authoritative common proof across all three
executors.

### Original-thread replacement

The cloud transcript is structurally rewritten only for:

- active provider ID `t`; and
- the reviewed destination project path.

Existing staging, no-follow validation, target expectations, backup, atomic
rename, and directory-sync behavior remain mandatory. The binding `T -> t` is
updated only after the committed target matches the cloud lineage and semantic
digests.

Mallard does not edit Codex's SQLite state directly. A bounded app-server
`thread/list` call with `useStateDbOnly: false` and source filters covering the
affected thread is a best-effort derived-index refresh after material content
is safe. Failure becomes a retryable warning, not a rollback.

## Cloud-only transaction

The explicitly approved cloud-only path:

```text
1. Revalidate all replacement pins.
2. Stage the cloud transcript for provider ID t.
3. Back up and atomically replace t.
4. Verify t and refresh binding T -> t.
5. Request provider inventory scan-and-repair.
6. Apply or defer the non-empty cloud display title for t.
7. Persist the receipt and LocalCommitted state.
```

It never creates `F` or `t'`. Provider quiescence and target safety checks are
identical to the replacement half of the recommended path.

## Journal and recovery

The current journal fields describe the old rekey/materialize flow and should
be replaced with a tagged operation for the new transaction.

The preserving-local operation records at least:

- `T`, pre-minted `F`, and source provider ID `t`;
- optional verified result `t'`;
- ordered executor capabilities and pinned artifact fingerprints;
- pre-existing native-child fingerprints;
- every executor attempt and whether it may have created a thread;
- local/cloud title expectations, desired names, name attempts, and final
  naming statuses;
- cloud, source, target, profile, project, and binding pins; and
- the current phase.

Required phases:

```text
Prepared
→ ReplacementStaged
→ ForkAttemptPending(executor)
  ├─ definitive no-fork → next executor
  ├─ manual → AwaitingManualFork
  └─ candidate → ForkVerified
→ ForkNamingRecorded
→ LocalForkBound
→ ProviderCommitted
→ BindingsCommitted
→ OriginalNamingRecorded
→ LocalCommitted
```

The two naming phases record either success, user override, unchanged,
deferred, or failure; they do not require a successful rename to advance.
`RecoveryRequired` is terminal until a safe continuation is proven.

### Recovery rules

After a crash or ambiguous executor result, Mallard scans for children of `t`
that were absent from the pinned pre-fork inventory:

| Candidates | Recovery |
| --- | --- |
| none, executor never ran | revalidate and continue with the next executor |
| none, executor may have run | perform one bounded rescan, then require recovery |
| exactly one exact child | adopt it as `t'` and continue from `ForkVerified` |
| one non-exact child | require recovery; leave `t` untouched |
| multiple children | require recovery; never guess |

Mallard never retries or switches executors after a possibly successful
attempt, and never automatically deletes an uncertain Codex thread.

Once `ForkVerified` is durable, recovery must not create another fork:

- before `ForkVerified`, `t` is unchanged;
- after `ForkVerified`, the local branch exists as both `t` and `t'`;
- after `ForkNamingRecorded`, recovery reuses the stored name result and never
  generates a new suffix;
- after `LocalForkBound`, `F -> t'` is durable;
- after `ProviderCommitted`, verify `t` and repair `T -> t`;
- after `BindingsCommitted`, apply or defer the pinned original-thread name;
- after `OriginalNamingRecorded`, write the receipt and finish cleanup.

## Data and schema impact

### Portable bundle

Bundle schema 1 does not change:

- Pull leaves cloud `T` unchanged;
- a later Push of `F` uses the existing `forked_from = T` and
  `display_title` fields;
- naming `t` or `t'` during Pull remains local until a later reviewed Push;
  and
- provider IDs, paths, executables, attempts, backups, and journals remain
  local.

### Machine-local state

The plan, selection, receipt, and journal representations change
incompatibly. This is intentional under the clean-cutover assumptions above.
Keep one current machine-state schema and replace the old representation:

- no old-plan adapter;
- no old-journal recovery;
- no serde defaults added only for compatibility;
- no dual writer; and
- no migration.

Manual acceptance testing starts only after the owner has cleared
`~/.mallard` and the designated test cloud buckets. Automated tests use
isolated temporary state instead.

## Code-change evaluation

| Area | Required change | Risk |
| --- | --- | --- |
| `conversation_sync.rs` | map Push divergence to `RequirePull`, Push unknown to `Blocked`, and Pull divergence to `ResolveDivergence` | Low |
| `domain.rs` | add typed resolution selections, planned names/statuses, and replace the old journal shape | Medium |
| new `codex_native_fork.rs` | coordinate stdio, PTY, manual wait, verification, `thread/name/set`, and ambiguous-result recovery | High |
| PTY helper packaging | harden the POSIX prototype and add a tested native Windows ConPTY implementation before enabling that capability | High |
| `commands.rs` | build options, title expectations, desired names, and pins; execute or pause the transaction; report executor and naming results; remove divergent Push publication | High |
| `conversation_materialization.rs` | replace rekey-first divergence handling with fork-name-bind-replace recovery | High |
| `persistence.rs` | validate only the new local operation representation | Medium |
| `src/types.ts` and `api.ts` | mirror the new Rust DTOs and Apply request | Medium |
| Pull review model and flow | store one resolution per divergence row and support manual resume | Medium |
| `RestorePlanView.tsx` | render resolution radios, name previews/results, executor progress, manual checkpoint, and optional restart | Medium |
| Push/history UI | render divergence as Pull-required and non-selectable | Low |
| `lib.rs` | register any new resume, cancel, or restart commands | Low |

Remove rather than retain:

- Push `Diverged -> Fork`;
- Push `Unknown -> Fork`;
- divergent Push publication and binding rekey;
- Pull's pre-generated provider ID for the cloud half;
- Pull's `T -> F` rekey-first path;
- old composite-fork journal fields and recovery;
- UI copy that offers "Publish as new thread" for divergence; and
- tests that encode the removed behavior.

## Test strategy

Tests must use disposable provider and Mallard homes. They must never point to
a user's configured `~/.codex`, `~/.mallard`, or real cloud bucket.

### Policy and contract

- Push divergence is `RequirePull` and non-selectable.
- Push unknown is `Blocked`.
- Pull divergence advertises exactly the two resolution options.
- keep-both is the default; cloud-only requires destructive approval.
- a failed native fork never selects cloud-only.
- invalid, duplicate, or stale selections are rejected.

### Executor ordering

- stdio success does not invoke PTY or manual.
- definitive stdio no-fork invokes PTY.
- definitive PTY no-fork enters manual wait.
- ambiguous stdio or PTY never invokes the next executor.
- manual resume rejects missing, duplicate, changed, or unrelated children.
- a changed CLI or helper fingerprint makes the plan stale.
- native Windows does not invoke the POSIX helper.

### Naming

- base-title priority uses local explicit name, cloud title, local summary,
  then `Untitled conversation`;
- a provider ID is rejected as a base title;
- plan time produces one stable UTC suffix across Apply and recovery;
- normalization collapses whitespace, rejects controls, and truncates the base
  without removing the suffix;
- complete names never exceed 180 Unicode scalar values;
- stdio naming uses the existing app-server connection;
- PTY and automatic manual-result naming use a short-lived app-server rather
  than TUI keystrokes or direct provider-index edits;
- an explicit manual name is preserved as `user_override`;
- absent cloud title leaves the original title unchanged;
- non-empty cloud title is applied to the restored original thread;
- deferred or failed naming is reported without rolling back safe content;
- recovery retries only an idempotent, non-conflicting name operation; and
- titles are never used to choose or recover a fork candidate.

### Safety and recovery

- source active before or during fork;
- source changes before or after fork;
- cloud head, profile, project, or binding revision changes;
- app-server rejection, timeout, oversized output, malformed JSON, or wrong
  request ID;
- PTY early exit, timeout, oversized output, unexpected input, or unreaped
  child;
- crash before invocation, after Codex creates `t'`, after verification, after
  each binding update, after provider replacement, and during cleanup;
- wrong result path, symlink, special file, provider identity, or native
  parent;
- changed lineage or semantics;
- one ambiguous candidate and multiple ambiguous candidates; and
- derived-index refresh failure after material content is safe.

Every failure before `ProviderCommitted` leaves `t` unchanged.

### End-to-end result

Starting with:

```text
T -> t local = (m1, m2, m3)
T cloud      = (m1, m2, m4)
```

Recommended Pull must produce exactly:

```text
T -> t  = (m1, m2, m4)
F -> t' = (m1, m2, m3)
cloud T = (m1, m2, m4)
```

Also prove:

- `t' != t`;
- `t'.forked_from_id = t`;
- `F.forked_from = T`;
- `t'` has the planned local-before-Pull name or a recorded manual override;
- `t` has the non-empty cloud display title, or retains its prior title when
  cloud has none;
- cloud objects and head are unchanged;
- the next comparison is `T = Identical`, `F = CloudMissing`;
- the next selected Push publishes only `F`; and
- repeated scan or recovery does not create another fork.

Cloud-only must produce only `T -> t = cloud`, retain the private backup, and
make the next Push a no-op.

### Real Codex contract tests

Keep opt-in smoke tests outside the default suite:

- app-server `thread/fork` against a disposable `CODEX_HOME`;
- PTY helper against the same kind of disposable fixture on each advertised
  platform; and
- a release checklist for the manual path and optional App restart.

Each automated smoke verifies exact ID, parent, content, persisted name,
process exit, and cleanup of only the disposable directory.

## Implementation order

1. Change the pure directional policy and its tests.
2. Add typed Pull resolution selections and DTO validation.
3. Implement deterministic name planning, the shared fork verifier, and the
   journal shape.
4. Implement and test stdio app-server fork and `thread/name/set`.
5. Harden and package the PTY executor; keep unsupported platforms disabled.
6. Add the durable manual checkpoint and resume/cancel flow.
7. Implement fork-name-bind-replace and recovery.
8. Delete old Push fork and Pull rekey/materialize behavior; do not retain
   compatibility branches.
9. Update Pull, Push, naming receipt, and optional restart UI.
10. Run Rust, frontend, and disposable Codex contract tests.

## Acceptance criteria

1. Divergence blocks Push and directs the user to Pull.
2. Pull defaults to preserving the local branch with a Codex-native fork.
3. Fork execution tries stdio, then PTY, then verified manual fork.
4. A fallback occurs only after the previous executor is proven not to have
   created a thread.
5. The original provider ID `t` contains the cloud branch after Pull.
6. `t'` contains the exact reviewed local branch and has immediate native
   parent `t`.
7. Mallard binds `T -> t` and `F -> t'`, with `F.forked_from = T`.
8. Mallard applies the stable local-before-Pull name to `t'`, preserves an
   explicit manual override, or reports the exact deferred name.
9. Mallard applies a non-empty cloud title to `t`; an absent cloud title does
   not clear or synthesize one.
10. Pull leaves cloud unchanged.
11. Cloud-only requires explicit destructive approval and creates no visible
   fork.
12. Recovery never guesses, overwrites before exact fork verification, or
    creates a second fork after an ambiguous attempt.
13. Optional App restart is offered only after `LocalCommitted`, requires
    explicit approval, and cannot change the Pull result.
14. No migration, compatibility reader/writer, old-plan adapter, feature flag,
    or legacy divergence executor remains.
15. Unsupported stale local state is rejected rather than migrated, and no
    production code automatically clears Mallard or cloud state.

## Codex references

- App-server protocol, including `thread/fork` and `thread/name/set`:
  <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>
- CLI commands, including `codex app-server` and `codex fork`:
  <https://learn.chatgpt.com/docs/developer-commands?surface=cli>
- Codex App deep links:
  <https://developers.openai.com/codex/app/commands#deep-links>
- Codex App troubleshooting and restart guidance:
  <https://learn.chatgpt.com/docs/reference/troubleshooting>

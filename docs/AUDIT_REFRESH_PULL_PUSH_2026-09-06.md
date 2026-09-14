**Mallard refresh, Pull, and Push audit — 2026-09-06**

The first fix is to separate Push selection from stored-content retention.
The current path captures already-synced sessions and can reject a small
selection at the aggregate capture limit. Two cache defects also cause
repeated reads. Reduce this repeated work before increasing worker counts.

Scope: the current working tree, including pending changes. This is a code
audit with isolated fixture checks. Application code was not changed. No real
provider home, user session, remote storage, or machine state was used in a
test. File positions refer to the code inspected on this date.

| Priority | Finding | Evidence |
| --- | --- | --- |
| P1 | Push captures retained sessions that the user did not select | Frontend helper and backend capture fixtures |
| P2 | Activity cache removes other projects' entries | Production activity-cache fixture |
| P2 | Pull discards staged payloads between its apply stages | Production object-source fixture and call trace |
| P2 | Activity refresh reads the remote snapshot twice in sequence | Code trace |
| P2 | Full refresh bypasses the inventory cache | Code trace |
| P2 | Push checks a blocking destination after capture | Code trace |

1. **P1 — Use an explicit capture selection for Push.**

   `preserveSyncedPushRecipeEntries` adds synced sessions to the submitted
   recipe. `pushBudgetTotal` counts only the visible selection.
   `capture_recipe_from_discovery` then captures every recipe entry.

   Fixture: five retained sessions of 96 MiB each and one selected session of
   40 MiB. The UI helper reports 40 MiB. The submitted recipe contains six
   sessions and 520 MiB. Production capture preflight rejects this recipe at
   its 512 MiB limit. Sparse fixture files tested the size gate without
   parsing large transcripts. The frontend and backend checks were separate
   fixtures, not a complete desktop Push run.

   Below the limit, the same path still reads and analyzes unchanged session
   bytes. This makes a small Push cost depend on retained local history.

   Keep the selected capture IDs separate from retained remote entries.
   Preserve unselected cloud content through the existing retained-file
   publication path. Use the same capture set for the UI budget and backend
   preflight. Revalidate selected sources during apply.

   References: `src/components/project-sync/PushResourceWorkspace.tsx:291`
   and `:369`; `src/components/project-sync/ProjectSync.tsx:2059`;
   `crates/mallard-infrastructure/src/provider_capture.rs:890` and `:719`;
   `src-tauri/src/project_sync/service.rs:9207`.

2. **P2 — Correct activity-cache ownership.**

   `scan_local_session_views` retains entries seen in the current project.
   Its `key_belongs_to_project` helper identifies ownership by provider and
   provider home only. Two projects bound to the same home therefore remove
   each other's unseen entries.

   Fixture: read project A, read project B, then read A again. The cache
   contains only B after the second call. The third call removes B and
   rebuilds A. Neither source file changed. A cache miss calls the provider
   summary reader; the Claude reader reads the complete bounded transcript.

   Track the project when pruning project-owned entries, or prune a shared
   file cache only from a complete provider-home inventory. A project subset
   cannot prove that another project's file is absent.

   References: `crates/mallard-infrastructure/src/project_activity.rs:442`,
   `:489`, and `:515`;
   `crates/mallard-infrastructure/src/providers/claude/history.rs:109`.

3. **P2 — Keep one payload reader through Pull apply.**

   `build_restore_plan_for_apply` creates an `EngineByteSource`. Its
   `PullPlan` pass prefetches selected, unclaimed cloud transcripts. That
   reader is dropped when plan construction returns. Apply creates another
   reader before `prepare_pull_conversation_actions` fetches the selected
   payloads. Only the process cache connects these stages. Its limit is
   256 MiB.

   Fixture: two successive production `EngineByteSource` instances share the
   production cache and read three 96 MiB objects. The first makes three
   object reads. The second makes one extra read. Thus 288 MiB of selected
   content causes 384 MiB of verified object reads. This proves the cache
   lifetime cost with a local counting store; it is not an R2 timing result.

   Pass one reader, or its verified bodies, through both stages. Keep the
   selected bodies alive until consumption. The global cache can remain an
   optional optimization for separate operations.

   References: `src-tauri/src/project_sync/service.rs:9687`, `:4894`, and
   `:11213`; `crates/mallard-infrastructure/src/bundle_engine.rs:125` and
   `:305`.

4. **P2 — Share remote reads during activity refresh.**

   `ProjectActivityPage.loadWorkspace` awaits the activity result, then starts
   the comparison. Both backend paths inspect the remote head and manifest.
   The activity path waits for remote inspection before building its local
   result. Slow or unavailable storage therefore delays local history as
   well as the comparison. A branch change or history-page request also
   fetches the snapshot through the activity path.

   Keep local history available while remote work runs. Use one remote
   snapshot for remote rows and comparison during a refresh. Preserve the
   final head and revision checks at apply.

   References: `src/components/project-sync/ProjectActivityPage.tsx:2238`;
   `src-tauri/src/project_sync/service.rs:1445` and `:6214`;
   `crates/mallard-infrastructure/src/project_activity.rs:574`;
   `crates/mallard-infrastructure/src/bundle_engine.rs:1119`.

5. **P2 — Route full refresh through the existing discovery service.**

   Full refresh loads the shell, loads project inventory, then increments
   the activity refresh epoch. `get_bundle_inventory_with_repository` calls
   `discover_full` directly. It neither uses nor fills the watcher inventory
   used by Activity and comparison.

   A warm full refresh still walks provider files. A cold full refresh can
   walk them again when Activity fills its cache. The current comparison
   test does not cover this complete UI path.

   Use the existing discovery result for inventory and Activity. Limit shell
   recovery and setup reads to the operations that need them.

   References: `src/components/project-sync/ProjectSync.tsx:1160`;
   `src-tauri/src/project_sync/service.rs:2841` and `:2863`;
   `src-tauri/src/project_sync/session_watch_service.rs:710`.

6. **P2 — Check the Push destination before reading payloads.**

   Push already has an observed remote snapshot, but it captures and analyzes
   the recipe before `classify_push_destination` rejects a changed, missing,
   or replaced history. If the destination changed after review, this work
   cannot produce a successful Push.

   Move this check immediately after remote inspection. Keep the final
   scope checks and compare-and-swap during publication.

   Reference: `src-tauri/src/project_sync/service.rs:9207`.

**Further simplification and scaling work**

The main complexity is overlapping stage ownership. A recipe represents both
current selection and retention. A global cache passes payloads between
stages of one apply. These are direct causes of findings 1 and 3. Give each
operation one explicit selection and ownership of its prepared data.

The stored-subagent check also runs a full Codex transcript analysis to keep
one boolean in a separate cache. The Pull preparation worker later verifies
the transcript again. Return this flag from the required provider verification
and reject it before installation. Keep the subagent exclusion rule.
Reference: `src-tauri/src/project_sync/service.rs:1009`.

`provider_thread_review` spans about 1,500 lines. It combines state loads,
provider-specific seeding, observation workers, cache updates, classification,
and executable action construction. Provider-specific work should live in
the existing provider adapters. Keep operation sequencing in the service and
pure decisions in the domain. This is a focused module-boundary change; it
does not require a new registry or framework.

Pull persistence is a separate scaling risk. Each prepared session reloads
machine state under the persistence lock and rewrites the complete
conversation-bindings, config, and materializations documents. For K applied
sessions and N stored records, metadata work can grow as K × N. This is a
static cost finding, not a measured latency result. Measure bytes serialized,
state-load time, and lock duration before choosing a persistence redesign.
Preserve independent failure receipts and revision checks. References:
`src-tauri/src/project_sync/service.rs:11409`;
`crates/mallard-infrastructure/src/persistence.rs:911` and `:259`.

Retain the existing integrity checks, review approval, immutable publication,
compare-and-swap, bounded workers, and ordered installation. The focused
tests confirm that Compare captures no payloads and reuses quiet observations.
These are useful properties. The audit does not support removing them.

**Verification and limits**

- `npm run test:frontend-integration`: 282 passed, including frontend test
  type checking.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib comparisons_capture_no_payloads`:
  1 passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib planning_reads_cloud_payloads_only_for_offerable_pull_threads`:
  1 passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib push_apply_captures_a_session_that_changed_after_compare`:
  1 passed.
- Scratch probes exercised production frontend helpers, Rust capture
  preflight, activity-cache code, and object-source code. All data was
  generated in temporary directories.

The desktop application, live R2 transfers, release build, and complete Rust
suite were not run. Fixture results establish repeated work and a capture
limit failure. They do not establish user-visible latency or a speedup ratio.

Fix order: Push selection, activity-cache ownership, Pull payload lifetime,
refresh read sharing, then the early destination check. Add regression checks
at the complete operation boundary. Record the time to local rows, time to
comparison, payload bytes read, object-read count, and bytes serialized.


**2026-09-14 implementation update — finding 4 and Push head reads**

Full Activity refresh now sends local rows before remote inspection. One
verified remote snapshot then supplies remote Activity rows and comparison.
The page uses `refresh_project_activity` in place of the separate comparison
command. Activity-only branch, pagination, and review reads remain in use.
In-flight refresh requests are separated by project, storage, branch, binding
revision, and refresh epoch. No head cache is persisted.

Changed Push publication now consumes the verified apply snapshot and its
CAS token. Review and apply still read the head. Publication makes one CAS
attempt without another head read. Changes after the apply observation,
including disjoint-scope changes, require comparison again. A no-change Push
checks the head before success because it makes no CAS write.

Tag publication retains its metadata read and CAS. Head CAS and tag update
now have separate timing phases. All persisted schemas remain 1. No migration
or legacy execution path was added.

Fixture checks cover head-read counts, stale and invalid publication input,
missing and recreated heads, disjoint-scope retention, no-change freshness,
remote failure after local rows, and comparison after the head becomes invalid.
The frontend checks cover shared requests, result replay, and revision isolation.
Live Dropbox latency and an isolated native UI run remain unverified. The
native QA launch displayed the normal workspace and was closed before any
manual sync action.

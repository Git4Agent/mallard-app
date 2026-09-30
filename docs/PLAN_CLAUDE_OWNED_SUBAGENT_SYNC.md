**Plan: Sync Claude subagents owned by a task**

Status: Implemented. Date: 2026-09-08.

Implemented settings amendment: [provider child-Push controls](PLAN_CHILD_SYNC_SETTINGS.md).
It uses the cloud child stamp for Compare and the local setting for Push.
Use that amendment for the current Push and Compare scope rules.

See [verification and performance results](CLAUDE_OWNED_SUBAGENT_SYNC_VERIFICATION.md).
The design below records the implemented contract and acceptance scope.

Mallard syncs separate Claude **direct-child** transcripts with their selected
parent. Each child has local and normalized content evidence. A change
to an included child affects the parent's comparison result. Pull restores
the listed children as native Claude conversations.

The parent remains the only review row, selection, cloud revision, conflict
action, and receipt unit. Reuse the shared behavior from the implemented
[Codex child plan](PLAN_CODEX_OWNED_SUBAGENT_SYNC.md). Claude file identity,
ownership, and parsing remain provider-specific. Transfer conversation history;
do not transfer running processes or live tool state.

**Compatibility is a clean cutover.** Keep local state, portable manifests,
and storage at schema **1**. Add no migration, compatibility reader, legacy
execution path, or feature flag. The user will remove Mallard state and cloud
data before retesting. Do not remove native provider files for the cutover.

This is the sole Claude child-sync proposal. It supersedes the Claude design
and acceptance scope in
[the local mock plan](PLAN_LOCAL_MOCK_MULTI_SUBAGENT_E2E.md), including its
Sidecar representation and identity reservations in Review. Its useful test
scenarios remain design history. Codex retains its implemented child scope.
The shared neutral child label pair below applies to both
providers; it does not change either main conversation codec.

**This cutover changes Claude behavior as follows.**

| Operation | Before this cutover | Implemented behavior |
| --- | --- | --- |
| Discovery | Main transcripts, `todos`, and `file-history` | Also find proven direct children of the selected parent |
| Hashing | Main local and normalized states | The same pair for each included child |
| Compare | Main conversation evidence | Run the existing comparator per rollout, then fold one parent result |
| Push | Main and existing sidecars | Main, existing sidecars, and the reviewed child set |
| Pull | Restore the main and existing sidecars | Restore exactly the manifest's main and child files, plus existing sidecars |
| Clear storage | Delete all managed objects and clear accepted main/child state | Use the same path; include Claude children in its tests |

The main Claude parser excludes `isSidechain` records. Add a child entry point
that validates child identity and reuses the existing chain and hash code.
Keep the main entry point's behavior unchanged.
See [discovery](../crates/mallard-infrastructure/src/providers/claude/discovery.rs),
[the codec](../crates/mallard-infrastructure/src/providers/claude/codec.rs), and
[materialization](../crates/mallard-infrastructure/src/providers/claude/materialization.rs).

**Use Claude's native parent namespace.** The standard layout is:

~~~text
CLAUDE_CONFIG_DIR/projects/<project-bucket>/
  <parent-session-id>.jsonl
  <parent-session-id>/subagents/
    agent-<agent-id-D>.jsonl
    agent-<agent-id-E>.jsonl

T owns D and E directly. Claude nesting is outside this cutover.
~~~

Claude documents this layout and requires the containing session to resume
a child after restart. See [Claude subagent persistence](https://code.claude.com/docs/en/sub-agents#resume-subagents).
The repository's [native POC](POC_LOCAL_MOCK_SUBAGENTS.md) verified a parent
tool-result `agentId` linked to a child with `isSidechain: true`. That POC
used Claude 2.1.260. The installed package inspected for this plan is 2.1.263;
the older POC is not a validation pass for that version. The new verification
record covers native 2.1.263 with two parents and three children per parent
in both synchronous and asynchronous runs.

The user-supplied corpus contains no nested-child instances in 12,569 child
records. Each child starts at its own root and carries its `agentId` on every
record. Of 95 agents, 62 are asynchronous; the call/result gap reaches 2,070
records. These findings support direct-child admission and a single-record
link, without a pending-call map. Confirm this grammar in isolated fixtures
for the target CLI and desktop runtime before implementation.

Async results also carry an `outputFile` outside the transcript tree. That
path is a historical reference to runtime output, not child ownership evidence
or an additional capture source. The current project-path mapper does not
make that external path usable on another machine. This cutover transfers
the durable child transcript; it does not restore temporary output files or
running tasks. Do not follow, copy, or recreate the source `outputFile` path.
The async fixture must prove that the restored parent can obtain the child's
durable result and resume it by agent ID without that file. A stale path can
remain in historical result metadata or text; it is not a restored resource.
If the target runtime requires it for these operations, async support fails
the acceptance gate. Do not treat a synchronous test as proof or silently
exclude async children to pass the gate.

**Ownership requires both the reference and the native file evidence.**

1. Resolve the main's authoritative leaf session and project through the
   existing Claude identity tracker. Use its actual transcript directory.
   Do not select a bucket only from its lossy encoded name.
2. Filter inherited references before lookup. A result record must belong
   to the main's authoritative session and main stream. Copied records with
   an ancestor's `sessionId` do not identify children of this parent. The
   supplied corpus has seven inherited references in three forked parents;
   looking up their paths before filtering would incorrectly block them.
3. Read a child link from one durable native tool-result record: its
   `tool_use_id` supplies the key, and `toolUseResult.agentId` supplies the
   child file ID. Require an unambiguous association inside that record.
   Do not join an earlier assistant `tool_use` block or retain pending calls.
   Ordinary text containing an agent ID is not a link. Until this result is
   durable, no new child membership is established.
4. The first owned result for an agent ID establishes its key. Deduplicate
   later results or resumes of that agent against the existing membership
   map; retain its original key. Reject one key naming different agent IDs.
   Use the existing bounded `SubagentKey` type, with a single segment for
   Claude. Reject Claude keys with an owner prefix in discovery and manifests.
5. Resolve each candidate at
   `<main-directory>/<parent-session-id>/subagents/agent-<agent-id>.jsonl`.
   Validate both IDs as safe, bounded filename components. Use exact-path,
   no-follow reads and the existing file-count, byte, and traversal limits.
   The owned result plus this file's identity is the complete ownership proof.

Each parsed child record must satisfy three identity checks:

~~~text
record.agentId     == agent ID in the filename
record.sessionId   == selected parent session ID
record.isSidechain == true
~~~

Run these checks during the required parse, before projection. Children use
their own complete root chain. Add no copied-history boundary, ordinal
search, or inherited-context classification inside a child.

A structured agent reference inside a child is an unsupported native shape.
Detect its presence with a boolean guard during that parse. Do not extract
referenced IDs, collect edges, or open another file. The child's own `agentId`
identity field and ordinary conversation text do not trigger this guard.

For a task installed by Pull, use the existing binding map to find and
re-verify known children. Discover new children only after the main's
`installed_end` byte offset. Keep that offset separate from
canonical message counts. Do not rediscover inherited or omitted references
from imported history. A stale cutoff or invalid installation map is an error.
The shared child `installed_end` field remains an installed-file byte cutoff;
Claude does not use it for descendant discovery.

> Reversed on 2026-09-29 for the main transcript. The Claude desktop app
> rewrites a transcript when it imports it, so a byte offset does not stay
> valid. A child that came with a Pull is now known by its ID: from the
> binding map, or from `installed_absent_subagents` when it came without
> its transcript. See `PLAN_PUSH_AFTER_DESKTOP_IMPORT.md`, section 13.

~~~text
P creates C
|
+-- fork --> T inherits P's messages
             |
             +-- creates D
             +-- creates E

Sync P: P + C              Sync T: T + D + E
T's copied reference to C does not transfer ownership of C.

After Pull:
  [ imported records .......... ][ new records .......... ]
  0                      installed_end                   EOF
  known children: binding map       new child discovery
~~~

**The omission policy has two outcomes.**

| Source evidence | Result for the selected parent |
| --- | --- |
| All required direct children have valid, supported records | Include the main and all direct children |
| A child contains a structured agent reference or another explicitly unsupported native shape | Sync the main only; omit every child |

A missing, unreadable, corrupt, or unsafe required file, or an identity
conflict, makes the parent Unavailable. These are validation errors, not
omission cases. Inherited main references were already excluded before lookup.

A valid main is always required. Once an unsupported owned child determines
main-only scope, stop child payload work. Do not hash omitted siblings or
follow the unsupported reference. The omission decision lives in the temporary
review, not in a stored mode field. Reuse the Push Review icon and tooltip:

> Only this task will sync. All child rollouts are excluded because their history cannot be verified.

Add no child selector or additional notice. Comparison explicitly excludes
all child evidence in this scope. An empty cloud child inventory means the
snapshot contains only the main; it does not assert that no native child
existed. Pull must still restore every child listed in its reviewed manifest.

**Reuse the hash contract and one neutral child label pair.**

- The child entry point applies the three identity checks above, then admits
  its durable sidechain records. Use the same rule in full analysis and the
  incremental feeder. Keep children excluded as independent root tasks.
  Preserve native `isSidechain` flags. Use borrowed expected IDs and the
  existing parse; add no stored context or separate validation pass.
- Reuse the existing Claude projection, message IDs, effective predecessor
  rules, semantic fingerprint helper, and `TailAccumulator`. Preserve the
  complete child transcript and fold its canonical chain.
- Compute local state from source native content. Compute normalized state
  from the encoded, path-mapped, redacted child content. After Pull, compute
  local state from the actual output bytes. Never relabel the normalized
  hash as a local hash.
- Use `mallard-subagent-tail-v1` and `mallard-subagent-tail-wire-v1` as the
  neutral child evidence label pair for both Codex and Claude. Keep
  `whole_thread`, start 0, and the existing compact
  `{ message_count, sha256 }` state. Wrapping, validation, and
  `compare_subagent_tail` need no provider parameter.
- Scope child comparisons by the parent provider/task and child key. The
  compact state stores no codec. Native analyzers and builders keep their
  parser contracts; convert their validated whole-child results to compact
  evidence, then wrap with the neutral pair. Update the existing Codex child
  wrappers in the same cutover. Keep main codec labels and hash folds intact.

The file entry's existing SHA-256 and size verify exact object bytes. The
canonical state is comparison evidence. Add no combined tree hash or new
hash algorithm. The fold contract remains in
[conversation_tail.rs](../crates/mallard-domain/src/conversation_tail.rs).

**Reuse the existing metadata; extend provider checks.**

| Location | Change |
| --- | --- |
| Cloud `ConversationThread.files` | Permit Claude `SubagentTranscript { child_key, normalized }` entries |
| Cloud `BundleScope.files` | Store children with existing object key, SHA-256, size, and file metadata |
| Child logical path | Parameterize the existing helper: `state/conversations/claude/<cloud-task>/subagents/<child-key>.jsonl` |
| Parent binding | Reuse `installed_end` and `installed_subagents: key -> { provider_thread_id, installed_end }`; the child ID is Claude's `agentId` |
| Storage-link checkpoint | Reuse `accepted_subagents: key -> { local, normalized }` |
| Observation cache | Retain parent result links and child identities, states, stamps, support result, and append cursors; no child reference or pending-call map |
| Path-origin ledger | Reuse the parent's existing ledger, scoped to each main/child stream |
| Temporary review and prepared output | Reuse child selection, omission flag, estimates, source stamps, and immutable verification results |

Allow Claude installation metadata in the existing binding and materialization
validators. Validate parent root IDs as today. Codex child IDs remain unique
within the provider profile. Claude child file identity is
`(profile, parent-session-id, agent-id)`: the same agent ID under two different
parent directories is valid. Apply provider-aware identity checks to live
documents and journal after-states. Keep context-free bounds and per-map
uniqueness checks; do not add a provider field to each binding or weaken
Codex identity checks to accommodate Claude.

Refresh saves observations only. Successful Push accepts main and child pairs
together for that storage link; main-only Push clears that link's child map.
Successful Pull also records installed IDs and byte cutoffs. Keep both retains
the local installation data and creates a separate imported parent and children.
Clear storage clears parent and child acceptance together, deletes all their
stored objects, and retains local provider files and installation mappings.

Add no mode tri-state, wire boundary, per-child checkpoint, global child index,
or new metadata file. Required existing fields stay explicit. Schema stays 1.

**Compare uses the existing implementation.**

Read the usual HEAD and manifest. Child normalized states come from the
manifest; download no transcripts for Refresh. Observe local children with
validated caches. For first contact, extend the existing proof to every
overlapping child; a matching main cannot supply missing child evidence.
If that proof fails, return Unavailable instead of manufacturing an acceptance.

Use `VerifiedTailEvidence`, `compare_subagent_tail`, and
`fold_conversation_relations` from
[conversation_tail_relation.rs](../crates/mallard-domain/src/conversation_tail_relation.rs).
That module is the sole authority for presence, outstanding work, divergence,
and parent result folding. Add no provider parameter or second truth table.
Main-only scope excludes every child from this comparison.

> Changed on 2026-09-29. The fold takes one fact beside the relations:
> whether the Pull that updates the session leaves the children that only
> this computer has in their place. It is so for Claude, which is updated
> in place. Such a child beside a change in the storage then leaves the
> session behind the storage, where it was diverged before, and the
> children that two computers made of one session come together. The
> module is still the one authority, and it names no provider: the adapter
> says the fact, and a reviewed plan pins it. See
> `PLAN_PUSH_AFTER_DESKTOP_IMPORT.md`, section 15.

**Recover a missing accepted local child with a reviewed Keep both Pull.**

This shared change applies to Claude and Codex. The previous planner blocked
Pull for every Unknown result. Keep that comparison result
when an accepted child disappears locally. Keep Push blocked. Permit Keep
both in Pull Review only when the main and binding are valid, all other
required local evidence is valid, and each missing local child has an
accepted entry and a corresponding entry in the valid cloud manifest.
Missing means confirmed file absence; unreadable files, identity conflicts,
unsafe paths, and missing cloud children do not qualify.

The row must name the missing child and state the action:

> Missing local child: <key>. Keep both restores the cloud copy and retains the local files.

Reuse the existing Keep both materialization and metadata update:

1. Pin the cloud snapshot and normal source, destination, and revision guards
   in Review. Revalidate the missing-child cause at Apply. Fetch and validate
   every listed cloud file before installation.
2. Restore the cloud main and its listed children under fresh native parent
   identity rules. Preserve the remaining local files in place, including
   local work that has not been pushed. Do not overwrite surviving children.
3. Retain the damaged local copy under the existing preserved-task binding.
   Move the tracked cloud task to the restored copy and save its main/child
   acceptance together. Clear acceptance on the preserved copy for this
   storage link as ordinary Keep both does. Other tasks are unaffected.

The restored copy can sync normally. The preserved copy still reports its
missing child until the user restores that native file from another device
or backup and refreshes. Do not claim to recover work that never reached the
cloud. No Clear storage or standalone pin reset is required.

Carry the missing-child cause through the existing review/plan evidence.
Extend the shared action policy and plan validation together in
[conversation_reconciliation.rs](../crates/mallard-domain/src/conversation_reconciliation.rs).
Do not relabel Unknown as Divergent or admit a generic Unknown override.
Add no stored recovery mode, per-child action, or second comparison rule.

**Push Review fixes selection; Push Apply captures current bytes.**

~~~text
PUSH REVIEW
  observe main + children; read cloud metadata
  retain child keys, include/omit decision, and size estimates
                         |
                    user reviews
                         |
PUSH APPLY, per parent
  capture latest main and included child bytes with SourceStamp
  validate reviewed membership and source support
    scope changed -> fail this parent
                         |
  encode captured content; compute local/normalized pairs
  prepare existing immutable object entries
                         |
  final SourceStamp::verify for each captured source
    source changed -> fail this parent
                         |
  publish valid parents through existing object storage and HEAD CAS
  save accepted pairs and receipts through existing persistence
  report each parent's success or failure
~~~

An ordinary append to a known member before Apply capture is allowed.
An append, replacement, truncation, or rewrite detected after capture fails
that parent. A newly identifiable child changes membership and requires a
new review. Reuse the existing scope/header witnesses for omission decisions;
an omitted child's body append alone is outside the captured scope. Use the
existing message:

> This task changed during Push. Push it again to include the latest work.

Do not reopen files to encode them again or add a second Review byte snapshot.
Reuse exact-byte integrity checks and the final source gate. A source stamp
does not lock Claude through network publication. Parents rejected before
publication keep their previous cloud rows and pins. Other valid parents can
succeed. An error after publication leaves completed writes in place.

**Pull preserves Claude agent IDs inside a separate parent directory.**

Unlike a Codex child, a Claude child is not an independently addressed root
session UUID. For Create local and Keep both, use the existing fresh parent
session-ID allocation. Preserve each child's native `agentId` and tool-result
key. For an update of an existing binding, retain that binding's parent
ID and known child mappings. Reject occupied unrelated targets.

> Made precise on 2026-09-29. A known child of an update in place is one
> that a Pull installed, or one that the local main transcript refers to
> and whose file the provider wrote on this computer. The review pins its
> file, and Apply replaces it under that state. Every other occupied
> target is rejected. See `PLAN_PUSH_AFTER_DESKTOP_IMPORT.md`, section 14.

A fresh parent must have both an unused main transcript path and an unused
child-directory namespace. An orphan directory is an occupied target even
if its main file is absent. Recovery may reuse only proven journal-owned
targets. Do not attach pre-existing child files to a newly allocated parent.

Restore D at `<destination-parent>/subagents/agent-<D-agent-id>.jsonl`.
Restamp the containing `sessionId` in the main's owned records and every
included child record. Retain ancestor IDs in copied main history. Preserve
native agent IDs in structural references, resume requests, and returned
result text. Apply the existing path mapping, origin tracking, and redaction
rules. Do not perform global string replacement. This avoids a new agent-ID
text rewriter and keeps the two Keep both copies in separate native directories.

~~~text
PULL REVIEW
  pin cloud manifest and existing destination guards
  plan T + exactly the listed direct children; reject nested Claude keys
  accepted local child missing -> offer eligible Keep both recovery
                         |
PULL APPLY
  fetch listed objects; verify exact size and SHA-256
  parse wire content once per file
  verify normalized states, direct ownership, keys, and declared inventory
  reject a child containing a structured agent reference
    invalid listed child -> fail before installation; never silently omit
                         |
  reserve destination parent ID using existing rules
  decode all files for that parent; retain child agent IDs
  verify each output once; compute local states and installed_end
                         |
  stage all outputs; reuse immutable verification results
  validate target identity tuples, mappings, and accepted pairs
  commit through existing journal and staged-digest checks
                         |
  save parent binding, child map, accepted pairs, and receipt
  report success only after all required writes complete
~~~

Portable inventory validation must match every declared child to its native
identity and the parent's tool-result link. No nested Claude inventory is
valid. Old native references outside that inventory can be inherited or
omitted; they do not authorize fetching or attaching destination files.
Never search another parent's directory to satisfy them.
Known installed children are re-verified through the binding map on Refresh.
An in-place update does not delete pre-existing native files absent from the
manifest. Those files receive no new acceptance. An empty child inventory is
not a native-file deletion request.

Keep the current Apply failure behavior. Validate and stage all required files
before installation. Stop on an installation or metadata error and report
failure. Completed writes remain; do not undo them. Do not report an incomplete
installation as a successful parent. Reuse the current journal recovery and
revision guards.

**Share the orchestration, not the native parser.**

Move the small shared selection/capture result and verified child-output types
out of `codex_subagents.rs` into a neutral `owned_subagents.rs` module. Reuse
one parent-level comparison fold, publication path, materialization path,
acceptance write, and error handling path for both providers. Extend the
existing provider adapter boundary for child operations. Keep Codex graph
resolution in its current module; do not enable `reconstructs_rollout_graph`
for Claude. Put Claude ownership, child parsing, path resolution, and output
validation under `providers/claude/`. Add no second sync engine or generic
provider-plugin framework.

**Keep added work proportional to the parent and its direct children.**

| Case | Expected additional work |
| --- | --- |
| Parent without child references | Extract references during required main parsing; zero child file opens |
| Cold parent with K children and B child bytes | Exact child-path lookup, bounded identity checks, and O(B) parsing; no scan of all Claude projects |
| Warm unchanged parent and children | Reuse valid observations; O(K) stamp checks and map access, no full child body reads |
| One child appends | Process that child's new bytes with the existing feeder and fold; reparse that child only if its chain shape requires it |
| Push/Pull | Process included bytes and existing immutable objects; use the normal transport batching and cache |

Collect child links only while parsing the main. Full analysis computes the cursor watermark in that same parse. On a cold
child cache, rebuild references and use the existing main cursor checks.
If the cursor requires full analysis, reuse the bytes already read. Child parses perform three
bounded identity checks, the unsupported-reference presence guard, and the
existing projection and fold. They do no reference extraction or recursive
discovery. Reuse each wire parse for state and identity checks and retain each
verified output through staging. Do not add another semantic analysis there.
Warm discovery must still notice child-only changes through watch dependencies
or stamp checks. Refresh adds no object downloads or per-child network calls.

These are work bounds, not latency claims. The verification record reports
measured release results. Compare
release builds with 0, 1, 8, and 16 children at equal byte sizes. Record median
and p95 for cold/warm Refresh, child append, Push Apply, and Pull Apply.
The acceptance target is no repeatable main-only regression above 5%, no full
body reads for unchanged children, and zero Refresh transcript downloads.

**Implementation and acceptance scope.**

1. Refresh the native fixtures with temporary homes, a loopback mock, fixed
   placeholder credentials, and explicit tool lists. Include both a direct
   synchronous child and an actual asynchronous child with
   `run_in_background: true`; the current POC sets it to false. Capture the
   async result's `tool_use_id`, `agentId`, and external `outputFile`, then
   let the child finish. Prove single-record linking, a long async gap,
   repeated resume, and inherited-reference filtering in a forked main.
   Build a destination-native fixture in a distinct home/project with the
   source output file absent or inaccessible. Verify the restored parent can
   obtain the durable child result and resume by agent ID through the installed
   transcript. Record actual versions; fix the POC's version pin before using it.
2. Generalize the shared child domain helpers, path helper, evidence wrappers,
   and provider-aware binding validation. Use the neutral child label pair
   for both providers. Keep Codex tests passing. Add Claude child
   classifier/full-parser/feeder agreement and identity-mismatch tests.
3. Implement Claude discovery and cached observation, then Push and comparison
   through the shared parent flow. Test child-only append, conflicting sides,
   same-count edits, missing accepted children, and first-contact evidence.
4. Add exact-inventory Pull and native child output validation. Test a distinct
   destination home/project, two parents with equal agent IDs, Keep both,
   round-trip hash stability, source isolation, collisions, and interrupted
   installation. Delete an accepted child only in the test home. Verify
   Unavailable plus the eligible Keep both recovery, preservation of unsaved
   local files, and unchanged unrelated tasks. Test that other invalid
   evidence cannot use this recovery and that a reappearing child makes its
   reviewed recovery stale. A no-op Refresh of the restored copy after Pull
   must be Synchronized.
5. Extend the isolated service E2E: two parents, three direct children each.
   Include an async child. Remove access to the test-owned source output
   artifact before destination checks. Open all restored child histories in
   the target native client. Restart the client and resume the async child
   from its restored parent with a follow-up that requires prior child context.
   Verify that only the destination file changes. Parent summaries or files
   on disk alone do not pass this gate.
6. Use a synthetic agent-reference record inside a child to verify all-child
   omission on source discovery and rejection of a listed child during Pull.
   Reject nested Claude keys. Cover stale review membership, allowed pre-capture
   append, rejected post-capture change, mixed batch results, tampered child
   bytes, Clear storage, and re-Push after Clear. Run performance checks and
   the appropriate Rust/frontend suites before handoff.

Required checks include `cargo test --manifest-path src-tauri/Cargo.toml --lib`
and the affected domain/infrastructure crate tests. Run `npm run build` and
`npm run test:frontend-integration` if shared DTOs or UI change. Run native
checks only in test-owned homes. Record transport, native visibility, and
native resume separately; an unavailable native client is not a passing test.

The feature is complete when both providers use the shared parent workflow,
Claude child-only work affects comparison, missing accepted local children
have the reviewed recovery path, and Pull restores usable independent child
files, including async children without source output artifacts. Round trips
must remain synchronized, and the performance gate must pass.

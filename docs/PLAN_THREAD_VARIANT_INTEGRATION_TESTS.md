# Thread Variant Integration Test Plan

## Purpose

This document defines the comprehensive integration-test strategy for the
thread-variant continuation behavior described in
`PLAN_THREAD_VARIANT_CONTINUATION.md`.

The suite must prove that Push and Pull preserve conversation branches, local
provider-thread continuity, cloud primary selection, local preference, and
active-provider state across ordinary synchronization, conflicts, retries,
concurrent writers, crashes, schema cutover, and storage backends.

The central test architecture is a state-machine integration suite built around
three isolated Mallard installations sharing one real test store. The complete
relation-by-direction matrix is tested deterministically. Orthogonal concerns
such as provider capability, stale plans, failure recovery, host identity, and
storage transport are covered by focused scenarios rather than by an
unmaintainable Cartesian product.

## Contract decisions required before test implementation

The following behaviors must have one typed, deterministic result before the
integration tests can be authoritative.

### Push action types

The proposed `ConversationPlanAction` list includes `FastForwardLocal`, but it
does not name the Push outcomes for:

- fast-forwarding the cloud variant; or
- blocking Push because Pull is required.

Add explicit outcomes such as `FastForwardCloud` and `RequirePull`, or use
direction-specific Push and Pull action enums. Tests must assert action and
reason codes, not infer behavior from display messages.

### Unknown ancestry policy

Resolve the current “preserve or block/store-only” choice by reason:

- A safely capturable local branch with an existing group binding may be
  preserved as a separate cloud variant.
- A safely readable incoming cloud branch whose lineage cannot be proven may
  be retained locally as `StoredOnly`.
- Missing or ambiguous bindings block Push; title, provider ID, and display
  text never resolve the ambiguity.
- Malformed, cyclic, branched, duplicate-ID, or oversized transcripts are
  unavailable or stored-only and are never advertised as resumable.

### No-op publication

Confirm that a true semantic and lineage no-op does not:

- publish a new generation;
- create a new commit;
- update publisher provenance;
- update last-editor provenance; or
- rewrite local provider or binding state.

### Group tombstone resurrection

Define whether a group tombstone racing with a modified local branch revives
the same conflict group or creates a new group. The resulting live and
tombstoned representation must be specified before an exact integration-test
oracle can be written.

## Test boundaries

### Production boundary under test

Scenario tests should exercise the production orchestration used by the Tauri
commands:

1. capture provider state;
2. fetch and validate the reviewed cloud snapshot;
3. build a typed Push or Pull plan;
4. apply the reviewed actions;
5. publish through immutable objects and head CAS when required;
6. persist bindings, preferences, receipts, and journals; and
7. restart and run normal recovery.

The tests should not reproduce reconciliation logic in a test-only
implementation. A narrow test facade around internal orchestration is
preferable to making every private `*_with_repository` helper
production-public.

### Proposed file layout

~~~text
src-tauri/src/sync_tests/
  mod.rs
  thread_variants/
    mod.rs
    harness.rs
    relation_matrix.rs
    continuation.rs
    conflicts.rs
    recovery.rs
    schema_and_privacy.rs
    storage_parity.rs

src-tauri/testdata/thread_variants/
  codex/<codec-version>/...
  claude/<codec-version>/...

tests/frontend/
  thread-variants.integration.test.tsx
~~~

Register the frontend file in
`scripts/run-frontend-integration-tests.mjs`.

Do not add the new scenarios to the existing large
`pull_review_apply_flow_materializes_a_codex_session_for_the_replica` test in
`commands.rs`. Retain one focused schema-4 read-only regression there if
needed, and place cross-module variant flows under `src-tauri/src/sync_tests/`.

## Integration harness

### `VariantWorld`

Create a reusable world fixture with:

- one deterministic bundle ID;
- a local-folder store or stub S3 store;
- machines A, B, and Fresh;
- one or more project replicas per host;
- independent project roots and provider homes; and
- a normalized snapshot and invariant checker.

A, B, and Fresh must each use a separate
`V3Repository::from_home_dir(...)` root. Creating two projects in one
repository tests two replicas on one host; it does not test two hosts.

### Replica fixture

Each replica records:

- Mallard home and `V3Repository`;
- host ID and replica ID;
- local project and bundle IDs;
- project root;
- provider profile and provider home;
- conversation bindings and preferences; and
- a launcher/focus spy for proving that Pull does not activate a provider.

### Transcript fixture builder

The builder must emit valid, real-format provider fixtures with stable message
IDs, parent IDs, timestamps, roles, content, tool calls, and tool results. It
must support:

- exact copies;
- exact extensions;
- forks after a common prefix;
- no common ancestor;
- structural provider-ID and `cwd` rewrites;
- identical display text with different message IDs;
- reused message IDs with changed semantic content;
- missing IDs;
- malformed JSONL;
- duplicate IDs;
- cycles;
- branched parentage;
- oversized lines, files, and record counts; and
- provider index and sidecar data.

Simplified two-line transcripts may be used for orchestration smoke tests, but
not for provider resumability or continuation proofs.

### Normalized world snapshot

Capture the following before planning, after planning, after apply, after
retry, and after restart:

- cloud generation, commit, manifest digest, primary, groups, and variants;
- immutable object keys and SHA-256 values;
- local bindings, preferences, aliases, and materialization states;
- provider transcript, index, and sidecar inventories;
- canonical lineage and semantic digests;
- receipts and journal records;
- staging and backup inventories; and
- launcher/focus calls.

Strip nondeterministic timestamps and ephemeral plan IDs when comparing
snapshots. Capture generated provider and variant IDs on their first
appearance, then assert stability, uniqueness, and reuse.

### Failure injection

Provide deterministic failure points for:

- journal persistence;
- staging writes and fsync;
- each provider transcript, index, and sidecar commit;
- binding and preference persistence;
- receipt persistence;
- immutable cloud object publication;
- head compare-and-swap;
- post-CAS local finalization; and
- cleanup.

A failing wrapper around `BundleObjectStore` can cover cloud failures. Local
orchestration needs explicit test failpoints or an injected filesystem writer.
A controllable clock is also needed for expiry and timestamp-sensitive tests.

## Complete relation matrix

Every row must be exercised for both plan generation and apply. Each case must
assert the typed relation and action, final state, retry behavior, and restart
behavior.

| Local/cloud state | Push expectation | Pull expectation |
| --- | --- | --- |
| Same lineage and semantics | `NoOp`; generation and provenance unchanged | `NoOp`; no provider or binding writes |
| Raw bytes differ only through verified thread-ID or `cwd` rewrites | Same as identical | Same as identical |
| Cloud is an exact prefix of local | Fast-forward the same cloud variant; create no variant | `KeepLocal`; perform no writes |
| Local is an exact prefix of cloud | Typed `RequirePull`; publish nothing | Fast-forward the existing provider thread while preserving its ID and preference |
| Same variant ID with divergent common prefix | Rekey local once, retain cloud V, and publish stable V-local | Rekey local once, retain its provider ID, and materialize cloud V under a new provider ID |
| Already-separated bound alternate advances or diverges | Update only its existing variant | Add or update the alternate without rekeying or changing preference |
| Same message IDs with changed semantic fingerprints | Corruption or `Unknown`, never equal | Preserve both or store-only; never overwrite |
| Same rendered text with different message IDs | Diverged | Diverged |
| No known common ancestor | Diverged when already bound; ambiguous recovery otherwise blocks | Preserve local and add or store the cloud alternate |
| Local missing, cloud present | No Push action | Materialize or `StoreOnly` |
| Local present, cloud missing, no tombstone | Add local variant | Keep local; absence never deletes |
| Both missing | `NoOp` | `NoOp` |
| Tombstone versus unchanged copy | Explicit reviewed deletion only | Explicit reviewed deletion and recoverable backup only |
| Tombstone versus modified copy | Treat as divergence and preserve modified content | Treat as divergence and preserve modified content |

Add a second local-prefix-of-cloud Pull case where provider quiescence cannot
be established. The result must be deferred or stored-only, with unchanged
local bytes, binding, provider ID, preference, and active conversation.

## Common assertions for every matrix case

### Plan assertions

- The plan contains the exact typed relation and action.
- The plan pins host, replica, generation, commit, manifest, local target,
  provider state, binding revisions, preference revision, profile revision,
  codec, and expiry.
- Proposed provider IDs and target paths are reserved without mutating provider
  state.
- Planning does not modify cloud content, provider content, bindings, or
  preferences.

### Apply assertions

- Only reviewed actions are executed.
- Cloud primary, local preference, and active provider thread change only when
  explicitly permitted.
- An existing provider thread is never renamed because an incoming variant
  collides with its ID.
- Unselected and remote-only variants remain in the next manifest.
- Generated IDs are unique in scope and stable across retries.
- A resumable binding is exposed only after the complete provider file set and
  binding validate together.
- Cloud publication is immutable and SHA-256 verified.
- No unexpected journal, staging, backup, or temporary state remains.

### Repeat and restart assertions

- Reapplying the same reviewed plan is either idempotent or rejected with a
  typed already-completed result.
- Replanning and repeating Push or Pull creates no duplicate variants,
  provider files, index rows, sidecars, bindings, receipts, or journals.
- Reopening the repository and running startup recovery produces the same
  normalized fixed point.

## End-to-end continuation scenarios

### A/cloud/B continuation

Implement `a_cloud_b_continuation_preserves_each_active_branch`:

1. Create base `[a, b]`, V1 `[a, b, c1]`, and V2 `[a, b, c2]`.
2. Make V1 cloud-primary.
3. Bind A's existing provider thread and preference to V1.
4. Bind B's existing provider thread and preference to V2.
5. Push A and prove V2 and cloud primary are retained.
6. Pull B and prove its V2 bytes, provider ID, preference, and resume command
   are unchanged; V1 receives a distinct local provider ID.
7. Append to B's V2 and Push; only V2 advances.
8. Pull A; A remains on V1 and materializes V2 under a distinct provider ID.
9. Append to A's V1 and Push; only V1 advances.
10. Repeat Pull and Push on both machines and restart both repositories.
11. Assert A and B have distinct host identities and privacy-scoped origin
    tokens.

### Same-ID divergent Pull

Implement `same_id_divergent_pull_rekeys_once`:

- Start with divergent cloud and local content both named V.
- Pull must retain cloud V.
- The local branch becomes one stable V-local.
- Its provider ID, bytes, and active conversation remain unchanged.
- Preference follows the same branch from V to V-local.
- Incoming cloud V receives a distinct provider ID.
- Repeat Pull and restart create nothing new.

### Same-ID divergent Push

Implement `same_id_divergent_push_reuses_rekey_after_cas_failure`:

- Start with divergent cloud and local content both named V.
- Push rekeys local to V-local before publication.
- Inject a failed cloud CAS.
- Assert V-local remains a valid local-only variant with its existing provider
  ID and preference.
- Replan and retry.
- Assert the same V-local is published exactly once.
- Inject success followed by local finalization failure and prove recovery
  finds the published commit rather than creating another variant.

### Bound alternate continuation

Implement `bound_alternate_push_updates_only_its_variant`:

- Cloud contains primary V1 and alternate V2.
- The local provider thread is bound to V2 with a different provider ID from
  the source provenance ID.
- Append locally and Push.
- Assert only V2 changes; V1's descriptor, files, immutable object keys,
  provenance, and primary status remain unchanged.

## Provider-ID allocation and binding recovery

Cover these cases:

- Two cloud variants share one source provider ID on a fresh replica.
- The source provider ID is already occupied by unrelated local content.
- The collision belongs to another project using the same provider profile.
- A primary and alternates allocate in deterministic plan order.
- Two concurrent plans reserve the same free provider ID; after one applies,
  the other rejects as stale and replans with a different ID.
- Repeat Pull reuses the saved binding and receipt.
- A missing binding is recovered only when one unique longest compatible
  cloud ancestor exists.
- Equal compatible candidates, title-only matches, provider-ID-only matches,
  and unknown ancestry block recovery.

After every successful case, assert provider-ID injectivity throughout the
entire provider profile, not merely within one project.

## Stale-plan and concurrent-writer matrix

Create a table-driven test that changes exactly one reviewed pin after plan
creation:

- cloud generation;
- commit ID;
- manifest digest;
- local transcript digest;
- provider transcript, index, or sidecar digest;
- an expected-absent target becomes occupied;
- provider-profile revision;
- project-binding revision;
- conversation-bindings document revision;
- individual conversation-binding revision;
- preference revision;
- host identity or identity revision;
- replica identity;
- codec version or capability;
- plan expiry; and
- provider quiescence.

Every case must reject before destructive mutation. Compare the normalized
world snapshot before and after rejection, excluding the expected plan or
diagnostic record.

A provider append after review and before apply must always win. Pull must not
overwrite it, truncate it, or advertise cloud content as the active resumable
branch.

## Crash and I/O recovery matrix

Inject termination or failure:

1. before and after `Prepared` journal persistence;
2. during staging;
3. after staged verification;
4. before and after each provider transcript, index, and sidecar write;
5. before and after `ProviderCommitted`;
6. before and after binding and preference persistence;
7. before and after receipt persistence;
8. before and after immutable cloud object publication;
9. immediately before and after head CAS;
10. before and after `CloudPublished`;
11. before and after `LocalCommitted`; and
12. during staging, backup, and journal cleanup.

For each checkpoint:

1. close the original repository instance;
2. reopen it as a process restart;
3. run startup recovery twice;
4. assert convergence to exactly one valid result; and
5. validate every portable and machine-local document.

The fixed point must contain no duplicate variant, duplicate provider ID,
half-advertised resumable binding, orphan index row, lost provider content, or
unrecoverable staging tree.

## Primary, preference, and active-thread safety

Test independently:

- Pull cloud-primary V1 while A prefers V2; A remains on V2.
- Pull cloud-primary V2 while B prefers V1; B remains on V1.
- Same-ID rekey updates the preferred variant ID but preserves the provider
  thread and transcript.
- Only explicit local promotion changes local preference.
- Only explicit cloud promotion changes cloud primary, with a reviewed CAS.
- Fresh Pull initializes preference from cloud primary only when that variant
  becomes resumable.
- A stored-only primary does not become the local preference automatically.
- Pull never invokes provider launch, focus, open, or resume behavior.

## Tombstone scenarios

Cover unapproved, approved, stale, and repeated forms:

- Missing without a tombstone never deletes anything.
- An unchanged non-primary variant requires explicit deletion approval and a
  recoverable backup.
- A primary variant cannot be tombstoned without selecting a replacement
  primary in the same CAS.
- Deleting the final live variant uses a group tombstone.
- Variant deletion racing with modified local content preserves the modified
  branch as live.
- Group deletion racing with modified local content follows the explicitly
  selected resurrection rule.
- A stale deletion digest rejects before mutation.
- Repeated application and post-crash recovery are idempotent.

## Manifest and metadata validation

Use valid baseline manifests and mutate one invariant at a time. Reject:

- group-map keys that differ from embedded group IDs;
- variant-map keys that differ from embedded variant IDs;
- duplicate group or resource identities;
- missing, duplicate, or unrecognized transcript roles;
- orphan conversation descriptors or files;
- shared logical paths across variants;
- invalid primary pointers;
- self-parenting and parent cycles;
- cross-group parents;
- provider and codec mismatches;
- incomplete “complete lineage” metadata;
- misleading complete-only fields on unknown lineage;
- live variant/tombstone collisions;
- recipe/resource disagreement;
- duplicate provider-ID ownership;
- excessive collection, path, file-size, and metadata bounds; and
- stale host, document, binding, preference, and provider-profile revisions.

Also prove that pushing one reviewed local variant carries every unchanged
remote variant and its existing immutable objects into the next manifest.

## Host, replica, alias, and privacy scenarios

### Identity behavior

- Two fresh installations generate different host IDs and origin keys.
- Repeated operations on one bundle derive the same host token.
- One host derives unlinkable tokens for different bundle IDs.
- Two replicas on one host share a host token but use distinct replica tokens.
- Current-host resolution produces “This host.”
- Unknown remote tokens use bounded fallback labels.
- Local aliases never cross into portable state.
- Spoofed origin tokens never affect classification, approval, replacement,
  binding recovery, or CAS.

### Copied and rotated state

- Copying state without the protected origin key requires explicit
  “Register as new host.”
- Byte-for-byte cloning including the key remains deliberately undetectable.
- Explicit host rotation creates a new host ID, origin key, and replica IDs.
- Rotation invalidates outstanding plans.
- After provider-scope revalidation, group IDs, variant IDs, transcripts, and
  provider thread IDs remain continuous.
- Old receipts retain truthful old-host pins and cannot authorize new-host
  operations.

### Portable-data privacy scan

Use unique sentinel values for:

- host and replica IDs;
- origin key bytes and key reference;
- hostname and username;
- hardware-like identifiers;
- project and provider absolute paths;
- provider-profile ID;
- preference and binding IDs;
- plan, receipt, backup, and journal IDs; and
- provider thread ID.

Inspect manifests, commits, tags, object keys, DTOs, and activity records.
Assert that forbidden machine-local values are absent. A permitted bounded
`source_thread_id` may appear only in its provenance field; it must not become
a group ID, variant ID, resource ID, object key, authorization signal, or log
field.

## Schema cutover and compatibility

Cover:

- Two independent replicas derive identical initial group and variant IDs from
  one schema-4 manifest.
- Exact local content binds to the migrated variant.
- A locally divergent schema-4 transcript becomes a separate stable variant.
- Migrated historical content does not falsely name the cutover host as its
  creator or last semantic editor.
- Immutable schema-4 history remains readable.
- Outstanding older Push and Pull plans are rejected.
- Schema-4 writers reject a schema-5 head.
- No post-cutover path writes schema 4.
- Local-folder and stub-S3 transports preserve immutable objects and CAS
  semantics through cutover.

Use committed synthetic golden fixtures for legacy documents instead of
constructing them through the new writer.

## Provider continuation proof

Run this suite independently for every provider and codec version advertised
as resumable:

1. Discover the complete authoritative source file set.
2. Materialize it under a new destination provider ID and project root.
3. Verify transcript filename and structural thread IDs.
4. Verify structural `cwd` rewrites.
5. Verify provider index rows and required sidecars.
6. Verify message IDs, parentage, roles, content, timestamps, tool calls, and
   tool results are semantically unchanged.
7. Parse the staged and committed results through the production codec.
8. Invoke or simulate the provider's actual resume lookup against the
   temporary provider home.
9. Append a new message through a supported-format fixture update.
10. Capture and Push it.
11. Prove the same `variant_id` advances rather than creating another variant.

Negative fixtures for every supported codec must prove that ambiguous,
malformed, incomplete, or unsupported representations become `StoredOnly` or
unavailable rather than resumable.

## Storage parity

Run the complete deterministic relation matrix against
`LocalBundleObjectStore`. Re-run these protocol-critical scenarios against the
stub S3 server:

- exact no-op;
- safe fast-forward;
- same-ID divergence;
- A/cloud/B continuation;
- concurrent writers where one CAS loses and retries;
- remote-only variant retention;
- tombstone/modify race; and
- schema-4-to-schema-5 cutover.

For both transports, assert immutable object behavior, object SHA-256 values,
manifest integrity, conditional head updates, and retention of unchanged
remote object keys.

The stub S3 suite covers the S3-compatible/R2 protocol behavior. It does not
need to contact a real account.

## Frontend integration coverage

Frontend tests should consume typed backend fixtures and must not reimplement
ancestry classification.

Cover:

- variants grouped by conflict group;
- cloud-primary and local-preferred indicators shown independently;
- resumable, pending, and stored-only states;
- “This host,” fallback origin labels, and local aliases;
- safe default selections;
- explicit promotion, replacement, and deletion approval;
- stale-plan refresh behavior;
- provider IDs shown only in details and continuation controls;
- different variants with the same title remaining distinct;
- remote-only and locally preferred versions both remaining visible; and
- no open or resume callback during Pull.

The frontend need only cover every DTO relation, action, materialization state,
and reason-code rendering. The backend integration suite remains authoritative
for classification and mutation semantics.

## Global invariant checker

Run `assert_world_consistent()` after every successful apply, rejected apply,
recovery, retry, and restart. It must validate:

- every portable and local document;
- one descriptor and complete file-role set per live variant;
- no live/tombstone collision;
- primary resolves to one live variant;
- preference resolves to one locally known variant;
- provider IDs are injective throughout each provider profile;
- each resumable binding has a complete verified provider file set;
- canonical lineage and semantic digests match expected content;
- raw-object hashes match immutable bytes;
- cloud primary, local preference, and active provider state changed only when
  allowed;
- unselected and remote-only variants survived;
- generations advance only after successful publication;
- no duplicate bindings, receipts, index rows, sidecars, or journal records;
- no unexpected staging or temporary files remain; and
- portable and logged state passes privacy checks.

## Implementation order

1. Lock the ambiguous action and policy decisions listed above.
2. Add real-format transcript fixtures and the transcript builder.
3. Add `VariantWorld`, normalized snapshots, and the invariant checker.
4. Add the complete Push and Pull relation matrix as plan-generation tests.
5. Enable successful apply paths for no-op, prefix, missing, and structural
   equality.
6. Add same-ID divergence and bound-alternate continuation.
7. Add the A/cloud/B scenario and provider-ID collision tests.
8. Add stale-plan and provider-concurrency cases.
9. Add journal and I/O failure injection and recovery tests.
10. Add tombstone, host identity, privacy, and schema-cutover suites.
11. Add provider continuation proofs.
12. Add stub-S3 parity and frontend DTO/rendering coverage.

Tests should land with the implementation phase they protect. Do not defer the
failure, privacy, or cutover suites until after enabling resumable variants.

## Completion gate

The feature is test-complete only when:

- every state-classification row has at least one Push and one Pull test;
- every typed Push and Pull action is planned, applied, rejected when stale,
  and rendered;
- every unknown-reason and materialization-state branch is covered;
- every plan pin is independently invalidated in a test;
- every journal phase and material provider write has a restart test;
- every tombstone transition has unapproved, approved, stale, and repeated
  coverage;
- local-folder and stub-S3 protocol-critical scenarios pass;
- every advertised resumable provider codec passes its continuation proof;
- A and B keep their own branches, provider IDs, preferences, and active
  conversations throughout the full continuation scenario; and
- privacy scans find no forbidden local identity or path data in portable or
  logged state.

## Verification commands

Run the smallest focused suite while iterating:

~~~bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sync_tests::thread_variants
~~~

Then run the backend, frontend, and complete integration handoff set:

~~~bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:integration
~~~

Stub-S3 tests bind a loopback server and may require explicit localhost
permission in restricted environments.

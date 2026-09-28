# Compare-Only Review and Apply-Time Capture

- Status: Implemented
- Date: 2026-09-03
- Scope: Project Sync Push and Pull

## Decision

Project Sync has two user actions:

```text
Compare -> Apply
```

Compare reads only the data that it needs to classify current state. It does
not capture full session payloads. It does not build or save executable
conversation actions.

Apply is one backend command and one user-visible phase. Apply captures the
selected sessions, builds fresh pins and actions in memory, and applies them.
Internal checks remain part of Apply. There is no separate Validate command or
Commit command.

Project Sync does not persist Push plans, Restore plans, or a conversation
recovery journal. Project Sync does not automatically retry, replay, resume,
roll back, or roll forward an interrupted Apply.

## Constraints

- Use a clean cutover.
- Keep the machine-local schema at 1.
- Keep the portable bundle schema at 1.
- Do not add a migration.
- Do not add a legacy reader or execution path.
- Treat prior local and cloud test data as disposable.
- Keep the implementation small.
- Preserve immutable publication and the remote head compare-and-swap.
- Preserve canonical-path, no-follow, digest, and target checks.
- Preserve atomic replacement for an individual file.
- Preserve backups and result receipts.

The operator removes `~/.mallard` and test cloud data before clean-cutover
testing.

## Push

### Compare

Push Compare:

1. Loads project, profile, binding, checkpoint, and recipe metadata.
2. Discovers provider session paths.
3. Reads file metadata and bounded comparison evidence.
4. Reads the current storage head and manifest.
5. Classifies each conversation.
6. Returns every eligible session with its estimated capture bytes.
7. Returns the aggregate Apply limit and the non-conversation byte reserve.

Push Compare does not:

- read a complete transcript for capture;
- run full-payload redaction or secret scanning;
- calculate full-payload SHA-256 values;
- transform provider payloads;
- mint cloud conversation identities;
- build executable conversation actions;
- upload objects;
- update the remote head;
- save a Push plan; or
- create a conversation recovery journal.

For a provider codec without an incremental cursor, bounded comparison
evidence can include the complete transcript up to that codec's file-read
limit. Compare does not retain that content as a capture or build actions from
it.

### Selection

Every eligible session remains selectable. Age and list order do not affect
eligibility. There is no newest-first prefix and no hidden admission set.

The current aggregate limit is:

```text
512 MiB = 536,870,912 bytes
```

The review meter calculates:

```text
selected session bytes
+ selected project-file bytes
+ reserved provider-state bytes
```

The UI disables Apply when that total exceeds the limit. Apply calculates the
total again from current file metadata. The UI estimate is not authoritative.

The product UI formats the limit as `512 MB`. The code uses the binary value
shown above.

### Apply

Push Apply:

1. Loads current project, profile, binding, and storage state.
2. Rediscovers the selected resources.
3. Rechecks the storage head.
4. Recalculates the selected byte total.
5. Captures selected resources with a bounded worker pool.
6. Runs full-payload checks, analysis, hashing, and transformation.
7. Builds fresh conversation pins and actions in memory.
8. Builds one candidate manifest.
9. Uploads immutable objects and the immutable commit.
10. Advances the remote head once with compare-and-swap.
11. Updates normal bindings, checkpoints, receipts, and recipe state.

Push has one shared publication boundary. A capture or validation failure
before publication fails that Apply without advancing the head. A successful
head compare-and-swap is not rolled back if a later local metadata write fails.

Mallard makes no second publication attempt and saves no state for a later
replay.

## Pull

### Compare

Pull Compare:

1. Loads project, profile, binding, checkpoint, and recipe metadata.
2. Discovers provider session paths.
3. Reads file metadata and bounded comparison evidence.
4. Reads the current storage head and manifest.
5. Builds review rows for files and conversations.
6. Returns ephemeral selection IDs with the review.
7. Returns each row's storage-side size (every file the manifest lists for a
   stored session; a file write's own size) and the aggregate Apply limit.

Pull Compare does not:

- capture complete local transcripts;
- download complete remote transcripts for materialization;
- derive final destination identities;
- build final conversation target pins;
- save a Restore plan; or
- create a conversation recovery journal.

### Selection

Every row stays selectable. The review meter calculates:

```text
selected session bytes (as stored)
+ selected setup and tools file bytes
+ selected project-file bytes
```

Nothing is reserved: a Pull carries nothing it does not show as a row. The UI
disables Apply past the same 512 MiB limit Push uses. The sessions step's
recommended selection is the newest sessions that fit, keyed on the stored
session's last-message time, after the safe setup and tools rows and
whatever the files step has ticked (`PLAN_PULL_APPLY_LIMIT.md`).

### Apply

Pull Apply:

1. Validates the in-memory review anchor.
2. Rechecks the current storage head.
3. Builds a fresh schema-1 `RestorePlan` value in memory for the existing
   restore engine, and refuses the whole Apply before its first write when
   the approved selection, priced again from the pinned manifest, exceeds
   the limit.
4. Captures only selected local counterparts with bounded workers.
5. Builds fresh conversation actions and target pins.
6. Downloads only selected remote payloads.
7. Applies each selected conversation and records its result.
8. Serializes updates to shared provider indexes, bindings, and configuration.
9. Applies approved project-file actions through the existing lazy restore
   engine.
10. Saves normal receipts and completed materialization history.

Conversation preparation and materialization failures are reported per action
where the current engine can safely continue. A successful sibling action is
not rolled back. Shared document failures remain operation-level failures.

Keep Both mints the preserved identity and builds its restore targets during
Apply. It does not need a capture-time plan or recovery journal.

## One Apply phase

There is no global Validate/Commit protocol.

The code can use ordered internal stages inside one Apply command:

```text
recheck -> capture -> validate -> build -> write -> report
```

These stages are implementation details. They are not separate user actions,
commands, persisted phases, or resumable states.

The bounded worker count is:

```text
min(available parallelism, configured comparison-worker cap, selected count)
```

The default cap is four. Selected-resource capture runs full reads, scans,
hashes, and capture-time analysis in the bounded worker pool. Pull prefetches
remote payloads in bounded transfer batches. Writes that replace the same
shared JSON document remain serialized.

## In-memory schema-1 values

The clean cutover keeps the current schema-1 DTO shapes.

- Pull Compare still returns a `RestorePlan`-shaped review value.
- The review can contain a `plan_id` and selection `action_id` values.
- These IDs correlate the open UI review and its result only.
- The frontend sends the complete in-memory review and the selected action IDs
  to Apply.
- Apply does not load a plan by ID.
- Apply builds new executable pins and actions from current state.
- No review or executable plan is written to `~/.mallard`.

Push uses the reviewed `BundleRecipe` selection and current comparison
context. It does not create or persist a `ConversationPushPlan`.

Ephemeral `ConversationApplyOperation`,
`ConversationPublicationApply`, and `ConversationMaterializationApply`
values remain because they are the smallest inputs to the existing safety
helpers. They exist for one command only and are never persisted.

## Removed persistence

The Project Sync path no longer reads or writes:

```text
restore_plans/
conversation_push_plans/
conversation_sync_journal.json
conversation_operations/
```

The implementation also removes:

- plan save and load methods;
- plan discard methods;
- conversation journal mutation methods;
- startup conversation Apply recovery;
- conversation Apply replay;
- conversation Apply resume;
- conversation Apply rollback and roll-forward; and
- journal failpoint tests.

Setup and host-identity rotation transactions are separate from Push and Pull.
Their recovery records remain. They do not contain conversation Apply actions.

Normal state remains:

- configuration;
- project and provider bindings;
- conversation bindings;
- accepted checkpoints;
- object cache data;
- backups;
- Apply receipts;
- materialization result history; and
- immutable cloud objects and heads.

Receipt and materialization IDs are result correlation. They are not
executable plans and are not replay inputs.

## Failure behavior

Comparison can fail only when Mallard cannot produce a safe comparison. For
example:

- invalid configuration or binding metadata;
- an unsafe or ambiguous path;
- an invalid storage marker, head, or manifest;
- ambiguous identity; or
- unsupported schema data.

Full-payload and write failures occur during Apply. For example:

- a selected file changed or disappeared;
- the current selected total exceeds 512 MiB;
- a full read fails;
- a secret or unsupported payload is found;
- transcript analysis or transformation fails;
- a target changed;
- a digest does not match;
- an identity collides;
- the storage head compare-and-swap fails; or
- a local or remote write fails.

After any failure, Mallard returns the current result. It does not start an
automatic retry. The next attempt is:

```text
new Compare -> new selection -> new Apply
```

The next attempt does not resume the prior Apply.

## Crash trade-off

Removing durable plans and the recovery journal removes crash recovery for
conversation Apply.

| Event | Result |
| --- | --- |
| Failure before the first write | No selected content is changed |
| One Pull conversation succeeds | That success remains |
| A later Pull conversation fails | The failure is reported; prior success remains |
| Push fails before the head compare-and-swap | The remote head does not advance |
| Push head advances and a local metadata write fails | The head remains advanced |
| The process stops | The operation can remain partial |
| Mallard starts again | Mallard does not replay the operation |
| The user tries again | The user must run a new Compare and Apply |

Atomic file replacement prevents a torn individual file. The remote
compare-and-swap prevents a lost head update. Neither mechanism makes the
whole Apply atomic or recoverable.

## Performance

### Complexity

Compare cost is:

```text
O(discovered metadata + bounded tail probes + manifest metadata)
```

Compare no longer has an `O(total selected payload bytes)` capture cost.

Apply cost is:

```text
O(total selected bytes + provider analysis + transport)
```

Push currently retains selected captured payloads in memory until candidate
assembly completes. Peak retained payload memory is therefore
`O(selected bytes)`, bounded by 512 MiB, plus worker and manifest overhead.

Pull streams remote objects through the existing byte source, but local
counterpart capture and provider transformations add bounded working memory.

### Measured local baseline

The implementation baseline measured capture, analysis, and hashing at about
39.5 MiB/s on the test machine. It measured a fresh discovery pass at about
3.55 seconds.

| Selected payload | Capture-only estimate |
| ---: | ---: |
| 50 MiB | 1.3 s |
| 100 MiB | 2.5 s |
| 250 MiB | 6.3 s |
| 500 MiB | 12.7 s |
| 512 MiB | 13.0 s |

For a 500 MiB Push, the measured local pre-upload estimate is approximately:

```text
3.55 s discovery + 12.7 s capture = 16.25 s
```

Network transfer, cloud latency, provider transformation, disk speed, and
content shape are additional. Removing plan and journal writes saves only
small JSON writes; payload reads, hashing, transformation, and network
transfer dominate.

There is no automatic retry amplification.

## Verification requirements

Run:

```text
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
npm run test:release-config
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The last two commands make paid provider calls.

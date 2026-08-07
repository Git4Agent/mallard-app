# Plan: High-severity current-implementation remediation

## Status

- **State:** proposed; no product code has been changed by this plan
- **Scope:** H-01 through H-12 from the
  [July 27, 2026 implementation audit](research/current-implementation-bug-audit-2026-07-27.md)
- **Audit snapshot:** commit `ab8b7d0` plus the uncommitted working-tree changes
  present during the audit
- **Target product path:** `ProjectSyncV3`, machine-local schema 3, and portable
  bundle schema 4

This plan closes every high-severity finding. H-10 is included even though the
sanitizer defect's production exploitability depends on an upstream error
containing an authorization value.

## Outcome

After this plan is complete:

- Push publishes only bytes and modes that were part of the accepted review.
- A remote manifest cannot disguise project content, executable content, or a
  dependency as a safer resource type.
- Restore and repair never overwrite or delete a concurrently replaced file.
- Every accepted skill tree is completely classified, backed up, and verified.
- A storage ID cannot change meaning during review, Push, Pull, or Apply.
- Full configuration saves and machine bindings cannot commit mutually invalid
  state.
- S3 response limits bound memory while bytes are arriving, not after buffering.
- Authorization values are removed as whole values before logs or errors are
  persisted or displayed.
- Pull renders only the immutable plan that Apply will execute.
- Explicit consent is tied to an exact plan and action fingerprint, never just
  a resource ID.

## Scope boundary

This plan covers all twelve `H-*` findings. The audit's medium and low findings
remain separate work. Two narrow adjacent changes are included because the
high-severity contract cannot be changed safely without them:

- synchronizing the missing TypeScript `conversation_actions` field while plan
  schema 2 is introduced; and
- making H-10's sanitizer reusable so later path-redaction work does not create
  a competing logging boundary.

Case-only renames remain unsupported under bundle schema 4. This plan blocks
them safely; it does not invent a legacy or implicit rename protocol.

## Non-negotiable design rules

1. **Fail closed.** A changed file, unsupported atomic primitive, traversal
   overflow, stale plan, storage mismatch, or malformed manifest stops before
   publication or mutation.
2. **No check-then-path-mutate fallback.** An immediate second `stat` narrows a
   race but does not fix it. Target mutation must use no-follow handles,
   exclusive rename/create operations, and recoverable quarantine.
3. **One limit means one contract.** Discovery, capture, staging, target
   classification, backup, and verification use the same skill-tree limits and
   the same overflow-detecting walker.
4. **Plans are authorization records.** Live browsing data may provide
   navigation elsewhere in the app, but it cannot supply Pull review rows or
   approvals.
5. **Portable data remains portable.** Storage fingerprints, credentials,
   absolute paths, backup locations, journals, and approval state remain
   machine-local.
6. **No legacy safety bypass.** Malformed existing manifests are rejected with
   recovery guidance. Old short-lived local plans are discarded and rebuilt.
7. **Tests control interleavings.** Concurrency coverage uses deterministic
   barriers or failpoints, never sleeps or probabilistic races.

## Dependency map

| Foundation | Findings that depend on it | Reason |
| --- | --- | --- |
| Manifest semantic validation | H-02, H-03, H-12 | Action identity must be computed from already-valid resource semantics. |
| Checked filesystem transactions | H-01, H-04, H-05, H-06 | Capture, restore, backup, deletion, and repair need the same no-follow guarantees. |
| Bounded tree traversal | H-04 | Every skill-tree reader must agree on limits and digest shape. |
| Cross-document repository transaction and lock order | H-07, H-08 | Storage mutation and machine-state validation must share one serialization model. |
| Plan schema 2 | H-07, H-11, H-12 | Storage identity, review rows, approval reasons, and action fingerprints should change in one local-plan contract revision. |
| Bounded streaming and shared sensitive-text sanitization | H-09, H-10 | Remote bytes and remote error text must be constrained before higher layers receive them. |

## Shared backend foundations

### Checked filesystem module

Add `src-tauri/src/project_sync_v3/safe_fs.rs` and expose it only within
`project_sync_v3`.

The core read result is:

```rust
struct FileSnapshot {
    identity: FileIdentity,
    sha256: String,
    len: u64,
    mode: u32,
    modified_secs: u64,
}
```

The module must provide:

- a bounded, no-follow `read_regular_file_snapshot(root, path, limit)` that
  returns the bytes and snapshot from one opened object;
- `replace_checked`, `delete_checked`, and `delete_empty_directory_checked`;
- an exclusive `rename_noreplace`/create primitive;
- durable same-parent staging and quarantine; and
- recovery records for an interrupted checked mutation.

A checked mutation follows this order:

1. Open and snapshot the expected object, or record that the target is absent.
2. Prepare and `fsync` the incoming bytes and any required backup metadata.
3. Move an existing target to a unique same-parent quarantine without replacing
   anything.
4. Verify that the quarantined object is exactly the object authorized by the
   plan.
5. Promote that same displaced object to the durable same-filesystem backup;
   if a second backup copy is required, verify it but retain the displaced
   object as well.
6. Activate staged content with a no-replace operation.
7. Post-verify the active target and mark the journal complete.
8. Keep the displaced object linked at its recovery location for the configured
   backup-retention period; immediate apply cleanup never unlinks it.

If another process creates a new target during the transaction, Mallard must
not overwrite it. It preserves the quarantine/backup, reports a recovery
conflict, and leaves no Applied receipt.

If the object moved to quarantine does not match the authorized snapshot,
Mallard immediately restores that exact object to the original vacant path with
no-replace semantics. If the path has already become occupied, it preserves
both objects, reports the quarantine as recovery-required state, and never
chooses one by overwriting the other.

On Unix, a process can continue writing through an open handle after a rename.
For that reason, a copied snapshot is not a substitute for the displaced
object. The displaced inode/tree itself remains reachable as the authoritative
backup, so late writes remain recoverable. On Windows, sharing rules may make
the rename fail; that failure aborts before activation.

Use a reserved opaque same-parent recovery name so the rename cannot cross a
filesystem. Provider and project discovery hard-exclude this reserved prefix,
and restore planning never treats it as user or portable content. The UI refers
to its opaque recovery ID, not its absolute path.

Platform implementations:

- Unix uses parent-directory descriptors with `openat`/`fstatat` and
  `AT_SYMLINK_NOFOLLOW`.
- Linux uses `renameat2(RENAME_NOREPLACE)` where available.
- macOS uses `renameatx_np(RENAME_EXCL)` or a proven equivalent.
- Windows opens with reparse-point protection, compares `FileIdInfo`, and uses
  a fail-if-exists rename.
- An unsupported platform returns a clear unsupported-safety error; it never
  falls back to `fs::read`, `fs::rename`, or `remove_file` after a pathname
  check.

The read path rejects symlinks and special files, applies the byte bound while
streaming, and verifies handle metadata before and after the read. File identity
is an enforcement detail, not portable identity: a same-byte atomic
replacement may be accepted only when every reviewed semantic field is
identical.

Every relative component is resolved from an already-open authorized root with
no-follow descriptor-relative opens. Protecting only the leaf is insufficient
because an ancestor directory can also be replaced. Every journal phase that
renames or creates an object `fsync`s the affected file and parent directory
before advancing.

### Bounded tree module

Add `src-tauri/src/project_sync_v3/bounded_tree.rs` with:

```rust
struct TreeLimits {
    max_files: usize,
    max_entries: usize,
    max_depth: usize,
    max_file_bytes: u64,
    max_bytes: u64,
}
```

Use the already-published capture contract as the production skill limit:

- 20,000 regular files;
- at most `max_files * (max_depth + 1)` visited entries (660,000 with the
  production values);
- depth 32;
- 16 MiB per regular file; and
- 512 MiB aggregate bytes.

The walker must:

- be deterministic and no-follow;
- inspect one entry/level beyond each limit so overflow is an error rather than
  silent truncation;
- reject special files;
- bound directories/total visited entries as well as files, without rejecting a
  tree the current publisher can produce;
- use the same relative-component depth convention, path validation, and
  case-fold collision rules as capture;
- produce one canonical digest over sorted relative path, executable bit, and
  content digest; and
- support injected small limits for fast unit tests.

Discovery, capture, staging verification, live-target classification, backup,
post-apply verification, and recovery all call this module.

### Recovery journals and failpoints

Checked target and skill transactions persist a bounded journal below the
plan's machine-local backup directory. Phases are:

`staged -> displaced -> backed_up -> activated -> verified -> complete`.

Startup or the next Apply reconciles incomplete phases. Immediate transaction
cleanup never removes the displaced backup. Later retention cleanup is a
separate, visible backup-lifecycle operation and must not claim a stale digest
if an old open handle changed the retained object.

Journals contain only plan/action IDs, validated relative target keys, digests,
modes, phases, and opaque recovery IDs. They do not contain credentials,
transcript bytes, portable metadata, or local absolute paths.

Add test-only no-op-in-production interfaces:

- `CaptureFailpoints`
- `RestoreFailpoints`
- `RepairFailpoints`
- configuration/storage transaction barriers

The hooks are placed immediately around the protected check/use boundaries
listed in the finding plans below.

## Local contract changes

Bundle manifest schema 4 does not change. Restore and dependency plans are
machine-local, short-lived authorization records and move together to schema 2.
Schema-1 plans are rejected as stale and replanned; there is no compatibility
apply path.

### Restore plan schema 2

Add:

- `storage_fingerprint`;
- validated `review_items`;
- typed `approval_reasons` on every restore and conversation action; and
- `action_fingerprint` on every restore and conversation action.

`PlanReviewItem` is a bounded, domain-owned projection of the pinned manifest
and final action set. It includes the resource ID, typed category, display name,
optional provider, bounded detail, relevant logical paths/source timestamp, and
the action IDs it represents. Validation requires:

- every action to appear in exactly one review item;
- no extra action IDs;
- matching resource IDs;
- category/provider agreement with the action and descriptor; and
- bounded strings, paths, rows, and total serialized size.

The projection is canonical approval data, not decoration. Every visible field
and its sorted action-ID membership is encoded into the corresponding action
fingerprints. Fields that come from a descriptor are copied from the pinned
manifest during planning and included in that encoding. Plan validation
recomputes the canonical review projection/fingerprint and rejects any changed
label, detail, path, category, provider, timestamp, operation label, or action
membership.

Do not embed live `ProjectChatHistory`, `ThreadSyncComparison`, or
`CapabilityStatusReport` DTOs in the domain plan.

### Dependency plan schema 2

Add the same storage pin, typed approval reasons, and action fingerprints.
Existing dependency action display/provider/kind fields are sufficient to form
their immutable review rows.

### Action fingerprint

Compute SHA-256 from a versioned, length-delimited encoding, never `Debug`
formatting or incidental JSON order. Include:

- storage ID and operation fingerprint, bundle, host, replica, host-identity
  revision, bundle revision, commit, manifest, and binding pins;
- action ID and resource ID;
- every tagged action-kind field;
- for `ConversationPlanAction`, direction, action/reason/relation, provider,
  thread/result/source identities, codec and lineage pins, every local/provider
  target expectation, every document/profile revision, and expiry;
- target path;
- source, materialized, expected-target, and deletion digests;
- expected mode, source mode, and source time where applicable;
- dependency display name, provider, program, and complete argument vector; and
- every canonical approval-visible field and sorted action-ID membership of
  the action's `PlanReviewItem`; and
- the ordered typed approval reasons.

Exclude only the random plan ID from the digest. Authorization identity is the
pair `(plan_id, action_fingerprint)`. Recomputing and validating the fingerprint
inside `RestorePlan::validate` and `DependencyPlan::validate` detects any
approval-relevant mutation.

Use a closed, tagged `ApprovalReason` enum rather than display strings. Its
initial variants cover:

- project-content add, replace, delete, and executable content;
- overwrite of an existing provider-state file;
- conversation replace/conflict;
- custom-skill install and overwrite;
- plugin or standalone-skill installation; and
- manual resolution.

Reasons are deduplicated and encoded in canonical enum order. The existing
stored `requires_explicit_approval` boolean is removed from schema 2 so there is
only one approval authority. Rust and TypeScript derive
`requiresExplicitApproval` from `!approval_reasons.is_empty()`. A temporary
conversion commit may mirror both representations for compilation, but that
dual form must not merge or be persisted.

Change restore, conversation, and dependency Apply payloads from bare IDs to
bounded `ApprovedPlanAction { action_id, action_fingerprint }` lists. Before
any filesystem, network, or installer side effect, the backend rejects
unknown, duplicate, or mismatched pairs and derives its internal selected-ID
set.

Update Rust DTOs, `src/types.ts`, `src/components/project-sync/api.ts`, and
frontend call sites together. While synchronizing the contract, add the
currently serialized `conversation_actions` field to the TypeScript
`RestorePlan` definition so the mirror remains complete.

### Repair result

H-06 adds per-file results to the existing conversation-repair response:

```text
file_results[] = { thread_id, status: repaired | changed | failed, message? }
partial: boolean
```

Keep the existing repaired IDs and backup-directory fields for callers. These
are additive Tauri response fields and do not change a persistent schema.

## Finding-by-finding implementation

### H-01 — Bind discovery and review to captured bytes

**Files**

- `src-tauri/src/project_sync_v3/provider_capture.rs`
- `src-tauri/src/project_sync_v3/commands.rs`
- `src-tauri/src/project_sync_v3/global_inventory.rs`
- shared `safe_fs.rs`

**Changes**

1. Replace the implicit live/derived distinction on `SourceFile` with a sealed
   source:

   ```rust
   enum SourceMaterial {
       Derived(Vec<u8>),
       Live(FileSnapshot),
   }
   ```

2. Record a snapshot for source directories as well.
3. Make JSONL identity parsing consume the same bounded byte snapshot that is
   sealed for capture. Session ID, working directory, digest, and captured
   bytes can no longer come from different file versions.
4. Preserve project-file scan digest, length, mode, warning classification, and
   executable classification in the discovery snapshot rather than discarding
   them.
5. Carry the validated `SKILL.md` digest/effective name from global inventory
   into standalone-skill discovery and revalidate it during capture.
6. During capture, read each live source through `safe_fs`, compare all reviewed
   fields to the sealed snapshot, and only then accept its bytes. Derived bytes
   are immutable and are not re-read.
7. On any mismatch, return a typed changed-after-review error. Commands map it
   to “Project or provider files changed; rescan and review,” publish no
   objects, and do not advance the head.

**Deterministic tests**

- `capture_rejects_project_file_changed_after_discovery`
- `capture_rejects_mode_changed_after_discovery`
- `capture_rejects_conversation_identity_changed_after_discovery`
- `capture_rejects_skill_declaration_changed_after_inventory`
- `push_rejects_mutation_after_review_token_validation`

`CaptureFailpoint::AfterReviewValidatedBeforeCapture` atomically replaces a
reviewed file with secret-shaped, private-key, executable, and
conversation-reassignment variants. Every case must leave the remote head,
revision, and object set unchanged.

**Done when**

Every captured live byte and mode matches its discovery snapshot, and ownership
parsing is performed on those same bytes.

### H-02 — Enforce a semantic matrix for every manifest resource

**Files**

- `src-tauri/src/project_sync_v3/domain.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`

**Domain changes**

Add `LogicalNamespace` and one manifest-level
`validate_resource_semantics(...)` call after file/directory ownership maps are
complete.

First reconcile the parser with the repository's sole production contract:
`BundleHead` and `BundleManifest` accept schema 4 only. The current working
tree's schema-6 acceptance must not leave a second unvalidated restore path. If
schema 6 is intentionally promoted in a separately approved product change,
that schema needs its own exhaustive matrix and adversarial tests before the
parser accepts it; it is not a compatibility bypass for this plan.

The initial exhaustive matrix is:

| Logical namespace | Required resource semantics |
| --- | --- |
| `project/*` | `ProjectContentFile` or `ProjectContentDirectory`; no provider; Project scope; matching `ProjectLocal` provenance; `ExplicitReview`; exactly one matching file or directory. |
| `state/codex/sessions/*`, `state/codex/archived_sessions/*` | `CodexConversation`; Codex; ProviderState; unknown provenance; `SafeFile`. |
| exact `state/codex/session_index.jsonl` | `CodexConversation`; Codex; ProviderState; unknown provenance; `Merge`; exact single payload. |
| `state/claude/projects/*`, `state/claude/file-history/*`, `state/claude/todos/*` | `ClaudeConversation`; Claude; ProviderState; unknown provenance; `SafeFile`. |
| `state/claude/memory/*` | `ProjectMemory`; Claude; ProviderState; unknown provenance; `Merge`. |
| `state/{provider}/skills/{install_dir}/*` | `StandaloneSkill`; matching provider; ProviderState; `StandaloneSnapshot`; `SafeFile` or `ExplicitInstall`; root matches validated install-directory metadata. |
| no payload | `Plugin`; matching provider; Dependency scope; matching Plugin provenance; `ExplicitInstall`; zero files/directories. |
| no portable form | `Requirement` is inventory-only and is invalid in a manifest. |

Also enforce:

- recipe policy equals descriptor policy;
- no materializable path belongs to an unknown namespace;
- namespace, provider, kind, scope, provenance, file count, and directory count
  agree; and
- executable mode is allowed only where the matrix and explicit-review/install
  policy permit it.

Reject structural checkout metadata such as `.git`, `.svn`, and `.hg` below
`project/` even if a forged descriptor claims another kind.

**Restore defense in depth**

- Derive project actions from the `project/` namespace, not the claimed
  descriptor kind.
- Pass logical path and mode into approval classification.
- Require explicit approval for every project path and executable materialized
  entry.
- Recheck namespace/descriptor agreement when mapping a logical target.
- Ensure the Git-managed checkout guard activates for every `project/*` action.

**Tests**

- one accepted fixture for every publisher-emitted matrix row;
- one rejected mutation for kind, provider, scope, provenance, policy,
  namespace, skill root, executable mode, and payload count;
- `manifest_rejects_project_path_disguised_as_safe_resource`;
- `manifest_rejects_recipe_descriptor_policy_mismatch`;
- `manifest_rejects_executable_safe_state_file`; and
- a forged `project/.git/hooks/pre-commit` flow that never yields an implicit
  `WriteFile` and never touches the checkout.

**Compatibility**

No bundle schema bump. Valid schema-4 current-client bundles continue to
validate; nonproduction schema values are rejected before restore. Malformed
heads fail with the offending resource/path/dimension and guidance to republish
from a trusted current client. Run every committed schema-4 manifest fixture
through the matrix before merge.

### H-03 — Reject case-fold collisions involving tombstones

**Files**

- `src-tauri/src/project_sync_v3/domain.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`

**Changes**

1. Centralize the current lowercase portable key as `portable_path_key`.
2. Add every file and directory tombstone to the same folded-path collision
   map used for live files and directories.
3. Reject live/tombstone, tombstone/tombstone, file/directory, and cross-kind
   collisions even when their exact strings differ.
4. Detect a case-only rename during reconciliation and report that schema 4
   cannot represent it safely. The user must keep the previous spelling or
   choose a final name that differs by more than case.
5. Do not silently discard the tombstone and do not add a schema-4 rename
   action. A future typed atomic rename is outside this fix.

**Tests**

- `manifest_rejects_casefold_live_file_tombstone_collision`
- `manifest_rejects_casefold_live_directory_tombstone_collision`
- `manifest_rejects_casefold_tombstone_tombstone_collision`
- `publication_rejects_case_only_project_rename_as_unsupported`
- `restore_never_schedules_casefold_colliding_delete`

These are string-semantic tests and must pass identically on case-sensitive and
case-insensitive test filesystems.

### H-04 — Make skill traversal, verification, and backup complete

**Files**

- `src-tauri/src/project_sync_v3/provider_capture.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`
- shared `bounded_tree.rs` and `safe_fs.rs`

**Changes**

1. Remove the 4,096-file/depth-16 verifier constants and every silent
   `WalkDir::max_depth(16)` path.
2. Use the shared production tree limits and canonical digest in discovery,
   capture, staged verification, target state, backup, post-verification, and
   recovery.
3. Stage and verify the incoming tree before touching the target.
4. Atomically move the old target to quarantine, then fully classify the
   quarantined tree against the plan's expected digest.
5. Promote that displaced tree itself to the authoritative retained backup. If
   policy also creates a copy below the plan backup, require its digest to
   equal the classified target and retain the displaced tree too.
6. Activate staging with no-replace, post-verify it, then complete the journal.
7. Any depth/file/byte overflow blocks before displacement. Any later failure
   leaves the old tree recoverable.

Renaming a directory does not make it immutable: another process may retain an
open handle or working directory. Read every quarantined file through
`safe_fs`, verify each handle before/after copy, enumerate and digest the tree
again after backup, and require the entry set, identities, modes, and digests to
match both the approved target and verified backup. A mismatch prevents
activation and triggers no-replace restoration of quarantine when the original
path remains vacant. After activation, keep the displaced tree reachable for
backup retention because an old open directory/file handle can still write to
it.

**Tests**

- `skill_target_depth_32_is_classified_and_backed_up`
- `skill_target_depth_33_fails_before_displacement`
- `skill_target_20000_files_is_supported`
- `skill_target_file_limit_overflow_fails_closed`
- `skill_backup_digest_covers_deep_local_only_file`
- `published_skill_with_contract_limits_restores`
- `skill_quarantine_mutation_blocks_activation_and_restores_or_preserves_target`
- `skill_open_handle_write_after_activation_remains_in_retained_backup`
- fail at `AfterSkillDisplacedBeforeBackupVerified` and prove the old tree is
  recoverable and staging is inactive

Use injected small limits for most unit cases and keep at least one contract
boundary integration test.

**Compatibility**

Raising restore/backup traversal to the existing capture contract makes
previously publishable 4,097–20,000-file and depth-17–32 skills restorable.
The aggregate byte bound remains 512 MiB.

### H-05 — Replace pathname TOCTOUs with checked target transactions

**Files**

- `src-tauri/src/project_sync_v3/bundle_engine.rs`
- shared `safe_fs.rs`

**Changes**

Route all of these through the checked transaction:

- generic and project file writes;
- conversation materialization writes;
- project-file deletion;
- exact-empty project-directory deletion; and
- skill activation/displacement.

For an expected existing target, the quarantined object must match the plan's
digest, mode, and opened identity before Mallard uses it as the backup or
deletes it. For an expected absent target, activation is no-clobber. Directory
deletion moves only the exact directory and finishes with non-recursive
`remove_dir`; if anything appears inside, deletion fails and preserves it.

A post-displacement digest/identity mismatch is not allowed to leave the
unexpected concurrent object missing from its pathname: restore it immediately
with no-replace when vacant, or retain and report it if another object now
occupies the path. A successful activation likewise retains the displaced
object itself as the authoritative backup; it does not unlink it merely because
an earlier copied digest matched.

Add recovery-journal reconciliation before accepting another Apply for the same
target. Receipts are Applied only after activation/deletion and final
verification are durable.

**Failpoints and tests**

- `AfterTargetOpened`
- `AfterBackupDurable`
- `AfterTargetDisplaced`
- `BeforeActivation`
- `BeforeDeleteDisplace`

At each hook, atomically replace or create the target and prove:

- `restore_does_not_overwrite_replacement_after_check`;
- `restore_missing_target_does_not_clobber_concurrent_create`;
- `delete_preserves_replacement_after_digest_check`;
- `delete_backup_matches_exact_displaced_object`;
- `mismatched_displaced_target_is_restored_with_no_replace`;
- `open_handle_write_after_file_activation_remains_in_retained_backup`;
- `open_handle_write_after_directory_activation_remains_in_retained_backup`;
- `restore_rejects_symlink_swap`; and
- `restore_recovers_interrupted_checked_transaction`.

Concurrent bytes must remain either at the target or in a reported recovery
artifact; they must never disappear.

### H-06 — Make conversation repair independently commit-safe

**Files**

- `src-tauri/src/project_sync_v3/commands.rs`
- `src/types.ts`
- `src/components/project-sync/ProjectSyncV3.tsx`
- `tests/frontend/conversation-path-repair.integration.test.tsx`
- shared `safe_fs.rs`

**Changes**

1. Treat each transcript as an independent checked transaction with original
   and repaired digests.
2. Persist its prepared journal/backup before commit.
3. Once its repaired bytes are verified, mark that file committed. A later file
   or final-audit failure does not roll it back.
4. On retry:
   - current digest equals repaired: already complete;
   - current digest equals original: safe to attempt repair;
   - current digest equals neither: preserve it and report `changed`.
5. Continue independent files where safe, refresh the audit, and return the
   per-file/partial result.
6. Show “N repaired, M changed—review and retry” rather than an all-success
   notice or an opaque batch error.

If product requirements retain batch rollback, the minimum acceptable fallback
is conditional rollback only when the current digest exactly equals the
repaired digest. Per-file commit is preferred because transcripts are
independent and Codex is an uncoordinated writer.

**Tests**

- `repair_preserves_append_when_later_file_fails`
- `repair_partial_result_reports_committed_and_changed_files`
- `repair_retry_is_idempotent`
- `repair_journal_recovers_interrupted_commit`
- frontend partial-result notice and audit refresh

Use `AfterFileCommitted`, `BeforeNextFile`, and `BeforeFinalAudit` failpoints.
Append a valid JSONL turn after the first commit, force the later failure, and
prove the append survives.

### H-07 — Pin storage meaning throughout every operation

**Files**

- `src-tauri/src/project_sync_v3/domain.rs`
- `src-tauri/src/project_sync_v3/commands.rs`
- `src-tauri/src/project_sync_v3/persistence.rs`
- `src/types.ts`
- `src/components/project-sync/api.ts`
- Push/Pull orchestration components

**Storage fingerprint**

Add `StorageConfigV3::operation_fingerprint()`. Hash a manually
length-prefixed encoding headed by `mallard-storage-operation-v1`.

Include:

- ID and kind;
- local directory for local storage;
- bucket, access-key ID, secret-access-key bytes, account ID, endpoint, region,
  and conditional-write setting for S3; and
- exclusions and every other non-display field that changes operation
  behavior.

Exclude only the display name. Never log the preimage or fingerprint and never
place it in a portable manifest. Resolve the cloned config, fingerprint, and
engine together as one `ConfiguredStorage` so they cannot come from different
config loads.

**Push**

1. `PushReviewContext` returns the fingerprint used to prepare review.
2. The Push request includes `expectedStorageFingerprint`.
3. After acquiring the host-operation lock, Push rejects a mismatch before
   remote inspection.
4. Recheck immediately before publication.
5. Recheck inside final config mutation before recording recipe/base state.
6. A pre-publication mismatch performs no remote write. A post-publication
   mismatch records no local base and returns a specific “remote head may have
   advanced; local base was not associated” recovery result.

**Pull and dependencies**

Planning writes the fingerprint from the same `ConfiguredStorage` used to fetch
the snapshot into schema-2 restore/dependency plans. Restore Apply, dependency
planning, and dependency Apply compare it before fetching, filesystem
mutation, or installer execution.

**Tests**

- review on storage A, repoint the same ID to B, then reject before any B GET or
  PUT;
- pause after engine construction and prove a settings save is excluded by the
  host lock;
- bypass the normal lock in a test before publication and prove the defensive
  fingerprint check prevents a remote write;
- change storage after Pull planning and reject before target/installer side
  effects; and
- verify post-publication mismatch produces no false local base.

### H-08 — Save configuration against one machine-state snapshot

**Files**

- `src-tauri/src/project_sync_v3/persistence.rs`
- `src-tauri/src/project_sync_v3/commands.rs`

**Lock order**

Document and enforce:

1. host-operation lock;
2. persistence lock;
3. storage-engine/local-store lock.

Code holding the persistence lock must never attempt to acquire the host lock.

**Repository transaction**

Add a repository method equivalent to:

```rust
save_config_against_machine_state(
    submitted,
    validate_submitted_config_and_storage_isolation,
)
```

While one persistence guard is held it:

1. loads the current config and checks its submitted revision;
2. loads/creates host identity;
3. loads machine bindings/profiles, conversation bindings, materializations,
   and dependency-application receipts against the current config;
4. refuses to proceed while an unrecovered setup transaction could publish a
   competing config snapshot;
5. assigns schema/current revision + 1 and validates the submitted config;
6. revalidates every loaded config-dependent document against the submitted
   config, host identity, and machine state;
7. validates path/storage isolation using those same loaded bindings/profiles;
8. atomically writes config; and
9. releases the guard.

Replace the current load-bindings/validate/save sequence. Keep the weak
full-document `save_config` private or test-only so production callers cannot
bypass cross-document validation.

This is intentionally broader than checking bindings alone. Removing or
reidentifying a project/storage must not strand `materializations.json`,
conversation bindings, dependency applications, or a setup transaction. If
loading any required document fails, the config save fails without writing; it
does not quarantine or silently discard that state.

`save_project_sync_config` acquires the host-operation lock before the
transaction. Every project-root, provider-profile, or storage-path mutation
uses the same order and repeats its path probe after acquiring the lock.

**Tests**

- pause the transaction after it loads bindings, race a binding mutation, and
  prove the final documents validate together;
- run the reverse ordering;
- attempt removals referenced by conversation bindings, materializations,
  dependency applications, and an active setup transaction, and prove each is
  rejected without a config revision change;
- interleave project/profile/local-storage overlap changes without sleeps; and
- include full config save in
  `host_sensitive_workflows_share_the_host_operation_lock`.

### H-09 — Enforce S3 bounds while streaming

**Files**

- `src-tauri/src/project_sync_v3/s3_store.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`

**Bounded body collector**

Replace `ByteStream::collect()` with a helper that receives a limit and one
absolute deadline.

It must:

1. reject negative or over-limit declared lengths before reading;
2. reserve at most a small initial capacity, not the full untrusted length;
3. read one chunk at a time;
4. check `chunk_len > limit - accumulated` before extending the vector;
5. keep one total-body timeout rather than restarting it per chunk;
6. drop/cancel the body immediately on overflow or timeout; and
7. omit body bytes and credentials from errors.

Keep `MAX_FILE_BYTES` as the per-object ceiling. For content-addressed bundle
objects, add a bounded bulk-fetch contract that passes each validated
manifest-declared expected length alongside its key. Build a batch whose full
expected lengths total at most 512 MiB *before* spawning any transfer, and pass
that expected length as the collector's tighter limit. This avoids a
partial-reservation hold-and-wait deadlock while guaranteeing that all returned
buffers fit the aggregate budget. A dishonest body that exceeds its expected
length is cancelled even when it remains below `MAX_FILE_BYTES`.

Abort remaining tasks on the first failure. Prefetch batches are limited by
both six-object concurrency and expected aggregate size, with one
maximum-sized object allowed as a single-item batch. Single metadata/head reads
continue to use their existing tighter document limits or the per-object limit;
they do not join an unbounded concurrent group.

Make the transport limits explicit in the object-store boundary, for example
`get_bounded(key, max_bytes)` and
`get_many_bounded([(key, expected_bytes)])`, so S3 cannot allocate before the
bundle engine applies a later check. Local storage implements the same
contract. Move the publisher's 512-MiB aggregate capture limit into the domain
and validate the checked sum of all manifest file sizes against it. This also
bounds bytes retained across successive prefetch batches, not just one batch.

**Scripted streaming tests**

Use small injected limits:

- no content length, exact-limit chunks succeed;
- one byte beyond the limit fails before append;
- over-limit declared length fails before body production;
- dishonest small length plus large body still fails;
- slow chunks prove the timeout covers the entire body;
- producer drop proves overflow cancels the response;
- concurrent dishonest streams never exceed the precomputed aggregate budget
  and cannot deadlock on incremental reservations;
- successive prefetch batches plus the existing LRU cache remain within the
  operation and cache budgets until their `Arc` owners are dropped; and
- empty/exact-limit objects preserve bytes and ETag behavior.

### H-10 — Redact complete Authorization values

**Files**

- new `src-tauri/src/sensitive_text.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/activity_log.rs`
- `src-tauri/src/project_sync_v3/s3_store.rs`
- `src/components/project-sync/LogManagerDialog.tsx` if a migration-status
  response is added

**Sanitizer**

Run a dedicated header pass before generic marker redaction:

1. Recognize `Authorization` and `Proxy-Authorization` case-insensitively at
   token boundaries.
2. Accept `:` or `=` with arbitrary ASCII whitespace.
3. Support plaintext lines, JSON strings, and Rust/debug-map quoted values.
4. Replace an entire quoted value while honoring escaped quotes.
5. Replace an unquoted value through CR/LF, not merely the first
   whitespace-delimited word.
6. Preserve the header name and harmless surrounding structure.
7. Be idempotent.

Remove `authorization:` from the generic single-token loop. Retain bounded
query/form redaction for `x-amz-credential`, `x-amz-signature`, tokens,
passwords, and standalone Bearer tokens.

The activity logger remains the mandatory final boundary before persistence or
frontend emission. S3 also sanitizes SDK/body errors before returning them so a
raw value does not travel through other layers.

**Existing retained logs**

Introduce a one-time sanitizer-version migration for retained JSONL segments.
It parses each bounded entry, re-sanitizes every persisted message/context
string, writes a sibling temporary segment, `fsync`s it, and atomically
replaces the original. Malformed raw lines are dropped rather than retained as
potential secret text; the migration records only sanitized counts. It creates
no raw backup.

Rotate the active segment under a short writer lock, then scrub immutable
segments outside that lock one at a time. Queries do not expose an unsanitized
segment while migration is pending; new appends continue in a fresh
sanitizer-v2 segment. Persist the migration marker only after every retained
segment is replaced. Each replacement is independently atomic and records no
secret-bearing backup. On restart, the absence of the final marker causes a
full idempotent rescan: already-sanitized segments remain unchanged and pending
segments resume. The activity UI reports that older history is temporarily
unavailable instead of returning a partial segment as if it were complete.

**Table-driven tests**

Cover:

- Basic, Bearer, and AWS4-HMAC-SHA256 with Credential/SignedHeaders/Signature;
- Proxy Authorization;
- mixed case and whitespace;
- CRLF followed by a harmless header;
- JSON and Rust/debug quoted forms, including escaped quotes;
- query-style X-Amz-Credential and X-Amz-Signature with adjacent parameters;
- already-redacted input; and
- near matches such as `reauthorization`.

For each secret fixture, assert absence from the sanitizer result,
`ActivityLogEntry::new`, serialized JSONL, re-sanitized historical segment, and
S3 error text. Also assert sanitizer idempotence.

Add migration failpoints immediately before and after a segment replacement and
before/after the final marker write. Crash/restart tests prove the migration
resumes idempotently, never exposes an unsanitized or half-replaced segment,
and does not lose new sanitizer-v2 appends. A deterministic append/query
barrier test proves writers continue only in the new segment while queries
either return fully sanitized history or the explicit migration status.

### H-11 — Make the persisted plan the only Pull review data source

**Files**

- `src-tauri/src/project_sync_v3/domain.rs`
- restore/dependency planning in `bundle_engine.rs` and `commands.rs`
- `src/types.ts`
- `src/components/project-sync/pullReviewModel.ts`
- `src/components/project-sync/RestorePlanView.tsx`
- new pure plan-list components

**Changes**

1. Construct `RestorePlan.review_items` from the fetched manifest and final
   filtered action list before persisting the plan.
2. Build every conversation, skill, plugin, project-data, setup, and
   project-content Pull row from schema-2 plan data.
3. Remove `ProjectChatHistoryPage` and `SkillsPluginStatusPage` from Pull
   selection mode. They remain live browsing components outside Pull.
4. Render pure `PullConversationReviewList` and
   `PullCapabilityReviewList` components with no data-fetching effects.
5. Include every planned conversation, even when it is older than 30 days or
   outside the current branch/history browsing window.
6. Label the exact planned operation—restore, overwrite, install, delete,
   keep-local/manual—from action data.
7. Treat readiness as non-authoritative context. It cannot add mutation rows,
   change action identity, or auto-select anything.

There is one canonical builder for each row category. The same builder feeds
serialization, fingerprinting, and validation; components do not manufacture
display text from live data or a second interpretation of the action.

**Mounted tests**

- A plan containing an old/out-of-branch conversation, skill, and plugin shows
  every row.
- History, thread-comparison, and capability endpoints are instrumented to fail
  if called; Pull makes zero calls.
- Mutating live fixtures without changing the plan cannot change the review.
- Replacing the plan replaces all rows without mixing old/new data.
- Mutating any approval-visible row field without the corresponding canonical
  action data invalidates the plan; changing a legitimately fingerprinted
  visible field changes the review key and requires renewed consent.

### H-12 — Bind selection and result state to exact action fingerprints

**Files**

- `src-tauri/src/project_sync_v3/domain.rs`
- apply paths in `commands.rs`
- `src/components/project-sync/pullReviewModel.ts`
- `src/components/project-sync/pullReviewFlow.ts`
- `src/components/project-sync/RestorePlanView.tsx`
- `src/components/project-sync/ProjectSyncV3.tsx`
- `src/components/project-sync/api.ts`

**Selection model**

Give each merged review row a composite key from its sorted component action
fingerprints:

```text
review key = restore plan ID
             + resource ID
             + restore action fingerprints
             + dependency plan ID/fingerprints when present
```

Resource IDs remain presentation/grouping keys only.

Rules:

- A restore plan-ID change discards every explicit decision and initializes the
  new plan from defaults.
- Safe implicit actions may default selected; explicit actions start unselected.
- Dependency support arriving under the same restore plan preserves only rows
  whose composite key is unchanged.
- If a selected skill gains or changes an installer action, its key changes and
  it requires confirmation.
- An exact same-plan/same-fingerprint failed action may remain selected for
  retry.
- Completion and failure ledgers store plan ID plus fingerprint. They never
  suppress or decorate a new action merely because its resource ID matches.

Selection construction emits exact `(action_id, action_fingerprint)` pairs.
The backend validates every pair before either restore or dependency side
effects.

**Mounted test harness**

The current server-rendered Pull tests cannot exercise effects or rerenders.
Add `jsdom`, `@testing-library/react`, and
`@testing-library/user-event`, a bounded shared DOM setup, and register
`pull-review-mounted.integration.test.tsx` in the frontend integration runner.

Test a table where plan N's explicit item is selected, then plan N+1 retains the
resource/action ID but changes one of:

- plan ID;
- action kind;
- target path;
- source/materialized/expected digest;
- target mode; or
- approval reason.

Every changed authorization identity clears the selection and is absent from
Apply. Also prove:

- same plan/fingerprint preserves selection;
- new implicit defaults initialize correctly;
- a newly added dependency component clears the affected row only;
- stale completed/failed state does not transfer; and
- empty/nonempty typed approval reasons are the sole source of default versus
  explicit behavior; and
- backend rejects unknown, duplicate, or mismatched fingerprint pairs before a
  fake engine/runner records a side effect.

## Delivery sequence

### Tranche 1 — Immediate fail-closed validation

1. H-02 semantic matrix.
2. H-03 folded tombstone collision rejection.

This tranche is pure validation/planning behavior, has no migration, and
immediately blocks the forged-manifest and destructive case-only paths.

### Tranche 2 — Remote input and secret boundaries

1. H-09 bounded S3 body and aggregate budget.
2. H-10 shared authorization sanitizer and historical-log scrub.

These changes are independent of filesystem transaction work and can proceed
in parallel with it.

### Tranche 3 — Filesystem substrate

1. Land `safe_fs`, `bounded_tree`, recovery-journal primitives, and
   platform-specific tests without switching production callers.
2. H-05 moves all restore/deletion mutations to checked transactions.
3. H-04 moves skill handling to the shared tree/transaction contract.
4. H-01 seals discovery/capture.
5. H-06 moves repair to independent checked commits.

Do not merge a caller conversion that retains an unprotected fallback.

### Tranche 4 — Configuration, storage, and Pull authorization

Treat this as one coordinated schema-2 merge group:

1. Finalize the schema-2 domain shape with storage pins, review items,
   approval reasons, and action fingerprints.
2. Implement H-08's cross-document repository transaction and lock order.
3. Implement H-07's storage serialization/fingerprint checks.
4. Change Apply APIs to fingerprint pairs.
5. Implement H-11 pure plan-derived Pull views.
6. Implement H-12 fingerprint-keyed selection/result state and mounted tests.

Stacked commits are appropriate, but do not release a partially converted
Rust/TypeScript/apply contract.

## Suggested reviewable change sets

| Change set | Findings | Principal risk |
| --- | --- | --- |
| Manifest semantic firewall | H-02, H-03 | Rejecting a valid publisher-emitted shape; mitigate with fixture matrix. |
| Bounded S3 reader | H-09 | Cancellation and aggregate batch-budget correctness. |
| Sensitive-text boundary | H-10 | Parser boundary mistakes and historical-log rewrite safety. |
| Checked filesystem substrate | foundation | Platform atomic-operation parity. |
| Guarded restore and delete | H-05 | Recovery after displacement. |
| Complete skill transaction | H-04 | Large-tree performance and backup completeness. |
| Sealed capture | H-01 | False stale-review failures during active provider writes. |
| Independent history repair | H-06 | Clear partial-result UX and journal recovery. |
| Config/storage transaction | H-07, H-08 | Deadlock prevention and post-publication ambiguity. |
| Plan-derived Pull authorization | H-11, H-12 | Coordinated Rust/TypeScript contract and rerender state. |

## Verification gates

### Per-change checks

- Rust-only changes:
  `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- Tauri DTO or frontend changes:
  `npm run build` and `npm run test:frontend-integration`
- Formatting:
  `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`

### Final handoff

Run:

```text
npm run build
npm run test:frontend-integration
npm run test:release-config
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:integration
```

Also require:

- targeted checked-transaction tests on macOS and Windows, plus Linux where
  supported;
- scripted S3 tests for chunked, dishonest-length, timeout, cancellation, and
  aggregate-budget behavior;
- repeated deterministic concurrency tests for config/storage and repair;
- a complete native Tauri Pull review with screenshots showing plan-derived
  conversations, skills, plugins, explicit reapproval after replan, partial
  repair status, and storage-change rejection; and
- no test using a real `~/.codex`, `~/.claude`, `~/.mallard`, or real storage
  credentials.

## Release and recovery behavior

- **Malformed remote manifest:** Pull is blocked with the exact invalid
  resource/path dimension and trusted-client republish guidance.
- **Case-only rename:** publication is blocked because schema 4 cannot
  represent it safely; keep the previous spelling or choose a name that differs
  by more than case.
- **Old local restore/dependency plan:** discard and replan under schema 2.
- **Capture mismatch:** no publication; rescan and review.
- **Concurrent target change:** no overwrite/delete; preserve quarantine or
  backup and report its recovery state without logging an absolute path.
- **Interrupted restore/skill/repair:** reconcile the bounded machine-local
  journal before retry.
- **Storage mismatch before publication/apply:** no side effect; re-open review.
- **Storage mismatch after remote publication:** do not record a base; state
  clearly that the remote head may have advanced and require inspection/replan.
- **Historical authorization text:** atomically re-sanitize retained logs once;
  keep no raw backup.

## Definition of done

- [ ] H-01: captured bytes, mode, warnings, and ownership are sealed to review.
- [ ] H-02: every manifest resource passes the exhaustive namespace semantic matrix.
- [ ] H-03: no live/tombstone portable-key collision validates or reaches Apply.
- [ ] H-04: skill overflow is explicit and every displaced tree has a complete verified backup.
- [ ] H-05: writes and deletions use checked no-clobber transactions on every supported platform.
- [ ] H-06: a concurrent transcript append survives every later repair failure.
- [ ] H-07: review, remote operation, and recorded base share one storage fingerprint.
- [ ] H-08: full config validation and persistence occur under one machine-state transaction.
- [ ] H-09: single and bulk S3 memory remain within their streaming budgets.
- [ ] H-10: complete Authorization values are absent from new errors/logs and migrated retained logs.
- [ ] H-11: Pull makes no live history/thread/capability calls and renders every planned row.
- [ ] H-12: selections, completion, failures, and Apply payloads are bound to plan/action fingerprints.
- [ ] All final verification gates pass and the native Tauri review is recorded.

# Test Plan: Non-Git Project Content Sync

**Status:** core executable coverage implemented; extended catalogue retained
**Date:** 2026-07-20
**Feature design:** [PLAN_NON_GIT_PROJECT_FILE_SYNC.md](PLAN_NON_GIT_PROJECT_FILE_SYNC.md)
**Scope:** integration, fault-injection, security, performance, and stress
coverage for optional file and directory sync in non-Git project folders.

This document defines the release coverage for the implemented feature.
Schema 4 is a clean cutover, so this plan does not include migration or
backward-compatibility cases. The core two-machine, deletion, scanner-limit,
review-token, and frontend-selection cases are executable; the larger
fault-injection catalogue remains the ongoing hardening backlog.

## 1. Test-first release strategy

The first executable specification is a backend integration test, not a unit
test of a proposed helper. The implementation sequence is:

1. Extend the existing two-machine harness with project-folder bindings,
   reviewed selections, typed Pull approvals, and filesystem snapshots.
2. Add the P0 backend scenarios below and confirm that they fail for the
   missing feature for the intended reason.
3. Add frontend integration tests for the conditional tab, selection model,
   and final Review summary.
4. Implement the smallest vertical slice that makes the nested-file round
   trip pass through the real Push/Pull command boundary.
5. Add the remaining integration scenarios before filling in focused unit
   tests for scanners, validators, and three-way classification.
6. Run boundary and fault tests on every pull request. Run the full-size
   stress and deterministic soak suites on a scheduled job and before a
   release.

The release is blocked if a safety property is covered only by a mocked unit
test. File writes, directory creation, deletion, manifest publication,
selection persistence, and review-token validation must all be exercised
through the production command path.

## 2. What is under test

The system under test includes:

- non-Git eligibility checks at scan, Push, Pull-plan, and Pull-apply time;
- lazy project-content discovery and default selection after an explicit
  scan;
- destination-specific local recipe and exclusion metadata;
- schema-4 file entries, directory entries, objects, and tombstones;
- stable path-based resource IDs and project-relative logical paths;
- reviewed Push publication through immutable objects and head CAS;
- typed Pull planning, per-entry approval/keep-local decisions, backups,
  ordered directory creation, writes, and safe deletion;
- three-way local/storage/base classification;
- filesystem and secret-safety exclusions;
- UI step order, tree selection, action summaries, and large-tree rendering;
  and
- local-folder storage and the stub-S3 storage path.

The suite must not access a real home directory, real user project, real
credentials, or live cloud bucket.

## 3. Test locations and commands

Current executable coverage:

```text
src-tauri/src/project_sync_v3/commands.rs                 # command-boundary two-machine cases
src-tauri/src/project_sync_v3/provider_capture.rs         # scanner safety and 20,001-entry stress
src-tauri/src/project_sync_v3/domain.rs                   # schema-4 tree/tombstone validation
tests/frontend/project-files.integration.test.tsx
```

The frontend test is registered in
`scripts/run-frontend-integration-tests.mjs`. The normal backend integration
script also runs the existing `sync_tests` local-folder and stub-S3 matrix.

The normal integration command must include the end-to-end sync harness. The
current `test:backend-integration` script runs command tests but omits
`sync_tests`; update it as part of this feature.

```sh
npm run build
npm run test:frontend-integration
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib sync_tests::project_content
cargo test --manifest-path src-tauri/Cargo.toml --lib project_sync_v3::commands::tests
```

The bounded 20,001-entry project-content stress case runs in the normal Rust
suite and can be targeted directly:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --lib \
  project_content_stress_scan_enforces_the_twenty_thousand_resource_cap \
  -- --test-threads=1
```

Use `KEEP_SYNC_TEST_DIRS=1` only as the existing debug escape hatch. A normal
success or failure must clean up all temporary machine, project, backup, and
storage directories.

## 4. Harness extensions

### 4.1 Machines and project roots

Extend `Machine` with an arbitrary temporary non-Git project root, independent
from its temporary `$HOME`. A test machine needs helpers equivalent to:

```rust
machine.create_project("alpha");
machine.project_file("alpha", "docs/specs/a.md", bytes, mode, mtime);
machine.project_dir("alpha", "empty", mode, mtime);
machine.bind_project("alpha", &cloud);
machine.inspect_project_content("alpha", &cloud);
machine.push_project_content(review);
machine.plan_project_content_pull("alpha", &cloud);
machine.apply_project_content(plan, approved_action_ids, kept_local_ids);
```

Add no-follow helpers for symlinks, hard links, FIFOs where supported,
read-only entries, path replacement, and target mutation between review and
apply. Platform-specific fixtures must skip with an explicit reason when the
filesystem cannot represent the entry; the portable validator tests must
still run everywhere.

### 4.2 Snapshot oracles

Every scenario should be able to capture and compare four independent views:

1. **Local tree snapshot** — normalized relative path, entry type, byte hash,
   safe mode, and mtime for informational assertions. The snapshot walker
   never follows links.
2. **Local link metadata** — recipe resource IDs, exclusion IDs, preference
   revision, binding revision, and reviewed base commit.
3. **Remote snapshot** — head revision/commit, manifest file and directory
   entries, tombstones, object hashes, and head-linked commit chain.
4. **Operation record** — planned actions, approvals, blockers, backups,
   receipts, activity counts, and sanitized error messages.

Assertions must inspect the manifest and target filesystem directly. A
`success: true` response alone is never a sufficient integration assertion.

### 4.3 Backend parity

Put backend-independent cases in shared `run_*` bodies with one wrapper for:

- stub S3 through the real AWS SDK request path; and
- local-folder storage through the real local `Store` implementation.

Only transport-specific cases, such as an ambiguous HTTP response, may run on
one backend. The behavioral result after a successful operation must otherwise
match across both backends.

### 4.4 Deterministic fault injection

Extend existing hooks rather than using sleeps. Required injection points are:

- after scan but before capture;
- while hashing a selected file;
- after object upload but before manifest publication;
- after manifest/commit upload but before head CAS;
- immediately before and after head CAS;
- after remote publication but before local link metadata is saved;
- after Pull planning but before preflight;
- after Pull preflight but before the first filesystem mutation;
- after directory creation but before file write;
- after backup but before replace/delete;
- between file deletion and parent-directory deletion; and
- before receipt persistence.

Each hook fires once and records that it fired. Tests must not rely on timing
races that can become flaky.

### 4.5 Test data and privacy

Use unmistakably synthetic secret markers and assert that neither file bytes
nor markers appear in logs, errors, receipts, snapshots intended for the UI,
or test output. On failure, diagnostic output may contain relative paths and
hashes but not file contents or bearer-token-shaped fixture values.

## 5. Global safety invariants

Check these invariants after every applicable scenario:

| ID | Invariant |
| --- | --- |
| INV-01 | Opening the review or scanning never uploads data and never mutates saved selection metadata. |
| INV-02 | A successful Push publishes only reviewed resource IDs and exact reviewed bytes/modes. |
| INV-03 | A failed or stale Push never advances the remote head or local recipe/preferences. Immutable orphan uploads are allowed. |
| INV-04 | No source-machine absolute project path appears in a resource ID, logical path, object key, manifest, commit, or activity event. |
| INV-05 | Every stored file has an independently stored entry for every ancestor directory below the project root. |
| INV-06 | No manifest path is both a file and a directory, and directory resources never own byte objects. |
| INV-07 | Pull performs no project-content write, replacement, mode change, or deletion without an approved typed action. |
| INV-08 | Pull revalidates reviewed source and target digests immediately before mutation. |
| INV-09 | A file deletion requires a file tombstone, explicit approval, matching local digest, and a recoverable backup. |
| INV-10 | A directory deletion is exact, explicit, empty-only, deepest-first, and never recursive. |
| INV-11 | Symlinks, hard links, special files, path escapes, VCS data, app storage, and known credential paths never enter a bundle or receive a write. |
| INV-12 | Required directories are created shallowest-first; final modes/mtimes are applied deepest-first after children. |
| INV-13 | A selected future child is never inferred from an earlier directory selection; it is considered only by a later explicit scan. |
| INV-14 | Project-content selections and exclusions cannot cross project-storage links. |
| INV-15 | Mtime-only changes do not publish a revision or cause a repeated Pull; safe mode-only changes do. |
| INV-16 | A no-op Push/Pull is idempotent: no bundle revision, receipt, backup, or local preference revision changes. |
| INV-17 | Initializing Git after review fails closed before publish or local mutation. |
| INV-18 | Corrupt, incomplete, unsupported, or malicious remote state fails before any local project-content mutation. |
| INV-19 | Skipping project content cannot prevent explicitly selected session/tool work from completing. |
| INV-20 | After a partial operational failure, the result identifies every applied, failed, and unattempted action, and a refreshed retry is safe. |

## 6. P0 integration scenarios to write first

These are the initial red tests and the minimum release gate. Run every
backend-independent scenario against both storage implementations.

### IT-P0-01 — Nested file and empty-directory round trip

Given Machine A has:

```text
<A-root>/docs/
<A-root>/docs/specs/
<A-root>/docs/specs/a.md
<A-root>/empty/
```

When A opens Push, no ordinary project scan occurs. After A explicitly scans,
`docs`, `docs/specs`, `docs/specs/a.md`, and `empty` are all new and selected.
A Pushes them.

Assert remotely:

- `docs` and `docs/specs` are separate directory resources;
- `empty` is a separate directory resource;
- `docs/specs/a.md` is a file resource with one byte object;
- all paths are project-relative and all IDs are stable path/type hashes;
- there is no resource for the project root itself; and
- directory entries do not have upload objects.

Machine B maps the bundle to a different absolute project root. Pull initially
keeps every project-content action local. When B selects `a.md`, `docs` and
`docs/specs` become required. When B also selects `empty`, Apply must:

1. create `<B-root>/docs`;
2. create `<B-root>/docs/specs`;
3. create `<B-root>/empty`;
4. write `<B-root>/docs/specs/a.md`; and
5. finalize directory metadata deepest-first.

Assert that `/spec` is never created, the bytes match, the empty directory
exists, the receipt has three directory actions plus one file action, and a
second Pull is a no-op.

### IT-P0-02 — New entries selected only after an explicit scan

Start with a linked empty non-Git project. Opening Push and entering other tabs
must not discover or select ordinary files. Create `one.md`, run Scan, and
assert that it appears checked as `New · selected after scan`.

Uncheck it and cancel Push. Reopen and scan: saved metadata is unchanged, so
the entry is new and checked again. Uncheck it and successfully Push another
selected entry: its resource ID is now stored in the destination-specific
exclusion set. On a later scan `one.md` remains unchecked.

Create `later.md` beneath a previously selected directory. It does not enter
the bundle in the background. The next explicit scan discovers it and checks
it by default, unless that exact resource ID was previously excluded.

### IT-P0-03 — Local and remote metadata contract

After a successful Push, assert locally that the link recipe, exclusions,
preference revision, and reviewed base commit update together. Assert that a
scan, cancellation, failed upload, and lost head CAS change none of them.

Assert remotely that the manifest contains only included relative paths,
resource IDs, byte hashes, sizes, safe modes, and informational source mtimes.
It must not contain local exclusions, an absolute project root, ownership,
ACLs, xattrs, or local link IDs.

Change only a file mtime and Push: no revision advances. Change only its
safe mode and Push: a reviewed update is published. Repeat for a directory.
Set set-id bits on a supported platform and assert they are stripped.

### IT-P0-04 — File removal requires confirmation on both machines

A and B first converge on `docs/file_a`. A deletes its local file and scans.
The stored version is retained and Push is blocked until A chooses **Remove
from storage** for that exact file. Confirming the removal publishes a file
tombstone containing the last byte digest; it does not silently remove the
ancestor directory resource.

B Pull sees `Delete requested` unselected. Without approval, B's file remains.
With approval and an unchanged digest, Apply backs it up, verifies the backup,
deletes only that regular file, and records the action. Reapplying or pulling
again is an idempotent no-op.

### IT-P0-05 — Modified local file blocks a remote deletion

Repeat IT-P0-04, but modify B's file after its last Pull. Approval must not
override the digest mismatch: Apply blocks deletion, creates no misleading
success receipt, and preserves B's bytes. Other approved safe actions may
still complete and are reported separately.

### IT-P0-06 — Directory removal is descendant-by-descendant and empty-only

A and B converge on:

```text
docs/
docs/specs/
docs/specs/a.md
docs/notes.md
```

A removes `docs` locally. Scan must not translate absence into a deletion.
The bulk **Remove from storage** review enumerates two files and two
directories as independent removals. After A confirms, the remote manifest
contains exact descendant file tombstones and exact directory tombstones.

On B, deletion actions start unselected. When all are approved, Apply backs
up and deletes files first, then removes `docs/specs`, then `docs`. If B has an
extra untracked `docs/local.md`, the two approved tracked files may be removed,
but `docs` remains because it is non-empty. No operation recursively deletes
`local.md`.

### IT-P0-07 — Partial directory-deletion approval

Using IT-P0-06, approve deletion of `a.md` but keep `notes.md` local. Approving
the directory tombstones cannot delete either non-empty directory. The result
must distinguish the applied file deletion, the kept-local file, and blocked
directory removals. A refreshed plan remains stable and does not repeatedly
offer the already-satisfied file delete as a destructive action.

### IT-P0-08 — Three-way update and divergence

From a shared base:

- storage changes while B still matches base: Pull offers an optional update;
- B changes while storage still matches base: Push reports local ahead;
- both change to identical bytes/mode: status is synced;
- both change differently: status is diverged and neither Push nor Pull offers
  a force-overwrite shortcut; and
- a missing/unloadable base makes the affected path unknown and blocks it.

Approving a normal update creates a backup of an existing file, writes the
storage bytes, restores safe metadata, and advances the reviewed base. Keeping
it local does not alter bytes and produces an explicit keep-local receipt.

### IT-P0-09 — Review-token and target-digest races

After A reviews Push, mutate a selected file without changing its apparent
path. Capture must reject the stale review and publish nothing. Cover both a
size change and same-size byte change.

After B reviews Pull, mutate the target, replace an ancestor directory with a
symlink, or move the storage head. Apply must fail preflight before the stale
action mutates the target. A refresh presents the new state.

### IT-P0-10 — Git eligibility fails closed at every boundary

Cover a project that is:

- a Git work tree root;
- nested below a Git work tree;
- a linked worktree with a `.git` file; and
- unknown because the eligibility probe fails.

The ordinary Project files tab is omitted when there is no stored project
content. If remote project content exists, the locked tab remains visible.
Also initialize Git after Scan, after final Push review, and after Pull
planning. Each operation fails before upload, tombstone publication,
directory creation, or file write.

### IT-P0-11 — Scanner exclusions and no-follow behavior

In one fixture include a normal text file, binary file, empty directory,
`.git`, `.hg`, `.svn`, `.env`, `.env.production`, `.npmrc`, private key,
symlink to an internal file, symlink outside the root, hard link, FIFO,
provider home, storage root, nested registered project, and a path owned by an
agent resource.

Only the normal file, binary file, and allowed directories may be selected.
Blocked entries remain visible with bounded reasons where safe; no excluded
bytes or absolute targets reach a manifest. Swap an allowed file for a link
after review and assert capture fails no-follow.

### IT-P0-12 — Ignore rules and explicit exclusions are not deletions

Cover root `.mallardignore`, `.gitignore`, and `.ignore` precedence,
negation, nested patterns, ignored directories, and the ignore files
themselves. Ignored descendants have no file or directory candidates.

Changing an ignore rule so that a previously published entry disappears from
discovery retains the remote version and blocks the affected Push; it does
not create a tombstone. Removing an ignore rule makes newly eligible entries
visible and checked at the next explicit scan.

### IT-P0-13 — Stale head CAS and ambiguous publication

Use the existing CAS hooks to move the head after A's review. If the external
commit is unrelated, A refreshes/rebases only after revalidating every
project-content decision and may retry. If it changes a reviewed path, A must
stop for a new review. A losing attempt may leave immutable orphans but must
not change local selection metadata.

For stub S3, stall a successful head response. Re-reading the head and finding
A's exact commit reports success once and persists local link metadata once;
it must not duplicate a revision or tombstone.

### IT-P0-14 — Corrupt or malicious remote state applies nothing

Before B Pull, independently test:

- manifest SHA mismatch;
- missing or byte-corrupt file object;
- file entry without an ancestor directory entry;
- file/directory collision at one logical path;
- duplicate/colliding resource ID;
- absolute, parent-traversal, empty-component, or non-normalized path;
- tombstone kind/path mismatch; and
- directory entry that incorrectly references byte content.

Planning or preflight fails with a sanitized error. B's full project snapshot,
modes, mtimes, backups, and receipts remain unchanged.

### IT-P0-15 — Project/storage isolation and mixed-category Pull

Link one local project to storages S1 and S2 with different selections and
exclusions. A successful Push to S1 must not update S2 metadata or content.
Link two local projects with similar relative paths to one storage and assert
that their bundle/link identity remains separate.

Create one Pull plan containing a project file, session, and plugin action.
Keep the project file local and approve the other categories. The other work
completes, the project file is untouched, and the next comparison reports the
correct project-file direction from the explicit receipt/base decision.

### IT-P0-16 — Failure after remote success and retry recovery

Inject failure after the remote head is successfully advanced but before the
local recipe/preferences are saved. The result must say that remote
publication succeeded and local metadata repair is needed; it must not report
the Push as wholly absent. A reload must recognize the published commit,
repair or reconstruct the link metadata through a reviewed recovery path, and
must not republish the same revision or repeat a deletion.

This scenario is a design gate: implementation may not ship with an
ambiguous generic error that encourages a blind retry.

## 7. Detailed backend integration catalogue

The P0 scenarios above are supplemented by the following focused cases. Each
row is one independently named test or one clearly labelled subcase.

### 7.1 Discovery and selection

| ID | Case | Expected result |
| --- | --- | --- |
| DISC-01 | Open Push but never enter Project files | No scan, pending candidate, or metadata change. |
| DISC-02 | Enter Project files for an unused link | Eligibility may run; ordinary content still waits for Scan. |
| DISC-03 | Scan a completely empty root | Empty inventory; project root is not a resource. |
| DISC-04 | Scan one nested file | File and every descendant directory ancestor are checked. |
| DISC-05 | Scan an independently empty directory | Directory is checked and can round-trip. |
| DISC-06 | Uncheck a new file | Required ancestors unlock only when no selected child needs them. |
| DISC-07 | Attempt to uncheck a required ancestor | Request is rejected or normalized back to selected. |
| DISC-08 | Select a directory row | Only currently discovered selectable descendants change. |
| DISC-09 | Add a child after selecting its parent | Child is absent until Rescan, then visibly checked as new. |
| DISC-10 | Exclude a path, successfully Push, then recreate it | Stable ID remains excluded until Reset exclusions or explicit selection. |
| DISC-11 | Exclude a path but cancel/fail Push | Exclusion is not persisted. |
| DISC-12 | Reset exclusions | Previously excluded eligible entries are checked on the next scan. |
| DISC-13 | Search/filter then bulk select | Only the documented shown set changes; hidden selections are preserved. |
| DISC-14 | Clear with published entries present | It does not schedule remote deletion. |
| DISC-15 | Bulk Remove from storage | Exact published resources and counts require a second confirmation. |
| DISC-16 | Rename a file | New path plus missing old path; no rename inference. |
| DISC-17 | Rename a directory subtree | Exact new entries plus explicit old descendant removals. |
| DISC-18 | File becomes unreadable after Scan | Previous remote version retained; Push blocked for that entry. |
| DISC-19 | Ignore rule changes during Scan | Review token invalidated; no partial discovery accepted. |
| DISC-20 | Duplicate scan with unchanged tree | Stable ordering, IDs, selection, revision, and zero remote writes. |

### 7.2 Identity, manifest, and metadata

| ID | Case | Expected result |
| --- | --- | --- |
| META-01 | Same relative tree under two different absolute roots | Resource IDs and logical paths match. |
| META-02 | Same path used once as file and once as directory | IDs differ by type; one manifest cannot contain both. |
| META-03 | Root directory | Never serialized as a portable resource. |
| META-04 | Directory containing no selected descendants | Stored only if the directory itself is selected. |
| META-05 | Selected file with unselected ancestor request | Backend inserts/requires the ancestor or rejects the forged request. |
| META-06 | Object byte length disagrees with manifest size | Remote state rejected. |
| META-07 | Object SHA disagrees with file entry | Remote state rejected before write. |
| META-08 | File mtime changes only | No new reviewed digest or revision. |
| META-09 | Directory mtime changes only | No new reviewed digest or revision. |
| META-10 | File executable bit changes | Reviewed metadata update and revision. |
| META-11 | Directory safe mode changes | Reviewed metadata update and revision. |
| META-12 | Setuid/setgid/sticky inputs | Only documented safe bits survive; set-id bits never restore. |
| META-13 | Filesystem cannot apply a source mtime exactly | Pull succeeds with best-effort status and no future sync loop. |
| META-14 | Filesystem has coarse mtime precision | Repeated scan remains synced. |
| META-15 | Ownership, ACL, xattr, creation time differ | No sync change and no portable manifest field. |
| META-16 | Windows/non-POSIX mode unavailable | Mode is absent/normalized without a perpetual diff. |
| META-17 | Remote manifest inspection | No exclusions, absolute roots, local binding IDs, or secret warnings containing content. |
| META-18 | Local preference update | Recipe and exclusions commit together with one monotonic revision. |
| META-19 | No-op Push after success | No preference revision or base commit churn. |
| META-20 | Forged resource ID for a valid path | Backend rediscovery rejects it. |

### 7.3 Pull directory and write behavior

| ID | Case | Expected result |
| --- | --- | --- |
| PULL-01 | Missing nested target | Ancestors created shallowest-first before file. |
| PULL-02 | Ancestors already exist as real directories | Reused without destructive replacement. |
| PULL-03 | Ancestor path is a regular file | Entire dependent subtree blocked. |
| PULL-04 | Ancestor path is a symlink | Entire dependent subtree blocked no-follow. |
| PULL-05 | Target file path is a directory | File action blocked; directory is untouched. |
| PULL-06 | Directory target path is a file | Directory action and descendants blocked. |
| PULL-07 | Stored directory mode is read-only | Create temporarily owner-writable, write children, then finalize. |
| PULL-08 | Child write fails under a new directory | Failure is reported; retry can safely complete and final mode is not falsely reported. |
| PULL-09 | Existing identical file | No write, backup, mtime churn, or receipt noise. |
| PULL-10 | Existing base-matching file updated | Backup then atomic replace after approval. |
| PULL-11 | New binary file | Exact byte round trip, no text decoding. |
| PULL-12 | Zero-byte file | Preserved as a file, distinct from directory. |
| PULL-13 | File with executable mode | Warning/review required; bytes never executed. |
| PULL-14 | Select child but forge keep-local ancestor | Backend rejects inconsistent decisions. |
| PULL-15 | Keep all project content local | Zero project mutations; explicit receipts permit other categories. |
| PULL-16 | Apply subset from two independent subtrees | Only selected subtree and its ancestors are touched. |
| PULL-17 | Target changes after preflight hook | Digest/type recheck blocks before replace. |
| PULL-18 | Storage object changes without head change | SHA verification blocks before replace. |
| PULL-19 | Apply same plan twice | Expired/consumed plan cannot repeat destructive actions. |
| PULL-20 | Refresh after partial result | Already-applied state classifies correctly and retry is idempotent. |

### 7.4 File and directory deletion

| ID | Case | Expected result |
| --- | --- | --- |
| DEL-01 | Previously selected file is merely missing | No tombstone until explicit Remove from storage. |
| DEL-02 | Previously selected directory is merely missing | No descendant or directory tombstone. |
| DEL-03 | Missing path was never published | Deselect/exclude only; no tombstone. |
| DEL-04 | Confirm one file removal | Exact file tombstone with last digest. |
| DEL-05 | Confirm directory only, descendants still stored | Reject inconsistent removal set. |
| DEL-06 | Confirm whole subtree removal | Exact descendant tombstones; no wildcard tombstone. |
| DEL-07 | B skips file deletion | File remains byte/mode identical. |
| DEL-08 | B approves unchanged file deletion | Verified backup then exact unlink. |
| DEL-09 | B file already absent | Treat as already satisfied; do not create fake backup. |
| DEL-10 | B file has different bytes | Block and preserve it. |
| DEL-11 | B path changed from file to directory | Block; never recurse. |
| DEL-12 | B path changed from file to symlink | Block; never unlink link or target. |
| DEL-13 | B approves an empty-directory tombstone | Remove exact real empty directory. |
| DEL-14 | Directory contains an untracked local file | Directory removal blocked; local child remains. |
| DEL-15 | Directory contains a kept-local tracked file | Directory removal blocked. |
| DEL-16 | Directory contains a newly created file after review | Final empty check fails safely. |
| DEL-17 | Directory target is a symlink | Block and do not unlink it. |
| DEL-18 | Nested empty directories | Remove deepest-first. |
| DEL-19 | Parent deletion approved, child-directory deletion skipped | Parent remains. |
| DEL-20 | Backup write/verification fails | Original file is not deleted. |
| DEL-21 | Failure after backup but before delete | Original and recoverable backup exist; retry does not corrupt either. |
| DEL-22 | Failure after file deletes before directory deletes | Exact partial receipt; retry removes only now-empty approved dirs. |
| DEL-23 | Rename represented as add plus deletion | New path can apply while old path still requires separate approval. |
| DEL-24 | Forged client deletion without tombstone | Backend rejects it. |
| DEL-25 | Directory tombstone presented as recursive request | Backend rejects it regardless of UI approval. |

### 7.5 Three-way state and concurrency

| ID | Case | Expected result |
| --- | --- | --- |
| STATE-01 | Local = storage = base | Synced/no action. |
| STATE-02 | Local = base, storage changed | Storage ahead/Pull update. |
| STATE-03 | Storage = base, local changed | Local ahead/Push update. |
| STATE-04 | Local and storage changed identically | Synced after hash comparison. |
| STATE-05 | Local and storage changed differently | Diverged and blocked. |
| STATE-06 | Local added, absent from base/storage | New and selected only after Scan. |
| STATE-07 | Storage added, absent locally/base | Optional Pull addition. |
| STATE-08 | Local missing, storage/base present | Missing is not deletion. |
| STATE-09 | Storage tombstone, local matches base | Optional explicit local deletion. |
| STATE-10 | Storage tombstone, local changed | Deletion blocked. |
| STATE-11 | File changes to directory on one side | Type conflict; no implicit replacement. |
| STATE-12 | Directory changes to file on one side | Type conflict; no implicit replacement. |
| STATE-13 | Base commit missing/corrupt | Unknown; affected Push/Pull blocked. |
| STATE-14 | Binding revision changes after review | Token stale; operation rejected. |
| STATE-15 | Storage head changes after review | Token stale; refresh/rebase rules applied. |
| STATE-16 | Two writers add disjoint paths | CAS retry preserves both exact trees after revalidation. |
| STATE-17 | Two writers update same path | Loser returns to review; no generic conflict-copy behavior. |
| STATE-18 | Writer removes path while another updates it | Explicit divergence; neither action silently wins. |
| STATE-19 | Ten repeated no-op scans/pushes | Revision and commit chain remain unchanged. |
| STATE-20 | Late-joining Machine C | One reviewed Pull can reproduce the complete selected tree. |

### 7.6 Path, credential, and content safety

| ID | Case | Expected result |
| --- | --- | --- |
| SAFE-01 | `.git/**`, `.hg/**`, `.svn/**` | Hard excluded. |
| SAFE-02 | Mallard config/storage/backup roots | Hard excluded. |
| SAFE-03 | Provider home nested under project | Hard excluded. |
| SAFE-04 | Separately registered child project | Entire child root hard excluded. |
| SAFE-05 | Path already owned by agent resource | Not duplicated as generic content. |
| SAFE-06 | `.env`, `.env.*`, `.npmrc`, `.netrc`, auth/token names | Hard blocked regardless of ignore rules. |
| SAFE-07 | Private-key content/name | Hard blocked and content absent from diagnostics. |
| SAFE-08 | Token-like marker in an ordinary file | Exact digest warning and explicit acknowledgement required. |
| SAFE-09 | Warned file changes after acknowledgement | Acknowledgement invalid; Push rejected. |
| SAFE-10 | Binary file without blocked marker | Allowed within limits. |
| SAFE-11 | Symlink file or directory | Never followed or captured. |
| SAFE-12 | Hard link with link count greater than one | Blocked. |
| SAFE-13 | FIFO/socket/device where supported | Blocked without opening it. |
| SAFE-14 | Absolute or `..` malicious client path | Rejected before filesystem access. |
| SAFE-15 | Canonical path escapes root through race | Revalidation rejects it. |
| SAFE-16 | Case-fold collision (`A.md`, `a.md`) | Both blocked as non-portable collision. |
| SAFE-17 | Unicode normalization collision | Both blocked deterministically. |
| SAFE-18 | Windows reserved name/trailing dot/space | Blocked on every source OS. |
| SAFE-19 | Empty component, `.`, repeated separator | Normalize once or reject; never create ambiguous identity. |
| SAFE-20 | Invalid UTF-8 filename on Unix | Blocked with bounded display-safe reason. |
| SAFE-21 | Maximum allowed UTF-8 path | Round trips exactly. |
| SAFE-22 | Path beyond component/length limit | Blocked without walking or allocating unbounded state. |
| SAFE-23 | Ignore file contains pathological patterns | Scan remains bounded and deterministic. |
| SAFE-24 | Error/activity logging for warned or corrupt file | Relative path/code/hash only; never bytes or secret marker. |

### 7.7 Failure and recovery

| ID | Injection | Required result |
| --- | --- | --- |
| FAIL-01 | Scanner cannot read one candidate | Candidate blocked; safe siblings remain reviewable. |
| FAIL-02 | Project root disappears during Scan | Inventory rejected as stale/unknown. |
| FAIL-03 | File mutates during hashing | Stable-read check rejects reviewed capture. |
| FAIL-04 | Upload fails before any immutable object lands | Head/local metadata unchanged. |
| FAIL-05 | Upload fails after some objects land | Orphans allowed; head/local metadata unchanged. |
| FAIL-06 | Manifest write fails | Head/local metadata unchanged. |
| FAIL-07 | Commit write fails | Head/local metadata unchanged. |
| FAIL-08 | Head CAS loses once | Revalidate and retry or return to review according to changed paths. |
| FAIL-09 | Head CAS loses on every attempt | Bounded failure; winner survives. |
| FAIL-10 | Head response times out after successful write | Read-after-write resolves exact commit as success. |
| FAIL-11 | Local metadata save fails after remote success | Explicit repair state; no blind duplicate Push. |
| FAIL-12 | Pull object download fails | No selected project-content mutation. |
| FAIL-13 | Pull object digest fails | No selected project-content mutation. |
| FAIL-14 | Target preflight permission fails | No selected project-content mutation. |
| FAIL-15 | Backup creation/verification fails | Existing target remains unchanged. |
| FAIL-16 | Atomic file replace fails | Backup and original remain recoverable; exact failure receipt. |
| FAIL-17 | Directory create fails midway | No unreported success; retry handles existing safe ancestors. |
| FAIL-18 | Directory final-mode update fails | File result and metadata failure reported separately; retry does not rewrite bytes unnecessarily. |
| FAIL-19 | Receipt save fails after filesystem actions | Restart reconciles from filesystem/journal; destructive action is not blindly repeated. |
| FAIL-20 | App restarts with an incomplete apply journal | Recovery reports exact prior phase and reaches a safe reviewed state. |

## 8. Frontend integration catalogue

Use the existing `node:test` plus `renderToStaticMarkup` style for pure render
and review-model contracts. Keep selection and step-order logic in pure
functions so interaction semantics can be tested without a browser. Add a
small DOM-capable test only if keyboard/focus behavior cannot be proven with
the current harness.

| ID | Case | Expected result |
| --- | --- | --- |
| UI-01 | Eligible non-Git Push | Tabs are Git & sessions, Skills, Plugins, Project files, Review. |
| UI-02 | Eligible non-Git Pull | Project files appears immediately after Plugins. |
| UI-03 | Git project without remote content | Project files tab omitted. |
| UI-04 | Git project with remote content | Visible locked tab explains why actions cannot apply. |
| UI-05 | Before explicit Scan | No ordinary candidates or misleading selected count. |
| UI-06 | Scan returns new file and directories | Every eligible new row checked and labelled as scan-selected. |
| UI-07 | Scan returns ignored/blocked entries | Counts and safe reasons shown; controls disabled. |
| UI-08 | Nested file checked | Ancestor directories checked and locked as required. |
| UI-09 | Last selected descendant cleared | Ancestor unlocks unless independently selected. |
| UI-10 | Empty directory | Visible, typed as folder, and independently selectable. |
| UI-11 | Tri-state directory row | State reflects only currently discovered selectable descendants. |
| UI-12 | Later child after Rescan | Appears as a new checked suggestion, not a prior wildcard match. |
| UI-13 | Previously excluded entry | Unchecked with a clear explanation; Reset can reconsider it. |
| UI-14 | Clear action | Clears pending additions but does not create remote removals. |
| UI-15 | Published path missing locally | Separate Remove from storage control and confirmation. |
| UI-16 | Bulk subtree removal | Confirmation enumerates file/folder counts and says deletion is exact. |
| UI-17 | Pull new files | Start keep-local/unselected. |
| UI-18 | Selecting a Pull child | Required ancestor actions become selected automatically. |
| UI-19 | Pull tombstones | Start unselected with backup/empty-only wording. |
| UI-20 | Diverged or digest-stale row | No force-overwrite/delete shortcut. |
| UI-21 | Review summary | Separates creates, replacements, file deletes, folder deletes, and skips. |
| UI-22 | Mixed categories | Keeping project files local leaves other approved actions enabled. |
| UI-23 | Backend eligibility changes | Pending controls lock and final Apply/Push cannot proceed. |
| UI-24 | Keyboard arrows/Home/End | Traverse only visible dynamic steps in correct order. |
| UI-25 | Back/Next labels | Plugins leads to Project files; Project files leads to Review. |
| UI-26 | Search and expanded branches | Hidden selections remain stable; counts remain global and explicit. |
| UI-27 | 20,000-entry inventory model | Stable grouping/selection without quadratic work. |
| UI-28 | Large tree render | Only expanded/visible bounded rows render, not 20,000 DOM nodes. |
| UI-29 | Error text | Contains relative path and remediation, never content/absolute source root. |
| UI-30 | Result view | Reports restored/created/kept/deleted/blocked files and folders separately. |

Accessibility assertions:

- every checkbox has a unique relative-path-based accessible name;
- file versus folder and required/blocked/deletion state are not color-only;
- locked ancestors expose `aria-disabled` or equivalent semantics;
- the scan result announcement uses a polite live region;
- keyboard focus survives a rescan when the same resource ID remains; and
- the virtualized tree preserves level, expanded, selected, and set-size
  semantics for screen readers.

## 9. Boundary and stress suite

Stress tests are correctness tests at scale first and timing tests second.
Every stress case checks the global invariants, deterministic ordering,
bounded error output, cleanup, and idempotent retry.

### 9.1 Limit boundaries

| ID | Load | Expected result |
| --- | --- | --- |
| STRESS-01 | Exactly 20,000 combined file/directory resources | Scan completes with all eligible entries represented once. |
| STRESS-02 | 20,001 combined resources | Bounded limit result; no truncated selection may publish. |
| STRESS-03 | Exactly 32 traversal components | Deepest allowed file and every ancestor validate. |
| STRESS-04 | 33 traversal components | Over-depth subtree blocked without affecting safe siblings. |
| STRESS-05 | One file exactly 16 MiB | Capturable and byte-exact. |
| STRESS-06 | One file 16 MiB + 1 byte | Blocked before upload. |
| STRESS-07 | Selected bytes total exactly 512 MiB | Capturable in scheduled release run. |
| STRESS-08 | Selected bytes total 512 MiB + 1 byte | Entire reviewed Push rejected; head unchanged. |
| STRESS-09 | Maximum valid path/component lengths | IDs remain fixed-size and paths round-trip. |
| STRESS-10 | One path over each length boundary | Bounded per-entry blocker; no panic. |

Boundary tests must use real bytes for byte limits; sparse-file metadata alone
cannot prove the capture path enforces bytes actually read. Generate test
content in chunks and keep it only in temporary directories.

### 9.2 Tree shapes and churn

| ID | Load | Assertions |
| --- | --- | --- |
| STRESS-11 | One directory with 19,999 files | Stable lexicographic inventory, linear-ish scan, bounded UI rows. |
| STRESS-12 | 10,000 directories plus 10,000 files | All ancestors/resource types counted exactly once. |
| STRESS-13 | Deep chain near depth limit | Shallow-create/deep-finalize ordering remains exact. |
| STRESS-14 | 5,000 empty directories | Empty-directory manifest/Pull round trip without placeholder objects. |
| STRESS-15 | 5,000 tracked files removed in 500 directories | Exact tombstones; approved deletes files then empty dirs; no recursion. |
| STRESS-16 | 1,000 adds, 1,000 updates, 1,000 explicit removes | Review totals, manifest, objects, and receipts agree. |
| STRESS-17 | 100 unchanged scans | No preference revision, bundle revision, object, or memory growth trend. |
| STRESS-18 | 100 Push/Pull convergence cycles | Final trees/bases match; no repeated mtime/mode churn. |
| STRESS-19 | 10,000 explicit exclusions | Bounded metadata, deterministic selection, Reset remains usable. |
| STRESS-20 | Exclusion/resource limit reached | Clear actionable error; existing metadata is not silently pruned. |

### 9.3 Concurrency, failures, and soak

| ID | Load | Assertions |
| --- | --- | --- |
| STRESS-21 | Ten disjoint external writers around head CAS | Published head chain is valid; final reviewed union contains every accepted path. |
| STRESS-22 | Ten writers contend on one path | One reviewed winner; every loser becomes stale/diverged without lost bytes. |
| STRESS-23 | Repeated ambiguous S3 head responses | Each committed revision recognized exactly once. |
| STRESS-24 | Fault at each Push injection point | Head/local preference invariants hold at every phase. |
| STRESS-25 | Fault at each Pull injection point | Backups, journal, receipts, and retry state remain coherent. |
| STRESS-26 | File continuously changes during capture | Bounded retry/failure; never publish mixed bytes. |
| STRESS-27 | Target continuously changes during Apply | Bounded stale failure; never overwrite the moving target. |
| STRESS-28 | 50 restart/review/retry cycles | No duplicate deletion, revision, backup corruption, or stuck temporary mode. |
| STRESS-29 | Three machines, 1,000 seeded random operations | Model oracle and all machine/storage snapshots agree after convergence. |
| STRESS-30 | Seven-day scheduled seed rotation | Every failure prints seed and minimal operation trace for exact replay. |

The seeded operation model may choose:

- scan;
- add/update/chmod/touch/rename/remove a file;
- add/chmod/remove a directory;
- include/exclude/reset an entry;
- approve/skip a Pull action;
- explicitly remove a remote entry;
- Push/Pull/restart;
- advance a competing remote head; and
- initialize Git.

The model oracle does not duplicate production algorithms. It tracks only the
reviewed contract: exact selected remote entries, explicit tombstones, last
accepted base, local content, and whether each destructive action was
approved. After every step it checks that no unreviewed mutation occurred.

### 9.4 Performance budgets

Initial budgets are deliberately generous and must be measured on the same
release CI class. Record elapsed time and peak RSS for all scheduled runs.

| Operation | Provisional release ceiling |
| --- | --- |
| Scan 20,000 small entries without content capture | 15 seconds, +300 MiB peak RSS |
| Build/filter/select the 20,000-entry inventory model | 2 seconds, no quadratic growth |
| Render one expanded 500-row window from a 20,000-entry model | 500 ms, at most 750 project tree row elements |
| Capture/hash/upload 512 MiB to local storage | 120 seconds, streaming memory use |
| Pull/verify/write 512 MiB from local storage | 120 seconds, streaming memory use |
| Plan 5,000 exact deletions | 10 seconds, bounded DTO and log output |

A timing result fails only when it exceeds both the absolute ceiling and twice
the rolling median for that CI class. Correctness, resource-count limits, DOM
row limits, and evidence of quadratic growth always fail immediately. Ratify
or revise these ceilings after the first implementation measurement; never
silently relax them in response to a regression.

## 10. Platform matrix

Run the complete backend integration suite on the primary macOS CI target.
Before release, run the portable subset on macOS, Linux, and Windows.

| Concern | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Stub S3 and local storage round trip | Required | Required | Required |
| POSIX file/directory modes | Required | Required | Normalization/no-loop contract |
| Symlink no-follow | Required | Required | Required when test privileges allow; validator always required |
| Hard-link rejection | Required | Required | Required where supported |
| FIFO/special-file rejection | Required | Required | Validator-only where unavailable |
| Case-fold collision portability | Required | Required | Required |
| Windows reserved names | Validator required | Validator required | Filesystem plus validator required |
| Unicode normalization collision | Required | Required | Required |
| Read-only directory finalization | Required | Required | Required with platform semantics |
| Mtime precision/best effort | Required | Required | Required |

Platform skips must name the missing OS capability. A security case cannot be
considered covered solely because one OS could not create its fixture.

## 11. Manual smoke tests

Automation remains the release authority, but one packaged-app smoke pass
should verify native dialogs, real permissions, and visual clarity:

1. Create a disposable non-Git folder with `docs/specs/a.md` and `empty/`.
2. Confirm Project files appears after Plugins and nothing scans until Scan.
3. Scan, review the default-checked tree, Push to disposable local-folder
   storage, and inspect the Review counts.
4. Map the bundle to a second disposable folder and Pull only `a.md`; confirm
   required directories are selected and recreated.
5. Pull the empty directory separately.
6. Modify the second copy, delete the first copy, publish a reviewed remote
   removal, and confirm the modified second copy cannot be deleted.
7. Add an untracked child to a directory with a tombstone and confirm the
   directory remains.
8. Run `git init` after opening review and confirm Push/Apply locks before any
   mutation.
9. Verify dark/light layout, keyboard navigation, screen-reader labels, and a
   large-tree search.
10. Delete the disposable projects and storage after the smoke pass.

A live S3/R2 smoke test is optional and credential-gated. It must use a new
disposable prefix and must never be part of the deterministic integration
gate.

## 12. Exit criteria

The feature is ready to merge only when:

- all P0 scenarios pass through the real backend command boundary against
  both local-folder storage and stub S3, except documented transport-only
  cases;
- all frontend cases relevant to implemented behavior pass and the new test
  is part of the normal frontend test command;
- every global safety invariant has at least one integration-level assertion;
- exact file and directory removal, modified-target blocking, and non-empty
  directory preservation pass under fault injection;
- limit+1 cases fail closed without publishing or applying partial reviewed
  state;
- the P0 suite passes 100 repeated runs with zero flakes;
- the full stress suite passes at least three consecutive release runs with
  recorded seed, duration, and peak memory;
- no test touches the real home/project folders or leaves temporary data;
- errors, logs, receipts, and activity records contain no fixture secrets or
  file bytes; and
- `src-tauri/src/sync_tests/README.md` lists the new scenarios and documents
  how to reproduce any seeded stress failure.

## 13. Explicitly out of scope

- Migration from or interoperability with older bundle schemas.
- Syncing any project inside a Git work tree.
- Background watchers or automatic scans.
- Directory wildcard rules for future children.
- Rename inference, text merge, or force-overwrite conflict resolution.
- Symlink, hard-link, special-file, ownership, ACL, or xattr synchronization.
- Proving the secret scanner can detect every secret.
- Recursive directory deletion under any approval shape.

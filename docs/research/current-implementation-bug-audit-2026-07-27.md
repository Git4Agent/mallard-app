# Current implementation bug audit

- Snapshot date: July 27, 2026
- Code snapshot: commit `ab8b7d0`, plus the uncommitted working-tree changes
  present during the audit

## Executive summary

This audit found 11 confirmed high-severity defects, one high-severity
sanitizer gap whose production exploitability is conditional, 13 confirmed
medium-severity defects, one medium-severity behavior needing a product
decision, and one low-severity contract drift in the current on-disk
implementation. The most important themes are:

- reviewed bytes and approvals are not always the bytes and approvals later
  acted on;
- several restore and repair paths can overwrite or delete changes made by
  another process after Mallard's last check;
- bundle validation does not completely bind logical paths to their permitted
  resource semantics;
- storage/configuration mutations are not consistently serialized with Push;
- some supposedly bounded reads allocate or retain data before applying the
  bound; and
- frontend review surfaces sometimes render live data rather than the immutable
  plan the backend will apply.

The build and committed test suites all passed. These findings therefore mostly
occupy gaps in concurrency, adversarial input, cross-platform path behavior,
mounted React state transitions, and crash recovery rather than ordinary happy
paths.

The first remediation tranche should cover H-01 through H-07 and H-11 through
H-12. Those defects cross the product's review, no-follow, explicit-approval,
digest-checked deletion, and recoverability boundaries.

## Audit basis

The audit treated the current source and active `ProjectSyncV3` path as
authoritative. Historical plan documents were not used as implementation
evidence. The working tree was already dirty; no pre-existing edits were
discarded or rewritten. Finding M-08 concerns the current uncommitted
`PushDestinationStatus` implementation and should be rechecked if those edits
are changed or dropped.

The review covered:

- `src/components/project-sync/`, `src/types.ts`, and frontend integration tests;
- Tauri command registration and the typed `invoke` boundary;
- domain and manifest validation;
- provider discovery and capture;
- bundle publication, restore planning, apply, backups, and receipts;
- config/binding persistence and cross-document mutations;
- local-folder and S3-compatible storage;
- Codex chat-history parsing and repair; and
- activity logging and sanitization.

The following baseline checks passed:

| Check | Result |
| --- | --- |
| `npm run build` | Passed |
| `npm run test:frontend-integration` | Passed, 93 tests |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | Passed, 290 tests |

The Rust run emitted roughly 150 warnings, mostly dead-code warnings. Passing
these checks is useful baseline evidence, but none inject mutations into the
small check/use windows described below, simulate dishonest streaming S3
responses, or mount and rerender the affected React workflows.

## Severity and confidence

- **High**: can publish unreviewed or secret data, bypass an explicit review
  boundary, lose user data, install/execute content outside the intended
  policy, retain credentials, corrupt the meaning of a storage base, or exhaust
  memory from a remote response.
- **Medium**: can lose a draft edit, present or execute an inconsistent
  workflow, leave recovery state incomplete, omit supported content, expose
  sensitive local metadata, or cause a substantial availability problem.
- **Low**: a dormant contract mismatch or confusing/inert behavior with limited
  present-day impact.
- **Confirmed** means the complete faulty path is present in source. A race is
  called confirmed when the check and mutation are separated and there is no
  lock or identity recheck covering the intervening actor; it does not mean the
  race was reproduced probabilistically during this audit.
- **Decision required** means the implementation is unambiguous but the desired
  product behavior needs confirmation before calling it definitively wrong.

## Prioritized finding index

| ID | Severity | Confidence | Area | Finding |
| --- | --- | --- | --- | --- |
| H-01 | High | Confirmed | Push/capture | Reviewed resource bytes are re-read without being bound to the reviewed digest |
| H-02 | High | Confirmed | Manifest/restore | Logical namespaces are not bound to resource kind, provider, scope, and policy |
| H-03 | High | Confirmed | Restore/delete | A case-only rename tombstone can delete the new live file |
| H-04 | High | Confirmed | Skill restore | Deep local skill files are omitted from verification and backup |
| H-05 | High | Confirmed | Restore/delete | Target digest checks are pathname TOCTOUs |
| H-06 | High | Confirmed | History repair | Rollback can erase a turn appended after repair |
| H-07 | High | Confirmed | Push/storage | A storage ID can be repointed while Push still uses its old destination |
| H-08 | High | Confirmed | Persistence | Full config save has a cross-document validation race |
| H-09 | High | Confirmed | S3 | S3 response limits are enforced after buffering the body |
| H-10 | High | Confirmed sanitizer gap; exploitability conditional | Logging | Authorization sanitization can leak Basic and SigV4 credentials |
| H-11 | High | Confirmed | Pull review | The UI reviews live data while Apply uses a pinned restore plan |
| H-12 | High | Confirmed | Pull review | Explicit selections survive replanning by resource ID |
| M-01 | Medium | Confirmed | Publication | Reused schema-4 objects are assumed present without verification |
| M-02 | Medium | Confirmed | Logging/privacy | Retained activity entries contain local absolute paths |
| M-03 | Medium | Confirmed | Logging/availability | Log queries and stats scan the full retention set under the writer lock |
| M-04 | Medium | Confirmed | Frontend concurrency | Old project requests can overwrite the newly selected project |
| M-05 | Medium | Confirmed | Setup | Closing setup can discard the latest debounced draft edit |
| M-06 | Medium | Decision required | Pull orchestration | Dependency installation continues after a non-throwing failed restore |
| M-07 | Medium | Confirmed | Dependency recovery | External dependency effects are journaled only after the whole loop |
| M-08 | Medium | Confirmed | Push UI | Two non-ready destination states remain actionable |
| M-09 | Medium | Confirmed | Project-file UI | Directory removal can construct a request the backend must reject |
| M-10 | Medium | Confirmed | Provider inventory | Config reads are unbounded or follow symlinks, and warnings are dropped |
| M-11 | Medium | Confirmed | Git identity | Worktrees and submodules lose repository identity |
| M-12 | Medium | Confirmed | Capture | Resource depth limits silently truncate trees |
| M-13 | Medium | Confirmed | History UI | Variant rows expose inert or out-of-plan checkboxes |
| M-14 | Medium | Confirmed | Capture/publication | Capture and domain logical-path validators disagree |
| L-01 | Low | Confirmed, dormant | DTO contract | TypeScript omits a serialized restore-plan field |

## High-severity findings

### H-01 — Reviewed resource bytes are not pinned through capture

**Impact.** A Push can publish bytes, modes, warnings, or even conversation
ownership that the user did not review. A benign project file can become
credential-shaped, private-key material, or executable after the review token
is accepted. Because project content is marked opaque, the newly detected
secret becomes a warning rather than a hard block, but acknowledgement was
already checked against the old warning digest.

**Trigger.**

1. Scan and review a non-Git project file.
2. Start Push with the returned review token.
3. Change or atomically replace the file after token validation and before
   capture.
4. Capture reads the replacement bytes under the old descriptor and approval.

The same shape exists for conversations: discovery establishes ownership and
resource identity by parsing the transcript, then capture later reads the path
without re-establishing that identity.

**Evidence.**

- `commands.rs:5579-5598` validates the review token.
- `commands.rs:5692-5693` invokes capture later.
- `provider_capture.rs:990-1038` reads scan-time bytes and records
  `_local_content_sha256` and `_local_review_digest`.
- Project files deliberately store `derived_bytes: None` at
  `provider_capture.rs:1059-1064`.
- `provider_capture.rs:356-410` validates metadata and then re-reads the live
  path without comparing the bytes or mode to the descriptor's reviewed
  digest.
- `provider_capture.rs:392-397` only appends a warning for newly detected
  secret material in opaque content.

**Why tests miss it.** The exact-bytes test at
`commands.rs:11352-11428` changes the file before a new Push discovery, so that
new discovery correctly rejects the stale token. No test mutates the file
between token acceptance and capture.

**Recommended correction.** Retain the scan bytes for reviewed project content,
or re-read through a bounded no-follow handle and compare content digest, mode,
file identity, warning digest, and resource ownership immediately before
accepting the captured result. Any mismatch must require a new scan and
approval. Add a deterministic failpoint between validation and capture.

### H-02 — Logical namespaces are not bound to resource semantics

**Impact.** A forged or malformed, but internally hash-consistent, remote
manifest can classify a `project/*` file as a non-project resource with
`SafeFile` policy. Restore then maps it into the checkout as a generic write,
can default-approve a missing target, and does not activate the Git-managed
project-content guard. For example, `project/.git/hooks/pre-commit` with mode
`0755` can bypass both project-file review and executable-install
classification.

This matters because SHA-256 proves that the bytes match the manifest; it does
not prove the publisher was authorized or that the manifest's semantics are
safe. The trigger requires a malformed bundle or an actor able to rewrite the
configured storage head and manifest; the normal publisher does not currently
emit this combination.

**Evidence.**

- `domain.rs:690-729` validates descriptor text and metadata, but not the legal
  combinations of kind, provider, scope, apply policy, or logical namespace.
- `domain.rs:2978-3021` applies project-content identity and size rules only
  when the descriptor already claims `ProjectContentFile`.
- `bundle_engine.rs:4451-4456` maps every `project/*` logical path into the
  checkout regardless of descriptor kind.
- `bundle_engine.rs:4390-4409` turns other descriptor kinds into generic
  `WriteFile` actions.
- `bundle_engine.rs:4413-4435` does not derive explicit approval from
  executable mode and permits a missing `SafeFile` target to remain implicit.
- `bundle_engine.rs:5668-5688` gates Git-managed folders only when the action
  is already one of the typed project-content action kinds.

**Why tests miss it.** Manifest tests validate known-good descriptors and
project-content identities. None assigns a valid namespace to the wrong
resource kind/policy and follows it through restore planning.

**Recommended correction.** Add one domain-level semantic matrix that binds
each logical namespace to allowed resource kinds, providers, scopes, policies,
file counts, and modes. Treat every `project/*` path as project content
independent of the claimed descriptor, and derive executable/explicit approval
from the materialized path and mode as a second line of defense.

### H-03 — Case-only rename tombstones can delete the new live file

**Impact.** On the default case-insensitive macOS and Windows filesystems, a
case-only rename such as `Foo.txt` to `foo.txt` can end with no file at all.
The manifest can contain live `project/foo.txt` and a tombstone for
`project/Foo.txt`. Restore writes or no-ops the live entry, then processes the
tombstone against the same physical pathname. If the bytes did not change, the
deletion digest still matches and the live file is removed.

An empty directory case-only rename has the analogous failure. Nonempty
directory deletion is correctly blocked by the exact-empty check at
`bundle_engine.rs:6102-6111`.

**Evidence.**

- Live file/directory collisions are case-folded at `domain.rs:2936-2993`.
- Tombstones are checked against live paths only by exact key at
  `domain.rs:3075-3124`.
- Restore plans live writes and tombstone deletions separately in
  `bundle_engine.rs:1904-2178`.
- Apply performs write actions before queued file and directory deletions at
  `bundle_engine.rs:5767-5793` and `bundle_engine.rs:5866-5891`.

**Why tests miss it.** Tombstone validation tests at
`domain.rs:4513-4540` cover exact identity, not case-fold equivalence or
cross-platform apply ordering.

**Recommended correction.** Reject any live/tombstone case-fold collision at
manifest validation time. If case-only rename support is desired, model it as
a dedicated atomic rename action and test it on case-sensitive and
case-insensitive filesystems.

### H-04 — Deep local skill files are omitted from verification and backup

**Impact.** A local custom skill with content at depth 17 or deeper can be
approved for overwrite without Mallard classifying the deep content. The pre-overwrite
backup uses the same silent depth cutoff, so those files are absent from the
backup. Apply then displaces and removes the original target. The overwrite
succeeds, but the deep local files are lost and cannot be recovered from the
recorded backup.

There is a related portability mismatch: capture permits up to 20,000 files and
depth 32, while staging/target verification permits 4,096 files and depth 16.
Such a published skill is rejected during staging verification. That rejection
happens before the target is displaced, so it is an availability defect rather
than the destructive path above.

**Evidence.**

- Live target classification walks only to depth 16 and does not probe for an
  overflow entry at `bundle_engine.rs:3907-3908` and
  `bundle_engine.rs:4020-4064`.
- Pre-overwrite backup copying also stops without an overflow error at
  `bundle_engine.rs:6469-6503`.
- Apply displaces the existing skill and later removes the displaced tree at
  `bundle_engine.rs:6416-6447`.
- Staging is correctly verified before displacement at
  `bundle_engine.rs:6368-6395`, but it uses the smaller verifier limits.
- Capture permits `MAX_DISCOVERED_FILES = 20_000` and
  `MAX_TREE_DEPTH = 32` at `provider_capture.rs:23-28` and
  `provider_capture.rs:2229-2280`.

**Why tests miss it.** Skill tests use small, shallow fixtures. None puts an
existing local-only file below depth 16 and verifies backup/overwrite
behavior, or publishes a 4,097-file/depth-17 skill and checks that restore
rejects it safely.

**Recommended correction.** Use one shared tree limit and one traversal helper
for discovery, capture, backup, staging verification, and target verification.
Every traversal must probe one level/entry beyond the limit and fail closed
instead of truncating. Retain the displaced target until final verification
succeeds, and prove the backup digest covers the same complete tree that was
classified.

### H-05 — Restore target checks are pathname TOCTOUs

**Impact.** An editor or provider process can atomically replace a target after
Mallard verifies its digest but before Mallard writes or unlinks the pathname.
Mallard can then overwrite an unbacked replacement file or delete bytes that
never matched the reviewed deletion digest. If a target was absent when
checked and appears before rename, it can be overwritten without any backup.

**Evidence.**

- File apply validates target state at `bundle_engine.rs:6164-6173`, performs
  backup work, and finally replaces the target at
  `bundle_engine.rs:6227-6242`.
- Project-file deletion reads and verifies the target at
  `bundle_engine.rs:6022-6037`, writes and verifies a backup, then unconditionally
  unlinks the pathname at `bundle_engine.rs:6045-6061`.
- These checks are path-based; no opened inode or parent-directory handle binds
  the checked object to the final mutation.

**Why tests miss it.** Existing tests mutate a target before apply starts and
correctly observe rejection. None injects a rename between inspection and the
final replacement/unlink.

**Recommended correction.** Use no-follow descriptor-relative operations
(`openat`/equivalent) and bind check, backup, and mutation to the same object
identity. At minimum, recheck file identity, digest, and mode immediately
before the final rename/unlink and abort if anything changed. Add failpoints on
both sides of backup creation.

### H-06 — Conversation repair rollback can erase appended turns

**Impact.** Codex can append a turn after Mallard repairs one transcript but
before a later repair or final audit fails. Rollback unconditionally writes
the original pre-repair bytes over every completed file, destroying the newly
appended turn. Mallard's host-operation lock does not coordinate with Codex.

**Evidence.**

- Repair takes immutable backups and checks the original bytes at
  `commands.rs:10010-10042`.
- The rollback closure unconditionally rewrites each completed path with the
  original bytes at `commands.rs:10044-10053`.
- It runs after later read, mutation, write, or audit failures at
  `commands.rs:10059`, `10076`, `10091`, `10117`, and `10133`.

**Why tests miss it.** The repair test around `commands.rs:11474` covers a
successful, quiescent repair. It does not append to an already repaired
transcript before forcing a later failure.

**Recommended correction.** Before rollback, require the current digest to
equal the exact repaired digest. If it differs, preserve the concurrent change
and report a recovery conflict. Prefer per-file transactions, or retain the
displaced inode until the whole repair group commits.

### H-07 — Storage settings can be repointed during Push

**Impact.** Push can publish to an old endpoint/bucket/local directory and then
record the returned generation as the base for a newly repointed storage with
the same ID. Mallard can report the project as synced while future Pull/Push
operations address a different store.

**Trigger.** While Push is uploading, save settings that change an existing
storage ID's endpoint, bucket, credentials, or local directory.

**Evidence.**

- Push acquires the host-operation lock and builds its engine from the current
  storage at `commands.rs:5478` and `commands.rs:5576-5577`.
- It publishes at `commands.rs:5703-5724`.
- Finalization checks project and link recipe fields, but not the storage
  configuration or the config revision, at `commands.rs:5732-5813`.
- `save_project_sync_config` at `commands.rs:966-999` does not acquire the
  host-operation lock.

**Why tests miss it.** Host-lock exclusion tests around `commands.rs:10455`
cover several workflows but omit full config save/storage mutation during
Push.

**Recommended correction.** Serialize storage mutation with sync operations,
and pin a normalized storage-configuration fingerprint in every prepared Push
or Pull. Revalidate it immediately before publication and again before
recording the base.

### H-08 — Full config save has a cross-document validation race

**Impact.** Persisted config can remove a project or storage that a concurrently
created binding references, or can invalidate the path-isolation decision that
was just checked. Subsequent binding loads may fail validation, leaving the
machine state inconsistent.

**Evidence.**

- `commands.rs:992-999` loads bindings, validates the submitted config against
  them, validates storage isolation, and only then calls `save_config`.
- `persistence.rs:584-605` and `persistence.rs:624-650` acquire separate
  persistence locks for config and binding operations. The first lock is
  released before config save acquires another.
- A binding mutation can therefore commit between cross-document validation
  and config persistence.

**Why tests miss it.** Persistence tests cover atomicity and optimistic
revision within one document. Storage-isolation tests are synchronous and do
not interleave a binding mutation.

**Recommended correction.** Add a repository transaction that loads config,
host identity, bindings, and profiles under one persistence guard; validates
the submitted config against that current state; and writes config before
releasing the guard. A host lock alone is weaker unless every binding mutation
also participates.

### H-09 — S3 read limits are enforced after buffering the body

**Impact.** A dishonest or merely chunked S3-compatible server can make Mallard
allocate far beyond the configured object limit before rejection. The domain
limit is 512 MiB per object, and bulk fetch permits six concurrent GETs, so the
process can approach multiple gigabytes of response allocation.

**Evidence.**

- `s3_store.rs:134-138` rejects an oversized declared `Content-Length`.
- `s3_store.rs:143-155` calls `response.body.collect()` before checking the
  actual length.
- `s3_store.rs:397-427` permits six concurrent bulk downloads.
- `domain.rs:44` sets `MAX_FILE_BYTES` to 512 MiB.

**Why tests miss it.** S3 tests cover ordinary GET, CAS, listing, and error
paths, but not a chunked response or a falsely small `Content-Length` that
streams past the limit.

**Recommended correction.** Stream into a bounded reader, stop at
`MAX_FILE_BYTES + 1`, and cancel the response immediately when exceeded.
Consider an aggregate in-flight byte budget in addition to the concurrency
semaphore.

### H-10 — Authorization sanitization can leak Basic and SigV4 credentials

**Impact.** If a storage/provider error includes an `Authorization` header,
activity JSONL and the UI can retain credentials. For
`Authorization: Basic <base64>`, the sanitizer replaces only `Basic` and leaves
the base64 credential. For AWS SigV4, it replaces only the scheme and leaves
`Credential=...` and `Signature=...`.

The sanitizer failure is confirmed. This audit did not observe a production
AWS SDK error that actually includes its request `Authorization` value, so the
reachable credential leak remains conditional on an emitter passing such text
to the activity logger.

**Evidence.**

- `activity_log.rs:929-949` includes `authorization:` in the marker list.
- `activity_log.rs:954-995` special-cases only a following `Bearer ` token, then
  redacts one whitespace-delimited word.
- The marker list covers query-style `x-amz-credential=` and
  `x-amz-signature=`, not the generic SigV4 header fields.
- Push and Pull persist error strings through activity events, including
  `commands.rs:1725-1730` and `commands.rs:1931-1936`.

**Why tests miss it.** `activity_log.rs:1308-1326` covers Bearer and
`x-amz-signature`, but not Basic, AWS4-HMAC-SHA256, quoted JSON header values,
or mixed punctuation.

**Recommended correction.** Parse `Authorization:` as a whole sensitive header
and replace its complete value up to the line boundary. Add table-driven tests
for Basic, Bearer, SigV4, proxy authorization, quoted JSON, and
case/whitespace variants.

### H-11 — Pull review renders live data instead of the pinned plan

**Impact.** A user can inspect one set of sessions, skills, and plugins while
Apply submits actions from another set. Planned sessions outside the live
30-day/current-branch history window can also be invisible during review.
Backend pin checks may reject some stale cases, but they cannot repair the fact
that the user's selection and understanding were formed from the wrong data.

**Evidence.**

- The immutable restore plan pins generation, commit, manifest, binding, and
  actions in `src/types.ts:739-755`.
- `RestorePlanView.tsx:602-615` embeds `ProjectChatHistoryPage` without a
  plan-derived comparison override.
- `ProjectChatHistoryPage.tsx:833-878` fetches current 30-day history and a live
  `getThreadSyncComparison`.
- `RestorePlanView.tsx:632-667` embeds `SkillsPluginStatusPage` without a
  report override.
- `SkillsPluginStatusPage.tsx:407-432` therefore fetches live
  `getCapabilityStatus`.
- The Push review already demonstrates the intended pattern by passing
  prepared overrides in `PushResourceWorkspace.tsx:617` and `637`.

**Why tests miss it.** Pull tests mostly render static markup or inspect source
text. They do not mount effects, mutate local/remote state after planning, and
assert that every visible row still comes from the plan.

**Recommended correction.** Build all review rows from plan-pinned support
data, pass explicit comparison/report overrides, and prohibit live fetches
inside a Pull review. Include planned sessions even when they fall outside the
normal History browsing window.

### H-12 — Explicit selections survive replanning by resource ID

**Impact.** If a resource keeps the same ID but its action, bytes, target, or
approval reason changes, a selection made for plan N is automatically applied
to plan N+1. This can carry explicit approval for an overwrite/install into a
materially different action without renewed user consent.

**Evidence.**

- `RestorePlanView.tsx:323-344` keys both `knownItems` and `selected` only by
  `resourceId`.
- `ProjectSyncV3.tsx:2115-2126` replaces the restore plan after a partial or
  skipped pass without remounting the review.
- `ProjectSyncV3.tsx:2144-2170` refreshes and replaces the plan; even when it
  clears result/completion sets for a new generation or manifest, it does not
  clear the child's resource-ID selection memory.
- Selection building maps those retained resource IDs onto the current plan's
  new action IDs.

**Why tests miss it.** There is no mounted rerender test that changes plan ID,
generation, manifest, action kind, or materialized digest while preserving a
resource ID.

**Recommended correction.** Scope review state to `plan_id` and to an approval
fingerprint containing action ID, kind, target, source/materialized digest, and
explicit-approval reason. Clear or reconfirm every explicit selection whenever
that fingerprint changes.

## Medium-severity findings

### M-01 — Reused schema-4 objects are assumed present

**Impact.** If an operator, lifecycle rule, or storage fault deletes or corrupts
a content-addressed object, a later non-noop Push can advance the head while
still referencing the missing object. The new generation is structurally valid
but cannot be fully restored.

**Evidence.**

- `bundle_engine.rs:1069-1082` derives `previous_shas` solely from the previous
  manifest.
- `bundle_engine.rs:1116-1120` skips uploading captured bytes whenever their
  digest appears in that set.
- The schema-4 path does not re-read and hash each reused object before
  advancing the head.

**Recommended correction.** Before reuse, verify object existence, size, and
SHA-256 from the current store. If missing or corrupt, re-upload the captured
bytes before publication. Add local-folder and S3 tests that delete/corrupt a
previous object between generations.

### M-02 — Activity logs persist absolute local paths

**Impact.** Usernames, checkout locations, provider-home locations, CLI paths,
and thread identifiers remain in retained JSONL. This contradicts the explicit
repository invariant that local absolute paths must not be logged.

**Evidence.**

- `commands.rs:1140-1160` and `commands.rs:1168-1188` log the complete command
  used to open a thread.
- `chat_history.rs:2318-2360` includes absolute CLI, project-root,
  `CODEX_HOME`, and deep-link path values in those commands.
- Successful Pull also constructs project/provider-home commands at
  `commands.rs:6792-6817` and logs them from the Pull flow.
- `ActivityLogEntry::new` persists `sanitize_message` output at
  `activity_log.rs:141-163`; `activity_log.rs:929-951` performs credential
  marker replacement but no path redaction.

**Recommended correction.** Log a structured event such as “Opened thread in
Terminal” with non-sensitive stable IDs; never log an executable command.
Apply a conservative absolute-path scrubber as defense in depth and add
persistence tests.

### M-03 — Log query and stats scan all retained data under the writer lock

**Impact.** A broad query over a large retained history can allocate and sort
all matching entries despite a requested page of at most 2,000. Query/stats
also hold the process-global log lock used by append, delaying progress logs
and other command activity. Policy permits up to 10 GiB retained.

**Evidence.**

- `activity_log.rs:327-378` reads every line, pushes every match, sorts the full
  vector, and truncates only afterward.
- `activity_log.rs:397-401` and `activity_log.rs:554-577` parse all lines for
  stats under the same lock.
- `activity_log.rs:299-324` uses that lock for append.
- `activity_log.rs:208-220` permits a 10 GiB policy.

**Recommended correction.** Iterate files newest-first, maintain only a
bounded top-k/page window, and stop when cursor/limit make older segments
irrelevant. Do not hold the process-global writer lock while parsing immutable
segments; use short metadata/rotation locks instead.

### M-04 — Old project requests can overwrite the new selection

**Impact.** Rapidly switching projects can show project B's detail, inventory,
providers, resource counts, readiness, or error under active project C.
Detail-dependent actions can then run against mismatched UI state.

**Evidence.**

- `ProjectSyncV3.tsx:837-905` guards `loadProjectData` only with the host-wide
  workflow generation, which does not change on project selection.
- `ProjectSyncV3.tsx:1019-1022` can start another project request immediately.
- Project buttons are disabled only by `busy`, not `loading`, in
  `ProjectSidebar.tsx:253-258`.
- Background reconcile at `ProjectSyncV3.tsx:946-973` and post-Push reconcile
  at `ProjectSyncV3.tsx:1466-1492` have the same missing project/request check.
- `ProjectSyncV3.tsx:563-594` attributes the shared inventory/readiness state to
  whichever project ID is currently active.

**Recommended correction.** Use a monotonic project-detail request ID and
capture the project ID for every state commit. Commit only when both still
match. Apply the same guard to background/post-Push reconcile and add a
deferred-Promise rapid-switch test.

### M-05 — Closing setup can lose the latest draft edit

**Impact.** The close button says the draft is kept, but an edit made within
the debounce window exists only in component state. Unmount cancels the timer,
so the server keeps the older draft and the latest user input silently
disappears.

**Evidence.**

- `ProjectSetupWorkspace.tsx:163-200` debounces autosave and clears the timer in
  effect cleanup.
- `ProjectSetupWorkspace.tsx:202-207` records edits only in local state until
  that save runs.
- The close button at `ProjectSetupWorkspace.tsx:502-504` is disabled only
  while finalizing.
- `ProjectSyncV3.tsx:1137-1140` closes by unmounting the draft workspace and
  refreshing summaries.

**Recommended correction.** Flush and await dirty draft state before closing,
or keep the workspace mounted until the save completes. Change the label only
if loss is intentionally allowed. Test edit-immediately-close with fake
timers.

### M-06 — Dependency installation continues after `RestoreResult.success=false`

**Impact.** A normal, non-throwing partial restore result can be followed by
global plugin/tool installation. The composite result is marked failed only
after those side effects. If the intended contract is “restore first, install
only when restore succeeded,” this violates phase ordering.

**Evidence.**

- `pullReviewFlow.ts:127-135` records a returned restore result but sets
  `executionError` only on a thrown exception.
- `pullReviewFlow.ts:137-147` proceeds to dependency installation whenever
  `executionError` is empty.
- `pullReviewFlow.ts:158-162` consults `restoreResult.success` only when
  computing the final outcome.

**Why this needs a decision.** The behavior may be an intentional
progress-making policy because dependencies were separately approved. The UI
and tests do not state that policy, and the phased naming implies a gate.

**Recommended correction.** Decide and document the contract. If restore
failure is a gate, stop before installation and add a `success: false` test. If
progress is intended, surface that clearly and journal it as an independent
operation.

### M-07 — Dependency side effects are journaled only after the loop

**Impact.** A process crash or persistence failure after `codex plugin add` or
`claude plugin install` succeeds but before finalization loses the receipt.
Retry can rerun an already successful external operation, and readiness cannot
reconstruct partial progress.

**Evidence.**

- `commands.rs:6315-6372` executes actions sequentially and retains receipts
  only in memory.
- Durable mutation occurs once at `commands.rs:6374-6429`, after all actions
  have run.

**Recommended correction.** Persist a prepared operation before the first
side effect and durably append/update each action receipt immediately after
execution. Make resume reconcile current provider state before rerunning an
action.

### M-08 — `requires_pull` and `unavailable` destinations remain actionable

**Impact.** The current Push review can show an enabled Push action even though
the backend deterministically rejects it. The user receives a generic failure
and can retry a state that requires Pull or storage recovery.

**Evidence.**

- `src/types.ts:665-683` defines `ready`, `remote_history_missing`,
  `requires_pull`, and `unavailable`.
- `PushResourceWorkspace.tsx:413-416` and `PushResourceWorkspace.tsx:756`
  special-case only `remote_history_missing`.
- `ProjectSyncV3.tsx:2296-2314` disables Continue only for that same state.
- Backend `commands.rs:5181-5209` rejects every non-ready state.
- The new integration test at
  `resource-inventory.integration.test.tsx:243-277` covers only missing remote
  history.

**Recommended correction.** Treat `state !== "ready"` as a Push blocker and
render state-specific recovery: Pull for `requires_pull`, retry/settings for
`unavailable`, and unlink/relink for missing history. Add one test per union
variant.

### M-09 — Directory removal creates a predictably invalid Push

**Impact.** “Remove from storage” can remain enabled for a stored directory
while selected stored descendants remain—for example, a selected empty
descendant directory. Selected file descendants normally mark their ancestors
as required and disabled, but the requirement calculation does not cover every
selected directory descendant. The UI removes only the parent directory ID,
then the backend rejects the request.

**Evidence.**

- `ProjectFilesReviewPage.tsx:299-303` renders the removal button for every
  storage-present row, including directories.
- `PushResourceWorkspace.tsx:287-303` marks ancestor directories required only
  from selected file entries.
- `ProjectSyncV3.tsx:2355-2375` toggles only that resource ID.
- `commands.rs:5554-5561` preserves every unmarked stored entry.
- `commands.rs:5659-5676` rejects directory removal while stored descendants
  remain selected.

**Recommended correction.** Either cascade removal to all stored descendants
with a visible count/confirmation, or disable directory removal until
descendants are removed. Unit-test request construction, not only static
markup.

### M-10 — Provider config reads are not consistently bounded or no-follow

**Impact.** A huge `config.toml` can cause unbounded allocation during
inventory/setup inspection. A symlinked provider config is followed outside
the mapped home. Inventory failures can also silently remove plugins from the
Push review because warnings are discarded.

**Evidence.**

- Codex inventory uses unrestricted `fs::read` at
  `global_inventory.rs:176-205`.
- Codex setup/inspection repeats unrestricted reads at
  `codex_config.rs:143-149` and `codex_config.rs:631-649`.
- Claude inventory bounds size but follows the leaf symlink via `fs::metadata`
  and `fs::read` at `codex_plugins.rs:2164-2180`.
- `commands.rs:7271-7280` consumes skills/plugins/blocked skills from
  `GlobalInventory` but drops `inventory.warnings`.

**Recommended correction.** Centralize a bounded, regular-file, no-follow
provider-config reader. Surface inventory warnings in `Discovery` and the
review UI so failure cannot look like an intentional empty inventory.

### M-11 — Git worktrees and submodules lose repository identity

**Impact.** Setup in a linked worktree or submodule produces no repository
fingerprint. Verified remote matching is disabled, leading to unnecessary
mismatch acknowledgement or unidentified-repository behavior for common Git
layouts.

**Evidence.**

- `commands.rs:7317-7345` reads only `<project>/.git/config` and requires it to
  be a regular file.
- Linked worktrees and submodules normally use a `.git` text file that points
  to the actual gitdir.

**Recommended correction.** Resolve `git rev-parse --git-dir` and
`git config --get remote.origin.url` with the same bounded subprocess policy
used elsewhere, or safely parse the `.git` gitdir file and commondir. Add
worktree and submodule fixtures.

### M-12 — Resource depth limits silently truncate trees

**Impact.** A standalone skill or Claude memory tree can contain shallow files
and deeper files, symlinks, or special nodes beyond depth 32. Discovery and
capture succeed with a partial snapshot; deep content silently disappears and
deep forbidden nodes do not block capture.

**Evidence.**

- `provider_capture.rs:2248-2279` walks with
  `.max_depth(MAX_TREE_DEPTH)` and never probes one level deeper.
- Preflight at `provider_capture.rs:2283-2287` has the same limit.
- The project-content walker elsewhere uses `max_depth + 1` and explicitly
  rejects overflow, showing the missing boundary check is not inherent.

**Recommended correction.** Walk one level beyond the permitted depth and
fail the whole resource if any entry exists there. Reuse the same helper for
preflight and capture.

### M-13 — Conversation variant rows expose inert or out-of-plan checkboxes

**Impact.** Ordinary History can display checkboxes that do nothing. Pull/Push
can display a live comparison row that is not selectable in the current plan;
the user can toggle it visually, but selection building later drops it.

**Evidence.**

- `ProjectChatHistoryPage.tsx:693-700` renders `ThreadVariantReview` whenever
  thread entries exist, without passing `selectionMode` or the caller's
  plan-derived selectable set.
- `ThreadVariantReview.tsx:113-156` decides checkbox visibility only from the
  live `proposed_action` and invokes an optional handler.
- Ordinary thread rows correctly gate on selection mode and
  `selectableResourceIds` at `ProjectChatHistoryPage.tsx:552-562` and
  `ProjectChatHistoryPage.tsx:712-715`.

**Recommended correction.** Pass selection mode and allowed resource IDs into
variant review, and render a checkbox only when both authorize it. Add
integration coverage through the normal History/Pull page rather than testing
the leaf component alone.

### M-14 — Capture and domain logical-path validators disagree

**Impact.** A resource can appear valid in inventory and be fully read during
capture, then fail only during publication. Capture permits paths up to 4,096
bytes and does not reject colons or trailing spaces/dots; the portable domain
caps paths at 1,024 bytes and rejects those forms.

**Evidence.**

- Capture validation is at `provider_capture.rs:2409-2426`.
- Portable `LogicalPath` validation is at `domain.rs:410-449`.

**Recommended correction.** Remove the duplicate validator and parse
`LogicalPath` at discovery time. Add parity/property tests over boundary
lengths, Windows-reserved forms, colons, and trailing characters.

## Low-severity contract drift

### L-01 — TypeScript omits `RestorePlan.conversation_actions`

Rust serializes `conversation_actions` at `domain.rs:3473-3492`, while the
TypeScript `RestorePlan` at `src/types.ts:739-755` omits it. The current
schema-4 production restore builder initializes the field empty at
`bundle_engine.rs:2184-2200`, and current apply behavior uses `plan.actions`,
so no active user-path failure was found. It still violates the repository's
DTO synchronization rule and will become a runtime/UI defect if conversation
actions become nonempty before the frontend contract is updated.

Add the field now, or remove it from the serialized production DTO until it is
active. Include a Rust-to-TypeScript contract fixture that exercises every
serialized field.

## Cross-cutting causes

### Check/use gaps across external actors

The host-operation and persistence locks serialize Mallard operations, but
they cannot serialize Codex, Claude, editors, filesystem watchers, or remote
storage administrators. Several implementations treat a prior pathname read
as if it pinned a file:

- review token to capture (H-01);
- restore inspection to rename/unlink (H-05);
- conversation repair to group rollback (H-06); and
- previous manifest reference to reused object (M-01).

The durable boundary must either retain the exact bytes/object handle being
approved or revalidate identity and digest immediately at the mutation.

### Approval identity is weaker than action identity

The frontend often uses stable resource IDs as UI identity, but approval is for
an action against exact bytes, target, mode, and plan generation. H-11 and H-12
are two manifestations of the same mismatch. Backend APIs should accept an
approval token/fingerprint issued for the exact plan action, not rely on the
frontend to translate a remembered resource selection into new action IDs.

### Bounds are duplicated rather than shared

Capture, portable validation, skill verification, backup copy, S3 buffering,
history parsing, and log queries each implement their own limits. H-04, H-09,
M-03, M-12, and M-14 result from applying a limit after allocation, choosing
incompatible values, or silently stopping at the boundary. A bounded helper
should fail closed and be shared by all phases that must agree.

### Tests favor static or quiescent behavior

The backend suite has strong coverage for static revision, digest, CAS,
symlink, and backup behavior. The frontend suite has broad rendering and
contract coverage. The missing layer is deterministic interleaving:

- mutation failpoints between check and use;
- dishonest streaming transports;
- crash/restart after each external side effect;
- cross-platform case-fold fixtures; and
- mounted React tests with deferred promises, effects, rerenders, and unmount.

## Recommended remediation sequence

1. **Freeze safety-critical inputs.** Fix H-01, H-03, H-05, and H-06. Add
   deterministic mutation failpoints first so each fix has a stable regression
   test.
2. **Harden semantic validation.** Fix H-02, H-04, M-12, and M-14 with shared
   path/tree validators and fail-closed namespace rules.
3. **Restore approval integrity.** Fix H-11 and H-12 by making plan data the
   only Pull review source and scoping approvals to exact action
   fingerprints.
4. **Serialize configuration and storage identity.** Fix H-07 and H-08 with a
   cross-document transaction and a pinned storage fingerprint.
5. **Close secret and memory-exhaustion paths.** Fix H-09, H-10, M-02, M-03,
   and M-10.
6. **Make side effects recoverable.** Fix M-01 and M-07, then exercise
   crash/restart after every publication/install phase.
7. **Repair workflow correctness.** Address M-04 through M-06, M-08 through
   M-09, M-11, M-13, and L-01.

## Regression test matrix

| Boundary | Minimum new test |
| --- | --- |
| Review/capture | Change bytes, mode, warning status, and transcript ownership at a failpoint after token validation |
| Manifest semantics | Forge each valid namespace with every disallowed resource kind/policy and executable mode |
| Case-fold restore | Live/tombstone case-only file and empty-directory rename on a case-insensitive target model |
| Skill transaction | Existing target with a local-only depth-17 file; assert overwrite is blocked or the complete file survives in backup. Separately assert a 4,097-file/depth-17 captured skill is rejected before target mutation |
| File apply/delete | Replace target after inspection and after backup; assert no overwrite/unlink |
| Conversation repair | Append to the first repaired file, fail the second, and assert the append survives |
| Storage/config race | Repoint storage and mutate bindings while Push/config save is paused at deterministic barriers |
| S3 bounds | Chunked and dishonest-length responses that exceed the object and aggregate budgets |
| Log sanitizer | Basic, Bearer, SigV4, quoted JSON, URLs, absolute POSIX paths, and Windows paths |
| Pull review | Mount effects, change live state after planning, and assert rows/selections remain plan-pinned |
| Replan approval | Preserve resource ID while changing action/digest; assert explicit selection is cleared |
| Object reuse | Delete and corrupt a reused local/S3 object before the next publication |
| Project switch | Resolve project requests out of order and assert only the current project commits state |
| Draft autosave | Edit and close before the debounce expires; assert the persisted draft has the edit |
| Dependency recovery | Crash after each successful external action and resume without rerunning it |
| Push destinations | Render and attempt every `PushDestinationStatus` union member |
| Provider configs | Oversized and symlinked Codex/Claude config files with visible warning assertions |
| Git identity | Normal checkout, linked worktree, submodule, bare/worktree edge, and missing origin |
| Tree/path limits | Entry at depth 33 and logical-path boundary/property parity |
| DTO parity | Serialize a fully populated Rust `RestorePlan` and validate the frontend fixture |

## Caveats

- This was a source and test audit, not a penetration test.
- Race findings were established from deterministic check/use separation and
  missing coordination; probabilistic live reproductions were not added.
- No real provider home, real `~/.mallard`, or production storage was used.
- No S3 server was asked to stream an oversized dishonest response.
- The report documents defects only; it intentionally does not modify product
  code.
- Line numbers refer to the audited working tree and will drift as fixes land.

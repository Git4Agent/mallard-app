# Claude Sidecar Restore on Pull

- Status: Implemented 2026-09-05 at minimal scope (see §10 for the deltas
  from the design); performance measured 2026-09-05 (§9)
- Date: 2026-09-05
- Scope: Project Sync Pull of Claude conversations that carry `todos` and
  `file-history` sidecars
- Baseline: branch `extensible-providers`, `HEAD` a5a47a1 + working tree.
  Line numbers are from this tree and will drift.
- Finding: P2 "Claude session attachments are published but not restored"

## 1. Decision

A Pull that materializes a Claude conversation installs every sidecar the
cloud row carries, in the same provider operation as the transcript, under
the destination session id. A conversation whose sidecars cannot be fetched,
verified, or placed fails as one thread; the transcript is not installed
without its attachments.

Nothing changes for Push, for the bundle layout, for schema 1, or for Codex.
No sidecar is ever deleted by Pull.

## 2. Where it stands today

### 2.1 Push is complete

Discovery associates `file-history/<session>/**` and
`todos/<session>-agent-<agent>.json` with a proven session through the exact
path grammar (`providers/claude/discovery.rs:17-60`, `:135-172`). Capture
consolidates them under the conversation resource
(`provider_capture.rs:1136-1160`), scrubs them at capture
(`provider_capture.rs:526-549`), and the publication builder certifies and
ships each one under
`state/conversations/claude/<cloud-thread-id>/sidecars/<kind>/<relative>`
(`service.rs:8028-8062`, committed at `service.rs:8254-8256`). The thread
record lists them with `ConversationFileRole::Sidecar { sidecar_kind }`
(`service.rs:7320-7336`). The round-trip test pins that they ship scrubbed
(`service.rs:21617-21624`).

### 2.2 Pull stops at the names

| Stage | What happens to sidecars today |
| --- | --- |
| Review pins (`service.rs:5764-5790`) | in-place `MaterializeCloud` computes sidecar targets through `materialization_targets` and pins each with `bounded_file_expectation` — paths only |
| Prepare (`service.rs:9820-9835`, `:9849`, `:9980`) | prefetch and `body()` cover the transcript only; `cloud_sidecars` carries `(kind, relative_path)` pairs, no bytes |
| Apply, minted identity (`service.rs:10385-10412`) | `materialization_targets` plans the sidecar targets so the collision check covers them, then discards everything but the transcript target |
| Apply, in place (`service.rs:10375-10383`) | takes the first non-index pinned target as the transcript; the pinned sidecar targets are ignored |
| Payload (`service.rs:10414-10420`, `:10486-10493`) | `targets` = transcript, plus the Codex index fragment |
| Engine (`conversation_materialization.rs:582-620`) | requires exactly one transcript; accepts any number of sidecars; a `Sidecar` payload is validated as one JSON value (`:692-695`) |

The generic file planner skips every file of a conversation resource
(`bundle_engine.rs:1408-1410`). That is correct and stays: sidecar logical
paths live under `state/conversations/…`, which `provider_state_target`
(`bundle_engine.rs:4111-4124`) cannot map, so routing them through the
generic path would produce "No safe materializer" manual rows, not
restores. The conversation action is the only owner of these files.

The gap is documented as known in
`PLAN_CREDENTIAL_REDACTION.md` §11 item 6 ("Sidecars do not materialize on
pull … noted for whoever builds sidecar pull") and in
`DESIGN_QODER_SUPPORT.md` §2 ("Pull preparation fetches the transcript but
carries only sidecar names"). `git log -S cloud_sidecars` shows the
name-only plumbing arrived in 9d6964e and was never extended.

### 2.3 Three latent traps the fix would trip

1. **The engine's JSON check rejects file-history.** `verify_provider_payload`
   parses every `Sidecar` payload as one JSON value
   (`conversation_materialization.rs:692-695`). `todos` files are JSON
   arrays; `file-history` entries are raw snapshots of edited project files
   (the redaction plan's routing table calls them "routing, else raw
   `Bytes`"; the round-trip fixture writes `APP=demo\nDB_PASSWORD=…`). The
   first file-history restore would fail the thread with "conversation
   sidecar is not valid JSON".
2. **Three unrelated caps of 64.** `ConversationTargetPins::validate`
   refuses more than 64 pinned provider targets (`lib.rs:3287`);
   `ConversationApplyOperation::validate` refuses more than 64 apply
   targets (`lib.rs:2282`); `prepare_operation` refuses more than 64
   payloads (`conversation_materialization.rs:366`). Push has no such
   bound: a thread may list up to `MAX_FILES` (100 000) files
   (`lib.rs:2858`), and file-history grows one entry per edit per file.
   A session with 63 or more sidecars pushes fine today and would fail
   review (in place) or apply (any action) once sidecars are installed.
3. **Transcript selection by position.** The in-place branch picks the
   transcript as the first pinned target that is not the index
   (`service.rs:10375-10383`). It works because
   `materialization_targets` emits the transcript first. `ProviderStateExpectation`
   carries no role, so once sidecars are consumed at apply this selection
   must be by role, not by order.

### 2.4 Facts that shape the design

- Sidecar bytes need no decoding. They are wire-form at capture (scrubbed
  once, certified at publish) and carry no path map; the manifest digest
  verified by `EngineByteSource::ensure_loaded` is the whole integrity
  check. The transcript's `redaction_ruleset_mismatch` gate at prepare
  covers the row, sidecars included.
- Destination names are already derived. `materialization_targets`
  (`providers/claude/materialization.rs:55-78`) renames every occurrence of
  the source session id in the kind-relative path to the destination id,
  which also covers the main-agent todos name `<id>-agent-<id>.json`.
- The engine already does the hard part. `ensure_provider_target_parents`
  creates missing nested parents (`file-history/<id>/`) as 0700 real
  directories inside the profile root; `prepare_operation` stages beside
  the target, backs up every `Sha256`-expected target under
  `~/.mallard/backups/conversations/<operation>/`, and `commit_one_target`
  publishes with no-clobber hard links or digest-guarded renames.
- Expectations exist for both identities. Minted identities (`CreateLocal`,
  `KeepBoth`) require every planned target `Absent` (`service.rs:10397-10404`).
  In-place updates pin each sidecar's current digest at review
  (`service.rs:5780-5790`) and re-derive it at prepare
  (`service.rs:9883-9899`); `validate_conversation_action_pins` compares
  the two as path-keyed maps (`conversation_sync.rs:212-222`).
- This machine has neither `~/.claude/todos` nor `~/.claude/file-history`
  (its buckets carry `<session>/tool-results/` instead, which discovery does
  not capture). Sidecar coverage is fixture-driven; the live Tuzi/aimock
  gates will not exercise it unless a fixture seeds both directories.

## 3. Principles

- **One operation per thread.** Sidecars ride in the transcript's
  `ConversationMaterializationRequest`. No second engine call, no separate
  receipt, no partial "transcript restored, attachments pending" state.
- **Fail closed per thread, never per file.** A missing object, an
  oversized member, a target outside the profile, or a failed payload
  check fails that conversation's receipt and leaves its other approved
  siblings untouched (the per-thread isolation already in
  `apply_bundle_restore_with_progress`).
- **Adapters own provider shapes; the core owns safety.** The Claude
  adapter decides what a valid `todos` or `file-history` payload is. The
  core keeps the boundary, no-follow, collision, size, digest, backup, and
  no-clobber checks.
- **Pull writes, Pull never deletes.** A local sidecar the cloud row no
  longer carries stays on disk. Deletion remains opt-in and reviewed, and
  no review row exists for sidecars.
- **One bound, shared by both directions.** A conversation is either
  syncable end to end or blocked at discovery with a reason. Push must not
  accept what Pull cannot install.

## 4. Design

### 4.1 Domain: one file-count bound (`crates/mallard-domain/src/lib.rs`)

Add `pub const MAX_CONVERSATION_FILES: usize = 4_096;` — the most files
(transcript, sidecars, index fragment) one conversation operation may
carry. Replace the three literal 64s:

- `ConversationTargetPins::validate` (`lib.rs:3287`);
- `ConversationApplyOperation::validate` (`lib.rs:2282`);
- `prepare_operation` (`conversation_materialization.rs:366`).

Apply the same bound to `ConversationThread::validate` in place of
`MAX_FILES` for `files.len()` (`lib.rs:2858`), so a manifest cannot describe
a thread the engine refuses. `MAX_FILES` keeps its other uses.

4 096 is a ceiling, not a target: a file-history entry is one edit of one
file, and a long session with a few hundred edits stays far below it. The
review's pinned `provider_state` for an in-place update lists every sidecar,
and `bounded_file_expectation` reads and hashes each, so the bound also caps
review-time I/O per thread.

### 4.2 Discovery: block a session over the bound (`provider_capture.rs`)

In `install_provider_discovery` (`:1136-1170`), after consolidating the
sidecar refs, if `1 + sidecar_refs.len() > MAX_CONVERSATION_FILES` and no
other reason is set, set `blocked_reason` to a fixed message ("session
carries N attachments; Mallard syncs at most M") the way the oversized
capture reason is set at `:1164-1166`. Blocked sessions never enter a
review, so Push and Pull agree.

### 4.3 Adapter: sidecar payload verification (`providers/traits.rs`, `providers/claude/`)

Add to `SessionAdapter`, beside `verify_index_fragment`:

```rust
/// Verify one staged sidecar payload of `sidecar_kind` before commit.
/// Providers without sidecars refuse every kind.
fn verify_sidecar(&self, _sidecar_kind: &str, _bytes: &[u8]) -> Result<(), String> {
    Err("provider has no sidecar files".to_string())
}
```

Claude implements it in `providers/claude/materialization.rs`:

- `todos`: the bytes parse as one JSON value (today's check, moved here);
- `file-history`: any bytes (the size bound is the core's);
- any other kind: error, matching `materialization_targets`.

`verify_provider_payload` (`conversation_materialization.rs:664-698`) routes
`ConversationFileRole::Sidecar { sidecar_kind }` to
`adapter.verify_sidecar(sidecar_kind, &bytes)`. The Codex and test
providers keep the default refusal, consistent with their
`materialization_targets` rejecting any sidecar request.

### 4.4 Prepare: fetch and verify sidecar bytes (`service.rs`)

Replace `cloud_sidecars: Vec<(String, String)>` on
`PreparedPullConversation` (`:9689-9691`) with:

```rust
struct PreparedPullSidecar {
    sidecar_kind: String,
    relative_path: String,
    logical_path: LogicalPath,
    body: VerifiedBody,
}
cloud_sidecars: Vec<PreparedPullSidecar>,
```

In `prepare_pull_conversation_actions`:

1. **Prefetch** (`:9820-9835`): extend the prefetched set with every
   `Sidecar` file of every approved thread, keeping the existing per-file
   `conversation_file_limit_bytes()` filter (that is the bound discovery
   applied at capture, `providers/claude/discovery.rs:169-172`). One
   batched transfer still covers the whole Pull; a transfer failure still
   ends the Apply (DESIGN_FAST_PUSH_PULL "Prefetch partial failure"). This
   is the Pull workload's "selected set" of DESIGN_PUSH_PULL_FLOW step 5a,
   not planning: `thread_review(PullPlan)` still opens transcripts only, so
   the byte-traffic gate
   `planning_reads_cloud_payloads_only_for_offerable_pull_threads`
   (`service.rs:23225-23315`) is unaffected.
2. **Per thread** (`prepare_one`, after the transcript body at `:9849`):
   for each sidecar ref from `conversation_sidecar_refs` — keep that helper,
   it decodes the namespace — call `bytes.body(&logical_path)`; reject a
   body over `conversation_file_limit_bytes()` with "cloud conversation
   sidecar exceeds its read limit"; collect a `PreparedPullSidecar`. Any
   error here is already a per-thread `PreparedPullConversationFailure`.
3. **Memory weight**: unchanged. Sidecar bytes are read in the serial
   consume stage (inside `apply_prepared_pull_conversation`), never in the
   parallel prepare workers, so the `for_each_prepared` size function at
   `:11185` keeps weighing the transcript alone and the 64 MiB preparation
   budget is untouched. A prepared sidecar holds a `VerifiedBody`, which is
   a temp file on disk, not heap.

`materialization_targets` still receives `&[(String, String)]`; build that
slice from `cloud_sidecars` at both call sites.

### 4.5 Apply: plan every target by role (`apply_prepared_pull_conversation`)

Replace the two-branch target resolution (`:10375-10412`) with one call to
`adapter.materialization_targets(...)` for both identities:

- `existing_transcript_path`: for the in-place update, the pinned
  transcript path (the one entry of `pins.target.provider_state` whose path
  equals `prepared.local_source_path`; the source check at `:10509`
  already requires this equality); `None` for minted identities;
- `provider_session_id`: `destination_provider_thread_id`;
- `source_provider_session_id`: `Some(cloud_provider_thread_id)`, as today.

Then per planned target:

- **Minted identity** (`CreateLocal`, `KeepBoth`): every target must be
  `Absent` on disk (today's loop); the payload expectation is `Absent`.
- **In place** (`MaterializeCloud`, non-graph providers): the payload
  expectation is the pinned `ProviderStateExpectation` for that exact path.
  A planned target with no pin, or a pin with no planned target, fails the
  thread with "conversation sidecar set changed after review". This closes
  the by-position transcript selection: the transcript is the planned
  target whose role is `Transcript`, and its pinned expectation is the one
  `read_pull_conversation_source` checks.

Build `targets` in commit order: sidecars first, then the transcript, then
the Codex index fragment. The engine commits in payload order
(`conversation_materialization.rs:325-327`) and has no journal, so a failure
mid-commit leaves inert extra attachments rather than a resumable
transcript without them. Every `Sha256`-expected sidecar gets its backup
from the existing staging loop; no new backup code.

Bytes: `body.read_bounded(conversation_file_limit_bytes(), "cloud conversation sidecar")`
into `PayloadBytes`. The read happens once, at apply, in the serial consume
stage of the thread's own job; the bytes live only as long as the engine
request.

### 4.6 Review pins: no change to the shape

The in-place pin list at `:5764-5790` already contains sidecar targets with
their current digests. Two consequences the implementation must keep:

- A local sidecar edited between Compare and Apply is not a conflict.
  Apply builds its own review (`PLAN_COMPARE_ONLY_APPLY_TIME_CAPTURE.md`),
  pins the bytes it finds, replaces them, and backs them up under
  `~/.mallard/backups/conversations/<operation>/`. The stale-pin check at
  `Target(ProviderState)` guards only the window between the apply-time
  plan and prepare inside one Apply. This is the same rule the transcript
  follows.
- An in-place update may also install a sidecar the local session never
  had (`Absent` pin → no-clobber create) and overwrite one it has (`Sha256`
  pin → backup, then rename). Local sidecars the cloud row does not list
  are not pinned and not touched.

### 4.7 Receipts and result

`ApplyReceipt.target_path` stays the transcript path;
`ConversationApplyReceipt.local_file_sha256_after` stays the transcript
digest. The operation's `targets` (`ConversationApplyOperation.targets`)
already list every installed path with its backup; nothing new is
persisted. The frontend outcome is per thread and needs no DTO change.

### 4.8 Resource budget: staged objects must not hold file descriptors

`VerifiedBody` wraps a `NamedTempFile` (`storage_core/object_store.rs:30-60`),
which keeps its temp file open. Every prefetched object therefore holds one
descriptor for the whole Pull (`EngineByteSource.loaded`) and, through the
process-wide `ObjectCache` (256 MiB, no entry-count cap, `bundle_engine.rs:125`),
possibly for the life of the process. The packaged app runs under launchd's
default soft limit (`launchctl limit maxfiles` reports 256 on this machine)
and nothing in the tree raises it (no `setrlimit`, no `rlimit` crate).
`npm run tauri dev` inherits the terminal's limit (1 048 576 here), so the
dev loop never shows the failure.

Today that limit is reachable only by pulling more than ~250 threads at
once. With sidecars, one session with a few hundred file-history entries
reaches it alone, and the failure is a whole-Pull storage error at
prefetch, not a per-thread receipt. Part of this change:

- `VerifiedBodyInner` holds a `tempfile::TempPath`
  (`NamedTempFile::into_temp_path`, tempfile 3.27) instead of the open
  file; `VerifiedBody::open` opens its own `File` per reader and
  `VerifiedBodyReader` reads through it. Zero descriptors at rest, one per
  active reader; the temp file still deletes on the last drop.
- `ObjectCache::insert` also caps the entry count (2 048), because with
  kilobyte-sized objects the byte cap no longer bounds the count and
  `get` walks the eviction order linearly.

## 5. What is out of scope

- **Sidecar-only changes are not a sync state.** Comparison is a tail
  relation over the transcript; a todos edit without a transcript change
  produces no `LocalAhead` and is pushed with the next transcript change.
  Named in `DESIGN_QODER_SUPPORT.md` §2; unchanged here.
- **Unchanged sidecars re-upload on every push of their thread.** The
  transcript has `PublishedTranscript::Retained`; sidecars are always
  `Captured` (`service.rs:8053-8061`). The object store is content-addressed
  and the manifest entry is rebuilt from the same digest, so this costs
  transfer, not correctness. A follow-up can retain by `(resource_id,
  sha256, size)` exactly as the transcript does.
- **`projects/<bucket>/<session>/tool-results/`** is not a Claude sidecar
  kind today and is not added by this plan.
- **Restored file-history snapshots are scrubbed.** `/rewind` on the
  destination restores the redacted bytes. That is the redaction plan's
  documented trade-off, not a Pull defect.
- **No UI.** No review row, badge, or count for attachments.

## 6. Cutover

Decisions confirmed 2026-09-05: clean cutover, no migration, no legacy
execution path, schema stays 1. There are no customers; `~/.mallard` and
the storage data are wiped for retest.

Nothing serialized changes. The bundle manifest (schema 1) already carries
sidecar file refs and roles; the machine-local documents (schema 1) gain no
field; `ConversationApplyOperation.targets` is in-memory only (receipts
persist without it); `PreparedPullSidecar` and the adapter method are
process-internal. `MAX_CONVERSATION_FILES` is a validation bound, not a
schema field. A bundle pushed before this change pulls its sidecars with
the new code, so the wipe is the standing retest recipe, not a requirement
of the change.

The `(String, String)` sidecar carrier is replaced, the literal 64s are
deleted, and the JSON check moves out of the core. Order of work, each step
green before the next:

1. `VerifiedBody` without a resident descriptor and the cache entry cap
   (§4.8). Storage tests; this step is independent of the rest and fixes a
   latent large-Pull failure on its own.
2. Domain bound (§4.1) and discovery block (§4.2). Rust tests.
3. Adapter `verify_sidecar` (§4.3); engine routes to it. Engine unit test
   with a non-JSON `file-history` payload and a non-JSON `todos` payload.
4. Prepare (§4.4): prefetch set, per-thread bodies, bound.
5. Apply (§4.5): role-based planning, ordered payloads.
6. Tests (§7), then the handoff set: `cargo test --manifest-path
   src-tauri/Cargo.toml --lib`, `cargo clippy` at the crate's usual
   settings, `npm run build` (no DTO change expected; the build proves it).
7. Retest: quit Mallard, remove `~/.mallard`, empty the storage folder or
   bucket, relaunch, bind a Claude home whose fixture carries both sidecar
   directories (this machine's own home has neither, §2.4), Push from one
   replica, Pull on another, and check `todos/` and `file-history/` under
   the destination home.

## 7. Tests

Extend `claude_round_trip_scrubs_secrets_end_to_end` (`service.rs:21444`),
which already seeds a `file-history/<session>/env@v1` and a
`todos/<session>-agent-main.json` on machine A:

- **CreateLocal on B**: after the pull, `claude-b/file-history/<minted>/env@v1`
  exists with `APP=demo` and `DB_PASSWORD=<password>` and no `SECRET_A_PASSWORD`;
  `claude-b/todos/<minted>-agent-main.json` exists, parses as JSON, and
  contains `rotate <anthropic-api-key>`; no file under `claude-b` carries
  the source session id in its name.
- **In-place MaterializeCloud on A** (the existing final hop): seed a local
  todos edit on A before B's push is pulled, assert the thread fails stale
  at `Target(ProviderState)`; restore the file, pull again, assert A's
  todos are replaced by the cloud bytes and the backup under
  `~/.mallard/backups/conversations/<operation>/` holds A's prior bytes.
- **KeepBoth**: a diverged Claude thread keeps the preserved copy's
  sidecars under the original id and materializes the cloud branch's
  sidecars under the minted id, all absent-checked.

New focused tests:

- `providers/claude/materialization.rs`: `verify_sidecar` accepts raw
  file-history bytes, rejects non-JSON todos, rejects an unknown kind.
- `conversation_materialization.rs`: a request with sidecar payloads
  commits sidecars before the transcript (assert order through a failing
  transcript expectation: the sidecars are on disk, the transcript is not).
- `provider_capture.rs`: a session with `MAX_CONVERSATION_FILES` sidecars
  is blocked with the count reason and offered in no review.
- Domain: a thread with more than `MAX_CONVERSATION_FILES` files fails
  `validate`; `ConversationTargetPins` and `ConversationApplyOperation`
  accept `MAX_CONVERSATION_FILES` and refuse one more.
- Per-thread isolation: a Pull of two Claude threads where one thread's
  sidecar object is missing from the store fails that thread's receipt and
  applies the other. (A missing object surfaces at prefetch as a whole-Pull
  storage error today; the test must remove the object after prefetch or
  drive `prepare_one` with a `BundleBytes` stub that fails one path, the
  `CountingBytes` pattern at `service.rs:22959-22990`.)

## 8. Open questions for the implementer

- Whether `verify_sidecar` should also cap `todos` at a small size (a todos
  list is kilobytes; a 100 MiB one is a shape error). Recommendation: no;
  one bound per conversation file is enough and the engine's
  `MAX_PROVIDER_TARGET_BYTES` is the backstop.
- Whether to refresh the Claude sidecar grammar against the current Claude
  Code layout (this machine writes `projects/<bucket>/<session>/tool-results/`
  and no top-level `todos` or `file-history`). Separate decision; this plan
  restores what discovery captures today.

## 9. Performance impact

Measured 2026-09-05 on this machine (Apple M3 Pro, APFS on the internal
SSD) with two standalone Rust programs that replay the engine's file
recipe and the store's staging recipe byte for byte, and one `curl` sample
against the Cloudflare edge. The programs live in the session scratchpad
(`scratchpad/bench/filebench.rs`, `scratchpad/bench/fsyncbench.rs`); each
is 60–100 iterations per row, `rustc -O`.

### 9.1 Primitives

| Primitive | Measured | Where it runs |
| --- | --- | --- |
| `File::sync_all` on macOS with dirty data (it is `F_FULLFSYNC`) | ≈ 3 ms | every `write_new_file`, every `sync_directory`, every store staging |
| Engine target, `Absent` (stage + 2 full fsyncs, 2 stage reads, hard link, directory fsync) | 8.9–10.0 ms, flat from 4 KiB to 1 MiB | `conversation_materialization.rs` per target |
| Engine target, `Sha256` (adds backup write + 2 fsyncs and 3 target reads) | 11.4–14.9 ms | in-place replacement only |
| Store staging of one object (temp write + `sync_all`) | 3.4–4.2 ms | `local_store.rs:199`, `s3_store.rs:640`, per object |
| SHA-256 | ≈ 2.5 GB/s; 64 KiB in 22 µs | every digest check |
| Cloudflare edge, cold connection, first byte | 73–101 ms (TCP 25–48, TLS 53–78) | R2 GET; warm keep-alive GETs are 30–50 ms (`PERF_SYNC_PATH_MEASUREMENTS.md:546`) |
| Directory fsync with nothing dirty | 0.05 ms | the audit's 47 µs figure (`BACKEND_AUDIT_2026-09-04.md:48`); not the case here, every sidecar dirties its directory |

The engine cost is fsync-bound, not byte-bound: a 1 MiB snapshot costs the
same as a 4 KiB todos list within noise.

### 9.2 Cost model per sidecar

| Storage | New target | Replaced target |
| --- | --- | --- |
| Local folder | 3.5 ms staging + 9.5 ms engine ≈ **13 ms** | ≈ **16 ms** |
| R2 | one GET at `cloud_transfer_concurrency` (default 6) + 9.5 ms engine | + 3 ms |

Transfer on R2 is latency-bound for these objects: ⌈K/6⌉ rounds of ≈ 80 ms.
The staging fsync overlaps the network inside the concurrent tasks.

### 9.3 Scenarios, one Claude thread, all sidecars new

Baseline today per materialized thread, engine side: transcript target
9.5–12 ms plus three `write_json_atomic` documents (bindings, config,
materializations) at 6.4 ms each ≈ **30 ms**, before transcript transfer
and decode.

| Sidecars K | Example | Local folder | R2 (6-way) |
| --- | --- | --- | --- |
| 6 | one todos list, five snapshots | +0.08 s | +0.14 s |
| 30 | a working session | +0.4 s | +0.7 s |
| 100 | a long session | +1.3 s | +2.3 s |
| 300 | a heavy edit session | +3.9 s | +6.9 s |
| 4 095 | the §4.1 ceiling | +53 s | +94 s |

A typical session stays well under a second. The ceiling is minutes, which
is why §4.1 blocks beyond it rather than allowing `MAX_FILES`.

### 9.4 What does not change

- Push: no new work; sidecars are already captured, scrubbed, certified,
  and uploaded. The discovery count check is O(1) per session.
- Refresh and comparison: no sidecar reads (tail relation over the
  transcript only).
- Pull review: in-place rows already read and hash every local sidecar
  (`bounded_file_expectation`), twice per Pull (review and prepare). At
  0.1 ms per small file that is 60 ms for K = 300; the only change is that
  the pin cap rises from 64 to 4 096.
- RAM: per thread at apply, transcript plus sidecars in memory for the
  engine request, bounded by the 512 MiB capture ceiling and the 100 MiB
  per-file limit that Push already enforces; same class as a transcript
  today. The parallel prepare budget (64 MiB) is untouched (§4.4).
- Disk: K temp files under `TMPDIR` for the Pull's duration, plus the
  object cache's existing 256 MiB.

### 9.5 Mitigations, measured, if the heavy end matters

None is required for the typical case; each is independent.

| Mitigation | Measured gain | Cost |
| --- | --- | --- |
| Batch durability per phase: plain `fsync` per staged file, one `F_FULLFSYNC` per directory after staging and one after commit | 7.6 ms → **0.64 ms** per target (12×); K = 300 falls from 3.9 s to ≈ 0.4 s locally | engine change; same durability at the phase boundary, weaker between files |
| Raise `cloud_transfer_concurrency` from 6 to 16 (the existing Settings knob; `DESIGN_FAST_PUSH_PULL.md` D6 intended 16) | R2 transfer 2.7×; K = 300 from 4 s to 1.5 s | none |
| Drop `sync_all` on store staging temp files (a cache; refetched on loss) | 3.4 ms per object locally; overlapped on R2 | two lines; the audit's read-path fsync family (`BACKEND_AUDIT_2026-09-04.md` A4) |

Recommendation: ship the plan as designed and measure a real heavy session
before taking any of these; the first row is the one to take if K in the
hundreds turns out to be common.

## 10. Implementation notes (2026-09-05, minimal scope)

Landed uncommitted on `extensible-providers`. Deltas from the design:

- **§4.8 not taken.** `VerifiedBody` still holds its temp file open and the
  object cache is still byte-capped only. The packaged app's ~250
  prefetched-object ceiling is unchanged; a session with a few hundred
  file-history entries would hit it. Left out as a separate change.
- **§2.3 item 3 kept.** The in-place branch still selects its reviewed
  transcript pin by position; the source-path equality check at apply
  fails closed if that ever picks a sidecar. Every target's expectation is
  now taken from the pins by path, so the sidecar side is role-based.
- **§4.6 corrected.** Under the compare-only cutover an edit between
  Compare and Apply is replaced with a backup, not refused; the doc now
  says so and the round-trip test pins it.
- **§9.5 mitigations not taken.**
- **Tests landed:** `claude_round_trip_scrubs_secrets_end_to_end` now
  asserts the CreateLocal restore on B (file-history raw bytes and todos
  JSON under the minted id, nothing under the source id) and the in-place
  replacement on A (cloud bytes installed, the post-Compare todos edit and
  the plaintext snapshot both in the backup), and
  `sidecar_payloads_are_verified_by_kind` covers the adapter verifier.
  Not added: KeepBoth, the discovery bound, the domain bound, the engine
  commit order, and the missing-object isolation case from §7.
- **Results:** `cargo test --manifest-path src-tauri/Cargo.toml --lib`
  273/273, `cargo test -p mallard-infrastructure` 442/442 (5 ignored),
  `cargo test -p mallard-domain` 80/80, `cargo check --workspace
  --all-targets` clean. Clippy findings are the pre-existing set; the
  `capture_request_for_binding` dead-code warning predates this change.

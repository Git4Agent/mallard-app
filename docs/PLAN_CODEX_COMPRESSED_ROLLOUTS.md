# Plan: read compressed Codex rollouts (`.jsonl.zst`)

- Status: implemented on 2026-10-14, uncommitted; `cargo test --workspace
  --all-targets` and the node aimock harness tests green that day; the
  owner's live pass on the six sessions is open. *What was built* at the
  end says where the code differs from this text. Replaces TODO items
  C152-008 and C152-009 in `TODO_CODEX_0_152.md`.
- Problem (owner's debug of 2026-10-14): six sessions of
  `mallard-with-pelican` show the red "!" on the Activity page and sit under
  "Only in Mallard Cloud". Each one's saved `.jsonl` path no longer exists;
  a `.jsonl.zst` twin does. Decoded, every twin matches its saved
  checkpoint: same native id and project folder, same decoded size, same
  probe-window hash. Mallard never looks at a `.zst` file, so the bound
  thread's local copy counts as unproven loss (`TailSideEvidence::Invalid`
  → `unknown_evidence` → red "!").
- Scope: the Rust backend only. No DTO, schema, manifest, or frontend
  change. Mallard reads compressed rollouts; it never writes one.
- Constraints: every existing bound stays (file count, entry count, byte
  limit, no-follow, regular-file, atomic reads, stamp verification). Clean
  cutover: one rule for file names, one read path, no fallbacks.
- Related: `TODO_CODEX_0_152.md` (matrix rows "Compressed rollout files" and
  "Shared-lineage compression"), `DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md`
  (byte offsets in `history_base` references).

## What Codex does

Read from `openai/codex` on 2026-10-14 (`codex-rs/rollout/src/compression.rs`,
`codex-rs/rollout/src/seekable_reader.rs`,
`codex-rs/thread-store/src/local/helpers.rs`):

- The experimental flag `[features] local_thread_store_compression = true`
  turns compression on. At Codex start-up a rollout that has been idle for
  a while (third-party reports say a week) is rewritten as
  `rollout-<stamp>-<uuid>.jsonl.zst`.
- The file is the same JSONL bytes in one zstd stream: one
  `zstd::stream::write::Encoder` at level 3, with
  `set_pledged_src_size(plain length)` so the frame header carries the
  decoded size. It is written to a temp file, verified by decoding, given
  the plain file's mtime and permissions, published with a no-clobber
  rename, and only then is the plain file removed. An interrupted run
  leaves both files, and the plain file wins.
- Codex never appends to a `.zst`. Before a write it materialises the
  rollout back to `.jsonl` (temp file, hard link or no-clobber persist),
  then removes the `.zst`.
- Everywhere Codex names a rollout it uses the plain name:
  `plain_rollout_path` strips `.zst`, `thread_id_from_rollout_path` strips
  `.zst` then `.jsonl`, `RolloutFile::from_path` returns nothing for a
  compressed file whose plain sibling exists (`should_skip_compressed_sibling`),
  and `RolloutReader::open` tries the plain path first.
- Byte offsets in paginated history references "always address the
  original JSONL bytes". Codex decodes a compressed rollout to a temp file
  and seeks there.

## Rules Mallard adopts

1. **A rollout's name is its plain name.** `X.jsonl.zst` is the rollout
   `X.jsonl`. Thread and physical ids come from the plain name. The portable
   logical path is already built from the session id
   (`state/providers/codex/sessions/<id>/transcript.jsonl` in
   [provider_capture.rs:1052](../crates/mallard-infrastructure/src/provider_capture.rs:1052)),
   so compression never changes a resource's identity, manifest, or cloud
   row.
2. **One file per name.** A `.jsonl.zst` whose plain sibling exists is not
   a rollout; the plain file is. Two files with the same plain name across
   `sessions/` and `archived_sessions/` stay the existing "duplicate
   filenames" refusal.
3. **Bytes are the decoded bytes.** Every offset, length, digest, probe
   window, `history_base.end_byte_offset`, `installed_end`, and
   `source_file_size` means the decoded JSONL, exactly what the saved state
   already means (the owner's check proved it).
4. **The stamp is the physical file.** `SourceStamp` keeps describing the
   file on disk (compressed length, mtime, identity). Reuse and change
   detection stay stat-based. A checkpoint saves no `source_state` for a
   compressed source: the domain requires a saved state's size to cover the
   cursor, and the compressed size cannot. A cursor without a state is never
   reused, so a compressed source is read in full on each observation; the
   process-local observation cache still holds it while the file is
   unchanged.
5. **Bounds hold on both sides.** The compressed file must fit the
   conversation limit (stat), and the decoded output is read through
   `take(limit + 1)` and refused past the limit. The frame header's content
   size is a hint used to refuse early at discovery; the decoder enforces
   the limit regardless of the header.
6. **Mallard writes plain files only.** A Codex Pull never writes in
   place: a newer cloud version lands in a new, complete rollout under a
   fresh provider id, and the previous local copy stays exactly as it is,
   a retained copy with its own checkpoint. A compressed local copy is
   retained the same way; `retained_root_is_byte_identical` decodes it when
   a later displacement asks whether it still holds its accepted bytes.

## Dependency

Add `zstd = "0.14"` to `crates/mallard-infrastructure/Cargo.toml`
(0.14.2 was published on 2026-10-10). It is the crate Codex writes with,
so frames, pledged sizes, and skippable frames decode exactly as written;
its `Decoder` implements `Read`, concatenates frames until EOF by default,
and takes a bounded reader. The bundled C library builds through `cc`,
which already builds `aws-lc-sys` on all three release targets. Tests
build fixtures with `zstd::encode_all(bytes, 3)` in temp homes, so no
binary fixture is checked in. The pure-Rust `ruzstd` was considered and
rejected: it decodes one frame per decoder and would need checked-in
`.zst` fixtures or the `zstd` crate as a dev-dependency anyway.

## Changes

### A. File names (`codex_rollout_selection.rs`)

- `plain_name(path) -> Option<&str>`: the file name with a final `.zst`
  removed, `Some` only when what remains ends in `.jsonl`. This is the one
  predicate every walker and lookup uses for "is this a rollout file".
- `is_compressed(path) -> bool`: final extension `zst`.
- `native_ids` parses the plain name
  ([codex_rollout_selection.rs:8](../crates/mallard-infrastructure/src/codex_rollout_selection.rs:7)).
- `select_current` orders and compares candidates by plain name
  ([codex_rollout_selection.rs:30](../crates/mallard-infrastructure/src/codex_rollout_selection.rs:31)).

### B. Walkers and lookups

Every place that lists rollouts accepts both names and drops a compressed
file whose plain sibling is in the same listing (a set of plain names built
from the listing, no extra stat):

- `RolloutLookup::paths`
  ([codex_subagents.rs:558](../crates/mallard-infrastructure/src/codex_subagents.rs:559));
  `find_rollout` and `find_current` match on the plain name
  ([codex_subagents.rs:582](../crates/mallard-infrastructure/src/codex_subagents.rs:589)).
- `discover`
  ([discovery.rs:146](../crates/mallard-infrastructure/src/providers/codex/discovery.rs:147)),
  `recent_session_homes`
  ([discovery.rs:87](../crates/mallard-infrastructure/src/providers/codex/discovery.rs:86)).
  `locate_session` already goes through `rollouts_by_thread`.
- `inspect_candidate_file_with_limit`
  ([provider_capture.rs:1849](../crates/mallard-infrastructure/src/provider_capture.rs:1849))
  keeps its physical size check and, for a compressed file, also reads the
  frame header (`zstd::zstd_safe::get_frame_content_size` on the first 18
  bytes): a declared decoded size above the limit blocks the session at
  discovery with the same oversize reason a plain file gets.

`verify_current` needs no change: after a mid-flight compression
`find_current` returns the `.zst` path, the captured `.jsonl` path no
longer matches, and the Push fails closed with "refresh and push again".
The stamp check fails the same way.

### C. One read path (`jsonl_tail_io.rs`)

The module already is the choke point for transcript bytes; it dispatches
on `is_compressed(path)` and stays provider-neutral.

- Private `decoded_snapshot(path) -> (Vec<u8>, SourceStamp)`: open
  no-follow, stamp, refuse a compressed length over the limit, read the
  compressed bytes, re-stamp (unchanged-while-read, as today), decode
  through `zstd::stream::read::Decoder::with_buffer` + `take(limit + 1)`,
  refuse output over the limit or a decode error, record metrics, verify
  the stamp.
- `read_snapshot` dispatches to it
  ([jsonl_tail_io.rs:295](../crates/mallard-infrastructure/src/jsonl_tail_io.rs:295)).
- `read_tail_window`, `read_suffix_window`, `read_range` for a compressed
  path take one decoded snapshot and slice it; `file_len` is the decoded
  length, `stamp` the physical one. No retry loop: the file is immutable
  while compressed and the snapshot already proves it did not change.
- New `open_head(path) -> (SourceStamp, Box<dyn BufRead>)` for bounded head
  reads: a `BufReader<File>` for a plain file, a
  `BufReader<Decoder<BufReader<File>>>` for a compressed one. zstd decodes
  block by block, so reading a 1 MiB head of a compressed file costs about
  a 1 MiB head. Callers:
  - `scan_session_metadata_with`
    ([provider_capture.rs:1677](../crates/mallard-infrastructure/src/provider_capture.rs:1677)),
    which also stops using `fs::File::open` (follows links today). Its
    `is_empty` test compares the first record with the physical length
    ([provider_capture.rs:1704](../crates/mallard-infrastructure/src/provider_capture.rs:1704));
    replace that with "no byte follows the first record"
    (`fill_buf().is_empty()`), which is right for both forms and costs
    nothing on a buffered reader.
  - `read_child_metadata`
    ([codex_subagents.rs:1190](../crates/mallard-infrastructure/src/codex_subagents.rs:1190)).
  - `codex_child_head`
    ([history.rs:2969](../crates/mallard-infrastructure/src/providers/codex/history.rs:2969)).
  - `summarize_rollout_file`
    ([history.rs:631](../crates/mallard-infrastructure/src/providers/codex/history.rs:631)),
    which also stops using `File::open`.

### D. Seeking readers

- `TimelineSources`
  ([timeline_sources.rs:112](../crates/mallard-infrastructure/src/providers/codex/timeline_sources.rs:112))
  holds `Vec<(File, u64)>` and seeks by decoded offset. Make the source an
  enum: `Plain(File)` or `Decoded(Cursor<Vec<u8>>)` from `read_snapshot`,
  with the "shorter than its cutoff" check on the decoded length. `read_value`
  seeks either. The cached index already stores the `.zst` relative path,
  and `source_path` accepts it.
- `retained_root_is_byte_identical`
  ([codex_history_graph.rs:116](../crates/mallard-infrastructure/src/codex_history_graph.rs:109))
  opens and reads the file by hand. Replace that with `read_snapshot`, then
  compare the decoded length and digest. Smaller and correct for both forms.
- `resolve_captured_codex_root` already reads bases through `read_snapshot`
  and checks `end_byte_offset` against the decoded length; a compressed
  base in a plain leaf's chain (or the reverse) resolves without further
  change.

### E. Pull from a compressed local copy

- Codex never materialises in place ("every Codex MaterializeCloud,
  CreateLocal, and KeepBoth consumes a reviewed absent target"), so the
  target planner needs no change: the new rollout gets a fresh plain path.
- `bounded_file_expectation`
  ([service.rs:4039](../src-tauri/src/project_sync/service.rs:4039)) pins a
  compressed source by its decoded bytes (read through `jsonl_tail_io`, so
  the bound applies). The review's expectation and the apply's source check
  both see decoded bytes, so a relocation from a compressed base matches.
- `read_pull_conversation_source` already decodes through `read_bounded`.
- The compressed copy is left in place as the retained copy of the
  relocation; nothing is written beside it.

### F. What heals on its own

For the six sessions: discovery lists the `.zst`, the saved
`root_mapping.root_path` (the old `.jsonl`) no longer equals the source, so
the review takes the full-resolve branch
([service.rs:4696](../src-tauri/src/project_sync/service.rs:4696)), reads the
decoded bytes, finds the accepted tail, and shows the row in sync. The next
acceptance re-pins the checkpoint to the `.zst` path and its physical stamp.
When Codex later materialises the file back to `.jsonl`, the same branch
runs once more. No migration, no state edit.

## Tests

Unit tests beside each helper, fixtures made with `zstd::encode_all` in
temp homes:

- `codex_rollout_selection`: `plain_name` and `native_ids` on `.jsonl.zst`;
  a plain and a compressed copy of one name across two roots is a duplicate.
- `RolloutLookup`: `find_current`, `find_rollout`, `rollouts_by_thread` find
  a compressed rollout; a compressed file with a plain sibling is hidden;
  `verify_current` fails after the captured file is compressed.
- `discovery`: a compressed session is listed with its `.zst` physical path
  and unchanged session id; the hidden sibling makes no duplicate;
  `locate_session` and `recent_session_homes` see it; a metadata-only
  compressed session is empty; a header declaring more than the limit is
  blocked at discovery with the oversize reason.
- `jsonl_tail_io`: a compressed snapshot equals the plain bytes and carries
  the physical stamp; output past the limit (a small file that decodes to
  more than the limit) is refused; a truncated or corrupt frame is refused;
  tail window, suffix window, and range slice decoded offsets with
  `file_len` equal to the decoded length.
- `scan_session_metadata_with`, `read_child_metadata`, `codex_child_head`,
  `summarize_rollout_file`: compressed heads parse; `is_empty` by trailing
  byte.
- `history`: `build_timeline_index` and `TimelineSources::read_value` on a
  compressed current source and on a mixed chain (compressed base, plain
  leaf, and the reverse).
- `codex_history_graph`: `retained_root_is_byte_identical` on a compressed
  retained root.
- `src-tauri/src/sync_tests/thread_variants/` (new `compression.rs` on the
  existing harness): push a plain rollout, compress it in place (encode,
  delete the plain file, keep the mtime), refresh: the row is in sync, no
  `unknown_evidence`, the Activity page lists it; Pull in place writes the
  plain sibling, leaves the `.zst`, and the next refresh reads the plain
  file; Push after a mid-flight compression fails closed with "refresh and
  push again"; the next acceptance re-pins `root_path` to the `.zst`.

Handoff set: `cargo test --manifest-path src-tauri/Cargo.toml --lib` and
`npm run test:backend-integration`. Then the owner's live pass on the real
home: the six `mallard-with-pelican` sessions lose the red "!" and leave
"Only in Mallard Cloud"; one Push/Pull review each way. The pinned
`codex-cli 0.162.1` has the flag, so a live end-to-end that backdates a
temp-home rollout by eight days, sets the flag, and starts Codex once is
possible but optional; the unit fixtures reproduce Codex's writer exactly.

## Docs

- `TODO_CODEX_0_152.md`: C152-008 and C152-009 done; matrix rows
  "Compressed rollout files" and "Shared-lineage compression" → Pass.
- `FEATURES.md`: one sentence that Codex sessions compressed by Codex
  (`.jsonl.zst`) are read like plain ones, and that Mallard writes only
  plain files.

## Out of scope

- Writing or deleting `.zst` files.
- Removing a hidden `.zst` after Pull (see rule 6).
- Claude: it has no compressed transcripts.
- `codex migrate-rollouts` detection (C152 "Legacy rollout migration").

## What was built

Implemented on 2026-10-14 as above, with these differences:

- Rule 4 as it now reads: `SourceStamp::saved_state(path)` returns no state
  for a compressed path, and the two checkpoint sites in `service.rs` use
  it. The first text claimed the saved size was never compared with a
  decoded offset; the domain's checkpoint validation does compare them.
- Test fixtures compress through `jsonl_tail_io::compress_like_codex`
  (test-support), which pledges the plain length into the frame header as
  Codex does; `zstd::encode_all` does not, and the declared-size hint
  needs it. No zstd dev-dependency on the Tauri crate.
- Section E as first written assumed an in-place Pull (a plain sibling
  written beside the `.zst`, a twin-aware target expectation). Codex Pulls
  relocate instead, so the target planner is untouched and the expectation
  only decodes a compressed pin. `verify_cutoff` in `codex_subagents.rs`
  (the installed cutoff of a pulled binding) also had to read through the
  shared range reader: it sought in the physical file.
- The end-to-end suite is one `TwoComputers` test in `service.rs`
  (`a_rollout_codex_compressed_in_place_stays_the_same_session`), not a
  new `thread_variants/compression.rs`: that harness drives the bundle
  engine, not a provider home. A Push after a mid-flight compression is
  covered at the lookup (`verify_current` fails closed on the captured
  plain path), not end to end.

## Follow-up: the session page

Built on 2026-10-14 as planned below (`addressed_len` in
`session_detail.rs`, one test), uncommitted. Found that day after the first
build, on a compressed session of
`mallard-with-pelican`: the session page shows "Timeline leaf exceeds the
current file." and no transcript.

### Cause

`ResolvedLocalSession.file_size` is the transcript's size on disk
([session_detail.rs:1388](../crates/mallard-infrastructure/src/session_detail.rs:1388)),
and it serves two different purposes:

1. identity: with the modification time it keys the timeline cache and the
   page's snapshot id, so a changed file invalidates open cursors;
2. the length the index addresses: `validate_index`
   ([session_detail.rs:708](../crates/mallard-infrastructure/src/session_detail.rs:708))
   refuses a Codex index whose leaf `length` exceeds it, and an index
   without sources whose event spans end past it.

The Codex index's leaf length and spans are decoded offsets (rule 3). For
a compressed rollout the size on disk is the stream's, smaller than the
decoded leaf, so the check fails on the cached index, the rebuilt index
fails it again, and the page errors. Both call sites pass the physical size
([session_detail.rs:1492](../crates/mallard-infrastructure/src/session_detail.rs:1492),
[session_detail.rs:1520](../crates/mallard-infrastructure/src/session_detail.rs:1520),
[session_detail.rs:2613](../crates/mallard-infrastructure/src/session_detail.rs:2613)).

### Fix

Keep `file_size` as the physical identity; it is right for purpose 1, and
a compression changes it, so the cache and cursors refresh as they should.
Give `validate_index` the addressed length instead:

- `addressed_len(session) -> SessionDetailResult<u64>` in
  `session_detail.rs`: the file size for a plain transcript; for a
  compressed one, the length of `jsonl_tail_io::read_bounded`, which is
  exact, bounded, and never trusts the frame header. One extra decode per
  request, for compressed sessions only; the page decodes once more to
  serve its records, and a cold session decodes in milliseconds.
- Computed once per attempt beside `expected_size` in the load-or-build
  loop and used for both `validate_index` calls there, and once in the
  stored and child path at line 2613.

No DTO, schema, or frontend change. The error text stays for the case it
was written for: a plain file shorter than its cached index.

### Tests

In `session_detail.rs`, beside the existing project-session tests, one
Codex case through `get_project_session_detail`: a compressed rollout
renders its timeline; then the plain-to-compressed transition, where the
index was cached from the plain file, Codex compresses it in place (same
bytes, same modification time, new name), and the next request renders
again without the "leaf exceeds" error. Handoff set: the infrastructure lib
tests, then the owner's live pass on the page in the screenshot.

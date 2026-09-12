# Plan: one derivation per byte in the Push builders

Status: proposed 2026-09-11, awaiting review. Fixes finding 1 of
`docs/SYNC_PATH_AUDIT_2026-09-11.md` (the Push builder proves its own output
by re-parsing it; 66% of a local Push). Scope: the Claude and Codex Push
builders and the review-to-builder handoff. The Pull decoder is not in scope
except where one shared function loses its byte-based twin (noted below).

## Decision

Every check that proves a property of a value the builder already holds in
memory is computed on that value. Every check that witnesses something
external stays exactly as it is: capture digests, source stamp freshness,
observed tail equality with the review pins, the changed-record redaction
gate, the Codex projection scan for entity-hidden secrets, the head CAS.

Nothing persisted changes. Wire bytes are unchanged for every record the
mapper and the redactor leave alone, and for changed records the bytes are
the serde serialization of the redacted value, which is what the raw-line
splice produces today (the existing publication test asserts this
equivalence on its fixture and stays as the proof). Schema stays 1. No
migration, no flag, no legacy path; the byte-based verifiers are deleted.

## Constraints from the review

- Clean cutover; no compatibility path. The user wipes `~/.mallard` and the
  cloud data before retesting; there are no customers.
- The Codex wire-projection detector pass stays. It is the only guard for a
  secret hidden behind HTML entities in the source
  (`codex_standalone_builder.rs:377`). The earlier audit bullet that
  proposed dropping it was retracted in review.

## Changes

### 1. Path map: prove restoration only where a path was mapped

`session_path_map.rs::encode_record_with_origins` runs
`decode(wire) == record` only when `origins.fields` is non-empty.

Why it loses nothing: the encoder escapes literal control-namespace text
with patterns marked `literal: true`, so any record containing
`/__mallard_root__` or `/__mallard_escape__` is a changed record with fields.
An unchanged record therefore contains no namespace text, and `decode_text`
rewrites nothing outside those two namespaces (`session_path_map.rs:254`).
On an unchanged record the gate is the identity by construction.

Test: add a unit test that a record carrying literal namespace text is
reported changed and round-trips; keep
`stored_origins_still_check_hashes_and_exact_restoration` unchanged (it uses
a path field, so the gate still runs).

### 2. Redaction: report whether a record changed

`RedactionRun::redact_record_gated` and `RedactionRuleset::redact_record_gated`
return `(Value, bool)`; the bool is `redact_value`'s own changed flag. The
changed-record gate (span diff + fixpoint re-scan) is unchanged. All three
production callers switch (`codex_standalone_builder.rs:678`,
`providers/claude/subagents.rs:547`, `pull_transcript.rs:433`); the pull
caller ignores the flag.

After change 3 no production code calls `redact_jsonl_text`; the two public
wrappers are deleted and their tests retargeted to `redact_document`, which
keeps the raw-line splice for sidecars and provider state at capture.

### 3. Claude main transcript: one parse, value-mode redaction, streamed analysis

`providers/claude/publication.rs::encode_main_transcript` becomes a single
loop over source lines:

1. parse the line, `record_link`;
2. `encode_record_with_origins` (gate per change 1);
3. `redact_record_gated` on the encoded value; when it changed, run
   `mapper.verify_wire_record` on the redacted value (a splice across a path
   token is the one thing redaction could break, and only changed records
   can have it);
4. serialize the value into the wire text with the same newline layout as
   today; feed the watermark with the record's end offset;
5. yield the value into `analyze_claude_records`, which drains the loop as
   an iterator (`std::iter::from_fn`), so no record is retained.

`verify_main_wire` is deleted. The analyzer's `bytes` argument receives the
source bytes (it uses them only for the analysis-bytes metric and the
all-whitespace check, which are equivalent for source and wire). The output
is hashed once into `PayloadBytes` and wrapped in `RedactedWireBytes` as
today.

What is removed: parse 2 (redact_jsonl_text), parse 3 (verify_main_wire),
the second detector run on unchanged records, and the second decode walk on
every record.

Tests: `combined_checks_preserve_wire_bytes_analysis_and_pending_watermark`
keeps its byte-equality assertion against the raw-line splice as the proof
that changed lines serialize identically, plus analysis and seed equality.
`final_checks_include_records_omitted_from_the_chain` is rewritten from a
wire-bytes test into a source test: a thinking-only record (not in the
chain) carrying a secret and a path is redacted and path-encoded in the
output; a malformed source line fails.

### 4. Claude adoption probe uses the publisher

`service.rs` `observe_claude_local` (around line 4590) encodes the local
slice by hand, redacts the text, and re-parses it to compare folds. It
calls `encode_main_transcript` instead and reads `.analysis`. One code path
produces the wire form the probe compares against, and the re-parse goes.

### 5. Claude children: same treatment

`providers/claude/subagents.rs::capture_children` already redacts per value
but then re-parses its own wire to run `verify_wire_record` on every record
and analyze it (`:585-600`). The encode closure keeps the redacted values
(children are small) and the wire analysis runs over them;
`verify_wire_record` runs only for changed records.

### 6. Codex builder: verify on values, analyze before serializing

`codex_standalone_builder.rs::reconstruct_codex_root`:

- `SourceRecord` gains `changed: bool` (path fields non-empty or redaction
  changed). In encode mode `mapper.verify_wire_record` (`:401`) runs only
  for changed records. Identity rewrites insert `mallard-main`,
  `mallard-child:<key>` and Codex thread ids, none of which can contain
  namespace text.
- Output records are retained as values after the ordinal is inserted, and
  serialized exactly as `append_output_record` does today, so the bytes are
  unchanged (`fixtures/standalone_codex_wire.jsonl` stays byte-exact).
- `analyze_standalone_output(&bytes)` becomes
  `analyze_standalone_records(&bytes, records)`: the same layout checks
  (ordinal sequence, one session_meta at ordinal 0) and
  `analyze_codex_records` over the retained values. Both modes use it; the
  byte-parsing twin is deleted. This also removes the "verify output
  transcript" re-parse on the Codex Pull side, which is the same function.
  `validate_projection_fixpoint` (decode only) is untouched.
- Kept: head `verify_wire_value`, the native projection, the wire
  projection with its detector callback, `seed_codex_root` and the tail
  equality against the pins in `service.rs`.

Memory: the retained output values are the same values moved out of
`original`; no second copy.

### 7. Codex: the review's resolved root feeds the builder

Today the apply-time review resolves the graph on first contact and the
builder resolves it again from the same bytes (`service.rs:8502`).

- `PushObservationEvidence` gains `resolved: Option<ResolvedCodexRoot>`,
  set on `observe_codex_local`'s full-resolve path. The struct is already
  `#[serde(skip)]`, so it never enters the observation disk cache; it lives
  in the in-memory observation cache like the rest of the evidence.
- `ThreadReview` gains `resolved_roots: BTreeMap<ThreadId, ReviewedCodexRoot>`
  (`resolved` plus the evidence's per-file stamps), filled in
  `provider_thread_review` for PushPlan when a push action is built, from
  the observation that produced the action, whether it came from the
  in-memory cache or a fresh derivation.
- `push_bundle_reviewed_with_progress` passes the map through
  `publish_reviewed` and `build_publish_request` to each job;
  `build_conversation_publication` takes `Option<ReviewedCodexRoot>`.
- Builder rule: reuse only when the review's leaf stamp equals the captured
  transcript's stamp. Then `codex_history_graph::reload_resolved_sources`
  verifies the leaf stamp, re-reads each ancestor segment and requires its
  stamp to equal the review's recorded stamp, enforces the consumed length
  and the aggregate capture cap, and returns the source snapshots. Any
  mismatch falls back to `resolve_captured_codex_root` from the captured
  bytes, which is today's path; the builder's final stamp verification over
  every source it used is unchanged.

What this saves: one full parse of the leaf and every ancestor per Codex
thread on first-contact pushes (the dialog compare resolves, the push
reuses). Steady-state pushes ride the checkpoint cursor at review and the
builder's resolve remains the only one, as today.

Memory bound: a resolved root holds the canonical chain (message id,
parent id, one 64-hex fingerprint per message) plus segment metadata and
leaf creations; roughly 0.2 KB per message. Retained only for threads whose
last observation in this process took the full path, inside the existing
4,096-entry cache with per-profile eviction; tens of MB at worst on this
machine's corpus. If that is not acceptable, the fallback is to set
`resolved` only in PushPlan reviews, which keeps the memory near zero and
loses the dialog-to-push reuse.

## Unchanged on purpose

- Sidecar `certify_wire_document` at publish (`service.rs:8880`, `:9242`):
  small inputs, and it is the `RedactedWireBytes` boundary for files no
  builder produced.
- The Codex `seed_codex_root` in the builder and the equality with
  `pins.tail.local`.
- Every `stamp.verify`, the capture digests, the head CAS, the publish
  scope pins.
- The Pull side beyond change 6 (finding 2 of the audit).

## Tests that will move

- `session_path_map.rs`: one new unit test (change 1).
- `credential_redaction.rs`: `redact_record_gated` callers in tests;
  `redact_jsonl_text` tests retargeted to `redact_document`.
- `providers/claude/publication.rs`: the two tests above.
- `codex_standalone_builder.rs`:
  `combined_output_check_rejects_invalid_layout_and_malformed_suffix`
  retargeted to parsed values; the byte-exact fixture test stays.
- `service.rs` metric assertions on `full_graph_resolves` and
  `records_parsed` around pushes (`push_observation_reuse_*`,
  `a_comparison_reads_no_*`, lines 21088-21162, 21626-21721, 23304-23436):
  expected counts drop by the removed parses; one new test that a push whose
  review resolved the root does not resolve again and still rejects a
  changed ancestor.

## Verification bar

- `cargo test -p mallard-infrastructure`, `cargo test -p mallard-domain`,
  `cargo test --manifest-path src-tauri/Cargo.toml --lib`, rustfmt on
  touched files, clippy clean on touched files.
- Release probe (the audit's scratchpad crate) timing
  `encode_main_transcript` on the 45 / 57 / 138 MB Claude transcripts and
  `encode_standalone_codex_root` on a real rollout, before and after.
- Then the user's live retest after wiping `~/.mallard` and the cloud data;
  the push log's `build publish request` phase is the number to compare
  against 3.44 s.

## Files

`crates/mallard-infrastructure/src/session_path_map.rs`,
`credential_redaction.rs`, `providers/claude/publication.rs`,
`providers/claude/subagents.rs`, `codex_standalone_builder.rs`,
`codex_history_graph.rs`, `pull_transcript.rs` (signature only),
`src-tauri/src/project_sync/service.rs`. Peer session mallard-app-34 is
editing `commands.rs` and the R2 setup files, which do not overlap; the
other peers will be messaged before `service.rs` is edited.

# Bounded Claude metadata scan: tail-window decide under authorized-home ownership

- **Status:** IMPLEMENTED 2026-09-01 (uncommitted); corpus gate passed, constant
  frozen — see "Measured results" at the end
- **Baseline:** `f2cdac4` plus the current working tree (which already carries
  the PERF-031 canonicalization memo)
- **Motivating finding:** PERF-025 in
  [`PERFORMANCE_AUDIT_2026-09-01.md`](PERFORMANCE_AUDIT_2026-09-01.md) —
  Claude discovery reads 1.09 GB and parses 237,661 records per pass on the
  audited machine to keep a handful of sessions
- **Compatibility:** clean cutover. No migration, no legacy execution path, no
  flag. No schema, IPC, or persisted-state change (this is a pure read path;
  wiping `~/.mallard` is unnecessary for this change and harmless). Schema
  stays 1.

## 1. The ownership model (normative)

Mallard separates two facts a Claude transcript records:

```
authorized_home = /repo/project       immutable session ownership
current_cwd     = /repo/project/src   mutable execution context
```

Only `authorized_home` decides which project owns a session. It is established
when the session is authorized — the earliest recorded home fact of the leaf
session — and later `cwd` values are per-turn shell state: activity metadata
that must never move the session. A session whose `authorized_home` cannot be
proven is owned by nobody (skipped with a discovery warning), never owned by
drift.

Concretely, in a Claude transcript: chain records (`type` ∈ {user, assistant,
system, attachment}, not sidechain, `uuid` key present — null counts,
`isMeta` not consulted) partition into segments wherever consecutive chain
records' effective session ids differ; the **leaf segment** is the final one,
its id is the session's identity (the filename must match it), and
`authorized_home` is the **first non-empty `cwd` among the leaf segment's
chain records**. Resume/fork files copy history under the original session
ids, so the leaf segment begins at the file's last id transition. This is the
rule the tracker's `leaf_session_home` machinery already implements; this
design makes it the *only* rule.

Codex is already compliant by construction: its bounded head read takes the
`cwd` from the first metadata record — the session's start.

### What the model deletes (doctrine cutover)

`ClaudeSessionIdentityTracker::leaf_identity()`
(`providers/claude/codec.rs:225`) currently carries two escapes that let
`current_cwd` leak into ownership. Both are removed in this change:

1. **The drift fallback.** When `leaf_home_session_id` does not match the
   resolved session id, `leaf_identity()` falls back to
   `leaf_cwd.or(last_cwd)` (`codec.rs:242`) — the last chain record's cwd,
   else the last cwd seen anywhere. That is execution context deciding
   ownership. New rule: home = the leaf segment's `authorized_home`, or
   `None`, which the scan surfaces as the existing "home was not found" error
   → discovery warning + skip. The `leaf_cwd` field, whose only reader is
   this branch, is deleted.
2. **Last-write metadata identity.** For degenerate files with no chain
   records at all, `leaf_identity()` returns `last_metadata_identity` — the
   *last* durable-typed record carrying both `sessionId` and `cwd`
   (`codec.rs:202`). Under the model the authorization fact is the earliest
   one: the field becomes `first_metadata_identity`, set once.

`last_session_id`/`last_cwd` and `observed_identity()` stay: current_cwd
remains available as activity/diagnostic metadata (the codec's `unknown()`
paths, activity surfaces). It is barred from ownership, not erased.

Note the tracker is shared: `analyze_claude_transcript`'s success path also
resolves identity through `leaf_identity()` (`codec.rs:370`), so the doctrine
automatically applies to capture's `source_cwd` — which anchors the in-root
content remap. That is intended: the remap anchor must be the authorized
home, not drift. Implementation checkpoint: verify the analyze path degrades
a `None` home into its bounded `Unknown` reason (StoredOnly capture), not a
panic or a mislabel.

### Naming follows the model

`SessionFileMetadata.cwd` (`providers/traits.rs:76`) is renamed
`authorized_home`, along with `validated_session_metadata`'s parameter
(`provider_capture.rs:1718`), Codex's `ParsedSessionMetadata.cwd`, and every
`meta.cwd` consumer (claude/codex/test_provider discovery,
`session_detail.rs:825`, `service.rs:15021`, the `project_activity.rs:1941`
test mock). Validation rules are unchanged. Tracker fields:
`leaf_session_home` → `authorized_home`, `leaf_home_session_id` →
`authorized_home_session_id`. The scan's cwd error message becomes
"authorized Claude session home was not found" (test substring at
`claude/mod.rs:272` updated). Mechanical, grep-driven, same change.

## 2. Why full reads happen today, and what a bounded read must prove

`scan_claude_session_metadata` (`claude/mod.rs:35`) reads every line and
parses every record into `serde_json::Value` because the leaf segment's start
is not at either end of the file by construction:

- A **head read is unsound**: the leaf identity comes from the tail (resumed
  files put copied foreign-id history first).
- A **tail read of a single-id window is unsound**: the leaf segment — and
  its first cwd, the `authorized_home` — may begin before the window.
- **Head+tail is unsound**: the unread middle can hide an `L…M…L` id
  reappearance; nothing per-file guarantees id monotonicity, and ownership
  must not rest on an unverifiable corpus property.

So the bounded read must *prove*, from the bytes it read, that it saw the
file's last id transition. When it cannot, it falls back to a full scan.

## 3. The decide rule

Read a suffix window of `SCAN_TAIL_WINDOW_BYTES` (only when `file_len >`
window; smaller files take the full-stream path directly, which is the same
computation). Align to complete records: drop through the first `\n` (the
window starts mid-record; if it happens to start on a boundary, one complete
record is harmlessly treated as pre-window), and drop the unterminated tail
after the last `\n` — the existing "not a durable record yet" rule. Run the
tracker forward over the window's records. The window **decides** iff:

1. every chain record in the window carries its own non-null `sessionId`, and
2. at least one chain record's own id differs from the last chain record's
   own id `L` (equivalently: an adjacent chain-id difference was observed).

**Soundness.** (2) places the file's last id transition inside the window:
the leaf segment is the maximal all-`L` suffix of chain records, and the last
in-window chain record with id ≠ `L` bounds it, no matter what the unread
prefix contains — an id reappearance before the window is irrelevant because
anything pre-window precedes that in-window non-`L` record. (1) makes every
in-window effective id the record's own id, so segment boundaries and the id
fallback never consult pre-window `last_session_id` state. Together: the leaf
segment lies entirely in-window, `leaf_session_id = L`,
`authorized_home_session_id = L`, and `authorized_home` is computed from
exactly the same records the whole-file tracker would use. Window-local
artifacts (the window's first chain record spuriously "opening" a segment)
are overwritten at the transition. With the drift fallback deleted, the
equivalence needs no argument about fallback reachability: `leaf_identity()`
reads only leaf-segment state.

A decided window whose leaf segment has no non-empty cwd yields home `None` —
the same "home was not found" error a full scan of that file now produces.
Decided-as-error is still decided; no fallback read.

**Fail-closed.** Anything else — no chain record in the window, any chain
record without its own `sessionId`, a single id throughout the window — falls
back to the full stream scan. Never-resumed single-id files therefore
legitimately pay a full read: their `authorized_home` was established at the
file's head and the middle cannot be vouched for. They pay it at
narrow-struct speed (§4, measured 2.4×). Resumed and forked files — every
`--resume`/`--continue` writes copied history plus an id transition near the
tail, and they are the large files — decide from one window.

Files with no chain records at all always take the full scan (the window
cannot know the whole file lacks them), where `first_metadata_identity`
applies.

### Forks and resumes

Fork behavior is unchanged and forks are the design's fast path, not a
casualty:

- A fork/`--resume` file — copied history under the parents' original ids,
  new records under the fork's id — is the decidable shape: the id
  transition sits near the tail, the leaf segment is the fork's own records,
  and `authorized_home` is where the fork was launched (first cwd of the
  fork's records), exactly as the full scan resolves it today. Nested forks
  resolve at the last transition. A fork whose own tail has outgrown the
  window falls back to the full scan with identical semantics.
- In-file rewind/branching keeps one `sessionId`, so those files take the
  fallback path; branch tolerance lives in the `claude.lineage.v1` chain
  walk, which this design does not touch. Fork lineage, divergence
  detection, and the tail cursor all consume the codec's full parses, not
  this scan.
- A session resumed into a different directory stays owned by where the
  fork was launched — the exact-home rule and discovery's root-bucket
  self-consistency check are untouched. Real fork files never hit the
  deleted drift fallback (Claude Code stamps `sessionId` on every chain
  record it writes); any corpus file that did depend on it surfaces as a
  classified doctrine divergence in the parity harness.
- Materialized pull files decide either way: single-id throughout → full
  scan; preserved historical ids → window; both yield the same identity,
  and the remapped structural cwd homes them at the project root as before.

## 4. Changes

### `jsonl_tail_io.rs` — one new primitive

`read_suffix_window(path, max_window_bytes) -> Result<TailWindow, String>`:
the `read_tail_window` contract addressed by size instead of a cursor
mapping — `open_no_follow`, regular-file and `conversation_file_limit_bytes`
checks, stat → seek(len − window) → read-to-EOF → stat, retrying
`TAIL_WINDOW_READ_RETRIES` times while the EOF moves, recomputing the offset
per attempt. On the successful attempt only, attribute the bytes with
`record_transcript_bytes_read` (probe/append attribution is cursor
semantics; this is not a cursor read).

### `providers/claude/codec.rs` — narrow record, shared extraction, doctrine cutover

- **`ClaudeScanRecord`**, the narrow `#[derive(Deserialize)]` mirror of what
  identity needs: `sessionId`, `cwd`, `type`, `isSidechain`, `isMeta`,
  `uuid` **presence** (a custom `deserialize_with` — `Option<T>` folds
  present-null into absent, but `is_chain_record` uses `contains_key`
  semantics where null counts), and `message.content` **block types**
  (required by the thinking-only skip; the audit's field list missed it —
  without it a thinking-only trailing assistant row corrupts leaf identity).
  `parentUuid` is not needed: only `classify_claude_record` reads it, not
  `observe`. Content must be parsed with a hand-written visitor — **not**
  `#[serde(untagged)]`, which buffers the content into an owned tree and
  forfeits the win. String content → not blocks; non-map array elements →
  a `None` block type (→ non-thinking). serde skips all unknown fields
  without allocating.
- **`IdentityParts { session_id, cwd, is_chain, is_durable_metadata }`** with
  two constructors — `from_value(&Value)` and
  `ClaudeScanRecord::identity_parts()` — both returning `None` for
  thinking-only rows (and the Value side for non-objects). Both encode the
  non-empty-cwd filter, the durable-metadata rule (`type` ∈
  {user, assistant, system} ∧ ¬sidechain ∧ ¬meta), and the chain rule (§1).
  These constructors are the only duplicated surface and are differentially
  tested.
- **Tracker**: all logic moves into a private
  `observe_parts(&mut self, IdentityParts)`; `observe(&Value)` becomes a
  thin wrapper so `analyze_claude_transcript` and `history.rs` are untouched
  beyond behavior they inherit. Doctrine cutover from §1: delete `leaf_cwd`
  and the drift fallback branch, flip `last_metadata_identity` →
  `first_metadata_identity`, rename the home fields, and rewrite the struct
  doc comment around the two-fact model and the decide rule.

### `providers/claude/mod.rs` — the scan itself

Rewrite `scan_claude_session_metadata` as a wrapper over an internal
`scan_with_window(path, window_bytes)` (tests drive tiny windows;
production passes `SCAN_TAIL_WINDOW_BYTES`):

1. Open/stat/limit checks and `record_file_head_read` unchanged.
2. `file_len ≤ window` → full stream scan.
3. Otherwise `read_suffix_window`, align, and per complete line: parse
   `ClaudeScanRecord` (parse failure skips the line, as today), take
   `identity_parts`, feed the tracker and the sufficiency accumulator
   (`all_chain_have_own_id`, `prev_chain_own_id`, `transition_seen`).
4. Decided → `leaf_identity()` → filename-matches-id check →
   `validated_session_metadata`, all unchanged. Not decided → full stream
   scan.
5. **Full stream scan** = today's `BufReader`/`read_until` loop verbatim —
   streaming deliberately kept so parallel refresh workers never hold whole
   transcripts in memory — with `ClaudeScanRecord` replacing `Value` (the
   audit's measured 2.4×), preserving the blank-line skip, the torn-tail
   break, the byte re-bound, and `MAX_CLAUDE_RECORDS`.
6. Both paths call `record_records_parsed` so `MALLARD_SYNC_METRICS=1`
   quantifies the fix (the scan currently never increments it).
7. The `serde_json::Value` loop is deleted. One production path.

`SCAN_TAIL_WINDOW_BYTES = 256 * 1024` is the opening proposal; the corpus
harness freezes the value and decides whether a second widen tier (e.g.
4 MiB before full) earns its place. Default recommendation: single tier —
simplest thing the data supports.

### `examples/claude_scan_corpus_parity.rs` — the freeze gate (dev-only)

Walks the real `~/.claude/projects` with the discovery shape (depth 2–3,
`.jsonl`, `provider_walk_entry`), runs the new scan against an **embedded**
~30-line reference implementation of the old `Value` loop *with the old
fallback semantics* (the reference lives only in this example and in
`#[cfg(test)]`; production keeps one path), and classifies every file:

- **identical** — expected for approximately the whole corpus;
- **doctrine divergence** — the old scan resolved a home through the deleted
  drift fallback or last-metadata rule and the new scan errors or differs;
  listed per file for manual review, expected ≈ 0;
- **unexplained divergence** — merge blocker.

It also reports, for window sizes {64 KiB, 256 KiB, 1 MiB, 4 MiB} and a
two-tier variant: decided fraction, bytes read, projected wall, and the
leaf-segment-size distribution. Gate: 113/113 files identical or individually
explained as doctrine divergences, before the constant freezes.

## 5. Semantic deltas (exhaustive)

1. **Drift never owns.** Sessions whose leaf segment yields no
   `authorized_home` are skipped with a warning instead of being homed by
   the last observed cwd. Chain records lacking `sessionId` can no longer
   route ownership through file-wide last-seen state.
2. **No-chain files** resolve identity from their *first* durable metadata
   record, not their last.
3. **Record-count acceptance.** A file with more than `MAX_CLAUDE_RECORDS`
   (100,000) records whose tail window decides is now accepted by discovery,
   where today the scan errors and discovery skips it. The cap is a work
   bound and the window bounds work structurally; downstream stays bounded
   (`analyze` returns its bounded `Unknown` reason → StoredOnly;
   `claude_history_tail.rs:416` enforces the ordinal cap). The full-scan
   path keeps the cap.
4. **Error wording** for the missing-home case changes to name the
   authorized home.

Oversized-bytes rejection (pre-read stat check), filename/id equality,
`validated_session_metadata` rules, and every caller contract
(`discovery.rs:89`, `session_detail.rs:825`, `service.rs:15021`) are
unchanged.

## 6. Tests

Full bar green as usual, `clippy -D` included.

- `jsonl_tail_io`: suffix window vs tiny/huge windows, moving-EOF retry,
  truncation race, non-regular file, over-limit file.
- Codec differential table over nasty lines — thinking-only assistant,
  sidechain, `isMeta`, `uuid: null`, absent uuid, content-as-string,
  non-object content elements, nested `sessionId` in message payloads,
  non-object lines — asserting `from_value` ≡ `identity_parts`.
- Doctrine pins: drift cwd never returned (segment-first cwd wins after a
  transition); chain-without-id files error on home instead of falling back;
  no-chain files use the first metadata identity.
- Scan parity on synthetic transcripts through `scan_with_window(tiny)`:
  resume chain S1→S2→S3; transition exactly at the window edge; leaf segment
  larger than the window (→ fallback); single-id file (→ fallback); one
  record larger than the window (aligned window empty → fallback);
  thinking-only last row; torn tail; the record-count delta. The three
  existing `claude/mod.rs` tests stay, with only the error-substring update.
- Path-taken assertions via `sync_metrics` deltas (bytes/records), not
  `cfg` hooks.
- Live confirmation with `MALLARD_SYNC_METRICS=1`, the audit's instrument.

## 7. Out of scope

PERF-026/PERF-027 (per-home inventory cache, single-flight walk) —
independent and composing; `analyze`/`history` full parses (they need full
values for fingerprints and the timeline); the Codex adapter beyond the
rename; any settings knob; any schema change.

## 8. Expectations, honestly

Window-decided files drop from full size to ~one window; fallback files run
at the narrow-struct 2.4× (0.54 s corpus-wide worst case against today's
1.28 s warm). The audit's 7.8 ms bounded-head figure is not the target —
correctness forbids head reads for single-id files. The real number is the
corpus mix; the harness prints it before anything freezes.

## 9. Measured results (2026-09-01 corpus gate, constant freeze)

`claude_scan_corpus_parity_and_window_sweep` (ignored test in
`providers/claude/mod.rs`, run `--release` against this machine's real
`~/.claude/projects`, 114 files / 1.10 GB at the walker's depth):

| Check | Result |
| --- | --- |
| Both-Ok identity parity | 106/106 identical |
| Both-rejected (same class old and new) | 8 |
| Doctrine divergences (old fallback carried a home) | **0** |
| Acceptance divergences (record-cap etc.) | **0** |
| Reference `Value` scan, full pass | 1.778 s |

| Window | Large files decided | Bytes read | Wall |
| --- | ---: | ---: | ---: |
| 64 KiB | 4/112 | 1094 MB | 0.571 s |
| 256 KiB | 8/111 | 1104 MB | 0.564 s |
| 1 MiB | 14/99 | 1152 MB | 0.584 s |
| 4 MiB | 18/63 | 1118 MB | 0.618 s |

**Frozen: `SCAN_TAIL_WINDOW_BYTES = 256 KiB`, single tier, no widen.**

Read honestly: this corpus is dominated by single-id sessions, so the decide
rate is low (7% of large files) and the wall win — 1.778 s → 0.564 s, 3.2× —
comes almost entirely from the narrow-record parse. The window tier is kept
because it is free at this size (wall-optimal in the sweep, ~1% byte
overhead), it is the piece that bounds the worst case a fallback cannot — a
huge resumed transcript with a fresh leaf segment reads one window instead of
its full size — and freshly-resumed files are exactly what a
new-session-triggered inventory rebuild rescans. A widen tier is refused by
the data: 4 MiB decides 18/63 while adding bytes and wall over 256 KiB.

The doctrine cutover measurably changed nothing on this corpus: every file
the old drift fallback could have homed differently either agrees or was
already rejected by both implementations.

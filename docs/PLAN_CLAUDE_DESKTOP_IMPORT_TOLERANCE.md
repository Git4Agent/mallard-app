# Plan: Claude Desktop Import Tolerance (projection semantics)

Status: **Implemented** — 2026-08-20 (v2 proposed and implemented same day; see
Implementation notes at the end for deltas)
Depends on: `docs/PLAN_CLAUDE_SESSION_CURSOR_PARITY.md` (implemented 2026-08-20)

v1 of this plan (severed-root tolerance) made Desktop imports *survivable*:
one honest Diverged, one push, sync continues. The user then set a stronger
requirement: **thinking contents must be invisible to sync comparison
entirely** — Mallard must not compute or compare thinking fingerprints, a
stripped and an unstripped transcript are the same conversation, and captures
must stay verbatim (no stripping anywhere in storage). v2 delivers that:
imports become a **no-op for sync** (Settled stays Settled), proven equivalent
on 106/106 corpus files. v1's severed-root arms survive as the backstop for
non-sanitizer damage.

## 1. Incident (what Desktop does)

Mallard's `claude.open-in-app` action launches `claude://resume?session=<id>`
(`crates/mallard-infrastructure/src/providers/claude/actions.rs:44`). On first
open, Claude Desktop 1.32885.1 **imports** the CLI session and rewrites the
live transcript under `CLAUDE_CONFIG_DIR` in place:

- strips every `thinking` / `redacted_thinking` content block from assistant
  rows;
- **deletes** assistant rows left empty (thinking-only rows);
- never repairs children's `parentUuid` — the links are destroyed;
- atomic replace (`.tmp` + rename; new inode, no backup).

Verified on the damaged transcript
(`…/-Users-hequ-Desktop-project-HorrorGame/54894825-….jsonl`) and Desktop's
own `main.log` ("1487 lines, 159 empty-after-strip dropped"): 1,487 → 1,328
lines, 159 dangling parents each absent from the whole file, zero surviving
thinking blocks, walk-back breaks 5 records from the leaf.
`analyze_claude_transcript` fails closed (`NonLinearParentage`,
`providers/claude/codec.rs:181`), the observer refuses evidence
(`src-tauri/src/project_sync/commands.rs:5447`), and the UI shows the generic
"could not prove the local and stored session state" row — the session is
permanently unsyncable. Every first Desktop open of a thinking-bearing
session does this.

## 2. Why the codec validates, and what stays

Sync comparisons feed capture selection, retention, and apply; a fabricated
"these states match" is how data gets silently discarded. The fail-closed
rules exist so the codec never claims what it cannot derive from bytes. All
of these stay exactly as shipped:

| Rule (fail-closed) | What a false pass would do |
|---|---|
| Malformed JSON / non-object line | No structural claim is trustworthy |
| Non-string uuid / bad `parentUuid` type | Identity arithmetic over non-identities |
| Duplicate uuid ON the derived path | Digest ambiguity, fold cycles (corpus: 0) |
| > 100,000 records | Unbounded parse cost per comparison |
| Zero chain records / no sessionId | Nothing to identify |

What changes is not *whether* Mallard validates but *what object* it
validates: the canonical conversation is redefined so that the states Desktop
freely rewrites between are literally the same object (§3), and damage the
projection cannot express falls to deterministic severed-root arms instead of
a dead end (§4).

## 3. D1 — projection semantics (the decision)

**The canonical conversation is derived from the Desktop-sanitized projection
of the transcript, not from the raw rows.**

- **P1 (projection).** `project(rows)`: assistant rows lose their
  `thinking`/`redacted_thinking` content blocks; assistant rows left with
  empty content are dropped entirely — uuid, links, sessionId/cwd
  contributions and all. Exactly Desktop's observed rewrite, which is
  **idempotent**: `project(project(x)) = project(x)`, and
  `project(original) = project(imported)`. All other rows pass through.
- **P2 (stitch).** The chain walk runs on the projection with the shipped
  positional rules; a `parentUuid` absent from the entire projection
  *stitches* — the effective parent is the nearest preceding chain record in
  file order. Canonical chains record normalized parents (each record's
  parent is the previous chain record; root `None`), so
  `validate_conversation_chain` and the domain stay untouched.
- **P3 (sever backstop, from v1).** A parent present in the projection only
  at later positions (forward-only) roots the walk there; a stitch with no
  preceding chain record roots. Conservative arms for damage the projection
  cannot express. Corpus: 0 — all 106 files walk to a true `null` root.
- **P4 (identity).** sessionId/cwd fallback and `record_count` come from the
  projection, so every derived artifact — chain, digests, folds, identity,
  count — is invariant across an import.

**Raw bytes stay raw everywhere else.** Captures publish verbatim transcript
bytes (thinking included — nothing is stripped at capture time, per explicit
user requirement); byte pins, probe-window hashes, torn-tail handling, and
truncation detection all operate on raw bytes unchanged. The projection
exists only inside the semantic derivation.

Consequences:

- An import changes bytes but not semantics: the watcher fires, the refresh
  sees `Invalid{Replaced}`, one bounded full re-analyze runs (D3a), the
  derived state is **identical**, and the row stays **Settled** (or
  LocalAhead, or whatever it was). No Diverged, no user action, no error.
- Thinking-only differences between any two states are invisible to
  comparison — by decree, that is the requested semantics. Thinking content
  still crosses machines verbatim inside captures; a capture pushed before an
  import remains the only surviving copy of that thinking (Desktop keeps no
  backup), and one pushed after carries the stripped bytes. Digests agree
  either way.

## 4. D2 — why the stitch is defined by consistency, not truth

The stitch cannot recover the true link in general, and does not need to.
The corpus forced this formulation:

1. *Naive thinking-invariant fingerprints* (strip thinking from fingerprints,
   nothing else) fail on extent: deleted rows sever the walk near the leaf
   (5/978 in the incident file) — still Diverged. Rejected by analysis.
2. *Link-transparent exclusion* (thinking rows excluded but their links
   followed on the original; stitch on the sanitized side) scored
   **103/106**. The three failures (`e77ec068`, `2045acc4`, `e7d1fab3`) share
   one shape: a **rewind whose branch begins with a thinking-only record** —
   deleting that row destroys the branch-attachment information, which is
   unrecoverable from the surviving bytes. Any "recover the truth" rule
   guesses.
3. *Projection semantics* dissolve the problem: both forms project to the
   same record sequence, so the stitch — whatever it picks — picks
   identically on both. Equivalence holds **by construction** and the corpus
   confirms it: **106/106** (§7).

Properties that make the stitch safe as an internal instrument:

- **Deterministic** pure function of bytes; replicas and refold-equality
  checks agree.
- **Append-monotone**: stitching only looks backward, so appends extend the
  chain and stored folds remain prefixes; ahead/behind detection is
  unaffected.
- **No false Settled** between genuinely different visible content:
  fingerprint-anchored fold states still separate any two chains that differ
  in surviving records.
- Footnote: in the three rewind-onto-thinking files the stitched chain folds
  in some abandoned-branch records — identically on both forms. The chain is
  a sync instrument, not a display artifact (session-detail rendering parses
  raw bytes through the timeline parser and is untouched).

## 5. D3 — cursor and feeder (`claude_history_tail.rs`)

`classify_claude_record` currently treats thinking-only rows as chain
records; the feeder and refresh need the projection applied incrementally:

- New `pub(crate) fn is_thinking_only_record(...)`: assistant row whose
  `message.content` is a non-empty array of only thinking blocks.
- `semantic_record_fingerprint` strips thinking blocks from assistant
  `message.content` (this plus dropping thinking-only rows *is* the
  projection — no value rewriting needed).
- `ClaudeFeedOutcome` gains **`Pending`**: a thinking-only row adds its uuid
  to an in-memory skipped-set and does **not** advance the consumed
  watermark. The `Appended` arm widens to `parent ∈ {leaf} ∪ skipped-set`
  (normalized parent = leaf, matching the full walk); a chain append clears
  the set and advances consumed through everything pending.
- **Watermark invariant:** consumed never passes a thinking-only row until a
  chain row lands beyond it. A trailing thinking run behaves exactly like the
  existing torn tail: unconsumed, refresh returns Unchanged, re-parsed next
  refresh (bounded by the run's size, transient within a turn). This keeps
  the skipped-set purely in-memory — no checkpoint schema change — and keeps
  warm appends O(append) even when a review lands between a thinking row and
  its continuation.
- Anything else (unknown absent parent, rewind, foreign session) stays
  `NeedsFullResolve` → one bounded full re-analyze, which reaches the same
  conclusion the feeder would have — the fold-split property test enforces
  exactly this agreement.
- `seed_claude_root` places consumed at the same invariant (stops before a
  trailing pending run).

## 6. D4 — lifecycle after this plan

| Event | Before (shipped) | v1 (severed-root) | v2 (this plan) |
|---|---|---|---|
| First Open in Claude on a bound session | Permanent generic error; unsyncable | One Diverged; push to resolve | **Nothing** — one bounded re-analyze, row unchanged (Settled stays Settled) |
| Session already imported long ago | Unsyncable | Syncs; short chain | Syncs; full-extent chain |
| Genuine damage (forward refs, etc.) | Unsyncable | Roots at the break | Roots at the break (P3) |

v1's "pre-open preservation push" stage is dropped as moot — the import no
longer needs defending against. (Consequence, accepted: thinking never pushed
before an import is simply gone; the user explicitly does not care.)

## 7. Corpus gates (run 2026-08-20, 420 files under `~/.claude/projects`)

The one-time equivalence scan, removed after the rules were frozen, checked
(`analyze(F) == analyze(desktop_sanitize(F))` over chain length, record
count, leaf uuid, sessionId, digest):

| Result | Value |
|---|---|
| Complete | **106 / 106** real main sessions (incl. both damaged files, now full-extent) |
| Equivalent original == sanitized | **106 / 106 — zero mismatches** |
| Root kinds | all `null` (no severs, no floors needed on real data) |
| Files whose walk stitches | 101 of 106 |
| Stitches on active paths | 9,493 across 49,476 chain records (~19% of links cross a projected-out thinking row — stitching is the norm, not the exception) |

The one-time sever-only backstop scan showed that the two damaged files yield
chains of 5/978 and 12/7,338 under sever-only rules; under projection they
stitch to full extent. The parity plan's §R8 verdict ("genuine corruption") is
revised: the old 53.2 MB reject bears the sanitizer damage class (987 distinct
absent parent uuids; its one surviving thinking block is post-import appends)
— almost certainly an earlier Desktop import.

Serialization robustness: fingerprints already parse-then-canonicalize
(sorted keys via `canonical_json`; default `serde_json` BTreeMap), so
Desktop's row re-serialization noise (key order, whitespace, string escaping)
cannot reach the digest. The only theoretical residue is JS number
re-representation (`1.0` → `1`) in rewritten mixed rows; no float-bearing
fields are known in assistant rows — S6's real-import gate settles it
empirically.

## 8. Stages

Every stage lands green on the standing bar: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, domain + infra +
src-tauri lib suites; frontend untouched.

**S1 — codec projection** (`providers/claude/codec.rs`)
`is_thinking_only_record`; the analyzer drops projected-out rows at the top
of the loop (before sessionId/cwd fallback capture — P4); fingerprint strips
thinking blocks; walk gains stitch (P2: effective parent = previous chain
record) and keeps sever arms (P3); canonical chain records normalized
parents; `record_count` = projected count. Tests: projection-equality
property `analyze(bytes) == analyze(project(bytes))` over fixtures including
a rewind-onto-thinking fixture (the 103/106 shape); pinned stitch digest;
sever/floor arms; dup unchanged; existing pinned digests re-pinned only where
fixtures carry thinking blocks (fixtures without thinking must keep their
digests — conservation check).

**S2 — feeder + refresh** (`claude_history_tail.rs`)
`Pending` outcome, skipped-set, watermark invariant, trailing-pending ⇒
`Unchanged` (torn-tail analogue). Fold-split property extended: splitting the
byte stream at every record boundary — including inside thinking runs — must
reproduce the one-shot analysis (fold, consumed, chain). Batch tests: T
pending → Unchanged; T,C in one batch → single Extended; C referencing an
unseen absent uuid → NeedsFullResolve, and the full re-analyze agrees with
the would-be stitch.

**S3 — seeding invariant** (`claude_history_tail.rs`, commands.rs untouched)
`seed_claude_root` consumed watermark stops before a trailing pending run;
push-apply and pull-install flows are otherwise unchanged (same call shape,
new semantics flow through).

**S4 — corpus + incident agreement gate**
Re-run both scans; assert the Rust analyzer agrees with the scan on the
incident file and one thinking-heavy healthy file (chain length, leaf,
digest-bearing fold equality via a temporary agreement harness, as the parity
plan did for its corpus freeze).

**S5 — lifecycle regression test** (src-tauri lib)
Bind → push a fixture session with thinking-only rows → simulate the Desktop
import (strip + drop + unrepaired parents + rewrite) → review: row stays
**Settled**, exactly one `record_full_graph_resolve` (the Replaced
re-analyze), zero pushes needed → append → cursor rides the probe window
(byte-metrics twin assertions). Variants: import *before* first push (push
works, digests equal the never-imported twin); rewind-onto-thinking fixture
(still Settled).

**S6 — real-import gate + docs + memory**
One-shot manual gate (documented, not CI): isolated `CLAUDE_CONFIG_DIR` with
a synthetic session (mixed row + thinking-only row + child), real
`claude://resume`, assert Mallard's analysis digests are identical pre/post
import (settles the JS-number residue empirically). Docs: parity plan §R8
verdict note; `CURRENT_METADATA_LAYOUT.md` unchanged (no schema change).
Memory updated. Record the wipe requirement (§9) in the landing note.

## 9. Compatibility

- **Digests change** for every transcript containing thinking blocks
  (fingerprints and chain membership change) — **`~/.mallard` and cloud data
  must be wiped** before retesting, per the standing no-customers clean-cutover
  policy. No migration, no legacy path.
- Codec key stays **`claude.lineage.v1`** (standing decision: semantics
  revise in place; nothing survives the wipe to disagree).
- No domain, wire-schema, checkpoint-schema, or UI changes. Codex untouched.
  Timeline parser / session-detail rendering untouched (raw bytes).
- **Sanitizer-spec pinning:** P1 mirrors Desktop 1.32885.1's observed rewrite.
  If a future Desktop changes the rewrite (strips more, or repairs parents),
  equivalence degrades to an honest Diverged — never corruption, never a dead
  end. (Repair-to-grandparent in particular would still stitch to the same
  effective parent in the common adjacent case.)

## 10. Out of scope (documented hazards, separate decisions)

- **`open -n` duplicate Desktop instances** (actions.rs:44): `-n` is
  load-bearing for delivering `CLAUDE_CONFIG_DIR` to non-default homes but
  spawns a second instance when Desktop is already running. Any fix trades
  against multi-home support; separate action-hardening pass.
- **Claude Desktop's own bugs** — duplicate `local_<id>` import when a native
  Desktop record already maps the session; the global import registry keyed
  by bare UUID across different `CLAUDE_CONFIG_DIR`s (anthropics/claude-code
  #80773). Upstream; not fixable from Mallard.

## 11. Implementation notes (landed 2026-08-20, deltas from the plan text)

All stages S1–S5 landed; the standing bar is green (fmt, clippy `-D
warnings`, domain 50 / infrastructure 269 / src-tauri 199, workspace 552).
Frontend untouched. Deltas and decisions made during implementation:

- **The record cap counts projected records** on both the analyzer and the
  cursor ordinal (the scan applied it to raw rows before sanitizing — a
  divergence only at the untested >100k margin). Rationale: the cap must
  fire identically on `F` and `sanitize(F)` for equivalence, and parse cost
  stays bounded by `MAX_CONVERSATION_FILE_BYTES` regardless.
  `consumed_record_ordinal` is therefore the projected record count at the
  watermark, not `analysis.record_count`.
- **`project_claude_record` is `pub`** (exported through
  `providers::claude`): the analyzer, the feeder, and the src-tauri
  lifecycle tests all consume the same projection.
  `semantic_record_fingerprint` itself is unchanged — callers hand it the
  projected record.
- **Seeding re-parses the record-aligned slice once** (`pending_watermark`
  in `claude_history_tail.rs`) to place the consumed watermark before a
  trailing pending run; `refresh_claude_tail` carries the incremental twin
  of the same bookkeeping and returns `Unchanged` when the whole batch is
  still pending. `RootPhysicalMapping` tolerates `consumed <
  source_file_size` natively; no schema or commands.rs change was needed
  (S3 held: call shape identical).
- **Analyzer failure-path counts**: a line that fails to parse is still
  counted in the reported `record_count` (`saturating_add(1)` at the return),
  so Unknown reasons keep a sensible record number without a second counter.
- **S4 gates re-run at landing**: Python scan 420 files / 106 Complete /
  106÷106 equivalent / zero mismatches; Rust `#[ignore]` gate
  `claude_corpus_import_equivalence` (codec.rs) agrees — 420 / 106 / 106÷106
  on the same corpus, incident file `54894825` records=1,328 chain=971
  (was 5 under shipped rules), old reject `9d825d28` now Complete chain=608.
  Chain-record totals drift by a few records between runs because the live
  corpus grows while the gates run; outcome counts are the invariant.
- **S5 tests**: `a_claude_desktop_import_is_invisible_to_sync` (push →
  simulated import → exactly one `full_graph_resolve`, row stays `Synced`,
  re-seeded cursor rides the next append which turns the row honestly
  `LocalAhead`) and `a_claude_import_before_first_push_is_equivalent`
  (rewind-onto-thinking fixture, digests equal the never-imported twin,
  push lands `Synced`). The UI state name for "Settled" is
  `ThreadSyncState::Synced`.
- **S6 real-import gate: pending the user's wipe + retest.** The headless
  half is done (both real Desktop-import artifacts in the corpus analyze
  Complete and equivalence holds on 106/106 files). The live half —
  isolated `CLAUDE_CONFIG_DIR`, real `claude://resume`, digest equality
  pre/post import, settling the JS-number-re-representation residue — runs
  with the planned `~/.mallard` + cloud wipe. Procedure stays as §8 S6.
- **Unrelated fix in passing**: the activity-workspace test
  (`activity_workspace_projects_one_local_and_remote_read_into_both_views`)
  began failing on 2026-08-21 because its 2026-07-22 fixture aged out of a
  wall-clock 30-day window; its `before_time` is now pinned. Two sibling
  call sites with 90-day windows carry the same latent rot (flagged as a
  follow-up task).

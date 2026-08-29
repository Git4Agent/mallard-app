# Plan: Claude Session Cursor Parity

Status: **Implemented** — 2026-08-20 (proposed and landed the same day; uncommitted at the time of writing). Stages A0–B5 are all in: the corpus-gate numbers below are the frozen acceptance record, the R7/R4/A3 decisions and the D4 implementation note record where the landed code deliberately refines the proposal, and the twin lib tests (`a_comparison_reads_no_claude_transcript_bytes_beyond_the_probe_window`, `a_claude_rewind_degrades_to_one_full_reanalyze_and_stays_syncable`) plus the extended live aimock e2e are the behavioral fence.
Goal: make Claude session observation behave like Codex session observation — checkpoint-cursor refresh (probe window + append-only fold) instead of full-file capture on every comparison — and, as a corpus-forced prerequisite, make the Claude lineage codec accept real Claude Code transcripts.

Constraints (user-set, hard):

- **Clean cutover.** No migration code, no legacy execution path, no dual-path compatibility. Replaced behavior is deleted in the same change ([[feature-changes-are-clean-cutovers]]).
- **Schemas unchanged.** `LOCAL_SCHEMA = 1` and every document shape stay as they are. Values flowing through existing fields may change (codec strings, tail states, cursor fields).
- **Wipe-based retest.** `~/.mallard` and cloud data are deleted before retesting. No customers, no compatibility obligations to existing pins, manifests, or checkpoints.
- **Publication integrity unchanged.** Push/Pull apply keep their own authoritative full captures and re-proofs; the cursor only accelerates *observation*.

---

## 1. Problem statement (measured)

Every Refresh / Push-review / Pull-plan comparison fully captures every Claude conversation, every time. This includes every load of the storage-scoped Activity / Git-history page: `get_project_activity_workspace` ([commands.rs:1333](../src-tauri/src/project_sync/commands.rs)) embeds `build_comparison_from_reads` → `build_thread_sync_comparison`, so opening that screen *is* a refresh (its activity half is already mtime-cached in `activity_cache.json`; the comparison half is the uncached, recurring cost):

- `base_selection` captures every conversation whose provider is not exactly `"codex"` — [commands.rs:6997-7009](../src-tauri/src/project_sync/commands.rs) (Refresh/Push twin at :10322 for Pull, comment: *"Providers without a cursor codec capture up front"*).
- Local-tail observation dispatches on `is_codex` at [commands.rs:6035](../src-tauri/src/project_sync/commands.rs); the non-Codex arm hard-requires captured bytes.
- Per captured file, the pipeline does: full `fs::read` → full-buffer lowercase copy for the secret scan → SHA-256 of all bytes → `analyze_claude_transcript` (full JSONL parse) → `summarize_bytes` (a **second** full parse) → a **third** full-buffer SHA-256 for the whole-file `RawRangePin` (`provider_capture.rs:434-543`, `commands.rs:6070-6074`).
- Nothing is persisted: Claude checkpoints are only written by Push/Pull apply, and the Phase-A seeding + checkpoint-maintenance loops are Codex-gated (`commands.rs:5463`, `:5826-5900`). Claude re-derives everything from scratch on every review.
- The watch service treats any non-Codex `.jsonl` modify event as a membership change, forcing full re-discovery (`codex_watch_service.rs:433`), and its observation/candidate-pin caches are hardcoded to `codex_provider()` (`:737`, `:793`).

Codex, by contrast, refreshes a bound thread by reading a ≤4 KiB probe window plus appended bytes only (`codex_history_graph::read_rollout_tail_window` + `refresh_codex_tail`), and reads **zero** bytes when the watcher proves the file quiet.

Local corpus (`~/.claude/projects`, scanned 2026-08-20): **414 files, 875 MB, largest 53.2 MB, p95 7.5 MB**. All of it re-read and re-parsed twice per comparison.

## 2. The corpus finding that reshapes the plan

Replaying the shipped codec rules (`analyze_claude_transcript`, [codec.rs:30](../crates/mallard-infrastructure/src/providers/claude/codec.rs)) over the local corpus:

| Outcome (shipped strict rules) | Files | Notes |
|---|---|---|
| `Complete` | **16** | **7.2 MB of 875 MB** |
| `NonLinearParentage` | **87** | rewind/branch appends; includes nearly every large file |
| No chain records | 289 | pure-sidechain `agent-*` subagent transcripts |
| No `sessionId` | 22 | metadata-only files (`queue-operation`/`started`/…) |

The shipped rule — every chain record's `parentUuid` must equal the *immediately preceding* chain record's uuid ([codec.rs:163-173](../crates/mallard-infrastructure/src/providers/claude/codec.rs)) — rejects ~70% of real main sessions, because real Claude Code files legitimately contain:

- **Rewind/branch appends** (1,896 records; user rewinds and re-asks → new record parented to an *earlier* uuid; the file stays append-only but the active chain switches branch).
- **Mid-file roots** (27 files; a chain record with `parentUuid: null` mid-file, e.g. after compaction).
- **Re-emitted uuids, re-parented** (7 files, but **6 of the 10 largest**, incl. the 53.2/42.0/36.3/31.4/30.3/30.0 MB files): 1–3 records per file re-appear later with a different `parentUuid` — some byte-identical minus the parent, some with changed bodies.
- **Mixed `sessionId`s** (30 files; resume-copied history keeps its original ids; 21 of the branch-tolerant-Complete files mix ids *on the active path*).

A cursor can only ride files whose full parse is `Complete`, so **cursor parity built on codec v1 would accelerate 16 files / 7.2 MB — under 1% of the bytes — while the 53 MB and 34.6 MB files stay on the every-refresh full-capture path.** This is precisely the fail-closed-rules-vs-real-corpus lesson already learned for Codex ([[codex-projection-corpus-validation]]: v1 projection rules rejected 22.1% of real rollouts and were rewritten to mirror the provider's own tolerances).

Replaying a branch-tolerant rule (Part A below) over the same corpus: **96 files / 395.5 MB Complete**, and the remaining 7 `DuplicateMessageId` files (≈224 MB, the largest ones) are recovered by the positional-resolution rule (§5 R4), for a target of **~103 of the ~103 real main sessions**; everything still excluded is subagent transcripts (289) and chain-less metadata files (22).

**Therefore the plan has two parts, in order:**

- **Part A — branch-tolerant chain derivation** (the `claude.lineage.v1` rules revised in place; key unchanged, see R7) so real transcripts are `Complete` (also fixing that ~70% of real Claude sessions are currently un-syncable `StoredOnly`).
- **Part B — cursor observation:** checkpoint seed/probe/append-fold refresh, Codex-parity dispatch, watcher integration.

Part B's equivalence target is Part A's parser; building B against the shipped strict rules would be wasted work.

---

## Part A — branch-tolerant chain derivation (`claude.lineage.v1`, revised in place)

### 3. New chain rules

Replaces the linear-scan fold in `analyze_claude_transcript` ([codec.rs:30](../crates/mallard-infrastructure/src/providers/claude/codec.rs)). Chain-record classification is unchanged (`is_chain_record`, codec.rs:227-238): `type ∈ {user, assistant, system, attachment}`, not `isSidechain`, has `uuid`.

- **R1 — leaf.** The canonical leaf is the **last chain record in file order**. (Claude Code's own resume semantics: the file tail is the live conversation.)
- **R2 — walk-back with positional resolution.** The canonical chain is the walk from the leaf to a root via `parentUuid`, where a parent reference from the record at file position *i* resolves to the **latest occurrence of that uuid at a position < i**. Resolution is strictly position-decreasing, so the walk is well-founded — no cycle detection needed. Records off the path (abandoned rewind branches, shadowed occurrences) are excluded from the chain but still counted in `record_count`.
- **R3 — mid-file roots.** A `parentUuid: null` record terminates the walk wherever it sits. A post-compaction segment naturally becomes the whole chain; pre-compaction records become non-chain history. No `compact-boundary` special-casing.
- **R4 — re-emitted uuids.** Under R2 they need no special rule: earlier segments of the file are immutable (a record's parent always resolves strictly earlier), and a re-emission simply shadows the old occurrence for later children. The same uuid may legally appear more than once on the derived chain; the wire invariant `chain[i].parent_message_id == chain[i-1].message_id` still holds by construction. *(Implementation check: confirm `validate_conversation_chain` (mallard-domain/src/lib.rs:3661) asserts only linkage, not id-uniqueness; if it asserts uniqueness, Claude's analyze validates linkage itself and skips that helper.)*
- **R5 — session identity policy.** `source_provider_session_id` = the `sessionId` carried by the leaf chain record (falling back to the last record carrying one). Mixed ids elsewhere in the file (resume-copied history) are tolerated and preserved. Fail-closed only when **no** record carries a `sessionId`. `source_cwd` is the first non-empty cwd in the active leaf session ID's current chain-record segment: a new ID resets the home, while later per-turn cwd changes within that ID do not move it. `rewrite_claude_transcript` keeps its exact-value-match semantics (codec.rs:386-402): only occurrences equal to the leaf identity are restamped on Pull; copied-history ids remain verbatim as provenance.
- **R6 — unchanged fail-closed set.** Malformed JSON / non-object line, non-string `uuid`, `parentUuid` of a non-null/non-string type, dangling parent **on the active path** (a parent uuid with no earlier occurrence), `> 100_000` records, zero chain records → `CodecLineage::Unknown` exactly as today. Unknown record *types* stay fail-open (skipped).
- **R7 — codec key stays `claude.lineage.v1` (decided 2026-08-20).** `CLAUDE_LINEAGE_CODEC_KEY` and `codec_version()` are unchanged ([mod.rs:105-111](../crates/mallard-infrastructure/src/providers/claude/mod.rs)); the chain-derivation semantics are revised in place and the strict-linearity behavior is deleted. Normal discipline would mint a new key for changed semantics; it is deliberately waived here: nothing outside this repo has ever consumed v1 values (no customers; `~/.mallard` and cloud data are wiped before retest), and keeping the key avoids churning every manifest value, validation pin, and test assertion that names it — the same precedent as resetting schemas to 1 during the backward-compat removal. The corpus gate (R8) and the live-CLI e2e stand in as the behavioral fence. If a build carrying the strict rules is ever released first, revisit this.
- **R8 — corpus gate.** Before freezing R1–R6, port the three scan scripts from this investigation into `scripts/` (types-and-structure only, no content) and record the acceptance numbers in this doc — the same discipline as the Codex projection v2 work. Acceptance target on this machine's corpus: every non-subagent, non-metadata-only file `Complete` (currently 96 + 7 = 103).

**Corpus gate results (2026-08-20, rules frozen).** Three one-time Python scans
covered the strict, revised, and duplicate UUID rules. They were removed after
the rules were frozen. Corpus at scan time: 420 files.

| | Strict (shipped) | Revised (R1–R6) |
|---|---|---|
| `Complete` | 15 files / 6.5 MB | **105 files / 585.5 MB** |
| Rejected main sessions | 91 `NonLinearParentage` + 22 no-sessionId | **1** (below) |
| No chain records (subagent + metadata files) | 292 + 22 | 314 |

- **The one remaining rejection is genuine corruption**: the 53.2 MB file's active path hits, 11 records from its live tail, a `parentUuid` that occurs nowhere in the file (not even as a sidechain record). Fail-closed (`NonLinearParentage`) is correct; the strict rules rejected it too, so no regression. It stays `StoredOnly`.
  - *Verdict revised 2026-08-20* (`docs/PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md`): the damage class matches Claude Desktop's resume-import sanitizer (987 distinct absent parent uuids; the file's one surviving thinking block sits in post-import appends) — almost certainly an earlier Desktop import, not random corruption. Under the projection semantics that plan shipped, the file analyzes `Complete` (chain 608) and is syncable.
- **Dup-on-path: 0 files.** No walk ever visits the same uuid twice, so `validate_conversation_chain`'s id-uniqueness assertion stays as the post-derivation gate (R4 amended: a uuid revisited by the walk fails closed as `DuplicateMessageId` instead of being legal — corpus shows it never happens; the 7 re-emitted-uuid files all resolve to exactly one occurrence per path node).
- **Trim classes across the 105 `Complete` files**: 33,362 `pre_root` (pre-compaction history), 2,821 `abandoned_subtree` + 150 `superseded_branch` (rewinds), 204 `shadowed_reemission`, and **640 `dead_end_leaf`**. The dead ends are 602 parallel-tool-result `user` records (`tool_result` + `toolUseResult`, `isMeta: false`), plus a handful of stray text records: real Claude Code files do fan tool results out, and the walk-back excludes childless ones from the chain. Accepted, with rationale: transcript **bytes** stay authoritative (Push publishes the full file; the chain is only the comparison proxy), every append still changes the digest (a newly appended record is the leaf at the moment it lands, so no state change is ever classified in-sync), and the walk mirrors Claude's own resume ancestry. The alternative — a file-order projection that keeps every record — was rejected because it cannot represent re-emitted uuids (6 of the 10 largest files) without synthetic id munging that would poison the `last_canonical_message_id` cursor.
- Leaf-wins identity: 24 `Complete` files mix `sessionId`s on the active path; 0 files needed the sessionId fallback (the leaf always carries one). 20 files have a mid-file active root (compaction).

### 4. Part-A consequences elsewhere

- ~70% of real Claude sessions flip from `Unknown → StoredOnly` to resumable/pushable. This is a functional fix independent of performance.
- Lineage and semantic digests change for any transcript containing branches (chain now excludes abandoned branches). Wipe covers it.
- Tests updated in place: `claude_codec_rejects_conflicting_sessions_branches_and_duplicates` becomes the tolerance suite for the revised rules (branch switch → shorter chain, not Unknown; conflicting sids → leaf-wins; re-emission → shadowing), plus new fixtures for R2/R3/R4. Live e2e chain-length assertions (`live_claude_aimock_e2e.rs:2645-2664`) are re-derived.
- **Subagent exclusion (A3) — verified 2026-08-20: no change needed (the "record that and skip" branch).** The 292 pure-sidechain `agent-*` files never become conversation resources today, for two independent structural reasons: (1) Claude discovery walks `~/.claude/projects` with `min_depth(2).max_depth(3)` ([discovery.rs:74-80](../crates/mallard-infrastructure/src/providers/claude/discovery.rs)) while every `agent-*` file sits at depth 4 (`<enc>/<parentUuid>/subagents/…`) or depth 6 (`…/subagents/workflows/<wf>/…`) — the walk never visits them; (2) even if visited, the head scan returns `None` for every sidechain record, so no metadata can form. They appear in no Activity list, no comparison row, and no recipe. Flipping `is_internal_subagent` at mod.rs:68 would be a no-op (that line is on the non-sidechain branch). **Do not add detection under the current identity scheme**: an `agent-*` file's `sessionId` is the *parent transcript's* id (its own identity is a separate `agentId` field), so routing them into `excluded_subagent_session_ids` would exclude the real parent conversation from bundle recipes, and discovering them as resources would collide with the parent's resource id (`duplicate resource ID` hard error). Proper subagent modeling would key on `agentId` and is out of scope.
- `history.rs` (timeline/presentation) and Activity summaries are untouched — separate, tolerant parser.

---

## Part B — cursor observation

### 5. Design inventory: what already fits

The checkpoint schema is provider-neutral and needs **zero changes**. Claude's resume state maps 1:1 onto existing fields:

| `RootPhysicalMapping` field ([conversation_tail.rs:344-363](../crates/mallard-domain/src/conversation_tail.rs)) | Codex meaning | Claude meaning |
|---|---|---|
| `provider_thread_id` | rollout thread id | leaf `sessionId` (= file stem) |
| `root_path` / `source_file_size` | leaf rollout file | session `.jsonl` file |
| `consumed_byte_offset` | position after last complete record | same (`safe_record_end`) |
| `consumed_record_ordinal` | global rollout ordinal | count of physical lines consumed (also enforces the 100k cap) |
| `probe_window_len` / `probe_window_sha256` | trailing ≤4096-byte hash | same, unchanged semantics |
| `last_canonical_message_id` | resume cursor for the chain builder | **uuid of the canonical leaf** — the one datum incremental linearity needs |
| `last_synthetic_turn_id` / `next_synthetic_turn_ordinal` | Codex synthetic-id state | `None` / `0` (validator already allows this) |

Likewise unchanged: `ConversationCheckpoint`, `PhysicalMappingState`, `RawRangePin` (seeded `[0, consumed]`; after apply the Codex convention `[accepted_end, consumed]` — see the D4 implementation note), `RootTailLayout` (constant `(claude tail codec, WholeThread, 0)`), `ConversationPrefixWitness` (**never present** for Claude — it is pinned to `FixedPaginatedTail && start > 0`, which Claude never has). Push apply already writes a degenerate whole-file mapping for non-Codex threads with `last_canonical_message_id: None` (`commands.rs:9130-9161`) — Part B replaces that hand-roll with a real seed; the shape was already right.

### 6. D1 — Tail values move to `TailAccumulator`

Today Claude tails are synthesized: `accepted_tail_state = semantic_digest` (`synthesized_whole_thread_tail`, commands.rs:4838-4850). That digest is one streamed SHA-256 over the whole chain (`conversation_chain_digests`, lib.rs:3682) — **it cannot be resumed from**, so it cannot ride a cursor. `TailAccumulator` (conversation_tail.rs:139-215) exists precisely for this: per-record chained fold, `(count, state_hex)` is a genuine resume pair.

- Claude tails are computed with `compute_portable_conversation_tail_with_codec` / `TailAccumulator` over the revised canonical chain, under **new codec strings** mirroring Codex's twin: `mallard-claude-portable-tail-v1` (local/observed) and `mallard-claude-portable-tail-wire-v1` (accepted/cloud). Same fold, different labels — the fold's hash-domain constants stay the shared `mallard-codex-tail-*-v1` byte strings (renaming them would force a Codex codec bump for zero semantic gain; accept the misnomer, note it in the constant docs).
- Cloud manifest threads for Claude now carry `portable_tail: Some(...)` (wire codec), written by `conversation_thread_from_capture` (today: `None` for non-Codex). `ConversationThread::validate` pins Claude threads to the Claude wire codec, mandatory-`Some` — the exact analogue of `validate_codex_wire` (conversation_tail.rs:87-96). Old tail-less Claude manifests fail closed; wipe covers.
- The cloud-evidence arm (`commands.rs:6006-6014`) and local-evidence arm (`:6040-6081`) for cursor-capable providers read/derive real tails; `synthesized_whole_thread_tail` **survives only for cursor-less providers** (the test provider) — that is its defined role, not a legacy path.
- All three evidence sides (accepted / local / cloud) therefore carry the same codec strings and are field-wise comparable by `compare_conversation_tail` with no classifier changes.

### 7. D2 — Module layout: shared byte-plumbing, parallel Claude graph

Per the layer audit of `codex_history_graph.rs`: the byte-plumbing layer is 100% format-agnostic; the append-validation and chain-codec layers are genuinely provider-specific (dense-ordinal continuity vs parent-pointer continuity). No trait until a third provider exists.

- **Extract `crates/mallard-infrastructure/src/jsonl_tail_io.rs`** (mechanical move, Codex deletes its copies in the same change): `TailWindow` (né `RolloutTailWindow`), `read_tail_window`, `read_range`, `read_bounded`, `open_no_follow`, `safe_record_end`, `hex_sha256`, `PROBE_WINDOW_BYTES`, the retry loop, and the metrics calls. Error strings de-Codexed.
- **New `crates/mallard-infrastructure/src/claude_history_tail.rs`**, mirroring the Codex module's surface:
  - `ClaudeChainFeeder` — the resumable analogue of `CanonicalChainBuilder`: `resume(last_canonical_message_id)` + `feed(&Value) → Appended(CanonicalConversationMessage) | Ignored | NeedsFullResolve | Malformed`, applying `is_chain_record` + `semantic_record_fingerprint` + the v2 append rules (§8). Also the one-shot `analyze` path is expressed through the same feeder so one-shot and incremental share one implementation (the fold-equivalence property then holds by construction for the linear case).
  - `seed_claude_root(bytes, analysis) → ClaudeSeededRoot { layout, observed_tail, mapping, raw_range_pin }` — consumes an already-run analyze; `consumed = safe_record_end(bytes)`; **skip seeding if the file does not end at a record boundary** (torn tail; corpus: 0/414 today) so a stored checkpoint always describes a `\n`-terminated prefix.
  - `refresh_claude_tail(layout, observed_tail, mapping, window) → ClaudeTailRefresh { Unchanged, Extended{..}, NeedsFullResolve, Invalid{Truncated|Replaced|MalformedAppend} }` — same probe/size/safe-end steps as Codex (those ~40 lines are generic and copied), Claude rules for the appended records.
  - No `resolve_*` graph walk — Claude has no `history_base` pagination; one file is the whole root. "Full resolve" for Claude = one bounded read + one full analyze + `seed_claude_root`.

### 8. D3 — Claude append rules and outcome mapping

For each complete appended line (never past `safe_record_end`; a trailing partial line is left unconsumed exactly as Codex does):

| Appended record | Outcome |
|---|---|
| Parses as object, not a chain record (metadata types, `isSidechain`, no `uuid`) | `Ignored` — advance byte/line cursors; a batch of only these yields `Extended` with an unchanged tail count (sidechain traffic must advance the cursor without folding) |
| Chain record, `parentUuid == last_canonical_message_id`, fresh linkage | **fold** — `TailAccumulator::fold_message`; update `last_canonical_message_id` |
| Chain record, `parentUuid` ≠ leaf (rewind/branch), or `parentUuid: null` (mid-file root), or `sessionId` present ≠ `mapping.provider_thread_id`, or `uuid == last_canonical_message_id` (re-emission of the leaf) | `NeedsFullResolve` — the canonical chain changed shape; one full re-analyze re-derives it and re-seeds |
| Line count would exceed 100k, malformed JSON, non-string `uuid`, bad `parentUuid` type | `MalformedAppend` |
| File shrank below `consumed_byte_offset` / probe hash mismatch | `Truncated` / `Replaced` |

**D3a — degradation posture (differs from Codex deliberately):** at the observation call-site, every non-fast-path outcome for Claude — `NeedsFullResolve`, `Invalid{Truncated|Replaced|MalformedAppend}` — degrades to the full path (bounded read + full re-analyze + re-seed), whose result is whatever today's full capture would conclude (including `Unknown → TailSideEvidence::Invalid` for genuinely bad files). Codex maps `Invalid` to a blocked thread because paginated rollouts are contractually append-only and a violated contract is evidence of corruption; Claude has no such contract, and full re-analysis is always available ground truth. Net effect: **Part B is a pure fast path — observable behavior is identical to today's full capture except for bytes read.** The equivalence obligation is: for any file state, cursor-path evidence ≡ full-capture evidence.

Trust posture (same as Codex): the probe window proves only the trailing ≤4 KiB; an in-place mutation *before* the window is invisible to observation. This matches Codex's cursor ride, and Push apply still re-captures and re-proves everything (digest pins, raw-range hash), so a stale observation can at worst produce a plan that apply rejects — the standing `codex_watch_service.rs` module-doc rule.

### 9. D4 — `commands.rs` cutover, site by site

Dispatch: add `SessionAdapter::supports_tail_cursor(&self) -> bool` (default `false`; Codex + Claude `true`) following the existing defaulted-method precedent (`index_fragment_limit`, traits.rs:527) — the existing `ProviderCapabilityLevel` ladder cannot discriminate (both providers already sit at `ResumableWithConflictPreservation`, and `capability()` has zero call sites today). Implementation fan-out (which observe/seed/refresh functions to call) stays a single provider-key match at the observation site; the many *gate* sites use the capability bool.

| Site | Today | After |
|---|---|---|
| `base_selection` :6997 and Pull twin :10322 | captures every provider ≠ `"codex"` | captures every provider with `!supports_tail_cursor()` (test provider); Claude rows become metadata-only (`LocalSessionView`), like Codex. Also fold the duplicated comment block at :6987-6996 into one. |
| Phase A first-contact seeding :5463 (`is_codex`) | Codex-only | any cursor provider; Claude branch: bounded read + full analyze + `seed_claude_root`; writes `root_tail_layout` + generation-0 checkpoint via the same `.or_insert` flow. Bound threads only (unbound = Codex parity, see D8). |
| Phase B observation block :5694 | `codex_observations` only | per-provider observation fan-out; `observe_claude_local` mirrors `observe_codex_local`'s skeleton (cache query → cursor ride → full-resolve fallback), minus pagination. *(Implementation note: Claude keeps the full range-overlap re-proof after all — Claude pins follow the same accepted-start convention as Codex (empty `[consumed, consumed]` after apply, `[accepted_start, consumed]` on observation), because a constant `[0, consumed]` pin would force a whole-file `read_range` on every cursor-ride extension, defeating the fast path.)* |
| Checkpoint maintenance :5826-5900, retention :5914 | Codex-gated | cursor-provider-gated; same generation-CAS, same skip-Detached/skip-identical rules |
| Cloud evidence arm :6006 | Codex requires `portable_tail`, others synthesize | cursor providers require `Some` (Claude wire codec); synthesize only for cursor-less providers |
| Local evidence arm :6035 | cursor vs captured-bytes | cursor for cursor providers; captured-bytes arm survives only for cursor-less providers |
| `local_capturable` :6117 | Codex from evidence, others from captured lineage | evidence-based for cursor providers |
| `expected_local_target` :6283 / candidate pins :6879-6960 | "Non-Codex candidates always capture" | Claude joins lazy two-pass candidate capture + the watcher's frozen candidate-pin cache |
| `required_codec` :6299 | falls back to captured lineage codec | metadata-only rows use `adapter.codec_key()` (same value) |
| Adoption probe (§6.4 refold, Codex-only today; `observe_codex_local` forces full resolve when probing) | Codex-only | Claude analogue: probe forces the full path; refold = `compute_conversation_tail_state` at `probe.end` over the v2 chain; exact match seeds `verified_prefix_tail`. Restores re-registration reclaim for Claude. |
| Push apply non-Codex checkpoint :9130-9161 | hand-rolled mapping, `last_canonical_message_id: None` | Claude: `seed_claude_root` over the captured bytes/analysis → real cursors. Test provider keeps the hand-roll. |
| Pull install mapping :11463-11480 | degenerate re-derivation | Claude: seed from the rewritten installed bytes (staging already re-analyzes them) |
| **Unchanged `is_codex` sites** | — | :5271-5272 (alias owners), :6433 (in-place vs relocated materialization — a different axis; Claude stays in-place), :6587-6590 (UUIDv7 CLI allocator), :8913-8914 (standalone artifact vs verbatim bytes — Claude publishes verbatim), :10976-11116 (relocated materialization + staged re-resolve), :11349 (titles), plus all Codex-CLI plumbing |

Frontend: no DTO changes; `ThreadSyncEntry`, decisions, and `compared_against_storage` are untouched.

### 10. D5 — Watch service

- Rename `codex_watch_service.rs` → `session_watch_service.rs` (the struct is already `SessionWatchService`; imports are mechanical).
- `take_reusable_observations` (:722) and `take_reusable_candidate_pins` (:793) take the provider key instead of hardcoding `codex_provider()`; `ObservationKey` is already provider-neutral (keyed by canonical home).
- Fix the membership rule (:430-433): drop `|| self.provider.as_str() != "codex"` so Claude `.jsonl` modify events take the cheap dirty-root path instead of forcing full re-discovery; membership changes remain Create/Remove/unknown-path.
- Extend the inventory cache to Claude requests (today "pure-codex requests only").
- `session_index.jsonl` title refresh (:393) stays Codex-only; Claude titles keep coming from head scans as today.
- Watch roots for Claude already exist (`session_root_names` :327-333).

### 11. Behavior matrix (target)

For a bound Claude thread with a 34.6 MB transcript:

| Scenario | Today | After |
|---|---|---|
| First review ever (no checkpoint) | full capture | full read + analyze once (Phase A seed), checkpoint persisted |
| Cold app start, file unchanged | full capture (~34.6 MB read, 2 parses, 3 hashes) | **≤4 KiB probe read**, `Unchanged`, evidence from checkpoint |
| Warm, watcher quiet | full capture | **zero filesystem work** (served from observation cache) |
| Linear append (new turns) | full capture | probe + appended bytes only; fold; checkpoint CAS-advanced |
| Rewind / re-emission / mid-file root append | full capture (strict rules: file → Unknown) | one full re-analyze, re-seed (and the file remains syncable, unlike under the shipped strict rules) |
| Truncated / rewritten file, torn tail, malformed append | full capture decides | full re-analyze decides — identical conclusions |
| Push / Pull apply | authoritative capture + re-proofs | unchanged (plus real cursor seed in the written checkpoint) |
| Unbound (never-synced) thread, cold | full capture | full analyze per cold review (Codex parity; watcher covers warm). Revisit only if this cold cost matters in practice. |

### 12. Test plan

- **Feeder/refresh unit tests** (in `claude_history_tail.rs`, mirroring the Codex module's suite): fold-split property — for corpus-shaped fixtures and any record-boundary split k, `refresh(seed(file[..k]), file) ≡ one-shot(file)`; probe mismatch → `Replaced`; shrink → `Truncated`; torn tail unconsumed; sidechain-only append advances cursor with unchanged count; rewind / mid-file root / re-emitted-leaf / foreign-sessionId appends → `NeedsFullResolve`; 100k-cap enforcement; malformed appends.
- **Revised-codec tests**: rewrites of the existing rejection test into the tolerance suite + fixtures for R1–R6 (branch switch, shadowing re-emission, mid-file root/compaction, mixed sessionIds, dangling-on-path still Unknown). There are no checked-in Claude JSONL fixtures today — build them inline (`json!`) as the existing tests do.
- **Corpus gate (R8)**: ported scan scripts under `scripts/`, acceptance numbers recorded here before freezing rules.
- **Sync-layer tests**: Claude twins of the Codex cursor tests — *a comparison reads no Claude transcript bytes when the watcher is quiet* (sync_metrics-based, precedent: `a_comparison_reads_no_codex_transcript_bytes…`), *refresh reads only probe+append on cold unchanged*, *checkpoint maintenance persists advanced observed tails*, *candidate byte-pin freezing for Claude*, plus the review/plan flows over metadata-only Claude rows.
- **Live e2e**: extend `live_claude_aimock_e2e.rs` — after the existing push/pull/divergence scenario, run a second review and assert cursor reuse (metrics), then append a real CLI turn and assert `Extended` + review equality with a from-scratch analyze.
- **Perf proof**: `MALLARD_SYNC_METRICS=1` before/after on this machine's real profile (875 MB corpus), recorded in `docs/PERF_SYNC_PATH_MEASUREMENTS.md`.

### 13. Stages and gates

Every stage lands green on the full bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p mallard-domain` / `-p mallard-infrastructure`, src-tauri lib == `npm run test:backend-integration`, frontend suite untouched.

1. **A0** — port scan scripts into `scripts/`, record corpus numbers (R8).
2. **A1** — revised chain rules in `providers/claude/codec.rs` (key and version unchanged per R7; test rewrite). Everything downstream still compiles: `analyze` signature and `CodecLineage` are unchanged.
3. **A2** — subagent exclusion for Claude: verified and skipped (A3 above — `agent-*` files are already structurally invisible; detection under the current identity scheme would be harmful).
4. **B1** — domain: Claude tail codec constants + Claude-thread validation pinning (mirror of `validate_codex_wire`).
5. **B2** — infra: extract `jsonl_tail_io.rs` (Codex cut over in the same change); build `claude_history_tail.rs` + its test suite.
6. **B3** — sync layer: the §9 table — dispatch capability, both `base_selection` sites, Phases A/B, maintenance, evidence arms, candidate pins, adoption, Push/Pull seeds. Delete the Claude-reachable synthesized/captured-bytes arms in the same change.
7. **B4** — watch service rename + provider-keyed caches + membership rule + inventory cache.
8. **B5** — live e2e extension + perf run + `docs/CURRENT_METADATA_LAYOUT.md` note (Claude checkpoints now carry real cursors; Claude threads carry wire tails).

### 14. Risks and open questions

- **R2 semantics vs Claude Code's own loader.** Positional latest-earlier resolution is inferred from corpus structure, not from Claude Code source. The corpus gate (R8) plus live-CLI e2e are the guards; if a real file class contradicts it, the rule gets revised before freeze — that's what A0 is for.
- **`validate_conversation_chain` uniqueness** (§3 R4): verify before B1; fallback is Claude-local chain validation.
- **Future Claude Code format changes** (e.g. `summary` records — zero in this corpus — or new metadata types): unknown types are fail-open in both one-shot and incremental paths, and any structural surprise degrades to full analyze (D3a). The cap `MAX_LINE_BYTES = 100 MiB` on the presentation side is unrelated to the observer, which never buffers more than the appended range.
- **Working-tree note (unrelated to this plan):** the uncommitted `history.rs` change reverts `TIMELINE_PARSER_VERSION` 2 → 1 while changing parser semantics (binary-preview redaction, tool summaries, stop_sequence scoring). Since `session_detail.rs` uses this version for cache invalidation, a same-version semantic change silently reuses stale caches. Worth double-checking that revert is intentional before it ships.
- **Deliberately out of scope:** Activity/timeline caching changes; checkpoints for unbound threads; a provider trait for tail engines (revisit at a third provider); any schema evolution.

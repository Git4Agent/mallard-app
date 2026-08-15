# PLAN: Path-invariant canonical identity and full content remap (`codex.lineage.v2`)

Status: proposed (2026-08-14).

Successor to the structural-only rewrite in `codex_standalone_builder.rs`
(`Reidentify` today touches only `session_meta`/`turn_context` cwd keys).
Corpus research (2026-08-14, 1,267 rollouts / 10.07 GB, aggregate-only scan)
found ~519k in-cwd path occurrences; ~194k live inside canonical
`response_item` records (`custom_tool_call.input`, `function_call.arguments`,
message text, tool outputs) whose bytes are digest-locked today.

## 1. Problem

After Pull, machine B's materialized rollout must reference B's project root
everywhere — including inside canonical message content — while sync still
recognizes A's thread and B's rewritten thread as the *same* conversation:
equal `lineage_digest`/`semantic_digest`, equal portable-tail fold states,
message-for-message classifier equality across machines and hops.

Today that is impossible by construction: `semantic_record_fingerprint`
hashes the full record content, the tail folds those fingerprints
(`conversation_tail.rs` fold), and every gate re-proves equality
(builder re-analysis `chain == resolved.chain`, Pull refold vs the pinned
cloud tail, prefix witnesses, the classifier). Rewriting one path inside one
`function_call` breaks all of them.

Constraints (user-set):

- **Minimal performance impact.** Refresh (watcher/cursor riding) and Push
  planning are hot; Pull apply already does full parse + full re-serialize.
- **Clean cutover.** No migration, no legacy fingerprint path, no dual-codec
  execution branches. Old pins fail closed.
- The tail machinery keeps working as-is ("still use the tail hash").

## 2. Design core: normalize before hashing

Do not change what the tail *does* — change what a semantic fingerprint
*is*. The v2 fingerprint hashes a **path-normalized image** of the record's
canonical JSON: every occurrence of a machine-local base path (in its
declared spellings) is replaced by an unforgeable indexed token before the
bytes reach SHA-256.

- A hashes `.../projact_alpha/readme` under base `/Users/a/project/projact_alpha`
  → `⟨0:r⟩/readme`.
- Pull rewrites content base-for-base; B's file says
  `/home/b/machineB/projectB/project_alpha/readme`; B normalizes under its
  own declared base → `⟨0:r⟩/readme`.
- Fingerprints agree ⇒ chains, digests, tail states, pins, and the
  classifier agree, **with zero changes to `TailAccumulator`,
  `compare_conversation_tail`, checkpoints, or the wire shape of
  `PortableConversationTail`** (only its `codec` string bumps).

Two properties make this minimally invasive:

1. **Bases come from the transcript itself** (§3.1), so
   `analyze_codex_transcript(bytes)` stays a pure function of bytes. No
   context threading through capture, graph resolution, or refresh.
2. **The existing verification gates become the remap's correctness proof.**
   The builder's re-analysis equality, the Pull refold against the pinned
   cloud tail, and the projection fixpoint all run unchanged — any path the
   rewriter misses, over-matches, or mangles produces a fingerprint mismatch
   and fails the build closed. No new proof machinery.

The normal form is **never serialized**. It exists only inside the hasher.
There is no canonical wire spelling of paths; artifacts carry the pushing
machine's real bytes, and each hop maps them to its own real paths. A →
B → A round-trips to A's original spellings.

## 3. Normal form specification (fingerprint v2)

### 3.1 Base registration

The ordered base list `B` is derived per file (or per resolved graph) from
exactly two sources, in observation order, first-appearance deduplicated:

1. The **authoritative** `session_meta` cwd (the first meta of the active
   thread; for the history-graph resolver, the leaf's authoritative cwd is
   seeded as base 0 *before* any segment is fed).
2. Every `turn_context` cwd, at the position its record is fed.

Later same-thread `session_meta` updates and **ancestor segments'
`session_meta` records do not register bases.** This is load-bearing: the
standalone builder drops ancestor metas and folds leaf metas into one head
record, so any base registered from a non-authoritative meta would exist in
the resolver's fold but not in a re-analysis of the flattened output, and
indices would skew. `turn_context` records survive flattening verbatim (they
are ordinary body records), and the authoritative head is always ordinal 0,
so both derivations of `B` are identical by construction. Ancestor-cwd
references that have no surviving `turn_context` carrier simply stay
un-normalized — consistently, on every machine (they also stay unrewritten;
§5.3's scope rule keeps the two aligned).

A record's fingerprint uses the bases registered **at or before** its
position. Bases only grow, so fingerprints are prefix-stable: an incremental
fold over an append window equals the one-shot fold (§4).

Validation: each base must be absolute, clean, 6..=4096 bytes, ≥2
components, no `"`, `\`, or control characters (the existing
`parse_session_meta`/`structural_cwd` bounds already reject most of this; a
base failing the residual checks ⇒ analysis `Unknown`, fail closed).
`MAX_NORMALIZATION_BASES = 16`; exceeding it ⇒ `Unknown`.

### 3.2 Patterns

For base `b` at index `i`, three spellings (deduplicated):

| k | pattern |
|---|---------|
| `r` | `b` |
| `u` | `file://` + `b` |
| `p` | `file://` + percent-encode(`b`) — exactly `path_to_file_uri`'s escape set (`codex_paginated_projection.rs`), emitted only when it differs from `u` |

Tilde spellings are **out of scope by design**: the home directory is not
derivable from file bytes, and the corpus shows 73 occurrences (0.014%).
They stay byte-identical on every machine, so identity is unaffected.

### 3.3 Boundary rule

A pattern occurrence counts only if the byte before and the byte after are
each **not** in `[A-Za-z0-9._-]` (start/end of the string scalar counts as a
boundary). This rejects sibling-directory false positives
(`/root/alpha` inside `/root/alphabet`, `/root/alpha-2`, `/root/alpha.bak`)
and foreign-mount suffixes (`/host/Users/a/...`), while accepting
`path:42`, `(path)`, quoted and space-delimited paths. The rule evaluates
identically on decoded strings (rewriter side) and on canonical-JSON text
(normalizer side): no character of the boundary set is ever JSON-escaped,
and bases may not contain `"` or `\`, so serialization cannot move a match
or flip a boundary decision.

### 3.4 Token

Replacement token: `\x00MPV2:{i}:{k}\x00` (raw NUL bytes, decimal `i`,
`k ∈ {r,u,p}`). `canonical_json` escapes every control character as
`\uXXXX`, so a literal NUL in the hashed text is unforgeable by any file
content. Distinct `(i, k)` keep distinct tokens, so duplicate detection in
`CanonicalChainBuilder::feed` (the `seen` fingerprint map) cannot conflate
records that differ only in which base or spelling they mention.

### 3.5 Pipeline

`semantic_record_fingerprint` v2 = strip volatile keys (unchanged) →
`canonical_json` (unchanged) → single Aho-Corasick **leftmost-longest** pass
over the text with all active patterns, streaming the between-match segments
and tokens straight into the SHA-256 hasher (no intermediate buffer).
Leftmost-longest resolves nesting (`file://b` beats `b` at the same region;
a longer base beats its prefix base). The automaton is cached on the builder
and rebuilt only when `B` grows (≤ a handful of times per file, ≤48 short
patterns).

### 3.6 Object keys

The scan runs over the full canonical text, keys included, so path-valued
keys (corpus: only `event_msg:patch_apply_end.changes.<path>`) normalize
too. Because `canonical_json` sorts keys by their *raw* bytes, remapping a
key could reorder it relative to a sibling and change the normalized text.
This is only reachable if a **canonical** record carries a path-valued key —
zero corpus occurrences — so the rewriter refuses it (§5.3, typed error) and
cross-machine canonical key bytes stay identical, making key order and
normalization agree everywhere. Chain-Ignored records rewrite keys freely.

## 4. Codec and fold changes

- `CanonicalChainBuilder` observes `session_meta`/`turn_context` records to
  maintain `B` + the automaton *before* returning `Ignored`; durable records
  fingerprint through §3.5. `appended` outcomes and chain semantics are
  untouched.
- `CanonicalChainBuilder::resume(last_message_id, synthetic_turn, bases)` —
  the saved split point now includes the ordered base list.
- `RootPhysicalMapping` (domain, `conversation_tail.rs`) gains
  `normalization_bases: Vec<String>` — **required, no serde default**, so
  every pre-v2 checkpoint fails closed (consistent with the
  `conversation_checkpoints` precedent). Validated per §3.1 bounds.
- `codex_history_graph`: `ResolvedCodexRoot` exposes the accumulated
  `normalization_bases`; `seed_codex_root` persists them into the mapping;
  `refresh_codex_tail` seeds the resumed builder from the checkpoint and
  extends the list when the append window contains new `turn_context`
  records (persisting the extended list with the advanced mapping). A
  `session_meta` in the append window is already `Invalid(Replaced)` —
  unchanged, and it protects base 0.
- Codec identity: `PORTABLE_TAIL_CODEC_V1`/`CODEX_LINEAGE_CODEC_KEY` →
  `codex.lineage.v2`. The v1 constants and every branch mentioning them are
  deleted, not aliased.

Domain fold, `TailAccumulator`, golden vectors: **unchanged**. The
`TAIL_VECTOR_*` vectors take fingerprints as inputs, so they keep passing
byte-for-byte — assert this explicitly in review.

## 5. Pull: the full-content rewriter

### 5.1 Mode

```rust
StandaloneBuildMode::Reidentify {
    root_provider_thread_id: String,
    path_map: Vec<PathMapEntry>, // (source_base, mapped_base), index-aligned with B
}
```

replaces today's `target_cwd: String`. The map covers exactly the resolved
root's `normalization_bases`, in order.

### 5.2 Map derivation (Pull site, `apply_prepared_pull_conversation`)

- `source_root` = staged leaf `session_meta.cwd` with the thread's
  `presentation.relative_cwd` suffix stripped (validated: the cwd must end
  with the suffix; `.` or absent ⇒ `source_root` = the cwd itself).
- For each base: under `source_root` ⇒ `binding.project_root` + preserved
  suffix (this also fixes the current flattening of subdirectory sessions —
  today a session run in `rootA/sub` materializes with `cwd = rootB`);
  otherwise ⇒ identity (foreign cwds of the 1.7% multi-cwd sessions stay
  truthful instead of being collapsed).

### 5.3 Rewrite scope and guards

One matcher (§3.2 patterns, §3.3 boundaries) drives everything:

- `session_meta`/`turn_context` cwd **fields** are rewritten by exact
  base-for-base lookup (no more collapse-to-target).
- Every record — canonical and chain-Ignored alike — has its string values
  rewritten through the matcher, form-preserving (`r`→raw, `u`→URI,
  `p`→percent URI). Object keys are rewritten in chain-Ignored records; a
  matcher hit inside a **canonical** record's key blocks the build with a
  typed error (§3.6).
- **Rewriter scope ≡ normalizer scope.** No extra spellings, no clever
  additions on canonical records: any asymmetry is a permanent false
  divergence. (Chain-Ignored records could legally take extras; keeping one
  scope keeps one code path.)

Fail-closed build guards (typed errors): the map is index-complete and
injective; mapped bases satisfy §3.1 validation; no mapped base creates a
new boundary-valid match of any base or any other mapped base (substring
audit over the ≤16×2 set); `target ⊃ source` prefix nesting that would
re-trigger matching is rejected.

### 5.4 Verification (unchanged code, new meaning)

`build_standalone_codex_root` keeps its exact proof obligations: re-analyze
output ⇒ `chain == resolved.chain`, digest equality, refolded tail equality,
layout partition via `effective_tail_partition`, `validate_projection_fixpoint`
(the rewrite runs before projection completion, so synthesized
`item_completed` items inherit rewritten event text; adopted items are
validated at envelope depth as today). Under v2 these equalities hold **iff
the rewrite was exactly inverse to normalization** — the gates are the
proof. Pull's refold against `cloud_tail.accepted_tail_state` and the prefix
witness recomputation likewise run unchanged.

The same-machine repair flow (`remap_codex_structural_cwds` +
`inspect_codex_structural_cwds`) is replaced by this rewriter with an
identity-outside-root map; the audit's "stale cwd" detection widens to the
matcher scope.

## 6. Push, republish, multi-hop

- `PreserveIdentity` stays byte-verbatim for body records — Push cost is
  untouched.
- B republishes its rewritten thread: the artifact carries B-form bytes, but
  its digests/tail are normalized values equal to A's pins, so
  `LocalOutstanding → Publish` classifies exactly as an ordinary append.
- C pulls B's artifact with a B-bases→C map; A pulls it with a B-bases→A map
  and — because forms are preserved and maps are inverses over the same
  matcher — recovers its original spellings byte-for-byte on canonical
  records. Composability needs no canonical wire form: the normal form is
  the invariant and it never leaves the hasher.

## 7. Performance budget

| Flow | Change |
|---|---|
| Watcher quiet reuse, dirty accounting, byte-pin caches | none — all keyed on file bytes/paths, which normalization never reads or writes |
| Refresh (cursor riding) | +1 automaton pass per **appended** record, streamed into the existing hasher; base list rides the checkpoint; big-O unchanged |
| Push planning / publish | none (byte digests `expected_local_target`, `artifact_sha256`, probe windows are raw-byte and stay raw-byte) |
| Full analyze (capture, verification points) | +1 scan beside the existing `canonical_json` allocation + SHA-256; estimated <5% of that step, well under parse cost |
| Pull apply | matcher passes amortized into the already-present full parse + full re-serialize of every record; guards are O(|map|²) over ≤16 entries |

Dependency: `aho-corasick` (already in `Cargo.lock` transitively via
`regex`) becomes a direct dependency of `mallard-infrastructure`.

## 8. Cutover (clean, no migration)

- Codec strings flip in: `conversation_codec.rs`, domain
  `conversation_tail.rs` (`validate_codex_wire` pin), `providers/codex/mod.rs`,
  and the three `sync_tests` fixtures referencing `codex.lineage.v1`.
- `BUNDLE_SCHEMA: u32 = 1 → 2` (domain): every existing cloud snapshot fails
  closed at decode; storages read as fresh; threads classify
  `MissingCloud → CreateCloud` and re-publish v2 artifacts. No v1 reader.
- Local state: `normalization_bases` is required inside checkpoint mappings
  and the codec pin moves, so pre-v2 `sync_config`/bindings rows fail
  closed; recovery is the existing first-contact path (Phase A seeding
  re-derives layouts/checkpoints from local bytes as v2). No repair
  shims, no dual validation.
- Deleted in the same change: v1 fingerprint function, `Reidentify.target_cwd`,
  the collapse-style `rewrite_turn_context`, `rewrite_codex_transcript`'s
  structural-only body (repair adopts the unified rewriter).

Cost stated plainly: all machines re-push, remote-only history in old
snapshots becomes unreadable dead weight in the bucket, and cross-machine
sync resumes only after each side re-reviews. This is the accepted
schema-reset pattern (Tier-1 precedent).

## 9. Non-goals and residuals

- `~`-spelled paths (73 corpus occurrences) and `$HOME`-relative paths
  outside the project (~360k) stay as written — identical on every machine.
- Ancestor-cwd references without a surviving `turn_context` carrier stay
  as written (consistently un-normalized, §3.1).
- Claude provider untouched (`PLAN_CLAUDE_PROJECT_PATH_REMAP` remains
  directory-name-level; Claude content has no fingerprint fold).
- Future subagent transport (`PLAN_CODEX_SUBAGENT_CLOSURE_TRANSPORT`) must
  reuse this matcher for child files; note added there when implemented.

## 10. Tests

New (names in suite style):

- `normalized_fingerprints_are_equal_across_a_full_content_remap` — A-form
  file vs rewriter output under a real map: equal chains, digests, tails;
  covers raw/URI/percent forms, nested-JSON `arguments`, patch-key rewrite
  in `patch_apply_end`, and boundary negatives (`alphabet`, `alpha-2`,
  `/host/Users/...`).
- `base_indices_agree_between_resolver_folds_and_flattened_reanalysis` —
  forked P0/P1/P2 fixture with an ancestor-era `turn_context` cwd change;
  also pins that ancestor metas register no bases.
- `multi_cwd_sessions_map_injectively_and_foreign_cwds_stay` (+ the
  non-injective and key-in-canonical-record typed-error cases).
- `duplicate_detection_distinguishes_base_and_form_tokens`.
- `refresh_extends_normalization_bases_from_appended_turn_contexts` —
  resumed fold == one-shot fold across a base-adding append.
- `a_to_b_to_a_round_trip_restores_original_canonical_bytes`.
- Extend `cross_machine_paginated_tail_survives_pull_divergence_and_republish`
  with in-content paths end to end.
- Cutover: v1-codec docs/artifacts fail closed with typed reasons;
  `TAIL_VECTOR_*` golden vectors unchanged.

Update-in-place: codec tests asserting v1 fingerprints, harness/relation
fixtures pinning the codec string, builder tests asserting the old
collapse rewrite.

## 11. Touch list

| Area | Files |
|---|---|
| Normal form + builder | `crates/mallard-infrastructure/src/conversation_codec.rs` |
| Base plumbing | `crates/mallard-infrastructure/src/codex_history_graph.rs`, `crates/mallard-domain/src/conversation_tail.rs` (`RootPhysicalMapping`, codec pin) |
| Rewriter | `crates/mallard-infrastructure/src/codex_standalone_builder.rs` |
| Pull site + repair | `src-tauri/src/project_sync/commands.rs` (map derivation; repair/audit rewrite) |
| Codec key | `crates/mallard-infrastructure/src/providers/codex/mod.rs`, domain constant |
| Cutover | domain `BUNDLE_SCHEMA` |
| Tests | codec/builder/graph module tests, `src-tauri/src/sync_tests/*` |

## 12. Open questions

1. `MAX_NORMALIZATION_BASES = 16` — measure the real per-file maximum
   before freezing (types-only scan, same discipline as the projection
   corpus work).
2. Boundary set `[A-Za-z0-9._-]` — accept the documented space-in-sibling
   false positive, or extend? (Recommend: freeze as specified; it is the
   rewriter/normalizer *shared* definition, so any later change is a codec
   bump.)
3. Whether the repair audit widens to matcher scope in this change or a
   follow-up (recommend: same change, it shares the rewriter).

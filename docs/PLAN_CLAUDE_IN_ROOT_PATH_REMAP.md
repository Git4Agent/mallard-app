# PLAN: Claude in-root path remap — compose pathenc into the Claude wire

Status: **implemented** (2026-08-21). Full bar green: domain 51 +
infrastructure 312 + src-tauri 221 backend tests, frontend 226,
`clippy -D warnings`, `fmt --check`. Deltas from the proposal are recorded
in §12; D1–D3 resolutions in §11. Live wipe-based retest (wipe `~/.mallard`
+ cloud) remains with the user — the new `wire_path_map` stamp is mandatory
for claude rows, so pre-cutover cloud data fails typed gates by design.

Companion to `docs/PLAN_CODEX_IN_CWD_PATH_REMAP.md` (implemented
2026-08-15) and `docs/PLAN_CREDENTIAL_REDACTION.md` (implemented
2026-08-21). Those two changes did the heavy lifting — token grammar,
dual-pin identity, pin-relative classification, and a genuine Claude wire
transform already exist. This plan is a *composition*: Claude's wire
becomes `W(x) = redact(pathenc(x))`, exactly the Codex composition order,
using the same mapper, the same token, and the same gates.

Compatibility (user, 2026-08-21): clean cutover; no migration or legacy
execution path; keep every schema number the same. The user wipes
`~/.mallard` and cloud data before retesting; there are no customers.

## 1. Problem

Pull relocates a Claude transcript into the destination bucket and
rewrites only the structural `sessionId`/`cwd` fields
(`rewrite_claude_transcript`, `providers/claude/codec.rs:421`; invoked at
`commands.rs:11389`). Every project path embedded in *content* —
`message.content[].input.file_path`, tool results, prose — still spells
the source machine's checkout.

Visible failure chain (verified 2026-08-21):

- The rewrite contract is pinned to exactly two keys
  (`rewrite_touches_only_declared_structural_fields`,
  `providers/claude/codec.rs:1455` — asserts prose and `snapshot.path`
  occurrences of the old root survive).
- Claude's timeline parser only relativizes a tool path that falls under
  the *current* project root or a canonical alias
  (`providers/claude/history.rs:864`); a foreign source root never
  matches, `affected_paths` stays empty, and `tool_summary` falls back to
  the raw absolute source path (`history.rs:1371`).
- The Session Detail sanitizer knows only destination-root spellings
  (`session_detail.rs:1324`), so the foreign path renders verbatim.

A sanitizer-only fix would hide the symptom while the installed
transcript — and every future resume's model context — keeps dead
source-machine paths. Rejected.

Why the naive fix is illegal: Claude fingerprints strip only
`sessionId`/`cwd`/`uuid`/`parentUuid`/`timestamp`/`version`/`requestId`
(`providers/claude/codec.rs:368`); content is digest-relevant. A recursive
replace at pull would change chain fingerprints, and the pull refold gate
(`commands.rs:11411`) would correctly reject the transcript against the
accepted cloud pin. The remap has to live in the wire form, with the
local form folded separately — which is precisely the dual-pin machinery
Codex already uses and Claude already carries for redaction.

## 2. What already exists (do not rebuild)

- **Shared mapper** (`codex_path_map.rs`): `ROOT_TOKEN =
  "/__mallard_root__"` (absolute-path-shaped; contextual `file://`
  URI-ness), `CodexPathMapper` with leftmost-longest boundary scan,
  `encode_record` / `encode_record_gated` (span gate
  `decode(wire) == canonical`), `decode_record`, `verify_wire_record`
  (`encode∘decode = id`; doubles as the collision detector),
  `validate_pathmap_root` (POSIX gate, Windows blocked),
  `PATH_MAP_CODEC_V1 = "mallard.pathmap.v1"`. `encode_record` /
  `decode_record` walk arbitrary `serde_json::Value`s — string values
  *and object keys* — and are provider-neutral already.
- **Dual pins, provider-neutral**: `ConversationCheckpoint.accepted`
  (wire) + `accepted_local_tail`; pin-relative 4-input classifier; the
  form ledger (every digest/tail/witness belongs to exactly one form).
- **Claude wire pipeline** (from credential redaction):
  `CLAUDE_PORTABLE_TAIL_CODEC_V1` / `_WIRE_V1`; push builds
  `ClaudeWirePublication` from `redact_jsonl_text` + genuine wire fold
  (`commands.rs:9036-9103`) behind the cross-form layout gate; pull
  certifies the redaction fixpoint before install (`commands.rs:11386`);
  adoption compares wire-vs-wire by redacting the local slice
  (`commands.rs:5206-5218`, `adopt_claude_acceptance` at 4995);
  `ConversationWireRedaction` stamp on the thread row.
- **Precedent for the missing piece**: Codex `StandaloneBuildMode::
  {Encode, Decode}` composes `redact` after `pathenc` inside the builder;
  pull re-encodes under the future-push pattern set as its gate.

The single gap: Claude's wire transform is `redact(x)` where it must be
`redact(pathenc(x))`, and Claude's pull decodes nothing.

## 3. Corpus facts (2026-08-21 scan: 140 main sessions, 710 MB, aggregate-only)

`~/.claude/projects`, main sessions only (345 `agent-*` sidecar files
excluded), root basis = leaf-wins top-level `cwd`:

- 115/140 files carry in-root path occurrences. By entry type:
  `assistant` ~79k, `user` ~72.6k, `attachment` ~11.7k, `system` ~1.3k,
  `last-prompt` 785, `queue-operation` 488, `file-history-snapshot` 157,
  `file-history-delta` 26. Only `user`/`assistant` are chain records
  (`classify_claude_record`); the rest are wire-only content.
- Hot keypaths: `$.cwd` (112.8k — every entry), `input.file_path` (11k),
  `toolUseResult.filePath` (7.3k), `toolUseResult.stdout` (5.8k),
  `input.command` (5.5k), `toolUseResult.file.filePath` (3.6k),
  `attachment.filename`, `lastPrompt`, `structuredPatch[].lines[]`,
  `backup.realParentDir`, nested prose.
- **Zero paths used as JSON object keys** (Codex had them in
  `patch_apply_end.changes`; the shared mapper handles keys anyway —
  keep the uniform walk).
- Spellings: tilde 39 occurrences / 5 files; `file://` 266 / 10 files —
  both already in the mapper's pattern set.
- 47/140 files hold more than one distinct structural `cwd` (vs 1.7% for
  Codex): Claude sessions re-root mid-file legitimately. The immutable
  base stays `binding.project_root` (+ canonical alias) per the Codex
  hard invariant — in-root cwds (root or subdir) tokenize with their
  suffix; foreign cwds ship byte-identical.
- 2 unparseable lines corpus-wide; both outside lineage-Complete
  captures. `analyze_claude_transcript` already returns `MalformedJson`
  for any unparseable line (`codec.rs:165-172`), so a pushable
  (Complete) transcript parses line-by-line by construction.

## 4. Design

One sentence: Claude's wire transform becomes `W(x) =
redact(pathenc(x))` at push build, pull verifies the wire form against
the cloud pin *before* decoding, then decodes token → destination root
per line, restamps `sessionId`, installs, and folds the installed bytes
as the genuine `accepted_local_tail` — deleting today's relabel.

### 4.1 Mapper reuse and module naming

Rename `codex_path_map.rs` → `session_path_map.rs` (precedent:
`codex_watch_service.rs` → `session_watch_service.rs` in the cursor
parity cutover); `CodexPathMapper` → `SessionPathMapper`. Mechanical —
the record transforms are already provider-neutral. Add one thin
line-level facade used by the Claude sites:

- `encode_jsonl_line_gated(&self, line: &str) -> Result<String, String>` —
  parse, `encode_record_gated`, serialize. Typed error (with the line
  ordinal) on unparseable input; unreachable behind a Complete analysis,
  defensive otherwise.
- `decode_jsonl_line_verified(&self, line: &str) -> Result<Value, String>`
  — parse, `verify_wire_record` (this *is* the
  `encode(decode(w)) == w` pull gate under the future-push pattern set),
  then `decode_record`. Returns the `Value` so the caller can restamp
  `sessionId` before serializing once.

No token, boundary, or grammar changes. `serde_json::Map` is `BTreeMap`
here (no `preserve_order`), so encode/decode re-serialization key-sorts
every line; the installed transcript is *already* key-sorted today by
`rewrite_claude_transcript`'s parse-and-reserialize, and the live aimock
`--resume` of installed transcripts is green, so this property is
proven tolerable. Wire fold states are unaffected either way
(fingerprints hash `canonical_json`, which sorts).

### 4.2 Push (the `supports_cursor` branch, `commands.rs:9007`)

Order inside the existing branch, after the seeded-local pin check
(`:9030`):

1. Operation-level root gate: `validate_pathmap_root("project root",
   &binding.project_root)` — same placement as the Codex push site
   (non-POSIX/Windows binding roots block before any parsing).
2. Build the mapper from `binding.project_root` +
   `canonical_project_root` alias (identical inputs to Codex `Encode`).
3. **pathenc pass**: map each line of the captured consumed slice
   through `encode_jsonl_line_gated` (per-record span gate included).
   Reserved-token occurrence in source content = typed block, no
   escaping — unchanged Codex policy.
4. **redact pass**: existing `redact_jsonl_text` +
   `certify_wire_document` over the pathenc output (composition order
   `redact ∘ pathenc`, matching the Codex builder).
5. **wire self-check pass**: `verify_wire_record` per final wire line —
   the collision detector now also certifies that redaction placeholders
   did not manufacture or destroy token spellings.
6. Existing steps unchanged: wire re-analysis, cross-form layout gate
   (now also proving pathenc is per-record content-only), genuine wire
   fold, `ClaudeWirePublication`.

The wire-form structural `cwd` becomes the token spelling
(`/__mallard_root__` or `/__mallard_root__/sub`); the token is
absolute-path-shaped precisely so provider structural validation keeps
parsing (`validated_session_metadata` requires an absolute cwd). The
cloud row's `cloud_cwd` (derived from wire analysis at prepare,
`commands.rs:10788`) becomes token-form — consumed only by the pull flow
below, which expects it.

Local side is untouched: `accepted_local_tail` still seeds from the
original local bytes; refresh/cursor riding never sees the mapper.

### 4.3 Pull (the `provider_supports_tail_cursor` branch, `commands.rs:11377`)

Reordered to match the Codex form ledger ("wire-form verification ends
here" before any decode):

1. Root gate: `validate_pathmap_root` on `binding.project_root`.
2. Certify redaction fixpoint on the downloaded bytes (existing).
3. Cloud-tail shape gate (existing): claude wire codec, WholeThread@0.
4. **Wire verification (moved before decode)**: analyze the *downloaded*
   wire bytes; refold the wire chain to
   `cloud_tail.accepted_tail_state` at the pinned end. Today this check
   runs on the rewritten bytes (`:11402-11423`) and only works because
   the identity rewrite is fingerprint-exempt; with content decoding it
   must anchor to the wire form.
5. **decode pass**: per line, `decode_jsonl_line_verified` under the
   destination mapper (`binding.project_root` +
   `canonical_project_root` — the future-push pattern set, so the
   verify step doubles as the collision detector: destination-root text
   already present verbatim in out-of-root wire content fails closed,
   the documented Codex residual), then restamp `sessionId` on
   occurrences equal to the leaf identity (resume-copied history keeps
   foreign ids as provenance — unchanged rule), serialize once.
   Top-level `cwd` needs no dedicated rewrite: a bound session's cwd is
   the binding root, so its token decodes to the destination root.
   Foreign cwds pass through verbatim. *(The proposal's subdir-suffix
   preservation claim was superseded during implementation — user,
   2026-08-21: a session launched in a subdir is its own project; see
   §12.)*
6. Re-analyze the decoded bytes; require Complete; the **genuine local
   fold** of the installed bytes becomes `accepted_local_tail` — the
   relabel at `commands.rs:11429-11435` (local label wrapped around the
   cloud state) is deleted. Digest pins and `artifact_sha256` in the
   checkpoint seed are computed from the installed (local-form) bytes,
   honoring the form ledger: materialization op digest pins are
   LOCAL-form, matching what the engine verifies against staged bytes.
7. Leaf cursor seeding from installed bytes: unchanged mechanics.

Identity checks currently owned by `rewrite_claude_transcript`
(source-session-id equality, resumability round-trip) move into this
flow: the wire analysis in step 4 proves
`source_provider_session_id == prepared.cloud_provider_thread_id`, and
step 6's Complete analysis under the destination identity replaces the
round-trip proof. `prepared.cloud_cwd` keeps its role as the reviewed
wire-form cwd (now token-form); the step-4 analysis must match it.

### 4.4 Adoption / first contact (`commands.rs:5206-5218`)

The probe comparison composes the same wire transform: parse each line
of the analyzed local slice, `encode_record` (ungated — adoption is a
best-effort fast path; any failure leaves the pair unproven), serialize,
then the existing `redact_jsonl_text` → re-analyze → wire chain →
`adopt_claude_acceptance` (function unchanged: wire-vs-wire state
equality plus the local fold for the twin pin). `observe_claude_local`
gains the binding root + canonical alias parameters to construct the
mapper — the Codex `adopt_codex_acceptance` signature precedent.

### 4.5 Declaration stamp (decision D1)

Codex declares the path map in-band (`__mallard_path_map__` marker in
the authoritative meta). Claude wire is pure provider JSONL with no meta
record, so in-band is out (it would corrupt replay/resume).

**Recommended**: mirror the redaction stamp — add
`ConversationThread.wire_path_map: Option<ConversationWirePathMap>`
(`{ codec: "mallard.pathmap.v1" }`), required when `provider == claude`
(same validation pattern as `wire_redaction`, `domain lib.rs:3059/3177`),
mirrored in `src/types.ts`, with a typed review blocker on a foreign
codec value. Field addition only; schema numbers unchanged; the wipe
covers the transition.

Rejected alternatives: bumping the `CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1`
string (label is pinned across many tests; a semantic bump hidden in a
label rename gives worse errors than a typed stamp mismatch), and no
declaration at all (fails closed at the pull verify gate, but with a
collision-shaped message rather than a version-shaped one).

### 4.6 Gates summary

| Gate | Site | Rule |
| --- | --- | --- |
| Root grammar | push + pull entry | `validate_pathmap_root`; Windows/non-POSIX binding roots block before parsing |
| Span gate | push, per record | `decode(wire) == canonical_source` (`encode_record_gated`) |
| Reserved token | push, per record | literal `__mallard` namespace in source content ⇒ typed block |
| Redact fixpoint | push + pull | `certify_wire_document` (unchanged) |
| Wire self-check | push (post-redact) + pull (pre-decode) | `verify_wire_record`: `encode(decode(w)) == w`; collision detector |
| Cross-form layout | push | wire fold partitions exactly like the local fold (unchanged check, now covering pathenc too) |
| Wire refold | pull, before decode | wire chain refolds to the accepted cloud pin |
| Local Complete | pull, after decode | decoded bytes re-analyze Complete under destination identity |
| Malformed line | push/pull, per line | typed error naming the ordinal (defensive; unreachable behind Complete) |

## 5. Flow deltas at a glance

- **Push**: `capture → seed local fold → [pathenc → redact → self-check]
  → wire fold → publish`; checkpoint `accepted` = wire tail,
  `accepted_local_tail` = seeded local (unchanged shape, now genuinely
  divergent content forms).
- **Pull**: `certify → verify wire vs pin → [decode + restamp] → local
  re-analysis → install`; checkpoint pins = (cloud wire tail, genuine
  local fold).
- **Refresh / watcher / candidate pins**: zero change — the mapper runs
  only at push-build, pull-apply, and the adoption probe.
- **Repush after pull**: `encode_B(decode_B(w)) == w` by the verify
  gate, so an unchanged thread republishes byte-stable wire; extended
  threads extend it — the A→B→A lifecycle holds as it does for Codex.

## 6. Cutover (delete in the same change)

- `rewrite_claude_transcript` + `rewrite_matching_string`/
  `validate_rewrite_value` (if claude-only) in
  `providers/claude/codec.rs`; the Claude `SessionAdapter::rewrite`
  implementation becomes a typed error (Codex precedent). The trait
  method survives only for the non-cursor provider branch
  (`commands.rs:11455`, test-provider territory).
- The pinned test `rewrite_touches_only_declared_structural_fields`
  (`codec.rs:1455`) — replaced by a wire round-trip fixture test
  (Codex precedent:
  `committed_codex_fixture_proves_wire_round_trip...`). The new
  assertion is the exact inverse: embedded content paths under the
  source root are ABSENT from the pulled transcript.
- The pull relabel block (`commands.rs:11429-11435`).
- No migration: cloud rows without `wire_path_map` (and pre-token wire
  objects) simply fail typed review/pull gates; the user wipes
  `~/.mallard` + cloud before retesting.

## 7. Non-goals and residuals

- **Sidecars** (`agent-*` transcripts, todos, file-history): shipped
  scrubbed since redaction, never materialized on pull (§15.6 gap
  there). Their source-machine paths stay verbatim in the cloud form —
  cross-machine sidecar diff noise, accepted. Revisit when sidecar
  materialization lands.
- **`display_title`**: first-user-message fallback may embed source
  paths; cosmetic, both providers, unchanged (shared residual with
  Codex).
- **Out-of-root paths** (`$HOME`, other projects, system): ship
  byte-identical — the hard invariant. Foreign structural cwds included.
- **Collision residual** (inherited from Codex): destination-root text
  appearing verbatim in *out-of-root* wire content fails the pull
  verify gate closed; universal portability is deliberately not
  guaranteed.
- **Same-machine Claude "project moved" repair**: no Claude twin of
  `remap_codex_conversation_paths` exists today and none is added here.
- **Windows**: blocked at the root gate, as for Codex v1.

## 8. Performance

Refresh, watcher, and byte-pin caches: untouched (the plan's only hot
path). Push build and pull apply already run multiple full passes per
transcript (redact + certify + two analyses + fold); pathenc adds one
parse/walk/serialize per line under an Aho-Corasick scan — the Codex
implementation measured this as noise, and the Claude corpus is smaller
(710 MB total, ~5 MB median file). The adoption probe already redacts
and re-analyzes the full slice; one more per-line pass is the same
order.

## 9. Tests

1. **Mapper facade units** (`session_path_map.rs`): line encode/decode
   round-trips — subdir cwd suffix, foreign cwd verbatim, tilde and
   `file://` spellings, reserved-token block, malformed-line typed
   error, key-position defense (corpus shows zero, keep the walk
   uniform).
2. **Wire round-trip fixture** (replaces the pinned structural test):
   committed two-root fixture where prose, `input.file_path`,
   `toolUseResult.filePath`, `attachment.filename`, and a
   `file-history-snapshot` path all remap; assert the source root is
   byte-absent from the decoded output and the wire is a
   `verify_wire_record` fixpoint.
3. **Command-level A→B→A lifecycle** (mirror of the Codex test): push
   from root A; pull into root B asserting (a) no source-root
   occurrence in installed bytes, (b) cwds = B root with a subdir case
   preserved, (c) cursor seeded, (d) genuine `accepted_local_tail` ≠
   wire pin states; extend at B; republish (wire continuity); pull back
   at A.
4. **Session Detail regression** (the reported bug): after the B pull,
   the Write-tool `tool_summary` classifies under B's root and shows the
   relative path — source-root spelling absent from every timeline
   event.
5. **Redaction composition**: a record carrying both a secret and an
   in-root path publishes `<label>` + token together; pull installs
   placeholder + B-root together; both self-checks pass.
6. **Adoption cross-root**: first contact on B with a local copy under
   B's root adopts against the tokenized wire pin.
7. **Stamp**: claude row without/with foreign `wire_path_map` blocks
   review both directions (mirror the redaction blocker test).
8. **Live aimock e2e extension (optional stage)**: a `pulled_remap`
   scenario — push under root A, pull under root B, real-CLI `--resume`
   of the remapped transcript with a follow-up turn.

## 10. Touch list

- `crates/mallard-infrastructure/src/codex_path_map.rs` →
  `session_path_map.rs`: rename + two line-level facades (+ lib.rs decl,
  import sweep across builder/commands/tests).
- `crates/mallard-infrastructure/src/providers/claude/codec.rs`: delete
  rewrite band + pinned test; add the fixture test.
- `crates/mallard-infrastructure/src/providers/claude/mod.rs` (adapter):
  `rewrite()` → typed error.
- `crates/mallard-domain/src/lib.rs`: `ConversationWirePathMap` +
  `wire_path_map` field + required-for-claude validation + blocker
  variant (decision D1); `src/types.ts` mirror.
- `src-tauri/src/project_sync/commands.rs`: push branch (`~9007`) —
  root gate + pathenc + self-check; pull branch (`~11377`) — reorder
  verification, decode pass, genuine local fold, delete relabel;
  adoption sites (`~5206`, `4995` callers) — mapper threading through
  `observe_claude_local`; audit remaining `rewrite` callers.
- Tests: `sync_tests/` claude flows, codec tests, aimock scenario
  (optional), frontend none.
- Docs: `CURRENT_METADATA_LAYOUT.md` (claude wire declaration + stamp);
  this plan's status line on completion.

## 11. Open questions — all resolved at implementation (2026-08-21)

- **D1** (§4.5): **RESOLVED — out-of-band stamp implemented.**
  `ConversationWirePathMap { codec }` on `ConversationThread`, required
  for claude rows and *forbidden* elsewhere (Codex declares in-band);
  `ConversationBlocker::PathMapCodecMismatch` blocks both review
  directions on a foreign codec, and typed
  `path_map_codec_mismatch` gates guard push (previous-row stamp) and
  pull (prepared-row stamp). `src/types.ts` union mirrored.
- **D2**: **RESOLVED — permissive.** Zero-token wires decode cleanly; the
  refold + per-line self-check gates bind content to the pin.
- **D3**: **RESOLVED — no new aimock scenario in this change.** The
  committed command-level lifecycle test (§12) covers the two-root hop;
  real-CLI `--resume` of a remapped transcript rides the user's standing
  wipe-based live retest. A dedicated `pulled_remap` scenario stays
  available as a named follow-up if live coverage is wanted.

## 12. Implementation notes (deltas from the proposal)

- **Subdir cwds are out of scope** (user, 2026-08-21, mid-implementation:
  "don't worry about subdir — if codex/claude launches at the subdir,
  treat the subdir as another project"). The engine's staged-payload gate
  (`conversation_materialization.rs`, `verify_provider_payload`) keeps
  requiring the installed leaf cwd to equal the binding root or its
  canonical alias *exactly*; a subdir or foreign leaf cwd fails that
  per-action gate closed. The shared mapper still tokenizes/decodes
  subdir *content* spellings uniformly (mid-file subdir records remap as
  content — pinned by the codec fixture test); only the leaf-identity
  claim is strict.
- **The structural rewrite moved, not vanished**: `rewrite_claude_transcript`
  and its helpers left the claude codec, but the non-cursor pull branch
  (test-provider territory) still exercises `SessionAdapter::rewrite`, so
  the logic lives on as `structural_session_rewrite` inside
  `providers/test_provider.rs` — which is `#[cfg(any(test,
  feature = "test-support"))]`, so it is gone from production binaries.
  The claude adapter's `rewrite()` is a typed error (Codex precedent).
  `lineage_digests` moved into the codec's test module.
- **Facades preserve unmatched bytes**: `encode_jsonl_line_gated` returns
  the *original line* when no span matched (mirroring
  `redact_jsonl_text`'s untouched-line byte-identity) instead of always
  re-serializing; both passes use the `split('\n')` convention with
  blank-line passthrough so document shape survives. The pull decode
  serializes every line once (the old rewrite did too).
- **Push adds a size floor**: the pathenc output is checked against
  `MAX_CONVERSATION_FILE_BYTES` before redaction, failing at push
  instead of at every future pull (Codex builder parity).
- **Pull identity checks**: step 4's wire analysis is compared against
  `prepared.cloud_provider_thread_id` *and* `prepared.cloud_cwd`
  (token-form); step 6 requires the decoded analysis Complete, the
  destination session id, and wire/local chain-length equality. The
  `accepted_local_tail` relabel is deleted; the genuine local fold and
  local-form digest pins feed the materialization operation, matching
  the engine's staged-bytes verification comment verbatim.
- **Adoption threading**: `observe_claude_local` gained
  `project_root`/`canonical_project_root` parameters (the
  `observe_codex_local` signature precedent); the probe composes
  ungated `encode_jsonl_line_gated` per line before the existing redact →
  re-analyze → `adopt_claude_acceptance` chain, all failures leaving the
  pair unproven.
- **Tests landed**: mapper facade units + collision/typed-error cases
  (`session_path_map.rs`); `claude_wire_fixture_remaps_content_paths_and_round_trips`
  (codec, replaces the pinned structural test with its inverse);
  `claude_round_trip_scrubs_secrets_end_to_end` extended into the full
  A→B→A path lifecycle (store token-only, installed-bytes respelling,
  Session Detail `affected_paths == ["readme"]` regression, secrets and
  paths on one wire); `claude_first_contact_adoption_survives_redaction`
  extended with an in-root content path;
  `foreign_path_map_codec_blocks_review` (stamp blocker);
  `wire_path_map_stamp_is_required_for_claude_and_forbidden_elsewhere`
  (domain validation).
- **In passing**: a pre-existing `clippy::collapsible_if` (new in
  rust-clippy 1.96) in `credential_redaction.rs` was collapsed to keep
  the `-D warnings` gate green; `docs/CURRENT_METADATA_LAYOUT.md`'s
  conversation-row listing now names `wire_redaction` and
  `wire_path_map`.

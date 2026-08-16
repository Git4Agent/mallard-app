# PLAN: In-cwd path remap across machines — canonical wire + dual-pin identity

Status: implemented (2026-08-15). One deliberate deltas from the §6.1
sketch: `StandaloneBuildMode::Decode` also carries
`canonical_target_root`, so the pull gate re-encodes under exactly the
patterns a future push from the pulling machine will use (target-alias
collisions surface at pull, not at that later push).

One plan for the whole problem: after Pull, every path that lives under the
**project working directory** — structural fields, sandbox/permission
entries, display events, *and canonical message content* — reads as the
pulling machine's paths, while refresh/push/pull still recognize the thread
as one conversation across machines and hops. Paths **outside** the project
working directory are never remapped, anywhere, by construction (§3.1).

Supersedes `PLAN_CODEX_PATH_INVARIANT_CONTENT_REMAP.md` (the
normalize-before-hash pole, rejected after evaluation: it kept the resolver
untouched but required bit-identical matcher parity across machines at every
comparison, forever — fragility at compare time). This design places the
transform at push/pull boundaries with pins bridging in between: encoder
bugs degrade to visible content bugs provable locally, never to false
divergence.

Compatibility (user-set): **clean cutover; no migration, no legacy
execution path, schema constants unchanged** (`BUNDLE_SCHEMA` stays 1).
Local state (`~/.mallard`) and cloud data are wiped for retesting; there are
no customers, so no fail-closed versioning choreography for old data is
designed here.

## 1. Problem

- Machine A, project at `/homeA/projact_alpha`; rollout content says
  `created /homeA/projact_alpha/readme`.
- Machine B binds the same project at `/homeB/projact_beta`; after Pull the
  materialized rollout must say `created /homeB/projact_beta/readme` —
  including inside canonical `response_item` records.
- Sync must still classify A's thread, the cloud object, and B's rewritten
  thread as the same conversation: Sync / LocalOutstanding /
  CloudOutstanding / Divergent must keep working from manifest-only
  refreshes, cursor-riding observation, and the existing journaled engines.

Today's `Reidentify` rewrites only `session_meta`/`turn_context`
cwd *fields*. Everything else is frozen because the canonical chain
fingerprints full record content and every gate (build re-analysis, Pull
refold, classifier equality) proves byte-level identity.

## 2. Corpus facts (2026-08-14 scan: 1,267 rollouts, 10.07 GB, aggregate-only)

Every file contains in-cwd absolute paths (~519k occurrences). Spellings:
absolute ≈ 99.9%, `file://` URIs ≈ 375 (exec `cwd`, tool outputs; percent
encoding per `path_to_file_uri`), tilde ≈ 73. 22 sessions (1.7%) carry
multiple structural cwds; 2 have cwds outside `$HOME`. Field inventory that
this plan must cover:

| Class | Where paths live |
|---|---|
| `session_meta` | `cwd` (~5k) |
| `turn_context` | `cwd` (~12k), `permission_profile.file_system.entries[].path.path` + `file_system_sandbox_policy.entries[].path.path` (~103k), `workspace_roots[]` (~10k), `sandbox_policy.writable_roots[]` |
| `event_msg:thread_settings_applied` | `thread_settings.cwd`, `thread_settings.permission_profile...path` (~44k) |
| `event_msg:*` display events | `user_message.message`, `agent_message.message`, `task_complete.last_agent_message`, `exec_command_end.{cwd,command[],parsed_cmd[],aggregated_output}`, `mcp_tool_call_end` invocation/result trees (~60k) |
| `event_msg:patch_apply_end` | `stdout` and **`changes.<absolute-path>` as JSON object keys** (~57k) |
| `world_state` | `state.environments.filesystem`, `.environments.local.cwd`, `.agents_md.directory`, `.permissions.approved_command_prefixes[][]` (~16k) |
| `compacted` | `replacement_history[].content[].text`, `message` (~16k) |
| **canonical** `response_item` | `custom_tool_call.input` (63k, `exec`/`apply_patch`), `function_call.arguments` (38k), `function_call_output`/`custom_tool_call_output` (~43k), message text user/assistant/developer (~50k incl. `<environment_context>` blocks) |

Under this plan the canonical/ignored distinction stops mattering: **every
record passes through one encoder/decoder.**

## 3. Architecture

### 3.1 Canonical wire form

Cloud conversation objects store a **machine-neutral** encoding: every
in-base path occurrence is replaced by an indexed token that is itself an
absolute-path-shaped string, so all existing structural validation
(`parse_session_meta`, `structural_cwd`, `resolve_codex_root` over staged
cloud bytes) keeps working unmodified:

```
path context:   /__mallard_root__/frontend/readme
URI context:    file:///__mallard_root__/read%20me
```

**One immutable base — the project root — and one token.** There is no
base table to derive, no indices, and no per-base tags. URI-ness is
*contextual*: the same token preceded by `file://` is the URI wire form.
Two earlier shapes were tried and rejected by review:

- *Spelling tags* (`p`/`t`) — decode was non-injective (a `p`-tagged base
  onto an escape-free target, or a `t`-tagged base onto an out-of-home
  target, collapses into the untagged form; re-encode cannot restore the
  tag). Canonical rule instead: **same semantic path form ⇒ same
  canonical wire form ⇒ same canonical target spelling.**
- *A dynamic structural-cwd base table* — three defects: a subdir-only
  session (`cwd = root/frontend`) derived `bases = ["frontend"]` and left
  root-level paths (`root/README.md`) unremapped, violating the core
  requirement; a base added later changed leftmost-longest matches inside
  already-accepted wire prefixes (append instability); and first-contact
  adoption could infer a different table than the one the cloud artifact
  was encoded with (hidden state). A single root base covers every
  in-root path (subdir cwds are just suffixes: `session_meta.cwd =
  root/frontend` encodes as `/__mallard_root__/frontend`), never changes
  an old token choice, and leaves first contact nothing to infer.

The root is **`binding.project_root`** — the encoder and decoder each use
their own machine's binding; nothing is derived from transcript content.
The matcher additionally accepts the binding's `canonical_project_root`
spelling when it differs (macOS `/var` ↔ `/private/var` aliasing), both
canonicalizing to the one token. Root validation: absolute, clean, ≥ 2
components, 6..=4096 bytes, no `"`/`\`/control bytes; violation ⇒ typed
build/decode block.

The artifact's authoritative `session_meta` payload still carries a
mallard-owned marker (tolerated as an unknown field by all parsers, which
are `serde_json::Value`-based) — now purely a codec declaration:

```json
"__mallard_path_map__": { "codec": "mallard.pathmap.v1" }
```

It carries **no paths at all** — consistent with the repository contract
that absolute checkout paths stay on the machine (README §"stays local").
The only machine paths that ship are out-of-root occurrences the
transcript itself always contained (§9).

- **Hard invariant (out-of-root immutability): only paths under the
  project working directory are ever encoded, decoded, or remapped.**
  Every other path — foreign structural cwds from multi-cwd sessions,
  `$HOME` paths outside the project, other projects, system paths — ships
  on the wire and materializes on every machine byte-identical. A
  structural cwd outside the root registers nothing: its cwd field and
  every content mention of it ship verbatim. "Under" is a
  component-boundary string prefix check on the recorded values — no
  filesystem canonicalization beyond the declared canonical-root alias.
- `codec` is the single wire version. Decode of an unknown codec ⇒ typed
  block ("update Mallard"). Evolution = bump + wipe (no dual readers, per
  the compatibility rule above).
- `__mallard_path_map__` is **wire-only**: decode strips it, so
  materialized local rollouts stay codex-native. A later push re-emits
  the constant marker; the encoder's patterns come from its own
  `binding.project_root` (+ canonical alias).

The wire object is **not** codex-consumable (tokens instead of paths);
manual recovery requires a Mallard decode. Accepted trade for a neutral
form every hop can decode without knowing any other machine's layout.

### 3.2 Dual-pin identity (how the tail hash keeps working)

Fingerprints and folds are **unchanged v1 mechanics** — plain
`semantic_record_fingerprint` over record bytes, `TailAccumulator`,
`compute_portable_conversation_tail`. What changes is *which bytes* each
tail is computed over, and what gets compared to what:

There are three codec layers, and only the middle one changes. The
provider lineage analyzer keeps its key (`codex.lineage.v1`, thread docs'
`lineage_codec`) — it is genuinely unchanged. The **portable-tail
envelope codec** (`PortableConversationTail.codec`) is where the form
split lives, as two domain constants:

- **Local tail (`mallard-codex-portable-tail-v1`, the existing
  constant)** — folds the machine's real rollout bytes. Exactly today's
  observation path: cursor riding, probe windows, watcher caches,
  byte-pin caches all untouched.
- **Wire tail (`mallard-codex-portable-tail-wire-v1`, new constant)** —
  folds the token-form artifact bytes. Computed only where artifacts
  already get fully analyzed: push build and pull staging.
  `validate_codex_wire` (cloud `ConversationThread` docs) pins this
  constant; checkpoint `accepted_local_tail`/`observed_tail` pin the
  local constant. Distinct labels make any accidental cross-form
  comparison fail structurally (codec inequality ⇒ Unknown ⇒ Blocked)
  instead of silently mis-comparing.

The third layer, `mallard.pathmap.v1`, versions only the wire path-map
table (§3.1).

`ConversationCheckpoint` becomes dual-pinned at acceptance:

- `accepted: AcceptedConversationVersion` — the **wire** pin, as today
  (`artifact_sha256`, accepted-at coordinates, `portable_tail` = wire
  tail), plus two new fields pinned atomically at the same event:
  `accepted_local_tail: PortableConversationTail` (the local-form twin).
  With the single immutable root base (§3.1) there is no table to pin:
  every machine's patterns derive from its own `binding.project_root`,
  and token choices can never drift between pushes.
- `observed_tail` — unchanged: the local-form fold riding the cursor
  forward **from `accepted_local_tail`'s saved accumulator state**. Cursor
  continuity plus the probe window is the standing proof that the local
  prefix still equals the accepted prefix — no refolds at refresh time.
  Checkpoint validation adds: `accepted.portable_tail` carries the wire
  codec, `accepted_local_tail`/`observed_tail` the local codec, layouts
  agree across all three (partition and counts are form-independent —
  records map one-to-one).

In the sketch's vocabulary: `pin.hash.original` ≡ `accepted.portable_tail`,
`pin.hash.remapped` ≡ `accepted_local_tail`, `local.hash.remapped` ≡
`observed_tail`, `cloudHash` ≡ the manifest's wire tail.

**Form ledger (load-bearing rule).** Because wire and local bytes differ,
*every* persisted digest, tail, and witness belongs to exactly one form,
and no comparison ever crosses forms. The dual tail pins are the visible
half of this rule; these are the full assignments:

| Wire-form values (compare only to wire) | Local-form values (compare only to local) |
|---|---|
| cloud `ConversationThread` `portable_tail`, `lineage_digest`/`semantic_digest`, `artifact_sha256` | local thread rows' digests (from local capture analysis) |
| checkpoint `accepted.portable_tail` | checkpoint `accepted_local_tail`, `observed_tail` |
| publication op `tail_after`; recovery's content witness | checkpoint `prefix_witness` (§6.1) |
| staging-time refold of downloaded bytes | materialization op `pinned_lineage_digest`/`pinned_semantic_digest` (verified against staged **local** bytes, §6.2); probe windows, byte-pin caches |

Any surviving code path that classified via direct local-vs-cloud digest
or tail equality (the old "wire bytes == local bytes" era shortcuts) is a
cross-form comparison under this design and must be replaced by the
pin-relative reading — deleting it is part of §8's cutover, not an
optional cleanup.

## 4. Encoder / decoder

One shared matcher module drives both directions and the same-machine
repair flow.

### 4.1 Patterns and boundaries

For the root `r` (= the encoder's `binding.project_root`; its
`canonical_project_root` alias is matched identically when it differs),
match patterns and their canonical wire form:

| Source spelling matched | Wire form emitted |
|---|---|
| raw `r` (either alias spelling) | `/__mallard_root__` |
| `~` + (`r` − `$HOME`), when `r` is under the encoding machine's home | `/__mallard_root__` (tilde collapsed) |
| `file://r` (legacy unescaped, either alias) | `file:///__mallard_root__` |
| `file://` + percent-encoded `r` (`path_to_file_uri`'s escape set; distinct pattern only when it differs) | `file:///__mallard_root__` |

The complete-URI patterns are matched with priority over the embedded
bare-root at the same region (leftmost-longest gives this for free: the
URI match starts seven bytes earlier). Tilde spellings are matched (they
are in-cwd paths and must remap — a verbatim `~/...` from machine A would
silently resolve against machine B's home) but their spelling is not
preserved: they canonicalize with the rest. The encoder always runs on
the machine that owns the spelling, so `$HOME` is knowable; there is no
cross-machine parity requirement, only the local round-trip gates (§4.4).

A match counts only when the bytes before and after are outside
`[A-Za-z0-9._-]` (string edge counts as a boundary): rejects
`/root/alphabet`, `/root/alpha-2`, `/host/homeA/...`; accepts `path:42`,
quoted/space-delimited paths. Matching is leftmost-longest (Aho–Corasick;
the crate is already in `Cargo.lock` transitively — add as a direct
dependency). A false positive is a *content* bug on one machine's
materialization, never an identity break — the pins carry whatever bytes
were actually produced.

### 4.2 Scope

Patterns are built **only** from the project root (§3.1) — the matcher is
physically incapable of touching an out-of-root path. Sandbox and
permission entries pointing outside the project (`/tmp`, home-scoped
grants) therefore ship and materialize verbatim, by construction.

- **Structural cwd fields** (`session_meta.cwd` + variants,
  `turn_context.cwd` + variants): in-root cwds encode as the token plus
  their subdirectory suffix (`root/frontend` → `/__mallard_root__/frontend`)
  and decode to the target root plus the same suffix — subdirectory
  sessions survive without any table entry. A foreign cwd field is left
  exactly as written (multi-cwd sessions stay truthful).
- **Everything else, every record type**: all string values and all object
  keys pass through the matcher (`patch_apply_end.changes.<path>` keys ride
  the key rewrite). `serde_json`'s default `Map` (BTreeMap) re-sorts keys
  deterministically on every re-serialize — encode and decode both go
  parse → transform → re-serialize, so key order is stable within each form
  and the round-trip gate below is byte-exact. (The canonical-record
  key-order hazard of the superseded plan does not exist: wire fingerprints
  are computed over wire bytes, local over local bytes; forms are never
  cross-hashed.)

### 4.3 Decode mapping

The decoder's target is its own `binding.project_root` (`R`). Canonical
expansion — one deterministic target spelling per wire context,
independent of the target root's properties:

- `/__mallard_root__` in path context ⇒ `R` raw (never tilde, even when
  `R` is under the puller's home); suffix bytes verbatim, so an in-root
  subdir cwd or file path keeps its relative remainder untouched;
- `file:///__mallard_root__` ⇒ `file://` + percent-encoded `R` + suffix
  verbatim. The **root portion** of the URI is always validly escaped —
  suffix bytes are preserved as written, so a wire suffix that was
  unescaped stays unescaped (the promise is scoped to the root prefix,
  not the whole URI; §4.5's projection-emitted URIs carry escaped
  suffixes and stay fully valid end to end).

Decode is **injective by construction**: the two contexts produce target
spellings distinguishable in every case (`file://` prefix). Injectivity
is what makes the §4.4 re-encode gate passable on arbitrary targets —
including targets outside the puller's home and targets needing no
percent escapes. One guard (typed error): `R` must not itself contain
the reserved token namespace.

### 4.4 Round-trip gates and token collisions

There is **no escaping machinery**. The reserved namespace
`/__mallard_root__` appearing literally in source content, or any record
that fails its round-trip gate, blocks the build/pull with a typed error
naming the record.

The build has two independent transform layers, and each gate proves
**only its own layer** — the gates run *before* the other layer's rewrite,
never across it:

1. **Path layer** (the matcher): byte-exact and involutive by proof.
2. **Identity layer** (fresh root id, head-meta fold, `item_completed`
   `thread_id` stamps, ordinals, the wire-only `__mallard_path_map__`
   field): *not* byte-preserved across a pull by design, and verified by
   the existing structural checks — re-analysis pinning
   `source_thread_id == output root id`, `validate_projection_fixpoint`,
   `validate_standalone_record_layout`. No new machinery.

Gate placement:

- **Push build** — per record, after path-encode and **before** the
  identity stage (head fold, projection insertion, ordinal assignment).
  Encode is a *canonicalizing projection* (§4.1: tilde and unescaped-URI
  spellings collapse), so `decode(encode(r)) == r` does not hold for
  non-canonical source spellings and is **not** the gate. Two checks
  replace it:
  1. **Span-bounded structural diff**: compare `r` and
     `decode_paths(encode_paths(r), identity map)` as parsed JSON trees
     walked in parallel — every string value and key must be byte-equal
     except where a recorded matcher span permits exactly the
     deterministic canonical respelling of that occurrence. Structural
     (not flat-byte) comparison is required because canonicalizing an
     object *key* (tilde → absolute) can change its BTreeMap sort
     position, legally moving untouched bytes in the serialized form.
     Proves encode invented nothing and touched nothing beyond its
     matches.
  2. **Wire self-check**: `encode_paths(decode_paths(w, identity map),
     identity map) == w` per wire record, run over the **complete
     emitted body — projection-inserted records included** (they need no
     source counterpart; the check is wire-only). This is the same
     encode∘decode = id gate the pull side runs, guaranteed passable by
     §4.3 injectivity, and it is what pins §4.5's guarantee that
     projection output lands in a declared wire form — any future
     projection change that minted an undeclared spelling would fail
     here, at push, not at some later pull.
  The folded head is covered by the identity-layer checks, not these
  gates; the span diff alone skips projection-inserted records (no
  source to diff against).
- **Pull apply** — per record over the **staged wire body, before
  reidentification**: `encode_paths(decode_paths(w, map), map) ==
  normalize(w)` (head's `__mallard_path_map__` excluded — it is wire-only).
  Only after this gate passes does the identity stage assign the fresh
  provider id, fold the head, strip the path-map field, and re-serialize;
  its correctness is proven by the identity-layer checks above.

Ordering matters: comparing *final* materialized bytes against the wire
would fail on every pull — the fresh root id, every `item_completed`
`thread_id`, and the stripped `__mallard_path_map__` field legitimately
differ. Those fields are all chain-Ignored, so the honest-pin statement
is: the **path layer** round-trips byte-exactly, and a future push from
this machine is **tail- and digest-equal** to what classification expects —
byte-different only in the identity layer, which no comparison ever reads
(artifact_sha256 is per-artifact transport integrity, never compared
across artifacts).

The pull gate is also the dynamic proof of §4.3's guards: a mapped base
that created or destroyed a boundary-valid match would break the
re-encode equality on the exact record involved.

These gates replace the superseded plan's "fingerprint equality proves
the remap" role, and are stronger (byte equality, not hash equality).

### 4.5 Interplay with the paginated item projection

Push build order: collect flattened records → encode → complete the
`item_completed` projection → serialize with fresh ordinals. Projection
runs on token-form inputs, and this is exactly why the wire grammar is
**one token with contextual URI-ness** (§3.1): when projection converts a
token-form exec `cwd` through `path_to_file_uri`, it produces
`file:///__mallard_root__/` + percent-escaped suffix — which *is* the
declared URI wire context, by construction. (Under the earlier
`0u`-tagged grammar this same call minted an undeclared third spelling —
`file://` + bare path token — that decoded and re-encoded into the
tagged form and blocked the pull gate on any transcript with legacy exec
display events. The unified grammar makes the hazard unrepresentable,
and §4.4's wire self-check over projection-inserted records is the
regression tripwire.) The escaped suffix stays valid through decode —
§4.3 escapes the root portion and preserves suffix bytes — so
projection-emitted URIs remain fully valid end to end.
`validate_projection_fixpoint` runs unchanged on the wire body; decode
preserves event/item consistency (both sides of each pair decode through
the same matcher), so re-projection of decoded bytes is also a fixpoint —
asserted by the existing check at decode build.

## 5. Classification (pin-relative)

`compare_conversation_tail(L, C)`'s direct local-vs-cloud equality is
meaningless across forms and is **deleted**. Its replacement keeps the same
output type (`TailClassification`) and the same downstream decision tables,
journal phases, and engines — the blast radius is the relation function,
its evidence gates, and their tests:

```
classify(P_wire, P_local, L_observed, C_manifest) -> TailClassification
```

Core rows (the reviewed sketch, formalized; all tail comparisons are
field-wise including codec, so W-vs-W and L-vs-L only):

| Evidence | Relation |
|---|---|
| `L == P_local` ∧ `C == P_wire` | Sync |
| `C == P_wire` ∧ `L.end > P_local.end` ∧ cursor-continuous from `P_local` | LocalOutstanding |
| `L == P_local` ∧ `C.end > P_wire.end` | CloudOutstanding (cloud prefix trusted at classify; re-proven at apply) |
| `C != P_wire` ∧ `L != P_local` | Divergent |
| `C != P_wire` ∧ `C.end <= P_wire.end` | Divergent (cloud rewrite/containment) |
| `L != P_local` ∧ `L.end <= P_local.end`, or probe/cursor invalid | Divergent / Invalid evidence per today's rules |
| no checkpoint | first contact (§6.4) |
| missing sides | MissingCloud / MissingLocal / MissingBoth as today |
| Detached physical state | as today (`physical_detached` blocks Push) |

Notes:

- "Cursor-continuous" is not a refold: `observed_tail` only ever advances
  from `accepted_local_tail`'s accumulator state under probe-window guard,
  so `L.end > P_local.end` with valid observation *is* the prefix proof.
  Any prefix mutation surfaces as `Invalid(replaced/truncated)` at
  observation, exactly as today.
- CloudOutstanding's prefix assumption is re-proven at Pull apply by
  refolding the downloaded wire chain against `P_wire`'s `(end, state)`
  before anything is journaled — the existing
  `apply_prepared_pull_conversation` check, unchanged in shape.
- Divergent keeps flowing into the existing KeepBoth machinery.

**Review-to-apply contract.** Apply re-runs this classifier over the
action's pinned evidence and requires the planned relation — so the
serialized action must carry all four inputs. Today's
`ConversationPlanAction` pins three (`pinned_accepted_tail` = P_wire,
`pinned_local_tail` = L, `pinned_cloud_tail` = C); `pinned_local_tail`
is the *observed* value and cannot double as the accepted local twin
(review can legitimately see `L ≠ P_local` — that is the LocalOutstanding
row). The contract gains, in both the Rust DTO and `src/types.ts`:

- `pinned_accepted_local_tail: Option<PortableConversationTail>` — P_local
  as reviewed;
- the reviewed checkpoint `generation` — the build's stale check becomes
  pin-vs-checkpoint equality (any checkpoint advance between review and
  apply ⇒ typed stale error), not a trusting read of live state. (An
  earlier draft also pinned a `wire_bases` table; the single immutable
  root base of §3.1 leaves nothing table-shaped to pin.)

`FORMAL_VERIFICATION_PORTABLE_TAIL_CONFLICT_RESOLVER.md` and the
relation-matrix suite are rewritten against this table (task, not
afterthought: it is the main cost of this design and was priced in when
choosing it).

## 6. Flows

### 6.1 Push

`StandaloneBuildMode::{PreserveIdentity, Reidentify}` are **replaced** by:

```rust
StandaloneBuildMode::Encode {                  // Push objects
    root: String,             // binding.project_root
    canonical_root: Option<String>,            // alias spelling when it differs
}
StandaloneBuildMode::Decode {                  // Pull targets
    root_provider_thread_id: String,
    target_root: String,      // binding.project_root on the pulling machine
}
```

Build (Encode): resolve → verify `seeded.observed_tail == pinned_local_tail`
(unchanged, local form) → encode under the binding root (a constant
matcher — nothing accumulates across pushes, so re-encoding old records
always reproduces the accepted wire prefix byte-for-byte) → projection →
serialize → §4.4 gates → wire tail from re-analysis of output.
Verification deltas: chain **message IDs and count** must equal the
resolved chain's (IDs are never touched); fingerprint/digest equality
across forms is *not* required (by design); the artifact carries wire
digests. On CAS success the checkpoint writes `accepted` (wire tail +
artifact sha) and `accepted_local_tail` (the pinned local tail) in one
mutation; `observed` continues from it.

Two form-sensitive push details (both instances of the §3.2 ledger rule):

- **Prefix witness is local-form.** Witness *positions* are
  form-independent (records map one-to-one), but the witness digests fold
  semantic fingerprints, which are content- and therefore form-dependent —
  a `FixedPaginatedTail` prefix containing a remapped path has different
  wire and local witnesses. The stored checkpoint witness is declared
  **local-form**: the push guard (stored witness vs the full local
  reconstruction's recomputed witness, `build_publish_request`) then
  compares local against local, exactly as it does today on the pushing
  machine. Wire-side prefix integrity needs no second witness — it is
  already covered by `artifact_sha256` and the staging-time wire-tail
  refold.
- **Push-recovery's content witness compares wire against wire.** The
  inspect closure (`snapshot_contains_actions`) that identifies a
  published candidate by content must compare the op's `tail_after` (the
  artifact's wire tail, already journaled) against the manifest thread's
  wire tail — not `pinned_local_tail` against the manifest, which is a
  cross-form comparison that would never match and would strand every
  recovery.

### 6.2 Pull

Stage cloud bytes → `resolve_codex_root` (token-form parses as a normal
rollout) → refold wire chain vs the manifest/action pin (unchanged check) →
validate the `__mallard_path_map__` codec → **§4.4 paths gate on the
staged body (before reidentification)** → Decode build's identity stage
(fresh root id, head fold, path-map field stripped) → install via the
journaled materialization engine (unchanged phases).

Everything persisted from a pull is sourced from the correct form (§3.2
ledger). From the **staged wire chain**: the refold against the pinned
cloud tail — wire-form verification ends there. From the **decode build's
re-analysis of the final materialized local bytes** (one pass, already in
hand — no extra IO): `accepted_local_tail`; the checkpoint
`prefix_witness` (`conversation_prefix_witness` over the *local* chain —
computing it from the staged wire chain would store a wire-form witness
that every later push's local recomputation mismatches, falsely reporting
the pre-start prefix as mutated); and the materialization op's
`pinned_lineage_digest`/`pinned_semantic_digest` (the engine's
`verify_provider_payload` hashes the staged **local** target bytes, so
wire-form pins would abort every install at `TargetStaged`). Checkpoint
seeds: `accepted` = cloud wire pin;
`accepted_local_tail` and `observed` = the local fold above. KeepBoth:
unchanged — the preserved local thread keeps its bytes and gets re-keyed;
the incoming cloud thread decodes as above.

### 6.3 Refresh

**Zero mechanical change.** Discovery, capture, watcher generations, dirty
sets, `refresh_codex_tail` tail windows, candidate byte-pin caches: all
operate on local bytes and local tails exactly as today. Only the
classification call changes per §5. Manifest-only refresh compares
manifest wire tails to `accepted` and local observation to
`accepted_local_tail` — still no transcript reads on the quiet path.

### 6.4 First contact / adoption / reclaim

No checkpoint but local and cloud both present (re-registration, fresh
machine with pre-existing files): full-resolve the local thread, **encode**
it under its own `binding.project_root`, fold the wire tail, compare to
the cloud pin. Equal ⇒ seed the dual pins (adopt), giving back
reclaim-without-repull. Not equal ⇒ Unknown ⇒ Blocked (as today's failed
adoption probe). This replaces `adoption_probe` refold — same trigger
sites, same fail-closed posture, O(file) once. With the single immutable
root base there is no table to infer, so first contact cannot derive
different hidden state than the machine that encoded the artifact — the
encoding is a pure function of (bytes, binding root).

### 6.5 Same-machine repair ("project moved")

`remap_codex_structural_cwds` + `inspect_codex_structural_cwds` are
replaced by the shared module, and the flow is checkpoint-aware. A repair
is an equal-record-count in-place rewrite: the §5 table deliberately
classifies that as Divergent, and cursor observation reports it as
`Invalid(replaced/truncated)` (probe-window and consumed-size mismatch) —
so "repair, then let refresh sort it out" cannot work and is not the
design:

- **Pinned threads: repair ≡ `decode_new(encode_old(x))`** — the pull
  pipeline applied locally. `encode_old` tokenizes under the old root,
  `decode_new` expands under the new root; by §4.3 injectivity the wire
  form is **unchanged**, proven byte-exactly by the gate
  `encode_new(repaired) == encode_old(original)`. The gate doubles as
  the collision detector: if the transcript already contains verbatim
  occurrences of the *new* root (previously out-of-root content),
  re-encode tokenizes them too, equality fails, and repair blocks with a
  typed error naming the colliding record — foreign paths can never be
  silently captured into remap scope and rewritten by a later pull
  elsewhere. Atomically with the file replace, the checkpoint
  **rebases**: `accepted` (the wire pin) is untouched — the
  wire really is identical; `accepted_local_tail`, `observed_tail`, the
  local-form `prefix_witness` (§6.1), the physical mapping (size, probe
  window, resume cursors), and the raw-range pin are re-derived from the
  repaired bytes. With no unpushed suffix the thread lands in **Sync —
  nothing to republish**, because the wire never changed; a genuine
  local suffix rides through as LocalOutstanding.
- **Unpinned threads** (never pushed): no wire to gate against — run an
  explicit collision pre-scan (boundary-valid occurrences of the new
  root ⇒ typed block), then the plain matcher rewrite old→new;
  observation re-baselines through the watcher's drop-and-reseed path.

Values and keys, boundary rules, and byte-level backups as elsewhere; the
out-of-root invariant holds (only occurrences under the old root move).
The audit's stale-cwd detection widens to matcher scope (it currently
sees only cwd fields, so it misses ~113k sandbox/permission entries).

## 7. Performance

| Path | Cost delta |
|---|---|
| Refresh quiet path / watcher / dirty accounting | none (no transform, no new hashes) |
| Observation (cursor riding) | none — local fold unchanged; no per-record matcher anywhere in refresh |
| Push planning (byte-pin caches, manifest-only compare) | none |
| Push build | + encode pass + gates: matcher passes over records already being parsed and re-serialized. The single-root grammar makes the automaton a **constant ~4–6 patterns built once per build** (the dynamic-table design rebuilt up to ~64 patterns mid-file as bases accumulated); the span gate's structural tree walk compares already-parsed `Value`s, saving the flat-diff's extra normalize serialization. Aho–Corasick runs at memory bandwidth; corpus-scale full-scan baseline is ~51 s **in Python** for 10 GB, and a build touches one file |
| Pull apply | + decode pass + re-encode gate on one file, amortized into the existing parse/serialize/refold pipeline |
| First contact adoption | + one encode of one file (replaces one refold of the same file) |
| Repair (user-initiated, one-shot) | three matcher passes over one file + one checkpoint mutation; **removes** the old flow's spurious republish (wire unchanged ⇒ no upload, no CAS) |
| Classification proper (all flows) | tail/pin comparisons remain field-wise equality over small structs — the review fixes (form ledger, local-form witness, four-input action pins) changed the *provenance* of compared values, never the comparison cost; the witness and digest pins are sourced from analysis passes that already run |

This is strictly cheaper at steady state than the superseded plan, which
put a matcher pass inside every fingerprint on every analyze/refresh fold.
Transform cost exists only at sync boundaries, which are one-shot,
journaled, and already O(file).

## 8. Cutover

Schema constants unchanged: `BUNDLE_SCHEMA = 1`, local repository schema
untouched, `codex.lineage.v1` remains the local codec. New shapes
(`accepted_local_tail`, wire `__mallard_path_map__` marker, wire tail
codec label) simply exist from the first build; pre-existing local state
and cloud data are wiped by hand before retesting, so no version gates, no
fail-closed migration paths, and no dual readers are built. Deleted in the
same change (no legacy execution paths):

- `StandaloneBuildMode::PreserveIdentity` / `Reidentify` (replaced by
  `Encode`/`Decode`), the collapse-style `rewrite_turn_context`.
- `rewrite_codex_transcript`'s structural-only body and
  `remap_codex_structural_cwds` (repair adopts the shared matcher).
- `compare_conversation_tail`'s direct L/C equality core (replaced by the
  pin-relative classifier), `adoption_probe` refold plumbing.
- Any verification comparing fingerprints across forms.

## 9. Non-goals and residuals

- **Out-of-root immutability is the rule, not a residual** (§3.1): paths
  outside the project working directory are never remapped, by
  construction. Concretely byte-identical everywhere: `$HOME` paths
  outside the project (~360k occurrences: `~/.codex/...`, other
  projects), foreign structural cwds from the 22/1,267 multi-cwd
  sessions, and out-of-root sandbox/permission grants.
- **Universal portability is not guaranteed — collisions fail closed.**
  If wire content contains a verbatim out-of-root path that equals the
  *target* root + suffix (e.g. Alice's transcript mentions
  `/b/project/x` and Bob's project *is* `/b/project`), decode makes it
  indistinguishable from a decoded token and the §4.4 pull gate blocks
  with a typed error naming the colliding record — safe, never silent,
  but that artifact cannot be materialized at that target root (re-root
  the project or accept non-materialization). The same collision at
  repair time is blocked by §6.5's gate/pre-scan. Immutability +
  transparent materialization + no per-occurrence provenance cannot all
  hold on colliding content; this design keeps immutability and
  no-per-occurrence-provenance, and **sacrifices transparent
  materialization** for that content — the honest accounting of the
  trade.
- **Windows is out of scope for v1 — gated at the operation level,
  before any rollout parsing.** Windows paths violate the root grammar
  (`\` is prohibited; drive/UNC forms, case-insensitivity, and separator
  mixing need their own design), and the wire cwd token
  `/__mallard_root__` is not an absolute Windows path, so
  `resolve_codex_root` over staged wire bytes would fail there. An
  earlier draft gated this as "empty table on Windows," which is not a
  gate at all: a Windows→Windows pull would install the source machine's
  `C:\` cwds verbatim — *worse* than today's shipped `Reidentify`, which
  at least rewrites cwd fields. Since Windows builds ship
  (`release.yml`): conversation **Push and Pull block with a typed
  error on any non-POSIX binding root, checked as a precondition before
  rollout parsing**. POSIX↔POSIX (macOS/Linux) is fully supported;
  Windows semantics are a follow-up plan if ever wanted.
- **Spelling churn on in-root occurrences (accepted cost of the
  canonical wire, §3.1/§4.1):** tilde spellings materialize as absolute
  paths, and legacy unescaped `file://` spellings as percent-escaped
  URIs — including on the origin machine when a pull re-materializes its
  own history. Semantics preserved, bytes respelled; corpus impact ≈ 73
  tilde + a handful of unescaped-URI occurrences.
- Content that names the root with spelling the matcher does not cover
  (e.g. mid-word concatenations, or an ancestor-era root recorded under
  a third alias) ships verbatim; consistent everywhere.
- Claude provider untouched (`PLAN_CLAUDE_PROJECT_PATH_REMAP` stays
  directory-name-level).
- Future subagent transport (`PLAN_CODEX_SUBAGENT_CLOSURE_TRANSPORT`) must
  encode/decode child files with the same module and root.

## 10. Tests

- `wire_round_trip_is_byte_exact_and_decode_is_injective` — encode∘decode
  = id per wire record across all four source spellings collapsing to the
  two canonical forms; values and keys, `patch_apply_end` path-keys,
  nested-JSON `function_call.arguments`, boundary negatives (`alphabet`,
  `alpha-2`, `/host/homeA/...`), key re-sort stability. Injectivity
  regression cases pinned: space-containing source base → escape-free
  target, escape-free source → space-containing target (URI stays valid),
  tilde source → target outside the puller's `$HOME`.
- `push_span_bounded_diff_gate_blocks_out_of_span_mutations` — canonical
  respelling inside spans passes; any byte change outside a recorded span
  fails with the record named.
- `wire_objects_parse_and_refold_as_ordinary_rollouts` — staged token-form
  bytes through `resolve_codex_root`; wire tail refold vs pinned state.
- `reserved_namespace_in_content_blocks_with_a_typed_error` (both gates).
- `pull_paths_gate_runs_on_wire_form_before_reidentification` — a normal
  pull (fresh root id, `item_completed` stamps, stripped path-map field)
  installs cleanly, proving the gate never compares across the identity
  layer; a mapped base that breaks re-encode equality still blocks with
  the record named.
- `pulled_fixed_tail_prefix_witness_is_local_form` — pull a
  `FixedPaginatedTail` with `start_record_count > 0` and a tokenized path
  in the excluded prefix: an unchanged subsequent push succeeds (stored
  witness == local recomputation); an actual local edit to the prefix
  still blocks push with the witness error; a whole-thread / zero-start
  pull stores no witness; the stored witness provably comes from the
  decoded local chain, not the staged wire chain.
- `pull_materialization_digest_pins_match_staged_local_bytes` — the
  engine's `verify_provider_payload` passes on a pulled thread whose wire
  and local digests differ (wire-form pins would abort at `TargetStaged`).
- `push_recovery_content_witness_compares_wire_tails` — CAS-ambiguity
  inspect identifies the published candidate via op `tail_after` vs the
  manifest's wire tail on a thread whose local tail differs from both.
- `target_root_collision_blocks_pull_with_the_record_named` — wire
  content mentioning a verbatim path equal to the target root + suffix:
  pull blocks at the §4.4 gate; the same content pulls cleanly onto a
  non-colliding root.
- `repair_rebases_pins_and_blocks_collisions` — pinned thread: repair =
  `decode_new(encode_old(x))`, wire unchanged, checkpoint rebased
  atomically; no-suffix repair lands in Sync with nothing to publish; a
  local suffix stays LocalOutstanding; pre-existing new-root occurrences
  block (gate for pinned, pre-scan for unpinned); a real prefix edit
  after repair still trips the local witness.
- `apply_repins_the_reviewed_four_input_classification` — action carries
  `pinned_accepted_local_tail` + the checkpoint generation; a checkpoint
  advance between review and apply fails with the typed stale error
  instead of reading live state.
- `windows_roots_block_push_and_pull_before_parsing` — a non-POSIX
  binding root fails conversation Push and Pull with the typed
  precondition error, before any rollout bytes are read
  (cfg/platform-gated test).
- `subdirectory_sessions_materialize_under_target_root_plus_suffix` and
  `out_of_root_paths_are_byte_identical_end_to_end` — a multi-cwd fixture
  with a foreign `turn_context` cwd, out-of-root sandbox entries, and
  `$HOME`-but-not-project content mentions: assert the foreign cwd never
  tokenizes, and every out-of-root occurrence is byte-identical across
  encode → wire → decode (plus injectivity guards).
- `dual_pins_classify_the_four_core_relations` — the §5 table, driven
  through the single classifier call site; plus Missing*/Invalid/Detached
  rows and the no-pin adoption row (encode-compare equal and unequal).
- `root_level_paths_remap_for_subdir_only_sessions` — a session whose
  every structural cwd is `root/frontend` still remaps a
  `root/README.md` content mention (the dynamic-table design's coverage
  hole, pinned as a regression test); a later push with an additional
  subdir cwd reproduces the accepted wire prefix byte-for-byte (token
  choice can never drift).
- `projection_synthesized_uris_round_trip_through_the_wire` — a legacy
  exec display event with a space-containing in-root cwd: projection
  emits `file:///__mallard_root__/...%20...`, the §4.4 wire self-check
  passes at push, and pull decode → re-encode reproduces the wire record
  (the undeclared-third-form hazard, pinned).
- `a_to_b_to_a_round_trip_is_stable_after_canonicalization` — full
  cross-machine story extended from
  `cross_machine_paginated_tail_survives_pull_divergence_and_republish`,
  with in-content paths, a B-side append, divergence + KeepBoth, and
  re-adoption after checkpoint loss. Asserts: already-canonical
  occurrences (raw absolute, escaped URIs) byte-restore on A;
  tilde/unescaped-URI occurrences respell canonically on the first
  pull-back and are byte-stable on every subsequent hop; identity fields
  are fresh per hop; tails and digests stay equal throughout.
- Repair: matcher-scope audit finds sandbox-entry staleness; repaired
  thread republishes.
- Unchanged-suites assertion: domain `TAIL_VECTOR_*` golden vectors and all
  fold/accumulator tests pass untouched (fingerprint function is not
  modified).

## 11. Touch list

| Area | Files |
|---|---|
| Matcher/encoder/decoder module (new) | `crates/mallard-infrastructure/src/codex_path_map.rs` |
| Wire build/decode | `crates/mallard-infrastructure/src/codex_standalone_builder.rs` |
| Base plumbing, staging refold | `crates/mallard-infrastructure/src/codex_history_graph.rs` |
| Checkpoint twin (`accepted_local_tail`), codec labels, validation | `crates/mallard-domain/src/conversation_tail.rs` |
| Pin-relative classifier + evidence gates | `crates/mallard-domain/src/conversation_tail_relation.rs`, `crates/mallard-infrastructure/src/conversation_sync.rs` |
| Reviewed-action pins (`pinned_accepted_local_tail`, checkpoint generation) | `crates/mallard-domain/src/lib.rs` (`ConversationPlanAction`), `src/types.ts` |
| Push/pull sites, adoption, repair + checkpoint rebase, audit | `src-tauri/src/project_sync/commands.rs` |
| Wire tail codec constant | `crates/mallard-domain/src/conversation_tail.rs` (`PORTABLE_TAIL_CODEC_WIRE_V1`; `validate_codex_wire` pins it; local constant and `codex.lineage.v1` analyzer key untouched) |
| Windows gating | encoder base registration + decode block (`codex_path_map.rs`), platform-gated tests |
| Tests | codec/builder/graph module tests, `src-tauri/src/sync_tests/*`, formal-verification doc rewrite |

## 12. Open questions

1. ~~`MAX_PATH_BASES`~~ — resolved (2026-08-15 review): the dynamic
   structural-cwd table is gone; the single immutable root base (§3.1)
   removes the bound, index pinning, injectivity guards, and
   first-contact table inference entirely.
2. ~~Wire codec label spelling~~ — resolved (2026-08-15 review): the
   split lives at the portable-tail envelope layer, not the lineage
   analyzer. Local form keeps `mallard-codex-portable-tail-v1`; wire form
   is a new domain constant `mallard-codex-portable-tail-wire-v1`;
   `codex.lineage.v1` is untouched (§3.2).
3. Whether `world_state.state.environments.filesystem` (rendered file
   trees, largest single field) should be size-capped per record in the
   matcher pass or treated like any string (recommend: no special case;
   the pass is linear either way).
4. ~~Tilde form~~ — resolved (2026-08-15 review): spelling tags were
   non-injective on decode (`p` onto escape-free targets, `t` onto
   out-of-home targets), so the wire canonicalizes to two forms and tilde
   collapses to the path form (§3.1/§4.1). Tilde occurrences are still
   remapped — only their spelling is not preserved.

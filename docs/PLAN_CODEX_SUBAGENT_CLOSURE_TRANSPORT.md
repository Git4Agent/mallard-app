# Plan: Codex subagent closure transport — Push/Pull follow-up

- Status: Superseded on 2026-09-07 by
  [PLAN_CODEX_OWNED_SUBAGENT_SYNC.md](PLAN_CODEX_OWNED_SUBAGENT_SYNC.md).
  Historical proposal; not an implementation authority.
- Date: August 11, 2026
- Follows:
  [PLAN_PORTABLE_TAIL_CONFLICT_RESOLVER_BACKEND.md](PLAN_PORTABLE_TAIL_CONFLICT_RESOLVER_BACKEND.md)
  (fills the subagent closure that v1 ships empty)
- Design authority:
  [DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md](DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md)
  "Subagent behavior", with the two amendments in Section 10
- Supersedes: [PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md)
  (pre-portable-tail architecture; its provider evidence carries over)
- Provider scope: Codex 0.147.x only; older rollout shapes are not a design
  input
- Scope: Rust domain wire/plan additions, one new infrastructure discovery
  module, builder closure attachment, Push planning/apply, Pull apply, engine
  target gates, backend tests
- Excluded: React UI work, Claude sidechains, any subagent conflict state
- Compatibility: additive within the schema-1 in-place contract; a manifest
  without subagent entries reads as an empty closure

## 0. Product posture

The following posture is superseded. The owned-child plan makes included
children required comparison evidence and can block the affected parent on
missing required evidence or fail its Push on a source change after Apply
capture. A known unsupported child selects main-only source scope; this is
not a stored mode. Pull restores exactly the manifest-listed files. The plan
also replaces additive compatibility with a clean schema-1 cutover. The text
below records the earlier decision.

Subagent rollouts are best-effort snapshot payload. The parent conversation
remains the only synced resource; its tail, classifier evidence, relations,
checkpoints, and bindings are untouched by anything in this plan. The closure
rides along only when a root-driven Push already uploads the artifact and a
root-driven Pull already materializes it.

Deliberate tolerance (product decision, 2026-08-11): **a missed subagent is
acceptable; a delayed or blocked user is not.**

1. Push planning discovers the closure as **metadata only**: it never reads,
   hashes, or trims a child transcript. Per publishing thread it costs one
   scan over parent bytes plus one bounded first-line read and one `stat`
   per directly referenced child. Child bytes are read exactly once, at
   apply, where upload needs them anyway.
2. No closure condition ever blocks or stales a Push or Pull. Every
   closure-membership failure degrades to "skip this child and warn."
3. What does ship is fully integrity-checked: every transported child object
   is SHA-256 verified by the existing object path, and every installed
   child passes a bounded payload verifier. Tolerance applies to membership,
   not to bytes.
4. Push carries the then-current snapshot; a child that grows or appears
   after review ships on the next publishing Push. There is no subagent-only
   operation, drift state, or relation (unchanged from the main plan §1).

This narrows the main plan's §2.1 "missing children … block" clause and §12
"never omit an unrecognized subagent" for the closure only; Section 10
records the amendments.

### Checks deliberately eliminated

Named here once so their absence reads as a decision, not an oversight:

- **No recursion at planning.** Planning admits direct spawns of the root
  chain only (403/425 observed children are depth-1) and never opens a
  child transcript. Descendants — grandchildren and deeper — are instead
  discovered **at apply**, by scanning bytes apply has already read to
  upload (Section 4): the scan itself is free, and the only added IO is the
  descendants' own files (11 of 51 observed closures have any; ~43 MB
  across the entire corpus, ~20 MB worst single root). The trade, accepted
  deliberately: descendants ship as unreviewed additions, and review counts
  derived from the plan cover direct children only.
- **No child hashing or byte pins at planning**, and therefore no
  plan-vs-apply byte-staleness gate for children. The plan pins the child
  *list* (id + source path); the shipped snapshot is whatever those files
  contain at apply, trimmed to a record boundary. Fresher is better for a
  payload that no conflict decision reads.
- **No re-admission of pinned children at apply.** Apply trusts the plan's
  admitted list; its only per-pinned-child check is that the file still
  parses to a `session_meta` whose id equals the pinned id (a file-swap
  guard), else skip + warn. Descendants found at apply run the admission
  gates exactly once, there.
- **No guardian/user-thread allowlists.** One shape gate (the
  `thread_spawn` source with a matching back-edge) excludes both by
  construction.

## 1. Provider contract, verified on 0.147.x

The v1 empty-closure decision was premised on "Codex writes no structural
parent-to-subagent reference records." That premise is wrong; a full scan of
this machine's `~/.codex/{sessions,archived_sessions}` (1,186 rollouts, scan
script pinned as a test fixture) shows, restricted to `cli_version 0.147.*`:

| Fact | Count |
| --- | --- |
| Roots / subagent rollouts | 61 / 261 (223 `thread_spawn`, 38 guardian) |
| Parent-side reference records (`event_msg` / `sub_agent_activity`, `kind` started/interacted/interrupted, carrying `agent_thread_id`) | 223/223 `thread_spawn` children referenced |
| Child-side back-edge: top-level `parent_thread_id` and `source.subagent.thread_spawn.parent_thread_id`, both present, pointing at an existing rollout | 223/223 |
| Guardian (`source.subagent.other`) referenced by any parent | 0/38 |
| Subagent rollouts with a non-null `history_base` | 0 |
| Maximum spawn depth (`thread_spawn.depth`) | 2 |
| `sub_agent_activity` references resolving to ordinary user threads (`thread_source: "user"`, VS Code collab) | 42 across the full corpus |

Closure sizes across the full corpus: 51 roots have a non-empty closure,
median 4 children, maximum 51 children / 276.5 MB.

Contract this plan builds on (all four facts hold bidirectionally on 0.147.x
and are consistent with the AgentControl design visible in the
`openai/codex` sources):

1. Parents reference spawned children only via `event_msg` records with
   `payload.type == "sub_agent_activity"` and `payload.agent_thread_id`.
   These records are chain-Ignored: they never affect the canonical chain,
   digests, or tail.
2. `thread_spawn` children carry the parent back-edge in their authoritative
   `session_meta` (`parent_thread_id`, plus the nested copy under
   `source.subagent.thread_spawn`). `session_id` is parent-valued.
3. Children are standalone files: no `history_base`, absent from native
   indexes, filename `rollout-<ts>-<child-uuid>.jsonl` under the same
   session roots (already proven readable in isolation by the design doc's
   disposable 0.147.0 tests).
4. A referenced id is not necessarily a child: it may be a user thread
   (collab) or dangling (pruned). Membership must be decided by the resolved
   child's own metadata, never by the reference alone.

Contract drift is absorbed by construction: every gate below fails toward
"skip and warn," so a future Codex shape change degrades coverage, never
availability.

## 2. Transport model: verbatim bytes, original identity

A closure child is transported as an opaque, byte-verbatim file under its
original provider thread id — on Push and on Pull, including `Reidentify`
installs. No child record is parsed beyond the authoritative meta line, no
child byte is rewritten, no event payload is rewritten, and children receive
no `ThreadId`, binding, checkpoint, layout, index row, or portable tail.

Why verbatim beats rewriting (evaluated against the main plan §2.1's
"consistently rewrites" clause; Section 10 amends it):

- The parent→child link lives in `sub_agent_activity` payloads, which the
  builder never touches. Preserving child ids keeps those pointers resolving
  on the destination with zero rewrite machinery.
- Byte-identical children are sha-stable end to end: content-addressed
  dedupe works across repeated pushes and across machines, and a re-pull of
  the same tree is a natural no-op.
- Keep-both needs nothing new: `T -> P-remote`'s closure arrives with the
  same child ids `C -> P-local` already has on disk, and the existing-file
  rule below makes them shared, exactly the outcome the superseded plan
  wanted.
- The only cost is a dangling child-side back-edge (`parent_thread_id` still
  names the source root) after a reidentified Pull. The parent-side pointers
  — the ones the viewer walks — stay valid. Section 6 covers the manual
  viewer check and the contingency.

Wire shape (additive):

```rust
enum ConversationFileRole {
    Transcript,
    ProviderIndexFragment,
    Sidecar { sidecar_kind: String },
    SubagentTranscript { provider_thread_id: String }, // new, kind = "subagent_transcript"
}
```

- Logical path: `state/conversations/{provider}/{thread_id}/subagents/{original_basename}`,
  where `{thread_id}` is the parent row's manifest key — since the cloud
  thread id decoupling (PLAN_CLOUD_THREAD_ID_DECOUPLING.md) that key is the
  parent's **cloud** id (`cloudthread-…`), never a machine-local id — and
  the basename is the source rollout filename (it embeds the child
  uuid; validation requires the `-{provider_thread_id}.jsonl` suffix).
  `pinned_subagent_sources` remains a plan-time pin of **existing local
  files**, which stays legal under that plan's D2 (apply-time minting bans
  minting fresh identities at plan time, not pinning existing state).
  `ConversationThread::validate` gains this one namespace rule; the
  exactly-one-transcript rule and every other rule are unchanged.
- Per-file sha256/size stay on `BundleFileEntry`, joined by logical path, as
  for every other file. Objects are content-addressed (`_objects/{sha}`), so
  unchanged children cost one manifest entry and zero uploads.
- `StandaloneConversationArtifact.subagents` (already on the struct) becomes
  live; `artifact_digest` already folds the ordered
  `(role, provider_id, byte_len, sha)` file list, so `artifact_sha256`
  commits to the closure with no format change. The artifact's subagent list
  is sorted by `provider_thread_id`; Pull folds the installed closure into
  the accepted `artifact_sha256` the same way.
- Old manifests contain no `subagent_transcript` entries and read as an
  empty closure. New manifests are unreadable by pre-cutover binaries, which
  mixed-version policy already declares unsupported.

Plan-side pin (machine-local plan document, not cloud wire):

```rust
// ConversationPlanAction += pinned_subagent_sources: Vec<SubagentSourcePin>
struct SubagentSourcePin {
    provider_thread_id: String, // lowercase uuid, validated
    source_path: String,        // absolute, machine-local
}
```

Validation: count ≤ `MAX_SUBAGENT_CLOSURE_FILES`, unique ids, absolute clean
paths; present (possibly empty) only on `Publish`/`CreateCloud` actions.
Because the list lives on the reviewed action, review-time disclosure counts
are derivable by the frontend later without further backend work. The pins
enumerate **direct children only**; descendants discovered at apply
(Section 4) are unreviewed additions on top of this list, so any disclosure
derived from it understates by design.

Bounds (constants in `mallard-domain`):

- `MAX_SUBAGENT_CLOSURE_FILES = 62` — with the transcript and index fragment
  this fits the engine's existing 64-target cap; observed maximum is 51.
- `MAX_SUBAGENT_CLOSURE_TOTAL_BYTES = 512 MiB`, estimated at planning from
  `stat` sizes — matches `MAX_PROVIDER_TARGET_BYTES`; observed maximum
  closure is 276.5 MB.
- Per file: existing `MAX_CONVERSATION_FILE_BYTES` (100 MiB) via
  `read_rollout_bounded` at apply.
- `MAX_SUBAGENT_CLOSURE_DEPTH = 4` — apply-side recursion bound (Section 4);
  observed maximum is 2.

All caps cover the transitive set: pinned children consume them first, then
apply-discovered descendants in discovery order. Exceeding a bound drops the
not-yet-admitted remainder in admission order and warns; it never blocks.

## 3. Discovery — a metadata-only Push-planning step

New module `crates/mallard-infrastructure/src/codex_subagent_closure.rs`.
Planning invokes it only after an action classifies as `Publish` or
`CreateCloud` (every other relation pays nothing), at the
`build_conversation_plan_action` call site where the root was just resolved,
so segment bytes are page-cache-hot on the resolve path and read once on the
checkpoint-cursor path.

```rust
fn discover_codex_subagent_sources(
    resolved: &ResolvedCodexRoot,
    lookup: &dyn Fn(&str) -> Result<Option<PathBuf>, String>, // codex_rollout_lookup
) -> CodexSubagentDiscovery // { pins: Vec<SubagentSourcePin>, warnings: Vec<String> }
```

1. **Reference extraction.** For each resolved segment, scan only its
   consumed byte range (`bytes[..consumed_byte_len]`, the exact ranges the
   builder later emits) line by line; collect `payload.agent_thread_id` from
   records with `type == "event_msg"` and
   `payload.type == "sub_agent_activity"`, first-seen order, deduplicated.
   Consumed-range scoping is what keeps a child spawned by an ancestor
   *after* the fork cutoff out of this root's closure. Lines without the
   `sub_agent_activity` substring are skipped without JSON parsing.
2. **Admission per candidate id** — one bounded first-line read plus one
   `stat`, no transcript read (each failure ⇒ skip + warning, never an
   error):
   - `lookup(id)` finds a rollout (`sessions/` preferred over
     `archived_sessions/`, existing helper). Absent ⇒ dangling reference —
     legal provenance per the design doc.
   - The authoritative first `session_meta` (bounded read, existing
     metadata-scan pattern) must classify as an internal subagent and carry
     the `thread_spawn` source shape. This excludes user threads (the 42
     collab references) and guardians by shape, not by list.
   - The parent back-edge (nested `thread_spawn.parent_thread_id`, falling
     back to top-level `parent_thread_id`) must be in the root's segment-id
     set. A foreign back-edge is another tree's transcript — skipping it is
     the privacy boundary; it ships with its own root instead. Planning
     admits direct children only; descendants are discovered at apply from
     bytes it already holds (Section 4).
   - A non-null `history_base` (unseen on 0.147.x) ⇒ skip + warn.
   - `stat` size feeds the estimated total-bytes cap.
3. **Result.** Admitted pins, sorted by `provider_thread_id`, attached to
   the action as `pinned_subagent_sources`; warnings logged with the plan.

Supporting parsers land in `codex_rollout.rs` (its single-file charter):
`subagent_activity_reference(&Value) -> Option<&str>` and
`session_meta_thread_spawn_parent(&Value) -> Option<String>`. Note:
`CodexSessionMetaRecord.thread_source` currently holds the serialized
`source` object, not the `thread_source` string — the new helpers read the
raw payload and do not touch that field.

## 4. Push apply

All in `build_publish_request` / `conversation_thread_from_capture`
(`src-tauri/src/project_sync/commands.rs`), Codex branch only:

1. After the root artifact builds and the existing apply-time staleness gate
   passes (`seeded.observed_tail == action.pinned_local_tail`), materialize
   the pinned closure: for each `SubagentSourcePin`, `read_rollout_bounded`
   the pinned path, require the first-line `session_meta` id to equal the
   pinned id (file-swap guard — the only apply-time check for pinned
   children), trim to the last complete newline (mid-run children snapshot
   at a record boundary; zero complete records ⇒ skip), and hash. Missing,
   oversized, or id-mismatched sources ⇒ skip + warning. The plan's list is
   authoritative for direct children: apply never re-discovers at depth 1.
2. **Descendant recursion (the one apply-side discovery step).** Scan each
   shipped child's in-hand bytes for `sub_agent_activity` references — free,
   since those bytes were just read for upload — and admit descendants
   through the same gates as Section 3.2, with one substitution: the
   back-edge must name an **already-shipped closure member**. Bounded by
   `MAX_SUBAGENT_CLOSURE_DEPTH`, a visited set, and the shared count/byte
   caps; each admitted descendant is read (`read_rollout_bounded` — the
   only new IO, ~43 MB across the whole observed corpus), trimmed, hashed,
   shipped, and scanned in turn. A pinned child that was skipped is never
   read, so its subtree never ships. Descendants are unreviewed additions:
   they appear in the manifest and receipt like any shipped child but not
   in the plan's pins or review counts (Section 0 tolerance).
3. `build_standalone_codex_root` gains a closure parameter so
   `artifact.subagents` and `artifact_sha256` are populated in one place
   (its stale "codec v1 declares no reference records" comment is deleted).
   The root re-analysis verification is unchanged — children never enter
   the canonical chain.
4. `conversation_thread_from_capture` chains one
   `ConversationFileRef { role: SubagentTranscript, .. }` per shipped file
   (pinned children and descendants alike) after the existing sidecar refs,
   and the scope loop inserts one `CapturedBundleFile` per file at the
   subagent logical path (the sidecar insertion site is the template). The
   engine's sha-addressed `pending_uploads` dedupes unchanged children; the
   existing old-thread cleanup already de-references children dropped since
   the previous push.
5. Skipped-child warnings ride the existing receipt-warning channel
   (`CompleteWithWarnings` semantics untouched; no schema change).
6. Recovery is unchanged: `snapshot_contains_actions` stays a tail witness.
   Closure completeness is explicitly not part of the publish witness — an
   ambiguous-CAS recovery adopts whatever closure the surviving candidate
   carries, consistent with best-effort snapshot semantics.
7. Non-Codex providers and non-publishing relations are untouched.

## 5. Pull integration

Enumeration is manifest-driven at apply; Pull planning,
`expected_provider_state`, and `validate_conversation_action_pins` are
unchanged (child targets are deliberately *not* pinned — this also avoids
disturbing the transcript-target selector that keys off the pinned `Absent`
entry).

1. `prepare_pull_conversation_actions`: enumerate the selected thread's
   `SubagentTranscript` refs; join them to the existing single `prefetch`
   batch; `read_bounded` each (existing double sha verification). A child
   object that is missing, oversized, or mismatched ⇒ skip + warning; the
   root transcript keeps its hard failure. `PreparedPullConversation` gains
   the prepared child list `(provider_thread_id, basename, bytes, sha256)`.
2. `apply_prepared_pull_conversation`: for each prepared child, the target
   path is the plan's dated `sessions/YYYY/MM/DD/` directory (same as the
   root target) with the original basename. **Existing file wins:** if the
   path is occupied, skip the child — no byte comparison, no prefix logic,
   no backup. Otherwise append a
   `ConversationTargetPayload { role: SubagentTranscript, expected: Absent, bytes }`.
   Target order: children first, root transcript then index fragment last,
   so an aborted batch leaves no root pointing at a half-installed closure.
3. Engine (`conversation_materialization.rs`):
   - `verify_target_role_set`: allow up to `MAX_SUBAGENT_CLOSURE_FILES`
     subagent targets on a `Resumable` operation (transcript/index rules
     unchanged).
   - `verify_provider_payload`: new arm — bounded check that the payload's
     first line is a `session_meta` classifying as an internal subagent
     whose id equals the role's `provider_thread_id`, and the payload ends
     with a newline. (The existing `Sidecar` single-JSON-value arm and the
     `Transcript` root-identity arm both reject JSONL children — this arm
     is why the role-reuse shortcut doesn't work.)
   - **Best-effort Absent, stated once:** a `SubagentTranscript` target
     whose path is occupied at any expectation or commit point is treated
     as already satisfied — journal it committed, warn, continue. It is
     never an `AbortedProviderCollision` and never fails the operation. All
     other roles keep hard no-clobber semantics.
4. Children ride the existing journal/recovery verbatim (intent payloads,
   staged siblings, roll-forward). The all-or-nothing restage caveat (every
   intent payload must survive to restage) is accepted and tested.
5. Unchanged by design: no child index rows (`codex_index_payload` writes
   the parent row only), no child bindings/checkpoints/allocations
   (`mint_provider_thread_id` is not called for children), metadata-only
   actions move no subagent bytes, keep-both's `P-local` closure stays
   attached to `C` while `T`'s incoming closure lands via the
   existing-file-wins rule, and a `Detached` link classifies exactly as
   today.

## 6. Viewer check and contingency (informative, non-gating)

Verbatim transport keeps parent-side `sub_agent_activity` pointers valid on
the destination; the child-side back-edge dangles when the root was
reidentified. Early in implementation, run the superseded plan's Step-0
check on 0.147.x: copy a fixture tree into a scratch `CODEX_HOME`, rename
only the root id (simulating Pull), and confirm the viewer still expands
subagent activity. If — and only if — the viewer proves to validate the
child back-edge, the contingency is an equality-guarded rewrite of exactly
three child meta fields (`parent_thread_id`, parent-valued `session_id`,
`source.subagent.thread_spawn.parent_thread_id`) at install, accepting the
loss of cross-machine sha stability. Do not build this speculatively.

## 7. Implementation sequence

1. `codex_rollout.rs` helpers (`subagent_activity_reference`,
   `session_meta_thread_spawn_parent`) with unit tests over real-shape
   fixtures.
2. `codex_subagent_closure.rs`: reference extraction, metadata-only
   admission, caps, warnings, and the apply-side descendant recursion
   driver; module tests including the full admission matrix.
3. Domain: `SubagentTranscript` role + namespace rule,
   `SubagentSourcePin` + `pinned_subagent_sources` with validation, bounds
   constants; validation tests.
4. Builder: closure parameter, sorted digest attachment, comment fix; test
   that root re-analysis, chain, digests, and tail vectors are unchanged
   with a populated closure.
5. Push planning wiring (Section 3) and push apply wiring (Section 4) with
   integration tests.
6. Engine gates and best-effort Absent (Section 5.3) with crash-matrix
   coverage of child targets.
7. Pull band wiring (Sections 5.1–5.2) with integration tests.
8. Cross-machine scenario + gates; Section 10 doc amendments in the same
   change.

## 8. Backend test plan

Temporary Mallard/Codex/project/storage roots throughout; never a real home.

- Discovery: parent-events-only membership (a back-edged child never
  mentioned in consumed events stays local); consumed-range scoping (a
  child spawned past an ancestor cutoff is excluded); user-thread and
  guardian references skipped; foreign back-edge skipped; dangling
  reference warns and proceeds; malformed child meta warns and proceeds;
  planning is depth-1 (a grandchild never enters the pins);
  count/byte caps drop deterministically with warnings; incidental
  `agent_thread_id` text in tool output is never a reference (the
  `event_msg` + `sub_agent_activity` shape gate); discovery runs zero times
  for non-publishing relations (test spy).
- Planning cost envelope: discovery performs no child transcript reads
  (test spy on the bounded reader — first-line scans only).
- Push apply: pinned-but-deleted child skips with a warning; a swapped file
  at the pinned path (different meta id) skips; a child that grew after
  review ships its apply-time bytes trimmed at a record boundary; a direct
  child spawned after review does not ship (never pinned).
- Descendant recursion: a grandchild referenced by a shipped child ships,
  and its own children ship up to `MAX_SUBAGENT_CLOSURE_DEPTH`; a
  grandchild whose back-edge names a non-member skips; a skipped pinned
  child's subtree is never read or shipped; cycles and depth overflow drop
  with warnings; descendants share the count/byte caps with pinned
  children (pins consume first); descendants never appear in
  `pinned_subagent_sources`; a descendant referenced only in apply-time
  child bytes ships (unreviewed-addition semantics); recursion performs no
  reads beyond admitted descendants' own files (test spy).
- Codec neutrality: all existing `TAIL_VECTOR_*` golden values and the root
  lineage/semantic digests are byte-identical with 0, 1, and max children;
  `artifact_sha256` differs.
- Push: manifest gains one `subagent_transcript` ref + object per shipped
  child; re-push with unchanged children uploads zero child objects;
  dropping a child locally de-references it on the next push; non-Codex
  providers unaffected.
- Pull: fresh install writes children (children before root in the target
  vec); re-pull of the same tree skips every existing child; occupied child
  path at commit time skips without aborting the operation (root still
  installs); missing/corrupt child object skips with a warning while the
  root hard-fails on its own corruption; no child index rows, bindings, or
  checkpoints; keep-both shares on-disk children between `C` and `T`; the
  64-target engine cap is never exceeded; crash injection at every journal
  phase reaches the same fixed point with child targets present.
- Round trip: A pushes root+children → B pulls (root reidentified, children
  verbatim) → B pushes after a new root turn → A pulls and every unchanged
  child is skip-satisfied.
- Contract fixture: the pinned scan script's assertions over a synthetic
  0.147-shaped fixture home (reference shape, back-edge shape, guardian
  invisibility), so provider drift fails a test instead of silently
  emptying closures.

## 9. Acceptance criteria

1. A publishing Push of a 0.147.x parent uploads its admitted `thread_spawn`
   closure — pinned direct children plus apply-discovered descendants,
   within the caps — as verbatim, sha-verified objects; a materializing
   Pull installs them under their original ids next to the root.
2. Push planning latency grows only by bounded metadata work on publishing
   threads: no child transcript is read or hashed before apply, proven by
   test spies.
3. No closure condition blocks, stales, or fails a Push or Pull; every skip
   is warned and enumerable.
4. Tail values, lineage/semantic digests, classifier evidence, relations,
   checkpoints, bindings, and index rows are provably byte-identical with
   and without closures.
5. Unchanged children never re-upload and never re-install; guardians,
   user-thread references, and foreign-parented children never ship.
6. Crash recovery over child targets is deterministic and no-clobber, and
   an aborted batch never leaves an installed root referencing a
   half-installed closure.

## 10. Amendments to prior documents (same change)

1. Main plan §2.1 "Missing children, ambiguous ownership … make the
   resource `Blocked`/`StoreOnly`" and §12 "never omit an unrecognized
   subagent" are narrowed for the subagent closure to the Section 0
   tolerance posture (skip + warn). Cycles, bounds, and root/history-segment
   failures still block.
2. Main plan §2.1 "the builder retains and consistently rewrites
   [subagent references]" and the design doc's "Reidentify the selected
   root/subagent closure with plan-persisted provider IDs" are amended:
   children are transported verbatim under original ids; only the root
   reidentifies. Rationale in Section 2; contingency in Section 6.
3. `codex_standalone_builder.rs`: delete the false "Codex declares no
   structural parent-to-subagent reference records" comment.
4. [PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md) is
   marked superseded by this plan (its `event_msg` evidence is reconfirmed
   on 0.147.x; its attached-file architecture, sha-dedupe, and
   viewer-preservation reasoning carry over; its child-sha conflict rule,
   prefix fast-forward, index alignment, and UI items do not).

## 11. Verification commands

```text
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -p mallard-domain
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Run opt-in Codex viewer/compatibility checks (Section 6) only against
explicitly created disposable provider homes.

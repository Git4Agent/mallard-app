# DESIGN: Flat thread forks

Status: design accepted in principle; supersedes the earlier plan draft in
this file.

## Locked decisions

1. **Flat threads.** A fork is an ordinary new thread with a fresh `ThreadId`.
   The conflict-group / variant / primary / preference axis is deleted.
2. **Never converge local content.** Conflict resolution never rewrites a
   local provider session file. The only operations that ever write local
   transcripts are fast-forward pull, force pull, and materializing a new
   local conversation — all user-pulled, none automatic.
3. **Fork is always the default.** `Diverged` (and `Unknown`-lineage push)
   defaults to fork and is auto-selected in review. Force actions exist but
   always require explicit approval.

## Re-evaluation under these constraints

The earlier draft flagged "never converge" as a re-fork loop. That is true
only if the fork leaves the local conversation *bound to the same cloud
thread*. The loop disappears when the fork **rekeys the binding** instead of
the content:

> Identity converges; content never does.

After fork push, the local conversation is bound to `t'`, whose cloud chain
is identical to the local chain — relation `Identical`, no re-fork, and
future local messages fast-forward `t'`. Cloud `t` is untouched and simply
becomes an unmaterialized "cloud only" row on this host. No local file is
written at any point.

This is not a new mechanism. The current push-diverged path already behaves
exactly this way: `AddAlternate` publishes the local chain as a new cloud
identity and `rekey_same_id_divergence` moves the local binding to it, with
`ConversationSyncOperation { old_variant_id, rekeyed_local_variant_id,
rekey_committed }` making the two-step commit crash-safe. The design keeps
that pattern verbatim and deletes the variant/group wrapper around it. So
"never converge locally" is not a compromise — it is the smaller, safer
design: fork push requires no codec rewrite, no provider quiescence, and no
local writes.

## Model

- **Thread** — cloud identity: manifest key `ThreadId` → message chain,
  transcript files, metadata. `ThreadId`s are random, minted fresh on fork
  (never derived names like `t-prime`), so concurrent forks on two hosts
  cannot collide.
- **Local conversation** — provider session file(s) plus one binding row
  `(host, replica, project, profile) → thread_id` with last-observed digests.
- **Chain** — `CanonicalConversationMessage` list (`message_id`,
  `parent_message_id`, `semantic_fingerprint`); unchanged. Relations are
  computed by the existing `classify_conversation_relation`.

Invariants:

- At most one local conversation is bound to a given `ThreadId` per
  (project, profile).
- A fork records `forked_from: Option<ThreadId>` on the new thread —
  provenance only, never consulted by the resolver.
- Cloud manifest publishes are CAS on `generation`; fork publishes are purely
  additive, so a lost race rebases trivially.

## Decision tables

Push (local conversation bound to `t`):

| Relation | Action | Notes |
|---|---|---|
| Identical | NoOp | |
| CloudPrefixOfLocal | FastForwardCloud | safe fast-forward push |
| LocalPrefixOfCloud | RequirePull | status: cloud ahead |
| Diverged | **Fork** (default) | publish local chain as `t'`, rekey binding `t → t'` |
| Unknown | Fork | content preserved on both sides, so guessing is safe |
| CloudMissing | CreateCloud | publish as new thread (new local conversations, and minted-but-unpublished forks under their recorded `ThreadId`) |
| LocalMissing / BothMissing | NoOp | |

Pull (cloud thread `t`):

| Relation | Action | Notes |
|---|---|---|
| Identical | NoOp | |
| CloudPrefixOfLocal | KeepLocal | status: local ahead |
| LocalPrefixOfCloud | FastForwardLocal | gated on quiescence + resumable codec, else StoreOnly |
| Diverged | **Fork** (default) | materialize cloud chain as new local conversation bound to `t`; unbind the divergent local conversation |
| Unknown | StoreOnly | |
| LocalMissing | CreateLocal (or StoreOnly) | materialize cloud-only thread on demand |
| CloudMissing / BothMissing | NoOp / KeepLocal | |

Explicit-approval actions (never default): `ReplaceCloud` (force push),
`ReplaceLocal` (force pull), `TombstoneThread`.

Deleted actions: `PromoteLocal`, `PromoteCloud`, `TombstoneVariant`,
`TombstoneGroup` (collapses to `TombstoneThread`); `AddAlternate` is renamed
`Fork` / `CreateCloud` / `CreateLocal` per the tables above.

## Fork push

Local bound to `t`, chains diverged.

1. Plan: mint `t'` (random `ThreadId`) and record it in the plan action
   (`result_thread_id`), with the reviewed local chain digests pinned.
2. Journal `Prepared` with `old_thread_id = t`, `result_thread_id = t'`.
3. Upload transcript objects; CAS-publish manifest adding thread `t'`
   (chain = local chain, `forked_from = t`, title inherited). Journal
   `CloudPublished`.
4. Rekey local binding `t → t'` (uniqueness-checked, revision-pinned, same
   pattern as today's rekey minus the preference pointer). Journal
   `LocalCommitted`, `rekey_committed = true`.

No local file is read except to capture, none is written. No provider
quiescence needed. Crash between 3 and 4: recovery replays the journal —
`t'` is already published with pinned digests, so retry completes only the
rekey; no duplicate fork is possible because the minted `t'` travels in the
journal, not recomputed.

Post-state: local ≡ cloud `t'` (Identical). Cloud `t` unchanged; on this
host it now shows as a cloud-only row, pullable as a separate conversation.
Other hosts bound to `t` are completely unaffected — a fork never
invalidates anyone else's binding.

## Fork pull

Local bound to `t` with chain `a b c2`, cloud `t` has `a b c0`; user pulls.

1. Mint `t'` and rekey the divergent local conversation `t → t'` — the
   same journaled primitive as fork push, with publication deferred. Its
   file is untouched; it now classifies as `CloudMissing` under `t'`, so
   the next push publishes it via CreateCloud under the already-minted ID.
   Until then the UI shows it as "not yet published".
2. Materialize cloud `t`'s chain as a **new** local conversation: mint a
   provider thread ID, codec structural rewrite (existing machinery), bind
   it to `t`. Journaled with staging/backup targets as today.

Both binding writes go through the same revision-pinned atomic
bindings-document mutation used today, so the one-conversation-per-
`ThreadId` invariant never has a transient violation. Eager minting also
means fork is a single primitive (mint + rekey; push publishes now, pull
publishes on the next push) and no unbound state exists — a deliberately
forked conversation can never be silently re-attached to a compatible
cloud ancestor by `recover_unique_compatible_binding`.

Pull never writes to the cloud, so the divergent chain reaches the cloud on
the next push rather than during the pull — acceptable: nothing is lost
locally in the meantime, and a host that never pushes never publishes it.

## Force (explicit only)

- `ReplaceCloud`: cloud `t` := local chain. Destroys the cloud chain — the
  first cloud-destructive conversation operation, therefore gated on
  explicit approval plus pinned cloud generation/digests. Optional
  mitigation (recommended, cheap): auto-record the losing chain as a
  content-bearing tombstone so force is recoverable.
- `ReplaceLocal`: local file := cloud chain. The only conflict action that
  writes a local transcript; requires provider quiescence and staged
  commit with backup (existing journal targets).

## Data model changes

| Item | Today | After |
|---|---|---|
| `BundleManifest.conversation_groups: Map<ConflictGroupId, ConversationGroup>` | nested group → variants | flat `conversations: Map<ThreadId, ConversationThread>` |
| `ConflictGroupId` | group key | **renamed** `ThreadId` (83 refs survive as a rename; `conversation_resource_id` drops its variant component) |
| `VariantId` (144 refs / 12 files) | variant key | **deleted** |
| `ConversationGroup` (`primary_variant_id`, variants map, `validate_variant_parent_chain`) | | **deleted**; `display_title` moves onto the thread |
| `parent_variant_id`, `fork_message_id` | in-group lineage | one optional `forked_from: Option<ThreadId>` |
| `LocalConversationPreference` + `preferred_variant_id` + its revision pin + journal `initialize_preference` | per-host preferred variant | **deleted** — nothing to prefer |
| `rekey_same_id_divergence` | variant rekey + preference move | **kept, simplified** to a thread-binding rekey; becomes the core fork-push primitive |
| `LocalConversationBinding.{conflict_group_id, variant_id}` | two-part key | single `thread_id` |
| Journal `{conflict_group_id, source/old/rekeyed/result_variant_id}` | variant-shaped | thread-shaped (`old_thread_id`, `result_thread_id`, `rekey_committed` kept) |
| Promote/variant-tombstone actions, `RejectPrimaryWithoutReplacement` | | **deleted**; one `TombstoneThread` with pinned-digest + preserve-modified guards |
| `ThreadVariantReview` grouping, primary/preferred badges | grouped variant UI | flat rows, fork pre-selected, `forked_from` hint |

What stays, unchanged: message chains and `classify_conversation_relation`;
`head_message_id` / `message_count` / lineage + semantic digests (relation
classification and pin validation without transcript downloads); guard
states (`Unknown`, missing sides, quiescence, codec resumability, binding
resolution, `recover_unique_compatible_binding` over all threads); provider
thread ID reservation + codec rewrite for **pull-side** materialization; the
reviewed-plan pin architecture minus the preference/variant pins; the
journal and its phase machine.

## UI

- Thread list: flat. A fork's origin shows as a "forked from" hint via
  `forked_from`; after fork push the user's live conversation continues
  under the same title with zero interruption, and the origin thread appears
  as a cloud-only row. Duplicate titles are disambiguated by the hint and
  timestamps, not by renaming.
- Review: `ThreadVariantReview` → flat thread review; fork/create rows
  auto-selected (today's `AddAlternate` default behavior, preserved); force
  and tombstone rows keep explicit-approval styling; statuses map 1:1 to the
  relation labels already shipped ("Cloud ahead — pull required",
  "Local ahead", "Diverged — will fork", …).

## Migration (one-way, bundle schema 5 → 6)

- **Deterministic `ThreadId` derivation**: every variant becomes a top-level
  thread with `thread_id = {conflict_group_id}.{variant_id}`; non-primaries
  get `forked_from = {conflict_group_id}.{primary_variant_id}`; the primary
  inherits the group's title. Because the mapping is a pure function of IDs
  already present in every document, cloud manifests, per-host bindings, and
  journals can migrate independently, in any order, idempotently — no host
  needs to know which variant was primary, and no IDs are minted during
  migration. (Trade-off vs "primary inherits the bare group ID": every
  thread ID changes; acceptable because raw IDs are not user-visible —
  verify with a grep that nothing else persists `ConflictGroupId`, e.g.
  activity log or git `thread_refs`, before locking this rule.)
- **Cloud rewrite is metadata-only**: one CAS manifest commit per bundle
  flattens `conversation_groups` → `conversations`, rewrites `resources`
  keys and `recipe` selections (resource IDs embed the old
  `{group}:{variant}` pair; manifest validation requires recipe and
  resources to match), rewrites `files` logical paths
  (`state/conversations/{provider}/{group}/{variant}/…` →
  `…/{thread_id}/…`), and rewrites conversation tombstone keys. Every
  `BundleFileEntry` keeps its `object_key`, which is upload-scoped and
  independent of the logical path — no cloud objects are copied or moved.
- Preferences are dropped. Bindings document (schema v2) and journal migrate
  per host on first run using the same derivation; in-flight journal
  operations must drain or be recovered first.
- Old clients are fenced by the existing `schema_version` validation.

## Removal plan (metadata and data structures)

Staged so the tree compiles and tests pass after every stage. Persisted
field spellings (`conflict_group_id`, …) survive as `#[serde(alias)]` read
support until the owning document's schema bump lands, then are deleted.

**Stage 0 — preconditions.** Green baseline (`cargo test` incl.
`sync_tests`, frontend integration tests). Drain or recover every
conversation journal operation not at `LocalCommitted`. Grep for any
consumer that persists `ConflictGroupId` outside the sync documents
(activity log, git `thread_refs`) to lock the ThreadId derivation rule.

**Stage 1 — mechanical rename, zero behavior.** `ConflictGroupId` →
`ThreadId` (83 refs / 13 files); serialized names unchanged for now.

**Stage 2 — flatten manifest types** (`domain.rs`).
Delete: `ConversationGroup` (2330), `validate_variant_parent_chain` (2382),
`validate_conversation_groups`, and — once the last reference falls —
`VariantId`. Replace `ConversationVariant` (2198) with `ConversationThread`
= same struct minus `variant_id` / `parent_variant_id` / `fork_message_id`,
plus `display_title` and `forked_from: Option<ThreadId>`.
`BundleManifest.conversation_groups` (3034) → `conversations:
BTreeMap<ThreadId, ConversationThread>` behind `BUNDLE_SCHEMA_V6`
(schema 4/5 become read-only inputs to migration).
`conversation_resource_id` (2855) drops the variant component →
`{provider}:conversation:{thread_id}`; new captures write logical paths
`state/conversations/{provider}/{thread_id}/…` (bundle_engine.rs:667 loses
the variant segment). `TombstoneTarget::{ConversationVariant,
ConversationGroup}` (2922) collapse into `ConversationThread { thread_id }`
with canonical key `conversation-thread:{thread_id}`.

**Stage 3 — plan, resolver, and journal shapes.**
- `ConversationPlanActionType`: delete `PromoteLocal`, `PromoteCloud`,
  `TombstoneVariant`, `TombstoneGroup`; split `AddAlternate` into `Fork` /
  `CreateCloud` / `CreateLocal`; add `TombstoneThread` and `ReplaceCloud`.
- `ConversationPlanAction` (2515) and `ConversationPlanActionRequest`:
  `conflict_group_id` → `thread_id`; delete `cloud_variant_id`,
  `local_variant_id`, `expected_preference_revision`;
  `result_local_variant_id` → `result_thread_id`.
- `ConversationSyncOperation` (1764): `conflict_group_id` → `thread_id`;
  delete `source_variant_id`, `expected_preference_revision`,
  `initialize_preference`; `old_variant_id` → `old_thread_id`;
  `result_variant_id` → `result_thread_id`; keep `rekey_committed`
  (`rekeyed_local_variant_id` folds into `result_thread_id`).
- `ConversationStalePin::PreferenceRevision` and its check in
  `validate_conversation_action_pins` deleted.
- `rekey_same_id_divergence` (conversation_sync.rs:520) →
  `rekey_thread_binding`: scope loses variant fields; the whole
  preference-pointer block and `ConversationRekeyResult.preference` go.
- `review_conversation_tombstone` (conversation_sync.rs:344): drop the
  `replacement_primary` parameter, `RejectPrimaryWithoutReplacement`, and
  the group/variant split; `TombstoneDecision` shrinks to
  `{RejectUnapproved, RejectStaleDigest, PreserveModified,
  TombstoneThread}`.
- `recover_unique_compatible_binding`: candidate set becomes all threads;
  logic unchanged; skipped for conversations already bound (forked
  conversations are always bound, never re-attached).

**Stage 4 — bindings document v2** (`persistence.rs`).
`LocalConversationBinding` (1453): `{conflict_group_id, variant_id}` →
`thread_id`. `ConversationBindings` (1571): delete the `preferences` vec and
`LocalConversationPreference` (1544). Bump
`CONVERSATION_BINDINGS_SCHEMA_V2` with an on-load migration (derivation
rule above; preferences discarded). Delete every preference query/mutation
API in `persistence.rs` and their `commands.rs` handlers.

**Stage 5 — engine rewire** (`bundle_engine.rs`, `commands.rs`,
`conversation_materialization.rs`, `chat_history.rs`,
`provider_capture.rs`). Group/variant lookups become thread lookups;
fork-push = publish + rekey, fork-pull = materialize + rekey (per the
sections above); promote paths and variant enumeration deleted.

**Stage 6 — cloud manifest migration 5 → 6** as specified in the Migration
section (one CAS commit; resource IDs, recipe, logical paths, tombstone
keys rewritten; object keys untouched).

**Stage 7 — frontend.** `types.ts` `ThreadSyncEntry`: delete
`conflict_group_id`, `variant_id`, `cloud_primary`, `local_preferred`; add
`forked_from`; update the action-type union. `ThreadVariantReview.tsx`:
delete `groupThreadVariants`, the group section renderer, and
promote/variant labels — flat rows with fork pre-selected. Sweep
`pullReviewModel.ts`, `pullReviewFlow.ts`, `ProjectChatHistoryPage.tsx`,
`ProjectSidebar.tsx`.

**Stage 8 — tests and deletion gates.** Rewrite `sync_tests/thread_variants`
(`conflicts.rs`, `relation_matrix.rs`, `recovery.rs`, `continuation.rs`) —
the conflict matrix collapses to fork + two explicit forces; update
`tests/frontend/project-v3-cutover.integration.test.tsx`. Add the
loop-freedom regression: diverge once, sync twice — exactly one fork thread
after round one, zero new threads and all-`Identical` after round two. Add
a migration fixture: schema-5 manifest with two groups (multi-variant, one
tombstoned) asserting the flattened schema-6 output. Final gates: grep for
`VariantId` / `variant_id`, `preferred_variant` / `preferences`,
`conflict_group` returns hits only inside the migration module and serde
aliases.

## Consequences

1. **Fork push touches nothing local** — no codec rewrite, no quiescence
   gate, no provider writes. Strictly fewer failure modes than today's
   variant publish and than the abandoned converge-local design.
2. **Loop-freedom depends on the rekey.** The binding move is the
   convergence mechanism; it must stay journaled and revision-pinned (it
   already is). Skipping it re-introduces infinite re-forks.
3. **Thread proliferation is user-visible and permanent**: one new thread
   per divergence event, both sides keep everything, no merge-back, no
   preferred-variant pinning. Bounded: post-fork the pair is `Identical`,
   so a given divergence forks exactly once.
4. **Removed UX**: "Make preferred here" / "Make cloud primary". Adopting
   the other side = pull it as its own conversation (or explicit force).
5. **Fork pull leaves the divergent chain unpublished until the next push.**
   A pull-only host keeps it local-only indefinitely — by design, but worth
   surfacing as a "not yet published" badge.
6. **Force push is the first cloud-destructive operation**; keep it behind
   explicit approval + pinned digests, ideally with a content-bearing
   tombstone for recovery.
7. **Other hosts are never disturbed by a fork** — their bindings to `t`
   remain valid; they see `t'` as a new cloud-only thread. No cross-host
   coordination is introduced.
8. **One-way schema migration**; old clients fenced by existing validation.
9. **Net effect**: conflict policy unchanged (fork was already the
   default); deletion is concentrated in the variant/group/preference axis,
   its tombstone split, its UI grouping, and its tests — estimated 1.5–3k
   LOC net removal — while the chain classifier, pins, and journal survive
   intact.

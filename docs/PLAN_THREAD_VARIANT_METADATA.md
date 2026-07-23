# Thread Variant Metadata Plan

## Purpose and precedence

`THREAD_SYNC_CASES.md` defines the Push/Pull matrix for one linear thread.
`PLAN_THREAD_VARIANT_CONTINUATION.md` extends that matrix to multiple
coexisting versions and defines the continuation contract. Neither says which
fields must exist, where each one is persisted, or which are forbidden from
crossing machines.

This plan answers that. It is the metadata specification for the behavior the
other two documents describe.

Precedence on conflict: `THREAD_SYNC_CASES.md` governs the case matrix,
`PLAN_THREAD_VARIANT_CONTINUATION.md` governs the continuation contract, this
document governs field names, placement, and validation.

## What exists today

Three concrete facts in the current code set the shape of the work.

**Conversation identity collides.** Capture keys a Codex conversation as
`codex:session:<session-id>` (`src-tauri/src/project_sync_v3/provider_capture.rs:1947`).
Two divergent transcripts that began life as the same provider session produce
the same `ResourceId`, so the bundle cannot hold both. This is the blocker that
forces a schema revision, not a taste preference.

**No lineage is parsed.** `scan_jsonl_metadata`
(`provider_capture.rs:2493`) extracts exactly two things: a session ID and a
`cwd`. It reads only the *head* of the transcript, bounded by
`MAX_METADATA_LINES = 128` and `MAX_METADATA_BYTES = 1 MiB`
(`provider_capture.rs:24`, `:23`). Prefix-versus-divergence classification
needs the *last* message ID, so a bounded tail read is new work — the existing
scanner cannot be extended in place by raising a limit.

**Conversations carry no metadata.** Every conversation capture site passes
`metadata: BTreeMap::new()` (`provider_capture.rs:1977`, `:2015`, `:2093`).
The channel exists and is validated — `ResourceDescriptor.metadata` allows 256
entries, 256-byte keys, 4096-byte values, no control characters
(`domain.rs:557-570`) — it is simply unused for chat.

## Metadata design rule

Every new field belongs to exactly one of three tiers, and the tier decides
where it lives and what may reference it:

| Tier | Crosses machines | Home |
| --- | --- | --- |
| **Portable** | Yes | Bundle manifest (schema 5) |
| **Machine-local** | No | New local document, `conversation_bindings.json` |
| **Plan-time** | No; expires | Pull/Push plan actions |

A portable field may never contain an absolute path, a provider profile ID, a
credential, or a machine-local thread ID used as identity. `ProjectBinding`
already models this split with `#[serde(skip)]` on resolved provider homes
(`domain.rs:1007-1010`); apply the same discipline here.

## New portable metadata

Add a `conversation_groups` section to `BundleManifest` (`domain.rs:1529`),
alongside the existing `resources`, `files`, and `tombstones` maps.

### ConversationGroup

| Field | Type | Notes |
| --- | --- | --- |
| `conflict_group_id` | `ConflictGroupId` | New `named_id!` type (`domain.rs:67`) |
| `provider` | `Provider` | Existing enum |
| `primary_variant_id` | `VariantId` | Must resolve to a live variant in this group |
| `variants` | `BTreeMap<VariantId, ConversationVariant>` | Non-empty |

The group carries no revision of its own. Concurrency is already handled by
`BundleHead.generation` plus compare-and-swap on head publication
(`s3_store.rs:47`); adding a second CAS domain would be redundant machinery.

### ConversationVariant

| Field | Type | Notes |
| --- | --- | --- |
| `variant_id` | `VariantId` | New `named_id!` type |
| `resource_id` | `ResourceId` | Points at the existing descriptor and file entry |
| `head_message_id` | `MessageId` | New; last message in the transcript |
| `message_count` | `u64` | Cheap pre-check before a full lineage compare |
| `lineage_digest` | `String` (sha256) | Digest over the ordered message-ID list |
| `parent_variant_id` | `Option<VariantId>` | Provenance only; never a merge instruction |
| `fork_message_id` | `Option<MessageId>` | Where this branch left its parent |
| `source_thread_id` | `Option<String>` | Bounded provenance hint; not a binding |
| `origin_replica_label` | `Option<String>` | Display only; no machine identity |
| `created_at` | `u64` | Existing timestamp convention |

**Do not add `transcript_sha256`.** The digest already lives in
`BundleFileEntry.sha256` (`domain.rs:1391`), reached through `resource_id`.
Duplicating it creates a field that can disagree with itself. Manifest
validation should instead cross-check that every variant's `resource_id`
resolves to exactly one file entry.

### Message lineage

`message_id` and `parent_message_id` are parsed from the transcript but are
**not** stored per-message in the manifest — that would make manifests grow
without bound. The manifest holds only the bounded summary above
(`head_message_id`, `message_count`, `lineage_digest`); the full chain stays in
the immutable transcript object and is read on demand when classification needs
more than the summary.

`lineage_digest` equality proves identity. It does not prove prefix: for
`C ≺ L` the classifier must read the candidate transcripts and compare message
IDs positionally. The summary exists to make the common no-op case cheap, not
to replace the comparison.

### Variant tombstones

Extend the existing `TombstoneTarget` enum (`domain.rs:1442`) with:

```text
TombstoneTarget::ConversationVariant { conflict_group_id, variant_id }
```

This reuses `Tombstone.last_sha256`, `deleted_at`, `canonical_key()`, and the
existing tombstone validation and merge paths. `THREAD_SYNC_CASES.md` requires
that absence never imply deletion; the tombstone machinery for that is already
built and needs one new target shape, not a parallel system.

## New machine-local metadata

Store in a **new** document, `conversation_bindings.json`, next to the existing
`machine_projects.json` and `materializations.json` (`persistence.rs:69`, `:73`).

Not inside `MachineProjectState`: that document is capped at
`MAX_BINDINGS = 4096` (`domain.rs:27`) and is rewritten on project and profile
changes. Conversation bindings are an order of magnitude more numerous and
change on every Pull. Sharing one document would couple two very different
write frequencies onto one revision counter.

Reuse the `schema` + `revision` + atomic-save shape that `Materializations`
already uses (`domain.rs:2431-2447`).

### LocalConversationBinding

| Field | Type | Notes |
| --- | --- | --- |
| `replica_id` | `ReplicaId` | Existing type |
| `local_project_id` | `LocalProjectId` | Existing type |
| `provider_profile_id` | `LocalProviderProfileId` | Existing type |
| `conflict_group_id` | `ConflictGroupId` | Portable half of the key |
| `variant_id` | `VariantId` | Portable half of the key |
| `provider_thread_id` | `String` | The real Codex/Claude ID on this machine |
| `last_observed_sha256` | `String` | Detects local edits since last sync |
| `last_observed_head_message_id` | `MessageId` | Detects appends without a full re-read |
| `materialization_state` | `enum { Resumable, StoredOnly }` | Gates whether the UI offers Continue |
| `revision` | `u64` | Pinned by plans, as `ProjectBinding.revision` is (`domain.rs:1012`) |
| `updated_at` | `u64` | |

### LocalConversationPreference

| Field | Type | Notes |
| --- | --- | --- |
| `replica_id`, `local_project_id`, `provider_profile_id` | | Existing types |
| `conflict_group_id` | `ConflictGroupId` | |
| `preferred_variant_id` | `VariantId` | Never changed by ordinary Pull |
| `updated_at` | `u64` | |

### Required uniqueness

Document validation must reject, in the same style as
`MachineProjectState::validate` (`domain.rs:1051`):

- two bindings sharing `(replica, project, profile, provider_thread_id)` —
  this is the injectivity that keeps two visible variants from colliding;
- two bindings sharing `(replica, project, profile, group, variant)`;
- a preference naming a group with no binding on this replica.

## New plan-time metadata

Every conversation action in a Pull or Push plan pins the state it was reviewed
against, and apply revalidates all of it. Add to the action payload:

| Field | Purpose |
| --- | --- |
| `variant_action` | `NoOp \| FastForwardLocal \| KeepLocal \| AddAlternate \| StoreOnly \| PromoteLocal \| ReplaceLocal` |
| `relation` | Classifier output, shown in review (see next section) |
| `pinned_generation`, `pinned_manifest_sha256` | Cloud state at review |
| `pinned_variant_lineage_digest` | The exact variant reviewed |
| `expected_local_digest` | `Option<String>`; `None` asserts local absence |
| `expected_binding_revision` | Rejects a binding changed since review |
| `expected_profile_revision` | Rejects a profile changed since review |
| `proposed_provider_thread_id` | Reserved at plan time so two actions cannot pick one ID |
| `plan_expires_at` | Existing plan-expiry convention |

`MaterializationRecord` already pins `generation`, `commit_id`,
`manifest_sha256`, and `binding_revision` (`domain.rs:2403-2406`). Extend the
receipt to also record the `variant_id` and the `provider_thread_id` actually
allocated, so a repeat Pull can prove idempotence from the receipt.

Add the matching `RestoreActionKind` variants (`domain.rs:1844`) rather than
overloading `SafeFile`; a conversation clone-with-new-ID is not a file copy and
should not inherit file-copy apply logic.

## Case matrix to metadata

This is how `THREAD_SYNC_CASES.md` classification is computed. `relation` is
derived per variant, never persisted:

| Relation | Detected by |
| --- | --- |
| Identical | `lineage_digest` equal |
| Cloud is prefix of local | Cloud `head_message_id` appears in local chain at index `cloud.message_count - 1` |
| Local is prefix of cloud | Mirror of the above |
| Diverged | Both heads absent from the other chain |
| Unknown | Message IDs missing or unparseable on either side |
| Local missing | No binding for `(group, variant)` on this replica |
| Cloud missing | Binding exists, variant absent from manifest, no tombstone |

`Unknown` is a distinct value, not a synonym for `Diverged`, even though both
route to the same default action. Review must be able to tell the user *why* it
is preserving both, and an `Unknown` caused by a parser gap is a bug signal
that a genuine divergence is not.

## Schema cutover

Bundle schema 4 readers would silently misread a variant-addressed
conversation resource, so this is a schema 5 revision with one reviewed
cutover — not a compatibility flag and not a permanent dual writer:

1. Derive a deterministic `conflict_group_id` from bundle ID, provider, and the
   existing stable session identity.
2. Assign the existing linear resource a deterministic initial `variant_id`.
3. Bind the currently present local provider thread ID to that variant.
4. Publish the upgraded manifest through the normal immutable publication and
   head compare-and-swap flow.
5. Read schema-4 manifests as immutable history only. Do not write them again.

`CapturedWith.codec_versions` (`domain.rs:1385`) is the existing place to record
the conversation codec version, so a future reader can tell which lineage
parser produced a variant's summary.

## Implementation sequence

1. Add `ConflictGroupId`, `VariantId`, and `MessageId` via `named_id!`, plus
   the lineage summary type. Pure domain, no filesystem.
2. Add the bounded tail-read to transcript scanning and populate
   `head_message_id`, `message_count`, and `lineage_digest`. This is
   independently testable against fixture transcripts.
3. Add `conversation_groups` to the manifest with strict validation: unique
   variant IDs, primary resolves to a live variant, every `resource_id`
   resolves to exactly one file entry.
4. Add `conversation_bindings.json` with the revision and atomic-save pattern
   and the uniqueness rules above.
5. Replace whole-file conversation comparison with the per-variant relation
   classifier.
6. Add provider clone-with-new-ID staging for Codex, then Claude. Do not mark a
   provider `Resumable` until its full selected file set — transcript,
   structural IDs, sidecars, index rows, `cwd` fields — is inventoried.
7. Add the pinned plan actions and apply-time revalidation.
8. Update Push assembly to update only the captured `variant_id`.
9. Mirror the DTOs into `src/types.ts` and group variants in the History and
   review UI.

Steps 1-3 are additive and land safely before any behavior changes.

## Tests

Beyond the A/cloud/B acceptance scenario in
`PLAN_THREAD_VARIANT_CONTINUATION.md`, the metadata itself needs:

- **Tier leakage.** Assert no serialized portable field contains an absolute
  path, profile ID, or local thread ID used as identity. Worth a test that
  walks the serialized manifest generically, so new fields are covered by
  default rather than by remembering.
- **Digest cross-check.** A manifest whose variant `resource_id` has no file
  entry, or two entries, must fail validation.
- **Prefix is not digest equality.** A transcript with identical rendered text
  but different message IDs must classify as `Diverged`, not `Identical`.
- **Tail-read bounds.** A transcript larger than the scan budget must still
  yield a correct `head_message_id`, or explicitly classify `Unknown` — never a
  wrong head.
- **Binding uniqueness.** Two bindings claiming one `provider_thread_id` in one
  profile must be rejected at document load.
- **Pin rejection.** Mutate each pinned field after review and assert apply
  fails safely for each one individually.
- **Receipt idempotence.** A repeat Pull must reuse the `provider_thread_id`
  recorded in the prior receipt and create no second binding.

## Open decisions

1. **Typed fields versus the existing string metadata map.** Recommendation:
   typed fields on the manifest for the group and variant model, because it
   needs referential validation (`primary_variant_id` must resolve) that a
   `BTreeMap<String, String>` cannot express. Keep the existing metadata map
   (`domain.rs:529`) for bounded provenance hints only.
2. **`lineage_digest` construction.** Recommendation: sha256 over
   newline-joined message IDs in order. It must be defined once and versioned
   through `codec_versions`, since changing it silently reclassifies every
   existing variant.
3. **Group assignment for pre-existing conversations.** The deterministic
   derivation in cutover step 1 assumes one session ID maps to one conversation.
   Confirm this holds for Codex subagent transcripts, which already carry both
   `id` and `session_id` (`provider_capture.rs:2542-2550`).

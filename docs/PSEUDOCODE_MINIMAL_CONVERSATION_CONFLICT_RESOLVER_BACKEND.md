# Pseudocode: Minimal portable-tail Refresh, Push, and Pull

- Status: Proposed
- Date: August 10, 2026
- Normative plan: [PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
- Compatibility: current numeric schema `1` only; no migration, legacy reader,
  legacy writer, or legacy execution path

This document shows the important backend paths for the reduced root-only
schema, shared comparator, and bounded recovery protocols.

## 1. Core types

### 1.1 Portable root tail

```rust
const PORTABLE_TAIL_CODEC: &str = "mallard-codex-portable-tail-v1";

enum TailPartitionKind {
    WholeThread,
    FixedPaginatedTail,
}

// Exact current schema-1 cloud wire shape.
struct PortableConversationTail {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    accepted_end_record_count: u64,
    accepted_tail_state: Sha256Digest,
}

struct PortableRootTailLayout {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
}

// Local observed values use the same equality fields; only the end field name
// differs because an observation is not necessarily accepted.
struct RootTailValue {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    end_record_count: u64,
    tail_state: Sha256Digest,
}

fn as_comparison_value(value: &RootTailValue) -> PortableConversationTail {
    PortableConversationTail {
        codec: value.codec.clone(),
        partition_kind: value.partition_kind,
        start_record_count: value.start_record_count,
        accepted_end_record_count: value.end_record_count,
        accepted_tail_state: value.tail_state,
    }
}

fn validate_portable_tail(value: &PortableConversationTail) -> Result<()> {
    require(value.codec == PORTABLE_TAIL_CODEC)?;
    require(value.start_record_count <= value.accepted_end_record_count)?;
    if value.partition_kind == WholeThread {
        require(value.start_record_count == 0)?;
    }
    // Tail count is derived, never serialized.
    checked_sub(
        value.accepted_end_record_count,
        value.start_record_count,
    )?;
    validate_sha256(value.accepted_tail_state)?;
    Ok(())
}
```

There is no node vector, subagent state, attachment state, prefix witness,
boundary checksum, tail-count field, layout-version integer, or extra hash over
this structure.

### 1.2 Appendable state

```rust
fn lp_tuple(fields: &[EncodedField]) -> Bytes {
    let out = U32_BE(fields.len());
    for field in fields {
        out += U64_BE(field.bytes.len());
        out += field.bytes;
    }
    out
}

fn root_message_fingerprint(message: &CanonicalRootMessage) -> Sha256Digest {
    SHA256(lp_tuple(&[
        utf8("mallard-codex-canonical-root-message-v1"),
        canonical_root_message_bytes_v1(message),
    ]))
}

fn empty_tail_state(layout: &PortableRootTailLayout) -> Sha256Digest {
    SHA256(lp_tuple(&[
        utf8("mallard-codex-portable-tail-start-v1"),
        utf8(&layout.codec),
        partition_tag(layout.partition_kind),
        U64_BE(layout.start_record_count),
    ]))
}

fn append_tail_message(
    prior_state: Sha256Digest,
    prior_tail_count: u64,
    message: &CanonicalRootMessage,
) -> Sha256Digest {
    SHA256(lp_tuple(&[
        utf8("mallard-codex-portable-tail-record-v1"),
        U64_BE(prior_tail_count),
        raw_sha256(prior_state),
        raw_sha256(root_message_fingerprint(message)),
    ]))
}

fn compute_tail_one_shot(
    root: &[CanonicalRootMessage],
    layout: &PortableRootTailLayout,
) -> Result<RootTailValue> {
    require(layout.codec == PORTABLE_TAIL_CODEC)?;
    let start = usize_from_u64(layout.start_record_count)?;
    require(start <= root.len())?;

    let mut state = empty_tail_state(layout);
    for (tail_index, message) in root[start..].iter().enumerate() {
        state = append_tail_message(state, tail_index as u64, message);
    }

    Ok(RootTailValue {
        codec: layout.codec.clone(),
        partition_kind: layout.partition_kind,
        start_record_count: layout.start_record_count,
        end_record_count: root.len() as u64,
        tail_state: state,
    })
}

fn extend_tail(
    prior: &RootTailValue,
    appended: &[CanonicalRootMessage],
) -> Result<RootTailValue> {
    let mut next = prior.clone();
    let mut tail_count = checked_sub(
        next.end_record_count,
        next.start_record_count,
    )?;
    for message in appended {
        next.tail_state = append_tail_message(
            next.tail_state,
            tail_count,
            message,
        );
        tail_count = checked_add(tail_count, 1)?;
        next.end_record_count = checked_add(next.end_record_count, 1)?;
    }
    Ok(next)
}
```

The canonicalizer reads only normalized root conversation messages. It ignores
subagent membership and child lifecycle/control records unless they change a
canonical root message.

### 1.3 Cloud thread and accepted pin

```rust
struct SerializedConversationThread {
    // Existing schema-1 fields retain their current wire names.
    thread_id: ThreadId,
    files: BoundedVec<ConversationFileRef>,
    lineage_codec: String,
    message_count: u64,
    lineage_digest: Sha256Digest,
    semantic_digest: Sha256Digest,

    // New current Codex fields.
    artifact_sha256: Option<Sha256Digest>,
    portable_tail: Option<PortableConversationTail>,
}

struct AcceptedConversationPin {
    portable_tail: PortableConversationTail,
    artifact_sha256: Sha256Digest,
    accepted_at_commit_id: CommitId,
    accepted_at_manifest_sha256: Sha256Digest,
}

fn validate_current_codex_thread(entry: &SerializedConversationThread) -> Result<()> {
    validate_existing_conversation_fields(entry)?;
    validate_sha256(require_some(entry.artifact_sha256.as_ref())?)?;
    validate_portable_tail(require_some(entry.portable_tail.as_ref())?)?;
    Ok(())
}
```

`artifact_sha256` is produced and verified by the standalone artifact layer. The
resolver uses it directly rather than defining another thread-version codec or
canonical file-join hash.

### 1.4 Binding and destination checkpoint

```rust
struct LocalConversationBinding {
    // Existing scope/identity/state fields.
    host_id: HostId,
    replica_id: ReplicaId,
    local_project_id: LocalProjectId,
    provider_profile_id: LocalProviderProfileId,
    thread_id: ThreadId,
    provider_thread_id: Option<String>,
    materialization_state: ConversationMaterializationState,
    revision: u64,
    created_at: u64,
    updated_at: u64,

    // Required for current Codex bindings; absent for providers that do not use
    // this codec.
    tail_layout: Option<PortableRootTailLayout>,
}

struct FileIdentity {
    canonical_path: CanonicalPath,
    device: u64,
    inode_or_file_id: u128,
}

struct PhysicalPosition {
    provider_record_ordinal: u64,
    byte_offset: u64,
}

struct RootPhysicalMapping {
    provider_thread_id: String,
    file_identity: FileIdentity,
    tail_start: PhysicalPosition,
    accepted_end: Option<PhysicalPosition>,
    observed_end: PhysicalPosition,
    safe_eof: PhysicalPosition,
}

struct RawRangePin {
    file_identity: FileIdentity,
    start_byte: u64,
    end_byte: u64,
    raw_sha256: Sha256Digest,
}

enum PhysicalMappingState {
    Ready(RootPhysicalMapping),
    Detached {
        expected_provider_thread_id: String,
        expected_binding_revision: u64,
    },
}

struct ConversationCheckpoint {
    generation: u64,
    accepted: Option<AcceptedConversationPin>,
    observed: RootTailValue,
    physical: PhysicalMappingState,
    raw_read_range_pin: RawRangePin,
    provider_profile_revision: u64,
    binding_revision: u64,
}
```

There is no logical-tail registry, layout provenance, task-layout generation,
aggregate artifact cache, lineage cache, or semantic cache.
The current binding shape also removes the shipping conflict-only `fork_stamp`
field and its native-fork executor; it is not accepted through a default or
compatibility path.

## 2. Strict current schema 1

```rust
fn decode_current_document(
    kind: DocumentKind,
    raw: BoundedBytes,
) -> Result<CurrentDocument> {
    let object = parse_json_rejecting_duplicate_keys(raw)?;
    require(exact_schema_key_for(kind, &object) == 1)?;
    require_current_key_set_with_declared_optional_fields(kind, &object)?;
    let current = deserialize_current_wire_dto_without_aliases_or_compat_defaults(
        kind,
        object,
    )?;
    validate_all_current_nested_shapes(&current)?;
    Ok(current)
}
```

Unknown-field rejection is generic. Do not add fixtures naming keys that existed
only in unshipped design drafts. Explicit removal fixtures are reserved for real
shipping fields, such as the conflict-only `fork_stamp`.

All affected numeric schemas remain `1`. Old writers are excluded; a prior
schema-1 shape fails current exact-shape validation without migration.

## 3. Root discovery, layout seeding, and mapping

### 3.1 First unmanaged discovery

```rust
fn discover_and_bind_codex_root(
    ctx: &Context,
    provider_thread_id: String,
) -> Result<LocalConversationBinding> {
    let source = discover_bounded_root_history_only(ctx, &provider_thread_id)?;
    let root = flatten_and_canonicalize_root(&source.history_segments)?;

    let layout = if let Some(base) = source.append_bearing_leaf.history_base {
        PortableRootTailLayout {
            codec: PORTABLE_TAIL_CODEC.into(),
            partition_kind: FixedPaginatedTail,
            start_record_count:
                map_native_cutoff_to_canonical_root_count(&root, &base)?,
        }
    } else {
        PortableRootTailLayout {
            codec: PORTABLE_TAIL_CODEC.into(),
            partition_kind: WholeThread,
            start_record_count: 0,
        }
    };

    let binding = new_codex_binding(provider_thread_id, layout);

    // Alias and layout become visible in one ConversationBindings replacement.
    mutate_conversation_bindings_atomically(|document| {
        require_binding_absent(document, binding.key())?;
        document.bindings.push(binding.clone());
        Ok(())
    })?;

    Ok(binding)
}
```

This path does not discover subagents and needs no first-discovery journal.

### 3.2 Load or rebuild physical mapping

```rust
fn ensure_ready_mapping(
    binding: &LocalConversationBinding,
    checkpoint: &mut ConversationCheckpoint,
    mode: OperationMode,
) -> Result<&RootPhysicalMapping> {
    let layout = require_some(binding.tail_layout.as_ref())?;

    if let Ready(mapping) = &checkpoint.physical {
        if mapping.provider_thread_id == binding.provider_thread_id?
            && checkpoint.binding_revision == binding.revision
            && validate_current_file_identity(mapping)?
        {
            return Ok(mapping);
        }
    }

    let scanned = bounded_full_root_scan(binding.provider_thread_id?)?;
    let root = flatten_and_canonicalize_root(&scanned.history_segments)?;
    require(layout.start_record_count <= root.len() as u64)?;

    let mapping = map_layout_and_current_end_to_physical_root(
        &root,
        &scanned,
        layout,
        checkpoint.accepted.as_ref(),
    )?;
    checkpoint.observed = compute_tail_one_shot(&root, layout)?;
    checkpoint.raw_read_range_pin = pin_current_root_range(&scanned)?;
    checkpoint.physical = Ready(mapping);
    checkpoint.binding_revision = binding.revision;
    persist_observation_with_generation_cas(checkpoint, mode)?;

    match &checkpoint.physical {
        Ready(mapping) => Ok(mapping),
        Detached { .. } => unreachable(),
    }
}
```

A current root can be mapped even when the destination's accepted cloud branch
is not a physical prefix. In that case `accepted_end` is `None`; the shared
P/L/C comparison still works.

## 4. Shared evidence and comparator

```rust
enum TailPresence<T> {
    Present(T),
    AbsentConfirmed,
    ExpectedMissing,
    Invalid(EvidenceError),
}

struct TailComparisonEvidence {
    pin: TailPresence<PortableConversationTail>,
    local: TailPresence<PortableConversationTail>,
    cloud: TailPresence<PortableConversationTail>,
    successors_compatible_with_pin: bool,
}

enum TailClassification {
    Sync { pin_tail_is_current: bool },
    LocalOutstanding,
    CloudOutstanding,
    Divergent,
    MissingLocal,
    MissingCloud,
    MissingBoth,
    Unknown(EvidenceError),
}

fn compare_conversation_tail(
    e: TailComparisonEvidence,
) -> TailClassification {
    if !e.successors_compatible_with_pin {
        return Unknown(IncompatibleTailLayout);
    }

    match (&e.pin, &e.local, &e.cloud) {
        (AbsentConfirmed, Present(_), AbsentConfirmed) => MissingCloud,
        (AbsentConfirmed, AbsentConfirmed, Present(_)) => MissingLocal,
        (AbsentConfirmed, AbsentConfirmed, AbsentConfirmed) => MissingBoth,

        (Present(_), ExpectedMissing, _)
        | (Present(_), _, ExpectedMissing) =>
            Unknown(ExpectedSideMissing),

        (Invalid(err), _, _)
        | (_, Invalid(err), _)
        | (_, _, Invalid(err)) => Unknown(copy_error(err)),

        (AbsentConfirmed, Present(_), Present(_)) =>
            Unknown(MissingAcceptedPin),

        (Present(p), Present(l), Present(c)) => {
            if l == c {
                Sync { pin_tail_is_current: p == c }
            } else if c == p {
                LocalOutstanding
            } else if l == p {
                CloudOutstanding
            } else {
                Divergent
            }
        }

        _ => Unknown(UnconfirmedPresence),
    }
}
```

`pin_tail_is_current` is tail-value equality only. A newer manifest with the
same root tail—such as a subagent-only snapshot change—remains `Sync` with no
metadata action.

The shared evidence gatherer—not the comparator—verifies:

```rust
fn gather_tail_evidence(
    repository: &Repository,
    binding: Option<&LocalConversationBinding>,
    checkpoint: Option<&ConversationCheckpoint>,
    current_manifest: &VerifiedManifest,
) -> Result<TailComparisonEvidence> {
    let pin = verify_exact_accepted_manifest_witness_or_presence(checkpoint)?;
    let local = verify_local_binding_mapping_and_tail_or_presence(
        binding,
        checkpoint,
    )?;
    let cloud = verify_current_manifest_tail_or_presence(current_manifest)?;

    let compatible = validate_commit_ancestry_scope_and_common_layout(
        repository,
        checkpoint,
        current_manifest,
        &pin,
        &local,
        &cloud,
    )?;

    Ok(TailComparisonEvidence {
        pin,
        local,
        cloud,
        successors_compatible_with_pin: compatible,
    })
}
```

No subagent path, object hash, attachment state, or complete artifact SHA enters
this evidence.

## 5. Refresh

```rust
fn refresh_conversation(
    ctx: &Context,
    key: ConversationDestinationKey,
) -> Result<RefreshRow> {
    reconcile_nonterminal_journal_for(&key)?;

    let binding = load_binding(key.thread_id)?;
    let mut checkpoint = load_or_initialize_checkpoint_from_binding(
        &binding,
        key,
    )?;
    let mapping = ensure_ready_mapping(&binding, &mut checkpoint, Refresh)?;
    let layout = binding.tail_layout?;

    let observation = match layout.partition_kind {
        FixedPaginatedTail => {
            let range = read_stable_newline_terminated_range(
                mapping.file_identity,
                mapping.observed_end,
                current_safe_eof(mapping)?,
            )?;
            let messages = canonicalize_root_messages_only(range.bytes)?;
            let value = extend_tail(&checkpoint.observed, &messages)?;
            ObservedRoot {
                value,
                mapping: mapping.advanced_to(range.safe_end),
                raw_pin: range.raw_pin,
            }
        }
        WholeThread => {
            let scan = bounded_full_root_scan(binding.provider_thread_id?)?;
            let root = flatten_and_canonicalize_root(&scan.history_segments)?;
            ObservedRoot {
                value: compute_tail_one_shot(&root, &layout)?,
                mapping: map_layout_and_current_end_to_physical_root(
                    &root,
                    &scan,
                    &layout,
                    checkpoint.accepted.as_ref(),
                )?,
                raw_pin: pin_complete_root_range(&scan)?,
            }
        }
    };

    restat_open_root_handles_or_busy(&observation)?;

    let current_manifest = load_and_verify_current_manifest_without_objects(key)?;
    let evidence = gather_tail_evidence_with_observation(
        &binding,
        &checkpoint,
        &observation,
        &current_manifest,
    )?;
    let relation = compare_conversation_tail(evidence.clone());

    // Scanning is unlocked. Final persistence must lose to an operation journal
    // reserved after the scan.
    with_persistence_lock(|| {
        require_no_conflicting_nonterminal_journal(&key)?;
        require_exact_binding_revision(binding.revision)?;
        require_exact_checkpoint_generation(checkpoint.generation)?;
        require_exact_sync_config_revision(checkpoint_config_revision)?;
        require_exact_source_identity_and_end(&observation)?;
        persist_observation_only(&key, observation, relation.clone())?;
        Ok(())
    })?;

    Ok(RefreshRow::from(relation))
}
```

Refresh does not promote accepted state, discover subagents, reconstruct output,
or download transcript objects.

## 6. Standalone artifact builder

```rust
struct StandaloneConversationArtifact {
    root: VerifiedOutputFile,
    subagents: BoundedVec<VerifiedOutputFile>,
    artifact_sha256: Sha256Digest,
    root_tail: RootTailValue,
    root_lineage_digest: Sha256Digest,
    root_semantic_digest: Sha256Digest,
}

fn build_standalone_artifact(
    selected_root: &BoundedRootSnapshot,
    mode: ReconstructionMode,
    layout: &PortableRootTailLayout,
) -> Result<StandaloneConversationArtifact> {
    let history_segments = resolve_root_history_chain(selected_root)?;
    let root_records = flatten_root_segments(history_segments)?;

    // Reached only after a root byte-consuming action is selected.
    let subagent_closure = resolve_bounded_current_subagent_closure(
        selected_root,
        &root_records,
    )?;

    let output_records = encode_standalone_paginated_null_base(
        root_records,
        subagent_closure,
        mode,
    )?;
    require_every_authoritative_output_metadata_has_null_history_base(
        &output_records,
    )?;
    require_valid_bounded_jsonl_records_ordinals_and_rewritten_references(
        &output_records,
    )?;

    let root = canonical_root_messages(&output_records.root)?;
    let root_tail = compute_tail_one_shot(&root, layout)?;
    require(root_tail.end_record_count == root.len() as u64)?;

    // The validated typed records are serialized once. Emitted bytes feed the
    // output SHA-256 and private stage directly; there is no output reparse.
    let staged = serialize_hash_and_stage_validated_outputs_once(
        output_records,
    )?;

    Ok(StandaloneConversationArtifact {
        root: staged.root,
        subagents: staged.subagents,
        artifact_sha256: staged.artifact_sha256,
        root_tail,
        root_lineage_digest: derive_root_lineage_digest(&root)?,
        root_semantic_digest: derive_root_semantic_digest(&root)?,
    })
}
```

Subagent bytes contribute to file and artifact integrity only.

## 7. Bounded local document mutations

```rust
enum LocalDocumentKind {
    SyncConfig,
    ConversationBindings,
}

struct DocumentPrecondition {
    document: LocalDocumentKind,
    expected_revision: u64,
    expected_sha256: Sha256Digest,
}

enum TypedEntryMutation {
    InsertBinding {
        key: BindingKey,
        require_absent: bool,
        after: LocalConversationBinding,
    },
    ReplaceBinding {
        key: BindingKey,
        expected_before: LocalConversationBinding,
        after: LocalConversationBinding,
    },
    UpdateCheckpoint {
        key: ConversationDestinationKey,
        expected_before: Option<ConversationCheckpoint>,
        after: ConversationCheckpoint,
    },
    UpdateScopeBase {
        key: ScopeBaseKey,
        expected_before: Option<ScopeBase>,
        after: ScopeBase,
    },
}

struct DocumentMutation {
    precondition: DocumentPrecondition,
    operations: BoundedVec<TypedEntryMutation>,
    expected_after_sha256: Sha256Digest,
}

enum MutationInspection {
    ExactBefore(CurrentDocument),
    ExactAfter,
    ThirdValue,
}

fn inspect_document_mutation(m: &DocumentMutation) -> Result<MutationInspection> {
    let current = load_current_document(m.precondition.document)?;
    let current_sha = canonical_document_sha256(&current)?;

    if current.revision == m.precondition.expected_revision
        && current_sha == m.precondition.expected_sha256
    {
        return Ok(ExactBefore(current));
    }
    if current_sha == m.expected_after_sha256 {
        return Ok(ExactAfter);
    }
    Ok(ThirdValue)
}

fn apply_document_mutation_idempotently(m: &DocumentMutation) -> Result<()> {
    match inspect_document_mutation(m)? {
        ExactAfter => Ok(()),
        ExactBefore(mut current) => {
            apply_typed_entry_mutations(&mut current, &m.operations)?;
            current.revision = checked_add(current.revision, 1)?;
            validate_current_document(&current)?;
            require(canonical_document_sha256(&current)? ==
                m.expected_after_sha256)?;
            atomic_replace_and_fsync(m.precondition.document, &current)
        }
        ThirdValue => Err(RecoveryBlockedCorruption),
    }
}
```

The journal stores only these bounded mutations and exact digests—not Q0/Q1/Q2,
B0/B1, or any other complete document snapshots.

Journal storage is metadata-only and proportional to bounded selected entries
and output-file pins, never transcript bytes. Staged Pull files and immutable
Push objects are the material storage cost; terminal journals use the existing
bounded retention policy.

## 8. Push

### 8.1 Push planning and batch gate

```rust
enum PushAction {
    Publish,
    CreateCloud,
    NoChanges,
    RequirePullMetadata,
    RequirePull,
    Blocked,
    NotPushable,
}

fn push_action(classification: &TailClassification) -> PushAction {
    match classification {
        LocalOutstanding => Publish,
        MissingCloud => CreateCloud,
        Sync { pin_tail_is_current: true } => NoChanges,
        Sync { pin_tail_is_current: false } => RequirePullMetadata,
        CloudOutstanding | Divergent => RequirePull,
        Unknown(_) => Blocked,
        MissingLocal | MissingBoth => NotPushable,
    }
}

struct PushConversationInputPlan {
    key: ConversationDestinationKey,
    classification: TailClassification,
    evidence: TailComparisonEvidence,
    layout: PortableRootTailLayout,
    planned_local_tail: PortableConversationTail,
    source_pins: BoundedVec<ExactSourceRangePin>,
    local_document_preconditions: BoundedVec<DocumentPrecondition>,
}

fn plan_push_batch(selection: Selection) -> Result<PushReviewOutcome> {
    let mut reviewed = Vec::new();

    for key in selection.conversations {
        let observation = refresh_for_planning(key)?;
        let manifest = load_and_verify_current_manifest_without_objects(key)?;
        let evidence = gather_tail_evidence_with_observation(
            observation.binding,
            observation.checkpoint,
            observation.current,
            &manifest,
        )?;
        let classification = compare_conversation_tail(evidence.clone());
        reviewed.push((key, evidence, classification.clone(),
            push_action(&classification)));
    }

    if reviewed.iter().any(|x| matches!(
        x.action,
        RequirePullMetadata | RequirePull | Blocked | NotPushable
    )) {
        return Ok(PushReviewOutcome::NonPublishing(reviewed));
    }

    let publishing = reviewed.iter()
        .filter(|x| matches!(x.action, Publish | CreateCloud));

    if publishing.is_empty() && selection.other_resources.is_empty() {
        return Ok(PushReviewOutcome::NoChanges(reviewed));
    }

    let mut conversation_inputs = Vec::new();
    for item in publishing {
        let binding = load_exact_binding(item.key)?;
        let layout = binding.tail_layout?;
        let pinned = resolve_and_pin_push_source_closure_without_reconstruction(
            item.key,
            &layout,
        )?;
        require(pinned.recomputed_root_tail ==
            item.evidence.local.present_value()?)?;

        conversation_inputs.push(PushConversationInputPlan {
            key: item.key,
            classification: item.classification.clone(),
            evidence: item.evidence.clone(),
            layout,
            planned_local_tail: pinned.recomputed_root_tail,
            source_pins: pinned.source_pins,
            local_document_preconditions:
                capture_push_document_preconditions(item.key)?,
        });
    }

    Ok(build_input_pinned_push_plan_from_verified_current_head(
        reviewed,
        conversation_inputs,
        selection.other_resources,
    )?)
}
```

`Sync(Advanced)` does not invoke Pull from Push. The review says the content is
up to date but local acceptance metadata is behind and must be advanced by Pull.
Planning resolves and hashes source ranges only. It does not call the standalone
artifact builder, create output files, compute output/artifact hashes, or build
the candidate manifest/commit.

### 8.2 Push apply

The HEAD CAS below is the last cloud mutation. The journal bridges only the
possible-successful-CAS to local-checkpoint crash window.

```rust
enum PushPhase {
    CandidateUploaded,
    HeadCommitted,
    LocalCommitted,
    Complete,
    AbortedCasLost,
    RecoveryBlockedCorruption,
}

fn apply_push(plan: PushPlan) -> Result<PushReceipt> {
    require_not_expired(plan.expires_at)?;
    reconcile_touched_journals(&plan.resource_keys)?;

    let current_remote = reobserve_exact_push_remote_metadata(&plan)?;
    let mut artifacts = Vec::new();
    for input in &plan.conversation_inputs {
        // This one streaming pass verifies every planned source identity,
        // cutoff/EOF, and raw range hash while producing the output.
        let artifact = reconstruct_from_exact_source_pins_once(
            &input.source_pins,
            PreserveIdentity,
            &input.layout,
        )?;
        require(as_comparison_value(&artifact.root_tail) ==
            input.planned_local_tail)?;
        artifacts.push((input, artifact));
    }

    for (input, artifact) in &artifacts {
        let evidence = gather_push_evidence_with_reconstructed_local_tail(
            &plan,
            input,
            &current_remote,
            &artifact.root_tail,
        )?;
        let relation = compare_conversation_tail(evidence);
        require(relation == input.classification)?;
        require(matches!(push_action(&relation), Publish | CreateCloud))?;
    }

    let candidate = build_candidate_from_verified_outputs_and_reviewed_head(
        &plan,
        &artifacts,
    )?;
    let local_mutation = build_push_local_mutation_from_candidate(
        &plan,
        &candidate,
        &artifacts,
    )?;
    upload_and_verify_immutable_objects_manifest_and_commit(&candidate)?;

    // This conditional local transaction is immediately before HEAD CAS. A
    // Refresh or config write during upload makes it fail before cloud mutation.
    atomic_create_push_journal_and_reserve_documents_if_exact(
        PushPhase::CandidateUploaded,
        &candidate,
        &local_mutation,
        &plan.conversation_inputs,
        &plan.resource_keys,
    )?;

    require_current_head_and_scope_equal_review(&plan)?;
    match cas_head(plan.parent_commit, candidate.commit_id)? {
        Won => journal_advance(HeadCommitted)?,
        Lost => {
            journal_terminalize_and_release(AbortedCasLost)?;
            return Err(RequirePull);
        }
        Ambiguous => return recover_push(plan.operation_id),
    }

    apply_document_mutation_idempotently(&local_mutation)?;
    journal_advance(LocalCommitted)?;
    journal_terminalize_and_release(Complete)?;
    let receipt = receipt_from_push_journal_id(plan.operation_id)?;
    best_effort_persist_push_receipt(&receipt);
    Ok(receipt)
}
```

### 8.3 Push recovery

```rust
fn recover_push(operation_id: OperationId) -> Result<PushReceipt> {
    with_operation_and_persistence_locks(operation_id, || loop {
        let journal = load_push_journal(operation_id)?;
        match journal.phase {
            CandidateUploaded => {
                match verify_candidate_commit_ancestry(&journal)? {
                    CandidateOrDescendant => journal_advance(HeadCommitted)?,
                    CandidateExcluded => {
                        journal_terminalize_and_release(AbortedCasLost)?;
                    }
                    Unknown => return Err(RecoveryRequired),
                }
            }
            HeadCommitted => {
                if let Err(RecoveryBlockedCorruption) =
                    apply_document_mutation_idempotently(
                        &journal.document_mutation,
                    )
                {
                    journal_advance_and_narrow_gates(
                        RecoveryBlockedCorruption,
                    )?;
                    continue;
                }
                journal_advance(LocalCommitted)?;
            }
            LocalCommitted => {
                journal_terminalize_and_release(Complete)?;
            }
            Complete => {
                idempotently_release_leftover_reservations(operation_id)?;
                return Ok(receipt_from_push_journal(&journal));
            }
            AbortedCasLost => {
                idempotently_release_leftover_reservations(operation_id)?;
                return Err(RequirePull);
            }
            RecoveryBlockedCorruption => {
                return Err(operator_repair_required(&journal));
            }
        }
    })
}
```

## 9. Pull

### 9.1 Action selection

```rust
enum PullAction {
    NoChanges,
    AdvanceAcceptedPin,
    KeepLocalOutstanding,
    MaterializeCloud,
    KeepBoth,
    MaterializeMissingLocal,
    ReportMissingCloud,
    NotFound,
    Blocked,
}

fn pull_action(classification: &TailClassification) -> PullAction {
    match classification {
        Sync { pin_tail_is_current: true } => NoChanges,
        Sync { pin_tail_is_current: false } => AdvanceAcceptedPin,
        LocalOutstanding => KeepLocalOutstanding,
        CloudOutstanding => MaterializeCloud,
        Divergent => KeepBoth,
        MissingLocal => MaterializeMissingLocal,
        MissingCloud => ReportMissingCloud,
        MissingBoth => NotFound,
        Unknown(_) => Blocked,
    }
}
```

### 9.2 Planning

```rust
fn plan_pull(key: ConversationDestinationKey) -> Result<PullReview> {
    reconcile_nonterminal_journal_for(&key)?;

    let local = observe_local_root_for_pull(key)?;
    let remote = load_and_verify_current_manifest_without_objects(key)?;
    let evidence = gather_tail_evidence_with_observation(
        local.binding,
        local.checkpoint,
        local.observation,
        &remote,
    )?;
    let classification = compare_conversation_tail(evidence.clone());
    let action = pull_action(&classification);

    match action {
        NoChanges | ReportMissingCloud | NotFound | Blocked =>
            Ok(PullReview::NonMutating { evidence, classification, action }),

        AdvanceAcceptedPin | KeepLocalOutstanding => {
            let sync_mutation = plan_zero_target_sync_config_mutation(
                key,
                local,
                &remote,
                &classification,
            )?;
            Ok(PullReview::MetadataOnly(ZeroTargetPullPlan {
                evidence,
                classification,
                action,
                sync_config_mutation: sync_mutation,
                reviewed_remote: remote.exact_manifest_expectation(),
                expires_at: bounded_plan_expiry(),
            }))
        }

        MaterializeCloud | MaterializeMissingLocal => {
            let remote_pins = pin_immutable_cloud_snapshot_from_manifest(
                &remote,
            )?;
            let ids = allocate_fixed_provider_ids_cli_first(
                remote_pins.provider_target_shape(),
            )?;
            let target_intent = plan_remote_target_ids_and_paths(
                &remote_pins,
                &ids,
            )?;
            let after_template = plan_materialized_after_template(
                &key,
                &remote,
                &local,
                &target_intent,
            )?;
            Ok(PullReview::Plan(PullPlan::metadata_only_one_target(
                evidence,
                classification,
                action,
                remote_pins,
                target_intent,
                capture_pull_document_preconditions(&key)?,
                after_template,
            )))
        }

        KeepBoth => plan_keep_both(key, local, remote, evidence),
    }
}

fn plan_keep_both(
    key: ConversationDestinationKey,
    local: VerifiedLocalObservation,
    remote_manifest: VerifiedManifest,
    evidence: TailComparisonEvidence,
) -> Result<PullReview> {
    require(compare_conversation_tail(evidence.clone()) == Divergent)?;

    // This proves that P-local is still T's exact current alias, that its
    // observed checkpoint/source pins still represent L, and that no other
    // logical binding claims the alias. It does not reconstruct or edit it.
    let local_rebind = pin_existing_local_rollout_for_unique_rebind(&local)?;
    let remote_pins = pin_immutable_cloud_snapshot_from_manifest(
        &remote_manifest,
    )?;

    let logical_c = allocate_mallard_thread_id_once()?;
    let remote_ids = allocate_fixed_provider_ids_cli_first(
        remote_pins.provider_target_shape(),
    )?;
    let target_intent = plan_remote_target_ids_and_paths(
        &remote_pins,
        &remote_ids,
    )?;

    let bindings_template = plan_keep_both_bindings_template(
        /* T before */ &local.binding,
        /* T after */ binding_for_remote_t_intent(&target_intent),
        /* C after */ binding_for_existing_local_c_without_provider_edit(
            logical_c.clone(),
            &local.binding,
            &local_rebind,
            local.binding.tail_layout.clone(),
        ),
    )?;

    let after_template = plan_keep_both_after_template(
        &key,
        &local,
        &remote_manifest,
        &target_intent,
        &local_rebind,
        &bindings_template,
        logical_c.clone(),
    )?;

    Ok(PullReview::Plan(PullPlan::metadata_only_one_target_keep_both(
        evidence,
        Divergent,
        KeepBoth,
        remote_pins,
        target_intent,
        local_rebind,
        capture_pull_document_preconditions(&key)?,
        after_template,
        logical_c,
    )))
}
```

`pin_immutable_cloud_snapshot_from_manifest` validates only manifest metadata:
the reviewed HEAD/manifest witness, object keys, declared sizes, SHA-256 values,
roles, root tail, and artifact SHA. It does not open a transcript object.
`P-local` is not a target: Divergent planning pins its existing binding,
observed checkpoint, file identity, safe EOF, and raw root range only.

The keep-both after-template says to rekey T's observed checkpoint to C as local
metadata. When apply instantiates it, it copies the observed tail, physical
start/observed end, source identity, and raw range pin; clears the accepted cloud
pin and physical `accepted_end`; and pins C's new binding revision with a fresh
C checkpoint generation. It does not inherit T's checkpoint concurrency
generation or write provider files, `session_meta`, `history_base`, ordinals,
compaction state, subagent references, or provider index/title metadata.

Planning never creates source-retirement records or backup paths. The old bound
rollout and all of its `history_base` dependencies remain untouched after the
alias moves from T to C. If the existing alias cannot be proven safe and unique
to rebind, planning returns a typed conflict instead of constructing a copy.

The current plan and journal shapes use singular target fields:

```rust
struct ExistingLocalRebindPin {
    logical_t: ThreadId,
    provider_thread_id: String,
    binding_revision: u64,
    checkpoint_generation: u64,
    observed: RootTailValue,
    physical: RootPhysicalMapping,
    raw_read_range_pin: RawRangePin,
    source_pins: BoundedVec<ExactSourceRangePin>,
}

struct PullPlan {
    // Reviewed evidence, expiry, and resource keys omitted.
    remote_pins: ImmutableCloudSnapshotPins,
    target_intent: RemoteTargetIntent, // fixed IDs and paths; no output bytes
    existing_local_rebind: Option<ExistingLocalRebindPin>, // Divergent only
    document_preconditions: BoundedVec<DocumentPrecondition>,
    after_template: PullAfterTemplate, // instantiated from verified output
}

struct PullJournal {
    // Phase, immutable cloud pins, and reservations omitted.
    target: StagedTargetGraphPin,
    existing_local_rebind: Option<ExistingLocalRebindPin>,
    bindings_mutation: Option<DocumentMutation>,
    sync_config_mutation: DocumentMutation,
}
```

There is no target vector or target-count branch in the current design. A plan
contains no staged identity, output size/SHA, physical output mapping, or exact
output-dependent document mutation. Apply derives those values once and stores
them in the journal.

```rust
fn retained_source_receipts(journal: &PullJournal) -> Vec<SourceReceipt> {
    journal.old_source_pins.iter().map(|source| SourceReceipt {
        source: source.sanitized_identity(),
        disposition: "retained",
    }).collect()
}
```

The literal receipt value is derived; there is no source-disposition enum,
journal phase, mutable per-source state, backup path, or future-retirement
branch.

### 9.3 Apply

Zero-target Pull needs no journal because it performs one atomic SyncConfig
replacement:

```rust
fn apply_zero_target_pull(plan: ZeroTargetPullPlan) -> Result<PullReceipt> {
    require_not_expired(plan.expires_at)?;
    let current = reobserve_exact_pull_evidence(&plan)?;
    require(compare_conversation_tail(current) == plan.classification)?;
    require(matches!(
        pull_action(&plan.classification),
        AdvanceAcceptedPin | KeepLocalOutstanding
    ))?;
    require_remote_manifest_matches(&plan.reviewed_remote)?;

    with_persistence_lock(|| {
        require_no_conflicting_nonterminal_journal_for_document(SyncConfig)?;
        match inspect_document_mutation(&plan.sync_config_mutation)? {
            ExactBefore(_) => {
                apply_document_mutation_idempotently(
                    &plan.sync_config_mutation,
                )?;
            }
            ExactAfter => {}
            ThirdValue => return Err(StalePlan),
        }
        Ok(receipt_from_zero_target_plan(&plan))
    })
}
```

Materializing Pull has no cloud CAS, but it does need a journal. Provider target
installation, bindings replacement, and SyncConfig replacement cannot share
one filesystem atomic commit.

```rust
enum PullPhase {
    Prepared,
    CommitDecided,
    TargetCommitted,
    BindingsCommitted,
    SyncConfigCommitted,
    Complete,
    AbortedPrecommit,
    AbortedProviderCollision,
    RecoveryBlockedCorruption,
}

fn apply_pull(plan: PullPlan) -> Result<PullReceipt> {
    require_not_expired(plan.expires_at)?;
    reconcile_touched_journals(&plan.resource_keys)?;
    require(compare_conversation_tail(
        reobserve_exact_pull_evidence(&plan)?
    ) == plan.classification)?;

    require_current_head_manifest_and_object_descriptors_match(
        &plan.remote_pins,
    )?;
    require_target_paths_absent(&plan.target_intent)?;
    require_document_preconditions_exact(&plan.document_preconditions)?;
    if let Some(local_rebind) = &plan.existing_local_rebind {
        require_existing_local_rebind_still_safe_unique_and_exact(local_rebind)?;
    }

    // No transcript object or output existed during planning. Each immutable
    // input object is downloaded and SHA-256 verified once here.
    let cloud = download_and_verify_pinned_cloud_objects_once(
        &plan.remote_pins,
    )?;

    // Reconstruct/reidentify once. The builder validates typed output
    // invariants, then serializes once while emitted bytes feed SHA-256 and the
    // private stage.
    let staged = reconstruct_reidentify_validate_hash_and_stage_once(
        &cloud,
        &plan.target_intent,
    )?;
    require(staged.root_tail == plan.remote_pins.portable_tail)?;

    let mutations = instantiate_exact_pull_document_mutations(
        &plan.after_template,
        &staged,
    )?;
    let journal = build_prepared_pull_journal_from_apply(
        &plan,
        staged.exact_identity_size_and_sha_pins(),
        mutations,
    )?;

    if let Err(err) =
        atomic_create_pull_journal_and_reserve_resources_if_documents_exact(
            PullPhase::Prepared,
            &journal,
            &plan.document_preconditions,
        )
    {
        best_effort_remove_private_stage(plan.operation_id);
        return Err(err);
    }

    if let Err(stale) = final_precommit_check(&plan, &staged, &journal) {
        journal_terminalize_and_release(AbortedPrecommit)?;
        best_effort_remove_private_stage(plan.operation_id);
        return Err(stale);
    }
    journal_advance(CommitDecided)?;

    // The uninterrupted path retains the just-verified stage identity/handles;
    // it does not hash or reconstruct the output again.
    if let Err(ForeignTargetCollision) =
        commit_verified_staged_target_no_clobber_idempotently(
            &staged,
            &journal.target,
        )
    {
        record_committed_target_files_as_unbound_orphans()?;
        journal_terminalize_and_release(AbortedProviderCollision)?;
        return Err(ProviderCollision);
    }
    journal_advance(TargetCommitted)?;

    if let Some(binding_mutation) = &journal.bindings_mutation {
        apply_document_mutation_idempotently(binding_mutation)?;
    }
    journal_advance(BindingsCommitted)?;

    apply_document_mutation_idempotently(&journal.sync_config_mutation)?;
    journal_advance(SyncConfigCommitted)?;

    // Old source files are deliberately left in place.
    journal_terminalize_and_release(Complete)?;
    let receipt = receipt_from_pull_journal_id(plan.operation_id)?;
    best_effort_refresh_provider_index();
    best_effort_persist_pull_receipt(&receipt);
    Ok(receipt)
}
```

`final_precommit_check` is repeated after the potentially long download and
reconstruction. It includes exact reviewed HEAD/scope, root source
identities/safe ends/raw ranges, private staged identity/size, target absence,
binding/config preconditions, provider quiescence, and—when Divergent—the proof
that P-local is still T's exact safe and unique alias to move to C. A missing,
oversized, or corrupt cloud object fails before journal creation. After
`CommitDecided`, recovery completes the fixed reviewed result even if HEAD or an
old source later changes.

### 9.4 Recovery

```rust
fn recover_pull(operation_id: OperationId) -> Result<PullReceipt> {
    let mut recovered_target = None;
    with_operation_and_persistence_locks(operation_id, || loop {
        let journal = load_pull_journal(operation_id)?;
        match journal.phase {
            Prepared => {
                recovered_target = match
                    reopen_no_follow_and_rehash_journaled_target_once(
                        &journal.target,
                    )
                {
                    Ok(target) => Some(target),
                    Err(MissingOrCorruptStage) => {
                        journal_terminalize_and_release(AbortedPrecommit)?;
                        continue;
                    }
                    Err(transient) => return Err(transient),
                };
                if final_precommit_check_from_journal(
                    &journal,
                    recovered_target.as_ref().unwrap(),
                ).is_err() {
                    journal_terminalize_and_release(AbortedPrecommit)?;
                } else {
                    journal_advance(CommitDecided)?;
                }
            }

            CommitDecided => {
                let target = match recovered_target.take() {
                    Some(target) => target,
                    None => match
                        reopen_and_rehash_journaled_stage_or_exact_target_once(
                            &journal.target,
                        )
                    {
                        Ok(target) => target,
                        Err(MissingOrCorruptStage) => {
                            journal_advance_and_narrow_gates(
                                RecoveryBlockedCorruption,
                            )?;
                            continue;
                        }
                        Err(transient) => return Err(transient),
                    },
                };
                match commit_recovered_target_no_clobber_idempotently(
                    target,
                    &journal.target,
                ) {
                    Ok(()) => journal_advance(TargetCommitted)?,
                    Err(ForeignTargetCollision) => {
                        record_committed_target_files_as_unbound_orphans()?;
                        journal_terminalize_and_release(
                            AbortedProviderCollision,
                        )?;
                    }
                    Err(transient) => return Err(transient),
                }
            }

            TargetCommitted => {
                if let Some(mutation) = &journal.bindings_mutation {
                    if let Err(RecoveryBlockedCorruption) =
                        apply_document_mutation_idempotently(mutation)
                    {
                        journal_advance_and_narrow_gates(
                            RecoveryBlockedCorruption,
                        )?;
                        continue;
                    }
                }
                journal_advance(BindingsCommitted)?;
            }

            BindingsCommitted => {
                if let Err(RecoveryBlockedCorruption) =
                    apply_document_mutation_idempotently(
                        &journal.sync_config_mutation,
                    )
                {
                    journal_advance_and_narrow_gates(
                        RecoveryBlockedCorruption,
                    )?;
                    continue;
                }
                journal_advance(SyncConfigCommitted)?;
            }

            SyncConfigCommitted => {
                journal_terminalize_and_release(Complete)?;
            }

            Complete => {
                idempotently_release_leftover_reservations(operation_id)?;
                best_effort_remove_staging(operation_id)?;
                return Ok(receipt_from_pull_journal(&journal));
            }

            AbortedPrecommit => {
                idempotently_release_leftover_reservations(operation_id)?;
                best_effort_remove_staging(operation_id)?;
                return Err(StalePlan);
            }

            AbortedProviderCollision => {
                idempotently_release_leftover_reservations(operation_id)?;
                return Err(ProviderCollisionWithOrphans(
                    journal.committed_target_receipts(),
                ));
            }

            RecoveryBlockedCorruption => {
                return Err(operator_repair_required(&journal));
            }
        }
    })
}
```

The apply-side private staging guard removes partial output on every error before
journal creation. Recovery never downloads cloud objects or reconstructs
output. It reopens by no-follow identity and rehashes the journaled stage—or an
already-installed exact target—once per recovery invocation before continuing.

Provider-index and presentation-receipt failures after `SyncConfigCommitted`
are warnings reconstructed from the durable journal. They do not create more
journal phases or keep task/document gates active.

## 10. CLI-first Codex provider ID allocation

Keep the CLI-first allocator exactly as the preferred production path.

```rust
fn allocate_codex_provider_ids(
    count: usize,
    collision_index: &CollisionIndex,
) -> Result<Vec<String>> {
    let temp = create_mallard_owned_temporary_root()?;
    create_empty_directory(temp.join("codex-home"))?;
    create_empty_directory(temp.join("cwd"))?;

    let cli_result = (|| {
        let codex = resolve_supported_absolute_codex_binary()?;
        let mut server = spawn_bounded(
            codex,
            ["app-server", "--stdio"],
            env = sanitized_environment_with(
                "CODEX_HOME",
                temp.join("codex-home"),
            ),
            cwd = temp.join("cwd"),
        )?;

        server.request("initialize", bounded_initialize_params())?;
        server.notify("initialized", {})?;

        let mut ids = Vec::new();
        for _ in 0..count {
            let response = server.request("thread/start", {
                "cwd": temp.join("cwd"),
                "ephemeral": true,
            })?;
            let id = response.result.thread.id;
            require_canonical_lowercase_rfc9562_uuid_v7(&id)?;
            require(!collision_index.contains(&id))?;
            require(!ids.contains(&id))?;
            ids.push(id);
        }

        // Never send turn/start or a model request.
        server.terminate_bounded()?;
        Ok(ids)
    })();

    remove_entire_temporary_root_best_effort(&temp)?;

    match cli_result {
        Ok(ids) => Ok(ids),
        Err(_) => allocate_local_uuid_v7_batch_with_bounded_collision_retry(
            count,
            collision_index,
        ),
    }
}
```

After any CLI resolution/spawn/initialize/protocol/timeout/output/ID failure,
stop using that CLI session and allocate the batch locally. Apply and recovery
only consume IDs serialized into the reviewed plan; they never invoke this
function.

## 11. Critical executable tests

```rust
#[test]
fn nested_history_has_one_root_tail() {
    let root = chain(
        p0([m1, m2]),
        p1(history(p0, after(m2)), [m3]),
        p2(history(p1, after(m3)), [m4, m5]),
    );
    let tail = derive_portable_tail(root).unwrap();
    assert_eq!(tail.start_record_count, 3);
    assert_eq!(tail.accepted_end_record_count, 5);
    assert_eq!(tail.accepted_tail_state, hash_tail([m4, m5]));
}

#[test]
fn subagents_do_not_affect_relation() {
    let a = root_with_subagents([m4, m5], [s4_1, s5_1]);
    let b = root_with_subagents([m4, m5], [s4_2, s5_2, s5_3]);
    assert_ne!(a.artifact_sha256, b.artifact_sha256);
    assert_eq!(a.portable_tail, b.portable_tail);
    assert!(matches!(
        compare_conversation_tail(evidence(P = a.tail, L = a.tail, C = b.tail)),
        Sync { .. }
    ));
}

#[test]
fn both_advanced_prefix_is_conservatively_divergent() {
    let p = hash_tail([m4, m5]);
    let l = hash_tail([m4, m5, m6]);
    let c = hash_tail([m4, m5, m6, m7]);
    assert_eq!(compare_conversation_tail(evidence(p, l, c)), Divergent);
}

#[test]
fn binding_and_layout_are_one_atomic_record() {
    discover_and_bind_codex_root(fixture(), provider_id()).unwrap();
    let binding = load_binding(thread_id()).unwrap();
    assert!(binding.tail_layout.is_some());
    assert_eq!(writes_for_discovery(), [LocalDocumentKind::ConversationBindings]);
}

#[test]
fn bounded_journal_does_not_embed_source_documents() {
    let journal = build_pull_journal(maximum_valid_config_and_bindings()).unwrap();
    assert!(journal.contains_only_preconditions_mutations_and_after_digests());
    assert!(!journal.contains_serialized_sync_config());
    assert!(!journal.contains_serialized_conversation_bindings());
}

#[test]
fn pull_always_retains_old_sources() {
    let before = source_tree_snapshot();
    let receipt = apply_and_recover_every_pull_crash_point().unwrap();
    assert_eq!(source_tree_snapshot_for_old_aliases(), before);
    assert_eq!(receipt.source_dispositions().unique_values(), ["retained"]);
    assert!(!pull_journal_schema_has_source_cleanup_fields());
}

#[test]
fn divergent_pull_reuses_local_rollout_and_builds_one_remote_target() {
    let before = snapshot_bound_local_rollout_and_checkpoint();
    let plan = plan_divergent_pull().unwrap();

    assert_eq!(plan.target_intent.graph_count(), 1);
    assert_eq!(plan.after_template.logical_c_binding.provider_thread_id,
        before.binding.provider_thread_id);
    assert_eq!(plan.after_template.logical_c_binding.tail_layout,
        before.binding.tail_layout);
    assert_eq!(plan.after_template.logical_c_checkpoint.accepted, None);
    assert_eq!(plan.after_template.logical_c_checkpoint.observed,
        before.checkpoint.observed);
    assert_eq!(
        plan.after_template.logical_c_checkpoint.physical.ready()
            .unwrap().accepted_end,
        None,
    );
    assert_eq!(cloud_transcript_downloads_during_planning(), 0);
    assert_eq!(local_snapshot_reconstruction_calls_during_planning(), 0);
    assert_eq!(remote_snapshot_reconstruction_calls_during_planning(), 0);

    apply_pull(plan).unwrap();
    assert_eq!(remote_snapshot_reconstruction_calls_during_apply(), 1);
    assert_eq!(read_exact_provider_rollout(before.provider_path), before.bytes);
    assert_eq!(binding_for_thread(logical_c()).provider_thread_id,
        before.binding.provider_thread_id);
    assert_ne!(binding_for_thread(logical_t()).provider_thread_id,
        before.binding.provider_thread_id);
}

#[test]
fn zero_target_pull_is_one_atomic_config_write_without_a_journal() {
    let plan = plan_sync_advanced_pull().unwrap();
    let receipt = apply_zero_target_pull(plan).unwrap();
    assert_eq!(receipt.target_count(), 0);
    assert_eq!(
        durable_sync_document_writes_for_apply(),
        [LocalDocumentKind::SyncConfig],
    );
    assert!(nonterminal_operation_journals().is_empty());
}

#[test]
fn materializing_pull_plans_metadata_then_builds_once_at_apply() {
    let plan = plan_cloud_outstanding_pull().unwrap();
    assert_eq!(cloud_transcript_downloads_during_planning(), 0);
    assert_eq!(pull_reconstruction_calls_during_planning(), 0);
    assert_eq!(pull_output_hash_passes_during_planning(), 0);
    assert!(!plan.contains_staged_identity_or_output_hash());

    apply_pull(plan).unwrap();
    assert!(each_pinned_cloud_object_was_downloaded_and_hashed_once());
    assert_eq!(pull_reconstruction_calls_during_apply(), 1);
    assert_eq!(typed_output_invariant_validation_passes_during_apply(), 1);
    assert_eq!(pull_output_hash_passes_during_apply(), 1);
    assert_eq!(staged_output_reparse_passes_during_apply(), 0);
    assert_eq!(staged_target_rehash_passes_during_uninterrupted_apply(), 0);
}

#[test]
fn corrupt_cloud_object_fails_before_pull_journal_or_provider_write() {
    let plan = plan_cloud_outstanding_pull_with_corrupt_object().unwrap();
    assert!(matches!(apply_pull(plan), Err(ObjectShaMismatch)));
    assert!(nonterminal_operation_journals().is_empty());
    assert_eq!(provider_target_write_count(), 0);
    assert_eq!(local_document_write_count(), 0);
}

#[test]
fn invalid_reidentified_output_fails_before_serialization_or_journal() {
    let plan = plan_with_injected_inconsistent_rewritten_reference().unwrap();
    assert!(matches!(apply_pull(plan), Err(OutputInvariantViolation)));
    assert_eq!(pull_output_hash_passes_during_apply(), 0);
    assert_eq!(staged_output_reparse_passes_during_apply(), 0);
    assert!(nonterminal_operation_journals().is_empty());
    assert_eq!(provider_target_write_count(), 0);
}

#[test]
fn pull_recovery_rehashes_stage_but_never_downloads_or_reconstructs() {
    let operation = crash_after_prepared_pull_journal();
    reset_io_counters();
    recover_pull(operation).unwrap();
    assert_eq!(cloud_transcript_downloads_during_recovery(), 0);
    assert_eq!(pull_reconstruction_calls_during_recovery(), 0);
    assert_eq!(journaled_target_rehashes_during_recovery(), 1);
}

#[test]
fn push_sync_advanced_requires_pull_metadata() {
    let relation = Sync { pin_tail_is_current: false };
    assert_eq!(push_action(&relation), RequirePullMetadata);
    assert!(!push_planner_invokes_pull_apply());
}

#[test]
fn push_reconstructs_only_during_apply() {
    let plan = plan_local_outstanding_push().unwrap();
    assert_eq!(standalone_reconstruction_calls_during_planning(), 0);
    assert!(plan.contains_source_identities_eofs_raw_hashes_tail_and_head());
    assert!(!plan.contains_output_or_candidate_hashes());

    apply_push(plan).unwrap();
    assert_eq!(standalone_reconstruction_calls_during_apply(), 1);
    assert_eq!(output_hash_passes_during_apply(), 1);
}

#[test]
fn cli_first_id_allocation_precedes_fallback() {
    let fake = fake_codex_app_server_returning_uuid_v7();
    let ids = allocate_codex_provider_ids(2, &empty_collision_index()).unwrap();
    assert_eq!(fake.calls(), [
        "initialize",
        "initialized",
        "thread/start(ephemeral=true)",
        "thread/start(ephemeral=true)",
    ]);
    assert!(!fake.saw("turn/start"));
    assert!(ids.iter().all(is_canonical_uuid_v7));
}
```

Also cover:

- strict current schema-1 rejection without naming unshipped draft keys;
- every split point for one-shot versus incremental tail hashing;
- exact `Ready -> Detached -> Ready` remap behavior;
- Refresh persistence racing journal reservation;
- Push source-pin mismatch before upload and exactly-one apply reconstruction;
- Push CAS win/loss/ambiguous recovery;
- zero-target Pull exact-before/exact-after/stale cases and no-journal behavior;
- one-target materializing Pull crash points, including Divergent alias transfer;
- materializing Pull planning performs zero transcript downloads,
  reconstructions, output hashes, or staging writes;
- uninterrupted Pull apply downloads/verifies each pinned input object once,
  reconstructs once, validates typed output once, serializes/hashes it once,
  and performs no output reparse or rehash;
- missing/corrupt objects and reconstruction failure leave no journal or visible
  mutation; recovery performs no download/reconstruction and rehashes only the
  journaled stage;
- invalid typed output metadata, ordinals, or rewritten references fail before
  serialization/journaling without requiring a staged-output reparse;
- foreign target collision while installing the one remote target graph;
- third-value local document corruption;
- old-source retention after every Pull outcome;
- receipt/index failure after logical commit; and
- CLI missing/spawn/protocol/timeout/EOF/malformed/non-v7 fallback, temporary
  root cleanup, real-home sentinel, and apply/recovery no-remint behavior.

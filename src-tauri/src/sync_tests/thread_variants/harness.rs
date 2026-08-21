use crate::project_sync::bundle_engine::{
    self, BundleBytes, BundleEngine, CapturedBundleFile, HeadToken, PublishBundleRequest,
    PublishExpectation, ScopePublishInput,
};
use crate::project_sync::conversation_codec::{
    analyze_codex_transcript, CodecLineage, CODEX_LINEAGE_CODEC_KEY,
};
use crate::project_sync::domain::{
    cloud_conversation_resource_id, cloud_conversation_transcript_path, ApplyPolicy, BundleId,
    BundleIdentity, BundleKind, BundleManifest, BundleRecipe, BundleScope, BundleSnapshot,
    CapturedWith, CloudThreadId, ConversationFileRef, ConversationFileRole,
    ConversationLineageStatus, ConversationThread, PortableOrigin, PreservedProvenance, Provenance,
    ProviderKey, RecipeEntry, ReplicaId, ResourceDescriptor, ResourceKind, SessionPresentation,
    StorageId, SyncScopeKey,
};
use crate::project_sync::persistence::LocalStateRepository;
use crate::project_sync::storage_core::{LocalObjectStore, ObjectStore};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

pub fn codex() -> ProviderKey {
    ProviderKey::parse("codex").unwrap()
}

pub fn codex_scope_key() -> SyncScopeKey {
    SyncScopeKey::Provider(codex())
}

/// The manifest's Codex provider scope. Post-redesign manifests are scoped,
/// so thread fixtures published by this harness always land here.
pub fn codex_scope(manifest: &BundleManifest) -> &BundleScope {
    manifest
        .provider_scope(&codex())
        .expect("manifest has no Codex provider scope")
}

pub struct VariantWorld {
    _temp: tempfile::TempDir,
    pub store: LocalObjectStore,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub repo_a: LocalStateRepository,
    pub repo_b: LocalStateRepository,
    pub repo_fresh: LocalStateRepository,
}

#[derive(Clone, Debug)]
pub struct PublishedFixture {
    pub published: bool,
    pub snapshot: BundleSnapshot,
    pub head_token: HeadToken,
}

pub fn publish_with_head<S: ObjectStore>(
    engine: &BundleEngine<S>,
    request: PublishBundleRequest,
) -> Result<PublishedFixture, String> {
    let outcome = engine.publish(request)?;
    let (_, head_token) = engine
        .read_head(&outcome.snapshot.head.bundle_id)?
        .ok_or_else(|| "published bundle has no authoritative head".to_string())?;
    if head_token.revision != outcome.snapshot.head.revision
        || head_token.commit_id != outcome.snapshot.head.commit_id
        || head_token.manifest_sha256 != outcome.snapshot.head.manifest_sha256
        || head_token.manifest_size != outcome.snapshot.head.manifest_size
    {
        return Err("published snapshot does not match authoritative head".to_string());
    }
    Ok(PublishedFixture {
        published: outcome.published,
        snapshot: outcome.snapshot,
        head_token,
    })
}

impl VariantWorld {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store_root = temp.path().join("store");
        let home_a = temp.path().join("machine-a");
        let home_b = temp.path().join("machine-b");
        let home_fresh = temp.path().join("machine-fresh");
        for path in [&store_root, &home_a, &home_b, &home_fresh] {
            fs::create_dir_all(path).unwrap();
        }
        let store = LocalObjectStore::open(&store_root).unwrap();
        let repo_a = LocalStateRepository::from_home_dir(&home_a).unwrap();
        let repo_b = LocalStateRepository::from_home_dir(&home_b).unwrap();
        let repo_fresh = LocalStateRepository::from_home_dir(&home_fresh).unwrap();
        repo_a.load_or_create_host_identity().unwrap();
        repo_b.load_or_create_host_identity().unwrap();
        repo_fresh.load_or_create_host_identity().unwrap();
        Self {
            _temp: temp,
            store,
            storage_id: StorageId::parse("storage-variant-tests").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            repo_a,
            repo_b,
            repo_fresh,
        }
    }

    pub fn engine(&self) -> BundleEngine<LocalObjectStore> {
        BundleEngine::open(self.store.clone(), self.storage_id.clone()).unwrap()
    }

    pub fn origin(&self, repo: &LocalStateRepository, replica: &str) -> PortableOrigin {
        let replica_id = ReplicaId::parse(replica).unwrap();
        let (host, replica) = repo
            .derive_origin_tokens(&self.bundle_id, &replica_id)
            .unwrap();
        PortableOrigin {
            origin_host_token: host,
            origin_replica_token: replica,
            shared_label: None,
        }
    }

    pub fn current_head(&self) -> Option<HeadToken> {
        self.engine()
            .read_head(&self.bundle_id)
            .unwrap()
            .map(|(_, token)| token)
    }

    pub fn publish(
        &self,
        threads: &[ThreadSpec],
        publisher: PortableOrigin,
        updated_at: u64,
    ) -> PublishedFixture {
        let engine = self.engine();
        let current = engine.read_head(&self.bundle_id).unwrap();
        let previous = current
            .as_ref()
            .map(|_| engine.inspect(&self.bundle_id).unwrap().manifest);
        let request = publish_request(
            self.bundle_id.clone(),
            threads,
            publisher,
            updated_at,
            previous.as_ref(),
            current
                .map(|(_, token)| PublishExpectation::Match(token))
                .unwrap_or(PublishExpectation::Absent),
        );
        publish_with_head(&engine, request).unwrap()
    }
}

/// One cloud manifest row spec: `thread_id` is the row's cloud identity.
#[derive(Clone, Debug)]
pub struct ThreadSpec {
    pub thread_id: CloudThreadId,
    pub title: String,
    pub source_thread_id: String,
    pub transcript: Vec<u8>,
    pub created_by: Option<PortableOrigin>,
    pub last_updated_by: Option<PortableOrigin>,
    pub forked_from: Option<CloudThreadId>,
    pub preserved: Option<PreservedProvenance>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ThreadSpec {
    pub fn new(
        thread: &str,
        source_thread_id: &str,
        transcript: Vec<u8>,
        origin: PortableOrigin,
        updated_at: u64,
    ) -> Self {
        Self {
            thread_id: CloudThreadId::parse(thread).unwrap(),
            title: "Thread test conversation".to_string(),
            source_thread_id: source_thread_id.to_string(),
            transcript,
            created_by: Some(origin.clone()),
            last_updated_by: Some(origin),
            forked_from: None,
            preserved: None,
            created_at: 1,
            updated_at,
        }
    }
}

/// The reviewed `(revision, content_sha256)` pin for one scope of the
/// previous manifest, or the absent-scope pin when the scope (or the whole
/// manifest) does not exist yet.
pub fn scope_pin(previous: Option<&BundleManifest>, scope_key: &SyncScopeKey) -> (u64, String) {
    previous
        .and_then(|manifest| manifest.scope(scope_key))
        .map(|scope| (scope.revision, scope.content_sha256.clone()))
        .unwrap_or_else(bundle_engine::absent_scope_pin)
}

pub fn harness_captured_with() -> CapturedWith {
    CapturedWith {
        app_version: "integration-test".to_string(),
        provider_versions: BTreeMap::new(),
        codec_versions: BTreeMap::from([(CODEX_LINEAGE_CODEC_KEY.to_string(), 1)]),
    }
}

pub fn publish_request(
    bundle_id: BundleId,
    threads: &[ThreadSpec],
    publisher: PortableOrigin,
    updated_at: u64,
    previous: Option<&BundleManifest>,
    expected_head: PublishExpectation,
) -> PublishBundleRequest {
    let provider = codex();
    let scope_key = codex_scope_key();
    let previous_scope = previous.and_then(|manifest| manifest.scope(&scope_key));
    let mut recipe = BundleRecipe::default();
    let mut resources = BTreeMap::new();
    let mut retained_files = BTreeMap::new();
    let mut captured_files = BTreeMap::new();
    let mut conversations = BTreeMap::new();
    for spec in threads {
        let analysis = analyze_codex_transcript(&spec.transcript);
        let resource_id = cloud_conversation_resource_id(&provider, &spec.thread_id).unwrap();
        let logical_path = cloud_conversation_transcript_path(&provider, &spec.thread_id).unwrap();
        // Post-cutover manifests carry only proven-lineage conversations, so
        // the fixture demands the same of its specs.
        let CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } = analysis.lineage
        else {
            panic!(
                "harness thread '{}' transcript is not canonicalizable",
                spec.thread_id
            );
        };
        let (lineage_status, head_message_id, message_count) = (
            ConversationLineageStatus::Complete,
            chain.last().map(|message| message.message_id.clone()),
            chain.len() as u64,
        );
        let thread = ConversationThread {
            thread_id: spec.thread_id.clone(),
            provider: provider.clone(),
            display_title: Some(spec.title.clone()),
            presentation: SessionPresentation::default(),
            resource_id: resource_id.clone(),
            files: vec![ConversationFileRef {
                logical_path: logical_path.clone(),
                role: ConversationFileRole::Transcript,
            }],
            lineage_status,
            lineage_codec: CODEX_LINEAGE_CODEC_KEY.to_string(),
            head_message_id,
            message_count,
            lineage_digest,
            semantic_digest,
            portable_tail: Some(
                // Cloud thread docs carry the wire-form codec. Harness
                // fixtures publish their transcripts verbatim (no in-cwd
                // content paths), so the folded state is form-identical and
                // only the label distinguishes the columns.
                mallard_domain::compute_portable_conversation_tail_with_codec(
                    mallard_domain::PORTABLE_TAIL_CODEC_WIRE_V1,
                    mallard_domain::TailPartitionKind::WholeThread,
                    0,
                    &chain,
                )
                .expect("harness chains fold to a portable tail"),
            ),
            // The real binary stamp: harness rows flow through the real
            // pull gates, which require fingerprint equality.
            wire_redaction: Some(crate::project_sync::credential_redaction::shared().stamp()),
            forked_from: spec.forked_from.clone(),
            preserved: spec.preserved.clone(),
            source_thread_id: Some(spec.source_thread_id.clone()),
            created_by_origin: spec.created_by.clone(),
            last_updated_by_origin: spec.last_updated_by.clone(),
            created_at: spec.created_at,
            updated_at: spec.updated_at,
        };
        resources.insert(
            resource_id.clone(),
            ResourceDescriptor {
                resource_id: resource_id.clone(),
                kind: ResourceKind::Conversation,
                provider: Some(provider.clone()),
                provider_resource_type: None,
                display_name: spec.title.clone(),
                provenance: Provenance::Unknown,
                apply_policy: ApplyPolicy::SafeFile,
                relative_cwd: Some(".".to_string()),
                codec_version: 1,
                metadata: BTreeMap::new(),
            },
        );
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        let captured = CapturedBundleFile {
            resource_id: resource_id.clone(),
            bytes: spec.transcript.clone(),
            source_mtime: spec.updated_at,
            mode: Some(0o600),
        };
        let digest = sha256(&captured.bytes);
        let retained = previous_scope.and_then(|scope| scope.files.get(&logical_path));
        if let Some(entry) = retained.filter(|entry| {
            entry.resource_id == resource_id
                && entry.sha256 == digest
                && entry.size == captured.bytes.len() as u64
                && entry.source_mtime == captured.source_mtime
                && entry.mode == captured.mode
        }) {
            retained_files.insert(logical_path, entry.clone());
        } else {
            captured_files.insert(logical_path, captured);
        }
        conversations.insert(spec.thread_id.clone(), thread);
    }
    let scope_input = ScopePublishInput {
        recipe,
        captured_with: harness_captured_with(),
        conversations,
        resources,
        retained_files,
        captured_files,
        directories: BTreeMap::new(),
        tombstones: BTreeMap::new(),
    };
    PublishBundleRequest {
        identity: BundleIdentity {
            bundle_id,
            display_name: "Thread test bundle".to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        },
        published_by_origin: publisher,
        reviewed_scope_pins: BTreeMap::from([(scope_key.clone(), scope_pin(previous, &scope_key))]),
        writable_scopes: BTreeMap::from([(scope_key, scope_input)]),
        expected_head,
        updated_at,
    }
}

#[derive(Clone, Debug)]
pub struct TranscriptBuilder {
    // Retained on the builder for suites that re-derive expectations from it.
    #[allow(dead_code)]
    thread_id: String,
    #[allow(dead_code)]
    cwd: String,
    records: Vec<Value>,
    last_message_id: Option<String>,
}

impl TranscriptBuilder {
    pub fn new(thread_id: &str, cwd: &str) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            cwd: cwd.to_string(),
            records: vec![json!({
                "timestamp": "2026-07-22T00:00:00Z",
                "type": "session_meta",
                "payload": {"id": thread_id, "cwd": cwd}
            })],
            last_message_id: None,
        }
    }

    pub fn message(mut self, id: &str, role: &str, text: &str) -> Self {
        let parent = self.last_message_id.clone();
        self.records.push(json!({
            "timestamp": format!("2026-07-22T00:00:{:02}Z", self.records.len()),
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": id,
                "parent_id": parent,
                "role": role,
                "content": [{"type": if role == "user" { "input_text" } else { "output_text" }, "text": text}]
            }
        }));
        self.last_message_id = Some(id.to_string());
        self
    }

    pub fn message_with_parent(
        mut self,
        id: &str,
        parent: Option<&str>,
        role: &str,
        text: &str,
    ) -> Self {
        self.records.push(json!({
            "timestamp": format!("2026-07-22T00:00:{:02}Z", self.records.len()),
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": id,
                "parent_id": parent,
                "role": role,
                "content": [{"type": "input_text", "text": text}]
            }
        }));
        self.last_message_id = Some(id.to_string());
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut output = Vec::new();
        for record in self.records {
            if record == Value::String("__MALFORMED__".to_string()) {
                output.extend_from_slice(b"{malformed-json}\n");
            } else {
                output.extend_from_slice(serde_json::to_string(&record).unwrap().as_bytes());
                output.push(b'\n');
            }
        }
        output
    }
}

/// Test-fixture identity retarget: rewrite the authoritative session id and
/// the structural cwd fields of a fabricated transcript. This reproduces
/// the retired structural rewrite for fixture setup only — production pulls
/// materialize exclusively through the standalone Decode build, and these
/// fixtures carry no in-cwd content paths, so their canonical digests are
/// unchanged by the retarget.
pub fn rewrite_transcript_identity(
    bytes: &[u8],
    source_thread_id: &str,
    destination_thread_id: &str,
    _source_cwd: &str,
    destination_cwd: &str,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(bytes.len());
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let mut value: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|_| "fixture transcript contains malformed JSON".to_string())?;
        let record_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if let Some(payload) = value
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
        {
            if record_type.as_deref() == Some("session_meta") {
                for key in ["id", "thread_id", "threadId", "session_id", "sessionId"] {
                    if payload.get(key).and_then(serde_json::Value::as_str)
                        == Some(source_thread_id)
                    {
                        payload.insert(
                            key.to_string(),
                            serde_json::Value::String(destination_thread_id.to_string()),
                        );
                    }
                }
            }
            if matches!(
                record_type.as_deref(),
                Some("session_meta") | Some("turn_context")
            ) {
                for key in ["cwd", "working_directory", "workingDirectory"] {
                    if payload.contains_key(key) {
                        payload.insert(
                            key.to_string(),
                            serde_json::Value::String(destination_cwd.to_string()),
                        );
                    }
                }
            }
        }
        output.extend_from_slice(
            &serde_json::to_vec(&value)
                .map_err(|error| format!("serialize fixture record: {error}"))?,
        );
        output.push(b'\n');
    }
    Ok(output)
}

pub fn chain(thread_id: &str, suffix: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = TranscriptBuilder::new(thread_id, "/tmp/project");
    for (index, (id, text)) in suffix.iter().enumerate() {
        builder = builder.message(id, if index % 2 == 0 { "user" } else { "assistant" }, text);
    }
    builder.build()
}

pub fn analyzed_chain(
    bytes: &[u8],
) -> Vec<crate::project_sync::domain::CanonicalConversationMessage> {
    analyze_codex_transcript(bytes)
        .lineage
        .chain()
        .unwrap()
        .to_vec()
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn assert_manifest_consistent(world: &VariantWorld) {
    let engine = world.engine();
    let snapshot = engine.inspect(&world.bundle_id).unwrap();
    assert_snapshot_consistent(&engine, &snapshot);
    for (scope_key, scope) in &snapshot.manifest.scopes {
        assert_eq!(
            scope.resources.len(),
            scope.recipe.entries.len(),
            "scope '{scope_key}' recipe and live resources disagree"
        );
    }
    for repository in [&world.repo_a, &world.repo_b, &world.repo_fresh] {
        repository.load_config().unwrap();
        repository.load_bindings().unwrap();
        repository.load_conversation_bindings().unwrap();
        repository.load_conversation_sync_journal().unwrap();
        repository.load_materializations().unwrap();
        let operations = repository.root().join("conversation_operations");
        if operations.exists() {
            assert!(
                fs::read_dir(&operations).unwrap().next().is_none(),
                "repository '{}' retained an unfinished conversation operation",
                repository.root().display()
            );
        }
    }
    for entry in walkdir::WalkDir::new(world.store.local_root().unwrap()).follow_links(false) {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy();
        assert!(
            !name.starts_with(".tmp") && !name.contains(".mallard-stage"),
            "storage retained temporary path '{}'",
            entry.path().display()
        );
    }
}

pub fn assert_snapshot_consistent<S: ObjectStore>(
    engine: &BundleEngine<S>,
    snapshot: &BundleSnapshot,
) {
    snapshot.validate().unwrap();
    let mut bytes = engine.byte_source(snapshot, None, None);
    for (path, entry) in snapshot.manifest.all_files() {
        let body = bytes.body(path).unwrap();
        assert_eq!(body.size(), entry.size);
        assert_eq!(body.sha256(), entry.sha256);
    }
}

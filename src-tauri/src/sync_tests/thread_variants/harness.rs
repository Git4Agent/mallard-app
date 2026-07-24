use crate::project_sync_v3::bundle_engine::{
    BundleEngine, BundleObjectStore, HeadToken, LocalBundleObjectStore, PublishExpectation,
    PublishSchema6Request, Schema6CapturedFile, Schema6PublishOutcome,
};
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, CodecLineage, CODEX_LINEAGE_CODEC_KEY,
};
use crate::project_sync_v3::domain::{
    conversation_resource_id, ApplyPolicy, BundleId, BundleIdentity, BundleKind, BundleRecipe,
    CapturedWith, ConversationFileRef, ConversationFileRole, ConversationLineageStatus,
    ConversationThread, LogicalPath, PortableOrigin, Provenance, Provider, RecipeEntry, ReplicaId,
    ResourceDescriptor, ResourceKind, ResourceScope, StorageId, ThreadId,
};
use crate::project_sync_v3::persistence::V3Repository;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

pub struct VariantWorld {
    _temp: tempfile::TempDir,
    pub store: LocalBundleObjectStore,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub repo_a: V3Repository,
    pub repo_b: V3Repository,
    pub repo_fresh: V3Repository,
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
        let store = LocalBundleObjectStore::open(&store_root).unwrap();
        let repo_a = V3Repository::from_home_dir(&home_a).unwrap();
        let repo_b = V3Repository::from_home_dir(&home_b).unwrap();
        let repo_fresh = V3Repository::from_home_dir(&home_fresh).unwrap();
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

    pub fn engine(&self) -> BundleEngine<LocalBundleObjectStore> {
        BundleEngine::open(self.store.clone(), self.storage_id.clone()).unwrap()
    }

    pub fn origin(&self, repo: &V3Repository, replica: &str) -> PortableOrigin {
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
    ) -> Schema6PublishOutcome {
        let engine = self.engine();
        let current = engine.read_head(&self.bundle_id).unwrap();
        let previous = current
            .as_ref()
            .map(|_| engine.inspect(&self.bundle_id).unwrap().manifest);
        let request = schema6_request(
            self.bundle_id.clone(),
            threads,
            publisher,
            updated_at,
            previous.as_ref(),
            current
                .map(|(_, token)| PublishExpectation::Match(token))
                .unwrap_or(PublishExpectation::Absent),
        );
        engine.publish_schema6(request).unwrap()
    }
}

#[derive(Clone, Debug)]
pub struct ThreadSpec {
    pub thread_id: ThreadId,
    pub title: String,
    pub source_thread_id: String,
    pub transcript: Vec<u8>,
    pub created_by: Option<PortableOrigin>,
    pub last_updated_by: Option<PortableOrigin>,
    pub forked_from: Option<ThreadId>,
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
            thread_id: ThreadId::parse(thread).unwrap(),
            title: "Thread test conversation".to_string(),
            source_thread_id: source_thread_id.to_string(),
            transcript,
            created_by: Some(origin.clone()),
            last_updated_by: Some(origin),
            forked_from: None,
            created_at: 1,
            updated_at,
        }
    }
}

pub fn schema6_request(
    bundle_id: BundleId,
    threads: &[ThreadSpec],
    publisher: PortableOrigin,
    updated_at: u64,
    previous: Option<&crate::project_sync_v3::domain::BundleManifest>,
    expected_head: PublishExpectation,
) -> PublishSchema6Request {
    let mut recipe = BundleRecipe::default();
    let mut resources = BTreeMap::new();
    let mut retained_files = BTreeMap::new();
    let mut captured_files = BTreeMap::new();
    let mut conversations = BTreeMap::new();
    for spec in threads {
        let analysis = analyze_codex_transcript(&spec.transcript);
        let resource_id = conversation_resource_id(Provider::Codex, &spec.thread_id).unwrap();
        let logical_path = LogicalPath::parse(format!(
            "state/conversations/codex/{}/transcript.jsonl",
            spec.thread_id
        ))
        .unwrap();
        let (lineage_status, head_message_id, message_count, lineage_digest, semantic_digest) =
            match analysis.lineage {
                CodecLineage::Complete {
                    chain,
                    lineage_digest,
                    semantic_digest,
                } => (
                    ConversationLineageStatus::Complete,
                    chain.last().map(|message| message.message_id.clone()),
                    Some(chain.len() as u64),
                    Some(lineage_digest),
                    Some(semantic_digest),
                ),
                CodecLineage::Unknown { reason } => (
                    ConversationLineageStatus::Unknown {
                        reason_code: reason.as_str().to_string(),
                    },
                    None,
                    None,
                    None,
                    None,
                ),
            };
        let thread = ConversationThread {
            thread_id: spec.thread_id.clone(),
            provider: Provider::Codex,
            display_title: Some(spec.title.clone()),
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
            forked_from: spec.forked_from.clone(),
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
                kind: ResourceKind::CodexConversation,
                provider: Some(Provider::Codex),
                scope: ResourceScope::ProviderState,
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
        let captured = Schema6CapturedFile {
            resource_id: resource_id.clone(),
            bytes: spec.transcript.clone(),
            source_mtime: spec.updated_at,
            mode: Some(0o600),
        };
        let digest = sha256(&captured.bytes);
        let retained = previous.and_then(|manifest| manifest.files.get(&logical_path));
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
    PublishSchema6Request {
        identity: BundleIdentity {
            bundle_id,
            display_name: "Thread test bundle".to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        },
        recipe,
        captured_with: CapturedWith {
            app_version: "integration-test".to_string(),
            codex_version: Some("fixture-v1".to_string()),
            claude_version: None,
            codec_versions: BTreeMap::from([(CODEX_LINEAGE_CODEC_KEY.to_string(), 1)]),
        },
        published_by_origin: publisher,
        conversations,
        resources,
        retained_files,
        captured_files,
        directories: BTreeMap::new(),
        tombstones: BTreeMap::new(),
        expected_head,
        updated_at,
    }
}

#[derive(Clone, Debug)]
pub struct TranscriptBuilder {
    thread_id: String,
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

pub fn chain(thread_id: &str, suffix: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = TranscriptBuilder::new(thread_id, "/tmp/project");
    for (index, (id, text)) in suffix.iter().enumerate() {
        builder = builder.message(id, if index % 2 == 0 { "user" } else { "assistant" }, text);
    }
    builder.build()
}

pub fn analyzed_chain(
    bytes: &[u8],
) -> Vec<crate::project_sync_v3::domain::CanonicalConversationMessage> {
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
    let fetched = world.engine().fetch(&world.bundle_id).unwrap();
    fetched.snapshot.validate().unwrap();
    for (path, entry) in &fetched.snapshot.manifest.files {
        assert_eq!(sha256(&fetched.files[path]), entry.sha256);
    }
    let live = fetched.snapshot.manifest.resources.len();
    assert_eq!(live, fetched.snapshot.manifest.recipe.entries.len());
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


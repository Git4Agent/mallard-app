use super::*;

fn bundle_id() -> BundleId {
    BundleId::parse("0123456789abcdef0123456789abcdef").unwrap()
}

fn resource_id(value: &str) -> ResourceId {
    ResourceId::parse(value).unwrap()
}

fn provider(key: &str) -> ProviderKey {
    ProviderKey::parse(key).unwrap()
}

fn portable_origin() -> PortableOrigin {
    PortableOrigin {
        origin_host_token: OriginHostToken::parse("a".repeat(64)).unwrap(),
        origin_replica_token: OriginReplicaToken::parse("b".repeat(64)).unwrap(),
        shared_label: None,
    }
}

fn canonical_message(
    id: &str,
    parent: Option<&str>,
    semantic_byte: char,
) -> CanonicalConversationMessage {
    CanonicalConversationMessage {
        message_id: MessageId::parse(id).unwrap(),
        parent_message_id: parent.map(|value| MessageId::parse(value).unwrap()),
        semantic_fingerprint: semantic_byte.to_string().repeat(64),
    }
}

#[test]
fn ids_fail_closed_during_deserialization() {
    assert!(BundleId::parse("../escape").is_err());
    assert!(BundleId::parse("ABCDEF0123456789ABCDEF0123456789").is_err());
    assert!(ResourceId::parse("skill:release").is_ok());
    assert!(ResourceId::parse("Skill/Release").is_err());
    assert!(serde_json::from_str::<BundleId>("\"../../etc/passwd\"").is_err());
}

#[test]
fn provider_keys_are_a_bounded_lowercase_namespace() {
    for valid in ["codex", "claude", "a", "prov1", "my-agent", "co.dex", "a_b"] {
        assert!(ProviderKey::parse(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "",
        "Codex",
        "1codex",
        "co dex",
        "codex/",
        "codex\\",
        "codex:one",
        "a..b",
        ".codex",
        "codex.",
        "codex-",
        "сodex", // Cyrillic lookalike
        &"a".repeat(65),
    ] {
        assert!(ProviderKey::parse(invalid).is_err(), "{invalid}");
    }
    assert!(serde_json::from_str::<ProviderKey>("\"co/dex\"").is_err());
    assert!(ResourceTypeKey::parse("file-history").is_ok());
    assert!(ResourceTypeKey::parse("File").is_err());
}

#[test]
fn sync_scope_keys_have_canonical_string_forms() {
    let project: SyncScopeKey = serde_json::from_str("\"project\"").unwrap();
    assert_eq!(project, SyncScopeKey::Project);
    let codex: SyncScopeKey = serde_json::from_str("\"provider:codex\"").unwrap();
    assert_eq!(codex, SyncScopeKey::Provider(provider("codex")));
    assert_eq!(serde_json::to_string(&codex).unwrap(), "\"provider:codex\"");
    for invalid in ["\"provider:\"", "\"provider:Bad\"", "\"other\"", "\"\""] {
        assert!(
            serde_json::from_str::<SyncScopeKey>(invalid).is_err(),
            "{invalid}"
        );
    }
    // Ordering is deterministic: the project scope sorts before providers.
    let mut scopes = vec![
        SyncScopeKey::Provider(provider("claude")),
        SyncScopeKey::Project,
        SyncScopeKey::Provider(provider("codex")),
    ];
    scopes.sort();
    assert_eq!(scopes[0], SyncScopeKey::Project);
}

#[test]
fn initial_conversation_ids_are_stable_and_bundle_scoped() {
    let first = initial_conversation_thread_id(
        &bundle_id(),
        &provider("codex"),
        "019f742a-a206-7932-876c-9db8d8ce575a",
    )
    .unwrap();
    assert_eq!(
        first,
        initial_conversation_thread_id(
            &bundle_id(),
            &provider("codex"),
            "019f742a-a206-7932-876c-9db8d8ce575a",
        )
        .unwrap()
    );
    assert_ne!(
        first,
        initial_conversation_thread_id(
            &BundleId::parse("11111111111111111111111111111111").unwrap(),
            &provider("codex"),
            "019f742a-a206-7932-876c-9db8d8ce575a",
        )
        .unwrap()
    );
    // Identical provider-native IDs from different providers never collide.
    assert_ne!(
        first,
        initial_conversation_thread_id(
            &bundle_id(),
            &provider("claude"),
            "019f742a-a206-7932-876c-9db8d8ce575a",
        )
        .unwrap()
    );
}

#[test]
fn host_identity_and_scoped_origin_tokens_have_fixed_validated_encodings() {
    assert!(HostId::parse("00112233445566778899aabbccddeeff").is_ok());
    assert!(HostId::parse("00112233445566778899AABBCCDDEEFF").is_err());
    assert!(HostId::parse("short").is_err());

    let host_id = HostId::parse("00112233445566778899aabbccddeeff").unwrap();
    let bundle = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
    let replica = ReplicaId::parse("replica-a").unwrap();
    let host_token = derive_origin_host_token(&host_id, &bundle).unwrap();
    let replica_token = derive_origin_replica_token(&host_id, &bundle, &replica).unwrap();

    // Golden vectors freeze the domain-separated u64-length-prefixed
    // encoding used by every portable-bundle writer.
    assert_eq!(
        host_token.as_str(),
        "a4ab94f1dfb193b63b7b1862aa6c9cabe17ff1c463287f2958281527c75bc62d"
    );
    assert_eq!(
        replica_token.as_str(),
        "0399716f9fe42796b64a32c5763e91334d58017c93f0c6dd56590320530edd07"
    );
    assert_eq!(
        derive_origin_host_token(&host_id, &bundle).unwrap(),
        host_token
    );
    assert_ne!(
        derive_origin_replica_token(&host_id, &bundle, &ReplicaId::parse("replica-b").unwrap())
            .unwrap(),
        replica_token
    );
}

#[test]
fn origin_labels_are_bounded_local_presentation_only() {
    let token = OriginHostToken::parse("7f3a".repeat(16)).unwrap();
    assert_eq!(fallback_origin_host_label(&token), "Host 7F3A");
    let valid = PortableOrigin {
        origin_host_token: token,
        origin_replica_token: OriginReplicaToken::parse("ab".repeat(32)).unwrap(),
        shared_label: Some("Work laptop".to_string()),
    };
    valid.validate().unwrap();
    let mut invalid = valid;
    invalid.shared_label = Some("x".repeat(129));
    assert!(invalid.validate().is_err());
}

#[test]
fn logical_paths_are_portable_and_namespace_bounded() {
    assert!(LogicalPath::parse("project/.agents/skills/release/SKILL.md").is_ok());
    assert!(LogicalPath::parse("state/conversations/claude/thread-a/transcript.jsonl").is_ok());
    for path in [
        "/project/a",
        "project/../secret",
        "project/a\\b",
        "project/CON/file",
        "unknown/file",
        "state",
    ] {
        assert!(LogicalPath::parse(path).is_err(), "{}", path);
    }
}

fn sample_project() -> LocalProjectRegistration {
    LocalProjectRegistration {
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        bundle_id: bundle_id(),
        display_name: "Project A".to_string(),
        local_alias: None,
        repository_fingerprint: None,
        recipe: BundleRecipe::default(),
        revision: 0,
        created_at: 1,
        updated_at: 1,
    }
}

fn sample_storage() -> StorageConfig {
    StorageConfig {
        id: StorageId::parse("personal").unwrap(),
        name: "Personal".to_string(),
        kind: StorageKind::Local,
        bucket: String::new(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        account_id: String::new(),
        s3_endpoint: String::new(),
        region: String::new(),
        local_dir: "/tmp/project-sync-store".to_string(),
        included_default_exclusions: Vec::new(),
        supports_conditional_writes: None,
    }
}

#[test]
fn config_rejects_split_bundle_identity_and_duplicate_cells() {
    let project = sample_project();
    let mut config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: vec![sample_storage()],
        projects: vec![project.clone()],
        links: vec![ProjectStorageLink {
            local_project_id: project.local_project_id.clone(),
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: BundleId::parse("11111111111111111111111111111111").unwrap(),
            recipe: None,
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            pinned: false,
            created_at: 1,
        }],
    };
    assert!(config.validate().unwrap_err().contains("differs"));
    config.links[0].bundle_id = bundle_id();
    config.links.push(config.links[0].clone());
    assert!(config.validate().unwrap_err().contains("duplicate"));
}

#[test]
fn link_scope_bases_are_keyed_by_their_own_scope() {
    let project = sample_project();
    let base = ScopeBase {
        scope: SyncScopeKey::Provider(provider("codex")),
        scope_revision: 1,
        scope_sha256: "a".repeat(64),
        reviewed_in: RepositoryBase {
            revision: 1,
            commit_id: "commit-1".to_string(),
            manifest_sha256: "b".repeat(64),
            manifest_size: 10,
        },
        binding_revision: 0,
        last_pull_at: None,
        last_push_at: Some(5),
    };
    base.validate().unwrap();
    let mut config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: vec![sample_storage()],
        projects: vec![project.clone()],
        links: vec![ProjectStorageLink {
            local_project_id: project.local_project_id.clone(),
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: bundle_id(),
            recipe: None,
            scope_bases: BTreeMap::from([(
                SyncScopeKey::Provider(provider("claude")),
                base.clone(),
            )]),
            project_content_preferences: Default::default(),
            pinned: false,
            created_at: 1,
        }],
    };
    assert!(config
        .validate()
        .unwrap_err()
        .contains("does not match its scope"));
    config.links[0].scope_bases =
        BTreeMap::from([(SyncScopeKey::Provider(provider("codex")), base)]);
    config.validate().unwrap();

    let mut zero_revision = ScopeBase {
        scope: SyncScopeKey::Project,
        scope_revision: 0,
        scope_sha256: "a".repeat(64),
        reviewed_in: RepositoryBase {
            revision: 1,
            commit_id: "commit-1".to_string(),
            manifest_sha256: "b".repeat(64),
            manifest_size: 10,
        },
        binding_revision: 0,
        last_pull_at: None,
        last_push_at: None,
    };
    assert!(zero_revision.validate().is_err());
    zero_revision.scope_revision = 1;
    zero_revision.reviewed_in.manifest_size = 0;
    assert!(zero_revision.validate().is_err());
}

#[test]
fn baseline_contracts_are_all_schema_one() {
    for (name, schema) in [
        ("local config", LOCAL_SCHEMA),
        ("machine projects", MACHINE_PROJECT_SCHEMA),
        ("host identity", HOST_IDENTITY_SCHEMA),
        ("conversation bindings", CONVERSATION_BINDINGS_SCHEMA),
        (
            "conversation sync journal",
            CONVERSATION_SYNC_JOURNAL_SCHEMA,
        ),
        ("conversation push plan", PUSH_PLAN_SCHEMA),
        ("bundle", BUNDLE_SCHEMA),
        ("recipe", RECIPE_SCHEMA),
        ("restore plan", RESTORE_PLAN_SCHEMA),
        ("setup draft", SETUP_DRAFT_SCHEMA),
        ("setup transaction", SETUP_TRANSACTION_SCHEMA),
        (
            "project content preferences",
            PROJECT_CONTENT_PREFERENCES_SCHEMA,
        ),
    ] {
        assert_eq!(schema, 1, "{name} must be schema 1");
    }

    let current = SyncConfig::default();
    current.validate().unwrap();
    for unsupported_schema in [2, 3] {
        let mut unsupported = current.clone();
        unsupported.schema = unsupported_schema;
        let error = unsupported.validate().unwrap_err();
        assert!(
            error.contains("unsupported project-sync schema"),
            "{unsupported_schema}: {error}"
        );
    }
}

#[test]
fn active_bindings_share_a_checkout_only_through_distinct_provider_configs() {
    let project = |id: &str, bundle: &str| LocalProjectRegistration {
        local_project_id: LocalProjectId::parse(id).unwrap(),
        bundle_id: BundleId::parse(bundle).unwrap(),
        display_name: id.to_string(),
        local_alias: None,
        repository_fingerprint: None,
        recipe: BundleRecipe::default(),
        revision: 0,
        created_at: 1,
        updated_at: 1,
    };
    let config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: Vec::new(),
        projects: vec![
            project("project-a", "0123456789abcdef0123456789abcdef"),
            project("project-b", "11111111111111111111111111111111"),
        ],
        links: Vec::new(),
    };
    let profile = |id: &str, path: &str| ProviderProfile {
        profile_id: LocalProviderProfileId::parse(id).unwrap(),
        provider: provider("codex"),
        display_name: id.to_string(),
        path: path.to_string(),
        canonical_path: path.to_string(),
        revision: 0,
        created_at: 1,
        updated_at: 1,
    };
    let binding = |replica: &str, project: &str, bundle: &str, profile: &str| ProjectBinding {
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse(replica).unwrap(),
        local_project_id: LocalProjectId::parse(project).unwrap(),
        bundle_id: BundleId::parse(bundle).unwrap(),
        project_root: "/home/user/projects/health-game".to_string(),
        canonical_project_root: "/home/user/projects/health-game".to_string(),
        profile_ids: BTreeMap::from([(
            provider("codex"),
            LocalProviderProfileId::parse(profile).unwrap(),
        )]),
        state: BindingState::Active,
        revision: 0,
        updated_at: 1,
    };
    let mut machine = MachineProjectState {
        schema: MACHINE_PROJECT_SCHEMA,
        revision: 0,
        profiles: vec![
            profile("profile-main", "/home/user/.codex"),
            profile("profile-alt", "/home/user/conf2/.codex"),
        ],
        bindings: vec![
            binding(
                "replica-a",
                "project-a",
                "0123456789abcdef0123456789abcdef",
                "profile-main",
            ),
            binding(
                "replica-b",
                "project-b",
                "11111111111111111111111111111111",
                "profile-alt",
            ),
        ],
    };
    machine.validate(&config).unwrap();
    machine.bindings[1].profile_ids = BTreeMap::from([(
        provider("codex"),
        LocalProviderProfileId::parse("profile-main").unwrap(),
    )]);
    let error = machine.validate(&config).unwrap_err();
    assert!(error.contains("same provider config"), "{}", error);
}

#[test]
fn bindings_reject_mismatched_profile_provider_keys() {
    let config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: Vec::new(),
        projects: vec![sample_project()],
        links: Vec::new(),
    };
    let mut machine = MachineProjectState {
        schema: MACHINE_PROJECT_SCHEMA,
        revision: 0,
        profiles: vec![ProviderProfile {
            profile_id: LocalProviderProfileId::parse("profile-claude").unwrap(),
            provider: provider("claude"),
            display_name: "Claude".to_string(),
            path: "/home/user/.claude".to_string(),
            canonical_path: "/home/user/.claude".to_string(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        }],
        bindings: vec![ProjectBinding {
            host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: ReplicaId::parse("replica-a").unwrap(),
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: bundle_id(),
            project_root: "/home/user/projects/app".to_string(),
            canonical_project_root: "/home/user/projects/app".to_string(),
            profile_ids: BTreeMap::from([(
                provider("codex"),
                LocalProviderProfileId::parse("profile-claude").unwrap(),
            )]),
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        }],
    };
    let error = machine.validate(&config).unwrap_err();
    assert!(error.contains("assigns"), "{}", error);
    machine.bindings[0].profile_ids = BTreeMap::from([(
        provider("claude"),
        LocalProviderProfileId::parse("profile-claude").unwrap(),
    )]);
    machine.validate(&config).unwrap();
    machine.bindings[0].profile_ids = BTreeMap::new();
    let error = machine.validate(&config).unwrap_err();
    assert!(error.contains("no provider profile"), "{}", error);
}

fn project_content_descriptor(entry_type: &str, relative_path: &str) -> ResourceDescriptor {
    let logical_path = LogicalPath::parse(format!("project/{}", relative_path)).unwrap();
    let (_, resource_id) = project_content_identity(entry_type, &logical_path).unwrap();
    ResourceDescriptor {
        resource_id,
        kind: match entry_type {
            "file" => ResourceKind::ProjectContentFile,
            "dir" => ResourceKind::ProjectContentDirectory,
            _ => panic!("unsupported test entry type"),
        },
        provider: None,
        provider_resource_type: None,
        display_name: relative_path.to_string(),
        provenance: Provenance::ProjectLocal {
            relative_path: relative_path.to_string(),
        },
        apply_policy: ApplyPolicy::ExplicitReview,
        relative_cwd: None,
        codec_version: 1,
        metadata: BTreeMap::new(),
    }
}

fn project_scope_content() -> (
    BundleRecipe,
    BTreeMap<ResourceId, ResourceDescriptor>,
    BTreeMap<LogicalPath, BundleFileEntry>,
    BTreeMap<LogicalPath, BundleDirectoryEntry>,
) {
    let mut recipe = BundleRecipe::default();
    let mut resources = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    for relative_path in ["docs", "docs/specs", "docs/empty"] {
        let descriptor = project_content_descriptor("dir", relative_path);
        let resource_id = descriptor.resource_id.clone();
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::ExplicitReview,
                required: relative_path != "docs/empty",
            },
        );
        resources.insert(resource_id.clone(), descriptor);
        directories.insert(
            LogicalPath::parse(format!("project/{}", relative_path)).unwrap(),
            BundleDirectoryEntry {
                resource_id,
                mode: Some(0o755),
                source_mtime: 1,
            },
        );
    }
    let descriptor = project_content_descriptor("file", "docs/specs/a.md");
    let resource_id = descriptor.resource_id.clone();
    recipe.entries.insert(
        resource_id.clone(),
        RecipeEntry {
            resource_id: resource_id.clone(),
            apply_policy: ApplyPolicy::ExplicitReview,
            required: false,
        },
    );
    resources.insert(resource_id.clone(), descriptor);
    files.insert(
        LogicalPath::parse("project/docs/specs/a.md").unwrap(),
        BundleFileEntry {
            resource_id,
            sha256: "a".repeat(64),
            size: 1,
            source_mtime: 1,
            object_key: "_uploads/upload-1/files/project/docs/specs/a.md".to_string(),
            mode: Some(0o644),
        },
    );
    (recipe, resources, files, directories)
}

fn sealed_project_scope() -> BundleScope {
    let (recipe, resources, files, directories) = project_scope_content();
    BundleScope::sealed(
        &SyncScopeKey::Project,
        1,
        recipe,
        CapturedWith {
            app_version: "test".to_string(),
            provider_versions: BTreeMap::new(),
            codec_versions: BTreeMap::new(),
        },
        BTreeMap::new(),
        resources,
        files,
        directories,
        BTreeMap::new(),
    )
    .unwrap()
}

struct ProviderScopeParts {
    thread_id: ThreadId,
    scope: BundleScope,
}

fn sealed_provider_scope(provider_key: &str, thread_suffix: &str) -> ProviderScopeParts {
    let key = provider(provider_key);
    let thread_id = ThreadId::parse(format!("thread-{}", thread_suffix)).unwrap();
    let resource = conversation_resource_id(&key, &thread_id).unwrap();
    let transcript_path = conversation_transcript_path(&key, &thread_id).unwrap();
    let chain = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
    ];
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain).unwrap();
    let codec = format!("{}.lineage.v1", provider_key);
    let thread = ConversationThread {
        thread_id: thread_id.clone(),
        provider: key.clone(),
        display_title: Some("Conversation".to_string()),
        presentation: SessionPresentation {
            started_at: Some(1),
            ended_at: Some(2),
            relative_cwd: Some(".".to_string()),
            branch: Some("main".to_string()),
            recorded_sha: None,
            metrics: SessionMetrics {
                user_rounds: 1,
                agent_messages: 1,
                tool_calls: 0,
                total_tokens: None,
                complete: true,
            },
        },
        resource_id: resource.clone(),
        files: vec![ConversationFileRef {
            logical_path: transcript_path.clone(),
            role: ConversationFileRole::Transcript,
        }],
        lineage_status: ConversationLineageStatus::Complete,
        lineage_codec: codec.clone(),
        head_message_id: Some(MessageId::parse("m-b").unwrap()),
        message_count: 2,
        lineage_digest,
        semantic_digest,
        forked_from: None,
        source_thread_id: Some("provider-native-thread".to_string()),
        created_by_origin: None,
        last_updated_by_origin: None,
        created_at: 1,
        updated_at: 2,
    };
    let descriptor = ResourceDescriptor {
        resource_id: resource.clone(),
        kind: ResourceKind::Conversation,
        provider: Some(key.clone()),
        provider_resource_type: Some(ResourceTypeKey::parse("session").unwrap()),
        display_name: "Conversation".to_string(),
        provenance: Provenance::Unknown,
        apply_policy: ApplyPolicy::SafeFile,
        relative_cwd: Some(".".to_string()),
        codec_version: 1,
        metadata: BTreeMap::new(),
    };
    let recipe = BundleRecipe {
        schema_version: RECIPE_SCHEMA,
        revision: 1,
        entries: BTreeMap::from([(
            resource.clone(),
            RecipeEntry {
                resource_id: resource.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        )]),
    };
    let scope = BundleScope::sealed(
        &SyncScopeKey::Provider(key.clone()),
        1,
        recipe,
        CapturedWith {
            app_version: "test".to_string(),
            provider_versions: BTreeMap::from([(key.clone(), "test".to_string())]),
            codec_versions: BTreeMap::from([(codec, 1)]),
        },
        BTreeMap::from([(thread_id.clone(), thread)]),
        BTreeMap::from([(resource.clone(), descriptor)]),
        BTreeMap::from([(
            transcript_path,
            BundleFileEntry {
                resource_id: resource,
                sha256: "c".repeat(64),
                size: 10,
                source_mtime: 1,
                object_key: "_objects/".to_string() + &"c".repeat(64),
                mode: Some(0o600),
            },
        )]),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    ProviderScopeParts { thread_id, scope }
}

fn scoped_manifest(scopes: BTreeMap<SyncScopeKey, BundleScope>) -> BundleManifest {
    BundleManifest {
        schema_version: BUNDLE_SCHEMA,
        revision: 1,
        commit_id: "commit-1".to_string(),
        updated_at: 2,
        bundle: BundleIdentity {
            bundle_id: bundle_id(),
            display_name: "Project".to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        },
        published_by_origin: portable_origin(),
        scopes,
    }
}

#[test]
fn mixed_provider_manifest_validates_with_unknown_scopes_preserved() {
    let codex = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let claude = sealed_provider_scope("claude", "11111111111111111111111111111111");
    // A structurally valid scope for a provider this binary has no adapter
    // for must still validate: unknown providers stay visible and preserved.
    let future = sealed_provider_scope("futureprov", "22222222222222222222222222222222");
    let manifest = scoped_manifest(BTreeMap::from([
        (SyncScopeKey::Project, sealed_project_scope()),
        (SyncScopeKey::Provider(provider("codex")), codex.scope),
        (SyncScopeKey::Provider(provider("claude")), claude.scope),
        (SyncScopeKey::Provider(provider("futureprov")), future.scope),
    ]));
    manifest.validate().unwrap();
    assert_eq!(manifest.provider_scopes().count(), 3);
    assert!(manifest
        .find_conversation(&codex.thread_id)
        .is_some_and(|(scope, _)| scope == &SyncScopeKey::Provider(provider("codex"))));

    // Round-trip through serde keeps every scope byte-identical.
    let encoded = serde_json::to_vec(&manifest).unwrap();
    let decoded: BundleManifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, manifest);
    decoded.validate().unwrap();
}

#[test]
fn scope_digests_are_deterministic_and_tamper_evident() {
    let parts = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let scope_key = SyncScopeKey::Provider(provider("codex"));
    let recomputed = parts.scope.compute_content_sha256(&scope_key).unwrap();
    assert_eq!(recomputed, parts.scope.content_sha256);

    // The digest is scope-key-bound: the same content under another key
    // hashes differently.
    let other = parts
        .scope
        .compute_content_sha256(&SyncScopeKey::Provider(provider("claude")))
        .unwrap();
    assert_ne!(other, parts.scope.content_sha256);

    // Tampering with content invalidates the stored digest.
    let mut tampered = parts.scope.clone();
    tampered.files.values_mut().next().unwrap().sha256 = "d".repeat(64);
    let manifest = scoped_manifest(BTreeMap::from([(scope_key, tampered)]));
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("content digest mismatch"));
}

#[test]
fn scopes_reject_cross_scope_and_misplaced_content() {
    // A provider scope cannot hold another provider's conversation.
    let codex = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let manifest = scoped_manifest(BTreeMap::from([(
        SyncScopeKey::Provider(provider("claude")),
        codex.scope.clone(),
    )]));
    assert!(manifest.validate().is_err());

    // The project scope cannot hold conversations at all.
    let manifest = scoped_manifest(BTreeMap::from([(SyncScopeKey::Project, codex.scope)]));
    assert!(manifest.validate().is_err());

    // A provider-scope file outside its namespaces is rejected.
    let parts = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let mut scope = parts.scope;
    let stray = LogicalPath::parse("state/providers/claude/memory/MEMORY.md").unwrap();
    let owner = scope.files.values().next().unwrap().resource_id.clone();
    scope.files.insert(
        stray,
        BundleFileEntry {
            resource_id: owner,
            sha256: "e".repeat(64),
            size: 1,
            source_mtime: 1,
            object_key: "_objects/".to_string() + &"e".repeat(64),
            mode: Some(0o600),
        },
    );
    let scope_key = SyncScopeKey::Provider(provider("codex"));
    scope.content_sha256 = scope.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key, scope)]));
    let error = manifest.validate().unwrap_err();
    assert!(
        error.contains("outside its namespaces") || error.contains("owns undeclared file"),
        "{error}"
    );
}

#[test]
fn duplicate_threads_across_scopes_are_rejected() {
    let codex = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    // Build a claude scope embedding a thread with the same thread id.
    let claude_key = provider("claude");
    let thread_id = codex.thread_id.clone();
    let resource = conversation_resource_id(&claude_key, &thread_id).unwrap();
    let transcript_path = conversation_transcript_path(&claude_key, &thread_id).unwrap();
    let chain = vec![canonical_message("m-a", None, 'a')];
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain).unwrap();
    let thread = ConversationThread {
        thread_id: thread_id.clone(),
        provider: claude_key.clone(),
        display_title: None,
        presentation: SessionPresentation::default(),
        resource_id: resource.clone(),
        files: vec![ConversationFileRef {
            logical_path: transcript_path.clone(),
            role: ConversationFileRole::Transcript,
        }],
        lineage_status: ConversationLineageStatus::Complete,
        lineage_codec: "claude.lineage.v1".to_string(),
        head_message_id: Some(MessageId::parse("m-a").unwrap()),
        message_count: 1,
        lineage_digest,
        semantic_digest,
        forked_from: None,
        source_thread_id: None,
        created_by_origin: None,
        last_updated_by_origin: None,
        created_at: 1,
        updated_at: 1,
    };
    let descriptor = ResourceDescriptor {
        resource_id: resource.clone(),
        kind: ResourceKind::Conversation,
        provider: Some(claude_key.clone()),
        provider_resource_type: None,
        display_name: "Conversation".to_string(),
        provenance: Provenance::Unknown,
        apply_policy: ApplyPolicy::SafeFile,
        relative_cwd: None,
        codec_version: 1,
        metadata: BTreeMap::new(),
    };
    let recipe = BundleRecipe {
        schema_version: RECIPE_SCHEMA,
        revision: 1,
        entries: BTreeMap::from([(
            resource.clone(),
            RecipeEntry {
                resource_id: resource.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        )]),
    };
    let claude_scope = BundleScope::sealed(
        &SyncScopeKey::Provider(claude_key.clone()),
        1,
        recipe,
        CapturedWith {
            app_version: "test".to_string(),
            provider_versions: BTreeMap::new(),
            codec_versions: BTreeMap::from([("claude.lineage.v1".to_string(), 1)]),
        },
        BTreeMap::from([(thread_id, thread)]),
        BTreeMap::from([(resource.clone(), descriptor)]),
        BTreeMap::from([(
            transcript_path,
            BundleFileEntry {
                resource_id: resource,
                sha256: "f".repeat(64),
                size: 5,
                source_mtime: 1,
                object_key: "_objects/".to_string() + &"f".repeat(64),
                mode: Some(0o600),
            },
        )]),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    let manifest = scoped_manifest(BTreeMap::from([
        (SyncScopeKey::Provider(provider("codex")), codex.scope),
        (SyncScopeKey::Provider(claude_key), claude_scope),
    ]));
    assert!(manifest.validate().unwrap_err().contains("multiple scopes"));
}

#[test]
fn scope_continuity_covers_every_decision_row() {
    let base = ScopeBase {
        scope: SyncScopeKey::Provider(provider("codex")),
        scope_revision: 3,
        scope_sha256: "a".repeat(64),
        reviewed_in: RepositoryBase {
            revision: 5,
            commit_id: "commit-5".to_string(),
            manifest_sha256: "b".repeat(64),
            manifest_size: 10,
        },
        binding_revision: 1,
        last_pull_at: None,
        last_push_at: None,
    };
    let digest = "a".repeat(64);
    let other = "c".repeat(64);
    assert_eq!(
        classify_scope_continuity(None, None),
        ScopeContinuity::Fresh
    );
    assert_eq!(
        classify_scope_continuity(None, Some(&base)),
        ScopeContinuity::RemoteMissing { base_revision: 3 }
    );
    assert_eq!(
        classify_scope_continuity(Some((4, &digest)), None),
        ScopeContinuity::Unreviewed { remote_revision: 4 }
    );
    assert_eq!(
        classify_scope_continuity(Some((3, &digest)), Some(&base)),
        ScopeContinuity::InSync { revision: 3 }
    );
    assert_eq!(
        classify_scope_continuity(Some((4, &other)), Some(&base)),
        ScopeContinuity::Advanced {
            base_revision: 3,
            remote_revision: 4
        }
    );
    assert_eq!(
        classify_scope_continuity(Some((3, &other)), Some(&base)),
        ScopeContinuity::Replaced {
            base_revision: 3,
            remote_revision: 3
        }
    );
    assert_eq!(
        classify_scope_continuity(Some((2, &digest)), Some(&base)),
        ScopeContinuity::Replaced {
            base_revision: 3,
            remote_revision: 2
        }
    );
}

#[test]
fn recipe_key_is_identity_and_digest_is_only_version_metadata() {
    let id = resource_id("memo:release");
    let mut recipe = BundleRecipe::default();
    recipe.entries.insert(
        id.clone(),
        RecipeEntry {
            resource_id: id,
            apply_policy: ApplyPolicy::ExplicitReview,
            required: false,
        },
    );
    assert!(recipe.validate().is_ok());
    let encoded = serde_json::to_string(&recipe).unwrap();
    assert!(encoded.contains("memo:release"));
    assert!(!encoded.contains("source_digest"));
}

#[test]
fn project_content_manifest_validates_exact_tree_identity() {
    let manifest = scoped_manifest(BTreeMap::from([(
        SyncScopeKey::Project,
        sealed_project_scope(),
    )]));
    manifest.validate().unwrap();

    let mut unsupported = manifest.clone();
    unsupported.schema_version = BUNDLE_SCHEMA + 1;
    assert!(unsupported.validate().unwrap_err().contains("unsupported"));

    let mut missing_ancestor = manifest.clone();
    {
        let scope = missing_ancestor
            .scopes
            .get_mut(&SyncScopeKey::Project)
            .unwrap();
        let docs = LogicalPath::parse("project/docs").unwrap();
        let docs_resource = scope.directories.remove(&docs).unwrap().resource_id;
        scope.resources.remove(&docs_resource);
        scope.recipe.entries.remove(&docs_resource);
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    assert!(missing_ancestor
        .validate()
        .unwrap_err()
        .contains("lacks directory entry"));

    let mut orphan = manifest.clone();
    {
        let scope = orphan.scopes.get_mut(&SyncScopeKey::Project).unwrap();
        scope
            .files
            .remove(&LogicalPath::parse("project/docs/specs/a.md").unwrap());
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    assert!(orphan
        .validate()
        .unwrap_err()
        .contains("must own exactly one file"));
}

#[test]
fn project_content_tombstones_are_typed_and_path_bound() {
    let mut manifest = scoped_manifest(BTreeMap::from([(
        SyncScopeKey::Project,
        sealed_project_scope(),
    )]));
    {
        let scope = manifest.scopes.get_mut(&SyncScopeKey::Project).unwrap();
        let path = LogicalPath::parse("project/docs/specs/a.md").unwrap();
        let entry = scope.files.remove(&path).unwrap();
        scope.resources.remove(&entry.resource_id);
        scope.recipe.entries.remove(&entry.resource_id);
        let tombstone = Tombstone {
            target: TombstoneTarget::ProjectContentFile {
                resource_id: entry.resource_id,
                logical_path: path,
            },
            last_sha256: Some(entry.sha256),
            deleted_at: 2,
        };
        scope
            .tombstones
            .insert(tombstone.canonical_key(), tombstone);
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    manifest.validate().unwrap();

    // A conversation tombstone can never live in the project scope.
    {
        let scope = manifest.scopes.get_mut(&SyncScopeKey::Project).unwrap();
        let tombstone = Tombstone {
            target: TombstoneTarget::ConversationThread {
                thread_id: ThreadId::parse("thread-a").unwrap(),
            },
            last_sha256: Some("a".repeat(64)),
            deleted_at: 2,
        };
        scope
            .tombstones
            .insert(tombstone.canonical_key(), tombstone);
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("cannot tombstone conversation thread"));
}

#[test]
fn executable_restore_actions_cannot_be_implicitly_approved() {
    let action = RestoreAction {
        action_id: ActionId::parse("action-1").unwrap(),
        resource_id: resource_id("project:file:docs-readme"),
        kind: RestoreActionKind::WriteProjectFile {
            logical_path: LogicalPath::parse("project/docs/README.md").unwrap(),
            mode: 0o644,
            source_mtime: 1,
        },
        target_path: None,
        source_sha256: None,
        expected_target_sha256: None,
        expected_target_mode: None,
        materialized_sha256: None,
        requires_explicit_approval: false,
    };
    assert!(action.validate().unwrap_err().contains("explicit approval"));
}

#[test]
fn conversation_relation_matrix_uses_ids_parentage_and_semantics() {
    let base = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
    ];
    let local = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
        canonical_message("m-local", Some("m-b"), 'c'),
    ];
    let cloud = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
        canonical_message("m-cloud", Some("m-b"), 'd'),
    ];

    assert_eq!(
        classify_conversation_relation(Some(&base), Some(&base)),
        ConversationRelation::Identical
    );
    assert_eq!(
        classify_conversation_relation(Some(&local), Some(&base)),
        ConversationRelation::CloudPrefixOfLocal
    );
    assert_eq!(
        classify_conversation_relation(Some(&base), Some(&cloud)),
        ConversationRelation::LocalPrefixOfCloud
    );
    assert_eq!(
        classify_conversation_relation(Some(&local), Some(&cloud)),
        ConversationRelation::Diverged
    );
    assert_eq!(
        classify_conversation_relation(None, Some(&cloud)),
        ConversationRelation::LocalMissing
    );
    assert_eq!(
        classify_conversation_relation(Some(&local), None),
        ConversationRelation::CloudMissing
    );
    assert_eq!(
        classify_conversation_relation(None, None),
        ConversationRelation::BothMissing
    );

    let same_text_different_id = vec![canonical_message("other", None, 'a')];
    assert_eq!(
        classify_conversation_relation(Some(&base[..1]), Some(&same_text_different_id),),
        ConversationRelation::Diverged
    );

    let mut reused_id = base.clone();
    reused_id[1].semantic_fingerprint = "e".repeat(64);
    assert_eq!(
        classify_conversation_relation(Some(&base), Some(&reused_id)),
        ConversationRelation::Unknown
    );

    let mut broken_parent = base.clone();
    broken_parent[1].parent_message_id = None;
    assert_eq!(
        classify_conversation_relation(Some(&base), Some(&broken_parent)),
        ConversationRelation::Unknown
    );
}

#[test]
fn conversation_digests_separate_lineage_from_semantics() {
    let first = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
    ];
    let mut semantic_change = first.clone();
    semantic_change[1].semantic_fingerprint = "c".repeat(64);
    let different_lineage = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-c", Some("m-a"), 'b'),
    ];

    let first_digests = conversation_chain_digests(&first).unwrap();
    let semantic_digests = conversation_chain_digests(&semantic_change).unwrap();
    let lineage_digests = conversation_chain_digests(&different_lineage).unwrap();
    assert_eq!(first_digests.0, semantic_digests.0);
    assert_ne!(first_digests.1, semantic_digests.1);
    assert_ne!(first_digests.0, lineage_digests.0);
    assert_ne!(first_digests.1, lineage_digests.1);
}

fn refresh_meta(chain: &[CanonicalConversationMessage]) -> ((String, String), u64) {
    (
        conversation_chain_digests(chain).unwrap(),
        chain.len() as u64,
    )
}

fn cloud_meta<'a>(pair: &'a (String, String), count: u64) -> RefreshCloudMeta<'a> {
    RefreshCloudMeta {
        message_count: count,
        lineage_digest: &pair.0,
        semantic_digest: &pair.1,
    }
}

/// Every row of the decision table in
/// docs/DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md, straight down the table.
#[test]
fn manifest_only_refresh_covers_every_decision_row() {
    let shared = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
    ];
    let extended = {
        let mut chain = shared.clone();
        chain.push(canonical_message("m-c", Some("m-b"), 'c'));
        chain
    };
    let (shared_pair, shared_count) = refresh_meta(&shared);
    let (extended_pair, extended_count) = refresh_meta(&extended);
    let branched = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-x", Some("m-a"), '4'),
        canonical_message("m-y", Some("m-x"), '5'),
    ];
    let (branched_pair, _) = refresh_meta(&branched);

    assert_eq!(
        classify_thread_refresh(RefreshLocalState::Missing, None, None),
        ConversationRelation::BothMissing
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Missing,
            Some(cloud_meta(&shared_pair, shared_count)),
            None
        ),
        ConversationRelation::LocalMissing
    );
    assert_eq!(
        classify_thread_refresh(RefreshLocalState::Chain(&shared), None, None),
        ConversationRelation::CloudMissing
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Uncapturable,
            Some(cloud_meta(&shared_pair, shared_count)),
            None
        ),
        ConversationRelation::Unknown
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&shared),
            Some(cloud_meta(&shared_pair, shared_count)),
            None
        ),
        ConversationRelation::Identical
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&extended),
            Some(cloud_meta(&shared_pair, shared_count)),
            None
        ),
        ConversationRelation::CloudPrefixOfLocal
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&branched),
            Some(cloud_meta(&shared_pair, shared_count)),
            None
        ),
        ConversationRelation::Diverged
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&shared),
            Some(cloud_meta(&extended_pair, extended_count)),
            Some((&shared_pair.0, &shared_pair.1))
        ),
        ConversationRelation::LocalPrefixOfCloud
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&shared),
            Some(cloud_meta(&extended_pair, extended_count)),
            Some((&branched_pair.0, &branched_pair.1))
        ),
        ConversationRelation::Diverged
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&shared),
            Some(cloud_meta(&extended_pair, extended_count)),
            None
        ),
        ConversationRelation::Diverged
    );
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&branched),
            Some(cloud_meta(&extended_pair, extended_count)),
            None
        ),
        ConversationRelation::Diverged
    );
}

/// The loss scenarios that defeated the timestamp- and position-based
/// refresh designs, kept as regressions so they cannot return.
#[test]
fn manifest_only_refresh_defeats_the_recorded_loss_scenarios() {
    let synced = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
        canonical_message("m-c", Some("m-b"), 'c'),
    ];
    let (synced_pair, _) = refresh_meta(&synced);
    let cloud_extended = {
        let mut chain = synced.clone();
        chain.push(canonical_message("m-d", Some("m-c"), 'd'));
        chain.push(canonical_message("m-e", Some("m-d"), 'e'));
        chain
    };
    let (cloud_pair, cloud_count) = refresh_meta(&cloud_extended);

    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&synced),
            Some(cloud_meta(&cloud_pair, cloud_count)),
            Some((&synced_pair.0, &synced_pair.1))
        ),
        ConversationRelation::LocalPrefixOfCloud
    );

    let regrown = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
        canonical_message("m-r", Some("m-b"), '7'),
    ];
    assert_eq!(refresh_meta(&regrown).1, synced.len() as u64);
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&regrown),
            Some(cloud_meta(&cloud_pair, cloud_count)),
            Some((&synced_pair.0, &synced_pair.1))
        ),
        ConversationRelation::Diverged
    );

    let truncated = &synced[..2];
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(truncated),
            Some(cloud_meta(&cloud_pair, cloud_count)),
            Some((&synced_pair.0, &synced_pair.1))
        ),
        ConversationRelation::Diverged
    );

    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&synced),
            Some(cloud_meta(&cloud_pair, cloud_count)),
            None
        ),
        ConversationRelation::Diverged
    );

    let replaced = vec![
        canonical_message("m-a", None, 'a'),
        canonical_message("m-b", Some("m-a"), 'b'),
        canonical_message("m-z", Some("m-b"), '9'),
    ];
    let (replaced_pair, replaced_count) = refresh_meta(&replaced);
    assert_eq!(
        classify_thread_refresh(
            RefreshLocalState::Chain(&synced),
            Some(cloud_meta(&replaced_pair, replaced_count)),
            Some((&synced_pair.0, &synced_pair.1))
        ),
        ConversationRelation::Diverged
    );
}

#[test]
fn manifest_rejects_conversations_without_refresh_metadata() {
    let parts = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let scope_key = SyncScopeKey::Provider(provider("codex"));

    let mut unproven = parts.scope.clone();
    unproven
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .lineage_status = ConversationLineageStatus::Unknown {
        reason_code: "missing-parent".to_string(),
    };
    unproven.content_sha256 = unproven.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), unproven)]));
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("unproven lineage"));

    let mut empty = parts.scope.clone();
    empty
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .message_count = 0;
    empty.content_sha256 = empty.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), empty)]));
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("incomplete lineage metadata"));

    let mut malformed = parts.scope;
    malformed
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .lineage_digest = "not-a-digest".to_string();
    malformed.content_sha256 = malformed.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key, malformed)]));
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("not a lowercase SHA-256"));
}

#[test]
fn manifest_enforces_conversation_referential_integrity() {
    let parts = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let scope_key = SyncScopeKey::Provider(provider("codex"));

    let mut missing_file = parts.scope.clone();
    missing_file.files.clear();
    missing_file.content_sha256 = missing_file.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), missing_file)]));
    assert!(manifest.validate().unwrap_err().contains("missing file"));

    let mut wrong_provider = parts.scope.clone();
    wrong_provider
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .provider = provider("claude");
    wrong_provider.content_sha256 = wrong_provider.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), wrong_provider)]));
    assert!(manifest.validate().is_err());

    let mut duplicate_transcript = parts.scope.clone();
    {
        let thread = duplicate_transcript
            .conversations
            .values_mut()
            .next()
            .unwrap();
        let duplicate = thread.files[0].clone();
        thread.files.push(duplicate);
    }
    duplicate_transcript.content_sha256 = duplicate_transcript
        .compute_content_sha256(&scope_key)
        .unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), duplicate_transcript)]));
    assert!(manifest.validate().unwrap_err().contains("repeats file"));

    let mut self_fork = parts.scope;
    {
        let thread = self_fork.conversations.values_mut().next().unwrap();
        thread.forked_from = Some(thread.thread_id.clone());
    }
    self_fork.content_sha256 = self_fork.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key, self_fork)]));
    assert!(manifest
        .validate()
        .unwrap_err()
        .contains("cannot fork from itself"));
}

#[test]
fn conversation_sidecars_stay_inside_their_thread_namespace() {
    let key = provider("claude");
    let parts = sealed_provider_scope("claude", "11111111111111111111111111111111");
    let scope_key = SyncScopeKey::Provider(key.clone());
    let mut scope = parts.scope;
    let sidecar_path =
        conversation_sidecar_path(&key, &parts.thread_id, "todos", "todo.json").unwrap();
    let owner = scope.files.values().next().unwrap().resource_id.clone();
    {
        let thread = scope.conversations.values_mut().next().unwrap();
        thread.files.push(ConversationFileRef {
            logical_path: sidecar_path.clone(),
            role: ConversationFileRole::Sidecar {
                sidecar_kind: "todos".to_string(),
            },
        });
    }
    scope.files.insert(
        sidecar_path,
        BundleFileEntry {
            resource_id: owner.clone(),
            sha256: "d".repeat(64),
            size: 2,
            source_mtime: 1,
            object_key: "_objects/".to_string() + &"d".repeat(64),
            mode: Some(0o600),
        },
    );
    scope.content_sha256 = scope.compute_content_sha256(&scope_key).unwrap();
    scoped_manifest(BTreeMap::from([(scope_key.clone(), scope.clone())]))
        .validate()
        .unwrap();

    // A sidecar role declared under the wrong kind directory is rejected.
    {
        let thread = scope.conversations.values_mut().next().unwrap();
        let sidecar = thread.files.last_mut().unwrap();
        sidecar.role = ConversationFileRole::Sidecar {
            sidecar_kind: "file-history".to_string(),
        };
    }
    scope.content_sha256 = scope.compute_content_sha256(&scope_key).unwrap();
    assert!(scoped_manifest(BTreeMap::from([(scope_key, scope)]))
        .validate()
        .unwrap_err()
        .contains("sidecar namespace"));
}

fn scope_pin_absent() -> (u64, String) {
    (0, "0".repeat(64))
}

fn push_action(
    action_id: &str,
    provider_key: &str,
    profile: &str,
    thread_suffix: &str,
) -> ConversationPlanAction {
    let (scope_revision, scope_sha256) = scope_pin_absent();
    ConversationPlanAction {
        action_id: ActionId::parse(action_id).unwrap(),
        direction: ConversationPlanDirection::Push,
        action: ConversationPlanActionType::CreateCloud,
        reason_code: ConversationReasonCode::CloudMissing,
        relation: ConversationRelation::CloudMissing,
        provider: provider(provider_key),
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse(profile).unwrap(),
        thread_id: ThreadId::parse(format!("thread-{}", thread_suffix)).unwrap(),
        divergence: None,
        source_resource_id: None,
        proposed_provider_thread_id: None,
        required_codec: format!("{}.lineage.v1", provider_key),
        pinned_revision: 0,
        pinned_commit_id: "absent".to_string(),
        pinned_manifest_sha256: "0".repeat(64),
        pinned_scope_revision: scope_revision,
        pinned_scope_sha256: scope_sha256,
        pinned_lineage_digest: None,
        pinned_semantic_digest: None,
        expected_local_target: TargetExpectation::Sha256("a".repeat(64)),
        expected_provider_state: Vec::new(),
        expected_profile_revision: 1,
        expected_project_binding_revision: 2,
        expected_host_identity_revision: 3,
        expected_bindings_document_revision: 4,
        expected_binding_revision: None,
        expires_at: 10,
    }
}

#[test]
fn push_plans_carry_per_provider_pins() {
    let plan = ConversationPushPlan {
        schema_version: PUSH_PLAN_SCHEMA,
        plan_id: PlanId::parse("plan-1").unwrap(),
        storage_id: StorageId::parse("personal").unwrap(),
        bundle_id: bundle_id(),
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        expected_host_identity_revision: 3,
        expected_project_binding_revision: 2,
        expected_bindings_document_revision: 4,
        revision: 0,
        commit_id: "absent".to_string(),
        manifest_sha256: "0".repeat(64),
        created_at: 1,
        expires_at: 10,
        actions: vec![
            push_action("action-codex", "codex", "profile-codex", "1111"),
            push_action("action-claude", "claude", "profile-claude", "2222"),
        ],
    };
    plan.validate().unwrap();

    // Two different scope pins for the same provider cannot coexist.
    let mut conflicting = plan.clone();
    conflicting.actions[1] = push_action("action-codex-2", "codex", "profile-codex", "3333");
    conflicting.actions[1].pinned_scope_revision = 5;
    conflicting.actions[1].pinned_scope_sha256 = "b".repeat(64);
    assert!(conflicting
        .validate()
        .unwrap_err()
        .contains("two scope states"));
}

#[test]
fn adapter_divergence_requires_paired_fork_pins() {
    let base = AdapterDivergenceOperation {
        origin: ForkOrigin::PullPreserve,
        resolution: ConversationDivergenceResolution::PreserveLocalWithNativeFork,
        local_branch_preserved: false,
        preserved_thread_id: ThreadId::parse("thread-fork").unwrap(),
        source_provider_thread_id: "session-1".to_string(),
        source_transcript_path: "/home/user/.claude/projects/bucket/session-1.jsonl".to_string(),
        fork_provider_thread_id: Some("session-2".to_string()),
        fork_transcript_path: Some(
            "/home/user/.claude/projects/bucket/session-2.jsonl".to_string(),
        ),
        fork_transcript_sha256: Some("a".repeat(64)),
        local_lineage_digest: "b".repeat(64),
        local_semantic_digest: "c".repeat(64),
        local_message_count: 3,
    };
    base.validate(&ThreadId::parse("thread-src").unwrap())
        .unwrap();

    let mut unpaired = base.clone();
    unpaired.fork_transcript_sha256 = None;
    assert!(unpaired
        .validate(&ThreadId::parse("thread-src").unwrap())
        .unwrap_err()
        .contains("paired"));

    let mut self_overwrite = base.clone();
    self_overwrite.fork_transcript_path = Some(self_overwrite.source_transcript_path.clone());
    assert!(self_overwrite
        .validate(&ThreadId::parse("thread-src").unwrap())
        .is_err());

    let mut same_thread = base;
    same_thread.preserved_thread_id = ThreadId::parse("thread-src").unwrap();
    assert!(same_thread
        .validate(&ThreadId::parse("thread-src").unwrap())
        .is_err());
}

#[test]
fn divergence_plans_validate_per_execution_kind() {
    let plan = ConversationDivergencePlan {
        available_resolutions: vec![
            ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            ConversationDivergenceResolution::UseCloudOnly,
        ],
        recommended_resolution: ConversationDivergenceResolution::PreserveLocalWithNativeFork,
        local_branch_preserved: false,
        preserved_thread_id: ThreadId::parse("thread-fork").unwrap(),
        source_provider_thread_id: "session-1".to_string(),
        local_lineage_digest: "b".repeat(64),
        local_semantic_digest: "c".repeat(64),
        local_message_count: 3,
        pinned_local_title: None,
        pinned_cloud_title: None,
        desired_fork_name: "Preserved branch".to_string(),
        desired_original_name: None,
        execution: DivergenceExecution::AdapterFileFork {
            fork_provider_thread_id: "session-2".to_string(),
            fork_transcript_path: "/home/user/.claude/projects/bucket/session-2.jsonl".to_string(),
        },
    };
    // Indirect validation through a plan action.
    let mut action = push_action("action-div", "claude", "profile-claude", "src");
    action.direction = ConversationPlanDirection::Pull;
    action.action = ConversationPlanActionType::ResolveDivergence;
    action.reason_code = ConversationReasonCode::Diverged;
    action.relation = ConversationRelation::Diverged;
    action.divergence = Some(plan.clone());
    action.thread_id = ThreadId::parse("thread-src").unwrap();
    action.validate().unwrap();

    // A Codex native-fork execution demands the codex provider key.
    let mut codex_exec = action.clone();
    if let Some(divergence) = &mut codex_exec.divergence {
        divergence.execution = DivergenceExecution::CodexNativeFork {
            codex_executable: "/usr/local/bin/codex".to_string(),
            codex_executable_sha256: "d".repeat(64),
            available_executors: vec![
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
            ],
        };
        divergence.desired_fork_name = "Preserved branch".to_string();
    }
    assert!(codex_exec
        .validate()
        .unwrap_err()
        .contains("requires the codex provider"));
}

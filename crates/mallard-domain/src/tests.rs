use super::*;

#[test]
fn shared_hash_and_time_primitives_keep_the_existing_encoding_and_epoch_policy() {
    use sha2::{Digest as _, Sha256};
    use std::time::Duration;

    assert_eq!(hex_lower(&[0x00, 0x0f, 0x10, 0xff]), "000f10ff");
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    let mut framed = Sha256::new();
    update_length_prefixed(&mut framed, b"ab");
    let mut explicit = Sha256::new();
    explicit.update(2_u64.to_be_bytes());
    explicit.update(b"ab");
    assert_eq!(framed.finalize(), explicit.finalize());

    assert_eq!(unix_timestamp_secs(UNIX_EPOCH), 0);
    assert_eq!(
        unix_timestamp_secs(UNIX_EPOCH + Duration::from_secs(42)),
        42
    );
    assert_eq!(unix_timestamp_secs(UNIX_EPOCH - Duration::from_secs(1)), 0);
}

fn bundle_id() -> BundleId {
    BundleId::parse("0123456789abcdef0123456789abcdef").unwrap()
}

fn resource_id(value: &str) -> ResourceId {
    ResourceId::parse(value).unwrap()
}

fn provider(key: &str) -> ProviderKey {
    ProviderKey::parse(key).unwrap()
}

#[test]
fn conversation_decision_wire_shape_matches_the_review_contract() {
    let selectable = ConversationDecision {
        action: ConversationReviewAction::Publish,
        blocker: None,
        selectable: true,
        blocked_reason: None,
    };
    assert_eq!(
        serde_json::to_value(selectable).unwrap(),
        serde_json::json!({
            "action": "publish",
            "selectable": true,
        })
    );

    let blocked = ConversationDecision {
        action: ConversationReviewAction::Blocked,
        blocker: Some(ConversationBlocker::MissingBinding),
        selectable: false,
        blocked_reason: Some("Connect the provider profile.".to_string()),
    };
    assert_eq!(
        serde_json::to_value(blocked).unwrap(),
        serde_json::json!({
            "action": "blocked",
            "blocker": "missing_binding",
            "selectable": false,
            "blocked_reason": "Connect the provider profile.",
        })
    );
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
fn resource_and_conversation_titles_preserve_control_characters() {
    let parts = sealed_provider_scope("codex", "title-controls");
    let mut resource = parts.scope.resources.values().next().unwrap().clone();
    let mut thread = parts.scope.conversations[&parts.thread_id].clone();
    let controls: String = ('\0'..='\u{009f}').filter(|ch| ch.is_control()).collect();

    for title in [controls.as_str(), " \t\r\n", "标题\u{202e}"] {
        resource.display_name = title.to_string();
        resource.validate().unwrap();
        let encoded = serde_json::to_vec(&resource).unwrap();
        let decoded: ResourceDescriptor = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.display_name, title);

        thread.display_title = Some(title.to_string());
        thread.validate().unwrap();
        let encoded = serde_json::to_vec(&thread).unwrap();
        let decoded: ConversationThread = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.display_title.as_deref(), Some(title));
    }
}

#[test]
fn title_bounds_do_not_relax_other_display_metadata() {
    for title in ["a".repeat(1_024), "é".repeat(512)] {
        validate_title("title", &title).unwrap();
    }
    for title in [String::new(), "a".repeat(1_025), "é".repeat(513)] {
        assert!(validate_title("title", &title).is_err());
    }

    assert!(validate_display_text("storage name", "Storage\u{0002}", false).is_err());
    assert!(validate_display_text("codec", "codex.lineage.v1\u{0013}", false).is_err());
    assert!(validate_provider_thread_id("thread id", "thread\u{0002}").is_err());
    assert!(validate_absolute_clean_path("project root", "/project\u{0013}").is_err());
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
fn schema_one_session_metrics_round_trip_token_breakdown() {
    let metrics = SessionMetrics {
        total_input_tokens: Some(200),
        total_output_tokens: Some(50),
        total_tokens: Some(250),
        complete: true,
        ..SessionMetrics::default()
    };

    let encoded = serde_json::to_value(&metrics).unwrap();
    assert_eq!(encoded["total_input_tokens"], 200);
    assert_eq!(encoded["total_output_tokens"], 50);
    assert_eq!(encoded["total_tokens"], 250);
    assert_eq!(
        serde_json::from_value::<SessionMetrics>(encoded).unwrap(),
        metrics
    );
    assert_eq!(LOCAL_SCHEMA, 1);
    assert_eq!(BUNDLE_SCHEMA, 1);
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
    let mut scopes = [
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

    let resource_id = conversation_resource_id(&provider("codex"), &first).unwrap();
    assert_eq!(
        parse_conversation_resource_id(resource_id.as_str()).unwrap(),
        (provider("codex"), first)
    );
    assert!(
        parse_conversation_resource_id("codex:session:019f742a-a206-7932-876c-9db8d8ce575a")
            .is_err()
    );
}

#[test]
fn retained_conversation_ids_are_stable_and_domain_separated() {
    let native_id = "019f742a-a206-7932-876c-9db8d8ce575a";
    let displaced = ThreadId::parse("thread-displaced").unwrap();
    let retained =
        retained_conversation_thread_id(&bundle_id(), &provider("codex"), &displaced, native_id)
            .unwrap();
    assert_eq!(
        retained,
        retained_conversation_thread_id(&bundle_id(), &provider("codex"), &displaced, native_id,)
            .unwrap()
    );
    assert_ne!(
        retained,
        initial_conversation_thread_id(&bundle_id(), &provider("codex"), native_id).unwrap()
    );
    assert_ne!(
        retained,
        retained_conversation_thread_id(
            &BundleId::parse("11111111111111111111111111111111").unwrap(),
            &provider("codex"),
            &displaced,
            native_id,
        )
        .unwrap()
    );
    assert_ne!(
        retained,
        retained_conversation_thread_id(
            &bundle_id(),
            &provider("codex"),
            &ThreadId::parse("thread-other-owner").unwrap(),
            native_id,
        )
        .unwrap()
    );
}

#[test]
fn machine_local_recipes_reject_portable_conversation_identities() {
    let thread_id = ThreadId::parse("thread-local-recipe-boundary").unwrap();
    let portable_id = conversation_resource_id(&provider("codex"), &thread_id).unwrap();
    let mut portable_recipe = BundleRecipe::default();
    portable_recipe.entries.insert(
        portable_id.clone(),
        RecipeEntry {
            resource_id: portable_id.clone(),
            apply_policy: portable::contracts::safe_file(),
            required: false,
        },
    );

    // The same recipe is valid in a portable provider scope but never as
    // machine-local project or storage-link selection state.
    portable_recipe.validate().unwrap();
    let error = portable_recipe.validate_machine_local().unwrap_err();
    assert!(error.contains("machine-local recipe contains portable conversation resource"));
    assert!(error.contains(portable_id.as_str()));

    let local_session_id = resource_id("codex:session:019f742a-a206-7932-876c-9db8d8ce575a");
    let mut local_recipe = BundleRecipe::default();
    local_recipe.entries.insert(
        local_session_id.clone(),
        RecipeEntry {
            resource_id: local_session_id,
            apply_policy: portable::contracts::safe_file(),
            required: false,
        },
    );
    local_recipe.validate_machine_local().unwrap();

    let mut project = sample_project();
    project.recipe = portable_recipe.clone();
    assert!(project
        .validate()
        .unwrap_err()
        .contains("machine-local recipe contains portable conversation resource"));

    project.recipe = BundleRecipe::default();
    let config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: vec![sample_storage()],
        projects: vec![project.clone()],
        links: vec![ProjectStorageLink {
            managed_cloud_repository_id: None,
            local_project_id: project.local_project_id,
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: project.bundle_id,
            recipe: Some(portable_recipe),
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::new(),
            pinned: false,
            created_at: 1,
        }],
    };
    assert!(config
        .validate()
        .unwrap_err()
        .contains("machine-local recipe contains portable conversation resource"));
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
        provider: StorageProviderKey::parse("local").unwrap(),
        provider_config: StorageProviderConfig::new(
            1,
            BoundedConfigObject::from_json(&serde_json::json!({
                "directory": "/tmp/project-sync-store"
            }))
            .unwrap(),
            SecretMap::empty(),
        ),
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
            managed_cloud_repository_id: None,
            local_project_id: project.local_project_id.clone(),
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: BundleId::parse("11111111111111111111111111111111").unwrap(),
            recipe: None,
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::new(),
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
            managed_cloud_repository_id: None,
            local_project_id: project.local_project_id.clone(),
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: bundle_id(),
            recipe: None,
            scope_bases: BTreeMap::from([(
                SyncScopeKey::Provider(provider("claude")),
                base.clone(),
            )]),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::new(),
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
        provider_config: AgentProviderConfig::version_1(),
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
            provider_config: AgentProviderConfig::version_1(),
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
            "file" => portable::contracts::project_content_file(),
            "dir" => portable::contracts::project_content_directory(),
            _ => panic!("unsupported test entry type"),
        },
        provider: None,
        provider_resource_type: None,
        display_name: relative_path.to_string(),
        provenance: portable::contracts::provenance_project_local(relative_path).unwrap(),
        apply_policy: portable::contracts::explicit_review(),
        codec_version: 1,
        attributes: BoundedPortableObject::empty(),
    }
}

type ProjectScopeContent = (
    BundleRecipe,
    BTreeMap<ResourceId, ResourceDescriptor>,
    BTreeMap<LogicalPath, BundleFileEntry>,
    BTreeMap<LogicalPath, BundleDirectoryEntry>,
);

fn project_scope_content() -> ProjectScopeContent {
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
                apply_policy: portable::contracts::explicit_review(),
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
            apply_policy: portable::contracts::explicit_review(),
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
    thread_id: CloudThreadId,
    scope: BundleScope,
}

/// A syntactically valid redaction stamp for validation tests. The domain
/// checks shape only; fingerprint equality with a binary ruleset is an
/// application-boundary concern.
fn test_wire_redaction() -> Option<ConversationWireRedaction> {
    Some(ConversationWireRedaction {
        codec: "mallard.redact.v1".to_string(),
        fingerprint: "ab".repeat(32),
    })
}

fn sealed_provider_scope(provider_key: &str, thread_suffix: &str) -> ProviderScopeParts {
    let key = provider(provider_key);
    let thread_id = CloudThreadId::parse(format!("cloudthread-{}", thread_suffix)).unwrap();
    let resource = cloud_conversation_resource_id(&key, &thread_id).unwrap();
    let transcript_path = cloud_conversation_transcript_path(&key, &thread_id).unwrap();
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
            branch: Some("main".to_string()),
            recorded_sha: None,
            metrics: SessionMetrics {
                user_rounds: 1,
                agent_messages: 1,
                tool_calls: 0,
                total_input_tokens: None,
                total_output_tokens: None,
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
        portable_tail: match provider_key {
            // Cloud thread docs carry the wire-form tail: cursor providers
            // fold under their wire codec labels.
            "codex" => Some(
                compute_portable_conversation_tail_with_codec(
                    PORTABLE_TAIL_CODEC_WIRE_V1,
                    TailPartitionKind::WholeThread,
                    0,
                    &chain,
                )
                .unwrap(),
            ),
            "claude" => Some(
                compute_portable_conversation_tail_with_codec(
                    CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1,
                    TailPartitionKind::WholeThread,
                    0,
                    &chain,
                )
                .unwrap(),
            ),
            _ => None,
        },
        wire_children: crate::ConversationWireChildren::On,
        wire_redaction: test_wire_redaction(),
        wire_path_map: (provider_key == "claude").then(|| ConversationWirePathMap {
            codec: "mallard.pathmap.v1".to_string(),
        }),
        forked_from: None,
        preserved: None,
        source_thread_id: Some("provider-native-thread".to_string()),
        created_by_origin: None,
        last_updated_by_origin: None,
        created_at: 1,
        updated_at: 2,
    };
    let descriptor = ResourceDescriptor {
        resource_id: resource.clone(),
        kind: portable::contracts::conversation(),
        provider: Some(key.clone()),
        provider_resource_type: Some(ResourceTypeKey::parse("session").unwrap()),
        display_name: "Conversation".to_string(),
        provenance: portable::contracts::provenance_unknown(),
        apply_policy: portable::contracts::safe_file(),
        codec_version: 1,
        attributes: BoundedPortableObject::empty(),
    };
    let recipe = BundleRecipe {
        schema_version: RECIPE_SCHEMA,
        revision: 1,
        entries: BTreeMap::from([(
            resource.clone(),
            RecipeEntry {
                resource_id: resource.clone(),
                apply_policy: portable::contracts::safe_file(),
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

/// The out-of-band path-map stamp is claimed exactly where no in-band
/// declaration exists: a claude row without one is unreadable by design,
/// and an in-band provider carrying one is a writer bug.
#[test]
fn wire_path_map_stamp_is_required_for_claude_and_forbidden_elsewhere() {
    let claude = sealed_provider_scope("claude", "31111111111111111111111111111111");
    let mut manifest = scoped_manifest(BTreeMap::from([(
        SyncScopeKey::Provider(provider("claude")),
        claude.scope,
    )]));
    manifest.validate().unwrap();
    let reseal = |manifest: &mut BundleManifest| {
        for (key, scope) in manifest.scopes.iter_mut() {
            scope.content_sha256 = scope.compute_content_sha256(key).unwrap();
        }
    };
    for scope in manifest.scopes.values_mut() {
        for thread in scope.conversations.values_mut() {
            thread.wire_path_map = None;
        }
    }
    reseal(&mut manifest);
    manifest.validate().unwrap();
    assert!(manifest
        .resolve_all(&portable::test_resolver::Version1Resolver)
        .unwrap_err()
        .contains("wire path-map stamp"));

    let codex = sealed_provider_scope("codex", "40123456789abcdef0123456789abcde");
    let mut manifest = scoped_manifest(BTreeMap::from([(
        SyncScopeKey::Provider(provider("codex")),
        codex.scope,
    )]));
    manifest.validate().unwrap();
    for scope in manifest.scopes.values_mut() {
        for thread in scope.conversations.values_mut() {
            thread.wire_path_map = Some(ConversationWirePathMap {
                codec: "mallard.pathmap.v1".to_string(),
            });
        }
    }
    reseal(&mut manifest);
    manifest.validate().unwrap();
    assert!(manifest
        .resolve_all(&portable::test_resolver::Version1Resolver)
        .unwrap_err()
        .contains("in-band"));
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
    let resource = cloud_conversation_resource_id(&claude_key, &thread_id).unwrap();
    let transcript_path = cloud_conversation_transcript_path(&claude_key, &thread_id).unwrap();
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
        portable_tail: Some(
            compute_portable_conversation_tail_with_codec(
                CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1,
                TailPartitionKind::WholeThread,
                0,
                &chain,
            )
            .unwrap(),
        ),
        wire_children: crate::ConversationWireChildren::On,
        wire_redaction: test_wire_redaction(),
        wire_path_map: Some(ConversationWirePathMap {
            codec: "mallard.pathmap.v1".to_string(),
        }),
        forked_from: None,
        preserved: None,
        source_thread_id: None,
        created_by_origin: None,
        last_updated_by_origin: None,
        created_at: 1,
        updated_at: 1,
    };
    let descriptor = ResourceDescriptor {
        resource_id: resource.clone(),
        kind: portable::contracts::conversation(),
        provider: Some(claude_key.clone()),
        provider_resource_type: None,
        display_name: "Conversation".to_string(),
        provenance: portable::contracts::provenance_unknown(),
        apply_policy: portable::contracts::safe_file(),
        codec_version: 1,
        attributes: BoundedPortableObject::empty(),
    };
    let recipe = BundleRecipe {
        schema_version: RECIPE_SCHEMA,
        revision: 1,
        entries: BTreeMap::from([(
            resource.clone(),
            RecipeEntry {
                resource_id: resource.clone(),
                apply_policy: portable::contracts::safe_file(),
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

/// One record name per step: `(commit byte, parent index, created_at_ms)`.
/// A parent index of `None` makes a first publication.
fn record_names(steps: &[(u8, Option<usize>, u64)]) -> Vec<RecordName> {
    let mut names = Vec::<RecordName>::new();
    for (id, parent, created_at_ms) in steps {
        let token = PublicationToken::new(
            format!("commit-{}", format!("{id:02x}").repeat(12)),
            format!("{id:02x}").repeat(32),
        )
        .unwrap();
        let parent = parent.map(|index| &names[index]);
        names.push(
            RecordName::new(
                parent.map_or(1, |parent| parent.revision() + 1),
                token,
                parent.map(|parent| parent.token().clone()),
                *created_at_ms,
            )
            .unwrap(),
        );
    }
    names
}

fn repository_base_at(name: &RecordName) -> RepositoryBase {
    RepositoryBase {
        revision: name.revision(),
        commit_id: name.commit_id().to_string(),
        manifest_sha256: "b".repeat(64),
        manifest_size: 10,
    }
}

#[test]
fn remote_continuity_covers_every_decision_row_by_identity_and_ancestry() {
    // 1 -> 2 -> 3 is selected; 4 -> 5 -> 6 -> 7 is a later-ranked sibling
    // branch of 1 that runs deeper than the selected chain.
    let names = record_names(&[
        (1, None, 10),
        (2, Some(0), 20),
        (3, Some(1), 30),
        (4, Some(0), 25),
        (5, Some(3), 26),
        (6, Some(4), 27),
        (7, Some(5), 28),
    ]);
    let mainline = Mainline::select(names.iter().cloned());
    assert_eq!(mainline.selected(), Some(&names[2]));
    let empty = Mainline::default();
    let base = |index: usize| repository_base_at(&names[index]);
    // Every verdict but `Displaced` asks nobody who published anything.
    let nobody = |_: &RecordName| -> Result<SavedBy, String> { unreachable!("not displaced") };
    let classify = |mainline: &Mainline, bases: &[&RepositoryBase]| {
        classify_remote_continuity(mainline, &[], bases.iter().copied(), nobody).unwrap()
    };

    assert_eq!(classify(&empty, &[]), RemoteContinuity::Fresh);
    assert_eq!(
        classify(&empty, &[&base(1)]),
        RemoteContinuity::RemoteMissing { base_revision: 2 }
    );
    assert_eq!(
        classify(&mainline, &[]),
        RemoteContinuity::Unreviewed { remote_revision: 3 }
    );
    assert_eq!(
        classify(&mainline, &[&base(2)]),
        RemoteContinuity::InSync { revision: 3 }
    );
    assert_eq!(
        classify(&mainline, &[&base(0)]),
        RemoteContinuity::Advanced {
            base_revision: 1,
            remote_revision: 3
        }
    );
    // Several scope bases: every one of them must be on the selected chain,
    // and the newest is the one reported.
    assert_eq!(
        classify(&mainline, &[&base(0), &base(2)]),
        RemoteContinuity::InSync { revision: 3 }
    );
    assert_eq!(
        classify(&mainline, &[&base(1), &base(0)]),
        RemoteContinuity::Advanced {
            base_revision: 2,
            remote_revision: 3
        }
    );
    assert_eq!(
        classify(&mainline, &[&base(2), &base(3)]),
        RemoteContinuity::Replaced {
            base_revision: 3,
            remote_revision: 3
        }
    );

    // A base on the losing sibling branch while its records are not listed:
    // same depth as a selected ancestor, and then deeper than the selected
    // publication itself. Revision order would call the first one advanced.
    assert_eq!(
        classify(&mainline, &[&base(3)]),
        RemoteContinuity::Replaced {
            base_revision: 2,
            remote_revision: 3
        }
    );
    assert_eq!(
        classify(&mainline, &[&base(6)]),
        RemoteContinuity::Replaced {
            base_revision: 5,
            remote_revision: 3
        }
    );

    // The same bases while the losing branch is listed: its records connect
    // them to the chain, so they were displaced, not cleared. The newest
    // displaced publication is reported, and who published it is asked
    // once, with that record.
    let unselected = names[3..].to_vec();
    let asked = std::cell::RefCell::new(Vec::new());
    let ask = |name: &RecordName| -> Result<SavedBy, String> {
        asked.borrow_mut().push(name.clone());
        Ok(SavedBy::Pull)
    };
    assert_eq!(
        classify_remote_continuity(&mainline, &unselected, [&base(3)], ask).unwrap(),
        RemoteContinuity::Displaced {
            base_revision: 2,
            remote_revision: 3,
            saved_by: SavedBy::Pull
        }
    );
    assert_eq!(
        classify_remote_continuity(&mainline, &unselected, [&base(3), &base(6)], ask).unwrap(),
        RemoteContinuity::Displaced {
            base_revision: 5,
            remote_revision: 3,
            saved_by: SavedBy::Pull
        }
    );
    assert_eq!(asked.take(), vec![names[3].clone(), names[6].clone()]);
    // A displaced base wins over a cleared one on the same link, a base on
    // the chain does not hide it, and a stream that selects nothing still
    // reads missing.
    let cleared = repository_base_at(&record_names(&[(9, None, 50)])[0]);
    assert_eq!(
        classify_remote_continuity(
            &mainline,
            &unselected,
            [&cleared, &base(3), &base(0)],
            |_| Ok(SavedBy::Push)
        )
        .unwrap(),
        RemoteContinuity::Displaced {
            base_revision: 2,
            remote_revision: 3,
            saved_by: SavedBy::Push
        }
    );
    assert_eq!(
        classify_remote_continuity(&empty, &unselected, [&base(3)], nobody).unwrap(),
        RemoteContinuity::RemoteMissing { base_revision: 2 }
    );
    // The read the verdict needs is the caller's: its failure is an error,
    // never a verdict.
    assert_eq!(
        classify_remote_continuity(&mainline, &unselected, [&base(3)], |_| Err(
            "record unreadable".to_string()
        )),
        Err("record unreadable".to_string())
    );

    // The selected chain runs deeper than a base on the losing branch: a
    // greater remote revision is still not an advance.
    let deeper = record_names(&[
        (1, None, 10),
        (2, Some(0), 20),
        (3, Some(1), 30),
        (4, Some(2), 40),
        (8, Some(0), 21),
    ]);
    assert_eq!(
        classify(
            &Mainline::select(deeper.iter().cloned()),
            &[&repository_base_at(&deeper[4])]
        ),
        RemoteContinuity::Replaced {
            base_revision: 2,
            remote_revision: 4
        }
    );

    // A cleared stream that is pushed again restarts at revision 1 under a
    // new commit id: the old base is on no chain of it.
    let recreated = record_names(&[(9, None, 50)]);
    assert_eq!(
        classify(&Mainline::select(recreated), &[&base(0)]),
        RemoteContinuity::Replaced {
            base_revision: 1,
            remote_revision: 1
        }
    );
}

#[test]
fn scope_continuity_covers_every_decision_row() {
    let names = record_names(&[(1, None, 10), (2, Some(0), 20), (3, Some(0), 21)]);
    let mainline = Mainline::select(names.iter().cloned());
    assert_eq!(mainline.selected(), Some(&names[1]));
    let base_reviewed_in = |name: &RecordName| ScopeBase {
        scope: SyncScopeKey::Provider(provider("codex")),
        scope_revision: 3,
        scope_sha256: "a".repeat(64),
        reviewed_in: repository_base_at(name),
        binding_revision: 1,
        last_pull_at: None,
        last_push_at: None,
    };
    // A usable base was reviewed in a publication on the selected chain. A
    // stale one was reviewed in a publication that is not: here a sibling
    // that was not selected, below a stream that was cleared.
    let usable = base_reviewed_in(&names[0]);
    let stale = base_reviewed_in(&names[2]);
    let digest = "a".repeat(64);
    let other = "c".repeat(64);
    // A1: absent, no base.
    assert_eq!(
        classify_scope_continuity(None, None, &mainline),
        ScopeContinuity::Fresh
    );
    // A2: absent, usable base.
    assert_eq!(
        classify_scope_continuity(None, Some(&usable), &mainline),
        ScopeContinuity::RemoteMissing { base_revision: 3 }
    );
    // A3: absent, stale base. The base is read as if it were not recorded.
    assert_eq!(
        classify_scope_continuity(None, Some(&stale), &mainline),
        ScopeContinuity::Fresh
    );
    // A4: present, no base.
    assert_eq!(
        classify_scope_continuity(Some((4, &digest)), None, &mainline),
        ScopeContinuity::Unreviewed { remote_revision: 4 }
    );
    // A5: present with the reviewed content, whatever the base's history.
    assert_eq!(
        classify_scope_continuity(Some((3, &digest)), Some(&usable), &mainline),
        ScopeContinuity::InSync { revision: 3 }
    );
    assert_eq!(
        classify_scope_continuity(Some((3, &digest)), Some(&stale), &mainline),
        ScopeContinuity::InSync { revision: 3 }
    );
    // A6: present with other content, usable base.
    assert_eq!(
        classify_scope_continuity(Some((4, &other)), Some(&usable), &mainline),
        ScopeContinuity::Advanced {
            base_revision: 3,
            remote_revision: 4
        }
    );
    // The scope counter is a per-branch number, so a changed digest on the
    // reviewed history is an advance whatever the counter says.
    assert_eq!(
        classify_scope_continuity(Some((2, &other)), Some(&usable), &mainline),
        ScopeContinuity::Advanced {
            base_revision: 3,
            remote_revision: 2
        }
    );
    // A7: present with other content, stale base. It reads as never
    // reviewed, even at a greater counter.
    assert_eq!(
        classify_scope_continuity(Some((9, &other)), Some(&stale), &mainline),
        ScopeContinuity::Unreviewed { remote_revision: 9 }
    );

    // A cleared stream lists empty: every base is stale, and every scope is
    // a first publication target again.
    let cleared = Mainline::default();
    assert_eq!(
        classify_scope_continuity(None, Some(&usable), &cleared),
        ScopeContinuity::Fresh
    );
    // A cleared stream that was pushed again restarts at revision 1 under a
    // new commit id.
    let recreated = Mainline::select(record_names(&[(9, None, 50)]));
    assert_eq!(
        classify_scope_continuity(None, Some(&usable), &recreated),
        ScopeContinuity::Fresh
    );
    assert_eq!(
        classify_scope_continuity(Some((1, &other)), Some(&usable), &recreated),
        ScopeContinuity::Unreviewed { remote_revision: 1 }
    );
}

#[test]
fn mainline_finds_a_publication_by_commit_id_alone() {
    let names = record_names(&[(1, None, 10), (2, Some(0), 20), (3, Some(0), 21)]);
    let mainline = Mainline::select(names.iter().cloned());
    assert!(mainline.contains_commit(names[0].commit_id()));
    assert!(mainline.contains_commit(names[1].commit_id()));
    // Listed and not selected.
    assert!(!mainline.contains_commit(names[2].commit_id()));
    assert!(!Mainline::default().contains_commit(names[0].commit_id()));
}

#[test]
fn recipe_key_is_identity_and_digest_is_only_version_metadata() {
    let id = resource_id("memo:release");
    let mut recipe = BundleRecipe::default();
    recipe.entries.insert(
        id.clone(),
        RecipeEntry {
            resource_id: id,
            apply_policy: portable::contracts::explicit_review(),
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
    missing_ancestor.validate().unwrap();
    assert!(missing_ancestor
        .resolve_all(&portable::test_resolver::Version1Resolver)
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
    orphan.validate().unwrap();
    assert!(orphan
        .resolve_all(&portable::test_resolver::Version1Resolver)
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
            target: portable::contracts::tombstone_project_content_file(&entry.resource_id, &path)
                .unwrap(),
            last_sha256: Some(entry.sha256),
            deleted_at: 2,
        };
        scope
            .tombstones
            .insert(tombstone.structural_key(), tombstone);
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    manifest.validate().unwrap();

    // A conversation tombstone can never live in the project scope.
    {
        let scope = manifest.scopes.get_mut(&SyncScopeKey::Project).unwrap();
        let tombstone = Tombstone {
            target: portable::contracts::tombstone_conversation_thread(
                &CloudThreadId::parse("cloudthread-a").unwrap(),
            )
            .unwrap(),
            last_sha256: Some("a".repeat(64)),
            deleted_at: 2,
        };
        scope
            .tombstones
            .insert(tombstone.structural_key(), tombstone);
        scope.content_sha256 = scope
            .compute_content_sha256(&SyncScopeKey::Project)
            .unwrap();
    }
    manifest.validate().unwrap();
    assert!(manifest
        .resolve_all(&portable::test_resolver::Version1Resolver)
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
            preserve: None,
        },
        target_path: None,
        source_sha256: None,
        expected_target_sha256: None,
        expected_target_mode: None,
        materialized_sha256: None,
        source_size: None,
        requires_explicit_approval: false,
    };
    assert!(action.validate().unwrap_err().contains("explicit approval"));
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
        cloud_conversation_sidecar_path(&key, &parts.thread_id, "todos", "todo.json").unwrap();
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
        action: ConversationPlanActionType::CreateCloud,
        provider: provider(provider_key),
        thread_id: Some(ThreadId::parse(format!("thread-{}", thread_suffix)).unwrap()),
        cloud_thread_id: None,
        source_resource_id: None,
        pins: ConversationActionPins {
            subagent_selection: Default::default(),
            credential_redaction_enabled: true,
            export_settings_changed: false,
            context: ConversationContextPins {
                host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
                replica_id: ReplicaId::parse("replica-a").unwrap(),
                local_project_id: LocalProjectId::parse("project-a").unwrap(),
                provider_profile_id: LocalProviderProfileId::parse(profile).unwrap(),
                host_identity_revision: 3,
                profile_revision: 1,
                project_binding_revision: 2,
                bindings_document_revision: 4,
                binding_revision: None,
            },
            revision: ConversationRevisionPins {
                bundle_revision: 0,
                commit_id: "absent".to_string(),
                manifest_sha256: "0".repeat(64),
                scope_revision,
                scope_sha256,
            },
            tail: ConversationTailPins {
                subagents: Default::default(),
                subagents_omitted: false,
                missing_local_subagents: Default::default(),
                accepted: None,
                accepted_local: None,
                local: Some(portable_tail(
                    TailPartitionKind::WholeThread,
                    0,
                    5,
                    TAIL_VECTOR_WHOLE_AT_0_M1_M5,
                )),
                cloud: None,
                checkpoint_generation: None,
                lineage_digest: None,
                semantic_digest: None,
                raw_range: Some(RawRangePin {
                    start_byte_offset: 0,
                    end_byte_offset: 512,
                    range_sha256: "a".repeat(64),
                }),
            },
            target: ConversationTargetPins {
                local: TargetExpectation::Sha256("a".repeat(64)),
                provider_state: Vec::new(),
            },
            codec: format!("{}.lineage.v1", provider_key),
            expires_at: 10,
        },
    }
}

// --- Portable tail codec v1 -------------------------------------------------

// Golden vectors pin the current codec framing. Any drift is a codec break,
// not an automatic test update.
const TAIL_VECTOR_EMPTY_WHOLE_AT_0: &str =
    "267171df350b0854b2a599f63c7c6929e3ab9461da4bf037b953dce7f6bb7758";
const TAIL_VECTOR_EMPTY_FIXED_AT_0: &str =
    "061b4a431cbe975bffb12ac9252a0eea115a2dfbb0f9d6a0f3a5f845b5c5304a";
const TAIL_VECTOR_EMPTY_FIXED_AT_3: &str =
    "d26d78574a391922df9059b0f07fe0250bf8479156d46141ee9fd24eb2920e94";
const TAIL_VECTOR_FIXED_AT_3_M4: &str =
    "e0985bf84b7a91771f32455189af0ec53da2252cec3a2058975f13cf8230f221";
const TAIL_VECTOR_FIXED_AT_3_M4_M5: &str =
    "d4aac94a830407da14075320f855b80122b7cb72ce441754c35b81836491bca8";
const TAIL_VECTOR_WHOLE_AT_0_M1_M5: &str =
    "daf73b84ad35d60fac695aa3c3797aa5685ea75c4a72068a57db65626f748369";

// The P0/P1/P2 lifecycle chain: base contributes m1..m3, the append-bearing
// leaf contributes m4,m5, so the fixed boundary is 3.
fn tail_chain() -> Vec<CanonicalConversationMessage> {
    vec![
        canonical_message("m1", None, 'a'),
        canonical_message("m2", Some("m1"), 'b'),
        canonical_message("m3", Some("m2"), 'c'),
        canonical_message("m4", Some("m3"), 'd'),
        canonical_message("m5", Some("m4"), 'e'),
    ]
}

fn portable_tail(
    partition_kind: TailPartitionKind,
    start: u64,
    end: u64,
    state: &str,
) -> PortableConversationTail {
    PortableConversationTail {
        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind,
        start_record_count: start,
        accepted_end_record_count: end,
        accepted_tail_state: state.to_string(),
    }
}

#[test]
fn tail_codec_golden_vectors_are_pinned() {
    let chain = tail_chain();
    let seeds = [
        (
            TailPartitionKind::WholeThread,
            0,
            TAIL_VECTOR_EMPTY_WHOLE_AT_0,
        ),
        (
            TailPartitionKind::FixedPaginatedTail,
            0,
            TAIL_VECTOR_EMPTY_FIXED_AT_0,
        ),
        (
            TailPartitionKind::FixedPaginatedTail,
            3,
            TAIL_VECTOR_EMPTY_FIXED_AT_3,
        ),
    ];
    for (kind, start, expected) in seeds {
        assert_eq!(
            TailAccumulator::seed(kind, start).unwrap().state_hex(),
            expected
        );
    }
    assert_eq!(
        compute_conversation_tail_state(TailPartitionKind::FixedPaginatedTail, 3, 4, &chain)
            .unwrap(),
        TAIL_VECTOR_FIXED_AT_3_M4
    );
    assert_eq!(
        compute_conversation_tail_state(TailPartitionKind::FixedPaginatedTail, 3, 5, &chain)
            .unwrap(),
        TAIL_VECTOR_FIXED_AT_3_M4_M5
    );
    assert_eq!(
        compute_conversation_tail_state(TailPartitionKind::WholeThread, 0, 5, &chain).unwrap(),
        TAIL_VECTOR_WHOLE_AT_0_M1_M5
    );

    // Empty tails at different kinds or boundaries are unequal, and every
    // vector is distinct: no state is an unqualified H(empty).
    let all = [
        TAIL_VECTOR_EMPTY_WHOLE_AT_0,
        TAIL_VECTOR_EMPTY_FIXED_AT_0,
        TAIL_VECTOR_EMPTY_FIXED_AT_3,
        TAIL_VECTOR_FIXED_AT_3_M4,
        TAIL_VECTOR_FIXED_AT_3_M4_M5,
        TAIL_VECTOR_WHOLE_AT_0_M1_M5,
    ];
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), all.len());

    // The full five-field value always ends at the complete chain end.
    let tail = compute_portable_conversation_tail(TailPartitionKind::FixedPaginatedTail, 3, &chain)
        .unwrap();
    assert_eq!(
        tail,
        portable_tail(
            TailPartitionKind::FixedPaginatedTail,
            3,
            5,
            TAIL_VECTOR_FIXED_AT_3_M4_M5
        )
    );
    tail.validate().unwrap();
}

#[test]
fn tail_codec_one_shot_equals_incremental_at_every_split() {
    let chain = tail_chain();
    for (kind, start) in [
        (TailPartitionKind::FixedPaginatedTail, 3_u64),
        (TailPartitionKind::WholeThread, 0),
    ] {
        let final_state = compute_conversation_tail_state(kind, start, 5, &chain).unwrap();
        for split in start..=5 {
            // Fold to the split on one machine, hand the saved (count, state)
            // pair across, and extend on the other: the simulated Pull.
            let saved = compute_conversation_tail_state(kind, start, split, &chain).unwrap();
            let mut resumed = TailAccumulator::resume(kind, start, split, &saved).unwrap();
            for message in &chain[split as usize..] {
                resumed.fold_message(message).unwrap();
            }
            assert_eq!(resumed.folded_end_record_count(), 5);
            assert_eq!(resumed.state_hex(), final_state, "split at {split}");
        }
        let mut incremental = TailAccumulator::seed(kind, start).unwrap();
        for message in &chain[start as usize..] {
            incremental.fold_message(message).unwrap();
        }
        assert_eq!(incremental.state_hex(), final_state);
    }
}

#[test]
fn tail_state_binds_partition_start_index_and_content() {
    let chain = tail_chain();
    let base = compute_conversation_tail_state(TailPartitionKind::FixedPaginatedTail, 3, 5, &chain)
        .unwrap();

    // Same window content folded from a moved boundary lands at different
    // absolute indexes and a different seed: never equal.
    let mut moved = TailAccumulator::seed(TailPartitionKind::FixedPaginatedTail, 2).unwrap();
    moved.fold_message(&chain[3]).unwrap();
    moved.fold_message(&chain[4]).unwrap();
    assert_ne!(moved.state_hex(), base);

    // Parent presence and semantic content are both bound.
    let mut parentless = TailAccumulator::seed(TailPartitionKind::WholeThread, 0).unwrap();
    parentless
        .fold_message(&canonical_message("m1", None, 'a'))
        .unwrap();
    let mut parented = TailAccumulator::seed(TailPartitionKind::WholeThread, 0).unwrap();
    parented
        .fold_message(&canonical_message("m1", Some("m0"), 'a'))
        .unwrap();
    let mut reworded = TailAccumulator::seed(TailPartitionKind::WholeThread, 0).unwrap();
    reworded
        .fold_message(&canonical_message("m1", None, 'f'))
        .unwrap();
    assert_ne!(parentless.state_hex(), parented.state_hex());
    assert_ne!(parentless.state_hex(), reworded.state_hex());
}

#[test]
fn portable_tail_wire_shape_is_frozen() {
    let tail = portable_tail(
        TailPartitionKind::FixedPaginatedTail,
        3,
        5,
        TAIL_VECTOR_FIXED_AT_3_M4_M5,
    );
    assert_eq!(
        serde_json::to_value(&tail).unwrap(),
        serde_json::json!({
            "codec": "mallard-codex-portable-tail-v1",
            "partition_kind": "fixed_paginated_tail",
            "start_record_count": 3,
            "accepted_end_record_count": 5,
            "accepted_tail_state": TAIL_VECTOR_FIXED_AT_3_M4_M5,
        })
    );
    let round_tripped: PortableConversationTail =
        serde_json::from_value(serde_json::to_value(&tail).unwrap()).unwrap();
    assert_eq!(round_tripped, tail);
    assert_eq!(
        serde_json::to_value(TailPartitionKind::WholeThread).unwrap(),
        serde_json::json!("whole_thread")
    );
    assert!(serde_json::from_value::<TailPartitionKind>(serde_json::json!("paginated")).is_err());

    // Classification equality is field-wise over the complete value.
    let mut other_codec = tail.clone();
    other_codec.codec = "mallard-codex-portable-tail-v2".to_string();
    let mut other_kind = tail.clone();
    other_kind.partition_kind = TailPartitionKind::WholeThread;
    let mut other_start = tail.clone();
    other_start.start_record_count = 2;
    let mut other_end = tail.clone();
    other_end.accepted_end_record_count = 6;
    let mut other_state = tail.clone();
    other_state.accepted_tail_state = TAIL_VECTOR_FIXED_AT_3_M4.to_string();
    for different in [other_codec, other_kind, other_start, other_end, other_state] {
        assert_ne!(different, tail);
    }
}

#[test]
fn portable_tail_validation_fails_closed() {
    let valid = portable_tail(
        TailPartitionKind::FixedPaginatedTail,
        3,
        5,
        TAIL_VECTOR_FIXED_AT_3_M4_M5,
    );
    valid.validate().unwrap();
    // A zero-boundary FixedPaginatedTail stays explicit and valid.
    portable_tail(
        TailPartitionKind::FixedPaginatedTail,
        0,
        0,
        TAIL_VECTOR_EMPTY_FIXED_AT_0,
    )
    .validate()
    .unwrap();

    // Structural validation accepts a foreign provider codec string; each
    // provider's wire pin accepts only its own wire-form codec — the local
    // codecs are themselves foreign to the wire columns.
    let mut foreign_codec = valid.clone();
    foreign_codec.codec = "example.foreign.codec.v1".to_string();
    foreign_codec.validate().unwrap();
    assert!(foreign_codec.validate_codex_wire().is_err());
    assert!(foreign_codec.validate_claude_wire().is_err());
    assert!(valid.validate_codex_wire().is_err());
    let mut wire = valid.clone();
    wire.codec = PORTABLE_TAIL_CODEC_WIRE_V1.to_string();
    wire.validate_codex_wire().unwrap();
    assert!(wire.validate_claude_wire().is_err());
    let mut claude_local = valid.clone();
    claude_local.codec = CLAUDE_PORTABLE_TAIL_CODEC_V1.to_string();
    claude_local.validate().unwrap();
    assert!(claude_local.validate_claude_wire().is_err());
    let mut claude_wire = valid.clone();
    claude_wire.codec = CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1.to_string();
    claude_wire.validate_claude_wire().unwrap();
    assert!(claude_wire.validate_codex_wire().is_err());

    let mut malformed_codec = valid.clone();
    malformed_codec.codec = "not a codec".to_string();
    let mut whole_with_offset = valid.clone();
    whole_with_offset.partition_kind = TailPartitionKind::WholeThread;
    let mut start_past_end = valid.clone();
    start_past_end.accepted_end_record_count = 2;
    let mut end_past_limit = valid.clone();
    end_past_limit.accepted_end_record_count = MAX_FILES as u64 + 1;
    let mut uppercase_state = valid.clone();
    uppercase_state.accepted_tail_state = valid.accepted_tail_state.to_uppercase();
    let mut short_state = valid.clone();
    short_state.accepted_tail_state = "d4aa".to_string();
    for broken in [
        malformed_codec,
        whole_with_offset,
        start_past_end,
        end_past_limit,
        uppercase_state,
        short_state,
    ] {
        assert!(broken.validate().is_err(), "{broken:?}");
    }
}

#[test]
fn tail_accumulator_bounds_fail_closed() {
    let chain = tail_chain();
    assert!(TailAccumulator::seed(TailPartitionKind::WholeThread, 1).is_err());
    assert!(
        TailAccumulator::seed(TailPartitionKind::FixedPaginatedTail, MAX_FILES as u64 + 1).is_err()
    );
    assert!(TailAccumulator::resume(
        TailPartitionKind::FixedPaginatedTail,
        3,
        2,
        TAIL_VECTOR_EMPTY_FIXED_AT_3
    )
    .is_err());
    assert!(
        TailAccumulator::resume(TailPartitionKind::FixedPaginatedTail, 3, 4, "not-a-digest")
            .is_err()
    );

    // Folding past the record limit fails instead of wrapping.
    let mut full = TailAccumulator::resume(
        TailPartitionKind::FixedPaginatedTail,
        0,
        MAX_FILES as u64,
        TAIL_VECTOR_EMPTY_FIXED_AT_0,
    )
    .unwrap();
    assert!(full.fold_message(&chain[0]).is_err());

    // A malformed semantic fingerprint never enters the state.
    let mut accumulator = TailAccumulator::seed(TailPartitionKind::WholeThread, 0).unwrap();
    assert!(accumulator
        .fold_message(&canonical_message("m1", None, 'z'))
        .is_err());

    // One-shot windows must stay inside the validated chain.
    assert!(
        compute_conversation_tail_state(TailPartitionKind::FixedPaginatedTail, 4, 3, &chain)
            .is_err()
    );
    assert!(
        compute_conversation_tail_state(TailPartitionKind::FixedPaginatedTail, 3, 6, &chain)
            .is_err()
    );
    assert!(compute_conversation_tail_state(TailPartitionKind::WholeThread, 1, 5, &chain).is_err());
    assert!(compute_conversation_tail_state(TailPartitionKind::WholeThread, 0, 0, &[]).is_err());
}

// --- Portable tail relation classifier --------------------------------------

fn whole_tail_over(chain: &[CanonicalConversationMessage]) -> PortableConversationTail {
    compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, chain).unwrap()
}

/// Single-form evidence (non-codex providers): the accepted wire pin and
/// its local twin carry the same value.
fn tail_evidence(
    accepted: TailSideEvidence,
    local: TailSideEvidence,
    cloud: TailSideEvidence,
) -> VerifiedTailEvidence {
    try_dual_tail_evidence(accepted.clone(), accepted, local, cloud).unwrap()
}

fn dual_tail_evidence(
    accepted: TailSideEvidence,
    accepted_local: TailSideEvidence,
    local: TailSideEvidence,
    cloud: TailSideEvidence,
) -> VerifiedTailEvidence {
    try_dual_tail_evidence(accepted, accepted_local, local, cloud).unwrap()
}

fn try_dual_tail_evidence(
    accepted: TailSideEvidence,
    accepted_local: TailSideEvidence,
    local: TailSideEvidence,
    cloud: TailSideEvidence,
) -> Result<VerifiedTailEvidence, TailEvidenceBlocker> {
    VerifiedTailEvidence::new(accepted, accepted_local, local, cloud)
}

#[test]
fn compare_conversation_tail_covers_every_relation_row() {
    use TailSideEvidence::{ConfirmedAbsent, Invalid, Present};
    let chain = tail_chain();
    let t3 = whole_tail_over(&chain[..3]);
    let t4 = whole_tail_over(&chain[..4]);
    let t5 = whole_tail_over(&chain[..5]);

    // The four-row table with pin alignment.
    let rows = [
        (
            t5.clone(),
            t5.clone(),
            t5.clone(),
            TailClassification::Sync(SyncCurrency::Current),
        ),
        (
            t4.clone(),
            t5.clone(),
            t5.clone(),
            TailClassification::Sync(SyncCurrency::Advanced),
        ),
        (
            t4.clone(),
            t5.clone(),
            t4.clone(),
            TailClassification::LocalOutstanding,
        ),
        (
            t4.clone(),
            t4.clone(),
            t5.clone(),
            TailClassification::CloudOutstanding,
        ),
        // Canonical containment m1..3 / m1..4 / m1..5 is still Divergent:
        // there is deliberately no containment refinement.
        (
            t3.clone(),
            t4.clone(),
            t5.clone(),
            TailClassification::Divergent,
        ),
    ];
    for (accepted, local, cloud, expected) in rows {
        assert_eq!(
            compare_conversation_tail(&tail_evidence(
                Present(accepted),
                Present(local),
                Present(cloud)
            )),
            expected
        );
    }

    // Directional absence is decided first, and only without a pin.
    let absence_rows = [
        (
            ConfirmedAbsent,
            Present(t5.clone()),
            ConfirmedAbsent,
            TailClassification::MissingCloud,
        ),
        (
            ConfirmedAbsent,
            ConfirmedAbsent,
            Present(t5.clone()),
            TailClassification::MissingLocal,
        ),
        (
            ConfirmedAbsent,
            ConfirmedAbsent,
            ConfirmedAbsent,
            TailClassification::MissingBoth,
        ),
    ];
    for (accepted, local, cloud, expected) in absence_rows {
        assert_eq!(
            compare_conversation_tail(&tail_evidence(accepted, local, cloud)),
            expected
        );
    }

    // A pair without accepted pins is one that first-contact adoption could
    // not prove: nothing relates the two copies, so neither replaces the
    // other. Even equal values stay unproven here; adoption is what proves
    // them, and it passes its proof as the accepted pins.
    for cloud in [t5.clone(), t4.clone()] {
        assert_eq!(
            compare_conversation_tail(&tail_evidence(
                ConfirmedAbsent,
                Present(t5.clone()),
                Present(cloud)
            )),
            TailClassification::Divergent
        );
    }

    // Invalid sides never become classifier input.
    assert_eq!(
        try_dual_tail_evidence(
            Present(t4.clone()),
            Present(t4.clone()),
            Invalid,
            Present(t5.clone()),
        ),
        Err(TailEvidenceBlocker::InvalidSide)
    );

    // Tail equality is meaningful only under a compatible codec, partition,
    // and start: a moved start or foreign codec is Unknown, not Divergent.
    let fixed =
        compute_portable_conversation_tail(TailPartitionKind::FixedPaginatedTail, 3, &chain)
            .unwrap();
    assert_eq!(
        try_dual_tail_evidence(
            Present(t5.clone()),
            Present(t5.clone()),
            Present(t5.clone()),
            Present(fixed.clone()),
        ),
        Err(TailEvidenceBlocker::LayoutMismatch)
    );
    let mut foreign = t5.clone();
    foreign.codec = "claude.lineage.v1".to_string();
    assert_eq!(
        try_dual_tail_evidence(
            Present(t5.clone()),
            Present(t5.clone()),
            Present(t5.clone()),
            Present(foreign),
        ),
        Err(TailEvidenceBlocker::LayoutMismatch)
    );
    let mut moved = fixed.clone();
    moved.start_record_count = 2;
    moved.accepted_end_record_count = 4;
    assert_eq!(
        try_dual_tail_evidence(
            Present(fixed.clone()),
            Present(fixed.clone()),
            Present(fixed.clone()),
            Present(moved),
        ),
        Err(TailEvidenceBlocker::LayoutMismatch)
    );

    // A completed Push of the local turn flips the follow-up machine to
    // CloudOutstanding: its pin still matches its local value while the
    // cloud carries the newly accepted tail.
    assert_eq!(
        compare_conversation_tail(&tail_evidence(
            Present(t4.clone()),
            Present(t4.clone()),
            Present(t5.clone())
        )),
        TailClassification::CloudOutstanding
    );

    // Equal-or-shorter ends under an unequal value are rewrites, never
    // outstanding work: local at-or-below its pinned end is Divergent, and
    // a cloud rewrite at the pinned end is Divergent.
    assert_eq!(
        compare_conversation_tail(&tail_evidence(
            Present(t4.clone()),
            Present(t3.clone()),
            Present(t4.clone())
        )),
        TailClassification::Divergent
    );
    let mut cloud_rewrite = t4.clone();
    cloud_rewrite.accepted_tail_state = "e".repeat(64);
    assert_eq!(
        compare_conversation_tail(&tail_evidence(
            Present(t4.clone()),
            Present(t4.clone()),
            Present(cloud_rewrite)
        )),
        TailClassification::Divergent
    );

    // Dual-form pins (codex): the wire column and local column never
    // cross-compare, so the same relations hold with distinct codecs and
    // states per column.
    let wire = |tail: &PortableConversationTail| {
        let mut wire = tail.clone();
        wire.codec = PORTABLE_TAIL_CODEC_WIRE_V1.to_string();
        wire.accepted_tail_state = "f".repeat(64);
        wire
    };
    let w4 = wire(&t4);
    let w5 = wire(&t5);
    assert_eq!(
        compare_conversation_tail(&dual_tail_evidence(
            Present(w4.clone()),
            Present(t4.clone()),
            Present(t4.clone()),
            Present(w4.clone())
        )),
        TailClassification::Sync(SyncCurrency::Current)
    );
    assert_eq!(
        compare_conversation_tail(&dual_tail_evidence(
            Present(w4.clone()),
            Present(t4.clone()),
            Present(t5.clone()),
            Present(w4.clone())
        )),
        TailClassification::LocalOutstanding
    );
    assert_eq!(
        compare_conversation_tail(&dual_tail_evidence(
            Present(w4.clone()),
            Present(t4.clone()),
            Present(t4.clone()),
            Present(w5.clone())
        )),
        TailClassification::CloudOutstanding
    );
    // Both columns moved: Divergent — the same-form Advanced row is
    // unreachable across forms because the codec labels differ.
    assert_eq!(
        compare_conversation_tail(&dual_tail_evidence(
            Present(w4.clone()),
            Present(t4.clone()),
            Present(t5.clone()),
            Present(w5.clone())
        )),
        TailClassification::Divergent
    );
    // A half-present pin pair is corrupted acceptance evidence.
    assert_eq!(
        try_dual_tail_evidence(
            Present(w4.clone()),
            ConfirmedAbsent,
            Present(t5.clone()),
            Present(w4.clone()),
        ),
        Err(TailEvidenceBlocker::IncompleteAcceptance)
    );
    // Wire and local pins disagreeing on coordinates is Unknown.
    let mut torn_local = t3.clone();
    torn_local.accepted_end_record_count = 3;
    assert_eq!(
        try_dual_tail_evidence(
            Present(w4.clone()),
            Present(t3.clone()),
            Present(t3.clone()),
            Present(w4.clone()),
        ),
        Err(TailEvidenceBlocker::AcceptanceCoordinateMismatch)
    );
}

// --- Checkpoints and machine-local state ------------------------------------

fn ready_mapping(consumed: u64) -> PhysicalMappingState {
    PhysicalMappingState::Ready {
        root_mapping: RootPhysicalMapping {
            provider_thread_id: "0199a1b2-0000-7000-8000-000000000001".to_string(),
            root_path: "/home/user/.codex/sessions/rollout.jsonl".to_string(),
            source_file_size: consumed + 100,
            consumed_byte_offset: consumed,
            consumed_record_ordinal: consumed / 100,
            probe_window_len: consumed.min(4_096),
            probe_window_sha256: "b".repeat(64),
            last_canonical_message_id: Some("m5".to_string()),
            last_synthetic_turn_id: None,
            next_synthetic_turn_ordinal: 0,
        },
        raw_range_pin: RawRangePin {
            start_byte_offset: 0,
            end_byte_offset: consumed,
            range_sha256: "c".repeat(64),
        },
    }
}

fn fixed_checkpoint(chain: &[CanonicalConversationMessage]) -> ConversationCheckpoint {
    let observed =
        compute_portable_conversation_tail(TailPartitionKind::FixedPaginatedTail, 3, chain)
            .unwrap();
    ConversationCheckpoint {
        accepted_subagents: Default::default(),
        generation: 1,
        accepted: Some(AcceptedConversationVersion {
            artifact_sha256: "a".repeat(64),
            accepted_at_commit_id: "commit-1".to_string(),
            accepted_at_manifest_sha256: "d".repeat(64),
            portable_tail: observed.clone(),
        }),
        accepted_local_tail: Some(observed.clone()),
        observed_tail: observed,
        physical: ready_mapping(2_000),
        prefix_witness: conversation_prefix_witness(chain, 3).unwrap(),
        binding_revision: 7,
    }
}

#[test]
fn a_stale_acceptance_is_read_as_absent_and_never_forgotten_by_reading() {
    let chain = tail_chain();
    let names = record_names(&[(1, None, 10), (2, Some(0), 20), (3, Some(0), 21)]);
    let mainline = Mainline::select(names.iter().cloned());
    let accepted_in = |name: &RecordName| {
        let mut checkpoint = fixed_checkpoint(&chain);
        checkpoint.accepted.as_mut().unwrap().accepted_at_commit_id = name.commit_id().to_string();
        checkpoint.accepted_subagents.insert(
            SubagentKey::parse("child-call").unwrap(),
            AcceptedSubagentState {
                local: SubagentContentState {
                    message_count: 2,
                    sha256: "a".repeat(64),
                },
                normalized: SubagentContentState {
                    message_count: 2,
                    sha256: "b".repeat(64),
                },
            },
        );
        checkpoint.validate().unwrap();
        checkpoint
    };

    // Accepted in a publication on the selected chain: read as saved.
    let usable = accepted_in(&names[0]);
    assert_eq!(usable.read_on(&mainline), usable);

    // Accepted in a publication that is listed and not selected, or that is
    // not listed at all: the acceptance and its subagents read as absent,
    // and the cursor, the mapping and the generation are untouched.
    let stale = accepted_in(&names[2]);
    for chain_read in [&mainline, &Mainline::default()] {
        let saved = if chain_read.is_empty() {
            &usable
        } else {
            &stale
        };
        let read = saved.read_on(chain_read);
        assert!(read.accepted.is_none());
        assert!(read.accepted_local_tail.is_none());
        assert!(read.accepted_subagents.is_empty());
        assert_eq!(read.generation, saved.generation);
        assert_eq!(read.observed_tail, saved.observed_tail);
        assert_eq!(read.physical, saved.physical);
        assert_eq!(read.prefix_witness, saved.prefix_witness);
        read.validate().unwrap();
    }
    // Reading decides again every time: the saved checkpoint counts again
    // when its publication is listed again.
    assert_eq!(usable.read_on(&Mainline::default()).accepted, None);
    assert_eq!(usable.read_on(&mainline), usable);
}

#[test]
fn forgetting_an_acceptance_keeps_the_cursor_and_moves_the_generation() {
    let chain = tail_chain();
    let mut checkpoint = fixed_checkpoint(&chain);
    let before = checkpoint.clone();
    assert!(checkpoint.forget_acceptance());
    assert!(checkpoint.accepted.is_none());
    assert!(checkpoint.accepted_local_tail.is_none());
    assert!(checkpoint.accepted_subagents.is_empty());
    assert_eq!(checkpoint.generation, before.generation + 1);
    assert_eq!(checkpoint.observed_tail, before.observed_tail);
    assert_eq!(checkpoint.physical, before.physical);
    assert_eq!(checkpoint.prefix_witness, before.prefix_witness);
    assert_eq!(checkpoint.binding_revision, before.binding_revision);
    checkpoint.validate().unwrap();
    // Nothing left to forget: the generation does not move again.
    let forgotten = checkpoint.clone();
    assert!(!checkpoint.forget_acceptance());
    assert_eq!(checkpoint, forgotten);
}

#[test]
fn conversation_checkpoints_validate_layout_witness_and_mapping() {
    let chain = tail_chain();
    let checkpoint = fixed_checkpoint(&chain);
    checkpoint.validate().unwrap();

    // The prefix witness matches the truncated whole-chain digests and is
    // absent exactly at a zero boundary.
    let witness = conversation_prefix_witness(&chain, 3).unwrap().unwrap();
    let (lineage, semantic) = conversation_chain_digests(&chain[..3]).unwrap();
    assert_eq!(witness.lineage_digest, lineage);
    assert_eq!(witness.semantic_digest, semantic);
    assert!(conversation_prefix_witness(&chain, 0).unwrap().is_none());
    assert!(conversation_prefix_witness(&chain, 6).is_err());

    // Witness presence follows the layout.
    let mut missing_witness = checkpoint.clone();
    missing_witness.prefix_witness = None;
    assert!(missing_witness.validate().is_err());
    let mut whole = checkpoint.clone();
    whole.observed_tail = whole_tail_over(&chain);
    whole.accepted = None;
    whole.accepted_local_tail = None;
    assert!(whole.validate().is_err());
    whole.prefix_witness = None;
    whole.validate().unwrap();

    // The dual pins are recorded together; a half-present pair or torn
    // coordinates fail validation.
    let mut half_pinned = checkpoint.clone();
    half_pinned.accepted_local_tail = None;
    assert!(half_pinned.validate().unwrap_err().contains("together"));
    let mut torn = checkpoint.clone();
    if let Some(accepted) = &mut torn.accepted {
        accepted.portable_tail.accepted_end_record_count += 1;
        accepted.portable_tail.accepted_tail_state = "e".repeat(64);
    }
    assert!(torn
        .validate()
        .unwrap_err()
        .contains("partition or coordinates"));
    // The wire pin may carry a different codec label; only the local twin
    // must share the observed layout.
    let mut dual_form = checkpoint.clone();
    if let Some(accepted) = &mut dual_form.accepted {
        accepted.portable_tail.codec = PORTABLE_TAIL_CODEC_WIRE_V1.to_string();
        accepted.portable_tail.accepted_tail_state = "f".repeat(64);
    }
    dual_form.validate().unwrap();

    // Accepted and observed layouts must agree.
    let mut split_layout = checkpoint.clone();
    split_layout.observed_tail = whole_tail_over(&chain);
    split_layout.prefix_witness = None;
    assert!(split_layout
        .validate()
        .unwrap_err()
        .contains("layouts disagree"));

    // The raw range pin ends at the mapping cursor.
    let mut torn_pin = checkpoint.clone();
    if let PhysicalMappingState::Ready { raw_range_pin, .. } = &mut torn_pin.physical {
        raw_range_pin.end_byte_offset = 1_999;
    }
    assert!(torn_pin.validate().unwrap_err().contains("mapping cursor"));

    // Mapping bounds fail closed.
    let mut shrunk = checkpoint.clone();
    if let PhysicalMappingState::Ready { root_mapping, .. } = &mut shrunk.physical {
        root_mapping.source_file_size = 1_000;
    }
    assert!(shrunk.validate().is_err());
    let mut empty_probe = checkpoint.clone();
    if let PhysicalMappingState::Ready { root_mapping, .. } = &mut empty_probe.physical {
        root_mapping.probe_window_len = 0;
    }
    assert!(empty_probe.validate().is_err());

    // Native paginated ordinals are global across the resolved lineage, so
    // a small append-bearing segment can have a next ordinal greater than
    // its local byte cursor.
    let mut global_ordinal = checkpoint.clone();
    if let PhysicalMappingState::Ready { root_mapping, .. } = &mut global_ordinal.physical {
        root_mapping.consumed_record_ordinal = root_mapping.consumed_byte_offset + 1;
    }
    global_ordinal.validate().unwrap();

    // Detached preserves accepted evidence and the witness rule.
    let mut detached = checkpoint.clone();
    detached.physical = PhysicalMappingState::Detached {
        expected_provider_thread_id: "0199a1b2-0000-7000-8000-000000000002".to_string(),
        expected_binding_revision: 9,
    };
    detached.validate().unwrap();

    // Link-level: conversation checkpoints live only in provider scopes.
    let project = sample_project();
    let mut config = SyncConfig {
        schema: LOCAL_SCHEMA,
        revision: 0,
        storages: vec![sample_storage()],
        projects: vec![project.clone()],
        links: vec![ProjectStorageLink {
            managed_cloud_repository_id: None,
            local_project_id: project.local_project_id.clone(),
            storage_id: StorageId::parse("personal").unwrap(),
            bundle_id: bundle_id(),
            recipe: None,
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::from([(
                SyncScopeKey::Project,
                BTreeMap::from([(ThreadId::parse("thread-1").unwrap(), checkpoint.clone())]),
            )]),
            pinned: false,
            created_at: 1,
        }],
    };
    assert!(config
        .validate()
        .unwrap_err()
        .contains("outside a provider scope"));
    config.links[0].conversation_checkpoints = BTreeMap::from([(
        SyncScopeKey::Provider(provider("codex")),
        BTreeMap::from([(ThreadId::parse("thread-1").unwrap(), checkpoint)]),
    )]);
    config.validate().unwrap();
}

#[test]
fn conversation_bindings_carry_validated_layouts() {
    let binding = LocalConversationBinding {
        installed_end: None,
        pulled_children_excluded: false,
        installed_subagents: Default::default(),
        path_origins_ref: None,
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        thread_id: ThreadId::parse("thread-1").unwrap(),
        cloud_thread_id: Some(CloudThreadId::parse("cloudthread-1").unwrap()),
        forked_from: None,
        preserved: None,
        root_tail_layout: Some(RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: TailPartitionKind::FixedPaginatedTail,
            start_record_count: 3,
        }),
        provider_thread_id: Some("0199a1b2-0000-7000-8000-000000000001".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        revision: 1,
        created_at: 1,
        updated_at: 2,
    };
    binding.validate().unwrap();

    let mut encoded = serde_json::to_value(&binding).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("pulled_children_excluded");
    let without_exclusion: LocalConversationBinding = serde_json::from_value(encoded).unwrap();
    assert!(!without_exclusion.pulled_children_excluded);
    without_exclusion.validate().unwrap();
    let mut excluded = binding.clone();
    excluded.pulled_children_excluded = true;
    assert!(
        excluded.validate().is_err(),
        "native threads cannot carry Pull exclusions"
    );
    excluded.installed_end = Some(128);
    excluded.validate().unwrap();
    excluded.installed_subagents.insert(
        SubagentKey::child(None, "start").unwrap(),
        InstalledSubagent {
            provider_thread_id: "child-1".into(),
            installed_end: 64,
        },
    );
    assert!(
        excluded.validate().is_err(),
        "excluded children cannot be installed"
    );

    let mut moved_whole = binding.clone();
    moved_whole.root_tail_layout = Some(RootTailLayout {
        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 3,
    });
    assert!(moved_whole.validate().is_err());

    let mut malformed_codec = binding.clone();
    malformed_codec.root_tail_layout = Some(RootTailLayout {
        codec: "not a codec".to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
    });
    assert!(malformed_codec.validate().is_err());
}

#[test]
fn preserved_titles_compose_and_strip_deterministically() {
    assert_eq!(
        compose_preserved_title(Some("Fix parser"), 1, "Machine A"),
        "Fix parser (Preserved 1 · Machine A)"
    );
    // Without a usable base the marker stands alone, and stripping it
    // returns the empty base.
    assert_eq!(
        compose_preserved_title(None, 2, "Machine A"),
        "Preserved 2 · Machine A"
    );
    assert_eq!(
        compose_preserved_title(Some("   "), 2, "Machine A"),
        "Preserved 2 · Machine A"
    );
    assert_eq!(strip_preserved_marker("Preserved 2 · Machine A"), "");
    // Re-preserving a preserved copy replaces the marker; it never nests.
    assert_eq!(
        compose_preserved_title(Some("Fix parser (Preserved 1 · Machine A)"), 2, "Machine B"),
        "Fix parser (Preserved 2 · Machine B)"
    );
    assert_eq!(
        compose_preserved_title(Some("Preserved 1 · Machine A"), 2, "Machine B"),
        "Preserved 2 · Machine B"
    );
    // Unicode device labels survive intact in both directions.
    assert_eq!(
        compose_preserved_title(Some("Fix parser"), 3, "机器 A"),
        "Fix parser (Preserved 3 · 机器 A)"
    );
    assert_eq!(
        strip_preserved_marker("Fix parser (Preserved 3 · 机器 A)"),
        "Fix parser"
    );
    // Titles without a marker come back unchanged.
    assert_eq!(
        strip_preserved_marker("Fix parser (part 2)"),
        "Fix parser (part 2)"
    );
    // The base char-truncates so the whole title fits the read-side budget
    // and the marker always survives whole.
    let long_base = "b".repeat(400);
    let composed = compose_preserved_title(Some(&long_base), 12, "Machine A");
    assert_eq!(composed.chars().count(), PRESERVED_TITLE_MAX_CHARS);
    assert!(composed.ends_with("(Preserved 12 · Machine A)"));
    let marker_cost = " (Preserved 12 · Machine A)".chars().count();
    assert_eq!(
        strip_preserved_marker(&composed),
        "b".repeat(PRESERVED_TITLE_MAX_CHARS - marker_cost)
    );
}

#[test]
fn preserved_provenance_validates_on_every_carrier() {
    // Provenance is always cloud-space; a local carrier can no longer be
    // named by it at all, so the self-preserve check exists only where the
    // carrier is itself cloud-space (cloud rows).
    let carrier = CloudThreadId::parse("cloudthread-copy").unwrap();
    let origin = CloudThreadId::parse("cloudthread-origin").unwrap();
    PreservedProvenance {
        preserved_from: origin.clone(),
        ordinal: 1,
    }
    .validate_for_cloud_row(&carrier)
    .unwrap();
    assert!(PreservedProvenance {
        preserved_from: origin.clone(),
        ordinal: 0,
    }
    .validate()
    .unwrap_err()
    .contains("zero ordinal"));
    assert!(PreservedProvenance {
        preserved_from: carrier.clone(),
        ordinal: 1,
    }
    .validate_for_cloud_row(&carrier)
    .unwrap_err()
    .contains("preserved from itself"));

    // Binding-level: the local carrier is type-distinct from the cloud-space
    // provenance, so only the ordinal rule applies.
    let binding = LocalConversationBinding {
        installed_end: None,
        pulled_children_excluded: false,
        installed_subagents: Default::default(),
        path_origins_ref: None,
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        thread_id: ThreadId::parse("thread-copy").unwrap(),
        cloud_thread_id: None,
        forked_from: None,
        preserved: Some(PreservedProvenance {
            preserved_from: origin.clone(),
            ordinal: 1,
        }),
        root_tail_layout: None,
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    };
    binding.validate().unwrap();
    let mut zero_ordinal = binding;
    zero_ordinal.preserved = Some(PreservedProvenance {
        preserved_from: origin.clone(),
        ordinal: 0,
    });
    assert!(zero_ordinal.validate().is_err());

    // Thread-level: a portable conversation record enforces the same rules.
    let parts = sealed_provider_scope("codex", "prov");
    let mut thread = parts
        .scope
        .conversations
        .get(&parts.thread_id)
        .unwrap()
        .clone();
    thread.preserved = Some(PreservedProvenance {
        preserved_from: origin.clone(),
        ordinal: 1,
    });
    thread.validate().unwrap();
    thread.preserved = Some(PreservedProvenance {
        preserved_from: thread.thread_id.clone(),
        ordinal: 1,
    });
    assert!(thread
        .validate()
        .unwrap_err()
        .contains("preserved from itself"));
    thread.preserved = Some(PreservedProvenance {
        preserved_from: origin,
        ordinal: 0,
    });
    assert!(thread.validate().unwrap_err().contains("zero ordinal"));
}

#[test]
fn apply_index_rows_pair_with_the_index_fragment_role() {
    let fragment = ConversationApplyTarget {
        target_path: "/home/user/.codex/session_index.jsonl".to_string(),
        role: ConversationFileRole::ProviderIndexFragment,
        expected: TargetExpectation::Absent,
        staged_path: None,
        backup_path: None,
        staged_sha256: None,
        index_rows: vec![
            "0199a1b2-0000-7000-8000-000000000001".to_string(),
            "0199a1b2-0000-7000-8000-000000000002".to_string(),
        ],
    };
    fragment.validate().unwrap();

    // An index fragment must name its rows; other roles must not.
    let mut rowless = fragment.clone();
    rowless.index_rows = Vec::new();
    assert!(rowless
        .validate()
        .unwrap_err()
        .contains("index rows must pair"));
    let mut transcript_with_rows = fragment.clone();
    transcript_with_rows.role = ConversationFileRole::Transcript;
    assert!(transcript_with_rows
        .validate()
        .unwrap_err()
        .contains("index rows must pair"));

    let mut repeated = fragment.clone();
    repeated.index_rows = vec![
        "0199a1b2-0000-7000-8000-000000000001".to_string(),
        "0199a1b2-0000-7000-8000-000000000001".to_string(),
    ];
    assert!(repeated
        .validate()
        .unwrap_err()
        .contains("repeats an index row"));
    let mut oversized = fragment;
    oversized.index_rows = (0..9)
        .map(|index| format!("0199a1b2-0000-7000-8000-00000000000{index}"))
        .collect();
    assert!(oversized
        .validate()
        .unwrap_err()
        .contains("too many index rows"));
}

// --- Journal payloads --------------------------------------------------------

fn cloud_anchor() -> ConversationCloudAnchor {
    ConversationCloudAnchor {
        storage_id: StorageId::parse("personal").unwrap(),
        bundle_id: bundle_id(),
        revision: 3,
        commit_id: "commit-3".to_string(),
        manifest_sha256: "e".repeat(64),
    }
}

fn publication_operation(chain: &[CanonicalConversationMessage]) -> ConversationApplyOperation {
    let before = whole_tail_over(&chain[..4]);
    let after = whole_tail_over(chain);
    ConversationApplyOperation {
        operation_id: ConversationOperationId::parse("op-1").unwrap(),
        plan_id: Some(PlanId::parse("plan-1").unwrap()),
        plan_action_id: Some(ActionId::parse("action-1").unwrap()),
        cloud: cloud_anchor(),
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        provider: provider("codex"),
        thread_id: ThreadId::parse("thread-1").unwrap(),
        cloud_thread_id: CloudThreadId::parse("cloudthread-1").unwrap(),
        operation: ConversationApplyKind::CloudPublication(ConversationPublicationApply {
            export_settings_changed: false,
            candidate_commit_id: "commit-4".to_string(),
            candidate_manifest_sha256: "f".repeat(64),
            artifact_sha256: "a".repeat(64),
            tail_before: Some(before),
            tail_after: after,
            planned_relation: TailClassification::LocalOutstanding,
        }),
        provider_thread_id: Some("0199a1b2-0000-7000-8000-000000000001".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        expected_host_identity_revision: 1,
        expected_bindings_document_revision: 2,
        expected_binding_revision: Some(3),
        expected_profile_revision: 4,
        expected_project_binding_revision: 5,
        targets: Vec::new(),
        created_at: 1,
        updated_at: 2,
    }
}

#[test]
fn apply_operations_validate_payloads() {
    let chain = tail_chain();
    let operation = publication_operation(&chain);
    operation.validate().unwrap();
    assert_eq!(operation.direction(), ConversationPlanDirection::Push);

    // CreateCloud has no accepted-before value; LocalOutstanding requires one.
    let mut create_with_before = operation.clone();
    if let ConversationApplyKind::CloudPublication(publication) = &mut create_with_before.operation
    {
        publication.planned_relation = TailClassification::MissingCloud;
    }
    assert!(create_with_before.validate().is_err());
    let mut outstanding_without_before = operation.clone();
    if let ConversationApplyKind::CloudPublication(publication) =
        &mut outstanding_without_before.operation
    {
        publication.tail_before = None;
    }
    assert!(outstanding_without_before.validate().is_err());

    // A publication that moves the layout is rejected outright.
    let mut moved_layout = operation.clone();
    if let ConversationApplyKind::CloudPublication(publication) = &mut moved_layout.operation {
        publication.tail_before = Some(
            compute_portable_conversation_tail(TailPartitionKind::FixedPaginatedTail, 3, &chain)
                .unwrap(),
        );
    }
    assert!(moved_layout
        .validate()
        .unwrap_err()
        .contains("moves the portable tail layout"));

    // Push reserves no provider targets.
    let mut with_targets = operation.clone();
    with_targets.targets.push(ConversationApplyTarget {
        target_path: "/home/user/.codex/sessions/new.jsonl".to_string(),
        role: ConversationFileRole::Transcript,
        expected: TargetExpectation::Absent,
        staged_path: None,
        backup_path: None,
        staged_sha256: None,
        index_rows: vec![],
    });
    assert!(with_targets.validate().is_err());

    // A non-publishing relation can never reach a publication journal.
    let mut wrong_relation = operation.clone();
    if let ConversationApplyKind::CloudPublication(publication) = &mut wrong_relation.operation {
        publication.planned_relation = TailClassification::Divergent;
    }
    assert!(wrong_relation.validate().is_err());

    // Materialization: keep-both exactly for Divergent, never self-preserving.
    let host = HostId::parse("00112233445566778899aabbccddeeff").unwrap();
    let mut materialization = operation.clone();
    materialization.operation =
        ConversationApplyKind::ProviderMaterialization(ConversationMaterializationApply {
            planned_relation: TailClassification::Divergent,
            accepted_tail_after: whole_tail_over(&chain),
            pinned_lineage_digest: "a".repeat(64),
            pinned_semantic_digest: "b".repeat(64),
            keep_both: Some(KeepBothApply {
                preserved_thread_id: ThreadId::parse("thread-preserved").unwrap(),
                remote_provider_thread_id: "0199a1b2-0000-7000-8000-000000000002".to_string(),
                preserved: PreservedProvenance {
                    preserved_from: CloudThreadId::parse("cloudthread-1").unwrap(),
                    ordinal: 1,
                },
                preserved_display_title: "Conversation (Preserved 1 · Machine A)".to_string(),
            }),
            expected_config_revision: 0,
            sync_config_after: SyncConfig::default(),
            bindings_after: ConversationBindings::empty(host),
        });
    materialization.validate().unwrap();
    assert_eq!(materialization.direction(), ConversationPlanDirection::Pull);

    let mut self_preserving = materialization.clone();
    if let ConversationApplyKind::ProviderMaterialization(pull) = &mut self_preserving.operation {
        pull.keep_both.as_mut().unwrap().preserved_thread_id = ThreadId::parse("thread-1").unwrap();
    }
    assert!(self_preserving
        .validate()
        .unwrap_err()
        .contains("own thread"));

    // The minted provenance must name the operation's own cloud thread.
    let mut mismatched_provenance = materialization.clone();
    if let ConversationApplyKind::ProviderMaterialization(pull) =
        &mut mismatched_provenance.operation
    {
        pull.keep_both.as_mut().unwrap().preserved.preserved_from =
            CloudThreadId::parse("cloudthread-other").unwrap();
    }
    assert!(mismatched_provenance
        .validate()
        .unwrap_err()
        .contains("must name the operation's cloud thread"));

    let mut keep_both_without_divergence = materialization.clone();
    if let ConversationApplyKind::ProviderMaterialization(pull) =
        &mut keep_both_without_divergence.operation
    {
        pull.planned_relation = TailClassification::CloudOutstanding;
    }
    assert!(keep_both_without_divergence.validate().is_err());

    let mut sync_relation = materialization.clone();
    if let ConversationApplyKind::ProviderMaterialization(pull) = &mut sync_relation.operation {
        pull.planned_relation = TailClassification::Sync(SyncCurrency::Current);
        pull.keep_both = None;
    }
    assert!(sync_relation.validate().is_err());
}

// --- Plan action rules -------------------------------------------------------

#[test]
fn plan_actions_validate_relation_action_and_pins() {
    let chain = tail_chain();
    let t4 = whole_tail_over(&chain[..4]);
    let t5 = whole_tail_over(&chain);

    // The action is checked against the relation recomputed from nested pins.
    let mut wrong_relation = push_action("action-1", "codex", "profile-codex", "1");
    wrong_relation.pins.tail.accepted = Some(t4.clone());
    wrong_relation.pins.tail.accepted_local = Some(t4.clone());
    wrong_relation.pins.tail.local = Some(t5.clone());
    wrong_relation.pins.tail.cloud = Some(t4.clone());
    assert!(wrong_relation
        .validate()
        .unwrap_err()
        .contains("disagrees with its tail evidence"));

    // A pair without an acceptance reads Divergent: a publishing action
    // cannot carry it, so a stored copy is never replaced unproven.
    let mut extra_cloud_pin = push_action("action-2", "codex", "profile-codex", "2");
    extra_cloud_pin.pins.tail.cloud = Some(t5.clone());
    assert_eq!(
        extra_cloud_pin.pins.tail.classification(),
        Ok(TailClassification::Divergent)
    );
    assert!(extra_cloud_pin
        .validate()
        .unwrap_err()
        .contains("disagrees with its tail evidence"));

    // Publishing actions carry the raw range pin.
    let mut no_raw_range = push_action("action-3", "codex", "profile-codex", "3");
    no_raw_range.pins.tail.raw_range = None;
    assert!(no_raw_range.validate().is_err());

    let mut publish = push_action("action-4", "codex", "profile-codex", "4");
    publish.action = ConversationPlanActionType::Publish;
    // Publish always carries its mapping; only an unmapped CreateCloud may
    // omit the cloud id (minted at apply).
    publish.cloud_thread_id = Some(CloudThreadId::parse("cloudthread-4").unwrap());
    publish.pins.tail.accepted = Some(t4.clone());
    publish.pins.tail.accepted_local = Some(t4.clone());
    publish.pins.tail.local = Some(t5.clone());
    publish.pins.tail.cloud = Some(t4.clone());
    publish.validate().unwrap();

    publish.pins.tail.local = Some(t4.clone());
    assert!(publish.validate().is_err());
    publish.pins.export_settings_changed = true;
    publish.validate().unwrap();
    publish.pins.tail.local = Some(t5.clone());
    publish.pins.tail.cloud = Some(t5.clone());
    assert!(
        publish.validate().is_err(),
        "mode change must not bypass metadata adoption"
    );

    // Informational outcomes use the small review row and cannot inhabit the
    // executable action enum.
    let informational = ConversationReviewRow {
        stored_without_credential_redaction: false,
        provider: provider("codex"),
        thread_id: Some(ThreadId::parse("thread-info").unwrap()),
        cloud_thread_id: None,
        resource_id: ResourceId::parse("codex:conversation:thread-info").unwrap(),
        action: ConversationReviewAction::NoChanges,
        blocker: None,
        blocked_reason: None,
        action_id: None,
        display_title: None,
    };
    informational.validate().unwrap();
    // A title is optional, but "untitled" is one state on the wire: absent.
    // An empty string would be a second spelling of it.
    let mut titled = informational.clone();
    titled.display_title = Some("Add horror story document".to_string());
    titled.validate().unwrap();
    let mut blank = informational.clone();
    blank.display_title = Some(String::new());
    assert!(blank.validate().is_err());
    // A row must name at least one identity.
    let mut nameless = informational.clone();
    nameless.thread_id = None;
    assert!(nameless
        .validate()
        .unwrap_err()
        .contains("names no identity"));

    // Keep-both is pure intent: it carries the id pair and mints nothing.
    let mut keep_both = push_action("action-6", "codex", "profile-codex", "6");
    keep_both.action = ConversationPlanActionType::KeepBoth;
    keep_both.cloud_thread_id = Some(CloudThreadId::parse("cloudthread-6").unwrap());
    keep_both.pins.tail.accepted = Some(t4.clone());
    keep_both.pins.tail.accepted_local = Some(t4.clone());
    keep_both.pins.tail.local = Some(t5.clone());
    let mut cloud_divergence = t5.clone();
    cloud_divergence.accepted_tail_state = "e".repeat(64);
    keep_both.pins.tail.cloud = Some(cloud_divergence);
    keep_both.pins.tail.raw_range = None;
    keep_both.pins.tail.lineage_digest = Some("a".repeat(64));
    keep_both.pins.tail.semantic_digest = Some("b".repeat(64));
    keep_both.validate().unwrap();
    // Every pull action names its reviewed cloud row.
    let mut cloudless = keep_both.clone();
    cloudless.cloud_thread_id = None;
    assert!(cloudless
        .validate()
        .unwrap_err()
        .contains("requires its reviewed cloud thread id"));

    // Materializing actions need manifest digests; provider identities are
    // minted at apply and cannot inhabit a plan at all.
    let mut materialize = push_action("action-7", "codex", "profile-codex", "7");
    materialize.action = ConversationPlanActionType::MaterializeCloud;
    materialize.cloud_thread_id = Some(CloudThreadId::parse("cloudthread-7").unwrap());
    materialize.pins.tail.accepted = Some(t4.clone());
    materialize.pins.tail.accepted_local = Some(t4.clone());
    materialize.pins.tail.local = Some(t4);
    materialize.pins.tail.cloud = Some(t5);
    materialize.pins.tail.raw_range = None;
    materialize.pins.tail.lineage_digest = Some("a".repeat(64));
    materialize.pins.tail.semantic_digest = Some("b".repeat(64));
    materialize.validate().unwrap();
    let mut no_digests = materialize.clone();
    no_digests.pins.tail.lineage_digest = None;
    no_digests.pins.tail.semantic_digest = None;
    assert!(no_digests.validate().is_err());

    // CreateLocal is the only action without a local id; the pair matrix
    // rejects every other combination.
    let mut local_missing = materialize.clone();
    local_missing.thread_id = None;
    assert!(local_missing
        .validate()
        .unwrap_err()
        .contains("local identity disagrees with its action"));

    // An action cannot be retagged without changing its tail evidence.
    let mut pull_publish = keep_both.clone();
    pull_publish.action = ConversationPlanActionType::Publish;
    assert!(pull_publish
        .validate()
        .unwrap_err()
        .contains("disagrees with its tail evidence"));
}

// --- Codex wire rules --------------------------------------------------------

#[test]
fn codex_threads_require_an_end_aligned_portable_tail() {
    let parts = sealed_provider_scope("codex", "0123456789abcdef0123456789abcdef");
    let scope_key = SyncScopeKey::Provider(provider("codex"));

    let mut absent = parts.scope.clone();
    absent
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .portable_tail = None;
    absent.content_sha256 = absent.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), absent)]));
    manifest.validate().unwrap();
    assert!(manifest
        .resolve_all(&portable::test_resolver::Version1Resolver)
        .unwrap_err()
        .contains("requires a portable tail"));

    let mut misaligned = parts.scope.clone();
    misaligned
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .portable_tail
        .as_mut()
        .unwrap()
        .accepted_end_record_count = 1;
    misaligned.content_sha256 = misaligned.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key.clone(), misaligned)]));
    assert!(manifest.validate().is_err());

    let mut foreign_codec = parts.scope;
    foreign_codec
        .conversations
        .values_mut()
        .next()
        .unwrap()
        .portable_tail
        .as_mut()
        .unwrap()
        .codec = "claude.lineage.v1".to_string();
    foreign_codec.content_sha256 = foreign_codec.compute_content_sha256(&scope_key).unwrap();
    let manifest = scoped_manifest(BTreeMap::from([(scope_key, foreign_codec)]));
    manifest.validate().unwrap();
    assert!(manifest
        .resolve_all(&portable::test_resolver::Version1Resolver)
        .unwrap_err()
        .contains("unknown codex portable tail wire codec"));
}

#[test]
fn reshaped_documents_reject_unknown_and_retired_keys() {
    let mut storage = serde_json::to_value(sample_storage()).unwrap();
    storage.as_object_mut().unwrap().insert(
        "included_default_exclusions".to_string(),
        serde_json::json!([]),
    );
    assert!(serde_json::from_value::<StorageConfig>(storage).is_err());

    let chain = tail_chain();
    let checkpoint = fixed_checkpoint(&chain);
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("legacy_field".to_string(), serde_json::json!(1));
    assert!(serde_json::from_value::<ConversationCheckpoint>(value).is_err());

    // An unknown binding key fails closed without explicit enumeration.
    let binding = LocalConversationBinding {
        installed_end: None,
        pulled_children_excluded: false,
        installed_subagents: Default::default(),
        path_origins_ref: None,
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        thread_id: ThreadId::parse("thread-1").unwrap(),
        cloud_thread_id: None,
        forked_from: None,
        preserved: None,
        root_tail_layout: None,
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    };
    let mut value = serde_json::to_value(&binding).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("fork_stamp".to_string(), serde_json::json!({}));
    assert!(serde_json::from_value::<LocalConversationBinding>(value).is_err());

    // Schema 1 rejects the unsupported `preserved_local` key instead of
    // accepting it silently.
    let mut value = serde_json::to_value(&binding).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("preserved_local".to_string(), serde_json::json!(false));
    assert!(serde_json::from_value::<LocalConversationBinding>(value).is_err());

    // A link without the checkpoint map fails closed.
    let project = sample_project();
    let link = ProjectStorageLink {
        managed_cloud_repository_id: None,
        local_project_id: project.local_project_id.clone(),
        storage_id: StorageId::parse("personal").unwrap(),
        bundle_id: bundle_id(),
        recipe: None,
        scope_bases: BTreeMap::new(),
        project_content_preferences: Default::default(),
        conversation_checkpoints: BTreeMap::new(),
        pinned: false,
        created_at: 1,
    };
    let mut value = serde_json::to_value(&link).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("conversation_checkpoints");
    assert!(serde_json::from_value::<ProjectStorageLink>(value).is_err());

    // Journal operations are closed against unknown keys too.
    let operation = publication_operation(&chain);
    let mut value = serde_json::to_value(&operation).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("binding_forked_from".to_string(), serde_json::json!(null));
    assert!(serde_json::from_value::<ConversationApplyOperation>(value).is_err());
}

// --- Cloud thread id decoupling ------------------------------------------------

#[test]
fn thread_id_spaces_are_structurally_disjoint() {
    // The grammar boundary that makes cross-space collision unconstructible:
    // a CloudThreadId must use the cloud namespace, a ThreadId never can.
    CloudThreadId::parse("cloudthread-0123456789abcdef01234567").unwrap();
    assert!(CloudThreadId::parse("thread-0123456789abcdef01234567")
        .unwrap_err()
        .contains("must start with 'cloudthread-'"));
    assert!(ThreadId::parse("cloudthread-0123456789abcdef01234567")
        .unwrap_err()
        .contains("must not use the 'cloudthread-' cloud namespace"));
    ThreadId::parse("thread-0123456789abcdef01234567").unwrap();

    // A minted cloud id always parses into its own space and never the local
    // one.
    let minted = CloudThreadId::generate().unwrap();
    assert!(minted.as_str().starts_with("cloudthread-"));
    assert!(ThreadId::parse(minted.as_str()).is_err());

    // A manifest conversation row outside the cloud namespace fails at the
    // boundary: the map key cannot even deserialize as a CloudThreadId.
    let parts = sealed_provider_scope("codex", "boundary");
    let mut scope_value = serde_json::to_value(&parts.scope).unwrap();
    let conversations = scope_value
        .get_mut("conversations")
        .unwrap()
        .as_object_mut()
        .unwrap();
    let row = conversations
        .remove(parts.thread_id.as_str())
        .expect("sealed scope carries its conversation row");
    conversations.insert("thread-0123456789abcdef01234567".to_string(), row);
    assert!(serde_json::from_value::<BundleScope>(scope_value).is_err());
}

#[test]
fn documents_fail_closed_on_missing_identity_keys() {
    // Plain `Option` fields deserialize missing keys as `None` even without
    // `#[serde(default)]`; the presence-requiring deserializer is what makes
    // these keys mandatory. The fixtures omit the required identity keys.
    let binding = LocalConversationBinding {
        installed_end: None,
        pulled_children_excluded: false,
        installed_subagents: Default::default(),
        path_origins_ref: None,
        host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        replica_id: ReplicaId::parse("replica-a").unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        thread_id: ThreadId::parse("thread-1").unwrap(),
        cloud_thread_id: Some(CloudThreadId::parse("cloudthread-1").unwrap()),
        forked_from: None,
        preserved: None,
        root_tail_layout: None,
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    };
    let mut value = serde_json::to_value(&binding).unwrap();
    // An explicit null is a valid unmapped row; a missing key is not.
    value
        .as_object_mut()
        .unwrap()
        .insert("cloud_thread_id".to_string(), serde_json::Value::Null);
    let unmapped: LocalConversationBinding = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(unmapped.cloud_thread_id, None);
    value.as_object_mut().unwrap().remove("cloud_thread_id");
    assert!(serde_json::from_value::<LocalConversationBinding>(value).is_err());
    // The mapped row always serializes the key.
    let serialized = serde_json::to_value(&binding).unwrap();
    assert!(serialized
        .as_object()
        .unwrap()
        .contains_key("cloud_thread_id"));

    // A malformed CreateCloud action with a string `thread_id` and no
    // `cloud_thread_id` would otherwise execute silently under apply-minting.
    let action = push_action("action-missing-ids", "codex", "profile-codex", "1");
    let mut value = serde_json::to_value(&action).unwrap();
    {
        let object = value.as_object_mut().unwrap();
        object.insert(
            "thread_id".to_string(),
            serde_json::json!("thread-0123456789abcdef01234567"),
        );
        object.remove("cloud_thread_id");
    }
    assert!(serde_json::from_value::<ConversationPlanAction>(value).is_err());
    let mut value = serde_json::to_value(&action).unwrap();
    value.as_object_mut().unwrap().remove("thread_id");
    assert!(serde_json::from_value::<ConversationPlanAction>(value).is_err());
    // Unsupported minted-identity keys fail closed via deny_unknown_fields.
    let mut value = serde_json::to_value(&action).unwrap();
    value.as_object_mut().unwrap().insert(
        "proposed_provider_thread_id".to_string(),
        serde_json::json!("0199a1b2-0000-7000-8000-000000000001"),
    );
    assert!(serde_json::from_value::<ConversationPlanAction>(value).is_err());
    let mut value = serde_json::to_value(&action).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("keep_both".to_string(), serde_json::json!(null));
    assert!(serde_json::from_value::<ConversationPlanAction>(value).is_err());

    // A journal operation without `cloud_thread_id` fails on the required
    // field.
    let operation = publication_operation(&tail_chain());
    let mut value = serde_json::to_value(&operation).unwrap();
    value.as_object_mut().unwrap().remove("cloud_thread_id");
    assert!(serde_json::from_value::<ConversationApplyOperation>(value).is_err());

    // Review rows carry the same present-on-wire pair.
    let row = ConversationReviewRow {
        stored_without_credential_redaction: false,
        provider: provider("codex"),
        thread_id: Some(ThreadId::parse("thread-1").unwrap()),
        cloud_thread_id: None,
        resource_id: ResourceId::parse("codex:conversation:thread-1").unwrap(),
        action: ConversationReviewAction::NoChanges,
        blocker: None,
        blocked_reason: None,
        action_id: None,
        display_title: None,
    };
    let mut value = serde_json::to_value(&row).unwrap();
    value.as_object_mut().unwrap().remove("cloud_thread_id");
    assert!(serde_json::from_value::<ConversationReviewRow>(value).is_err());
}

#[test]
fn bindings_reject_duplicate_cloud_claims_per_scope() {
    let host = HostId::parse("00112233445566778899aabbccddeeff").unwrap();
    let row = |thread: &str, replica: &str, cloud: Option<&str>| LocalConversationBinding {
        installed_end: None,
        pulled_children_excluded: false,
        installed_subagents: Default::default(),
        path_origins_ref: None,
        host_id: host.clone(),
        replica_id: ReplicaId::parse(replica).unwrap(),
        local_project_id: LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-codex").unwrap(),
        thread_id: ThreadId::parse(thread).unwrap(),
        cloud_thread_id: cloud.map(|value| CloudThreadId::parse(value).unwrap()),
        forked_from: None,
        preserved: None,
        root_tail_layout: None,
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    };
    let mut document = ConversationBindings::empty(host.clone());
    document
        .bindings
        .push(row("thread-a", "replica-a", Some("cloudthread-shared")));
    document
        .bindings
        .push(row("thread-b", "replica-a", Some("cloudthread-shared")));
    assert!(document
        .validate_structure()
        .unwrap_err()
        .contains("claimed by multiple conversations"));

    // The same cloud id across scopes (different replicas) is legal, as are
    // any number of unmapped rows.
    let mut document = ConversationBindings::empty(host.clone());
    document
        .bindings
        .push(row("thread-a", "replica-a", Some("cloudthread-shared")));
    document
        .bindings
        .push(row("thread-b", "replica-b", Some("cloudthread-shared")));
    document.bindings.push(row("thread-c", "replica-a", None));
    document.bindings.push(row("thread-d", "replica-a", None));
    document.validate_structure().unwrap();
}

#[test]
fn storage_setup_draft_validate_bounds_half_edited_fields_without_echoing_values() {
    let draft = StorageSetupDraft {
        schema: STORAGE_SETUP_DRAFT_SCHEMA,
        draft_id: StorageSetupDraftId::parse("storagedraft-0123456789abcdef01234567").unwrap(),
        source: StorageSetupDraftSource::New,
        storage: StorageConfig {
            name: String::new(),
            provider: StorageProviderKey::parse("r2").unwrap(),
            provider_config: StorageProviderConfig::new(
                1,
                BoundedConfigObject::empty(),
                SecretMap::empty(),
            ),
            ..sample_storage()
        },
        revision: 0,
        created_at: 1,
        updated_at: 1,
    };
    // An all-empty half-edited form is a valid draft; strictness waits for
    // the adapter's structural validation and the commit transaction.
    draft.validate().unwrap();

    let mut wrong_schema = draft.clone();
    wrong_schema.schema = 2;
    assert!(wrong_schema
        .validate()
        .unwrap_err()
        .contains("unsupported storage setup draft schema"));

    let mut unversioned = draft.clone();
    unversioned.storage.provider_config.version = 0;
    assert!(unversioned
        .validate()
        .unwrap_err()
        .contains("config version must be positive"));

    let mut control_name = draft.clone();
    control_name.storage.name = "shown-once\u{1b}".to_string();
    let error = control_name.validate().unwrap_err();
    assert!(!error.contains("shown-once"));

    // Secrets are bounded by type: a control character never parses, and
    // the persisted shape rejects unknown fields.
    assert!(SecretValue::parse("shown-once\u{1b}").is_err());
    let mut value = serde_json::to_value(&draft).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("secret_access_key".to_string(), serde_json::json!("x"));
    assert!(serde_json::from_value::<StorageSetupDraft>(value).is_err());

    let mut missing_source = serde_json::to_value(&draft).unwrap();
    missing_source.as_object_mut().unwrap().remove("source");
    assert!(serde_json::from_value::<StorageSetupDraft>(missing_source).is_err());

    let mut backwards = draft.clone();
    backwards.created_at = 2;
    backwards.updated_at = 1;
    assert!(backwards.validate().unwrap_err().contains("timestamps"));
}

#[test]
fn missing_child_recovery_requires_exact_missing_evidence() {
    let mut action = push_action("recover-child", "claude", "profile-claude", "1");
    let main = action.pins.tail.local.clone().unwrap();
    action.action = ConversationPlanActionType::KeepBoth;
    action.cloud_thread_id = Some(CloudThreadId::parse("cloudthread-recover").unwrap());
    action.pins.tail.accepted = Some(main.clone());
    action.pins.tail.accepted_local = Some(main.clone());
    action.pins.tail.cloud = Some(main);
    action.pins.tail.raw_range = None;
    action.pins.tail.lineage_digest = Some("1".repeat(64));
    action.pins.tail.semantic_digest = Some("2".repeat(64));
    let key = SubagentKey::parse("child-call").unwrap();
    let content = SubagentContentState {
        message_count: 1,
        sha256: "3".repeat(64),
    };
    action.pins.tail.subagents.insert(
        key.clone(),
        SubagentTailPins {
            accepted: Some(AcceptedSubagentState {
                local: content.clone(),
                normalized: content.clone(),
            }),
            local: None,
            cloud: Some(content.clone()),
        },
    );
    assert!(action.validate().is_err());
    action.pins.tail.missing_local_subagents.insert(key.clone());
    assert_eq!(
        action.pins.tail.classification().unwrap(),
        TailClassification::Unknown
    );
    assert!(action.validate().is_ok());
    let saved = action.clone();
    action.pins.tail.subagents.get_mut(&key).unwrap().cloud = None;
    assert!(action.validate().is_err());
    action = saved.clone();
    action.pins.tail.subagents.get_mut(&key).unwrap().local = Some(content);
    assert!(action.validate().is_err());
    action = saved.clone();
    action.pins.tail.subagents.get_mut(&key).unwrap().accepted = None;
    assert!(action.validate().is_err());
    action = saved;
    action.pins.tail.local = None;
    assert!(action.validate().is_err());
    let decision = ClassifiedThread::new(
        TailClassification::Unknown,
        ConversationCapabilities::default(),
    )
    .decision(ConversationPlanDirection::Pull);
    assert_eq!(decision.action, ConversationReviewAction::Blocked);
}

#[test]
fn child_wire_stamp_is_required_and_off_rejects_children() {
    for provider in ["codex", "claude"] {
        let parts = sealed_provider_scope(provider, "stamp-test");
        let mut thread = parts.scope.conversations[&parts.thread_id].clone();
        let value = serde_json::to_value(&thread).unwrap();
        for invalid in [None, Some(serde_json::Value::Null), Some(serde_json::json!("unknown"))] {
            let mut bad = value.clone();
            match invalid {
                None => { bad.as_object_mut().unwrap().remove("wire_children"); }
                Some(value) => { bad["wire_children"] = value; }
            }
            assert!(serde_json::from_value::<ConversationThread>(bad).is_err());
        }
        thread.wire_children = ConversationWireChildren::Off;
        thread.validate().unwrap();
        let key = SubagentKey::parse("child-1").unwrap();
        thread.files.push(ConversationFileRef {
            logical_path: cloud_subagent_transcript_path(&thread.provider, &thread.thread_id, &key).unwrap(),
            role: ConversationFileRole::SubagentTranscript { child_key: key, normalized: SubagentContentState {
                message_count: 1, sha256: "a".repeat(64),
            } },
        });
        assert!(thread.validate().unwrap_err().contains("Off child stamp"));
        thread.wire_children = ConversationWireChildren::On;
        thread.validate().unwrap();
    }
}

#[test]
fn cloud_repository_id_is_optional_and_omitted_when_absent() {
    let value = serde_json::json!({
        "local_project_id": "project-optional-cloud", "storage_id": "storage-cloud",
        "bundle_id": "a".repeat(32), "scope_bases": {}, "conversation_checkpoints": {},
        "pinned": true, "created_at": 1
    });
    let mut link: ProjectStorageLink = serde_json::from_value(value).unwrap();
    assert!(link.managed_cloud_repository_id.is_none());
    assert!(serde_json::to_value(&link)
        .unwrap()
        .get("managed_cloud_repository_id")
        .is_none());
    link.managed_cloud_repository_id = Some("repo_exact".into());
    let encoded = serde_json::to_value(&link).unwrap();
    assert_eq!(encoded["managed_cloud_repository_id"], "repo_exact");
    assert_eq!(
        serde_json::from_value::<ProjectStorageLink>(encoded).unwrap(),
        link
    );
}

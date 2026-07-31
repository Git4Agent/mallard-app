# Excerpts: Rust tests stripped from surviving suites

Skill/plugin test cases removed from files that otherwise survive. Line
numbers are the pre-removal positions at the time each cut was made (cuts
landed in sequence, so ranges are relative to the then-current file). The
`capability-status` frontend suite was removed whole and archived verbatim
under `tests/frontend/`; persistence-test edits were small inline changes
recorded in the frontend/persistence diffs rather than here.


## bundle_engine.rs tests: plugin/global-skill fixtures and cases (pre-removal lines 2950–3404)
```rust
    #[test]
    fn plugin_install_intent_exists_only_in_the_dependency_plan_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        let mut request = CaptureRequest::for_project(&source);
        request.global_plugins.push(GlobalPluginSource {
            provider: CaptureProvider::Codex,
            plugin_id: "computer-use@openai-bundled".to_string(),
            marketplace: Some("openai-bundled".to_string()),
            source_type: None,
            source: None,
            observed_version: Some("1.0.0".to_string()),
            enabled: true,
            provided_skills: vec!["computer-use".to_string()],
        });
        let inventory = discover_project(&request).unwrap();
        let plugin = inventory
            .resources
            .iter()
            .find(|resource| resource.kind == CaptureResourceKind::Plugin)
            .unwrap();
        let recipe = recipe_for([plugin.resource_id.clone()]);
        let id = bundle_id(51);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Plugin intent"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        assert_eq!(fetched.dependency_actions.len(), 1);
        assert!(matches!(
            fetched.dependency_actions[0].kind,
            DependencyActionKind::InstallCodexPlugin
        ));
        let binding = active_binding(id, &target, None, None);
        let restore = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert!(restore.actions.is_empty());
    }

    /// Machine-A provider home with one global skill, ready for capture via
    /// the global inventory adapter.
    fn global_skill_fixture(temp: &Path) -> (PathBuf, CaptureRequest) {
        let project = temp.join("project");
        let codex_home = temp.join("codex-home");
        fs::create_dir_all(&project).unwrap();
        let skill = codex_home.join("skills/deploy");
        write(
            &skill.join("SKILL.md"),
            b"---\nname: deploy\n---\nDeploy helper\n",
        );
        write(&skill.join("bin/run.sh"), b"#!/bin/sh\necho deploy\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(skill.join("bin/run.sh"), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &codex_home,
        );
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex_home.clone()),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        (codex_home, request)
    }

    fn publish_skill_bundle(
        temp: &Path,
        request: &CaptureRequest,
        id: BundleId,
    ) -> BundleEngine<LocalBundleObjectStore> {
        let recipe = recipe_for(["codex:standalone-skill:deploy".to_string()]);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id, "Global skill"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        engine
    }

    #[test]
    fn custom_skill_installs_into_mapped_provider_home() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(21);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        // Machine B: fresh project and provider home.
        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .expect("one custom-skill install action");
        assert!(install.requires_explicit_approval);
        assert!(install.expected_target_sha256.is_none());
        assert!(!plan
            .actions
            .iter()
            .any(|action| matches!(action.kind, RestoreActionKind::WriteFile { .. })));

        // Keep is the default: an unapproved action mutates nothing.
        let skipped = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &BTreeSet::new(),
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(skipped
            .receipts
            .iter()
            .all(|receipt| receipt.status == ActionStatus::Skipped));
        assert!(!target_home.join("skills/deploy").exists());

        let approved = plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                102,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::InstallCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        assert!(receipt.target_sha256_after.is_some());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/bin/run.sh")).unwrap(),
            b"#!/bin/sh\necho deploy\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(target_home.join("skills/deploy/bin/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit survives install");
            assert_eq!(mode & 0o7000, 0, "set-id bits are stripped");
        }

        // Idempotent replan: a matching target is an install no-op pinned to
        // the same digest.
        let replan = engine
            .build_restore_plan(&fetched, &binding, 103, 0)
            .unwrap();
        let noop = replan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        assert_eq!(noop.expected_target_sha256, noop.source_sha256);
    }

    #[test]
    fn custom_skill_overwrite_is_digest_pinned_and_backed_up() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(22);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        // A different local skill already occupies the target, including an
        // extra file the cloud version does not carry.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        write(&target_home.join("skills/deploy/local-note.txt"), b"mine");
        let binding = active_binding(id, &target_project, Some(&target_home), None);

        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .expect("a different target requires an explicit overwrite");
        assert!(overwrite.expected_target_sha256.is_some());
        assert_ne!(overwrite.expected_target_sha256, overwrite.source_sha256);

        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let backups_root = temp.path().join("backups");
        let applied = engine
            .apply_restore_plan(&fetched, &binding, &plan, &approved, &backups_root, 101)
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        // Whole-directory replacement removes files deleted by the cloud
        // version instead of leaving stale code behind.
        assert!(!target_home.join("skills/deploy/local-note.txt").exists());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nDeploy helper\n"
        );
        // The displaced directory is recoverable from the plan backup.
        assert_eq!(applied.backups.len(), 1);
        let backup_dir = PathBuf::from(&applied.backups[0].backup_path);
        assert_eq!(
            fs::read(backup_dir.join("local-note.txt")).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn custom_skill_target_change_after_planning_aborts_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(23);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .unwrap();

        // The target changes between review and apply.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nEdited after review\n",
        );
        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Failed);
        assert!(receipt
            .error
            .as_deref()
            .unwrap()
            .contains("changed after planning"));
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nEdited after review\n"
        );
    }

    #[test]
    fn custom_skill_preserves_install_directory_when_declared_name_differs() {
        let temp = tempfile::tempdir().unwrap();
        let source_project = temp.path().join("source-project");
        let source_home = temp.path().join("source-codex");
        fs::create_dir_all(&source_project).unwrap();
        write(
            &source_home.join("skills/capture-lsservice-detail/SKILL.md"),
            b"---\nname: get-real-hardware-rh-service\n---\nRun ~/.codex/skills/capture-lsservice-detail/scripts/run.py\n",
        );
        write(
            &source_home.join("skills/capture-lsservice-detail/scripts/run.py"),
            b"print('ok')\n",
        );
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &source_home,
        );
        assert!(inventory.blocked_skills.is_empty());
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: source_project,
            codex_home: Some(source_home),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let resource_id = "codex:standalone-skill:get-real-hardware-rh-service";
        let recipe = recipe_for([resource_id.to_string()]);
        let id = bundle_id(24);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Differently named skill"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let descriptor = fetched
            .snapshot
            .manifest
            .resources
            .get(&ResourceId::parse(resource_id).unwrap())
            .unwrap();
        assert_eq!(
            descriptor
                .metadata
                .get("effective_name")
                .map(String::as_str),
            Some("get-real-hardware-rh-service")
        );
        assert_eq!(
            descriptor
                .metadata
                .get("install_dir_name")
                .map(String::as_str),
            Some("capture-lsservice-detail")
        );

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        match &install.kind {
            RestoreActionKind::InstallCustomSkill { skill_name, .. } => {
                assert_eq!(skill_name, "get-real-hardware-rh-service")
            }
            _ => unreachable!(),
        }
        assert!(install
            .target_path
            .as_deref()
            .unwrap()
            .ends_with("skills/capture-lsservice-detail"));

        let approved = BTreeSet::from([install.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(applied
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
        assert!(target_home
            .join("skills/capture-lsservice-detail/SKILL.md")
            .is_file());
        assert!(!target_home
            .join("skills/get-real-hardware-rh-service")
            .exists());
    }

    #[test]
    fn unselected_global_resources_never_enter_the_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        // Discovery sees the skill, but the recipe selects nothing global.
        let captured = capture_selected(&request, &BTreeSet::new()).unwrap();
        assert!(captured.resources.is_empty());
        assert!(captured.files.is_empty());
        assert!(captured.dependency_actions.is_empty());
```

## commands.rs tests: thread/capability classification + dependency lock/diagnostics (pre-removal lines 11848–11887, 11905–12028)
```rust
    #[test]
    fn thread_sync_direction_uses_the_reviewed_base() {
        assert_eq!(
            classify_thread_sync_state(Some("same"), Some("same"), true, Some("base")),
            ThreadSyncState::Synced
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("base"), true, Some("base")),
            ThreadSyncState::LocalAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("base"), Some("storage"), true, Some("base")),
            ThreadSyncState::StorageAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("storage"), true, Some("base")),
            ThreadSyncState::Diverged
        );
        assert_eq!(
            classify_thread_sync_state(Some("new"), None, true, None),
            ThreadSyncState::LocalAhead
        );
        assert_eq!(
            classify_thread_sync_state(None, Some("new"), true, None),
            ThreadSyncState::StorageAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("storage"), false, None),
            ThreadSyncState::Unknown
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), None, false, None),
            ThreadSyncState::LocalOnly
        );
        assert_eq!(
            classify_thread_sync_state(None, Some("storage"), false, None),
            ThreadSyncState::StorageOnly
        );
    }

// ...
    #[test]
    fn capability_status_marks_two_unbased_versions_for_review() {
        assert_eq!(
            classify_capability_sync_state(Some("local"), Some("storage"), false, None),
            ThreadSyncState::Diverged
        );
        assert_eq!(
            classify_capability_sync_state(Some("same"), Some("same"), false, None),
            ThreadSyncState::Synced
        );
        assert_eq!(
            classify_capability_sync_state(Some("local"), None, false, None),
            ThreadSyncState::LocalOnly
        );
    }

    #[test]
    fn plugin_status_compares_portable_intent_not_observed_version() {
        let plugin = |source: &str, version: &str| ResourceDescriptor {
            resource_id: ResourceId::parse("codex:plugin:tools").unwrap(),
            kind: ResourceKind::Plugin,
            provider: Some(Provider::Codex),
            scope: ResourceScope::Dependency,
            display_name: "tools".to_string(),
            provenance: Provenance::Plugin {
                provider: Provider::Codex,
                plugin_id: "tools".to_string(),
            },
            apply_policy: ApplyPolicy::ExplicitInstall,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::from([
                ("plugin_source_type".to_string(), "git".to_string()),
                ("plugin_source".to_string(), source.to_string()),
                ("plugin_observed_version".to_string(), version.to_string()),
                (
                    "dependency_argv_json".to_string(),
                    "[\"plugin\",\"add\",\"tools\"]".to_string(),
                ),
            ]),
        };
        let first = plugin("https://example.test/tools.git", "1.0.0");
        let newer_observation = plugin("https://example.test/tools.git", "2.0.0");
        let different_source = plugin("https://mirror.test/tools.git", "2.0.0");

        assert_eq!(
            descriptor_capability_digest(Some(&first)),
            descriptor_capability_digest(Some(&newer_observation))
        );
        assert_ne!(
            descriptor_capability_digest(Some(&first)),
            descriptor_capability_digest(Some(&different_source))
        );
    }

    #[test]
    fn codex_dependency_builds_a_portable_managed_or_git_lock() {
        let resource_id = ResourceId::parse("codex:plugin:documents").unwrap();
        let action = DependencyAction {
            action_id: ActionId::parse("dependency:codex:plugin:documents").unwrap(),
            resource_id: resource_id.clone(),
            kind: DependencyActionKind::InstallCodexPlugin,
            display_name: "documents@openai-primary-runtime".to_string(),
            provider: Some(Provider::Codex),
            argv: vec![
                "plugin".to_string(),
                "add".to_string(),
                "documents@openai-primary-runtime".to_string(),
            ],
            requires_explicit_approval: true,
        };
        let mut descriptor = ResourceDescriptor {
            resource_id,
            kind: ResourceKind::Plugin,
            provider: Some(Provider::Codex),
            scope: ResourceScope::Dependency,
            display_name: action.display_name.clone(),
            provenance: Provenance::Plugin {
                provider: Provider::Codex,
                plugin_id: action.display_name.clone(),
            },
            apply_policy: ApplyPolicy::ExplicitInstall,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::from([
                (
                    "plugin_marketplace".to_string(),
                    "openai-primary-runtime".to_string(),
                ),
                ("plugin_source_type".to_string(), "local".to_string()),
            ]),
        };

        let managed = codex_plugin_lock_for_action(&action, &descriptor).unwrap();
        assert_eq!(managed.plugins[0].id, "documents@openai-primary-runtime");
        assert!(managed.marketplaces.is_empty());

        let mut git_action = action.clone();
        git_action.display_name = "documents@team-tools".to_string();
        git_action.argv[2] = git_action.display_name.clone();
        descriptor.display_name = git_action.display_name.clone();
        descriptor.metadata = BTreeMap::from([
            ("plugin_marketplace".to_string(), "team-tools".to_string()),
            ("plugin_source_type".to_string(), "git".to_string()),
            ("plugin_source".to_string(), "owner/repo".to_string()),
        ]);
        let git = codex_plugin_lock_for_action(&git_action, &descriptor).unwrap();
        assert_eq!(git.marketplaces[0].name, "team-tools");
        assert_eq!(git.marketplaces[0].repository, "owner/repo");
    }

    #[test]
    fn plugin_failure_diagnostics_are_redacted_and_bounded() {
        let noisy = format!(
            "Authorization: Bearer do-not-log\nactual catalog error\n{}",
            "x".repeat(MAX_PLUGIN_DIAGNOSTIC_BYTES * 2)
        );
        let detail = plugin_failure_detail(&[], noisy.as_bytes());
        assert!(detail.contains("[redacted]"));
        assert!(detail.contains("actual catalog error"));
        assert!(!detail.contains("do-not-log"));
        assert!(detail.len() <= MAX_PLUGIN_DIAGNOSTIC_BYTES);
    }

```

## commands.rs test: linked_local_only_project_publishes_with_an_unselected_blocked_skill (pre-removal lines 13805–13890)
```rust
    #[test]
    fn linked_local_only_project_publishes_with_an_unselected_blocked_skill() {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("local-only-publish").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Local-only destination".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(codex_home.join("skills/_backup/openai-docs")).unwrap();
        std::fs::write(
            codex_home.join("skills/_backup/openai-docs/SKILL.md"),
            "---\nname: openai-docs\n---\n",
        )
        .unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        let inventory =
            get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        let blocked = inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.display_name == "_backup")
            .unwrap();
        assert!(blocked.blocked_reason.is_some());
        assert!(!inventory
            .recipe
            .entries
            .contains_key(&blocked.descriptor.resource_id));

        let status = get_project_capability_status_with_repository(
            &repo,
            &project.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        assert!(status.items.iter().any(|item| {
            item.descriptor.resource_id == blocked.descriptor.resource_id
                && item.state == "blocked"
                && !item.selected_in_recipe
        }));

        let result =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(result.generation, Some(1));
        let remote = list_remote_bundle_snapshots_with_repository(&repo, &storage_id).unwrap();
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].bundle_id, project.bundle_id);
    }

```

## commands.rs tests: dependency plan generation/apply over S3 (pre-removal lines 14034–14183)
```rust
    #[tokio::test]
    async fn dependency_plan_generation_uses_a_valid_id_and_supports_retry() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("dependency-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Dependency store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();

        let binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .cloned()
            .unwrap();
        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &project.bundle_id, &binding)
                .unwrap();
        let plan = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();
        assert!(plan.plan_id.as_str().starts_with("plan-"));
        assert_eq!(repo.load_dependency_plan(&plan.plan_id).unwrap(), plan);

        let first = apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new())
            .await
            .unwrap();
        let retry = apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new())
            .await
            .unwrap();
        assert!(first.success);
        assert!(retry.success);
        assert_eq!(
            repo.load_dependency_applications().unwrap().records.len(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dependency_application_fetches_s3_bundle_off_runtime_worker() {
        let temp = tempfile::tempdir().unwrap();
        let cloud_root = temp.path().join("cloud");
        std::fs::create_dir_all(cloud_root.join("bundle-tests")).unwrap();
        let server = StubS3::start(cloud_root).await;
        let repo = repository(&temp);
        let storage_id = StorageId::parse("dependency-s3-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Dependency S3 store".to_string(),
            kind: StorageKind::S3,
            bucket: "bundle-tests".to_string(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret".to_string(),
            account_id: String::new(),
            s3_endpoint: server.endpoint.clone(),
            region: "auto".to_string(),
            local_dir: String::new(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let binding = bind_and_link_test_replica(
            &repo,
            &project,
            &storage_id,
            &temp.path().join("project"),
            &temp.path().join("codex-home"),
        );

        let push_repo = repo.clone();
        let push_project_id = project.local_project_id.clone();
        let push_storage_id = storage_id.clone();
        run_blocking(move || {
            push_bundle_with_repository(&push_repo, &push_project_id, &push_storage_id)
        })
        .await
        .unwrap();

        let restore_repo = repo.clone();
        let restore_storage_id = storage_id.clone();
        let restore_bundle_id = project.bundle_id.clone();
        let restore = run_blocking(move || {
            plan_bundle_restore_with_repository(
                &restore_repo,
                &restore_storage_id,
                &restore_bundle_id,
                &binding,
            )
        })
        .await
        .unwrap();
        let dependency_repo = repo.clone();
        let restore_plan_id = restore.plan_id;
        let plan = run_blocking(move || {
            plan_dependencies_with_repository(&dependency_repo, &restore_plan_id)
        })
        .await
        .unwrap();

        let result =
            apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new()).await;
        assert!(
            result.is_ok(),
            "S3 dependency application failed: {result:?}"
        );
        assert_eq!(
            repo.load_dependency_applications().unwrap().records.len(),
            1
        );
    }

```

## commands.rs test: global_skill_with_distinct_declared_name_round_trips_across_push_and_pull (pre-removal lines 15196–15431)
```rust
    #[test]
    fn global_skill_with_distinct_declared_name_round_trips_across_push_and_pull() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("skill-roundtrip-store").unwrap();
        let bundle_id = BundleId::parse("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Skill round-trip store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        // Machine A: the runtime-visible name deliberately differs from its
        // physical directory, and the payload refers to that physical path.
        let source = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Source project".to_string(),
                repository_fingerprint: Some("e".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let source_root = temp.path().join("source-project");
        let source_codex = temp.path().join("source-codex");
        let source_skill = source_codex.join("skills/capture-lsservice-detail/SKILL.md");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(source_skill.parent().unwrap()).unwrap();
        let skill_contents = b"---\nname: get-real-hardware-rh-service\n---\nRun ~/.codex/skills/capture-lsservice-detail/scripts/run.py\n";
        std::fs::write(&source_skill, skill_contents).unwrap();
        let source_profile = add_profile(&repo, Provider::Codex, &source_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, source_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        // Discovery must expose a selectable effective identity while
        // retaining the physical directory in portable metadata and paths.
        let inventory =
            get_bundle_inventory_with_repository(&repo, &source.local_project_id).unwrap();
        let resource_id =
            ResourceId::parse("codex:standalone-skill:get-real-hardware-rh-service").unwrap();
        let skill = inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.resource_id == resource_id)
            .expect("declared-name skill should be discoverable");
        assert!(skill.blocked_reason.is_none());
        assert_eq!(
            skill.descriptor.display_name,
            "get-real-hardware-rh-service"
        );
        assert_eq!(
            skill
                .descriptor
                .metadata
                .get("install_dir_name")
                .map(String::as_str),
            Some("capture-lsservice-detail")
        );
        assert!(skill
            .logical_paths
            .iter()
            .all(|path| path.starts_with("state/codex/skills/capture-lsservice-detail/")));
        assert!(!inventory.recipe.entries.contains_key(&resource_id));

        let mut recipe = inventory.recipe;
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        save_bundle_recipe_with_repository(&repo, &source.local_project_id, recipe).unwrap();
        let pushed =
            push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();
        assert!(pushed.success, "{}", pushed.message);
        assert!(pushed
            .results
            .iter()
            .any(|result| result.resource_id == resource_id));
        let source_status = get_project_capability_status_with_repository(
            &repo,
            &source.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let source_skill_status = source_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("source status should include the custom skill");
        assert_eq!(source_skill_status.state, "synced");
        assert!(source_skill_status.local_present);
        assert!(source_skill_status.storage_present);
        assert_eq!(source_status.profiles[0].shared_project_count, 1);

        // Machine B: Pull plans by effective identity but pins the original
        // physical directory as the mutation target.
        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Target project".to_string(),
                repository_fingerprint: Some("e".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let target_root = temp.path().join("target-project");
        let target_codex = temp.path().join("target-codex");
        std::fs::create_dir_all(&target_root).unwrap();
        let target_profile = add_profile(&repo, Provider::Codex, &target_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: target.local_project_id.clone(),
                project_root: target_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, target_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: target.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let before_status = get_project_capability_status_with_repository(
            &repo,
            &target.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let missing_skill = before_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("target status should include the stored custom skill");
        assert_eq!(missing_skill.state, "storage_only");
        assert!(!missing_skill.local_present);
        assert!(missing_skill.storage_present);
        let target_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&target.local_project_id)
            .cloned()
            .unwrap();
        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        let install = restore
            .actions
            .iter()
            .find(|action| action.resource_id == resource_id)
            .expect("pull should contain the custom-skill action");
        match &install.kind {
            RestoreActionKind::InstallCustomSkill { skill_name, .. } => {
                assert_eq!(skill_name, "get-real-hardware-rh-service")
            }
            other => panic!("expected custom-skill install, got {other:?}"),
        }
        assert!(install.requires_explicit_approval);
        assert!(install
            .target_path
            .as_deref()
            .unwrap()
            .ends_with("skills/capture-lsservice-detail"));
        let dependencies = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();
        assert!(dependencies.actions.is_empty());

        let applied = apply_bundle_restore_with_repository(
            &repo,
            &restore.plan_id,
            vec![install.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{}", applied.message);
        assert_eq!(
            std::fs::read(target_codex.join("skills/capture-lsservice-detail/SKILL.md")).unwrap(),
            skill_contents
        );
        assert!(!target_codex
            .join("skills/get-real-hardware-rh-service")
            .exists());
        let readiness =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(readiness.state, "ready");
        let after_status = get_project_capability_status_with_repository(
            &repo,
            &target.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let installed_skill = after_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("installed skill should remain visible in status");
        assert_eq!(installed_skill.state, "synced");
        assert!(installed_skill.local_present);
        assert!(installed_skill.storage_present);
    }

```

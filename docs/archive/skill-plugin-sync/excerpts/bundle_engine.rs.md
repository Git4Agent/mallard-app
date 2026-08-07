# Excerpts: src-tauri/src/project_sync/bundle_engine.rs

Custom-skill restore machinery removed from `bundle_engine.rs`. Line numbers are pre-removal.

## Restore-plan build: one typed action per custom-skill resource, duplicate-name/target guards (lines 1586–1627)
```rust
        // Executable dependencies have their own immutable DependencyPlan.
        // Keeping plugin/installer placeholders out of RestorePlan prevents a
        // single logical resource from appearing twice in Pull review and
        // prevents the restore engine from reporting expected native work as
        // a blocked file action.
        // Custom skills are directory units: one typed install/overwrite
        // action per resource, never independently selectable file writes.
        let mut skill_targets = BTreeMap::<String, ResourceId>::new();
        let mut skill_capabilities = BTreeMap::<String, ResourceId>::new();
        for descriptor in snapshot.manifest.resources.values() {
            if descriptor.kind != ResourceKind::StandaloneSkill {
                continue;
            }
            let identity = custom_skill_identity(descriptor)?;
            let capability_key = format!(
                "{:?}:{}",
                identity.provider,
                identity.effective_name.to_ascii_lowercase()
            );
            if let Some(previous) =
                skill_capabilities.insert(capability_key, descriptor.resource_id.clone())
            {
                return Err(format!(
                    "custom skills '{}' and '{}' claim one effective name",
                    previous, descriptor.resource_id
                ));
            }
            let action = custom_skill_action(&snapshot.manifest, descriptor, binding)?;
            if let Some(target) = &action.target_path {
                // Case-folded so two snapshots cannot claim one materialized
                // directory on a case-insensitive filesystem.
                if let Some(previous) =
                    skill_targets.insert(target.to_lowercase(), descriptor.resource_id.clone())
                {
                    return Err(format!(
                        "custom skills '{}' and '{}' claim one target directory",
                        previous, descriptor.resource_id
                    ));
                }
            }
            actions.push(action);
        }
```

## File-pass exclusion of skill payload files (lines 1687–1691)
```rust
        for (logical_path, entry) in &snapshot.manifest.files {
            let descriptor = snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("file '{}' has no resource", logical_path))?;
            if descriptor.kind == ResourceKind::StandaloneSkill
                || typed_conversation_resources.contains(&entry.resource_id)
            {
                continue;
            }
            let Some(target) = map_logical_target(logical_path, descriptor, binding)? else {
```

## Custom-skill identity, tree digests, target-state scan, action builder (lines 3461–3740)
```rust
const MAX_SKILL_TREE_FILES: usize = 4_096;
const MAX_SKILL_TREE_BYTES: u64 = 512 * 1024 * 1024;

/// The effective identity of a custom-skill resource: bound provider plus
/// the runtime-visible skill name recorded at capture. The physical install
/// directory is tracked independently because valid skills may declare a
/// different runtime name and may refer to their installed path internally.
struct CustomSkillIdentity {
    provider: Provider,
    effective_name: String,
    install_dir_name: String,
}

fn custom_skill_identity(descriptor: &ResourceDescriptor) -> Result<CustomSkillIdentity, String> {
    let provider = descriptor
        .provider
        .ok_or_else(|| format!("custom skill '{}' lacks a provider", descriptor.resource_id))?;
    let effective_name = descriptor
        .metadata
        .get("effective_name")
        .cloned()
        .ok_or_else(|| {
            format!(
                "custom skill '{}' lacks its effective name",
                descriptor.resource_id
            )
        })?;
    let install_dir_name = descriptor
        .metadata
        .get("install_dir_name")
        .cloned()
        .ok_or_else(|| {
            format!(
                "custom skill '{}' lacks its install directory",
                descriptor.resource_id
            )
        })?;
    domain::validate_skill_name("custom skill effective name", &effective_name)?;
    domain::validate_skill_name("custom skill install directory", &install_dir_name)?;
    Ok(CustomSkillIdentity {
        provider,
        effective_name,
        install_dir_name,
    })
}

fn custom_skill_logical_root(provider: Provider, install_dir_name: &str) -> String {
    let provider = match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    };
    format!("state/{}/skills/{}", provider, install_dir_name)
}

/// Manifest files belonging to one custom skill, keyed by their path relative
/// to the skill directory.
fn custom_skill_manifest_files<'a>(
    manifest: &'a BundleManifest,
    resource_id: &ResourceId,
    logical_root: &str,
) -> Result<Vec<(String, &'a LogicalPath, &'a BundleFileEntry)>, String> {
    let prefix = format!("{}/", logical_root);
    let mut files = Vec::new();
    for (logical_path, entry) in &manifest.files {
        if &entry.resource_id != resource_id {
            continue;
        }
        let relative = logical_path.as_str().strip_prefix(&prefix).ok_or_else(|| {
            format!(
                "custom skill file '{}' is outside its skill root '{}'",
                logical_path, logical_root
            )
        })?;
        files.push((relative.to_string(), logical_path, entry));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn entry_is_executable(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o111 != 0)
}

/// Canonical tree digest over relative path, file type/executability, and
/// content hash. Computable both from a manifest and from a live directory so
/// "already installed" and "changed after review" are byte-grounded claims.
fn skill_tree_digest(entries: &[(String, bool, String)]) -> String {
    let mut material = String::new();
    for (relative, executable, digest) in entries {
        material.push_str(relative);
        material.push('\0');
        material.push(if *executable { 'x' } else { 'r' });
        material.push('\0');
        material.push_str(digest);
        material.push('\n');
    }
    sha256(material.as_bytes())
}

fn custom_skill_source_digest(files: &[(String, &LogicalPath, &BundleFileEntry)]) -> String {
    let entries = files
        .iter()
        .map(|(relative, _, entry)| {
            (
                relative.clone(),
                entry_is_executable(entry.mode),
                entry.sha256.clone(),
            )
        })
        .collect::<Vec<_>>();
    skill_tree_digest(&entries)
}

enum SkillTargetState {
    Missing,
    Present { digest: String },
    Blocked { reason: String },
}

/// No-follow scan of a live skill directory. Symlinks, special files, and
/// oversized trees block classification instead of being read through.
fn custom_skill_target_state(dir: &Path) -> Result<SkillTargetState, String> {
    inspect_existing_ancestors_no_symlink(dir)?;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SkillTargetState::Missing)
        }
        Err(error) => return Err(format!("inspect '{}': {}", dir.display(), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(SkillTargetState::Blocked {
            reason: "target is not a regular directory".to_string(),
        });
    }
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(dir).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", dir.display(), error))?;
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Ok(SkillTargetState::Blocked {
                reason: format!("'{}' is not a regular file", entry.path().display()),
            });
        }
        let relative = entry
            .path()
            .strip_prefix(dir)
            .map_err(|_| "skill tree escaped its root".to_string())?
            .to_str()
            .ok_or_else(|| "skill tree contains a non-UTF-8 path".to_string())?
            .replace('\\', "/");
        let meta = entry
            .metadata()
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;
        total_bytes = total_bytes.saturating_add(meta.len());
        if entries.len() >= MAX_SKILL_TREE_FILES || total_bytes > MAX_SKILL_TREE_BYTES {
            return Ok(SkillTargetState::Blocked {
                reason: "target skill tree is too large to verify".to_string(),
            });
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read '{}': {}", entry.path().display(), error))?;
        let executable = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        entries.push((relative, executable, sha256(&bytes)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(SkillTargetState::Present {
        digest: skill_tree_digest(&entries),
    })
}

/// Resolve the provider-home target directory for one custom skill.
fn custom_skill_target_dir(
    binding: &ProjectBinding,
    provider: Provider,
    install_dir_name: &str,
) -> Result<PathBuf, String> {
    let home = match provider {
        Provider::Codex => canonical_bound_home(binding.codex_home.as_deref(), "Codex")?,
        Provider::Claude => canonical_bound_home(binding.claude_home.as_deref(), "Claude")?,
    };
    safe_lexical_join(&home, &format!("skills/{}", install_dir_name))
}

/// Build the single typed action for one custom-skill resource: Install for a
/// missing or matching target, Overwrite pinned to the current target digest
/// for a different one, Manual when the target cannot be classified.
fn custom_skill_action(
    manifest: &BundleManifest,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<RestoreAction, String> {
    let identity = custom_skill_identity(descriptor)?;
    let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
    let files = custom_skill_manifest_files(manifest, &descriptor.resource_id, &logical_root)?;
    if files.is_empty() {
        return manual_action(
            &descriptor.resource_id,
            &format!(
                "Custom skill '{}' has no payload in this revision",
                identity.effective_name
            ),
        );
    }
    let target =
        match custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name) {
            Ok(target) => target,
            Err(error) => {
                return manual_action(
                    &descriptor.resource_id,
                    &format!(
                        "Custom skill '{}' needs a mapped provider home: {}",
                        identity.effective_name, error
                    ),
                )
            }
        };
    let source_digest = custom_skill_source_digest(&files);
    let state = custom_skill_target_state(&target)?;
    let (kind, expected) = match state {
        SkillTargetState::Missing => (
            RestoreActionKind::InstallCustomSkill {
                provider: identity.provider,
                skill_name: identity.effective_name.clone(),
            },
            None,
        ),
        SkillTargetState::Present { digest } => {
            let kind = if digest == source_digest {
                RestoreActionKind::InstallCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            } else {
                RestoreActionKind::OverwriteCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            };
            (kind, Some(digest))
        }
        SkillTargetState::Blocked { reason } => {
            return manual_action(
                &descriptor.resource_id,
                &format!(
                    "Custom skill target '{}' cannot be replaced automatically: {}",
                    target.display(),
                    reason
                ),
            )
        }
    };
    let action = RestoreAction {
        action_id: action_id_for(&descriptor.resource_id, &logical_root, kind.action_type())?,
        resource_id: descriptor.resource_id.clone(),
        kind,
        target_path: Some(path_text(&target)?),
        source_sha256: Some(source_digest),
        expected_target_sha256: expected,
        expected_target_mode: None,
        materialized_sha256: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}
```

## action_needs_explicit_approval with ExplicitInstall + skill/plugin kinds (lines 3858–3875)
```rust
fn action_needs_explicit_approval(
    descriptor: &ResourceDescriptor,
    target_digest: Option<&str>,
    source_digest: &str,
) -> bool {
    matches!(
        descriptor.apply_policy,
        ApplyPolicy::ExplicitInstall
            | ApplyPolicy::ExplicitReview
            | ApplyPolicy::ManualOnly
            | ApplyPolicy::Merge
    ) || target_digest.is_some_and(|target| target != source_digest)
        || matches!(
            descriptor.kind,
            ResourceKind::StandaloneSkill | ResourceKind::Plugin
        )
}

```

## Apply dispatcher arm for InstallCustomSkill/OverwriteCustomSkill (lines 5157–5187)
```rust
                }
                RestoreActionKind::InstallCustomSkill {
                    provider,
                    skill_name,
                }
                | RestoreActionKind::OverwriteCustomSkill {
                    provider,
                    skill_name,
                } => {
                    let receipt = self.apply_custom_skill_action(
                        snapshot,
                        bytes,
                        binding,
                        action,
                        *provider,
                        skill_name,
                        &plan_backup,
                        applied_at,
                        &mut backups,
                    );
                    receipts.push(match receipt {
                        Ok(receipt) => receipt,
                        Err(error) => receipt_for(
                            action,
                            ActionStatus::Failed,
                            applied_at,
                            None,
                            None,
                            Some(error),
                        ),
                    });
```

## apply_custom_skill_action: staged whole-directory swap under the provider-home lock (lines 5620–5821)
```rust
    /// Materialize one custom skill as a whole-directory transaction: stage
    /// the complete verified tree next to the target, back up and journal the
    /// existing directory, swap by rename, and roll back on failure. Runs
    /// under the canonical provider-home operation lock.
    #[allow(clippy::too_many_arguments)]
    fn apply_custom_skill_action(
        &self,
        snapshot: &BundleSnapshot,
        bytes: &mut dyn BundleBytes,
        binding: &ProjectBinding,
        action: &RestoreAction,
        provider: Provider,
        skill_name: &str,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        domain::validate_skill_name("custom skill name", skill_name)?;
        let descriptor = snapshot
            .manifest
            .resources
            .get(&action.resource_id)
            .ok_or_else(|| "custom-skill resource is missing from the manifest".to_string())?;
        if descriptor.kind != ResourceKind::StandaloneSkill {
            return Err("action resource is not a custom skill".to_string());
        }
        let identity = custom_skill_identity(descriptor)?;
        if identity.provider != provider || identity.effective_name != skill_name {
            return Err("custom-skill action identity differs from its descriptor".to_string());
        }
        let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
        let files =
            custom_skill_manifest_files(&snapshot.manifest, &action.resource_id, &logical_root)?;
        if files.is_empty() {
            return Err("custom skill has no payload files".to_string());
        }
        let source_digest = custom_skill_source_digest(&files);
        if action.source_sha256.as_deref() != Some(source_digest.as_str()) {
            return Err("custom-skill payload changed after planning".to_string());
        }
        let target =
            custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("custom-skill target differs from the pinned plan".to_string());
        }
        let skills_dir = target
            .parent()
            .ok_or_else(|| "custom-skill target has no parent".to_string())?
            .to_path_buf();
        let home = skills_dir
            .parent()
            .ok_or_else(|| "skills directory has no parent".to_string())?;
        inspect_existing_ancestors_no_symlink(&skills_dir)?;
        if !skills_dir.exists() {
            fs::create_dir_all(&skills_dir)
                .map_err(|error| format!("create '{}': {}", skills_dir.display(), error))?;
        }
        let canonical_skills = canonical_real_dir(&skills_dir, "global skills directory")?;
        let canonical_home = canonical_real_dir(home, "provider home")?;
        if !canonical_skills.starts_with(&canonical_home) {
            return Err("global skills directory escapes the provider home".to_string());
        }

        // Canonically equivalent provider homes resolve to the same lock file,
        // so concurrent operations on one home serialize regardless of path
        // spelling. `.`-prefixed names stay invisible to inventory and sync.
        let lock_path = canonical_skills.join(".agent-sync.lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open provider-home lock: {}", error))?;
        lock_file
            .lock()
            .map_err(|error| format!("acquire provider-home lock: {}", error))?;

        // Revalidate the live target under the lock; approval is pinned to
        // the reviewed digest and a changed target aborts without mutation.
        let state = custom_skill_target_state(&target)?;
        let current_digest = match (&state, &action.expected_target_sha256) {
            (SkillTargetState::Missing, None) => None,
            (SkillTargetState::Present { digest }, _) if digest == &source_digest => {
                // Already-matching content: record the installed claim
                // without rewriting anything (install no-op / adopt).
                return Ok(receipt_for(
                    action,
                    ActionStatus::Applied,
                    applied_at,
                    None,
                    Some(source_digest),
                    None,
                ));
            }
            (SkillTargetState::Present { digest }, Some(expected)) if digest == expected => {
                Some(digest.clone())
            }
            (SkillTargetState::Blocked { reason }, _) => {
                return Err(format!("custom-skill target is blocked: {}", reason))
            }
            _ => return Err("custom-skill target changed after planning".to_string()),
        };

        // Stage the complete tree on the same filesystem as the target so the
        // final activation is a rename, then verify every staged byte.
        let staging = canonical_skills.join(format!(".agent-sync-staging-{}", action.action_id));
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .map_err(|error| format!("clear stale staging: {}", error))?;
        }
        let staging_result = (|| -> Result<(), String> {
            for (relative, logical_path, entry) in &files {
                let payload = bytes.get(logical_path)?;
                let staged = safe_lexical_join(&staging, relative)?;
                if let Some(parent) = staged.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
                }
                // Set-id and non-permission bits were stripped at capture;
                // strip again on the write path for defense in depth.
                write_target_atomic(&staged, payload, entry.mode.unwrap_or(0o600) & 0o777)?;
            }
            match custom_skill_target_state(&staging)? {
                SkillTargetState::Present { digest } if digest == source_digest => Ok(()),
                _ => Err("staged custom skill failed tree verification".to_string()),
            }
        })();
        if let Err(error) = staging_result {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        // Journal before mutating the target so an interrupted swap is
        // detectable and recoverable from the retained plan backup.
        let backup_dir = plan_backup.join(format!("{}.skill-backup", action.action_id));
        let journal_path = plan_backup.join(format!("{}.journal.json", action.action_id));
        let write_journal = |phase: &str| -> Result<(), String> {
            let journal = serde_json::json!({
                "action_id": action.action_id.as_str(),
                "resource_id": action.resource_id.as_str(),
                "target": path_text(&target)?,
                "staging": path_text(&staging)?,
                "backup": path_text(&backup_dir)?,
                "source_tree_sha256": source_digest,
                "phase": phase,
            });
            fs::write(&journal_path, journal.to_string())
                .map_err(|error| format!("write custom-skill journal: {}", error))
        };
        write_journal("staged")?;

        let displaced = canonical_skills.join(format!(".agent-sync-old-{}", action.action_id));
        if let Some(current_digest) = &current_digest {
            if let Err(error) = copy_skill_tree(&target, &backup_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("custom-skill backup failed: {}", error));
            }
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_dir)?,
                sha256: current_digest.clone(),
            });
            write_journal("backed-up")?;
            if let Err(error) = fs::rename(&target, &displaced) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("displace existing skill: {}", error));
            }
        }
        if let Err(error) = fs::rename(&staging, &target) {
            // Restore the displaced directory before reporting failure; the
            // journal and backup stay behind for manual recovery if this
            // rollback rename also fails.
            if current_digest.is_some() {
                let _ = fs::rename(&displaced, &target);
            }
            let _ = fs::remove_dir_all(&staging);
            write_journal("rolled-back")?;
            return Err(format!("activate staged skill: {}", error));
        }
        if current_digest.is_some() {
            let _ = fs::remove_dir_all(&displaced);
        }
        let verified = match custom_skill_target_state(&target)? {
            SkillTargetState::Present { digest } if digest == source_digest => digest,
            _ => {
                write_journal("verify-failed")?;
                return Err(
                    "installed custom skill failed post-activation verification".to_string()
                );
            }
        };
        write_journal("complete")?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            None,
            Some(verified),
            None,
        ))
    }
```

## copy_skill_tree backup helper (lines 5824–5859)
```rust
/// Bounded no-follow recursive copy used for pre-overwrite backups. The
/// source tree was already verified to contain only regular files.
fn copy_skill_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create '{}': {}", destination.display(), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(destination, fs::Permissions::from_mode(0o700));
    }
    for entry in WalkDir::new(source).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", source.display(), error))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|_| "backup tree escaped its root".to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        let file_type = entry.file_type();
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("create '{}': {}", target.display(), error))?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("copy '{}': {}", entry.path().display(), error))?;
        } else {
            return Err(format!(
                "backup source '{}' is not a regular file",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
```

## bundle_engine.rs: dependencies_from_manifest (pre-removal lines 4450–~4495)
```rust
pub(crate) fn dependencies_from_manifest(
    manifest: &BundleManifest,
) -> Result<Vec<DependencyAction>, String> {
    let mut actions = Vec::new();
    for descriptor in manifest.resources.values() {
        let Some(kind) = descriptor.metadata.get("dependency_kind") else {
            continue;
        };
        let kind = match kind.as_str() {
            "codex_plugin" => DependencyActionKind::InstallCodexPlugin,
            "claude_plugin" => DependencyActionKind::InstallClaudePlugin,
            "standalone_skill" => DependencyActionKind::InstallStandaloneSkill,
            other => return Err(format!("unknown dependency kind '{}'", other)),
        };
        let argv = descriptor
            .metadata
            .get("dependency_argv_json")
            .map(|json| serde_json::from_str::<Vec<String>>(json))
            .transpose()
            .map_err(|e| format!("parse dependency argv: {}", e))?
            .unwrap_or_default();
        if argv.len() > 64
            || argv.iter().any(|argument| {
                argument.len() > 4096 || argument.chars().any(|character| character == '\0')
            })
        {
            return Err(format!(
                "dependency '{}' has unsafe structured arguments",
                descriptor.resource_id
            ));
        }
        actions.push(DependencyAction {
            action_id: ActionId::parse(format!("dependency:{}", descriptor.resource_id))?,
            resource_id: descriptor.resource_id.clone(),
            kind,
            display_name: descriptor.display_name.clone(),
            provider: descriptor.provider,
            argv,
            requires_explicit_approval: true,
        });
    }
    actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));
    Ok(actions)
}

fn validate_conversation_retention(
```

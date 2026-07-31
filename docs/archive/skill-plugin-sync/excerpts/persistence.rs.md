# Excerpts: src-tauri/src/project_sync/persistence.rs

Dependency plan/applications persistence removed. Line numbers are pre-removal.

## Size limits (lines 29–30), quarantine flag (line 59)
```rust
const MAX_DEPENDENCY_PLAN_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPENDENCY_APPLICATIONS_BYTES: u64 = 64 * 1024 * 1024;
    conversation_bindings: bool,
    #[serde(default)]
    materializations: bool,
    #[serde(default)]
    dependency_applications: bool,
}

```

## Paths (lines 207–209, 217–221)
```rust
    pub fn dependency_applications_path(&self) -> PathBuf {
        self.root.join("dependency_applications.json")
    }

    pub fn restore_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("restore_plans")
            .join(format!("{}.json", plan_id.as_str()))
    }

    pub fn dependency_plan_path(&self, plan_id: &PlanId) -> PathBuf {
        self.root
            .join("dependency_plans")
            .join(format!("{}.json", plan_id.as_str()))
    }
```

## Host-rotation validation of dependency receipts (lines 356–362)
```rust
        // Validate dependency receipts before changing the current host so a
        // damaged document is quarantined instead of silently carried over.
        match self.load_dependency_applications_unlocked(&config) {
            Ok(_) => {}
            Err(_) if allow_damaged_recovery => quarantine.dependency_applications = true,
            Err(error) => return Err(error),
        }
```

## load/mutate_dependency_applications (lines 708–734)
```rust
    pub fn load_dependency_applications(&self) -> Result<DependencyApplications, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        self.load_dependency_applications_unlocked(&config)
    }

    pub fn mutate_dependency_applications<T>(
        &self,
        mutate: impl FnOnce(&SyncConfig, &mut DependencyApplications) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = persistence_guard(&self.root)?;
        self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let mut applications = self.load_dependency_applications_unlocked(&config)?;
        let result = mutate(&config, &mut applications)?;
        applications.schema = super::domain::LOCAL_SCHEMA;
        applications.revision = applications.revision.saturating_add(1);
        applications.validate(&config)?;
        write_json_atomic(
            &self.root,
            &self.dependency_applications_path(),
            &applications,
            MAX_DEPENDENCY_APPLICATIONS_BYTES,
        )?;
        Ok(result)
    }
```

## save/load_dependency_plan (lines 1087–1105)
```rust
    pub fn save_dependency_plan(&self, plan: &DependencyPlan) -> Result<(), String> {
        let _guard = persistence_guard(&self.root)?;
        plan.validate()?;
        let path = self.dependency_plan_path(&plan.plan_id);
        self.ensure_directory(path.parent().ok_or("dependency plan has no parent")?)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!("dependency plan '{}' already exists", plan.plan_id));
        }
        write_json_atomic(&self.root, &path, plan, MAX_DEPENDENCY_PLAN_BYTES)
    }

    pub fn load_dependency_plan(&self, plan_id: &PlanId) -> Result<DependencyPlan, String> {
        let _guard = persistence_guard(&self.root)?;
        let path = self.dependency_plan_path(plan_id);
        let plan: DependencyPlan = read_json_bounded(&self.root, &path, MAX_DEPENDENCY_PLAN_BYTES)?
            .ok_or_else(|| format!("dependency plan '{}' does not exist", plan_id))?;
        plan.validate()?;
        Ok(plan)
    }
```

## Host-scoped state list entry (line 1142), rotation quarantine apply (lines 1661–1667)
```rust
        for path in [
            self.machine_projects_path(),
            self.conversation_bindings_path(),
            self.materializations_path(),
            self.dependency_applications_path(),
        ] {
        if transaction.quarantine.dependency_applications {
            self.quarantine_host_rotation_file_unlocked(
                transaction,
                &self.dependency_applications_path(),
                "dependency-applications",
            )?;
        }
```

## load_dependency_applications_unlocked (lines 1772–1786)
```rust
    fn load_dependency_applications_unlocked(
        &self,
        config: &SyncConfig,
    ) -> Result<DependencyApplications, String> {
        let path = self.dependency_applications_path();
        let Some(value) =
            read_json_bounded::<Value>(&self.root, &path, MAX_DEPENDENCY_APPLICATIONS_BYTES)?
        else {
            return Ok(DependencyApplications::default());
        };
        let applications: DependencyApplications = serde_json::from_value(value)
            .map_err(|error| format!("parse '{}': {}", path.display(), error))?;
        applications.validate(config)?;
        Ok(applications)
    }
```

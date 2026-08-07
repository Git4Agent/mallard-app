# Plan: Back up and install global plugins and custom skills

> **Historical and superseded.** The repository-scoped implementation
> described here was removed on 2026-07-30. It must not be reintroduced.
> Its storage-scoped successor is
> [`DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md`](./DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md).
> The removed source snapshot remains under
> [`archive/skill-plugin-sync/`](./archive/skill-plugin-sync/).

**Status:** historical; formerly implemented, then removed
**Date:** 2026-07-18 (implemented 2026-07-18)
**Scope:** schema-3 project sync; legacy profile sync stops syncing global
`skills/**` (schema-3 is the sole owner of global custom skills)

## Implementation status (2026-07-18)

Landed:

- `project_sync_v3/global_inventory.rs` — ownership-ordered provider-home
  inventory: plugin inventory (Claude via `codex_plugins::claude_inventory`,
  Codex via the global `config.toml` plugins/marketplaces tables), plugin
  install-root ownership index, `skills/*` enumeration and classification
  (standalone / plugin-provided / blocked), bounded `SKILL.md` name
  validation. Adapter contract v2 separates the declared runtime-visible
  effective name from the physical install-directory name; either may differ,
  while duplicate effective-name claims block selection.
- Capture: `CaptureResourceKind::StandaloneSkill` maps to
  `ResourceKind::StandaloneSkill` + provider-state scope +
  `Provenance::StandaloneSnapshot`; resource identity is provider +
  effective name (`codex:standalone-skill:<name>`), so edits keep identity
  and declared-name changes are remove/add. Logical roots preserve the source
  install directory as `state/<provider>/skills/<install-dir>`.
  Global plugins become payload-free intent resources that coalesce with
  project-config observations; `ResourceCandidate.metadata` carries plugin
  source/version/provided-skills and skill naming evidence into descriptors.
- Restore: typed `InstallCustomSkill`/`OverwriteCustomSkill` actions (one per
  skill directory, never per-file writes), digest-pinned to the live target
  tree at plan time. Apply is transactional under a canonical provider-home
  lock (flock on `skills/.agent-sync.lock`): same-filesystem staging, full
  staged-tree verification, journal + recoverable backup in the plan backup
  area, rename swap with rollback on failure, post-activation verification.
  Whole-directory replacement removes files deleted by the cloud version.
- Legacy allowlist: `skills/**` removed from both providers' Optional tier
  (`lib.rs`, AGENT_SYNC_FILE_SETS.md) so the two engines never co-manage one
  skill directory.
- Claude global plugin installs use scope-free argv; project-declared plugins
  keep `--scope project` (both validated against a closed set).

Deviations / not yet implemented:

- **Install as renamed copy** (§6.4) is not offered; duplicates resolve to
  Keep (default, unapproved) or Overwrite only.
- **Version-history selection on pull** (§8.3) is not built; pull installs
  the head revision. History remains derivable from immutable manifests.
- The **machine-local plugin requirement graph** (§6.6) and cross-bundle
  source-conflict blocking are not persisted yet; same-plugin coalescing is
  per-bundle.
- **Journal crash-recovery on restart** (§11) is passive: the journal and
  backup are retained for manual recovery, but no startup scanner replays
  them.
- Post-install **plugin re-inventory/verification** (§6.5) still matches the
  pre-existing dependency runner (exit-status receipts); no dry-run/version
  pinning.
- `CapabilityClaim`/overlap preflight exists only as capture-time target
  collision detection (two snapshots claiming one directory fail the plan);
  plugin-vs-custom effective-name overlap is surfaced via plugin
  `provided_skills` metadata, not a blocking claim graph.

## 1. Outcome

Let a project bundle include individually selected resources discovered from its
mapped Codex and Claude profiles:

- installed global plugins, represented as portable install intent;
- global custom skills, represented as reviewed, versioned file snapshots; and
- the provenance needed to distinguish standalone skills from skills supplied
  by plugins.

Push must never copy plugin payloads, caches, repositories, marketplace clones,
manager databases, credentials, or machine-local paths. Pull must reinstall a
plugin through its provider's native CLI and install a selected custom skill
into the mapped target provider home.

In this plan, **custom skill** means a global standalone skill under a mapped
provider home's `skills/<install-dir>` directory that is not proven to be owned by a
plugin. **Plugin skill** means a skill exported and lifecycle-managed by a
plugin.

Project-authored skills already under `.agents/skills` or `.claude/skills` are
not changed by this work.

## 2. Product decisions

1. **Backup and install are independently selected.** Push includes only the
   plugins and custom skills checked by the user in the saved backup recipe.
   Pull presents the bundle's backed-up plugins and skills unchecked and
   installs only the actions approved for that target machine.
2. **Global custom skills restore globally.** A selected custom skill is
   installed under its preserved `skills/<install-dir>` in the mapped `CODEX_HOME` or
   `CLAUDE_CONFIG_DIR`. The UI must disclose that every project sharing that
   provider home can observe the change.
3. **Plugins remain installer-owned.** Agent Sync stores plugin identity,
   marketplace provenance, and supported install arguments, but no plugin
   files.
4. **Plugin ownership wins over skill snapshotting.** A skill proven to be
   supplied by a plugin is represented by the plugin dependency only. It is
   never copied again as a standalone skill.
5. **Custom skill updates create cloud history.** Content changes keep the same
   resource identity and publish a new immutable bundle revision. Version
   history reuses today's manifests, file entries, digests, commits, and object
   storage; it is not a second versioning system.
6. **Existing custom skills are never silently replaced.** Missing targets can
   be installed; matching targets are no-ops; different targets require an
   explicit Keep, Overwrite, or Install as copy decision.
7. **Plugin skills are replaced only by their owning native installer.** Agent
   Sync may approve a same-plugin update/reinstall, but it never writes directly
   into plugin payload directories.
8. **Every executable or overwrite action requires local approval.** A remote
   recipe is not permission to execute a plugin installer or activate skill
   scripts.
9. **Removal is non-destructive.** Removing a plugin or skill from a recipe
   removes the bundle requirement. It does not uninstall a plugin or delete a
   previously restored skill automatically.

## 3. Current gap and reusable implementation

Schema 3 already has most of the transport and restore model:

- `StandaloneSkillSource` and bounded standalone-skill capture;
- dependency actions for Codex plugins, Claude plugins, and executable skills;
- revision-pinned restore and dependency plans;
- explicit approval, receipts, backups, and readiness reporting; and
- immutable revision manifests, commit links, file entries, content digests,
  object storage, and materialization receipts.

The command layer currently passes `standalone_skills: Vec::new()`, and plugin
discovery reads only project config. The legacy implementation already has the
stronger provider adapters that should be reused:

- Codex CLI inventory, marketplace provenance, normalization, planning, apply,
  and verification;
- Claude plugin intent capture from settings and manager records; and
- native repair with target-profile environment binding.

The existing standalone restore target is project-local, so that part must be
changed to the mapped provider home's global `skills/<install-dir>` directory and put
behind the provider-home mutation lock. Do not port the legacy whole-profile
file selection or plugin-cache opt-in. Port only its Tauri-free inventory,
provenance, planning, and verification logic.

## 4. Resource and ownership model

### 4.1 Standalone skill

A selected global standalone skill becomes:

```text
kind        = standalone_skill
scope       = provider_state
provider    = codex | claude
provenance  = standalone_snapshot(stable_key, source_digest)
target      = <mapped provider home>/skills/<install-dir>
identity    = (provider, declared effective name)
```

Within a project bundle, derive a stable resource ID from provider plus the
validated effective skill name. It is never the absolute source path or content
digest. Editing files therefore preserves identity; renaming the declared skill
is an explicit remove/add operation. Verified Git provenance remains metadata,
not the identity required for a normal custom skill.

The existing `ApplyReceipt` already records resource ID, target path, source
digest, and resulting target digest; its containing `MaterializationRecord` is
pinned to the bundle revision and manifest. Reuse them together as the
installed-version claim. Do not add a second custom registry unless an
implementation gap is proven.

### 4.2 Plugin

A selected plugin becomes one resource keyed by provider and normalized plugin
ID. Its version state contains:

- marketplace or managed-source identifier;
- credential-free repository fingerprint where applicable;
- immutable Git ref when available;
- observed provider/plugin version for diagnostics; and
- declared effective capabilities, including exported skill names when the
  provider exposes them.

The plugin resource contains no file payload. Marketplace source/ref and plugin
ID are validated again on pull before structured arguments are passed directly
to the native process API.

### 4.3 Plugin-provided skill

A plugin-provided skill is a derived capability, not a separately selectable
payload resource:

```text
provider + effective skill name -> provided_by(plugin resource ID)
```

It can be displayed beneath the plugin in inventory, but selecting the plugin
is the only way to transport it. This guarantees that plugin updates continue
to own their skill files and avoids freezing a second, detached copy in the
bundle.

### 4.4 Custom skill versions using today's cloud metadata

Do not create a parallel `SkillVersion` database or a second mutable head. The
existing schema-3 bundle history is the source of truth:

- `ResourceDescriptor.resource_id` is the stable custom-skill identity;
- `ResourceDescriptor.metadata["content_sha256"]` is today's resource digest
  over logical paths, bytes, safe modes, and dependency intent;
- `BundleManifest.revision` identifies the immutable bundle snapshot;
- `BundleFileEntry` stores each file's hash, size, mode, and immutable object
  key;
- `CommitRecord.previous_commit_id` links bundle history and its file deltas;
- the bundle head CAS prevents concurrent pushes from losing an update; and
- `MaterializationRecord` plus `ApplyReceipt` record which revision/digest was
  installed locally.

The user-visible custom-skill version is therefore:

```text
bundle revision + custom-skill content_sha256
```

The history view walks immutable commits/manifests and keeps only points where
that resource's content digest changed. An unchanged skill does not acquire a
fake new version merely because conversations caused another bundle revision.
Previous file objects remain reachable through their immutable manifests.

Publishing an updated skill follows the existing optimistic concurrency model:

1. Compare the local skill digest with the recipe base and current remote head.
2. If unchanged, reuse the prior resource/files in the new manifest.
3. If locally changed and the remote still matches the reviewed base, publish a
   new bundle revision through head CAS.
4. If both local and remote changed, block push and require Pull review; do not
   choose a winner from timestamps.
5. If capture observes files changing while they are read, retry a bounded
   number of times and then block the resource as unstable.

Restoring an old version never rewinds the bundle head. It selects files from an
older immutable manifest and, if the user later pushes them, creates a new head
revision whose digest matches that restored content. Cloud garbage
collection must retain every manifest, commit, and object still covered by the
configured history-retention policy.

### 4.5 Plugin versions

Plugin intent records the existing observed version, marketplace/source
fingerprint, and immutable ref when available. That metadata is diagnostic
unless the provider's native installer supports an exact version/ref. Pull must
state whether it will install the recorded version or the provider's current
marketplace version. Agent Sync must never claim byte-exact plugin restoration
when the provider cannot guarantee it.

## 5. Inventory pipeline

Inventory must run in ownership order, not by independently scanning folders.

1. Resolve and canonicalize the bound Codex and Claude profile homes.
2. Inventory plugins using provider-native machine-readable data and the
   reusable legacy adapters.
3. Normalize plugin IDs and marketplace provenance, rejecting credentials,
   unsafe local paths, and unsupported sources.
4. Build a plugin ownership index from authoritative provider inventory,
   bounded plugin manifests, and canonical install roots.
5. Enumerate immediate children of each provider's global `skills` directory.
6. Classify every skill candidate against the ownership index.
7. Return standalone skills, plugin resources, derived plugin capabilities,
   blocked candidates, and warnings to the schema-3 resource inventory.

Plugin directories may be read in a bounded, no-follow manner for ownership
classification, but their content never becomes a captured file.

## 6. Resolving skill/plugin overlap

### 6.1 Ownership evidence

Use this precedence, strongest first:

1. The provider's native inventory explicitly attributes the skill to a plugin.
2. A validated plugin manifest declares the skill and maps it to that plugin.
3. The candidate is a symlink whose canonical target is inside the exact
   install root of an inventoried plugin.
4. The candidate's canonical directory is itself inside an inventoried plugin
   install root.
5. No authoritative claim exists: treat a regular, safe directory as a
   standalone skill.

Name equality and content-digest equality are **not ownership evidence**. Two
authors can independently create the same name or bytes. Those signals may
produce a warning, but they must not silently reclassify or discard a skill.

### 6.2 Classification table

| Candidate | Classification | Selectable behavior |
|---|---|---|
| Regular directory, no plugin claim | Standalone skill | Select skill snapshot |
| Symlink into known plugin root | Plugin-provided skill | Select owning plugin only |
| Directory inside known plugin root | Plugin-provided skill | Select owning plugin only |
| Manifest/native inventory claim | Plugin-provided skill | Select owning plugin only |
| Symlink to unknown or external root | Blocked/manual | Never dereference or snapshot |
| Regular directory with only a name/hash match | Ambiguous independent skill | Show warning; apply collision rules |
| Plugin cache/repository/marketplace subtree | Plugin payload | Never selectable as a skill |

### 6.3 Effective capability key

Do not build the key as `(provider, lowercase(folder_name))`. Skill naming and
plugin namespacing are provider behavior, so a versioned provider adapter must
resolve the runtime-visible identifier.

```rust
struct SkillCapabilityKey {
    provider: Provider,
    effective_name: String,
}
```

The `provider` comes from the bound profile adapter that discovered the
resource. It is never inferred from an arbitrary path or remote display name.

#### 6.3.1 Standalone skill name

For each standalone skill candidate:

1. Read its bounded `SKILL.md` without following symlinks.
2. Parse the provider-supported metadata and declared skill name.
3. Validate the declared name using the adapter for the installed provider
   version.
4. Preserve the independently validated directory basename as the physical
   install-directory name.
5. Ask the adapter for the runtime-visible effective name. Adapter v2 uses a
   valid declared name and falls back to the directory basename only for a
   legacy skill without a declaration.

An invalid declaration blocks automatic classification. A declared name may
legitimately differ from the install directory. Two directories claiming the
same canonical effective name are both blocked until the local ambiguity is
resolved.

For example, a validated standalone skill declared as `security-review` could
produce:

```text
(codex, "security-review")
```

#### 6.3.2 Plugin-provided skill name

For each inventoried plugin:

1. Normalize the plugin ID using native inventory.
2. Enumerate exported skills from native inventory or a bounded, validated
   plugin manifest.
3. Parse and validate each exported skill's declared name.
4. Let the provider adapter apply the tested namespace rule for that provider
   version.

If the provider exposes a plugin skill as `acme-tools:security-review`, that
exact runtime identifier becomes the effective name:

```text
(claude, "acme-tools:security-review")
```

If a provider exposes the same plugin skill without a namespace, the key may
instead be:

```text
(codex, "security-review")
```

The latter collides with the standalone example. The examples are
illustrative; native inventory is authoritative when it exposes the effective
identifier. Manifest-based composition is allowed only for provider versions
covered by adapter fixtures. If neither source can prove the effective name,
mark the capability `unverified` and do not silently deduplicate it.

#### 6.3.3 Canonicalization contract

Each provider adapter owns canonicalization. A generic cross-provider
normalizer must not rewrite skill identities.

- Reject leading/trailing whitespace, control characters, path separators,
  traversal components, and identifiers outside the provider's supported
  grammar.
- Preserve namespace separators that are meaningful to the provider.
- Apply case folding only when the provider treats names case-insensitively.
- Do not translate `_` to `-`, remove punctuation, or otherwise merge names the
  provider could treat as distinct.
- Keep the raw display name separate from the canonical collision key.
- Record the adapter/version that produced the key.

Store enough evidence in the descriptor metadata to audit and revalidate the
decision:

```text
declared_name
effective_name
provider_adapter_version
owner_resource_id
ownership_evidence
```

Pull recomputes the key using the target adapter and validates it against the
bundle. A changed or unsupported provider naming rule becomes a review blocker,
not an automatic reinterpretation of remote metadata.

#### 6.3.4 Runtime and target-path indexes

Before saving a recipe and again before applying a pull, build a runtime
capability map:

```text
(provider, canonical effective skill name) -> selected owner resource IDs
```

Also build a separate materialization-path map:

```text
(provider, case-safe target relative path) -> selected standalone resource IDs
```

The first map prevents ambiguous runtime activation. The second prevents two
snapshots from writing the same provider-home path even if their runtime
metadata is different. Filesystem case folding follows the target platform's collision
rules and does not change the runtime capability key.

Insert one `CapabilityClaim` per selected owner. Then apply these rules:

- duplicate observations of the same plugin ID coalesce into one resource;
- a plugin found in both global inventory and project configuration is one
  plugin with multiple discovery origins, not two install actions;
- a proven plugin-provided skill never creates a standalone claim;
- selecting an independent custom skill and a plugin that exports the same
  effective skill name is allowed for backup but recorded as an install
  overlap; Pull cannot activate both without a resolution;
- selecting two standalone skills that target the same provider/name is blocked;
- selecting two plugins with overlapping effective names is allowed for backup;
  installing both is blocked unless the provider adapter proves that its runtime
  namespaces those capabilities; and
- the same name across Codex and Claude is not a collision because the target
  providers and directories differ.

Claims with the same key and the same owner resource ID merge. Claims with the
same key but different owner resource IDs create an overlap record. An
unverified capability is shown as such and is never merged into a verified owner
merely because its display name or digest matches.

An overlap does not prevent backing up two independent resources. It is an
install conflict, not last-writer-wins behavior: Pull must identify both owners
and require deselection, namespaced coexistence, or a separate approved removal
of the currently active custom skill.

### 6.4 Duplicate custom skill and overwrite policy

Pull scans the exact target `skills/<install-dir>` with no-follow metadata and computes
its canonical tree digest before offering an action. Classify it as one of:

| Target state | Pull action |
|---|---|
| Missing | Offer **Install** |
| Same resource and same digest | **Already installed**, no write |
| Same digest but no prior receipt/known identity | Offer **Adopt existing**, no write |
| Prior receipt matches target; cloud digest is newer | Offer **Update**, unchecked until approved |
| Same effective name/path but different digest or unknown owner | **Duplicate custom skill**; ask Keep or Overwrite |
| Target changed after review | Abort that action and require a new plan |
| Unknown/external symlink, special file, unsafe tree | Block overwrite |
| Proven plugin-owned target | Route to the owning plugin action; never treat it as custom payload |

For every duplicate, show provider, target path, installed digest/version,
cloud digest/version, changed file summary, executable files, and ownership
evidence. The choices are:

- **Keep installed** — default; skip the cloud skill and record no installed
  claim for it.
- **Overwrite with cloud version** — explicit destructive approval; first make
  a local recoverable backup and then replace the entire directory.
- **Adopt matching content** — when the complete target tree already matches,
  write only an installation receipt; do not rewrite files or infer that
  matching content proves common authorship.
- **Install as renamed copy** — offer only when the provider adapter supports a
  valid renamed declaration. Preview the projected `SKILL.md` name change and
  resulting digest; otherwise do not show this option.

Custom skills are directory units, not a sequence of unrelated file writes.
There is no automatic per-file merge for opaque/executable skill trees.
Implement a typed `InstallCustomSkill`/`OverwriteCustomSkill` transaction:

1. Acquire the canonical provider-home operation lock.
2. Revalidate the pinned bundle revision, binding, source objects, target
   type, target digest, and free-space requirement.
3. Materialize the complete cloud tree into a same-filesystem staging directory.
4. Verify every staged path, byte hash, mode, total size, and final tree digest;
   strip set-id bits.
5. Write a local journal and copy/move the existing target into an app-owned,
   mode-`0700` backup area without dereferencing links.
6. Rename the staged directory into place. Whole-directory replacement removes
   files deleted by the newer cloud version instead of leaving stale code.
7. If the final rename or verification fails, restore the backup and retain the
   journal for recovery.
8. Record the existing `ApplyReceipt` fields with before/after digest, target
   path, resource ID, and revision.

If backup creation fails, disk space is insufficient, the target is in use in a
way that prevents safe replacement, or the target changes after review, perform
no overwrite. Backups stay local, are never pushed, and follow a bounded
retention policy with a visible Restore backup action.

### 6.5 Safely replacing plugin-owned skills

Plugin-owned skills follow a different rule: the owning provider installer may
replace them as part of an explicitly selected plugin install/update, but Agent
Sync never copies or edits those files itself.

Before installation:

1. Re-inventory the target plugin ID, marketplace/source/ref, installed version,
   install root, and exported skill ownership.
2. Confirm every replaceable plugin-skill path is still inside that exact
   provider-managed install root or is a validated manager-owned symlink to it.
3. Compare current provenance with the bundle intent.

Apply rules:

- Same plugin ID and same source, older/different version: show **Update plugin**;
  the one plugin approval also approves replacement of that plugin's owned
  skills.
- Same plugin ID, same version/source: no-op and verify.
- Same-owner payload with detected local modifications: show an elevated drift
  warning because the native update may replace those edits. When safely
  bounded, preserve the modified plugin-skill subtree in the local backup area;
  never upload it as custom-skill content.
- Same plugin ID but different source/ref: require a separate **Replace plugin
  source** warning; never treat it as an ordinary update.
- Skill owned by a different plugin: block as an ownership/source conflict.
- Independent custom skill with the same effective capability: ask the user to
  keep the custom skill and skip the plugin, or separately approve backing up
  and removing/replacing the custom skill. Plugin approval alone is not consent
  to delete custom content.
- Proven provider namespacing that makes both effective names distinct: allow
  both and show the resolved names.

Run only the provider's native structured install/update command. Capture the
current portable plugin intent before mutation, then re-inventory and verify the
plugin ID, source, version/ref when enforceable, and exported capabilities. If
the provider supports a native rollback or an exactly pinned reinstall, offer
it on verification failure. Otherwise report a truthful `partial` state and a
Repair action; do not claim atomic rollback of provider-managed payloads.

When exact version pinning is unavailable, resolve and inspect the target
marketplace version before installation when the native CLI supports a dry-run
or metadata query. If the installed version exports capabilities not present in
the reviewed plan, post-install verification must mark the action `partial` and
offer plugin disable/rollback or a new collision review. It must never resolve
the surprise by deleting an independent custom skill automatically.

If the user explicitly chooses **Plugin wins** for an overlapping independent
custom skill, create a compound, digest-pinned action rather than deleting it:

1. Back up and archive the custom-skill directory under the provider-home lock.
2. Run the native plugin install/update.
3. Verify the plugin and the expected effective skill capability.
4. Commit both receipts only after verification.
5. If installation fails before the plugin becomes active, restore the archived
   custom skill automatically.
6. If the provider is left partially active, attempt native rollback first;
   restore the custom skill only when doing so cannot create a second active
   collision, otherwise retain the backup and present an explicit recovery
   choice.

Never implement **Plugin wins** as `rm -rf skills/<name>`.

### 6.6 Source conflicts between bundles

Different project bundles can share one provider home. Maintain a machine-local
requirement graph keyed by canonical provider home and normalized plugin ID.

- The same plugin ID and same source fingerprint is one idempotent installation.
- The same plugin ID with different marketplace/source/ref is blocked before
  installation and lists every claiming bundle.
- Detaching a bundle removes its requirement edge but never auto-uninstalls the
  plugin.
- A Codex install that affects every project using the selected `CODEX_HOME`
  must display broader-scope consent.
- Claude uses project scope when the supported CLI provides it.

## 7. Push behavior

1. Refresh global plugin and custom-skill inventory from the mapped provider
   profiles.
2. Open **Choose backup resources**. Plugins and custom skills are individually
   checkable, newly discovered items are unchecked, and prior recipe selections
   remain checked.
3. Show each selected resource as `new`, `unchanged`, `updated`, `missing`,
   `blocked`, or `conflict`, including the current cloud revision/digest.
4. Saving this selection updates the persistent recipe. Push uses that exact
   saved recipe; there is no hidden transient file filter.
5. Validate plugin/skill ownership overlap, effective capability claims, source
   conflicts, and target-name case collisions before capture.
6. Snapshot only checked custom skills using bounded no-follow traversal,
   stability checks, secret warnings, special-file rejection, nested-VCS
   rejection, and executable-content review.
7. Reuse unchanged resource descriptors and `BundleFileEntry` objects from the
   previous manifest. Upload immutable objects only for new or changed files.
8. Emit only checked plugin descriptors and dependency intent; never emit
   plugin payloads.
9. Present a final Push summary of added/updated/removed backup resources. A
   removed checkbox creates an explicit recipe change/tombstone; a temporarily
   missing local resource never does.
10. Publish the immutable manifest and commit, then update the single bundle head
    through the existing CAS boundary.

If a selected resource is unavailable locally, preserve its recipe entry and
report it as unavailable. Do not silently delete it from the next revision.
If the cloud head advanced or the same custom skill changed remotely since the
reviewed base, Push stops and directs the user to Pull review.

## 8. Pull and apply behavior

1. Fetch and validate the complete bundle before creating either plan.
2. Open **Choose resources to install** containing only plugins and custom
   skills present in the fetched revision. Install/update/overwrite actions
   start unchecked; already-matching resources are marked ready.
3. Let the user choose a historical custom-skill version when more than one
   immutable digest exists. Default to the bundle head's version.
4. Compose one overlap preflight from selected cloud resources, target custom
   skills, target plugins, plugin-exported skills, prior receipts, and all
   active claims on the shared provider home.
5. Produce typed directory actions for custom skills and separate native
   dependency actions for plugins. Never represent a custom-skill overwrite as
   many independently selectable file writes.
6. For every existing different custom skill, stop at an explicit Keep,
   Overwrite, or supported renamed-copy decision. Default to Keep.
7. For every plugin, distinguish install, matching no-op, same-source update,
   source replacement, ownership conflict, missing authentication, unsupported
   provider version, and offline/unavailable marketplace.
8. Pin the chosen actions to storage, bundle, revision, commit, manifest
   digest, binding revision, source digests, and expected target digests.
9. Apply approved custom skills transactionally under the provider-home lock.
10. Run approved plugin commands with the bound project as cwd and only the
    bound `CODEX_HOME` or `CLAUDE_CONFIG_DIR` environment override.
11. Re-inventory and verify every result before writing existing apply/dependency
    receipts.
12. Report independent readiness for each selected custom skill and plugin,
    including skipped and partially applied actions.

File apply and plugin installation remain independently retryable. Failure to
install one plugin must not erase successfully restored conversations or skills.

## 9. UI changes

### Push: choose backup resources

- Add **Custom skills** and **Global plugins** subsections under the
  existing Skills and Plugins groups.
- Keep every new global resource unchecked by default.
- Show source profile, provenance, shared-profile scope, observed version,
  current content digest, last cloud revision, change state, and whether a
  plugin supplies skills.
- Render plugin-provided skills as non-selectable children labelled
  `Provided by <plugin>`.
- Show blocked/manual candidates with the ownership evidence and reason.
- Allow independent overlapping resources to be backed up, but show their
  effective-name overlap and explain that Pull will require an install choice.
- Prevent recipe save only when ownership/source ambiguity makes capture itself
  unsafe, such as two payload resources claiming the same logical target.
- Require an explicit Save backup selection before Push and show the exact
  selected plugin/skill count on the Push button or confirmation.

### Pull: choose install resources

- List every backed-up custom skill and plugin with an independent checkbox;
  global mutation actions start unchecked.
- Show cloud version history for custom skills as revision, capture time,
  digest prefix, and changed file count.
- For duplicates, show installed versus cloud version and require **Keep
  installed**, **Overwrite with cloud**, or an adapter-supported renamed copy.
- Keep **Apply approved changes** for transactional custom-skill installs and
  overwrites.
- Keep **Install selected** for native plugin actions and label install versus
  update versus source replacement clearly.
- Show broader-scope warnings before Codex provider-home installs.
- Group a plugin with its marketplace setup and verification as one user-facing
  action while retaining separate internal receipts.
- Show plugin-provided skills beneath the owning plugin and state that the
  native installer will replace only provider-owned payloads.
- Require a second warning before replacing plugin source/ref or removing an
  overlapping independent custom skill.
- After apply, show `ready`, `needs setup`, `conflict`, or `manual` per resource.

## 10. Security boundaries

- Never capture `.codex/plugins/cache`, `.codex/.tmp`,
  `.claude/plugins/cache`, `.claude/plugins/repos`, or
  `.claude/plugins/marketplaces`.
- Never copy plugin manager JSON containing target-local paths.
- Never dereference a skill symlink for payload capture.
- Canonicalize only to classify ownership and enforce containment.
- Allow custom-skill writes only to one validated immediate child of the mapped
  provider home's `skills` directory; reject provider-home/root replacement.
- Treat an existing symlink, hard link, special file, nested mount, or
  ownership-changing path swap as a blocker unless it is a proven
  manager-owned plugin link handled by the native installer.
- Reject nested `.git`, credentials in URLs, special files, path traversal,
  case-fold collisions, reserved names, oversized trees, and known secret files.
- Treat skill scripts and assets as executable/opaque user content and show
  hashes plus file inventory during review.
- Keep overwrite backups and journals local with restrictive permissions; never
  include them in discovery or Push.
- Pass installer arguments as structured argv; never store or execute a shell
  command string from a bundle.
- Never sync marketplace authentication. A private plugin remains blocked until
  the target provider is authenticated locally.
- Revalidate revision, manifest digest, binding revision, target paths, and
  live provider inventory immediately before mutation.

## 11. Edge-case policy

### Selection and identity

- A resource newly discovered on Push is unchecked. A resource newly available
  on Pull has no install approval. Defaults never expand authority.
- Applying only some Pull actions must not consume approval for the others.
  Create a fresh revision-pinned plan for remaining actions or persist
  receipts per action; do not make a skipped plugin permanently un-installable.
- A custom skill name change is an explicit remove/add because its effective
  runtime identity and target path changed. Preserve the old cloud versions and
  never delete the old local target unless a prior receipt proves it is
  unchanged and the user separately approves removal.
- A removed/tombstoned skill remains visible under **Archived backup history**
  while its retained immutable manifests exist. Restoring it does not silently
  re-add it to the Push recipe.
- Same bytes with no prior receipt can be adopted without rewriting, but a hash
  match alone never merges authorship or provenance.

### Shared provider homes

- Missing or unmapped provider home blocks only that provider's actions.
- Canonically equivalent profile paths share the same operation lock and claim
  graph.
- Two bundles requiring the same plugin/source or same custom-skill digest can
  share the installed result and record independent receipts.
- Two bundles requiring different custom-skill digests at the same global path
  remain in conflict; the most recent Pull does not automatically win.
- External changes made by Codex, Claude, another Agent Sync process, or the
  user invalidate the expected target digest and force replanning.

### Custom skill filesystem behavior

- Directory digests include normalized relative paths, file bytes, safe mode
  bits, and file type. They exclude mtimes, owners, ACLs, and extended
  attributes unless a future codec explicitly supports them.
- Unsupported metadata is disclosed before overwrite. Agent Sync must not claim
  an exact backup of ACLs, quarantine attributes, signatures, or platform-only
  metadata it did not capture.
- Empty skills, oversized files/trees, excessive depth/count, case-only names,
  Unicode normalization collisions, reserved device names, and paths exceeding
  target limits are blocked.
- A target directory containing extra local files is a different digest; the
  overwrite preview lists those files as deletions because replacement is a
  complete-tree operation.
- `Install as renamed copy` creates a new projected digest and local identity.
  It must not masquerade as the original cloud version on a later Push.
- Cancellation before mutation removes staging. Cancellation after the journal
  enters the replacement phase completes rollback/recovery before returning.

### Plugin behavior

- Installed-but-disabled and installed-and-enabled are distinct target states.
  Pull shows whether the approved native action will enable the plugin.
- Marketplace setup shared by several selected plugins is deduplicated, but
  each plugin keeps an independent result and retry path.
- A partially installed/corrupt plugin is routed through native Repair rather
  than classified as safely absent.
- A local-path marketplace is manual-only unless a supported adapter can express
  safe portable provenance; absolute source paths never enter the bundle.
- Private marketplace authentication, license acceptance, OS packages, and
  provider trust prompts remain target-local prerequisites.
- If the provider cannot pin or preview the resolved plugin version, the UI
  states that install may differ from the observed backup version and requires
  confirmation.
- A plugin update that succeeds but cannot be verified is `partial`, never
  `ready`; retries start from fresh inventory rather than replaying stale argv.
- Plugin uninstall is outside this flow. Recipe removal and bundle detach never
  remove provider-managed payloads.

### Cloud history and failure recovery

- History listing is bounded and paginated; malformed or missing historical
  commits/manifests do not invalidate the current verified head but make the
  affected old version unavailable.
- Current Pull verifies every chosen historical manifest and object before
  planning a write.
- Storage offline, object missing, digest mismatch, CAS conflict, timeout, or
  cancellation produces no new head and no fabricated version receipt.
- Local backup retention never participates in cloud garbage collection.
  Cloud retention never deletes an object referenced by a retained manifest.
- Backup directories may contain user secrets because custom skills are opaque;
  keep them permission-restricted, disclose their location, and delete them only
  through an explicit retention/cleanup policy.

## 12. Implementation slices

### Slice 1: domain and provenance

- Correct standalone captures to emit `ResourceKind::StandaloneSkill`,
  provider-state scope, and `Provenance::StandaloneSnapshot`.
- Define the versioned provider-adapter contract for declared names, effective
  runtime names, namespaces, canonicalization, and target-path collision keys.
- Add normalized plugin source metadata and provided-capability metadata.
- Reuse resource IDs, `content_sha256`, revision manifests, commit history,
  file entries, and apply receipts for custom-skill identity and versions.
- Add typed `InstallCustomSkill` and `OverwriteCustomSkill` plan actions with
  expected target tree digests and explicit overwrite decisions.
- Add capability-map and source-conflict validators.

### Slice 2: provider inventory adapters

- Extract reusable Codex and Claude inventory/provenance functions from
  `codex_plugins.rs` without depending on legacy lock-file synchronization.
- Add bounded plugin capability/manifest discovery.
- Add recorded fixtures for native effective identifiers and plugin namespace
  behavior on every supported provider version.
- Add global skill enumeration and ownership classification.
- Replace `standalone_skills: Vec::new()` with discovered, selectable sources.

### Slice 3: capture and recipes

- Merge global plugin/skill candidates into registration and refreshed
  inventory.
- Keep them off by default and persist exact selections.
- Coalesce project-config and global-inventory observations of the same plugin.
- Enforce selected-only payload/dependency capture.
- Reuse unchanged manifest entries/objects and derive custom-skill history by
  comparing resource digests across existing immutable revisions.
- Add history traversal and selection APIs without creating another cloud head
  or skill-version database.

### Slice 4: restore and shared-home coordination

- Add combined overlap preflight.
- Reuse hardened native plugin planning/apply/verification.
- Persist shared-provider plugin requirement claims.
- Change standalone targets from project-local directories to the mapped global
  provider home's preserved `skills/<install-dir>` directory.
- Implement provider-home operation locking, complete-directory staging,
  local backup/journal, atomic replacement where the platform supports it, and
  rollback on Agent Sync-managed overwrite failure.
- Use existing apply receipts as the installed revision/digest claim.

### Slice 5: UI and readiness

- Add separate Push backup selection and Pull install selection surfaces.
- Add provenance, version history, ownership, duplicate, overwrite, and
  shared-profile-scope presentation.
- Add Keep/Overwrite/renamed-copy decisions and conflict-directed deselection.
- Extend readiness to distinguish missing payload, missing plugin, source
  conflict, duplicate custom skill, partial plugin update, and manual setup.

### Slice 6: integration coverage

- Add backend command-flow and frontend Pull-review integration tests.
- Run legacy plugin tests unchanged to prove the port did not regress legacy
  mode.

## 13. Test matrix

### Ownership and overlap unit tests

- regular global skill is classified as standalone;
- standalone `SKILL.md` name and directory mismatch is blocked;
- symlink into an inventoried plugin root is classified as plugin-provided;
- symlink outside approved roots is blocked;
- regular directory under a plugin root is never captured;
- equal names or hashes alone do not establish plugin ownership;
- native effective identifiers take precedence over manifest composition;
- a tested plugin namespace produces a distinct runtime capability key;
- an unverified effective name is not silently merged;
- runtime capability and target-path collisions are detected independently;
- pull blocks when the target provider adapter cannot revalidate the recorded
  naming rule;
- plugin discovered from config and global inventory coalesces;
- standalone/plugin effective-name overlap can be backed up but blocks
  simultaneous installation until resolved;
- two source fingerprints for one plugin ID block installation; and
- same effective name across different providers is allowed.

### Capture tests

- unselected global skills/plugins produce no descriptor, file, or dependency;
- selected standalone skill produces only its bounded snapshot;
- selected plugin produces intent without payload;
- executable skill requires explicit file and dependency approval;
- plugin cache and manager-state paths never enter a manifest;
- stable resource ID survives content changes;
- unchanged skill files reuse previous manifest/object entries;
- file addition, modification, mode change, and deletion change the tree digest;
- a skill changing during capture retries and then blocks without publishing a
  torn snapshot;
- missing selected skill remains selected/unavailable and does not become a
  tombstone;
- explicit recipe removal creates the intended resource tombstone; and
- head CAS conflict prevents a stale skill update from replacing a newer remote
  revision.

### Cloud history tests

- the first skill snapshot appears in one immutable manifest revision;
- an unchanged skill across later bundle revisions appears as one user-visible
  skill version;
- an updated skill produces a new digest and history point without changing its
  resource ID;
- history traversal follows existing commits/manifests and validates every
  digest;
- previous manifests still fetch their original immutable file objects;
- choosing an old skill version installs those exact files;
- pushing a restored old version creates a new revision instead of rewinding
  the head; and
- history retention never removes objects referenced by a retained manifest.

### Custom skill install/overwrite tests

- machine A global skill -> bundle -> machine B mapped global skill directory;
- machine B edits the installed skill and pushes under the same resource ID;
- missing target offers Install but remains unchecked until selected;
- matching target is an idempotent no-op;
- matching unclaimed target can be adopted without rewriting;
- different same-name target is detected automatically and defaults to Keep;
- Overwrite cannot run without explicit approval tied to the expected target
  digest;
- overwrite stages and verifies the complete directory before target mutation;
- overwrite removes files deleted by the selected cloud version;
- failure before backup leaves the target unchanged;
- failure after backup restores the prior directory and retains a recovery
  journal;
- target change between plan and apply aborts without overwrite;
- unknown symlink, hard link, special file, nested mount, and case-only collision
  block replacement;
- insufficient disk space or backup failure prevents mutation;
- executable modes survive safely while set-id bits are stripped;
- renamed-copy is offered only when the adapter can project a valid declaration;
- local backups are excluded from discovery and Push; and
- two bundles claiming the same target/digest coexist, while a later divergent
  update requires review.

### Plugin install/update tests

- matching installed plugin becomes an idempotent no-op;
- missing plugin invokes the exact native argv with the bound profile env;
- conflicting plugin provenance blocks before process launch;
- same-source plugin update is explicitly selectable and replaces only
  provider-owned plugin skills;
- plugin source/ref replacement requires the stronger confirmation;
- plugin install never deletes an independent custom skill without its separate
  approved overwrite/removal action;
- **Plugin wins** archives the custom skill, verifies the plugin, and restores
  the custom backup when installation fails before activation;
- manager-owned symlink replacement is allowed only when its target remains in
  the proven plugin install root;
- plugin owned by a different plugin ID cannot be overwritten;
- private marketplace without target-local authentication is blocked without
  exposing or requesting synced credentials;
- offline marketplace, missing CLI, timeout, cancellation, and nonzero exit all
  produce accurate receipts;
- successful install is re-inventoried and verified;
- verification failure invokes native pinned rollback when supported and
  otherwise reports `partial` plus Repair;
- recorded plugin version is labelled observational when exact pinning is not
  supported;
- one failed plugin does not roll back successful file actions; and
- detaching the last bundle claim does not uninstall a plugin.

### Concurrency and recovery tests

- two simultaneous operations on one provider home serialize on the same
  canonical lock;
- differently spelled paths resolving to the same provider home share the lock;
- stale plans fail on revision, binding, source, or target digest mismatch;
- app termination with a custom-skill journal is detected and recoverable on
  restart;
- provider process activity that makes replacement unsafe blocks the action;
- a plugin update and custom-skill overwrite with overlapping capabilities
  cannot race; and
- partial receipts never mark skipped or failed resources ready.

### Frontend integration tests

- Push backup inventory shows global resources individually and new resources
  start unchecked;
- saved backup selection is the exact set published;
- plugin-provided skills are visible but not separately selectable;
- selecting independent overlapping owners backs up both with a warning, while
  Pull blocks activating both until the user resolves the overlap;
- updated skills display current and prior cloud revisions/digests;
- Pull install actions start unchecked and allow independent plugin/skill
  selection;
- applying one subset leaves remaining resources available through a fresh
  pinned plan;
- duplicate custom skill shows Keep and Overwrite, with Keep as default;
- Pull review separates custom-skill apply from plugin install;
- same-plugin update explains that provider-owned plugin skills will be
  replaced;
- independent custom content requires separate approval before plugin install
  can remove it;
- broader-scope Codex warning appears before approval; and
- readiness updates after successful and failed verification.

## 14. Acceptance criteria

1. Push backs up exactly the global plugins and custom skills selected by the
   user; new discoveries never enter a bundle automatically.
2. A custom skill can be updated under the same resource identity, and its cloud
   history is derived from existing immutable bundle revisions and digests.
3. Pull lets the user independently select which backed-up plugins and custom
   skills to install into mapped provider homes.
4. Existing different custom skills are detected automatically and cannot be
   overwritten without a digest-pinned explicit decision and recoverable local
   backup.
5. A same-owner plugin update may safely replace its plugin-provided skills only
   through the native provider installer and post-install verification.
6. Plugin approval alone never overwrites an independent custom skill or a skill
   owned by another plugin.
7. A user can select installed Codex and Claude plugins individually and
   reinstall or update them through native provider commands on pull.
8. A plugin-provided skill never appears as a copied standalone payload.
9. A plugin/standalone effective-name overlap is visible and cannot silently
   produce two owners.
10. A Push-unselected global plugin or skill never enters the manifest, object
    store, or dependency intent. A Pull-unselected item remains visible for
    later installation but produces no mutation or successful apply receipt.
11. No plugin cache, clone, manager database, credential, trust state, or
   machine-local source path is uploaded.
12. Plugin application is idempotent, provenance-checked, explicitly approved,
   and post-install verified.
13. Custom skill identity survives pull, local edit, and push from another
   machine.
14. Existing schema-3 project resources and legacy profile sync continue to pass
   their integration suites unchanged.

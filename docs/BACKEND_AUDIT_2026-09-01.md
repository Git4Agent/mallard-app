# Mallard backend audit

- **Audit date:** 2026-09-01
- **Baseline:** `dd8d420` plus the 25 modified/untracked files in the working tree
- **Scope:** all Rust in the workspace — `mallard-domain`, `mallard-infrastructure`,
  `mallard-application`, `mallard-cli`, and the `src-tauri` shell
  (`project_sync/`, `storage_catalog/`). ~122k lines including tests. The React
  frontend was reviewed only where it participates in a backend contract
  (single-flight, error DTOs).
- **Method:** four parallel auditors (filesystem safety, crypto/transport,
  concurrency/command surface, architecture), followed by a verification pass in
  which every headline finding was re-read at the cited `file:line` by the
  lead. Seven auditor claims did not survive verification and are recorded in
  [Refuted and corrected claims](#refuted-and-corrected-claims) so they are not
  re-raised later.
- **Purpose:** assess whether the safety invariants declared in `AGENTS.md` are
  actually enforced in code, and what the backend costs to maintain. This is
  **audit only** — no source files were modified.

---

## Executive assessment

**The backend is in good shape, and the safety-critical core is genuinely
sound.** `cargo clippy --workspace --all-targets` exits 0 with zero warnings.
`cargo test --workspace --all-targets` exits 0: **774 passed, 0 failed, 8
ignored**. The invariants that matter most for a tool that restores files into
provider homes and publishes conversations to object storage — path
containment, no-follow traversal, verify-before-use integrity, real
compare-and-swap, plan-gated apply, opt-in deletion — are all correctly
implemented and defended in depth. See
[Invariants verified as enforced](#invariants-verified-as-enforced).

The defects found are concentrated in two places the compiler and the test
suite cannot see:

1. **Validation that lives at the wrong boundary.** Several checks are enforced
   where a value *enters* the system but not where it is *used*. The S3
   endpoint allowlist is the clearest case (D1): it is enforced on save and in
   `list_storages`, but the storage-catalog transport path calls only the
   generic `StorageConfig::validate`, which never inspects the endpoint.
2. **The catalog subsystem, which is newer and less tested than the project-sync
   core.** Four of the five highest-severity defects (D3, D4, D5, and the
   coverage gap A4) are in `storage_catalog/`. `apply.rs` — 1,425 lines that
   execute journaled mutations including a recursive delete on recovery — has
   zero inline tests and is covered by exactly one e2e test.

None of the defects is remotely exploitable as the product ships today. The
realistic threat model for D1–D4 requires write access to `~/.mallard`, which
already grants near-total control of the app. They are worth fixing because
they are **asymmetries**: the project-sync restore path defends the same
operations correctly, and the catalog path does not. When a future change makes
a catalog plan persistable or remotely sourced, these gaps become reachable.

The dominant maintainability risk is not a defect but a single file:
`src-tauri/src/project_sync/service.rs` at **28,400 lines, 56.6% of it
production code** (A1). Compounding it, `AGENTS.md`'s source map is materially
stale (A2) — it names four backend files that do not exist and omits the entire
five-crate workspace, which actively misdirects every agent session that loads
it as authoritative context.

Headline numbers:

| | |
|---|---|
| Clippy warnings, `--workspace --all-targets` | **0** |
| Tests passed / failed / ignored | **774 / 0 / 8** |
| Confirmed defects | **14** (0 Critical, 0 High, 6 Medium, 8 Low) |
| Auditor claims refuted on verification | **7** |
| `AGENTS.md`-named backend files that do not exist | **5** |
| Largest single file | `project_sync/service.rs`, **28,400 lines** |
| Largest single function | `provider_thread_review`, **1,454 lines** |
| Registered Tauri commands / dead commands | **87 / 0** |
| Production modules with zero tests | **3** (`conversation_materialization`, `project_sync/commands`, `storage_catalog/apply`) |

---

## Invariants verified as enforced

Recorded so that these are not re-audited or, worse, "simplified" away.

**Path containment on restore is four-layered and correct.**
`validate_target_for_logical` (`crates/mallard-infrastructure/src/bundle_engine.rs:4010`)
applies a lexical `target.starts_with(&root)` check, then
`prospective_canonical_path` (`:4075`, which walks ancestors with
`symlink_metadata`, errors on *any* symlink, canonicalizes the deepest existing
ancestor and re-appends missing components), then re-checks containment against
the canonical root, then `inspect_existing_ancestors_no_symlink` (`:4053`).
Roots come from `canonical_real_dir` (`:4442`), which requires absolute +
non-symlink + real directory before `fs::canonicalize`. Manifest-derived
logical paths never reach the filesystem unvalidated: `safe_lexical_join`
(`:3934`) rejects non-absolute roots, backslashes, and any non-`Normal`
component, and the `LogicalPath`/`ProviderKey` grammars reject `..`
(`crates/mallard-domain/src/lib.rs:592`, `:871`). Directory creation is
per-component `fs::create_dir` — never `create_dir_all` — with containment
re-checked (`create_safe_directory_tree`, `:4362`). Apply re-derives targets and
refuses a plan/target mismatch (`:5151`, `:5213`) rather than trusting
`action.target_path`.

**No-follow traversal holds on both the read and write sides.** Every `WalkDir`
in the workspace sets `.follow_links(false)`:
`provider_capture.rs:1245,2037,2073`; `providers/claude/discovery.rs:77,143`;
`providers/codex/discovery.rs:34`; `storage_core/local_store.rs:365`. Capture
rejects symlinks and special files at `provider_capture.rs:1307-1325`.
`skill_tree.rs:87` uses `symlink_metadata` + `libc::O_NOFOLLOW` plus a
before/after length stability check.

**Bounded reads and counts are pervasive.** Capture limits: 20,000 discovered
files, 16 MiB per file, 512 MiB per capture, 1 MiB / 128 lines of metadata,
depth 32 (`provider_capture.rs:21-28`). Manifest limits: 128 storages, 20,000
resources, 100,000 files and actions, 1,024-byte logical paths, 100 MiB
conversation files (`mallard-domain/src/lib.rs:34-47`), enforced in
`BundleManifest::validate` (`:4868`). Head and tag reads are capped at 64 KiB
(`bundle_engine.rs:38-39`). **There is no zip/tar/gz decompression anywhere in
the workspace**, so no archive-bomb or archive-path-escape vector exists.

**Atomic persistence is correct.** `persistence.rs:2775-2795` uses
`NamedTempFile::new_in(parent)` (same directory, so the rename is atomic on one
filesystem), `set_permissions(0o600)`, `sync_all()`, then `persist()`.
Directories are created `0o700` (`:1920`, `:2070`, `:2415`), and reads use
`O_NOFOLLOW` (`:245`). `write_target_atomic_from_reader` (`bundle_engine.rs:4107`)
additionally re-inspects the expected target digest, size, and mode *after*
staging and *before* `persist`, closing the TOCTOU window.

**Deletion is opt-in, digest-verified, and never recursive.**
`bundle_engine.rs:5157-5188` verifies `before_digest == last_sha256` *and* the
plan's `expected_target_sha256` *and* the mode, writes an immutable backup,
re-inspects, and only then calls `fs::remove_file`. `:5231` refuses non-empty
directories ("project directory is not empty; recursive deletion is forbidden")
and uses `fs::remove_dir`. There is **no `remove_dir_all` in the apply path** —
the only occurrence in `bundle_engine.rs` is `#[cfg(test)]` code at `:2959`.

**Compare-and-swap is a real conditional put, and the service is empirically
probed.** `s3_store.rs:682-688` maps `CasExpectation::Absent` to
`if_none_match("*")` and `Match(etag)` to `if_match(...)`. Losses surface as
`CasOutcome::Conflict` (`:768-775`), never a silent overwrite; ambiguous
timeout/dispatch results are resolved by verified read-back (`:779-796`).
`probe_r2_capabilities_async` (`:269-358`) *proves at setup time* that the
service honors preconditions, including `StaleConditionalRejection`, and rejects
servers that do not. Commit happens only after the probe passes
(`storage_setup.rs:934`), which is the sole writer of
`supports_conditional_writes: Some(true)` (`:772`).

**Integrity is verified before use, everywhere.** `stage_verified_async`
(`s3_store.rs:588-663`) streams into a `NamedTempFile` while hashing and returns
`Verified` only on an exact size + digest match; the temp file is dropped
otherwise. `read_manifest` (`bundle_engine.rs:1608-1622`),
`inspect_manifest_version` (`:1119`), and `fetch_manifest_version_optional`
(`:1167`) all stage-verify before parsing. On write, `copy_and_verify`
(`:4309-4339`) runs before `temp.persist(path)` (`:4151`). The chain of trust
is intact: head (etag, CAS-protected) → `manifest_sha256` + `manifest_size` →
manifest → per-entry `sha256` → objects. The three mutable roots trusted
without a per-object digest (`_head.json`, `_tag.json`, `.mallard/_storage.json`)
are each byte-capped and schema-validated, and the head is the CAS anchor.

**Expiry and revision checks cannot be bypassed by omitting fields.**
`RestorePlan.expires_at` is a *required* serde field
(`mallard-domain/src/lib.rs:5304`, no `#[serde(default)]`), so its absence in
remote JSON fails deserialization rather than defaulting to "never expires".
`validate_plan_pin` (`bundle_engine.rs:4400`) rejects `now > expires_at` **and**
`now < created_at`, and pins storage, bundle, host, replica, revision, commit,
manifest digest, and binding revision. Apply re-reads the live head and rejects
drift (`:4836-4842`).

**Transport security.** rustls with aws-lc (`s3_store.rs:1312-1318`). No
`danger_accept_invalid_certs`, no custom verifier, no raw-socket HTTP client
anywhere in the workspace. `parse_r2_api_url` (`storage_setup.rs:264-368`)
requires `https`, rejects userinfo/port/query/fragment/backslash, accepts only
`[acct].r2.cloudflarestorage.com` plus the `.eu.` and `.fedramp.`
jurisdictions, and rebuilds the endpoint canonically.

**Credential handling.** `zeroize` is genuinely used, not merely declared:
`SecretString(Zeroizing<String>)` (`storage_setup.rs:34`),
`PendingStorage::drop` (`:1284-1290`), and the draft secret is zeroized on every
return path (`:447,473,496,813,1058,1287`). `R2SetupJournalEntry` is
`deny_unknown_fields` with a test asserting no secret/path/token channel
(`r2_setup_journal.rs:648-681`). `R2CapabilityProbeError` deliberately drops the
SDK error, endpoint, bucket, and key (`s3_store.rs:75-82`). Constant-time
comparison is used exactly where a secret is compared
(`r2_browser_setup.rs:617-626`); digest comparisons correctly do not need it,
since SHA-256 here is a public integrity value rather than a keyed MAC.

**Concurrency discipline.** No `std::sync::Mutex` guard is held across an
`.await`. Every command is an `async fn` that dispatches blocking filesystem and
parse work through `tauri::async_runtime::spawn_blocking` (`commands.rs:19-25`,
`resource_commands.rs:19-25`, `activity_log.rs:165-171`, `app_settings.rs:15-21`,
`app_reset.rs:22-28`). The synchronous S3 `block_on` (`s3_store.rs:211`)
executes *inside* a `spawn_blocking` thread, where it is legal, and
`catch_unwind`s with a message telling callers to use `spawn_blocking`. Because
work runs under `spawn_blocking`, a panic surfaces as a `JoinError` mapped to a
command error (`commands.rs:22-24`) — not an app crash and not an unresolved
frontend promise. Backend single-flight uses `try_lock` with an RAII guard
(`acquire_host_operation`, `persistence.rs:266`), so it cannot get stuck;
frontend single-flight (`src/components/project-sync/api.ts:63-78`) clears on
both settle paths and is identity-guarded.

**Layering is a clean DAG:** domain ← infrastructure ← application ←
{src-tauri, mallard-cli}. No cycles. `mallard-domain` purity verified — zero
references to `std::fs`, `std::env`, `std::net`, `std::process`, `PathBuf`, or
`tauri` across all five of its files including `tests.rs`. `storage_core` and
`storage_catalog` never import `project_sync` (structurally enforced: `mod
project_sync` exists only at `src-tauri/src/lib.rs:15`).

**No backward-compatibility debt.** Zero `#[deprecated]` workspace-wide. All 12
schema constants are `= 1` (`mallard-domain/src/lib.rs:16-28`); no v1/v2 dual
paths and no migrations. The `legacy` tokens in
`codex_paginated_projection.rs:5,1113,1222` and
`providers/codex/history.rs:2028-2119` refer to *upstream Codex's own* rollout
format, which Mallard must read — not to Mallard compatibility shims.

---

## Defects

No Critical or High severity defects were found. Six Medium, eight Low.

### D1 — Medium: S3 endpoint is not validated at the transport boundary

`StorageConfig::validate` (`crates/mallard-domain/src/lib.rs:809-825`) checks
only that `bucket` is non-empty for the S3 kind; it **never inspects
`s3_endpoint`**. `make_s3_client` then uses that field verbatim:

```rust
let endpoint = if !config.s3_endpoint.is_empty() {
    config.s3_endpoint.clone()          // s3_store.rs:1287-1289
```

passed straight to `.endpoint_url(endpoint)` (`s3_store.rs:1309`) with no scheme
check.

The HTTPS + R2-host allowlist *is* enforced — but only at two entry points:
`save_project_sync_config_with_repository` (`src-tauri/src/project_sync/service.rs:1672`)
and `list_storages` (`crates/mallard-application/src/storage_setup.rs:376`). The
storage-catalog transport path skips it: `engine_for`
(`src-tauri/src/storage_catalog/service.rs:66-72`) calls only
`storage.validate()`, never `validate_r2_storage_destinations`.

**Impact.** A `~/.mallard` config carrying `http://…` or a non-R2 host is
honored by catalog operations, transmitting the static secret access key and
signed requests in cleartext to an arbitrary endpoint.

**Honest severity note.** Producing such a config requires local write access to
`~/.mallard`, which already grants near-total control of the app; the file is
mode `0600`. This is therefore defense-in-depth plus an inconsistency between
entry points, not a remotely reachable vulnerability. It is rated Medium
because the same value is treated as security-critical three call-sites away.

**Fix.** Assert `https://` and the R2 host allowlist inside
`S3ObjectStore::from_config` / `make_s3_client`, so validation lives where the
value is consumed rather than where it happens to enter.

### D2 — Low: `supports_conditional_writes: None` is accepted

```rust
if config.supports_conditional_writes == Some(false) {   // s3_store.rs:164
```

Only an explicit `false` is rejected; `None` builds a store whose CAS has never
been probed.

Not reachable through the UI: `payload_storage`
(`src-tauri/src/project_sync/service.rs:1468-1481`) sets `None` only for inert
setup drafts, commit sets `Some(true)` (`storage_setup.rs:772`), and
`same_verified_r2_settings` includes the field (`service.rs:1275`) so
`require_verified_r2_changes` (`:1282`) blocks the generic config save from
creating or altering any S3 storage. Reachable only via a hand-edited config.

**Related and more substantive:** the capability probe is a one-time *setup*
gate. Nothing re-verifies at push time, so a service that later stops honoring
`If-Match` would silently overwrite heads. **Fix:** require `Some(true)`, and
consider re-probing on first push per session.

### D3 — Medium: catalog journal recovery renames and recursively deletes on unvalidated persisted paths

`CatalogOperationsDocument::validate` (`src-tauri/src/storage_catalog/domain.rs:1816-1840`)
validates the schema, the entry count, `entry.target`, and duplicate IDs — but
**not** `target_path`, `backup_path`, or `copy_target_path`. Recovery then maps
those raw strings onto the filesystem:

```rust
let target_path = entry.target_path.as_ref().map(PathBuf::from);   // apply.rs:1331
...
restored = fs::rename(backup, target_path).is_ok();                // apply.rs:1365
...
let _ = fs::remove_dir_all(stage_dir_for(root, &entry.operation_id, ""));  // apply.rs:1407
```

with `root = target_path.parent()`. This journal **is persisted**
(`persistence.rs:399-408`), and recovery runs on every scope listing
(`service.rs:1060`), not only at startup.

**Mitigants.** Exploitation requires write access to `~/.mallard`.
`operation_id` is a hex-only validated newtype (`domain.rs:64-90`), so the
`remove_dir_all` leaf is constrained to
`.mallard-stage-catop-<24hex>[-copy|-incoming|-fork]`. Rust's `remove_dir_all`
is no-follow for traversal, so symlinked *contents* are unlinked rather than
followed. But `root` and the `rename` destination are unconstrained.

This is the only place in the backend where a recursive delete is driven by a
persisted, unvalidated path — contrary to the spirit of the "directory deletion
must never become recursive" invariant, which the project-sync restore path
upholds strictly.

**Fix.** Run `validate_absolute_clean_path` over all three fields in
`CatalogOperationEntry` validation, and canonicalize `root` before deriving
stage paths.

### D4 — Medium: `CatalogPullAction::validate` never validates `target_path`

`src-tauri/src/storage_catalog/domain.rs:2069` declares
`pub target_path: Option<String>`. The validator at `:2075-2123` checks digests,
kind/target pairing, destructive-approval requirements, and
`copy.copy_target_path` (`:2119`) — but not `self.target_path`. Apply consumes
it raw: `apply.rs:1204` `target_path: Path::new(target_path)`, and
`replace_directory_target` derives its staging root from `target_path.parent()`.

The project-sync equivalent does validate it:

```rust
validate_absolute_clean_path("restore target", path)?;   // mallard-domain/src/lib.rs:5236
```

**Not remotely exploitable today:** catalog plans are in-memory only and never
deserialized (`plan_store.rs`), and `verify_pins` re-binds skill roots before
apply (`service.rs:1653`). **Fix:** add the one-line check for symmetry, so the
validator is already correct if plans ever become persistable.

### D5 — Medium: `unlink_catalog_scope_storage` revision check is a TOCTOU

`src-tauri/src/storage_catalog/service.rs:1454-1456` checks the revision after a
standalone `load_scope_links()`:

```rust
if link.revision != expected_revision {
    return Err("the catalog link changed; refresh before unlinking".to_string());
}
```

then performs **four separate** guarded mutations (`:1480`, `:1492`, `:1504`,
`:1512`). The final `mutate_scope_links` (`:1512`) does not re-check the
revision. A concurrent link mutation between the check and the removal is not
rejected.

This is inconsistent with `delete_scope` and `rename_profile`, whose revision
checks sit *inside* the `mutate_*` closure (`service.rs:694`, `:797`) and are
therefore atomic — the pattern `AGENTS.md` asks to preserve.

**Impact.** Unlink can proceed on a stale UI revision. Low blast radius on a
single-user desktop. **Fix:** move the revision check inside the
`mutate_scope_links` closure.

### D6 — Low/Medium: local-store CAS is atomic only within one machine

`storage_core/local_store.rs:272-291` takes an advisory `File::lock`, compares,
then calls `temp.persist(&path)` — an *overwriting* rename. Backups in the same
codebase correctly use `persist_noclobber` (`bundle_engine.rs:4304`).

Two Mallard installations over one synced folder (Dropbox, NFS, SMB) therefore
get no atomicity, producing a head lost-update. This violates both "mutable
heads require compare-and-swap semantics" and the requirement that the
local-folder and S3 stores be interchangeable implementations of one model.
Advisory `flock` is also not honored by file-sync daemons at all.

**Fix:** use `persist_noclobber` for the head and retry on `AlreadyExists`, or
document the single-writer constraint explicitly in the local-store contract.

### D7 — Medium/Low: `head.manifest_size` has no upper bound

```rust
if self.manifest_size == 0 {   // mallard-domain/src/lib.rs:4217
    return Err("head manifest size must be nonzero".to_string());
}
```

Only zero is rejected. `bundle_engine.rs:1607-1620` passes the value as
`ExpectedObject.size`, so `stage_verified` streams that many bytes into a
`NamedTempFile` *before* the SHA-256 comparison can fail
(`local_store.rs:157-205`).

**Impact.** A hostile or compromised bucket can force arbitrary temporary-disk
consumption. Integrity is unaffected — the digest check still rejects the
content — so this is denial-of-service only. Note the contrast: head and tag
reads *are* capped at 64 KiB (`bundle_engine.rs:38-39`). **Fix:** add an upper
bound commensurate with `MAX_LOGICAL_PATH_BYTES × MAX_RESOURCES`.

### D8 — Low: absolute local paths reach the persisted activity log

Infrastructure errors embed paths via `.display()`:
`persistence.rs:1320,1324,1327,1459,1461` and
`storage_catalog/persistence.rs:82,88,101,103`, e.g.

```rust
Ok(_) => return Err(format!("'{}' is not a real directory", directory.display())),
```

Command handlers then format those strings into log messages
(`commands.rs:335`: `&format!("Conversation recovery is unavailable: {error}")`),
which are appended to the on-disk JSONL activity log.

`sanitize_message` (`crates/mallard-infrastructure/src/activity_log.rs:786-808`)
redacts **only credential-shaped markers** — `authorization:`, `bearer `,
`access_key_id=`, `secret_access_key=`, `password=`, `token=`,
`x-amz-signature=`, `x-amz-credential=`, `x-amz-security-token=`. It does not
strip filesystem paths.

`AGENTS.md` states: "Never commit or log credentials, bearer tokens, private
keys, **local absolute paths**, personal provider data…". Credential redaction
is working well; path redaction is absent. Exposure is limited — these are the
user's own `~/.mallard` paths in a local log that never becomes portable — so
this is a stated-contract violation rather than a leak with real blast radius.

### D9 — Low: poison-handling asymmetry on process-global locks

`session_watch_service` (`session_watch_service.rs:764,901`) and `plan_store`
(`plan_store.rs:33`) recover from a poisoned lock via
`unwrap_or_else(|e| e.into_inner())`. But the two process-global persistence
locks **fail permanently**:

```rust
"persistence lock is poisoned"    // persistence.rs:2578-2580
```

and likewise `storage_catalog/persistence.rs:114-116`. A panic inside any
`mutate_*` closure would brick all persistence until restart.

**Mitigant:** those closures validate through `Result` (`.validate()?`) rather
than panicking, so poisoning is unlikely in practice. This is a latent hazard
and an inconsistency, not an active bug. **Fix:** either recover uniformly with
`into_inner`, or document why the persistence locks deliberately do not.

### D10 — Low: `StorageConfig` derives `Debug` with a plaintext secret

`crates/mallard-domain/src/lib.rs:784-806`. `secret_access_key` is a plain
`String` in a `#[derive(Debug)]` struct. No `{:?}` call site exists today, but a
single future `format!("{:?}", config)` in an error path leaks the key.
**Fix:** a manual redacting `Debug` impl.

### D11 — Low: `BundleTag` has no `validate()`

`bundle_engine.rs:1231-1234` checks only `schema_version` and `bundle_id`, so
unvalidated `display_name`, `repository_fingerprint`, and an unbounded
`providers: Vec<ProviderKey>` flow from remote-controlled bytes into
`RemoteBundleSummary` and on to the UI. The tag is documented as advisory
(`:438-442`) and acting flows re-read the head, so this is a display/injection
hygiene issue rather than an integrity one.

### D12 — Low: `remove_directory_tree_if_owned` uses a purely lexical prefix guard

`crates/mallard-infrastructure/src/conversation_materialization.rs:1854` guards
with `if !path.starts_with(root)` and no canonicalization, then calls
`remove_dir_all` at `:1860`. The single caller (`:1641`) passes
`intent_directory()`, built from the same uncanonicalized `repository.root()`
(`persistence.rs:227`), so both sides share one prefix and the check is sound
today. It would not survive a future caller passing an independently derived
path. **Fix:** canonicalize both sides, matching `bundle_engine.rs:4010`.

### D13 — Low: `put_many` omits `abort_and_drain`

`s3_store.rs:1140-1144` propagates the first error via `??` and relies on
`JoinSet`'s drop-abort, unlike its siblings `get_metadata_many` (`:1006`) and
`stage_many_verified` (`:1079`). An in-flight PUT can still land server-side
after the caller sees a failure. Benign for immutable content-addressed
objects, but inconsistent with the established pattern in the same file.

### D14 — Low: `fs::read` in `session_detail` follows symlinks

`crates/mallard-infrastructure/src/session_detail.rs:434` reads without
`O_NOFOLLOW`, unlike `skill_tree.rs:94` and `persistence.rs:245`. Mitigated: the
path is already canonical, `symlink_metadata` at `:417` confirmed a non-symlink
regular file immediately prior, and the size is re-checked post-read at `:440`.
Residual TOCTOU only.

---

## Architecture and maintainability

### A1 — High: `project_sync/service.rs` is a 28,400-line god file

Measured precisely: **16,077 production lines (56.6%)** and **12,323 test lines
(43.4%)** across 7 `#[cfg(test)]` blocks, the dominant one spanning
16,219–28,399 (12,181 lines, 122 `#[test]`). **453 functions**, 24 `pub fn`,
~70 structs, 6 enums, and **0 `#[tauri::command]`** — those live in
`commands.rs`.

The bulk is production logic, not tests. Largest functions:

| Function | Lines | At |
|---|---|---|
| `provider_thread_review` | 1,454 | L4722 |
| `apply_prepared_pull_conversation` | 980 | L10772 |
| `build_publish_request` | 979 | L8253 |
| `build_thread_sync_comparison` | 436 | L6756 |

Seven functions exceed 250 lines; four exceed 400.

Every *other* large backend file is test-heavy — `bundle_engine.rs` 1,721
production / 3,688 test, `provider_capture.rs` 2,386 / 1,034, `persistence.rs`
2,798 / 1,384. `service.rs` is the sole outlier, which is why "it's mostly
tests" is not an available defense.

Concrete costs:

- `commands.rs:3` is `use super::service::*;` — a **glob import of a
  16k-line production module**, so the Tauri adapter's namespace is unbounded
  and no reader can tell which of 453 functions a command actually touches.
- The test module imports `crate::storage_catalog::persistence` and
  `crate::sync_tests::thread_variants::harness` (L16226-16229), so **no unit
  test in this file can run without the whole application linked**.
- Every push, pull, review, session, and setup feature touches this one file,
  making it a guaranteed merge-conflict surface for parallel work.

**Recommendation.** Extract along the seams that already exist in the function
names — thread review, publish-request construction, pull application, and
session observation are four cohesive units. Replace the glob import with
explicit imports as the first step; that alone will reveal the true coupling.

### A2 — High: `AGENTS.md`'s source map is materially stale

`AGENTS.md` is loaded as authoritative context for every agent session, so a
wrong source map actively misdirects work.

**Named but nonexistent:**

| `AGENTS.md` claims | Reality |
|---|---|
| `src-tauri/src/project_sync/domain.rs` owns IDs, schemas, manifests, plans | **Does not exist.** Those types live in `crates/mallard-domain/src/lib.rs` (5,711 lines) |
| `chat_history.rs` parses Codex history | **Does not exist.** The work is in `codex_history_graph.rs`, `providers/*/history.rs`, `session_path_map.rs` |
| `provider_capture.rs`, `bundle_engine.rs`, `persistence.rs` under `src-tauri/src/` | **All three moved** to `crates/mallard-infrastructure/src/` |
| `activity_log.rs` owns sanitized JSONL history and retention | `src-tauri/src/activity_log.rs` exists but is a **171-line** Tauri emission wrapper; the retention logic described is `crates/mallard-infrastructure/src/activity_log.rs` (1,235 lines) |

**Never mentioned:** the entire five-crate workspace — `mallard-domain` (9,129
lines), `mallard-application` (3,047), `mallard-cli` (~2,900), and 31
infrastructure modules including `project_activity.rs` (2,710),
`session_detail.rs` (2,628), `codex_paginated_projection.rs` (2,326),
`credential_redaction.rs` (2,259), `conversation_materialization.rs` (1,941),
and `providers/` (10,667).

**Stale commands:** `AGENTS.md` says `test:backend-integration` runs "the Rust
library test suite"; `package.json:16` actually runs
`cargo test --workspace --all-targets`. `test:cli`,
`test:storage-catalog-e2e`, `test:aimock-servers`, and four live e2e runners are
unmentioned. `cargo check --manifest-path src-tauri/Cargo.toml` checks only
`default-members`, not the workspace.

### A3 — Medium: blanket dead-code suppression across all five top-level modules

```rust
#[cfg_attr(test, allow(dead_code))]
mod activity_log;      // lib.rs:8 — and likewise :10, :12, :14, :16
```

plus `project_sync/mod.rs:9`. The justifying comment (`lib.rs:1-4`) is
legitimate — `#[tauri::command]` wrappers are reachable only through
`generate_handler!` inside a `cfg(not(test))` runtime, so the lib-test target
would otherwise flag them. But the suppression is applied **wholesale** to
`project_sync`, `storage_catalog`, `activity_log`, `app_reset`, and
`app_settings`, not just to the command modules.

Since `cargo test` is the development loop, dead code accumulates unlinted
crate-wide. The prior
[over-engineering audit](./BACKEND_OVERENGINEERING_AUDIT_2026-08-21.md) found 19
registered commands with no possible caller; this blanket allow is why the lint
cannot catch a regression of that class.

**Fix.** Scope the attribute to `project_sync::commands` and
`storage_catalog::resource_commands` only.

### A4 — Medium: the modules that write to disk have no unit tests

| Module | Lines | Inline tests | Risk |
|---|---|---|---|
| `crates/mallard-infrastructure/src/conversation_materialization.rs` | 1,941 | **0** | Writes conversation data into provider homes. Exercised only via `sync_tests/thread_variants/{continuation.rs:392,400,743, recovery.rs:20}` — `cargo test -p mallard-infrastructure` covers none of it directly |
| `src-tauri/src/project_sync/commands.rs` | 1,596 | **0** | The entire Tauri command surface — argument decoding, error mapping, event emission — is e2e-only |
| `src-tauri/src/storage_catalog/apply.rs` | 1,425 | **0** | Journaled mutation execution, covered by exactly **one** e2e test (`tests/storage_catalog_e2e.rs`) |

`apply.rs` is where D3 lives: an untested recovery path performing
`fs::rename` and four `remove_dir_all` calls on unvalidated persisted strings.
That combination — destructive, recovery-only, untested — is the highest-value
place in the backend to add tests.

This contradicts the `AGENTS.md` rule "Put Rust unit tests beside the protected
helper." That rule is otherwise well followed: 63 files carry inline
`#[cfg(test)]`, and the workspace has 774 passing tests. One minor deviation:
`mallard-domain` places all 55 tests in a separate 3,418-line `tests.rs` wired
at `lib.rs:5710`, so locating coverage inside a 5,711-line `lib.rs` becomes a
search.

### A5 — Low: undocumented coupling and a second responsibility

- `project_sync` → `storage_catalog` at `service.rs:427,1668,2018,2034`.
  One-directional and documented at `storage_catalog/mod.rs:33`, but absent from
  `AGENTS.md`, which describes the two subsystems as independent.
- `provider_capture.rs` holds 7 `pub(crate)` shared filesystem helpers
  (`canonical_existing_dir`:2143, `normalized_relative`:2159,
  `provider_walk_entry`:2231, `collect_regular_tree_files`:2012,
  `inspect_candidate_files`:1885, `session_home_is_project_root`:1782) imported
  by *both* provider adapters. That is a second responsibility beyond the
  "discovery, filters, stable resource identity, and capture" charter
  `AGENTS.md` gives it; these belong in a shared `fs_safety` module, which would
  also give D12 and D14 one canonical home.

### A6 — Informational: a live API key sits in the working tree

`.env.local` (122 bytes, mode `0600`) contains a JSON object with a live
third-party API key for `api.tu-zi.com` and a channel type of
`newapi_channel_conn`. **Value withheld from this report.**

It is correctly excluded from version control — matched by `*.local` at
`.gitignore:15` and confirmed untracked (`git ls-files --error-unmatch` fails) —
so nothing has been committed. It is flagged because a rotating credential
living unmanaged in a repository working tree tends to get copied into fixtures,
shell history, or screenshots. **Recommend rotating the key** and moving it to a
location outside the repository.

---

## Refuted and corrected claims

Recorded so they are not re-raised by a later audit.

**R1 — "The S3 SigV4 signature is hand-rolled with `hmac` + `sha2`."**
**Refuted.** The client is the official AWS SDK:
`use aws_sdk_s3::{Client as S3Client, Config as S3Config}` and
`aws_smithy_http_client` (`s3_store.rs:11-15`), with `aws-sdk-s3 = "1"` and
`aws-smithy-http-client` (feature `rustls-aws-lc`) in
`crates/mallard-infrastructure/Cargo.toml`. A workspace-wide grep for
`SigV4`, `CanonicalRequest`, `string_to_sign`, `x-amz-content-sha256`,
`UNSIGNED-PAYLOAD`, and `signing_key` returns **zero hits**. Canonical-request
construction, header folding, RFC 3986 encoding, and clock-skew handling are all
SDK-owned and out of scope of this repository. The only `hmac` uses are
`derive_origin_token` (`mallard-domain/src/lib.rs:327`,
`storage_catalog/domain.rs:965`) — an HMAC-SHA256 pseudonym keyed by the
machine-local `host_id`, used correctly. What the repo adds is defense in depth:
every PUT sets `checksum_sha256` (`s3_store.rs:674`), making an acknowledged PUT
a content-verified receipt.

**R2 — "The provider registry is speculative abstraction."** **Refuted.**
`registry.rs` is 86 lines. `production()` is a `OnceLock` over two hardcoded
adapters (`:30-36`); `with_test_provider()` (`:40-47`) and `test_provider.rs`
(229 lines) are gated behind `#[cfg(any(test, feature = "test-support"))]` and
cost nothing in production. The module doc (`:1-5`) explicitly disclaims being a
plugin system. The payoff is measurable: shared code never branches on the
provider key — the only production `ProviderKey::parse("codex")` is
`provider_capture.rs:928`. With 10,667 lines of provider code behind the trait,
the indirection earns its keep. Caveat worth noting: `SessionAdapter` is a wide
19-method trait (`providers/traits.rs:519-607`) whose defaults exist to
accommodate two differing implementations.

**R3 — "The codex/claude providers and the local/s3 stores are heavily
duplicated."** **Refuted.** `discovery.rs` 129 vs 259 lines with ~85% distinct;
`materialization.rs` 125 vs 104 with ~92% distinct; `actions.rs` 398 vs 244 with
~76% distinct. The formats genuinely differ (Codex JSONL rollout vs Claude
chain-resumed records), and shared logic is already extracted into the 7
`pub(crate)` helpers noted in A5. `local_store.rs` and `s3_store.rs` share
**only the 6 trait-required method names and zero private helpers**; their
bodies are necessarily protocol-specific (flock + fsync + atomic rename +
no-follow versus async SDK + etag CAS + ambiguous-error resolution). The shared
contract correctly lives in `object_store.rs` (306 lines).

**R4 — "`mallard-cli` is an orphan crate."** **Refuted.** Built at
`scripts/tauri-build.mjs:101`, bundled as an `externalBin` (`:138`), resolved at
`service.rs:1565-1574`, and exposed to the frontend via `get_mallard_cli_path`
(`commands.rs:255`, `lib.rs:90`). Verified by `verify-macos-bundle.mjs:31,52,65`.
29 tests plus `tests/cli.rs`.

**R5 — "`service.rs` is mostly tests, so its size is forgivable."**
**Refuted.** 56.6% production code (16,077 of 28,400 lines). See A1.

**R6 — "19 of 69 registered Tauri commands have no possible caller"** (from the
2026-08-21 audit). **No longer true.** All **87** commands registered in
`generate_handler!` (`lib.rs:79-167`) have a matching string literal in `src/`.
Every name is snake_case per the `AGENTS.md` contract. Adjacent pairs that look
duplicative are distinct by design: `save_project_link` (create/pin) versus
`connect_project_to_remote_bundle` (carries `expected_bundle_id` for CAS);
`reset_project_sync_metadata` (per-project) versus `clear_storage_data`
(per-storage). This is a real improvement over the prior baseline.

**R7 — "`notify` and `hmac` are declared but never referenced"** (from the
2026-08-21 audit). **No longer true** for `notify`, which is now used by
`config_watcher.rs` and `session_watch_service.rs`. `hmac` is used by
`derive_origin_token` (see R1).

---

## Verification

Commands run against the baseline, both exiting 0:

```
cargo clippy --workspace --all-targets     # 0 warnings
cargo test   --workspace --all-targets     # 774 passed, 0 failed, 8 ignored
```

Per-target test counts: `tauri_app_lib` 259, `mallard_infrastructure` 376 (3
ignored), `mallard_domain` 55, `mallard_application` 37, `mallard_cli` 19 + 10,
`storage_catalog_e2e` 1, `live_codex_e2e` 8 (2 ignored),
`live_codex_aimock_e2e` 7 (1 ignored), `live_claude_aimock_e2e` 2 (2 ignored).

The 8 ignored tests are the live-provider e2e cases, which require credentials
and external services; they are correctly gated rather than silently skipped.

**Not verified.** No finding was confirmed by runtime exploitation — this was a
static audit plus the test suite. `project_sync/service.rs` was audited by
targeted grep for join/delete/read/canonicalize patterns rather than read in
full, so its coverage is sampling-based; no unvalidated join-and-write was found
there, but absence of evidence in a 28,400-line file is weaker than in the
modules read end to end. The frontend integration suite
(`npm run test:frontend-integration`) and `npm run build` were not run, as the
scope was the backend.

---

## Suggested order of work

1. **A2 — correct the `AGENTS.md` source map.** Cheapest item on this list and
   the highest leverage: it is loaded into every agent session and currently
   names five files that do not exist.
2. **D3 + A4 — validate the catalog journal's path fields and add tests for
   `apply.rs` recovery.** The only destructive, untested, recovery-only path in
   the backend.
3. **D1 — move endpoint validation into `S3ObjectStore::from_config`.** One
   function, closes the transport-boundary gap for every caller at once.
4. **D4, D5, D7 — the remaining one-line-to-one-function validations.** Each is
   small, local, and restores symmetry with the project-sync path that already
   does it correctly.
5. **A1 — begin splitting `service.rs`,** starting by replacing the
   `use super::service::*` glob in `commands.rs:3` with explicit imports.
6. **A3 — narrow the `allow(dead_code)` scope** so the lint can catch regressions
   again.
7. **A6 — rotate the key in `.env.local`** and move it out of the working tree.

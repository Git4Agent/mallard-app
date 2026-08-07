# Plan: Agent-Assisted Cloudflare R2 Setup

- **Status:** Proposed
- **Date:** August 6, 2026
- **Product scope:** User-owned Cloudflare R2 storage
- **Implementation baseline:** Mallard 0.1.2, machine-local schema 1, portable
  bundle schema 1
- **Relevant code baseline:** `a5d41c5` and `6f39737`

## 1. Outcome

Mallard should let a user delegate Cloudflare R2 setup to a terminal coding
agent without asking that agent to edit Mallard metadata or expose
credentials in a prompt.

The completed experience is:

1. The user opens **New storage → Cloudflare R2**.
2. The user chooses **Set up with an agent**.
3. Mallard copies an instruction containing the exact packaged CLI path and
   non-secret setup intent.
4. The agent runs `mallard-cli`.
5. The CLI either connects an existing R2 bucket or provisions a new one.
6. The CLI verifies read, write, list, and conditional-write behavior.
7. Mallard adds the storage through its normal revisioned, atomic persistence
   layer.
8. An already-running app shows the new storage without a restart.

The agent orchestrates the task. Mallard remains the authority for validation,
secret handling, connectivity checks, and local persistence.

## 2. Product boundaries

This plan covers Cloudflare R2 owned by the user. It does not implement the
Mallard-operated managed cloud described in
[CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE_V2.md](CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE_V2.md).

The first release targets packaged macOS builds. Windows follows after the
command, storage-service, and event contracts are stable.

This plan does not:

- provision generic AWS S3 or arbitrary S3-compatible services;
- install Wrangler, npm, or other global tooling;
- edit shell startup files or PATH automatically;
- let an agent hand-edit `~/.mallard/sync_config.json`;
- store a Cloudflare bootstrap token;
- use expiring R2 temporary credentials as Mallard's persistent storage
  credential;
- delete a bucket automatically during rollback; or
- change the portable bundle schema.

## 3. Experience design

### 3.1 Connect an existing bucket

This is the minimum shippable path. The user already has:

- an R2 S3 API URL;
- an Access Key ID; and
- a Secret Access Key.

Mallard copies an instruction similar to:

```text
Set up the Cloudflare R2 storage “Team backup” in Mallard.

Use this Mallard CLI:
<absolute packaged CLI path>

Run its interactive R2 connection flow. Do not edit Mallard JSON, place a
secret in a command argument, or print credentials. When the CLI asks for the
Secret Access Key, let me enter it through its hidden prompt. Finish by
verifying the storage and confirming it appears in the running app.
```

The CLI command is:

```text
mallard-cli storage r2 connect \
  --name "Team backup" \
  --api-url "https://<account>.r2.cloudflarestorage.com/<bucket>" \
  --access-key-id "<id>"
```

The secret is read from a hidden terminal prompt. Non-interactive callers may
use `--secret-stdin`; there is no secret-valued command-line flag.

If an agent cannot give the user a private interactive prompt, it must hand
the final command to the user instead of requesting the secret in chat.

### 3.2 Provision a new bucket

The follow-up path provisions R2 through Cloudflare's REST API:

```text
mallard-cli storage r2 provision \
  --name "Team backup" \
  --account-id "<account id>" \
  --bucket "team-backup" \
  --jurisdiction default
```

The CLI requests a Cloudflare **Create additional tokens** bootstrap token
through a hidden prompt. It does not use Wrangler authentication state.

Provisioning:

1. verifies the Cloudflare account and bootstrap token;
2. creates or confirms the bucket;
3. creates an R2 token scoped to that bucket;
4. derives the token's S3 credentials;
5. verifies the S3 destination;
6. saves the derived R2 credentials in Mallard; and
7. revokes every temporary provisioning token.

The user performs Cloudflare's bootstrap-token creation once. Each later
storage setup can then complete without copying an R2 Secret Access Key from
the dashboard.

### 3.3 Completion feedback

Human output:

```text
Connected “Team backup”.
Storage ID: storage-…
Mallard config revision: 12
Remote capability probe: passed
```

Machine output with `--json`:

```json
{
  "schema": 1,
  "status": "connected",
  "storage_id": "storage-...",
  "config_revision": 12,
  "verified": true
}
```

Output must not contain the bucket, account ID, endpoint, Access Key ID,
Secret Access Key, Cloudflare token ID, bootstrap token, or a local path.

## 4. Current Mallard contracts

Implementation must use the active ProjectSync path:

- `src-tauri/src/project_sync/`
- `src/components/project-sync/`
- `src/types.ts`

The local document is `~/.mallard/sync_config.json` with schema 1:

```text
SyncConfig
├── schema
├── revision
├── storages
├── projects
└── links
```

Relevant current behavior:

- `LocalStateRepository` performs bounded, atomic JSON persistence.
- `persistence.lock` already coordinates separate Mallard processes.
- `save_config` enforces optimistic document revision checks.
- `mutate_config` performs a locked read-modify-write.
- the Tauri save path validates bindings and storage isolation before writing;
- the frontend rebases storage-only saves on the latest config snapshot; and
- `S3BundleObjectStore` is the production R2 client path.

No replacement schema, compatibility file, or second persistence system is
needed.

## 5. Architecture

### 5.1 Components

Add:

```text
src-tauri/src/bin/mallard-cli.rs
src-tauri/src/cli.rs
src-tauri/src/project_sync/storage_setup.rs
src-tauri/src/project_sync/config_watcher.rs
```

Add `cloudflare_r2_setup.rs` only when provisioning work starts.

Responsibilities:

| Component | Responsibility |
| --- | --- |
| `mallard-cli.rs` | Process entry point and exit status |
| `cli.rs` | Argument parsing, hidden input, redacted output |
| `storage_setup.rs` | R2 URL parsing, probe, de-duplication, config mutation |
| `config_watcher.rs` | Detect newer config revisions and notify the frontend |
| `cloudflare_r2_setup.rs` | Cloudflare API provisioning and recovery |

`project_sync` remains private. `lib.rs` exposes only a narrow `pub mod cli`
entry point required by the second binary.

The CLI must not instantiate a parallel DTO or persistence model. It calls
the same crate-private storage service as the Tauri command.

### 5.2 Shared R2 setup service

The service accepts a typed request and returns a sanitized result:

```rust
struct ConnectR2StorageRequest {
    name: String,
    api_url: String,
    access_key_id: String,
    secret_access_key: SecretString,
    expected_config_revision: Option<u64>,
}

struct ConnectStorageResult {
    storage_id: String,
    config_revision: u64,
    verified: bool,
}
```

Secret-bearing request types must not implement `Debug` or `Serialize`.

The service:

1. parses and canonicalizes the R2 API URL;
2. constructs a pending `StorageConfig`;
3. validates the pending storage;
4. checks for an existing canonical destination;
5. runs the full remote capability probe;
6. mutates the config under `persistence.lock`;
7. revalidates the complete local document;
8. performs one atomic write and revision increment; and
9. returns the exact committed revision.

Add a repository mutation method that returns the saved `SyncConfig` while
the persistence lock is still held. Loading the document again after
`mutate_config` returns could observe another process's later revision.

### 5.3 Tauri command

Add a focused command:

```text
connect_r2_storage
```

The storage page uses this command when creating R2 storage. Local-folder
creation and existing editing can keep the current full-document save until a
separate refactor is justified.

This gives the desktop app and CLI identical behavior:

- the same URL parser;
- the same credential normalization;
- the same remote probe;
- the same duplicate check;
- the same atomic config mutation; and
- the same sanitized error mapping.

Register the command in `src-tauri/src/lib.rs` and expose it only through
`projectSyncApi`.

### 5.4 Endpoint model

Cloudflare documents three R2 S3 endpoint forms:

```text
https://<ACCOUNT_ID>.r2.cloudflarestorage.com
https://<ACCOUNT_ID>.eu.r2.cloudflarestorage.com
https://<ACCOUNT_ID>.fedramp.r2.cloudflarestorage.com
```

Mallard's setup URL adds one bucket path segment:

```text
<endpoint>/<bucket>
```

The parser must reject:

- non-HTTPS URLs;
- embedded username or password;
- ports, query strings, or fragments;
- account IDs that are not exactly 32 hexadecimal characters;
- missing or multiple bucket path segments;
- bucket names outside Cloudflare's lowercase letter, number, hyphen, and
  length rules;
- public/custom R2 domains; and
- endpoint/account/jurisdiction disagreement.

Canonical destination identity is:

```text
(jurisdiction, lowercase account ID, exact bucket)
```

Before agent setup supports EU or FedRAMP, update `StorageSettings.tsx` to
derive its displayed API URL from the stored `s3_endpoint`. The current helper
reconstructs the default endpoint from `account_id`, which would otherwise
silently change jurisdiction during editing.

Backend parsing is authoritative. Frontend helpers mirror it for immediate
feedback and share the same test vectors.

### 5.5 Remote capability probe

A successful client construction or bucket metadata request is insufficient.
Mallard depends on conditional writes for safe mutable heads.

The setup service uses the existing R2 client construction and a unique key
under:

```text
.mallard/v1/_setup_probes/<random setup id>
```

It must:

1. perform a bounded list;
2. create a small object with an absent precondition;
3. read it back and verify its size and SHA-256;
4. update it with the current ETag;
5. confirm that the stale ETag is rejected;
6. delete the exact probe object; and
7. confirm absence.

All calls use production timeouts and bounded responses.

The local config is not changed unless the probe and cleanup both succeed.
If cleanup fails, return a sanitized setup ID and cleanup-required error. Do
not print the remote key or silently leave setup marked successful.

Record that setup ID and probe stage in the same bounded, non-secret setup
journal used by provisioning. A retry with the same destination must reconcile
the known probe key before creating another one.

### 5.6 Live config refresh

Use the existing `notify` dependency.

The app watches the `~/.mallard` directory rather than the file itself because
atomic persistence replaces `sync_config.json` by rename.

On a relevant create, modify, or rename event:

1. debounce the event burst;
2. load the config under the normal persistence lock;
3. validate it;
4. compare its revision with the last emitted revision; and
5. emit `project-sync-config-changed` with `{ revision }`.

Never emit the config document; it contains credentials.

`ProjectSync.tsx` listens for the event and fetches the config only when the
event revision is newer than the React snapshot.

The storage workspace must report whether its draft differs from its baseline:

- a clean draft adopts the new storage snapshot;
- a dirty draft stays unchanged and shows an external-change warning;
- saving a dirty draft over an externally modified or removed storage is
  blocked until the user reloads or explicitly reviews the conflict.

An in-app save also produces a filesystem event. The revision comparison makes
that event a no-op.

### 5.7 CLI packaging

Add `mallard-cli` as a second Rust binary. The repository's Tauri build wrapper
must build it for the same target triple and release profile as the app, then
place it at a deterministic generated bundle path.

Do not commit compiled binaries.

The storage page resolves the executable's packaged path and places it in the
copied agent instruction. A bare command name is not the primary contract
because `~/.local/bin` is not guaranteed to be on macOS PATH.

An optional later action may install a copy at:

```text
~/.local/bin/mallard-cli
```

That action must use an atomic replacement, refuse to overwrite an unrelated
file, show the final absolute path, and avoid editing shell configuration.

## 6. Cloudflare provisioning

### 6.1 Credential derivation

Cloudflare documents the mapping from an API-created R2 token to S3
credentials:

```text
Access Key ID     = token response id
Secret Access Key = lowercase hex SHA-256(token response value)
```

Mallard persists only the derived S3 secret. It must not persist the raw token
value, which is also a Cloudflare bearer credential.

Permission-group IDs are resolved from Cloudflare's permission-groups API.
Do not hard-code IDs or rely on display names as identifiers.

The final token policy grants:

```text
Workers R2 Storage Bucket Item Write
```

to one exact bucket resource and jurisdiction.

### 6.2 Provisioning state machine

1. Validate account, bucket, jurisdiction, and local destination uniqueness.
2. Verify the bootstrap token and account.
3. Determine whether the bucket exists.
4. If absent:
   1. create a short-lived token with account-scoped
      `Workers R2 Storage Write`;
   2. create the bucket through the R2 REST API; and
   3. revoke that token immediately.
5. Create the final bucket-scoped R2 token.
6. Derive its S3 credentials in memory.
7. Run the remote capability probe.
8. Commit the new `StorageConfig`.
9. revoke any remaining temporary token;
10. zeroize raw and derived transient secrets; and
11. return the sanitized result.

The bootstrap token should carry only token-creation permission. Cloudflare
recommends a short lifetime and IP restrictions when practical.

### 6.3 Partial failure

Cloudflare and local persistence cannot share a transaction. Maintain a
bounded local journal under:

```text
~/.mallard/r2_setup_transactions/
```

The journal may contain:

- setup ID;
- account ID, bucket, and jurisdiction;
- current stage;
- created token IDs;
- whether this flow created the bucket;
- timestamps; and
- a sanitized error code.

It must not contain:

- bootstrap or R2 token values;
- S3 secrets;
- authorization headers;
- raw HTTP bodies;
- signed requests; or
- local absolute paths.

Recovery rules:

- revoke a final token if verification or local persistence fails;
- retain the journal if revocation cannot be confirmed;
- let a retry reconcile known token IDs before continuing;
- report a cleanup-required setup ID without exposing credentials;
- never delete a bucket automatically; and
- remove the journal only after the config commit and token cleanup are both
  confirmed.

## 7. Security requirements

These are release gates:

- No secret-valued command argument.
- No secret in a copied agent instruction.
- No secret in stdout, stderr, JSON, logs, journals, panics, or tests.
- No raw Cloudflare response body in a surfaced error.
- No manual JSON mutation by the CLI or agent.
- No unbounded Cloudflare or S3 response.
- No config commit before capability verification and probe cleanup.
- No duplicate canonical R2 destination.
- No change to projects or links while adding storage.
- No config event containing credential-bearing fields.
- No automatic bucket deletion.
- Bootstrap tokens and raw token values use zeroizing in-memory wrappers.
- The stored secret is the derived R2 S3 secret, not the raw Cloudflare token.

Mallard currently stores R2 S3 credentials in its machine-local config. This
feature preserves that behavior but must not broaden it. A future keychain
migration is separate work.

If an explicitly saved existing configuration contains a raw `cfat_...` token
value, the save path may verify and replace it with the derived SHA-256 S3
secret. Do not perform that rewrite silently during startup.

## 8. Delivery plan

### Phase 1 — Storage setup foundation

Implement:

- backend R2 API URL parsing and canonicalization;
- default, EU, and FedRAMP endpoint preservation in the UI;
- shared setup request/result DTOs;
- exact committed-config return from a locked mutation;
- canonical destination de-duplication; and
- the S3 capability probe.

Acceptance:

- current manual R2 creation still works;
- every supported endpoint round-trips without jurisdiction drift;
- a failed or partially cleaned probe never changes the local config;
- two concurrent adds cannot create duplicate destinations; and
- tests use temporary homes and stub storage only.

### Phase 2 — Focused Tauri create command

Implement:

- `connect_r2_storage`;
- `projectSyncApi.connectR2Storage`;
- R2 creation through the shared service; and
- current storage-page error and completion feedback.

Acceptance:

- UI and service errors use actionable R2 language;
- credentials are absent from errors and activity logs;
- the config revision increments once;
- projects and links are byte-for-byte unchanged; and
- local-folder behavior is unchanged.

### Phase 3 — Existing-credential CLI

Implement:

- `mallard-cli storage list`;
- `mallard-cli storage r2 connect`;
- `mallard-cli storage verify`;
- hidden and stdin secret input;
- redacted human and schema-1 JSON output; and
- stable exit statuses.

Recommended exit statuses:

| Status | Meaning |
| --- | --- |
| `0` | Success or verified idempotent result |
| `2` | Invalid input |
| `3` | Config revision or destination conflict |
| `4` | Authentication or remote capability failure |
| `5` | Local persistence failure |
| `6` | Remote cleanup required |

Acceptance:

- a subprocess using a temporary home adds one valid storage;
- simultaneous UI-style and CLI-style mutations preserve both changes;
- the supplied secret canary appears nowhere in captured output or files;
- forced termination exposes no partial config; and
- the CLI never reads or writes a caller-selected config path.

### Phase 4 — Live refresh and agent entry point

Implement:

- config watcher lifecycle;
- revision-only Tauri event;
- frontend reload and dirty-draft conflict behavior;
- **Set up with an agent**;
- copied instruction generation; and
- packaged CLI path resolution.

Acceptance:

- an external config commit appears in the app within one second;
- self-generated file events produce no duplicate notice;
- a dirty draft is never overwritten or blindly rebased;
- copied text contains the resolved CLI path and no secret; and
- a malformed external config does not replace the active UI snapshot.

### Phase 5 — macOS packaging

Update:

- `src-tauri/Cargo.toml`;
- `scripts/tauri-build.mjs`;
- Tauri bundle configuration; and
- release-config tests.

Acceptance:

- the CLI is present in the Apple Silicon DMG;
- it runs from the mounted image and from a temporary installed location;
- the app and nested CLI signatures verify;
- release validation fails when the CLI build artifact is absent; and
- the exact target triple used for the app is also used for the CLI.

### Phase 6 — Automated Cloudflare provisioning

Implement:

- Cloudflare REST client with TLS and JSON only;
- permission-group resolution;
- bucket creation;
- short-lived provisioning-token lifecycle;
- final bucket-scoped token creation;
- S3 credential derivation;
- recovery journal and cleanup command; and
- failure-injection tests.

Acceptance:

- the bucket-existing and bucket-absent flows both complete;
- the final policy references exactly one bucket;
- temporary tokens are revoked on success;
- every injected failure either cleans up or leaves a non-secret recovery
  journal;
- no raw token value reaches output or disk; and
- a credential-gated disposable R2 smoke test passes.

### Phase 7 — Windows

Implement and verify:

- `mallard-cli.exe` packaging;
- hidden console input;
- cross-process persistence locking;
- atomic config replacement events;
- absolute-path agent instructions; and
- optional per-user installation without default PATH mutation.

The config, CLI JSON, setup-service, and Cloudflare contracts remain
cross-platform.

## 9. Test plan

### Rust unit and integration tests

- Valid default, EU, and FedRAMP URLs.
- Invalid scheme, host, account, path, port, query, fragment, and credentials.
- New destination, idempotent retry, duplicate conflict, and revision
  conflict.
- Config collection bounds and malformed current documents.
- Exact committed revision under concurrent writers.
- Every capability-probe step and cleanup failure.
- Secret-canary redaction across output, logs, journals, and error paths.
- Cloudflare policy construction and credential derivation test vectors.
- Every provisioning partial-failure boundary.

### Frontend integration tests

- Endpoint helpers preserve jurisdiction.
- Current R2 field layout and progress count remain correct.
- Focused create command payloads match Rust DTOs.
- Watcher reload updates clean state.
- Watcher reload preserves dirty state and blocks overwrite.
- Copied instructions contain the provided executable path.
- Copied instructions contain no credential values.

### Release checks

- `npm run build`
- `npm run test:frontend-integration`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- CLI subprocess integration tests
- `npm run test:release-config`
- `MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build`

A live R2 smoke test must use a disposable bucket/account, keep credentials out
of CI logs, revoke generated tokens, and never point at a personal production
bucket.

## 10. Definition of done

- A packaged macOS user can delegate R2 setup to a terminal agent from one
  copied instruction.
- The agent never edits Mallard JSON.
- Secret values never enter the prompt, process arguments, output, logs, or
  setup journal.
- The UI and CLI call one schema-1 R2 setup service.
- Mallard verifies list, read, write, conditional-write, stale-CAS rejection,
  and cleanup before saving.
- The config mutation uses the existing cross-process lock and atomic writer.
- The running app shows the new storage within one second.
- A dirty storage draft cannot be overwritten by an external change.
- Default, EU, and FedRAMP endpoints round-trip without drift.
- Automated provisioning creates a bucket-scoped final token and reconciles
  every partial-failure state.
- The packaged CLI is present, runnable, and signature-verified in the DMG.
- Existing local-folder and R2 sync tests remain green.

## 11. References

Mallard:

- [`domain.rs`](../src-tauri/src/project_sync/domain.rs)
- [`persistence.rs`](../src-tauri/src/project_sync/persistence.rs)
- [`commands.rs`](../src-tauri/src/project_sync/commands.rs)
- [`s3_store.rs`](../src-tauri/src/project_sync/s3_store.rs)
- [`StorageSettings.tsx`](../src/components/project-sync/StorageSettings.tsx)
- [`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx)

Cloudflare:

- [R2 authentication and S3 credential derivation](https://developers.cloudflare.com/r2/api/tokens/)
- [Create API tokens through the API](https://developers.cloudflare.com/fundamentals/api/how-to/create-via-api/)
- [Create an account-owned token](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/methods/create/)
- [Delete an account-owned token](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/methods/delete/)
- [Create an R2 bucket](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/create/)
- [R2 S3 endpoint setup](https://developers.cloudflare.com/r2/get-started/s3/)
- [R2 temporary credentials](https://developers.cloudflare.com/r2/api/s3/temporary-credentials/)

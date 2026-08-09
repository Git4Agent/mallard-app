# Plan: Agent-Assisted Cloudflare R2 Setup

- **Status:** Implemented; macOS DMG verified locally; Windows and live-cloud validation remain
- **Last updated:** August 7, 2026
- **Product scope:** User-owned Cloudflare R2 storage
- **Platforms:** Apple Silicon macOS and x64 Windows
- **State contract:** Clean schema-1 cutover; no migration or compatibility path

## 1. Decision

Mallard ships its command-line interface inside the desktop installer and uses
that CLI as the safe boundary between a terminal coding agent and Mallard's
storage setup service.

The user can either connect an existing R2 bucket or let the CLI provision a
new bucket and exact-bucket credential. The agent may coordinate the flow, but
Mallard remains responsible for input validation, hidden secret input, remote
capability checks, cleanup, concurrency control, and atomic local persistence.

The implemented experience is:

1. The user opens **New storage → Cloudflare R2**.
2. The user selects **Set up with an agent**.
3. Mallard copies an instruction containing only the storage name and the
   exact packaged CLI path.
4. The agent inspects the CLI's R2 help and chooses connect or provision.
5. The user enters the secret through a hidden terminal prompt. A bounded
   stdin mode exists for automation; secrets are never accepted through an
   argument or environment variable.
6. Mallard verifies the destination's list, read, write, compare-and-swap,
   stale-CAS rejection, delete, and absence behavior.
7. The shared application service commits the storage through the existing
   schema-1 repository lock and atomic writer.
8. The open R2 setup page notices the new config revision, stays open, and
   fills its fields from the exact committed storage without a restart.

## 2. Clean-cutover policy

This implementation deliberately has no migration or backward-compatibility
branch. Before using it, development and test environments may clear their
old `~/.mallard` state and old cloud objects as agreed for this cutover.

The active rules are:

- every `s3` storage is a canonical Cloudflare R2 destination;
- generic AWS S3 and arbitrary S3-compatible endpoints are rejected;
- a complete config cannot contain the same jurisdiction, account, and bucket
  more than once;
- direct connection treats the supplied value as an R2 S3 Secret Access Key;
- Mallard never interprets or rewrites a supplied `cfat_...` bearer token;
- provisioning persists the documented SHA-256-derived S3 secret, not the raw
  Cloudflare token value;
- schema 1 remains the only local and portable schema; and
- projects, links, and bundle formats are unchanged.

The feature does not:

- edit Mallard JSON by hand;
- install Wrangler, npm, or other global tools;
- edit shell startup files or PATH;
- store a Cloudflare bootstrap token;
- delete a bucket during rollback;
- add an updater or signing system; or
- implement Mallard-operated managed cloud storage.

## 3. Supported CLI surface

The packaged executable is named `mallard-cli` on macOS and
`mallard-cli.exe` on Windows.

### 3.1 Inspect and verify storage

```text
mallard-cli storage list
mallard-cli storage verify <storage-id>
```

### 3.2 Connect an existing bucket

```text
mallard-cli storage r2 connect \
  --name "Team backup" \
  --api-url "https://<account>.r2.cloudflarestorage.com/<bucket>" \
  --access-key-id "<id>"
```

The Secret Access Key is read from a hidden prompt. Automation may add
`--secret-stdin` and provide exactly one bounded line on standard input.

### 3.3 Provision a bucket and credential

```text
mallard-cli storage r2 provision \
  --name "Team backup" \
  --account-id "<32-hex-account-id>" \
  --bucket "team-backup" \
  --jurisdiction default
```

The Cloudflare bootstrap token is read from a hidden prompt. Automation may
add `--bootstrap-token-stdin`.

The bootstrap token must be created from Cloudflare's **Create additional
tokens** template. It is used only against account-owned token endpoints and
is never persisted.

### 3.4 Explicit recovery

```text
mallard-cli storage r2 cleanup <setup-id>
```

Cleanup also reads the bootstrap token through a hidden prompt or bounded
`--bootstrap-token-stdin`. Repeating cleanup after successful reconciliation
is safe.

### 3.5 Output and exit contract

Human output reports only display names, Mallard storage/setup IDs, config
revision, and sanitized status. `--json` emits schema-1 objects with a stable
`status`; failures also include a stable `code`, numeric `exit_code`, and an
optional sanitized `setup_id`.

Output never contains the bucket, account, endpoint, Access Key ID, Secret
Access Key, token value, Cloudflare token ID, authorization header, or a local
filesystem path.

| Exit | Meaning |
| --- | --- |
| `0` | Success or verified idempotent result |
| `2` | Invalid input |
| `3` | Config revision, destination, or operation conflict |
| `4` | Authentication or remote capability failure |
| `5` | Local persistence failure |
| `6` | Explicit remote cleanup required |

## 4. Implemented architecture

The implementation follows the workspace architecture instead of adding a
second binary under `src-tauri`:

```text
mallard-app/
├── Cargo.toml
├── crates/
│   ├── mallard-domain/
│   │   └── schema-1 storage and config domain types
│   ├── mallard-application/
│   │   └── src/
│   │       ├── storage_setup.rs
│   │       └── cloudflare_r2_setup.rs
│   ├── mallard-infrastructure/
│   │   └── src/
│   │       ├── persistence.rs
│   │       ├── r2_setup_journal.rs
│   │       └── storage_core/
│   │           └── s3_store.rs
│   └── mallard-cli/
│       ├── src/
│       │   ├── args.rs
│       │   ├── backend.rs
│       │   ├── error.rs
│       │   ├── lib.rs
│       │   ├── main.rs
│       │   └── output.rs
│       └── tests/
├── src-tauri/src/project_sync/
│   ├── commands.rs
│   └── config_watcher.rs
├── src/components/project-sync/
│   ├── ProjectLinksWorkspace.tsx
│   ├── ProjectSync.tsx
│   ├── StorageSettings.tsx
│   └── api.ts
├── scripts/
│   ├── tauri-build.mjs
│   ├── verify-macos-bundle.mjs
│   └── verify-windows-bundle.ps1
└── .github/workflows/release.yml
```

The dependency direction is:

```text
desktop adapter ─┐
                 ├─> mallard-application ─> domain + infrastructure
CLI adapter ─────┘
```

The desktop and CLI therefore share one parser, probe, de-duplication rule,
transaction, provisioning state machine, error model, and recovery journal.

## 5. Storage setup service

`mallard-application/src/storage_setup.rs` owns the use case. Its
secret-bearing requests intentionally do not implement `Debug`, `Clone`, or
`Serialize`. Secret allocations use zeroizing wrappers.

Direct connect performs this sequence:

1. validate the display name and credential bounds;
2. parse and canonicalize the R2 API URL;
3. validate the complete clean-cutover storage document;
4. reconcile a previously journaled probe for the destination;
5. create a journal entry before starting a new probe;
6. execute the exact R2 capability probe;
7. open a locked config transaction;
8. revalidate destination uniqueness and the complete document;
9. add the storage or return the existing exact idempotent result;
10. atomically persist exactly one new revision; and
11. return the exact config committed while the lock was held.

`LocalStateRepository::transact_config` distinguishes changed and unchanged
results. It does not reload after releasing the lock, so a later writer cannot
make the service report the wrong revision.

A process-wide host-operation guard also covers the full connect, verify,
provision, and cleanup operations. This prevents cleanup from racing another
R2 setup while the ordinary persistence lock continues to protect the short
config transaction.

## 6. Canonical R2 destination

Mallard accepts the current Cloudflare endpoint forms:

```text
https://<ACCOUNT_ID>.r2.cloudflarestorage.com/<bucket>
https://<ACCOUNT_ID>.eu.r2.cloudflarestorage.com/<bucket>
https://<ACCOUNT_ID>.fedramp.r2.cloudflarestorage.com/<bucket>
```

The parser requires:

- HTTPS;
- exactly one 32-hex account label;
- exactly one bucket path segment;
- a 3–63 character lowercase bucket using letters, digits, and hyphens;
- a letter or digit at both bucket boundaries; and
- one of the three supported Cloudflare hosts.

It rejects credentials, ports, queries, fragments, percent-encoded bucket
aliases, multiple path segments, trailing slash aliases, public/custom R2
domains, and surrounding whitespace.

Canonical identity is:

```text
(jurisdiction, lowercase account ID, exact bucket)
```

The frontend mirrors these rules for immediate feedback. The backend remains
authoritative, and full-document saves reject noncanonical or duplicate S3
records.

## 7. Exact remote capability probe

Client construction or bucket metadata is not sufficient because Mallard
depends on compare-and-swap writes for mutable heads.

The production S3/R2 adapter owns a dedicated probe under:

```text
.mallard/v1/_setup_probes/<setup-id>
```

The ordinary object-store namespace remains closed; the setup adapter alone
may access this key family.

The probe:

1. performs a bounded list;
2. creates a small object with an absent precondition;
3. reads it and verifies the exact bytes and SHA-256 digest;
4. updates it using the current ETag;
5. proves a stale ETag is rejected;
6. deletes that exact object; and
7. confirms it is absent.

Failure before a clean delete never commits local config. A cleanup failure
returns exit 6 and the sanitized setup ID. The retry/cleanup path reconciles
that exact known probe key; it never performs recursive deletion.

## 8. Cloudflare provisioning state machine

Provisioning uses Cloudflare's account-owned API token endpoints and R2 bucket
API. Permission-group IDs are resolved from Cloudflare at runtime by exact
name and exact scope; IDs are not hard-coded.

The bootstrap token is intentionally not used to inspect or create buckets.
The implemented order is:

1. validate account, bucket, jurisdiction, name, expected revision, and local
   destination uniqueness;
2. reconcile unfinished setup journals for that destination;
3. resolve the account R2-write and bucket-item-write permission groups;
4. create a 15-minute account-scoped `Workers R2 Storage Write` token;
5. use that temporary token to inspect the bucket;
6. create the bucket if absent; if creation races and returns conflict,
   re-read and require the exact requested bucket;
7. revoke the temporary token immediately, regardless of bucket outcome;
8. create a durable `Workers R2 Storage Bucket Item Write` token scoped to
   exactly one account, jurisdiction, and bucket;
9. derive the S3 credentials in memory:

   ```text
   Access Key ID     = created token response ID
   Secret Access Key = lowercase hex SHA-256(created token value)
   ```

10. reconcile prior probes and execute the new capability probe;
11. atomically commit the storage;
12. adopt the final token only after confirming the committed config contains
    its access ID;
13. zeroize transient raw and derived secret allocations; and
14. remove the completed journal.

The Cloudflare client uses rustls, HTTPS-only requests, disabled redirects,
connect and whole-request timeouts, JSON content-type checks, bounded response
bodies, fixed sanitized errors, and zeroized response buffers. Raw Cloudflare
error bodies never cross the application boundary.

## 9. Recovery journal

Provisioning and local persistence cannot form one distributed transaction.
Mallard therefore keeps bounded, atomic, no-follow journal entries below:

```text
~/.mallard/r2_setup_transactions/
```

The journal records only:

- schema and setup ID;
- account, bucket, and jurisdiction needed for exact recovery;
- state and timestamps;
- whether this flow created the bucket;
- separate `temporary_token_id`, `final_token_id`, and `cleanup_token_id`
  roles;
- a sanitized error code; and
- whether exact probe cleanup remains.

It never records a token value, S3 secret, authorization header, HTTP body,
signed request, credential-bearing config, or local absolute path.

Recovery rules are:

- a token known to be adopted by the current config is never revoked;
- an unadopted final token is revoked after probe or persistence failure;
- a temporary account token is revoked immediately;
- exact probe cleanup may use the configured credential or a new 15-minute,
  exact-bucket cleanup token;
- a cleanup token is journaled before use and revoked afterward;
- any unconfirmed revocation preserves a cleanup-required journal;
- a retry reconciles known token roles before creating more credentials;
- the bucket is never automatically deleted; and
- a journal is removed only when probe cleanup and token cleanup are both
  confirmed.

## 10. Desktop integration and concurrency

The Tauri layer is deliberately thin:

- `connect_r2_storage` calls the shared direct-connect service;
- `get_mallard_cli_path` resolves the packaged executable beside the running
  desktop executable;
- path resolution rejects missing, symlinked, non-file, or escaped siblings;
  and
- `save_project_sync_config` invokes the complete R2 destination validator for
  existing-storage edits.

The storage workspace generates an agent instruction from only the normalized
display name and CLI path. It tells the agent to quote the literal path, inspect
R2 help, use connect or provision, and keep secrets out of arguments,
environment variables, prompts, and output.

When the instruction is copied, the UI stores a bounded, credential-free
marker containing the draft identity, redacted change fingerprint, requested
name, and pre-existing storage IDs. If exactly one new same-name R2 storage
appears while the draft stays clean, the editor switches to that committed
storage, fills independent draft and baseline snapshots, remains expanded,
and reports that the settings were filled automatically. A changed or
ambiguous draft is preserved.

Existing edits carry their exact baseline into save. The save is blocked when
that storage was changed or removed externally, or when a new draft's ID now
collides. This closes the time-of-check/time-of-use gap instead of blindly
rebasing a dirty editor.

## 11. Live config refresh

Persistence replaces `sync_config.json` atomically, so the native watcher
observes the parent `~/.mallard` directory.

After a relevant create, modify, rename, or removal burst, it:

1. debounces for 150 ms;
2. loads and validates config through the normal repository lock;
3. ignores malformed, duplicate, or older snapshots;
4. emits `project-sync-config-changed` with only `{ revision }`; and
5. keeps credentials behind the command boundary.

React fetches config only for a newer revision. It preserves a newer valid
event-delivered snapshot when a concurrent shell load finishes late or fails.
A malformed external document never replaces the active UI snapshot.

## 12. Packaging and release

### 12.1 Shared build wrapper

`scripts/tauri-build.mjs` builds `mallard-cli` with `cargo build --locked` for
the same target and debug/release profile as Tauri. It stages the target-named
sidecar expected by Tauri and injects `externalBin` through a final build-only
config overlay.

The base `tauri.conf.json` intentionally has no `externalBin`. Ordinary Cargo
commands therefore do not depend on an uncommitted generated executable, and
compiled sidecars are ignored by Git.

### 12.2 macOS DMG

The Apple Silicon release job builds a DMG. Verification:

- validates the DMG;
- mounts it read-only;
- checks the app and nested CLI signatures;
- runs `mallard-cli --help` from the mounted image;
- copies the complete app with `ditto` to a temporary installed location;
- detaches the DMG; and
- repeats signature and CLI checks from the installed copy.

The current release remains ad-hoc signed and unnotarized; this work does not
claim Apple notarization.

### 12.3 Windows NSIS

The Windows job runs after macOS creates the draft release. It builds an x64
MSVC NSIS installer in per-user mode.

Verification refuses to disturb a pre-existing Mallard installation, installs
to a bounded temporary directory, verifies the registered install location,
proves process/user/machine PATH were not changed, runs
`mallard-cli.exe --help`, uninstalls, and confirms the CLI, install directory,
and uninstall registry key are removed.

Only the verified installer is uploaded to the still-draft release, without
clobbering an existing asset. The installer is currently unsigned; this work
does not claim Windows code signing.

## 13. Security invariants

These remain release gates:

- no secret-valued argument or environment-variable interface;
- no secret in copied text, stdout, stderr, JSON, activity logs, journals,
  panics, or fixtures;
- no manual CLI or agent edit of Mallard metadata;
- no config commit before verified probe cleanup;
- no generic, noncanonical, or duplicate S3 destination;
- no event payload containing config fields;
- no unbounded Cloudflare or S3 response;
- no redirect while sending Cloudflare bearer credentials;
- no automatic bucket deletion;
- no recursive probe cleanup;
- no accidental revocation of an access ID adopted by committed config;
- no test against a real `~/.mallard`, `~/.codex`, or `~/.claude`; and
- no generated bundle, credential, or personal absolute path in Git.

Mallard still stores the persistent R2 S3 credential in machine-local schema-1
config. A future operating-system keychain change is separate scope.

## 14. Verification status

Implemented automated coverage includes:

- strict default, EU, and FedRAMP parser vectors;
- malformed URL, bucket, account, and alias rejection;
- exact config revision, idempotency, duplicate, and concurrent-writer cases;
- full-document clean-cutover validation;
- every probe stage and cleanup failure class;
- bounded/no-follow/atomic journal behavior;
- Cloudflare permission, resource-policy, derivation, token-role, recovery,
  and failure-injection tests;
- CLI unit and subprocess tests, including secret canaries;
- frontend endpoint, focused-create, watcher revision, dirty-draft conflict,
  TOCTOU baseline, instruction, and agent reconciliation tests;
- static macOS and Windows release-contract tests; and
- packaged CLI help/signature/install/uninstall verification scripts.

Required handoff commands are:

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
npm run build
npm run test:frontend-integration
npm run test:release-config
npm run check:release-version
git diff --check
```

The Apple Silicon DMG gate passed locally on August 7, 2026: the release
wrapper produced the DMG, verified its checksum, mounted it read-only, checked
the app and nested CLI signatures and CLI help, copied the complete app to a
temporary installed location, detached the image, and repeated those checks.

Two environment-dependent gates remain:

1. run provisioning and cleanup against a disposable Cloudflare account and
   bucket, with credentials kept out of logs; and
2. let the release workflow execute the NSIS installer verifier on Windows.

The live smoke must revoke generated tokens, remove its disposable bucket only
through explicit test teardown, and never use a personal production bucket.

## 15. Definition of done

Source implementation is complete when all of the following hold:

- macOS and Windows packages contain the matching CLI executable;
- the copied instruction uses the exact packaged path and contains no secret;
- connect, provision, verify, cleanup, and list use one application service
  boundary and one schema-1 repository;
- R2 capabilities, including stale-CAS rejection, are proven before commit;
- config mutation is locked, atomic, duplicate-safe, and reports the exact
  committed revision;
- the open R2 setup page adopts and fills a clean external storage addition;
- dirty or ambiguous editor state is never overwritten;
- every partial provisioning failure either cleans up or leaves a bounded,
  non-secret recovery record;
- the app never revokes an adopted final credential or deletes a bucket;
- macOS installed-copy verification passes;
- Windows install/help/uninstall verification passes; and
- a disposable live R2 smoke passes.

## 16. References

Mallard implementation:

- [`storage_setup.rs`](../crates/mallard-application/src/storage_setup.rs)
- [`cloudflare_r2_setup.rs`](../crates/mallard-application/src/cloudflare_r2_setup.rs)
- [`r2_setup_journal.rs`](../crates/mallard-infrastructure/src/r2_setup_journal.rs)
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs)
- [`mallard-cli`](../crates/mallard-cli/)
- [`commands.rs`](../src-tauri/src/project_sync/commands.rs)
- [`config_watcher.rs`](../src-tauri/src/project_sync/config_watcher.rs)
- [`ProjectLinksWorkspace.tsx`](../src/components/project-sync/ProjectLinksWorkspace.tsx)
- [`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx)
- [`StorageSettings.tsx`](../src/components/project-sync/StorageSettings.tsx)
- [`tauri-build.mjs`](../scripts/tauri-build.mjs)
- [`release.yml`](../.github/workflows/release.yml)

Cloudflare primary documentation:

- [R2 authentication and S3 credential derivation](https://developers.cloudflare.com/r2/api/tokens/)
- [Create tokens through the API](https://developers.cloudflare.com/fundamentals/api/how-to/create-via-api/)
- [Create an account-owned token](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/methods/create/)
- [Delete an account-owned token](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/methods/delete/)
- [List account token permission groups](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/subresources/permission_groups/methods/list/)
- [Create an R2 bucket](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/create/)
- [Get an R2 bucket](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/get/)

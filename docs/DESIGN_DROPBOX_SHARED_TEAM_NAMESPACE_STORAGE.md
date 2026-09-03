# Design: Dropbox collaborator namespace storage

**Status:** proposed for implementation

**Date:** 2026-09-02

**Scope:** Dropbox storage for Project Sync and the storage Library

## 1. Decision

Mallard will add Dropbox as a third storage destination.

The first release has one access model: an existing collaborator configures a
Dropbox namespace that they can already edit. The namespace can be a shared
folder, team folder, or team-space namespace that is available to that user.

The user supplies:

- a pre-issued Dropbox user access token;
- the exact Dropbox namespace ID; and
- a local Mallard storage name.

Mallard does not implement OAuth. It does not open an authorization page,
receive a callback, exchange a code, refresh a token, register a Dropbox app,
request consent, or revoke a grant.

Mallard also does not use the Dropbox Business API. It does not accept a team
token, select a team member, or send `Dropbox-API-Select-User` or
`Dropbox-API-Select-Admin`.

The selected namespace is the storage root. Mallard stores its existing
`.mallard/` object tree directly below that root.

Dropbox implements the existing revision model:

- Dropbox file `rev` is Mallard's opaque object revision;
- strict add implements create-if-absent;
- strict revision update implements compare-and-swap;
- revision-pinned file deletion implements exact deletion; and
- Mallard SHA-256 remains the content-integrity authority.

### 1.1 Removed complexity

| Removed design element | Replacement |
|---|---|
| OAuth browser and callback | User supplies a pre-issued token |
| PKCE, state, and code exchange | None |
| Refresh-token lifecycle | User replaces an expired access token |
| Authorization profiles | Token is part of the machine-local storage destination |
| Native credential vault | Existing restricted Mallard persistence |
| Team administrator grant | Configuring user is already a collaborator |
| Team member selection | User-linked token acts as its own user |
| Namespace discovery | User supplies the namespace ID |
| Connect, reconnect, and disconnect flows | Test, save, replace token, or remove storage |

This limitation is intentional. If automatic token renewal becomes a product
requirement, it needs a separate design and explicit authorization boundary.

## 2. Clean schema-1 cutover

This change is a clean cutover. All affected schemas stay at `1`.

- Replace the machine-local storage shape in schema 1.
- Replace the machine-local storage-setup draft shape in schema 1.
- Replace machine-local documents that embed either shape.
- Replace the R2-only setup journal with a provider-neutral schema-1 journal.
- Keep portable bundle schema 1 unchanged.
- Keep storage-marker schema 1 and its layout unchanged.
- Add no migration, compatibility reader, alias, default, fallback, dual
  writer, old command, or legacy execution path.
- Reject the old flat storage shape.
- Reject the old persisted `"s3"` destination tag.
- Persist Cloudflare R2 with the `"r2"` destination tag.

There are no customers and no retained test data. The user will delete their
disposable `~/.mallard` state and disposable cloud data before retesting.
Implementation and automated tests must not perform that manual deletion.
Tests must use an injected temporary home or fixture repository.

## 3. Goals

- Let a collaborator use an existing shared or team namespace as Mallard
  storage.
- Let another collaborator configure the same namespace with their own token.
- Preserve immutable publication and compare-and-swap head updates.
- Preserve Push, Pull, restore, Library, and deletion review.
- Keep credentials out of portable data, project directories, logs, and API
  responses.
- Use the current `.mallard/` object-key layout.
- Bind storage continuity to a namespace ID, not a display path or token.
- Keep every remote operation bounded.
- Keep Local and R2 working after each implementation layer.

## 4. Non-goals

- OAuth or Dropbox consent inside Mallard.
- Access-token refresh.
- Unattended Dropbox access beyond the supplied token's lifetime.
- Refresh-token or app-secret storage.
- Dropbox app registration or production approval.
- Team-linked tokens or Dropbox Business API calls.
- Team member or administrator impersonation.
- Listing Dropbox accounts, team members, shared folders, or namespaces.
- Creating, sharing, unsharing, mounting, moving, renaming, archiving, or
  deleting the selected namespace.
- Selecting a second child folder as the storage root.
- Dropbox App Folder access.
- Personal Dropbox-home storage.
- Recursive folder deletion.
- Dropbox webhooks, delta synchronization, or a local Dropbox cache.
- Upload sessions or batch mutation in the first release.
- A generic provider plug-in framework.

## 5. User prerequisites and flow

### 5.1 Prerequisites

Before configuration, the user must have:

- a Dropbox user-linked access token issued outside Mallard;
- Full Dropbox content access on the app that issued the token;
- `account_info.read`;
- `files.metadata.read`;
- `files.content.read`;
- `files.content.write`;
- the stable namespace ID; and
- edit access to that namespace.

Mallard does not request, expand, or inspect an OAuth grant. It verifies only
the behavior that it needs. The token must let Mallard call
`/users/get_current_account` and the required `/files/*` routes.

### 5.2 Configuration flow

1. The user selects **Dropbox** in storage setup.
2. The user enters a storage name, namespace ID, and access token.
3. Mallard saves the secret only in the restricted storage-setup draft.
4. Mallard tests the exact namespace and revision behavior.
5. Mallard creates or validates the unchanged schema-1 storage marker.
6. The user reviews and saves the storage.
7. Mallard moves the tested destination into the machine-local configuration.
8. Mallard removes the setup draft after the commit succeeds.

Another collaborator repeats this flow with their own user token and the same
namespace ID. Both configurations have the same storage identity.

### 5.3 Expired token flow

Mallard does not refresh an access token.

Dropbox access tokens are short-lived. Manual token replacement is therefore
an accepted product constraint. Mallard must not claim unattended Dropbox
sync beyond the current token's lifetime.

When Dropbox returns a token-related `401`, Mallard:

1. stops the operation;
2. reports **Dropbox access token expired or invalid**;
3. keeps local and remote data unchanged; and
4. asks the user to replace and retest the token.

The user obtains a replacement token outside Mallard. A focused storage update
tests that token before it replaces the stored token.

## 6. Machine-local storage model

Replace the flat `StorageConfig` with a tagged destination:

```rust
#[serde(deny_unknown_fields)]
struct StorageConfig {
    id: StorageId,
    name: DisplayName,
    destination: StorageDestination,
}

#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StorageDestination {
    Local {
        directory: LocalStorageDirectory,
    },
    R2 {
        bucket: BucketName,
        access_key_id: AccessKeyId,
        secret_access_key: SecretAccessKey,
        account_id: R2AccountId,
        endpoint: R2Endpoint,
        region: R2Region,
    },
    Dropbox {
        namespace_id: DropboxNamespaceId,
        access_token: DropboxAccessToken,
    },
}
```

The R2 wrappers above are new validated types for the cutover. Preserve the
current R2 validation behavior.

### 6.1 Dropbox field rules

`namespace_id` is a case-sensitive opaque value. Apply Dropbox's identifier
grammar and a conservative Mallard byte bound. Do not require it to be
numeric. Reject an empty value, controls, surrounding whitespace, or a value
that does not round-trip exactly.

`access_token` is an opaque secret. It must be nonempty, control-free, and at
most 64 KiB. Its debug and display forms are always redacted.

Do not store:

- a Dropbox account ID;
- a team or member ID;
- an app key or secret;
- a refresh token;
- an OAuth scope list;
- a display path; or
- a namespace label.

The Mallard storage name is sufficient presentation data.

### 6.2 Storage identity

The Dropbox continuity identity is the SHA-256 hash of this canonical tuple:

```text
dropbox-v1 || namespace_id
```

It excludes the access token and storage name. Token replacement does not
change storage continuity. Two collaborators who configure the same namespace
get the same continuity identity.

Reject two configured Dropbox storages with the same namespace ID. The first
release supports one Mallard storage per namespace.

For an existing Dropbox storage, the tested upsert can replace the name or
token but not the namespace ID. A different namespace requires a new
`StorageId`. This prevents one local storage identity from silently moving to a
different remote tree.

### 6.3 Secret persistence and redaction

The access token follows the existing R2 secret boundary:

- persist it only in schema-1 machine-local configuration or a restricted
  storage-setup draft under `~/.mallard`;
- use the existing canonical, no-follow, mode-`0700` root and mode-`0600`
  atomic JSON persistence on Unix;
- use the equivalent restricted file permissions on supported Windows builds;
- clear the token before any configuration or draft DTO reaches the webview;
- treat a blank token on an existing destination as **keep the stored token**;
- accept a nonblank replacement only through the focused tested upsert;
- never include it in portable manifests, receipts, plans, activity records,
  logs, errors, or project files; and
- never echo a malformed token in a validation error.

The setup form can hold a new token only while the user edits it. Clear the
frontend value as soon as the draft update completes. Stored values never
return to frontend state.

There is no native-vault entry and no separate Dropbox authorization document.
The existing bounded local reset removes the token with the rest of
`~/.mallard` state.

## 7. Draft and configuration rules

Replace `StorageSetupDraft` with a provider-tagged schema-1 draft. The Dropbox
variant contains the same namespace and token fields as the destination plus
the existing draft ID, storage ID, revision, name, and timestamps.

Add `deny_unknown_fields` to each replaced document and tagged variant.

Project setup can select only a committed `StorageId`. Remove
`DraftStorageSelection::Pending`, `SetupTransaction.storage`, and the frontend
path that opens an uncommitted `StorageConfig`. Project setup must not copy a
Dropbox token into a second document.

Replace the R2-specific generic-save guard with a remote-storage guard. The
generic configuration command can rename or remove a storage. It cannot:

- create an R2 or Dropbox destination;
- change a destination kind;
- change R2 connection or credential fields; or
- change a Dropbox namespace or access token.

Only the provider-specific tested upsert can make those changes.
For Dropbox, namespace replacement is not an update operation. Create a new
storage instead.

## 8. Namespace validation and mapping

### 8.1 User-token validation

Mallard calls `/users/get_current_account` without a Dropbox selector header.
The call must succeed as User Authentication. A team-linked token that needs a
selector is unsupported and fails configuration.

Reject the selected namespace when it equals `home_namespace_id`. If it equals
`root_namespace_id`, accept it only when `root_info` is the Dropbox team-root
variant and the root ID differs from the home ID. Any other configured
namespace must pass the file probe. This permits an accessible shared or team
namespace but rejects a personal user root.

Mallard does not use `/sharing/list_folders` or `/team/namespaces/list`.

### 8.2 Namespace root

Every Dropbox file call sends:

```text
Dropbox-API-Path-Root: {".tag":"namespace_id","namespace_id":"<id>"}
```

Paths are relative to that root:

```text
/.mallard/_storage.json
/.mallard/v1/repositories/...
/.mallard/v1/library/...
```

Never fall back to the token's default root, home mode, root mode, a display
path, or a matching name.

At setup, Mallard lists `path: ""` through the exact namespace root. It rejects
an unavailable namespace or one that cannot pass the capability probe.

A namespace rename preserves its ID and storage continuity. A different
same-name namespace has a different ID and is not adopted.

### 8.3 No child-root selector

The selected namespace itself is the storage root. The first release does not
accept a path or folder ID below it as a second root. This avoids a mutable
path-resolution and replacement race.

### 8.4 Path validation

Only a validated `ObjectKey` can reach the Dropbox transport. The transport
adds one leading slash and evaluates it relative to the selected namespace.

Reject:

- empty keys;
- absolute input paths;
- backslashes;
- `.` or `..` components;
- NUL or control characters;
- invalid Dropbox components;
- a component or total path above its bound; and
- any normalized value that escapes the namespace.

## 9. Configured store integration

Do not add an authorization repository, token manager, or credential-aware
factory.

Extend the existing concrete dispatch enum:

```rust
enum ConfiguredObjectStore {
    Local(LocalObjectStore),
    R2(S3ObjectStore),
    Dropbox(DropboxObjectStore),
}
```

Extend `ConfiguredObjectStore::from_config` with the Dropbox destination. The
method constructs the store directly from the tested machine-local config.

This keeps the current construction pattern for:

- project bundle engines;
- project activity snapshots;
- managed clear; and
- the storage Library.

Add exhaustive delegation for the complete `ObjectStore` contract. Do not add
a trait-object forwarding layer.

## 10. Dropbox transport

Add:

```text
crates/mallard-infrastructure/src/storage_core/dropbox_store.rs
```

The core type is:

```rust
struct DropboxObjectStore {
    client: reqwest::Client,
    namespace_id: DropboxNamespaceId,
    access_token: SecretDropboxAccessToken,
    runtime: tokio::runtime::Handle,
}
```

The runtime secret type does not implement `Debug`, `Display`, `Serialize`, or
unrestricted `Clone`. Convert from the persisted secret wrapper only at the
configured-store boundary. The persisted wrapper has a redacted `Debug`
implementation. Zeroize owned token buffers when the store drops.

Use Dropbox API v2 through the existing HTTP stack. Dropbox's Rust SDK
repository states that it has no formal Dropbox support and can have missing
features. Do not add it. Use one bounded `reqwest::Client` with explicit
connect, request, body, and idle timeouts.

Use only fixed Dropbox HTTPS origins and fixed endpoint paths. Accept no
user-supplied host, base URL, redirect target, proxy credential, or TLS
override. Disable automatic redirects. Bound response headers and bodies.

The `ObjectStore` trait remains synchronous. Bridge to the application's
existing Tokio runtime at the transport boundary. Do not create a runtime or
HTTP client per request.

### 10.1 Route mapping

| Mallard operation | Dropbox route and behavior |
|---|---|
| `get_metadata` | `/files/download`; return bounded bytes and `rev` |
| `stage_verified` | Stream `/files/download` to a private temp file |
| `put_immutable` | `/files/upload` with strict add and no autorename |
| `compare_and_swap(Absent)` | Strict add; conflict when the path exists |
| `compare_and_swap(Match)` | Strict update by the expected `rev` |
| `delete` | `/files/delete_v2` with the expected `parent_rev` |
| `list` | Recursive `/files/list_folder` plus continuation |

Every file route sends the user access token and exact namespace Path-Root.
No route sends a team selector.

### 10.2 Upload rules

All current Mallard object limits are below Dropbox's 150 MB direct-upload
limit. Use `/files/upload` only. Set a transport limit of 150,000,000 bytes and
also enforce the smaller existing object-class limit.

Every upload uses:

- `autorename: false`;
- `strict_conflict: true`;
- the exact expected write mode; and
- a staged Dropbox content hash when supported by the route.

For immutable publication, an existing path is successful only when a bounded
download proves the expected size and Mallard SHA-256.

A stale revision is always a conflict, including when the proposed bytes are
identical to current bytes.

### 10.3 Ambiguous mutations

A compare-and-swap reports `Written` only after Mallard receives and validates
the successful response for that attempt.

If the response is lost, matching read-back bytes do not prove that this writer
won. Another collaborator can have written the same bytes. Return a fixed
indeterminate-mutation error. The caller must reload remote state and replan.

Apply the same correction to the current R2 ambiguous-CAS resolver. Local
storage can return a definite result because its compare and write occur under
one local lock.

### 10.4 Content hashes

Dropbox `content_hash` is not Mallard's standard whole-file SHA-256.

Compute both during staging. The Dropbox hash algorithm is:

1. Split content into 4,194,304-byte blocks.
2. SHA-256 each block.
3. Concatenate the binary block digests.
4. SHA-256 the concatenation.
5. Encode the result as lowercase hexadecimal.

Mallard SHA-256 remains the manifest and integrity value. A present Dropbox
hash adds a provider-transport check. It never replaces Mallard SHA-256.

### 10.5 Dropbox API argument header

Dropbox content routes put JSON in `Dropbox-API-Arg`. Implement one tested JSON
encoder that produces a valid HTTP-header value and escapes non-ASCII input.
Reject CR, LF, and values that cannot be represented safely. Do not construct
the JSON with string interpolation.

## 11. Revision-bearing list and conditional delete

The current key-only list and unconditional delete cannot protect a
collaborator's replacement file. Change the provider-neutral contract for
Local, R2, and Dropbox:

```rust
struct StoreListEntry {
    key: ObjectKey,
    etag: String,
}

struct StoreListPage {
    entries: Vec<StoreListEntry>,
    next_cursor: Option<String>,
}

enum DeleteOutcome {
    Deleted,
    Missing,
    Conflict { current_etag: Option<String> },
}

trait ObjectStore {
    fn delete(
        &self,
        key: &ObjectKey,
        expected_etag: &str,
    ) -> Result<DeleteOutcome, String>;
}
```

There is no delete-any variant.

- Local verifies its SHA-256 revision and deletes under its store lock.
- R2 uses `DeleteObject` with `If-Match`.
- Dropbox uses the file `rev` as `parent_rev`.
- Missing is idempotent success.
- A changed revision returns `Conflict`.
- Managed clear stops on the first conflict.

The R2 implementation and a live Cloudflare R2 test must prove conditional
delete before this cutover merges. If R2 cannot provide this guarantee, revise
the provider-neutral clear design before implementation. Do not add an
unconditional fallback.

### 11.1 Dropbox delete rules

Dropbox file deletion can delete a folder recursively. The adapter must prevent
that behavior.

For one delete:

1. Receive the expected `rev` from the list, plan, or recovery journal.
2. Read metadata at the exact logical path.
3. Require the same revision and `FileMetadata`.
4. Call `/files/delete_v2` with the exact path and expected `parent_rev`.
5. Validate the returned metadata when a response is available.

Never delete by file ID. An ID follows a file after a move and can leave
Mallard's owned prefix. Never call folder delete, batch delete, permanent
delete, or recursive cleanup.

After an ambiguous result, read only the exact logical path:

- absent means the logical path is deleted;
- the same file and revision permits one bounded retry;
- a different ID, revision, or type is a conflict.

## 12. Folder creation, listing, and case rules

### 12.1 Folder creation

Mallard can create only the directories required by a validated object key
below the selected namespace:

```text
.mallard
.mallard/v1
closed descendants required by the object key
```

Create one component at a time with `autorename: false`. On conflict, continue
only when exact-path metadata is the expected folder. Reject a file or unknown
metadata type.

Mallard never deletes these folders. Clear can leave empty `.mallard/`
directories.

### 12.2 Listing

Use recursive `/files/list_folder` and process all continuation pages. Apply
limits to:

- request count;
- entries;
- encoded metadata bytes;
- total object-key bytes; and
- elapsed time.

For each entry:

1. Verify that it is below the requested Mallard prefix.
2. Strip only the verified prefix.
3. Validate the remaining value as an `ObjectKey`.
4. Accept only a regular downloadable file with a valid nonempty `rev`.
5. Return the `rev` as the list-entry `etag`.
6. Ignore expected directory metadata.
7. Fail closed on an unexpected type inside an owned prefix.

Dropbox cursors stay inside one adapter call. The public list cursor remains a
Mallard lexical object key. Sort accepted keys before returning a page.

### 12.3 Case-insensitive paths

Dropbox paths are case-insensitive. Mallard keys are exact.

- Use `path_lower` for normalized containment and duplicate detection.
- Use `Metadata.name` to verify the exact final component.
- Enumerate ancestors when their exact spelling is required.
- Treat `path_display` as display-only data.
- Reject duplicate normalized paths and case-only aliases.
- Never lowercase a Mallard object key silently.

Audit every repository and Library key constructor. Dropbox remains disabled
if two legal Mallard keys can differ only by Dropbox case rules.

## 13. Setup probe and recovery

Replace the R2-only setup journal with a provider-neutral schema-1 journal. Do
not read the old journal shape.

The Dropbox journal entry contains:

- setup ID;
- exact storage-draft ID and revision;
- namespace ID;
- destination fingerprint;
- deterministic probe object suffixes;
- current probe revisions, sizes, and Mallard hashes;
- bounded stage and timestamps; and
- one fixed sanitized error code.

It contains no access token or provider response. The exact draft is the one
credential source during pre-commit recovery. A draft cannot be deleted while
a journal entry references it. If the draft is missing or has a different
revision, automatic cleanup stops with `manual_cleanup_required`.

Use this reserved prefix:

```text
.mallard/v1/_setup_probes/<setup-id>/
```

The setup service uses a private validated probe-key type. It does not widen
the normal repository or Library key spaces.

### 13.1 Probe sequence

1. Validate the draft and expected configuration revision.
2. Call `/users/get_current_account` with no selector.
3. Verify the namespace is not the personal home namespace.
4. List the configured namespace root.
5. Persist the pre-mutation journal entry.
6. Create required folders one component at a time.
7. Prove the deterministic probe key is absent.
8. Client A performs strict absent creation.
9. Client B proves a second absent creation conflicts.
10. Both clients read and verify the same bytes and `rev`.
11. Client A updates by the current revision.
12. Client B proves its stale revision cannot update, including identical
    proposed bytes.
13. Delete by the current revision, recreate, and prove the old revision
    cannot update or delete the replacement.
14. List and verify that no autorenamed or conflict-copy file exists.
15. Delete the exact replacement by its revision.
16. Repeat the delete and require idempotent `Missing`.
17. Verify that no owned probe file remains.
18. Create or validate the unchanged schema-1 storage marker.
19. Persist a bounded successful probe receipt.

The two setup clients have independent HTTP state but use the same configured
user token. The live collaborator gate uses two independent user tokens.

### 13.2 Cleanup

Cleanup lists only the deterministic setup prefix. It deletes only a regular
file whose expected revision, size, Mallard SHA-256, and known probe content
all match. It never deletes a folder or unknown object.

Persist each witnessed revision before the next mutation. If a crash can have
written a file without recording its revision, require manual cleanup. Do not
guess a revision.

`test_dropbox_storage` can leave the valid storage marker and empty folders.
The UI must state this before the test. Mallard never rolls back a valid marker
by deleting it.

## 14. Commands and DTOs

Add only two Dropbox-specific commands:

```text
test_dropbox_storage
upsert_dropbox_storage
```

Use the existing provider-neutral storage-draft commands to create, update,
list, and discard the setup draft.

`test_dropbox_storage` requires:

- storage-draft ID and revision; and
- expected global configuration revision.

`upsert_dropbox_storage` requires the same pins plus:

- successful probe ID;
- destination fingerprint;
- probe digest; and
- probe expiry.

It rechecks all pins before marker validation and inside the final config
transaction. A failed pin changes no configuration.

Command rules:

- command names use snake case;
- JavaScript arguments use camel case;
- serialized Rust fields use snake case;
- input structs deny unknown fields;
- Rust and `src/types.ts` change together;
- the frontend calls typed `projectSyncApi` helpers;
- stored tokens never appear in command responses; and
- raw Dropbox errors never cross IPC.

There are no authorize, callback, status, member, namespace, reconnect,
disconnect, forget, or token-refresh commands.

## 15. Concurrency

Use the existing `LocalStateRepository::acquire_host_operation()` OS lock. Do
not add a Dropbox-specific lock.

Hold the host-operation guard for a setup test, upsert, token replacement,
Push, Pull, restore, Library remote operation, managed clear, and storage
removal. A competing Mallard process fails fast with the existing busy result.

Do not hold the Project Sync persistence or Library catalog-state lock during
Dropbox network I/O. Reacquire state locks after the remote phase, reload all
revision pins, and commit only when they still match.

For a cross-document storage-reference mutation, use this order:

1. host-operation guard;
2. Project Sync persistence lock; and
3. Library catalog-state lock.

Storage removal and Library link creation use this same order. Each rechecks
storage existence and all references before it writes. This prevents a Library
link from appearing between a removal check and the configuration write.

## 16. Error and retry policy

Use fixed Mallard errors:

```text
dropbox_access_token_invalid
dropbox_namespace_unavailable
dropbox_permission_denied
dropbox_conflict
dropbox_mutation_outcome_unknown
dropbox_rate_limited
dropbox_data_transport_limit
dropbox_transport_unavailable
dropbox_response_invalid
dropbox_manual_cleanup_required
```

Never log or return:

- the access token;
- the `Authorization` header;
- the full `Dropbox-API-Arg` or Path-Root header;
- account or namespace IDs;
- remote paths;
- Dropbox response bodies or request IDs; or
- raw HTTP-client debug output.

Parse bounded structured Dropbox errors before sanitization.

Retry rules:

- Do not refresh or automatically retry a token-related `401`.
- Do not retry a permission-related `403`.
- Map a Path-Root `422` to namespace unavailable.
- Retry `429` and transient `5xx` responses with bounded backoff and jitter.
- Honor a valid bounded `Retry-After` value.
- Fail when `Retry-After` exceeds the operation deadline.
- Reconcile an ambiguous mutation before any retry.
- Never retry a definite conflict as an overwrite.

Start with at most four concurrent reads and two concurrent writes per storage.
Use one operation deadline and one retry budget across all attempts.

## 17. UI

Add one **Dropbox** storage editor with:

- storage name;
- namespace ID;
- access token;
- **Test**; and
- **Save**.

The editor states:

- the user must already be a collaborator with edit access;
- the token and namespace ID must be obtained outside Mallard;
- Mallard does not authorize Dropbox or renew the token;
- an expired token must be replaced manually;
- the test can create a schema-1 marker and empty `.mallard/` folders; and
- Mallard never deletes the selected namespace or its folders.

After draft submission, blank the token field. A saved storage displays only:

```text
Dropbox · <Mallard storage name>
```

Never show a token suffix, account identity, namespace ID, or raw provider
error.

Update storage cards, setup summaries, project selectors, sidebar status, the
Library selector, empty states, and error copy with exhaustive Local, R2, and
Dropbox branches.

## 18. Implementation layers

### Layer 1: schema-1 cutover

- Add the tagged Local, R2, and Dropbox destinations.
- Rename the persisted `s3` tag to `r2`.
- Remove `supports_conditional_writes`.
- Replace setup drafts and embedded snapshots.
- Remove pending inline project storage.
- Add strict rejection tests for every old shape.
- Keep Dropbox creation hidden.
- Make all Local and R2 tests pass.

### Layer 2: provider-neutral delete correction

- Add revision-bearing list entries.
- Add required expected revisions to delete.
- Implement Local conditional delete.
- Implement and prove R2 conditional delete.
- Correct ambiguous R2 compare-and-swap handling.
- Keep Dropbox creation hidden.

### Layer 3: Dropbox transport

- Add `DropboxObjectStore`.
- Extend `ConfiguredObjectStore::from_config`.
- Add scripted-server transport tests.
- Run the provider-neutral conformance suite.
- Keep Dropbox creation hidden.

### Layer 4: tested setup

- Add the Dropbox draft branch.
- Add the provider-neutral probe journal.
- Add test and upsert commands.
- Add token redaction and remote-change guards.
- Add marker and recovery tests.
- Enable Dropbox only in developer builds.

### Layer 5: product integration

- Add the Dropbox editor and summaries.
- Route Project Sync, activity, managed clear, and Library through the new
  configured-store branch.
- Pass frontend and complete-app tests.
- Pass the two-collaborator live gate.
- Set list and clear limits from measured Dropbox behavior.
- Enable Dropbox in production builds.

Each layer must leave Local and R2 working. Do not merge a layer with a bypass
or disabled invariant.

## 19. Test plan

### 19.1 Domain and persistence

- Round-trip each new schema-1 destination and draft.
- Reject the old flat storage shape and old `s3` tag.
- Reject unknown tags and fields.
- Reject old inline project-storage and R2-journal shapes.
- Reject invalid namespace IDs and tokens.
- Prove every configuration and draft response redacts the token.
- Scan JSON fixtures, IPC snapshots, errors, and logs with a token canary.
- Prove Dropbox identity changes only with namespace ID.
- Prove project setup can reference only a committed storage.

### 19.2 Transport

Use a scripted local server to verify:

- exact method, endpoint, content type, and headers;
- Path-Root on every file route;
- no Select-User or Select-Admin header;
- strict add and strict revision update;
- stale revision conflict with identical bytes;
- stale revision conflict after delete and recreate;
- revision-pinned file delete;
- replacement-before-delete conflict;
- refusal to delete a folder;
- ambiguous upload, update, and delete handling;
- direct-upload limits;
- Mallard and Dropbox hash validation;
- multi-page bounded listing and lexical output;
- case-only alias rejection;
- structured `401`, `403`, `409`, `422`, `429`, `5xx`, timeout, truncated
  body, oversized body, and invalid JSON behavior; and
- no provider data in errors or logs.

Run one reusable `ObjectStore` conformance suite against Local, R2, and
Dropbox.

### 19.3 Setup and commands

- Reject a team token that cannot call User Authentication without a selector.
- Reject a personal home namespace.
- Reject a namespace without edit access.
- Prove strict create, CAS, list, conditional delete, and cleanup.
- Prove marker creation and validation keep schema `1`.
- Prove stale draft, config, probe, digest, and expiry pins fail.
- Prove generic save cannot change a token or namespace.
- Prove an existing Dropbox storage cannot change namespace ID; a new
  namespace requires a new `StorageId`.
- Prove token replacement keeps the old token after a failed test.
- Prove recovery deletes only fully witnessed probe files.
- Prove a missing or changed draft requires manual cleanup.
- Verify both commands are registered and typed.

### 19.4 Concurrency

- Prove two processes cannot run host-sensitive operations concurrently.
- Race storage removal against Library link creation.
- Prove no catalog record can reference a missing storage.
- Prove persistence and catalog locks are not held during network waits.

### 19.5 Frontend

- Cover exact Local, R2, and Dropbox branches.
- Prove Dropbox has no OAuth, Connect, Reconnect, team, actor, or namespace
  discovery UI.
- Prove stored tokens never return to frontend state.
- Cover test, save, failed test, expired token, token replacement, stale
  response, and revision conflict.
- Verify Library receives only its neutral storage option.

### 19.6 Live collaborator gate

Use two independent user-linked access tokens whose users can edit the same
disposable shared or team namespace:

1. Both configure the same namespace ID independently.
2. Both pass the exact namespace-root capability probe.
3. User A publishes a project bundle and Library content.
4. User B reads and accepts both.
5. A and B race a head update; exactly one wins.
6. The loser reloads remote state, replans, and completes normally.
7. Replace A's token with another user token for the same namespace; continuity
   remains unchanged.
8. Use an expired token; Mallard stops and requests manual replacement.
9. Rename the namespace; both configurations continue through the same ID.
10. Replace it with a same-name namespace in a disposable area; Mallard does
    not follow the new ID.
11. Clear one owned branch and prove unrelated files and all folders remain.

### 19.7 Handoff checks

Run at minimum:

```text
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
npm run test:storage-catalog-e2e
```

Run the complete Tauri application for token entry, setup, Push, Pull, Library,
and clear behavior. Include screenshots or a recording for the storage editor.

## 20. Acceptance criteria

Dropbox support is complete only when all statements are true:

- A user who already has edit access can configure a shared or team namespace
  with a pre-issued user access token.
- Mallard performs no OAuth, token refresh, team authorization, or member
  impersonation.
- Two collaborators can configure the same namespace with different tokens.
- Both configurations have the same storage identity.
- Every file route uses the exact namespace-ID Path-Root.
- No request sends Select-User or Select-Admin.
- Immutable create and compare-and-swap pass the conformance suite.
- A lost CAS response never becomes success from matching read-back bytes.
- A stale revision never overwrites or deletes another writer's file.
- Managed clear deletes only the listed revision and stops on replacement.
- Mallard never calls Dropbox folder delete.
- Tokens exist only in restricted machine-local config or setup drafts and
  transient input/runtime memory.
- Stored tokens never enter frontend responses, logs, project files, or
  portable data.
- Token expiry produces a manual replacement flow without data mutation.
- Mallard does not claim unattended Dropbox access beyond token expiry.
- Every replaced schema-1 shape has no legacy reader or execution path.
- Bundle and storage-marker schemas remain `1` and unchanged.
- Local and R2 remain green.
- The R2 conditional-delete gate and Dropbox live collaborator gate pass.

## 21. Expected code areas

Implementation is expected to touch:

```text
crates/mallard-domain/src/lib.rs
crates/mallard-domain/src/tests.rs
crates/mallard-application/src/dropbox_storage_setup.rs
crates/mallard-application/src/lib.rs
crates/mallard-application/src/storage_setup.rs
crates/mallard-infrastructure/Cargo.toml
crates/mallard-infrastructure/src/storage_probe_journal.rs
crates/mallard-infrastructure/src/r2_setup_journal.rs (remove)
crates/mallard-infrastructure/src/persistence.rs
crates/mallard-infrastructure/src/project_activity.rs
crates/mallard-infrastructure/src/storage_core/mod.rs
crates/mallard-infrastructure/src/storage_core/object_store.rs
crates/mallard-infrastructure/src/storage_core/dropbox_store.rs
crates/mallard-cli/src/backend.rs
crates/mallard-cli/tests/cli.rs
src-tauri/src/project_sync/commands.rs
src-tauri/src/project_sync/service.rs
src-tauri/src/storage_catalog/persistence.rs
src-tauri/src/storage_catalog/resource_engine.rs
src-tauri/src/storage_catalog/resource_planning.rs
src-tauri/src/storage_catalog/service.rs
src-tauri/src/lib.rs
src/types.ts
src/components/project-sync/api.ts
src/components/project-sync/ProjectSync.tsx
src/components/project-sync/StorageSettings.tsx
src/components/project-sync/StorageIndexPage.tsx
src/components/project-sync/BundleConnectionDialog.tsx
src/components/project-sync/ProjectPicker.tsx
src/components/project-sync/ProjectLinksWorkspace.tsx
src/components/project-sync/ProjectSetupWorkspace.tsx
src/components/project-sync/ProjectSidebar.tsx
src/components/storage-catalog/StorageCatalogPage.tsx
src/storageError.ts
tests/frontend/
src-tauri/src/sync_tests/
```

Before implementation, search exhaustively for `StorageConfig`, `StorageKind`,
`DraftStorageSelection`, `StorageSetupDraft`, `SetupTransaction`,
`R2SetupJournal`, `ConfiguredObjectStore::from_config`, secret-redaction
helpers, and all `ObjectStore::delete` implementations and test doubles.

## 22. POC status

The repository POC is under
[`poc/dropbox-shared-team-namespace/`](../poc/dropbox-shared-team-namespace/).
Its result is recorded in
[`DROPBOX_SHARED_TEAM_NAMESPACE_POC_2026-09-02.md`](./DROPBOX_SHARED_TEAM_NAMESPACE_POC_2026-09-02.md).

All 18 offline tests pass. The harness already accepts pre-issued tokens and an
explicit namespace ID. This design adopts that credential boundary.

The live Dropbox run has not occurred. Offline tests do not prove live Dropbox
CAS behavior, collaborator permissions, nested `.mallard/` mapping, throttling,
or production transfer limits.

## 23. Official references

Dropbox route and type claims were checked on 2026-09-02 against official
`dropbox-api-spec` commit
[`33adb3af6ad7199532c03d880f0520619ef17763`](https://github.com/dropbox/dropbox-api-spec/tree/33adb3af6ad7199532c03d880f0520619ef17763).
Recheck the pinned basis and live documentation before implementation.

- [Dropbox HTTP API documentation](https://www.dropbox.com/developers/documentation/http/documentation)
- [Dropbox OAuth guide and short-lived token behavior](https://developers.dropbox.com/oauth-guide)
- [Dropbox authentication types](https://www.dropbox.com/developers/reference/auth-types)
- [Dropbox Team Files Guide](https://developers.dropbox.com/dbx-team-files-guide)
- [Dropbox Path-Root modes](https://www.dropbox.com/developers/reference/path-root-header-modes)
- [Dropbox File Access Guide](https://developers.dropbox.com/dbx-file-access-guide)
- [Dropbox error handling guide](https://developers.dropbox.com/error-handling-guide)
- [Dropbox performance guide](https://developers.dropbox.com/dbx-performance-guide)
- [Dropbox data transport limit](https://www.dropbox.com/developers/reference/data-transport-limit)
- [Dropbox content hash](https://www.dropbox.com/developers/reference/content-hash)
- [Dropbox API specification](https://github.com/dropbox/dropbox-api-spec)
- [Dropbox Rust SDK status](https://github.com/dropbox/dropbox-sdk-rust)
- [Cloudflare R2 S3 API compatibility](https://developers.cloudflare.com/r2/api/s3/api/)
- [Amazon S3 conditional deletes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-deletes.html)

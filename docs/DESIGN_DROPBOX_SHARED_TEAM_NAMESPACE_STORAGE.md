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

## 2. Schema-1 contract

Dropbox uses the current provider-neutral schema-1 storage configuration,
storage draft, and setup journal. It adds provider key `dropbox` at config
version `1`; it does not change a top-level or portable schema.

- Keep portable bundle schema 1 unchanged.
- Keep storage-marker schema 1 and its layout unchanged.
- Add no migration, compatibility reader, alias, fallback, dual writer, or
  provider-specific public command.
- Keep pre-cutover flat storage records and the old persisted `"s3"` tag
  invalid.
- Use injected temporary homes and stores in automated tests.

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
- Keep Local and R2 behavior unchanged.

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

Register Dropbox through the current provider envelope:

```rust
#[serde(deny_unknown_fields)]
struct StorageConfig {
    id: StorageId,
    name: String,
    provider: StorageProviderKey, // "dropbox"
    provider_config: StorageProviderConfig,
}

#[serde(deny_unknown_fields)]
struct StorageProviderConfig {
    version: u32,                  // 1
    settings: BoundedConfigObject, // namespace_id
    secrets: SecretMap,            // access_token
}
```

The Dropbox adapter owns typed decoding and validation of `namespace_id` and
`access_token`. Common storage types do not add a Dropbox variant.

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

For an existing Dropbox storage, a tested draft commit can replace the name or
token but not the namespace ID. A different namespace requires a new
`StorageId`. This prevents one local storage identity from silently moving to
a different remote tree.

### 6.3 Secret persistence and redaction

The access token follows the common storage-secret boundary:

- persist it only in schema-1 machine-local configuration or a restricted
  storage-setup draft under `~/.mallard`;
- use the existing canonical, no-follow, mode-`0700` root and mode-`0600`
  atomic JSON persistence on Unix;
- use the equivalent restricted file permissions on supported Windows builds;
- expose only presence for committed storage, draft listings, update
  responses, and edit drafts;
- use an explicit `unchanged` operation to keep a stored token;
- accept a replacement only through the focused draft update and tested
  commit;
- never include it in portable manifests, receipts, plans, activity records,
  logs, errors, or project files; and
- never echo a malformed token in a validation error.

An exact fetch of a new draft can return that draft's own token for review.
An edit draft never returns a token copied from committed configuration.

There is no native-vault entry and no separate Dropbox authorization document.
The existing bounded local reset removes the token with the rest of
`~/.mallard` state.

## 7. Draft and configuration rules

The existing schema-1 `StorageSetupDraft` holds the Dropbox provider envelope.
The generic storage descriptor declares one `namespace_id` setting and one
`access_token` secret. Existing focused create, update, test, commit, and
discard commands handle the draft.

Project setup can select only a committed `StorageId`. It cannot open or embed
an uncommitted storage configuration. Project setup must not copy a Dropbox
token into a second document.

For Dropbox, an edit can change the name or access token only. A namespace
change requires a new storage ID. The commit transaction must reject an edit
whose continuity coordinates differ from its stored source.

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

Add one `DropboxAdapter` to `StorageProviderRegistry`. Its `open` method
constructs an `Arc<DropboxObjectStore>` from the validated machine-local
provider configuration. `ConfiguredObjectStore` remains the existing opaque
provider-neutral wrapper.

This keeps the current construction pattern for:

- project bundle engines;
- project activity snapshots;
- managed clear; and
- the storage Library.

`DropboxObjectStore` implements the complete `ObjectStore` contract and runs
through the same wrapper as Local and R2.

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

Dropbox implements the current revision-bearing list and conditional-delete
contract. Managed clear can use deletion only when the store declares support:

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
    Conflict { current_etag: String },
}

trait ObjectStore {
    fn supports_conditional_delete(&self) -> bool {
        false
    }

    fn delete(
        &self,
        key: &ObjectKey,
        expected_etag: &str,
    ) -> Result<DeleteOutcome, String> {
        Err("conditional object deletion is not supported by this storage".to_string())
    }
}
```

There is no public delete-any variant.

- Local verifies its SHA-256 revision and deletes under its store lock.
- R2 declares no conditional-delete support. Its descriptor sets
  `supports_managed_clear` to `false`, so the UI omits managed clear and the
  common backend rejects direct clear calls before remote I/O.
- Dropbox uses the file `rev` as `parent_rev`.
- Missing is idempotent success.
- A changed revision returns `Conflict`.
- Managed clear stops on the first conflict.

Cloudflare R2 does not evaluate `If-Match` for `DeleteObject`. The R2 setup
probe therefore tests only the operations required for normal sync: bounded
list, verified read, conditional create, conditional replacement, and stale
conditional-write rejection. Its private cleanup can unconditionally delete
only the unique generated setup-probe key and must verify that the key is
absent. This cleanup is not exposed through `ObjectStore` and is not a managed
clear fallback.

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

Use the existing provider-neutral schema-1 storage setup journal. The Dropbox
adapter returns the namespace ID as its bounded, non-secret journal facts.
The common entry contains only the setup ID, provider key, config version,
stage, sanitized error code, timestamps, and facts. It contains no token,
provider response, draft, or local path.

The common setup service writes the running entry before calling the adapter.
It removes the entry after a successful probe and cleanup. On restart, it
passes matching candidate or stored credentials to the adapter's
`reconcile_probe` implementation. Failed cleanup remains a bounded
`cleanup_required` entry.

Use this reserved prefix:

```text
.mallard/v1/_setup_probes/<setup-id>/
```

The setup service uses a private validated probe-key type. It does not widen
the normal repository or Library key spaces.

### 13.1 Probe sequence

1. Validate the pinned draft and complete Dropbox provider configuration.
2. Call `/users/get_current_account` with no selector.
3. Verify the namespace is not the personal home namespace.
4. List the configured namespace root.
5. Start remote mutations only after the common layer persists the journal.
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
18. Return success only after no owned probe file remains; the common layer
    then removes the journal entry.

The two setup clients have independent HTTP state but use the same configured
user token. The live collaborator gate uses two independent user tokens.

### 13.2 Cleanup

Cleanup lists only the deterministic setup prefix. It deletes only a regular
file whose expected revision, size, Mallard SHA-256, and known probe content
all match. It never deletes a folder or unknown object.

Persist each witnessed revision before the next mutation. If a crash can have
written a file without recording its revision, require manual cleanup. Do not
guess a revision.

The probe can leave empty Dropbox folders. The UI must state this before the
test. Mallard never deletes the selected namespace or a folder during cleanup.

## 14. Commands and DTOs

Add no Dropbox-specific command. Register the Dropbox descriptor and adapter
behind the existing provider-neutral command surface:

- `create_storage_setup_draft`;
- `update_storage_setup_draft`;
- `test_storage_setup_draft`;
- `commit_storage_setup_draft`; and
- `discard_storage_setup_draft`.

Test and commit take the draft ID and expected draft revision. Commit rechecks
the draft source and configuration transaction before it writes. A failed pin
changes no configuration.

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

Hold the host-operation guard for a setup test, draft commit, token
replacement, Push, Pull, restore, Library remote operation, managed clear,
and storage removal. A competing Mallard process fails fast with the existing
busy result.

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

Add one Dropbox storage descriptor with:

- storage name;
- namespace ID;
- access token;
- **Test**; and
- **Finish setup**.

The existing descriptor-driven storage editor renders these fields and
actions. Do not add Dropbox-specific frontend branching.

The editor states:

- the user must already be a collaborator with edit access;
- the token and namespace ID must be obtained outside Mallard;
- Mallard does not authorize Dropbox or renew the token;
- an expired token must be replaced manually;
- the test can create empty `.mallard/` probe folders; and
- Mallard never deletes the selected namespace or its folders.

Committed tokens and edit-draft values stay out of frontend state. A saved
storage displays only:

```text
Dropbox · <Mallard storage name>
```

Never show a token suffix, account identity, namespace ID, or raw provider
error.

Storage cards, setup summaries, project selectors, sidebar status, the Library
selector, empty states, and error copy use the existing provider descriptors
and generic unavailable state.

## 18. Implementation layers

### Layer 1: Dropbox adapter

- Add the version-1 settings and secret decoder.
- Add the descriptor, continuity coordinates, safe summary, journal facts,
  and registry entry.
- Enforce namespace immutability for edits.
- Keep the production descriptor disabled until transport and setup tests
  pass.

### Layer 2: Dropbox transport

- Add `DropboxObjectStore`.
- Add scripted-server transport tests.
- Pass the shared `ObjectStore` conformance suite, including conditional
  deletion.
- Keep the production descriptor disabled.

### Layer 3: setup and recovery

- Implement adapter `probe` and `reconcile_probe` through the existing
  provider-neutral journal.
- Add token redaction, remote-change, and cleanup tests.
- Enable Dropbox only in developer builds.

### Layer 4: product integration

- Verify the generic editor, summaries, Project Sync, activity, managed clear,
  and Library paths through the registered adapter.
- Pass frontend and complete-app tests.
- Pass the two-collaborator live gate.
- Set list and clear limits from measured Dropbox behavior.
- Enable Dropbox in production builds.

Each layer must leave Local and R2 working. Do not merge a layer with a bypass
or disabled invariant.

## 19. Test plan

### 19.1 Domain and persistence

- Round-trip the Dropbox version-1 provider envelope through the existing
  schema-1 storage and draft documents.
- Reject unknown Dropbox settings, secrets, and config versions.
- Reject invalid namespace IDs and tokens.
- Prove committed configuration, listings, update responses, and edit drafts
  redact the token.
- Prove an exact fetch can reveal only a new draft's own token.
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

Run the mandatory `ObjectStore` conformance cases against Local, R2, and
Dropbox. Run conditional-delete conformance only for adapters that declare
support.

### 19.3 Setup and commands

- Reject a team token that cannot call User Authentication without a selector.
- Reject a personal home namespace.
- Reject a namespace without edit access.
- Prove strict create, CAS, list, and cleanup.
- Prove conditional delete for each provider that advertises managed clear.
- Prove marker creation and validation keep schema `1`.
- Prove stale draft revisions and edit-source digests fail before commit.
- Prove generic rename cannot change a token or namespace.
- Prove an existing Dropbox storage cannot change namespace ID; a new
  namespace requires a new `StorageId`.
- Prove token replacement keeps the old token after a failed test.
- Prove recovery deletes only fully witnessed probe files.
- Prove unreconciled residue retains a sanitized cleanup-required journal.
- Verify the generic commands are registered and typed and no public command
  names Dropbox.

### 19.4 Concurrency

- Prove two processes cannot run host-sensitive operations concurrently.
- Race storage removal against Library link creation.
- Prove no catalog record can reference a missing storage.
- Prove persistence and catalog locks are not held during network waits.

### 19.5 Frontend

- Prove the Dropbox descriptor renders through the generic storage editor.
- Prove Dropbox has no OAuth, Connect, Reconnect, team, actor, or namespace
  discovery UI.
- Prove committed tokens never return to frontend state.
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
- No Dropbox-specific compatibility reader or execution path exists.
- Bundle and storage-marker schemas remain `1` and unchanged.
- Local and R2 remain green.
- R2 does not advertise managed clear and sends no conditional delete.
- The Dropbox live collaborator gate passes.

## 21. Expected code areas

Implementation is expected to touch:

```text
crates/mallard-application/src/storage_setup.rs
crates/mallard-infrastructure/Cargo.toml
crates/mallard-infrastructure/src/storage_providers/dropbox.rs
crates/mallard-infrastructure/src/storage_providers/mod.rs
crates/mallard-infrastructure/src/storage_providers/registry.rs
crates/mallard-infrastructure/src/storage_providers/traits.rs
crates/mallard-infrastructure/src/storage_core/mod.rs
crates/mallard-infrastructure/src/storage_core/dropbox_store.rs
src/storageError.ts
tests/frontend/
src-tauri/src/sync_tests/
```

The generic Tauri commands, DTOs, API wrappers, setup components, Project
Setup, and Library flows must not need a Dropbox branch. If adding Dropbox
requires one, fix the registry boundary instead of adding a compatibility
path.

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

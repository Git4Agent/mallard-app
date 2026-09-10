# Dropbox storage

Status: implemented, including agent setup and refresh. Local mock and native
setup checks pass. The one-account authorization POC passed; full live storage
transport verification is pending.
Updated: 2026-09-07.

## Cutover

Keep schema 1 for local state, drafts, setup journals, storage markers, and
portable data. Use Dropbox provider config version 1. Do not add migrations,
legacy execution paths, or compatibility code. The developer will remove
local and cloud test data before retesting.

## Setup

Select **Dropbox** in Storage. Agent setup uses the same controls as R2:
**Run with Codex/Claude**, browser consent, draft review, and **Finish setup**.
The helper creates a new Mallard shared folder and obtains its namespace.
The packaged public app key and local callback remove manual setup entry.
The backend stores OAuth credentials. An exact fetch of a new draft returns
the saved refresh token for Show/Hide review, as in R2. Listings, update
responses, and edit drafts of committed storage return presence only.

Manual setup accepts two connection values:

1. **Namespace ID**: an existing shared folder or team namespace.
2. **Access token**: a user-linked token with write access to that namespace.

Mallard suggests a storage name. The user can edit the name. The token is
masked and stored only in machine-local secret storage. The namespace stays
fixed after commit. To use another namespace, add another storage connection.
A token replacement keeps the same storage identity and links.

Use a Dropbox app with Full Dropbox access and these scopes:

- `account_info.read`
- `files.metadata.read`
- `files.content.read`
- `files.content.write`

The schema-1 secret map contains exactly one `access_token` for manual setup
or one `refresh_token` for agent setup. Manual tokens must be replaced when
they expire. OAuth access renews before use on the wall clock, with one
refresh in flight per open transport; a read that Dropbox answers with
`expired_access_token` is sent once more after a refresh, a write never is.
A credential Dropbox rejects is recorded beside it with the route, status,
error identifier, and request ID, and the process-shared transport refuses
further requests until the credential changes or a probe succeeds.
Reconnection uses the existing namespace and creates no folder. Refresh
failures do not fall back to another credential or replay an uncertain
mutation. Mixed and unknown secret keys are rejected.

Mallard does not select team members or use a personal home namespace. All
machines must use the same namespace ID. Each user can authorize their own
access. Creation of a new shared folder also requires `sharing.write`.
Full Dropbox authorization covers more than that folder.

The [agent setup plan](PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md) describes the
implemented CLI, callback, refresh, draft handoff, and provisioning journal.
The [authorization POC](../poc/dropbox-agent-setup/README.md) passed with one
live account on 2026-09-07. Full storage transport and two-account live
verification remain pending.

## Stored data and limits

The selected namespace is the root. Mallard uses its existing `.mallard/`
layout for project bundles, Library resources, and the storage marker.
Identity is the provider key plus the exact, case-sensitive namespace ID.
Tokens and display names are not identity coordinates.

Dropbox uploads use concurrent `files/upload_session/start` requests with
complete bodies and `close: true`, followed by `files/upload_session/finish_batch_v2`.
Each window uses the existing transfer setting: six objects by default,
sixteen maximum. Single-object and conditional writes use the same path.
There is no direct-upload fallback or persistent session state.

The maximum remains **150 MiB (157,286,400 bytes) per object**, inclusive.
A larger object is rejected before a request starts. All input keys, sizes,
and modes are checked before the first network request, including later
windows. Equal duplicate immutable keys share a transfer; unequal bytes for
one key fail preflight. Objects are not split.

This limit applies only to Dropbox uploads. Local-folder, R2, capture, and
caller-supplied download limits do not change. A payload archive is one object;
its archive size must meet the Dropbox limit.

## Transport contract

- Production requests use the fixed Dropbox API and content hosts over HTTPS.
- File requests specify `Dropbox-API-Path-Root` in `namespace_id` mode.
- Redirects and automatic request retries are disabled. Rate limits return a
  fixed error. The user can retry after waiting.
- Immutable writes use strict `add`, with autorename disabled. An existing
  object is accepted only when one `files/get_metadata` reports a file of the
  same size and Dropbox content hash as the staged bytes; no body is
  downloaded to prove presence.
- Mutable writes use strict `add` or revision-checked `update`. A lost response
  is an unknown outcome. Matching bytes do not prove that this writer won.
- Payload downloads stream to private temporary files; small control objects
  are read into a bounded buffer. Size, Dropbox content hash, and the expected
  standard SHA-256 are checked before use. Bulk reads run under the transfer
  concurrency setting and keep input order.
- One store per storage configuration is shared for the process lifetime: its
  connection pools, refreshed token, and verified storage marker survive
  across Compare, Push, and Pull.
- Deletion always uses a file path and `parent_rev`. Folder deletion is never
  sent. Managed clear preserves the marker and folders.
- Keys and returned component names must be canonical lowercase ASCII paths.
  Case aliases, duplicate list entries, and unsupported file types fail closed.
- Ancestor folders are never validated. Dropbox resolves a path to exactly
  one entry regardless of case, so a folder walk could not change which bytes
  a read returns or where a commit lands. Every response still proves its own
  entry by exact `path_lower` and `name`. Dropbox creates missing parent
  folders on commit with the canonical case supplied; a file where a folder
  belongs answers `not_found` below it and rejects commits below it with
  `conflict/file_ancestor`, which maps to an invalid-path error.
- Session uploads are bounded and cancelled and drained on failure. A failed
  window starts no later window. Partial immutable commits do not advance the
  publication head; their unreferenced objects are not deleted automatically.
- Session IDs remain in memory, are limited to 16 KiB, and never enter logs.
  Commit requests and responses are bounded by the 4 MiB RPC limit.
- Listing follows provider pages, checks bounds, sorts keys, and returns a
  Mallard lexical cursor. Provider cursors do not leave the transport.
- RPC bodies are capped at 4 MiB. Listings are capped at 100,000 entries,
  32 MiB of retained entry data, 200 pages, and 120 seconds. Request timeouts
  also bound connection, read, and total request time.
- Errors contain fixed text. Tokens, response bodies, and HTTP diagnostics
  do not enter user errors or logs.

## Setup probe and recovery

Setup checks the user account, rejects personal home storage, and tests strict
create, current and stale updates, identical-byte stale updates, read integrity,
revision-checked delete, delete/recreate conflicts, and listing.

Each probe uses a validated random setup ID beneath
`.mallard/v1/_setup_probes/`. The schema-1 setup journal holds destination facts
and separate recovery data. Recovery data contains only a known revision,
size, and SHA-256. It never contains tokens or HTTP responses.

The probe saves its recovery record before a mutation and records each
acknowledged write revision. Cleanup downloads and verifies the exact file,
then deletes it with that revision. Unknown or changed revisions require
manual cleanup. No write is adopted as probe-owned from matching bytes alone.
Empty probe folders can remain. A successful probe leaves no probe file.

The host operation guard covers setup, storage removal, and Library link
creation. Cross-document checks run outside document locks.

## Code

- [Dropbox adapter](../crates/mallard-infrastructure/src/storage_providers/dropbox.rs)
- [Dropbox transport](../crates/mallard-infrastructure/src/storage_core/dropbox_store.rs)
- [Setup probe](../crates/mallard-infrastructure/src/storage_core/dropbox_store/probe.rs)
- [Transport tests](../crates/mallard-infrastructure/src/storage_core/dropbox_store/tests.rs)
- [Provider registry](../crates/mallard-infrastructure/src/storage_providers/registry.rs)
- [Setup application service](../crates/mallard-application/src/storage_setup.rs)
- [Setup journal](../crates/mallard-infrastructure/src/storage_setup_journal.rs)
- [Storage form](../src/components/project-sync/StorageSettings.tsx)

No Dropbox-specific Tauri commands or frontend provider branches are needed.
Project and Library engines use the existing `ObjectStore` contract.

## Verification

The workspace library suite passed: 883 tests, with 5 ignored. The frontend
build and all 290 frontend tests passed. The native macOS build and the initial
and connected Dropbox setup cards passed with an injected temporary home and
synthetic credentials. The token refresh path also passed the local storage
probe and verified object operations. See the
[agent setup plan](PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md) for test scope and
remaining live checks.

Local check commands:

- `cargo test -p mallard-infrastructure --lib`
- `cargo test -p mallard-application --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `npm run build`
- `npm run test:frontend-integration`
- Native Tauri setup screen with a temporary home.

### Local HTTP mock

[MockDropboxServer](../crates/mallard-infrastructure/src/storage_core/dropbox_store/mock.rs)
is available only in unit tests or with the `test-support` feature. It binds
to loopback on a random port and accepts fake test tokens. Production builds
keep the fixed Dropbox hosts; there is no mock endpoint setting in stored
configuration and no schema change.

The mock retains files and revisions across two users. Tests cover:

- Setup test, commit, token replacement, and namespace-change rejection.
- Project publication, discovery, reviewed restore, and byte verification.
- Library publication, cross-user reads, and stale-write rejection.
- Immutable writes, conditional writes and deletion, and managed clear.
- Missing type tags on content responses, invalid types, and symlinks.
- Upload boundaries, full-input preflight, content hashes, and pagination.
- Concurrent windows, stable result order, and cancellation after staging errors.
- No folder probes or folder creation requests on any read or write; a file
  where a folder belongs fails closed.
- Partial commits and malformed results without head advancement or replay.
- The 39-object fixture: 39 session starts, 7 batch commits, 46 requests, and
  no `files/get_metadata` at concurrency six. Bulk reads reach the transfer
  concurrency bound and keep input order. These are local HTTP mock results,
  not live throughput measurements.
- Lost write responses, durable probe recovery, and cleanup refusal.
- Redirects, rate limits, and sanitized permission errors.

Run focused tests with:

```sh
cargo test -p mallard-infrastructure --lib dropbox
cargo test -p mallard-application --lib dropbox
cargo test --manifest-path src-tauri/Cargo.toml --lib dropbox
```

### GitHub review

The search found Dropbox's [official API specification](https://github.com/dropbox/dropbox-api-spec)
and [Swift SDK transport mocks](https://github.com/dropbox/SwiftyDropbox#mock-api-responses-in-tests).
The Swift mocks are internal to that SDK's tests. The
[Rust SDK](https://github.com/dropbox/dropbox-sdk-rust#tests) uses generated
serialization tests and states that it has no formal Dropbox support.

Use the existing Rust HTTP test stack for Mallard's stateful mock. The mock
response shapes follow the official specification. This tests real HTTP
requests from Mallard without adding an external server dependency.

### Live checks

Live checks need two user-linked test tokens for one writable shared namespace.
Run the [live POC](../poc/dropbox-shared-team-namespace/README.md), then test
project Push/Pull, Library Push/Pull, token replacement, and managed clear in
Mallard. A local mock result does not prove live Dropbox behavior.

## API references

- [File API specification](https://raw.githubusercontent.com/dropbox/dropbox-api-spec/main/files.stone)
- [Authentication types](https://www.dropbox.com/developers/reference/auth-types)
- [Dropbox content hash](https://www.dropbox.com/developers/reference/content-hash)

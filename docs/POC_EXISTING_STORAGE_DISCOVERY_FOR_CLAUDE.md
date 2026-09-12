# Existing R2 and Dropbox storage discovery: POC handoff for Claude

Date: 2026-09-10. Status: live discovery passed for both providers.

The user asked whether Mallard can find existing storage in R2 and Dropbox.
The user requested a POC first and approved live checks for both providers.
The POC is complete. This document records the result and proposed integration
work. It does not record approval to implement the full feature.

The app can connect to a known destination. It does not yet provide account
discovery. The CLI command `storage list` lists saved local connections.
The standalone POC lists remote candidates and reads their storage markers.
Its code is under [poc/storage-discovery](../poc/storage-discovery/README.md).

The final live scans produced these results:

| Provider | Scope | Candidates checked | Valid Mallard markers | Missing markers | Read errors |
| --- | --- | ---: | ---: | ---: | ---: |
| R2 | One account, default jurisdiction | 15 | 6 | 9 | 0 |
| Dropbox | One user, accessible shared folders | 8 | 2 | 6 | 0 |

Both scans returned `SCAN_COMPLETE`. The two Dropbox matches were mounted
folders with owner access. The POC made no storage-object writes and no
Mallard configuration writes. Tokens and raw provider responses were not
saved. Wrangler manages its own login cache.

The first Dropbox run completed consent but failed the POC's strict scope
check. The final run requested and verified all four read scopes listed
below. It passed. The first rejected scope set was not retained. Do not
state an exact cause for that mismatch.

Local verification passed: 35 Node tests, the mock command, JavaScript syntax
checks, and whitespace checks. This was a standalone POC. Production Rust,
Tauri, and frontend integration were not part of that verification.

Live coverage is limited to the accounts and destinations above. Local tests
cover pagination, duplicate namespaces, renamed storage, access errors,
unmounted shares, a team root, EU/US handling, and scan bounds. There was no
live check of another account, an unmounted share, a team space, or another
R2 jurisdiction. A valid marker does not prove write access or bundle integrity.

The common detection rule is to read this exact object at a candidate root:

```text
.mallard/_storage.json
```

```json
{"format":"mallard-storage","layout_version":1}
```

Read at most 4 KiB. Check the format and supported layout version. Keep
missing, invalid, unsupported, and unreadable results separate. Names such as
`mallard-…` or `Mallard …` are not proof of storage. A new bucket or folder
can lack a marker until Mallard initializes the layout.

For production, reuse `StorageMarker` and its validation in
[storage_core/marker.rs](../crates/mallard-infrastructure/src/storage_core/marker.rs).
Do not call `ensure_storage_marker()` during discovery: it creates a marker
when one is absent. Read the marker and validate it without a write. The
Node parser is a POC implementation, not the production validation authority.

The marker has no unique storage ID, display name, credentials, or local
trust state. It cannot restore another machine's connection record. Use the
provider destination to identify storage: account + jurisdiction + bucket for
R2; namespace ID for Dropbox. Display names are labels, not identity.

The R2 POC uses these read operations:

```text
GET https://api.cloudflare.com/client/v4/accounts/{account_id}/r2/buckets
GET https://api.cloudflare.com/client/v4/accounts/{account_id}/r2/buckets/{bucket}/objects/.mallard/_storage.json

Authorization: Bearer <in-memory Cloudflare token>
cf-r2-jurisdiction: <selected jurisdiction>
```

The list request uses `per_page=100`. Continue with `result_info.cursor`.
Keep the same account and jurisdiction for each page and marker read. Send
literal slashes in the object key. Do not filter by bucket name.

The live check used Wrangler's current OAuth login. The helper captures
`wrangler whoami --json` and `wrangler auth token --json` in memory. It
requires one account or an explicit selection from the returned account list.
It does not print either command's raw output or start login automatically.

A saved S3 key can lack bucket-list permission. Discovery needs account
access that permits listing and marker reads. Cloudflare's Admin Read only
permission supports these operations. The discovery token is not a saved
S3 connection credential. See [R2 permissions](https://developers.cloudflare.com/r2/api/tokens/),
[List Buckets](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/list/),
and [Get Object](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/subresources/objects/methods/get/).

Each POC run scans one jurisdiction: `default`, `eu`, `us`, or `fedramp`.
The current Rust adapter accepts default, EU, and FedRAMP. The POC marks US
matches as unsupported by the app. Verify the current adapter and credential
flow before adding jurisdiction support.

The Dropbox browser grant uses PKCE with these scopes:

```text
account_info.read files.metadata.read files.content.read sharing.read
```

It uses Mallard's public app ID and registered callback at
`http://127.0.0.1:53682/dropbox/callback`. It requests `token_access_type=online`.
The POC accepts a short-lived token with the requested read scopes. It rejects
added write scopes and a refresh token.

The Dropbox scan uses these operations:

```text
POST https://api.dropboxapi.com/2/users/get_current_account
POST https://api.dropboxapi.com/2/sharing/list_folders
POST https://api.dropboxapi.com/2/sharing/list_folders/continue
POST https://api.dropboxapi.com/2/sharing/list_mountable_folders
POST https://api.dropboxapi.com/2/sharing/list_mountable_folders/continue
POST https://content.dropboxapi.com/2/files/download
```

These POST routes read data. They do not create or mount a folder. Exclude
the personal home namespace. Add a distinct team root when account metadata
reports one. Follow each list's cursor and remove duplicate namespace IDs.
For marker download, use these headers:

```text
Dropbox-API-Path-Root: {".tag":"namespace_id","namespace_id":"<candidate namespace>"}
Dropbox-API-Arg: {"path":"/.mallard/_storage.json"}
```

A Dropbox 409 response means a missing marker only when its structured error
is `path/not_found`. Other failures remain unreadable results. An unmounted
share can be listed but fail its marker read. Keep that result; do not mount
the folder. A viewer role does not prove permission to Push. This scan does
not enumerate every namespace in a Dropbox team or search personal file trees.
See the [sharing API specification](https://github.com/dropbox/dropbox-api-spec/blob/main/sharing.stone),
[file access guide](https://developers.dropbox.com/dbx-file-access-guide),
and [OAuth scope guide](https://dropbox.tech/developers/customizing-scopes-in-oauth-flow).

Mallard's current `SETUP_SCOPES` includes `sharing.write` but omits
`sharing.read`. Its saved-storage token refresh requests only `STORAGE_SCOPES`,
which also omits `sharing.read`. Do not assume a refreshed storage token can
run discovery. The POC's online read token is also insufficient for a durable
sync connection. After selection, use the existing storage authorization
flow for that exact namespace.

The POC code is divided as follows:

| File | Responsibility |
| --- | --- |
| [storage-discovery.mjs](../poc/storage-discovery/storage-discovery.mjs) | CLI, live/mock routes, exit codes |
| [discovery.mjs](../poc/storage-discovery/discovery.mjs) | Provider lists, marker reads, candidate results, safe report |
| [auth.mjs](../poc/storage-discovery/auth.mjs) | Wrangler capture, Dropbox PKCE and callback, scope checks |
| [http.mjs](../poc/storage-discovery/http.mjs) | Bounded reads, timeouts, fixed errors, marker check |
| [mock.mjs](../poc/storage-discovery/mock.mjs) | Synthetic provider responses and POC assertions |
| [storage-discovery.test.mjs](../poc/storage-discovery/storage-discovery.test.mjs) | 35 tests, including a real loopback callback with a mock token exchange |

The internal result holds candidate names and destinations in memory. CLI
output contains counts, temporary references, roles, mount state, and fixed
errors. A reference such as `r2-1` applies only to that scan. It is not a
connection ID. Preserve this distinction in a future UI.

`SCAN_COMPLETE` exits with 0, including a scan with no matches.
`SCAN_PARTIAL` exits with 2 and retains earlier matches after a failed page,
unreadable candidate, or scan limit. `SCAN_FAILED` exits with 1 when the scan
cannot start. Do not show a partial result as an empty successful scan.

The limits are 100 pages per list route, 1,000 unique candidates, 1 MiB per
list response, 4 KiB per marker, and 16 KiB per error response. At most four
marker reads run together. Each request, including its body, has a 15-second
limit. A scan has a 120-second deadline. Browser consent has a separate
180-second limit. Repeated cursors stop the list. API redirects and automatic
retries are disabled.

Run these commands from the repository root with Node 20.19 or later:

```sh
node poc/storage-discovery/storage-discovery.mjs mock
node --test poc/storage-discovery/storage-discovery.test.mjs
node poc/storage-discovery/storage-discovery.mjs r2 --wrangler
node poc/storage-discovery/storage-discovery.mjs dropbox --browser
```

The [POC README](../poc/storage-discovery/README.md) describes explicit account
selection, jurisdiction selection, and environment-token input. Keep tokens
out of chat, command arguments, source files, and shell history. The original
live commands needed execution outside the sandbox to use Wrangler and open
the system browser. Automatic approval review allowed those read-only runs.
Do not bypass an execution restriction by changing a provider home.

If app integration is requested, use the following existing code:

| Code | Reuse and constraint |
| --- | --- |
| [CLI backend](../crates/mallard-cli/src/backend.rs) | R2 setup already uses a draft's existing `api_url` and skips bucket creation. Dropbox setup already uses an existing `namespace_id` and skips folder creation. |
| [R2 browser setup](../crates/mallard-cli/src/r2_browser_setup.rs) | `WranglerClient` handles account login and OAuth access. `authorize_bucket()` obtains credentials for a named bucket. Recheck account and jurisdiction handling. |
| [Dropbox auth](../crates/mallard-infrastructure/src/dropbox_auth.rs) and [browser callback](../crates/mallard-cli/src/dropbox_browser_setup.rs) | Reuse PKCE, bounded responses, token handling, and callback checks. Add discovery scope handling without assuming storage refresh grants include it. |
| [Dropbox setup](../crates/mallard-infrastructure/src/dropbox_setup.rs) | `prepare_namespace()` accepts an existing namespace and verifies access. Keep the selected namespace fixed. |
| [Storage setup service](../crates/mallard-application/src/storage_setup.rs) | `begin_storage_draft_operation()` pins the draft. `prepare_storage_draft_in_operation()` saves settings and secrets with revision checks. The user still commits through the normal setup flow. |
| [Storage provider adapters](../crates/mallard-infrastructure/src/storage_providers/mod.rs) | Keep provider discovery and destination validation within the current provider architecture. |
| [Storage UI](../src/components/project-sync/StorageSettings.tsx) and [API boundary](../src/components/project-sync/api.ts) | Add the picker through the existing typed API. Keep stale-response and concurrent-request checks. |

The proposed product flow is **Find existing storage → authorize discovery →
select a verified destination → prepare the setup draft → review → Finish
setup**. Keep discovery read-only. Credential creation and draft preparation
belong after explicit destination selection. Revalidate that destination and
its marker before connection. Keep normal connection checks and user approval.

Implement in small steps after approval of that work:

1. Add bounded Rust discovery for both providers. Reuse current auth,
   destination types, marker validation, and error handling. Keep the Node
   code as a protocol POC; do not ship it as the app backend.
2. Add typed commands and candidate results. Match saved connections by
   provider destination. Keep secrets out of scan DTOs and activity logs.
3. Add the selection UI. Show access limits and incomplete scan status.
   Preserve draft revision checks through credential preparation.
4. Run relevant Rust, CLI, frontend, and Tauri checks. Verify the complete
   desktop flow with temporary test state. Capture a screenshot for UI review.

Acceptance requires zero storage writes during discovery; detection of renamed
storage; explicit handling of missing, invalid, unsupported, and unreadable
markers; bounded pagination; exact destination selection; stale-result
rejection; and user approval before committing a connection. Do not add a
portable-manifest migration or weaken restore, digest, or trust checks.

The worktree contains unrelated changes. At handoff, the POC directory is
untracked. Read `AGENTS.md` and current source before editing. Preserve the
other work and include the POC files if the user later requests a commit.

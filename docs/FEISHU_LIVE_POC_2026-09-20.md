# Feishu live storage PoC

Date: 2026-09-20. Target: Feishu China. Data: synthetic files only.

**The desktop file round trip passed. Production support remains unproven.**
The Feishu desktop app uploaded and downloaded all eight fixture files.
Every relative path, byte count, and SHA-256 digest matched.

This extends the [feasibility report](FEISHU_STORAGE_FEASIBILITY_2026-09-19.md).
The [saved results](../poc/feishu-drive/evidence/desktop-2026-09-20.json)
contain expected paths and hashes. They contain no credentials or account
identifiers.

## Live desktop checks

The test used a new folder, `mallard-feishu-desktop-poc-85e6cfdbbf196e6a`.
The test folder remains in Drive. No user file was uploaded or deleted.

| Check | Result |
| --- | --- |
| Upload Folder | Eight of eight files completed |
| Download Folder | Eight of eight files completed |
| Exact relative paths | All eight matched; no missing or extra files |
| Binary bytes | All eight sizes and SHA-256 digests matched |
| Hidden directory | `.mallard` and its nested record directory survived |
| Storage marker | `_storage.json` retained all 54 bytes |
| Large file | 20,971,537 bytes, or 20 MiB + 17 bytes, retained |
| Long record names | The 223-character child record names remained exact |
| Publication selection | Current Rust code selected the expected two-record chain |
| Losing sibling | Remained present, with its original name and bytes |

The record set contains one root and two competing children. Selection ran
locally on the downloaded record names through the current `RecordName`
and `Mainline` code. The bodies are transport fixtures, not complete bundle
manifests. This is a static record-set check, not a concurrent Push test.

The desktop app downloaded ordinary files directly to a local folder.
Its warning about downloading online documents as URL files did not apply
to these binary and JSON fixtures.

An identical same-name upload was attempted after the round trip. The UI
did not give a separate completion result. Do not treat that attempt as
proof of duplicate-file behavior. The API probe still must test duplicate
names and compare every returned file token.

## Developer app

The signed-in API Explorer initially had no app. Its token and request
controls were disabled. A private enterprise app was then created with
these values:

- Name: `mallard`.
- Logo: the existing `public/mallard-logo-duck.png`, 1000 by 1000 pixels.
- Description: “Feishu Drive integration for Mallard to synchronize project
  sessions and provider state across devices.”

The user approved these four **user-identity** permissions. The developer
console then showed each permission as enabled:

| Scope | Purpose |
| --- | --- |
| `space:document:retrieve` | List a folder's contents |
| `space:folder:create` | Create folders |
| `drive:file:upload` | Upload files, including multipart uploads |
| `drive:file:download` | Download original file bytes |

No tenant-identity, delete, messaging, or sharing scope was added. User
scopes follow the user's accessible data; they do not enforce a single
test-folder boundary. The PoC must supply the dedicated folder token.

The user also approved the consent page's fifth permission, “Get user's
identifier” (`auth:user.id:read`). The API Explorer trusted redirect was
added after approval. No refresh-token permission was requested.

Before publication, authorization returned to API Explorer, but its token
exchange failed. The response had HTTP 200, wrapper code `20021`,
and service status `1108012: invalid client auth method`. The user-token
field remained empty. A fresh browser tab reproduced the failure.
The [saved status](../poc/feishu-drive/evidence/api-explorer-2026-09-20.json)
contains no response credentials or account identifiers.

This is an API Explorer authentication failure. It is not evidence that
Drive rejects the approved permissions. No Drive API write ran through
this flow. No console setting for the reported authentication method was
found.

The user then published version `1.0.0`. The developer console showed the
version as published, the app as enabled, and all changes as published.
API Explorer was reloaded and the existing permissions were authorized
again. Its token endpoint returned the same HTTP 200 response with
wrapper code `20021` and service status `1108012`. The returned user token
was empty. Publication did not resolve this failure. No Drive API write
ran during the retry. The
[post-publication status](../poc/feishu-drive/evidence/api-explorer-published-2026-09-20.json)
records this result separately from the earlier attempt.

A direct [OAuth v3 probe](../poc/feishu-drive/oauth-probe.mjs) uses the
documented client-secret exchange, random state, and PKCE S256. It checks
the exact five returned scopes before it starts the Drive probe. The user
approved the local callback and App Secret use. The callback was saved in
the developer console.

Chrome blocked the local form submission with `ERR_BLOCKED_BY_CLIENT`.
The same failure occurred with synthetic credentials. A local HTTP check
then confirmed that the process was still in `ready` state and had not
started authorization. The process was stopped. No direct OAuth exchange
or Drive API write ran. Browser security settings were not changed.
The [saved status](../poc/feishu-drive/evidence/local-oauth-2026-09-20.json)
records this boundary. The source of the browser block remains unknown.

The credential form was then removed. The current direct flow reads the
App Secret at a terminal prompt with no input echo. The browser only opens
Feishu consent and returns through the GET callback. The local process
exchanges the one-use code and App Secret at
`https://accounts.feishu.cn/oauth/v3/token`. It uses form-encoded
`client_secret` authentication, with no HTTP Basic Authorization header.
This follows the [current Feishu token contract][oauth]. API Explorer is
not part of this route. The failed Explorer response does not establish
which client authentication method its service used.

Nine local OAuth tests pass. A real terminal check also accepted the
synthetic secret without echoing it. The computer-use tool denied access
to Terminal for safety reasons. The live flow therefore awaits App Secret
entry by the user. The local callback remains registered. No live direct
token exchange is established yet.

The browser extension could not upload the logo through its file-chooser
API. The native Chrome file picker completed the same upload. No extension
permission was changed.

## Current non-CAS behavior

The model still needs put, get, and complete listing. It does not need a
conditional head update. SHA-256 verification and schema 1 remain intact.

The current [`observe_linked_bundle` implementation](../src-tauri/src/project_sync/service.rs)
also handles a Push that was confirmed and later outranked. It returns
affected scope bases to the shared selected publication and removes the
losing session acceptance. Read cursors and cloud ID mappings remain.
The next Push can publish those sessions again. This behavior was checked
in source; the complete application rollback flow was not run in this PoC.

Rollback does not establish Feishu listing consistency. Cold-client
visibility, complete pagination, competing writes, and late responses
still require API evidence. Desktop success cannot prove the documented
`upload_prepare` / `upload_part` / `upload_finish` transport, even though a
file larger than the small-upload limit survived the desktop round trip.

## Local checks

The checks ran against the current working files:

| Check | Result |
| --- | --- |
| Node transport and fault suite | 31 passed, 0 failed |
| Existing-token loopback form | 2 passed, 0 failed |
| Direct OAuth v3 loopback flow | 9 passed, 0 failed; mocked token endpoint |
| Real terminal secret entry | Synthetic input accepted; input not echoed |
| Desktop verifier regression | Passed; rejects changed bytes and a renamed winning record |
| Rust publication tests | 9 passed, 0 failed |
| Download verifier | Eight exact files; three records; expected chain selected |

Commands:

```sh
node --test poc/feishu-drive/feishu-drive.test.mjs
node --test poc/feishu-drive/desktop-fixtures.test.mjs
node --test poc/feishu-drive/browser-probe.test.mjs poc/feishu-drive/oauth-probe.test.mjs
cargo test --offline --locked -p mallard-domain publication::tests --lib
node poc/feishu-drive/desktop-fixtures.mjs prepare
node poc/feishu-drive/desktop-fixtures.mjs verify "$MANIFEST" "$DOWNLOADED_ROOT"
```

`prepare` creates a new temporary tree and prints its locations. Set
`MANIFEST` to its `expected.json` file. Set `DOWNLOADED_ROOT` to the new
folder produced by Feishu's download, not the upload source. The verifier
checks exact paths, sizes, hashes, and the selected record chain. It exits
with failure if any check fails.

The reusable [API probe](../poc/feishu-drive/README.md) remains the next
live test. Production OAuth, two-machine behavior, capacity, Library
sync, and a Rust Feishu adapter remain outside this evidence.

[oauth]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/get-user-access-token-v3

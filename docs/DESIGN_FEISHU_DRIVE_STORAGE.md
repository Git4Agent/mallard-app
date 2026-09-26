# Feishu Drive storage

Status: implemented on the mock, 2026-09-23; live checks pending (see
[the two-machine pass](LIVE_TWO_MACHINE_PASS.md), part 6). Plan:
[PLAN_FEISHU_DRIVE_STORAGE.md](PLAN_FEISHU_DRIVE_STORAGE.md).

Feishu Drive is a storage provider beside Mallard Cloud, R2, Dropbox, and
the local folder. A storage is one Drive folder, reached through Mallard's
own Feishu app. Mallard runs no service on the path: sign-in and every
request travel between the computer and Feishu, Mallard can always fetch
from Feishu with nothing of Mallard's in the way, and every piece of Feishu
state lives under `~/.mallard`.

## Authorization

- Mallard's own app, installed in the customer's tenant by its Feishu
  admin ([guide](FEISHU_APP_SETUP_GUIDE.md)); the user types nothing.
  Feishu's OAuth v3 token endpoint requires the app's `client_secret` for
  every developer-created app, and PKCE supplements it, so the release
  build carries the App ID and App Secret (`FeishuApp::built_in`, from
  `MALLARD_FEISHU_APP_ID` and `MALLARD_FEISHU_APP_SECRET` at build time).
  A build without them still offers Feishu and says so on the Sign in
  line; a development build reads the same two names from its environment.
  The storage's configuration holds no secret at all.
- The secret can be read out of a shipped binary, and that is harmless by
  construction: the app holds no tenant-identity scopes, so the secret
  alone reaches no data; the redirect is fixed to the loopback port, so a
  leaked secret cannot complete a sign-in from anywhere else; and a refresh
  needs the owner-only connection document, which only local access reads.
  The guide keeps those three facts true. This is the public-client
  posture of RFC 6749 §2.1 and RFC 8252 §8.5: the secret identifies the
  app, and PKCE with the loopback redirect protects the sign-in. A rotated secret refuses old
  builds with "Feishu refused Mallard's app · update Mallard" and never
  suspends the storage; updating restores it without a new sign-in.
- Sign-in is OAuth v3 with PKCE S256 and a random state, returning to
  `http://127.0.0.1:53683/feishu/callback` through the shared loopback
  receiver. The link is handed to the page to copy or open; Mallard never
  opens it by itself. The exchange must return exactly the six scopes
  (`offline_access` proved by the refresh token); anything else is refused
  and nothing is saved.
- The refresh chain lives in `~/.mallard/feishu_connections/<connection_id>.json`
  (`feishu_connections.rs`): the current refresh token, `reauthorize_at`,
  `refresh_uncertain`, `updated_at`. Schema 1, at most 16 KiB, atomic,
  opened no-follow, mode 0600, under one lock per state root. The desktop
  installs the store at startup; the CLI installs none and cannot open a
  Feishu storage. The document never enters `sync_config.json`, a manifest,
  a backup, or a log; rotation never touches the config, so the shared
  store's pinned configuration stays valid.
- Refresh (`feishu_auth.rs`, `FeishuAuthorization`): when fewer than 60 s
  remain the transport refreshes under its lock, one in flight per
  storage. The document is written with `refresh_uncertain: true` before
  the request and once more with the rotated token and `false` after a
  definitive answer. A lost answer leaves the flag; the next attempt sends
  one settling request, and Feishu's `20073` (token already used) means the
  earlier request landed: the storage reads Reconnect. A request that never
  reached Feishu clears the flag. `reauthorize_at` is set at sign-in from
  `refresh_token_expires_in` (the 365-day limit from consent) and never
  extended; past it nothing is sent. A document write failure fails one
  request with "Mallard could not save the renewed Feishu connection" and
  keeps the grant in memory.
- Health: `feishu_reconnect_required` (revoked, used, expired chain,
  missing document), `feishu_permission_denied`, `feishu_unreachable`,
  `feishu_app_rejected` (Mallard's app, not the user), `feishu_request_failed`,
  `feishu_connection_not_saved`. Evidence is the route literal, the HTTP
  status, the fixed Feishu code, and a short request ID.

## Transport

`storage_core/feishu_store.rs`, `FeishuObjectStore`:

- One request in flight per store, 200 ms apart; uploads and downloads are
  never concurrent, as Feishu documents. Objects up to 20 MiB use
  `upload_all`; larger ones prepare, parts, finish; the per-object cap is
  100 MiB.
- Logical keys are unchanged. Below the storage folder three families get
  one shard folder each: `_objects/<first two hex>/`,
  `_manifests/<revision ÷ 1000, seven digits>/`,
  `_commits/<bundle>/<revision ÷ 1000, seven digits>/` (and the same three
  in a Library repository). `_tag.json`, `_claims/`, and the marker are
  unsharded. Listings drop the shard component; a file in the wrong shard,
  or any name Mallard never writes, is reported and skipped.
- Same-named sibling folders are one logical directory: reads take the
  union, writes go to the folder with the smallest token, nothing is
  deleted. A same-named file is read from the smallest token; every read
  verifies size and SHA-256.
- A present name is the object. For a payload the name is its digest and
  the bytes are checked against it before presence counts. An upload whose
  answer was lost is settled by one listing of its folder, never resent.
  A record is one `upload_all` after the engine's own listing, never
  retried.
- `supports_claims` and `supports_conditional_delete` are false; clear is
  unavailable; no delete scope is requested. The setup probe writes
  `.mallard/v1/_setup_probes/<setup_id>.bin`, reads it back, and finds it
  in a listing; the file stays.
- Not automatic, by decision: no wait on a rate limit or an unreachable
  answer, no multipart resume, no reconnect, no rollback of local
  acceptance.

## Setup

A new draft asks Create new or Use existing, from the descriptor's
`setup_doors` (Feishu declares no `discovery`: it cannot look through the
Drive). The lane (`FeishuSetup.tsx`) then runs Sign in → Account → Folder →
Access → Review. The Folder line holds the door's one field: a proposed
`Mallard <word>` name for a new folder in My Space, or a pasted folder link;
the backend mode stays `create`, and only that field travels. The Access line
saves the document on this computer. No Manual setup row is offered: the
descriptor declares no `manual_setup_hint`, because a hand-typed folder token
has no connection document behind it. The Tauri commands
(`project_sync/feishu_setup.rs`) keep the grant in one ephemeral session;
the application (`feishu_setup.rs`) writes the document, then prepares the
exact draft.
A reconnect keeps the connection ID and the folder and overwrites the
document. Removing the storage deletes the document and nothing remote.

Settings: `{ connection_id, folder_token, folder_name?, account_name }`,
no secrets; identity is `folder_token`. The storage page shows the folder,
the account, and the date a fresh sign-in is required by.

## Boundaries

- Feishu's consent covers every Drive folder the user can reach. The
  sign-in on this computer can read and write them; folder choice is
  Mallard's convention.
- Two computers never share a refresh chain: each sign-in yields its own.
- Capacity: 1,500 children per folder, 400,000 nodes per space, 15 levels;
  5 uploads or downloads per second and 10,000 per day.
- Feishu China origins only (`open.feishu.cn`, `accounts.feishu.cn`).

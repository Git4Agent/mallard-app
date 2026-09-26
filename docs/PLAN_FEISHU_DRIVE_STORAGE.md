# Feishu Drive storage provider

Date: 2026-09-23; D1 settled on 2026-09-24 after four revisions (Mallard's
own app, no service, the key on the computer), D4 revised once more, D11
added. Status: steps 1 to 6 implemented on the mock; step 0 and the live
rows of step 6 are the owner's ([design](DESIGN_FEISHU_DRIVE_STORAGE.md),
[app guide](FEISHU_APP_SETUP_GUIDE.md)). Base: `a49c7eb` plus the uncommitted CAS-free working
tree.

Feishu Drive becomes Mallard's fifth storage provider beside Mallard Cloud,
R2, Dropbox, and the local folder. The
[feasibility report](FEISHU_STORAGE_FEASIBILITY_2026-09-19.md), the
[desktop round trip](FEISHU_LIVE_POC_2026-09-20.md), and the
[CAS-free rerun](FEISHU_CAS_FREE_POC_2026-09-23.md) established the
transport fit. The CAS-free storage protocol is unchanged by this plan.

**Decoupling rule.** Feishu sync and sign-in involve two parties only: the
desktop and Feishu. Mallard Cloud, its worker, its database, and its
accounts hold nothing about Feishu, and no other Mallard-operated service
exists on the path, so Mallard can always fetch from Feishu. A Feishu
storage never references Mallard Cloud. Every piece of Feishu state lives
under `~/.mallard`: the folder in `sync_config.json`, the refresh chain in
its own document beside it. The app is Mallard's, carried by the build.

## What the saved evidence proves

| Proved | By |
| --- | --- |
| Drive keeps Mallard's names, bytes, `.mallard` folder, 223-character records, and a 20 MiB + 17 byte file | Desktop round trip, 2026-09-20 |
| The domain selector and the CAS-free publication helper behave over a put/get/list transport with no claims | Rust suites and the mock sequence, 2026-09-23 |
| The developer console accepts the four Drive scopes on user identity and a loopback redirect for a self-built app | Console, 2026-09-20 |

Not proved, and still recorded as pending in
`poc/feishu-drive/evidence/cas-free-2026-09-23.json`: a live OAuth v3
exchange, any refresh, and refresh-token rotation. The earlier wording
"the POC proved user identity" is withdrawn.

## Outcome

- Storage → New storage → **Feishu**. The card runs one lane in the R2 and
  Dropbox grammar: Sign in → Account → Folder → Connection → Review →
  **Finish setup**. No CLI, agent, App ID, App Secret, or pasted token.
- The organization's Feishu admin installs Mallard's app once. Nothing is
  typed: Sign in opens Feishu consent in the browser the user chooses; the
  link is copied or opened, never opened by itself. Mallard keeps a refresh
  token on the computer and renews access by itself.
- A storage is one Drive folder: a new `Mallard <word>` folder in My Space,
  or an existing folder whose link is pasted in. Two computers share a
  storage by sharing that folder in Feishu and connecting each with the
  same app.
- Push, Pull, Compare, the Library, and the path bar work unchanged. Feishu,
  like a local folder, holds no claims: two computers that push the same
  version within seconds can displace one another, and the next Compare
  reports the displaced version. **Clear storage data** is unavailable.
- When Feishu rejects the saved sign-in, every listing marks the storage
  **Reconnect**, and the same lane reconnects, ending at **Save changes**.
  The storage page shows the date by which Feishu requires a fresh sign-in.

## Decisions

### D1. Authorization: Mallard's own app, the secret in the build, OAuth v3 on the desktop

Feishu's OAuth v3 token endpoint requires `client_secret` for every
developer-created app; PKCE supplements the secret rather than replacing
it, and public-client registration is reserved for Feishu's own apps. The
owner's decisions, in order: Mallard's own app so the user types nothing;
no Mallard-operated service on the path, on Cloudflare or anywhere else;
the key kept on the computer. Therefore the release build carries the App
ID and App Secret (`FeishuApp::built_in`, from `MALLARD_FEISHU_APP_ID` and
`MALLARD_FEISHU_APP_SECRET` at build time; a development build reads the
same names at run time), the desktop exchanges and refreshes against
Feishu directly, and a build without the app still offers Feishu and
reports the missing app on the Sign in line.

Safety of a secret in a shipped binary, which the app guide keeps true:
the app holds no tenant-identity scopes, so the secret alone reaches no
data; the redirect is fixed to the loopback port, so a leaked secret
cannot complete a sign-in from anywhere else; a refresh needs the
owner-only connection document, which only local access reads. A rotated
secret refuses old builds (`feishu_app_rejected`, not a reconnect) until
they update.

Consequences accepted with this decision:

- Customers in other tenants need Mallard's app published as a store app
  and installed by their admin; until then the owner's self-built app
  reaches the owner's tenant only. Store release facts are in the guide.
- Tenant identity (`tenant_access_token`) is rejected, and the app must
  never gain an app-identity scope: that is what makes the secret
  harmless.

**Live gates before step 2 starts** (owner):

1. In Mallard's app: enable `offline_access` beside the five approved
   scopes, on user identity only; keep `http://127.0.0.1:53683/feishu/callback`
   registered; make the app available to the users.
2. Run the direct OAuth probe with `offline_access`, then one refresh
   after the returned `expires_in`; record both in the CAS-free rerun doc.
3. Decide D8.

### D2. Identity is the folder token

Settings: `{ connection_id, folder_token, folder_name?, account_name }`,
no secrets. Continuity coordinates: `[folder_token]`. The connection and
the account are connection facts, never identity.
`connection_id` is a random 128-bit hex minted at sign-in; it names the
connection document (D4) and nothing else.

### D3. No config secret

The storage's configuration holds no secret: the app's credentials are the
build's, and the refresh chain is the connection document's. There is no
manual route (`manual_setup_hint: None`) and no managed secret; the
storage page shows the folder, the account, and the sign-in date.

### D4. Credential ownership model

| Holder | Holds | Never holds |
| --- | --- | --- |
| Feishu | The user's Drive; the consent record | — |
| Desktop, the release build | Mallard's App ID and App Secret | — |
| Desktop, `~/.mallard/sync_config.json` | The connection ID; the folder | The refresh token; any secret |
| Desktop, `~/.mallard/feishu_connections/<connection_id>.json` | The current refresh token; `reauthorize_at`; `refresh_uncertain`; `updated_at` | — |
| Desktop, memory | The current access token and its expiry, from Feishu's returned `expires_in` | — |
| Mallard Cloud or any Mallard service | Nothing | Everything above |

File bytes and every request travel desktop ↔ Feishu. **Privacy
boundary, stated in the lane and the README:** the refresh token on this
machine can read and write every Drive folder the user authorized. Folder
selection is Mallard's convention, not a Feishu-enforced token
restriction. The App Secret grants nothing by itself; a user's consent is
also required.

The connection document, `feishu_connections.rs` in the infrastructure
crate, follows `StorageSetupJournalStore`: one schema-1 JSON document per
connection under a directory of the local state root, at most 16 KiB,
written atomically, opened no-follow, mode 0600, under the repository's
lock. The store is installed once at startup with the local state root
(`feishu_connections::install(root)`); tests install a temporary root;
the CLI installs none and cannot open a Feishu storage. Rotation never
touches `sync_config.json`, so the process-shared store's pinned
configuration stays valid and the adapter contract is unchanged. Removing
the storage deletes the document; an orphan is harmless, and the app
reset wipes the root.

Refresh rules on the desktop, in `FeishuAuthorization`:

- The access grant lives in memory; when fewer than 60 s remain the
  transport refreshes, one refresh in flight per storage, held by the
  process-shared transport's lock.
- Before the refresh request is sent, the document is written with
  `refresh_uncertain: true`; after a definitive answer it is written once
  more with the rotated token and `refresh_uncertain: false`. One document,
  one atomic write each time: the flag and the token can never disagree.
- A lost response leaves the flag true. The next refresh sends one settling
  request with the stored token. Feishu's `20073` (token already used)
  means the earlier request landed and its answer was lost; the connection
  is marked `ReconnectRequired`. Any other definitive answer proceeds
  normally. Every refresh attempt sends exactly one request, and a refresh
  attempt happens only when a request needs a token: nothing loops. A
  request that never reached Feishu (a connection failure) is a definitive
  answer, so the flag returns to false and the health reads `Unreachable`.
- A document write failure fails that one request with a fixed message
  ("Mallard could not save the renewed Feishu connection"); the grant in
  memory stays, so the session continues and the next launch reconnects.
- `reauthorize_at` is set at sign-in from `refresh_token_expires_in`, the
  365-day limit counted from the user's consent; each refresh answer's
  value is checked against it and never extends it. Past it the transport
  refreshes nothing and records `ReconnectRequired` with the date.
- A Feishu answer that says the access token is invalid invalidates the
  grant and refreshes once; a read is sent once more, a write never.
- Health codes: `feishu_reconnect_required` (`ReconnectRequired`),
  `feishu_permission_denied` (`PermissionDenied`), `feishu_unreachable`
  (`Unreachable`, reads only), `feishu_request_failed`,
  `feishu_connection_not_saved`. Evidence keeps the route literal, HTTP
  status, the fixed Feishu `code`, and a short plain request ID.

Sign-in and reconnect: the Tauri command exchanges the code, writes the
connection document under the draft's connection ID, and then prepares
the exact draft. A reconnect keeps the connection ID and overwrites the
document; a missing document for a saved storage reads as
`ReconnectRequired`.

### D5. No claims, no conditional delete

`supports_claims` is false because Feishu allows two files of one name in
a folder and has no create-only write. `supports_conditional_delete` is
false because its delete names a token with no revision precondition;
`clear_mode` is `Unavailable`. A record is still one request after a fresh
listing, settled by one listing, never sent twice. The engine's
`Displaced` classification and *recovery is a user action* cover the
no-claim case.

### D6. Logical keys unchanged; three growth families get one shard folder

Feishu holds at most 1,500 direct children per folder. Same-named sibling
folders are one logical directory: reads take the union, writes go to the
folder with the smallest token, nothing is deleted. This replaces the
POC's fail-closed `ambiguous_folder`, which would strand a storage after
two computers create the same folder at once. A same-named file is read
from the smallest token; every read verifies the digest.

### D7. One request in flight per store, 200 ms apart

Feishu documents that upload and download calls must not be concurrent and
allows 5 per second; `cloud_transfer_concurrency` is not consulted. Objects
up to 20 MiB use `upload_all`; larger ones use `upload_prepare`,
`upload_part`, `upload_finish`. The per-object cap is 100 MiB.

### D8. The setup probe leaves one file

No delete scope is requested. The probe writes
`.mallard/v1/_setup_probes/<setup_id>.bin`, reads it back, and lists its
parent to find it. Owner's call: a delete scope and deletion of exactly
that file by its returned token, the R2 precedent, is the alternative.

### D9. Origins are fixed to Feishu China

`open.feishu.cn` and `accounts.feishu.cn`. Lark international is out of
scope, and the adapter stores no origin.

### D10. No account discovery in this change

`discovery` is absent, so the New-storage card shows no doors; the lane's
Folder line offers *New folder* or *Existing folder* (pasted link or
token). Shared-with-me folders have no listing API.

### D11. Retry and recovery: what is automatic, and what is not

Automatic, always bounded to one request per attempt:

- An access token that ran out is refreshed under the transport lock; a
  read is then sent once more, a write never (D4).
- An object upload whose outcome is unknown is settled by one listing of
  its folder, never by a resend: a present name with the expected digest is
  the object, and a resend would create a duplicate file. A Push that
  stopped part-way (a rate limit, a closed laptop) resumes by itself on the
  next Push, because every present object is skipped by name.
- A record is one `upload_all` after a fresh listing, settled by one
  listing; a later attempt is a new list-then-write.
- A not-found answer drops the path and token caches for that entry, and
  the next request resolves again.

Not automatic, by decision:

- No wait or backoff on a rate limit (HTTP 429, code `99991400`) or an
  unreachable answer. The request fails with its fixed message and the
  user tries again; the 200 ms gate keeps ordinary traffic under the 5
  requests per second Feishu documents, and the 10,000 requests per day
  cannot be waited out.
- No multipart resume. Feishu keeps an upload session for 24 hours, but
  Mallard persists nothing about it: a large upload that failed is
  abandoned, and the next Push starts it over; the session expires on
  Feishu's side.
- No automatic reconnect, re-consent, rollback of local acceptance,
  cleanup of probe files, or repair of duplicate folders.

## Failure behavior

| Failure | Desktop | User sees |
| --- | --- | --- |
| Mallard's app rejected at exchange or refresh | Nothing saved; `feishu_app_rejected`, not a reconnect | "Feishu refused Mallard's app · update Mallard" |
| Build without the app | Sign-in and every open refused with `feishu_app_unavailable` | "This build carries no Feishu app"; a development build names the two variables |
| Consent denied or timed out | Loopback reports it | The Sign in line says why; sign in again |
| Scope set differs from the six | Exchange refused; nothing saved | "Feishu granted different permissions" on the Sign in line |
| Mallard's app not installed or not available to this user | Feishu refuses consent | "ask your admin to install Mallard's app for you" |
| Access token expired | Refreshed under the transport lock; a read retried once | Nothing |
| Refresh response lost | `refresh_uncertain` persisted; that request fails as unreachable | "Feishu could not be reached. Try again." |
| Settling request answers `20073` | `ReconnectRequired` with evidence | Reconnect band with route, status, code, request ID |
| Refresh refused (revoked, invalid grant, chain expired) | `ReconnectRequired`, with the date when past `reauthorize_at` | Same, with the reauthorization date |
| Connection document could not be written | That request fails; grant kept; next launch reconnects | "Mallard could not save the renewed Feishu connection" |
| Feishu unreachable | Reads fail `Unreachable`; a write needing a fresh token is not started | "Feishu could not be reached" |
| Feishu rate limit (429, `99991400`) | `RateLimit` for that request | "Feishu rate limit reached. Wait and try again." |
| Folder or file quota reached | Fixed message for the Feishu code | "This Feishu folder or space is full" |
| Mallard's App Secret rotated | Refresh refused in old builds; not suspended | "update Mallard, then try again"; the update needs no new sign-in |
| Connection document missing or unreadable | `ReconnectRequired` | Reconnect lane |

Two computers never share a refresh chain: each sign-in yields its own
refresh token, so one computer's refresh cannot invalidate another's.

## Contract facts

| Fact | Value | Source |
| --- | --- | --- |
| Authorize URL | `https://accounts.feishu.cn/open-apis/authen/v1/authorize` | POC `oauth-probe.mjs` |
| Token URL, both grants | `https://accounts.feishu.cn/oauth/v3/token`, form-encoded, `client_secret` in the body, no Basic header | OAuth v3 and refresh v3 docs, read 2026-09-23 |
| Scopes | `space:document:retrieve space:folder:create drive:file:upload drive:file:download auth:user.id:read offline_access`; the exchange must return exactly these | POC plus `offline_access` for the refresh token |
| Access token life | `expires_in`, a non-fixed value; the example shows 7,200 s | OAuth v3 docs |
| Refresh token | Single use; a new one is returned and the old one is invalid at once; reuse is `20073`; no grace period; `refresh_token_expires_in` is the 365-day reauthorization limit from the user's consent | Refresh v3 docs |
| Lost refresh response | Not addressed by Feishu; one settling request is Mallard's rule | Refresh v3 docs |
| Callback | `http://127.0.0.1:53683/feishu/callback?code&state`, registered in the customer's app | POC; Dropbox uses 53682 |
| API root | `https://open.feishu.cn/open-apis/drive/v1`; success is HTTP 200 with `code: 0` | POC `drive.mjs` |
| List | `GET /files?folder_token&page_size=200&page_token`, direct children, `has_more` + `next_page_token`, `parent_token` checked | Feishu list docs |
| Create folder | `POST /files/create_folder {name, folder_token}` | Feishu docs |
| Small upload | `POST /files/upload_all`, multipart form, `parent_type=explorer`, ≤ 20 MiB, name ≤ 250 chars | Feishu docs |
| Large upload | prepare (4 MiB blocks, 24 h session), part, finish | Feishu docs |
| Download | `GET /files/:token/download`, complete body, size and SHA-256 checked | POC |
| Limits | 1,500 children per folder, 400,000 nodes per space, 15 levels; upload and download 5/s and 10,000/day, listing 20/s | Feasibility report |
| Account line | `GET /authen/v1/user_info` with the user token | Feishu docs |

## Physical layout

Below the storage folder. Logical keys never change; the store translates
on the way in and out, and listings drop the shard component.

| Logical key | Physical path |
| --- | --- |
| `.mallard/_storage.json` | unchanged |
| `.mallard/v1/repositories/<bundle>/_objects/<sha>` | `…/_objects/<first two hex of sha>/<sha>` |
| `.mallard/v1/repositories/<bundle>/_manifests/<rev>-<commit>.json` | `…/_manifests/<rev ÷ 1000, seven digits>/<rev>-<commit>.json` |
| `.mallard/v1/repositories/_commits/<bundle>/<record>` | `…/_commits/<bundle>/<rev ÷ 1000, seven digits>/<record>` |
| `.mallard/v1/library/<repo>/_objects`, `_manifests`, `_commits` | the same three rules |
| `.mallard/v1/library/<repo>/_tag.json`, any `_claims/` | unsharded; claims are never written |
| `.mallard/v1/_setup_probes/<setup_id>.bin` | unsharded, private to the probe |

Capacity: 1.5 million records per stream and 384,000 payloads per bundle;
the space's 400,000-node total is the real ceiling and the README states
it. A process-lifetime path cache maps each logical directory to its
folder token, filled on resolve or create and dropped on a not-found
answer.

## Modules and touchpoints

| Layer | Add | Touch |
| --- | --- | --- |
| Domain | nothing | nothing; schemas stay at 1 |
| Infrastructure | `feishu_auth.rs` (authorize URL with PKCE and state, exchange, refresh with rotation and settlement, `user_info`, evidence, health); `feishu_connections.rs` (the connection document store); `storage_core/feishu_store.rs` with `mock.rs` (Feishu Drive and OAuth loopback), `probe.rs`, `upload.rs`, `tests.rs`; `storage_providers/feishu.rs` | `storage_providers/traits.rs` (`StorageLocalSetup::Feishu`), `registry.rs` |
| Application | `feishu_setup.rs` (write the connection document, then prepare the exact draft: settings and the App Secret) | `storage_setup.rs` tests; storage removal deletes the document |
| Tauri | `project_sync/feishu_setup.rs`: `authorize_feishu_setup`, `propose_feishu_folder_name`, `prepare_feishu_storage`, `cancel_feishu_setup` | `lib.rs` (register; install the connection store with the local state root at startup), `project_sync/mod.rs` |
| Frontend | `FeishuSetup.tsx` (`useFeishuSetup`, `FeishuSetupLane`); `src/feishu-lane-mockup.tsx` specimen | `types.ts`, `api.ts`, `ProjectLinksWorkspace.tsx`, `StorageSettings.tsx` (`setupRouteIcon`, `setupRouteLabel`, the reauthorization date line) |
| Docs | `DESIGN_FEISHU_DRIVE_STORAGE.md`; the admin's app setup guide | `README.md`, `LIVE_TWO_MACHINE_PASS.md` (Feishu part), `AGENTS.md` source map, POC and feasibility status lines |

The adapter contract does not change. Nothing is added to
`mallardInternal`. The POC folder and the
`feishu_publication_selector` example stay as the record of the decision.
The CLI gains nothing.

## Steps

Each step leaves the tree green. The registry entry is the last line of
step 5, so the provider appears only once it can be set up.

### Step 0. Owner: the live gates

The three gates under D1. Done when the CAS-free rerun doc's live rows
read `PROBE_PASS, mode: live` and one refresh with rotation is recorded.

### Step 1. The connection document store

`feishu_connections.rs`: the document type, `install(root)`, `load`,
`save`, `delete`, bounds, no-follow, mode, lock; tests with a temporary
root, including a symlinked or oversized document refused. Done when
`cargo test --lib` is green with no behavior change for the four
providers.

### Step 2. `feishu_auth.rs`

Authorize request, exchange, refresh, `user_info`; `FeishuAuthorization`
with the rules under D4; a mocked token endpoint. Done when the client
tests under Acceptance pass.

### Step 3. `feishu_store`

`FeishuObjectStore`: bounded envelope parsing, request gate, path cache,
shard rules, folder identity rule, breadth-first listing with pagination,
cursor loop guard, entry limits and a 120 s bound; `etag` is the file
token; foreign names are reported and skipped. `upload.rs`: `upload_all`,
multipart plan validation, a present name is the object; `put_record` is
one `upload_all`, never retried; an object upload with an unknown outcome
is settled by one listing (D11). `probe.rs`: `user_info`, list the storage
folder, create `.mallard` and `v1` if absent, write the probe file, read
it back, find it in the listing. `mock.rs`: a hyper loopback Feishu with
tokens, duplicate names, pagination, multipart forms, the OAuth endpoints
with single-use rotation, and the POC's faults. Done when Push, Pull, and
a Library round trip pass on the mock with request counts printed.

### Step 4. Adapter and setup backend

Descriptor (rank 25, icon `cloud`, `local_setup: Feishu`, no managed
secret, fields `app_id` and `folder` with a `derive` from a pasted folder
URL, `setup_guide_url` to the admin guide), validation (version 1,
exactly one secret, `cli_` App ID, 32-hex connection ID, folder token
shape, bounded name),
summary (Folder, Folder token, Account, App ID), coordinates, journal
facts, probe, open. Application `feishu_setup.rs` prepares the exact
draft. Tauri commands mirror Dropbox minus discovery. The provider stays
unregistered.

### Step 5. The lane

Specimen first, reviewed as static HTML. Lines: Sign in (Copy link, Open
browser, Cancel, the app guide link), Account
(name, Continue, Change login), Folder (New with an editable proposal, or
Existing with a paste field), Connection, Review. Before-start copy: the
admin installs Mallard's app, consent covers the Drive the user can
reach, this machine keeps the sign-in, and nothing goes through Mallard. Then `FeishuSetup.tsx`, `api.ts`,
`types.ts`, dispatch, route label and icon, the reauthorization date line,
frontend rows, the registry entry, and the README provider list. Done
with a native run against the mocks and a screenshot in the PR.

### Step 6. Docs, checklist, live pass

`DESIGN_FEISHU_DRIVE_STORAGE.md`; the app guide (install Mallard's app
for the customer's admin; the app, its six user-identity scopes, the
callback, the store release, and the secret in the build for Mallard's
maintainers); README sections for the sign-in, the privacy boundary,
limits, rate cost, no claims, no clear, reconnect;
`LIVE_TWO_MACHINE_PASS.md` part 6 (part 2 expects the no-claim outcome;
part 3 does not apply). Owner runs the two-machine pass with two users of
one tenant sharing one folder.

## Acceptance tests

Desktop (Rust, Feishu OAuth and Drive loopback mocks):

1. `feishu_connections`: a document round-trips within its bounds, a
   symlink, a wrong mode, and an oversized document are refused, and a
   Feishu store refuses to open with no store installed; rotation never
   changes `sync_config.json`, so the shared store's entry stays pinned.
2. The authorize URL carries PKCE S256, a random state, the loopback
   callback, and exactly the six scopes; the exchange refuses any other
   returned scope set and saves nothing.
3. `FeishuAuthorization` refreshes only when fewer than 60 s remain, one
   in flight per storage, and keeps the access token in memory only.
4. Each refresh writes the document with `refresh_uncertain: true` before
   the request and once more with the rotated token and `false` after it;
   a response with `expires_in` of 600 caches for 600 s.
5. A lost response leaves the flag true; the next refresh sends exactly
   one settling request; `20073` records `ReconnectRequired` with
   evidence; a normal answer rotates once.
6. `invalid_grant`, a revoked token, and `now >= reauthorize_at` each
   record `ReconnectRequired` with the date and send no second request;
   a refresh answer never extends `reauthorize_at`.
7. A document write failure fails the request with the fixed message and
   keeps the grant; the next request uses it; a missing document for a
   saved storage reads as `ReconnectRequired`.
8. A Feishu token-invalid answer refreshes once and resends one read; a
   write is never sent twice.
9. The descriptor declares no secret and the `folder_token` field;
   validation refuses any secret, a malformed connection ID, and a
   malformed folder token; summary, journal facts, errors, and IPC
   responses contain no App Secret or token.
10. Adapter, engine, and Library tests as in Step 3, plus the application
    tests: prepare (document written before the draft), commit, reconnect
    (same connection ID, document overwritten), App Secret replacement,
    and a storage removal that deletes the document and nothing remote.

Frontend:

11. The lane shows every step from the first on, asks for nothing typed
    before Sign in, the before-start copy names the admin's install step,
    the privacy boundary, and the direct path, the reconnect lane ends at
    Save changes, and the storage page prints the reauthorization date.

Live (owner, recorded in the CAS-free rerun doc):

12. Direct OAuth exchange with `offline_access`; one refresh after
    `expires_in` with the rotated token used next; one forced lost
    response followed by the settling request; the two-machine pass part
    6.

## Safety invariants

Unchanged: immutable, SHA-256-verified publication; list-then-write with
one record request; no automatic resend; recovery is a user action; no
credential, token, folder token, or local path in a portable manifest;
bounded reads, listings, and counts; schema 1 everywhere.

Stated boundaries of this change:

- Feishu holds no claims. Section 2 of the two-machine pass has a
  different expected outcome for Feishu, and the README says so.
- The probe leaves one file per setup attempt (D8), and nothing in the
  store deletes.
- The refresh chain lives in `~/.mallard/feishu_connections/`, a new
  machine-local document family with the journals' bounds, atomic writes,
  no-follow opens, and mode; it never enters `sync_config.json`, a
  portable manifest, a backup, or a log. Test 1 proves the bounds.
- This machine holds a credential to every Drive folder the user
  authorized, the same boundary as the Dropbox refresh token today, and
  Mallard's App Secret in its build, harmless under the three facts of D1.
  No Mallard service holds either.

## Verification

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:integration
```

## Out of scope

Any Mallard-operated service on the Feishu path; the store-app
publication itself (the owner's, outside this change); Lark international
origins; tenant identity; account discovery of shared folders; a Feishu
CLI; storage clear.

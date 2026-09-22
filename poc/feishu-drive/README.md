# Feishu Drive storage PoC

Status: local tests and a live desktop file round trip pass. The Open API
probe remains a separate test. This is not a registered Mallard storage
provider. See the [live results](../../docs/FEISHU_LIVE_POC_2026-09-20.md).

Read the [feasibility report](../../docs/FEISHU_STORAGE_FEASIBILITY_2026-09-19.md)
for the decision, API sources, remaining constraints, and production work.

The probe uses synthetic bytes. It tests Feishu Drive file APIs against
Mallard's current publication names and selection rule. It does not capture
sessions, restore files, or read provider homes.

## Run locally

Requirements: Node.js 22, Cargo, and the repository's cached Rust dependencies.
The commands build a small example with the current `mallard-domain` crate.
Cargo runs with `--offline --locked`. No new package is required.

From the repository root:

```sh
node --test poc/feishu-drive/feishu-drive.test.mjs
node --test poc/feishu-drive/desktop-fixtures.test.mjs
node --test poc/feishu-drive/browser-probe.test.mjs poc/feishu-drive/oauth-probe.test.mjs
node poc/feishu-drive/probe.mjs mock
cargo test --offline --locked -p mallard-domain publication::tests --lib
```

The mock run uses native `Request`, `Response`, `FormData`, and `Blob` objects.
Multipart bodies pass through serialization and parsing. No network request
is sent. A mock `PROBE_PASS` proves only the local fixture results.

For a manual desktop test, run `node poc/feishu-drive/desktop-fixtures.mjs prepare`.
It creates synthetic files in a new temporary folder. Upload the returned
`upload` folder, then download it into a different empty directory. Run
`node poc/feishu-drive/desktop-fixtures.mjs verify "$MANIFEST" "$DOWNLOADED_ROOT"`
with the returned manifest path and the downloaded folder root. The verifier
checks exact paths, sizes, SHA-256 digests, and the current Rust record
selection. This does not run the Open API probe.

If `selector_build_failed` occurs, run this command to inspect the compiler
output:

```sh
cargo build --offline --locked -p mallard-domain --example feishu_publication_selector
```

## Prepare a live run

Use a dedicated test app and a Drive folder with sufficient free capacity.
The probe creates one child folder named `mallard-feishu-poc-<random hex>`.
It retains this folder on success or failure. It has no delete or permission
change operation.

In the Feishu developer console, enable these API permissions by their
display names. Request the narrow permission for each operation:

| Operation | Scope | Permission label in the English API reference |
| --- | --- | --- |
| List | `space:document:retrieve` | Get document list in My Space folder |
| Create folder | `space:folder:create` | Create folder in My Space |
| Upload, including multipart | `drive:file:upload` | Upload file |
| Download | `drive:file:download` | Download file in My Space |

These labels come from the official [list][list], [folder][folder],
[upload][upload], and [download][download] references. The API pages also
list broader alternatives. Folder access and download permission are
separate from API scopes. Ensure the test user can use the app and access
the test folder before running the probe.

These scope identifiers were verified in the developer console on
2026-09-20. Select the identity type that the probe uses. The live setup
uses user identity only. These user scopes cover the user's accessible
data; the configured folder token limits the probe's operations.

Use a short-lived `user_access_token` or `tenant_access_token`. For an app
identity, follow Feishu's [folder access procedure][faq]: enable the bot,
add it to a group, and share the test folder with that group. Check the
folder's download policy as well.

Obtain tokens outside this probe. For user authorization, follow the
current [OAuth v3 reference][oauth]. A developer-created app requires a
client secret. Do not put a shared app secret in a distributed desktop
binary. The credential-file runner neither exchanges nor refreshes tokens.

Create a UTF-8 credential file **outside the repository**. Use this shape:

```json
{
  "access_token": "<SHORT_LIVED_TOKEN_A>",
  "peer_access_token": "<OPTIONAL_SHORT_LIVED_TOKEN_B>",
  "parent_folder_token": "<DEDICATED_TEST_FOLDER_TOKEN>"
}
```

Omit `peer_access_token` to use the first token for both client objects.
When supplied, the second token must permit list, upload, and download in
the same folder. The probe uses both clients as writers. Distinct tokens
do not prove distinct users or distinct machines.

Use an absolute, canonical file path and private permissions:

```sh
chmod 600 /private/tmp/feishu-poc.json
node poc/feishu-drive/probe.mjs live --credentials /private/tmp/feishu-poc.json
```

The path above is a placeholder. The reader rejects symbolic links, hard
links, broad permissions, foreign ownership, changes during the read,
unknown fields, and files larger than 64 KiB. It supports macOS and Linux.
Do not paste tokens into chat or put them in command arguments.

The API origin is fixed to `https://open.feishu.cn`. HTTP redirects fail.
Provider responses, tokens, URLs, and credential paths are not printed.
The generated probe folder name is printed before the first write so that
an interrupted run can be found.

### Run without a credential file

To use an existing short-lived user token, run:

```sh
node poc/feishu-drive/browser-probe.mjs --parent-folder '<DEDICATED_TEST_FOLDER_TOKEN>'
```

Open the printed loopback URL. Enter the token in its password field and
submit the form once. The token remains in process memory. This mode uses
one token for both client objects. The listener stops after 15 minutes.

API Explorer returned `20021` with `1108012: invalid client auth method`
during the 2026-09-20 test. Consent succeeded, but no token was issued.
A retry after version `1.0.0` was published and the app was enabled
returned the same error. See the [live report](../../docs/FEISHU_LIVE_POC_2026-09-20.md).
For a direct test of the documented OAuth v3 flow, run:

```sh
node poc/feishu-drive/oauth-probe.mjs --app-id '<APP_ID>' --parent-folder '<DEDICATED_TEST_FOLDER_TOKEN>'
```

After approval, register `http://127.0.0.1:54321/oauth/callback` in the
test app's redirect URLs. Enter the App Secret at the terminal's hidden
prompt. Open the printed local URL and select **Continue to Feishu**.
Authorize the four Drive scopes plus `auth:user.id:read`. The API Explorer
consent page also required this identity scope. The probe does not call
the user-info API. Never put the secret in command arguments or chat.

The direct flow uses a random state and PKCE S256. It sends the app secret
only to `https://accounts.feishu.cn/oauth/v3/token`. The token exchange
must return exactly the five requested scopes. It rejects extra or
missing scopes and a refresh token. No `offline_access` scope is requested.
The app secret and user token are not written to files or logs. The
callback accepts one authorization result and rejects foreign hosts,
wrong state, replay, and expired authorization. The listener has no POST
endpoint or credential form. Stop the process with Ctrl-C when the run
ends; it also stops after 15 minutes.

This is a developer PoC for a user-owned app. It does not make a shared
app secret safe to include in a distributed desktop application. The
callback remains registered until the app owner removes it.

On 2026-09-20, the callback was registered after approval. Chrome blocked
the local form POST with `ERR_BLOCKED_BY_CLIENT`, including a control
attempt with synthetic credentials. The server remained in `ready`
state and was stopped. That form was removed. The current flow reads the
secret from a terminal and uses browser GET navigation for consent and
the callback. Its nine local tests and a real terminal no-echo check pass.
See the live report for the current live token-exchange result.

### How the user token is created

1. The browser opens Feishu's authorization endpoint with the app ID,
   redirect URI, scopes, random state, and PKCE S256 challenge.
2. After consent, Feishu redirects to the local callback with a one-use
   authorization code and the original state.
3. The local process checks state. It sends the code, app ID, App Secret,
   the same redirect URI, and PKCE verifier to the OAuth v3 token endpoint.
4. The token response contains `access_token`, which is the user token.
   The process checks its scopes and expiry, then runs the Drive probe.

The request uses `grant_type=authorization_code` and form encoding. The
secret goes in `client_secret`; no HTTP Basic Authorization header is
added. Feishu's [current token contract][oauth] requires a client secret
for developer-created apps. PKCE supplements that authentication. The
App Secret alone cannot produce a user token; the user's authorization
code is also required. Publishing an app does not produce a user token.

## Live probe sequence

1. Create a unique test folder. Confirm it through the second client.
2. Write and read the storage marker and a small payload.
3. Upload a synthetic 4 MiB + 17 byte payload through multipart upload.
4. Write and verify a synthetic manifest before a publication record.
5. Publish a root record. Use the Rust parser and selector to read it.
6. Let both clients list the same parent. Send two sibling records in
   sequence. Each send stays within the admission time limit.
7. Check that both clients list all records and select the same sibling.
8. Read the losing record and check its SHA-256 digest.
9. Repeat a same-name upload. Check exact names and all duplicate bytes.
10. Upload an abandoned payload. Check that selection does not change.
11. Repeat the visibility check three times.

Upload calls are sequential. This creates an overlapping publication
sequence without making simultaneous Feishu requests. Listing pages have
two entries to exercise pagination. One gate spaces all requests by at
least 250 ms after the preceding response finishes.

The default payload tests two multipart blocks. The local test suite also
tests automatic multipart selection above 20 MiB. The live probe does not
test the account's maximum file size or sustained throughput.

## Result interpretation

| Result | Meaning |
| --- | --- |
| `PROBE_PASS`, `mode: mock` | Synthetic API fixture passed |
| `PROBE_PASS`, `mode: live` | The sampled live operations passed |
| `record_visibility_gap` | An acknowledged record was absent from a later listing |
| `cross_client_visibility_gap` | A known record was absent for the other client |
| `outcome_unknown` | The probe cannot determine whether a record arrived |
| `remote_changed` | The proposed parent or publication is no longer selected |
| `ambiguous_folder` | A path component has more than one possible folder |
| `integrity_mismatch` | Downloaded content differs from the expected digest or size |
| `http_<status>` / `feishu_<code>` | A sanitized transport or API failure |

Every successful result includes `production_ready: false`. Sampling does
not prove that a remote listing is always complete. The hidden-sibling
test demonstrates this limit.

The probe uses the production `RecordName` and `Mainline` code. Its record
and manifest **bodies are transport fixtures**, not complete Mallard
DTOs. Its one-attempt publication helper is not the production
`write_record` loop. It rejects an acknowledged-but-unlisted record;
the production helper can retain its own acknowledged record in memory.
Do not import this folder into a real Mallard storage.

The adapter has no sharding or path cache. Duplicate folders stop a run.
The published namespace has no mutable head, index, or lock file. Files
with the same logical name are accepted only when every copy matches the
expected bytes. All operations remain bounded.

## Files

| File | Responsibility |
| --- | --- |
| `drive.mjs` | Bounded Drive requests, pagination, multipart, and key-to-token lookup |
| `probe.mjs` | Synthetic publication sequence, credential reader, and CLI |
| `browser-probe.mjs` | One-use loopback form for an existing user token |
| `oauth-probe.mjs` | Hidden terminal secret entry, OAuth v3, PKCE, and the one-use callback |
| `selector.mjs` | Build and call the current Rust selector |
| `mock.mjs` | Synthetic HTTP fixture with duplicate names and fault injection |
| `feishu-drive.test.mjs` | Transport, integrity, publication, and credential tests |
| `browser-probe.test.mjs`, `oauth-probe.test.mjs` | Token form, terminal input, callback, scope, and credential boundaries |
| `desktop-fixtures.mjs`, `desktop-fixtures.test.mjs` | Synthetic desktop tree and exact download verification |
| `../../crates/mallard-domain/examples/feishu_publication_selector.rs` | Read-only bridge to production domain code |

[list]: https://open.feishu.cn/document/server-docs/docs/drive-v1/folder/list
[folder]: https://open.feishu.cn/document/server-docs/docs/drive-v1/folder/create_folder
[upload]: https://open.feishu.cn/document/server-docs/docs/drive-v1/upload/upload_all
[download]: https://open.feishu.cn/document/server-docs/docs/drive-v1/download/download
[faq]: https://open.feishu.cn/document/server-docs/docs/drive-v1/faq
[oauth]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/get-user-access-token-v3

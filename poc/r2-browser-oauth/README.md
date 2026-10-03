# R2 browser OAuth POC

Checked on 2026-10-03. POC only. No production integration.

**Result: automatic S3 key creation is blocked at Cloudflare authorization
for the tested Mallard client. Browser sign-in and R2 listing work.**

## Live evidence

The test used the private **Mallard R2 browser POC** client, Authorization Code
with PKCE S256, token authentication `none`, and a loopback callback. It used
no Wrangler executable, Cloudflare-owned client ID, saved CLI token, or
Pipelines credential broker.

| Check | Live result |
| --- | --- |
| Own-client sign-in, token exchange, account confirmation | Passed |
| Earlier default-jurisdiction R2 listing | Passed; 28 buckets |
| Request `account_api_tokens:create` with Mallard's client | Cloudflare returned `invalid_scope`; the attempt ended before token exchange |
| Separate read-only `GET /oauth/scopes` | HTTP 200; 392 scopes; no pagination indicated |
| API-token creation permission in that catalogue | None |
| Other provisioning-name match | `scim-provisioning.write`; unrelated to API-token creation |
| Live API-token creation, bucket creation, S3 object access | Not reached |

The catalogue included these R2 grants:

- `workers-r2.read`
- `workers-r2.write`
- `workers-r2.metadata_read`
- `workers-r2-bucket-item.read`
- `workers-r2-bucket-item.write`

R2 access does not itself establish permission to create an API token.
Cloudflare's 2026-10-01 announcement describes OAuth account-token creation
with `account_api_tokens:create`, starting with Cloudflare CLI. Its third-party
client registration API rejects colon-delimited scopes. The live client scope
selector and authenticated catalogue offered no equivalent API-token grant.
[Provisioning announcement](https://developers.cloudflare.com/changelog/post/2026-10-01-account-api-token-provisioning/),
[Client scope rules](https://developers.cloudflare.com/api/resources/iam/subresources/oauth_clients/methods/create/),
[Scope catalogue API](https://developers.cloudflare.com/api/resources/iam/subresources/oauth_scopes/methods/list/).

This result blocks the tested own-client flow. It does not prove that every
Cloudflare token API is unavailable. No live token-creation POST was sent.
Do not remove the working production setup until Cloudflare exposes a supported
third-party grant and the complete S3 test passes.

The sanitized observations are in [live-evidence.json](live-evidence.json).
Screenshots are under the ignored `output/r2-browser-oauth/` directory:
`live-provisioning-scope-rejected.jpg` and `live-scope-catalog.jpg`.

## POC behavior

`Browser → OAuth consent → loopback callback → token exchange → account choice → one operation`

Account confirmation is required even with one account. **Change login** stays
available before the operation. Failure, expiry, or cancellation ends the
attempt. There is no retry, reset, recovery, or resume action.

The default operation lists buckets. `--scope-catalog` reads available grants
only. `--issue-s3-key` reads that catalogue and sends one request to
`POST /accounts/{account_id}/tokens`, if authorization succeeds. The request
names one new test bucket, permits Object Read & Write only for that bucket,
and expires after one hour. The POC does not create the bucket.

The key check validates the returned policy and expiry. It derives the S3
Access Key ID from the issued API token ID and the Secret Access Key from
SHA-256 of that token's value. It never converts the OAuth bearer into an S3
secret. This conversion passed fixture tests only.
[R2 credential rules](https://developers.cloudflare.com/r2/api/tokens/),
[Account-token creation](https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/methods/create/).

Tokens stay in process memory. No key, connection, or provider-home state is
saved. Results omit credentials and raw provider error text. An unreadable
create response reports an unknown outcome; it does not send another request.
If a future run issues a token, its result reports the test token name and
expiry. It does not claim S3 access until an actual S3 probe passes.

## Run and verify

Node 22 is sufficient. No packages or subprocesses are required.

```sh
node --test poc/r2-browser-oauth/browser-oauth.test.mjs
node poc/r2-browser-oauth/browser-oauth.mjs --mock
node poc/r2-browser-oauth/browser-oauth.mjs --mock --scope-catalog
node poc/r2-browser-oauth/browser-oauth.mjs --mock --issue-s3-key
```

The key mock refuses creation by default. Tests inject success and uncertain
responses. Mock results do not establish Cloudflare permission availability.

For live tests, register a private client with:

| Setting | Value |
| --- | --- |
| Response type | `code` |
| Grant type | `authorization_code` |
| Token authentication | `none` |
| Callback | `http://127.0.0.1:53684/r2-poc/callback` |
| Verified read scopes | `account-settings.read workers-r2.read` |

```sh
node poc/r2-browser-oauth/browser-oauth.mjs \
  --client-id YOUR_REGISTERED_CLIENT_ID \
  --scopes 'account-settings.read workers-r2.read' \
  --scope-catalog
```

The POC accepts the exact documented `account_api_tokens:create` string to test
its availability. Other colon-delimited scopes remain rejected locally. Do not
borrow first-party client credentials. Adding this scope to the tested client's
authorization request produced the live refusal recorded above.

Open the printed URL, complete sign-in, and confirm the account. An optional
`--account-id` preselects an account but does not bypass confirmation. The
attempt expires after three minutes. The result remains visible for one minute.
Use Ctrl+C to close the listener. Wait for listener closure before another test.
Each test is a separate attempt; failed attempts cannot resume.

**18 local tests passed**, including a run with `PATH=/nonexistent`. Coverage
includes PKCE, callback state and replay, denied or invalid scopes, bounded
responses, token redirects, account choice, Change login, foreign form origin,
terminal failure, one key request, restricted key policy, correct conversion,
redaction, and uncertain creation without retry.

No live S3 key or bucket was created. The existing private OAuth client and
its read grant remain. No additional OAuth permission was granted. No native
Mallard build, production change, deployment, commit, or push was done.

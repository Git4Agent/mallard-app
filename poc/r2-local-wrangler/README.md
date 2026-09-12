# Local Wrangler R2 setup POC

This POC tests R2 setup without Codex, Claude, or another agent. It uses the
installed `wrangler` executable and the current Mallard browser consent code.
It prepares the normal schema-1 draft in a temporary home. It does not save a
connection in the user's Mallard state.

The executable is a Cargo example in
[`r2_local_wrangler.rs`](../../crates/mallard-cli/examples/r2_local_wrangler.rs).
It includes the current CLI authentication and callback source without changing
production commands. No dependency or application UI change is required.

## Verified result: 2026-09-10

The live POC passed on macOS with Wrangler 4.125.0. It created the approved
test bucket in the default jurisdiction, received the S3 credential through
browser consent, and prepared a schema-1 draft at revision 2. No agent process
or model call was part of setup.

| Check | Result |
| --- | --- |
| Direct Wrangler browser login | Passed |
| Account, OAuth output, and bucket-list access | Passed |
| Direct Wrangler bucket creation and inspection | Passed |
| Browser consent and local credential callback | Passed |
| S3 list, conditional create, and content read | Passed |
| Conditional update and stale ETag rejection | Passed |
| Probe-object deletion and absence check | Passed |
| Normal draft preparation in a temporary home | Passed |
| Saved Mallard connection | None |
| POC and reused callback tests | 19 passed |
| Tauri backend library tests | 301 passed; 6 ignored |

The final executable result was:

```json
{
  "status": "REVIEW_READY",
  "agent_used": false,
  "temporary_draft": true,
  "draft_revision": 2,
  "s3_probe_passed": true,
  "connection_saved": false,
  "remote_bucket_created": true,
  "remote_credential_retained": true
}
```

The initial live check found an expired Wrangler session that could not
refresh. It failed both inside and outside the sandbox. The explicit POC
`login` action succeeded. The subsequent `check` and complete setup passed.
This is a required recovery path for a future desktop action.

The test bucket and its issued credential remain in Cloudflare. The capability
probe removed its test object. The POC did not publish a storage marker or any
user content. This run does not verify multiple accounts, other jurisdictions,
packaged-app executable discovery, or desktop progress and cancellation.

## Run

From the repository root, check an existing Wrangler login:

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- check
```

This checks `whoami --json`, `auth token --json`, and `r2 bucket list`.
It does not start login, open a browser, or change R2 resources. Wrangler can
refresh its own cached login. Account names, IDs, bucket names, tokens, and raw
Wrangler output are suppressed.

For a missing or expired login, run the direct login action:

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- login
```

This starts `wrangler login`. Complete consent in the browser, then run
`check` again. An expired session can make `whoami --json` fail before it
returns a logged-out JSON document. The explicit action covers that case.

Use an existing bucket in the default jurisdiction:

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- \
  setup --bucket YOUR_EXISTING_BUCKET
```

Or create an exact test bucket:

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- \
  setup --bucket mallard-poc-YOUR_UNIQUE_SUFFIX --create
```

Replace the example name with lowercase letters, digits, or hyphens. The POC
requires the `mallard-poc-` prefix for creation. It never retries a failed
create request or selects a replacement name. Inspect Cloudflare before another
create attempt after a failure. New buckets remain in Cloudflare after the run.

Setup opens the system browser for login if required and for credential consent.
Complete login, MFA, and consent yourself. Cloudflare creates an S3 credential
for the selected bucket. The POC removes its temporary local draft on exit.
The issued credential remains in Cloudflare. Revoke it after the experiment.

Add `--probe` to test the received S3 key with Mallard's current capability
probe. This writes and removes probe objects. Without this option,
`REVIEW_READY` proves draft preparation only, not S3 access.

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- \
  setup --bucket YOUR_EXISTING_BUCKET --probe
```

Run the local tests:

```sh
cargo test -p mallard-cli --example r2_local_wrangler
```

The tests use temporary homes, synthetic credentials, and local callback
fixtures. They include the current callback, token parsing, timeout, and process
tests. They do not use real provider homes or cloud resources.

## Setup boundary

| Step | Local implementation | Agent required |
| --- | --- | --- |
| Find login and account | `wrangler whoami --json` | No |
| Start login | `wrangler login`, completed by the user | No |
| Create a bucket | `wrangler r2 bucket create` | No |
| Check selected bucket | `wrangler r2 bucket info --json` | No |
| Obtain an S3 key | Existing Cloudflare browser consent and loopback callback | No |
| Fill the review draft | Existing Mallard application function | No |
| Test and finish | Current Mallard review flow; explicit user action | No |

Wrangler OAuth is a Cloudflare API bearer. It is not the S3 Secret Access Key.
The installed Wrangler 4.125.0 has no standalone R2 key-creation command. Its
Pipelines client uses `generateR2ServiceToken` and a hosted browser consent
service. Mallard already uses that same service. The POC reuses Mallard's
existing callback checks. It does not create a Worker or Pipeline.

The R2 CLI documents bucket management in
[Wrangler commands](https://developers.cloudflare.com/r2/reference/wrangler-commands/).
Wrangler documents the JSON token output and automatic refresh in
[general commands](https://developers.cloudflare.com/workers/wrangler/commands/general/#auth-token).
Cloudflare documents the separate R2 key and API-token derivation in
[R2 authentication](https://developers.cloudflare.com/r2/api/tokens/).
The broker implementation is in the
[Wrangler Pipelines client](https://github.com/cloudflare/workers-sdk/blob/main/packages/wrangler/src/pipelines/client.ts).
That broker is a Pipelines implementation detail, not a documented general R2
desktop OAuth contract. This remains a production dependency to review.

## Proposed application change

Use one **Set up with Wrangler** action. Run the setup operation from the Tauri
backend. Show progress for login, bucket selection or creation, browser consent,
and draft preparation. Keep **Test connection** and **Finish setup** in the
existing review flow.

For production, move the shared process and browser code from the CLI module
into a Tauri-free infrastructure module. Let the CLI and desktop application
call the same application operation. Keep the draft lock, account binding,
provisioning journal, revision checks, and explicit final review.

Use Wrangler for local authentication. Keep the current structured R2 API and
provisioning journal for production bucket creation. The POC proves that the
direct Wrangler create command also works, but it lacks the app's recovery
journal. `r2 bucket list` returns display text in the tested version, so use
the structured R2 list API for a production bucket picker.

The POC supports one account and the default jurisdiction. Multiple accounts
stop with an explicit error. Production account selection, cancellation, and
packaged-app PATH discovery still require application work.

## Local runtime check

The POC can check the same executable lookup and dependency commands used by
production setup. This does not authenticate or contact R2:

```sh
cargo run -p mallard-cli --example r2_local_wrangler -- runtime
```

After implementation, this check passed with the installed Homebrew Wrangler
and with a minimal desktop PATH. The example tests passed with:

```sh
cargo test -p mallard-cli --example r2_local_wrangler -- --test-threads=1
```

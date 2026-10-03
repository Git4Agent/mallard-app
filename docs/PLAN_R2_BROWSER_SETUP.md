# R2 browser setup plan

Date: 2026-10-03. Status: blocked by the live S3 authorization POC.
This document does not authorize implementation.

Replace the R2 setup dependency on Wrangler with direct browser sign-in. Use
four visible steps: **Sign in → Account → Bucket → Review**. Keep explicit final Save.
Preserve the current storage schema, S3 transport, and Push/Pull behavior.

User constraints: keep account selection and Change login. Keep account
confirmation even when only one account is available. Add no retry or recovery
flow. Remove the existing R2 retry, reset, and resume behavior.

The current flow has six steps: Wrangler, sign-in, account, bucket, key, and
review. It starts a packaged CLI, which runs Wrangler for authentication.
Bucket management already uses direct HTTP. The final
`commit_storage_setup_draft` already tests the connection before atomic Save.
A separate Test action is optional today.

The isolated `r2-browser-oauth` POC passed live browser sign-in, PKCE token
exchange, account confirmation, and R2 bucket listing without Wrangler. It
found one account and 28 buckets in the default jurisdiction. It saved no
credentials or Mallard connection and made no storage changes. Local tests
cover account choice, Change login, and terminal failure without retry.
The automatic S3 key POC then stopped at Cloudflare authorization. Adding
`account_api_tokens:create` to Mallard's own client request returned
`invalid_scope`, before token exchange. A separate read-only check returned
392 OAuth scopes. None offered API-token creation. SCIM provisioning was
the only broader provisioning-name match; it cannot issue R2 credentials.
No live token-creation request, bucket creation, or S3 probe occurred.
The key-request policy and token-to-S3 conversion passed fixture tests only.

The signed-in Cloudflare dashboard accepted a private PKCE test client with
`account-settings.read` and `workers-r2.read`. Its scope selector did not offer
API Tokens Write or Account API Tokens Write. Searching for token permissions
showed only Access Service Tokens. Do not treat this as proof that every API
route is unavailable.

Cloudflare announced OAuth account-token creation on 2026-10-01, starting with
its CLI and the `account_api_tokens:create` scope. The third-party client API
still documents rejection of colon-delimited scopes. The live scope catalogue
and authorization refusal confirm that the tested client cannot use this
permission. Obtain a supported third-party grant from Cloudflare before
promising automatic S3 key creation. This result does not establish that every
Cloudflare token API is unavailable. Do not borrow a Cloudflare-owned client
ID or bypass validation.
[Account-token announcement](https://developers.cloudflare.com/changelog/post/2026-10-01-account-api-token-provisioning/),
[Third-party scope rules](https://developers.cloudflare.com/api/resources/iam/subresources/oauth_clients/methods/create/).

Cloudflare documents a desktop OAuth flow with PKCE and no client secret.
This supports the proposed direction. It does not prove that the client's
permissions can create the required S3 key.
[OAuth client requirements](https://developers.cloudflare.com/fundamentals/oauth/create-an-oauth-client/).

| Step | Proposed behavior |
| --- | --- |
| Sign in | Open the system browser. Use one temporary OAuth session for the setup attempt. |
| Account | Keep the account selector, account identity, Continue, and Change login. A sole account can be preselected, but the user must confirm it. Keep the choice available before bucket operations. |
| Bucket | Keep Create new and Use existing. Show the exact account, bucket, and jurisdiction before creation or key issuance. Keep the four supported jurisdictions. Prepare the bucket and its restricted key through one action. |
| Review | Show the destination and access scope. Finish setup runs the existing connection test and Save. Remove the extra Test button from the normal R2 browser flow. Keep Test for manual entry and saved connections. |
| Saved credential update | Keep the existing user-initiated action. Sign in and confirm the account for the saved bucket. Skip bucket creation and discovery. Reject a destination mismatch. Review, then Save credential. Do not start this action automatically after an error. |

Keep Open browser, Copy link, and Cancel. Keep manual S3 key entry collapsed
by default. It supports credentials supplied by an account owner and remains
a separate, supported use case.

Implement in this order:

1. **Resolve the blocked S3 authorization before production replacement.**
   Reuse the private test client and verified loopback callback. Use PKCE
   S256 and token authentication `none`. Obtain any additional scope IDs
   from the live catalogue. Do not copy Wrangler's colon-delimited scopes.
   [OAuth scope rules](https://developers.cloudflare.com/api/resources/iam/subresources/oauth_clients/methods/create/).
   The tested client has no available token-creation grant. Continue only
   when Cloudflare exposes a supported permission for Mallard's own client.
   Select one documented route before implementation. Create a dedicated test bucket and
   obtain an Object Read & Write key restricted to that bucket. The documented
   user-token route requires API Tokens Write. Its availability through a
   third-party OAuth grant was not offered in the live scope catalogue.
   [Token creation](https://developers.cloudflare.com/api/resources/user/subresources/tokens/methods/create/).
   Derive S3 credentials only from the issued R2 API token. An OAuth bearer
   is not an S3 secret.
   [R2 credential rules](https://developers.cloudflare.com/r2/api/tokens/).
   Run Mallard's full S3 capability test with a temporary draft. Check that
   the issued policy names the exact account, jurisdiction, and bucket.
   Record the granted scopes, API route, result, and resource disposition
   without secrets. Do not save a production connection. If this fails,
   resolve the authorization design before removing the working setup.

2. **Add direct Rust authentication and shared setup orchestration.** Put
   Cloudflare OAuth/token requests in infrastructure. Reuse
   `loopback_browser` and the established Dropbox PKCE pattern. Put draft
   preparation in a small application-layer R2 module. Reuse `R2SetupClient`,
   existing discovery, and the current S3 provider. Keep OAuth tokens in
   Rust memory for one bounded attempt. Send only a session handle, account
   choices, and sanitized progress to the frontend. Do not request offline
   access or add a persistent login cache for this setup-only operation.
   Store the final S3 key through the existing draft secret handling.

3. **Connect Tauri directly to the application layer.** Replace the desktop
   R2 subprocess call with direct calls. Bind each session and discovery
   result to the draft ID, revision, mode, account, and expiry. Preserve
   single-flight execution, cancellation, exact draft adoption, and stale
   result rejection. Update `projectSyncApi`, Rust DTOs, TypeScript DTOs,
   progress events, and command registration together. Change the descriptor
   capability from `wrangler` to `r2` without an alias. This descriptor change
   does not require a stored configuration migration.

4. **Simplify the existing R2 UI.** Reduce `R2SetupLane` to the four steps
   above. Keep account choice and confirmation. Remove dependency installation,
   version/path details, and the separate key-authorization step after
   the live POC proves automatic key issuance. Reuse the session across
   account, discovery, and preparation calls. Replace hard-coded login and
   consent countdowns with backend expiry data. Keep the current form layout
   and labels outside the R2 setup area. Change review text to Finish setup
   or Save credential. Call the existing commit operation once; do not add
   a separate automatic test before it. Remove Try again, Scan again as a
   failure action, Reset setup, phase-based retry dispatch, and error text
   that tells the user to retry or recover. Failure, cancellation, or expiry
   ends the attempt. Show the result and Close. Account changes before bucket
   operations remain normal user choices; they must invalidate old selections.

5. **Remove the superseded implementation.** Use the cleanup map below. Adapt
   the supported terminal R2 commands to the same Rust implementation before
   deleting the old code. Each CLI invocation owns its temporary session.
   Ship one production authentication path. Do not retain Wrangler as a
   fallback, feature flag, or hidden compatibility route.

6. **Verify native behavior and distribution.** Run the checks below. Before
   general release, configure the Mallard OAuth client's public visibility
   and verified domain, then verify sign-in from outside its owning account.
   A private client test covers account members only. Do not make the POC
   client public as part of the experiment.
   [Client visibility](https://developers.cloudflare.com/fundamentals/oauth/create-an-oauth-client/).

The cleanup must follow actual callers. Some Wrangler-named code is shared.

| Area | Remove or change | Keep |
| --- | --- | --- |
| R2 frontend | Install commands and agent message; dependency errors; `wrangler_ready`; version/path state; retry dispatcher and controls; failure rescan; Reset setup; resume state; redundant normal-flow Test button and “Test, then” copy | Account choice, Continue, Change login, target summary, manual keys, explicit Save, cancellation, stale-form guards |
| Shared frontend | Rename `wranglerLane` to `directSetupLane`, `wranglerReviewReady` to `setupReviewReady`, and `loadPreparedR2Draft` to `loadPreparedStorageDraft`; update all callers | These helpers also serve other providers. Keep shared `StorageSetupStep` and CSS used by them |
| Desktop R2 adapter | CLI argument construction, child-process startup/termination, stdout parsing, `CliSuccess`, `R2Action`, and helper-specific errors | Session control, bounded discovery, scan identity/expiry, exact candidate checks |
| Desktop service | Remove `packaged_cli_path` and its tests once the R2 caller is gone | The packaged CLI remains a supported terminal tool |
| CLI authentication | Remove the legacy `r2_browser_setup` module: Wrangler/Node discovery, environment isolation, command/version/auth parsing, progress JSON, and the Pipelines credential broker callback | Supported R2 commands, neutral account output, redaction, revision validation |
| Setup probe | Remove the credential-readiness request loop from the setup probe. Set one attempt per setup request | Current list, create, read, create-only refusal, and list-after-write checks; one best-effort cleanup |
| Examples and tests | Remove `r2_local_wrangler` after replacement coverage exists. Remove the hidden legacy import from `dropbox_direct_setup`. Replace fake-Wrangler fixtures with injected OAuth/HTTP fixtures | Dropbox example, draft-conflict tests, callback and credential protections |
| Dependencies | Remove `httparse` and `socket2` from the CLI only if no remaining target uses them. Move `tempfile` to dev dependencies if its remaining uses are tests/examples | Shared infrastructure dependencies and active CLI packaging |
| Documentation and mocks | Update active setup instructions and descriptor fixtures. Mark old Wrangler/agent plans and POC instructions as superseded. Remove unreferenced executable mocks | Historical evidence and mocks still used for other providers |

Preserve these behavior limits:

- Discovery reads markers; it does not create or repair them. Keep pagination
  bounds, partial-list reporting, and jurisdiction-specific endpoints.
- Send each requested operation once. Do not retry requests, renew an expired
  attempt, resume a failed step, or reuse a failed attempt's progress. An
  uncertain bucket or token response ends the attempt with an unknown result.
  Do not create another resource, replay the operation, or reconcile it into
  success. A later setup must be a new user action through the normal entry.
- Cancel stops the matching local attempt. It does not delete a bucket or
  revoke the previous saved key. Show any remote resources already created.
- Keep exact destination checks during saved credential updates, draft revision
  checks, callback state/host/path checks, bounded reads, redaction, and expiry.
- Keep the current S3 capability checks before Save: list, create, verified
  read, refusal to overwrite an existing key, and immediate list visibility.
  Make one cleanup request for its own test object. A cleanup refusal does
  not change the probe verdict. Remove credential-readiness retries for setup.
- Do not add a new provisioning journal. Current source has retired the old
  journal described in earlier plans. Do not change storage schemas or add
  a Mallard cloud service for this flow.

Acceptance checks:

- Test account choice and confirmation for one and multiple accounts. Test
  Change login before bucket operations, denied/missing scopes, expired login,
  failed browser opening, port conflict, cancellation, replay, and late callback.
- Test create, existing storage, and saved credential updates with exact
  destination binding. Cover all current jurisdictions, incomplete discovery,
  and uncertain writes.
- Assert one request per operation. Assert that failure, cancellation, and
  expiry expose no retry, reset, rescan, resume, or automatic recovery action.
  A late result cannot turn an ended attempt into success.
- Test duplicate clicks, stale draft revisions, navigation during setup,
  failure before Save, and failure of the commit-time connection test.
  No connection is saved before the final user action.
- Verify existing storage connections and manual S3 entry. Check that Dropbox,
  Feishu, and Managed Cloud retain their shared setup behavior.
- Run `npm run build`, `npm run test:frontend-integration`,
  `cargo test --workspace --all-targets`, and `npm run test:release-config`.
  The workspace command includes the Tauri library, CLI tests, and examples.
  Use temporary homes and injected services for automated tests.
- Verify the complete Tauri app with Wrangler and Node absent from its runtime
  search path. Record screenshots for create, existing, and saved credential
  updates. Report live provider and platform coverage separately from fixtures
  and compilation.
- Confirm no production R2 caller, descriptor, command output, or executable
  example requires Wrangler or the old broker. Confirm dependency removal
  against every remaining Cargo target.

This turn adds this plan and extends the isolated POC. Production code is
unchanged. The private OAuth test client is a live test resource. No production
build, deployment, commit, or push is part of this turn.

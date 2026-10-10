# Dropbox create and discovery setup without an agent

> Historical reference: The `dropbox_direct_setup` example was removed on
> 2026-10-09. Its test command below no longer works in this checkout.
> The production Dropbox setup code and tests remain.

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Date: 2026-09-11. Status: implemented and verified with local fixtures. Live cloud retest is pending.

## Cutover

Keep all schemas at 1. Remove the Dropbox CLI setup command, storage agent
commands, prompts, and launch controls. Do not migrate old state or retain a
legacy execution path. The user will remove local and cloud test data.

## Outcome

Make Dropbox setup follow the current R2 step layout and controls. Support
new folders, discovery of existing Mallard storage, and reconnect. Use direct
Dropbox authorization. Do not require an agent, Wrangler, a Dropbox CLI,
an app secret, or a pasted authorization code.

Always provide a copyable link during a browser step. Provide **Switch
account** before the user creates or selects storage.

The current reference is `src/components/project-sync/R2Setup.tsx`, including
its account confirmation, editable name, candidate list, link controls,
countdown, cancel, retry, and exact draft adoption. The initial R2 plan is
older than that implementation.

## User flow

Keep the current **Create new storage** and **Find existing storage** choice.
Show the following rows in the same style and location as R2:

| Row | Create new folder | Find existing storage |
| --- | --- | --- |
| Sign in | Copy link or open Dropbox consent | Copy link or open Dropbox consent |
| Account | Show the returned account; Continue or Switch account | Same |
| Folder | Show a proposed name; edit, Suggest another, or Create folder | Scan, show results, then Use folder |
| Connection | Verify namespace and token refresh; fill the draft | Recheck the selected namespace and marker; verify refresh; fill the draft |
| Review | Test connection, then Finish setup | Same |

Dropbox has no dependency-check or jurisdiction row. Its browser consent
grants the permissions needed for the chosen setup mode. Normally one consent
is enough. The Connection row must not show a second browser step unless a
fresh grant is needed. Do not describe Dropbox credentials as an R2/S3 key.

Show the account display name and email returned by Dropbox. The backend
uses the account ID as identity. Do not infer the account from browser cookies,
a typed email, a folder name, or a previous UI selection. Confirm the account
before listing or creating folders.

### Browser links and account switching

- Bind the registered callback before publishing the authorization link.
- Validate the Dropbox authorization origin/path, public app ID, and exact
  registered redirect URI before forwarding a link to the UI. Never copy a
  callback URL that contains an authorization code.
- At every pending browser step, show a selectable, read-only URL plus
  **Copy link**, **Open browser**, and **Cancel**. The link is available before
  the user opens a browser, including on the first attempt and every retry.
- Do not open the default browser automatically. Match R2: the user can copy
  the link into another browser or profile on the same computer.
- Copy only on a user click. Show **Link copied** on success. Keep selectable
  text available if clipboard access fails. Opening a browser must not hide
  the copy action; a browser-open failure must not stop the callback listener.
- **Switch account** starts fresh authorization with new state and PKCE
  values, using `force_reauthentication=true`. Apply it only to an explicit
  switch. It can sign out the Dropbox web session in the selected browser.
- Cancel an active request and close its listener before starting the new
  request. Clear the previous account, scan, selection, and in-memory grant.
  Show and confirm the account returned by the new request.
- Keep Switch account available at paused setup steps. While work runs,
  require cancellation to finish first. Do not let a late result restore
  the previous account or draft.
- After a create request starts, the journal pins that draft to its account
  and folder. Switching to another account requires a new setup draft. Keep
  the old draft and journal available for recovery; explain that a created
  folder remains. Never retarget that journal or delete the remote folder.
- Clear a pending URL on success, denial, timeout, cancellation, switch, and
  navigation away. Retry publishes a fresh URL. Do not offer expired links.

Dropbox documents forced login separately from forced consent in its
[official Rust SDK](https://github.com/dropbox/dropbox-sdk-rust/blob/master/src/oauth2.rs).
Use forced login for account switching. A copied link uses the local callback
on this computer; it is not a cross-device authorization link.

### Create

Reuse the current name word list. Propose `Mallard <word>`, with an editable
folder name and **Suggest another**, as in the R2 name row. Validate one folder
name on both sides of the IPC boundary; do not accept an arbitrary path.

Create the exact displayed name only after **Create folder**. A confirmed
collision returns to the name field. Do not silently replace a user-approved
name or adopt an existing folder. Extend the existing provisioning function
to accept the exact name and preserve its journal checks.

Reuse the current create, share, namespace verification, and token refresh
operations. A shared namespace is required by the storage adapter. Do not
invite members, create public links, or mount unrelated folders. Show the
confirmed name once creation succeeds. Keep the connection display name
separate and editable in the existing review form.

### Find existing

Port the verified Node discovery POC into Rust. Reuse its tested behavior:

1. Read account root metadata; exclude the personal home namespace.
2. List accessible shared folders and mountable shared folders, with cursor
   handling. Include a distinct team root when the account reports one.
3. Deduplicate by namespace ID. Read only `.mallard/_storage.json` at each
   candidate root. Validate it with the existing Rust storage marker parser.
4. Reuse `CandidateGroups`, `CandidateRow`, empty results, partial results,
   and Scan again from the current discovery UI.
5. Enable **Use folder** only for a supported marker that is not already
   connected locally. Show other marker states and access errors separately.
6. Resolve the selection from the backend's saved scan, then recheck the
   account, namespace, and marker before preparing the draft.

Identify renamed storage by its marker and namespace, not its name. Show
access limits and an **Open storage** action for an existing local connection.
A read-only user may discover storage; do not report that as write permission.
The normal connection test and final save retain their capability checks.

Discovery sends no create, share, mount, upload, or delete operation. It does
not search personal file trees or enumerate an entire Dropbox team. Keep
the POC limits: 100 pages per listing route, 1,000 unique candidates, four
concurrent marker reads, 1 MiB list responses, and 4 KiB markers. Report a
partial scan when a limit or read error prevents completion.

## Small implementation

### Backend

Use direct Rust calls from Tauri to the existing Dropbox modules. Unlike
Wrangler, Dropbox has no external login cache to reuse between helper runs.
Keep the grant in one bounded in-memory setup session while the user confirms
the account and chooses the folder. This avoids a new credential file or an
interactive subprocess protocol. The user flow and UI stay aligned with R2.

- Add `src-tauri/src/project_sync/dropbox_setup.rs` for the desktop commands,
  one active Dropbox setup session, cancellation, and scan selection.
- Move the existing bounded callback receiver out of the CLI into a reusable
  infrastructure module. Add progress delivery and cancellation. Keep PKCE,
  state, host/path, response bounds, one-use callback, and port checks.
- Put discovery in `crates/mallard-infrastructure/src/dropbox_setup/discovery.rs`.
  Use the existing HTTP client, domain types, marker parser, and error types.
- Move the shared draft preparation orchestration from the CLI backend to
  the application layer. The desktop and POC use this implementation. Remove
  the production Dropbox CLI command in the clean cutover.
- Expose typed operations for authorize/account, propose name, create,
  discover, prepare selected storage, reconnect, and cancel through
  `projectSyncApi`. Register them in Tauri and update Rust/TypeScript DTOs
  together. Replace the unwired generic discovery calls with working calls.

The setup session holds the draft ID, expected revision, an opaque session
generation, confirmed account ID, grant, and latest scan. Keep tokens and PKCE
verifiers in Rust. Send only the authorization URL, bounded account labels,
progress, scan summaries, and opaque session/candidate IDs to the setup UI.
Credentials enter the existing private draft only when preparation succeeds.
The existing exact review API remains the only review path for their values.

Bind each operation to the active session, draft, revision, mode, and account.
Invalidate scans on account/mode changes, edits, reset, and a new scan. Use
the current R2 ten-minute scan limit; expire the idle Dropbox session after
ten minutes as well. Reauthorize on expiry. Do not refresh an idle setup in
the background. Zeroize grants when the session ends. A running operation
owns its grant until it finishes or reaches a safe cancellation checkpoint.

Request explicit scopes for the selected mode:

| Mode | Scopes |
| --- | --- |
| Create | Current storage scopes plus `sharing.write` |
| Existing | Current storage scopes plus `sharing.read` |
| Reconnect | Current storage scopes |

Current storage scopes are `account_info.read`, `files.metadata.read`,
`files.content.read`, and `files.content.write`. Request offline access for
the final connection. Existing-storage consent covers discovery and the
intended connection; the scan itself remains read-only. Do not add sharing
write access to discovery. Verify granted scopes and the returned account.
The registered app must enable `sharing.read` before release. Keep the UI
notice that Full Dropbox permission extends beyond the selected folder.

### Frontend and obsolete paths

Add `DropboxSetup.tsx` with focused local state and a hook shaped like R2's
setup hook. Reuse the workspace save/adopt guards, clipboard helper, external
link helper, discovery result components, and step styling. Extract only the
small step and browser-link presentation pieces shared by both providers.
Keep provider-specific behavior in separate hooks; do not build a generic
workflow engine or a global account manager.

Add a Dropbox direct-setup capability to the existing provider descriptor.
Move managed-refresh-secret metadata out of the agent descriptor so the
direct flow retains the current credential review and replacement rules.
Remove Dropbox's agent setup action, copied agent prompt, agent wait state,
and agent-dependent credential/help text. Remove storage agent APIs and
descriptor fields that have no remaining production caller. Do not alter
project session agent features. Keep the existing advanced manual entry and
the explicit review/save controls. Update README and affected tests.

## Delay, retry, and cancellation

Keep the current Dropbox callback limit of 180 seconds and show a countdown.
Keep the 15-second API limit, 120-second scan deadline, and bounded share-job
polling. A delayed user can finish within the callback window. After expiry,
show **Authorization expired. Try again** and create fresh authorization.

Cancel closes the listener and prevents later callbacks or stale IPC results
from changing the form. Stop new remote operations immediately. Preserve
confirmed create/share checkpoints and pending share jobs. An interrupted
mutation with an unknown result must not be retried automatically or replaced
with another folder. Resume only operations that the journal proves safe.

Reconnect always preserves the saved namespace. Switching the login during
reconnect can use another account only if that account has access to the
same namespace. A failed reconnect leaves the saved connection unchanged.

Preparation fills the exact draft and enters Review. It does not commit a
connection or automatically run the write probe. Preserve the normal
**Test connection**, **Finish setup**, and **Save changes** behavior.

## Revoked credentials

Distinguish normal access-token expiry from a rejected authorization. Reuse
the current automatic refresh for normal expiry. If Dropbox rejects the
saved grant, including `invalid_grant` from the refresh endpoint, mark the
connection **Reconnect required**. Do not repeatedly submit the rejected
credential. Network failures and permission failures keep their separate
states; do not describe every failed request as revocation.

Show **Dropbox access is no longer valid. Reconnect to continue**, with a
**Reconnect Dropbox** action. Stop new operations for that connection and
report the interrupted operation as failed or incomplete. Keep local data,
remote data, the saved namespace, and operation recovery records. Do not
delete storage or create a replacement folder.

Reconnect opens an edit draft for that exact connection. Provide Copy link,
Open browser, and Switch account through the direct flow. After consent,
verify access to the saved namespace, obtain a new refresh credential, and
fill the edit draft. Keep the old saved configuration until **Save changes**
passes the normal connection checks and commits the replacement credential.
A denied, cancelled, or failed reconnect leaves the connection suspended.

After a successful save, discard cached authentication and failure state for
the old credential revision. The user can retry the failed operation through
its normal review/recovery flow. Do not automatically replay a Push, Pull,
deletion, or unknown remote mutation. Reconnect cannot restore folder access
that an owner or administrator removed; report that permission failure.

Dropbox documents token refresh and revocation in its
[offline access guide](https://dropbox.tech/developers/using-oauth-2-0-with-offline-access).
The existing Rust auth layer already distinguishes reconnect, permission,
and network failures. This change connects those states to the direct UI.

## Delivery and acceptance

Implement in three working increments within this change:

1. Direct sign-in, copyable links, account confirmation/switch, create, and
   review. Keep the flow usable from start to finish before adding discovery.
2. Rust discovery, selection recheck, and existing-storage preparation in
   the same step layout.
3. Reconnect, error/cancel coverage, obsolete agent-path removal, and native QA.

Required coverage:

- Setup works without an installed or configured agent.
- Every browser wait has a working Copy link action before Open browser.
  Check another browser profile, clipboard failure, and browser-open failure.
- Account A cannot supply a late callback, scan, or credential after a switch
  to account B. Confirm B before any listing or creation. Check a switch after
  folder creation and preservation of the original draft/journal.
- Check editable names, collisions, unknown create outcomes, asynchronous
  sharing, and safe retry without duplicate folders.
- Check pagination, duplicate namespaces, renamed storage, unmounted shares,
  team roots, marker errors, partial scans, already connected storage, stale
  candidates, and read-only access.
- Check delayed consent, timeout, cancellation, occupied callback port,
  double clicks, draft edits, navigation, and late completions.
- Check normal token expiry versus revoked access/refresh credentials,
  suspension without repeated requests, reconnect failure, credential-cache
  replacement after save, and retry without replaying an interrupted write.
- Check that links, OAuth codes, tokens, and verifier values do not enter
  logs or saved scan state. Use temporary homes and synthetic credentials.

Run `npm run build`, `npm run test:frontend-integration`, relevant CLI tests,
`cargo test -p mallard-application --lib`,
`cargo test -p mallard-infrastructure --lib`, and
`cargo test --manifest-path src-tauri/Cargo.toml --lib`.

Verify the complete Tauri app with fixture state and include screenshots of
account confirmation, link controls, create, discovery, and review. Complete
live create and discovery checks with user browser consent. Verify account
switching in the browser; record any account or platform not tested. Do not
use the user's normal Mallard or provider homes for fixture tests.

## Existing evidence

The [discovery POC](../poc/storage-discovery/README.md) records a successful
live Dropbox scan on 2026-09-10: eight candidates, two valid markers, and no
read errors. Team roots and unmounted shares have local coverage only.

The [direct setup POC](../poc/dropbox-direct-setup/README.md) passed 22 example
tests and 51 Dropbox infrastructure tests. Its live create attempt timed out
before a callback. Live direct creation and the new account-switch/link flow
remain acceptance checks; do not report them as verified.

## Implementation verification — 2026-09-11

- `npm run build`: passed.
- `npm run test:frontend-integration`: 338 passed.
- `cargo test -p mallard-infrastructure -p mallard-application --lib`:
  569 infrastructure tests and 20 application tests passed; 5 infrastructure
  tests remain explicitly ignored.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 316 passed;
  6 explicitly ignored.
- `cargo test -p mallard-cli -- --test-threads=1`: 28 unit and 13 integration
  tests passed. The removed Dropbox setup command is rejected.
- `cargo test -p mallard-cli --example dropbox_direct_setup -- --test-threads=1`:
  22 passed, including the example's existing R2 helper tests.
- Browser fixture `poc/dropbox-direct-setup/flow-check.html`: all four checks
  passed. Create, discovery, and reconnect keep the authorized draft revision.
  Double clicks start one request; late consent cannot restore a cancelled
  session. The fixture uses the production hook with synthetic APIs.
- Native app with a verified temporary home: create and existing entry points,
  link display, successful copy feedback, cancellation, callback-port reuse,
  forced account switching, and discovery consent scopes passed. No live
  Dropbox consent or folder operation was completed in this check.
- Native screenshot verification remains incomplete: capture returned an
  empty content area although the accessibility controls were present and
  operated. No screenshot is offered as proof of the form layout.

The browser fixture reproduced the revision bug before the final fix. The
common save helper increments the draft revision. Dropbox now saves at login
and reuses that exact draft for subsequent steps. A new sign-in saves a fresh
revision. User edits invalidate the session through the existing form guards.

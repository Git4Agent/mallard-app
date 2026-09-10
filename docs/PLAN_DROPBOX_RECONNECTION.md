**Dropbox reconnection plan**

Status: implemented 2026-09-09. Builds on
`docs/PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md` (implemented) and
`docs/PLAN_DROPBOX_SYNC_PATH_REQUEST_DEPTH.md` (implemented 2026-09-09).
Deltas from the plan as written are listed at the end.

When Dropbox rejects a saved credential, Mallard should say so once, name the
cause it can prove, stop presenting that credential until it changes, and lead
the user to the storage's own settings page. The page keeps the same namespace
and asks only for the credential again: through the agent, which regains a
refresh token for that namespace, or by pasting a replacement token. Every
persisted schema stays at **1**: local state, storage configuration, drafts,
the setup journal, bundle manifests. Connection health is runtime state in the
process; nothing is migrated and no legacy path is added.

The user's failure was `invalid_grant` from the token endpoint: Dropbox no
longer honours the saved refresh token. Dropbox refresh tokens do not expire on
their own; they stop working when the user disconnects the app, when Dropbox
revokes them, or when the token was issued for a different app key. Nothing in
the current code retains what Dropbox answered, so the cause cannot be
established after the fact. This plan cannot prevent a revocation; it makes
the next one diagnosable and recoverable in one step.

**What the code does today.** Verified against the working tree.

1. *Refresh.* `DropboxAuthorization::header` (`dropbox_auth.rs:346-364`)
   refreshes before dispatch under one lock. `DropboxClient::response`
   (`dropbox_auth.rs:136-172`) maps HTTP 401 to `Reconnect`, 403 to
   `Permission`, the token endpoint's `invalid_grant` to `Reconnect` and
   `invalid_scope` to `Permission`, and everything else to `Remote`. Messages
   are fixed text. No endpoint, status, error tag, or request ID is kept.
2. *Transport.* `DropboxObjectStore::checked` (`dropbox_store.rs:344-366`)
   maps every 401 to `Error::Auth` and 403 to `Error::Permission`; the 401
   body is read within bounds and discarded. Dropbox reports
   `invalid_access_token`, `expired_access_token`, `missing_scope`,
   `user_suspended`, `route_access_denied`, `invalid_select_user`, and
   `invalid_select_admin` all as 401 `AuthError` values, so an expired manual
   token, a missing scope, and a revoked grant read as the same sentence:
   "Dropbox authorization failed. Reconnect Dropbox or replace the manual
   access token".
3. *Probe codes.* `probe_error` (`storage_providers/dropbox.rs:65-71`) maps
   every probe failure except cleanup to the one code `dropbox_probe_failed`.
   `StorageSetupError::code()`, the CLI's `verify` JSON, and Test connection
   therefore cannot tell a rejected credential from a network failure.
4. *Shared store.* `ConfiguredObjectStore::shared` (`storage_core/mod.rs:
   322-342`) keeps one store per storage ID for the life of the process and
   replaces it only when the `StorageConfig` differs. The store owns the
   `DropboxAuthorization` and its cached grant. Nothing records a failure:
   every Activity refresh, Compare, Push, Pull, and Library read re-sends the
   rejected credential and fails again with the same text.
5. *Grant expiry uses `Instant`.* `AccessGrant.expires_at` is a
   `std::time::Instant` (`dropbox_auth.rs:94, 303, 354`). `Instant` does not
   advance while the machine sleeps (Darwin `CLOCK_UPTIME_RAW`, Linux
   `CLOCK_MONOTONIC`). Dropbox access tokens live four hours. A grant cached
   before a sleep longer than its remaining life is presented after wake,
   Dropbox answers 401 `expired_access_token`, and finding 2 turns that into a
   reconnect message. Not reproduced; inferred from the clock semantics. Before
   the 2026-09-09 shared-store cutover a store rarely outlived a sleep, so
   this exposure is new.
6. *Listing.* `storage_summary` (`storage_setup.rs:192-224`) produces
   identity, availability, and redacted facts; no connection state. The
   editor derives "Connected" from secret presence: `hasManagedCredentials`
   (`StorageSettings.tsx:39-45`) drives the copy at `StorageSettings.tsx:415`
   whether or not Dropbox accepts the token. Confirmed defect.
7. *Failures cross IPC as strings.* Commands return `Result<_, String>`;
   `storageAccessErrorMessage` (`src/storageError.ts:23-33`) recognises S3
   prose only. The sidebar and Storages index badge come from `storageIssues`
   (`ProjectSync.tsx:302, 455`), set only by the storage page's repository
   listing (`ProjectLinksWorkspace.tsx:770-781`). Activity refresh already
   turns a remote failure into a warning and keeps the local rows
   (`service.rs:1483-1487`): local history stays available there.
8. *The reconnection route exists.* An edit draft
   (`create_storage_edit_draft`) plus the agent slot on an existing managed
   storage (`ProjectLinksWorkspace.tsx:1780-1782`) runs
   `mallard-cli storage dropbox setup` against the Edit draft. With a
   namespace present the CLI requests the storage scopes only,
   `prepare_namespace` creates no folder and verifies the namespace with the
   refreshed token (`dropbox_setup.rs:224-233, 353-370`), and
   `prepare_storage_draft_in_operation` swaps in the new refresh token.
   `update_storage_setup_draft` locks the namespace and refuses an
   access-token substitution on a managed draft (`storage_setup.rs:531-557`).
   The agent instruction is the new-setup text (`dropbox.rs:178-186`): it
   mentions folder creation and Finish setup and never says the purpose is
   regaining access. The CLI prints the same success line for both cases.

**Design decisions.**

1. *Health lives beside the credential.* A new `storage_core/connection.rs`
   defines the runtime record:

   ```rust
   #[serde(rename_all = "snake_case")]
   pub enum ConnectionState { Connected, ReconnectRequired, TokenExpired, PermissionDenied, Unreachable }
   pub struct ConnectionEvidence { endpoint: &'static str, http_status: u16, error_tag: Option<&'static str>, request_id: Option<String> }
   pub struct ConnectionHealth { state, code: &'static str, message: &'static str, observed_at: u64, evidence: Option<ConnectionEvidence> }
   ```

   `ConnectionHealth::suspends()` is true for `ReconnectRequired`,
   `TokenExpired`, and `PermissionDenied`: each needs a credential or consent
   change. `Unreachable` is recorded so a failure notice can say "temporary",
   but it never suspends and never shows as a badge. `DropboxAuthorization`
   holds `health: Mutex<Option<ConnectionHealth>>`; every classified failure
   writes it and every 2xx writes `Connected` (only on a state change, so the
   lock is not churned per request). `ObjectStore` gains one default method,
   `fn connection_health(&self) -> Option<ConnectionHealth> { None }`; the
   Dropbox store returns its authorization's record. Local and R2 stores keep
   the default. Nothing here names a provider outside the Dropbox module.
2. *The shared store gates and forgets.* `ConfiguredObjectStore` reads
   `inner.connection_health()` before delegating; a suspending state returns
   `Err(health.describe())` without a request. That is what suspends Activity
   refresh, Compare, Push, Pull, and Library reads uniformly. Recovery drops
   the entry: `ConfiguredObjectStore::forget(&StorageId)` is called after a
   successful probe (`probe_journaled` success in the application crate,
   which covers Test connection, Finish setup, Save changes, and CLI
   `verify`) and on `remove_storage`. A changed configuration already
   replaces the entry, so Save changes with a new credential starts clean.
   Health is never written to disk.
3. *Classification.* One table, applied in `checked` and in the token client.
   Unlisted 401 bodies fall to the first row.

   | Source | Dropbox response | Code | State | Suspends | Retry |
   | --- | --- | --- | --- | --- | --- |
   | Refresh | 400 `invalid_grant` | `dropbox_reconnect_required` | ReconnectRequired | yes | no |
   | Refresh | 400 `invalid_scope`, or a scope missing from the response | `dropbox_permission_denied` | PermissionDenied | yes | no |
   | API | 401 `invalid_access_token`, `invalid_select_user`, `invalid_select_admin`, unreadable | `dropbox_reconnect_required` | ReconnectRequired | yes | no |
   | API | 401 `expired_access_token`, refresh credential | see decision 4 | — | — | read once |
   | API | 401 `expired_access_token`, manual token | `dropbox_token_expired` | TokenExpired | yes | no |
   | API | 401 `missing_scope`, `user_suspended`, `route_access_denied`; any 403 | `dropbox_permission_denied` | PermissionDenied | yes | no |
   | Any | timeout, connect failure, 5xx, 429 | `dropbox_request_failed`, existing `RateLimit` | Unreachable | no | unchanged |
   | Any | 2xx | — | Connected | — | — |

   Messages are fixed per code and credential kind. Reconnect, managed:
   "Dropbox rejected the saved authorization. Reconnect Dropbox to restore
   access." Reconnect, manual: "Dropbox rejected the saved access token.
   Paste a replacement token in Storage settings." Expired, manual: "The
   Dropbox access token expired. Paste a replacement token in Storage
   settings." Permission: "Dropbox denied access for this connection.
   Reconnect Dropbox to grant the missing permission." Unreachable: "Dropbox
   could not be reached. Check the network and try again." `Error` gains an
   `Expired` variant; it stays `Copy`.
4. *Wall-clock expiry and one read retry.* `AccessGrant.expires_at` becomes
   Unix seconds, `now_secs() + expires_in - 30`, compared against
   `now_secs()`. A clock that jumps forward costs one early refresh; a clock
   that jumps back keeps an expired token, which the retry absorbs. Reads
   (`files/get_metadata`, `files/download`, `files/list_folder` and
   `/continue`, `users/get_current_account`) retry exactly once after
   `DropboxAuthorization::invalidate()` when the credential is a refresh
   token and the 401 tag is `expired_access_token`; a second 401 records
   `ReconnectRequired`. Writes (`upload_session/start`,
   `upload_session/finish_batch_v2`, `files/delete_v2`) never retry. A 401 on
   a write is a known outcome, Dropbox rejected the request before acting, so
   it surfaces as `dropbox_token_expired` and the user reruns the review. No
   mutation with an unknown result is ever replayed; that rule is unchanged.
5. *Evidence is an allowlist and rides in the message.* `checked` takes the
   route literal (`&'static str`), reads `X-Dropbox-Request-Id` (kept only if
   at most 64 ASCII letters, digits, or hyphens), and keeps the error tag only
   when it is one of the seven `AuthError` tags; for `missing_scope` the
   `required_scope` value is kept only when it is one of the five scopes
   Mallard requests. `ConnectionHealth::describe()` renders
   `"{message} ({endpoint}, HTTP {status}, {tag}, request {id})"`. Every
   auth-class `Err` the Dropbox store returns goes through `describe()`, so
   the existing Push, Pull, Activity, Library, Test, and Save failure notices
   and their activity-log lines carry the evidence with no new plumbing.
   Body text and tokens never enter it. The refresh client returns
   `AuthFailure { error: AuthError, evidence }` from `refresh` so the token
   endpoint's request ID is captured too; `exchange` and `rpc` keep
   `AuthError` for the setup path.
6. *Health reaches the UI on the listing, and a change announces itself.*
   `StorageSummary.connection: Option<ConnectionHealth>` is filled by
   `ConfiguredObjectStore::connection_health_for(&config)`, a table read with
   no open and no marker request. A state transition sends the storage ID on
   a process-wide `tokio::sync::broadcast` channel exposed as
   `storage_core::connection_events()`; `src-tauri/src/lib.rs` forwards it as
   the window event `project-sync-storage-connection-changed { storage_id }`,
   mirroring the drafts-changed event, and `ProjectSync.tsx` re-reads the
   config on it. A lagged receiver just re-reads. Surfaces:

   - Sidebar and Storages index: the existing warning triangle with
     "{provider_label} needs reconnection" (ReconnectRequired, TokenExpired)
     or "{provider_label} denied access" (PermissionDenied); opening from the
     badge opens the storage page with settings expanded.
   - Storage page: a standing danger notice in the page's zone with the
     message and evidence and one action, "Reconnect {provider_label}", which
     expands settings and opens the edit draft. The namespace field is
     read-only as today; only the credential is asked for again.
   - Editor guide row (`StorageSettings.tsx:412-421`) takes `connection`.
     Suspended: red copy with the message, the agent CTA reads "Reconnect
     with {agent}", and the credential field's placeholder reads "Rejected by
     {provider_label} — paste a replacement". Connected: "Connected. Access
     renews automatically." only after an observed success. No record yet:
     "Access renews automatically. Test connection to verify." A manual-token
     storage shows the Access token field and the same agent CTA, since the
     agent route converts it to a managed connection on the same namespace.
   - Push and Pull `action_failed` notices, the link chooser failure, and
     the storage page's repository-listing failure add the same Reconnect
     action when the storage's `connection` suspends.
   - No forced navigation. A Push that fails mid-review must not move the
     user off the review; the notice's action is the prompt. `storageIssues`
     and `storageAccessErrorMessage` keep their current roles.
7. *The agent is told it is regaining access.* `StorageSetupAssistantContext`
   gains `reconnect: bool`, true for an Edit-source draft. The Dropbox adapter
   renders a reconnect instruction: reconnect the existing Dropbox storage;
   Dropbox rejected the saved authorization; run the same CLI command with
   the same three options; the command reopens Dropbox consent for the same
   account, obtains a new refresh token for the existing namespace, and saves
   it in this exact draft; it creates no folder and changes no namespace;
   report success or the fixed error, then stop; the user selects Save
   changes. The CLI keeps one command, because the Edit draft's namespace
   already selects the reauthorize path (`backend.rs:227-262`); no
   `--reconnect` flag. `setup_dropbox` returns `SetupOutcome { reconnected }`
   so `output::setup` prints "Dropbox access regained for the existing
   namespace. Select Save changes in Mallard." for that case. R2 ignores the
   flag.

**Steps.**

1. **Transport.** `dropbox_store.rs`: add `Error::Expired`; make `checked` a
   method taking the route literal; parse the 401 tag and request ID; record
   health through the authorization; add `read_checked(route, build)` that
   performs decision 4's single retry; route every auth-class `Err` in the
   `ObjectStore` impl and `probe.rs` through `describe()`. `upload.rs`: pass
   route literals; no retry. `dropbox_auth.rs`: `AccessGrant.expires_at: u64`;
   `AuthFailure`; `DropboxAuthorization::{record_failure, record_connected,
   invalidate, health}`. New `storage_core/connection.rs` with the types,
   `suspends`, `describe`, and the events channel. `dropbox_store/mock.rs`:
   401 bodies in Dropbox's real shape
   `{"error_summary":"…","error":{".tag":"…"}}` with an
   `X-Dropbox-Request-Id` header, plus `expire_once`, `reject_token`, and
   `missing_scope` switches; `dropbox_setup/mock.rs`: a request ID on the
   `invalid_grant` reply.
2. **Shared store.** `storage_core/object_store.rs`: the default
   `connection_health`. `storage_core/mod.rs`: the gate in every
   `ConfiguredObjectStore` method, `forget`, `connection_health_for`, and the
   broadcast on transition.
3. **Adapter.** `storage_providers/dropbox.rs`: `probe_error` maps `Auth`,
   `Expired`, `Permission`, `Namespace`, and `Unknown` to their codes and
   keeps `dropbox_probe_failed` for the rest; the reconnect instruction.
4. **Application.** `storage_setup.rs`: `StorageSummary.connection`; `forget`
   after a successful probe and in `remove_storage`; `reconnect` in the
   assistant context.
5. **Desktop.** `lib.rs`: forward the events channel. `service.rs` and
   `commands.rs`: nothing else; `ProjectSyncConfigView` carries the field
   through `StorageSummary`, and the Activity warning string already embeds
   the store error.
6. **CLI.** `backend.rs`, `output.rs`, `lib.rs`: `SetupOutcome` and the
   reconnect line. `args.rs` already documents reconnection.
7. **Frontend.** `types.ts`: `StorageConnectionHealth`, the event name.
   `StorageSettings.tsx`: the `connection` prop and copy. `ProjectLinksWorkspace.tsx`:
   the standing notice, the CTA label, the Reconnect action on failures.
   `ProjectSync.tsx`: the event listener, the action on Push and Pull
   notices, `openStorageWorkspace(id, true)` as the target. `ProjectSidebar.tsx`
   and `StorageIndexPage.tsx`: badge text from `connection`. Reuse
   `storage-setup-guide-error` and `sync-storage-index-warning`; no new CSS
   expected.
8. **Docs.** `README.md:116-131` (the reconnection paragraph),
   `PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md` (reconnection and the instruction),
   `DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md:45-47`.

**Tests.** Rust, beside the code they protect:

- `dropbox_auth.rs`: a refused refresh records `ReconnectRequired` with
  endpoint `oauth2/token`, status 400, tag `invalid_grant`, and the request
  ID, and the message contains no token text; grant expiry compares Unix
  seconds; the existing single-flight test retargeted to the wall clock.
- `dropbox_store/tests.rs`: an expired token on a read refreshes once and
  retries (refresh count 2, request count 2, health `Connected`); an expired
  token on `upload_session/start` does not retry and no `finish_batch_v2` is
  sent; a rejected token suspends the shared store (second call makes zero
  requests, error text carries route, status, tag, and request ID, and never
  the body); `missing_scope` is `PermissionDenied`, not reconnect; a success
  after `forget` reports `Connected`; redaction canaries on every message.
- `storage_core/mod.rs`: `forget` rebuilds the entry with empty health;
  `connection_health_for` makes no request; a transition is broadcast once.
- `storage_providers/dropbox.rs`: probe errors carry their specific codes.
- `storage_setup.rs`: `storage_summary` carries runtime health after a
  refused open (`MockDropboxServer` with `deny`); a successful Test forgets
  the suspended entry; the Edit-draft instruction names the existing
  namespace, no folder, and Save changes, and the New-draft instruction is
  unchanged.
- `storage_setup.rs`, one lifecycle test for the user's item 5: setup, save,
  reload the config from the same directory, a fresh table standing in for a
  restart, one refresh, unchanged secrets digest across a `put_immutable`,
  three concurrent `header()` calls under `deny_refresh` producing one refresh
  request and one health record, then an edit draft prepared by the CLI path,
  committed, and reopened as `Connected`.
- `mallard-cli/backend.rs`: extend the existing reconnect assertion at
  `backend.rs:387` with the `reconnected` outcome and output line.

Frontend, in `tests/frontend/storage-settings.integration.test.tsx` and the
shell notice test: editor copy for each of the four `connection` states and
the "Reconnect with Codex" label; sidebar and index badge text; the storage
page notice's action expands settings; a Push `action_failed` notice gains the
action only when the storage suspends; the connection-changed event re-reads
the config.

**Validation.**

```sh
cargo test -p mallard-infrastructure --lib dropbox --locked
cargo test --locked -p mallard-infrastructure -p mallard-application -p mallard-cli --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm run build
npm run test:frontend-integration
```

Live check before release, isolated `HOME`, release build, one account:

1. Agent setup, Finish setup, one Push, quit, relaunch, one Activity refresh:
   no notice; the activity log shows one refresh request.
2. Leave the app across a four-hour gap or a laptop sleep longer than the
   grant, then refresh: one refresh, no notice.
3. In Dropbox, Settings, Connected apps, disconnect `mallard-app`. Refresh:
   one reconnect notice with evidence (`oauth2/token`, 400, `invalid_grant`,
   request ID) and the sidebar badge. Refresh again: zero Dropbox requests.
   Reconnect with the agent, Save changes: the badge clears, Push succeeds,
   the storage ID and links are unchanged.
4. Manual token: paste a short-lived token, wait for expiry, refresh: the
   token-expired notice; paste a replacement, Save changes: recovered.

| File area | Planned change |
| --- | --- |
| `crates/mallard-infrastructure/src/storage_core/connection.rs` | New: health types, `suspends`, `describe`, events channel |
| `crates/mallard-infrastructure/src/storage_core/object_store.rs` | Default `connection_health` |
| `crates/mallard-infrastructure/src/storage_core/mod.rs` | Gate, `forget`, `connection_health_for`, broadcast |
| `crates/mallard-infrastructure/src/dropbox_auth.rs` | Wall-clock expiry, `AuthFailure`, health cell, `invalidate` |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs`, `upload.rs`, `probe.rs` | Tag classification, evidence, read retry, `describe` on errors |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/mock.rs`, `dropbox_setup/mock.rs` | Real 401 shape, request IDs, new switches |
| `crates/mallard-infrastructure/src/storage_providers/dropbox.rs` | Probe codes; reconnect instruction |
| `crates/mallard-infrastructure/src/storage_providers/traits.rs` | `reconnect` on the assistant context |
| `crates/mallard-application/src/storage_setup.rs` | `StorageSummary.connection`; `forget` on probe success and removal |
| `crates/mallard-cli/src/backend.rs`, `output.rs`, `lib.rs` | `SetupOutcome`, reconnect line |
| `src-tauri/src/lib.rs` | Forward the connection event |
| `src/types.ts`, `src/components/project-sync/StorageSettings.tsx`, `ProjectLinksWorkspace.tsx`, `ProjectSync.tsx`, `ProjectSidebar.tsx`, `StorageIndexPage.tsx` | Health DTO, copy, notices, badges, event listener |
| `README.md`, `docs/PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md`, `docs/DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md` | Reconnection text |

**Not in this plan.**

- A periodic background health probe. Health is observed on the reads Mallard
  already makes; a timer would add requests to find out what the next read
  finds out anyway.
- Re-consent without a user click. Browser consent is the user's step.
- R2 credential health. S3 `InvalidAccessKeyId` and 403 can feed the same
  record later; the DTO is provider-neutral so nothing here blocks it.
- Persisting health across restarts. The user asked for runtime health; the
  first read after launch re-establishes it.
- A `--reconnect` CLI flag. The Edit draft already encodes the intent.
- Validating the refresh token at app start. That is a request per storage
  per launch to learn what the first refresh learns.

**Open question.** Whether a failed Activity refresh or repository listing
should open the storage page on its own rather than badge and notice. This
plan says no, for the same reason a Push must not be interrupted; if the user
prefers the jump, it is one call to `openStorageWorkspace(id, true)` in the
event listener.

**Deltas from the plan as written, 2026-09-09.**

- A first open whose marker read is refused for the credential keeps its
  store in the shared table (decision 2 said only successful opens were
  entered). Without it, the common case, launching the app after a
  revocation, would have re-sent the rejected credential on every open and
  never suspended. Any other marker failure still leaves no entry.
- `StorageAdapterError` carries a `String` message so a probe failure can
  print the same allowlisted evidence as a transport failure; its
  constructors take `impl Into<String>` and every existing call site is
  unchanged. Probe codes: `dropbox_reconnect_required`,
  `dropbox_token_expired`, `dropbox_permission_denied`,
  `dropbox_namespace_rejected`, `dropbox_request_failed`,
  `dropbox_rate_limited`, else `dropbox_probe_failed`.
- The transport records `Unreachable` only when no response arrived; a 5xx
  keeps its existing "result unknown" text and touches no health.
- The refresh client returns `AuthFailure` from `refresh` only; `exchange`
  and `rpc` keep `AuthError`, and `AuthFailure` converts into both
  `AuthError` and `SetupError` so the setup path is unchanged.
- The mock transport answers 401 in Dropbox's real shape with a request ID,
  and gained `expire_once`, `auth_tag`, and a refresh-capable adapter; the
  setup mock returns a request ID on `invalid_grant`. Tests that open a
  store through a registry enter the mock runtime first, as the desktop
  commands run inside Tokio.
- The CLI's `SetupOutcome { reconnected }` adds a `reconnected` field to the
  schema-1 JSON result (always present) and a distinct success line.
- The agent CTA on an existing storage always reads "Reconnect with …",
  not only after a refusal: on that page the agent's job is regaining
  access to the same destination either way.
- The frontend re-reads the config on the connection event without the
  revision gate, since a refused credential changes no file.

# OAuth loopback, sign-in and credential handling

Lead pass (the parallel auditor for this lane stalled on host sleep and saved
nothing). Snapshot 2026-09-15T09:42Z, HEAD `f226be4` + working tree; the live tree
was byte-identical at write time. **Partial:** the loopback receiver, how both
sign-ins start, and the secret type were checked; credentials at rest, secret-bearing
DTOs, log sanitizing and sign-in races were not — see the last section.

## AUTH-1 — Low: 32 junk requests to the sign-in port abort a pending sign-in; Dropbox's port is fixed
- Location: `crates/mallard-infrastructure/src/loopback_browser.rs:105-116, 140-146`;
  fixed Dropbox redirect `crates/mallard-infrastructure/src/dropbox_auth.rs:15-16`;
  production call `src-tauri/src/project_sync/dropbox_setup.rs:285-286`
- Confidence: Confirmed for the counting; Plausible for the web-page vector
  (assumption: the browser lets a page send requests to `127.0.0.1` — Chrome's
  Private Network Access rules may block some; any local process can do it)
- What happens: `receive` counts every accepted connection toward the 32-attempt
  limit, including requests that fail parsing (wrong path, no state), and returns
  the timeout error once the limit is reached. Invalid requests cannot consume the
  flow — the state check sees to that — but they can exhaust it.
  ```rust
  while Instant::now() < deadline && attempts < 32 {
      ...
      match self.listener.accept() {
          Ok((mut stream, peer)) => {
              attempts += 1;
  ```
- Failure scenario: during a Dropbox sign-in, something local sends 32 requests to
  `127.0.0.1:53682`; the user approves in the browser, but the receiver has already
  given up with "The sign-in timed out". Mallard Cloud binds an ephemeral port
  (`managed_cloud/mod.rs:123`), which a local process can still look up.
- Reachability: the Dropbox and Mallard Cloud setup lanes' browser sign-in.
  Denial of service only: no code or token can be taken (state + PKCE, below).
- Fix sketch: count only requests that reach the callback path with a well-formed
  query, and let the 180 s deadline bound everything else.

## Checked and sound
- Binds `127.0.0.1` only, re-checks that its own address is loopback, and ignores
  non-loopback peers (`loopback_browser.rs:72-81, 100-116`).
- Request parsing (`:224-288`): GET only; exactly one `Host` equal to
  `127.0.0.1:<port>`, which turns away DNS-rebinding pages (they send their own host);
  no `Content-Length` or `Transfer-Encoding`; exact callback path; no fragment; only
  `state`, the result key, `error` and `error_description`; no duplicate keys; the
  state must match before an error or a result is honoured; the result must be 1-4096
  printable ASCII bytes.
- Bounded I/O (`:98-155, 189-219`): 16 KiB and 3 s per connection, 180 s overall,
  cancellation checked on every pass and the port released before returning
  (`cancel_releases_the_listener_before_a_retry`, `:393-411`).
- The response page echoes nothing from the request and ships
  `Content-Security-Policy: default-src 'none'` with a per-response script nonce,
  `Cache-Control: no-store`, `Referrer-Policy: no-referrer` and `nosniff`;
  `history.replaceState` removes the code from browser history (`:124-138`).
- Dropbox: `state` and the PKCE verifier are 32 bytes each from `getrandom`, the
  challenge is S256, and the redirect URI is the fixed registered
  `http://127.0.0.1:53682/dropbox/callback` (`dropbox_auth.rs:15, 64-100`). A process
  that grabs port 53682 first receives a code it cannot redeem without the in-memory
  verifier.
- Dropbox refresh goes through one `tokio::sync::Mutex` over the cached grant
  (`dropbox_auth.rs:632, 663`), and a test asserts one refresh under concurrent
  callers (`:721-750`).
- Mallard Cloud: PKCE S256 from a 32-byte `getrandom` verifier held in `Zeroizing`
  (`managed_cloud/mod.rs:80-97`). The service stores each attempt's state with its
  challenge and loopback port and redeems a grant only for
  `{state, grant, code_verifier}` of that attempt (service `auth/desktop.ts:23, 47-63`),
  so a grant from another attempt cannot be injected. The authorize URL the service
  returns must be https, at most 8 KiB, and carry no userinfo (`mod.rs:129-137`).
- `SecretValue` has no `Display`, a redacted `Debug`, a 64 KiB bound and rejects
  control characters (`mallard-domain/src/secrets.rs:14-58`). `StorageConfig` no
  longer holds a plaintext secret (`mallard-domain/src/lib.rs`, `StorageConfig`), so
  **09-01 D10 is resolved**. The remaining `access_key_id: String` in
  `storage_providers/r2.rs:48` is an identifier, not the secret.
- Worth knowing: `SecretValue`'s `Serialize` writes the raw value
  (`secrets.rs:60-64`) because persistence needs it, so "never in a DTO" depends on
  each command's return type. Not audited here.

## Not examined
- Where the Dropbox refresh token and the Mallard Cloud account token live at rest
  (keychain or `~/.mallard` JSON) and with what file mode.
- Whether any Tauri command returns a secret-bearing type to the webview, other than
  the deliberate reveal/copy path in Storage settings.
- Whether error strings or activity-log messages can carry token values.
  `sanitize_message` (`activity_log.rs`) redacts only what follows fixed markers
  (`authorization:`, `bearer `, `token=`, `password=`, `x-amz-…`), so a bare Dropbox
  `sl.` token or a Mallard Cloud token with no marker in front would pass through.
- Sign-in lifecycle races: two sign-ins at once, cancel during the exchange, quitting
  mid-flow, revocation and reconnect.

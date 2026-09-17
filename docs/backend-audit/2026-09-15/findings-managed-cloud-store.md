# Managed Cloud object store and client

Lead pass (the parallel auditor for this lane stalled on host sleep and saved
nothing). Snapshot lines, HEAD `f226be4` + uncommitted tree. The service code
read for context is `mallardInternal/cloudflare/managed-cloud` (not audited).

## MC-1 — Medium: the working tree is red — the stale-writer parity test asserts the pre-01:22 upload contract
- Location: `src-tauri/src/sync_tests/thread_variants/storage_parity.rs:654-655, 671`;
  behaviour at `crates/mallard-infrastructure/src/managed_cloud/store.rs:610-615`
  and `:430-451, 508-512`
- Confidence: Confirmed (test run + code read)
- What happens: the test says "The later write loses at the commit, and its upload
  is abandoned so the next push is free" and asserts no upload is left `open`
  right after the stale publish fails. `commit_head` was changed (file mtime
  2026-09-15 01:22, after the test's 2026-09-14 21:53) to keep the losing upload
  open on purpose:
  ```rust
  // The upload is stale now. It stays open so the next push, built
  // on the moved head, reuses the objects it carried; that push ends
  // it, or it expires.
  Err(error) if error.kind == ErrorKind::HeadChanged => {
      Ok(CasOutcome::Conflict { current_etag: None })
  }
  ```
  `begin_upload` now ends the account's own stale uploads only after the next
  upload exists (`:448-451, 508-512`), so objects the stale upload carried stay
  protected from the service's orphan cleanup and are not re-sent.
- Failure scenario: `cargo test --workspace --lib` fails in `mallard`, and because
  cargo stops at the first failing crate, the other four crates' tests do not run
  at all unless `--no-fail-fast` is passed.
- Reachability: every test run of the tree as it stands.
- Fix sketch: the store behaviour is intentional and coherent with the service
  (see MC-2 for its one gap), so update the test: assert the stale upload is still
  `open` and reported `stale`, then push again and assert the next push ended it.

## MC-2 — Low: a stale upload kept open for reuse can block the next push at the quota edge for up to 24 h
- Location: `crates/mallard-infrastructure/src/managed_cloud/store.rs:491-507`
  (only `ErrorKind::UploadInProgress` triggers "end my stale uploads and retry");
  `client.rs:570` maps `quota_exceeded` to `ErrorKind::Quota`
- Confidence: Confirmed against the service code (`routes/uploads.ts:178-179, 214-220`;
  `validate/limits.ts:9, 13`)
- What happens: since MC-1's change, a push that loses the head race leaves its
  upload open. The service's quota guard counts every open upload's reservation,
  stale or not — `committed_bytes + new reservation + Σ open reservations ≤ 4 GiB`.
  A reservation is the bytes that were not yet stored when that upload was
  created. The next push creates its upload first and ends the stale one only on
  success (`:508-512`); the retry arm that ends stale uploads first fires only on
  `upload_in_progress`:
  ```rust
  Err(error) if error.kind == ErrorKind::UploadInProgress && !stale.is_empty() => {
      for open in &stale { client.abandon_upload(&repository_id, &open.id).await?; }
  ```
- Failure scenario: a repository close to its 4 GiB quota; device B's push loses
  the race to device A and leaves X bytes reserved; B compares and pushes again
  with Y new bytes. If `committed + X + Y` exceeds the quota the create fails with
  `quota_exceeded`, the stale upload is not ended, and every retry fails the same
  way until the service's cron expires it (`uploadLifetimeSeconds` = 24 h). Ending
  it would have freed X.
- Reachability: Push to Mallard Cloud after a lost CAS race, near the quota only.
- Fix sketch: treat `ErrorKind::Quota` like `UploadInProgress` in that retry arm
  (end own stale uploads, retry once).
- Related leftover: an own upload that is *not* stale is never ended by the next
  push. The filter is `open.stale && open.commit_id != manifest.commit_id`
  (`:448-451`), so an upload whose commit call failed without applying (network
  drop, app quit — see MC-4) keeps its reservation and its slot in the
  64-per-member cap until the 24 h expiry. Harmless until the quota edge or 64
  such failures in a day; the same fix should end every own open upload with a
  different commit id once the new upload exists.

## MC-3 — Medium: a failed Push keeps every staged object of that push in memory for the life of the app
- Location: `crates/mallard-infrastructure/src/managed_cloud/store.rs:773-776`
  (insert), `:582` (the only removal); the store is cached app-wide in
  `storage_core/mod.rs:328-353` (`SHARED_STORES`, reused until the storage's
  configuration changes)
- Confidence: Confirmed for retention; Plausible for growth (assumption: the
  session transcript changes between retries)
- What happens: `put_many` keeps every object body of a push in
  `staged[bundle_id]` until the manifest write, because the service's upload is
  created from the manifest. The map entry is cleared only at the start of
  `commit_head`. Every failure before that point — `create_upload` refused
  (quota, cap, revision moved), an object PUT failing, a missing object, the app
  losing the network — returns without clearing it, and the shared store outlives
  the operation.
  ```rust
  Key::Object { bundle_id, sha256 } => { ...
      Self::lock(&self.inner.staged)?.entry(bundle_id).or_default().insert(sha256, bytes);
  ```
- Failure scenario: a 136 MB Push (the size measured for HorrorGame on 09-11)
  fails at `create_upload`; 136 MB stays resident. The user retries while the
  session is still active; each retry stages a new transcript version under a new
  digest beside the old ones, so resident memory grows by roughly the transcript
  size per failed retry until a push of that bundle finally commits or the app
  restarts.
- Reachability: Push to Mallard Cloud on a flaky network, near the quota, or
  after a revision race.
- Fix sketch: clear `staged[bundle_id]` (and `pending`) on every error return of
  `begin_upload`, and at the start of each publish; or stage to temp files keyed
  by digest instead of heap `Bytes`.

## MC-4 — Low: a commit whose response is lost is reported as a failed Push; the S3 store resolves the same ambiguity
- Location: `crates/mallard-infrastructure/src/managed_cloud/store.rs:598-617`;
  `client.rs:461-470` (`retry::never()`), `client.rs:722-838` (a fresh
  `Idempotency-Key` per call); the engine propagates CAS errors with `?`
  (`bundle_engine.rs:665, 1104`)
- Confidence: Confirmed (code path); the service does replay a repeated key
  (`routes/uploads.ts:386-398`)
- What happens: if the commit request times out or the connection drops after the
  service applied it, `commit_upload` returns `Unreachable` and `commit_head`
  returns `Err`, not `Written`. `s3_store` resolves ambiguous CAS outcomes by
  verified read-back (`resolve_cas_write`, `s3_store.rs:746-778`); this store does not, and the
  fresh idempotency key means a retry could not be replayed safely either.
- Failure scenario: the user's Push actually landed but the app reports it failed
  and leaves local sync state at the old head; the next Push fails with "The
  Mallard Cloud snapshot changed since it was compared" against their own commit,
  and only a fresh compare converges. If the commit did not land, the open upload
  lingers (MC-2).
- Reachability: Push to Mallard Cloud with a dropped connection during commit.
- Fix sketch: on `Unreachable`/timeout from `commit_upload`, read the head by
  bundle; if it names the pending commit id return `Written`, otherwise re-send
  the commit once with the same idempotency key.

## Checked and sound
- `stub.rs` does not ship: `#[cfg(any(test, feature = "test-support"))]`
  (`managed_cloud/mod.rs:7-8`).
- Release builds talk to `https://mallard-ai.com` only: `origin_allowed`
  (`mod.rs:40-42`) runs in `validate_settings` (`storage_providers/managed_cloud.rs:100-112`),
  which `decode()` calls before every client is built (`:37-39, 58-70`), including
  the website bridge's `client_for` (`:53-57`); stores open only after
  `validate_config` (`storage_core/mod.rs:407-412`). Debug builds accept
  `MALLARD_MANAGED_CLOUD_ORIGIN` (`mod.rs:26-36`).
- HTTP client: redirects disabled, retries disabled, 15 s connect / 60 s read /
  600 s total timeouts (`client.rs:461-470`); origin re-parsed at construction
  (`:462`).
- Response bodies are bounded both by `Content-Length` and while streaming
  (`client.rs:860-887, 900-912`); a declared `x-mallard-sha256` that disagrees
  with the received bytes is rejected (`:889-891`).
- Verify-before-use holds: `stage_verified_async` streams at most `expected.size`
  bytes into a temp file and returns `Verified` only on an exact size + SHA-256
  match (`store.rs:316-344`).
- Uploaded objects are hashed against their key before staging (`store.rs:769-772`);
  heads and tags are writable only through compare-and-swap (`:790-792`); the
  `_uploads/` probe namespace and catalog keys are refused (`:788-789`).
- Head CAS is real on the service: `Absent` → `expected_commit_id: null` →
  guard `private_publication_id IS NULL`; `Match` → guard on the current commit
  id; every commit is also pinned to the private revision the upload was created
  against (`routes/uploads.ts:386-460`). A lost race surfaces as `HeadChanged` →
  `CasOutcome::Conflict` (`store.rs:613-614`).
- A head read is checked to name the requested bundle and kind before use and is
  re-validated as a `BundleHead` (`store.rs:256-270`).
- PKCE S256 with a 32-byte `getrandom` verifier kept in `Zeroizing`
  (`mod.rs:80-97`); the authorize URL the service returns must be https (http only
  in debug builds), ≤ 8 KiB, and carry no userinfo (`mod.rs:129-137`).
- The website link's host is the validated storage origin; only the path is
  server-supplied (`managed_cloud_link.rs:98`).

## Status of known items
- D7 (unbounded `head.manifest_size`) applies here too: the staged download is
  bounded by `expected.size`, which comes from the service's head answer
  (`store.rs:266-267, 330`) — a compromised service can make the client stream an
  arbitrarily large manifest into a temp file before the digest check fails.
- D11 (`BundleTag` has no validate) applies: tags are synthesized from the
  listing's `display_name`, counts and providers without a length bound
  (`store.rs:215-246`); providers are at least filtered through
  `ProviderKey::parse`.

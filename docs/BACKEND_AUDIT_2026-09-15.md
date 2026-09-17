# Mallard backend audit — code added since 2026-09-01 (partial)

- **Audit date:** 2026-09-15
- **Baseline:** branch `main`, `HEAD` = `f226be4`, plus the uncommitted working
  tree: 31 changed tracked files and 10 untracked Rust files (the Managed Cloud
  desktop wiring, 5,651 lines). The Rust sources were frozen as a snapshot at
  09:42 UTC. At write time all 169 `.rs` files were byte-identical to it, so every
  line number below is current.
- **Scope:** correctness, security and data integrity of the backend Rust added
  since the last correctness audit
  ([`BACKEND_AUDIT_2026-09-01.md`](BACKEND_AUDIT_2026-09-01.md), baseline
  `dd8d420`): 12 commits, about 21,000 lines in newly added non-test files, plus the
  uncommitted Managed Cloud code. Older code was not re-audited — 09-01 verified its
  safety invariants, and the 09-04, 09-06 and 09-11 audits covered performance.
- **Status: partial.** The plan was eight areas, seven of them handed to parallel
  read-only auditors. All seven stalled when the Mac went to sleep about 20
  minutes in, before saving anything, and two resume rounds produced no further
  work. Everything below was verified directly at the cited lines. Two areas are
  covered (Managed Cloud storage, sign-in) plus several cross-cutting sweeps; six
  are listed under [Not examined](#not-examined).
- **Audit only** — no source file was modified.
- Supporting notes: [`backend-audit/2026-09-15/`](backend-audit/2026-09-15/).

---

## Verification baseline

| Command | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | exit 0, no warnings |
| `cargo clippy --workspace --all-targets` | exit 0, **79 distinct warnings** (09-01: 0, 09-04: 2) |
| `cargo clippy --workspace` (production targets only) | **37 distinct warnings** |
| `cargo test --workspace --lib --no-fail-fast` | **1,066 passed, 1 failed, 11 ignored** |

Per crate: `mallard` (src-tauri) 326 / **1 failed** / 6 ignored,
`mallard-infrastructure` 605 / 0 / 5, `mallard-domain` 86 / 0 / 0,
`mallard-cli` 28 / 0 / 0, `mallard-application` 21 / 0 / 0.

**Watch the flag.** A plain `cargo test --workspace --lib` stops at the failing
`mallard` crate and reports `326 passed; 1 failed`. The other four crates — 740
tests — never run.

---

## Executive assessment

**What was examined is in good shape.** The Managed Cloud desktop store — the
newest and largest uncommitted piece — honours the storage contract that 09-01
verified for the S3 and local stores:
- downloads are size-bounded and checked against their SHA-256 before use;
- uploads are hashed against their keys;
- the head moves only through a compare-and-swap that the service enforces
  twice (expected commit id plus a private-revision pin);
- release builds can reach only `https://mallard-ai.com`;
- the HTTP client follows no redirects and retries nothing.

The shared loopback sign-in receiver is carefully built, and every place the
backend launches another program either passes an argument vector or quotes
correctly.

**The defects cluster in one place: upload lifecycle state.** A redesign landed
at 01:22 today. It keeps a losing upload open so the next push can reuse its
objects. The idea is sound, but three loose ends follow from the store keeping
per-push state in an app-wide shared object and cleaning it up only on the
success path:
- a failed push leaves its bytes in memory (MC-3);
- some failures leave an open upload on the service (MC-2);
- a lost commit response reads as a failed push (MC-4).

The redesign also left its own test behind, so **the tree is red** (MC-1).

**Build health slipped:**
- clippy went from 2 warnings to 79 in eleven days (LEAD-4);
- the loopback-receiver move is split between the git index and untracked files,
  so committing the index alone would produce a commit that doesn't build (LEAD-3).

| | |
| --- | --- |
| Product defects | **0 Critical, 0 High, 1 Medium, 4 Low**, 1 Info |
| Build and test health | the suite is red (MC-1 — Medium for the development loop, not counted above), clippy 2 → 79 (LEAD-4), a commit hazard (LEAD-3) |
| Areas examined / planned | **2 of 8**, plus cross-cutting sweeps |
| Earlier findings re-checked | D10 resolved; D7 and D11 also apply to Managed Cloud |

---

## Severity key

- **Critical** — reachable in normal use; loses or corrupts user sessions or
  files, or sends a secret off the machine.
- **High** — reachable in normal use, or by a semi-trusted storage/server/local
  process, with significant impact (silent overwrite, success reported for a
  failure, data restored to the wrong place, auth boundary bypassed).
- **Medium** — needs unusual but plausible conditions (a flaky network, a crash at
  a specific point, two machines racing), or a check present on one path but
  missing on its sibling.
- **Low** — latent hazard, defence in depth, misleading error.
- **Info** — observation, no defect.

---

## Act first: the tree is red

### MC-1 — the stale-writer parity test asserts the pre-redesign upload contract

**[`storage_parity.rs:654-671`](../src-tauri/src/sync_tests/thread_variants/storage_parity.rs#L654)**
against **[`managed_cloud/store.rs:610-615`](../crates/mallard-infrastructure/src/managed_cloud/store.rs#L610)**

`managed_cloud_noop_fast_forward_views_and_stale_writer_match_local_protocol`
fails with
`assertion failed: state.uploads.values().all(|upload| upload.state != "open")`.
The test says "the later write loses at the commit, and its upload is abandoned so
the next push is free".

The store no longer does that, on purpose. `store.rs`, `client.rs` and `stub.rs`
were all edited at 2026-09-15 01:22, after the test (2026-09-14 21:53):

```rust
// The upload is stale now. It stays open so the next push, built
// on the moved head, reuses the objects it carried; that push ends
// it, or it expires.
Err(error) if error.kind == ErrorKind::HeadChanged => {
    Ok(CasOutcome::Conflict { current_etag: None })
}
```

`begin_upload` now ends the account's own stale uploads only after the next
upload exists (`store.rs:430-451, 508-512`). Until then the service's orphan
cleanup leaves the stale upload's objects alone, so the next push can reuse them
instead of re-sending them.

**Fix.** Update the test to the new contract: assert that the stale upload is
still `open` and reported `stale`, then push again and assert that push ended it.
Severity here is about the development loop, not users: the red suite hides
regressions and, without `--no-fail-fast`, 740 tests.

### LEAD-3 — the loopback move is split across the index and untracked files (Info)

`crates/mallard-infrastructure/src/dropbox_browser.rs` is **staged** as deleted;
its replacement `loopback_browser.rs` is **untracked**; and the `lib.rs` change
that swaps `pub mod dropbox_browser;` for `pub mod loopback_browser; pub mod
managed_cloud;` is **unstaged**. Committing the index as it stands produces a
commit whose `lib.rs` declares a module whose file is gone. Stage the three
together, with the other nine untracked Managed Cloud files, when committing.

### LEAD-4 — clippy regressed from 2 to 79 warnings (Info)

79 distinct under `--all-targets`, 37 in production code. By lint: 21
`needless_borrow`, 18 `unnecessary_to_owned`, 11 `too_many_arguments`, 10
`cloned_ref_to_slice_refs`, 5 `needless_borrows_for_generic_args`, 4
`nonminimal_bool`, 3 `empty_line_after_doc_comments`, 7 others. By file:
`service.rs` 23, the two live Codex e2e tests 19, `conversation_materialization.rs`
8, `claude/history.rs` 4.

The production `too_many_arguments` sites are the ones worth more than a
suppression — the 09-04 audit (O7) suggested a borrowed context struct for the
same shape:
- `service.rs:1333, 3799, 4357, 10705, 10897`
- `owned_subagents.rs:54, 188`
- `codex_standalone_builder.rs:262`
- `project_sync/r2_setup.rs:492`
- `mallard-application/src/dropbox_setup.rs:17`

The mechanical lints are `cargo clippy --fix` material.

---

## Product defects

### MC-3 — Medium: a failed Push keeps every staged object of that push in memory

**[`managed_cloud/store.rs:769-777`](../crates/mallard-infrastructure/src/managed_cloud/store.rs#L769)**
(insert) and **[`:582`](../crates/mallard-infrastructure/src/managed_cloud/store.rs#L582)**
(the only removal)

The service creates an upload from the manifest, so `put_many` holds every object
body of a push in `staged[bundle_id]` until the manifest arrives:

```rust
Key::Object { bundle_id, sha256 } => { ...
    Self::lock(&self.inner.staged)?.entry(bundle_id).or_default().insert(sha256, bytes);
```

The entry is cleared only at the start of `commit_head`. Every failure before
that point returns without clearing it:
- `create_upload` refused (quota, the per-member cap, revision moved);
- an object PUT fails;
- a needed object is missing;
- the network drops.

The store itself is not per-operation. Push opens it through
`ConfiguredObjectStore::shared` (`service.rs:13902`), and the app-wide
`SHARED_STORES` table keeps it for as long as the storage's configuration is
unchanged (`storage_core/mod.rs:328-372`).

**Failure scenario.** A 136 MB Push (the size the 09-11 audit measured for
HorrorGame) fails at `create_upload`, and the 136 MB stays resident. The user
retries while the session is still active. Each retry stages a new transcript
version under a new digest beside the old ones, so memory grows by about the
transcript size per failed retry. It is released only when a push of that bundle
commits, the storage is reconfigured or re-probed, or the app restarts.
Retention is confirmed; the growth assumes the transcript changes between retries.

**Fix.** Clear `staged[bundle_id]` and `pending[bundle_id]` on every error return
of `begin_upload`. Or stage to temp files keyed by digest instead of heap `Bytes`.

### MC-2 — Low: leftover open uploads on the service

**[`managed_cloud/store.rs:448-512`](../crates/mallard-infrastructure/src/managed_cloud/store.rs#L448)**

Two gaps in cleaning up the account's own open uploads. Both follow from the
service's rules: every open upload's reservation counts against the 4 GiB
repository quota, stale or not, and each member may hold 64 open uploads
(service `routes/uploads.ts:178-220`, `validate/limits.ts:8-13`).

1. **The quota retry is missing.** The "end my stale uploads and retry once" arm
   fires only on `upload_in_progress` (`store.rs:496-505`), never on
   `quota_exceeded` (mapped to `ErrorKind::Quota`, `client.rs:570`). Near the
   quota, a push that follows a lost race fails with `quota_exceeded` until the
   stale upload expires — up to 24 h — although ending it would have freed its
   bytes.
2. **Own non-stale leftovers are never ended.** The cleanup filter is
   `open.stale && open.commit_id != manifest.commit_id` (`:448-451`). An upload
   whose commit call failed without applying (see MC-4) is not stale, so it keeps
   its reservation and its cap slot until expiry.

Both are harmless away from the quota edge or 64 failures a day.

**Fix.** Once the new upload exists, end every own open upload with a different
commit id. Treat `Quota` like `UploadInProgress` in the retry arm.

### MC-4 — Low: a commit whose response is lost is reported as a failed Push

**[`managed_cloud/store.rs:598-617`](../crates/mallard-infrastructure/src/managed_cloud/store.rs#L598)**

If the commit request times out, or the connection drops after the service applied
it, `commit_upload` returns `Unreachable` and `commit_head` returns `Err`, not
`Written`. The engine propagates head-write errors unchanged
(`bundle_engine.rs:665, 1104`). The client never retries (`client.rs:465`), and it
mints a fresh `Idempotency-Key` per call (`client.rs:722-838`), so the service's
replay of a repeated key (`routes/uploads.ts:395-398`) is never used. `s3_store`
resolves the same ambiguity by reading the head back and verifying it
(`resolve_cas_write`, `s3_store.rs:746-778`).

**Failure scenario.** The push landed, but the app reports a failure and keeps
local sync state at the old head. The next Push fails with "The Mallard Cloud
snapshot changed since it was compared" — against the user's own commit — and
only a fresh compare converges.

**Fix.** On `Unreachable` from `commit_upload`, read the head by bundle. If it
names the pending commit id, return `Written`. Otherwise re-send the commit once
with the same idempotency key.

### AUTH-1 — Low: 32 junk requests to the sign-in port abort a pending sign-in

**[`loopback_browser.rs:105-116`](../crates/mallard-infrastructure/src/loopback_browser.rs#L105)**

`receive` counts every accepted connection toward its 32-attempt limit, including
requests that fail parsing, and gives up with the timeout error once the limit is
reached. The state check stops a stray request from *consuming* the sign-in, but
not from *exhausting* it.

Dropbox's callback port is fixed at 53682 by the registered redirect URI
(`dropbox_auth.rs:15-16`). So during a Dropbox sign-in, any local process — or a
web page, if the browser allows it to reach `127.0.0.1` — can make the user's
approval arrive after the receiver has quit. Mallard Cloud's port is ephemeral
(`managed_cloud/mod.rs:123`) but still discoverable locally. This is denial of
service only: no code or token can be taken, because state and PKCE hold (see
[Verified sound](#verified-sound)).

**Fix.** Count only requests that reach the callback path with a well-formed
query, and let the 180 s deadline bound the rest.

### LEAD-1 — Low: `ManagedCloudClient::list_members` has no caller

**[`managed_cloud/client.rs:707`](../crates/mallard-infrastructure/src/managed_cloud/client.rs#L707)**

A `pub async fn` with zero references anywhere, tests included, in code that has
not shipped. `AGENTS.md` asks for no speculative surface. Delete it, or wire it up
when a Members view exists.

### LEAD-2 — Info: `prepare_pull_transcript` is a test-only entry point

**[`pull_transcript.rs:77-88`](../crates/mallard-infrastructure/src/pull_transcript.rs#L77)**

A thin wrapper that passes an empty identity map. Production calls
`prepare_pull_transcript_with_identities` directly (`service.rs:12652`), so the
module's unit tests never run a non-empty identity map — the child-identity path
is covered only by the service-level suites. This is 09-04 O3 in a milder form.
Drop the wrapper, or add one unit case with a non-empty map.

---

## Verified sound

Recorded so they are not re-audited or "simplified" away.

**Managed Cloud store and client** (`crates/mallard-infrastructure/src/managed_cloud/`)
- The stub server does not ship: `#[cfg(any(test, feature = "test-support"))]`
  (`mod.rs:7-8`).
- Release builds can reach `https://mallard-ai.com` only. `origin_allowed`
  (`mod.rs:40-42`) runs in `validate_settings`
  (`storage_providers/managed_cloud.rs:100-112`), and `decode()` calls it before
  every client is built (`:37-39, 58-70`) — including the website bridge's
  `client_for` (`:53-57`). Stores open only after `validate_config`
  (`storage_core/mod.rs:407-412`). The 09-01 D1 pattern — validated at entry,
  not where the value is used — does not recur here.
- The HTTP client follows no redirects and makes no retries. Timeouts are 15 s
  connect, 60 s read, 600 s total (`client.rs:461-470`).
- Response bodies are bounded both by `Content-Length` and while streaming
  (`client.rs:860-887, 900-912`). A declared `x-mallard-sha256` that disagrees with
  the bytes received is rejected (`:889-891`).
- Verify-before-use: `stage_verified_async` streams at most `expected.size` bytes
  into a temp file and returns `Verified` only on an exact size and SHA-256 match
  (`store.rs:316-344`).
- Objects are hashed against their key before staging (`store.rs:769-772`). Heads
  and tags are writable only through compare-and-swap (`:790-792`). The probe
  namespace and catalog keys are refused (`:788-789`).
- The compare-and-swap is real on the service:
  - `Absent` → `expected_commit_id: null` → guard `private_publication_id IS NULL`;
  - `Match` → guard on the current commit id;
  - every commit is also pinned to the private revision its upload was created
    against (service `routes/uploads.ts:386-460`);
  - a lost race surfaces as `CasOutcome::Conflict` (`store.rs:613-614`).
- A head answer must name the requested bundle and kind, and is re-validated as a
  `BundleHead` (`store.rs:256-270`).
- The website link's host is the validated storage origin; only the path comes
  from the server (`project_sync/managed_cloud_link.rs:98`).

**Sign-in** (details in
[`findings-auth-credentials.md`](backend-audit/2026-09-15/findings-auth-credentials.md))
- The loopback receiver binds `127.0.0.1` only and accepts only a GET with exactly
  one `Host` of `127.0.0.1:<port>` — which turns away DNS-rebinding pages — and no
  body. It requires the exact path, rejects duplicate keys, and demands an exact
  `state` match before anything is honoured. It is bounded at 16 KiB, 3 s per
  connection and 180 s overall, and the page reflects nothing under a strict nonce
  CSP (`loopback_browser.rs:72-288`).
- Both sign-ins use PKCE S256 with 32-byte `getrandom` secrets
  (`dropbox_auth.rs:64-100`, `managed_cloud/mod.rs:80-97`). Mallard Cloud's grant is
  redeemable only with the state and verifier of the attempt that started it
  (service `auth/desktop.ts:23, 47-63`), so neither a squatted port nor a login-CSRF
  grant yields a session.
- `SecretValue` has no `Display`, a redacted `Debug`, a 64 KiB bound, and no
  control characters (`mallard-domain/src/secrets.rs:14-58`).

**Process launches** — all argv-based or correctly quoted (the R2 `wrangler`
invocation was not examined):
- `providers/launch.rs:3-28`: POSIX single-quote quoting, then AppleScript-literal
  escaping.
- `providers/codex/actions.rs`: `terminal_resume_command` validates a UUID thread
  id and absolute paths (`:194-210`); the deep links percent-encode the path
  (`:36-67`).
- `providers/claude/actions.rs:55-127`: argv only.
- `codex_binary.rs:136-185`: the binary name must be `[A-Za-z0-9_-]` before it
  reaches `command -v`.
- `codex_app_refresh.rs:192-391`: the app bundle is validated (no symlinks,
  canonical, plist identity) and every command is time-bounded.
- `storage_catalog/adapters.rs:106-150`: argv with timeout, kill and bounded pipes.
  *Whether that argv can be steered by plugin intents synced from storage is still
  open* — see [Not examined](#not-examined).

**Other sweeps**
- Transcript previews cannot split a UTF-8 character
  (`providers/codex/history.rs:2552-2556`, `providers/claude/history.rs:1565-1569`).
- A Git remote URL with an embedded `user:token@` part cannot leak the token:
  `normalize_github_repository_url` rebuilds `https://github.com/{owner}/{repo}`
  from validated segments (`project_activity.rs:1738-1790`).
- The three production `git` calls pass the project root as the value of `-C` with
  fixed subcommands (`project_activity.rs:1805`, `session_detail.rs:1548`,
  `service.rs:2798`).
- No production code writes git hooks.
- The bundled `mallard-cli` cannot Push, Pull or apply. Its surface is
  `storage list | verify | r2 setup | r2 accounts | r2 discover`
  (`crates/mallard-cli/src/args.rs:21-122`), and `r2 setup` fills an existing draft
  pinned to `--expected-revision`.
- The only uncommitted dependency change is reqwest's `multipart` feature (adds
  `mime_guess`, `unicase`); the loopback `hyper` server stays behind `test-support`.

---

## Status of earlier findings

| Finding (09-01) | Status on this tree |
| --- | --- |
| D10 — `StorageConfig` derives `Debug` with a plaintext secret | **Resolved.** Secrets are `SecretValue` in a `SecretMap`, both with redacted `Debug`. R2's remaining `access_key_id: String` (`storage_providers/r2.rs:48`) is an identifier, not the secret. |
| D7 — `head.manifest_size` has no upper bound | **Still open, and it applies to Managed Cloud.** The manifest download is bounded by `expected.size`, which comes from the service's head answer (`store.rs:266-267, 330`). |
| D11 — `BundleTag` has no `validate()` | **Still open, and it applies to Managed Cloud.** Tags are synthesized from the listing's `display_name`, counts and providers with no length bound; providers are at least parsed (`store.rs:215-246`). |

Not re-verified this session: 09-01 D1–D6, D8, D9, D12–D14 and A1–A5; 09-04
A1–A13, B1–B3 and O1–O7; the nine 09-11 findings.

---

## Not examined

These areas were planned and assigned, but nobody reviewed them. Each row lists
the questions still open.

| Area | Files | Open questions |
| --- | --- | --- |
| Dropbox store and storage catalog | `storage_core/dropbox_store.rs` (+ `upload`, `probe`), `storage_core/connection.rs`; `storage_catalog/{registry,plugin_intent,agent_skill,codec,resource_engine,apply}.rs` | How head CAS maps onto Dropbox (rev-based update or autorename?); case-insensitive key collisions; 429 and retry handling. Status of D3/D4/D5. **Plugin intents synced from storage become the provider CLI's argv** — option injection, and whether the approved plan shows exactly what will run. |
| Storage setup and R2 | `mallard-application/src/storage_setup.rs` (2,937 lines), `storage_setup_journal.rs`, `r2_setup.rs` (+ `discovery`), `project_sync/r2_setup.rs`, `mallard-cli/src/r2_browser_setup.rs` | How `wrangler` is invoked and where its output goes; commit only after the capability probe, for every provider; journal crash recovery; storage collisions and silent re-pointing. Status of D1/D2. |
| Pull and restore | `pull_transcript.rs` (+206 uncommitted lines), `verified_transcript.rs`, `conversation_materialization.rs`, `codex_thread_allocator.rs`, `bounded_preparation.rs` | Whether the new write paths go through the verified restore helpers; partial-failure state; Claude sidecar and child installs. Status of D12. |
| Push capture and owned subagents | `codex_subagents.rs`, `providers/claude/subagents.rs`, `owned_subagents.rs`, `session_path_map.rs`, the credential-redaction changes, `portable.rs`, `bounded.rs` | Whether children and sidecars are redacted and path-mapped exactly like parents; settings read at review vs. build; child id collisions in the manifest. |
| Command surface and concurrency | `src-tauri/src/lib.rs`, `commands.rs`, `session_watch_service.rs`, `config_watcher.rs`, `app_reset.rs`, the `persistence.rs` changes | Single-flight coverage across Push/Pull/reset; blocking work on the async runtime; reset scope; swallowed errors; frontend calls to command names that are not registered. Status of D8/D9. (Checked: all 103 registered commands — 87 on 09-09 — have their name quoted under `src/`.) |
| Credentials, second half | `dropbox_auth.rs`, the setup lanes, `activity_log.rs` | Tokens at rest and file modes; secret-bearing command DTOs; token values in errors or logs — `sanitize_message` strips only what follows fixed markers, so a bare token passes; sign-in races. |

---

## Recommended order of work

1. **Now (minutes).**
   - Update the parity test to the new upload contract (MC-1).
   - Stage the loopback move together with the untracked Managed Cloud files before
     committing (LEAD-3).
   - Run `cargo clippy --fix` for the mechanical lints (LEAD-4).
2. **Before Managed Cloud leaves staging.**
   - MC-3: clear staged state on every failure.
   - MC-2: end own leftovers once the new upload exists; retry on `Quota`.
   - MC-4: read back after a lost commit.
   - Then run the staging exit checks the wiring plan still lists as not run.
3. **Finish the audit**, highest stakes first:
   - Pull/restore and Push capture — they write into provider homes and decide what
     leaves the machine;
   - then Dropbox and the catalog — shared-namespace writers and plugin argv;
   - then storage setup / R2 and concurrency.

   The lane questions above carry over as-is.

## Method and limits

- The Rust sources were frozen before reading, and `shasum -c` against the snapshot
  confirmed the live tree was unchanged at write time.
- Every finding was read at its cited lines and traced from caller to callee. Claims
  about the Mallard Cloud service rest on reading its code in
  `mallardInternal/cloudflare/managed-cloud`, not on calling it; the service itself
  was not audited.
- No probes or measurements were taken this time, and the Managed Cloud staging exit
  checks were not run.

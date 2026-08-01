# Mallard — 100 Improvement Suggestions, Sorted

- **Reviewed:** July 31, 2026 (two passes: survey, then code-path verification)
- **Against:** `main` @ `50812a0`, app version 0.1.2
- **Method:** full read of `README.md`, `AGENTS.md`, `TECHNICAL_REPORT.md`,
  `docs/TODO.md`, and the two in-flight design docs; static analysis of the Rust
  and TypeScript trees; a second pass reading the publish/CAS/transport paths
  directly; a clean `npm run build`; a full `cargo test --lib` run; `npm audit`;
  CI inspection; and web research into eight comparable projects and two
  emerging standards.
- **Verified during this review:** `npm run build` passed;
  `cargo test --manifest-path src-tauri/Cargo.toml --lib` passed with
  **282 tests, 0 failures** in 34.1s.

Every item is numbered in **global priority order** (1 = do first). Each carries
a tier, an effort estimate, and an impact estimate.

- Tier: **P0** ship-blocking · **P1** high · **P2** medium · **P3** low ·
  **P4** speculative / long-horizon
- Effort: **S** ≤1 day · **M** ≤1 week · **L** ≤1 month · **XL** >1 month

---

## Executive summary

Mallard is unusually well-engineered for its age (90 commits, 4 contributors).
The Rust core is disciplined — **~5 `.unwrap()`/`.expect()` calls in the entire
production path**, zero `as any` casts in TypeScript, zero stray `console.*`,
strict TS with unused-symbol checks, and a genuinely serious safety model
(CAS-published immutable manifests, SHA-256 binding at every boundary,
no-follow traversal, plan-based apply with digest revalidation, opt-in
non-recursive deletion). The domain modelling in `TECHNICAL_REPORT.md` is
better than most funded products.

The gaps are not in the core algorithm. They are in **four clusters**:

1. **Confidentiality.** Mallard uploads full agent transcripts — which routinely
   contain source code, `.env` dumps, and tool output — to a shared R2 bucket
   **in plaintext**, and stores the R2 secret key in plaintext JSON. The closest
   competitor ([claude-sync](https://github.com/tawanorg/claude-sync), 237★)
   ships age-based end-to-end encryption as its *headline* feature. This is the
   single biggest competitive and trust gap.
2. **Engineering safety net.** CI runs **only on tag push**, and never runs the
   Rust suite. There is no clippy, rustfmt check, ESLint, Prettier, dependency
   automation, or vulnerability scan. 282 excellent Rust tests currently defend
   nothing on a pull request.
3. **Distribution reality.** The website advertises Windows and Linux builds;
   CI produces exactly one artifact — an ad-hoc-signed, unnotarized Apple
   Silicon DMG, with no auto-updater. Every user is a manual Gatekeeper
   override away from the app, and permanently stranded on whatever version
   they installed.
4. **Scale headroom.** `MAX_CONVERSATION_FILE_BYTES` is 100 MiB
   ([domain.rs:41](../src-tauri/src/project_sync/domain.rs:41)). Real Codex
   rollouts are documented at
   [700 MB–1.95 GB](https://github.com/openai/codex/issues/24948) with a 91 GB
   sessions directory. Mallard hard-blocks the sessions its power users care
   most about.

**Top 10, in order:** #1 encrypt bundles · #2 CI on every PR · #3 credentials to
OS keychain · #4 probe conditional-write support · #5 set a CSP · #6 run Rust
tests before shipping a DMG · #7 notarize · #8 auto-updater · #9 lift the
100 MiB ceiling · #10 clippy + fmt gates.

> **Second-pass note.** Item #4 was found by reading the transport path directly
> and is the only item in this document that undermines a guarantee the project
> currently *advertises*. It is cheap to fix and should not wait behind #1–#3.

---

## What is already strong (do not regress these)

These are load-bearing and rare. Any refactor below must preserve them.

- Immutable publish-before-head-CAS ordering with `If-None-Match`/`If-Match`
  ([s3_store.rs:174-178](../src-tauri/src/project_sync/s3_store.rs:174)) and
  explicit `PutAttempt::Ambiguous` reconciliation on timeout — the hard case,
  handled correctly.
- Every trust boundary is a SHA-256, not a timestamp or a counter — including
  the manifest-only refresh decision table, which explicitly refuses to consult
  a clock.
- Object-level dedup already lands: `publish_skips_payloads_already_in_previous_manifest`
  and `publish_dedupes_identical_bytes_within_one_push` are real, tested
  behaviors in `bundle_engine.rs`.
- Bulk get/put already run under a bounded-concurrency semaphore with batching
  ([s3_store.rs:394, :439](../src-tauri/src/project_sync/s3_store.rs:394)).
- Deletion is opt-in, digest-verified, backed up, and non-recursive by design.
- `_local_`-prefixed metadata stripping keeps machine identity out of portable
  manifests, with a dedicated test
  (`portable_objects_do_not_leak_machine_local_identity_or_paths`).
- Codex `archived_sessions` is already handled alongside `sessions` in all six
  places that walk the provider home — easy to get wrong, and not wrong here.
- Cloudflare `cfat_`-prefixed API tokens are correctly SHA-256-derived into the
  S3 secret ([s3_store.rs:581](../src-tauri/src/project_sync/s3_store.rs:581)) —
  a detail most integrations get wrong.
- 282 Rust tests including a 20,000-resource stress boundary, checkpoint
  recovery fuzzing, and local-vs-S3 protocol parity suites.

---

# Tier P0 — Ship-blocking (1–13)

**1. Encrypt bundle payloads client-side before upload.** `P0 · XL · critical`
No `encrypt`/`cipher`/`aead` symbol exists anywhere in `src-tauri/src`, and no
`server_side_encryption` header is set on any S3 PUT. Full agent transcripts —
which contain source, credentials pasted into prompts, and raw tool output —
land in R2 as readable JSON. Adopt an age- or XChaCha20-Poly1305-based envelope:
per-bundle content key, passphrase-derived (Argon2id) or keychain-held master
key, encrypt payload objects and the manifest body, leave only routing metadata
(`_head.json`, object keys) in the clear. Keep the SHA-256 chain over
*ciphertext* so CAS and integrity logic survive unchanged.

**2. Run CI on every push and pull request.** `P0 · S · critical`
`.github/workflows/release.yml` is the only workflow, and it triggers on
`push: tags: v*`. A `ci.yml` on `push`/`pull_request` running `npm run build`,
`npm run test:frontend-integration`, and `cargo test --lib` is a few hours of
work and immediately protects 282 tests that currently gate nothing.

**3. Move storage credentials into the OS keychain.** `P0 · M · critical`
[domain.rs:570-572](../src-tauri/src/project_sync/domain.rs:570) stores
`access_key_id` and `secret_access_key` as plain `String` inside
`~/.mallard/sync_config.json`. Unix mode 0o600 is the only protection; any
process running as the user, any backup tool, and any `~/.mallard` support
bundle exposes them. Use `keyring-rs` (Keychain / Credential Manager / Secret
Service) and keep only an opaque handle in JSON. Add a migration that moves and
then zeroes existing values.

**4. Probe conditional-write support instead of assuming it.**
`P0 · S · critical`
[s3_store.rs:58](../src-tauri/src/project_sync/s3_store.rs:58) rejects a storage
only when `supports_conditional_writes == Some(false)`. But that field is
`#[serde(default)]` (→ `None`), the storage-settings UI never sets it
(`StorageSettings.tsx` contains zero references), and **all 12 assignments in
`commands.rs` are inside the test module** (lines 12016–15823, past the test
boundary at 5042). So in production the value is *always* `None` and the guard
can never fire. An S3-compatible endpoint that silently ignores `If-None-Match`
/`If-Match` — historically true of some MinIO, Ceph RGW, and B2-S3 deployments
— degrades head CAS to last-writer-wins **with no detection and no warning**,
directly violating the single-winner publication guarantee that
`TECHNICAL_REPORT.md` §11 states as a core property. Fix: on storage save, write
a scratch object twice with `If-None-Match: *`, assert the second attempt
returns 412/409, persist the result, and fail closed when the probe fails or is
inconclusive.

**5. Set a production Content Security Policy.** `P0 · S · high`
`src-tauri/tauri.conf.json` has `"csp": null`. Tauri ships that default for dev
convenience; production should pin
`default-src 'self' tauri:; script-src 'self' tauri:; style-src 'self'
'unsafe-inline'; img-src 'self' asset: data:; connect-src 'self' ipc:`. Mallard
renders untrusted third-party conversation text; a CSP is the cheap backstop.

**6. Run the Rust suite in the release workflow too.** `P0 · S · high`
The release job runs `test:frontend-integration` and `test:release-config` but
**not** `test:backend-integration`. A DMG can ship today with a broken bundle
engine. Add the cargo step before `tauri-action`.

**7. Notarize and hard-sign the macOS build.** `P0 · M · high`
`bundle.macOS.signingIdentity` is `"-"` (ad-hoc). README instructs judges to
approve the app manually under Privacy & Security. That is acceptable for a
hackathon and fatal for adoption. Enroll in the Apple Developer Program, add
`APPLE_CERTIFICATE`/`APPLE_ID`/`APPLE_TEAM_ID` secrets, set
`providerShortName`, enable hardened runtime, and notarize in CI.

**8. Ship an auto-updater.** `P0 · M · high`
Neither `tauri-plugin-updater` nor `@tauri-apps/plugin-updater` appears in any
manifest. With no update channel, a security fix for items #1–#5 can never
reach an installed user. Add the updater plugin with signature verification and
a GitHub Releases endpoint; generate and store the update signing keypair now,
before the first real user install.

**9. Lift or stream past the 100 MiB conversation ceiling.** `P0 · L · high`
`MAX_CONVERSATION_FILE_BYTES = 104857600`
([domain.rs:41](../src-tauri/src/project_sync/domain.rs:41)), enforced at
[codex_rollout.rs:83](../src-tauri/src/project_sync/codex_rollout.rs:83) and
five sites in `provider_capture.rs`. Documented real-world Codex rollouts reach
1.95 GB. Today those sessions are silently unsyncable — the exact long-lived
sessions whose reasoning is most worth preserving. Move to chunked, streamed
hashing and multipart upload so the cap becomes a *policy* knob (with a clear UI
warning) rather than a hard parse limit.

**10. Add clippy, rustfmt, and a TS linter as CI gates.** `P0 · S · high`
No `rustfmt.toml`, `clippy.toml`, `eslint.config.js`, or `.prettierrc` exists.
`cargo clippy -- -D warnings` and `cargo fmt --check` on a 54k-line Rust
codebase will find real issues on day one, and `AGENTS.md` already promises
rustfmt conventions that nothing enforces.

**11. Reconcile the website with the repository.** `P0 · S · high`
[mallard-ai.com](https://mallard-ai.com/) advertises **v0.1.0**, **Windows 10+
and Linux x64 builds**, and **Skill and Plugin management**. The repo is
**v0.1.2**, CI builds **only `aarch64-apple-darwin` DMG**, and skill/plugin sync
was **deleted in `50812a0`**. Three false claims on the front page is a trust
problem before it is a marketing problem. Either ship the platforms or correct
the copy this week.

**12. Fix the known-vulnerable transitive dependency.** `P0 · S · medium`
`npm audit` reports one **high** advisory: postcss ≤8.5.17 path traversal via
`sourceMappingURL` (GHSA-r28c-9q8g-f849). `npm audit fix` resolves it. Then keep
it fixed — see #13.

**13. Automate dependency and vulnerability scanning.** `P0 · S · high`
No `dependabot.yml`, no `renovate.json`, no `cargo-audit`/`cargo-deny`. Add
Dependabot for npm + cargo + GitHub Actions, and a scheduled
`cargo audit && npm audit --audit-level=high` job. For a security-positioned
product, an unattended dependency tree is the likeliest source of the first CVE.

---

# Tier P1 — High (14–33)

**14. Split `commands.rs`.** `P1 · L · high`
16,277 lines; production code runs to line 5,042 and 250 function definitions,
with 11k lines of inline tests after. Split by capability —
`commands/setup.rs`, `commands/push.rs`, `commands/pull.rs`,
`commands/history.rs`, `commands/storage.rs`, `commands/divergence.rs` —
keeping the Tauri attribute layer thin and delegating to existing engines.

**15. Replace `Result<T, String>` with a typed error enum.** `P1 · L · high`
Stringly-typed errors cross every Tauri boundary. The UI cannot branch on
failure kind, cannot localize, and cannot distinguish "retry" from "re-review"
from "fatal". Introduce `#[derive(Serialize)] enum SyncError { … }` with a
stable `code`, a human `message`, and an optional `remediation`, and map it in
`api.ts`. This is also the prerequisite for #48.

**16. Generate `src/types.ts` from Rust.** `P1 · M · high`
`AGENTS.md` instructs contributors to hand-maintain 1,059 lines of TypeScript
mirroring serialized Rust DTOs "together". That is a permanent drift hazard on a
72-command surface. Adopt `ts-rs` or `tauri-specta` and make the generated file
CI-checked.

**17. Break up `App.css`.** `P1 · M · medium`
10,707 lines, 1,232 rule blocks, 1,497 custom-property references, 22 media
queries, 55 hard-coded hex colors, and 13 `!important` escapes in one file. Split
into `tokens.css` + per-surface stylesheets (or CSS Modules) and move the 55
literal colors into tokens so light/dark stay consistent.

**18. Decompose `ProjectSync.tsx`.** `P1 · L · high`
2,932 lines with **68 `useState` hooks** in a single component. This is the
project's main maintainability risk on the frontend. Extract a `useReducer`
state machine for the review flow (its states are already well-defined by
`SyncReviewTabs`), and lift per-workspace state into the workspaces that own it.

**19. Add a React error boundary.** `P1 · S · medium`
Zero `ErrorBoundary`/`componentDidCatch` in `src/`. One render throw in a
2,900-line component blanks the entire desktop app with no recovery path and no
log entry. Wrap the workspace, report to the activity log, offer "reload
workspace".

**20. Compress payloads before upload.** `P1 · M · high`
No gzip/zstd anywhere in the transport path. JSONL transcripts compress 5–15×.
Combined with #1, the order is compress → encrypt → upload (never the reverse).
Directly cuts R2 egress cost, push latency, and the practical impact of #9.

**21. Publish transcript deltas, not whole files.** `P1 · L · high`
Object-level dedup already exists — an *unchanged* transcript is correctly
skipped. But a transcript that *grew by one turn* is a new content hash and
re-uploads in full. Rollout JSONL is append-only in the common case, and
`DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md` already proves prefix relationships via
canonical chain digests (`CloudPrefixOfLocal` / `LocalPrefixOfCloud`). Publish
an append-delta object keyed by the prior canonical digest and reserve full
snapshots for divergence. A 2 GB session that grew by 40 KB should cost 40 KB.

**22. Ship a `mallard` CLI.** `P1 · L · high`
Every direct competitor is a CLI first
([claude-sync](https://github.com/tawanorg/claude-sync),
[aisync](https://github.com/dantelex/aisync),
[session-sync](https://github.com/npow/session-sync)). A CLI unlocks CI usage,
headless dev boxes, remote/SSH work, and agent lifecycle hooks — and it costs
little, because the entire engine already lives in a Rust `rlib` behind a thin
Tauri layer.

**23. Add cross-platform CI builds.** `P1 · M · high`
Build and test on `ubuntu-latest` and `windows-latest` in addition to macOS.
28 `cfg(windows)`/`cfg(target_os)` sites exist in the Rust tree and **none are
exercised by any automation today**. Also add `x86_64-apple-darwin` or a
universal binary — Intel Macs are still a meaningful share of dev machines.

**24. Add a `SECURITY.md` and coordinated disclosure path.** `P1 · S · high`
A tool that holds cloud credentials, rewrites files in a user's project, and
publishes conversation history needs a stated reporting channel, a supported-
version table, and a response SLA before it has users, not after.

**25. Write and publish a threat model.** `P1 · M · high`
`AGENTS.md` lists excellent safety *invariants* but never states the *adversary*.
Enumerate: malicious bundle in a shared bucket, compromised R2 credentials,
hostile project content during Pull, a co-tenant on a shared machine, a
malicious `.mallardignore`, **a storage endpoint that lies about conditional
writes** (#4). Map each to the control that stops it. This doubles as the best
possible sales document for a security-conscious team.

**26. Capture commit↔session links deterministically via agent hooks.**
`P1 · M · high`
Today mapping is heuristic: a commit "during the session or the nearest
follow-up within 24 hours" (README). That is a good fallback and a poor primary.
Codex and Claude Code both expose lifecycle hooks; record `HEAD` SHA at turn
boundaries into a sidecar and treat the 24h window as the degraded path. This is
the feature the product is *named* for — it deserves ground truth.

**27. Interoperate with the Agent Trace specification.** `P1 · L · high`
[Agent Trace](https://agent-trace.dev/) (Cursor, CC BY 4.0) is the emerging
vendor-neutral standard for AI code attribution, already adopted by Cline,
OpenCode, and [git-ai](https://rywalker.com/research/git-ai), and on the
Thoughtworks Technology Radar. Emitting and consuming Agent Trace records turns
Mallard from an isolated app into a participant in a standard — and the spec
explicitly leaves *storage* unspecified, which is precisely the slot Mallard
fills.

**28. Add full-text search across synced sessions.** `P1 · L · high`
`rusqlite` is already a dependency. An FTS5 index over transcripts — scoped by
project, author, date, tool, and commit — is the highest-value feature not yet
built. "Which session decided we'd use CAS on the head object?" is the actual
daily question, and no competitor answers it well.

**29. Publish a doc index and archive superseded plans.** `P1 · S · medium`
`docs/` holds 60+ files, 45 of them `PLAN_*`, with no map. `AGENTS.md` says they
are "design history unless a task explicitly names them" — encode that: add
`docs/README.md` with a status table, and move superseded plans to
`docs/archive/`. Right now a newcomer cannot tell `PLAN_MULTI_STORAGE.md`
(historical) from `DESIGN_FAST_PUSH_PULL.md` (proposed, active).

**30. Correct the stale numbers in `TECHNICAL_REPORT.md`.** `P1 · S · medium`
§13 claims "290 Rust library tests" and `docs/TODO.md` claims 293; the suite
today reports **282**. Small drift, but this document is the project's
credibility artifact. Better: have CI regenerate the counts rather than
hand-maintaining them.

**31. Add a `CHANGELOG.md` and adopt a release convention.** `P1 · S · medium`
The release body is a hard-coded string in `release.yml` that says the same
thing for every version. Users cannot tell what changed, and the "clean cutover,
clear `~/.mallard`" policy makes *stating* breaking changes essential.

**32. Add `CONTRIBUTING.md`, PR/issue templates, and `CODEOWNERS`.**
`P1 · S · medium`
All absent. `AGENTS.md` is a superb contributor guide aimed at agents; humans
arriving from the Devpost link get nothing. Split the human-facing parts out and
link the two.

**33. Pin a Rust MSRV and toolchain.** `P1 · S · medium`
`Cargo.toml` declares `edition = "2021"` but no `rust-version`, and there is no
`rust-toolchain.toml`. CI uses floating `dtolnay/rust-toolchain@stable`, so a
future compiler release can break the build with no code change.

---

# Tier P2 — Medium-high (34–59)

**34. Remove unused dependencies.** `P2 · S · low`
`react-markdown` and `remark-gfm` are declared in `package.json` and imported
**nowhere** in `src/` or `tests/`. Drop them — or wire them up for the
conversation preview, which is presumably why they were added (see #50).

**35. Move `tempfile` to `[dev-dependencies]` if only tests use it.**
`P2 · S · low`
It sits in the production dependency list; verify whether the shipped binary
needs it, and shrink the runtime tree if not.

**36. Add code coverage reporting.** `P2 · M · medium`
`cargo-llvm-cov` plus a coverage summary on PRs. With a test suite this good,
the interesting question is which of the 5,042 production lines in `commands.rs`
are *not* covered.

**37. Split the 11k lines of inline tests out of `commands.rs`.**
`P2 · M · medium`
`sync_tests/` already exists as the home for cross-module flows. Inline unit
tests should stay beside the helper they protect; an 11,000-line trailing
`mod tests` is neither.

**38. Add a golden-fixture corpus for provider formats.** `P2 · M · high`
Claude Code's transcript entry format "is internal and changes between
versions"; Codex added `archived_sessions` in v0.136 mid-2026. Check in a
versioned fixture corpus (`tests/fixtures/codex/v0.1xx/…`) and assert graceful
degradation — unknown record kinds must be preserved verbatim and never drop a
thread.

**39. Detect and surface the provider version.** `P2 · S · medium`
Record the Codex/Claude version that produced each rollout in the manifest, and
warn on Pull when the local provider is older than the publisher's. Silent
format skew is the most likely future data-loss vector.

**40. Generalize the storage layer beyond Cloudflare R2.** `P2 · M · medium`
`AGENTS.md` and `TECHNICAL_REPORT.md` describe "S3-compatible" storage, but the
implementation is R2-shaped: `StorageSettings.tsx` is built around
`r2S3ApiUrl`/`parseR2S3ApiUrl` and hardcodes `region: "auto"`, and production
error strings read "R2 Access Key ID is not configured" / "R2 client creation
panicked" ([s3_store.rs:569, :617](../src-tauri/src/project_sync/s3_store.rs:569))
even for a MinIO or AWS endpoint. Add a generic S3 form (endpoint, region,
bucket, path-style toggle), keep the R2 quick-path as a preset, and neutralize
the messages. Pairs naturally with #4 — both are about honestly supporting
non-R2 backends.

**41. Add opt-in background sync.** `P2 · L · high`
`TECHNICAL_REPORT` §14: "Scans are explicit; no background watcher." That is the
right *default* and the wrong *only option*. An opt-in "auto-push on session
end" (via provider hooks, not a filesystem watcher) is what makes cross-machine
continuity actually seamless — the thing every competitor sells.

**42. Virtualize long lists.** `P2 · M · medium`
The 500-row render cap is a workaround for missing virtualization. `react-window`
or `virtua` removes the cap and improves scroll performance on
20,000-resource projects.

**43. Add a command palette and keyboard shortcuts.** `P2 · M · medium`
A review-first tool used repeatedly per day should be drivable from the
keyboard: ⌘K palette, ⌘⏎ to approve, ⌘R to refresh, j/k to move through review
rows.

**44. Follow the system theme by default.** `P2 · S · low`
`theme.ts` hard-defaults to dark and never consults
`window.matchMedia('(prefers-color-scheme)')`; `App.css` has no
`prefers-color-scheme` block, only `:root[data-theme="light"]`. Add a "System"
option and make it the default.

**45. Run an accessibility audit.** `P2 · M · medium`
311 `aria-*`/`role=` occurrences across 157 buttons suggests real effort, but
nothing verifies it. Add `axe-core` assertions to the frontend integration
suite, and check focus order, focus-visible rings, and contrast on the review
tables specifically.

**46. Add progress and cancellation to long operations.** `P2 · M · medium`
`BundleFetchProgress` exists; extend the same treatment to capture, hashing,
upload, and apply, with a real cancel that leaves no partial local state. On a
512 MiB aggregate capture this is the difference between "working" and "hung".

**47. Make transfer timeouts size-aware, and add resumable multipart.**
`P2 · L · high`
`R2_REQUEST_TIMEOUT` is a flat 120 s
([s3_store.rs:34](../src-tauri/src/project_sync/s3_store.rs:34)) wrapping the
whole `put_object` call *including* the SDK's internal retries, while
`MAX_FILE_BYTES` allows a 512 MiB object
([domain.rs:40](../src-tauri/src/project_sync/domain.rs:40)). Uploading at the
ceiling inside the budget demands a sustained ~4.4 MB/s (~35 Mbit/s); below
that, a large push **cannot succeed at any retry count**, and the user sees
"request timed out after 120000 ms" rather than "your connection is too slow for
a 512 MiB object". Scale the timeout by object size (or switch to a
per-byte-progress stall timeout), and add multipart upload with per-part retry
so a drop at 80% resumes instead of restarting.

**48. Add offline and degraded-network handling.** `P2 · M · medium`
Distinguish "no network", "credentials rejected", "bucket missing", and "CAS
conflict" in the UI with distinct remediation. Today they all surface as generic
strings (see #15).

**49. Add a "what changed" diff view between generations.** `P2 · M · high`
The commit chain already links generations and records file delta counts. A
generation-to-generation diff — threads added, messages appended, files changed
— makes bundle history *browsable* rather than merely *valid*.

**50. Export a session to Markdown/HTML/PDF.** `P2 · M · medium`
The most requested thing anyone does with a transcript is share it with someone
who does not have the tool. A one-click export (with a redaction pass) is a
strong, cheap adoption lever — and `react-markdown` is already installed (#34).

**51. Add redaction review before push.** `P2 · L · high`
Secret detection is documented as best-effort and applies to project files; a
transcript with a `cat .env` in tool output is the more common leak. Add an
inline, reviewable redaction pass over transcript content — entropy + known
credential patterns — with the redaction recorded in the manifest so Pull cannot
silently un-redact.

**52. Support more providers.** `P2 · L · high`
[aisync](https://github.com/dantelex/aisync) covers Claude Code, Codex CLI,
OpenCode, and Cursor; Mori adds Antigravity. Mallard's Claude support is partial
(`claude_home` is plumbed through binding, capture, and restore, but Codex has
the deep fork/divergence machinery). Finish Claude to parity, then add Gemini
CLI and OpenCode behind the existing `Provider` enum.

**53. Support folder-based storage backends beyond S3.** `P2 · M · medium`
Local-folder storage exists, but users on iCloud/Dropbox/Drive are explicitly
warned they lack a distributed lock (§11). Add WebDAV and explicit
Nextcloud/ownCloud support (as claude-sync does) so self-hosters have a
first-class path that *does* lock properly.

**54. Add per-project retention and pruning of old generations.**
`P2 · M · medium`
The commit chain is unbounded and every push writes a full manifest. Add a
retention policy (keep N generations, keep all tagged) with garbage collection
of unreferenced `_uploads/` objects.

**55. Reclaim orphaned upload objects after a failed CAS.** `P2 · M · medium`
§8 states plainly: "A failed CAS can leave unreachable upload/manifest/commit
objects." Correct, but it silently grows a user's storage bill forever. Add a
sweep that deletes upload objects unreachable from any retained commit.

**56. Add structured, queryable local telemetry (opt-in, local-only).**
`P2 · M · medium`
`activity_log.rs` is already sanitized JSONL with retention — a strong
foundation. Add operation timing and failure taxonomy so "why is push slow"
becomes answerable from the log alone, and expose it in `LogManagerDialog`.

**57. Add a support-bundle export.** `P2 · S · medium`
One button producing a redacted diagnostic zip (versions, sanitized activity
log, manifest heads, no credentials or paths) makes user bug reports actionable.
This must be built *after* #3, or it will exfiltrate the R2 secret key.

**58. Shrink the repository's image payload.** `P2 · S · low`
`docs/images/linkage-concepts/` holds five ~1.1 MB PNGs plus a 1.0 MB matrix
image — roughly 6 MB of design exploration in a 219-file repo. Convert to WebP
or move to a release asset; clone time is a first-impression cost.

**59. Add `.editorconfig`.** `P2 · S · low`
`AGENTS.md` specifies two-space indent, double quotes, semicolons. Encode it so
editors enforce it without anyone reading the doc.

---

# Tier P3 — Medium / lower (60–81)

**60. Add a rendered architecture diagram.** `P3 · S · medium`
The ASCII diagram in `TECHNICAL_REPORT.md` §2 is good but caps out. A Mermaid
sequence diagram of the Push CAS protocol and the Pull plan/apply split would
carry the design far better — and Mermaid renders natively on GitHub.

**61. Add integration tests against real S3-compatible storage.**
`P3 · M · medium`
§13 concedes a live R2 smoke test "remains an external release check". Run
MinIO in a CI service container and close that gap — which also gives #4 a real
non-R2 endpoint to probe against.

**62. Add property-based testing for path validation.** `P3 · M · medium`
Logical-path validation is a security boundary with a rich grammar (relative,
normalized, no dot/parent/backslash, case-collision detection). `proptest` will
explore it far more thoroughly than enumerated cases.

**63. Fuzz the JSONL parsers.** `P3 · M · medium`
`chat_history.rs`, `codex_rollout.rs`, and `conversation_codec.rs` parse
attacker-influenceable content from a shared bucket. `cargo-fuzz` on those three
entry points is high value per hour.

**64. Add a benchmark suite.** `P3 · M · medium`
`DESIGN_FAST_PUSH_PULL.md` promises cost proportional to what changed. Add
`criterion` benchmarks over synthetic 10/100/1,000-thread projects so the claim
is measured, not asserted, and regressions are caught.

**65. Add snapshot tests for the review UI.** `P3 · M · low`
The Push/Pull review screens are where a user's trust is won or lost. Snapshot
the rendered decision tables so wording and ordering changes are deliberate.

**66. Add a first-run onboarding flow.** `P3 · M · high`
Current setup requires pasting an S3 API URL, an access key, and a secret key
before anything works. Add a guided wizard, a "test connection" step with
specific diagnostics (fold #4's probe into it), and a zero-config local-folder
default so a new user reaches a working state without cloud credentials at all.

**67. Add device pairing for credential transfer.** `P3 · L · medium`
Retyping R2 credentials on each machine is the worst moment in the product. A
QR/code-based pairing handshake between two running instances (over the bucket
itself, encrypted with a short-lived code) removes it.

**68. Add an "explain this state" affordance.** `P3 · S · medium`
The comparison model has nine states (`synced`, `local_ahead`, `diverged`,
`blocked`, `unknown`, …). Each deserves a one-sentence hover explanation and a
"what should I do" line. This is where a review-first product earns its
positioning.

**69. Show storage cost and size estimates.** `P3 · S · medium`
Before a push: bytes to upload, resulting bundle size, cumulative storage. Users
paying for R2 will want this, and it makes #54 self-motivating.

**70. Add undo for an applied Pull.** `P3 · M · high`
Backups and receipts already exist per restore plan — the data for undo is
already on disk. Surfacing a one-click "revert this pull" turns a careful
feature into a confident one.

**71. Add dry-run mode.** `P3 · S · medium`
"Show me exactly what a push would do, change nothing." The plan/apply split
already supports it structurally; it just needs a UI entry point.

**72. Add a global cross-project session browser.** `P3 · M · medium`
Today history is per-project. "Every session across all my projects, newest
first" is a browsing pattern users already expect from CLI tools.

**73. Add session tagging and annotation.** `P3 · M · medium`
Let a user mark a session "this is the one that explains the auth rewrite". The
manifest already carries bounded per-resource metadata; tags are a natural fit
and dramatically improve #28's results.

**74. Add git notes output for commit↔session links.** `P3 · M · medium`
Writing the mapping into `refs/notes/` (as git-ai does) makes it travel with the
repository, visible in `git log`, and useful to teammates who do not run
Mallard. Pure upside for the project's core idea.

**75. Add a read-only web viewer for a published bundle.** `P3 · XL · medium`
A static site that renders a bundle from a signed URL removes "install the
desktop app" from the reviewer's path — the single biggest funnel drop for a
"GitHub for agent work" product. (Note the tension with #1: this needs a
share-specific key, not the bundle master key.)

**76. Define a team/multi-user model.** `P3 · XL · high`
Manifests carry origin tokens and host aliases, but there is no notion of
membership, roles, or per-user permissions on a shared bucket. Before any team
sells this internally, they need answers on access control and audit.

**77. Add a shared-bucket audit trail.** `P3 · M · medium`
Who published which generation, when, from which host. Origin tokens are already
in the manifest; surface them as a readable, filterable history.

**78. Add conflict-resolution presets.** `P3 · M · medium`
The divergence machinery is sophisticated. Expose two or three named policies
("always keep local", "always take cloud", "always ask") so routine cases stop
requiring a full review.

**79. Add per-thread selective sync.** `P3 · M · medium`
Users will want "sync this project's sessions, but not the three where I pasted
production data". Exclusion sets exist for project content; extend the same
model to threads.

**80. Add import from a competitor's format.** `P3 · M · low`
A `mallard import` for claude-sync or aisync layouts lowers switching cost from
the exact tools users are most likely to already have installed.

**81. Publish the bundle format as a spec.** `P3 · M · medium`
`TECHNICAL_REPORT.md` §5 is already 90% of a specification. Extracting it as a
standalone, versioned `SPEC.md` with a JSON Schema invites third-party readers
and makes the format defensible as a standard rather than an implementation
detail.

---

# Tier P4 — Speculative / long-horizon (82–100)

**82. Publish a Rust crate for the bundle engine.** `P4 · L · medium`
The engine is provider- and Tauri-agnostic by design (`AGENTS.md` enforces the
boundary). Publishing it lets others build on the format — and #22's CLI becomes
a thin consumer.

**83. Provide an MCP server over synced sessions.** `P4 · L · high`
Expose "search my team's past agent sessions" as MCP tools so agents can query
the archive during a task. This is the highest-leverage idea in this document:
it turns a sync tool into agent *memory infrastructure*, competing directly with
[Mori](https://github.com/fjwood69/mori) but on top of a rigorous, verifiable
storage layer they lack.

**84. Add semantic search / embeddings over transcripts.** `P4 · L · medium`
Layered on #28 and #83. Local embeddings keep the privacy story intact.

**85. Auto-summarize sessions into durable project knowledge.**
`P4 · L · medium`
Distill each synced session into a short decision record. This is Mori's "dream
pipeline"; Mallard's advantage is that its source of truth is content-addressed
and reviewable.

**86. Generate a `CHANGELOG`-style narrative from session history.**
`P4 · M · low`
"What did this project's agents actually do last sprint" is a compelling weekly
artifact for engineering managers.

**87. Detect and surface duplicated agent effort.** `P4 · L · medium`
When two teammates' sessions converge on the same file and question, say so.
This is a genuinely novel feature that only a cross-machine session store can
offer.

**88. Add cost and token analytics across sessions.** `P4 · M · medium`
Metrics are already parsed (tokens, turns, tool calls). Aggregate them per
project, per person, per week.

**89. Integrate with GitHub PRs.** `P4 · L · high`
Post the mapped session summary as a PR comment: "this diff came from these
three sessions." That is the product thesis expressed where reviewers already
are. Note that [GitHub's own Copilot app](https://www.helpnetsecurity.com/2026/06/08/github-copilot-app-ai-coding-agents/)
now occupies the "desktop home for agents" position, so integrating with GitHub
is likelier to succeed than competing with it head-on.

**90. Add a VS Code / JetBrains extension.** `P4 · XL · medium`
"Show me the session that wrote this line" as an editor gutter action.

**91. Support Jujutsu and Mercurial.** `P4 · L · low`
Agent Trace (#27) already spans Git, Jujutsu, and Mercurial. Non-Git project
sync is currently *blocked* rather than *supported*, so jj users are a natural
underserved audience.

**92. Add a compliance/attribution export.** `P4 · M · medium`
Structured AI-attribution requirements are arriving for regulated teams in 2026.
An auditable export of "which code came from which agent session, verified by
digest" is a credible enterprise wedge — and Mallard's SHA-256-everything design
is exactly what makes such a claim defensible.

**93. Add end-to-end encrypted sharing with a specific teammate.**
`P4 · L · medium`
Builds on #1 and #76: per-recipient key wrapping so a bundle can be shared with
one colleague rather than everyone holding the bucket credentials.

**94. Offer hosted storage.** `P4 · XL · high`
`docs/CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` already contemplates this.
Removing "go create an R2 bucket" is the single largest funnel improvement
available — and with #1 shipped, hosting it does not require users to trust the
host with plaintext.

**95. Explore a self-hosted server mode.** `P4 · XL · medium`
Some teams will not use object storage directly. A small server offering auth,
ACLs, and audit over the same bundle format serves them — and sidesteps #4
entirely by controlling both ends of the CAS contract.

**96. Add real-time presence.** `P4 · L · low`
"Alice is pushing to this project right now" prevents the CAS conflicts §11
warns about on shared local-folder storage.

**97. Add a plugin/extension API.** `P4 · XL · low`
Let others add provider adapters without forking. Note the deliberate removal of
skill/plugin *sync* in `50812a0` — this is a different axis and should not
resurrect that scope.

**98. Build a public gallery of exemplary agent sessions.** `P4 · L · medium`
Opt-in, redacted, published — a community-facing artifact that also serves as
the product's best marketing.

**99. Publish benchmark comparisons against competitors.** `P4 · M · medium`
Sync time, storage footprint, and correctness under concurrent push, measured
against claude-sync and aisync. Mallard's CAS-and-digest design should win the
correctness column decisively — *once #4 guarantees the CAS is real* — so say so
with numbers.

**100. Write the "why review-first" manifesto.** `P4 · S · medium`
Every competitor syncs automatically. Mallard deliberately does not, and the
reasoning in `AGENTS.md` and `TECHNICAL_REPORT.md` §14 is genuinely
well-argued — but it is buried in contributor documentation. A public essay on
why agent transcripts deserve review before they cross a machine boundary is
both the product's differentiation and its most defensible position.

---

## Competitive landscape

Researched July 31, 2026. Mallard's differentiation is real: **no competitor has
immutable, content-addressed, CAS-published generations with a reviewed apply
plan.** Most are "rsync with encryption". The gap runs the other way on
encryption, distribution, and automation.

| Project | Form | Overlap | What they do better | What Mallard does better |
| --- | --- | --- | --- | --- |
| [claude-sync](https://github.com/tawanorg/claude-sync) (237★, Go, MIT) | CLI | Very high | **age E2E encryption**, Argon2 key derivation, 6+ storage backends incl. WebDAV, gzip, npm distribution | Review-before-apply, immutable generations, integrity verification, commit mapping, GUI |
| [aisync](https://github.com/dantelex/aisync) | CLI | High | 4 agents (Claude/Codex/OpenCode/Cursor), works over iCloud/Dropbox/Drive | Correctness under concurrency, explicit conflict semantics, safety invariants |
| [session-sync](https://github.com/npow/session-sync) | Hooks + CLI | Medium | Real-time cross-*format* translation (Claude ⇄ Codex ⇄ Amp ⇄ Gemini) via lifecycle hooks | Durability, portability across machines, verified restore |
| [Mori](https://github.com/fjwood69/mori) | MCP server | Medium | Agents *consume* the shared memory at session start (`/brief`); zero-instrumentation capture | Verifiable storage, exact transcript fidelity, no lossy distillation |
| [git-ai](https://rywalker.com/research/git-ai) | Git integration | Medium | Line-level provenance in git notes, survives rebase/squash; 10+ agents; implements Agent Trace | Full conversation preservation, not just attribution |
| [Agent Trace](https://agent-trace.dev/) (Cursor, CC BY 4.0) | **Specification** | Standard | Vendor-neutral attribution format; Cline + OpenCode + git-ai adoption; on the Thoughtworks Radar | — (Mallard should implement it, see #27) |
| [GitHub Copilot app](https://www.helpnetsecurity.com/2026/06/08/github-copilot-app-ai-coding-agents/) | Desktop app | Adjacent | Platform distribution, "My Work" across repos, per-agent worktree isolation | Provider-neutral, local-first, no vendor lock-in |
| Langfuse / MLflow / Phoenix | Observability | Low | Production tracing, evals, cost tracking | Different problem — Mallard is portability and review, not monitoring |

**Ecosystem facts that should shape the roadmap:**

- Codex rollouts reach **700 MB–1.95 GB** with 91 GB session directories
  ([openai/codex#24948](https://github.com/openai/codex/issues/24948)) — drives
  #9, #20, #21, #47.
- Codex v0.136 (June 2026) added `archived_sessions/` — **already handled**.
- Claude Code **deletes transcripts older than 30 days by default**, and its
  entry format is internal and changes between versions — drives #38, #39, and
  a genuine marketing angle: Mallard is the archive that survives the sweep.
- Structured AI attribution is moving from optional to expected for regulated
  teams during 2026 — drives #27, #92.

---

## Measured baseline

Recorded 2026-07-31 so future reviews can measure movement.

| Metric | Value |
| --- | ---: |
| Rust lines (incl. tests) | 53,922 |
| Rust lines in `sync_tests/` | 6,541 |
| Largest Rust file (`commands.rs`) | 16,277 (prod through ~5,042) |
| Registered Tauri commands | 72 |
| Production `.unwrap()`/`.expect()` | ~5 |
| `unsafe` occurrences | 25 |
| `TODO`/`FIXME` markers | 2 |
| Rust tests passing | 282 |
| Frontend integration test files | 10 |
| TypeScript/TSX lines | ~19,800 |
| Largest component (`ProjectSync.tsx`) | 2,932 lines / 68 `useState` |
| `App.css` | 10,707 lines / 1,232 rules |
| `as any` casts | 0 |
| `console.*` calls | 0 |
| React error boundaries | 0 |
| S3 request timeout | 120 s flat |
| Max object / conversation bytes | 512 MiB / 100 MiB |
| Tracked files | 219 |
| Commits / contributors | 90 / 4 |
| CI workflows | 1 (tag-triggered only) |
| Known npm advisories | 1 high (postcss) |

---

## Suggested sequencing

- **Week 1 — trust floor.** #2, #4, #6, #10, #12, #13, #5, #30, #11. Cheap,
  mostly configuration, and each one removes a way the project can quietly break
  or mislead. **#4 belongs here despite its P0 rank** — it is a one-day fix for
  a broken guarantee.
- **Weeks 2–4 — confidentiality.** #1, #3, #24, #25. This is the work that makes
  Mallard safe to recommend to a team, and it is the competitive gap that
  matters most.
- **Weeks 4–8 — distribution.** #7, #8, #23, #31. Turn "a DMG judges can approve
  manually" into "software people install."
- **Weeks 6–12 — scale and structure.** #9, #20, #21, #47 alongside
  `DESIGN_FAST_PUSH_PULL.md`, plus #14, #15, #16, #18 as the refactor those
  changes will need anyway.
- **Quarter 2 — differentiation.** #22, #26, #27, #28, #83. Search over verified
  session history, exposed to agents over MCP, is where this project becomes
  something no competitor has.

---

## Sources

- [claude-sync](https://github.com/tawanorg/claude-sync) ·
  [claude-sync.com](https://claude-sync.com/)
- [aisync](https://github.com/dantelex/aisync)
- [session-sync](https://github.com/npow/session-sync)
- [Mori](https://github.com/fjwood69/mori)
- [git-ai research note](https://rywalker.com/research/git-ai)
- [Agent Trace specification](https://agent-trace.dev/) ·
  [cursor/agent-trace](https://github.com/cursor/agent-trace) ·
  [InfoQ coverage](https://www.infoq.com/news/2026/02/agent-trace-cursor/) ·
  [Thoughtworks Technology Radar](https://www.thoughtworks.com/en-us/radar/platforms/agent-trace)
- [openai/codex#24948 — session log size](https://github.com/openai/codex/issues/24948)
- [Codex CLI session archiving in v0.136](https://codex.danielvaughan.com/2026/06/02/codex-cli-session-archiving-lifecycle-management-v0136/)
- [Claude Code — Manage sessions](https://code.claude.com/docs/en/sessions)
- [Tauri — Content Security Policy](https://v2.tauri.app/security/csp/) ·
  [Tauri Ecosystem Security](https://v2.tauri.app/security/ecosystem/)
- [GitHub Copilot desktop app](https://www.helpnetsecurity.com/2026/06/08/github-copilot-app-ai-coding-agents/)
- [Best AI agent observability tools 2026](https://www.augmentcode.com/tools/best-ai-agent-observability-tools)
- [mallard-ai.com](https://mallard-ai.com/)

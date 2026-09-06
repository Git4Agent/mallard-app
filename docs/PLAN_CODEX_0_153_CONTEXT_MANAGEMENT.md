# Codex CLI 0.153.0 impact and plan

Status: Proposed. Assessment date: 2026-09-06.

Recommendation: Treat experimental context management as an unverified session
portability capability. Prove native resume after transfer before claiming
support. The likely work is in the Codex adapter and tests. The current evidence
does not justify a storage or application architecture change.

This assessment uses official documentation, the tagged GitHub source, and the
current working tree. GitHub source verification on 2026-09-06 resolved
`rust-v0.153.0` to commit `41e22fee981a63b3698df7ed36bad393cda24715`.
The installed CLI reports `codex-cli 0.152.0`. No 0.153.0 runtime test was run.
The client uses backend history and notes services. Transfer of that backend
state to a reconstructed Mallard session remains unverified.

**Verified upstream behavior**

The configuration is TOML:

```toml
[features.context_management]
experimental_mode = true
```

The feature is off by default. It uses notes and searchable history. It requires
ChatGPT sign-in on an eligible Plus, Pro, or Pro Lite plan. See the
[configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

The 0.153.0 release adds activation for token-budget context, history notes, and
`new_context`. API-key sessions, custom providers, and temporary structured
threads are excluded. The release also changes shared-history compression and
adds nullable model settings to app-server thread metadata. These are separate
changes; the release notes do not say that context management enables rollout
compression. See the [release notes](https://learn.chatgpt.com/docs/changelog).

The same release page lists 0.153.4. This plan assesses the requested 0.153.0
baseline. Later patch certification requires its own recorded version.

**Verified GitHub storage behavior**

History notes use virtual paths. The tagged tool implementation explicitly says
that these are not filesystem paths. It maps note reads and writes to
`alpha/notes/v2/*` routes and history reads to `alpha/history/v2/*` routes.
See [history and notes tools](https://github.com/openai/codex/blob/rust-v0.153.0/codex-rs/ext/history-notes/src/tools.rs#L24-L100).

The client sends authenticated POST requests with `session_id` and
`current_agent_name`. The implementation sets a 35-second request timeout.
This value is a timeout, not a measured latency. See
[backend implementation](https://github.com/openai/codex/blob/rust-v0.153.0/codex-rs/ext/history-notes/src/backend.rs#L14-L92).

The extension also gets a bounded note hint from the backend when its thread
context contribution runs. See
[context contribution](https://github.com/openai/codex/blob/rust-v0.153.0/codex-rs/ext/history-notes/src/extension.rs#L92-L132).

There is no local note-sidecar implementation in this extension. Adding file
capture support cannot by itself transfer its backend state. The earlier
sidecar proposal was conditional and is removed from the planned implementation.

Codex does use a local `session_index.jsonl` for thread names. Mallard already
handles selected index rows. Codex also uses the word sidecar for a positional
`replacement_history_metadata` array inside a rollout record. That array is not
a separate file. See the
[session index](https://github.com/openai/codex/blob/rust-v0.153.0/codex-rs/rollout/src/session_index.rs#L21-L68)
and [rollout metadata handling](https://github.com/openai/codex/blob/rust-v0.153.0/codex-rs/rollout/src/recorder.rs#L1173-L1193).

**Impact on the current code**

| Area | Impact | Evidence and implication |
|---|---|---|
| Default sessions | Low to medium regression risk | Existing tests target 0.152.x. A passing prior test does not certify 0.153.0. |
| Experimental context state | High correctness risk; backend transfer unverified | Native history and notes calls address backend state by session and agent identity. A local file copy does not establish equivalent state for the restored identity. |
| Reconstruction and forks | High correctness risk | The builder flattens lineage, clears `history_base`, changes thread identity, and assigns new ordinals. Any context reference to those coordinates needs verified handling. |
| Record projection | Medium risk | The projection has explicit event and item allowlists. Unknown events and item kinds fail. Unknown outer record types pass through. Byte preservation alone does not prove valid references. |
| Compressed histories | Confirmed existing gap | Discovery accepts only a final `.jsonl` extension. A compressed leaf is skipped. A compressed ancestor can leave a selected thread without its required base. |
| Test coverage | High confidence gap | Both current live Codex tests select custom providers. The paid test also uses an API key. Neither proves experimental activation. |
| Activity and metadata | Low to medium risk | Verify new context events, asynchronous input items, and nullable model fields. Add handling only for observed changes. |
| Storage transport | Low expected impact | Local-folder and S3/R2 storage can continue to transport verified immutable payloads. New state must use the same review and integrity rules. |
| Provider settings | Local prerequisite | Verify the destination provider home and actual resumed mode. Do not assume that copying conversation data activates the feature. |

Relevant code locations:

- `crates/mallard-infrastructure/src/providers/codex/discovery.rs:45`:
  plain JSONL filter.
- `crates/mallard-infrastructure/src/providers/codex/materialization.rs:11`:
  sidecar rejection.
- `crates/mallard-infrastructure/src/codex_paginated_projection.rs:28`:
  item allowlist; event rejection is at line 555.
- `crates/mallard-infrastructure/src/codex_standalone_builder.rs:485`:
  current item identity rewrite; metadata and ordinal rewrites start at line 635.
- `src-tauri/tests/live_codex_aimock_e2e.rs:128` and
  `src-tauri/tests/live_codex_e2e.rs:110`: current version gates.
- `src-tauri/tests/live_codex_aimock_e2e.rs:267` and
  `src-tauri/tests/live_codex_e2e.rs:253`: custom-provider selection.

The older 0.152 audit already records compression and imported execution-policy
gaps. Those are existing issues, not newly proved 0.153.0 regressions.

**Ordered implementation plan**

1. **Establish the exact contract: 1–2 engineer-days.**
   Pin CLI 0.153.0 in a disposable test environment. Audit its exact source tag
   and persisted protocol. Compare new sessions with the flag off and on.
   Also check resume of an existing session; do not assume the flag converts it.
   Use a dedicated eligible ChatGPT test login in a temporary provider home.
   Assert that the new tools and behavior activate. The flag alone is insufficient.
   Generate a context reset, history lookup, note update, restart, fork, and
   rollback using synthetic content. Record only sanitized schema evidence.
   Trace backend history ingestion and native fork behavior. Check what remains
   available after a session ID changes, the source home is removed, or the
   destination account changes. Identify thread, turn, item, window, ordinal,
   byte-offset, and path references. Establish whether native Codex can rebuild
   or transfer the required backend state. Internal alpha routes alone do not
   establish a supported export/import API for Mallard.

2. **Preserve the proved state: scope after the backend transfer check.**
   Update the current Codex parser and projection for the observed contract.
   Keep context records in order. Rewrite only verified structural references.
   Test identity changes, flattened bases, and reassigned ordinals together.
   Ensure that a note-only or context-only change reaches change detection,
   snapshot validation, and transfer even when no user message is added.
   Ensure that Keep Both gives each branch independent context state.
   Apply path mapping and credential redaction to every new payload.
   Verify that transformed data remains usable by native Codex.
   Preserve inline compaction metadata with its associated records. Add no local
   note file, sidecar scanner, or notes cache for this feature. Keep account-wide
   databases, credentials, and unrelated threads out of bundles.
   If required backend state cannot be restored through a verified native
   contract, report the affected session as unsupported before publication or
   apply. Do not claim full resume or assume tool results contain a complete
   recoverable copy of the note store.

3. **Prove transfer and publish support status: 2–3 engineer-days.**
   Move a synthetic session from temporary home A to clean home B through
   reviewed Push and Pull. Restore with a new thread ID and a different project
   path. Remove access to A. Resume through native Codex and check a fact that
   requires pre-reset history, plus the saved note and current context state.
   Combine behavioral evidence with structural checks; a successful process exit
   or a visible transcript is insufficient. Repeat through Keep Both, archive,
   rollback, restart, and a second Push/Pull cycle. Test local and S3 transports.
   Verify target execution policy. Test the CLI and desktop paths separately.
   Show a specific per-session blocker when state is incomplete or unsupported.
   Update the live version gates and evidence labels only after the proofs pass.

4. **Close compressed-history support: an additional 2–4 engineer-days.**
   Handle this as a separate acceptance track. Add bounded streaming
   decompression through an established Rust library. Apply limits to decompressed
   bytes, records, and the complete selected lineage. Keep physical file stamps
   separate from logical uncompressed byte offsets. Detect active, archived,
   shared, and mixed plain/compressed inputs. Verify source replacement and
   compression during capture. Until support passes, surface a specific blocker
   when compressed data prevents a complete capture. Do not silently omit it.

Use the current schema 1 contracts where they can represent the proved state.
Keep changes in the Codex adapter, shared file readers where required, and
existing restore boundaries. Add no legacy conversion path or version adapter
registry. If the contract needs a schema change, define that change after step 1.

**Required test evidence**

| Case | Required result |
|---|---|
| Flag off, CLI 0.153.0 | Existing capture, projection, Push/Pull, and native resume still pass. |
| Flag on with a custom provider | Activation remains excluded. This is a negative test only. |
| Flag on with an eligible ChatGPT session | Activation is observed before testing portability. |
| Transfer after context reset | Saved notes and required old history work after source files become unavailable. |
| Notes or context change without a user message | Mallard detects and transfers the change. |
| Fork, rollback, and Keep Both | Context references resolve to the correct branch and boundary. |
| Destination mode off or account ineligible | Behavior is explicit; Mallard does not silently claim equivalent context. |
| Missing, malformed, oversized, or unknown required state | A clear failure occurs before incomplete publication or apply. |
| Compressed and mixed lineage | Logical cutoff validation, full capture, and resume pass. |
| Backend unavailable or destination identity changed | The result states the actual recovery limit. A visible local transcript is not treated as proof of full context recovery. |

Retain bounded reads, no-follow traversal, digest checks, immutable publication,
compare-and-swap heads, review approval, and project ownership checks. Include
synthetic secret and foreign-path canaries in new context payloads. Preserve
target-side execution authority across imported settings and approval records.

Run focused Rust tests during implementation. For handoff, run
`cargo test --workspace --all-targets` and `npm run test:aimock-servers`.
Run `npm run build` and `npm run test:frontend-integration` for frontend or DTO
changes. Run the updated mock Codex E2E gate for the default path. Add a separate,
explicitly enabled ChatGPT context-management E2E gate. Use temporary homes and
synthetic conversations. Do not test against a personal provider home.
For native UI changes, verify the complete Tauri application and capture a screenshot.

**Estimate and release decision**

Reserve **1–2 engineer-days** for the backend transfer proof. The earlier
**5–8 day** total assumed portable local state; it is withdrawn after source
verification. Full context support has no reliable completion estimate until
the native backend transfer contract is proved. Compression remains a separate
**2–4 day** estimate. There is no planned note-sidecar implementation cost.

For performance, adding a scanner for local note files would add disk work
without handling the native feature. Backend note support instead requires
measurement of request count, latency, data volume, and retry behavior. Measure
native context startup and reset separately from Mallard inventory and sync.
Keep backend note calls out of routine inventory refresh. Do not estimate actual
latency from the configured 35-second timeout.

Re-estimate after step 1. Certify the default and experimental paths separately.
Until the experimental transfer proof passes, describe that path as unverified.
Configure the feature per provider home through an explicit user choice. Keep
configuration edits outside this assessment.

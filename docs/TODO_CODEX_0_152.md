# Codex CLI 0.152 Compatibility Status and TODO

- Status: Active
- Audit date: 2026-08-31
- Paid rerun date: 2026-09-01
- Codex version: `codex-cli 0.152.0`
- Claude Code version: `2.1.259 (Claude Code)`
- Mallard version during the audit: `0.1.2`

## 1. Executive status

Mallard's default, uncompressed Codex 0.152 session path passes the complete
credential-free live test. Full Codex 0.152 support is not certified.

The current status is:

- Default uncompressed Codex session discovery, sync, reconstruction, and
  resume: **Pass**.
- Native paginated lineage and UUID allocation: **Pass**.
- Active and archived session discovery: **Pass**.
- Paid Codex flow: **Pass**.
- Paid Claude flow: **Pass**.
- Imported approval and permission policy safety: **Open**.
- Compressed rollout handling: **Open, but disabled by default upstream**.
- Queued-submission portability: **Open pending a product-scope decision**.
- Native restored thread-name behavior: **Not proved**.
- `clock.sleep` Activity status: **Open**.

Do not describe Mallard as fully certified for Codex 0.152 until the completion
criteria in section 7 are satisfied.

## 2. Version-gate updates

The audit updated the live-test gates as follows:

| Test | Previous gate | Current gate | Status |
|---|---:|---:|---|
| Paid Codex | `codex-cli 0.147.x` | `codex-cli 0.152.x` | Updated |
| Credential-free Codex aimock | `codex-cli 0.149.x` | `codex-cli 0.152.x` | Updated |
| Paid Claude Tuzi | Claude Code `2.1.235` | Claude Code `2.1.259` exactly | Updated |
| Credential-free Claude aimock | Claude Code `2.1.250` exactly | Claude Code `2.1.259` exactly | Updated |

The current-format Codex fixture markers now use `cli_version: 0.152.0`.

Changed files:

- `src-tauri/tests/live_codex_e2e.rs`
- `src-tauri/tests/live_codex_aimock_e2e.rs`
- `src-tauri/tests/live_claude_aimock_e2e.rs`
- `scripts/run-live-claude-tuzi-e2e.mjs`
- `README.md`

Historical design documents still describe the version used to produce their
evidence. Do not change those version statements without re-running the
specific proof described by each document.

## 3. E2E results

All five E2E targets were invoked after the gate update.

| Target | Result | Test time | Evidence |
|---|---:|---:|---|
| `npm run test:storage-catalog-e2e` | Pass | 3.48 s | 1 passed, 0 failed |
| `MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e` | Pass | 17.58 s | 1 passed, 0 failed, 3 filtered |
| `MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e` | Pass | 53.60 s | 1 passed, 0 failed, 7 filtered |
| `MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e` | Pass | 32.33 s | 3 paid turns; divergence, Keep Both, storage reset, and republish completed |
| `MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e` | Pass | 65.67 s | 5 paid turns and 20 loopback resumes completed through revision 11 and reset republish |

### 3.1 Credential-free Codex coverage

The Codex 0.152 aimock target completed:

- 5 source turns.
- 5 native app-server graph turns.
- 7 graph mutation turns.
- 20 disposable history resumes.
- 38 local `/v1/responses` requests.
- Native paginated lineage reconstruction.
- Two-replica Push and Pull.
- Divergence in both directions.
- Both Keep Both flows.
- Storage reset and republish.
- Skills and Codex v1/v2 pet Library flows.

The target used no provider account, provider credential, paid token, remote
model, or external model request.

### 3.2 Paid-use accounting

The successful paid Claude target completed all three Sonnet 5 processes. The
successful paid Codex target completed five provider-backed CLI turns; one CLI
turn can make more than one provider HTTP exchange. Test output does not prove
the exact billed request count. Earlier failed runs and these successful runs
remain billable according to the provider log.

Both paid targets disabled automatic retries. Before another paid run:

1. Check the Tuzi and Codex provider consumption logs.
2. Confirm the remaining quota.
3. Confirm that another certification run is required.
4. Run each paid target once. Do not add an automatic retry path.

## 4. Compatibility status matrix

| Area | Status | Finding | Required action |
|---|---|---|---|
| Default rollout discovery | Pass | Plain `.jsonl` sessions are found under `sessions` and `archived_sessions`. | None for the default path. |
| Native paginated history | Pass | The 0.152 aimock graph, projection, restore, and resume flow passed. | Keep the 0.152 live gate. Add new fixtures when the persisted item set changes. |
| Native UUID allocation | Pass | App-server thread allocation passed in the 0.152 aimock flow. | None. |
| Active and archived sessions | Pass | Both roots are scanned. Active duplicates take precedence. | None. |
| Saved startup working directory | Partial | Codex 0.152 restores an owned settings-snapshot `cwd`. Mallard terminal resume also passes `-C`. | Keep the explicit target directory. Add an imported-snapshot ownership test. |
| Cross-project `/cd` capture | Open | Mallard selects a session from initial `session_meta.cwd`. Later foreign `turn_context.cwd` records remain in the portable history. | Define whether a later cross-project turn excludes, splits, or warns on the session. Add a test. |
| Approval and permission transfer | Open, safety priority | Codex restores approval and active permission-profile data from saved history. Mallard preserves foreign `turn_context` records. | Normalize imported policy or apply an explicit Mallard policy during resume. |
| Permission metadata update | Upstream improvement only | Codex 0.152 preserves permission snapshots during client metadata updates. It does not sanitize imported history. | Do not treat the upstream fix as a Mallard safety fix. |
| Paid Claude revision sequence | Pass | Registration revision 1, content revisions 2-4, and post-reset revision 1 passed. | Keep the current matrix. |
| Paid Codex first tool turn | Pass | The direct `exec_command` path and current `FileChange` evidence passed with Codex 0.152. | Keep the current clean-cutover path. |
| Compressed rollout files | Pass | A rollout Codex compressed in place (`.jsonl.zst`) is read as its decoded bytes under the plain name; a compressed file beside its plain sibling is hidden, as in Codex. See `PLAN_CODEX_COMPRESSED_ROLLOUTS.md`. | None. Mallard writes plain files only. |
| Shared-lineage compression | Pass | A compressed base or leaf resolves by the cutoffs the leaf declares, which address the decoded bytes. | None. |
| Legacy rollout migration | Open | `codex migrate-rollouts` exists. Mallard does not detect eligibility or migration state. | Add detection or a clear user-facing instruction if legacy mode blocks a Mallard action. |
| Queued submissions | Scope decision required | Codex stores durable thread queues in `queue_1.sqlite`. Mallard does not capture them. | Decide whether pending submissions are portable session state. If yes, use per-thread queue APIs. |
| Thread names | Partial | Mallard transfers selected `session_index.jsonl` rows for its own display. Codex 0.152 uses SQLite names for paginated native reads. | Add a native `thread/name/set` Push/Pull/read E2E test. Reapply the name after restore if required. |
| Sleep item projection | Pass | A persisted sleep is an `Extension`, which Mallard can project and transfer. | None for sync. |
| Sleep Activity status | Open | `clock.sleep` has no status field. Mallard can display it as incomplete after completion. | Treat completed `clock.sleep` as a completed stateless extension. Add a parser test. |
| MCP `output_token_limit` | No change required | This 0.152 setting changes MCP output bounds. It does not require a new Mallard portable item kind. | Retain opaque safe round-trip behavior. Add a fixture only if persistence changes. |
| Plugin scope | No functional regression found | Codex 0.152 still has no plugin install scope flag. Mallard intentionally runs the native command with the selected provider home as its working directory. | Re-run the native CLI contract test and update the adapter evidence comment from 0.145 to 0.152. |
| Frozen projection documentation | Stale evidence label | `codex_paginated_projection.rs` still describes a frozen 0.147 contract. The 0.152 aimock flow passes, but it does not cover every possible 0.152 item. | Add 0.152 fixtures, including `clock.sleep`, before changing the contract label. |

## 5. Ordered TODO

### P0: Restore paid-test certification

- [x] **C152-001 — Correct the paid Claude revision matrix.**
  - Completed by the 2026-09-01 paid gate.
  - Treat setup registration as revision 1.
  - Treat the first content Push as revision 2.
  - Shift later pre-reset content revisions by one.
  - Keep storage-reset republish at revision 1.
  - Update active README sequence text.
  - Acceptance: the three-turn paid target passes once with no retry.

- [x] **C152-002 — Diagnose the paid Codex `file_change` failure.**
  - Classified as test-harness behavior and proved by the paid gate.
  - Preserve credential and path redaction.
  - Record only a bounded error category and safe event fields.
  - Do not print raw prompts, patches, provider responses, absolute paths, or
    credentials.
  - Compare the paid call shape with the passing 0.152 aimock call shape.
  - Acceptance: the cause is classified as model compliance, CLI behavior, or
    test-harness behavior.

- [x] **C152-003 — Correct the paid Codex revision matrix.**
  - Completed by the 2026-09-01 paid gate.
  - Apply the registration-only revision offset already used by the aimock
    scenario.
  - Preserve the storage-reset fresh revision 1.
  - Update active README sequence text.
  - Acceptance: the five paid turns and twenty loopback resumes pass once with
    no retry.

- [ ] **C152-004 — Reconcile paid consumption.**
  - Check the Tuzi consumption log for the one completed Claude process.
  - Check the Codex provider log for the failed first CLI turn.
  - Record counts only. Do not record credentials or private provider data.

### P0: Close imported execution-policy risk

- [ ] **C152-005 — Define the restored policy contract.**
  - Select the approval policy, reviewer, sandbox policy, and permission
    profile that a restored thread must use.
  - Do not inherit execution authority only because it appears in imported
    history.
  - Document whether historical policy remains visible as transcript context.

- [ ] **C152-006 — Enforce the restored policy contract.**
  - Prefer an explicit resume override or `thread/settings/update`.
  - If policy fields are removed or normalized during reconstruction, keep the
    transformation deterministic and versioned.
  - Preserve the target `cwd` and validated project roots.
  - Acceptance: an imported `approval_policy: never` and a permissive profile
    cannot become active without an explicit target-side choice.

- [ ] **C152-007 — Add policy and `cwd` E2E coverage.**
  - Import a thread with foreign `cwd`, approval policy, reviewer, and
    permission-profile state.
  - Resume it through the supported Mallard action.
  - Verify the target project directory and target policy are active.

### P1: Prevent silent state loss

- [x] **C152-008 — Detect `.jsonl.zst` rollouts.**
  - Done on 2026-10-14: see `PLAN_CODEX_COMPRESSED_ROLLOUTS.md`.
  - Scan active and archived roots without following links.
  - Enforce the current file-count, entry-count, and byte bounds.
  - Surface a typed blocker until decompression support exists.
  - Acceptance: a compressed session cannot disappear silently.

- [x] **C152-009 — Decide whether to support compressed rollouts.**
  - Yes, done on 2026-10-14: bounded decoding through `jsonl_tail_io`, decoded offsets, plain-only writes.
  - If no, keep a clear blocker that names the disabled capability.
  - If yes, use bounded streaming zstd reads.
  - Preserve logical JSONL offsets for paginated lineage references.
  - Reject malformed, oversized, linked, or special files.
  - Test plain, compressed, and mixed shared lineages.

- [ ] **C152-010 — Decide queued-submission scope.**
  - Decide whether queued user submissions are part of portable project
    session state.
  - If no, document the exclusion in the product contract.
  - If yes, capture only selected thread queues through `thread/queue/*` APIs.
  - Never copy the complete provider SQLite database.

- [ ] **C152-011 — Prove native thread-name restoration.**
  - Set a name through `thread/name/set`.
  - Push and Pull into a clean Codex home.
  - Read or resume the restored thread through Codex 0.152.
  - Verify both the Codex-native name and Mallard display name.

- [ ] **C152-012 — Add legacy migration awareness.**
  - Detect relevant legacy or migration states without mutating the real
    provider home.
  - Give a bounded instruction for `codex migrate-rollouts` when applicable.
  - Do not run migration automatically during review, Push, or Pull.

### P1: Correct 0.152 presentation behavior

- [ ] **C152-013 — Mark completed `clock.sleep` events complete.**
  - Extend the stateless completed-extension rule.
  - Keep explicit failed status authoritative.
  - Add current-format JSONL parser coverage.

- [ ] **C152-014 — Add 0.152 projection fixtures.**
  - Add `clock.sleep` as its persisted `Extension` form.
  - Cover a current `thread_settings_applied` event with `thread_id` ownership.
  - Cover permission-profile and working-directory fields.
  - Keep the projection fixpoint assertion.

### P2: Refresh native contract evidence

- [ ] **C152-015 — Re-prove the Codex plugin CLI contract.**
  - Verify marketplace list, plugin list, marketplace add and upgrade, and
    plugin add against 0.152.
  - Confirm that no install scope flag exists.
  - Confirm the effect of the command working directory on repository plugin
    configuration.
  - Update the adapter evidence comment only after the test passes.

- [ ] **C152-016 — Define cross-project `/cd` behavior.**
  - Decide whether Mallard warns, excludes, or splits a session that later
    changes to a foreign project directory.
  - Keep discovery deterministic and project-scoped.
  - Add active and archived-session tests.

## 6. Required verification commands

Use temporary provider homes. Never run these tests against a real `~/.codex`,
`~/.claude`, or `~/.mallard`.

Credential-free checks:

```sh
npm run test:storage-catalog-e2e
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Paid checks, only after quota and consumption review:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

Relevant handoff checks after implementation changes:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
git diff --check
```

## 7. Completion criteria

Codex 0.152 support is complete only when all applicable items below are true:

- [ ] All three credential-free E2E targets pass.
- [ ] The paid Claude target passes once with exactly three model processes and
  no retry.
- [ ] The paid Codex target passes once with exactly five provider-backed CLI
  turns, twenty loopback resumes, and no retry.
- [ ] Provider consumption logs agree with the expected paid runs.
- [ ] Imported history cannot activate a foreign approval or permission policy.
- [ ] Restored threads start in the reviewed target project directory.
- [ ] Compressed rollouts are supported or produce a visible typed blocker.
- [ ] A queued-submission product-scope decision is documented and enforced.
- [ ] A native thread name survives or has an explicit documented exclusion.
- [ ] Completed `clock.sleep` activity does not appear incomplete.
- [ ] Active and archived sessions remain covered.
- [ ] No credentials, absolute local paths, or provider-private data appear in
  portable manifests, logs, fixtures, or committed documentation.
- [ ] `git diff --check` and the appropriate frontend and Rust handoff checks
  pass.

## 8. Evidence

Official Codex evidence:

- [Codex changelog](https://learn.chatgpt.com/docs/changelog)
- [0.152 shared-lineage compression change](https://github.com/openai/codex/pull/41357)
- [0.152 permission metadata preservation change](https://github.com/openai/codex/pull/41464)
- [0.152 saved working-directory change](https://github.com/openai/codex/pull/41567)
- [Sleep tool gate](https://github.com/openai/codex/pull/41243)
- [0.152 feature defaults](https://github.com/openai/codex/blob/rust-v0.152.0/codex-rs/features/src/lib.rs#L1075-L1092)
- [0.152 persisted resume settings](https://github.com/openai/codex/blob/rust-v0.152.0/codex-rs/app-server/src/request_processors/persisted_resume_settings.rs#L14-L47)
- [0.152 queue database name](https://github.com/openai/codex/blob/rust-v0.152.0/codex-rs/state/src/sqlite.rs#L29-L34)
- [0.152 queue-store implementation](https://github.com/openai/codex/blob/rust-v0.152.0/codex-rs/thread-store/src/queue_store.rs#L54-L67)

Mallard evidence:

- `crates/mallard-infrastructure/src/providers/codex/discovery.rs`
- `crates/mallard-infrastructure/src/providers/codex/history.rs`
- `crates/mallard-infrastructure/src/codex_standalone_builder.rs`
- `crates/mallard-infrastructure/src/codex_paginated_projection.rs`
- `crates/mallard-infrastructure/src/providers/codex/actions.rs`
- `src-tauri/src/storage_catalog/adapters.rs`
- `src-tauri/tests/live_codex_aimock_e2e.rs`
- `src-tauri/tests/live_codex_e2e.rs`
- `src-tauri/tests/live_claude_aimock_e2e.rs`

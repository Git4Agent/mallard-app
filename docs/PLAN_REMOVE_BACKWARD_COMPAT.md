# Plan: Remove all remaining backward-compat and migration support

Status: Tier 1 executed + schema versions reset (2026-07-24); Tier 2
superseded and executed by PLAN_CLOUD_GENERATION_HISTORY.md (2026-07-26) —
one strict `BUNDLE_SCHEMA = 1`, one publisher, no v4/v6 machinery remains
its B0 prerequisite. See "Execution log" at the end of this document.

Premise, confirmed by the user: **nobody uses this app today and there is no
data on any cloud storage.** Therefore every read-old-write-new shim, tolerant
decode of pre-current local state, and cutover helper can be deleted outright.
Strict validation that *rejects* old formats ("unsupported schema X") is the
desired end state and stays. No migration of any kind is written in its place.

Line numbers are as of the uncommitted working tree on top of commit 458c13d
(after the project-config-capture removal). Treat symbol names as
authoritative.

This plan has two tiers:

- **Tier 1 — pure removals.** Independent items with no product decisions and
  no feature prerequisites. Each leaves the tree compiling and green.
- **Tier 2 — schema-4 bundle retirement.** The single biggest compat surface
  is the dual schema-4/schema-6 bundle machinery. It cannot be deleted until
  production Push writes schema-6, because today **the only production
  publisher is the schema-4 path** (see B0). Tier 2 is gated on that wiring.

## Key finding driving Tier 2

`BundleEngine::publish` (bundle_engine.rs:609, writes `BUNDLE_SCHEMA_V4`
manifests) is called by exactly one production site: `push_bundle`'s
implementation at commands.rs:5014. `BundleEngine::publish_schema6`
(bundle_engine.rs:815) — the conversation-thread model that all recent
thread-variant/fork/conflict work targets — is called **only from
`sync_tests/thread_variants`**. Read paths already prefer v6
(commands.rs:3755 returns no thread entries for v4; commands.rs:4001-4003
filters snapshots to v6), so the thread-sync UI (`ThreadVariantReview`) is
dormant in production: nothing ever produces a v6 bundle. The v4↔v6
coexistence rules (manifest/head validation arms, deterministic cutover
thread-IDs, the "old writer stops" guard) exist purely to support a cutover
that — with zero deployed data — no longer needs to happen incrementally.
Skip the cutover: wire Push to schema-6 once, then delete everything v4.

## Explicitly NOT backward compat — keep (do not delete)

These match compat-sounding keywords but are either tolerance for **external
formats we don't control** or **live semantics**:

- `chat_history.rs` `ProjectedRecord`/`ProjectedGit` serde aliases
  (1610-1684: `session_id`/`thread_id`, `working_directory`/`project_path`,
  `created_at`/`started_at`, `commit`/`sha`/`git_commit`) and the
  legacy/malformed-rollout full-parse fallbacks (775, 1089) — they absorb
  format variance across Codex/Claude session files, which are external.
- `commands.rs` Codex desktop-state parsing in
  `audit_codex_conversation_paths_with_repository`, including the
  camelCase/snake_case dual keys and `legacy_cwd_matches` (8846-8864) — that
  file is another application's state; tolerance there is robustness.
- `codex_plugins.rs` `RegisteredMarketplace` `alias = "ref"` (385) and the
  deliberately narrow `is_broken_marketplace_clone_error` match (392) —
  external Codex config/CLI surface.
- `global_inventory.rs` acceptance of skills without a declared name
  (444-447) — mirrors provider behavior, external.
- `provider_capture.rs` bounded-metadata fallback precedence for session
  id/cwd (1860-1934) — parses external session files.
- Every strict `validate()` equality check against the current schema
  constants (local state, machine projects, host identity, conversation
  bindings, journal, push-plan, restore-plan, dependency-plan, setup
  drafts/transactions, activity-log schemas, the storage-marker
  `layout_version` check) — these *reject* other versions, which is the
  goal. (The plan originally said "do not renumber"; the user overrode that —
  all local document schemas were renamed and reset to 1, see Execution log.)
- Host-identity damage recovery: `collect_directory_host_ids_unlocked` +
  `synthetic_missing_identity` (persistence.rs:2100-2162) recovers a lost
  identity file from **current-format** records that carry `host_id`. Keep.
  (Only the *hostless*-record tolerance goes — Tier 1, A1.)
- `RecipeBase.last_pull_at`/`last_push_at` as `Option` + serde default —
  push-only links legitimately have no pull timestamp. Keep the fields;
  rename the test (A10).
- `ProjectStorageLink.recipe: Option<BundleRecipe>` — a link that has never
  pushed has no destination recipe. Keep; reword the stale doc comment (A10).
- `provider_fork.rs` "codec fallback" — a live fork strategy, not compat.
- `conversation_sync.rs` `UniqueCompatibleAncestor` — "compatible" describes
  chain ancestry, not version compat.
- The frontend cutover guard tests asserting legacy UI stays deleted
  (project-v3-cutover tests 1-2, project-chat-history.integration.test.tsx:886)
  — AGENTS.md:22-23 mandates this enforcement. Only the schema-2 comment
  coupling changes (A9).

## Tier 1 — pure removals

### A1. persistence.rs: hostless-record (pre-host-id) migration

The last data migration in the app. Local `materializations.json` /
`dependency_applications.json` records written before host identity existed
lack `host_id`; loads inject the current host id and rewrite the file.

- Delete `inject_host_id_into_records` (3551-3576).
- `load_materializations_unlocked` (3024-3048) and
  `load_dependency_applications_unlocked` (3050-3074): drop the inject call
  and the `if migrated { write_json_atomic(...) }` rewrite; decode strictly
  (a record missing `host_id` becomes a parse error → existing
  damaged-state/quarantine path).
- Delete `materializations_absent_or_valid_legacy_unlocked` (2234-2255) and
  `dependency_applications_absent_or_valid_legacy_unlocked` (2257-2281), and
  the `legacy_identity` zero-host probe (`HostId::parse("000…0")`,
  2203-2213) in the first-run compatibility check: a true first run now
  requires those files to be **absent**, not "hostless but injectable".
  Reword the comment at 2190-2193.
- Host rotation (~600-625): the rotation flow calls
  `load_dependency_applications_unlocked` before switching hosts under the
  comment "Migrate legacy receipts before changing the current host"
  (619-620). The load-and-validate stays (it feeds quarantine decisions);
  only the migration side effect disappears. Reword the comment.
- Tests: delete/retarget persistence tests that feed hostless fixtures
  through these loaders (sweep the persistence test module for records
  without `host_id`).

### A2. persistence.rs: stale-recipe purge shim from the previous refactor

`REMOVED_RESOURCE_ID_PREFIXES` + `purge_removed_resource_recipe_entries`
(3585-3608) and its call in config load (1405) exist only to clean recipe
entries left by pre-refactor installs. No installs exist. Delete all three
plus any test exercising the purge. (Stray ids would now surface as
Requirement placeholder rows only on a machine that never existed.)

### A3. activity_log.rs: legacy history-entry reclassification

- Delete `normalize_legacy_history_classification` (187-201) and its call in
  the query loop (365), plus the test
  `query_reclassifies_legacy_history_completion_as_info` (1141).
  Safe: the current writer already emits `info` + `history.scan_started/
  _completed/_failed` events (commands.rs:1052-1093), so the normalizer is a
  no-op for post-legacy data.
- Optional cosmetic: `ActivityLogLevel::from_legacy` (48-55) is misnamed —
  it is the live parser for the internal string-level API (`"ok"`/`"warn"`
  are what call sites pass today). Either rename it (`parse_lenient`) or
  migrate `emit(level: &str)` call sites to the enum. Not compat; do not
  block on it.

### A4. bundle_engine.rs: older-release digest equivalence in restore approval

Lines 1399-1413: a conversation target whose on-disk digest equals the
*portable* source digest is treated as the already-known session because "a
conversation copied by an older Agent Sync release can still have the exact
portable source digest". Delete the `approval_target_digest` remap and pass
`target_state.digest.as_deref()` straight to
`action_needs_explicit_approval`. Consequence: a target byte-identical to the
portable form but not to the materialized form now requires explicit
approval — correct, since only old releases produced such targets.

### A5. Stale-plan installer tolerance (vestigial RestoreAction kinds)

`RestoreActionKind::InstallStandaloneSkill { .. }` (domain.rs:3417) and
`RestoreActionKind::InstallPlugin { .. }` (domain.rs:3436) are constructed by
no production code (verified: only a domain test at 4626). New RestorePlans
route installs through `DependencyPlan`/`DependencyActionKind` (which stays);
the engine keeps Blocked arms only "so a stale in-memory plan fails safely"
(bundle_engine.rs:4409-4415, arms 4599-4615). No stale plans exist.

- Delete both `RestoreActionKind` variants, their `action_type()` arms
  (3455, 3458), `describe`/`validate` arms (3475-3501), and their entries in
  the explicit-approval `matches!` list (3541-3542).
- Delete the engine's Blocked dispatch arm (4599-4615) and rewrite the stale
  doc comment on `apply_restore_plan` (4409-4415).
- `RestoreActionType::InstallStandaloneSkill`/`InstallPlugin`: after the
  kinds are gone these types are unconstructible — delete them and let the
  compiler clean the match arms (commands.rs:5945-5949 readiness
  classification, receipt mappings). Keep `InstallCustomSkill`/
  `OverwriteCustomSkill` (live custom-skill flow) and keep
  `DependencyActionKind::InstallStandaloneSkill`/plugin variants (live
  Global-tools flow).
- Frontend: sweep `src/types.ts` `RestoreActionKind` union and
  `pullReviewModel.ts` for the two removed kind strings.

### A6. domain.rs: schema-4 cutover thread-ID derivation (dead)

`deterministic_schema4_thread_id` (2753-2771) derived stable v6 thread IDs
from v4 resources "during the cutover without coordination". Its cutover
caller (`build_schema6_cutover_request`) is already deleted; remaining
callers are tests only.

- Delete the function and the "legacy group" namespace remark in the comment
  at 2742-2746 (keep `stable_conversation_id` — live for thread minting).
- Delete its domain unit tests (4740-4770).
- Rework `schema4_cutover_ids_match_on_independent_replicas_and_old_writer_stops`
  (sync_tests/thread_variants/schema_and_privacy.rs:397-441): drop the
  ID-derivation assertions (use literal thread IDs); keep the second half —
  a v4 `publish` onto a v6 head must fail (bundle_engine.rs:620-627) — as a
  renamed test until Tier 2 deletes the v4 writer and the test with it.

### A7. Orphaned schema-4 test fixture

`src-tauri/testdata/thread_variants/legacy/schema4-manifest.json` has zero
references since `schema_cutover.rs` was deleted (verified by grep). Delete
the `legacy/` directory.

### A8. bundle_engine.rs: legacy-namespace listing test

`legacy_bundle_namespace_is_ignored_without_migration` (1852-1870) documents
that an old `v3/bundles/…` object namespace is skipped without migration.
The underlying behavior is generic unparseable-key skipping. Delete the test;
if `ObjectKey::parse` rejection of foreign prefixes has no other direct
coverage, keep the single `ObjectKey::parse(...).is_err()` assertion in a
generically named test.

### A9. schema-2 side-by-side wording + guard coupling

No schema-2 (pre-V3) installation exists anywhere, so the "we do not touch
schema-2 files" isolation notes are vacuous:

- persistence.rs:3-5 module doc ("schema-2 configuration, baselines, … are
  neither read nor overwritten") — reword to describe the V3 root only.
- mod.rs:4 ("Nothing in this module reads or rewrites schema-2 profile …") —
  same.
- tests/frontend/project-v3-cutover.integration.test.tsx:34-35 asserts that
  persistence.rs contains the schema-2 sentence — drop that assertion in the
  same change. Keep the rest of the file's guards (legacy-UI absence,
  removed-command absence); AGENTS.md mandates them. Optionally rename the
  file to project-v3-shell.integration.test.tsx (update
  scripts/run-frontend-integration-tests.mjs:17) since tests 3-6 are shell
  regression guards, not cutover checks.

### A10. Stale compat wording (no behavior change)

- domain.rs:906-907 `ProjectStorageLink.recipe` doc: "Older schema-3 links
  fall back…" → "Links that have never pushed fall back…".
- domain.rs:4114 rename test `legacy_recipe_base_defaults_sync_activity_timestamps`
  → `recipe_base_defaults_sync_activity_timestamps` (absent timestamps are a
  live state, not a legacy one).
- bundle_engine.rs:38 (persistence.rs:38 comment "bounded migration/default
  growth") and similar comment references to migration headroom: reword when
  touched; not worth standalone edits.
- types.ts:75 comment "schema-4 portable bundles" — becomes accurate only
  after Tier 2; update there.

## Tier 2 — schema-4 bundle retirement

### B0. Prerequisite (feature work, not compat removal): Push publishes schema-6

Wire `push_bundle` (commands.rs, currently `engine.publish(...)` at 5014) to
`publish_schema6`: derive `published_by_origin` (PortableOrigin tokens),
split captured vs. retained files, and build the thread live-set from the
existing conversation analysis (`schema6_thread_entries` and the
`sync_tests/thread_variants/harness.rs:147-250` request builder show the
shape). This is the remaining production wiring of the flat-thread-forks
design (docs/PLAN_FLAT_THREAD_FORKS.md — accepted; docs/
PLAN_THREAD_VARIANT_CONTINUATION.md's variant-group model is superseded by
it). It is the dominant cost of Tier 2 and should be planned/reviewed as its
own change. Everything below lands after it.

### B1. Engine: delete the v4 writer

- `BundleEngine::publish` (609-809) and `PublishBundleRequest` (412-437).
  Keep `PublishExpectation` (used by `PublishSchema6Request`).
- The v6-head guard inside it (620-627) disappears with it; its test
  coverage (A6's renamed remainder) is deleted here.
- Rename `publish_schema6` → `publish` and `PublishSchema6Request` →
  `PublishBundleRequest` once the old names are free (mechanical,
  compiler-guided, includes harness and all thread_variants tests).

### B2. Domain: single bundle schema

- Delete `BUNDLE_SCHEMA_V4` (domain.rs:21).
- `BundleHead::validate` (2911) and `BundleManifest::validate` (2955):
  accept only `BUNDLE_SCHEMA_V6`; delete the v4-specific arm
  "schema-4 manifest cannot contain schema-6 conversation metadata"
  (2961-2968) and keep the origin requirement (2969-2971) unconditional.
- `validate_conversations` (3231-3234): drop the `!= BUNDLE_SCHEMA_V6`
  early return — conversation invariants always apply.
- Check `CommitRecord` validation for a v4 arm and collapse likewise.
- Rename the constant `BUNDLE_SCHEMA_V6` → `BUNDLE_SCHEMA` and reset its
  value to 1 (user-approved renumbering, consistent with the Tier 1 schema
  reset; there is no deployed data to distinguish from).

### B3. Engine read paths

- Import list (23), tag filter `matches!(tag.schema_version, V4 | V6)`
  (1192) → v6 only.
- `publish_schema6`'s `previous.schema_version != BUNDLE_SCHEMA_V6` guard
  (4287) becomes an internal invariant (unreachable after B2 validation);
  keep as a defensive error with an updated message or drop.

### B4. Commands read paths

- commands.rs:3755 (`schema6_thread_entries` early return) and 4001-4003
  (v6 filter): all fetched bundles are v6 after B2 — simplify to plain
  reads; rename `schema6_thread_entries` → `thread_entries`.

### B5. Tests and fixtures

- bundle_engine.rs unit tests constructing `PublishBundleRequest` v4 bundles
  (1873-2700: round-trip, project-content, tombstone, tag suites) — migrate
  to the schema-6 publisher (add a small local request builder mirroring the
  thread_variants harness); fixture at 1967.
- domain.rs fixtures building v4 manifests: `project_content_manifest`
  (4154-4169), the referential-integrity manifest (4491), and the
  `downgraded.schema_version = BUNDLE_SCHEMA_V4` case (4908) — build v6
  manifests (`published_by_origin` set, `conversations` may be empty) and
  replace the downgrade case with a different head/manifest mismatch.
- domain.rs:4521-4523 (schema 3 rejected) — keep; strict rejection is the
  point. Adjust expected error text if it changes.
- Delete the A6 remnant old-writer test together with the writer.

### B6. Frontend

- types.ts:75 comment → schema-6; grep `src/` and `tests/frontend/` for
  "schema-4"/"schema 4" copy. DTO `schema_version` literals in test fixtures
  (pull-review 48/231 etc.) refer to plan/inventory schemas, not bundles —
  unaffected.

### B7. Docs

- TECHNICAL_REPORT.md: after Tier 2 the line-6 claim ("no superseded
  formats, migrations, or compatibility behavior") is finally true for
  bundles; refresh the bundle-schema sections to v6-only.
- docs/AGENT_SYNC_FILE_SETS.md, README.md sync wording: mention only the
  single bundle schema.
- Banner-mark docs/PLAN_THREAD_VARIANT_CONTINUATION.md's schema-4→5 cutover
  section as superseded (cutover skipped: no deployed data; flat forks per
  PLAN_FLAT_THREAD_FORKS.md).
- AGENTS.md:22-23 stays accurate as written.

## Old-data handling

None, deliberately. Any stray pre-current local state fails strict
validation with the existing "unsupported … schema"/parse errors and flows
into the existing damaged-state/quarantine handling; stray v4 heads/manifests
on a storage target fail fetch validation. No friendlier messages are added
because no such data exists.

## Execution order

Tier 1 items are independent; suggested order groups by file to minimize
rebases: A2 → A1 (persistence), A3 (activity log), A4 → A5 → A8 (engine +
domain action kinds), A6 → A7 (schema-4 dead code + fixture), A9 → A10
(wording + guards). Tier 2 strictly after B0 lands: B1 → B2 → B3 → B4
(compiler-guided), then B5 tests, then B6/B7.

## Verification

- After each Tier 1 item and each Tier 2 step:
  `cargo test`, `cargo clippy`, `tsc --noEmit`,
  `npm run test:frontend-integration`.
- Manual round trip after Tier 2: fresh project → push (inspect the bundle:
  `schema_version: 6`, `published_by_origin` present, conversations in the
  `conversations` map) → pull on a second profile → thread entries appear in
  pull review; then corrupt a local record's `host_id` and confirm the
  damaged-state path reports it instead of silently migrating.
- Grep gates when done: `rg -i 'legacy|migrat|cutover|backward'` over
  `src-tauri/src src tests` returns only the documented external-tolerance
  sites listed above.

## Execution log (2026-07-24)

Tier 1 (A1-A10) is fully executed, plus a user-directed addition:

- **Schema versions reset** (user decision, overriding this plan's original
  "do not renumber" note): every local document schema constant lost its
  version suffix and now equals 1 — `LOCAL_SCHEMA`, `MACHINE_PROJECT_SCHEMA`,
  `HOST_IDENTITY_SCHEMA`, `CONVERSATION_BINDINGS_SCHEMA`,
  `CONVERSATION_SYNC_JOURNAL_SCHEMA`, `PUSH_PLAN_SCHEMA`, `RECIPE_SCHEMA`,
  `RESTORE_PLAN_SCHEMA`, `DEPENDENCY_PLAN_SCHEMA`, `SETUP_DRAFT_SCHEMA`,
  `SETUP_TRANSACTION_SCHEMA`, `PROJECT_CONTENT_PREFERENCES_SCHEMA`,
  `ACTIVITY_LOG_SCHEMA`, `ACTIVITY_LOG_POLICY_SCHEMA`. Frontend DTO literal
  types and fixtures moved from 2 to 1 accordingly. `BUNDLE_SCHEMA_V4`/`_V6`
  keep their values until Tier 2 collapses them to `BUNDLE_SCHEMA = 1`.
- A1/A2: `inject_host_id_into_records`, both `*_absent_or_valid_legacy_unlocked`
  validators, the zero-host first-run probe, and the recipe purge shim are
  gone; the loaders decode strictly and dropped their now-unused
  `HostIdentity` parameter; a true first run now requires the materializations
  and dependency-applications files to be absent.
- A3: `normalize_legacy_history_classification` and its test are gone;
  `ActivityLogLevel::from_legacy` was renamed `parse_lenient` (it is the live
  parser for the internal string-level API).
- A4: the older-release digest remap is gone. Deviation from the written
  plan: `codex_session_materialization_rebinds_cwd_and_is_idempotent` now
  asserts the first restore over a byte-identical unrebound copy *requires*
  explicit approval (previously asserted it did not).
- A5 swept wider than written: the frontend carried live-typed handling for
  the vestigial kinds — `types.ts` union variants, `pullReviewModel.ts` copy
  branches and `NON_DIRECT_RESTORE_KINDS` entries, the
  `deferredRestoreKinds` filter in `ProjectSyncV3.tsx`, and a pull-review
  fixture — all removed; the fixture now exercises the real pairing (restore
  `install_custom_skill` + dependency `install_standalone_skill` on one
  resource). `ApplyBundleResult.deferred_dependencies` was deleted outright
  (only the removed engine arm ever fed it).
- A6: `deterministic_schema4_thread_id` and its determinism tests are gone;
  the old-writer guard coverage survives as
  `schema4_writer_is_rejected_on_a_schema6_head` until Tier 2 deletes the
  v4 writer.
- A7/A8: `testdata/thread_variants/legacy/` deleted; the legacy-namespace
  test reduced to `object_keys_outside_the_repository_namespace_are_rejected`.
- A9/A10: schema-2 wording removed from the persistence/module docs and the
  coupled guard-test assertion dropped; stale comments reworded; cosmetic
  `legacy`-named test fixtures renamed.

Verification: `cargo test` 291 passed, `cargo clippy --all-targets` 0 errors,
`tsc --noEmit` clean, `npm run test:frontend-integration` 86 passed. The grep
gate returns only the documented external-tolerance sites and the mandated
anti-reintroduction guard tests.

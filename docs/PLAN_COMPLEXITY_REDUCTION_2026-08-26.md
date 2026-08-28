# Complexity Reduction Plan — 2026-08-26

Behaviour-preserving. No feature changes, no schema changes, no IPC surface changes,
no restructuring for its own sake. Every item below deletes code or collapses two
spellings of one thing into one.

Audited at `7b22068`, **reconciled against `eb04e4f`** — see *Reconciliation* below.

## Method

A 26-agent audit: 12 area finders swept the tree, each finding was then handed to an
adversarial verifier told to refute it, and a cross-cutting pass plus a completeness
critic swept what a per-subtree agent cannot see. 145 raw findings → 3 refuted,
62 adjusted (usually a corrected line count or a missed test dependency), 63 confirmed
as stated, 17 from the final sweeps.

Baseline: `cargo check --workspace --all-targets` and `cargo clippy --workspace
--all-targets` are both clean. Nothing here is compiler-visible. Every removal below
is dead for a reason the compiler cannot see.

Total: **~7,450 lines of code and tests**, plus **~24,500 lines of committed non-code
debris**.

## Reconciliation — five commits landed during the audit

`570d13d`, `9031174`, `aa46e84`, `37e9572`, `eb04e4f` deleted ~1,700 lines on `main`
while the audit was running. All 52 affected items were re-measured against `eb04e4f`
by a second pass: **4 done, 1 invalid, 47 still open.** Corrected counts are already
folded into the batches below. Net effect on the total: roughly neutral — five items
shrank, six grew once re-measured.

**Already done — struck from the plan**
- `sync_metrics`' per-root attribution registry is now gated behind `test-support`,
  exactly as proposed (`aa46e84`).
- `r2_setup_journal.rs` is settled (`aa46e84`); nothing dead left there.
- `session-detail.integration.test.tsx`'s 6 dead-class CSS tombstones (`eb04e4f`).
- The five duplicate icon tests, and the `storage-catalog` 7-file tombstone loop (`eb04e4f`).

**One item was a measurement error, not a finding.** The audit reported `commands.rs` at
7,192 production lines against 21,716 test lines — a 3:1 inversion. That is wrong: it
treated the file's *first* `#[cfg(test)]` attribute as the start of the test module, but
that attribute guards an 11-line block at `:7193`, with ~9,900 lines of ordinary production
code after it. Measured properly by summing each `cfg(test)` item's real extent: **17,096
production : 11,965 test, about 1.4:1.** There is no test-bloat inversion in that file to
act on. The specific duplicate-preamble finding below survives on its own evidence.

**Four regressions the parallel commits introduced.** Flagging these because three of them
remove cover that this plan's own batches depend on:

1. **`eb04e4f` deleted the repo's only dead-CSS detector.**
   `css-hygiene.integration.test.tsx` lost `every class App.css styles is rendered by some
   component` — the test that walked every App.css rule and asserted the orphan list was
   empty; its own comment recorded that a looser version had let 80 rules through, "whole
   removed features still carrying their styling". It also lost `cross-file selector
   collisions can only fall` (the 423 ratchet), the type-scale gate, and the window-floor
   gate. The file is now 2 tests, both contrast gates. **Batch G's CSS work is exactly what
   that gate existed to backstop, and now has none.** Nothing is orphaned yet — no markup
   was deleted — but restore this gate before Batch G, not after.
2. **`eb04e4f` deleted the icon catalog test it should have consolidated onto.** The ten
   duplicate icon tests went, but so did `the shared icon catalog renders as monochrome
   pixel geometry` (46 names) and session-detail's 24-unit-grid size gate. `Icons.tsx` is
   now referenced by exactly one assertion in the entire suite
   (`project-sync-cutover.integration.test.tsx:81`, checking two union members). Icon
   geometry, pixel-fill and size coverage are zero. Per the icon-set ink-band convention
   this was load-bearing — re-add it as one catalog test, not as ten.
   Two more non-duplicates went with them: session-detail's
   `initialCheckpointId|findCheckpointJump|checkpoint_jumps` regression gate and
   settings-page's `clockOptions` single-source gate over 7 formatter sites.
3. **`9031174` deleted a helper and inlined it at both call sites.**
   `harness.rs`'s `rewrite_transcript_identity` (a Codex encode-then-decode round-trip) is
   gone; its ~28-line body is now copy-pasted at `continuation.rs:971-998` and
   `recovery.rs:1238-1265`, each with a duplicated two-line import pair. A third near-copy
   already existed at `provider_codec.rs:36-64`. The corpus went from 1 helper + 1 copy to
   3 copies and no helper — inside the module the commit was cleaning. Restore one
   `codex_wire_roundtrip(...)` in `harness.rs` and point all three at it (~56 lines back).
4. **`9031174` hollowed out the test provider.** `TestProviderAdapter` now delegates six
   `SessionAdapter` methods straight into Claude's implementation (`test_provider.rs:109`,
   `:182`, `:186`, `:194`, `:203`, `:212`). The commit tacitly acknowledged this by
   rewriting `registry.rs`'s doc from "prove the common layers are registration-driven" to
   "exercise shared discovery and capture". Any suite still using `with_test_provider()` to
   prove provider-neutrality of the timeline layers now re-runs Claude's own code under a
   second key and cannot catch a Claude-specific assumption leaking into a shared layer.
   Decide deliberately: accept the narrowed claim and stop pointing timeline tests at it,
   or give the test provider its own minimal history implementation again.

**One new instance of a pattern this plan already flags.** `aa46e84` split `WatchBackend`
with per-variant `cfg` attributes (`session_watch_service.rs:48-58`), so in *every* build
configuration the enum has exactly one variant, and its `watch`/`poll` methods each carry a
two-arm `match self` where one arm is always compiled out — 23 lines of dispatch that
dispatches nothing, ~37 counting the split imports. Two cfg'd type aliases carry the same
behaviour. The same commit added a `let _ = overflowed;` discard, the pattern Batch F item 1
exists to remove.

## Settled questions — do not re-audit these

These came back clean with evidence. Recording them so no future pass spends time here.

- **No dead Tauri commands.** All 85 registered in `src-tauri/src/lib.rs` reach a real
  frontend caller, and all 67 wrappers in `project-sync/api.ts` have a real consumer.
  Verified twice, independently.
- **No unused dependency in any of the five manifests** (beyond the two in Batch I), and
  no unused npm package. Every one was checked individually with `cfg(test)` stripped.
- **The 4-crate split is load-bearing.** `mallard-application` looks like one file wearing
  a crate, but both adapters use it and their symbol sets are nearly disjoint — src-tauri
  imports 7 symbols, the CLI imports 10, only `SecretString` overlaps. Underneath they
  share ~1,450 production lines (`prepare_r2_upsert`, the `CapabilityProbe` seam,
  `R2Destination`, the `StorageSetupError` vocabulary, the R2 journal reconciliation).
  Merging it into either adapter duplicates that into the other.
- **`ProjectSyncService` is 15/18 pure forwards but must stay.** `mod project_sync` is
  private, so the three integration binaries in `src-tauri/tests/` can only reach that
  code through this `pub` seam, and they use it heavily (47 references). Its module doc
  overstates the case, though — only 8 of 58 project-sync commands actually go through it.
  Fix the comment (Batch G), keep the seam.
- **`app_settings.rs` and `activity_log.rs` existing in both `src-tauri/src/` and
  `mallard-infrastructure/` is a correct split**, not duplication. Infra owns the
  document, schema, validation and atomic write; src-tauri owns the two `#[tauri::command]`
  wrappers and the one thing infra cannot see. Zero overlapping logic.
- **The R2 validation rules being spelled in both TS and Rust is a trust boundary,** not
  copy-paste. The rules were diffed and agree; TS drives live field feedback, Rust gates
  every write.
- **`providers/test_provider.rs` is correctly `#[cfg(any(test, feature = "test-support"))]`
  gated** and does not ship. The trait surface around it is not test-only architecture.
- **`CapabilityProbe` has 4 test implementors** — a real substitution need, not a
  needless abstraction.
- **The three `conversation_*` domain modules earn their split** and merging them deletes
  zero lines.
- **All 21 String newtypes carry real validation** in `parse`. None is a bare wrapper.
  The waste is in the conversion boilerplate (Batch D), not the types.

---

## Batch A — Committed non-code debris

~24,500 lines. Zero risk, zero code touched. Do this first; it is the largest single
reduction in the repository and it is pure deletion.

| Path | Lines | Why |
|---|---|---|
| `docs/archive/skill-plugin-sync/` | 13,374 | Full `.tsx`/`.rs` **source** of a feature removed in July, kept in-tree. 13 tracked files. Referenced only by a doc comment at `storage_catalog/adapters.rs:5`. |
| `docs/backend-complexity-edge-case-audit-2026-08-18/` | 7,671 | A generated HTML report plus its build script and JSON. Committed, unreferenced. |
| `frontend-reaudit-2026-08-25.md`, `frontend-audit-findings.md`, `core-domain-audit-findings.md` | 1,809 | Point-in-time audit reports at the repo root. Nothing links to them. |
| `scripts/verify_portable_tail_conflict_model.py` | 1,058 | Pins a stale digest; no npm script or CI runs it. |
| `scripts/poc-live-claude-tuzi.mjs` | 769 | The research precursor to `run-live-claude-tuzi-e2e.mjs`, which shipped and is wired into `package.json`. |
| `scripts/claude_corpus_scan_{strict,duplicates,revised}.py` | 468 | Model rule sets the shipped codec no longer implements. |
| `scripts/credential_corpus_scan.py` | 270 | One-time pre-freeze input; the rules it fed are frozen and covered by 31 Rust unit tests. |
| `docs/THREAD_VARIANT_SYNC_REQUIREMENTS_READABLE.md` | 145 | Uncited second spelling of a document kept in full beside it. |
| `docs/1. Download storage manifest (hash and m` | 39 | A scratch note committed with a truncated, extension-less filename. |

Keep `claude_corpus_scan_thinking_blind.py` and `claude_corpus_scan_severed.py` — both are
still cited, one from a production doc comment (`providers/claude/codec.rs:1332`).
The five corpus scans were checked for being variants of each other; they are not
(max pairwise similarity 0.65), so they are judged individually.

Delete `adapters.rs:5`'s pointer to the archive in the same change. Git history keeps
everything; that is what it is for.

**Open check:** `scripts/live-{claude,codex}-aimock-server.test.mjs` (1,051 lines) are run
by no npm script. Confirm before deciding.

## Batch B — Dead and write-only state

~750 lines. Fields written at many sites and read at none; functions with no caller;
enum variants never constructed. Each needs its test half retargeted rather than deleted
where the test is the only cover for a live rule.

**`mallard-infrastructure`**
- `persistence.rs:407-433, 606-628, 731-748` — the three origin-host-alias methods
  (`update_host_display_name`, `resolve_origin_host_label`, `mutate_origin_aliases`) have
  zero production callers. **155 lines.** Four test halves need surgery, including
  `sync_tests/thread_variants/continuation.rs:429-440`, which is not obvious and will not
  compile after the deletion. Note in the change that `HostIdentity.local_display_name`
  stays on disk and in the IPC payload but can no longer ever be set.
- `persistence.rs:434-482, 1916-1969` — `rotate_host_identity` and the fail-closed
  rotation branch only it reaches. **124 lines.** `allow_damaged_recovery` becomes
  unconditional. Delete two rotation tests; **keep**
  `verified_rotation_rejects_a_structurally_valid_foreign_host_binding` — its
  `"different host"` assertion is the only cover for a live rule.
- The profile-wide provider session inventory: built by Codex, merged, never read outside
  one test. **50 lines.**
- `validate_session_ownership` (`project_activity.rs:1411`) — a 48-line `pub fn` with zero
  callers anywhere. **48 lines.**
- ~~`sync_metrics.rs` per-root attribution registry~~ — **done in `aa46e84`.**
- `ProviderDescriptor.capabilities` and `.adapter_version` are write-only end to end;
  `ProviderCapabilityLevel` exists only to fill them. **32 lines.**
- `SessionDetailCapabilities` ships over IPC on every session-detail load and no frontend
  code reads it; two of its four fields are the same expression. **29 lines.**
- `ConversationCloudPublication.manifest_size` / `.scope_pins`, plus the manifest scan that
  fills them. **20 lines.**
- `StandaloneThreadAllocation` and the `id_map` nobody reads. **17 lines.**
- The always-empty `subagents` transport surface in the standalone builder. **21 lines.**
- Three unread `CodexSessionMetaRecord` fields. **21 lines** — accept the loss of
  `parent_thread_id` validation explicitly.
- `ResolvedSegment.canonical_messages`, computed per segment, never read. **5 lines.**
- `CodecUnknownReason::UnsupportedCodec` — never constructed. **3 lines.**
- The provider action contract's `transcript_path`, which no implementor reads. **7 lines.**
- Three identically-bodied timeline budget accessors pretending to be three knobs over one
  atomic. **14 lines.**
- `WatchBackend` is a one-variant enum in every shipped build and its `poll()` is a no-op on
  the watcher's hot path. **8 lines.**

**`storage_catalog`**
- `CatalogLocalInventory.plugin_owned_skills` is dead — removing it also kills the
  `owner_labels` map. **32 lines.**
- `PortablePluginRequirement` exists only so validation can reject it. **19 lines.**
- `BlockedCandidate.provider` is set at eleven sites and read at none. **13 lines.**
- `ResolvedCatalogContext` models two simultaneous provider profiles when exactly one ever
  exists. **20 lines.**
- Three enum variants never constructed anywhere. **12 lines.**
- `CatalogResourceForkPlan.source_kind` is written and never read. **11 lines** — the plugin
  refusal in its `match` must stay.
- `CatalogPushRow.removal_candidate` is hard-coded `false` and read by nobody. **4 lines.**

**`mallard-domain`**
- `conversation_transcript_path` / `conversation_sidecar_path` — superseded by the `cloud_*`
  twins, zero call sites repo-wide. **26 lines.**
- Four serde derives in `storage_setup.rs` never exercised. **2 lines.**
- Fourteen `pub` items referenced only inside the crate — narrow to `pub(crate)`.

## Batch C — Test-only architecture in production

~230 lines. AGENTS.md: *do not introduce extra architecture solely to make code testable.*
Each of these is production code whose only reason to exist is a test.

- `mallard-cli`: the `StorageBackend` trait plus a `#[cfg(test)]` parallel entry point exist
  only to host one test fake. **60 lines.**
- `CatalogPushApproval::recommended` / `CatalogPullApproval::recommended` — reachable only
  from tests. **35 lines.**
- Four public accessors reachable only from their own tests, and one with no caller at all
  (infra). **18 lines.**
- `PortableConversationTail::tail_record_count` — reachable only from its own two
  assertions. **16 lines.**
- `PrepareR2StorageDraftResult` — a return type whose only reader is its crate's test module.
  **14 lines.**
- `merge_activity_sessions` — a public wrapper reachable only from its own tests. **13 lines.**
- `SessionAnalysis::is_resumable` — every call site sits inside a `#[cfg(test)]` module
  (`claude_history_tail.rs`, `providers/claude/codec.rs`), as does the separate
  `CodexTranscriptAnalysis::is_resumable`. **7 lines**, clean. *Rescoped:* the audit paired
  this with a "dead `timeline_version` default", which is **not** dead — `ClaudeAdapter`
  and others override it. Only the `Option` nobody returns `None` for goes with it, for
  ~10 total, not 14.
- `push_bundle_with_recipe_with_repository` — a `cfg(test)` wrapper living in production
  `commands.rs` with one call site. **10 lines.**
- `Repository::derive_origin_host_token` — a `cfg(test)` method with no callers at all.
  **9 lines.**
- `allocate_codex_thread_ids_local` — a public function only its own tests call. **9 lines.**
- The `mallard-domain` `test-support` feature gates nothing and its comment names a deleted
  function. **7 lines.**
- `ManagedStorageClearResult::objects_deleted()` — one assertion in its own test. **6 lines.**
- `initialStep` is a test-only prop on both review workspaces; production never passes it.
  **5 lines.**
- Push-approval selection logic exists twice: in `PushApproval::recommended` and in the
  `cfg(test)` `push_reviewed_test_context`. **10 lines.**

## Batch D — One kit for hashing, time, and bounded reads

~460 lines, and the largest source of silent drift risk in the repo.

- **SHA-256 → lowercase hex is privately re-implemented 18 times** across all three Rust
  crates (`hex_digest` ×4, `hex_sha256` ×7, `sha256_hex`, and inline spellings). All 18
  produce the identical byte string, so no digest, manifest, or on-disk value changes.
  Export `hex_lower`, `sha256_hex`, and `update_length_prefixed` from `mallard-domain`
  (`hex_lower` already exists there); delete the other 15 and re-point imports. **~130 lines.**
- **Three trivial `SystemTime` helpers are redefined 15 times** across two crates and ten
  files (`now_secs`, `modified_secs`, `modified_nanos`). Add them to `mallard-domain`.
  **~105 lines.** Leave `persistence.rs:2654` alone — it deliberately propagates a pre-epoch
  clock as an error rather than saturating, a different contract.
- **Seven byte-identical copies of the String-newtype `TryFrom`/`From`/`Display`
  boilerplate** in `mallard-domain` collapse into one macro. **100 lines.**
- Two identical `canonical_json` copies. Folded into the hashing item above.
- `bundle_engine::canonical_real_dir` is a byte-identical private copy of
  `provider_capture::canonical_existing_dir`. **15 lines.**
- The plan-expiry predicate is written five times across two crates, twice with the same
  message. **15 lines.**
- `skill_tree::read_stable` and `codex_pet::read_bounded_no_follow` are the same bounded
  no-follow read. **22 lines.**
- `parse_optional_metadata_u64` and `parse_optional_metadata_u32` are the same function
  twice. **17 lines.**
- `s3_store` defines its own `MAX_LIST_PAGE`, shadowing the `storage_core` constant that
  `local_store` imports. **1 line.**

## Batch E — De-duplicate the provider history parsers

~390 lines. `codex/history.rs` (3,566) and `claude/history.rs` (2,642) are not one parser
written twice — but nine helpers in them are byte-identical, and a further set has already
drifted.

- **Nine helpers duplicated verbatim** between the two: `read_source_value`,
  `safe_project_relative_path`, `matching_entry`, `looks_like_binary_or_base64`,
  `normalized_type`, `source_span` (all byte-identical), plus `max_line_bytes`,
  `BoundedLine`/`read_bounded_line`, and `bounded_text` (differing only in visibility,
  `io::Result` vs `std::io::Result`, and early-return vs if/else). Move to
  `providers/traits.rs`, which already owns the shared session-detail helpers, as
  `pub(crate)`. **164 lines** (re-diffed at HEAD; neither file was touched by the five
  commits). Seven pairs are byte-identical; three differ only cosmetically — `max_line_bytes`
  (`fn` vs `pub(crate) fn`), `read_bounded_line` (`io::Result` vs `std::io::Result`, if/else
  vs early return), and `bounded_text` (Claude uses the named const `BINARY_PREVIEW_OMISSION`,
  Codex inlines the same literal). Do **not** merge `truncate_chars`, `value_as_preview`, or
  `tool_payloads` — those genuinely differ. Keep `read_event_detail` and `read_timeline_page`
  per-provider: their error strings name the provider.
- The same nine, counted again from the session-detail side — same work, one change.
- `session_detail.rs` and `project_activity.rs` keep private copies of the same five
  helpers, **and the copies have already drifted**. **36 lines.** Do this one first: the
  drift is a live correctness hazard, not weight. `modified_nanos` clamps with
  `.min(u64::MAX as u128)` in `session_detail` but does a bare `as u64` in
  `project_activity`, which silently **truncates** the high bits instead of saturating;
  and `git_output` produces different error text for the same failure in each copy.
  Pick the clamping spelling when merging.
- The same UUID validator written once per provider actions module. **6 lines.**
- Share the tail-window verification prologue between the Codex and Claude refreshes.
  **~0 net lines** — do it for drift protection, not for the count.

**Risk item, handle carefully:** session-detail checkpoints and Activity git history
duplicate four git invocations and the `%x1f` line parse, but the two copies differ in
branch cap, subject truncation, and failure handling. **30 lines** only if those three
differences are reconciled deliberately; otherwise leave both and note why.

## Batch F — Collapse per-command boilerplate

~820 lines. The command layers repeat the same prologue dozens of times.

**`project_sync/commands.rs`** (production runs to ~17,096; `mod tests` is 17,097–28,870,
plus eight scattered `cfg(test)` helpers totalling 191 lines)
- `fn repository<R>(_app: &AppHandle<R>)` binds its parameter to `_app` and the generic
  exists only to type it. 39 of 58 commands carry an `AppHandle` for nothing else. Change to
  `fn repository()` and drop the parameter from those 39, plus the `_app` parameters in
  `app_settings.rs`/`app_reset.rs` and the `let _ = app;` in `list_agent_providers`.
  `AppHandle` is injected by Tauri and never supplied by the JS caller, so the invoke
  contract is unchanged. **66 lines** including the 21 explicitly-unused `_app` parameters
  elsewhere. Add `session_watch_service.rs`'s new `let _ = overflowed;` from `aa46e84`.
- The "active binding for this project" lookup is copy-pasted at 8 production sites.
  **53 lines.**
- Conversation pin bundles are hand-built at four sites; three of the four pairs are exact,
  one differs on its first line. **30 lines.**
- The load-config / require-project / require-link prologue at 5 sites. **22 lines.**
- The "resolve profile, check provider match, check readable" block, duplicated between
  binding and draft resolution. **20 lines.**
- `ThreadSyncDecision` is a field-for-field identity DTO of
  `mallard_domain::ConversationDecision`. **18 lines.**
- Five identical `spawn_blocking` wrappers differing only in a label. **18 lines.**
- `RepositoryIdentity` — a one-field newtype with no invariant. **14 lines.**
- The `project_link` helper is bypassed by three inline copies of its own body. **9 lines.**
- `WatchBackend` (`session_watch_service.rs:48-82`, new in `aa46e84`) is a one-variant enum
  in every build configuration, with two `match self` methods whose second arm is always
  compiled out. Replace with two cfg'd type aliases. **~37 lines.**
- Five copies of the "could not open local state repository" prologue bypass the existing
  `session_detail_error` helper. **12 lines.**
- 13 of the 55 `pub use` re-exports in `lib.rs` have no consumer. **13 lines.** Note: most
  of the *other* seemingly-dead re-exports are load-bearing — they name the return types of
  `pub` service methods, and removing them trips `private_interfaces` and breaks the
  zero-warning baseline. Only remove the 13 that are genuinely free.

**`storage_catalog`**
- **Delete `service.rs` (386 lines).** All 11 of its methods are single-expression forwards
  to `*_with_repositories` free functions in `resource_commands.rs`. Unlike
  `ProjectSyncService`, nothing external needs this seam. **~190 net lines** — 166 forwarding
  lines plus ~24 of module doc and imports. `from_user_home`/`from_home_dirs` (~48 lines with
  their two absolute-path guards) must be **rehomed, not deleted**:
  `tests/support/catalog_flow.rs:176` constructs the service through `from_home_dirs` and
  `lib.rs:43-44` re-exports it. Also relocate its two unit tests and give
  `CatalogPushApproval`/`CatalogPullApproval` a home.
- `storage_catalog/persistence.rs` re-implements `mallard-infrastructure`'s bounded-read,
  atomic-write and symlink primitives. **128 lines** (`ensure_no_symlinks` must be exported).
- Six copy-pasted `load_X`/`mutate_X` document pairs in `CatalogStateRepository` — **95 lines**,
  larger than first measured: 12 methods over ~212 lines, each `load_X` a 9-10 line
  guard/load/validate and each `mutate_X` a 21-23 line guard/load/validate/mutate/bump/write,
  identical but for the type, the `*_path()` call, the `MAX_*` const and a label string.
  **Now easier:** `37e9572` deleted the two tests that pinned these bodies
  (`documents_default_and_round_trip_with_revision_bumps` and
  `legacy_catalog_documents_are_never_read`). Only `scopes_document_round_trips_with_revision_bumps`
  still pins a pair — note that the legacy-filename ignore rule now has no cover at all.
- The Catalog Push and Pull apply prologues restate the same 8-step staleness gate, differing
  only in three message strings. **28 lines.**
- The same unreachable tag-identity re-check at two call sites. **12 lines.**

**Elsewhere**
- `storage_setup.rs` re-spells three error/mapping constructors inline instead of calling the
  helpers that already exist. **17 lines.**
- `verify_storage_journaled` and `verify_storage_with_probe_for_setup` duplicate one
  load/validate/find/kind-check preamble — extract it, do **not** delete the second copy.
  **16 lines.**
- Fixed CLI errors constructed inline five times instead of through their constructor.
  **14 lines.**
- Collapse `inspect_manifest_version` and `fetch_manifest_version_optional` onto one body.
  **5 lines.**

## Batch G — Frontend dead paths, dead CSS, comment noise

~1,100 lines.

**CSS — the biggest item, and the one that needs care**

> **Do this first: restore the dead-CSS gate.** `eb04e4f` deleted
> `every class App.css styles is rendered by some component` and
> `cross-file selector collisions can only fall` from
> `css-hygiene.integration.test.tsx`, which is now down to its two contrast gates. Those
> were the only automatic backstop for everything in this section, and the orphan gate's own
> comment recorded that a looser version had let 80 dead rules through. Re-add both before
> deleting a single rule, so the batch is measured rather than eyeballed.
- **26 class vocabularies are rendered by no component** — 36 fully-dead rules, ~213
  non-blank lines. The two large clusters are `App.css:4008-4137` (the pre-scope-link
  `.storage-link-*` settings UI) and the `.log-header/.log-toolbar/.log-entry/.log-badge`
  family from a previous `LogPanel` shape. Ten grouped rules mix dead with live selectors
  (`.btn, .status-btn` and friends) — drop only the dead selector. **~250 lines.**
  Blocker: `project-activity.integration.test.tsx:701` asserts the literal
  `\.storage-link-menu`; update it in the same change.
- **392 App.css declarations are shadowed verbatim by RetroTheme.css**, 37 App.css rules
  dead in full. **~385 lines.** Load order is confirmed (`App.tsx:10-11`, both global, no
  scoping class, RetroTheme has zero `!important`).
  **Verify by computed style, not text.** This repo's CSS gates assert source text and
  cannot catch cascade bugs — that is exactly how seven prior fixes regressed silently.
  "Later wins" only holds at equal specificity *and* equal at-rule context; a same-selector
  App.css declaration inside `@media` is not beaten by an unconditional RetroTheme rule.
  Before deleting each declaration, also grep `tests/frontend/` for a source-text assertion
  pinning that (selector, property) — the known collision is
  `.sync-titlebar { min-height: 3.25rem }` at `project-sync-cutover.integration.test.tsx:82`.
  The css-hygiene cross-file ratchet that used to sit at 423 was deleted in `eb04e4f`;
  restore it at the post-cleanup number rather than the old one.
- 13 CSS custom properties, one `@keyframes`, and 6 section banners with no consumer.
  **35 lines.**
- **33 CSS comments narrate a removed design** instead of stating the current constraint —
  including two at `App.css:6436` and `:6602` that explain a compounding rule for
  `.sync-callout`, a class that no longer exists. **~40 lines.**

**Components**
- The non-embedded arm of `RestorePlanView` and `ProjectChatHistoryContent` is dead — all
  four production renders pass `embedded` (`ProjectSync.tsx:3578`,
  `PushResourceWorkspace.tsx:623`, `ProjectLinksWorkspace.tsx:3251`, `RestorePlanView.tsx:537`).
  **123 lines, but only ~48 of them are TSX** — the largest blocks are the project-context
  `<section>` at `ProjectActivityPage.tsx:1682-1696` and the meta `<p>` at
  `RestorePlanView.tsx:480-488`; most other sites are ternaries that collapse to a literal
  for zero net lines. The other ~75 lines are CSS reachable only from the dead arm
  (`.sync-history-context-item`, `.sync-history-storage-item`, `.sync-pull-back`). Land it
  with the CSS, not before it.
- `src/types.ts` carries the `ConversationActionPins` wire subtree — 8 interfaces plus 2
  aliases — that no component reads. **61 lines.**
- `SyncOutcomeNotice`'s "resource-neutral" view-model and its converter serve exactly one
  feature. **55 lines.**
- `ResourceInventory`'s per-resource status badge is unreachable: the `statuses` map is empty
  at every call site. **30 lines.**
- Three verbatim copies of the "Mon D, YYYY, HH:MM" formatter, plus `fullStamp` duplicating
  `formatFullDateTime` character for character. **22 lines.** These belong in `src/format.ts`,
  which both `storage-catalog` and `project-sync` may import without violating the
  no-cross-import rule. Note `570d13d` half-fixed the adjacent case: it hoisted
  `const stamp = rowStamp(row)` in `ResourceCard` (`StorageCatalogPage.tsx:472`) but left
  `stampSeconds(row)` beside it, so the card still walks the same two fields through two
  functions and formats one instant in two shapes onto one element. Fix both together.
- Four pass-through helpers that only rename or restate their body. **16 lines.**
- The frontend re-validates a config schema the Rust loader has already rejected — four
  unreachable guards. **14 lines.**
- `syncReviewStateLabel`'s post-switch fallback ladder is unreachable:
  `compared_against_storage:false` always implies `state:"unavailable"`. **12 lines.**
- The Catalog pull-action taxonomy is re-derived by prefix-matching prose the frontend itself
  just formatted. **12 lines.**
- `SyncOutcomeStat` is a byte-identical copy of `SyncOutcomeStatView`, in a file that already
  imports from that module. **7 lines.**
- `LogPanel`'s `onClear` prop and its Clear button are unreachable at the panel's only mount.
  **7 lines.**
- The ancestor-directory closure is walked twice, once per review direction. **6 lines.**
- `stampSeconds` duplicates the pick already inside `rowStamp`, and both run on the same row.
  **5 lines.**
- Three entries of `StorageCatalogPage`'s TONE map and its `?? "unknown"` fallback are
  unreachable. **4 lines.**
- Two different equality predicates for `StorageConfig` — field-wise and `JSON.stringify` —
  decide the same storage-draft questions. **3 lines.**
- `providerLabel` exists twice; the fetch-progress readout string is written three times.

**Comments**
- 16 history-narrating comments in project-sync describe superseded designs. **35 lines.**
- 5 module comments in the codex modules narrate refactor history. **5 lines.**
- 2 comments narrate a past change to `cloud_thread_id` rather than the current rule.
- `apply_restore_plan_lazy` carries two stacked summary sentences from an earlier rename.
- The `aws-smithy-types` comments in both `Cargo.toml` and the Rust source misstate why the
  dependency is not a dev-dependency (4 of 6 call sites are production).
- `project_sync/mod.rs`'s service docblock claims all commands cross the service; only 8 of
  58 do.

## Batch H — Prune the test corpus

~3,700 lines, re-measured at HEAD. Note the audit's "`commands.rs` is 3:1 test-to-production"
claim was a measurement artifact and is withdrawn — the real ratio is about 1.4:1. Every item
below stands on its own evidence.

**Rust — mostly relocation, not coverage loss**
- **76 byte-identical helper functions (1,931 lines)** are triplicated across the three e2e
  binaries. `src-tauri/tests/support/` already exists and is already shared by all three
  (`support/mod.rs`, `catalog_flow.rs` at 1,799 lines) — the helpers just never moved there.
  Four more are blocked by differing callees; leave those. **Zero coverage lost.**
- Restore `harness.rs`'s `rewrite_transcript_identity` as a single
  `codex_wire_roundtrip(...)` and point `continuation.rs:971-998`, `recovery.rs:1238-1265`
  and `provider_codec.rs:36-64` at it. **~56 lines back** — this reverses duplication
  `9031174` introduced, so it is an addition to the helper file and a net deletion overall.
- One whole test function (174 lines) is a true duplicate across the two Codex e2e binaries.
  Delete the `live_codex_e2e` copy, not the aimock one: `live_codex_e2e` additionally needs a
  real Codex binary and `MALLARD_LIVE_CODEX_PROVIDER_KEY`, so the aimock copy is the one
  contributors can actually run. **176 lines.**
- 26 `commands.rs` tests inline the same bind-and-link preamble across 34 call sites —
  16 `save_project_binding_with_repository` blocks and 18 `save_project_link_with_repository`
  blocks, 441 lines of preamble. **285 lines**, and the easiest item in this batch: the
  consolidation target is already written and already proven —
  `bind_and_link_test_replica` (`:18036`) does exactly this pair and has 35 callers, and
  `bind_and_link_claude_replica` (`:23145`) is its Claude twin. The test module is
  byte-identical to `7b22068`, so nothing here moved.
- The "republish the manifest with one mutated conversation stamp" block, duplicated in two
  foreign-stamp tests. **60 lines.**
- `live_codex_headless_resume_capture_poc` duplicates the gated live test's assertions —
  but it is the only unpaid live-CLI path. **56 lines**, and a judgement call: delete only if
  losing the unpaid path is acceptable.
- `bench_20mb_synthetic_rollout` asserts no throughput bound; it is an on-demand measurement,
  not a test. **40 lines.**
- Three CLI tests assert that removed subcommands and flags are still removed — 47 lines
  total, but only the 15-line tombstone loop at `cli.rs:88-102` is safe to delete outright.
  The other two also assert the CLI does not echo a secret argument back in its error output,
  a live privacy property with no other cover; they are near-duplicates *of each other*, so
  merge them into one rather than deleting either. **~31 lines.**

**Frontend — deleting brittle proxies, not coverage**
The pattern to remove: tests that `readFileSync` a source file and regex it. They pass while
the behaviour is broken, which is how seven prior fixes regressed silently. Suite-wide at
HEAD: 232 tests, 83 of which read source, concentrated in three files.

- `project-sync-cutover.integration.test.tsx` (now 949 lines, 25 tests, 20 reading source):
  18 tests are source-text-only, totalling 747 lines. **Keep the `generate_handler!` gate**
  (`L72-76`) — a real constraint — leaving **742 lines**. This *grew* from the audited 444
  because `eb04e4f` trimmed behaviour-free filler out of mixed tests, leaving several that
  were mixed as now-pure source-text. Only 5 tests carry real behaviour.
  **Judgement call:** 409 of the 747 are CSS/asset text gates, and source text is the only
  thing those *can* assert. If you keep them, the removable figure is **333**. Prefer keeping
  them until the css-hygiene orphan gate is restored, then reassess.
- `storage-settings.integration.test.tsx`: 7 source-text-only tests, 164 source-text
  assertions, zero behaviour assertions. **272 lines.** Untouched by the five commits.
  Correction to the audit: the seven are **not** consecutive —
  `the draft payload is the form minus the server-owned fields` (`L947-967`) sits between the
  first and second and is a genuine `storageDraftPayload` unit test. Do not sweep it up.
- `storage-catalog.integration.test.tsx` (537 lines, 18 tests, 17 reading source):
  **103 lines.** The 7-file tombstone loop is already gone (`eb04e4f`). Note the
  source-text-only population actually *grew* 11→12 tests, because stripping the `existsSync`
  calls turned one mixed test into a pure source-text one.
- Four frontend tests regex Rust for behaviour Rust unit tests already cover — not two:
  `catalog-scope-removal:43-55`, `resource-inventory:352-356`, and
  `storage-catalog:521-526` and `:528-537` (both over `storage_catalog/profiles.rs`, which
  carries 5 `#[test]` fns of its own). **35 lines.**
- `release-config.test.mjs`: two tombstones for features removed so long ago their names
  survive nowhere else, plus a version-consistency check that `check-release-version.mjs`
  performs one CI step earlier and which hard-codes the literal `0.1.2`. **101 lines.**
- The pull-review fixture still builds `ConversationPlanAction` fields the cutover retired and
  omits the one it made required. **11 lines.**
- ~~session-detail CSS tombstones~~, ~~five icon tests~~ — **done in `eb04e4f`**, which also
  over-deleted; see *Reconciliation*. Re-adding one icon catalog test is a prerequisite for
  calling this batch complete, not an optional extra.

**Do not touch:** tests covering data loss, backup/restore, remote object writes, credential
redaction, path traversal, schema boundaries, and the cutover-enforcement tests AGENTS.md
calls intentional.

## Batch I — Dependencies and build config

Small in lines, real in shipped weight.

- **`mallard-cli`: delete the `storage verify` subcommand.** No caller, no test. **77 lines**
  across `args.rs`, `lib.rs`, `backend.rs`, `output.rs`. It is the only thing that needs a
  direct tokio dependency in the CLI: tokio still links transitively, but the multi-thread
  runtime that `ProductionBackend::from_user_home` builds on *every* CLI invocation —
  including `storage r2 setup` — disappears. Knock-on: `storage_setup.rs:528`'s
  `verify_storage` loses its only workspace caller.
- The CLI exit-code table is a second copy of `StorageSetupErrorKind::exit_code()`, kept in
  sync by a test that exists only for that. **30 lines.**
- `src-tauri`'s `hmac` dependency exists only for a verbatim copy of `mallard-domain`'s
  origin-token HMAC. **14 lines**, one dependency.
- `src-tauri`'s `getrandom` exists only for `random_hex`, which reimplements
  `mallard_domain::generated_named_id`. **7 lines**, one dependency.
- `tokio features = ["full"]` in two crates pulls `signal-hook-registry` and tokio's
  signal/process/io-std into the release build for nothing. Narrow the feature list.
- Two grants in `capabilities/default.json` expose IPC nothing calls. **8 lines.**
- `[profile.debugging]` in the uncommitted `Cargo.toml` is a build mode nothing invokes or
  documents. **6 lines.** (The other dev-profile comments there *are* real design
  constraints — keep those.)

## Rejected — do not do these

- **Replacing `providers/timeparse.rs` with chrono.** This looks like an 88-line dedup and is
  a behaviour change in costume. `parse_timestamp_value` is the ingest boundary for
  third-party transcript envelopes and is deliberately lenient: it divides by 1000 above
  10^10, accepts both `i64` and `u64`, and splits on the first zone character rather than
  validating the RFC-3339 grammar. `chrono::parse_from_rfc3339` is strict; every input it
  rejects becomes `None`, which shows up as activity rows and session headers silently losing
  their time — on exactly the corpus nobody has locally. Only the Gregorian day arithmetic is
  shareable, and that is a fraction of the file.
- **Deleting `ProjectSyncService`** — see Settled Questions.
- **Merging `mallard-application` into an adapter** — see Settled Questions.
- **Collapsing `BundleKind` and `ConversationSourceDisposition`**, both single-variant enums.
  Both are `Serialize`/`Deserialize` on-disk document fields; collapsing them changes the
  manifest format.
- **Removing `ThreadSyncCounts`' four frontend-unread fields.** They are asserted in ~12 Rust
  tests as the comparison's observability surface; deleting them strips real coverage.
- **A wholesale sweep of "DTO fields never named in TS."** ~40 hits, but nearly all are
  internal persisted-document structs that are `Serialize` for disk, not for IPC. Wrong
  without per-struct triage.
- **Splitting `commands.rs` into modules.** It deletes nothing.

## Sequencing

**Restore the three deleted gates first** — the css-hygiene orphan + collision gates, and one
icon catalog test. They are what makes the rest of this measurable rather than eyeballed, and
Batch G is unsafe without them.

Then: A → B → C → I → D → E → F → G → H.

A is pure deletion and unblocks nothing, but it removes 24,500 lines of noise from every
future search. B and C are independent per item and can land in any order. D must precede
E and F, since both re-point call sites at the shared helpers. G's CSS work is the only item
requiring browser verification. H lands last so it prunes tests against the final code.

## Verification bar per batch

```bash
cargo check --workspace --all-targets && cargo clippy --workspace --all-targets -- -D warnings
```

```bash
npm run test:integration
```

Both must stay at zero warnings — that clean baseline is what makes the next audit cheap.

For Batch G's CSS work, additionally verify by **computed style** in the running app, not by
source text: read the resolved value off the element for each touched selector before and
after. The repo's CSS gates assert source text and cannot see a cascade regression.

For Batch B, run `npm run test:live-codex-aimock-e2e` and
`npm run test:live-claude-aimock-e2e` — the host-identity and provider-inventory removals
touch paths the `--lib` suite does not reach.

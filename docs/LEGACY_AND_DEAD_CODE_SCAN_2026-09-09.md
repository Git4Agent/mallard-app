# Legacy and dead-code scan — 2026-09-09

Scan only. No file in the working tree was modified by this pass.

**Baseline:** working tree on top of `f9628b0`, 176 uncommitted paths. Line
numbers are as of that tree, not as of HEAD.

**Corpus:** 148 Rust files (~152k lines) across `crates/` + `src-tauri/src`,
48.7k lines of TypeScript, `src/App.css` (692 class selectors), `scripts/*.mjs`,
177 files under `docs/`.

**Instruments.** `cargo check --workspace --all-targets` (authoritative for
non-`pub` dead code). Reference-counting passes, each with `#[cfg(test)]` blocks
and test files excluded from the *caller* side, over: 1 949 `pub` Rust items,
485 `pub` enum **variants** (path-agnostic, so `use E as Alias` construction is
counted), 2 089 `pub` struct **fields**, 491 TypeScript exports, and 496 DTO
field names in the `src/types.ts` mirrors. Plus a Tauri command surface diff, a
dependency-manifest-vs-source diff, a CSS selector-vs-markup diff that accounts
for template-literal class construction, and a compat-keyword sweep sorted by
*who owns the format*.

---

## Answer to the question asked

**Schema is already 1 everywhere, and there is no legacy support left to
remove.** Not "close enough" — exact:

| Check | Result |
| --- | --- |
| Schema constants in Rust | 20, **all `= 1`** |
| Schema constant in TypeScript | `PROJECT_SYNC_SCHEMA = 1` (`src/types.ts:594`) |
| Validators using exact `!=` / `==` against the constant | **all 38** |
| Validators accepting a *range* (`<`, `>`, `matches!`, `1 \| 2`) | **0** |
| `#[serde(alias = …)]` anywhere | **0** |
| `publish_schema*`, `schema-2`, `schema-4`, `BUNDLE_SCHEMA_V4/V6` | **0** — fully retired |
| Read-old-format tolerance | **1** (B1 below) |

`docs/PLAN_REMOVE_BACKWARD_COMPAT.md` records Tier 1 executed 2026-07-24 and
Tier 2 executed 2026-07-28. The code agrees with both execution logs — I
verified the claims rather than trusting them. Any note still describing Tier 2
as "gated on `publish_schema6` wiring" is stale; that wiring landed and the v4
machinery is gone.

### Other negative results worth having on record

- **Tauri surface is exactly balanced.** 87 commands declared, 87 registered in
  `generate_handler!`, 87 invoked from `src/`. Zero orphans in either direction.
- **`cargo check --workspace --all-targets` emits zero warnings.** No
  `dead_code`, `unused_imports`, or `unused_variables` anywhere, tests included.
- **Zero dead TypeScript exports** out of 491 — re-run with all 12 specimen
  files excluded from the *caller* side too, so a mockup-only reference cannot
  mask a dead export.
- **Zero fully-dead Rust struct fields.** All 2 089 `pub` fields are
  *referenced* somewhere in Rust. (The scan counts any textual mention, so a
  field only ever written and never read would pass it. The frontend side is a
  different story — see D3.)
- **Zero unreferenced test fixtures.** All 5 files under `src-tauri/testdata`
  are reached by name.
- **Zero dangling references** to the deleted `ordered_preparation.rs`. The
  surviving mentions are in two `docs/` plans, which are design history per
  `AGENTS.md`.
- **`scripts/*.mjs` is clean.** One keyword hit in the whole directory:
  `tests/release-config.test.mjs:88`, a guard asserting the release package
  contains *no* legacy Codex fork executor. Enforcement, not legacy.

---

## A. Dead code — removable

### A1. `write_target_atomic` — zero callers
`crates/mallard-infrastructure/src/bundle_engine.rs:4730` (15 lines)

A `pub fn` wrapping `write_target_atomic_from_reader` for a byte slice.
Repo-wide grep across `crates/`, `src-tauri/src`, `src-tauri/tests`, `src/`,
`tests/`, and `scripts/` finds **one** occurrence: the definition. Production
uses the reader-shaped function it wraps.

### A2. `write_immutable_backup` — zero callers
`crates/mallard-infrastructure/src/bundle_engine.rs:4760` (12 lines)

Same shape as A1, wrapping `write_immutable_backup_from_reader`. One
occurrence repo-wide. Production uses `write_immutable_backup_from_path`.

A1 and A2 are the only two Rust *items* in the workspace with no textual
reference outside their own definition.

### A3. `note_tombstone` — zero callers
`crates/mallard-infrastructure/src/portable_resources/test_contracts.rs:40`
(10 lines)

The module is live (`PortableResourceRegistry::with_test_contracts`,
`registry.rs:98`, used by `sync_tests/registry_proofs.rs`) and its siblings
`note_kind`, `review_twice_policy`, `imported_provenance` all have callers.
`note_tombstone` alone has none — `TEST_TOMBSTONE_TARGET` is registered as a
codec but no test ever mints an envelope through this helper.

### A4. `CatalogResourceRelation::UnavailableLocally` — never produced
`src-tauri/src/storage_catalog/domain.rs:1834`
`src/components/storage-catalog/types.ts:114` (`"unavailable_locally"`)

`relation_for_state` (`storage_catalog/resource_planning.rs:31–44`) is
exhaustive over all 8 `CatalogLinkState` values and yields the other seven
relations. Nothing anywhere — production, tests, or frontend — constructs or
branches on `UnavailableLocally`. Dead on both sides of the IPC boundary.

This is the only one of 485 `pub` enum variants with zero construction site.

### A5. Dead CSS — 7 selectors in `src/App.css`

Distinguished from the 16 other unmatched selectors that *are* built
dynamically (`` `status-${tool.status}` ``, `` `event-${kind}` ``,
`` `quality-${q}` ``, `` `cat-${category}` ``, `` `tool-${status}` ``,
`` `log-${level}` ``). These seven have no construction site found by
either literal or prefix search across `src/`, `tests/`, and the root
specimens — the instrument covers string literals and template-literal
prefixes, not a name assembled in two pieces or added via `classList.add`:

| Selector | Lines | Shape |
| --- | --- | --- |
| `.storage-link-actions-menu-item` | 3972–4005, 9695 | **4 standalone rules (34 lines)** + one selector in a live group |
| `.btn .sync-btn-name` | 9681–9684 | **standalone rule (4 lines)** |
| `.log-header`, `.log-toolbar` | 11197–11201 | **whole rule dead (5 lines)** — both selectors unused |
| `.log-entry`, `.log-entry:hover` | 11206, 11211 | dead selectors beside live `.log-line` |
| `.sync-project-file-row` | 10874, 10883, 10892 | dead selector in 3 live groups |
| `.sync-pull-item-status.installing` | 1888 | dead selector; neighbouring `.restoring` / `.restored` / `.installed` are live |

`LogPanel.tsx:113` builds `` `log-line log-${line.level}` `` — no component
emits `log-entry`, `log-header`, or `log-toolbar`.

### A6. Unused dependency `url`
`crates/mallard-application/Cargo.toml:13` — neither `url` nor `Url` appears
anywhere in that crate's sources.

### A7. Unused dependency `zeroize`
`src-tauri/Cargo.toml:27` — neither `zeroize` nor `Zeroize` appears anywhere in
`src-tauri/src`. The four crates that actually use it declare it themselves.

A6 and A7 are the only unused entries across all 87 dependency declarations in
the five manifests.

### A8. Tracked scratch probes
`src/zprobe.html`, `src/zprobe2.html` — both **tracked in git** (not untracked,
as the 2026-09-01 UI audit recorded them). Not reachable from `index.html` →
`main.tsx`, so they never ship; but they sit in the source root and are not
ignored. 8.7 KB.

---

## B. The one surviving compatibility tolerance

### B1. `TimelineCacheEntry` stamp defaults
`crates/mallard-infrastructure/src/session_detail.rs:191–194`

```rust
#[serde(default)]
preview_budget_bytes: u64,
#[serde(default)]
conversation_limit_bytes: u64,
```

Its own doc comment names it: *"the serde defaults make pre-stamp entries
(zeros) never match instead of failing the parse."* That is a read-old-format
path — the only one in production code.

Low-consequence: the entry carries its own `schema: u32`, the file is a
rebuildable disk cache (`session_timeline_cache.json`), and a zeroed entry can
never satisfy `entry_matches_limits`, so it degrades to a cache miss rather
than serving stale data. But under "schema 1 everywhere, no legacy support" the
strict form is to drop both attributes and let a pre-stamp entry fail the parse
into the existing miss path — the fields are already version-gated by the
entry's own schema field, which makes the defaults redundant with the gate.

Its test at `session_detail.rs:2493–2505` asserts the tolerance and would need
inverting. Note the contrast with the three other field-stripping tests in the
tree — `mallard-domain/src/tests.rs:3106`, `tests.rs:3427`,
`subagents.rs:289` — which all assert `is_err()`. B1 is the odd one out.

**How I bounded this claim.** Rather than triage all 380 `#[serde(default)]`
attributes, I scanned the nine modules that persist under `~/.mallard` for the
shape B1 has: `#[serde(default)]` on a bare scalar. Six hits, and the other four
are live semantics, not tolerance:

- `persistence.rs:297, 299, 301` — `HostRotationQuarantine`'s three `bool`s.
  The struct derives `Default`; absent-means-false is the sparse encoding of a
  same-version flag set, not an old format.
- `activity_log.rs:254` — `ActivityLogCleanupRequest.delete_all` is an
  **inbound** request field. `SettingsPage.tsx:225` sends `{}` when not
  deleting all, so the default carries the live wire shape.

---

## C. Hygiene

### C1. Two schema checks use a magic `1` instead of the named constant
- `crates/mallard-infrastructure/src/persistence.rs:756` — `file.schema != 1`
- `crates/mallard-infrastructure/src/session_detail.rs:1747` — `cursor.schema != 1`

The other 36 schema comparisons all name their constant. These two are the only
literals; if a schema ever moves off 1 they are the sites that silently keep
passing.

### C2. Stale `.sync-callout` references in comments
`src/App.css:6458`, `src/App.css:6622` — both explain a rule as "compounded
with `.sync-callout`". That class was deleted when the Notice system landed;
there is no `.sync-callout` rule or markup left, so the comments describe a
compounding that does not exist.

---

## D. Judgment calls — real, but not automatic deletions

### D1. `ProjectSyncService`: 7 methods with no production caller

`src-tauri/src/project_sync/service.rs` is documented as the "non-Tauri
application boundary". Production `commands.rs` reaches it at 10 sites and uses
only the progress-carrying variants. These seven have **no** production caller —
every reference is from `src-tauri/tests/` or a `#[cfg(test)]` block:

| Method | Line | What production uses instead |
| --- | --- | --- |
| `PushApproval::recommended` | 140 | commands build approvals from the review directly |
| `prepare_push_review` | 202 | `prepare_push_review_with_phases` (`commands.rs:1496`) |
| `push_reviewed` | 219 | `push_reviewed_with_progress` (`commands.rs:1112`) |
| `plan_bundle_restore` | 262 | `plan_bundle_restore_with_progress` (`commands.rs:1231`) |
| `apply_bundle_restore` | 295 | `apply_bundle_restore_with_progress` (`commands.rs:1334`) |
| `compare_threads` | 324 | `get_project_thread_sync_comparison_with_progress`, called directly by the command at `commands.rs:524` |
| `inspect_bundle` | 397 | no production path at all |

Each is a 5–12 line no-op-progress façade over the free function the command
already calls. Not *speculative* abstraction — E2E test ergonomics, carrying
roughly 150 call sites across `live_codex_e2e.rs`, `live_claude_aimock_e2e.rs`,
`live_codex_aimock_e2e.rs`, and `live_mixed_aimock_e2e.rs`.

**Not a recommendation to delete.** Removing them means rewriting ~150 E2E call
sites to pass `&mut |_, _| {}` — trading a 60-line façade for noise in the tests
that prove the sync protocol. My read is keep, and instead move the seven into
one clearly-labelled `impl` block so the next scan does not re-flag them.

### D2. `MaintenanceActionScope` — the whole field is inert

`crates/mallard-infrastructure/src/providers/traits.rs:104–116`,
`src/types.ts:121, 129`

Two problems stacked:

- The `Profile` variant is constructed **only** by
  `providers/test_provider.rs:106`. Both real providers declare `AfterPull`
  (`providers/codex/mod.rs:183`).
- The frontend **never branches on `scope` at all.** The only read of a
  `MaintenanceActionDescriptor` is `model.ts:220`, which finds by `action_id`.
  `MaintenanceActionScope` is declared, serialized, and mirrored — and nothing
  consumes it.

So today the field distinguishes nothing. But the surrounding doc comment
states the extensibility intent — *"the frontend renders declared actions and
never checks a provider key"* — and this is the provider-extension seam. The
choice is: drop `Profile` (leaving a one-variant enum, which argues for dropping
the field), or make the frontend honour the scope it is already being sent.
Either is defensible; the current middle is not.

### D3. 55 DTO fields mirrored in TypeScript that no component reads

I diffed the 496 DTO field names declared in `src/types.ts` and
`src/components/storage-catalog/types.ts` against every component, model, and
test — specimen files excluded. 55 names appear **zero** times outside their own
declaration: not read, not spread, not set on an outbound request. All are real
Rust-side fields, so each is serialized across IPC on every call and never
looked at.

The largest clusters:

- **`PortableConversationTail`** (`types.ts:409–434`) — 16 of them:
  `partition_kind`, `start_record_count`, `accepted_end_record_count`,
  `accepted_tail_state`, `provider_profile_id`, `materialization_state`,
  `lineage_digest`, `semantic_digest`, `local_file_sha256_after`,
  `cloud_revision`, `cloud_commit_id`, `cloud_manifest_sha256`,
  `published_origin_host_token`, `published_origin_replica_token`,
  `source_disposition`, `materialization_id`.
- **Clear/reset receipts** — `StorageDataClearResult.project_links_reset`,
  `.catalog_links_reset`, `.local_records_removed`;
  `ProjectSyncMetadataResetResult.links_reset`;
  `LocalStateWipeResult.files_removed`, `.bytes_removed`.
- **Never-set request fields** — `ActivityLogCleanupRequest.older_than_days`
  (the UI only ever sends `delete_all`).
- **Singles** — `ProviderProfileSummary.used_by_projects`,
  `ProviderCliLauncher.program`, `MaintenanceActionDescriptor.read_only`,
  `PushReviewContext.prepared_at`, `.project_files_error`,
  `HostIdentityStatus.local_display_name`, `LogLine.run_id`,
  `ActivityLogContext.project_name`, and 20 more.

**What makes a field decidable:** a mirrored field is safely droppable when the
frontend neither reads it *nor* round-trips the containing DTO back to Rust.
`PortableConversationTail` is receive-only — it arrives inside review and
comparison payloads and is never sent back — so its 16 unread fields are the
high-confidence subset. `ActivityLogCleanupRequest.older_than_days` is the
opposite case: an outbound request field the UI never sets, where dropping it
removes a capability rather than dead weight.

**This is a candidate set, not a delete list.** Some are deliberate receipt
completeness (a wipe result that reports what it removed is arguably right even
if no screen shows it); some are UI the design intended and never built
(`used_by_projects` on a profile card); some are genuinely inert. Each needs a
one-line decision: show it, or drop it from both sides. Worth doing as its own
pass — it is the largest single pocket of unused surface in the codebase.

### D4. Twelve mockup components tracked inside `src/`

`catalog-groups-mockup.tsx`, `catalog-setup-mockup.tsx`,
`files-activity-redesign-mockup.tsx`, `files-review-mockup.tsx`,
`files-selection-mockup.tsx`, `no-commit-icons.tsx`, `pathbar-mockup.tsx`,
`plug-diagonal.tsx`, `plugin-icons.tsx`, `push-budget-mockup.tsx`,
`push-limit-mockup.tsx`, `setup-storage-mockup.tsx`.

None is imported by anything. Vite's single `index.html` entry means none is
bundled, so **nothing ships**. But `npm run build` runs `tsc` over the whole
project, so all twelve must keep compiling against `src/types.ts` and the
component APIs they mimic — a real tax whenever a DTO changes, paid by frozen
specimens.

The specimen practice is deliberate and load-bearing (it is how UI directions
get reviewed before touching the app — see the root `*-mockup.html`). The
question is only whether these belong under `src/`, where the typechecker owns
them, or beside the root specimens where it does not.

---

## E. Keep — matched the keywords, is not Mallard legacy

The keyword sweep found 49 compat-flavoured lines in production code (test
bodies and `#[cfg(test)]` blocks excluded). **Every one** falls into a category
below. Recording them so a later pass does not delete them.

**Upstream format tolerance — Codex emits two event families, we don't control it.**
Codex 0.147 writes both legacy display events and paginated `item_completed`
records; reading both is a contract with another program's files, not a Mallard
version shim.
- `codex_paginated_projection.rs:5, 9, 1178, 1287`
- `codex_history_graph.rs:655`
- `codex_subagents.rs:989`
- `providers/codex/history.rs:2106, 2158–2278` (the whole
  `consume_legacy_event` / `consume_legacy_response_item` family)

**User-data format, not our format.**
- `credential_redaction.rs:301` — `"openai-legacy-key"` is the *name of a
  detector rule* matching the old OpenAI key shape in the user's own files.

**A comment that says the opposite of what it greps as.**
- `storage_catalog/codex_pet.rs:8` — "…**not** a compatibility fallback;
  unknown version markers block."

**Non-version senses of the words.**
- `ProjectSync.tsx:1068–1133, 2133, 2231`, `ProjectActivityPage.tsx:2283`,
  `model.ts:15` — "superseded" describes a *request* superseded by a newer one
  (epoch guarding), not a format.
- `project_activity.rs:173, 181`, `ProjectActivityPage.tsx:305` — "step
  backwards" is time navigation in the activity window.

**Shared Rust↔TS vocabulary where one side is the only producer.**
- `NoticeSeverity::Danger` (`mallard-domain/src/lib.rs:3206`) is never
  constructed in Rust — backend notices only ever carry `Info` or `Warning`.
  But `src/types.ts:1255` mirrors the enum for the frontend's own `Notice`
  component, which uses `severity="danger"` in ten places
  (`LogPanel.tsx:351`, `SettingsPage.tsx:297/327/349`,
  `PushResourceWorkspace.tsx:733`, …). Deleting the Rust variant would desync
  the mirror for no gain. **Keep.**

**Guards and negative tests that *enforce* the invariant.** Deleting these
removes the proof that legacy is rejected:
- `persistence.rs:4415` — `schema: 99` fixture; asserts the validator rejects it.
- `app_settings.rs:256` — `schema: 2` fixture; same.
- `session_path_map.rs:684`, `codex_standalone_builder.rs:1751`,
  `service.rs:24631` — `mallard.pathmap.v9` fixtures; assert an unknown codec
  marker is refused.
- `storage_core/dropbox_store/mock.rs:223–224` — `panic!("obsolete … route")`;
  asserts we never call the superseded Dropbox endpoints.
- `crates/mallard-cli/tests/cli.rs:104` — asserts removed help items stay removed.
- `tests/release-config.test.mjs:88` — asserts the release package carries no
  legacy Codex fork executor.
- `tests/frontend/project-v3-cutover.integration.test.tsx` — the
  anti-reintroduction guard `AGENTS.md:41–43` mandates.

**Specimen files** (`src/*-mockup.tsx`, root `*.html`) mention "cutover" because
they are *before/after* comparisons of shipped cutovers. Not code.

---

## Suggested order if you act on this

1. **A1–A4, A6–A8** — mechanical deletions, no product decision, compiler- and
   grep-verified. ~50 lines of Rust, one enum variant plus its TS union member,
   two manifest lines, two files.
2. **A5** — CSS; needs a computed-style re-run afterwards, since `src/App.css`
   has four cascade gates that assert *source text* and would not catch a
   regression.
3. **C1, C2** — two-line hygiene.
4. **B1** — the only real behaviour change: drop the two `#[serde(default)]`
   attributes and invert `session_detail.rs:2493–2505` to assert rejection.
   Requires wiping `~/.mallard/session_timeline_cache.json` before retesting.
5. **D3** — its own pass; the largest pocket, and every field is a decision.
6. **D1, D2, D4** — decide, don't default.

Full bar for any of the above:
`cargo test --workspace --all-targets`, `cargo clippy --all-targets`,
`tsc --noEmit`, `npm run test:frontend-integration`.

## Housekeeping, not findings

- `poc/` (27 tracked files) and `formal/` (29) are research artifacts outside
  the build. `poc/qoder-custom-provider/qoder-custom-provider.test.mjs` is a
  test no `npm` script invokes.
- `dist/` (1.0 MB), `output/` (33 MB), `target/` (5.3 GB) are untracked build
  and run products.
- `.sync-sync-selectable-row` (`App.css:10893`) is **live** — production
  `ProjectActivityPage.tsx:1223, 1423` emits it — but the doubled `sync-sync-`
  prefix reads as a typo and will keep drawing the eye in future scans.

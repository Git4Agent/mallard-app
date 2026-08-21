# Mallard backend over-engineering, redundancy, and dead-code audit

- **Audit date:** 2026-08-21
- **Baseline:** `e0d03b6` plus the current staged, unstaged, and untracked
  working-tree changes
- **Scope:** all Rust in the workspace — `mallard-domain`,
  `mallard-infrastructure`, `mallard-application`, `mallard-cli`, and the
  `src-tauri` shell (`project_sync/`, `storage_catalog/`), ~132k lines
  including tests
- **Method:** nine independent auditors, one per subsystem plus a mechanical
  `cargo`/grep pass, each followed by an adversarial verifier instructed to
  refute the findings against the code. 77 candidate findings were raised;
  **72 survived verification, 5 were refuted** and are listed in
  [Refuted claims](#refuted-claims) so they are not re-raised later.
- **Purpose:** find abstraction that costs more than it buys, checks that
  cannot fire, and code nothing reaches. This is **not** a recommendation to
  weaken restore, filesystem, or publication safety — see
  [Complexity that is earned](#complexity-that-is-earned) first.

Companion to the [backend-core simplification audit](./BACKEND_CORE_SIMPLIFICATION_AUDIT_2026-08-19.md),
which covered data-flow and locking. This one covers surface area.

---

## Executive assessment

The backend is well-built and, by the compiler's own reckoning, spotless:
`cargo check --workspace --all-targets` and `cargo clippy --workspace
--all-targets` both exit **0 with zero warnings**. There are no `todo!()`, no
`unimplemented!()`, and no commented-out code blocks anywhere in the tree. All
six production `unreachable!()` sites are exhaustive-match tails carrying
justifying messages. Comment discipline is genuinely good — comments explain
intent rather than parking old code.

That clean bill of health is also why this audit was needed. Every finding
below lives in one of the two places rustc's dead-code lint structurally cannot
see:

1. **`pub` items in library crates**, where reachability from the crate root
   makes everything "live" by definition.
2. **`#[tauri::command]` functions**, whose only real caller is TypeScript.

**One pattern accounts for the large majority of findings: a cutover landed,
the new path took over, and the old path was left `pub`** — or left in a
manifest, or left registered as an IPC command, or left as an unread struct
field. The project's own stated rule is that feature changes are clean
cutovers with no dead code left behind; these are the places where the second
half of that rule was not applied.

Headline numbers:

| | |
|---|---|
| Confirmed findings | **72** (5 refuted) |
| Registered Tauri commands with **no possible caller** | **19 of 69** in project-sync, plus 1 in storage-catalog |
| Production `pub` items with **zero references repo-wide** | 4 |
| Production symbols whose **only callers are tests** | ~15 |
| Struct fields written on every run and **never read** | ~20 |
| Enum variants that **cannot be constructed** | 8 across 5 enums |
| Cargo dependencies declared but **never referenced** | 4 (`rusqlite`, `toml`, `notify`, `hmac`) |
| Estimated removable production Rust | **~2,400 LOC**, plus the tests that go with it |

Three findings were raised as latent correctness defects caused by the
redundancy. **Verified post-fix: one of the three (D1) was not a defect** — see
the correction in its section. D2 is a narrow race, and D3 remains a genuine
leniency divergence. The lesson is worth keeping: "computed twice" reliably
finds redundancy, but whether the two computations *reach* a user-visible
outcome needs a non-vacuous test, not a reading. All three sections below now
state which is which.

---

## Correctness defects found while auditing redundancy

These were surfaced by asking "why is this computed twice?" and finding that
the two computations do not agree.

### D1 — The redaction blocker override is re-derived away on the plan path

> **Correction, 2026-08-21 (post-fix).** This was reported as a live defect —
> "the override never reaches the plan". That is **wrong**, and the fix commit
> proves it. A regression test built on two non-vacuous controls (a
> `LocalAhead` local extension that does publish while the stamps agree, and a
> `StorageOnly` replica that does adopt) shows planning already emits **no**
> action for a foreign-stamped row *before* the change. The override reaches
> planning through `ConversationDecision::selectable`: `Blocked` is never
> selectable, and push candidate selection filters on
> `push_decision.selectable`, so the row is never captured and classifies out
> of a planning action. Two further apply-stage stamp checks
> (`commands.rs` push and pull `redaction_ruleset_mismatch` gates) are hard
> `return Err` fail-closed boundaries underneath that.
>
> What remains true is the code-level observation: the gate asked
> `classified.planned_action(dir)`, re-deriving from raw inputs rather than
> reading the decision the override had just produced. That is redundant
> recomputation of exactly the kind this audit is about — so the two-line
> change still landed, as a **cleanup, not a bug fix**. Severity drops from
> "correctness defect" to P2 redundancy. The rest of this section describes
> the code shape accurately; only the claimed consequence was wrong.

**`crates/mallard-domain/src/lib.rs:3436`, `:3533`;
`src-tauri/src/project_sync/commands.rs:6495-6506`, `:6677`, `:6845`**

`commands.rs:6495-6505` rebinds `push`/`pull` to
`blocked_conversation_decision(ConversationBlocker::RedactionRulesetMismatch)`
— this is the redaction gate, the entire reason `blocked_conversation_decision`
exists. But `commands.rs:6677` and `:6845` then gate action construction on
`classified.planned_action(direction)`, which **re-derives the decision from
the raw inputs** and therefore returns the *pre-gate* action. The override
never reaches the plan.

The root cause is that the tail relation is re-classified 4–6 times per
conversation action rather than computed once and threaded through.
`ConversationTailPins::classification()` clones four tails, re-runs
`PortableConversationTail::validate` on each, and re-runs
`compare_conversation_tail` — and `lib.rs:3436` and `lib.rs:3533` do exactly
this **two statements apart in the same function body**.

The domain's own module doc says `compare_conversation_tail` is "the only place
in the workspace that compares accepted/local/cloud tail values" and that
constructing `VerifiedTailEvidence` "is the proof boundary". In practice that
boundary is crossed 4–6 times per action, and because `ClassifiedThread` is
`Copy`, each crossing recomputes from scratch. This is precisely the class of
divergence the single-truth-table design was built to prevent.

**Fix:** compute the decision once per `(thread, direction)` and pass the
resulting `ConversationDecision` through. Replace `commands.rs:6677`/`:6845`
`classified.planned_action(dir)` with `push.action.planned_action()` /
`pull.action.planned_action()`. Delete the duplicate `self.classification()?`
at `lib.rs:3533`. The load-bearing part of the fix is **two lines**.

### D2 — Plugin verification reports on a different snapshot than it applies

**`src-tauri/src/storage_catalog/apply.rs:897`, `:960`, `:964`**

`execute_plugin_action` calls `adapter.discover_plugins` (pre-launch recheck),
then `adapter.verify_plugin_apply`, then `adapter.discover_plugins` **again**
— and both `verify_plugin_apply` impls open with
`let inventory = self.discover_plugins(home, runner)?`, so that middle call is
itself a full discovery. Nothing mutates provider state between `:960` and
`:964`; the CLI run loop ends at `:955`.

The hazard: `verification.observation` comes from the inventory taken *inside*
`verify_plugin_apply`, but it is passed to
`adapter.portable_intent(observation, &fresh)` at `:967-970` against the
**later** `fresh` inventory. Marketplace ownership roots resolved from one
snapshot are applied to an observation from another. If the two disagree, the
action is reported `Partial` for no real reason.

Cost, separately: six subprocess spawns per Codex plugin install where four
suffice (`plugin marketplace list --json` and `plugin list --json`, each with a
300s timeout, per discovery round).

**Fix (✅ landed 2026-08-21):** rather than thread the inventory out of
`verify_plugin_apply`, the method was **deleted** — both impls were byte-identical
except a `"codex"`/`"claude"` literal, and the single call site now takes one
post-exit `discover_plugins` and derives presence, intent, and both failure
messages from that one snapshot. `PluginVerification` went with it. This also
resolves F11. Note the snapshot-mismatch hazard is a **narrow race** — it needs a
concurrent provider write between the two discoveries — so the durable
justification here is the removed duplication and the removed subprocess round,
not the race.

### D3 — Two RFC 3339 parsers with different leniency decide the same timestamps

**`crates/mallard-infrastructure/src/providers/timeparse.rs:25-88`**

`timeparse.rs` hand-parses RFC 3339 and implements Howard Hinnant's
`days_from_civil` — 64 lines of calendar arithmetic. Meanwhile `chrono = "0.4"`
is a **direct dependency of the same crate** and is already used for this exact
parse two modules away: `providers/codex/materialization.rs:81` and
`codex_paginated_projection.rs:931-941`.

The two parsers disagree: the hand-rolled one accepts impossible dates (day 31
in February silently rolls forward through `days_from_civil`) while the chrono
path rejects them. The same rollout timestamp can therefore be accepted by the
projection and rejected by the index, or vice versa.

**Fix:** replace the body with
`chrono::DateTime::parse_from_rfc3339(...)`, delete `days_from_civil`, and make
`parse_rfc3339` private (`parse_timestamp_value` is its only caller). Keep the
numeric-epoch seconds-vs-milliseconds heuristic at `:5-23` — chrono does not
cover that. Confirm first that no corpus record depends on the current
tolerance; `scripts/claude_corpus_scan_*.py` plus the history tests will show
it.

---

## Findings register

Priority is recommended remediation order. **P1** = high value or defect,
**P2** = clear win, **P3** = worth doing in passing.

### A. Dead IPC surface

| ID | Pri | Finding | Size |
|---|---|---|---|
| A1 | ✅ | **19 of 69 project-sync Tauri commands are unreachable from the UI.** 7 have no `api.ts` wrapper at all; 12 have a wrapper with zero call sites. `registration_freeze.rs` pins the names, so CI actively defends them. | 19 command bodies + ~230 LOC helpers/DTOs + ~57 LOC in `bundle_engine.rs` |
| A2 | ✅ | `BundleEngine::verify_objects` + `BundleObjectAudit` (`bundle_engine.rs:1055-1101`, `:72-80`) — a full download-and-rehash durability audit, reachable only from the dead `verify_bundle_objects` command, with **no test and no TS type**. | ~57 LOC |
| A3 | ✅ | `delete_catalog_resource_profile` (`resource_commands.rs:477-538`) is registered and frozen, and `tests/frontend/storage-catalog.integration.test.tsx:246` **asserts the UI must not reference it**. | 61 LOC Rust + 2 TS |

> **Correction (2026-08-21, on landing).** The count is **20, not 19**:
> `get_bundle_readiness` has no wrapper either and belongs with the other
> seven. The reverse-direction check below held exactly as stated.

A1 was found independently by two auditors (the subsystem pass and the
mechanical pass), which agreed exactly on the 7 no-wrapper commands. The
reverse direction is clean: all 87 distinct `invoke("...")` names in the
frontend resolve to a real command — **zero orphan invokes**.

The 7 with no wrapper at all: `discard_restore_plan`, `get_bundle_status`,
`get_local_project`, `list_project_materializations`, `remove_project_binding`,
`save_bundle_recipe`, `verify_bundle_objects`.

The 12 with a dead wrapper: `findRemoteBundleMatches`, `getBinding`,
`getProjectOriginTokens`, `listOriginHostAliases`, `listRemoteBundles`,
`registerProject`, `removeProviderProfile`, `renameProviderProfile`,
`resolveOriginHostLabel`, `rotateHostIdentity`, `saveOriginHostAlias`,
`updateHostDisplayName`.

> Note: `registration_freeze.rs:10`'s doc comment says 95 names; the array
> holds 94 and `lib.rs` registers 94. Stale comment.

### B. Post-cutover leftovers — dead `pub` items and write-only state

| ID | Pri | Finding | Size |
|---|---|---|---|
| B1 | P1 | `conversation_transcript_path` / `conversation_sidecar_path` (`domain/lib.rs:3926`, `:3948`) have **zero references repo-wide**. Superseded by the `cloud_*` twins in the cloud-thread-id decoupling. Anyone reaching for the obviously-named function would write local thread ids into cloud object keys. | ~26 LOC |
| B2 | P2 | `validate_session_ownership` (`project_activity.rs:1405-1453`) — a 49-line `pub` fn with **zero callers anywhere**, documented as a "deep-link guard". It runs a full `adapter.discover()` provider walk. A reader auditing deep-link handling finds a guard that never runs. | ~49 LOC |
| B3 | P2 | The **ETag threaded through `CasOutcome` and `VerifiedBody` is never read** — and it costs a full extra object GET on every S3 CAS conflict (`s3_store.rs:745-752` calls `stage_verified_async` purely to extract it), plus a second round trip when the service omits an ETag, plus a redundant `sha256` per local CAS write. Both engines re-derive their CAS token from `get_metadata` instead. | ~50-70 LOC |
| B4 | P2 | The whole `ProviderCapabilityLevel` ladder is write-only: `capability()` has **zero call sites**, `ProviderDescriptor.capabilities` is never read, and the frontend never reads it either. `BackupOnly` is constructed nowhere. Corroborated by `docs/PLAN_CLAUDE_SESSION_CURSOR_PARITY.md:155`. | ~40-45 LOC |
| B5 | P2 | `ProviderSessionInventory` (`provider_capture.rs:217-229`) is populated on every Codex discovery walk, merged in `Discovery::combine`, and read by **one unit test**. Claude never populates it and nothing notices. | ~40 LOC |
| B6 | P2 | `SessionDetailCapabilities` is computed and shipped over IPC on every session-detail load, read only by integration tests — and two of its four fields are the **same expression** (`session_detail.rs:1592-1593`). | ~25 LOC + 6-line TS |
| B7 | P2 | `ApplyBundleResult.backups` / `BackupRecord` is threaded through the two largest private apply functions as a `&mut Vec<..>` accumulator and **dropped by the only caller** (`commands.rs:12525` reads only `.receipts`). The backup *files* are still written, so nothing is weakened. | ~30 LOC |
| B8 | P2 | `get_catalog_resource_status` reads and fully validates **every plan file on disk** (`list_resource_plans` → `read_json_bounded` + `plan.validate()` per file) to fill `active_plan_ids`, which no reader consumes. Its `link_id` filter parameter is unreachable from the shipped UI. | ~30 LOC + one full plan-dir scan per detail-pane open |
| B9 | P2 | `StorageCatalogService::inspect_repository` + `CatalogRepositoryInspectionDto` exist for **one integration assertion** and are re-exported from `lib.rs`. `user_home()` has zero callers at all. It duplicates the cross-validation `verified_remote_choices` already does. | ~55 LOC + 3 |
| B10 | P2 | Four never-read fields and two never-constructed variants in the catalog domain: `CatalogOperationPhase::Activated`, `CatalogResourceRelation::UnavailableLocally`, `CatalogPreservationPin::reusable_witness` (literal `false`, apply decides from the witness document instead), `PluginObservation::observed_version`, `PluginObservation::enabled`. | ~25 LOC + 2 TS |
| B11 | P2 | **Seven production symbols reachable only from tests**, three carrying `#[cfg_attr(not(test), allow(dead_code))]` — i.e. already known to be dead and suppressed rather than resolved. `capture_recipe` is the loudest: its doc calls it "the sole selection authority" while every production caller uses `capture_recipe_from_discovery`, so **tests exercise a different composition than production runs**. | ~60 LOC + 3 suppressions |
| B12 | P3 | `CapturedWith.provider_versions` — an always-empty `BTreeMap` serialized into every scope of every published manifest, sitting next to `codec_versions`, which is genuinely load-bearing. | ~8 LOC |
| B13 | P3 | `ActivityLogContext.profile_id` / `.resource_id` / `.revision` are never assigned, and `LogPanel.tsx` never reads `context` at all. `sanitized()` sanitizes a field that is structurally always `None`. | ~12 LOC |
| B14 | P3 | `ResolvedSegment.canonical_messages` (`codex_history_graph.rs:42`) — computed per segment, four occurrences in the whole repo, none of them a read. | 4 LOC |
| B15 | P3 | `ConfigTransactionError::into_persistence` (`persistence.rs:96-104`) — definition is its only occurrence. | ~9 LOC |
| B16 | P3 | `R2SetupJournalStore::directory()` — only callers are tests, and it hands out the raw journal path that `sanitize_local_state_error` works to keep out of error strings. | 3 LOC |
| B17 | P3 | `merge_activity_sessions` — a `pub` wrapper whose only job is re-loading state the real caller already has; production calls the inner `_with_state` fn. | ~22 LOC |
| B18 | P3 | `ProvisionR2StorageRequest::expected_config_revision` is hardcoded `None` at its single production call site, making the `config_revision_conflict` branch unreachable. A load-bearing-looking optimistic-concurrency knob that is inert. | ~7 LOC |
| B19 | P3 | `CleanupR2SetupResult.cleaned` is `true` at both `Ok` returns and has no `false` producer anywhere, leaving `output.rs:204-208` ("still requires cleanup") **unprintable**. That state is actually signalled by exit code 6. | ~12 LOC |
| B20 | P3 | `verify_idempotent_provision` takes `_repository: &LocalStateRepository` it never uses — one of the 9 params that forced `#[allow(clippy::too_many_arguments)]`. | 1 param |
| B21 | P3 | `tail_record_count` (zero production callers), `strip_preserved_marker` and `matches_relation` (`pub` with only in-crate callers). | 1 fn + 2 visibility reductions |
| B22 | ✅ | `ThreadSyncEntry.origin_label` costs a **cross-process lock + full conversation-bindings document parse per thread** in `provider_thread_review` — which runs on every Push review, Pull plan, and Activity load — and no component renders it. It also props up the entire origin-host-alias slice, whose four commands are all in the A1 dead list. | 18 LOC in the hot loop; unblocks a whole vertical slice |

### C. Vocabulary modelling distinctions nobody makes

| ID | Pri | Finding | Size |
|---|---|---|---|
| C1 | P2 | **`BundleKind` is a single-variant enum** (`Project`) carried on `BundleIdentity`, `BundleHead`, `BundleTag`, `RemoteBundleSummary` and two DTOs — i.e. serialized into every head, tag, and listing — with a **provably unreachable guard** at `bundle_engine.rs:538` and a second dead comparison at `domain/lib.rs:4933`. Only 1 of its 8 construction sites is production. | ~25 LOC (wire-schema change) |
| C2 | P2 | `ApplyPolicy` has 5 variants; the producer enum `CaptureApplyPolicy` has 3, and both converters are total 3-arm matches. **`ManualOnly` and `Never` cannot be constructed** — yet the frontend guards against `"never"` in three places, and `lib.rs:985`'s `!= ApplyPolicy::Never` recipe guard reads as a live safety rule. | 2 variants + 2 Rust branches + 3 TS conditions |
| C3 | P2 | Two more single-variant enums and a write-only newtype: `ConversationSourceDisposition` (`Retained`, no reader), and `ResourceTypeKey` — 32 lines of parse/`TryFrom`/`From`/`Display` boilerplate for `provider_resource_type`, whose **own doc comment says "never branch off this string"** and which nothing reads. | ~10 + 32 LOC |
| C4 | P2 | `mallard-cli/src/backend.rs` re-declares **8 application-layer request/result types field-for-field** to satisfy a 5-method trait with one real impl, plus ~85 lines of pure re-mapping. The `StorageKind → &'static str` squash then forces an unreachable `_ => "Storage"` arm in `output.rs`. The test-seam justification does not require the DTO mirror — and `lib.rs:9` already imports application types directly. | ~70-80 LOC |
| C5 | P3 | `R2CapabilityProbeStage` — a 7-variant enum threaded through 10 construction sites, with a `Display` impl that is **never invoked**, because both production consumers match on `error.kind()` only. It reads as a diagnostic channel with nothing listening. | ~45 LOC |
| C6 | P3 | `TailEvidenceBlocker` has 7 typed variants; the doc says "failed proofs are data, not caller-controlled Booleans", but the single production consumer does `evidence_blocker.is_some()` and emits **one generic sentence for all seven**. Either surface the discriminant at `commands.rs:6605` (the reason to have typed blockers) or collapse it. | ~9 LOC |
| C7 | P3 | `SyncCurrency::Advanced` and `ConversationReviewAction::RequirePullMetadata` are unreachable: `Advanced` needs `local == cloud` including codec, but both providers use disjoint local/wire codec labels, and the only path that could produce equality requires `supports_tail_cursor() == false`, which only the test provider returns. **Note:** the cursor-less seam is documented design, so this is a "decide whether it ships" item, not an obvious delete. | ~6 match arms + 1 enum |
| C8 | P3 | `SessionAdapter::supports_tail_cursor()` defaults to `false`; both production adapters override to `true` and only the `#[cfg]`-gated test provider inherits the default. The false arm of ~10 branch sites in `commands.rs` is unreachable in production. Same "decide whether it ships" caveat as C7. | ~120 LOC of test-only arms |

### D. Speculative plumbing for a v2 that never came

| ID | Pri | Finding | Size |
|---|---|---|---|
| D1 | P2 | **~370 LOC of Codex app-server subprocess machinery to obtain a UUIDv7 the local fallback generates in 12 lines.** `CliAllocator::mint` spawns `codex app-server --stdio` against three `TempDir`s, scrubs a 60-key env denylist, runs a JSON-RPC handshake with byte/message/line/timeout bounds, and reads `/result/thread/id` — which then goes through the **same `validate_uuid_v7` and the same collision check** as the local generator. The thread is `ephemeral` and the `CODEX_HOME` is a `TempDir` dropped at return, so nothing is registered anywhere. ⚠️ CLI-preferred order is an explicit written decision in `DESIGN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER.md` — confirm with the author before removing. | ~370 LOC prod + ~283 LOC tests |
| D2 | P2 | **Timeline anchor pagination, turn filter, and tool-category filter are plumbed through 6 files with no production caller.** A repo-wide grep for `anchor_event_id` returns 31 hits and **not one constructs `Some(...)`** — including every test and all four live e2e suites. `session-detail.integration.test.tsx:956` actively asserts the page source contains no anchor reference: the mode was deliberately cut from the UI. The backend still validates it, fingerprints it into the opaque cursor, and re-implements the anchor→index scan **per adapter**. | ~40 production LOC |
| D3 | P2 | `CatalogPushRow` carries 15 review fields persisted into every plan file and cross-validated on every load; the **frontend reads 2** (`selectable`, `blocked_reason`). `removal_candidate` is written as literal `false` with zero readers anywhere; `selected_by_default` and `held_for_pull` are pure restatements of `selectable` / `blocked_reason.is_some()` read only by integration tests. | ~18-20 LOC + TS mirror |
| D4 | P3 | Standalone artifact subagent transport is **always an empty `Vec`** — struct, field, construction, and the `artifact_digest` loop that can therefore never iterate. The field's own doc says "Empty in codec v1". Carrying a provably empty file list through a wire-identity digest invites the belief that multi-file artifacts are supported and tested. | ~28 LOC |

### E. Repeated work on hot paths

| ID | Pri | Finding | Size |
|---|---|---|---|
| E1 | P1 | **Every captured payload is SHA-256'd twice per publish attempt** — `bundle_engine.rs:650` stores the digest, then `:681` recomputes it as the `pending_uploads` key over the same `input.captured_files`. Both loops are inside `for attempt in 0..MAX_CAS_ATTEMPTS` (=4), so a contended Push hashes each payload **up to 8 times**. Payloads are bounded by `MAX_CONVERSATION_FILE_BYTES = 100 MB`. Line 682 additionally does a full `Vec<u8>` heap copy inside the same retry loop. Fix is ~5 lines. | ~5 LOC changed |
| E2 | P2 | **`BundleScope::validate` re-serializes and re-SHA256s the whole scope**, and the same immutable snapshot is validated **2–3 times per Pull** (`bundle_engine.rs:1537`, `:1200`, `:4086`). A manifest can hold 20,000 conversations plus 100,000 file entries per scope. `BundleSnapshot` has no interior mutability, so passes 2 and 3 cannot observe anything pass 1 did not. | 2 redundant full-manifest hashes per Pull |
| E3 | P2 | `resolve_project_binding` **re-loads the machine-state document its caller just loaded** — 6 load-then-resolve pairs across push/pull/compare/session-detail. `load_bindings` takes a cross-process lock and re-parses the host-identity, sync-config, *and* machine-state documents each time. Also a correctness smell: the two reads are not atomic, so profiles can come from a different revision than the binding. | 6 duplicated locked loads; ~10 LOC signature change |
| E4 | P2 | **Every scope-page load fetches each repository's `_head.json` twice.** `list_resources` already returns the head; the loop then calls `inspect`, which re-GETs the same key, and compares the two — a TOCTOU check across two reads microseconds apart that reports "has inconsistent discovery metadata". On S3 this is N extra network GETs per refresh. | ~5 LOC; one GET per repo per refresh |
| E5 | P2 | **Codex `build_timeline_index` runs two full `TimelineBuilder`s over every rollout and discards one.** Modern rollouts contain both families, so both accumulate full event vectors, `tool_positions` maps, and `visible_ids` sets up to `MAX_TIMELINE_EVENTS = 50,000` — then `if canonical.saw_family` throws one away. Separately, six named no-op match arms at `:2049-2054` are immediately followed by `_ => {}`, so the "allowlist" enforces nothing. | ~1 builder's work per rollout; 6 dead arms |
| E6 | P2 | `verify_storage` loads the config, validates the whole document, locates the storage, and derives its destination — **twice per call**, in two independent copies of the not-found/not-r2 error contract. `validate_r2_storage_destinations` itself re-parses every S3 entry's URL. The outer copy, which users actually hit, has **no test**. | ~20 LOC |
| E7 | P3 | Every S3 upload computes SHA-256 of the payload twice (`s3_store.rs:697` and `:654`); the first is consumed **only on failure paths**. `put_many` fans this over every object in a publish. Fix: move one line into the two arms that use it. | 1 line moved |
| E8 | P3 | `connect_r2_storage` validates the same request and re-parses the same URL twice in one call. The strict R2 URL parser is the destination-aliasing security boundary — running it twice obscures which site is authoritative. (Partial: the inner copy *is* load-bearing for the provision path, which arrives with a Cloudflare-returned secret nothing else validates.) | ~4 LOC |
| E9 | P3 | CAS ETag expectation validated twice on the same `&str` in the same call path (`s3_store.rs:1078-1080` then `:665`). The outer is the removable copy — `conditional_put_async` is the only path to the wire and fails closed before building the request. | 3 LOC |
| E10 | P3 | `validate_setup_paths`'s `is_empty()` guard is unreachable: its only producer, `resolve_draft_profiles`, guards the same condition with the same message and unconditionally inserts per iteration. Separately, `RegisterNewHostRequest` duplicates `UpdateHostDisplayNameRequest` field-for-field. | ~4 LOC + 7-line struct |

### F. Duplication

| ID | Pri | Finding | Size |
|---|---|---|---|
| F1 | P1 | **`claude/history.rs` and `codex/history.rs` carry ~400 lines of byte-identical timeline machinery.** Nine helper pairs verified with `diff`: `read_source_value`, `matching_entry`, `normalized_type`+`source_span`, and `safe_project_relative_path` diff to **zero output**; `BoundedLine`/`read_bounded_line`, `looks_like_binary_or_base64`+`bounded_text` differ only in a const-vs-literal; `read_timeline_page` (~96 lines) differs in 4 hunks, all cosmetic. These helpers operate on `SessionTimelineIndexEntry` and `SessionSourceSpan`, not provider JSON — they are provider-agnostic by construction. Every timeline-contract change must currently be made twice, in two 2000+/2900+ line files, with no compiler help if only one is updated. | ~399 dup LOC; ~250-320 net removal |
| F2 | P2 | `BundleId` is a **hand-written copy of the `fixed_hex_id!` macro defined 140 lines above it** in the same file — same serde attrs, same alphabet check, byte-identical error string. The macro is already instantiated 3 times, and `HostId::generate` is exactly the separate-impl pattern `BundleId::generate` would need. | ~50 LOC → 1 macro line + 7 |
| F3 | P2 | Three byte-similar copies of the "scan the config for S3 storages at this destination" loop in `cloudflare_r2_setup.rs` (`:589`, `:915`, `:943`), plus a fourth variant in `storage_setup.rs:825-845`. Four places must agree on what "same destination" and "malformed stored endpoint" mean; one provision run executes the scan at least three times. | ~40 LOC |
| F4 | P2 | The **token-revocation reconciliation loop is written out twice** (`cloudflare_r2_setup.rs:541-562` and `:709-730`), identical modulo receiver spelling. This is the crash-recovery code that decides whether a live Cloudflare token gets revoked — two independent copies of the "skip tokens already adopted by the config" guard is exactly where a leak-or-over-revoke bug hides. | ~22 LOC |
| F5 | P2 | `path_text_boundary` implements the same UTF-8-safe scan-and-replace loop **three times** (`:57-99`, `:126-164`, `:171-215`), differing only in the accept predicate and a three-line emit tweak. In a redaction/remap module an off-by-one is a privacy bug, and the three copies are already subtly out of step — only one post-processes the replacement. | ~127 LOC → ~50 |
| F6 | P3 | **Nine or ten hand-rolled SHA-256-to-hex helpers across `mallard-infrastructure`**, in four different spellings (`sha256`, `sha256_bytes`, `hex_sha256`, `hex_digest`), so a reader cannot tell whether two digests in different modules are the same function. Several use per-byte `format!`, allocating 32 Strings per digest. | ~35-40 LOC → 1 |
| F7 | P3 | **Seven more copies of the same helpers inside `storage_catalog`** (`domain.rs:41`, `resource_commands.rs:118`, `resource_engine.rs:30`, `skill_tree.rs:43`/`:52`, `codex_pet.rs:54`/`:63`) — byte-identical bodies, in a module whose entire identity model is content digests. | ~48 LOC |
| F8 | P3 | `s3_store.rs` keeps private copies of `hex_lower`, `sha256`, and **`MAX_LIST_PAGE`** that already exist in the parent module — while `local_store.rs` correctly imports them from `super`. The two stores therefore validate list limits against two independently-declared constants; raising the cap in `mod.rs` would silently break the local/S3 parity the sync_tests suite asserts. | ~12 LOC |
| F9 | P3 | `reset_project_sync_metadata` and `clear_storage_data` duplicate their reset logic — 23 lines verified **byte-identical by md5**, plus link-clearing loops differing only in the filter predicate. The two already diverge in write ordering with no comment saying whether that is deliberate. | ~40-45 LOC |
| F10 | P3 | `storage_catalog/persistence.rs` repeats one load/mutate document pair **six times verbatim** (12 methods, ~200 LOC), differing only in document type, path accessor, size cap, and label. A fix to the locking or revision-bump discipline must land six times. (Note: `mutate_operations` additionally prunes terminal journal entries, so it keeps a body after any refactor.) | ~100 LOC net |
| F11 | ✅ | `verify_plugin_apply` is one function duplicated verbatim across both catalog adapters, differing only in a `"codex"`/`"claude"` literal that `ProviderKey::as_str()` already yields. Fully subsumed by the D2 fix. **Landed 2026-08-21.** | ~50 LOC |
| F12 | P3 | `validate_storage_name` is a character-for-character copy of `validate_name` in a sibling module, and **both run on the same value** in the provision path. | ~14 LOC |

### G. Manifest and build hygiene

| ID | Pri | Finding | Size |
|---|---|---|---|
| G1 | P2 | **`rusqlite` (with `features = ["bundled"]`) and `toml` are declared in two packages each and referenced by zero lines of Rust anywhere.** `cargo tree -i rusqlite` confirms both are really in the graph, and `Cargo.lock` shows `libsqlite3-sys` pulled in. This compiles the **entire SQLite C amalgamation from source** on every clean build and every toolchain/target change, for code that does not exist — and drags a C toolchain requirement and its supply-chain surface into a desktop app. `git log -S` dates both to specific cutover commits. Reading the manifest, any maintainer concludes the app has a SQLite store. It does not. | 4 manifest lines |
| G2 | P2 | The `test-support` feature on `mallard-domain` **gates nothing** — no `cfg(feature = ...)` exists anywhere in the crate — yet two crates still opt in from `[dev-dependencies]`. Its comment cites `classify_conversation_relation`, a function that no longer exists. (`mallard-infrastructure`'s own `test-support` is real and gates 7-9 sites — leave it alone.) | 1 feature + 2 opt-ins |
| G3 | P3 | `src-tauri/Cargo.toml` redeclares `aws-sdk-s3`, `aws-smithy-http-client`, `plist`, and `windows-sys` with **zero references in the package** (`build.rs` is 3 lines), and `aws-smithy-types` is used at exactly one test-only site. The duplicated `aws-smithy-http-client = { features = ["rustls-aws-lc"] }` reads like a deliberate TLS-backend choice at the shell layer when infrastructure already declares the identical set. No crates leave the build graph — this is a clarity fix. | 5 manifest lines |
| G4 | P3 | `mallard-infrastructure` declares `notify` and `hmac` and references neither. Every real `notify` use is in `src-tauri` (`config_watcher.rs`, `session_watch_service.rs`); `hmac` lives in `mallard-domain` and `src-tauri`. This makes a future "move the watcher down into infrastructure" refactor look half-done when it has not been started. | 2 manifest lines |

### H. Test scaffolding compiled into production

| ID | Pri | Finding | Size |
|---|---|---|---|
| H1 | P3 | `arm_backend` selects the watcher backend with **`if cfg!(test)` — a runtime boolean, not a conditional-compilation attribute** — so `ManualScanner` and both `WatchBackend::Manual` match arms ship in the release binary. `ManualScanner::scan` does a full recursive directory walk reading every file's bytes, and can never execute in production. Line 266 is its only construction site, inside that branch. | ~118 LOC |
| H2 | P3 | `sync_metrics`' per-root attribution registry has **zero production `register_root` callers** (all 12 verified test-only), so `roots()` is permanently empty in a shipped build — yet `record()` unconditionally takes a global `RwLock` read and iterates the empty Vec, and `record()` is the body of all 11 `record_*` entry points, which fire **per directory in the capture walk** and **per hash window**. The module whose purpose is proving "a warm no-change refresh reads no bytes" adds a lock acquisition to the very loops it measures. | ~45 LOC behind a `cfg` |

### Structural note (not a deletion)

`src-tauri/src/project_sync/commands.rs` is **16,851 lines of production code**
in one module (`mod tests` starts at 16,852; the file totals 26,902). The
Tauri command bodies are almost all 5–20 line forwarders; the bulk is domain
orchestration with no reason to live in a command module. The largest single
function is `provider_thread_review` at **1,393 lines**, followed by
`apply_prepared_pull_conversation` (927) and `build_publish_request` (895).

Eleven independent subsystems share one namespace of ~227 free functions, and
they already occupy contiguous line ranges, so the split is mechanical. This is
**already scoped** by `docs/PLAN_PROJECT_STRUCTURE_REORGANIZATION.md:306`; it
removes zero lines and is listed here only because it is the single largest
obstacle to reviewing anything else in this register.

---

## Complexity that is earned

The auditors were asked to say explicitly what should **not** be simplified.
Do not use this report as license to touch the following.

**Safety and proof machinery**
- The schema-version equality guards. Every constant is now `1` and the
  schema-4/schema-6 dual-bundle machinery is already gone; the remaining
  `!= X → Err` guards are the deliberate fail-closed policy in
  `PLAN_REMOVE_BACKWARD_COMPAT.md`, pinned by a test.
- `ConversationFailpoint` / `ConversationFailpoints` — a 25-variant
  crash-injection seam. All 25 variants are both raised in production and
  exercised by `recovery.rs`. This is what proves crash-consistency.
- `ObjectKey` / `ObjectPrefix` newtypes with their closed `.mallard/`
  allowlist: load-bearing security, not ceremony.
- The S3 ambiguity-resolution machinery (`PutAttempt::Ambiguous`,
  `resolve_ambiguous_delete`, `probe_failure_after_possible_write`) matches
  real R2 failure modes. There is no speculative retry/backoff layer.
- `ensure_storage_marker`'s post-write re-read is provably a no-op given
  `put_immutable`'s byte-equality contract, but its comment claims it for
  concurrent-first-writer safety and it costs one round trip per storage
  lifetime. Left alone deliberately.
- `parse_r2_api_url`'s hand-rolled pre-parse followed by a `Url::parse`
  re-check is strictly redundant on paper, but it guards destination aliasing
  on a security boundary and the module doc calls out the intent.
- The SecretString / PendingStorage zeroize-on-drop discipline and the
  fixed-string error contract.

**Abstractions with genuinely multiple implementations**
- `ObjectStore` — two real impls plus a runtime dispatcher; **no method is
  unused** and the generic parameter is instantiated at four distinct types.
- `BundleBytes` — three real impls, including a null object used in
  production.
- `CatalogProviderAdapter` — two real adaptees with materially different
  discovery formats (CLI JSON vs `settings.json` parsing).
- `CloudflareR2Api`, `CapabilityProbe`, `PluginProcessRunner` — one production
  impl each but a real injected test fake carrying the crash-recovery matrix.
- The `resource_planning` / `resource_engine` / `apply` split: plans are
  persisted and re-validated across process boundaries, so the split is
  load-bearing, not three layers over one operation.
- The `named_id!` / `fixed_hex_id!` macros add real invariants — they keep the
  local and cloud id namespaces prefix-disjoint.

**Correctly sized modules**
- `config_watcher.rs` (157 lines): atomic-replace-aware directory watching with
  a monotonic revision gate. Nothing to cut.
- `codex_paginated_projection.rs`: a frozen 0.147 adapter where every item-body
  mapper corresponds to a real Codex event type.
- `credential_redaction.rs`: **not** a layered detector abstraction — one
  struct of compiled regexes with six inline methods and a fixed 22-label
  vocabulary in which every label is reachable.
- `BundleScope::validate_project_scope` / `validate_provider_scope` (~465
  lines): distinct, non-overlapping structural rules with no duplication.
- The `MAX_*` limit constants that show zero external uses are all genuinely
  enforced inside the crate's own `validate` methods.

---

## Refuted claims

Recorded so they are not re-raised. Each was proposed by an auditor and killed
by the verifier.

1. **"`BundleFileEntry.object_key` is fully derived from `sha256`, so storing
   it is redundant."** Refuted — it is not purely derived at every site.
2. **"The session-detail command drops the entire watcher reuse cache while its
   siblings do not."** Refuted — the evidence did not establish the claimed
   asymmetry.
3. **"The R2 capability probe re-implements list/put/get/delete instead of
   reusing the store's transport helpers."** Mostly refuted — only
   `probe_conditional_put_async` genuinely overlaps (~20-25 LOC, not ~114).
4. **"Every pushed record is scanned by the credential detector 3–4 times."**
   Refuted — the eliminated pass only runs when a record actually matches, so
   the performance claim does not hold.
5. **"`detect_secret_material` is a second, weaker hand-rolled credential
   detector running on already-scrubbed bytes."** Refuted — `provider_capture.rs:481`
   is live on *unscrubbed* project-content bytes and its key-name branch is
   load-bearing. **Nothing here is safely removable.**

---

## Recommended remediation order

**Round 1 — ✅ LANDED 2026-08-21.** D1, D2, E1, G1. Full bar green: `clippy
--workspace --all-targets -D warnings` clean, 624 tests passing, `cargo fmt`
clean. What actually shipped, against what was planned:

| Item | Planned | Shipped |
| --- | --- | --- |
| D1 | 2 lines + threading, fixes a live bug | 2 lines, **no threading needed** (both locals were already live at their gates) — and it is a **cleanup, not a bug fix**; see the correction in D1 above. Added a self-proving regression test (2 controls) pinning "blocked rows never plan". |
| D2 | Return the inventory from `verify_plugin_apply` | **Deleted** `verify_plugin_apply`, both byte-identical impls, and `PluginVerification`; inlined one post-exit `discover_plugins` at the single call site. Net −66 LOC in `adapters.rs`. Subsumes F11. Also normalized a run of 18 stray spaces in a user-facing message. |
| E1 | ~5 lines | ~12 lines. The hoist also removes a per-attempt `Bytes::from(clone)` **full payload memcpy** that the register did not count, and lifts the mode/resource validation out of the retry loop. |
| G1 | 4 lines | 4 lines. `rusqlite` **and** `libsqlite3-sys` now have zero `Cargo.lock` entries (−98 lock lines). `toml` still enters the lock via `cargo_toml → tauri-build` as a host build dep; removing our two edges kills only its target-side compilation. |

**Round 2 — ✅ LANDED 2026-08-21.** A1, A2, A3, plus the live half of B22.
**21 commands removed** (96 registered → 75), `registration_freeze.rs` deleted,
and the per-command intent recorded in
`docs/IPC_SURFACE_DECISIONS_2026-08-21.md`. Full bar green: clippy
`--workspace --all-targets -D warnings` clean, **620 Rust tests** (624 − the 4
tests whose only subject was deleted code), 224 frontend tests, `cargo fmt`
clean. Net **−1,057 Rust lines**:

```
commands.rs                    −712   27,837 → 27,125, net of two helpers moved into mod tests
registration_freeze.rs         −138   file deleted
resource_commands.rs            −87
bundle_engine.rs                −64
persistence.rs                  −33
lib.rs                          −21   165 → 144
sync_tests/mod.rs                −1
thread_variants/continuation.rs  −1
```

Registered commands: **96 → 75**, verified by re-parsing the `generate_handler!`
block (careful: `connect_r2_storage` contains a digit, so a naive `[a-z_]+`
scan of that block undercounts by one — which is how the register reached 95).

| Item | Planned | Shipped |
| --- | --- | --- |
| A1 | 19 commands | **20** — the register missed `get_bundle_readiness`, which fails the same two-direction check. Verified before deleting: no `invoke("…")` for the 8 wrapper-less names, and zero references (in `src/` *and* `tests/`) to the 12 dead wrapper keys. |
| A2 | ~57 LOC | 63 LOC: `verify_objects` (53) + `BundleObjectAudit` (10), plus a stale doc reference on `publish`. |
| A3 | 61 LOC Rust + 2 TS | 87 LOC Rust (command + helper + the test that was its only caller) + the `deleteProfile` wrapper, its `catalog-live` stub, and two lines of the frontend test that had been asserting the UI must not use it. |
| B22 | 18 LOC in the hot loop | Done: `origin_label` gone from `ThreadSyncEntry`, its computation gone from `provider_thread_review`, the field gone from `types.ts`. The **cross-process lock + full conversation-bindings parse per thread** no longer runs on any review, plan, or Activity load. |

**Deliberately not deleted.** `resolve_origin_host_label`, `mutate_origin_aliases`,
`update_host_display_name`, and `rotate_host_identity` survive with test-only
callers. They are the only accessors of `ConversationBindings.origin_aliases`
and the host identity document; since `ConversationBindings` is
`#[serde(deny_unknown_fields)]`, removing the field makes every existing
`conversation_bindings.json` fail to parse. That is a wipe-based local-state
cutover, not dead-code removal, and belongs in its own change. Everything with
*no* callers — `derive_project_origin_tokens`, `load_origin_aliases` — went.

**Two helpers moved into `mod tests`** rather than being deleted, because live
tests use them for setup or observation: `register_local_project_with_repository`
(20 fixture uses) and a new `readiness_for_binding` probe over the live
`bundle_readiness_for_snapshot`. The first carries a named drift risk — see the
decisions doc.

**Round 3 — post-cutover leftovers.** B1–B11 as a batch; they are independent,
each is small, and together they retire ~400 LOC of misleading surface. B1 and
B2 first: both are security-shaped code that does nothing.

**Round 4 — the duplication with real drift risk.** F1 (the provider timeline
split — largest, and the only one where a missed edit is silent), then F4 and
F5, which are the two copies sitting on correctness-critical paths
(token revocation, redaction path rewriting).

**Round 5 — vocabulary and speculative plumbing.** C1–C4 and D1–D4. Note C1 and
C3(a) are wire-schema changes; they are consistent with the schema-1 reset,
which already renumbered every schema with no migration, but they need the same
wipe-based cutover treatment.

**Deferred pending a product decision, not a code decision:** C7 and C8 (the
cursor-less provider seam — either register a real cursor-less provider or
delete the seam), D1 (the CLI ID allocator — the CLI-preferred order is a
written design decision), and B2 (`validate_session_ownership` — confirm no
planned deep-link entry point needs it).

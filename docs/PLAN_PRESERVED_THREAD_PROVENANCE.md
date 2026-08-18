# Portable preservation provenance + unique preserved markers

## Context

The uncommitted working-tree fix added `LocalConversationBinding.preserved_local: bool`, set when a divergent Pull's KeepBoth preserves the local branch under a fresh logical thread id. Three gaps remain:

1. **Not portable**: the flag lives only in the receiving machine's `conversation_bindings.json`. A user pulling a preserved thread gets a binding with `preserved_local = false` — it looks like a normal thread. The origin link is destroyed at exactly one line (`row.forked_from = None`, commands.rs:10254).
2. **No unique marker**: every preserved copy shows the same static "Preserved" badge. Repeated keep-boths are distinguishable only by thread id/timestamp.
3. **Marker carrier chosen**: rename the preserved copy's title in Codex's `session_index.jsonl`. Verified safe — compaction/continuation linkage reads only transcript `session_meta` (session ids, `forked_from_id`, `history_base`); index titles are display-only. Titles already travel: Push publishes the index title as `ConversationThread.display_title`; Pull writes it into the receiving machine's index.

**Decisions**: marker format **`Preserved {N} · {device label}`**; Session Detail title precedence **flips to index-title-first**; clean cutover — no migration, no legacy path, **no schema-version bump** (`BUNDLE_SCHEMA` and `CONVERSATION_BINDINGS_SCHEMA` stay 1; shapes change in place); `~/.mallard` + cloud data are wiped before retesting; **edge-case ordinal collisions are acceptable** (no cross-device coordination).

## Design

- New domain struct **`PreservedProvenance { preserved_from: ThreadId, ordinal: u64 }`**, carried as `preserved: Option<PreservedProvenance>` on **both** `LocalConversationBinding` (replacing `preserved_local` — the bool dies everywhere) and portable `ConversationThread`. Serde: `#[serde(default, skip_serializing_if = "Option::is_none")]` — `None` records serialize byte-identically, so scope `content_sha256` digests of untouched threads never churn (same pattern as `shared_label`). Validation: `ordinal >= 1`, `preserved_from != carrier thread_id`; **no** referential-existence rule (origin may be tombstoned) and **no** cross-record ordinal-uniqueness rule.
- **Ordinal** = 1 + max ordinal where `preserved_from == origin`, over pinned cloud snapshot conversations ∪ local bindings (max+1, not count — no reuse after deletions). Both inputs are covered by existing pin re-validation at apply (scope sha + bindings revision → stale actions replan). Residual cross-device race accepted; device label disambiguates.
- **Minted at plan time** (no-mint invariant, lib.rs:2986-2989): `KeepBothPlan` gains `preserved: PreservedProvenance` + `preserved_display_title: String`. `KeepBothJournal` and `ConversationApplyReceipt` unchanged — crash recovery replays hash-pinned intent payloads, so the composed index bytes survive without recomposition.
- **Device label**: `HostIdentity.local_display_name` (validated ≤128 bytes, lib.rs:1477/1518-1524), fallback `fallback_origin_host_label(origin_host_token)` ("Host 7F3A" style) via `repository.derive_origin_tokens(bundle_id, replica_id)`. Also wire Push to actually publish it: `build_publish_request` sets `PortableOrigin.shared_label` from host identity (today every constructor passes `None`).
- **Title composition**: pure helpers in domain — `compose_preserved_title(base, ordinal, label)` and `strip_preserved_marker(title)`. `"{base} (Preserved {N} · {label})"`, marker alone when no base; strip an existing `(Preserved … )` suffix from the base (no nesting when a preserved copy is itself preserved); char-truncate base so total ≤ 180 chars (Codex read-side truncation budget; write-side 1024-byte cap then always passes). Base title source at plan time: `local.descriptor.display_name` (filtered non-empty, `!= provider_thread_id`) → cloud `display_title` (filtered vs `source_thread_id`) → none.
- **Two-row index write**: `codex_index_payload` reshaped to take `&[CodexIndexRowSpec { provider_thread_id, thread_name, updated_at, replace_existing }]` — per-line: drop old rows for a spec'd id when `replace_existing`, error when `!replace_existing`, append one row per spec. KeepBoth writes destination row (existing rules) + preserved-copy rename row (`replace_existing: true` — its id keeps the original provider session id and already has Codex rows). Still ONE index file target (`index_fragment_limit == 1` untouched). Verification learns the ids from a new `index_rows: Vec<String>` on `ConversationJournalTarget`/`ConversationTargetPayload`; the fragment verify arm loops `verify_index_fragment(bytes, id)` per id (`validate_codex_index_fragment` already asserts exactly-one-row-per-checked-id and tolerates other ids — no change).
- **Publish carry** (mirrors `forked_from` at commands.rs:7714-7719): `CreateCloud` → `local_binding.preserved`, every other action → `previous.preserved`. A preserved copy's first push is CreateCloud (binding is the only source); later pushes carry from the manifest.
- **Pull restore**: `PreparedPullConversation` gains `cloud_preserved`; `CreateLocal` restores it (replacing hard-coded `false` at 10297); KeepBoth's new origin row takes `prepared.cloud_preserved` (cloud wins; replaces the `original_was_preserved` local inheritance — provably equal post-wipe). `MaterializeCloud`/`AdoptCloudMetadata` untouched.
- **Marker propagation** needs no new machinery: the rename lands before the preserved copy's first Push, so `display_title` carries it; Pull writes it into other machines' indexes via the existing flow.

## Stage 1 — Domain (`crates/mallard-domain/src/lib.rs`)

1. Add `PreservedProvenance` (near `PortableOrigin`, ~1945) with `deny_unknown_fields`, `validate(&self, carrier: &ThreadId)` (ordinal ≥ 1, no self-preserve).
2. Add `compose_preserved_title` / `strip_preserved_marker` + `PRESERVED_TITLE_MAX_CHARS = 180` (deterministic, regex-free suffix parse: trailing `(Preserved <digits> · <label>)`).
3. `LocalConversationBinding` (1741-1764): replace `preserved_local: bool` (1750-1752) with `preserved: Option<PreservedProvenance>` (`default` + `skip_serializing_if`); call its validate in `validate()` (~1768). `deny_unknown_fields` rejects documents still carrying `preserved_local` (retired key).
4. `ConversationThread` (2694-2735): add `preserved` after `forked_from` (2725), same serde attrs; validate next to the self-fork check.
5. `KeepBothPlan` (2990-3004): add `preserved` + `preserved_display_title`; validate provenance against `preserved_thread_id`, `validate_display_text` on the title; in `ConversationPlanAction::validate` require `keep_both.preserved.preserved_from == self.thread_id`. Update the doc comment (2986-2989): ids **and marker title/provenance** minted during planning.
6. `ConversationJournalTarget` (2147-2188): add `index_rows: Vec<String>` (`default`, `skip_serializing_if = "Vec::is_empty"`); validate: non-empty exactly when role is `ProviderIndexFragment`, ≤8, valid + unique ids.
7. `KeepBothJournal`, `ConversationApplyReceipt`: no change.

## Stage 2 — Infrastructure (`crates/mallard-infrastructure`)

8. `conversation_materialization.rs`: `ConversationTargetPayload` (28-33) gains `index_rows`; `prepare_operation` (843-857) copies it into the journal target; fragment verify arm (1283-1291) drops `operation.provider_thread_id` sourcing, loops per `target.index_rows` id. Recovery replay needs nothing else (re-verifies from journaled target + intent payloads).
9. `providers/codex/materialization.rs`, `providers/traits.rs`, `providers/codex/mod.rs`: no change.
10. `project_activity.rs`: `ProjectSessionSummary.is_preserved_local: bool` (88) → `preserved: Option<PreservedProvenance>`; preserved-set (606-621) becomes `BTreeMap<ThreadId, PreservedProvenance>`; local row at 718 uses it; `RemoteSessionRecord` (519-525) + `remote_sessions_from_snapshot` (527-541) copy `thread.preserved` so **remote-only rows (783) show provenance too**. Update/rename the in-file test (1888-1939).
11. `session_detail.rs:1528-1532` — precedence flip: index `display_name` first (filtered non-empty, `!= provider_session_id`, mirroring project_activity.rs:677), transcript `summary.title` fallback, then `display_name` unconditionally.
12. `persistence.rs`: fixtures 2847/3100 → `preserved: None`; `storage_clear_keeps_local_conversation_preservation_provenance` (2925-3007) asserts the provenance value survives `clear_storage_data`.

## Stage 3 — Commands plan side (`src-tauri/src/project_sync/commands.rs`)

13. KeepBoth planning arm (5856-5911): compute ordinal (scan `provider_scope` conversations + `bindings_by_thread` values, filter `preserved_from == thread_id`, `max().unwrap_or(0) + 1`); resolve device label (host identity loaded at ~5069; fallback via `derive_origin_tokens` — `snapshot.expect(...)` precedent at 5632); pick base title; `compose_preserved_title`; extend the `KeepBothPlan` literal (5907-5910). Comment the pin-covered ordinal basis + accepted cross-device race.
14. Push carry — `conversation_thread_from_capture` (7650): add `preserved` selection next to `forked_from` (7714-7719) and set it in the `ConversationThread` literal (7747-7776, next to `forked_from` at 7766).
15. Push origin label — `build_publish_request` (7862-7869): set `shared_label` from host identity `local_display_name` (trim/filter).
16. Push-created binding rows (8404-8420): `preserved: None` (replacing `preserved_local: false` at 8412).

## Stage 3b — Commands apply side

17. `PreparedPullConversation` (9554-9564): add `cloud_preserved`; populate ~9829 from `thread.preserved`.
18. Index payload assembly (10152-10182): build `Vec<CodexIndexRowSpec>` — KeepBoth pushes the preserved rename row `{ id: live binding's provider_thread_id (from revision-pinned `document_before`, error if unbound), name: keep_both.preserved_display_title, updated_at: applied_at, replace_existing: true }`, then the existing destination row (cloud-title filter 10161-10167 unchanged, `replace_existing: action == MaterializeCloud`). Call reshaped `codex_index_payload(&index_path, &rows)`; set `index_rows` on the payload; transcript payload gets `index_rows: vec![]`. Update the doc comment at 9872-9876. Note: titleless-cloud KeepBoth now writes a payload (marker row only) where it previously wrote none.
19. Binding move (10242-10277): delete `original_was_preserved` (10252); moved row `preserved = Some(keep_both.preserved.clone())` (replacing 10255; `forked_from = None` stays); new origin row `preserved: prepared.cloud_preserved.clone()` (replacing 10265). `CreateLocal` (10289-10305): `preserved: prepared.cloud_preserved.clone()` (replacing 10297).
20. `PreservedThreadResult` (639-643): add `preserved_display_title: String`; builder (10766-10781) looks it up from `plan.conversation_actions` by `receipt.action_id` → `keep_both.preserved_display_title` (error on lookup miss — plan/receipt mismatch).

No new Tauri commands (registration freeze pins 95 names).

## Stage 4 — Frontend

21. `src/types.ts`: add `PreservedProvenance`; replace `is_preserved_local` (990) with `preserved?: PreservedProvenance | null`; extend `KeepBothPlan` (833-836) and `PreservedThreadResult` (905-909).
22. `ProjectActivityPage.tsx` (1185-1191): badge on `session.preserved`, text `Preserved {ordinal}`, tooltip includes origin thread short-id. `compactThreadTitle` (322-327) untouched (only strips trailing ` · <timestamp>`). CSS unchanged.
23. `syncOutcome.ts` (177-201/311): preserved outcome rows titled with `preserved_display_title`.

## Stage 5 — Tests

**Mechanical fixture updates** (`preserved_local: false` → `preserved: None`): domain tests.rs 2176/2569; continuation.rs 517/532/793/1001; recovery.rs 675/1407; schema_and_privacy.rs 358; persistence.rs 2847/3100; commands.rs test fixtures (15829/16748/19712/21337). `ConversationTargetPayload`/journal fragment fixtures in recovery.rs (453-522, 988, 1301-1313) and continuation.rs (1061-1070) gain `index_rows` (fragment: the row ids; transcript: `vec![]`). Harness `ThreadSpec` (harness.rs:152-183) gains `preserved: Option<PreservedProvenance>` → set on the built `ConversationThread` (243-274).

**Cutover assertion**: flip domain tests.rs:2584-2589 from "missing `preserved_local` fails" to retired-key rejection — inject `"preserved_local": false` into serialized binding JSON, assert deserialization fails.

**New tests**:
- Domain: title compose/strip (no-base, truncation to 180, nesting strip, unicode label); provenance rejects ordinal-0 + self-preserve (direct + via binding/thread validate); `KeepBothPlan` cross-check (provenance must name the action's thread); journal-target `index_rows`↔role pairing.
- schema_and_privacy.rs: two new `ScopeMutation` rejections via `mutate_codex_scope` (resealed digest): self-preserve, ordinal-0.
- continuation.rs: ordinal advances past max visible ordinal (cloud thread with ordinal 5 → next mint is 6, not 2); repeated keep-both → ordinal 2 with un-nested marker.
- recovery.rs: divergent keep-both crash-recovery sweep (reuse `MaterializationFixture`, two-row index payload) — fixed point always has exactly one marker row, one destination row, provenance in committed bindings, journal retired.
- E2E extension (commands.rs:19759 test, tail 20184-20270): set host display name "Machine A"; after divergent KeepBoth assert binding provenance `{preserved_from: origin, ordinal: 1}`; index has one row for the original session id titled `… (Preserved 1 · Machine A)` + one destination row; `preserved_threads[0].preserved_display_title` matches; push → manifest record carries `preserved`, marker `display_title`, `shared_label: "Machine A"`; pull on second repo → CreateLocal binding restores provenance; session detail title = marker title (flip verified).
- Frontend: project-activity.integration.test.tsx (badge `Preserved 2`, marker survives `compactThreadTitle`); pull-review/thread-variants fixtures for new fields; outcome row title shows marker.

## Stage 6 — Docs

- `docs/DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md:487-492` — index write now two rows (destination + keep-both rename), minted at plan time, same transaction.
- `docs/CURRENT_METADATA_LAYOUT.md` — bindings contents (139), keep-both walkthrough (300-330; fix the stale `forked_from = t` claim → provenance field), portable field list (405-410).
- `docs/PLAN_CODEX_ACTIVITY_FORK_LABELS.md:43` — amend the "never write thread_name" note.

## Verification

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mallard-domain
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:frontend-integration
```

Manual retest after wiping `~/.mallard` + cloud bucket: diverge a thread on machine A, Pull (keep-both) → title gains `(Preserved 1 · <label>)`, badge `Preserved 1`; Push; on machine B Pull → same badge/title, binding provenance present; diverge again → `Preserved 2`.

## Risks

- **Scope digest coupling**: `preserved` must keep `default` + `skip_serializing_if` so `None` records stay byte-identical in `content_sha256` (lib.rs:3720-3777); carried-verbatim threads (commands.rs:7984-7987) are never rebuilt, so no digest churn on untouched scopes.
- **No-mint invariant**: ordinal/label/title minted only in planning; apply composes bytes from planned strings + mechanical file read; recovery replays pinned payloads.
- **Ordinal race**: same-device fully pinned (stale → replan); concurrent cross-device same-origin preserves can duplicate an ordinal — accepted; device label + distinct thread ids disambiguate.

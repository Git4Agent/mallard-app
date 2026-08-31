# Plan: Preserve-as-Copy for Diverged Project Files on Pull

**Status:** implemented (backend + minimal frontend), uncommitted
**Date:** 2026-08-30
**Scope:** backend first — replace the diverged project-file Pull behavior
(overwrite with a hidden backup) with the Catalog's preservation contract:
copy the local version to a visible sibling, then write the storage version.
Frontend gets only the minimal adaptation that keeps the Pull review honest;
the card redesign ships separately with the files-review redesign.

## 1. Problem

A diverged project file (changed both locally and in storage since the
reviewed base) is planned today as a plain `WriteProjectFile` action with
`requires_explicit_approval` and a pinned `expected_target_sha256`
(`bundle_engine.rs:1425-1461`, approval rule at `bundle_engine.rs:3209`).
Apply backs the local bytes up to `<plan_backup>/<action_id>.bak` and
overwrites (`bundle_engine.rs:4771-4800`). The local version survives only in
a hidden, plan-scoped backup directory the user never sees; the visible
outcome is "storage won".

Skills and pets already solve this correctly. The Catalog plans a diverged
resource as `PreserveSkillAsCopyAndOverwrite` /
`PreservePetAsCopyAndOverwrite` (`storage_catalog/domain.rs:2018-2029`),
carrying a `CatalogPreservationPin` minted at plan time
(`storage_catalog/domain.rs:1981`): a collision-free copy name
(`<name>-local-copy`, then `-2`…, `resource_planning.rs:172`), the pinned
local/storage digests, and a `reusable_witness` flag so an interrupted apply
can reuse a verified copy (`apply.rs:1300-1400`). The local version stays a
first-class, visible resource; the pull is never lossy.

Project files should follow the same contract: **copy to local first, then
pull.**

## 2. Decisions

- **D1 — one action, no choice menu.** A diverged file plans exactly one
  action: preserve-and-write. There is no destructive "overwrite without
  copy" alternative and no per-row action menu. Rationale: the ordinary
  restore-action wire has no alternatives concept today, an unwanted copy is
  one Finder delete away (no data can be lost in either direction), and the
  clean-cutover rule says not to grow a second path speculatively. This
  diverges from the Catalog (which keeps a gated overwrite option); if a real
  need appears, the Catalog shape is the template. The mocked "action pill"
  (D2 in the files-review mock) is superseded.
- **D2 — copy naming follows the Catalog, extension-aware.** The copy of
  `ghost_story.docx` is `ghost_story-local-copy.docx`, then
  `ghost_story-local-copy-2.docx`… The suffix lands before the final
  extension (split at the last dot with a non-empty stem; `Makefile` →
  `Makefile-local-copy`). The stem is trimmed so the component respects the
  existing 255-byte logical-path component cap (`provider_capture.rs:2076`);
  generation fails closed after the same bounded attempt count the Catalog
  uses.
- **D3 — the copy lands beside the original.** Same directory, same mode,
  best-effort same mtime. It is a plain new local file: the next Push scan
  discovers it as `local_only`, `selected_after_scan`, pushable or deletable
  like anything else. No manifest entry, no tombstone, no provenance marker
  is written for it.
- **D4 — diverged means local ≠ base and local ≠ storage, and the base is
  threaded into the plan.** The restore planner is two-way today (manifest
  vs. disk, `bundle_engine.rs:1402-1412`); the three-way classification
  lives only in the service (`project_sync/service.rs:2965`). The plan
  request gains a per-resource reviewed-base digest map supplied by the
  service from the same source the Push review classifier reads. A file
  whose local digest equals the base (plain `storage_ahead`) keeps today's
  plain write. A file with **no known base fails closed to preservation** —
  when the app cannot prove the local bytes are already stored, it keeps
  them.
- **D5 — plan-time pin, apply-time verification, witness reuse.** The action
  carries a pin: copy relative path, the local digest (which is also the
  copy's expected digest — files are not renamed inside, so no transformed
  digest exists), the storage digest, and `reusable_witness`. If the copy
  path is already occupied at plan time by bytes matching the local digest
  (a previous interrupted apply), the plan reuses that name with
  `reusable_witness: true`; occupied by anything else, the next suffix is
  taken. At apply time an occupied copy path that does not match the pin is
  a hard refusal, mirroring "restore target changed after planning".
- **D6 — the hidden backup stays.** The `.bak` + `write_verified_body_atomic`
  machinery (`bundle_engine.rs:4771-4800`) is crash-safety for the write
  step and is untouched. The visible copy is additive, made **before** the
  backup/write sequence.
- **D7 — the pin itself names the copy.** The copy path travels on the
  action's pin, which the plan already hands the frontend; receipts
  reference the action, so outcome surfaces read the copy path from the
  plan rather than a duplicated receipt field. Skipping the action remains
  a valid completed outcome (`service.rs:11991`).

## 3. Backend design

### 3.1 Types (`mallard-domain` + `bundle_engine.rs`)

```rust
pub struct ProjectFilePreservationPin {
    /// Project-relative path of the copy, validated like any logical path.
    pub copy_relative_path: String,
    /// Digest the copy must have — the pinned local bytes.
    pub local_content_sha256: String,
    pub storage_content_sha256: String,
    /// A verified copy from an interrupted apply may be reused.
    pub reusable_witness: bool,
}

RestoreActionKind::WriteProjectFile {
    logical_path: LogicalPath,
    mode: u32,
    source_mtime: u64,
    /// Some(_) = diverged: copy the local version here before writing.
    preserve: Option<ProjectFilePreservationPin>,
}
```

The pin rides the existing `WriteProjectFile` variant rather than a new one:
the action type, `action_id_for` identity, and every existing match arm stay
put, and the wire tag stays `write_project_file` with an added field — a
preservation IS a write, distinguished by what it promises to keep. Diverged
plans always carry `requires_explicit_approval: true` (already guaranteed by
the digest-drift rule at `bundle_engine.rs:3209`).

### 3.2 Planning (`bundle_engine.rs`, files pass 2, `:1425`)

Inputs grow a `project_content_bases: BTreeMap<ResourceId, String>` supplied
by `plan_bundle_restore_with_progress` (`project_sync/service.rs`), read from
the reviewed-base source the Push classifier uses. In pass 2, for a
`ProjectContentFile` with a present target and `target_digest ≠
entry.sha256`:

- `target_digest == base` → today's `WriteProjectFile`, unchanged.
- otherwise (including absent base) → mint the pin:
  1. candidate name from `generate_project_copy_name(file_name)` (new helper
     beside `project_content_target`, `bundle_engine.rs:3263`), checked
     against: the walked directory contents, every other planned action's
     target and copy path in this plan, and the manifest's logical paths
     (so a later Pull of the same revision cannot collide with it);
  2. occupied-by-identical-bytes → same name, `reusable_witness: true`;
  3. emit `PreserveProjectFileAsCopyAndWrite` with
     `expected_target_sha256`/`expected_target_mode` pinned exactly as
     today.

The plan's human-facing detail line reads like the Catalog's: *"Keep the
local version as 'ghost_story-local-copy.docx' and update from Attic R2."*

### 3.3 Apply (`bundle_engine.rs:4700-4812`)

For the new kind, before the existing backup/write sequence:

1. Existing pin checks run first (target digest + mode drift refusals,
   `:4708-4717`).
2. Copy step: if `reusable_witness` and the copy path holds the pinned
   digest, skip; if the copy path is occupied otherwise, refuse
   ("preservation copy target changed after planning"); else copy the
   original to the copy path with its mode, verify the copy's digest by
   re-read, best-effort mtime.
3. Then the unchanged sequence: `.bak` backup → verified body from the
   manifest object → atomic write → mode + mtime restore.

Interruption windows and recovery, without a journal:

- **After copy, before write:** the original is untouched; the next plan
  sees the file still diverged, finds the verified copy, and pins it as a
  reusable witness (D5). No duplicate copies accumulate.
- **After write:** the file matches storage; the next plan sees `UpToDate`
  (or a plain fast-forward) and the copy is simply a new local file. Done is
  done.

Receipts change nothing: the copy path is readable from the plan action's
pin wherever the outcome is rendered (D7).

### 3.4 Explicitly out of scope

- Tombstone deletes of locally-modified files: the delete action already
  refuses on digest drift (`bundle_engine.rs:4568`); preservation for
  deletes is a separate decision.
- Directory mode drift, generic (non-project) files, and the Push direction
  (Push never overwrites newer storage; it holds for Pull).
- A destructive overwrite alternative and any per-row action choice (D1).

## 4. Wire and minimal frontend

`RestorePlan` lives in memory (plan store) and is rebuilt per review, so the
new kind and pin are additive serde changes with no persisted-schema impact.
The same change updates:

- `src/types.ts`: the `RestoreActionKind` union + pin shape.
- `src/components/project-sync/pullReviewModel.ts:105-140`: map the new kind
  to the existing `replace` operation with a `keepsLocalCopy` detail so the
  current Pull review renders it truthfully (row label "Replace file · keeps
  local copy", detail naming the copy path).
- `src/components/project-sync/RestorePlanView.tsx`: pass the detail through
  where `write_project_file` is labeled today (`:345`, `:555`).

The card-grammar redesign (files-review mock, Section D) then only restyles
this — D1's checkbox-only reading becomes exact, with the chip carrying
"keeps local copy" instead of a pill.

## 5. Tests

Rust, colocated with the existing project-content coverage:

1. **Name generation** (unit): extension split, extensionless names, dotfile
   stems, collision → `-2`, stem trim at the component cap, bounded-attempt
   failure is an error not a panic.
2. **Planning**: diverged (local ≠ base ≠ storage) → preserve action with
   pin; `storage_ahead` (local == base) → plain write; absent base →
   preserve (fail closed); byte-identical target with mode drift → plain
   write, no pin; occupied copy path with identical bytes → same name +
   `reusable_witness`; occupied with different bytes → suffixed name;
   copy name never collides with manifest paths or other planned targets.
3. **Apply**: happy path — copy exists with original bytes + mode, original
   holds storage bytes, receipt names the copy; target drift → refusal and
   no copy written; witness reuse path skips the copy step; occupied
   non-matching copy path → refusal; interrupted-after-copy simulation →
   re-plan pins witness and apply completes.
4. **Service round trip**: push from A, diverge on B, pull on B → plan
   carries preserve; after apply the next Push scan reports the copy as
   `local_only` + `selected_after_scan`, and the original as `synced`.
5. **Frontend** (integration tests): `pullReviewModel` maps the new kind to
   `replace` + copy detail; the Pull review renders the copy path.

Full bar (`cargo test --workspace --all-targets`, clippy `-D`, frontend
integration suite, `tsc`) before calling it done.

## 6. Cutover

Clean cutover, per the standing rule: no migration, no backward
compatibility. The diverged branch plans only the new kind — the old
"diverged → plain overwrite write" pairing is gone in the same change, and
no compatibility arm reads it (plans are transient, so none can be stored).
There is no user data to carry: `~/.mallard` and the cloud buckets are wiped
before the next live test anyway; no persisted format moves either way.

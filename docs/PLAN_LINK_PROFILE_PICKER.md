# PLAN: Link = local profile ⇄ chosen cloud profile within a storage

Status: implemented (2026-07-15), including the step 0 revision below.

## Problem

A `SyncLink` stored only `profile × storage`; the cloud side was resolved
*implicitly* on first sync (`resolve_profile_for_link`, `lib.rs`): auto-discover
by `root`, silently auto-create when none matched. The only manual control was
the free-text "Cloud path" pin field hidden in the link's "…" settings.

Observed failure (2026-07-15 debug session): push from `myconf2/.claude`
created cloud profile A in a local-folder storage; setting up `myconf4/.claude`
against the same storage carried a stale link id, fell through to discovery,
and discovery *excluded* profile A because a sibling link already targeted it
(the old `sibling_cloud_targets` guard, forced by baselines being keyed
`(storage, cloud profile)` with no local-profile component). With zero
candidates left it created an **empty** profile B, pulled 0 files, and plugin
repair had nothing to replay.

## What was built

### 0. Per-link baselines; shared cloud profiles become first-class

Revises the PLAN_MULTI_STORAGE.md "different cloud prefixes" rule (§ Config
model). Baselines are now keyed `(local profile id, storage id, cloud profile
id)` — `baseline_path` in `lib.rs` — making each link an independent replica,
exactly like a second machine. Consequences:

- Two local roots on one machine may link the SAME cloud profile in the same
  storage; changes flow between them through the profile (the
  simulate-two-machines case, and the natural user expectation).
- The duplicate-target guard is deleted everywhere: `sibling_cloud_targets`,
  the resolve-time "already synced by another profile" error, the discovery
  filter, and `validate_sync_config`'s "fight over baselines" rejection.
- Clean break like v2 (§3.1): old baseline files under the two-part key are
  never read again; the first sync per link re-verifies by hash — union
  semantics, no data loss, no migration code.
- The cloud cache stays keyed `(storage, cloud profile)` — it mirrors cloud
  state and is replica-independent.

### 1. Two-step link picker — `src/components/SyncPanel.tsx`

"Link another storage" → pick a storage → pick a cloud profile inside it
(from the `list_sync_profiles` probe): label, short id, revision, file
count, or **"Create new profile"** (the old auto-create, now explicit).
Picking an existing profile writes `cloud: { root, profile_id, profile_label,
pinned: true }` — no backend change; pinned resolution already honors it.
A profile another local root syncs is selectable and tagged
"also synced by <profile>" so shared linkage is a choice, not an accident.

### 2. Re-pick on an existing link — same picker in the link's "…" settings

Above the free-text "Cloud path" pin field (kept as the escape hatch for
PLAN_SYNC_LINKS-style prefixes like `001/.claude`). Confirms before
relinking; the baseline resets by construction (different key), next sync
re-verifies by content.

### 3. Auto-create label dedup — `unique_profile_label` in `lib.rs`

Auto-created profiles suffix their label ("Claude 2") when the base label is
already present in the storage, so same-root profiles stay tellable apart in
the picker and linkage lines.

## Tests

- `sync_tests` S26 (both backends): two local roots share one cloud profile —
  independent baseline files; a stale sibling's push converges to the newer
  revision instead of republishing old bytes; divergent edits produce a
  normal conflict sibling.
- Unit: `unique_profile_label_suffixes_taken_names`; the former
  "fight over baselines" validation test now asserts shared targets are valid.
- Frontend: `npm run build` (no frontend test framework).

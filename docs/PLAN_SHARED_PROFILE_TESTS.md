# PLAN: Integration tests — shared cloud profiles & the profile picker

Status: implemented (2026-07-15) — S27–S33 landed dual-backend, P1/P2 fixed;
suite at 247 tests.

Companion to PLAN_LINK_PROFILE_PICKER.md (implemented 2026-07-15): baselines
are per link `(local profile, storage, cloud profile)`, several local roots
may sync ONE cloud profile, and the UI picks the cloud profile explicitly.
S26 proved the core invariant (independent baselines; stale sibling
converges, never clobbers; divergent edits conflict normally). This plan
covers the remaining surfaces: multi-hop propagation, the shared cloud
cache, conflict resolution across sibling links, and the picker's resolve
semantics.

All scenarios follow the house pattern: `run_*` bodies executed against both
backends (stub S3 + local folder), harness `Machine`/`TestCloud`,
`cargo test --lib sync_tests`.

## Product fixes the test design exposed (implement first)

- **P1 — "Create new profile" has no create intent.** The picker writes
  `cloud: {}` (auto). Resolve's auto path creates only when the storage has
  ZERO root-matching profiles; with one it silently links the existing one,
  with several it errors "pin one explicitly" — so the button lies whenever
  a profile already exists. Fix without backend changes: the picker
  generates a fresh id client-side (`genId()`), computes a non-colliding
  label from the probe ("Claude 2"), and writes
  `cloud: { root, profile_id: <fresh>, profile_label: <deduped>, pinned: true }`
  — the existing pinned-create path materializes it at that exact name.
- **P2 — in-place re-pick leaks the old baseline.** `save_sync_config`'s
  §3.2 cleanups drop baselines for removed links and identity-changed
  storages, but NOT when a surviving link's `cloud.profile_id` changes in
  place (the picker's relink). The orphaned baseline is re-read if the user
  ever re-picks back to the old profile, potentially stale. Fix: in the
  save diff, when a prev link's cell survives but its cloud id changed,
  push the prev tuple into `stale_links`.

## Scenarios

### A. Two local profiles ⇄ one cloud profile

- **S27 — relay convergence, and a sibling equals a machine.**
  Machine M: default `.claude` (A) + custom "conf4" (B), both linked to one
  cloud profile P. A pushes v1 → B pulls → B edits, pushes v2 → A pulls.
  Then machine N (a real second machine) links the same P, pulls, edits,
  pushes v3; A and B pull. Assert: all three replicas converge at every
  hop (`assert_converged` for A/B via profile reads + N), revisions
  advance linearly (no conflict siblings — edits are sequential), and
  M holds two distinct baseline files for P while N holds one.

- **S28 — per-link statuses over the SHARED cloud cache.**
  The cloud cache stays keyed `(storage, cloud profile)` — one sibling's
  push refreshes it for both links; statuses must still differ per link
  because baselines don't. A and B both synced at rev 1. A edits + pushes
  (rev 2, cache updated by A's push). Without B pulling:
  `get_file_statuses(profile=B)` → the file is `cloud-ahead` for B and
  `synced` for A. After B pulls, both `synced`. Guards the
  baseline-per-link × cache-shared interplay that made the old design
  forbid sharing.

- **S29 — conflict resolution propagates between siblings.**
  A and B edit the same file divergently; both push (union) → loser lands
  as a `.sync-conflict-<hash>` sibling on both after pulls. Resolve the
  sibling from A via `resolve_conflict_copy` (publishes a manifest-only
  deletion to A's links on P). B pulls → the review copy disappears on B
  too (published resolution reaches the unchanged replica), main file
  content intact. Exercises `resolve_conflict_copy`'s baseline loop under
  the per-link key.

### B. Picker / resolve semantics

- **S30 — pick an existing profile among several (the myconf4 fix).**
  Storage holds P1 (411-file stand-in: a few files, rev ≥ 1) and P2
  (empty), both root `.claude` — the exact post-bug layout. Fresh root C
  linked with `cloud: { profile_id: P1, pinned: true }` (what the picker
  writes). Pull: C receives exactly P1's files (including
  `.claude/agent-sync/claude-plugins.lock.json` seeded into P1 — the
  plugin-repair precondition that was the original bug report), P2 stays
  empty, `profiles_for_root(".claude")` count unchanged (nothing created),
  and C's saved link still targets P1 (no silent relink).

- **S31 — the auto path (`cloud: {}`) is only safe alone.**
  (a) One candidate in storage → auto links it, `pinned: false`, no
  creation. (b) Two candidates → push/pull errors "pin one explicitly",
  storage untouched (no third profile, revisions unchanged). Documents
  why the UI always writes an explicit pick and why "Automatic" in the pin
  field is fine for the single-profile common case.

- **S32 — create-new actually creates (P1 fix).**
  Storage already holds one `.claude` profile. Root D linked the way the
  fixed picker writes create-new: fresh pinned id + deduped label. Push:
  a NEW profile exists at that exact id with label "Claude 2", the old
  profile's head/revision untouched, D's baseline keyed to the new id.
  Also assert `list_sync_profiles` (the probe the UI consumes) returns
  both with distinct labels.

- **S33 — re-pick moves the link and resets state (P2 fix).**
  Root A synced with P1 (baseline exists). Save a config where A's cell now
  targets P2 pinned (the picker's relink through `save_sync_config`).
  Assert: P1's baseline file for A is deleted (P2 cleanup), P2 pull
  hash-reverifies (all files land, no conflict siblings), and a later
  re-pick BACK to P1 re-verifies from scratch instead of trusting the old
  baseline. Sibling link on P1 (from another root) keeps its baseline
  untouched throughout.

## Harness additions (small)

- `Machine::pick_cloud_profile(&cloud, local_id, profile_id, label)` —
  write one link's cloud side pinned to an exact id (what the UI picker
  persists), without the per-root `pins` map (which applies to every
  storage; too broad for per-link picks).
- S30/S32 need a way to seed a second profile: push from a throwaway extra
  profile (existing `add_profile` + `push_profile`) — no new harness code.

## Non-goals

- UI-level testing of the picker (no frontend test framework; `npm run
  build` remains the frontend check). These scenarios pin the backend
  contract the picker relies on.
- Plugin repair execution (shells out to the Claude/Codex CLI). S30 asserts
  the synced lock file's presence — the exact gate behind "no plugin
  intent to repair" — not the CLI replay.

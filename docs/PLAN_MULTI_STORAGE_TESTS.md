# Plan: Integration Tests for Multiple Storages × Multiple Local Profiles

Status: IMPLEMENTED 2026-07-14 (suite at 230 tests, ~31s). Notes:
- When implementation started, S21–S23 (same-name isolation, fan-out,
  unlink/relink + identity edit) and every §4 guard/unit already existed in
  the tree — this plan's remaining gaps landed as **S24** (the flagship
  matrix, §3 here) and **S25** (storage/profile removal rows, §4's S24).
  Numbering below is the plan's original; the mapping table now lives in
  PLAN_MULTI_STORAGE.md §8.
- Harness (§2) landed as: `Machine::add_profile(id, root, dir, pin)` for
  custom local profiles, `push_profile`/`pull_profile` per-link ops,
  `seed_profile`/`read_profile`/`list_profile`/`profile_path` (including the
  per-profile `~/.agent-sync/{id}` remap), and `ensure_link_config`
  generalized to any profile id. No compat shim was needed — the v2 config
  builders already existed.
- The per-profile agent-sync remap is `~/.agent-sync/{profile id}` (ids are
  unique), not the `{slug}.{id}` scheme §3 item 6 sketched — asserted via
  `Roots::for_profile_with_home` in S24.
- All scenario expectations held; no product bugs surfaced this round.
- Cleanup verified: zero leftover `sync-*` dirs in `$TMPDIR` after the run.

Test plan for PLAN_MULTI_STORAGE.md — expands its §8 table into concrete
scenarios, harness changes, and cleanup rules.

## 1. Simulation model

Storages are simulated with the **local-folder backend** (`kind: "local"`) —
each storage is just a directory, so a 2-storage world is two directories:

```
<TempDir>/myconfstorage/personal/     ← storage "Personal"
<TempDir>/myconfstorage/team/         ← storage "Team"
```

Everything (storage dirs + machine homes) lives under `tempfile::TempDir`,
exactly like the existing harness: deleted on drop **including on test
failure/panic**, honoring the `KEEP_SYNC_TEST_DIRS=1` escape hatch. That is
the cleanup guarantee — a fixed path like
`~/Desktop/project/myconfstorage` would leak on abort and collide across
parallel runs, so the literal path is reserved for the manual smoke recipe
(§6), which ends with an explicit `rm -rf`.

Local-first, dual-backend second: scenarios are written as `run_*` bodies per
suite convention. The `*_local` wrappers are the primary deliverable (they
need no server — just hand each storage its directory). The S3 wrappers need
**one stub server per storage** (two `stub_s3.rs` instances on distinct
ports, each with its own bucket dir); cheap, but land after local is green.

## 2. Harness changes (prerequisite)

- **v2 config builders on `Machine`:**
  ```rust
  machine.add_storage("Personal", StorageKind::LocalDir(dir)) -> storage_id
  machine.add_profile(".claude", mount /* "" = ~/.claude */) -> profile_id
  machine.link(profile_id, storage_id, pin: Option<&str>) -> link_id
  machine.push_link(link_id) / machine.pull_link(link_id)   // per-link ops (§6 of the plan)
  ```
  A compat builder (one storage, two default profiles, auto links)
  re-expresses today's `config_for(cloud)` so the existing suite migrates
  mechanically and stays green through step 1 of the work order.
- **`mounts` keyed by profile id, not root kind** — two `.claude` profiles
  on one machine make the current `HashMap<&'static str, PathBuf>` (from
  PLAN_SYNC_LINK_TESTS §2) ambiguous.
- **`TestCloud::at(dir)`** — one `TestCloud` per storage directory so
  assertions (`head`, `manifest_of`, `commit_chain`, `profiles_for_root`)
  read the right universe. Mostly exists; today's single-cloud helpers get a
  thin per-storage constructor.
- **Baseline path helper** — `machine.baseline_path(storage_id, profile)`
  computing `baselines/{storage_id}__{flattened profile id}`, so tests
  assert keying (decision 2b) directly instead of re-deriving the format.

## 3. Flagship scenario — S21 `s21_matrix_two_storages_three_profiles`

The scenario the feature exists for: multiple local profiles ⇄ multiple
storage points, one machine, one link matrix. Composes fan-out, same-name
isolation, and same-kind neighbors because those properties only get
interesting when they coexist; each also keeps a narrow follow-up scenario
(§4) so failures localize.

Matrix on machine M (storages = two local dirs under
`<TempDir>/myconfstorage/`, per §1):

| local profile | Personal | Team |
|---|---|---|
| c1 = `~/.codex` (default) | ✓ auto | ✓ auto |
| a1 = `~/.claude` (default) | ✓ pinned `001/.claude` | — |
| a2 = `myconf/.claude` (custom mount) | — | ✓ pinned `001/.claude` |

Seed distinct content per profile (distinct **lengths** — stat fast path).
`auth.json` seeded in every profile (never-sync tier must hold per link).

Walk + asserts:

1. **Push all four links.** Read each storage dir directly with
   `serde_json`:
   - Personal holds a `.codex` profile (c1's files) and `001/.claude`
     with a1's content; Team holds a `.codex` profile (c1's files,
     byte-identical to Personal's) and `001/.claude` with a2's content.
   - The two `001/.claude` heads/manifests **differ** — same name,
     unrelated profiles (decision 2a).
   - No `auth.json` in any manifest of either storage.
2. **Baseline keying (2b).** Exactly four baseline files, at
   `{storage_id}__{flat profile id}` — in particular the two `001/.claude`
   links have distinct baselines. No file at the bare v1 `{profile_id}`
   key.
3. **Fan-out staleness.** Edit a c1 file (length change), push **only**
   c1→Personal: Personal's codex revision bumps, Team's is unchanged.
   `get_file_statuses` with `link = c1→Team` reports the file pending;
   with `link = c1→Personal` reports synced (statuses are per link, §6 of
   the plan). Push c1→Team → revisions converge, "up to date" after.
4. **No cross-talk.** After all pushes: a1's distinguishing rel never
   appears in Team's `001/.claude` manifest, a2's never in Personal's, and
   a1/a2 file bytes on disk are untouched by each other's pushes.
5. **Storage as a self-contained universe.** Fresh machine N links only to
   Team (codex auto + `.claude` pinned `001/.claude` at the default
   mount). Pull both links → N receives c1's codex content and **a2's**
   claude content — a1's Personal-only content is nowhere on N.
6. **Per-profile agent-sync remap.** a1's remap dir is
   `~/.agent-sync/claude`, a2's is `~/.agent-sync/claude.{a2_id}`; assert
   the two resolved `Roots.agent_sync` paths differ and files written
   under each mount's logical `agent-sync/` land in (and restore from) the
   right physical dir — no cross-contamination between same-kind
   neighbors.

## 4. Narrow scenarios (renumbered from plan §8)

### S22 — `s22_unlink_relink_via_save_diff`

From S21's end state: save a config **without** the a1→Personal link
(`save_sync_config` is the one mutation API — decision 2c).

- That link's baseline + cloud cache are deleted; the other three baselines
  and both storage dirs are byte-untouched (orphan philosophy).
- Relink (save with the link back, same pin): first push re-verifies by
  hash — "up to date", revision unchanged, **zero conflict siblings**
  (the §3.2 "safe by construction" claim).
- Then edit + push works as a normal change.

### S23 — `s23_storage_identity_edit_scopes_cleanup`

v2 of `save_sync_config_scopes_state_to_destination` / S10:

- Change Personal's identity (repoint `local_dir` at a fresh empty dir):
  its probe flag cleared, its links' resolved cloud ids/labels cleared,
  their baselines deleted. **Team's links, baselines, and probe are
  untouched.**
- Push to repointed Personal → profiles created fresh in the new dir.
- Repoint back to the original dir → relink (pinned name for a1, discovery
  for c1) recovers, no duplicate profiles, no conflict storm.

### S24 — `s24_storage_removed_profile_removed`

The remaining two §3.2 diff rows:

- Save without storage Team: c1→Team and a2→Team links + baselines gone;
  Team's directory still contains its full profile layout (cloud data
  untouched); Personal's links still sync.
- Save without profile a2: its link/baseline gone; the `myconf/.claude`
  mount's files still on disk, untouched.
- Save without a starter profile: the row stays removed after reload, while
  its `~/.codex` or `~/.claude` directory and files remain untouched.

### Guard + unit tests

| test | proves |
|---|---|
| `duplicate_cloud_target_rejected` | two links (same storage, same root kind) resolving to one cloud prefix → error on save and at resolve time; nothing written |
| `config_v2_roundtrip_and_clean_break` | v2 round-trip (including an intentionally empty profile list); v1/garbage file → fresh defaults (two starter profiles, no storages, no links) — §3.1 |
| `roots_for_profile_unit` | per-profile `abs`/`rel` round-trip incl. remap (`claude` vs `claude.{id}`); mount-overlap and `~/.agent-sync`-overlap rejection; local-storage `local_dir` overlapping a mount rejected |

## 5. Cleanup contract

- Every storage dir, machine home, and mount lives in a `TempDir` owned by
  the test body — dropped (deleted) on success, failure, and panic. No new
  cleanup mechanism; this is the existing harness guarantee.
- `KEEP_SYNC_TEST_DIRS=1` keeps and prints all of them (`[keep] ...`),
  including the per-storage dirs — extend the existing keep-hook to label
  storages (`[keep] storage Personal: <path>`).
- After landing, re-verify the README's "no leftover `sync-*` dirs in
  `$TMPDIR`" spot-check once with the new suite.

## 6. Manual smoke recipe (not a test)

For eyeballing the real app against the matrix UI once the frontend lands:

```sh
mkdir -p ~/Desktop/project/myconfstorage/{personal,team}
# app: add two local-folder storages pointing at those dirs,
# add a custom .claude profile, link cells, push/pull per cell
rm -rf ~/Desktop/project/myconfstorage   # cleanup when done
```

Automated tests never touch this path.

## 7. Gotchas (inherited + new)

- Stat fast path: same-second edits must change length.
- `$HOME` is process-global: machines sequential, hold
  `harness::lock_env()`; multiple *storages* are fine concurrently-defined
  since they're plain dirs, but ops stay one link at a time (plan §11).
- Default `.claude` auto-profile noise: S21 uses `.claude` deliberately on
  both machines; N's default `.claude` mount is the pinned link's mount, so
  no stray auto-profile should appear — assert Team's profile count stays
  at 2.
- Commit counting via `TestCloud::commit_chain`, never by listing
  `_commits/`.

## 8. Order of work (each step ends green)

1. Harness: v2 builders + compat shim; existing suite green unchanged.
2. Guard + unit tests (§4 table) — they pin the config/roots layer the
   scenarios stand on.
3. S21 (the matrix) — most likely to find the 2b keying bug class.
4. S22 → S23 → S24.
5. S3-stub wrappers (second server instance) for S21–S24.
6. Update `sync_tests/README.md` coverage table + PLAN_MULTI_STORAGE.md §8
   renumbering.

## 9. Sizing

~10–12 new tests (4 scenarios × 2 backends + 3 units/guards); estimated
+4–6s runtime on top of the current suite.

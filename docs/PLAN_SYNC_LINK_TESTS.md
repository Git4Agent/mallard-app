# Plan: Integration Tests for Multi-Home / Multi-Link Combinations

Status: IMPLEMENTED 2026-07-07 (suite at 81 tests, ~13s). Notes:
- The harness `relocate(move_files: true)` copies with read+write (never
  `fs::copy`) to force fresh mtimes — macOS `fs::copy` can preserve times,
  which would let the stat fast path mask the sha-verification behavior
  S17 exists to prove.
- `TestCloud::manifest_of(profile_id)` / `profiles_for_root(root)` were
  added because multi-profile buckets make the one-per-root helpers
  ambiguous.
- All scenario expectations held; no product bugs surfaced this round.
Builds on: PLAN_SYNC_LINKS.md (implemented 2026-07-07) and
`src-tauri/src/sync_tests/README.md` (current suite: 66 tests, dual-backend).

## 1. Coverage gap analysis

Already covered by S1–S14 + scoping tests:

- one custom **codex** mount (S11), one pinned prefix shared by two machines
  (S12), pin creation/recreation at an exact name (S13), nested discovery of
  a single `001/.codex` (S14), per-scope state persistence.

Not covered anywhere:

- custom **claude** mounts (the harness cannot even express them today)
- more than one profile of the same root kind in one bucket
- the `>1 match — link one explicitly` auto-link error branch
- mount relocation on a machine with history (moved files / empty dir)
- pins pointing at a prefix that holds the other root kind
- repointing a pin between prefixes and back (baseline isolation)
- `get_file_statuses` and the editor boundary under custom mounts
  (both read mounts from the *saved* config — a path no scenario walks)

## 2. Harness generalization (prerequisite)

`Machine` currently supports only `codex_root: Option<PathBuf>`.

```rust
pub struct Machine {
    ...
    /// Per-root mount overrides, mirroring SyncConfig.codex_root/claude_root.
    mounts: HashMap<&'static str, PathBuf>,
}

impl Machine {
    /// Builder: Machine::new("A").mount(".codex", dir).mount(".claude", dir2)
    pub fn mount(self, root: &'static str, dir: PathBuf) -> Machine;
    /// Move the mount; optionally move the physical tree with it.
    pub fn relocate(&mut self, root: &'static str, new_dir: PathBuf, move_files: bool);
}
```

- `path(rel)` and `config_for(cloud)` consult the map (generalizes the
  current single-root special case; `Machine::with_codex_root` becomes a
  thin wrapper or is inlined into its two call sites).
- **Injected vs. persisted mounts:** push/pull take the mount from
  `config_for`; `get_file_statuses` / `read_file_content` load the *saved*
  config. Tests touching those must persist mounts through the real
  `set_sync_link` first — add `Machine::persist_links()` sugar or call
  `set_sync_link` explicitly in the scenario.
- `assert_converged(machines: &[&Machine], rels: &[&str])` — content
  equality across machines, used by every multi-machine scenario.

## 3. New scenarios (S15–S20, all dual-backend via `run_*` wrappers)

### S15 — Three homes, one bucket (isolation)

Machines: A ⇄ `001/.codex` (default mount), B ⇄ `001/.codex` (custom
mount), C ⇄ `002/.codex` (default mount). All in one bucket.

Assert:
- A push → B pull → B push → A pull converge through `001/.codex`.
- C pushes different files: only `002/.codex`'s revision moves; `001`'s
  revision unchanged (and vice versa for A/B pushes).
- Manifests are disjoint: C's rels never appear in 001's manifest, A/B's
  never in 002's.
- `bucket/001/.codex/_head.json` and `bucket/002/.codex/_head.json` both
  exist; no top-level hex profile for `.codex` was created.

### S16 — Namespace pairs + the multi-match error

- A pins **both** `001/.codex` and `001/.claude`, with custom mounts for
  both roots (first claude-mount coverage; two children under one
  namespace stresses two-level discovery beyond S14's single child).
- Fresh D (no links) pulls → auto-links both roots while exactly one
  profile per root exists; receives A's files.
- C creates `002/.codex` (pin + push).
- Fresh E pulls → must **fail** with the "N cloud profiles for .codex —
  link one explicitly" error (currently untested branch). Nothing linked,
  nothing written locally for `.codex`.
- E pins `002/.codex` → pull succeeds with C's data, not A's.

### S17 — Mount relocation (the data-safety scenario)

Variant 1 — files moved with the mount:
- A pushes from the default mount; `relocate(".codex", scratch, move_files: true)`;
  push again → **"everything up to date"**, revision unchanged, no
  conflict siblings. (Baselines are sha-based: mtime mismatch falls
  through to the hash check.)

Variant 2 — empty new mount:
- A `relocate(".codex", empty_dir, move_files: false)`; push → union
  semantics must NOT delete anything cloud-side; instead the full tree is
  restored *into the new mount* (`Missing` + cloud entry + baseline record
  → ApplyCloud), revision unchanged.
- Follow-up pull is a no-op; old default dir untouched.

### S18 — Mixed link shapes on one machine

Codex pinned (`001/.codex`) + custom mount; claude auto + default mount.
`push_all` / `pull` in both directions with a second machine.

Assert: hex auto profile (claude) and named pin (codex) coexist; scope
filtering keeps files strictly per root; the claude profile is top-level
hex, the codex one nested-named.

### S19 — Wrong-root pin fails loudly

Seed `001/.codex` via A. Machine W pins `.claude ⇄ 001/.codex`.

Assert: push and pull both fail with "holds .codex — cannot sync it as
.claude"; the store is unmodified (001's revision unchanged, no new
profiles); W recovers by re-pinning `.claude ⇄ 001/.claude`.

### S20 — Repointing a pin (baseline isolation)

- A syncs `001/.codex` at v1.
- Repoint to `002/.codex` → push publishes the full tree as a fresh
  profile (per-profile baselines: no bleed); `001` still holds v1.
- Edit files (different length! — see gotchas) → repoint back to `001` →
  push publishes the edit as a normal change: **zero conflict siblings**,
  `001` at revision+1.

## 4. Smaller targeted tests

| Test | Proves |
|---|---|
| `statuses_under_custom_mounts` | after `set_sync_link` + push + `refresh_cloud_state`, `get_file_statuses` labels paths under the custom mount correctly (saved-config mount path) |
| `editor_boundary_follows_mounts` | file under `/scratch/.codex` editable; file under the old default `~/.codex` is outside the mounts → read-only with a reason |
| `mount_name_is_cosmetic` | mount at `/scratch/work-codex` (no `.codex` suffix) still syncs the `.codex/…` logical namespace |

## 5. Gotchas to respect while writing these

- **Stat fast path:** edits within one mtime second must change file
  length, or they're invisible (documented in sync_tests/README.md).
- **`.claude` auto-profile noise:** any machine whose default `.claude`
  dir exists creates/links a claude profile on pull. Either seed claude
  intentionally or assert around it (see S11's saved-config comparison).
- **`$HOME` is process-global:** machines act sequentially; hold
  `harness::lock_env()` for the whole test body as everywhere else.
- **Saved vs. injected mounts** (see §2) for status/editor tests.
- Commands like `get_file_statuses` are private crate fns — call them
  directly with `machine.handle()`, as the scoping tests already do.

## 6. Order of work (each step ends green)

1. Harness generalization (`mounts` map, builder, `relocate`,
   `assert_converged`) — existing 66 tests must pass unchanged.
2. S15 (isolation) → S16 (multi-match) — the multi-home core.
3. S17 (relocation) — the data-safety scenario; most likely to find a bug.
4. S18–S20.
5. Small targeted tests (§4).
6. Update the backend-coverage table in `src-tauri/src/sync_tests/README.md`.

## 7. Sizing

Roughly 80–84 tests after landing (from 66); estimated runtime ~14–16s
(from ~9s). All new scenarios dual-backend — the wrapper pattern makes the
second backend nearly free and it has caught backend-specific bugs before
(the slashed-prefix baseline bug surfaced through a dual-backend scenario).

## 8. Out of scope (unchanged product limits)

- Multiple simultaneous links per root kind on one machine (one `.codex`
  dir at a time) — a product feature before it can be a test.
- Live-bucket (real R2) smoke testing — still the standing gap for real
  TLS/ETag behavior; nothing here substitutes for it.

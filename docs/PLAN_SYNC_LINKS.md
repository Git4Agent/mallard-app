# Plan: User-Defined Sync Links

Status: IMPLEMENTED 2026-07-07 (all seven steps; suite at 66 tests).
Deviations from the plan as written:
- The local-store/root overlap check is a hard error in `make_store`, not a
  warning (there was no logging path there, and failing loudly is kinder
  than junk syncing).
- `baseline_path` flattens `/` in profile ids to `__` — slashed prefixes
  otherwise broke baseline persistence silently (found by S14).
- `read_file_content`/`write_file_content` now take the AppHandle to load
  the mount table; the JS call sites are unchanged (Tauri injects it).
Superseded in part by PLAN_FRESH_ROOT.md (2026-07-08): custom mounts now
use container semantics — a folder not named after the root hosts it as a
subdirectory — replacing the "mount name is cosmetic" rule below.
Superseded in part by PLAN_MULTI_STORAGE.md (2026-07-14): §9's "one link
per root kind" limit is lifted — links are now (local profile × storage)
matrix edges; `codex_root`/`claude_root` became `LocalProfile.path`, the
pinned cloud side lives on each link, and `set_sync_link` was absorbed
into `save_sync_config`. The logical-namespace decision (§2) is unchanged
and is what makes multi-storage fan-out work.
Builds on: DESIGN2.md (profile layout, head CAS, union reconciliation) and
`src-tauri/src/sync_tests/README.md` (dual-backend test suite, 53 tests).

## 1. Goal

A **sync link** pairs a local directory with a cloud location, per agent
root, and push/pull operate on that pair:

```
~/.codex      ⇄  001/.codex     (default local side, named cloud side)
/tmp/.codex   ⇄  001/.codex     (custom local side, same cloud profile)
~/.claude     ⇄  <auto>         (today's behavior — nothing set)
```

Both sides are optional and independent:

| Side | Meaning | Default | Stored as |
|---|---|---|---|
| local | where THIS machine keeps the root | `~/.codex`, `~/.claude` | per-machine config field |
| cloud | which profile prefix in the store | auto-discovered / random id | on the `ProfileLink` (per-remote) |

Two machines pointing different local dirs at the same cloud prefix sync the
same profile with full union/merge/conflict semantics — that is the feature.

## 2. The load-bearing design decision: logical namespace

Everything inside a profile — manifest keys, baselines, allowlist tiers,
merge drivers, conflict-sibling names — speaks **logical paths**
(`.codex/memories/notes.md`). This never changes, regardless of either side
of the link:

```
logical:   .codex/memories/notes.md
machine A: ~/.codex/memories/notes.md        (default local side)
machine B: /tmp/.codex/memories/notes.md     (custom local side)
cloud:     001/.codex/_uploads/{id}/files/.codex/memories/notes.md
```

Consequences (all for free):
- `AGENT_SYNC_FILE_SETS.md` tiers, `jsonl_merge_driver`, `conflict_copy_rel`,
  `relative_path_is_included` — untouched (all keyed on logical paths).
- Existing profiles and baselines keep working; relocating a local root later
  is safe (baselines store shas; a moved-but-identical tree re-verifies as
  synced through the hash path, one slower first scan).
- Cross-rooted machines converge because the cloud never sees physical paths.

## 3. State classification (interacts with the destination-scoping fix)

`save_sync_config` drops per-remote state (profiles, probe flag) when
`remote_identity()` changes. Placement here must respect that:

- `codex_root` / `claude_root` (local side): **per-machine** — excluded from
  `remote_identity()`, survives destination switches. A machine's `.codex`
  does not move because the bucket changed.
- cloud prefix + `pinned` (cloud side): live on `ProfileLink` inside
  `config.profiles` — **per-remote**, correctly cleared on destination
  switch.

## 4. Part 1 — local side (`Roots`)

### 4.1 New struct

```rust
/// Per-machine mount table: logical root -> physical directory.
struct Roots {
    codex: PathBuf,   // default: home/.codex
    claude: PathBuf,  // default: home/.claude
}

impl Roots {
    /// Reads codex_root/claude_root overrides; validates (see 4.3).
    fn from_config(config: &SyncConfig) -> Result<Roots, String>;
    /// ".codex/foo" -> <codex>/foo. Logical rel -> physical path.
    fn abs(&self, rel: &str) -> PathBuf;
    /// Physical path -> logical rel; None if outside both roots.
    fn rel(&self, path: &Path) -> Option<String>;
    /// ".codex" -> &self.codex (root presence checks, tree building).
    fn dir(&self, root: &str) -> &Path;
}
```

### 4.2 Refactor (pure, no behavior change — gate: all 53 tests green)

Replace `home: &Path` threading with `roots: &Roots` at every site (~34).
Inventory of functions whose signature or body changes:

- entry points resolving `dirs::home_dir()` (7): `do_upload_s3`,
  `do_download_s3`, `get_file_statuses`, `list_config_dirs`,
  `read_file_content`, `write_file_content` (+ editor helpers)
- rel/abs mapping: `relative_to_home` (replaced by `Roots::rel`),
  every `home.join(rel)` (→ `roots.abs(rel)`)
- scan/selection: `collect_upload_files`, `path_is_included`,
  `dir_path_may_contain_included`, `build_tree`, `read_source`
- apply path: `apply_cloud_bytes`, `apply_cloud_file`, `resolve_cloud_bytes`,
  `backup_local_file`
- status: `matrix_status` callers, `local_state_at` callers
- engine: `reconcile_with_cloud`, `push_profile` (baseline `file_record`
  paths)
- editor safety: `validate_editable_path` (must accept paths under either
  root, reject outside)
- root presence: `home.join(root).exists()` in `do_download_s3` →
  `roots.dir(root).exists()`

`Roots::from_config` is constructed once per command from the runtime config
(push/pull) or the saved config (editor/status commands).

### 4.3 Config + validation

```rust
// SyncConfig (per-machine fields; NOT part of remote_identity())
#[serde(default, skip_serializing_if = "String::is_empty")]
codex_root: String,    // "" = ~/.codex ; else absolute dir used as-is
#[serde(default, skip_serializing_if = "String::is_empty")]
claude_root: String,   // "" = ~/.claude
```

Rules (enforced in `Roots::from_config`, mirrored at save time):
- override must be an absolute path; used **as-is** (no `.codex` appended —
  what you type is what syncs)
- the two roots must not be equal or nested inside each other
- warn (log, non-fatal) when local-mode `local_dir` overlaps a root
  (syncing the store into itself)

### 4.4 UI

Sidebar source label shows the real path when overridden
(`/tmp/.codex` instead of `~/.codex`) — `read_source` label change.

## 5. Part 2 — cloud side (nameable, pinned profiles)

### 5.1 Profile-prefix grammar (`validate_profile_id`)

Extended to accept user-chosen names while keeping every existing random
hex id valid:

- 1 or 2 segments separated by `/` (e.g. `001` or `001/.codex`)
- segment charset `[a-z0-9._-]+`; a segment must not start with `_`
  (reserved) and must not be `.` or `..`; a leading `.` is allowed only so
  the second segment can literally be `.codex`/`.claude`
- total length ≤ 128
- rejected: backslashes, colons, control chars, empty segments

The rest of the key machinery (`profile_key`, head/manifest/upload keys)
already string-joins prefixes and handles `/` transparently.

### 5.2 Create-at-name

`create_profile_cloud` gains an explicit-prefix mode:

- `None` (today): random 16-byte hex id, 3 collision retries
- `Some(prefix)`: single attempt at that exact prefix; the existing
  put-if-absent head CAS decides — occupied prefix fails loudly with
  "already exists — link it instead" (resolve handles try-link-first, so
  users only see this on a genuine root-kind mismatch or race)

### 5.3 Pinned links

```rust
// ProfileLink gains:
#[serde(default)]
pinned: bool,   // true = user chose this cloud prefix explicitly
```

`resolve_profile_for_root` decision table (replaces the current
verify-or-rediscover from the S10 fix, which remains the unpinned branch):

| link state | head exists | action |
|---|---|---|
| pinned | yes | validate `head.root` matches kind → use |
| pinned | no | **create at that exact prefix** (user named it) |
| unpinned | yes | use (today) |
| unpinned | no | S10 heal: rediscover by root, else create random (today) |
| none | — | discover by root; 0 → create random; 1 → link; >1 → error (today) |

### 5.4 Two-level discovery

`discover_profiles` today reads `{top_prefix}/_head.json` only. Add one
nested pass: for each top-level prefix WITHOUT a readable head, list its
children and probe `{p1}/{p2}/_head.json`. Depth capped at 2. Applies to:

- `Store::list_top_prefixes` gets a sibling `list_child_prefixes(prefix)`
  (S3: `list_objects_v2` with `prefix="{p1}/"` + `delimiter="/"`; Local:
  `read_dir`)
- test harness `TestCloud::profiles_by_root` (fs walk, same 2-level rule)

Ensures `001/.codex` appears in `list_sync_profiles` and participates in
unpinned auto-link discovery.

### 5.5 `set_sync_link` command

```rust
#[tauri::command]
async fn set_sync_link(app, root: String, local_dir: String, cloud_prefix: String)
    -> Result<(), String>
```

- validates `root` ∈ allowed roots, `local_dir` per 4.3, `cloud_prefix` per
  5.1 (both may be empty = default/auto)
- persists `codex_root`/`claude_root` (per-machine side) and upserts a
  pinned `ProfileLink { root, profile_id: cloud_prefix, pinned: true, .. }`
  (per-remote side); empty `cloud_prefix` clears the pin (reverts to auto)
- deliberately network-free: profile creation/linking happens lazily at the
  next push/pull through 5.3 — "once the link is set, push/pull happen
  between them"

### 5.6 UI

Settings gains a "Sync links" section, one row per root:

```
.codex   [ local: ~/.codex          ]  ⇄  [ cloud: auto        ]
.claude  [ local: /tmp/.claude      ]  ⇄  [ cloud: 001/.claude ]
```

Plain text inputs (placeholders `~/.codex` / `auto`), saved with the
settings form via `set_sync_link`. The existing profile box keeps showing
the resolved link + revision. `types.ts`: `codex_root?`, `claude_root?`
on `SyncConfig`; `pinned?` on `ProfileLink`.

## 6. Tests (dual-backend via the existing `run_*` wrapper pattern)

| Test | Proves |
|---|---|
| `roots` unit tests | `abs`/`rel` round-trip; equal/nested-root rejection |
| grammar unit tests | `001`, `001/.codex`, hex ids accepted; `_x`, `a//b`, `..`, `A/b:c` rejected |
| S11 (both backends) | A on `~/.codex`, B on custom local dir → same auto profile; push/pull/conflict roundtrip; files land under B's custom root |
| S12 (both backends) | A pins `001/.codex` → push creates head literally at `bucket/001/.codex/_head.json`; B pins same prefix with different local dir → pulls A's files; divergent edits → standard conflict sibling |
| S13 | pinned prefix whose head was deleted → recreated at same prefix (not rediscovered); unpinned stale link → S10 heal unchanged |
| discovery test | nested `001/.codex` profile found by unpinned auto-link and `list_sync_profiles` |
| `set_sync_link` test | persists both sides into the right scopes; destination switch clears pin but keeps local roots |

Harness changes: `Machine` gets optional per-root local dirs (push/pull
inject them into the cloud config; `seed`/`read` map logical rels through
the same mount rule). Existing 53 tests must pass unchanged throughout.

## 7. Work order (each step ends with a green suite)

1. `Roots` struct + pure refactor, defaults only — **the risk step; the
   53-test suite is the safety net** (same playbook as the `Store` refactor)
2. `codex_root`/`claude_root` config + validation + harness support + S11
3. Grammar relaxation + create-at-name + `pinned` resolve table + S13
4. Two-level discovery (both Store backends + harness) + discovery test
5. `set_sync_link` command + its test
6. UI (types.ts, SyncPanel "Sync links" section, sidebar labels) — surgical;
   re-read the user-maintained frontend before touching
7. Docs: sync_tests/README.md, DESIGN2.md implementation-status note, memory

## 8. Compatibility

- Old configs (no new fields) behave byte-for-byte like today: serde
  defaults are `""`/`false`, `Roots` falls back to `~/.codex`/`~/.claude`,
  unpinned links resolve exactly as now.
- Existing random-hex cloud profiles remain valid under the relaxed grammar.
- No cloud-schema change: `CLOUD_SCHEMA_VERSION` stays 1 (prefix naming is
  outside the schema; heads/manifests/commits unchanged).

## 9. Limits (deliberate, say-so-if-wrong)

- **One link per root kind.** Syncing two different `.codex` dirs at once is
  a separate multi-link feature; the per-root uniqueness invariant
  (`upsert_profile_link`) stands.
- Renaming a cloud prefix = new profile. No server-side move; the old
  prefix stays as recoverable data (same orphan philosophy as DESIGN2).
- A pinned prefix occupied by the other root kind fails loudly at
  link/resolve time; namespaces never mix.
- Machine-scope caveat: `$HOME`-independent local dirs (e.g. `/tmp/.codex`)
  are trusted as typed; multi-user shared paths are the user's call.

# PLAN: Settings "Reset Mallard" — wipe ~/.mallard, factory-reset, reload

Status: IMPLEMENTED (2026-08-21, uncommitted). tsc, clippy `-D warnings`,
rustfmt, 211 desktop + 306 infrastructure Rust tests, and all 225 frontend
integration tests green. Follow-up to `PLAN_GLOBAL_SETTINGS_PAGE.md`.

One sentence: a sixth Settings group — **Local data** — whose one row
permanently deletes everything under `~/.mallard` (backups and logs included —
user-set), clears every `mallard.*` localStorage key (full factory reset —
user-set), re-arms the two long-lived backend services, and reloads the
webview — the process stays alive.

Revision (same day): the first implementation relaunched the process via
Tauri's core `request_restart` and **failed in the field with a white
window** — see Rejected designs. The cutover is now an in-place reload.

## Shape

- `LocalStateRepository::local_state_summary()` — lock-free measured readout
  (files, bytes, backups bytes; symlinks count as themselves) and
  `wipe_local_state()` in `crates/mallard-infrastructure/src/persistence.rs`.
  Wipe order is load-bearing: `.mallard` file-name check → missing-root check
  **before any lock** (the guard files live inside the root; lock-first would
  recreate state and break idempotence) → `acquire_host_operation()` (try-lock
  — a running push/pull fails the reset fast) → `persistence_guard` → measure
  → `remove_dir_all` with a bounded retry. Nothing recreates the root; the
  next repository construction does, lazily.
- `src-tauri/src/app_reset.rs`: `get_local_state_summary` and
  `reset_local_state`. The wipe also holds the catalog-state guard via the
  new `CatalogStateRepository::acquire_state_guard()`
  (`src-tauri/src/storage_catalog/persistence.rs`); lock order
  **catalog-state → host operation → persistence**, the order
  `clear_storage_data` established — inverting risks a flock deadlock.
- After a successful wipe, still inside the command, the only two long-lived
  holders of pre-wipe backend state are re-armed: the session watch service
  is cleared (`SessionWatchService::reset_all_caches` — its caches key on
  registrations that no longer exist) and the managed `ConfigWatcher` is
  replaced with a fresh one (its directory watch died with the deleted root;
  starting the replacement recreates the empty root, accepted). Everything
  else backend-side is constructed per call, and the two process-wide content
  caches (`object_cache`, transcript analyses) are content-addressed and
  immortal-correct.
- Activity log: `system.local_state_reset_started` (warning) before,
  `system.local_state_reset_failed` on error. **No persisted success entry**
  — it would recreate the `~/.mallard/logs` the reset just deleted; the next
  persisted entry is whatever the fresh state does first.
- Frontend: `SettingsPage.tsx` Local data group (icon `drive`), zero new CSS —
  readout + `btn btn-ghost danger`, error callout with Retry, reloading note.
  Flow: warning confirm (sizes incl. backups when known, "This cannot be
  undone.") → await `resetLocalState` → on success `clearStoredUiState()`
  (new `preferences.ts` export: prefix sweep of `mallard.*` + the abandoned
  `agent-sync-theme`) → "reloading" phase → `window.location.reload()`.
  The button stays enabled when the size walk failed: a user whose state is
  too broken to measure is exactly who needs the reset.

## IPC surface

| command | caller |
|---|---|
| `get_local_state_summary` | SettingsPage `loadLocalSummary` |
| `reset_local_state` | SettingsPage `resetMallard` |

## Rejected designs

- **Process restart via `request_restart` (shipped briefly, failed in the
  field)** — relaunching the raw debug binary orphans the app from
  `tauri dev`: the CLI kills its Vite server the moment the original process
  exits, so the relaunched webview loads `devUrl` from a dead port and shows
  a white window (backend healthy — it logged `system.app_started` into the
  fresh root; only the frontend had nothing to load). Verified from a live
  repro: orphaned `target/debug/mallard` process + recreated `~/.mallard`
  + nothing listening on 1420. Replaced by in-place reload + explicit
  re-arm, which behaves identically in dev and production.
- **Restart inside the wipe command** — `request_restart` there races process
  exit against IPC response delivery, so the wipe promise may never resolve
  and the frontend loses its slot to clear localStorage. (Superseded by
  dropping process restart entirely.)
- **Persisted success log entry** — recreates the just-deleted logs dir.
- **Move to Trash / keep backups / keep UI prefs** — user chose the full,
  permanent reset on all three axes.

## Known edges (accepted)

- The root does not stay absent: re-arming the config watcher (and any later
  backend call — `from_user_home` runs `ensure_root`) recreates it empty.
  Cosmetic; still a fresh install.
- After our lock files are unlinked mid-delete, another process could recreate
  entries; bounded by the `remove_dir_all` retry, and old data cannot
  resurrect.
- Reset during an active sync fails fast with the backend-worded
  "another host-sensitive operation is already in progress".
- macOS/Unix assumption: deleting flocked open files is legal; Windows would
  need a different hold-locks-while-deleting design.

## Tests

- `persistence.rs`: summary-ignores-symlink-targets; wipe → root gone →
  reopen fresh (empty config, new host identity); idempotent on missing root
  (same instance — pins the check-before-lock order).
- `app_reset.rs`: `reset_local_state_with_repositories` clears catalog state
  too, both repositories reconstruct.
- `settings-page.integration.test.tsx`: heading list gains "Local data"
  (regex widened for two-word headings); danger row renders enabled with `…`
  readout in static render; copy pins (backups, preferences, escaped
  apostrophe); a "Reset Mallard wipes…" test pins the confirm kind, the
  resolve→sweep→reload order, the two api invokes, the catalog guard, the
  re-arm calls, the absence of any `restart_app`/`request_restart`, the
  no-success-log rule, and the lib.rs registration. Pinned counts (2 selects,
  3 switches) unchanged; css-hygiene untouched (zero new CSS).

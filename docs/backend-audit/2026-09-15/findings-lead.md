# Lead pass — baseline and hygiene

Snapshot: HEAD `f226be4` + uncommitted working tree, frozen 2026-09-15T09:42Z
(169 `.rs` files hashed; the snapshot lives in the session scratchpad).

## Verification baseline

| Command | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | exit 0, no warnings, 9.2 s (incremental) |
| `cargo clippy --workspace --all-targets` | exit 0, **79 distinct warnings** (09-01: 0, 09-04: 2) |
| `cargo clippy --workspace` (production targets only) | **37 distinct warnings** |
| `cargo test --workspace --lib --no-fail-fast` | **1,066 passed, 1 failed, 11 ignored** |

Per crate: `mallard` (src-tauri) 326 passed / **1 failed** / 6 ignored;
`mallard-infrastructure` 605 / 0 / 5; `mallard-domain` 86 / 0 / 0;
`mallard-cli` 28 / 0 / 0; `mallard-application` 21 / 0 / 0.

A plain `cargo test --workspace --lib` stops at the first failing crate and
reports only `326 passed; 1 failed` — the other four crates never run. Use
`--no-fail-fast` for a baseline.

### The failing test

`sync_tests::thread_variants::storage_parity::managed_cloud_noop_fast_forward_views_and_stale_writer_match_local_protocol`
panics at `src-tauri/src/sync_tests/thread_variants/storage_parity.rs:671`:

```
assertion failed: state.uploads.values().all(|upload| upload.state != "open")
```

After the stale-writer scenario, an upload is left `open` on the stub server.
The test file was last written 2026-09-14 21:53; `managed_cloud/client.rs`,
`store.rs` and `stub.rs` were all edited later (2026-09-15 01:22). Root cause:
see the Managed Cloud lane (`findings-managed-cloud-store.md`).

### Clippy by lint (all targets)

21 `needless_borrow`, 18 `unnecessary_to_owned`, 11 `too_many_arguments`,
10 `cloned_ref_to_slice_refs`, 5 `needless_borrows_for_generic_args`,
4 `nonminimal_bool`, 3 `empty_line_after_doc_comments`, 7 singletons.
By file: `service.rs` 23, `live_codex_aimock_e2e.rs` 10, `live_codex_e2e.rs` 9,
`conversation_materialization.rs` 8, `claude/history.rs` 4, others ≤ 3.
Production `too_many_arguments` sites: `service.rs:1333, 3799, 4357, 10705,
10897`, `owned_subagents.rs:54, 188`, `codex_standalone_builder.rs:262`,
`r2_setup.rs:492`, `mallard-application/src/dropbox_setup.rs:17`.

## Hygiene findings

### LEAD-1 — Low: `ManagedCloudClient::list_members` has no caller
`crates/mallard-infrastructure/src/managed_cloud/client.rs:707`. A `pub async fn`
with zero references anywhere in the workspace, tests included — speculative
surface in code that has not shipped yet (AGENTS.md: no speculative
abstractions). Delete it, or wire it when a Members view exists.

### LEAD-2 — Info: `prepare_pull_transcript` is a test-only entry point
`crates/mallard-infrastructure/src/pull_transcript.rs:77-88`. It is a thin
wrapper that calls `prepare_pull_transcript_with_identities` with an empty
identity map. Production calls the `_with_identities` form directly
(`src-tauri/src/project_sync/service.rs:12652`); the only caller of the wrapper
is the in-file test helper at `:471`. So the module's unit tests only ever run
the empty-map case, and the child-identity remapping path is covered solely by
the service-level suites. Same shape as 09-04 O3, milder: the wrapper adds no
logic. Either drop it and have the tests pass `&BTreeMap::new()`, or add one
unit case with a non-empty identity map.

### LEAD-3 — Info: the loopback-receiver move is split across staged and untracked
`crates/mallard-infrastructure/src/dropbox_browser.rs` is **staged** as deleted
(`D `), while its replacement `loopback_browser.rs` is **untracked** and the
`lib.rs` change that swaps `pub mod dropbox_browser;` for
`pub mod loopback_browser; pub mod managed_cloud;` is **unstaged**. A plain
`git commit` of the index would produce a commit whose `lib.rs` still declares
`dropbox_browser` with the file gone — a broken build at that commit. Stage the
three together (plus the other untracked Managed Cloud files) when committing.

### Checked and clean
- No dangling code reference to `dropbox_browser` (the only hits are the stable
  error code string `"dropbox_browser_failed"` at `dropbox_auth.rs:36` and an
  unrelated `mod dropbox_browser_setup` inside the CLI example).
- The five Managed Cloud Tauri commands are registered
  (`src-tauri/src/lib.rs:184-188`) and their control state is managed (`:125`).
- `DropboxAuth::for_test` is gated `#[cfg(any(test, feature = "test-support"))]`
  (`dropbox_auth.rs:278`).
- The `portable::contracts::tombstone_*` constructors are live (called from
  `bundle_engine.rs:3754, 5407, 5426, 5428, 5443`).
- Transcript preview truncation cannot split a UTF-8 character: both
  `bounded_text` copies walk `end` back to `is_char_boundary` before slicing
  (`providers/codex/history.rs:2552-2556`, `providers/claude/history.rs:1565-1569`).
  No other computed `&str` slice in the history/session-detail/subagent readers.
- Git remote URLs cannot leak an embedded token: `normalize_github_repository_url`
  (`project_activity.rs:1738-1790`) takes the host after the last `@`, accepts only
  `github.com`, and rebuilds `https://github.com/{owner}/{repo}` from two validated
  segments; the value is used for display (activity header, session detail).
- External process launches are argv-based or correctly quoted (wrangler is
  covered by the setup lane):
  - `providers/launch.rs:3-28` — every Terminal command component goes through
    POSIX `shell_quote` (`'…'` with `'\''`), and the whole line is escaped for
    the AppleScript literal (`\` then `"`) before `do script`.
  - `providers/codex/actions.rs:194-210` — `terminal_resume_command` validates the
    thread id as a UUID and requires absolute paths before quoting;
    `run_codex_app_deep_link` (`:91-111`) and `providers/claude/actions.rs:55-127`
    pass `/usr/bin/open` an argv, never a shell string; `resolve_codex_cli`
    (`:165-192`) runs a fixed `command -v codex` and canonicalizes the result.
  - `codex_binary.rs:136-185` — `find_binary` restricts the name to
    `[A-Za-z0-9_-]` before interpolating it into `$SHELL -lc "command -v …"`.
  - `codex_app_refresh.rs:192-391` — bundle validated (no symlinks, canonical,
    plist bundle id + executable match, executable inside the bundle); osascript
    gets only a constant bundle id; every command is time-bounded; early exits are
    reaped.
  - `storage_catalog/adapters.rs:106-150` — `NativeProcessRunner` uses argv, a
    timeout with kill + wait, and bounded pipe readers. Whether its `argv` can be
    steered by remote plugin-intent data is delegated to the catalog lane.
- The three production `git` invocations (`project_activity.rs:1805-1812`,
  `session_detail.rs:1548-1555`, `service.rs:2798-2804`) pass the project root as
  the value of `-C` and fixed subcommands only; commit ids read back are checked
  as 40/64 hex before use (`project_activity.rs:1801`, `session_detail.rs:1544`).
- The only uncommitted dependency change is reqwest gaining the `multipart`
  feature (`crates/mallard-infrastructure/Cargo.toml:34`), which adds
  `mime_guess 2.0.5` and `unicase 2.9.0` to `Cargo.lock`; the loopback `hyper`
  server stays behind `test-support`.
- No production code writes git hooks into user repositories.
- The bundled `mallard-cli` cannot Push, Pull or apply anything: its surface is
  `storage list | verify <id> | r2 setup | r2 accounts | r2 discover`
  (`crates/mallard-cli/src/args.rs:21-57`). `r2 setup` fills an existing open
  draft pinned to `--expected-revision` (`:104-116`), so it cannot bypass a
  reviewed plan or race a form edit silently; `r2 discover` is read-only.

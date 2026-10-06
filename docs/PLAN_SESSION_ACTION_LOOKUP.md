# Session action lookup plan

Status: implemented 2026-10-06, uncommitted; the live Tauri pass (Open in
Terminal on a large Codex home) is open. Date: 2026-10-06.

Open a session in Terminal or in the app, or delete it, without first
inventorying the whole provider home. Locate the one requested session by the
identity its file name carries, read only its own transcript head, and keep
every ownership check the inventory applies to that session.

**1. Current behavior and problem**

The three session actions share one resolver. Before the Terminal window
opened, it ran the full `discover` of the bound provider home and then kept
one row of the result.

| Step of `discover` | Codex | Claude |
| --- | --- | --- |
| Walk | `sessions/` and `archived_sessions/`, every date folder | `projects/`, every project directory |
| Head read | Every rollout, to learn its thread id and `cwd` | Every transcript, tail window first, to learn its leaf session id and `cwd` |
| Grouping | Rollouts grouped by header thread id, then current-rollout selection | None |
| Extra reads | `session_index.jsonl`, for titles | `file-history/` and `todos/` walks for sidecars, `memory/` for project memory |
| Kept by the action | One session id and one path | One session id and one path |

The cost grew with the whole home, not with the project, and none of the
extra reads changed which session was opened.

Source: [resolver](../crates/mallard-infrastructure/src/project_activity.rs),
[Codex discovery](../crates/mallard-infrastructure/src/providers/codex/discovery.rs),
[Claude discovery](../crates/mallard-infrastructure/src/providers/claude/discovery.rs),
[rollout selection](../crates/mallard-infrastructure/src/codex_rollout_selection.rs).

**2. Decisions**

| Question | Decision |
| --- | --- |
| Where does the targeted lookup live? | One adapter method, `locate_session`, beside `discover`. The core passes the identity test; the adapter owns the layout. The core never learns a file-name grammar. |
| How is the requested session named without a conversation binding? | The thread id is a hash of the provider session id, so it cannot be inverted. The adapter offers each candidate (id, current path) from file names alone and the core applies the existing alias rules to each. No transcript is opened by the test. |
| Codex identity | Native file names: `rollout-<stamp>-<thread>[_<physical>].jsonl`. Candidates are grouped by the thread the name claims; `select_current` picks the current rollout by name order, as Push already does. A header that names another thread is an error, not a different session. |
| Claude identity | The file stem is the session id, as deletion and the corpus oracle already require. The same stem in a foreign project directory is passed over once its head names another folder. |
| What stays | `cwd` must equal the project root; subagents and empty sessions stay invisible; the per-file size, symlink, hard-link and denied-name inspection still fills `blocked_reason`. These cost one stat on the located file. |
| What goes, for actions only | Head reads of unrelated transcripts, session titles, sidecar and memory walks, inventory warnings. `discover` itself is unchanged for Activity, Push and project-wide deletion. |
| Providers without a name grammar | The trait default discovers and selects, so the test provider is unchanged. |

**3. Change**

- `providers/traits.rs`: `locate_session(context, is_requested)` with the
  discover-and-select default.
- `codex_rollout_selection.rs`: `logical_id`, the thread a native name claims.
- `codex_subagents.rs`: `RolloutLookup::rollouts_by_thread`, the bounded
  filename walk grouped by that thread.
- `providers/codex/discovery.rs` and `providers/claude/discovery.rs`: the two
  `locate_session` implementations.
- `project_activity.rs`: `resolve_action_session` calls `locate_session` with
  the binding comparison or the alias rules as the test.

**4. Verification**

Tests prove the read count with the sync metrics counters: one head read for
the requested session, none for a thread no name claims.

- `providers::codex::discovery::tests::locate_session_opens_only_the_requested_rollout`
- `providers::claude::discovery::tests::locate_session_opens_only_the_requested_transcript`
- `project_activity::activity_only_tests::action_session_is_located_without_a_binding_from_its_own_rollout_alone`

```
cargo test --manifest-path src-tauri/Cargo.toml -p mallard-infrastructure --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Open: the live pass. Open a Codex session in Terminal from a home with
thousands of rollouts and confirm the window opens without the pause.

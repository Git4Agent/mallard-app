# Design: Place each session on one commit by its last prompt

- Status: Accepted
- Last reviewed: September 28, 2026
- Compatibility: clean cutover

## Problem

Activity used to place a session under the first first-parent commit at or
after its **last activity**. A session whose final turn made the commit ("commit
and push this") replied after that commit, so it landed on the *next* commit, or
under **Uncommitted** when no later commit existed.

An interim design listed a session under every commit it spanned. Reviewers
found one session appearing under several commits confusing, so it was
replaced by the rule below.

## Decision

### Mapping rule

Each session is placed on **exactly one** first-parent commit of the selected
branch: the **closest commit at or after the session's last user input**
(`last_prompt_at`).

- The chosen commit is the one with the smallest `committed_at` that is at
  least `last_prompt_at`. If two commits share that second, the older one on
  the rail wins.
- When no commit follows the last input, the session is listed only under
  **Uncommitted** (the `unmapped` list).
- A session whose transcript records no input is placed by its `ended_at`.
- Every session appears exactly once: under one commit, or under
  Uncommitted.

Placement uses time only. The provider-recorded SHA and branch are provenance
shown in session detail. They never move a session, and they never control
visibility, Push, or Pull.

Session detail's commit marker (`checkpoint_support`) uses the same rule and
time, so the detail names the same commit as the list.

Examples, with commits at 20:43 and 21:58:

| Session | Last input | Last reply | Placed under |
| --- | --- | --- | --- |
| "commit and push" | 20:42 | 20:44 | 20:43 (the commit it made) |
| Began 20:34, kept working | 21:40 | 21:47 | 21:58 |
| Began 20:30, still going | 22:01 | 22:06 | Uncommitted |

### Parsing the last input from the JSONL transcript

`last_prompt_at` is the **latest `timestamp`** among the records that count as
a person's input. Timestamps are RFC 3339 UTC, for example
`2026-09-28T04:58:40.772Z`. `timeparse::parse_timestamp_value` parses them to
Unix seconds, and they are compared with Git's `%ct`, which is also Unix
seconds, so time zones do not matter.

Only the time is kept; the input's text is never stored. The same records
also give the first prompt (`started_at`) and the last reply (`ended_at`),
which drive the displayed time and duration.

**Codex rollout** (`~/.codex/sessions/**/rollout-*.jsonl`)

Each line is an envelope: `{"timestamp", "type", "payload"}`. Inputs come from
one source, the same source the prompt count uses:

1. **Canonical items**, used when the file has any: `type:"event_msg"`,
   `payload.type:"item_completed"`, `payload.item.type:"UserMessage"`, with
   the text in `item.content[].text`.
2. **Otherwise the event view:** `payload.type:"user_message"`, with the text
   in `payload.message`.

Excluded:

- `response_item` `{type:"message", role:"user"}`. This is the model-input
  copy, and it includes injected context such as `<environment_context>` and
  AGENTS.md blocks.
- Empty text.
- Text starting with `<environment_context>`, `<recommended_plugins>`,
  `<skill>`, `<permissions instructions>`, or `<app-context>`.

**Claude** (`~/.claude/projects/<dir>/<session>.jsonl`)

A `type:"user"` record (`{uuid, timestamp, message:{role, content}}`) is a
person's input only when all of these hold:

- it is not `isMeta`, `isSidechain`, `isSynthetic`, `isCompactSummary`, or
  `shouldQuery:false`;
- it has no `parentToolUseID`;
- it has no `tool_result` block, since tool results are also `user` records;
- `origin.kind` is `"human"`, when present;
- its text is non-empty.

Such a record is then one of two kinds:

- **A prompt.** This is text that does not start with `<system-reminder>`,
  `<local-command-caveat>`, `<command-name>`, `<command-message>`,
  `<local-command-stdout>`, `<ide_opened_file>`, or
  `[Request interrupted by user` (the marker Esc writes). Each `uuid` counts
  once. Prompts are what the Prompts count and the timeline show.
- **A slash command.** This is text starting with `<command-name>` or
  `<command-message>`. A typed slash command counts as the last input for
  mapping, but it is not a prompt in the counts.

The interruption marker and the compaction summary no longer count as
prompts, in the counts or in Claude's timeline.

Versions stay at 1: the activity cache schema and Claude's
`TIMELINE_PARSER_VERSION` are unchanged. Following the schema-1 clean
cutover, a cache entry is rebuilt only when its transcript's size or
modification time changes. A summary cached before this change has no
`last_prompt_at` and is placed by its `ended_at` until then. Start from fresh
Mallard state to reparse every session at once.

### Where the value lives

`last_prompt_at` is carried by:

- `SessionFileSummary`, the machine-local activity cache;
- `ProjectSessionSummary`, the Activity DTO;
- the portable `SessionPresentation` in the bundle manifest, recorded at
  Push.

Storage-only rows have no local transcript, and Activity never downloads one
to draw a row. Their placement therefore uses the `last_prompt_at` that Push
parsed from the local JSONL.

The field is optional and additive. Older readers ignore it: the Rust struct
accepts unknown fields, and Managed Cloud's presentation validator is open.
Rows from bundles pushed before this change fall back to `ended_at`.

### Tooltip

The session's timestamp tooltip reads:

```
Started Sep 27, 2026 at 21:52
Last input Sep 27, 2026 at 22:03
Ended Sep 27, 2026 at 22:06
```

It shows the time only, never the input's text. A session with no recorded
input keeps the two-line tooltip. The same lines form the accessible label.

### History scope

Each Activity build runs:

```
git log --first-parent --date-order -n 10001 refs/heads/<branch>
```

This reads up to the newest 10,000 first-parent commits, with no date limit.
Mallard warns when the cap is hit. The 30-day Activity window only limits
which sessions are loaded. A commit is drawn when it falls in the window or
holds a visible session. Nothing about Git is cached.

### When the mapping is rebuilt

| Trigger | Branch used |
| --- | --- |
| App open, project switch | Checked-out branch |
| **Reload app data** | Checked-out branch |
| Refresh button, retry, reload after a delete | The pin, or the checked-out branch |
| Git ref change outside Mallard (watcher) | The pin, or the checked-out branch |
| Branch menu | The picked branch |
| **Load earlier** | The branch already shown |

The *branch pin* stays `null` until the viewer picks a branch other than the
checked-out one. Picking the checked-out branch clears it. Loads never
overwrite the pin with the branch the backend resolved.

### Git watcher

`src-tauri/src/project_sync/git_watcher.rs` only signals the open Activity
page that the project's history moved; the page reloads, which rebuilds the
mapping.

- **One project at a time.** The watched project is held in managed state.
  `projectSyncApi.holdProjectGitWatch` counts holds per project, so a Push or
  Pull review showing the same project cannot stop the workspace page's watch.
- **Resolved from state.** `watch_project_git(localProjectId)` finds the
  active binding's canonical root in machine state; the frontend never sends a
  path. It then runs
  `git rev-parse --path-format=absolute --git-dir --git-common-dir`, which
  handles linked worktrees. A project that is not a Git work tree clears the
  watch.
- **Three watched paths.** Using `notify`, it watches the worktree's git dir
  (non-recursive, for `HEAD`), the common dir (non-recursive, for
  `packed-refs`), and `<common>/refs/heads` (recursive).
- **Filter.** Only `HEAD`, `packed-refs`, and branch refs count. `.lock`
  files, the index, objects, logs, tags, and `FETCH_HEAD` never do, so an
  IDE's `git status` polling stays quiet.
- **Debounce and event.** Changes are debounced by 500 ms. The event is
  `project-git-changed` with `{ local_project_id }`, so no path leaves the
  backend.
- **Reload.** The page calls `load(null, pin, fresh = true)`. A fresh read
  bypasses the in-flight single-flight, and it never rebuilds the storage
  comparison or touches a review's selection.
- **Cost.** An idle watch costs nothing, because the operating system pushes
  the events. A real change costs one first-page Activity build.

## Not changed

Managed Cloud (the Worker and web app) performs no session-to-commit
reconstruction. It stores `last_prompt_at` as an unvalidated extra field. An
explicit `optInt(presentation.last_prompt_at)` in
`validate/manifest.ts:validatePresentation` is optional hardening.

## Verification

**Mapping** (`project_activity.rs`):

- the examples above;
- a commit in the same second as the last input;
- two commits in one second go to the older;
- a session without an input is placed by its end;
- a recorded SHA never moves a session;
- each session is mapped or unmapped exactly once.

**Parsers** (Codex and Claude `history.rs`):

- Bounds and the last input come from counted records only. Injected Codex
  context and superseded event-view duplicates are ignored.
- A Claude slash command sets the last input's time, while the interruption
  marker and the compaction summary are ignored and not counted.

**Watcher** (`git_watcher.rs`):

- the ref-path filter, for an ordinary clone and for a linked worktree;
- a temporary-repo checkout plus commit yields one debounced signal.

**Frontend** (`project-activity.integration.test.tsx`):

- the tooltip's three lines, and two when no input is recorded;
- `branchPin`;
- reloads pass the pin, and paging keeps the branch shown.

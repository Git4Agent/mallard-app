# New Codex or Claude Session in a Project

Status: implemented October 7, 2026 (owner picked A). Not committed. Live
Tauri pass not yet run. Specimen: `mockups/new-session-mockup.html`
(`?dir=all|today|a..f&kind=flat|git&agents=both|codex&theme=light|dark`,
`&live=open|unreadable|empty` on A); A's frames are the shipped page.

All four items ship. *Claude app* uses the same `open --env` route as Open
in Claude and still needs the live check in decision 2.

## Problem

A project page lists its sessions, and each row can reopen its session in the
agent's app or in Terminal. There is no way to start a new session there. The
Library's profile launch opens the agent with no project. The after-Pull log
prints "Codex — new" and "Claude — new" command lines (`project_open_commands`)
for the user to copy.

## Rule

Each fact is stated once. The path bar already names the project and the
storage, and *Agent profile…* names the bound homes. The new control names only
the agent and where it opens: its app, or Terminal.

## Shipped: A · Menu button, plus the empty state

- **Where:** a "+ New session ▾" button at the end of the Sessions search row.
  It uses `NameSearchField`'s existing `trailing` slot, as the Library's
  *Add from Mallard Cloud…* does. It is shown on the Sessions face only, and
  never during a Push or Pull review. Below a 600px panel it shows its glyphs
  only (+ ▾) and keeps its accessible name. It uses the list's existing
  `sync-history-layout` container, so no new containment is added.
- **Menu:** the path bar's own `PathBarMenu`, with one labelled group per bound
  agent: *Codex app* and *Terminal*, then *Claude app* and *Terminal*. Agents
  follow the registry's display rank (Codex 10, Claude 20): the activity's
  `providers` list is sorted that way at its source (`in_display_order`), not
  by key, which put Claude first. Every row
  has a leading icon, following Primer. An agent whose profile can't be read
  keeps one inert row whose second line gives the reason
  ("Can't read its profile · Agent profile…").
- **One agent bound:** the same button and menu, with two rows. HIG asks a
  pull-down for three items or more. The menu stays anyway, so the control
  keeps one place and shape whether one or two agents are bound. Neither
  Carbon's menu button nor Primer's ActionMenu sets a minimum. The
  alternative, two plain buttons for one agent, is decision 5.
- **Empty project:** while the list is empty, the button moves into the empty
  box as its primary action (Primer Blankslate; NN/g "direct pathways"), so the
  page shows it once. The box clips overflow, so the menu must escape it: either
  render it in the top layer, as the session debug popover does, or let that
  one box overflow.
- **Opening:** the button shows a spinner and is disabled until the launch call
  returns. An error uses the existing session-action banner under the path bar.
- **No placeholder row.** A new session appears in the list the next time the
  list is read. Mallard has no watcher, and a row for a session that has not
  been written yet would be a guess.

Other directions in the specimen:
- B · Split button (VS Code and Windows Terminal +▾, Carbon combo, Material).
  Against: its default must be remembered state.
- C · Launch pairs (the session row's own pair, once per agent). Against: four
  bare glyphs, and Carbon caps a row at three.
- D · Add row (GitHub Projects). Against: a row in the list that is not a
  session.
- E · Path bar + (Zed, Windows Terminal). Against: it costs 34px. On a full
  580px Git bar (attention chip, Pull 12, Push 40), the actions then overlap
  the trail by 39px. A short trail keeps 16px.
- F · Project menu (GitHub Desktop's Repository menu). Against: the switcher's
  verbs are icon-only pills, and three of them would be identical Terminal
  pills.

## What runs

| Item | macOS | Windows |
| --- | --- | --- |
| Codex app | `open -n -a <ChatGPT.app> --env CODEX_HOME=<home> 'codex://threads/new?path=<root>'`, through `run_codex_app_deep_link`, the same route as the Library launch and the first step of Open in Codex | `codex://` scheme handler with `CODEX_HOME`, through `windows_app::launch_app` |
| Codex · Terminal | `CODEX_HOME=<home> <codex> -C <root>` | same, as a `cmd.exe` line |
| Claude app | `open -n -a /Applications/Claude.app --env CLAUDE_CONFIG_DIR=<home> 'claude://code/new?folder=<root>'` | `claude://` scheme handler with `CLAUDE_CONFIG_DIR` |
| Claude · Terminal | `cd <root> && CLAUDE_CONFIG_DIR=<home> <claude>` | same, as a `cmd.exe` line |

- `<home>` is the project binding's profile home and `<root>` is its checkout.
  The CLI is resolved the same way as for resume: the path saved in Settings,
  then the inherited PATH, then the login shell. The Terminal lines are the
  "— new" lines `project_open_commands` already builds; both will use one
  builder.
- No version gate. The 0.145.0 check guards resume of a saved thread; a new
  thread does not resume anything.
- Each launch writes the line it runs to the activity log, as `$ <command>`,
  just as session actions do.
- Deep links are documented: `codex://threads/new` with `path`
  ([Codex app commands](https://learn.chatgpt.com/docs/reference/commands)), and
  `claude://code/new` with `folder`
  ([Open Claude Desktop with a link](https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link)).
  Claude Desktop always asks the user to confirm a folder that arrives in a
  link.

## Backend shape

- **One command:** `start_project_session(localProjectId, provider, surface)`,
  where `surface` is `app` or `terminal`
  (`NewSessionSurface`, mirrored in `src/types.ts`). It resolves the binding
  with `resolve_activity_project_from_state`, the same way
  `execute_project_session_action` does, and refuses a provider the project
  does not bind or whose profile can't be read
  (`project_activity::start_project_session`).
- **One adapter method:** `SessionAdapter::start_session(NewSessionRequest)`,
  beside `execute_action` in `providers/{codex,claude}/actions.rs`. Codex's app
  launch shares `open_app_new_thread` with the Library's launch; Claude's
  shares `open_app_link` with Open in Claude. `percent_encode_uri_component`
  moved to `providers/launch.rs`. The "— new" after-Pull lines use the same
  `new_session_terminal_command` builders.
- **Frontend:** `NewSessionMenu.tsx` (the button and `PathBarMenu`), placed by
  `ProjectChatHistoryContent` through `NameSearchField`'s `trailing` slot, or
  in the empty flat list's box. `PathBarMenu`'s labelled groups now carry
  `role="group"` and `aria-labelledby`, so the two Terminal rows are told
  apart by assistive tech. Failures go through the row actions'
  `reportActionError`, and the two share one in-flight guard.
- **No DTO change beyond the request enum.** The menu comes from
  `activity.providers`.
- **No schema, manifest, or storage change.**

## Decisions

1. **Direction:** A (owner, 2026-10-07).
2. **Claude app item: shipped; live check pending.** Run it on a non-default
   profile and confirm two things: that `claude://code/new?folder=` honours
   `CLAUDE_CONFIG_DIR` passed through `open -n --env`, and that it opens a
   Code session rather than Cowork home. If the app ignores the variable, the
   session is written to the default `~/.claude` and this project never lists
   it. Resume has the same dependency, but there the failure shows. If the
   check fails, drop the item: filter `app` for Claude in
   `newSessionMenuGroups` and refuse it in Claude's `start_session`.
3. **Seeing the new session:** Refresh, as today. No read on focus.
4. **⌘N:** not added.
5. **One agent bound:** the same two-row menu.

## Verification when implemented

- Rust: `cargo test --manifest-path src-tauri/Cargo.toml --lib`. Add tests for
  the command builders: quoting, an absolute CLI, and `CODEX_HOME` or
  `CLAUDE_CONFIG_DIR` set to the bound home, never `~/.codex` or `~/.claude`.
  Also test that a provider the project does not bind is refused.
- Frontend: `npm run build` and `npm run test:frontend-integration`. Cover the
  menu groups, the inert row, the empty-state move, and the absence of the
  button in reviews and on the Files face.
- Live Tauri pass on macOS with a non-default profile for each provider. Start
  a session four ways, then refresh and see it listed in this project. Run the
  same pass on Windows.

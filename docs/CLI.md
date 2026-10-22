# The Mallard command line

`mallard-cli` does what the desktop app does, from a terminal, for people and
for coding agents (Codex CLI, Claude Code) working in a project folder. It
runs the same Rust services as the app, writes the same local state under
`~/.mallard`, and appends to the same activity log, so a running desktop app
shows what the command line did without a restart.

The packaged binary ships inside the app: `Mallard.app/Contents/MacOS/mallard-cli`
on macOS, `mallard-cli.exe` in the installation directory on Windows. The
installer does not change `PATH`; call it by its full path or link it from a
folder on your `PATH`. From a source checkout, `cargo run -p mallard-cli --`.

## The contract

- **One verb per process.** Nothing prompts. Browser sign-ins print a link and
  wait for the callback; everything else runs to completion or fails.
- **`--json`**: stdout carries exactly one JSON document
  `{ "schema": 1, "status": "<verb status>", "<noun>": <payload> }`. The payload
  is the same serialized shape the desktop receives (`src/types.ts` names the
  fields). Stderr carries one NDJSON line per event while the verb runs:
  `{ "schema": 1, "status": "event", "entry": <activity-log entry> }`,
  `{ "schema": 1, "status": "progress", "kind": "fetch|catalog|clone|authorization_link", ... }`.
  Without `--json`, both are plain text.
- **Errors** print nothing on stdout and end stderr with
  `{ "schema": 1, "status": "error", "exit_code": n, "code": "...", "message": "..." }`.
  Messages never echo argument values or credentials.
- **Exit codes**: 0 success · 2 invalid input or nothing matched · 3 a revision
  or review no longer holds (read or review again) · 4 the storage, provider,
  or network failed · 5 Mallard's local state could not be used.
- **Project from cwd.** A verb that concerns a project finds it from the
  current folder (any folder inside a registered project); `--project <id|path>`
  overrides. A storage is the project's only link unless `--storage <id|name>`
  says otherwise; with several links the error lists them.
- **Guarded writes.** Renames, removals, unlinks, deletions, and clears take
  the current revision (`--expected-revision`, as the preceding read printed
  it) and destructive ones also `--confirm`. Reviews pin what they saw: an
  apply whose review no longer matches exits 3.
- **Never printed**: credentials, tokens, secret values. The desktop's secret
  reveal and local-data reset have no command-line verb.

`mallard-cli` alone prints the help page: the commands in groups, the flags,
examples, and the output contract, bold on a terminal and plain when piped.
`mallard-cli <noun>` alone prints that noun's help, with examples; `--help`
works at every level.

## Nouns

| Noun | Verbs |
| --- | --- |
| `status` | this computer's identity, storages, projects, and the current folder's project |
| `project` | `list`, `add <path>`, `show`, `files`, `inventory`, `rename`, `remove` |
| `link` | `list`, `add --storage`, `connect --storage --bundle-id`, `remove --storage --confirm` |
| `push` | `review`, `apply`, `finish`, `status` |
| `pull` | `review`, `apply --review <file> --approve …`, `readiness` |
| `session` | `list`, `show`, `timeline`, `event`, `find`, `run`, `delete`, `delete-all` |
| `storage` | `list`, `show`, `verify`, `rename`, `remove`, `clear-data`, `snapshots`, `providers`, `add local|r2|dropbox|feishu|cloud`, `reconnect`, `draft …`, `r2 …` |
| `catalog` | `capabilities`, `profile …`, `scope …`, `repositories`, `link`, `unlink`, `items`, `item show|file`, `push`, `pull`, `fork`, `remove`, `finish-push`, `cloud …` |
| `cloud` | `repositories`, `starred`, `public`, `resolve`, `accept-invite`, `publish-version`, `discard-draft`, `clone`, `link` |
| `provider` | `list`, `cli inspect|test|refresh`, `maintenance` |
| `settings` | `get`, `set key=value …` |
| `log` | `query`, `stats`, `policy`, `cleanup`, `folder` |

## Reviews: two steps

Push and Pull are review-first, as in the desktop.

**Push.** `push review` compares the project with the storage and prints the
review: every session with its state and whether it is pushable, the project
files found, and whether the destination is ready or needs a Pull first.
`push apply` compares again and pushes the recommended selection: the
sessions the review ticks, adjusted by `--select <resource-id>` and
`--deselect <resource-id>`; with `--files` also the project's ordinary files
(non-Git projects only: the files already in the recipe plus every new,
unflagged file), `--remove-file <id>` to drop a stored file, and
`--acknowledge-warnings all|<digest>` for flagged files.

**Pull.** `pull review --json` prints the review document: every action with
its id, and `requires_explicit_approval` on the ones that replace local
content (every project file write). Save it, then
`pull apply --review <file> --approve all` applies every action that needs no
explicit approval; name the others with `--approve <action-id>` or approve
every action of a resource with `--resource <resource-id>`, which is an
approval on its own. A review of project files alone is all explicit
approval, and the text review says so. The review pins
one storage version: it stays applicable while that version exists and this
computer has not synced past it; otherwise apply exits 3 and the agent
reviews again.

**Library.** `catalog push`, `catalog pull`, `catalog fork` and
`catalog remove` print their review and apply it only with `--approve` in the
same run, because a Library review lives only as long as the process that
built it. `catalog pull --approve` takes each item's default action;
`--choose <id>=<action-id>` picks another, `--acknowledge-destructive <id>|all`
allows actions that replace local content.

## Worked example: two computers, one folder storage

On computer A, in the project folder:

```sh
mallard-cli --json storage add local --path /Volumes/shared/mallard-storage --name Shared
mallard-cli --json project add "$PWD" --provider codex="$HOME/.codex" --storage Shared
mallard-cli --json push review
mallard-cli --json push apply --files
```

On computer B, with the same storage folder mounted:

```sh
mallard-cli --json storage add local --path /Volumes/shared/mallard-storage --name Shared
mallard-cli --json storage snapshots Shared                     # the bundle id
mallard-cli --json project add "$PWD" --provider codex="$HOME/.codex" \
  --storage Shared --connect <bundle-id>
mallard-cli --json pull review > review.json
mallard-cli --json pull apply --review review.json --approve all
```

A project file write needs explicit approval: read `review.actions[].action_id`
from `review.json` and pass each with `--approve`, or `--resource <resource-id>`
for all of one file's actions.

## Worked example: a skill through the Library

```sh
mallard-cli --json catalog profile add --provider codex --path "$HOME/.codex"
mallard-cli --json catalog scope add --provider codex --profile <profile-id> \
  --kind agent_skill --format agentskills.io.skill
mallard-cli --json catalog link <scope-id> --storage Shared
mallard-cli --json catalog push <scope-id> --storage Shared            # the review
mallard-cli --json catalog push <scope-id> --storage Shared --approve  # the push
```

On the other computer, after the same profile, scope, and link:

```sh
mallard-cli --json catalog pull <scope-id> --storage Shared --approve
```

## Worked example: a project from Mallard Cloud

```sh
mallard-cli --json storage add cloud                      # opens a sign-in link
mallard-cli --json cloud resolve --storage "Mallard Cloud …" owner/name
mallard-cli --json cloud clone --storage "Mallard Cloud …" owner/name --into ~/code/name
mallard-cli --json project add ~/code/name --storage "Mallard Cloud …" --connect <bundle-id>
mallard-cli --json pull review --project ~/code/name > review.json
mallard-cli --json pull apply --project ~/code/name --review review.json --approve all
```

`cloud link 'mallard://open?address=…'` reads a website link the way the
desktop does and says what each signed-in account sees there.

## Sessions

`session list` is the project's Activity: every session with its provider,
times, turns, tool calls, and `resource_id`. `session list --storage <s>
--refresh` also compares with the storage. `session show <resource-id>` gives
the detail and its `snapshot_id`; `session timeline <resource-id>` pages
through the transcript (`--kind prompt --kind response`, `--tool shell`,
`--turn <id>`); `session event <resource-id> <event-id>` is one event in full.
`session run <resource-id> <action-id>` runs an action `session show` listed.

## Storages

`storage add local --path <dir>` saves a folder storage at once. `storage add
r2|dropbox|feishu|cloud` sign in through the browser: the command prints the
link (`authorization_link` on stderr), waits for the callback, prepares the
draft, probes the destination, and saves it. `storage reconnect <s>` renews a
lost credential the same way. The desktop's R2 setup page still drives the
`storage r2 setup|accounts|discover` helper with an open draft; that contract
is unchanged.

## For agents

An agent working in a project needs only `--json` output. The sequence it
runs is the same a person clicks: read (`status`, `project show`, `push
review`), decide, then write with the revision or review it read. Everything
the command line does is logged where the desktop's Activity drawer reads
(`log query`), so a person can follow along in the app.

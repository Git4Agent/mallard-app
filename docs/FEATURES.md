# Features

What Mallard does, in detail. The short introduction is the repository's
[README](../README.md); storage setup is in [STORAGE_SETUP.md](STORAGE_SETUP.md).

## What Mallard does

- Maps all project sessions onto any selected Git branch's first-parent commit
  history, so a team can explore the reasoning behind a diff.
- Parses session summaries and conversation previews locally, with metrics for
  start and end times, user turns, tokens, agent messages, and tool calls.
- Syncs selected project conversations and provider-home resources — memory
  and session state — through Mallard Cloud (sign in with GitHub; private
  snapshots, collaborators, and a website that shows released sessions and
  Library items), a local folder, Cloudflare R2, Dropbox, or Feishu Drive.
  Every storage holds the same immutable layout, and Mallard Cloud runs the
  same publication protocol as R2, with claims.
- Answers the website's links (`mallard://open?address=<a page's address>`).
  A repository page's **Open in Mallard** opens one sheet that says what it
  found — asking each Mallard Cloud account on the page's host — and where
  that stands on this computer, with one verb: the project when it is here,
  otherwise New project on From Mallard Cloud, filled in. A session page's
  **Pull in Mallard** opens the same sheet for its repository with *Pull
  session…*: a Pull review of that session alone, whose **Pull 1** brings it
  here, opens its page, and leaves the project's reviewed base, Pull badge
  and Push gate where they were. A skill, plugin or pet page's **Pull in
  Mallard** opens the item's page on its own Pull review, after Add Catalog
  resource when the item is not in the Catalog yet. The listing's **Open
  Mallard** (Share yours) asks
  nothing and goes straight to Projects, or to the Catalog when a Skills,
  Plugins or Pets chip was on; the sheet appears only when no storage signs
  in to that service. A link only reads, and its verb only opens a page or a
  review; nothing is connected, cloned, added, pulled or pushed until that
  page's own verb is pressed. While an operation,
  a review or a dialog is open, the link waits under the bar instead.
- Compares local and stored resources before Push or Pull, including
  local-only, storage-only, ahead, synchronized, and conflict states.
- Publishes immutable, SHA-256-verified bundle revisions after an explicit
  comparison review. Push and Pull capture and build current actions only in
  Apply; they persist no executable plan or conversation recovery journal.
  Pulls create backups and retain result receipts.
- Optionally syncs selected ordinary files and folders for non-Git projects.
  Each file can be up to 100 MiB; the total selected Push is limited to 512 MiB.
  Git-managed project content continues to travel through Git.
- Starts a new session in a project: **New session** at the end of the
  Sessions search row (in the empty list's box when there is none yet) runs
  a bound agent's CLI in Terminal in the project's folder, on the project's
  own agent home, or opens the agent's desktop app on that folder. The new
  session is listed once the agent writes it and the list is refreshed.
  On macOS, Claude desktop actions reuse the running app for session resumes,
  new sessions, and Library launches. A running Claude keeps its current
  profile; Mallard's selected Claude home applies when starting a closed app.
- Manages user-global Agent Skills, provider-native plugin install intent, and
  Codex pets as independent Library resources. The Catalog menu starts empty
  and grows one row per **(provider home × resource kind)** the user adds. A
  row's page works like a project's: one storage chosen in the path bar, one
  list of resources each wearing the same status badge a thread wears, and
  Push and Pull as page verbs with a review. Storage setup returns to the
  originating Catalog row. On every storage an item's name opens its page,
  made like its page on Mallard Cloud: a skill's SKILL.md and files, a
  plugin's own words and the provider CLI lines that install it, a pet playing
  on its desk — read from this Mac's copy, or the storage's when only it has
  the item — beside what only this Mac knows (edited, size, warnings, and the
  launch actions); its path bar carries Pull and Push for that item alone. On
  Mallard Cloud each item is its own repository, and the bar adds the Cloud
  chip (status, release, draft). An item with no repository shows the private one its first
  Push creates, and Change… opens the project's connect dialog: create it now,
  or choose one by address or invite code. A connected item keeps its
  repository. The row's + adds someone else's item from Mallard Cloud, and so
  does Add Catalog resource's From Mallard Cloud door; nothing is pulled on its
  own. An item two repositories hold waits on its page for a choice. Push
  reviews name each item's repository, new ones included.
  Push and Pull show preparation, per-item progress, and persistent results;
  a partial Push keeps successful items and offers a fresh review of the rest.
  Repository bindings are machine-local schema-1 state; portable schemas stay
  at 1. Refresh, Unlink, and local Fork remain available on each Catalog row.
  Add Catalog resource is New project's two steps: the agent home, chosen as
  the project chooses its agents (one card per provider, the saved profile
  preselected when the machine has exactly one, Custom path… and Browse for
  any other), then that home's setup page — one kind as the Kind cards with
  what the home holds on each, the storage, Finish — making one row per
  pass. Setup pages list storages Mallard Cloud first, then the other
  clouds, then local folders, and open on the first; a Catalog row opens on
  the storage a row used last.

The first Library catalog supports Codex and Claude skills/plugins and Codex
pet v1/v2. Pet formats are separate resources — and separate menu rows — and
the user chooses the exact format to operate on. Skill discovery is limited to
`<CODEX_HOME>/skills`, `$HOME/.agents/skills` (default `~/.codex` config
only), and `<CLAUDE_CONFIG_DIR>/skills`; project-local skill directories are
not scanned.

Library inventory reads local provider files. Listing Skills or Plugins,
linking storage, Push, Pull review, and skill Pull/Fork do not require a
provider executable. Codex plugin presence uses local config or remote-install
markers plus cached manifests; cache-only packages are not listed as installed.
Remote plugin enablement remains unknown when it is absent from local config.
Plugin installation, overwrite, or uninstall runs the provider CLI after approval,
with local state checked before and after execution. Schemas remain version 1.

Local item cards and item pages have a trash button. **Delete local skill** and
**Delete local pet** prepare a removal review, count the complete local folder,
and ask for confirmation. Deletion checks file digests and removes only empty
directories. **Uninstall local plugin** uses the selected provider home; Claude
uninstalls only the user scope. Changed or expired reviews must be prepared
again. Removal keeps storage copies and links, so a stored item can be pulled
again. Interrupted removal is reported and is never replayed automatically.

Library resources use a clean cutover: there is no backward compatibility,
migration, legacy execution path, or parallel storage-wide catalog.

## Install and updates

The fastest path is the packaged macOS DMG (Apple Silicon or Intel) or the
Windows x64 installer from the
[latest release](https://github.com/Git4Agent/mallard-app/releases/latest).
The macOS app is Developer ID signed and notarized, so it opens without a security
override. The first **Open in Terminal**, **New session › Terminal**, or Codex App
refresh asks for Automation permission; allow it. Mallard finds the Codex and Claude CLIs the
way your terminal does, including the `PATH` your shell sets: it asks your
login shell once per launch, in the background, with
`MALLARD_RESOLVING_ENVIRONMENT=1` set so a startup file can skip slow work
when it sees it. On macOS, automatic Codex CLI discovery also checks the
official ChatGPT desktop app in `/Applications` and `~/Applications` after
standalone CLI installations. Claude discovery also checks the host CLI that
Claude Desktop downloads under `~/Library/Application Support/Claude/claude-code`,
selecting the newest complete, valid installation after standalone CLIs.
If Desktop has not downloaded its CLI yet, open its Code tab to set it up.
If a machine keeps a CLI somewhere else, type
or browse to the executable under Settings › Agent CLIs (an empty field means automatic),
where **Test** runs its `--version`. Mallard
downloads newer releases in the
background; **Update** in the sidebar's version mark restarts into one, and
quitting installs it for the next launch. **Automatic updates** can be
turned off under Settings › Privacy, where **Check for updates** always
works.

You will also need Git and a local Codex profile. The default profile is
normally stored at `~/.codex`.
A session Codex has compressed in place (`rollout-….jsonl.zst`, its
experimental `local_thread_store_compression` feature) is read like a plain
one; Mallard itself writes only plain `.jsonl` files.

Sessions above the fixed 100 MiB size limit remain in the
project's session list with an **Unavailable for sync** badge and the blocking
reason. These rows use discovery metadata and the file's modification time;
Mallard does not read the oversized transcript to calculate activity metrics.
Push and Pull continue to exclude them.

## Files and media in session detail

Relative file links and media use each message's recorded working directory.
Codex turn context and Claude message records can change that directory within
one session. Links stay inside the bound project; paths outside it and paths
through symbolic links cannot be opened. An unavailable or outside working
directory disables relative file links.

Audio and video play through native byte-range requests, including seeking in
large files. Videos may be up to 500 MiB (524,288,000 bytes). Each response reads
at most 1 MiB; the complete video is never copied through command IPC. Container,
video-size and project-path checks apply to every request. Images retain their
8 MiB embedding limit. Media remains local project content; viewing a session
does not transfer its linked files.

## Claude widgets in session detail

Mallard displays complete SVG widgets from Claude's `mcp__visualize__show_widget`
tool in session detail. The Replies filter includes widgets. Other tool calls
remain under Tool calls. Each widget has a Source control with a copy button.

The widget source travels inside the Claude transcript through Push and Pull.
Mallard Cloud displays it from the stored session preview when that preview
contains the complete arguments. No extra widget file or bundle schema is needed.

SVG widgets load as images with local presentation styles. Their scripts and
external document resources do not run. HTML widgets, incomplete input, and
malformed SVG show a message and retain the available source. Widget interaction
state and external dependencies are not synced.

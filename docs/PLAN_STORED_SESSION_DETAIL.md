# Open a stored session's page

- Status: the owner picked **A, the status chip**, on 2026-10-07 ("Go with A,
  don't use cache for the cloud session"). Implemented the same day,
  uncommitted; see *As built* at the end for what landed and where it
  differs from the plan below. Open: a pass in the running Tauri app against
  a Mallard Cloud storage.
- Scope: a session row under *Only in {storage}* in a project's activity
  list, and the session page it opens. The Push and Pull flows, the website,
  the Library and the Files face are not changed.
- Visual contract: `mockups/stored-session-mockup.html`
  (`?dir=all|a|b|c|d|e|f&theme=light|dark`). Frame 0 is the shipped activity
  list on static data, before and after. The page frames are the shipped
  `SessionDetailPage` on canned stored data under the real path bar.
- Screens: `docs/images/stored-session/` (compare-all light and dark, one per
  direction).

## The problem

A session that was pushed from another computer is a row under *Only in
rikaqu0223-arch*, but the row is inert: its title is plain text, there is no
chevron, and the only way to read it is to Pull it into the local Codex or
Claude home first. Every other row opens a page.

## What the user sees

- A row under *Only in {storage}* opens like any other row: the title is the
  row's link and the chevron is there.
- The page is the shipped session page, read from the stored transcript: the
  facts strip, the model chip and its notes, the Show bar, prompts, replies,
  steps, tool calls with their bodies, agent rows, and checkpoints from this
  project's own Git history (the website's view cannot draw those).
- The bar's **Pull** counts this one session (`Pull 1`) and pulls only it, the
  way **Push** counts a local session's page. **Push** is off with the reason
  it already has: *No local session to push.* There are no launch buttons:
  nothing here opens in Codex or Terminal.
- One mark says the copy is the stored one. Which mark is the owner's pick
  (below).
- The rule holds for every storage kind. The data is the bundle, which every
  storage shares, so a local folder, S3/R2, Dropbox, Feishu and Mallard Cloud
  all open the page the same way. Restricting it to Mallard Cloud would be an
  exception with no data behind it.

### Directions

| | What | Seen in | Against |
|---|---|---|---|
| **A · Status chip** (recommended) | The row's status glyph with *Not on this computer* as the first fact chip; the row's sentence is its tooltip. | OneDrive Files On-Demand and Google Drive streaming mark the item itself with its online-only status. | Says the storage the bar's crumb already names (hence the face avoids the name). |
| B · Leaf mark | The row's glyph on the path bar's leaf crumb, after the provider mark, the way a live session wears a dot; the sentence in the crumb's tooltip. | VS Code Remote Repositories: a remote indicator in the frame, the words on hover. | Words only in the tooltip. |
| C · Chip + Pull | A, plus a Pull button where *Open in Codex* sits for a local session. | Dropbox and OneDrive: opening an online-only file is downloading it; HIG iCloud: let the person control the download. | Pull twice on one screen. |
| D · Banner | One info notice under the bar: *Not on this computer. Pull it from rikaqu0223-arch to open it in Codex.* with Pull. | Primer Banner, Carbon inline notification; VS Code's limited-functionality notice. | A state said as a message; stacks with the snapshot-changed notice. |
| E · Note | The sentence as an info note in the notes chip; nothing else. | NN/g: a persistent state is an indicator, not a notification; the page's own notes rule. | The fact behind a click. |
| F · Named Pull | A labelled *Pull from rikaqu0223-arch* where the launch buttons sit; no chip. | VS Code's *Continue Working On…*; HIG iCloud's download control. | A verb standing in for a fact. |

**Recommendation: A.** The row's glyph and sentence carry over unchanged, so
there is one vocabulary for "stored" across the list and the page; the verb
stays where Push already lives for a local session's page; and it is an
indicator for a state (NN/g), not a banner for a label (Primer). B is the
runner-up if the chip reads as a repeat of the bar. Not read first-hand:
Steam's library (install where play sits; only forum threads came back),
Material 3 (JS-rendered), Finder's iCloud glyph.

## How it works

The Mallard Cloud website already renders a stored session: at Push the
desktop builds a view from the wire bytes in a private temporary provider home
(`managed_cloud/views.rs`, `build_view`). The desktop page does the same read,
without the cap, and with the project's Git history.

1. The frontend opens the row with its id, `<provider>:conversation:cloudthread-…`,
   and the storage it sits under. A local row's id is unchanged.
2. The service tells the two apart by `parse_cloud_conversation_resource_id`.
   A cloud id needs a storage id and a local id refuses one (`InvalidRequest`).
3. For a cloud id: load the state snapshot, the project's link to that
   storage, and the storage engine (`storage_engine_from_state`); read the
   selected publication's manifest (`inspect_optional`, served from the object
   cache after an activity refresh); find the thread by provider and cloud id;
   take its `Transcript` file and that file's `BundleFileEntry`.
4. Fetch the object through `engine.byte_source(&snapshot).body(path)`:
   verified against the manifest's size and SHA-256, bounded by
   `conversation_file_limit_bytes`, cached in `ObjectCache::global()`.
5. Prepare the bytes with `pull_transcript::prepare_pull_transcript`, the
   same call Pull makes: lineage and semantic digests checked, the Codex
   standalone root rebuilt, paths mapped back to this project's root, the
   redaction stamp honoured. The destination id is the stored source thread
   id; nothing is written to a provider home. A transcript that fails reads
   *This stored session cannot be read by this version of Mallard.*
6. Write the prepared bytes once into a private temporary provider home
   (below), `<tempdir>/<provider>/sessions/<id>.jsonl`, the way `build_view`
   does for the website.
7. From there the existing code runs unchanged on a `ResolvedLocalSession`
   whose home is that directory and whose project root is the binding's:
   `load_or_build_index` (the timeline cache keys on path, size and mtime),
   `checkpoint_support` on the local repository, tokens, counts, title (the
   manifest's `display_title`), `settings`, `sanitize_boundary_text`. The
   actions list is empty and `is_active` is false.
8. Pages and event bodies resolve the same way and find the file in that
   home while it is held, so they download nothing. A home that was dropped
   is built again; its new path changes the snapshot id and the page's
   existing *This session changed* refresh handles it.

### The private home

A file is needed, not a cache: the provider adapters read a transcript by
path (`build_timeline_index`, `read_timeline_page`, `read_event_detail`), and
the detail, the pages and the event bodies are three separate commands. So
the prepared bytes live in a process-lifetime `tempfile::TempDir`, held in a
bounded registry keyed by `(storage_id, bundle_id, transcript sha256,
canonical project root)`: a handful of homes or a byte ceiling, oldest
dropped first, every directory removed on drop and at exit. Nothing is
written under `~/.mallard`, there is no eviction loop, no persistence lock
and no new state, which is how the in-memory object cache already works. The
cost is one more download and preparation the first time a stored session
is opened after a restart. A later Pull of the same session does not read
the home: Pull keeps its own verified path.

### Commands and DTOs

The three session commands gain one optional argument, `storageId`
(`storage_id: Option<StorageId>` in Rust):

- `get_project_session_detail(local_project_id, resource_id, storage_id?)`
- `get_project_session_timeline_page(…, storage_id?)`
- `get_project_session_event_detail(…, storage_id?)`

No DTO changes. `ProjectSessionDetail.actions` is empty for a stored session,
so the header's action slot is what the picked direction fills (C, F) or
leaves empty (A, B, D, E). The frontend knows the session is stored from the
row it opened, so the storage name is not repeated in the DTO.

### Frontend

- `ProjectActivityPage.tsx`, `SessionCard`: `canOpenDetails` drops the
  `remote_only` clause; a stored row opens when `access !== "unsupported"`
  (reading needs no provider profile). `OpenSessionDetailRequest` carries
  `storageId` for a stored row (`comparison.storage_id` or the active
  storage).
- `sessionDetailModel.ts`: `ActiveProjectSubview.session_detail` carries
  `storageId?`; `SessionDetailPage` takes
  `source: { kind: "local" } | { kind: "stored"; storageId; storageName }`
  and passes the storage id to the three API calls.
- `api.ts`: the three calls take an optional storage id and pass `storageId`.
- `ProjectLinksWorkspace.tsx`: on a stored page the Pull button's scope is
  `{ kind: "session", resource_id }` (the deep-link Pull), its count is 1 and
  its tooltip names the session; Push's reason is the existing *No local
  session to push.*; a finished Pull opens the pulled session's page through
  `openPulledSession`, as the deep link does. Direction B adds the glyph to
  the leaf crumb; A, C, E, F live in `SessionDetailHeader.tsx`; D is a
  `NoticeZone` item in `SessionDetailPage.tsx`.

### Safety

- No provider-home writes and no reads under `~/.codex` or `~/.claude`: the
  stored page reads only the bundle and its own cache directory.
- Bytes are shown only after the manifest's size and SHA-256 check and the
  Pull path's digest checks; a mismatch is an error, not a page.
- The copy is as stored: redacted if it was pushed with redaction on, path
  tokens decoded only against this project's bound root.
- The temporary home is process-lifetime, bounded and outside every provider
  home and `~/.mallard`; nothing about it enters a manifest.
- Push publication, Pull planning and approval, bases and pins are untouched.
  Pull from the page is the existing one-session Pull.
- `storage_id` is validated against the project's links; a cloud id with no
  link to that storage is `NotFound`.

### Performance

First open of a large stored session downloads once (the Pull-sized bound),
prepares once, indexes once; the page's loading scaffold covers it. While
the home is held, every page or event body reads its file and the timeline
cache. Nothing changes for local sessions.

## Tests

- Rust, `src-tauri/src/sync_tests/`: on `TwoComputers`, push a session from
  one, open its detail on the other without pulling: counts and title match,
  a timeline page holds the prompts, an event body reads a tool's result, a
  checkpoint from the local repository is drawn; no file appears under the
  second computer's provider home; the second request downloads nothing
  (`sync_metrics` read counts); a tampered object is refused; a cloud id
  without a storage id and a local id with one are `InvalidRequest`; past
  its ceiling the registry drops the oldest home and its directory is gone.
  Codex and Claude both.
- Frontend, `tests/frontend/session-detail.integration.test.tsx` and
  `project-activity.integration.test.tsx`: a stored row renders the title
  button and the chevron and its request carries the storage id; the three
  API calls pass `storageId`; the page's header for a stored session draws
  the picked mark and no launch buttons; the bar's Pull reads 1 and Push is
  disabled with its reason.
- Live: open a stored Codex and a stored Claude session from a Mallard Cloud
  storage and from a local-folder storage in the running Tauri app; pull one
  from its page and land on the local page.

## Layer 2

- Agent child pages of a stored session. The bundle holds a Claude child as a
  `SubagentTranscript` file and a Codex child as its own object; writing them
  into the same private home lets `locate_child_transcript` find them. In
  layer 1 an agent row shows but its opener is off (*No transcript is
  available for this agent.*).
- Reading the Mallard Cloud view instead of the transcript when the storage
  is Mallard Cloud. Rejected for layer 1: it is capped at 8 MiB and 20,000
  events, carries no tool bodies and no checkpoints, and would be a second
  code path for one storage kind.

## As built

Implemented 2026-10-07 after the owner's pick. Differences from the plan:

- **No cache of any kind.** The owner asked for none, so each of the three
  commands fetches the object (through the engine's existing verified read),
  prepares it with `prepare_pull_transcript`, writes it into a
  `tempfile::TempDir` that is its private provider home, indexes it, answers,
  and drops the directory. No registry, nothing under `~/.mallard`, no
  timeline-cache entry: `build_fresh_index` builds the index in memory. The
  snapshot id is the stored object's SHA-256 with the limits and parser
  version (`stored_snapshot_id`), so cursors stay valid across requests and a
  new publication reads as *This session changed*.
- **Where the code lives.** `crates/mallard-infrastructure/src/stored_session.rs`
  prepares and writes (`StoredSessionTranscript`, `prepare_stored_session`);
  `session_detail.rs` gained `get_stored_session_detail`,
  `get_stored_session_timeline_page` and `get_stored_session_event_detail`,
  and its three local functions were split into a resolve step and shared
  bodies (`session_detail_body`, `timeline_page_body`, `event_detail_body`)
  that both paths call. `service.rs` has `read_stored_session` (link, engine,
  manifest, object, prepare) and three `get_stored_session_*_with_repository`
  functions plus service methods; `commands.rs` dispatches on `storage_id`.
  The rule falls out of the parsers: a cloud id through the local path and a
  local id through the stored path are both `InvalidRequest`.
- **The chip's face** reads *Not on this computer*, with the row's sentence
  as its tooltip, so the storage is said once, on the bar's root crumb.
  `SessionDetailHeader` and `SessionDetailPage` take a required
  `source: SessionDetailSource` (`sessionDetailModel.ts`).
- **The row** opens only when the list knows its storage
  (`ProjectChatHistoryContent.storageId`, fed from `activeStorageId`); a
  stored row needs an adapter (`access !== "unsupported"`), not a profile.
- **The bar** on a stored page: Pull counts 1 and pulls
  `{ kind: "session", resource_id }` from the row's storage; Push is off with
  *No local session to push.*; a finished Pull lands on the local page
  through the existing `openPulledSession`.
- **Tests.** `src-tauri/src/project_sync/stored_session_tests.rs` (included
  in `session_refresh_regressions`): on `TwoComputers`, the second computer
  opens the first's pushed session without a Pull and its counts, tokens and
  event kinds equal the first computer's local page; a stale snapshot is
  `SnapshotChanged`, a missing event `InvalidRequest`; nothing lands in the
  second computer's provider home and its link does not move; a local id
  with a storage and a cloud id without one are refused; a changed stored
  object is refused after the object cache is cleared. Codex and Claude both.
  Frontend: the header chip and no launch verbs for a stored source; the
  three API calls carry `storageId`; the shell and workspace rules; a stored
  row's title button and chevron with a storage, plain without one
  (`session-detail.integration.test.tsx`, `project-activity.integration.test.tsx`).
- **Specimen.** A's frame in `mockups/stored-session-mockup.html` is now the
  shipped page with its `source` prop; the other five stay as drawn.

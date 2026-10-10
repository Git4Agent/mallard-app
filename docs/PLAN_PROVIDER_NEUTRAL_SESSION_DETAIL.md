# Plan: Provider-Neutral Session Detail Timeline

**Status:** proposed; research complete; implementation not started  
**Date:** 2026-08-07  
**Scope:** Codex and Claude Code session inspection inside Project Activity

## 1. Outcome

Let a user open a local session from Project Activity and inspect its full
available chronological history on a dedicated page. The page takes its
information architecture from Entire's session view without copying Entire's
visual skin:

- prompts;
- final responses;
- visible intermediate progress;
- session-related Git checkpoints when Mallard has evidence for them;
- per-turn tool-call summaries, with individual tool rows off by default and
  bounded arguments/results loaded on demand;
- File edit, Bash/shell, Read, and Other tool counts;
- checkpoint jumping, event filters, and tool-call expand/collapse; and
- the existing provider actions, expressed through provider-neutral action
  descriptors.

The first release supports locally materialized Codex and Claude sessions.
Storage-only sessions continue to say `Pull to open`; an unbound or unknown
provider remains metadata-only.

This is a clean replacement for the current ten-message inline disclosure.
There must not be a rich dedicated page plus a second legacy detail parser and
command.

## 2. Research conclusions

### 2.1 Entire is a useful interaction reference, not a data contract

Entire's page combines a chronological conversation, nested tool activity,
checkpoint navigation, and a persistent filter rail. Its checkpoints are
created by Entire's lifecycle hooks and linked to Git commits; they are not a
generic name for provider compaction records. Entire's public material also
describes capturing transcripts, tool calls, token usage, file changes, and
commit-linked checkpoints.

Mallard has the transcript and a Git correlation rail, but it does not install
Entire-style hooks and cannot retroactively prove every commit-to-session
association. The UI may borrow the layout, but it must disclose inferred or
unsupported data instead of manufacturing parity.

### 2.2 Current Mallard behavior is too narrow

The production path is still Codex-specific:

- `ProjectChatHistoryPage.tsx` embeds session cards in the project workspace.
- A session title is plain text; a chevron loads an inline preview.
- `CodexThreadDetailsPage` contains only `user | assistant`, timestamp, and a
  240-character preview.
- `get_project_chat_thread_details` scans and returns at most 50 visible text
  turns, while the frontend requests ten at a time.
- Tool payloads, intermediate events, and checkpoints are intentionally absent.
- The active history engine resolves only the bound Codex profile.

There is no router. `ProjectSync` and `ProjectLinksWorkspace` already implement
state-based workspace navigation, and `.sync-main` is the single workspace
scroll owner.

### 2.3 The provider adapter cutover is the correct foundation

The in-progress provider architecture already introduces:

- validated provider keys;
- a `ProviderRegistry` with Codex and Claude;
- provider-neutral `SessionFileSummary` and `SessionTurnPreview`;
- `SessionAdapter::summarize_file` and `details_page`; and
- provider-neutral session action descriptors.

That scaffolding is not yet the active history path. Rich timeline work must
finish and extend this cutover, not add more parsing to the old Codex-only
engine. This plan depends on the clean-cut command replacement described in
`PLAN_EXTENSIBLE_PROJECT_AGENT_PROVIDERS.md`.

### 2.4 Raw provider records are not interchangeable

Codex rollouts can contain canonical completed items, legacy display events,
and response items for the same logical activity. Unioning them double counts.
Some code-mode calls can also hide nested tool execution from the durable
top-level record set.

Claude JSONL uses user-role envelopes for both human prompts and tool results.
Multiple assistant records can share one API message ID, and compacted session
APIs may expose a shortened linked chain instead of the full raw history.

Mallard therefore needs versioned provider projections plus count quality. A
number that is only a lower bound must never look exact.

## 3. Settled product decisions

### D1 — One provider-neutral page

The detail identity is `resource_id + provider`, not a bare native thread ID.
Codex and Claude can use the same native ID without colliding. Provider names,
tool labels, actions, and capability gaps come from the adapter contract.

### D2 — A dedicated project subview, not a new router

Opening a session replaces the visible project overview inside the existing
workspace shell. The project sidebar and titlebar remain. A breadcrumb and Back
button return to the exact Activity state.

Do not add React Router solely for this feature. Mallard currently has no URL
navigation contract to preserve, and state-based navigation is already used
for Library and Settings.

### D3 — The session title is the navigation target

Do not turn the entire session card into a button; it already contains
checkboxes and action buttons. In normal browse mode, render the title and an
explicit `View session details` affordance as semantic buttons. Review-mode
rows do not navigate, and storage-only rows retain `Pull to open`.

### D4 — Preserve the Activity page in memory

Opening detail must not discard loaded 30-day windows, branch selection,
folds, comparison state, or scroll position. Keep the Activity controller
mounted but hidden and inert while the detail sibling is visible. On Back,
restore the prior workspace scroll position after layout.

### D5 — Replace the inline preview

Remove the legacy chevron disclosure, its `detailsByThread` cache, and
`get_project_chat_thread_details` when the dedicated page lands. Do not retain
an alias command or a second text-only parser.

### D6 — Intermediate means visible progress, never hidden reasoning

`Intermediate steps` includes assistant commentary, plan/progress text, or
text accompanying a tool-use phase that the provider made user-visible.

Version 1 excludes:

- raw or encrypted chain-of-thought;
- Codex raw reasoning content;
- Claude `thinking` and `redacted_thinking` blocks; and
- compaction summaries presented as if they were live reasoning.

If reasoning summaries are ever added, they require a separately named
capability and privacy review. They are not silently folded into this filter.

### D7 — Checkpoints are evidence-backed Git markers

A compaction record is not a checkpoint. A Claude restore point is also a
different feature from the commit-linked markers shown by Entire.

Version 1 uses only Mallard's session-to-Git correlation as a timeline
checkpoint source:

- the event carries commit SHA, subject, time, selected branch, and correlation
  quality;
- a non-Git project reports the capability as unsupported, not zero;
- an uncommitted session has no checkpoint event;
- the existing successor-commit correlation is marked `inferred` because no
  lifecycle hook proves it; and
- the contract supports many events, but current historical data will usually
  yield zero or one.

True Entire-style multiple checkpoints require a separate, explicitly approved
capture mechanism that records session/commit links when commits occur. This
detail feature must not guess them from every commit whose timestamp overlaps a
session.

### D8 — Tool classifications are conservative and mutually exclusive

Every unique tool call gets one primary display category:

1. `file_edit`
2. `shell`
3. `read`
4. `other`

The child counts therefore partition `Tool calls`; they do not create duplicate
timeline events. Shell commands that happen to read or modify files stay
`shell`. Unknown MCP or dynamic tools stay `other` unless an adapter has a
tested structural classifier.

The frontend never infers categories from display strings.

### D9 — Count quality is part of the contract

Every requested metric has one of:

- `exact` — the adapter can prove the count for this snapshot;
- `lower_bound` — malformed, oversized, unknown, or non-durable events may be
  missing;
- `inferred` — a documented correlation rule produced the count; or
- `unsupported` — the source cannot provide the metric.

Unsupported renders as `Unavailable`, not `0`. A lower bound renders as, for
example, `4+` with an explanation.

### D10 — Filtering happens before backend pagination

Counts describe the full session snapshot. A timeline request sends its event
and tool-category filters to the backend, which returns up to 50 matching
events in chronological order. Filtering only already-loaded rows would make a
long session appear empty or undercounted.

### D11 — Tool data is bounded, local, and collapsed by default

Arguments and results can contain credentials, full files, images, or very
large payloads. They are never written to Mallard metadata caches, logs,
portable manifests, or activity diagnostics. Timeline pages receive small
plain-text previews; one event can be expanded through a separately bounded
request. Binary, image, and base64-shaped content becomes a metadata
placeholder.

### D12 — Local transcripts first

The first implementation resolves an active local binding and re-proves local
transcript ownership. It does not download a storage-only transcript just to
render it. Verified, temporary, storage-backed inspection can be planned after
the local path is complete.

### D13 — Activity-only viewing pays no rich-detail cost

The Activity request path must not build or warm a timeline index, parse tool
arguments/results, pair tool events, prepare filtered pages, or read expanded
event details. It continues to use only the existing bounded session-summary
path and metadata cache. Rich parsing begins only after the user opens a local
session.

The frontend detail page is loaded through a separate lazy chunk on first
navigation, so Activity-only use does not parse or initialize the timeline UI.
The always-loaded session row gains only a stable resource ID and an open-detail
callback. This feature must not increase transcript I/O for a plain Activity
view; any future desire to show rich event counts on the Activity cards requires
a separate performance review.

### D14 — Conversation first; individual tool calls off initially

Opening a session detail page initially requests Prompts, Responses, and
supported Checkpoints. Intermediate steps and individual Tool call events are
off. This keeps the first rendered page focused on the conversation and avoids
shipping or mounting dozens of tool rows before the user asks for them.

Hiding individual calls must not erase evidence that they occurred. Every
logical turn with tools carries an always-visible summary such as
`39 tool calls`. The summary count comes from the same de-duplicated index as
the full-session count and carries the same exact/lower-bound quality. Its
disclosure is collapsed initially; opening it lazily requests only that turn's
bounded tool previews. The global Tool calls filter enables tool rows across
the timeline, but it never expands argument or result bodies automatically.

Supported child categories remain selected as preferences but are disabled
while the Tool calls parent is off. `Expand all tool calls` is also disabled
until individual rows are visible. No tool argument, result, or event-detail
request is prefetched for the initial page. This changes only the detail-page
cost; the stronger Activity-only guarantee in D13 remains unchanged.

### D15 — Prompts are contained; responses are open prose

Use asymmetric hierarchy rather than wrapping every timeline event in a card:

- a user Prompt uses one restrained neutral surface and border;
- an assistant Response renders as open prose directly on the workspace
  canvas, with no enclosing card background or border;
- Intermediate steps use quieter, lightly inset open prose;
- collapsed tool rows remain compact disclosures, while only expanded
  arguments/results receive contained code surfaces; and
- Checkpoints remain distinct full-width rail markers.

Keep Mallard's warm RetroTheme palette. Semantic colors belong on labels,
icons, focus states, and the chronological rail; they must not become large
tinted message panels. This hierarchy is a Mallard interpretation of the
reference interaction, not a copy of Entire's visual skin.

## 4. Experience specification

### 4.1 Visual thesis

**A quiet, audit-grade developer workspace: Mallard's existing neutral and
retro surfaces, a strong chronological rail, and one restrained provider
accent—dense but readable, with no dashboard-card mosaic.**

Use the current Mallard typography, spacing, icons, semantic colors, and theme
tokens. Do not copy Entire's near-black palette, introduce a new font, add
glass effects, or use decorative gradients. Conversation prose uses the
existing readable sans face; event metadata and tool payloads use the existing
mono treatment; small shell labels may retain the pixel face.

#### 4.1.1 Current RetroTheme contract

`src/RetroTheme.css` is the presentation authority. The default dark theme is
warm sepia/cream, not the navy/lilac palette in the older design-history
prototypes.

| Role | Active dark token/value |
| --- | --- |
| Frame gutter / app shell | `#040303` / `#0b0908` |
| Workspace / titlebar | `--bg-0: #12100d` / `--bg-2: #1a1714` |
| Cards / rows | `--surface-card: #171412` / `--surface-row: #14120f` |
| Borders | `--border: #2e2925`, `--border-mid: #443d36`, `--border-strong: #7a7065` |
| Primary / secondary / muted text | `#f6f1e6` / `#d8d2c5` / `#a59d8f` |
| Neutral accent | `--accent: #efe7d4` |
| Existing signal tokens | green `#63cd82`, red `#fb7361`, yellow `#f4b94f`, blue `#69c6f4`, purple `#bfa0f2` |

Follow the live typography contract as closely as the colors:

- `Departure Mono`/pixel face only for static chrome labels;
- system sans for prompts, responses, summaries, and other user-authored prose;
- system mono for timestamps, counts, IDs, SHAs, paths, arguments, and results;
- three-pixel near-black gutters between rounded shell tiles;
- approximately ten-pixel shell-panel radii and four-pixel row/control radii;
  and
- neutral buttons by default, with signal colors reserved for meaning.

The proposed event assignment uses only existing tokens: Prompt is red,
Response is blue, Intermediate step and Tool call are purple, and Git
checkpoint is green. Labels and icons carry the signal; large surfaces stay
neutral. The light theme must inherit the corresponding live light tokens
rather than use a hand-authored recolor.

Prompt and response must not share the same container treatment. The prompt's
neutral card establishes the start of a turn; the response is unboxed,
readable prose with spacing and typography providing hierarchy. Avoid a stack
of bordered message cards.

#### 4.1.2 Prototype images

The old prototype images were removed on 2026-10-09. Git history retains them.
They predate D14 and D15 and do not describe the current tool rows or response
bodies. Use this plan's requirements and semantic CSS tokens.

### 4.2 Layout

```text
Projects / Project name / Sessions / Session title          [provider actions]
< Back to Activity

Session title
summary · provider · branch · start/end · tokens · status
─────────────────────────────────────────────────────────────────────────────
Timeline                                             Inspector (sticky)
│ Prompt                                             Jump to checkpoint
│ ↳ 4 tool calls (collapsed turn summary)            ──────────────────
│ Intermediate step                                  Filters + counts
│ Response (open prose)                              [x] Prompts
│   Tool call (collapsed, when enabled)              [x] Responses
│   Tool call (expanded args/result, on demand)      [ ] Intermediate steps
◆ Git checkpoint                                     [x] Checkpoints
│ Prompt                                             [ ] Tool calls
│ ↳ 2 tool calls (collapsed turn summary)              [—] File edits
│ Response (open prose)                                [—] Bash / Shell
[Load 50 more]                                         [—] Read
                                                        [—] Other
                                                     Expand loaded tool calls (disabled)
```

The header is compact and functional. It reuses existing launch/fork actions
from adapter descriptors rather than inventing a new action bar.

### 4.3 Timeline behavior

- Start at the earliest event for a normal session entry.
- If the clicked visual occurrence is already associated with a Git commit,
  pass that occurrence as an optional initial anchor and land on its checkpoint
  after the header is available.
- Prompt and response receive the strongest hierarchy without receiving the
  same container: prompts use restrained cards and responses use open prose.
- Intermediate steps are quieter and off by default.
- A collapsed `N tool calls` summary remains visible for every turn containing
  tools even while individual Tool call events are filtered out.
- Expanding a turn summary lazily requests only that turn's bounded tool
  previews. Enabling the global Tool calls filter loads calls across the
  timeline.
- When enabled, Tool calls sit at their true chronological position and, when
  correlation is known, nest below the assistant event that initiated them.
- A tool row shows native tool name, compact target/command summary, category,
  time, and status before expansion.
- Expanded tool data uses labeled `Arguments` and `Result` plain-text code
  regions. It never uses raw HTML.
- Git checkpoints are full-width rail markers with short SHA and subject.
- `Load 50 more` appends chronologically and preserves the current scroll
  anchor. Add virtualization only after more than 50 rendered events; do not
  make virtualization a prerequisite for the first correct release.
- An active session exposes a deliberate Refresh action. Automatic polling is
  deferred until append-only index refresh is proven inexpensive.

### 4.4 Inspector behavior

Default state:

- Prompts: on
- Responses: on
- Intermediate steps: off
- Checkpoints: on when supported
- Tool calls: off
- every supported tool category: selected but disabled while Tool calls is off
- tool bodies: collapsed

Counts never change when a filter is toggled. The Tool calls parent uses a
tri-state checkbox when only some child categories are enabled. `Expand all`
applies to currently loaded tool rows, and newly loaded rows follow the active
expand state. It must not synchronously expand unrendered events and remains
disabled while Tool calls is off. Turn-level `N tool calls` summaries remain
visible regardless of the global Tool calls filter.

Selecting a checkpoint:

1. enables Checkpoints if necessary;
2. requests the filtered page anchored at the checkpoint when it is not
   loaded;
3. scrolls the workspace's single scroll root to the stable event anchor; and
4. moves programmatic focus to the checkpoint heading without trapping focus.

Filter state is page-local in version 1. It is not written to project or
portable state.

### 4.5 Narrow layouts

Use a container query because the resizable Mallard sidebar changes available
workspace width independently of the window.

| Container | Behavior |
| --- | --- |
| Wide | Timeline plus sticky right inspector |
| Medium | Narrower inspector; compact header metadata wraps |
| Narrow | Inspector becomes a `View options` disclosure above the timeline |

There is still one vertical workspace scroller. Avoid a nested scrolling
timeline beside a separately scrolling inspector.

### 4.6 Accessibility and motion

- Back is the first focusable control and returns focus to the originating
  session-title button.
- On open, focus the page heading after the view transition.
- Use semantic headings, `<fieldset>`/`<legend>` filters, `<details>` or
  button-controlled disclosures, and visible focus rings.
- Icon-only actions retain accessible names and tooltips.
- Status is never communicated by color alone.
- Keyboard users can reach every tool disclosure and checkpoint option.
- Page entry/exit may use a restrained 150–220 ms opacity/translation motion;
  tool disclosure uses the browser's normal layout response. Honor
  `prefers-reduced-motion` and do not animate height.

## 5. Normalized event semantics

### 5.1 Event kinds

| Kind | Codex projection | Claude projection |
| --- | --- | --- |
| Prompt | Prefer canonical completed `userMessage`; for legacy rollouts use filtered `event_msg.user_message`. Never union duplicate user response items. | Human-origin user text only. Exclude tool-result-only, meta, synthetic, compact-summary, and sidechain records. |
| Response | Completed `agentMessage` with `final_answer`; legacy final visible agent message after deduplication. | Deduplicate complete assistant records by API message ID; text with `end_turn` is final. Do not count the terminal result envelope. |
| Intermediate step | User-visible `commentary` or plan/progress item. In legacy data, visible assistant text followed by a tool call may be inferred. Exclude reasoning. | Visible assistant text in a tool-use phase. Exclude `thinking` and `redacted_thinking`. |
| Checkpoint | Common Git-correlation layer, not a rollout item. `compacted`/context compaction is ignored. | Common Git-correlation layer. Native prompt restore points and compaction are not mixed into this event kind. |
| Tool call | Prefer canonical completed tool items. For raw data, pair call/output records by call ID and do not union legacy duplicates. | One assistant `tool_use`/supported `server_tool_use` block, paired with following user-role `tool_result` by ID. The result is not a prompt. |

When a provider record family lacks an authoritative phase, the adapter may
infer final versus intermediate placement from the surrounding turn. That
metric and event carry `inferred` quality.

### 5.2 Tool categories

| Category | Codex | Claude |
| --- | --- | --- |
| File edit | Structured `fileChange` or direct `apply_patch` call | Exact `Write`, `Edit`, `NotebookEdit` |
| Bash / Shell | `commandExecution`, `local_shell_call`, or direct `exec_command`/shell command | Exact `Bash` (and a documented native PowerShell equivalent if present) |
| Read | An explicit durable `Read`/`read_file`-like call covered by a tested allow-list; otherwise unsupported or lower-bound | Exact `Read` |
| Other | MCP, dynamic, web, image, search, collaboration, `write_stdin`, and unknown calls | `Glob`, `Grep`, `LSP`, web, task, MCP, and unknown calls |

Do not parse arbitrary shell text to turn `sed`, `rg`, `cat`, or redirected
writes into Read/File edit. A direct shell invocation counts once as Shell.
`write_stdin` remains a tool call but is not a new shell process.

### 5.3 De-duplication and incomplete calls

- Prefer a provider's canonical completed item family when it is present.
- Otherwise select one legacy representation family; never union parallel
  display and response representations.
- A call start and its output are one event keyed by provider call ID.
- Completion is authoritative. An unmatched start remains one event with
  `incomplete` status and lower-bound session quality.
- Claude assistant chunks sharing one API message ID form one logical message;
  keep the most complete durable representation.
- Unknown record variants are ignored for display but make the relevant count
  lower-bound when they could contain activity.

## 6. Backend architecture

### 6.1 Adapter contract

Replace `SessionAdapter::details_page` with, or cleanly supersede it by, a rich
projection contract. Keep the raw format logic inside:

```text
crates/mallard-infrastructure/src/providers/codex/history.rs
crates/mallard-infrastructure/src/providers/claude/history.rs
```

Conceptually:

```rust
trait SessionAdapter {
    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext,
    ) -> Result<SessionTimelineIndex, String>;

    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
    ) -> Result<Vec<SessionTimelineEvent>, String>;

    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
    ) -> Result<SessionEventDetail, String>;
}
```

The common Activity engine owns project/profile resolution, resource lookup,
Git correlation, cache policy, and DTO conversion. It never switches on
`codex` or `claude` to parse records.

### 6.2 Public DTO shape

Names may be adjusted during the provider cutover, but the information contract
is fixed:

```rust
enum SessionCountQuality {
    Exact,
    LowerBound,
    Inferred,
    Unsupported,
}

struct SessionCount {
    value: Option<u64>,
    quality: SessionCountQuality,
    explanation: Option<String>,
}

struct SessionEventCounts {
    prompts: SessionCount,
    responses: SessionCount,
    intermediate_steps: SessionCount,
    checkpoints: SessionCount,
    tool_calls: SessionCount,
    file_edits: SessionCount,
    shell_calls: SessionCount,
    reads: SessionCount,
    other_tools: SessionCount,
}

struct ProjectSessionDetail {
    resource_id: String,
    provider: ProviderKey,
    provider_display_name: String,
    provider_session_id: String,
    title: String,
    summary: Option<String>,
    started_at: Option<u64>,
    ended_at: Option<u64>,
    is_active: bool,
    branch: Option<String>,
    recorded_sha: Option<String>,
    tokens: SessionCount,
    event_counts: SessionEventCounts,
    capabilities: SessionDetailCapabilities,
    actions: Vec<SessionActionDescriptor>,
    checkpoint_jumps: Vec<SessionCheckpointJump>,
    snapshot_id: String,
    warnings: Vec<SessionDetailWarning>,
}

enum SessionTimelineEventKind {
    Prompt,
    Response,
    IntermediateStep,
    Checkpoint,
    ToolCall,
}

struct SessionTimelineEvent {
    event_id: String,
    ordinal: u64,
    timestamp: Option<u64>,
    kind: SessionTimelineEventKind,
    quality: SessionCountQuality,
    turn_id: Option<String>,
    turn_tool_summary: Option<SessionTurnToolSummary>,
    parent_event_id: Option<String>,
    text: Option<String>,
    tool: Option<SessionToolCallPreview>,
    checkpoint: Option<SessionCheckpoint>,
}

struct SessionTurnToolSummary {
    turn_id: String,
    tool_calls: SessionCount,
}

struct ProjectSessionTimelinePage {
    resource_id: String,
    snapshot_id: String,
    events: Vec<SessionTimelineEvent>,
    next_cursor: Option<String>,
    warnings: Vec<SessionDetailWarning>,
}
```

Tool previews include provider call ID, native name, primary category, status,
short summary, bounded argument/result previews, truncation flags, and
project-relative affected paths when structurally known. Full local transcript
paths and provider-home paths never cross the Tauri boundary.

Every normalized event that belongs to a logical user/assistant turn carries
the same stable `turn_id`. `turn_tool_summary` appears once on that turn's
leading visible event, normally its Prompt; it is summary metadata, not a
synthetic Tool call event and does not affect event counts or pagination.

### 6.3 Commands

Replace the old detail command with:

```text
get_project_session_detail
get_project_session_timeline_page
get_project_session_event_detail
```

Inputs are always `localProjectId + resourceId` plus opaque snapshot/cursor or
event IDs. JavaScript keys remain camelCase.

`get_project_session_timeline_page` accepts:

- `snapshotId`;
- either `cursor` or `anchorEventId`;
- limit, clamped to 1–50;
- included event kinds;
- included tool categories; and
- an optional `turnId` for a bounded per-turn tool disclosure request.

The cursor binds provider, resource, snapshot, filter fingerprint, and next
index position. It never exposes a filesystem offset. A cursor from a changed
active transcript returns a typed `snapshot_changed` response so the frontend
can refresh rather than splice incompatible pages.

All filesystem/process work stays on `spawn_blocking`. Summary/detail requests
use per-resource single flight so rapid filter toggles cannot launch duplicate
full scans.

The ordinary Activity command must call only `summarize_file`; it must never
call `build_timeline_index`, `read_timeline_page`, or `read_event_detail` as a
prefetch optimization. Keep summary and timeline cache keys/entries separate so
an Activity cache miss cannot accidentally trigger rich-detail work.

### 6.4 Streaming index and cache

One bounded scan builds a metadata-only index:

- event ordinal, kind, phase, timestamp, and quality;
- provider record offsets and paired call/result offsets;
- stable logical turn IDs, per-turn de-duplicated tool counts, parent/call IDs,
  and tool category;
- counts, truncation state, and parser warnings; and
- the adapter timeline-parser version.

Do not cache prompt text, response text, tool arguments, tool results, file
contents, or binary data. Reuse Mallard's bounded, atomic, no-follow, mode-0600
cache policy and add parser version to invalidation. A snapshot fingerprint
includes resource identity, file size, nanosecond mtime, and adapter timeline
version. Active append-only refresh may extend an index only after validating
that the prior prefix still matches.

Convert Claude history from whole-file loading to the same bounded streaming
pattern used by Codex. Preserve the existing one-MiB ordinary-record limit and
selective oversized-record projection. The projector may retain only bounded
IDs, names, status, visible text fragments, and tool previews; image/base64 and
irrelevant fields must stream into ignored values.

Initial response limits:

| Data | Limit |
| --- | --- |
| Timeline events per page | 50 |
| Argument or result preview | 4 KiB |
| Expanded event detail | 64 KiB total |
| One Tauri timeline response | 1 MiB |
| Indexed events per session | bounded and diagnosed; align with the provider codec's existing record cap |

If a bound is reached, return a lower-bound metric and visible warning rather
than silently claiming completeness.

### 6.5 Lookup and ownership

Every detail, page, and expansion request must:

1. load the registered project and current binding;
2. resolve `resource_id` to one registered provider adapter and selected local
   profile;
3. canonicalize and compare saved project/profile paths;
4. find a regular, no-follow transcript below that provider home;
5. verify provider-native session identity and recorded cwd ownership;
6. reject symlinks, special files, traversal, stale snapshots, and ambiguous
   duplicate IDs; and
7. return only normalized, bounded local content.

Never accept a transcript path, provider-home path, command, or tool name from
the frontend as authority. Provider actions continue to use opaque action IDs
and are re-resolved immediately before execution.

## 7. Frontend architecture

### 7.1 Workspace navigation

Add an internal state owned by `ProjectSync`:

```ts
type ActiveProjectSubview =
  | { kind: "overview" }
  | {
      kind: "session_detail";
      localProjectId: string;
      resourceId: string;
      occurrenceKey: string;
      initialCheckpointId?: string;
    };
```

Pass `onOpenSessionDetail` through `ProjectLinksWorkspace` and
`ProjectChatHistoryPage` to the session title button. Keep
`ProjectChatHistoryPage` mounted when detail is active, but mark the overview
container `hidden`, `inert`, and `aria-hidden` as appropriate. Render the
detail page as a sibling inside `.sync-main`.

Load `SessionDetailPage` with `React.lazy`/dynamic import and mount its Suspense
boundary only when `ActiveProjectSubview.kind === "session_detail"`. Do not
initialize detail filters, request state, observers, or event components while
the overview is active.

Before opening, capture `.sync-main.scrollTop` and the originating button. On
Back, hide detail, restore the scroll position in `requestAnimationFrame`, then
restore focus. Clear detail on project, storage, Library, setup, Settings, and
review navigation, including reselecting the current project.

### 7.2 New modules

```text
src/components/project-sync/session-detail/
  SessionDetailPage.tsx
  SessionDetailHeader.tsx
  SessionDetailTimeline.tsx
  SessionDetailEvent.tsx
  SessionDetailToolCall.tsx
  SessionDetailInspector.tsx
  sessionDetailModel.ts
```

Responsibilities:

- `SessionDetailPage` owns request generations, pagination, refresh, and typed
  error states.
- `SessionDetailTimeline` owns chronological page merging and stable anchors.
- `SessionDetailEvent` renders prompt, response, intermediate, and checkpoint
  variants.
- `SessionDetailToolCall` owns one controlled disclosure and on-demand detail.
- `SessionDetailInspector` owns counts, filter fieldset, checkpoint jump, and
  expand state.
- `sessionDetailModel.ts` contains pure default-filter, count-label,
  page-merge, tri-state, and checkpoint-option helpers. It does not classify
  raw provider tools.

Do not add this page to the already oversized
`ProjectChatHistoryPage.tsx`. Add isolated base selectors to `App.css` and a
small presentation override block to `RetroTheme.css`; do not reuse coupled
commit-card selectors.

### 7.3 Rendering safety

Conversation text may use the installed `react-markdown` and `remark-gfm`, but
must not enable raw HTML. Restrict link protocols and route external links
through the app's vetted external-open behavior. Tool arguments/results render
only as escaped `<pre><code>` text. Never use `dangerouslySetInnerHTML`.

Loading, empty, partial, unsupported, stale-snapshot, and per-event expansion
errors have local visible states. A malformed tool result must not blank the
rest of the timeline.

## 8. Implementation sequence

### Phase 0 — Finish the provider Activity cutover

1. Export and wire the provider registry into the active history engine.
2. Resolve every bound provider independently and isolate provider failures.
3. Replace bare thread IDs in Activity/Git references with stable resource IDs.
4. Cut over the generic Activity, action, and details command names described
   by `PLAN_EXTENSIBLE_PROJECT_AGENT_PROVIDERS.md`.
5. Remove the old Codex-only parsing path rather than wrapping it.

**Exit:** Claude sessions can appear in the same provider-neutral Activity
contract, and no production history command hard-requires Codex.

### Phase 1 — Freeze event semantics with fixtures

1. Add minimal synthetic Codex canonical and legacy rollout fixtures.
2. Add minimal synthetic Claude raw JSONL fixtures.
3. Encode prompt/result discrimination, phase selection, de-duplication, tool
   pairing, category mapping, and quality rules as adapter tests.
4. Document every accepted provider record family and unknown-variant behavior
   beside the parser.
5. Add an instrumented adapter test proving an Activity-only request calls the
   summary method but none of the timeline methods.

**Exit:** the requested counts are deterministic without reading a real
provider home.

### Phase 2 — Build the bounded timeline projection

1. Add the common timeline types and adapter methods.
2. Implement streaming indexes for Codex and Claude.
3. Extend the oversized Codex projection with bounded tool fields.
4. Replace Claude's whole-file history read with bounded streaming.
5. Pair calls/results, assign stable logical turn IDs, compute de-duplicated
   per-turn tool summaries, emit normalized events, and produce
   quality/warnings.
6. Join the existing Git successor correlation as an inferred checkpoint
   event; preserve non-Git/uncommitted capability states.
7. Add metadata-only cache entries keyed by parser version and snapshot.

**Exit:** Rust tests can page/filter complete synthetic sessions without
unbounded reads or cached content.

### Phase 3 — Add the Tauri and TypeScript contract

1. Add the three generic detail commands and mirror every DTO in `src/types.ts`.
2. Add single-flight wrappers to `projectSyncApi`.
3. Register commands and update the command registration-freeze test.
4. Add typed stale-snapshot, unsupported, partial, and not-local errors.
5. Remove `get_project_chat_thread_details` and its DTOs in the same cutover.

**Exit:** a frontend fixture can retrieve header/counts, filtered pages, and
one bounded tool detail for either provider.

### Phase 4 — Add dedicated navigation

1. Add `ActiveProjectSubview` and explicit navigation-clear rules.
2. Convert the browse-mode session title to the detail action.
3. Preserve Activity mounting, state, scroll, and focus on open/back.
4. Lazy-load the detail component chunk only after session navigation.
5. Remove the inline preview disclosure and frontend detail cache.
6. Keep review rows and storage-only rows non-navigating.

**Exit:** opening and closing a placeholder detail page loses no Activity
state and creates no invalid nested interactive element.

### Phase 5 — Build the complete page

1. Add header, breadcrumb, actions, timeline, event variants, and inspector.
2. Render prompt cards and unboxed response prose using semantic RetroTheme
   tokens.
3. Start with individual Tool calls off, retain per-turn tool summaries, and
   lazy-load one turn's tool previews when its summary is opened.
4. Add filter-aware pagination, checkpoint anchoring, and per-event expansion.
5. Add count quality labels, warnings, unavailable states, and partial results.
6. Add container-query narrow behavior and reduced-motion styling.
7. Verify the page in both Mallard themes using Codex and Claude fixtures.

**Exit:** the page satisfies every acceptance criterion below.

### Phase 6 — Hardening and visual QA

1. Profile a near-limit synthetic transcript and rapid filter changes.
2. Inspect the cache to prove it contains no conversation or tool content.
3. Run keyboard-only and screen-reader-oriented checks.
4. Run the complete Tauri app and capture wide and narrow screenshots.
5. Verify active-session refresh and stale cursor recovery.

## 9. Test plan

### 9.1 Provider parser fixtures

Codex cases:

- canonical completed prompt/response/commentary/tool items;
- legacy parallel `event_msg` and `response_item` duplicates;
- final answer versus commentary phases;
- direct shell, patch, Read-like, MCP, web, and unknown tools;
- code-mode outer calls with unobservable nested activity;
- missing tool result and failed tool result;
- compaction and reasoning exclusion;
- malformed and unknown records;
- oversized argument/result/image records; and
- active append followed by snapshot change.

Claude cases:

- human prompt versus tool-result-only user envelope;
- duplicate assistant chunks sharing one message ID;
- assistant text ending in tool use versus final text;
- `tool_use`/`tool_result` pairing;
- Edit, Write, NotebookEdit, Bash, Read, and Other tools;
- meta, sidechain, synthetic, and compact-summary exclusion;
- thinking/redacted-thinking exclusion;
- incomplete and malformed records; and
- oversized content under the existing file cap.

### 9.2 Common backend tests

- identical native IDs under Codex and Claude remain distinct resources;
- project/profile canonical-path mismatch and symlink substitution fail closed;
- transcript ID/cwd ownership is re-proved for every command;
- cursor/filter tampering and stale snapshots are rejected;
- server-side filters return 50 matching events, not 50 raw events;
- tool child counts partition the unique tool total;
- per-turn tool summaries match the de-duplicated calls carrying that turn ID,
  do not create synthetic Tool call events, and remain available when Tool
  call events are filtered out;
- a `turnId` tool request cannot return calls from another turn, resource, or
  provider;
- exact/lower-bound/inferred/unsupported values serialize correctly;
- Git checkpoint placement, non-Git unsupported, and uncommitted empty cases;
- cache version invalidation, size eviction, atomic mode-0600 persistence, and
  proof that content strings are absent;
- one bad provider does not hide another provider's safe Activity rows; and
- an instrumented adapter records no timeline-index/page/detail calls during an
  Activity-only request, including an Activity cache miss.

### 9.3 Frontend integration tests

Extend `tests/frontend/project-chat-history.integration.test.tsx` for:

- browse-mode session-title navigation;
- no whole-card/nested button;
- review-mode and storage-only non-navigation;
- removal of the inline disclosure; and
- existing launch, fork, metrics, and debug behavior.

Add `tests/frontend/session-detail.integration.test.tsx` for:

- all event kinds and tool categories;
- default filters with Tool calls off, selected-but-disabled child categories,
  and Tool calls tri-state behavior after enabling the parent;
- visible per-turn tool summaries while individual tool rows are absent, plus
  one bounded lazy request when a turn disclosure opens;
- no initial tool-event-detail request and no automatic argument/result
  expansion when Tool calls is enabled;
- contained Prompt styling and open, unboxed Response styling;
- full-session counts remaining stable under filters;
- lower-bound and unavailable labels;
- checkpoint options and stable anchors;
- expand one/all loaded tool calls;
- loading, empty, partial, stale, malformed, and paginated states;
- Markdown raw-HTML and link-protocol safety;
- breadcrumb, heading, focus, and fieldset semantics;
- wide, medium, and narrow markup/classes; and
- the session-detail chunk and controller are absent before navigation and load
  only after the session action is invoked.

Add workspace-navigation coverage for preserved Activity state, scroll, focus,
and every route that clears session detail.

### 9.4 Required handoff checks

```text
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Then run the complete Tauri application with temporary fixture homes and
perform visual QA. Never point tests at real `~/.codex`, `~/.claude`, or
`~/.mallard`.

## 10. Acceptance criteria

1. Clicking a local Codex or Claude session title in browse mode opens a
   dedicated page inside the selected project workspace.
2. Back returns to the same branch, loaded windows, folds, comparison, scroll
   position, and originating focus target.
3. The page renders chronological Prompt, Response, Intermediate step, and
   Checkpoint events when supported, and renders Tool call events when the user
   enables or expands them.
4. The inspector shows full-session counts for Prompts, Responses,
   Intermediate steps, Checkpoints, Tool calls, File edits, Bash/shell, Read,
   and Other.
5. Tool subcategories partition unique tool calls, and results never count as
   prompts or new calls.
6. Exact, lower-bound, inferred, and unsupported data are visibly distinct; an
   unsupported capability never renders a false zero.
7. Filters operate across the full session before pagination. Checkpoint jump
   can load and focus an event that is not yet rendered.
8. Tool arguments/results are escaped, bounded, collapsed by default, and
   absent from logs, caches, manifests, and portable metadata.
9. Provider-native IDs cannot cross provider boundaries, and every request
   revalidates canonical ownership.
10. A near-limit transcript remains bounded and responsive; malformed or
    unknown records degrade only the affected metric/event.
11. Storage-only and unknown-provider sessions clearly explain why details are
    unavailable and do not trigger implicit Pull/download.
12. The legacy inline details command, DTO, parser path, and UI are removed.
13. Viewing or refreshing Activity without opening a session performs no
    timeline indexing, tool-payload parsing, event pairing, detail prefetch, or
    detail-component initialization beyond the existing bounded summary work.
14. A newly opened detail page has individual Tool calls off, keeps an
    `N tool calls` summary visible for each applicable turn, and makes no tool
    argument/result request until the user expands a tool row.
15. Prompts use restrained neutral cards, Responses render as open prose with
    no outer card surface or border, and both dark and light presentation come
    from live RetroTheme tokens.

## 11. Explicit non-goals and follow-ups

Not part of the first implementation:

- displaying raw chain-of-thought or Claude thinking blocks;
- treating compaction as a checkpoint;
- installing Git/provider hooks to create exact multi-checkpoint history;
- rewinding files to Claude native restore points;
- inspecting storage-only transcripts without Pull;
- editing, replaying, or rerunning tool calls;
- searching across all sessions;
- exporting transcripts; or
- adding a browser URL/deep-link router.

Potential follow-ups should be separate plans: exact commit checkpoint capture,
temporary verified remote inspection, transcript search/export, and provider
restore-point actions.

## 12. Research sources

- Entire overview and checkpoint model:
  <https://docs.entire.io/overview>
- Entire agent lifecycle hooks:
  <https://docs.entire.io/agents/overview>
- Entire terminology for sessions and checkpoints:
  <https://docs.entire.io/glossary>
- Entire CLI source and transcript behavior:
  <https://github.com/entireio/cli>
- Codex app-server thread/turn/item model:
  <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>
- Claude session storage:
  <https://code.claude.com/docs/en/sessions>
- Claude Agent SDK message types:
  <https://code.claude.com/docs/en/agent-sdk/typescript>
- Claude built-in tool names:
  <https://code.claude.com/docs/en/tools-reference>
- Claude checkpoint limitations:
  <https://code.claude.com/docs/en/checkpointing>
- Claude tool-use/result contract:
  <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls>

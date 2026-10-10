# Session page messages: chips, not boxes

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

- Status: owner picked **Chips only** on 2026-10-03, after ruling earlier the
  same day that *up to date* is not shown ("Up to date one doesn't have to
  show"). Implemented, uncommitted. The live Tauri pass is still open.
- Date: 2026-10-03
- Scope:
  - the two message zones around the session page's fact strip:
    `ProjectLinksWorkspace`'s "Session Push status" zone and
    `SessionDetailPage`'s own zone;
  - the caveat sentences the transcript parsers write
    (`providers/claude/history.rs`, `providers/codex/history.rs`).
  - The path bar and the transcript's rows are not changed.
- Visual contract: `mockups/session-notices-mockup.html`, preset *Chips only*
  (now the specimen's default). The shipped header is checked live in
  `mockups/session-notices-live.html`.

## Lineage

The owner sent a screenshot of a finished Claude session. It had two boxes,
one either side of the fact strip: *Note · This session is up to date.*
above, and *Warning · At least one Claude tool-use block has no durable
matching result.* below. The owner asked for a better design, mock first.

The specimen offered two axes:
- **Where a Push hold goes:** the page's box, its own box, or a chip under
  Push.
- **How caveats show:** footnote, box, notes chip, or a mark on the SHOW bar.

The owner ruled *up to date* out, asked for a plan, and then picked the
*Chips only* preset. The two questions left open in the plan were not
answered, so the recommended defaults stand:
- an interrupted call is a Note, hidden while the session runs;
- holds nobody can clear from the page stay in Push's tooltip.

## What was wrong

| # | Fact | Where |
|---|---|---|
| 1 | Two boxes sat either side of the fact strip. In the screenshot scene the SHOW bar started **170 px** below the path bar. With chips it starts 71 px below (measured in the specimen, 990 px pane). | `ProjectLinksWorkspace.tsx`, `SessionDetailPage.tsx` |
| 2 | The box above the strip drew a *status*, not a message. Push already says *up to date* and *Checking session…*: it is disabled, with the same sentence as its tooltip. Push's count badge says *ready*. | `sessionPushIssue`, `sessionPushDisabledReason` |
| 3 | The status box came and went on its own. *Checking session…* showed whenever the activity comparison reloaded (`refreshing = loading \|\| comparisonLoading`): on the bar's refresh, and after a Push or Pull. When the session turned out to be pushable, the box unmounted and the page **moved 52 px**. | same; `ProjectActivityPage.tsx` ~2792 |
| 4 | A comparison failure was drawn at the **Note** rung: the zone hard-coded `severity: "info"`. | `ProjectLinksWorkspace.tsx` |
| 5 | Caveats were printed in the parser's own words: "tool-use block", "durable matching result", "terminal stop reason", "canonical item". | both `history.rs` |
| 6 | An interrupted tool call was a **Warning**, though the transcript holds no result to miss. A *running* session warned too, because its in-flight call is `Incomplete` until the result lands. | `incomplete_tool_call` |
| 7 | The warning pointed at rows nobody could see. Tool calls are off by default, and nothing offered to turn them on. | `createDefaultSessionDetailFilters` |

## What the user sees

| Kind | Examples | Form |
|---|---|---|
| **Status** | up to date, checking, ready to push | **Nothing on the page.** Push says it: disabled with its tooltip, the count badge, the spinning refresh mark. |
| **Hold**: Push is held and the user clears it | Pull first (`require_pull`, `require_pull_metadata`, `blocked` + `detached_requires_pull`); couldn't compare | A **chip at the head of the strip's verbs**, right under Push. *Pull first* is in Note ink with the download glyph; *Can't compare* is in Error ink with the octagon. The full sentence is the tooltip and is printed for a screen reader. |
| **Event**: something that happened while the page was open | an action failed; the snapshot moved | The page's **one box** under the strip, as before. It is always `blocking`, because both arrive after paint. |
| **Caveat**: how the transcript was read | interrupted calls, a cut-short message, unreadable lines | **One chip at the end of the facts**, showing the loudest rung's glyph and *N notes*. It opens a panel titled *Notes on this session* with one line per note: glyph, **rung ·**, sentence. |

Details:

- **Interrupted calls.** The note on interrupted calls ends with **Show tool
  calls** while the Tool calls filter is off. The link turns the filter on,
  closes the panel and returns focus to the chip.
- **Running sessions.** While the session runs, the interrupted-call note is
  dropped: the last call is in flight, and its row already says
  `incomplete`.
- **Comparison failures under Mallard Cloud.** A failure the Cloud chip
  already reports is not said again (`managedCloudKey`). This is the same
  rule the activity list uses.
- **Chips never close.** This follows the 2026-09-25 rule (messages close,
  labels don't).
  - Caveats used to be messages with a ×; now they are labels. The owner
    accepted this by picking the preset.
  - A caveat never announces. Before, timeline warnings arriving after paint
    re-timed the box to `role="alert"`. Now they are read in place.
- **Accessibility of the panel.**
  - The panel is `PathBarPopover`'s off-bar branch: `TopLayer` on `body`,
    placed against the window, closed by an outside click, Escape or scroll,
    with Escape returning focus.
  - Focus enters the panel only when it holds a button. So the sentences are
    also printed visually hidden beside the chip.

## Implementation (as built)

- `session-detail/sessionDetailModel.ts`
  - Adds `SessionPushGate` and
    `sessionPushGate(entry, comparisonError)`.
  - A comparison error returns the danger chip. The three Pull decisions
    return *Pull first*, with `sessionPushDisabledReason` as the detail.
    Anything else returns `null`.
  - `sessionPushDisabledReason` is unchanged; it still owns Push's tooltip.
- `ProjectLinksWorkspace.tsx`
  - Adds `sessionComparison`: the activity header once it is current for
    this project and storage and not refreshing. It feeds both
    `sessionPushIssue` and the new `sessionPushHold`.
  - The "Session Push status" `NoticeZone` is deleted.
  - The page receives `pushGate={workspaceReviewOpen ? null : sessionPushHold}`.
- `session-detail/SessionDetailPage.tsx`
  - New `pushGate` prop.
  - The zone holds only the failed action and the moved snapshot, with
    `timing="blocking"`.
  - The merged warnings (minus `incomplete_tool_call` while `is_active`) go
    to the header as `notes`.
  - `onShowToolCalls` is set only while `filters.toolCalls` is off.
- `session-detail/SessionDetailHeader.tsx`
  - The hold chip leads `.session-detail-header-actions`.
  - `<SessionDetailNotes>` ends `.session-detail-chips`.
- `session-detail/SessionDetailNotes.tsx` (new): the notes chip and its panel
  through `PathBarPopover`, plus the in-flow, visually hidden sentences.
- `notice/Notice.tsx` exports `SEVERITY_LABEL` and `SEVERITY_ICON`, so the
  rung words and glyphs come from the one ladder.
- `App.css`: `.session-detail-push-gate`, `.session-detail-notes-chip`,
  `.session-detail-notes`, `.session-detail-note*`, after `.session-detail-chip b`.
  Measured contrast on their own surfaces is at least 5.44:1 in light and
  7.42:1 in dark.

### Backend: plain words, one rung

Both parsers write the sentence. The codes are unchanged. The page already
names the provider, so the words drop it.

| Code | Now |
|---|---|
| `incomplete_tool_call` | **Note** (was Warning) · Tool calls marked incomplete never recorded a result. They were most likely interrupted. |
| `orphan_tool_result` | Warning · A tool result had no matching call and is left out. |
| `message_preview_truncated` | Warning · Messages over 1 MiB are cut short here. The session file keeps the full text. |
| `message_phase_inferred` | Note · Some replies had no end marker, so Mallard guessed which are final replies and which are steps. |
| `legacy_message_phase_inferred` | Note · This session predates Codex's reply markers, so Mallard guessed which are final replies and which are steps. |
| `timeline_event_limit` (every variant) | Warning · This session has more events than Mallard can show. The rest are left out. |
| `record_limit` | Warning · The session file is longer than Mallard can read. The rest is left out. |
| `oversized_record` | Warning · A record in the session file is too large to read and is left out. |
| `malformed_record` | Warning · Some lines of the session file couldn't be read and are left out. |
| `malformed_assistant_record`, `malformed_assistant_content`, `malformed_canonical_item`, `malformed_response_item`, `malformed_event` | Warning · Some entries of the session file couldn't be read and are left out. |
| `tool_use_missing_id`, `tool_result_missing_id` | Warning · Some tool calls couldn't be paired with their results. |
| `unknown_tool_block`, `unknown_canonical_item`, `unknown_response_item`, `unknown_tool_event` | Warning · This session has entries this version of Mallard doesn't know. They are left out. |
| `affected_path_limit` | Warning · A tool call changed more files than Mallard can list. |

- `incomplete_tool_call` sets `degraded_tools` itself and passes
  `NoticeSeverity::Info`. Before, it went through `mark_tools_lower_bound`,
  which hard-codes Warning.
- **Cached indexes keep the old words.**
  - The timeline index is cached in
    `~/.mallard/session_timeline_cache/` together with its warnings. It is
    reused while the parser version matches. Claude's version is pinned at
    1 (the schema-numbers rule), so no version was bumped.
  - A session whose transcript has not changed keeps the old sentence and
    rung until its file changes or that cache folder is cleared.
  - The page's code-keyed rules (hide while running, *Show tool calls*)
    work either way.
- The count explanations (`... may be absent because ...`) are tooltips on
  counts, not caveats. They are unchanged.

## Tests

- `tests/frontend/push-budget.integration.test.tsx`: `sessionPushGate`
  returns `null` for:
  - up to date;
  - ready;
  - no entry;
  - no local copy;
  - an unclearable blocker.

  It returns *Pull first* for both Pull decisions and for a detached
  blocker, and *Can't compare* (danger) when the comparison failed.
- `tests/frontend/session-detail.integration.test.tsx`, "a Push hold and the
  transcript notes are chips, never boxes":
  - the hold chip leads the verbs;
  - the notes chip takes the loudest rung and the count, with a closed
    dialog;
  - no `.notice` and no Dismiss;
  - the hidden sentences sit in flow;
  - nothing is drawn without a hold or notes;
  - the workspace has no Push status zone and passes the hold;
  - the page's box is `blocking`, and running sessions drop the
    interrupted-call note.
- Rust: Claude's incomplete-call test asserts `NoticeSeverity::Info`. A new
  Codex test, `a_call_without_output_is_an_incomplete_note_not_a_warning`,
  pins the rung and the sentence. The preview-boundary test now matches
  "1 MiB".
- Live harness `mockups/session-notices-live.html`, real header, client
  mounted:
  - the panel opens on `body`, 6 px under the chip;
  - **Show tool calls** turns the filter on, closes the panel and refocuses
    the chip;
  - Escape, an outside click and a scroll each close the panel;
  - at the 580 px pane the frames show no overflow in either theme.
- Open: a live Tauri pass with three sessions:
  - a finished Claude session with an interrupted call (clear its cache entry
    first to see the new words);
  - a running session;
  - a project whose storage has changes.

## Website (mallardInternal/cloudflare/managed-cloud/web)

The owner asked for the web app too, mock first (`web/session-notes-mockup.html`), then
picked the chip there as well ("Go with the chip on the web too", 2026-10-03). Implemented,
uncommitted, **not deployed**: staging and production run the old page until the owner says
to deploy.

- **What the page did.** `SessionPage` stacked one `Notice` box per `data.warnings` entry
  between the header and the timeline.
- **What it does now.** A new `web/src/session/SessionNotes.tsx` chip ends `.chips`. It uses
  the loudest note's ink and shows *N notes*. The panel is the Invite popover's box
  (`.invite-pop.notes-pop`), with PullMenu's close rules: outside pointer or Escape, and
  Escape returns focus to the chip.
  - **Placement.** The panel opens from the chip's left edge and flips to its right edge when
    that would run past the window (measured before paint). At 640 px or narrower it becomes a
    bottom sheet.
  - **Show tool calls.** The interrupted-call note offers it while the toggle exists and is
    off.
  - **Screen readers.** The notes are printed in flow in the new site-wide `.visually-hidden`
    utility.
- **Other changes.**
  - `Notice.tsx` exports `SEVERITY_LABELS`.
  - CSS sits after `.chip b` in `styles.css`.
  - Measured contrast is at least 5.66:1 light and 8.78:1 dark.
- **Not ported.** Stored views carry no `is_active`, so the hide-while-running rule does not
  apply on the web.
- **Left as they were.** The "truncated preview" title pill and the bottom *Bounded preview*
  box are untouched; the owner was asked whether they should fold into the chip and has not
  said.
- **Stale words.** Views already pushed keep the words they were pushed with.
- **Checks.**
  - `web/test/session_notes.test.ts` covers the closed chip, the rung, the count, the hidden
    sentences, no Dismiss, and the page source.
  - `npm run test:web` passes 241/241, `npm run typecheck` passes, and the web build passes.
  - `node scripts/icons.mjs check <mallard-app>` passes 302 comparisons.
  - The mock's chip view mounts the shipped component. In it: open; Show tool calls → toggle
    on, panel closed, focus on the chip; Escape; an outside pointer; the flip at 720 px; the
    sheet at 375 px.

## Out of scope

- The path bar and Push's own tooltip copy.
- The per-row `Partial` and `incomplete` chips.
- Counting how many calls are incomplete. The parser dedups the warning by
  code, and the rows already show which calls they are.

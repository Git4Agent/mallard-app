# Message model and thinking labels

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

- Status: planned and mocked 2026-10-07 as a per-message stamp (A); the owner
  then asked for a standards-backed alternative, picked **D** from eight
  (`mockups/message-model-directions-mockup.html`), and D replaced A on both
  trees the same day, uncommitted (`mallard-app` and `mallardInternal`). Every
  suite in **Tests** is green. Open: the native Tauri pass on temporary
  provider homes, and the website deploy.
- Date: 2026-10-07
- Scope:
  - the desktop session page's fact strip and rail (`SessionDetailHeader.tsx`,
    `SessionDetailTimeline.tsx`, `SessionModel.tsx`) and the website's
    (`web/src/pages/SessionPage.tsx`, `web/src/session/SessionTimeline.tsx`,
    `web/src/session/ModelNotes.tsx`);
  - the Codex and Claude transcript parsers
    (`providers/codex/history.rs`, `providers/claude/history.rs`), their cached
    index, and the event DTO the page and the Mallard Cloud view share;
  - not changed: steps, tool rows, agent rows, checkpoints, the session header,
    and the Push/Pull flows.
- Visual contract: `mockups/message-model-mockup.html`
  (`?provider=codex|claude&scene=change|missing|mixed&theme=dark`). The
  website's is `web/message-model-mockup.html` in `mallardInternal`.
- Evidence: `poc/message-model-info/README.md`. The POC proved the fields
  exist in both clients' saved records; this plan is the production path.

## What the user sees

What the session started with is one chip among the header's facts, in mono:
`claude-opus-4-6 · low`. The transcript marks only the changes: a note on
the rail before the message a change applies from, `claude-sonnet-4-6 ·
xhigh` on a rule, the way the day divider draws its date. Chip and note are
buttons; each opens the path bar's popover box with three lines:

| Line | Value |
|---|---|
| Model | the recorded model id, in mono |
| Thinking | the recorded effort, exactly as written: `low`, `high`, `xhigh` |
| Source | Turn settings (Codex), Reply record (a Claude reply), Linked replies (a Claude input), Not recorded |

Rules:
- A message carries one model and one effort, as its own record (a reply, a
  Codex turn) or, for a Claude input, the first reply that answered it.
- A field the transcript never recorded reads **Unknown**; a session whose
  first message recorded neither starts with the one word `Unknown`, and the
  first note says where recording began.
- A session in which no message recorded anything has no chip and no notes.
- A message that recorded nothing says nothing: it neither gets a note nor
  moves the state. Two replies to one input at different levels read as a
  note between them, so there is no Mixed state.
- Nothing is read from the current profile or `settings.json`.

Seen in: ChatGPT's per-message note when the answering model differs from the
picker; Claude Code's session header ("with low effort") and its transcript
notice on `/model`. Primer, NN/g and Carbon ask that a fact the reader needs
stay on the page, and the app's own rule says each fact once.

## Data

One shape, in `providers/traits.rs` and `src/types.ts` (and the website's
`web/src/types.ts`):

```rust
pub struct SessionMessageSettings {
    pub model: Option<String>,   // absent when the transcript recorded none
    pub effort: Option<String>,
    pub source: SessionMessageSettingsSource, // turn | reply | linked | none
}
```

- `SessionTimelineEvent.settings: Option<SessionMessageSettings>` is set on
  `Prompt` and `Response` events only (`skip_serializing_if` when absent).
- `ProjectSessionDetail.settings: Option<SessionMessageSettings>` is the
  first message's settings, present when any message recorded a value.
- `SessionTimelineIndexEntry.settings: SessionMessageSettings` is required on
  every cached entry; a tool, agent or step entry carries the empty one. A
  cache written before this change lacks the key, fails to parse and is
  rebuilt. No parser version or schema number is raised: every schema stays 1.
- A value is admitted only as an ASCII token (`[A-Za-z0-9][A-Za-z0-9_.:/+-]*`,
  at most 128 bytes). Both fields absent force `source: none`.
- The index stays metadata-only: a model id is a setting, not message text.

## Codex

`turn_context` records carry `payload.turn_id`, `payload.model` and
`payload.effort`; one is written per turn, before the turn's items.

- The index builder collects each turn's contexts in file order.
- A canonical `UserMessage` or `AgentMessage` item takes the last context of
  its own `turn_id` written at or before it. A user item saved before its
  context takes the turn's first context. Never another turn's; an item of a
  turn with no context reads Unknown.
- The legacy event stream has no turn ids on items: an item takes the latest
  context seen so far, which is its own turn's by construction.
- Source is `turn` for both inputs and replies.
- A view pushed to Mallard Cloud projects some out-of-turn completions into
  synthesized `rollout-N` turns; those have no context and read Unknown there.

## Claude

- A visible assistant record gives `message.model` (never `<synthetic>`) and
  the outer `effort`. Records sharing `message.id` are one event; its values
  are the group's distinct values. Source `reply`.
- An input is linked to replies through `parentUuid`: from each assistant
  group's records, the chain is walked parent by parent, through tool results
  and attachments, until it reaches a record that is a prompt event. The walk
  stops at a missing parent, a cycle, or a record of another `sessionId`.
- An input wears the earliest linked group that recorded something (replies
  and steps alike). Source `linked`; with none, `none`. A group that recorded
  nothing, such as an API error with a `<synthetic>` model, lends nothing.
- `perTurnEffort` is not read. On the owner's machine it always equals
  `effort`; the POC saw it null. The outer `effort` is the recorded setting.

## Mallard Cloud and the website

- The view carries the same event struct, so `settings` travels with no
  mapping. `VIEW_SCHEMA` stays 1; the service validator admits the field when
  present and well formed, and the page draws no stamp when it is absent.
- Views are keyed by transcript digest and never rewritten: a session pushed
  before this change shows no stamp until its transcript changes and is
  pushed again.
- The website derives the chip and the notes from the view's events
  (`sessionStartSettings`, `sessionModelNotes`); the panel uses the native
  `popover` top layer so no ancestor's overflow clip can cut it.

## Tests

- Codex (`codex/history.rs`): a model change across two turns; a turn with
  no effort; a user item saved before its context; a turn without a context;
  the legacy stream.
- Claude (`claude/history.rs`): a model change across two inputs; a reply
  without the fields; two replies to one input that differ (the input wears
  the first); a `<synthetic>` model; a parent of another session.
- Cache (`session_detail.rs`): an entry without `settings` is a miss.
- Frontend (`tests/frontend/session-detail.integration.test.tsx`): the header
  chip for recorded, Unknown and absent; the notes walk; one note on the
  rail, none without a change.
- Website (`web/test`): the start settings and notes from a view's events;
  the chip and the note's markup; none for a view without the field; the
  worker rejects a malformed `settings`.
- Handoff: `npm run build`, `npm run test:frontend-integration`,
  `cargo test --workspace --lib`, the website's `npm run typecheck` and
  `npm run test:web`; then the native app on a temporary provider home.

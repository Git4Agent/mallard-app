# Session detail: agent rows v2 — a delegation is an exchange

Date: 2026-09-17. Status: **proposed**. Mock first: pick a direction in
`subagent-exchange-mockup.html` before any code moves. Clean cutover when it
lands: no migration, local state and cloud schemas stay at 1; the timeline
parser versions bump (Claude 1 → 2, Codex 6 → 7), so cached indexes rebuild
on first open.

Supersedes nothing: `DESIGN_SESSION_AGENT_ROWS.md` (direction A, shipped
2026-09-09) stays. This is the second layer on the same row.

## Why, measured

Counted today over the local corpora, both providers:

| | Codex (744 parent rollouts with activity) | Claude (49 parents with a `subagents/` dir) |
| --- | --- | --- |
| Delegations | 1,805 `started` | 161 `Agent` calls |
| Follow-ups | **4,781 `interacted`** | 30 `SendMessage`, 18 `TaskOutput`, 16 `TaskStop` |
| Ends the parent recorded | 294 `completed` (16%), 170 `interrupted` (9%) | 48 sync results (31%), 108 async (69%) |

Three things follow, and they are what this design fixes.

1. **A delegation is a conversation; the row shows a one-shot.** Codex
   averages 2.6 `interacted` events per spawn — the parent keeps talking to
   the child and the child keeps answering. Claude does the same through
   `SendMessage` / `TaskOutput` / `TaskStop`, which today render as ordinary
   tool rows somewhere else in the timeline, unlinked from the agent they
   address. The open row prints a brief and a final report and drops
   everything between.
2. **The fact chips are empty in the common case.** Codex hands the row
   `model: None, tokens: None, tool_calls: None` unconditionally; Claude
   fills counts only from a synchronous result, which is 31% of its calls.
   So the row in `subagent-detail-mockup.html` — role · model · mode · span ·
   calls · tokens — is a shape the shipped code can rarely fill.
3. **A burst reads as unrelated rows.** Codex issues three spawns in one
   parent turn; Claude never puts two `Agent` blocks in one assistant
   message (161 of 161 hold exactly one) but launches them seconds apart —
   counted by 2-minute proximity, the corpus holds 21 pairs, four triples,
   four quads and single bursts of 7, 8, 9 and 15. They stack as N
   sequential rows with no sign they ran at once. **The grouping key is
   therefore overlap in time, not the record's shape**; Codex's shared
   `turn_id` only confirms it.

## What each provider can prove, and what it costs

The discriminator: **a fact is layer 1 only if it is readable from records
the page already reads** — the parent record for the row, the child's first
line, the child's 256 KiB tail. Layer 2 needs a full child parse, which the
shipped cost note explicitly forbids ("Nothing parses a child in full").

### Layer 1 — free

| Fact | Claude | Codex |
| --- | --- | --- |
| Brief | `Agent` input `prompt` (also on the async result as `result.prompt`) | none: the spawn `message` is an encrypted token, and the child rollout opens with the environment preamble, not the brief |
| What the child started from | — | spawn arguments `fork_turns`: `all` \| `<n>` \| `none` — the child inherited the parent's turns instead of being briefed |
| Follow-up text | `SendMessage.input.message` and its `summary`, both plain | **none**: `send_message` arguments are Fernet tokens exactly like `spawn_agent`'s. Only the send, its clock and its `target` path are readable |
| Identity | `description`, `subagent_type` / `agentType`, `resolvedModel` (e.g. `claude-opus-5[1m]`) | child `session_meta`: `agent_nickname`, `agent_role`, `agent_path`, `source.subagent.thread_spawn.depth`; spawn arguments `agent_type`, `task_name` |
| Exchanges | `TaskOutput.input.task_id` names the agent id (joins 18/18); `SendMessage.input.to` joins 20/30 and carries `summary`, a ready one-line label | `SubAgentActivity` items on the parent: `started` / `interacted` / `interrupted` / `completed`, each with a clock and `agent_thread_id` |
| Report without the child | `TaskOutput` result `task.result` / `task.output` (plus `task.status`, `task.description`, `task.prompt`) | the child tail's last final `AgentMessage` (already read) |
| Work profile | sync result `toolStats`: `readCount`, `bashCount`, `editFileCount`, `linesAdded`, `linesRemoved`, `otherToolCount` — read today and dropped | — |
| Tokens | — (see below) | child tail's last `token_count.total_token_usage` — already inside the tail window |
| Fan-out | `Agent` calls whose spans overlap (never two in one message) | spawns under one `turn_id`, and overlapping spans |

Three joins, not one. `TaskOutput` is exact. `SendMessage` misses a third of
the time — those `to` values name no agent this parent launched, so an
unjoinable follow-up is simply left where it is, as its own tool row.
`TaskStop` is **not** usable: 9 of its 16 `task_id` values are from another
namespace entirely (`unrelated-task-id` against an `agent-id`).
A stop therefore becomes an exchange only when its id joins, and the row
never infers one from a `TaskStop` it cannot attribute.

### Layer 2 — a full child parse; **not proposed**

Claude async token and call counts (sum the child's `usage`, count its
`tool_use` blocks) and Codex tool-call counts (count the child's
`function_call` items). Both break the cost promise. The row prints the
honest absence instead.

### One decision this forces: what "tokens" means

The shipped row prints `result.totalTokens` for a synchronous Claude agent.
That number is the **final request's context** (measured: 146,491, of which
138,856 is cache read), not the work the child did — while the same page's
session-level token count is a sum across the session. Two different things
under one word.

Proposal: **drop `result.totalTokens`** and define an agent's tokens exactly
as the page already defines a session's, per provider. Codex then gets it
free from the tail; Claude's becomes layer 2 and is therefore not printed,
with the reason said in words. Carry it as a `SessionCount`
(`exact` / `inferred` / `unsupported` + `explanation`) — the app's existing
idiom for "this number is not what you think" — rather than a bare integer.

## The directions (pick one)

`subagent-exchange-mockup.html`, Claude and Codex samples in each, degraded
states drawn as first-class rows. Direction A of the shipped row (the head
band) is not re-litigated; this is about the structure under it.

- **A · Exchange list.** The open row keeps the head band; under it, one
  chronological list replaces the brief/report pair: `→ brief`,
  `→ follow-up` (its summary as the line, its text bounded), `← report`,
  `⨯ interrupted`, `· still running`. The collapsed row gains an exchange
  count. Smallest change, fixes gap 1, nothing moves in the timeline.
- **B · Agent lane.** The delegation opens a lane on the rail: its
  exchanges appear as small fork-tinted nodes at their own clocks, in
  place among the parent's own events, and a closing node ends it. The
  back-and-forth is chronologically true and overlap is visible, at the
  cost of interleaving child events into the parent's timeline model.
- **C · Roster card.** Delegations whose spans overlap collapse into one
  card: a head (`3 agents · 07:02 → 07:19`), one line per agent with a
  span bar drawn to scale, each line opening into A's pane. Fixes gap 3 on
  both providers — it is the only thing that makes a 15-agent Claude burst
  readable — and needs A underneath it for gap 1.

Recommendation: **A now, C after** — A is the one that pays for both
providers with records already in hand, and C is the burst case that A
alone leaves flat. B is the honest ideal and the biggest change to the
timeline model; worth having as the thing to argue against.

## Degraded states the mock draws

They are the common rows, not edge cases: async Claude with no counts and
no report yet; `running`; `no transcript`; a refused spawn; and a Codex row
with no brief, printing what it inherited (`fork_turns: all`) where the
brief would be.

## Contracts touched

- `SessionAgentPreview`: `exchanges: SessionAgentExchange[]`
  (`kind: brief | follow_up | report | stopped | interrupted | completed`,
  `at`, `label`, `text`, `truncated`); `agent_type`, `task_name`,
  `inherited_turns`, `depth`; `tool_profile`; `tokens` becomes a
  `SessionCount`.
- `SessionAgentIndexEntry`: the parent-recorded activity list (Codex) and
  the follow-up call ids keyed by agent id (Claude). Parser versions bump
  (Claude 1 → 2, Codex 6 → 7).
- Frontend: `AgentRow` in `src/components/project-sync/session-detail/SessionDetailEvent.tsx`;
  a new CSS block beside `.session-detail-agent-*`; the SHOW toggles and the
  header's `N agents` chip are unchanged.
- The follow-up tool rows stay where they are in the timeline (they are the
  parent's own calls); the agent row quotes them.

## Cost

No new *files* are opened, but the row's parent reads go from 1 to 1 + N:
each exchange's text lives in its own record of the parent transcript, so a
four-exchange agent reads four spans of a file the page already has open
(the same seek-and-read the tool rows do, ~0.1 ms each). The child head and
tail are unchanged, and Codex tokens come from a record already inside the
tail window. The index grows by roughly one small entry per interaction
(Codex averages 2.6 per agent).

Only the open row pays it: a collapsed row needs the count, which the index
carries.

## Not done

Nested children still render and do not open — and the corpus now shows
depth-2 is real: a spawned Codex child was seen calling `spawn_agent`,
`send_message` and `list_agents` of its own. Claude async token and call
counts stay unread. `outputFile` on an async Claude result points outside
the provider home (`/private/tmp/…`), so it is not a portable source and is
not used. `TaskStop` rows stay ordinary tool rows unless their id happens to
join, because more than half of them name another namespace.

# Session detail: agent rows

Date: 2026-09-09. Status: implemented (uncommitted). Clean cutover: no
migration, no legacy path; local state and cloud schemas stay at 1. The
timeline parser versions moved (Claude 1 → 2, Codex 4 → 5), so cached
timeline indexes rebuild on first open.

## What ships

A delegation to a subagent is its own timeline kind, `agent_call`, shown
by default beside prompts and replies (tool calls stay behind their
toggle). The row follows direction A of `mockups/session-agents-mockup.html`:

- A 24px fork tile in the node column, in the tool column's `other`
  purple; the tile takes the danger colour for a refused or failed run and
  amber for a run cut short or a transcript that cannot be read.
- The row reads `AGENT` · label · role pill · (`async` pill) · length,
  and prints a status word only when the run is not simply done:
  `running`, `failed`, `interrupted`, `incomplete`, `no transcript`.
- Opening the row shows fact chips (role, model, mode, span, and the
  child's own call and token counts when the parent recorded them), the
  brief and the report as bounded payload boxes, and one verb, **Open
  transcript**, which opens the child as its own session page.
- The header gains an `N agents` chip; SHOW gains an **Agents** toggle.
- A child page carries the parent as the crumb before its title (the way
  back), states its role, model and mode as chips, offers no launch verbs,
  and correlates no Git checkpoints.

## Where each fact comes from

| Fact | Claude | Codex |
| --- | --- | --- |
| Row identity | `Task`/`Agent` tool_use; `toolUseResult.agentId` on the result | `SubAgentActivity started` item (canonical) or `spawn_agent` call + `sub_agent_activity` (legacy) |
| Label, role, model, brief | tool_use input `description`, `subagent_type`, `prompt`; result `resolvedModel`/`agentType` | child `session_meta` `agent_nickname`, `agent_role`, `agent_path` (task slug); no brief (the spawn message is not readable from the rollout) |
| End, report, status | waited: result record (`totalDurationMs`, `totalToolUseCount`, `totalTokens`, result text); background: child tail (last clock, last final assistant text) | parent activity `interrupted`/`completed` (clock); child tail (last final `AgentMessage`) |
| Running vs incomplete | child file (or parent, when no result yet) written within 5 min | same |

The index (cached, stat-pinned to the parent) keeps metadata only: the
agent id, a provider-home-relative pointer to a Codex child rollout, and
the lifecycle the parent itself recorded. Labels, briefs and reports are
read from the records at page time like every other text.

## Cost

Per page: one parent record read per row (already the case for tool
calls), plus a 256 KiB tail read of the child (about 0.3 ms) and, for
Codex, a first-line head read (0.06 ms). Codex pays one sessions-tree
walk per index build (31 ms on 2,151 rollouts), cached with the index.
Nothing parses a child in full; call and token counts appear only when
the parent recorded them (synchronous Claude results).

## Addressing a child

`<parent resource id>/agent/<agent id>`, one segment per hop. The parent
is resolved through the same ownership checks as today; each hop is then
proven by the provider (`locate_child_transcript`): Claude derives the
path beside the parent and requires a regular, unlinked file; Codex finds
the rollout by id and requires its `session_meta` to name the parent.
Views for ownership are the parent's (`root_conversation_resource_id`).

## Contracts touched

- `SessionTimelineEventKind::AgentCall`, `SessionEventCounts.agent_calls`
- `SessionTimelineIndexEntry.agent: SessionAgentIndexEntry`
- `SessionTimelineEvent.agent: SessionAgentPreview`
- `ProjectSessionDetail.agent: SessionDetailAgent { parent_resource_id, parent_title, preview }`
- `SessionTimelineContext { provider_home, child }`; `read_timeline_page` takes the context
- Adapter: `summarize_child_file`, `locate_child_transcript`, `describe_agent_call`

## Not done

Nested children (a child's own agent rows) render but do not open: the
Claude corpus has none, and Codex depth-2 rollouts would need a chain of
hops the page does not yet build. Refused Codex spawns show only in
legacy-family rollouts; the canonical item family records no call for
them.

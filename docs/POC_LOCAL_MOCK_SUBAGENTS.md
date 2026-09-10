# Local mock API subagent POC

Test date: 2026-09-06.

Both installed CLIs completed a real subagent run against a local mock API.
Claude worked with stock `aimock`. Codex required a mounted Responses handler
that preserves namespaces and agent messages.

| Case | Result | Model requests | Child requests | Transcript files |
| --- | --- | ---: | ---: | ---: |
| Claude with `--bare` | Agent tool absent; expected control result | 1 | 0 | 1 |
| Claude with Agent enabled and stock `aimock` | One child completed; parent received the result | 3 | 1 | 2 |
| Codex with stock `aimock` | Call rejected; expected control result | 2 | 0 | 1 |
| Codex with the native Responses handler | One child completed; parent received the result | 4 | 1 | 2 |

Each Claude case also made one `HEAD /api/hello` probe. The four cases made
12 local HTTP requests: 10 model requests and two probes.

Run from the repository root:

```sh
node scripts/run-local-subagent-poc.mjs
```

The runner requires the tested versions: Codex CLI `0.152.0`, Claude Code
`2.1.260`, and `@copilotkit/aimock` `1.39.0`. The run used Node.js `20.19.4`.
The mock model labels are `gpt-5.4` and `claude-sonnet-4-6`. All responses are
fixed fixtures. No remote model or paid API key is used.

Claude requires a run without `--bare`. With that flag, the CLI did not expose
`Agent`, even when `--tools=Agent` and a custom agent definition were supplied.
The successful case uses an isolated home, empty settings sources, disabled
hooks, an empty MCP configuration, and an Agent-only tool list. The child has
no tools. Its output must appear in the parent's actual tool result before the
mock sends the final parent response.

Codex requires `features.multi_agent_v2=true` for this POC. A native call has
separate fields:

```json
{
  "type": "function_call",
  "namespace": "collaboration",
  "name": "spawn_agent"
}
```

The stock `aimock` Responses serializer omits `namespace`. Codex returned
`unsupported call: spawn_agent` and created no child. A dotted function name
also failed in the initial probe. The installed library also omits
`agent_message` input records from its normalized request. The successful
case uses `LLMock.mount` to serve a small Responses endpoint that retains
these native fields. It exercises spawn, child completion, wait, and result
delivery. The mounted endpoint is POC code; the existing test servers are not
changed.

The Codex POC uses the `gpt-5.4` label because the installed CLI exposes the
required tool schema with it. The initial `gpt-5.6-luna` probe omitted the API
`tools` field. A forced native call still ran, but that probe does not prove
tool discovery for that model label.

Both positive cases verify two native transcript files and the parent-child
link. Codex records the child's `parent_thread_id` and a parent
`agent_message` result. Claude saves a child transcript under the parent's
`subagents` directory, with `isSidechain: true` and an `agentId` that matches
the parent's tool result.

The runner uses fresh temporary process homes, provider homes, and projects.
It strips inherited credentials and endpoints before importing `aimock`.
Claude receives a fixed local placeholder key. The servers bind to
`127.0.0.1`. The output contains only version strings, counts, and result
flags. The runner removes its temporary state after the run.

Verification: all four POC checks passed. The existing mock server suite also
passed: `npm run test:aimock-servers`.

This proves local CLI orchestration, API transport, result delivery, and
transcript creation. It does not test Mallard capture, Push/Pull, or UI
rendering of these subagent records.

# Plan: Additional Deterministic Codex Integration Test with aimock

Status: implemented and verified. The complete ignored keyless gate passed on
August 19, 2026 with `1 passed, 0 failed, 7 filtered` in 90.48 seconds. The
aimock, native paginated app-server, app-server fork, public CLI fork, and
Mallard native-graph conformance POCs are complete.

Date: 2026-08-19 (America/Los_Angeles)

## 1. Outcome

The implementation adds a second live integration target,
`src-tauri/tests/live_codex_aimock_e2e.rs`, driven by one local, deterministic
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server owned by that
test. The authenticated custom-provider target remains independently available
as `src-tauri/tests/live_codex_e2e.rs`.

The additional test launches the real Codex executable, writes and resumes real
Codex rollouts, exercises Mallard's production Push/Pull paths, and covers the
same two-machine synchronization topology. It uses public
headless `codex exec` for source turns, later graph mutations, and final
reopening, plus `codex app-server --stdio` for native paginated thread creation
and forks. Response content comes from a test-generated, bounded scenario table
instead of a model.

This is an additive, isolated test transport:

- no real OpenAI, Anthropic, or third-party provider request;
- no GPU, local model, provider account, or paid token usage;
- the aimock target does not read `.env.local`, `auth.json`, a source profile,
  or a real API key; the separate API-backed target intentionally retains its
  bounded repository-root `.env.local` input;
- aimock is not a provider fallback, product compatibility branch, or legacy
  Mallard execution path;
- no directly written synthetic rollout in the live scenario;
- no product migration;
- both live targets exercise the same current machine-local and portable
  Mallard schema 1; and
- no product DTO, manifest, storage-protocol, or schema change.

The live POC exposed three bugs in existing schema-1 execution paths. The
implementation corrects those paths directly: Pull decodes a Codex portable
display title before writing the target session index; the Codex history
resolver validates native paginated global record ordinals rather than treating
an ordinal cutoff as a physical line count; and every Codex standalone cloud
history is materialized copy-on-write because an external fork can begin
depending on the old alias after review. The displaced source remains hidden
only while an immutable whole-file witness matches, so later local work fails
open as a retained branch instead of disappearing. These are current-path
fixes, not migrations, fallbacks, or compatibility layers.

The aimock test remains ignored and explicitly gated because it is a long-running
installed-Codex integration test, not because it is paid or authenticated.

## 2. Why aimock

The implementation pins `@copilotkit/aimock` at version `1.39.0` as a
development dependency.

The selected version is MIT licensed, requires Node `>=20.15.0`, and supports
the OpenAI Responses API used by Codex. It can return fixed text, structured
JSON, ordinary function calls, tool-result continuations, streaming events,
and fixed errors. Strict mode returns HTTP 503 when no fixture matches.

Relevant upstream references:

- [aimock repository](https://github.com/CopilotKit/aimock)
- [aimock Responses API](https://aimock.copilotkit.dev/responses-api/)
- [aimock fixtures](https://aimock.copilotkit.dev/fixtures/)
- [aimock strict replay](https://aimock.copilotkit.dev/record-replay/)
- [Codex CLI command reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
- [OpenAI Responses API reference](https://developers.openai.com/api/reference/java/resources/beta/subresources/responses)
- [Codex's own local Responses test harness](https://github.com/openai/codex/blob/main/sdk/python/tests/app_server_harness.py)

The official OpenAI API distinguishes ordinary function calls from custom
tool calls. aimock 1.39.0 supports ordinary Responses `function_call` items,
but it does not currently preserve arbitrary `custom_tool_call` items. The
plan accounts for this explicitly in the file-writing scenario.

## 3. POC environment

The POC was isolated under a disposable `/private/tmp` directory. It did not
read or modify the repository, `~/.codex`, `~/.mallard`, or any provider
credential.

Observed versions:

| Component | Version |
| --- | --- |
| Node | 20.19.4 |
| npm | 10.8.2 |
| Codex CLI | 0.147.0 |
| `@copilotkit/aimock` | 1.39.0 |

The package was installed only in the disposable POC directory. npm reported
zero audited vulnerabilities for that two-package POC tree.

## 4. POC results

### 4.1 Ephemeral loopback startup

aimock accepted port `0` and reported the OS-assigned loopback endpoint:

```text
[aimock] aimock server listening on http://127.0.0.1:<assigned-port>
```

The server used strict fixture replay with no record mode, proxy, or upstream.

### 4.2 Fixed text through the real Codex CLI

The fixture table contained:

```json
{
  "match": {
    "userMessage": "MALLARD_AIMOCK_POC_R1"
  },
  "response": {
    "content": "MALLARD_AIMOCK_RESPONSE_R1"
  }
}
```

Codex 0.147 completed the turn with exactly:

```json
{
  "type": "item.completed",
  "item": {
    "type": "agent_message",
    "text": "MALLARD_AIMOCK_RESPONSE_R1"
  }
}
```

The rollout was written under the isolated POC `CODEX_HOME`.

### 4.3 Resume includes the old history

The POC resumed the first thread with a second unique marker. aimock's
normalized request showed this ordered marker subsequence:

```text
user       MALLARD_AIMOCK_POC_R1
assistant  MALLARD_AIMOCK_RESPONSE_R1
user       MALLARD_AIMOCK_POC_R2
```

This is the key result for deterministic history verification. Codex loaded
the prior rollout and supplied the old user and assistant messages in the new
`/v1/responses` request. A model is not needed to interpret or repeat them;
the mock can compare the exact ordered markers before returning success.

### 4.4 Structured output

The fixture returned this object:

```json
{
  "status": "MALLARD_SCHEMA_OK"
}
```

`codex exec --output-schema ...` completed successfully and emitted exactly:

```json
{"status":"MALLARD_SCHEMA_OK"}
```

aimock does not generate data from the schema. The fixture must already hold a
schema-valid answer, which is the desired deterministic behavior.

### 4.5 Strict unmatched failure

An unconfigured marker returned:

```json
{
  "status": 503,
  "contentType": "application/json"
}
```

The aimock log reported a strict no-match. No fallback provider was configured.

### 4.6 Ordinary function-tool round

The POC configured two rows for the same prompt:

```json
[
  {
    "match": {
      "userMessage": "MALLARD_AIMOCK_TOOL_POC",
      "hasToolResult": false
    },
    "response": {
      "toolCalls": [
        {
          "id": "call_mallard_poc_1",
          "name": "exec_command",
          "arguments": {
            "cmd": "pwd"
          }
        }
      ]
    }
  },
  {
    "match": {
      "userMessage": "MALLARD_AIMOCK_TOOL_POC",
      "hasToolResult": true
    },
    "response": {
      "content": "MALLARD_AIMOCK_TOOL_OK"
    }
  }
]
```

The real Codex CLI executed the function tool, sent a second Responses request
containing a tool-result message, matched the second row, and completed with
`MALLARD_AIMOCK_TOOL_OK`.

Both `hasToolResult: false` and `hasToolResult: true` are required. Without the
first gate, the broad initial rule could match again after the tool result.

### 4.7 Real workspace patch

The POC then returned an ordinary `exec_command` function call whose bounded
command invoked `apply_patch` inside the disposable project. With
`workspace-write`, Codex:

- created the requested file;
- emitted a completed `file_change` item;
- wrote a successful durable `patch_apply_end` rollout record;
- sent a `function_call_output` back to aimock; and
- completed with the configured fixed assistant marker.

The created file had mode `0644` and exact bytes:

```text
MALLARD_AIMOCK_FILE_BYTES
```

The successful rollout used an ordinary Responses `function_call` named
`exec_command`, not a native Responses `custom_tool_call`. This distinction
must remain explicit in the test documentation and assertions.

The first write attempt was run inside the development agent's outer sandbox
and was blocked by nested sandboxing. The identical isolated POC outside that
outer sandbox succeeded. A normal user invocation of the live test does not
have that extra enclosing sandbox.

### 4.8 Credential absence

The initial aimock turn POC used an inert local value only. The isolated Codex
home did not contain `auth.json`. Separately, the loopback Responses POC
developed in `live_codex_e2e.rs` proves that Codex 0.147 accepts a custom
provider with `requires_openai_auth=false`, no `env_key`, no `auth.json`, and
no `Authorization` header. The additional aimock target therefore needs no
dummy key.

### 4.9 Important aimock journal bound

aimock bounds serialized journal bodies at 64 KiB. A current Codex request was
already approximately 46 KiB before adding a complex history because Codex
includes system and tool instructions.

Therefore, the final test must not rely on the post-request journal body as the
only history oracle. The mock must validate ordered history inside a
programmatic response factory before journal truncation occurs. The journal is
still suitable for safe request counts, paths, fixture identities, and status
summaries.

### 4.10 Native paginated creation and unauthenticated fork

A second disposable POC exercised both public Codex fork surfaces against
aimock without reading or creating `auth.json`.

The headless app-server path performed:

1. `initialize` with the experimental API capability;
2. `thread/start` with `historyMode: "paginated"`;
3. a deterministic turn against aimock; and
4. `thread/fork` with `excludeTurns: true`.

The parent rollout was native paginated history with `history_base: null`. The
child received a new thread ID, `history_mode: "paginated"`,
`forked_from_id` equal to the parent, and a non-null `history_base` containing
the parent ID plus exact ordinal and byte cutoffs. The fork itself made no
Responses request.

The public top-level `codex fork <thread-id>` command was then run without a
prompt in an isolated PTY, with the POC provider variable unset and all real
provider credentials removed. It displayed the inherited history, created
another native paginated child, and did not change the aimock request count.
This proves that fork is a local history operation and does not require
authenticated Codex when the active provider declares
`requires_openai_auth=false`.

Two negative results define the implementation boundary:

- forking a legacy source preserves legacy mode; it does not convert the
  source into paginated history; and
- `codex exec -c 'history_mode="paginated"'` still created a legacy source in
  Codex 0.147.

The live test therefore uses the deterministic headless app-server
`thread/start(historyMode="paginated")` and `thread/fork` APIs. It does not
automate the TUI-only top-level `codex fork` command.

### 4.11 Native graph conformance gate and current-path correction

The implementation creates the complete five-node native graph in a disposable
home and passes it through Mallard's rollout resolver, dependency seeder,
standalone builder, timeline projection, ancestry, and fixed-point validators.
It also proves that an inherited-only child is flushed and discoverable after
app-server shutdown, that a materialized standalone history reopens through
public `codex exec resume`, and that later non-ephemeral resumes keep the same
provider thread ID and `history_base` while appending only to the local segment.

That POC found an important Codex 0.147 protocol detail: a native
`history_base.end_ordinal_exclusive` is a boundary in the rollout's top-level,
global ordinal space. It is not the number of physical JSONL records in the
referenced slice. A nested parent can therefore have thirteen physical records
while its declared end ordinal is twenty-seven. A child's own first physical
record, its `session_meta`, uses exactly its history-base end ordinal; each
following row increments contiguously. The production resolver now requires
every current paginated record to carry that exact sequence, validates each
declared global endpoint independently from the byte cutoff, seeds the leaf
cursor with the next global ordinal, and rejects missing or gapped appended
ordinals. If an incremental tail's first complete appended record has no
ordinal, Mallard requests one authoritative full resolve so session metadata
selects Codex's current `legacy` or `paginated` mode; after a paginated ordinal
sequence starts, a missing or wrong ordinal is malformed. The byte cutoff still
must land exactly on a complete JSONL record.

The live target uses the final keyless provider overrides and requires every
request to omit `Authorization`. This is a zero-cost native-format conformance
gate between Codex 0.147 and Mallard's current schema-1 parser. The resolver
change corrects the only supported execution path; it adds no migration,
fallback, or path that accepts ordinal-less paginated data. Codex's upstream
`history_mode: "legacy"` remains a separate current, ordinal-less source mode;
it is not a Mallard legacy product path.

### 4.12 Codex copy-on-write materialization

Revision 10 exercises a subtle physical dependency boundary. A native
paginated child stores a direct `history_base` reference to its parent's
provider alias plus exact byte and global-ordinal cutoffs. Replacing that
parent file in place with a standalone cloud history can therefore corrupt the
child even when the replacement represents the same logical conversation.

The current planner resolves this without a compatibility representation or a
schema change. It cannot safely choose replacement from a pre-apply dependency
scan: Codex may create a new child after that scan without changing the parent
bytes, binding, or index. Therefore:

1. Every Codex `MaterializeCloud` Pull mints a fresh provider alias and plans
   an `Absent` destination. Profile-wide provider IDs collected by the bounded
   discovery scan prevent allocation collisions.
2. The old physical rollout remains byte-for-byte unchanged whether or not it
   was referenced at review time.
3. Apply writes the standalone rollout at the fresh alias and adds its session
   index entry with no-clobber behavior. It does not rewrite the referenced
   predecessor or its index row.
4. Current bindings plus already-applied materialization receipts form the
   alias ownership ledger used to derive a domain-separated retained-branch
   identity. They never suppress content on their own. Apply atomically saves
   an immutable whole-file source checkpoint under that retained identity;
   review and Activity hide the alias only while a bounded no-follow read
   exactly matches that checkpoint. Missing, unreadable, partial, or changed
   evidence fails open and exposes the retained branch. A direct current
   binding always wins.
5. Providers without Codex's physical rollout graph retain their existing
   SHA-pinned in-place materialization behavior.

The revision-10 gate snapshots the referenced ancestor before Pull, verifies
that its bytes are unchanged afterward, verifies that the pulled standalone
history uses a distinct fresh alias, resolves every descendant again through
the production history graph, and checks that the retained physical ancestor
does not surface as a duplicate conversation while unchanged. A focused
backend regression appends to the retained alias, proves it remains visible
across repeated comparisons plus Activity and Session Detail, publishes it,
then copy-on-writes that retained branch and proves the same native alias
surfaces under a distinct R2 identity after another append. Revision 7
performs the same identity/byte assertions for unreferenced P2, proving the
rule is unconditional for Codex rather than a racy pre-scan decision. This is
the only schema-1 execution path; it adds no migration, compatibility branch,
fallback, or alternate legacy behavior.

## 5. Selected architecture

The Rust integration test owns the mock process. This satisfies the
requirement that the local mock server starts at the beginning of the test and
ensures cleanup on success or panic.

### 5.1 Pinned dependency

The implementation pins this exact development dependency in `package.json`
and `package-lock.json`:

```json
{
  "devDependencies": {
    "@copilotkit/aimock": "1.39.0"
  }
}
```

Do not use a floating version.

### 5.2 Small Node harness

The implementation adds `scripts/live-codex-aimock-server.mjs`.

The script:

1. import the programmatic `LLMock` API from the pinned package;
2. read one bounded scenario document created by the Rust test;
3. reject unknown fields and malformed scenarios;
4. bind only `127.0.0.1` with port `0`;
5. enable strict mode;
6. configure no recorder, proxy, remote fixture, or upstream;
7. retain a bounded request journal;
8. register response factories that compare the complete ordered marker
   history before returning a fixed answer;
9. print one bounded readiness line containing only the selected loopback port;
10. accept a bounded shutdown message on stdin;
11. write one bounded, path-free audit result to the test-owned temporary
    directory; and
12. stop aimock and exit.

The audit result contains only scenario IDs, request counts, match status,
an aggregate no-authorization result, closed HTTP classifications, bounded
input/history counts, verifier-marker position, and numeric mismatch
information. For every scenario it preserves the current safe assertions:
POST, exact `/v1/responses` path, no query, no content encoding, JSON content
type, no authorization, exact prior-history item count, larger total input
count, and the expected current-marker position. It must not contain raw
prompts, system instructions, absolute paths, headers, or provider output.
The private scenario document itself is allowed to contain the reviewed R1
work directory and exact patch targets. It remains bounded and confined to the
test-owned temporary root; only the path-free audit crosses that boundary.

Starting aimock from either Node runner was considered and rejected. It would
require an asynchronous Cargo child and would make direct ownership less
obvious. A Rust-owned child plus one small Node harness keeps the lifecycle
local to the additional integration target.

Starting the legacy-named `llmock` binary directly was also considered. It is
adequate for simple JSON fixtures and the POC proved that port `0` works, but a
programmatic response factory is required to validate long histories before
the 64 KiB journal cap. Use the current `@copilotkit/aimock` API directly.

### 5.3 Rust mock-process guard

The additional Rust target contains a bounded `AimockProcess` helper inside
`live_codex_aimock_e2e.rs`.

Its responsibilities:

- locate the repository-root Node harness and pinned package without searching
  the user's home;
- keep the runner's prerequisite check metadata-only so third-party aimock code
  is first imported inside the sanitized server child;
- write the bounded scenario document inside the live test's temporary root;
- spawn `node` with a sanitized environment;
- remove all real provider credential variables, every documented
  `AIMOCK_PROVIDER_*` upstream key, and the aimock dummy-key marker;
- remove inherited proxy variables and allow only literal loopback through
  `NO_PROXY`/`no_proxy`;
- pipe stdin, stdout, and stderr;
- require a readiness line within a short timeout;
- validate that the selected endpoint is exactly
  `http://127.0.0.1:<nonzero-port>/v1`;
- reject userinfo, query, fragment, alternate hostnames, and alternate paths;
- drain output concurrently within fixed byte limits;
- request graceful shutdown over stdin;
- wait for bounded completion and force-kill only as cleanup fallback;
- read and validate the safe audit result after successful shutdown; and
- kill and reap the process from `Drop` if the test unwinds.

No mock-process output is copied into a panic message. Diagnostics remain
closed and numeric.

### 5.4 No provider credential

Do not replace the real credential with a dummy credential. aimock does not
require inbound authentication, and Codex 0.147 accepts this custom provider
without `env_key` when `requires_openai_auth=false`.

The aimock target removes inherited variables such as:

- `OPENAI_API_KEY`;
- `CODEX_API_KEY`;
- `AZURE_OPENAI_API_KEY`; and
- the old Mallard live-provider variables.

No provider key environment variable is set for Node or Codex. Neither home
contains `auth.json` or a supplied `config.toml`. Codex 0.147 can generate one
exact project-trust entry during the R1 workspace-write turn; the test accepts
only the expected no-follow regular file with mode `0600` and bounded exact
bytes, then removes it so the isolated home again has no `config.toml`. That is
cleanup of state generated by the current Codex run, not a migration or a
compatibility branch. The aimock runner and target never open or validate
repository-root `.env.local`; whether the separate API-backed test's ignored
connection document is present has no effect on this target. The bounded
aimock audit must report that every request omitted the `Authorization` header;
it must never retain or print raw headers.

### 5.5 Codex provider configuration

Both `myconf1` and `myconf2` use the same loopback-only Responses provider:

```toml
model_provider = "mallard_aimock"

[model_providers.mallard_aimock]
name = "Mallard deterministic local aimock"
base_url = "http://127.0.0.1:<assigned-port>/v1"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
stream_idle_timeout_ms = 60000

[features]
plugins = false
enable_request_compression = false
```

For `codex exec`, keep `--ignore-user-config` and `--ignore-rules` and supply
provider configuration through secret-free `--config` arguments. For
app-server, use the same overrides with an isolated empty `CODEX_HOME` and
process home; app-server has no `--ignore-user-config` flag. No provider
configuration file or `auth.json` is seeded, and the exact generated R1 trust
file described above is removed before final-state assertions.

The model slug may remain `gpt-5.6-luna` to preserve the current Codex model
profile. No Luna model is invoked; aimock returns every response locally.

### 5.6 Bounded Codex app-server guard

The test-local `CodexAppServer` RAII helper inside
`live_codex_aimock_e2e.rs` launches `codex app-server --stdio` only for native
paginated graph creation. The helper must:

- use the intended isolated Codex home, process home, and project CWD;
- remove every real provider credential variable;
- remove inherited proxy variables and pin `NO_PROXY`/`no_proxy` to
  `127.0.0.1`;
- pass the loopback provider and `features.plugins=false` through secret-free
  `--config` arguments;
- perform `initialize` with `experimentalApi: true`, then `initialized`;
- correlate every response by a monotonic request ID;
- provide bounded `thread_start_paginated`, `thread_fork_excluding_turns`, and
  `turn_start_and_wait` methods;
- send `excludeTurns: true` on every fork, exactly matching the successful
  POC, and validate the returned source identity;
- wait for the matching thread/turn completion and fail closed on a failed
  turn, wrong identity, malformed message, early EOF, or timeout;
- bound each JSONL line, aggregate stdout, stderr, and message count;
- never print RPC bodies, prompts, paths, provider output, or stderr;
- close stdin and reap the child after graph creation; and
- kill and reap from `Drop` only as a bounded cleanup fallback.

Use one app-server process to create P0-P4 on replica A, then stop it before
Mallard captures or replaces any rollout. Later authoritative mutations use
one-shot public `codex exec resume` processes. This deliberately reuses the
current proven disk-rehydration path and avoids keeping an app-server thread
cache alive across Mallard Push/Pull materialization.

## 6. Scenario-table contract

The Rust test generates one bounded scenario document. Every scenario has a
unique ASCII identifier. Prompt markers are also unique except for the two
intentional R1 legs: the initial tool-call request and its tool-result
continuation share `LIVE_USER_R1`, the same expected history, and an exact call
ID.

The serialized contract is:

```json
{
  "version": 1,
  "model": "gpt-5.6-luna",
  "expected_request_count": 38,
  "scenarios": [
    {
      "id": "source_r1_tool",
      "prompt_marker": "LIVE_USER_R1",
      "expected_history": [],
      "require_tool_result": false,
      "expected_tool_call_id": null,
      "response": {
        "kind": "tool_call",
        "name": "exec_command",
        "call_id": "call_mallard_r1",
        "arguments": {
          "cmd": "<bounded apply_patch command>",
          "workdir": "<temporary project1>"
        }
      }
    },
    {
      "id": "source_r1_final",
      "prompt_marker": "LIVE_USER_R1",
      "expected_history": [],
      "require_tool_result": true,
      "expected_tool_call_id": "call_mallard_r1",
      "response": {
        "kind": "text",
        "content": "MALLARD_BASE"
      }
    },
    {
      "id": "verify_1_9",
      "prompt_marker": "MALLARD_USER_VERIFY_1_9",
      "expected_history": [
        {"role": "user", "marker": "GRAPH_USER_BASE_1"},
        {"role": "assistant", "marker": "GRAPH_BASE_1"},
        {"role": "user", "marker": "GRAPH_USER_LEFT_PARENT"},
        {"role": "assistant", "marker": "GRAPH_LEFT_PARENT"},
        {"role": "user", "marker": "GRAPH_USER_B_REVERSE"},
        {"role": "assistant", "marker": "GRAPH_B_REVERSE"}
      ],
      "require_tool_result": false,
      "expected_tool_call_id": null,
      "response": {
        "kind": "text",
        "content": "MALLARD_HEADLESS_REOPEN_CAPTURED"
      }
    }
  ]
}
```

`HistoryMarker` entries are objects with exactly `role` and `marker`; `role`
is `user` or `assistant`. Every scenario has all six fields shown above.
`expected_tool_call_id` is a bounded call ID only when
`require_tool_result=true`; otherwise it is JSON `null`. A response is exactly
one of `{"kind":"text","content":...}` or the bounded
`{"kind":"tool_call",...}` shape shown above. The concrete document has
bounded counts, string lengths, command lengths, and total bytes. Unknown or
missing fields fail closed.

The table includes twelve graph-turn scenarios: five native app-server graph
turns and seven later mutation turns. It includes no scenario for app-server
initialization, `thread/start`, `thread/fork`, or a history-load operation
separate from its turn. Those operations must not contact aimock. Each of the
seven non-ephemeral CLI resumes has exactly one mutation-turn scenario and one
request. Any extra provider request is a strict miss and fails the final count
audit.

### 6.1 Marker extraction

The Node response factory receives aimock's normalized message array before
journal truncation. It:

1. ignore system messages and non-marker content;
2. consider only user and assistant messages;
3. require a marker at text position zero;
4. extract only the maximal leading ASCII `[A-Z0-9_]+` token;
5. use disjoint user and assistant prefixes;
6. preserve request order;
7. compare the complete resulting `(role, marker)` vector with the scenario's
   expected vector plus current verifier marker; and
8. return a fixed error response on any missing, extra, reordered, or
   role-changed marker.

No expected response may be returned after a history mismatch.

### 6.2 Fixed answers, generated envelopes

The scenario contract fixes the assistant marker or ordinary function call;
aimock generates the surrounding Responses/SSE envelope. Not every internal
SSE item ID or journal timestamp is fixed. Mallard assertions therefore
compare semantic markers, identities, lineage, content digests within the run,
and the explicit R1 call ID rather than whole provider envelopes across
independent runs.

## 7. Expanded live-test flow

The synchronization flow remains analogous to the current test.

### Phase 0: preflight and local mock

1. Require `MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1` as acknowledgement of a
   heavyweight installed-Codex test.
2. Verify Cargo, Node, the pinned aimock package, and Codex 0.147 before any
   turn-bearing Codex process starts. The isolated `codex --version` child is
   counted separately from conversation/app-server children.
3. Create the owned temporary root.
4. Generate the bounded aimock scenario document.
5. Start aimock on an ephemeral loopback port.
6. Verify readiness and strict/no-upstream configuration.

### Phase 1: first replica

1. Create local-folder storage `mystorage1`.
2. Create empty, secure Codex home `myconf1`; assert `auth.json` is absent.
3. Create `project1` and its outside sibling fixture directory.
4. Finish Mallard setup for `project1`, binding `myconf1` and `mystorage1`.
5. Run source turn R1 with workspace-write.
6. aimock returns one `exec_command` call containing a bounded `apply_patch`
   command that creates the required in-project and outside-project files.
7. Codex executes the patch and sends its tool result.
8. aimock validates the tool result and returns `MALLARD_BASE`.
9. Assert exact files, modes, `file_change`, durable `patch_apply_end`, and
   Session Detail path projection.
10. Push revision 1 to `mystorage1`.

### Phase 2: second replica

1. Create empty, secure Codex home `myconf2`; assert `auth.json` is absent.
2. Create `project1_copy`.
3. Copy only the explicitly selected project files from `project1` while
   preserving their internal relative paths.
4. Do not copy or remap the outside-project file.
5. Link the second project and Codex home to `mystorage1`.
6. Pull revision 1.
7. Assert in-project paths remap to `project1_copy` and outside paths retain
   their original absolute spelling.

### Phase 3: real Codex source turns and project revisions 2-5

Run the current symmetric five-turn conversation flow through fixed aimock
rows:

- R1: new thread on replica A, including the single tool round;
- R2: resume on replica B;
- R3: Keep Both on A;
- R4: advance the shared primary on A;
- R5: divergent B pull/Keep Both/push, followed by A pull.

Each source fixture validates the complete expected marker history before
returning its fixed assistant marker.

After R1, assert explicitly that this source thread remains
`history_mode: "legacy"` with `history_base: null`. Do not try to convert the
R1-R5 conversation family to paginated mode. This is Codex's native current
source-rollout mode, not a Mallard legacy or compatibility path.

Expected process and HTTP counts for this phase:

- five Codex processes;
- six `/v1/responses` requests, because R1 has one tool-call leg and one final
  leg; and
- zero retries.

### Phase 4: paginated graph and revisions 6-10

In the additional aimock scenario, use native Codex 0.147 app-server operations
instead of calling the direct JSONL writer. Keep the hand-built writer only as
an offline parser fixture; it is not an alternate live execution path.

Create the graph on replica A in this exact order:

1. `thread/start` with `historyMode="paginated"`, the loopback provider,
   `approvalPolicy="never"`, and read-only sandboxing creates dynamic P0.
2. Add base turn 1 to P0.
3. Fork P0 with `excludeTurns: true`, creating P1 at the base-1 cutoff.
4. Add the left-parent turn to P1 while P0 remains at that reviewed cutoff.
   Codex 0.147 hydrates a native fork lazily on its first turn, so this pins the
   persisted P1 base before its source advances.
5. Fork P1 twice with `excludeTurns: true` at the same durable boundary,
   creating P3 and inherited-only P4.
6. Add base turn 2 to P0.
7. Fork P0 again with `excludeTurns: true`, creating P2 at the complete-P0
   cutoff.
8. Add the right-parent turn to P2.
9. Add the left-leaf turn to P3, then stop the app-server and re-read the
   durable P1 boundary after its final bookkeeping flush.

The resulting histories match the current acceptance graph:

```text
P0: base 1, base 2
├── P1: base 1, left parent
│   ├── P3: base 1, left parent, left leaf
│   └── P4: base 1, left parent
└── P2: base 1, base 2, right parent
```

All five provider IDs are dynamic UUIDv7 values returned by Codex. Store them
in `PaginatedGraph.nodes` and derive Mallard logical IDs from those values.
Fixed P0-P4 constants may remain only in focused offline fixtures.

Capture the expected parent byte cutoff and top-level global ordinal endpoint
at each fork. Then stop and reap the app-server so every rollout, including
inherited-only P4, is durable and discoverable. After shutdown, assert:

- P0 is paginated, has `history_base: null`, and has no fork parent;
- every child is paginated and names its immediate source in both
  `forked_from_id` and `history_base.thread_id`;
- every child cutoff exactly matches the parent's byte boundary and global
  ordinal endpoint at fork time; the ordinal is never inferred from JSONL line
  count;
- P1 and P2 use different P0 cutoffs;
- P3 and P4 use the same P1 cutoff;
- all five IDs are distinct canonical UUIDv7 values;
- every rollout records `model_provider: "mallard_aimock"`;
- all five rollouts have the expected replica-A CWD; and
- Mallard's existing resolver, dependency seeder, standalone builder,
  timeline projection, ancestry, and fixed-point validators accept every
  native rollout.

Perform the seven later graph mutations through the existing one-shot public
`codex exec resume` path, each backed by one exact aimock history scenario:

- revision 7: A/P1 local-only, A/P3 divergent, B/P2 storage-side, and B/P3
  divergent;
- revision 9: A/P4 reverse, B/P4 reverse, and B/P0 local-root.

At revision 10, Pulling the cloud-side standalone P0 into replica A must not
overwrite the original P0 alias because P1 still references that physical
rollout. The reviewed `MaterializeCloud` action instead targets a fresh absent
alias, preserves the original P0 bytes and descendant cutoffs, appends the
fresh index entry without clobbering the ancestor entry, and suppresses the
displaced alias in review and Activity only while its immutable whole-file
witness still matches. Any subsequent append surfaces a stable retained
branch instead of disappearing. P2 is the unreferenced control: its
update must also use a fresh absent alias and leave the displaced P2 rollout
byte-for-byte unchanged.

This gives replica B the same fresh-process disk rehydration used by the final
reopen matrix after Mallard has materialized standalone rollouts. It avoids a
long-lived app-server cache crossing Push/Pull filesystem changes.

These seven mutation resumes are non-ephemeral and run against authoritative
`myconf1`/`myconf2`. Each must return the exact requested provider thread ID,
append to that existing rollout, preserve its `history_base`, and pass the
production resolver afterward. The twenty Phase 5 probes remain ephemeral and
operate only on disposable snapshots.

Preserve all current coverage for:

- standalone and inherited histories;
- dependency cutoffs;
- complex parent graphs;
- both-direction Push/Pull;
- stale reviewed plans;
- divergent Pull;
- Keep Both identities and provenance;
- revisions 6-10; and
- exact fixed-point behavior.

This phase uses one app-server process, seven `codex exec resume` processes,
twelve deterministic Codex turns, twelve `/v1/responses` requests, four local
forks, and zero retries. `thread/start` and all forks must make zero provider
requests.

### Phase 5: deterministic headless reopening

Retain twenty disposable headless Codex resumes: ten final aliases from
`myconf1` and ten from `myconf2`.

For each alias:

1. snapshot only the isolated Codex history/index state into a disposable home;
2. run public `codex exec --json resume --ephemeral` against aimock using the
   current plain-text verifier shape;
3. use a unique verifier scenario marker;
4. have the aimock response factory compare the complete ordered prior history
   before returning that scenario's fixed acknowledgement;
5. assert the exact acknowledgement and resumed provider thread ID;
6. assert the target rollout appended only inside the disposable snapshot; and
7. assert the authoritative `myconf1`/`myconf2` history digest inventories are
   unchanged.

Retain the current safety/isolation flags for every verifier process:

```text
codex exec --json
  --ask-for-approval never
  --ignore-user-config
  --ignore-rules
  --skip-git-repo-check
  --sandbox read-only
  --model gpt-5.6-luna
  --config <secret-free loopback overrides> ...
  --cd <replica project>
  resume --ephemeral <provider-thread-id> <unique verifier prompt>
```

This keeps the exact public headless-open proof while removing probabilistic
model interpretation.

Expected counts for this phase:

- twenty Codex processes;
- twenty `/v1/responses` requests; and
- zero retries.

The complete live flow therefore expects:

- thirty-two short-lived `codex exec` processes;
- one explicit bounded graph-building app-server process;
- thirty-three turn-bearing Codex child processes;
- one isolated `codex --version` preflight child;
- thirty-seven deterministic turns; and
- thirty-eight local Responses requests.

Mallard planning may also start a runtime-dependent number of bounded,
ephemeral, no-turn allocator app-servers. They disable plugins, use separate
disposable `CODEX_HOME`, `HOME`/`USERPROFILE`, and working directories, remove
provider/proxy credentials, and are always reaped. They are intentionally
excluded from the stable turn and Responses-request counts above.

There are no paid requests.

### Phase 6: skills and pets

Keep the existing storage-catalog skill and fake-pet matrix unchanged. It is
already deterministic and performs zero provider processes.

Assert that Library activity does not change the complete project repository
head, manifest, or object inventory.

### Phase 7: final audit and shutdown

1. Recheck all project and catalog fixed points.
2. Recheck exact project/local trees and modes.
3. Recheck portable privacy and path-remapping invariants.
4. Recheck that neither Codex home contains `auth.json`, `config.toml`, or
   provider authorization state, and scan both bounded process homes.
5. Ask the Node harness to stop gracefully.
6. Require a successful, bounded aimock audit:
   - exactly 38 Responses requests;
   - every expected scenario consumed exactly once, except explicitly paired
     tool legs;
   - no request carried an `Authorization` header;
   - every request used POST `/v1/responses`, no query, no content encoding,
     and JSON content type;
   - every scenario reported the exact prior-history count, a larger bounded
     total input count, and the expected current-marker position;
   - no unknown route;
   - no strict miss;
   - no history mismatch;
   - no retry; and
   - no unused fixture.
7. Reap the mock process.

## 8. False-positive protections

The implementation must include these protections:

- A fixed response is never selected using only a generic verifier prompt.
- Every history verifier has a unique scenario marker and exact expected
  ordered marker vector. The only repeated prompt marker is the intentional
  two-leg R1 tool round.
- History is validated inside the response factory before aimock's 64 KiB
  journal cap.
- An incorrect history returns a fixed error; it cannot receive `status: ok`.
- R1's initial and tool-result scenarios are separated by
  `require_tool_result` and `expected_tool_call_id`; the harness maps those to
  aimock's `hasToolResult` and exact `toolCallId` match fields.
- Request and stream retries remain zero.
- Unknown prompts fail with strict HTTP 503.
- The final audit checks both missing and extra requests.
- The combination of the exact 38-request total, exact per-scenario
  consumption, strict unmatched-request failure, no unknown route, and zero
  retries proves that root creation and all four native forks made no provider
  request.
- The mock has no record/proxy/upstream mode.
- All inherited real credential variables are removed from Node, Cargo, and
  Codex children.
- The test never reads the real user home.
- Every aimock request is rejected by the audit if it carries an
  `Authorization` header.
- App-server RPC output and stderr are independently bounded and never printed
  verbatim.
- Dynamic provider IDs and temporary paths never enter the safe audit file.
- Synthetic rollout builders are unreachable from the ignored live scenario.
- Mock stdout, stderr, request bodies, and audit failures are never printed
  verbatim.

## 9. Focused offline coverage

The additional Rust target includes seven non-ignored, zero-provider tests.
They cover the keyless loopback overrides and URL boundary, bounded
provider-state scanning and generated symlinks, R1 prompt/diagnostic safety,
the production timeline projection with paging, current nested file-edit path
projection, and a synthetic current-format paginated graph passed through the
production validators. The live scenario is a separate eighth, ignored test
and remains inert without its explicit gate.

The Node harness has nine tests covering the closed text and paired-tool
scenario schema, unknown/duplicate/unsafe input rejection, position-zero marker
extraction, exact pre-journal ordered-history validation including a body over
64 KiB, negative history permutations, current-turn tool-result and exact call
ID matching, bounded argument/shutdown parsing, and a real loopback lifecycle
with a path-free audit.

The live gate supplies the process-level checks that are not synthetic: real
app-server initialization and ID correlation, dynamic UUIDv7 lineage, fork
cutoffs, graceful and unwind cleanup, current Codex rollout structure, exact
scenario consumption, and the absence of credentials or authorization.

## 10. Implementation file set

Implemented tracked file set:

- `package.json`
- `package-lock.json`
- `scripts/live-codex-aimock-server.mjs` (new)
- `scripts/live-codex-aimock-server.test.mjs` (new)
- `scripts/run-live-codex-aimock-e2e.mjs` (new)
- `src-tauri/tests/live_codex_aimock_e2e.rs` (new)
- `crates/mallard-infrastructure/src/codex_history_graph.rs` (current native
  paginated ordinal semantics)
- `src-tauri/src/project_sync/commands.rs` (current Codex display-title decode
  on Pull)
- `README.md`
- this plan document

The existing API-backed files remain the separate authenticated-provider gate:

- `scripts/run-live-codex-e2e.mjs`;
- `src-tauri/tests/live_codex_e2e.rs`; and
- `docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md`.

Their `.env.local` contract and paid-provider coverage are intentionally not
removed or rewritten by the aimock implementation.

No changes are made to:

- Mallard product schemas;
- project-sync DTOs;
- storage protocols;
- catalog protocols; or
- migration code.

The two production edits above replace incorrect behavior in the only current
schema-1 execution path. They do not retain an old parser, add a second command
surface, introduce a fallback, or migrate persisted data. The user can delete
local Mallard state and storage data and retest from a clean schema-1 state.

The current worktree contains unrelated and overlapping uncommitted edits.
Implementation must preserve them. In particular, do not reset or overwrite
the existing path-remapping, paginated-history, storage-catalog, Session Detail,
or Claude attachment work.

### 10.1 Mapping to the current tests and implementation thread

The additional flow is implemented in
`src-tauri/tests/live_codex_aimock_e2e.rs`. The existing
`src-tauri/tests/live_codex_e2e.rs` remains the authenticated custom-provider
target:

- keep `.env.local`, real-provider configuration, provider-parser tests, and
  the paid-use gate confined to the existing API-backed target;
- give the aimock target focused literal-IPv4 loopback and keyless-override
  tests covering absent `env_key`, `requires_openai_auth=false`, disabled
  compression/plugins, and no credential or real-user-home material in argv;
- reuse the existing process bounds, privacy scans, exact marker parser,
  ProjectSync helpers, and 20-alias snapshot verifier;
- populate `PaginatedGraph` from app-server-returned IDs instead of fixed P0-P4
  constants;
- rename or scope `write_paginated_graph` and `append_paginated_turn` as
  offline fixture helpers, with no call from
  `live_codex_aimock_setup_push_pull_divergence`;
- absorb the native app-server/aimock conformance assertions from Section 4.11
  into the additional ignored scenario without deleting the API target's
  focused loopback POC;
- use native start/turn/fork operations and seven later one-shot
  `codex exec resume` turns in the aimock live graph;
- preserve `validate_paginated_rollout`, dependency cutoffs, frozen-prefix
  checks, `assert_pulled_standalone_rollout`, Session Detail projection,
  revision checks, privacy checks, and fixed-point assertions; and
- update final counters to distinguish source/reopen/mutation `codex exec`
  children from the single app-server child.

The referenced implementation history
`codex://threads/01a0172d-aafd-7332-9123-c3a8d4959bc1` was inspected read-only.
Its revisions 1-10, path-remapping, 20-alias reopening, skill, pet, privacy, and
schema-1 contracts remain acceptance requirements for both targets. Its
`.env.local`, paid-provider, and Luna execution details remain requirements of
the API-backed test; aimock supplies an independent, keyless test transport
for the additional target. Neither target is a fallback for the other.

## 11. Additional aimock runner

The additional runner is `scripts/run-live-codex-aimock-e2e.mjs`. It does not
rewrite `scripts/run-live-codex-e2e.mjs` or remove its `.env.local` and
paid-provider prerequisites.

The runner performs only these steps:

1. require the explicit heavyweight-test gate;
2. sanitize inherited credential and proxy variables;
3. verify repository and pinned aimock prerequisites;
4. run `codex sandbox -- /usr/bin/true` with disposable `HOME` and
   `CODEX_HOME` as a bounded, no-provider sandbox preflight;
5. report a fixed, path-free ordinary-Terminal instruction for the known
   nested macOS Seatbelt failure;
6. verify Cargo;
7. run the exact ignored Rust test; and
8. propagate its exit status.

The Rust test, not the runner, starts and owns aimock.

The aimock user-facing command is:

```sh
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Run that command from an ordinary Terminal. If a development agent is itself
already enclosed by macOS Seatbelt, the nested Codex `workspace-write` sandbox
can fail to initialize even though the same command works normally. The runner
detects that condition before Cargo and exits with a fixed instruction. An
agent-run gate therefore needs permission for the outer test process; the test
deliberately keeps the inner Codex `workspace-write` policy and reviewed
`--add-dir` scope unchanged.

The gate message must say that the command runs a heavyweight local Codex
integration test. It must not mention authentication, paid API use, or a source
profile.

## 12. Verification gates

Run these before the full live gate:

```sh
npm install
node --check scripts/live-codex-aimock-server.mjs
node --check scripts/run-live-codex-aimock-e2e.mjs
node --test scripts/live-codex-aimock-server.test.mjs
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_aimock_e2e --no-run
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_aimock_e2e
git diff --check
```

Then run the complete local live test:

```sh
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Recorded full-gate result on August 19, 2026:

```text
1 passed, 0 failed, 7 filtered; finished in 90.48s
```

The successful run used only the locally owned aimock server. Its 37
deterministic turns were exactly five source turns, five native app-server
graph turns, seven graph-mutation turns, and twenty final history-resume turns,
producing exactly 38 local Responses requests because R1 has two legs. No real
provider request or credential was involved.

Acceptance requires:

- all existing synchronization, divergence, pagination, path-remapping,
  Session Detail, skill, and pet assertions pass;
- exactly 32 short-lived turn-bearing `codex exec` processes, one explicit
  graph-building app-server, one isolated `codex --version` probe, 37
  deterministic turns, and 38 local Responses requests; bounded no-turn
  allocator app-servers are isolated, reaped, and excluded from this stable
  protocol count;
- exact scenario consumption with no retry or strict miss;
- all four forks have native paginated metadata and consume no Responses
  fixture;
- every Codex `MaterializeCloud` target uses copy-on-write; revision 10
  preserves referenced ancestor bytes and dependent histories, while revision
  7 proves the same fresh-alias/no-clobber behavior for unreferenced P2;
- unchanged displaced aliases are hidden only by exact immutable whole-file
  witnesses, while appended, unreadable, or unproven aliases fail open as
  stable retained branches and repeated displacement advances to a distinct
  identity;
- all 20 final aliases reopen with exact validated history;
- R1 creates the expected files and records a successful `patch_apply_end`;
- no real provider credential is read or required;
- no real provider network request occurs;
- `auth.json` and `config.toml` remain absent;
- no aimock request carries provider authorization;
- portable storage contains no provider authorization material or local
  control data;
- the mock and app-server processes are reaped; and
- schema 1 remains unchanged, with no migration or compatibility path.

## 13. Explicit non-goals

This test does not:

- validate model intelligence or prompt quality;
- validate OpenAI, Anthropic, or third-party provider availability;
- exercise a GPU or local inference engine;
- record or replay a real provider response;
- remove or alter the separate authenticated custom-provider test;
- use either live target as a fallback or compatibility branch for the other;
- automate the PTY-only top-level `codex fork` command;
- convert a legacy thread to paginated mode;
- retain a synthetic paginated writer in the live scenario;
- claim that an `exec_command` function call is a native Responses custom tool;
- change Mallard's product schema; or
- introduce migration or compatibility behavior.

## 14. Decision summary

The POC proves that aimock can drive the installed Codex CLI deterministically
for fresh turns, resumes, structured output, strict failures, tool-result
rounds, and real workspace patch application. It also proves that Codex 0.147
can create and fork native paginated threads headlessly, that public CLI fork
renders inherited history without provider authentication, and that fork makes
no provider request.

The implementation therefore adds aimock alongside, but isolated from, the
real-provider gate. A Rust-owned, loopback-only programmatic aimock child
provides the strongest lifecycle ownership. Pre-journal response
factories provide exact history validation without relying on a model or on
aimock's bounded journal body. A single bounded app-server creates the native
dependency graph; one-shot `codex exec resume` processes remain the
authoritative mutation and reopening surface after Mallard materialization.

This design adds a zero-cost deterministic live gate without removing the
authenticated test's external-provider coverage. Both gates exercise the same
current schema-1 Mallard product path; the difference is test-only provider
transport, not production compatibility, migration, or fallback behavior.

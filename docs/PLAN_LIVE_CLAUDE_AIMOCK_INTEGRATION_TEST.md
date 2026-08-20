# Plan: Deterministic Claude Code Integration Test with aimock

Status: implemented and verified. The dedicated live target, strict aimock
harness, harness regression suite, runner, package script, and user-facing
documentation are complete. The temporary protocol POC was removed after its
assertions were absorbed. No production behavior, DTO, manifest, schema, or
lockfile change was required for this test.

Date: 2026-08-19 (America/Los_Angeles)

## 0. Implementation result

The implemented gate is:

```sh
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

It passed on August 19, 2026 with `1 passed, 0 failed`; the measured Rust test
body was 12.91 seconds and the package command took 14.80 seconds in the final
sentinel-inheritance run. The Node harness regression suite passes 14 tests.

The final implementation uses Claude Code `2.1.235`, aimock `1.39.0`, and the
mock model identifier `claude-sonnet-4-6`. It completes all five source turns,
both Keep Both flows, the revision-5 fixed point, all six final snapshot
resumes, and the exact three-lineage history oracle.

The initial POC and aimock journal exposed only model requests. Raw HTTP
observation in the hardened implementation revealed one additional native
Claude compatibility probe per model-turn process:

```text
12  POST /v1/messages?beta=true   exact model requests
11  HEAD /api/hello               exact credential-free native probes
23  total raw loopback requests
```

The 12 model requests remain the semantic request budget in this document.
They are separately reconciled with the aimock journal and all 12 fixtures.
The 11 probes are not model requests; they carry no `X-Api-Key`,
`Authorization`, or alternate credential header. Any count change, malformed
probe, other endpoint, duplicate fixture, or unexpected raw request fails the
audit.

One verification run also showed that Claude can return its final JSON result
just before the corresponding assistant record is visible in the JSONL. The
Rust wrapper therefore performs a bounded five-second durability wait for the
complete expected transcript record. It never retries the Claude process or a
model request, so the 11-model-turn-process and 12-model-request budgets remain
exact.

The sections below retain the original plan and POC rationale where useful.
Implementation-specific corrections and measured behavior take precedence
over future-tense wording in that design record.

## 1. Decision and intended outcome

The implementation adds an ignored, explicitly gated integration target for
the real Claude Code CLI that is parallel to, but smaller than, the existing
Codex aimock target:

```sh
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

The target:

- runs the installed `claude` executable against one strict, programmatic
  `@copilotkit/aimock` server bound to an ephemeral literal-IPv4 loopback port;
- creates and resumes real Claude Code JSONL sessions under disposable
  `CLAUDE_CONFIG_DIR` roots;
- exercises Mallard's current schema-1 setup, reviewed Push, reviewed Pull,
  divergence, Keep Both, identity decoupling, and fixed-point paths for a
  Claude-only project binding;
- proves that a transcript reidentified and relocated by Mallard remains
  resumable by the real CLI on a second project path;
- validates the exact history sent to aimock for every final materialized alias;
  and
- makes no real Anthropic or other model-provider request.

The test must be described as **credentialless local-mock testing**, not as a
headerless or keyless test. Claude Code 2.1.235 in `--bare` mode requires a
non-empty `ANTHROPIC_API_KEY`. The test supplies one fixed non-secret
placeholder only to satisfy the client, points `ANTHROPIC_BASE_URL` at the
owned loopback server, and supplies no real credential. The request therefore
contains an `X-Api-Key` header whose value aimock redacts. It contains no
`Authorization` header.

The implementation is additive test infrastructure. It must not add an
aimock/provider fallback to Mallard, a legacy Claude format path, or a product
configuration option.

## 2. What the initial POC delivered

The initial executable protocol POC was:

```sh
node scripts/poc-live-claude-aimock.mjs
```

It deliberately stopped before Mallard setup, storage publication, and
reconciliation. It proved the protocol and native-file assumptions that would
otherwise have made the larger Rust integration target speculative. The POC
file is no longer retained: the committed Node regressions and Rust live target
now cover its claims through the production flow.

The POC:

1. removes inherited provider credentials, provider endpoints, and proxy
   variables before importing aimock;
2. verifies the pinned aimock package and the exact installed Claude Code
   version;
3. starts `LLMock` with `host: "127.0.0.1"`, `port: 0`, and `strict: true`;
4. creates isolated source and destination projects, process homes, and
   `CLAUDE_CONFIG_DIR` roots;
5. launches a real Claude Code source session with `--bare`, `--print`,
   `--output-format json`, `--permission-mode dontAsk`, and `--tools=Read`;
6. serves a deterministic `Read` tool call followed by a fixed assistant
   response;
7. reads the real source JSONL, mirrors the current Claude codec's exact
   top-level `sessionId`/`cwd` rewrite into a new session ID and canonical
   destination cwd, and installs it at the destination Claude bucket;
8. resumes the relocated destination ID through the real CLI;
9. validates the ordered old-user, old-assistant, new-user history received by
   aimock; and
10. proves that the authoritative source transcript was unchanged.

The POC uses only Node built-ins plus the already pinned aimock dependency. It
does not add another npm package or write outside its private temporary root.

## 3. Initial POC evidence

Observed versions:

| Component | Version |
| --- | --- |
| Node.js | 20.19.4 |
| `@copilotkit/aimock` | 1.39.0 |
| Claude Code | 2.1.235 |
| Mock model identifier | `claude-sonnet-4-6` |

The POC passed with this bounded result before removal:

```json
{
  "result": "passed",
  "request_count": 3,
  "request_paths": [
    "/v1/messages?beta=true",
    "/v1/messages?beta=true",
    "/v1/messages?beta=true"
  ],
  "source_durable_records": 4,
  "destination_durable_records": 6,
  "authoritative_source_unchanged": true,
  "authorization_header": "absent",
  "placeholder_api_key_header": "present_and_redacted"
}
```

The three counts in that result were aimock-journaled Messages requests. The
POC did not yet observe the raw HTTP boundary and therefore did not count
Claude's credential-free `HEAD /api/hello` probe. The three model requests
were:

| Request | Purpose | Result |
| --- | --- | --- |
| 1 | Source prompt | aimock returned one `Read` tool use |
| 2 | Source tool result | aimock verified the call ID and file marker, then returned the source response marker |
| 3 | Destination resume | aimock verified the complete marker history, then returned the destination response marker |

The normalized history observed on request 3 contained this exact marker
subsequence:

```text
user       MALLARD_CLAUDE_POC_USER_A
assistant  MALLARD_CLAUDE_POC_RESPONSE_A
user       MALLARD_CLAUDE_POC_USER_B
```

The relocated transcript retained the intervening Claude `tool_use` and
`tool_result` records as well. Its six durable records formed one exact parent
chain, and every durable record carried the destination session ID and
canonical destination cwd after the resume appended its second turn.

This proves the most important native compatibility boundary: moving a real
Claude Code transcript to
`<CLAUDE_CONFIG_DIR>/projects/<encoded-destination-cwd>/<new-id>.jsonl` while
rewriting only the top-level structural `sessionId` and `cwd` values produces a
session that Claude Code 2.1.235 can resume with its prior history intact.

## 4. POC findings that constrain the full test

### 4.1 Use the Anthropic Messages endpoint directly

Claude Code sent streaming `POST` requests to:

```text
/v1/messages?beta=true
```

aimock normalized those requests to its provider-neutral message model, so the
same fixture API validates text history, tool calls, and tool results. The
implemented audit pins the native method and path because the test is
specifically a Claude Code/Anthropic Messages conformance test. Raw-boundary
observation additionally pins Claude's separate credential-free
`HEAD /api/hello` compatibility probe.

### 4.2 Keep the placeholder distinction explicit

Official Claude Code configuration supports `ANTHROPIC_BASE_URL` for a custom
gateway and documents that `ANTHROPIC_API_KEY` is sent as `X-Api-Key` in
non-interactive mode. `CLAUDE_CONFIG_DIR` selects the isolated provider home.
See the official [environment-variable reference](https://code.claude.com/docs/en/env-vars).

The full test therefore asserts all of the following together:

- the child environment contains only the fixed placeholder, never an
  inherited Anthropic key, OAuth token, or custom auth token;
- `ANTHROPIC_BASE_URL` is the exact owned loopback URL;
- aimock sees a present-but-redacted `X-Api-Key` header;
- aimock sees no `Authorization` header;
- the placeholder string is absent from transcripts, Mallard state, manifests,
  payload objects, receipts, logs, and command arguments; and
- no upstream/proxy/record configuration exists on the aimock server.

### 4.3 Use `--bare` and a closed tool set

The installed CLI's `--bare` mode skips hooks, plugin synchronization,
auto-memory, background prefetch, keychain reads, and project/user instruction
discovery. The POC additionally exposes only the built-in `Read` tool with:

```text
--permission-mode dontAsk --tools=Read
```

This is sufficient for a real tool round without granting shell, write, web,
MCP, plugin, or subagent behavior. Official non-interactive usage and output
formats are documented in [Run Claude Code programmatically](https://code.claude.com/docs/en/headless)
and the [CLI reference](https://code.claude.com/docs/en/cli-usage).

Do not pass `--tools ""`. In the installed CLI, the variadic option consumed
the following prompt and the process failed with “Input must be provided.” The
single-argument `--tools=Read` form was proven and exposed exactly one tool in
the normalized request.

### 4.4 Serialize tool arguments explicitly

The aimock documentation accepts object or string tool arguments. In the
installed 1.39.0 programmatic Claude path, the POC observed that an object-form
`Edit` call arrived at Claude Code without its required fields, while the
explicit `JSON.stringify(...)` form worked. The full harness must retain the
proven string form and add a harness test that fails if the Claude wire builder
loses those arguments.

This is a test-fixture choice, not a product compatibility path. Relevant
aimock behavior is documented in [Fixtures](https://aimock.copilotkit.dev/fixtures/)
and [Multi-Turn Conversations](https://aimock.copilotkit.dev/multi-turn/).

### 4.5 Canonicalize before deriving the Claude bucket

On macOS, the disposable `/var/...` spelling canonicalized to
`/private/var/...`. Claude wrote the canonical cwd and used the corresponding
bucket. The POC initially demonstrated why string-comparing an uncanonicalized
temporary path is wrong.

The full test canonicalizes both project roots first, uses those canonical
values for setup and CLI current directories, and derives the expected bucket
through production `encode_claude_project_path` rather than a test-only path
guess.

### 4.6 Expect provider-owned non-chain records

Claude Code 2.1.235 wrote provider bookkeeping records such as `atis-latch`,
`queue-operation`, and `last-prompt` around the durable `user` and `assistant`
records. Mallard's current Claude codec correctly derives lineage only from
durable main-chain `user`, `assistant`, `system`, and `attachment` records with
UUID identity and excludes sidechains.

The test asserts the production analysis result and exact semantic
history. It does not delete or invent handling for the provider-owned
non-chain records.

## 5. What the POC does not claim

The POC does not yet prove:

- Mallard discovery or capture of the real transcript;
- typed conversation publication or immutable storage writes;
- reviewed Pull through the production Claude adapter;
- cloud/local/provider identity bindings;
- divergence classification or Keep Both;
- sidecar, todo, memory, or Library catalog behavior;
- S3/R2 parity;
- UI behavior;
- Windows Claude bucket behavior; or
- compatibility with any Claude Code version other than 2.1.235.

The implemented target now proves discovery/capture, typed publication,
reviewed production Pull, identity bindings, divergence, and both Keep Both
directions. Sidecars, Library behavior, S3/R2 parity, UI behavior, Windows
bucket behavior, and other Claude versions remain with existing focused suites
or outside this target. The POC's JavaScript rewrite was not promoted into a
second implementation.

## 6. Relationship to the existing Codex aimock test

The target copies the product scenario, not Codex's provider-specific protocol
machinery.

| Concern | Existing Codex aimock target | Implemented Claude aimock target |
| --- | --- | --- |
| Provider home | disposable `CODEX_HOME` | disposable `CLAUDE_CONFIG_DIR` |
| Native endpoint | OpenAI Responses | Anthropic Messages |
| Authentication shape | custom keyless provider, no auth header | fixed local placeholder in `X-Api-Key`, no real key and no `Authorization` |
| CLI isolation | Codex ignore-config/rules flags and sandbox | Claude `--bare`, `dontAsk`, `--tools=Read` |
| Session storage | Codex rollout tree and index | `projects/<encoded-cwd>/<session-id>.jsonl` |
| Pull identity | Codex UUIDv7 allocator plus copy-on-write aliasing | local UUIDv4 minting; non-Codex cloud updates replace in place |
| Provider graph | native paginated `history_base` DAG | one linear Claude parent chain |
| Native control process | `codex app-server --stdio` | none |
| Initial tool | `exec_command`/`apply_patch` | read-only Claude `Read` |
| Final native verification | `codex exec resume` on snapshots | `claude --resume` on transcript snapshots |
| Product topology | two replicas, five source revisions, Keep Both both ways | same five-revision linear topology |

The Claude test does not duplicate Codex-only pagination, app-server forks,
desktop index behavior, path-map tokens, subagent closure, or pet coverage.
Those are not Claude capabilities and would obscure the provider boundary this
target is meant to prove.

## 7. Implemented two-replica revision matrix

Use two isolated machine repositories, two canonical project roots, two
Claude homes, one local-folder storage, and one project bundle. Bind only the
Claude provider. This also proves that no unrelated Codex profile/readiness
gate is required for a Claude-only operation.

### Revision 1: real source session on replica A

1. Finalize a new project setup with replica A's Claude profile.
2. Create a bounded read-marker file in the disposable project.
3. Start one real Claude Code session with an explicit valid UUID.
4. aimock returns one `Read` tool call for the exact marker file.
5. Claude returns the tool result; aimock validates its call ID and marker and
   returns `MALLARD_CLAUDE_BASE`.
6. Locate the transcript through the production Claude bucket encoder.
7. Require production discovery, analysis, timeline summary, and Session
   Detail to agree on the provider ID, canonical cwd, four durable records,
   and complete lineage.
8. Perform reviewed Push and require revision 1.
9. Assert the local thread ID, provider session ID, and newly minted cloud ID
   occupy their intended namespaces and that no machine path occurs in the
   portable manifest or object key.

This is one Claude process and two aimock requests.

### Revision 2: pull and resume on replica B

1. Finalize replica B by linking the existing bundle with its own Claude home
   and canonical project copy.
2. Plan and apply reviewed Pull.
3. Require Mallard to mint a destination provider session ID, write exactly
   one transcript beneath B's encoded bucket, and register B's local thread
   against the same cloud ID without reusing A's local or provider identity.
4. Analyze the installed bytes with the production adapter and require all
   structural session IDs/cwds to name B while lineage and semantic digests
   remain equal to revision 1.
5. Resume that real materialized ID through Claude Code with
   `MALLARD_CLAUDE_USER_R2` and receive `MALLARD_CLAUDE_REMOTE_BRANCH`.
6. aimock must see the revision-1 user/assistant markers before R2.
7. Push replica B and require revision 2.

This adds one Claude process and one aimock request.

### Revision 3: divergent local work and Keep Both on replica A

1. Resume A's original revision-1 provider ID with
   `MALLARD_CLAUDE_USER_R3` and receive `MALLARD_CLAUDE_LOCAL_BRANCH`.
2. Require Push preparation to block on the revision-2 divergence.
3. Plan Pull and require exactly one `KeepBoth` conversation action for the
   tracking local thread.
4. Apply Keep Both and prove:
   - the local R1-R3 transcript remains under its original provider ID;
   - it moves to the apply-minted preserved local thread with provenance that
     names the original cloud ID;
   - the remote R1-R2 transcript is rewritten under a fresh provider ID and
     remains on the tracking local/cloud row; and
   - both provider IDs resolve to distinct regular no-follow files in the
     correct A bucket.
5. Native-resume snapshots of both branches once here, or defer the equivalent
   proof to the final six-alias matrix below.
6. Push A and require revision 3, publishing the preserved R1-R3 lineage under
   its own fresh cloud identity.

This adds one source Claude process and one source aimock request. Any optional
native checks use disposable snapshots and are counted separately; the
recommended design defers them to the final matrix.

### Revision 4: reverse-direction divergence

1. Pull revision 3 on B and require the first preserved lineage to materialize
   under B's own provider ID.
2. Resume A's current remote-primary alias from R1-R2 with
   `MALLARD_CLAUDE_USER_R4`; return `MALLARD_CLAUDE_REVISION_FOUR`.
3. Independently resume B's still-shared primary alias from R1-R2 with
   `MALLARD_CLAUDE_USER_R5`; return
   `MALLARD_CLAUDE_SECOND_LOCAL_BRANCH`.
4. Push A first and require revision 4.
5. Require B's Push to be gated as divergent.

This adds two Claude processes and two aimock requests.

### Revision 5: Keep Both on replica B and fixed point

1. Plan and apply B's reverse-direction Keep Both.
2. Require A's R1-R2-R4 cloud branch to become B's tracking primary and B's
   R1-R2-R5 branch to remain preserved under its old provider ID and a fresh
   local thread.
3. Push B and require revision 5, publishing the second preserved lineage
   under a third cloud ID.
4. Pull revision 5 on A.
5. Require both replicas to contain exactly the same three cloud lineages,
   each mapped to machine-local thread and provider identities.
6. Require comparison to report only `Synced` rows, zero divergence, and base
   revision 5.
7. Run reviewed no-op Pull on both replicas and reviewed no-op Push on both
   replicas. The bundle head must remain revision 5 and the storage digest
   inventory must remain byte-identical.

No model request is needed for planning, apply, UUIDv4 minting, publication,
or the fixed-point checks.

## 8. Final history oracle

The final cloud lineages are:

| Lineage | Ordered user/assistant markers |
| --- | --- |
| Primary | R1 user, base response, R2 user, remote response, R4 user, revision-four response |
| Preserved on A | R1 user, base response, R3 user, local response |
| Preserved on B | R1 user, base response, R2 user, remote response, R5 user, second-local response |

For all three cloud IDs in both homes:

1. resolve the machine-local thread and provider ID through the committed
   conversation binding;
2. prove the production Claude history parser and paged Session Detail output
   return the exact oracle without truncation or role drift;
3. copy only the exact transcript into a disposable verification
   `CLAUDE_CONFIG_DIR` under that replica's encoded cwd and provider ID;
4. run public `claude --resume <provider-id> -p ...` against aimock with a
   unique verification prompt;
5. have the response factory compare the complete prior marker history before
   returning its verification response; and
6. discard the snapshot so the authoritative home is byte-identical before
   and after verification.

The R1 tool round must also survive in each descendant history. The validator
should require one `Read` tool call with the fixed call ID and one matching tool
result containing the read-file marker between the R1 user marker and the base
assistant marker. Marker-only comparison is insufficient for this row.

Use snapshots instead of depending on `--no-session-persistence`. A snapshot
makes the non-mutation claim explicit even if Claude changes the semantics of
that option.

## 9. Exact process and request budget

The source scenario has five real Claude Code processes:

- R1 source turn: 1 process, 2 Messages requests because of the tool round;
- R2, R3, R4, and R5: 4 processes, 4 Messages requests.

The final oracle adds six snapshot-resume processes and six Messages requests.

The complete target budget is therefore:

| Counter | Exact value |
| --- | ---: |
| Source Claude model-turn processes | 5 |
| Final verification Claude model-turn processes | 6 |
| Total Claude model-turn processes | 11 |
| R1 tool-round requests | 2 |
| Other source requests | 4 |
| Final verification requests | 6 |
| Total `/v1/messages?beta=true` requests | 12 |
| Credential-free `HEAD /api/hello` probes | 11 |
| Total raw loopback requests | 23 |

The package runner and the Rust target each perform one bounded, isolated
`claude --version` preflight. They happen outside the model-turn accounting,
before the mock traffic, and do not contact aimock. “11 processes” below means
11 model-turn processes, not those two version checks.

No retry path is accepted. Claude Code 2.1.235 performs exactly one
credential-free hello probe for each of the 11 model-turn processes. The raw
observer admits only those 11 literal `HEAD /api/hello` probes and the 12
authenticated Messages requests. Any repeated scenario, other endpoint,
malformed or credential-bearing probe, unexpected request, or changed count
fails the audit. Claude UUID allocation is local and does not contact aimock.

## 10. Programmatic aimock harness

The implementation adds a Claude-specific server instead of weakening the
working Codex harness:

```text
scripts/live-claude-aimock-server.mjs
scripts/live-claude-aimock-server.test.mjs
```

The server should follow the proven Codex ownership pattern:

- a Rust-owned Node child;
- strict programmatic `LLMock`, not CLI record/replay;
- literal `127.0.0.1`, port `0`;
- a bounded, mode-0600 scenario document in a mode-0700 control directory;
- one JSON readiness line containing only type, schema version, and port;
- bounded stdin shutdown command;
- a create-new mode-0600 audit document;
- no proxy, provider key, remote fixture, recording, or fallback; and
- bounded journal/request/scenario counts.

The Claude scenario schema should contain only values the server needs:

```text
version
model
expected_request_count
expected_probe_request_count
scenarios[]:
  id
  prompt_marker
  expected_history[]        # role + exact marker
  require_tool_result
  expected_tool_call_id
  expected_tool_result_marker
  response                  # fixed text or the one bounded Read call
```

The server's dynamic response must validate, before returning a response:

- exact model and `stream: true`;
- bounded normalized message and tool counts;
- one current user marker;
- complete ordered prior marker history;
- exact R1 tool name, call ID, JSON arguments, and result marker;
- one request per scenario;
- no duplicate or reordered marker;
- no `Authorization` header;
- a present, redacted `X-Api-Key` header;
- native model method/path/status;
- exactly one credential-free literal `HEAD /api/hello` probe per Claude
  model-turn process; and
- absence of any other raw request route or method.

aimock's strict mode is the routing backstop; the Mallard audit is the stronger
semantic contract.

## 11. Runner design

The implementation adds:

```text
scripts/run-live-claude-aimock-e2e.mjs
```

and the package script:

```json
{
  "test:live-claude-aimock-e2e": "node scripts/run-live-claude-aimock-e2e.mjs"
}
```

The runner must:

1. require `MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1`;
2. require Node 20.15.0 or newer;
3. verify `@copilotkit/aimock` is exactly the pinned version by reading package
   metadata without importing third-party code in the caller environment;
4. verify Cargo is available;
5. run `claude --version` in a closed, credential-free environment and require
   exactly the version whose native transcript format the target currently
   proves;
6. remove inherited Anthropic/OpenAI/cloud-provider credentials, custom
   endpoints, Claude/Codex home overrides, and upper/lower-case proxy variables;
7. add only the live-test gate and loopback `NO_PROXY` values; and
8. launch one exact Cargo test with `--ignored --exact --nocapture
   --test-threads=1`.

Unlike the Codex runner, this runner needs no nested-sandbox preflight because
Claude is restricted through `--bare`, `dontAsk`, and `--tools=Read`; it does
not launch a nested Codex Seatbelt sandbox.

The runner must never read `.env.local`, macOS Keychain, a real
`~/.claude`, `.credentials.json`, or a source profile.

## 12. Rust integration target

The implementation adds:

```text
src-tauri/tests/live_claude_aimock_e2e.rs
```

with one ignored entry point:

```text
live_claude_aimock_setup_push_pull_divergence
```

The target uses production application/service boundaries exactly as the
Codex target does. The implemented test remains self-contained instead of
mechanically moving helpers out of the already active Codex integration target.
That kept this additive provider test isolated, avoided changing the working
Codex scenario, and required no production or shared-test compatibility layer.
Claude transcript, Messages, and native-resume behavior remains confined to
the Claude file.

The Claude-specific Rust helpers should be limited to:

- exact Claude version validation;
- a bounded `run_claude` process wrapper;
- Claude bucket/transcript resolution through production
  `encode_claude_project_path`;
- real-transcript structural assertions through `ClaudeAdapter`;
- Claude marker extraction for diagnostics;
- snapshot construction for final native resumes; and
- lifecycle/audit ownership for the Claude aimock child.

`run_claude` must build argv directly with `Command`, never through a shell. It
must bound argument count/length, prompt bytes, output bytes, process duration,
and diagnostics. It should parse the one JSON result and require one exact
response marker and the expected session ID.

## 13. Safety and privacy contract

The full target must assert these boundaries, not merely rely on convention:

### Process isolation

- All project, provider, process, Mallard, scenario, audit, and storage roots
  are fresh temporary no-follow directories.
- Provider/control directories are mode 0700; scenario/audit/transcript files
  created by the harness use mode 0600 on Unix.
- `HOME`, `USERPROFILE`, and `CLAUDE_CONFIG_DIR` point inside the temporary
  root for every Claude process.
- `--bare` is mandatory.
- Only `Read` is exposed; Bash, Edit, Write, WebFetch, MCP, plugins, hooks,
  agents, and browser tools are unavailable.
- Commands use bounded argv and no shell interpolation.

### Credential isolation

- No real provider credential is read or inherited.
- The fixed placeholder exists only in the closed Claude child environment and
  its redacted loopback header.
- No OAuth token, `ANTHROPIC_AUTH_TOKEN`, AWS credential, proxy credential, or
  `.credentials.json` exists in the test roots.
- The placeholder and known inherited-secret markers are absent from stdout,
  stderr summaries, transcripts, Mallard state, manifests, payload objects,
  storage keys, journals, receipts, and audits.

### Storage and restore invariants

- Push remains reviewed, immutable, SHA-256 verified, and CAS-published.
- Pull remains plan-based and explicitly approved.
- Revision, digest, binding, target, expiry, and warning pins remain active.
- Keep Both never overwrites the preserved Claude transcript.
- Materialized paths remain below the exact selected Claude home and encoded
  project bucket.
- Every target is a regular no-follow file; no symlink or special file enters
  provider state or storage.
- No deletion is needed in this scenario.
- Absolute machine paths, local IDs, profile IDs, storage IDs, credentials,
  and target paths are absent from the portable manifest and storage keys.

### Mock isolation

- The only model endpoint is the owned literal-loopback URL.
- aimock has no upstream, proxy, recorder, or remote fixture.
- Strict unmatched requests fail.
- Request counts and scenario counts are exact.
- Stopping the mock and validating its audit are mandatory even after all
  product assertions pass.

## 14. Scope exclusions

Do not add the following to this target:

- Codex paginated/history-base graphs or app-server calls;
- Claude sidechains/subagents or fork-session semantics;
- Claude memory, todo, and file-history fixture breadth already covered by
  adapter tests;
- storage Library skills/plugins, which have their own provider-neutral E2E;
- S3/R2 transport parity already covered below the shared bundle engine;
- UI automation;
- a paid Anthropic comparison target;
- a provider fallback in Mallard;
- a compatibility parser for older Claude Code JSONL; or
- schema/version changes.

If a real live transcript exposes a current-path product defect, fix the one
current path with a focused regression before extending the live scenario. Do
not hide it with fixture rewriting outside the production adapter.

## 15. Layered implementation order

The target was built in these working end-to-end layers:

1. **Harness unit:** Claude aimock server accepts one fixed text request and
   emits a bounded audit; malformed scenarios and shutdown commands fail.
2. **Native one-turn layer:** Rust starts the server and real Claude CLI under
   one temporary Claude home; no Mallard storage yet.
3. **R1-to-B layer:** production setup, Push revision 1, Pull to B, and native
   resume of Mallard's reidentified transcript.
4. **R2-R3 layer:** one divergence and Keep Both on A, ending at a synced
   revision 3 product.
5. **R4-R5 layer:** reverse-direction divergence, second Keep Both, and final
   revision 5 fixed point.
6. **History layer:** production timeline/Session Detail plus six snapshot
   native resumes.
7. **Safety layer:** exact request/process accounting, storage privacy scans,
   no-op digest inventory, and final audit.
8. **Cutover cleanup:** document the user-facing gate, absorb every POC
   assertion into committed harness/Rust tests, and delete
   `scripts/poc-live-claude-aimock.mjs` so the temporary probe does not become
   an obsolete parallel path.

Each layer passed before the next was added. The hardened raw-boundary layer
then exposed and pinned the native hello probe that the journal-only POC could
not see.

## 16. Implemented file changes

New:

- `scripts/live-claude-aimock-server.mjs`
- `scripts/live-claude-aimock-server.test.mjs`
- `scripts/run-live-claude-aimock-e2e.mjs`
- `src-tauri/tests/live_claude_aimock_e2e.rs`

Modified:

- `package.json` for the one new ignored live script
- `README.md` for the credentialless local-mock gate and exact version/current
  verification result
- this plan's status and measured accounting

Removed:

- `scripts/poc-live-claude-aimock.mjs`, after its checks were absorbed into the
  server unit tests and Rust integration target

No production Rust module, frontend file, DTO, schema, manifest version, or
lockfile changed for the test itself. The working Codex live targets were not
refactored or changed by this implementation.

## 17. Verification commands

The implemented verification sequence is:

```sh
node --check scripts/live-claude-aimock-server.mjs
node --check scripts/run-live-claude-aimock-e2e.mjs
node --test scripts/live-claude-aimock-server.test.mjs
cargo test --manifest-path src-tauri/Cargo.toml --test live_claude_aimock_e2e --no-run
cargo test --manifest-path src-tauri/Cargo.toml --test live_claude_aimock_e2e
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

The ungated Cargo invocation should report the ignored test without launching
Claude or aimock. The final gated command reports the exact budget of 11
model-turn processes, 12 model requests, 11 probes, and 23 raw requests, plus
a successful mock audit.

The final runner check was also executed with inherited Anthropic, aimock, and
upper/lowercase proxy variables set to the fixed
`mallard-inherited-secret-sentinel`. The runner removed them before Cargo, and
the Rust privacy scans found the sentinel nowhere in provider homes, Mallard
homes, storage, process output, or the audit.

## 18. Acceptance criteria

The full test is complete only when all of these are true:

1. The real supported Claude Code CLI creates and resumes real JSONL sessions
   under isolated homes.
2. All model-shaped responses come from the one owned aimock server.
3. No real provider credential, account, login, remote model, GPU, or paid token
   is needed.
4. The R1 `Read` tool request/result is exact and survives all descendant
   histories.
5. Mallard's production adapter, not a test rewrite, performs Pull relocation
   and reidentification.
6. The pulled B transcript resumes with the exact R1 history.
7. Both divergence directions offer and apply Keep Both without data loss.
8. Three distinct final cloud lineages exist on both replicas with
   machine-local local/provider IDs.
9. Production history and Session Detail match the three exact history
   oracles in both homes.
10. Six disposable native resumes send the exact expected histories and do not
    mutate authoritative provider homes.
11. Both replicas reach revision-5 fixed point; reviewed no-op Push/Pull does
    not create revision 6 or replace a storage object.
12. The exact count is 11 Claude model-turn processes, 12 successful local
    Messages requests, 11 credential-free hello probes, and 23 raw loopback
    requests, with no duplicate or unmatched request.
13. The final audit proves no `Authorization` header, only the redacted fixed
    placeholder header, and no configured upstream.
14. The privacy, no-follow, bounded-read, immutable-publication, and reviewed
    apply invariants remain intact.

## 19. Outcome

The dedicated Claude target is complete. The implementation confirms the POC's
main protocol result: aimock 1.39.0 can drive Claude Code 2.1.235 through a
deterministic tool round, and a real tool-bearing transcript relocated with
Mallard's current top-level identity rewrite remains natively resumable on a
second canonical project path.

The implementation remains focused on the five-revision linear Claude scenario.
Codex's native graph breadth should stay in the Codex target, while the Claude
target should prove the distinct provider contract that existing synthetic
tests cannot: real Claude JSONL creation, production relocation, and native
resume after reviewed cross-machine synchronization.

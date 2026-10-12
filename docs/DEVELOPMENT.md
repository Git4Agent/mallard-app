# Development

Building, verifying, and the live and end-to-end test suites. Current behavior is
described in [FEATURES.md](FEATURES.md), [STORAGE_SETUP.md](STORAGE_SETUP.md) and
[TECHNICAL_REPORT.md](../TECHNICAL_REPORT.md); repository conventions are in
[AGENTS.md](../AGENTS.md).

## Build from source

Requirements: Node.js with npm, a stable Rust toolchain, and Git.

```sh
npm install
npm run dev          # frontend only at http://localhost:1420
npm run tauri dev    # complete desktop app
```

Application configuration is created through the app; normal development does
not require an `.env` file. The opt-in paid live test has a separate local
connection input described below. The additional aimock live tests are
entirely local and do not use that connection file.

`npm run dev` starts only the Vite frontend and does not provide the Tauri
filesystem and storage backend used by these test flows.

## Verify

```sh
npm run build
npm run test:frontend-integration # strict test type-check plus frontend tests
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:backend-integration  # all Rust targets plus loopback aimock tests
npm run test:integration          # all committed standard suites
```

The backend suite includes Rust and aimock server tests that bind localhost.
Restricted environments may require permission for these test servers. The
aimock server tests use fixed local responses and require no credentials.

## Codex Desktop project placement

Windows Open in Codex uses Desktop's native project move and navigation
operations. It verifies the selected profile before either operation: Windows
Restart Manager identifies the Core process holding a state database in the
selected `CODEX_HOME`, that process must be a live child of the registered
Desktop executable, and the native pipe must belong to that same Desktop.
Profile discovery uses only query APIs; Mallard never shuts down or restarts
Desktop. If Core changes, Mallard re-verifies the same selected home and Desktop
before continuing on the existing pipe. It never replays an unacknowledged move.

Mallard starts or focuses the installed Codex Desktop, then verifies that it
uses the selected profile. If the destination is missing, Mallard sends its
folder deep link once, then polls Desktop's project list within the 60-second
placement deadline. Placement requires one exact local folder match; an
ambiguous or unconfirmed destination returns an error. A readable existing chat
can report `notLoaded`: Mallard waits for its
stored history, checks its identity and the native move acknowledgement, and
does not require an idle agent.
Backend project IDs and Desktop project IDs are distinct.
Backend `thread/metadata/update` is no longer used for Windows placement.

The [native placement POC](POC_CODEX_DESKTOP_PROJECT_PLACEMENT.md) documents the
connection contract and installed Desktop version. Run the production test with
the installation variables described there:

```powershell
node scripts/poc-codex-native-placement.mjs --phase=registration-integration `
  "--cli=$codexCli" "--executable=$desktopExe" `
  "--connector=$appToolsServer" "--family=$packageFamily"
```

This phase creates a disposable fixture with no registered project, starts an
isolated Desktop with its own backend, and invokes the Rust production code. It
checks automatic registration, placement of a restored `notLoaded` chat, native
membership after repeated opens, rejection of another home containing the same
chat, and unchanged history. It repeats after restarting only that fixture
Desktop, checking that the same project ID persists. Tests never use personal
provider homes.

macOS uses the same native assignment flow. Desktop rejects direct callers
outside its trusted process tree, so Mallard registers the
`mallard_project_placement` MCP server through Codex's config API. Its script and
private connection records stay in `CODEX_HOME/.mallard-project-placement`.
Desktop starts the helper with its bundled Node. The helper exposes no tools to
the model. Mallard checks the selected home and connection token before it sends
the native move and navigation requests.

Helper registration occurs before the existing macOS `open -n` launch. A loaded
chat does not reliably reload a new MCP registration. For a missing project,
Open in Codex opens the folder link before loading the existing chat. Opening
the folder after connecting can close that helper. Desktop keeps only the latest
pending route, so Mallard waits for the exact folder in the selected home's
project registry before sending the chat link. This bounded check only reads
Desktop state. Placement uses a helper started after the session launch whose
native project list contains that folder. An older window can retain its previous
list or close its helper during navigation. The native move still confirms
membership. Codex retains its normal folder-consent prompt.

Run the macOS native test with the installed official Desktop app:

```sh
node scripts/poc-codex-macos-placement.mjs
```

The runner uses a temporary Codex home, separate Electron data for each Desktop
instance, and one local mock response to create the fixture. Its generated folder
is trusted but initially absent from Desktop's project list. It tests folder
registration, native membership, repeated opens,
restart persistence, rejected requests, configuration
preservation, and complete chat history preservation. It closes only its fixture
processes. Native membership readback is distinct from a screenshot or complete
Mallard UI acceptance test.

## Manual test flows

### Test Flow 1: Pull the public demo project

This flow demonstrates how an existing Mallard project is discovered and
pulled onto a new machine.

1. Clone the public demo repository:

   ```sh
   git clone https://github.com/Git4Agent/mallard_test_demo.git
   ```

2. In Mallard, choose **Add project**, type or paste the path of the cloned
   `mallard_test_demo` folder (or pick it with **Browse…**), and choose
   **Use this folder**.
3. Select the local Codex profile and the R2 storage configured above.
4. Under **Repository**, choose the existing remote repository marked
   **Git match** instead of creating a new repository.
5. Choose **Finish & review**, inspect the Pull plan, approve the intended
   resources, and apply the Pull.
6. Open **Project History** and verify that the project shows:
   - its Git commits and branch timeline;
   - mapped sessions plus any intentionally unmapped sessions;
   - always-visible session summaries;
   - session metrics for activity time, turns, tokens, messages, and tools;
   - expandable conversation previews.

   ![Mallard project history after pulling the public demo project](images/test-flow-1-pulled-project-history.png)

7. Click **Open ChatGPT Desktop** for a session and verify that the restored
   conversation opens in the desktop app.

   ![Restored conversation open in ChatGPT Desktop](images/test-flow-1-open-chatgpt-desktop.png)

### Test Flow 2: Push and transfer your own project

This flow works with a new or existing Git project, or with a non-Git folder.

1. Use the project from Codex so it has at least one project-scoped session.
   For the clearest Git mapping, make one or more commits after or during that
   work.
2. In Mallard, choose **Add project** and give it the folder: type its path,
   pick it under **Where your agents have worked**, or use **Browse…**. Then
   select the Codex profile, choose the configured R2 storage, and select
   **New repository**.
3. Finish setup and open **Project History**. Git projects show all project
   sessions against the selected branch's commit history; both Git and non-Git
   projects show summaries, conversation previews, and session metrics.
4. Choose **Push** and review **Git & sessions** and the final **Review**
   step.
5. For a non-Git project, the review scans the folder as it opens. A file
   already in storage travels when it changed here. A new file waits: open
   **Project files** and tick the ordinary files and folders that should
   travel with the bundle. This step is unavailable for Git-managed content.
6. Complete the Push to publish the reviewed bundle to R2.
7. On another machine, configure the same R2 storage, clone the Git repository
   or create an appropriate local folder, add it to Mallard, select the
   existing remote repository, and complete a reviewed Pull.

### Test Flow 3: Get a project from a Mallard Cloud link

1. Choose **Add project** › **From Mallard Cloud**. Select a connected
   account, or add one and choose **Finish setup** to return here with it
   selected. Setup started from **Storages** opens the new storage's page.
   Or, in an installed build, choose **Open in Mallard** on the repository's
   page on the website and then **Set up project…** on the sheet: this page
   opens with the account ticked and step 2 done.
2. Paste the repository's page address, such as `mallard-ai.com/owner/name`,
   and choose **Look up**, or pick it from the list. A private repository you
   have not joined takes its invitation code.
3. Check **Folder**. Under **New folder** it is the repository's name,
   beside your other projects, and **Browse…** picks the parent and keeps the
   name. Already have the project here? Choose **Existing folder** (or **Use
   it as it is** when the path holds files) and pick the folder itself. It is
   connected as it is; a folder whose Git remote isn't the repository's asks
   for one tick first.
4. Choose **Clone and connect** (**Connect** for an existing folder). A
   repository linked to GitHub is cloned with `gh`, then `git`. If cloning
   fails, Mallard initializes an empty Git repository and shows how to fetch
   the source code. A missing GitHub URL also creates an empty Git repository.
   Git must be installed. Only non-Git projects get a plain folder.
   Nothing is pulled: choose **Pull**
   on the project when ready.

### Test Flow 4: A Mallard Cloud repository removed, and a draft discarded

1. Push a project to Mallard Cloud. On the website, open the repository's
   **Settings** and remove it. In Mallard, refresh: the project reads **Being
   deleted**, then **Repository gone**, every session keeps its own state,
   and Push is open.
2. Choose **Push**. The repository is created again at the same address, as
   version 1, and the note ends. A second computer of the same account reads
   **Created again**, pulls, and pushes.
3. Release a version, change a session, add one, and push again. As the
   Owner, open the Mallard Cloud chip in the path bar and choose
   **Discard draft…**, then
   **Discard**. The project reads **Draft discarded**: the changed session
   is ahead, the added one is only on this computer, and the rest are in
   sync with the public version. Pull, then Push, to start a new draft.

## Live and end-to-end tests

### Live Codex API end-to-end test

The API-backed live Codex test is deliberately separate from
`npm run test:integration`. Its five source-conversation turns use an
OpenAI-compatible Responses provider; its twenty final-history reopen checks
use a bounded loopback Responses recorder and make no remote model request.
Every phase uses only fresh temporary Codex homes, projects, Mallard
repositories, and local-folder storage. The test never reads a user Codex
home, requires a Codex login, or reads, copies, or creates `auth.json` or
another legacy credential file. The ignored `.env.local` described below is
the one intentional credential input.

Create an ignored `.env.local` at the repository root with the provider
connection document:

```json
{
  "_type": "newapi_channel_conn",
  "key": "<provider-api-key>",
  "url": "<https-provider-base-url>"
}
```

On Unix, restrict it to the current user before running the test:

```sh
chmod 600 .env.local
```

The runner accepts exactly those three JSON fields, validates the HTTPS base
URL, and fails closed for a missing, malformed, symlinked, oversized, or
over-permissive file. `.env.local` is covered by the repository's `*.local`
ignore rule and must never be committed.

The current rollout fixtures have a hard `codex-cli 0.162.1` prerequisite. The
test rejects another CLI version before making a model request; update the
current-format fixture in place before moving the gate to another version.
The resume checks accept Codex's unrecognized-configuration warning and the
exact missing-metadata warning for the selected model only before the model
turn starts. They still reject other error items and tool events. This does
not change machine-managed configuration.

Run it only when you intend to spend API tokens:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The explicit paid-use gate remains required even when `.env.local` is valid.
For each of the five provider-backed conversation processes, the harness keeps
user configuration disabled, passes a closed set of non-secret custom-provider
settings through repeated Codex CLI configuration overrides, and exposes the
API key only in that child process's dedicated environment variable. It does
not write the key into a Codex config, prompt, command argument, rollout,
snapshot, Mallard object, or test log. The twenty loopback children receive no
real provider URL or credential. All command shapes set
`features.plugins=false`: Codex marketplace synchronization is outside this
scenario, while the skill and pet phases use Mallard's isolated filesystem
fixtures directly.

The test runs exactly 25 Codex CLI turns with no retry path: five real,
provider-backed `gpt-6-luna` conversation turns at low reasoning effort and
twenty deterministic loopback headless resumes. The count is deliberately a
CLI-turn count; the first content tool-using turn can require more than one
provider HTTP exchange. The scenario exercises two isolated machine homes
through project revision 11, including paginated dependency histories,
reviewed Push/Pull, changed-after-comparison rejection, divergence in both
directions, Keep Both, and a no-op fixed point. It then clears the local-folder
storage through the production operation on both replicas, proves both Codex homes are
unchanged, republishes the same ten conversations as fresh revision 1, and
reaches a new fixed point before the Library and resume phases. The same
topology also shares and updates nested Codex skills and fake v1/v2 pets
through the storage Library without additional provider-backed turns.

For all ten final aliases in both homes, the test first resolves and pages the
exact history through the production Codex history and Session Detail paths.
It then runs public `codex exec --json resume --ephemeral` against a one-request
HTTP server bound to literal `127.0.0.1`. The recorder validates the complete
ordered history Codex sent for that exact alias before returning a fixed SSE
response. Each verifier operates on a disposable snapshot because Codex 0.152
appends a turn even with `--ephemeral`; both authoritative homes must remain
byte-identical.

The Codex 0.162.1 API gate passed on October 9, 2026: all five provider-backed
turns and twenty loopback resumes completed in one 55.10-second Rust test run.

See [`docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md`](PLAN_LIVE_CODEX_INTEGRATION_TEST.md)
for the API topology, POC evidence, revision matrix, and safety assertions.

### Live Claude Code Tuzi end-to-end test

The paid Claude target uses the same ignored repository-root `.env.local`
connection document, but requires its HTTPS URL to be the exact Tuzi site
root (an input ending in `/v1` is normalized back to the root because Claude
Code appends `/v1/messages`). It is separate from `npm run test:integration`
and from the credentialless AIMock target.

Run it only after checking the Tuzi key's quota and consumption log and when
you intend to pay for three Sonnet 5 turns:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
```

The runner requires Node.js 20.15.0 or newer and exactly Claude Code 2.1.266.
Before Cargo starts, it validates `.env.local` as a bounded, no-follow,
mode-`0600` three-field JSON document and performs a bounded authenticated
catalog lookup for exact model ID `claude-sonnet-5`. It never passes the key
through Cargo's environment. The Rust target independently reopens and
validates the connection file, then exposes the key only to each direct
Claude child as `ANTHROPIC_API_KEY`.

The test makes exactly three Claude model-turn processes with automatic
retries disabled, no fallback model, no tools, `--effort low`, a 1,024-token
output limit, and a USD 0.05 Claude client cap per process. It creates a real
session on replica A, publishes content revision 2 after setup registration
revision 1, pulls and reidentifies it onto replica B, resumes the relocated
transcript through Tuzi, and publishes revision 3. Replica A then resumes its
stale base independently. The test
requires Push to be blocked, reviews a divergent Pull offering exactly one
Keep Both action, preserves A's local continuation byte-for-byte, publishes
both lineages as revision 4, and reconciles exact bindings, comparison rows,
provider UUIDs, and root-level Claude JSONLs on both replicas. It applies
reviewed empty Pull plans and proves no-op Pushes, then clears storage through
the production operation on both replicas. Replica A republishes the same two
conversations as fresh revision 1, replica B pulls them, and the test verifies
unchanged Claude homes, semantic digests, lineage digests, identities, and
cross-replica topology.

All Claude homes, projects, process homes, Mallard repositories, and storage
are private temporary roots. The test uses fixed synthetic markers and sends
no repository source or personal conversation. It validates the production
Claude adapter, paged Session Detail history, transcript structural rewrites,
semantic lineage preservation, credential absence, portable-manifest path
privacy, and full-depth no-follow state scans. The USD caps are client-side
estimates; Tuzi's final consumption log remains authoritative, including for
failed or disconnected calls.

The Claude Code 2.1.266 gate passed on September 12, 2026 with `1 passed, 0
failed`; the Rust body completed in 19.89 seconds with explicit launch isolation. No execution automatically
retried.
The Tuzi consumption log remains authoritative for the earlier development
turns and the current three-turn certification run.

See [`docs/PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md`](PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md)
for the research, one-turn POC evidence, three-revision implementation, cost
boundary, and exact limitations.

### Keyless aimock Codex end-to-end test

The additional aimock suite drives the same installed Codex CLI and Mallard
two-replica behavior against a deterministic
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not open `.env.local`, needs no
API account, key, Codex login, GPU, or local model, and makes no external model
request. It is a separate test transport, not a fallback or compatibility path
inside Mallard.

Install dependencies and run it with its independent heavyweight-test gate:

```sh
npm install
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Run the gate from an ordinary Terminal. A development agent already enclosed
by macOS Seatbelt may need permission for the outer test process so Codex can
initialize its nested sandbox; the test still keeps Codex itself at
`workspace-write` with only the reviewed additional directory. The runner
detects the known nested-Seatbelt failure with a bounded, no-provider preflight
before starting Cargo and prints a fixed ordinary-Terminal instruction.
Implementation, protocol POCs, and the complete ignored live gate are verified.
The current native-format gate requires Codex CLI `0.162.1`. The exact keyless
gate passed on October 9, 2026 with `1 passed, 0 failed, 19 filtered` in 74.82
seconds. Every model-shaped response came from the one local
aimock child; the run used no provider account, credential, remote model, GPU,
or external model request.

The Rust test owns the mock lifecycle, supplies a keyless custom provider with
`requires_openai_auth=false`, strips inherited provider/proxy state, disables
plugins and request compression, and permits no retries or Authorization
header. Strict fixture matching rejects unknown, repeated, reordered, or
history-incomplete requests; no proxy, recorder, remote fixture, or upstream
is configured.

The aimock scenario adds native Codex pagination coverage. One bounded
`codex app-server --stdio` process creates P0-P4 with paginated
`thread/start`/`thread/fork` operations, which must consume no Responses
request. Five source turns, five native app-server graph turns, seven later
graph-mutation turns, and twenty final history-resume turns make 37
deterministic turns and exactly 38 local `/v1/responses` requests. The first
content turn uses two intentional scenario legs with the same prompt marker:
an ordinary `exec_command` function call invokes the reviewed `apply_patch`
command, then an exact tool-result call ID selects the fixed final response.
Every other prompt marker is unique, and every fixed response is selected only
after the mock validates the complete ordered marker history.

The ignored `pull_transcript::tests::preparation_benchmark` test measures eight
synthetic Codex transcripts with fixed destination IDs. Set
`MALLARD_PULL_BENCH_WORKERS=1` or `4` and run it with `--ignored --nocapture`.
It reports preparation wall time, summed worker time, input/output bytes,
queue bounds, and temporary transcript reads. It uses a temporary fixture
directory. The normal tests compare serial and parallel bytes and checkpoints
for both providers, check queue reservations, and verify the analysis count.

The same command also runs five-thread failure isolation on a separate pair
of temporary replicas. Push selects five threads, then retries the two failed
threads, then the last failed thread. Its completion counts are 3, 1, and 1.
Pull also completes 3, 1, and 1 threads. Each selected local source is checked
once during Apply. A detected update fails only that thread, with no automatic
retry. This includes an approved Keep both action. The final attempt writes a
new local message after the check and confirms that Apply still completes.
Service phase callbacks make real Codex turns append
fixed fake-API responses after capture. The checks cover exact failed
identities, unchanged completed content, accepted checkpoints, and the next
review's remaining work. Pull retries approve Keep both for divergent local
history and verify that the preserved branches keep every local message.
Seven disposable history resumes verify the five final stored histories and
two preserved branches. This adds 24 turns and requests.

A native revert flow retains the initial rollout and creates a suffixed
replacement with the same thread ID. It warms discovery and Session Detail
before the revert, checks that Mallard selects one current resource, and
rejects the old timeline cursor. Paged Session Detail before Push and after
Pull must show Turn 1 and the replacement Turn 2, with the discarded Turn 2
excluded. A real Codex CLI resume on the destination verifies the exact restored
history without the old source file. The old rollout remains byte-identical.
This flow adds four requests, for exactly 66 local Responses requests in the
complete suite.

The retry flow reports each attempt's elapsed time. Pull also reports the
source check count, byte count, and elapsed time.
It verifies two semantic analyses per prepared transcript and reports summed
preparation time, stage/commit time, and pipeline wall time.
Per-source read and hash counters verify that each attempt checks each source
once, including a failed check.
All fixtures remain temporary; the local and portable schemas stay at 1.

After the revision-11 fixed point, the live flow clears the whole disposable
storage through the production storage-reset operation. It proves that both
replicas forget remote acceptance without changing local Codex state, then
republishes all ten conversations at a fresh revision 1 and converges again.

Both suites exercise the current schema-1 product path. Adding the local mock
does not add a migration, legacy execution branch, or provider fallback to the
application. Copy-on-write materialization likewise changes only the current
execution path: schema 1 remains unchanged, with no migration or compatibility
path.

See [`docs/PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md`](PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md)
for the aimock topology, protocol POCs, exact accounting, and safety contract.

### Credentialless aimock Claude Code end-to-end test

The Claude Code aimock suite is a separate ignored target for the real Claude
CLI and Mallard's production Claude adapter. It runs against one strict
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not read `.env.local`, a user
Claude home, or `.credentials.json`, and it needs no provider account, login,
real API key, remote model, GPU, or paid token.

Install dependencies and run the explicitly gated target with:

```sh
npm install
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

Select the repeated Desktop-import scenario with:

```sh
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e -- --scenario desktop-import
```

This separate scenario has 16 main conversation rounds, four simulated imports,
and three further rounds for visible-edit refusal and Keep both. The mock emits
thinking and redacted-thinking blocks through the real CLI. A test helper removes
those blocks, deletes empty assistant rows, leaves parent links unchanged, and
replaces the transcript atomically. Round 6 also constructs explicit mixed and
consecutive thinking-only rows before Push. Each import must leave the session
Synced; the next turn must become Local ahead and Push back to Synced. The test
checks exact request history, the Read exchange, immutable stored objects, and
both replicas after Pull. It reports time and byte counts without timing limits.
It simulates the file rewrite; it does not launch or validate Claude Desktop.
The import after Pull is a required assertion, including the installed-cutoff
boundary. No padding or metadata reset hides that failure.

The current native-format gate requires exactly Claude Code `2.1.266`,
`@copilotkit/aimock` `1.39.0`, Node.js 20.15.0 or newer, and the mock model
identifier `claude-sonnet-4-6`. The runner strips inherited provider,
provider-home, endpoint, and proxy state before Cargo starts. Each Claude
turn disables settings discovery, hooks, skills, plugins, and MCP discovery
through explicit options. It uses `--permission-mode dontAsk` and a one-tool
`Read` allow list. The test avoids `--bare`: Claude Code 2.1.266 does not await
the initial transcript recording operation in that mode. Claude requires a
non-empty `ANTHROPIC_API_KEY` for this custom endpoint,
so the child receives the fixed non-secret placeholder
`mallard-local-aimock-placeholder`. The mock requires that exact value in
`X-Api-Key`; `Authorization` and alternate credential headers are forbidden.
This is credentialless local-mock testing, not a headerless or keyless flow.

The scenario creates five real source turns across two isolated Claude homes.
Setup publishes a registration-only revision 1, and the source turns publish
content revisions 2 through 6. The first content turn performs a real `Read`
tool round; later turns exercise Pull relocation and reidentification, native
resume on a second canonical project path, divergence in both directions,
both Keep Both flows, three final cloud lineages, paged Session Detail history,
and reviewed revision-6 no-op Push/Pull. Six additional snapshot resumes
verify every final lineage from both replicas without changing either
authoritative Claude home. The flow then clears the whole disposable storage,
proves both homes unchanged, republishes all three conversations at a fresh
revision 1, and verifies one incremental cursor append.

The accounting is exact: 12 Claude model-turn processes make 13 authenticated
`POST /v1/messages?beta=true` requests, because the first content tool round
uses two Messages calls. Claude Code 2.1.266 also makes one credential-free
`HEAD /api/hello` compatibility probe per process. The raw boundary therefore
requires exactly 25 requests: 13 model requests plus 12 probes. The Messages
requests are separately reconciled with all 13 semantic aimock scenarios. Any
duplicate scenario, other route or method, extra request, upgrade, wrong key,
credential-bearing probe, history mismatch, or exposed tool makes the audit
fail. No upstream, proxy, recorder, remote fixture, or retry path is
configured.

The package runner and Rust target also perform their own bounded, isolated
`claude --version` preflights. Those are not model turns, start before the mock
traffic, and make no request to aimock; the exact process budget above is
deliberately the model-turn budget.

The fully hardened gate passed on August 31, 2026: `1 passed, 0 failed`; the
Rust test body completed in 19.27 seconds, with all 12 Claude model-turn
processes, 13 model requests, and 12 native hello probes accounted for.

See [`docs/PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md`](PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md)
for the initial POC evidence, five-revision matrix, final history oracle,
implementation notes, and safety contract.

### Mixed provider E2E and performance test

The default mixed test creates **25 Codex sessions and 15 Claude sessions**
with real provider CLIs and local mock responses. It checks 20 Codex fork
edges, selected-history Push/Pull, native resume, path remapping, credential
redaction, Keep both, and three project paths. The two conflict rounds add
eight branches, for 48 final conversations. All test homes and storage are
temporary.

The original 100-Codex/20-Claude fixture is now **xlarge**. It retains its
80-edge graph and 132 final conversations.

```sh
npm run test:mixed-aimock-harness
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e -- --debug
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e -- --baseline-ref="$BASELINE_REF"
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e-xlarge
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e-xlarge -- --baseline-ref="$BASELINE_REF"
```

Set `BASELINE_REF` to a reviewed full product commit ID. The benchmark builds
both revisions with the same test worker. It alternates their samples against
the same checkpoints. It uses two discarded iterations and at least 20 measured
samples per operation. Without a baseline, it records measurements and exits 2
with status `uncalibrated`. Debug builds cannot produce benchmark results.

Reports are in the ignored `output/mixed-aimock/` directory, or
`output/mixed-aimock-xlarge/` for xlarge. They contain test
counts, mock audits, operation times, worker CPU and peak RSS, work counters,
and regression results. They do not contain transcripts or provider homes.

The periodic workflow requires a dedicated Linux runner with the
`mallard-mixed-aimock` label and the repository variable
`MIXED_AIMOCK_BASELINE_REF`. The 25/15 test runs nightly at 09:00 UTC. The Sunday
11:00 UTC run tests xlarge scales 10, 50, and 100 with one and four comparison
workers. The
execution phase uses a private network namespace with loopback only.

See [the test procedure](MIXED_PROVIDER_AIMOCK_E2E.md) for pins, test steps,
benchmark limits, baseline qualification, and runner setup. See the
[xlarge procedure](MIXED_PROVIDER_AIMOCK_E2E_XLARGE.md) for the larger fixture.

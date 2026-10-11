# Codex Desktop project-placement connection POC

Date: 2026-10-10. Platform: Windows. Connector package: `codex-app-tools` 0.1.6.

Result: **native project placement works in an isolated Windows POC.** An
existing chat with a completed local mock turn moved from projectless Chats to
the intended Desktop project. Membership survived two opens and an isolated
Desktop restart. The same chat ID, name, preview, and complete returned turns
were preserved. The Windows production integration now uses Desktop's own
backend and verifies its profile through Windows process and file ownership;
see the integration section below.

## Working native POC

Run [poc-codex-native-placement.mjs](../scripts/poc-codex-native-placement.mjs)
from a normal shell. It removes inherited Codex executor and connection
variables and reuses the installed app-tools connector. It does not use this
assistant's privileged app-tool connection or require an active model turn.

The successful run passed ten assertions:

| Check | Observed result |
| --- | --- |
| Selected `CODEX_HOME` | Matched App Server's initialization response. |
| Project lookup | Resolved the native Desktop project by its exact root. |
| Existing chat baseline | Native membership was explicitly projectless. |
| Current failing approach | Backend assignment succeeded; opening still left native membership projectless. |
| Native move | `move_thread_to_project` changed Desktop's reported membership to the intended project. |
| First open | Deep link plus native navigation succeeded; membership retained. |
| Second open | Same existing chat and membership retained. |
| Different home | Rejected before any native request. |
| Desktop restart | Native membership retained after restarting only the fixture instance. |
| Conversation preservation | ID, name, preview, and returned turns unchanged. |

A subsequent native read also passed an added connection check: Windows reported
that the isolated Desktop PID had an established TCP connection to the verified
backend PID, and that the native pipe was owned by that Desktop PID. The current
runner includes this as an additional assertion.

### How the connection is bound

1. Create fresh temporary provider-home, Desktop user-data, and project folders.
   Seed one existing project before Desktop starts; seed no chat assignments.
2. Start an isolated App Server on loopback with that `CODEX_HOME`, and create a
   real fixture chat through its API. Complete one turn with the existing local
   aimock dependency; no real model credentials or inference are used.
3. Launch the installed Windows app through its MSIX package context, explicitly
   passing `CODEX_HOME`, `CODEX_ELECTRON_USER_DATA_PATH`, and the fixture backend's
   WebSocket URL. The real running Desktop profile remains open.
4. Verify App Server's home, the Desktop process's isolated user-data argument,
   its live TCP peer, and the native pipe's owning PID. No inherited endpoint is
   trusted. Windows can expose several endpoints owned by that same instance.
5. Start the installed connector with the verified endpoint. Supply the real
   fixture chat ID through its normal MCP `_meta["openai/threadId"]` contract.
6. Resolve the Desktop project, call its native move operation, and read back
   membership with native `list_threads`. Open the same chat and repeat the read.

The backend-only negative control is significant: a successful
`thread/metadata/update` and a successful deep link are insufficient. The native
move updates Desktop's own project assignment, including while
`threadAssignmentsMigrated` remains false. No feature flags were changed, and
no live global-state file was patched to create the result.

### Run the POC

Set these variables to the installed CLI executable, Desktop executable,
app-tools connector `server.mjs`, and Windows package family. The CLI must be
executable outside WindowsApps; the installed relocated CLI is suitable.

```powershell
node scripts/poc-codex-native-placement.mjs --phase=run `
  "--cli=$codexCli" "--executable=$desktopExe" `
  "--connector=$appToolsServer" "--family=$packageFamily"
```

The runner prints `PASS` lines and writes a sanitized `report.json` into its
new temporary directory. Assertion failures exit nonzero. It stops its own
Desktop and backend processes after a full run, checking process identity before
termination. It preserves the temporary fixture/report for inspection and never
terminates a process by application name. Diagnostic phases are also available;
use `--phase=cleanup --root=<printed temporary root>` for a manually started POC.

The first automated attempts tried creating only a backend project after Desktop
startup. Desktop registration was not reliable, so those attempts failed rather
than reporting a false success. The final fixture starts with an existing
Desktop project, matching the existing-project placement problem. Registering
new Desktop projects is a separate unproved capability.

### Boundaries

- The original POC proved native assignment with an external WebSocket backend.
  The Windows integration below separately verifies a normal Desktop-owned
  backend and exercises the production Rust implementation.
- The POC deliberately uses Desktop's existing native tool. Its private endpoint
  and Windows bootstrap controls are version-specific, not a public integration
  guarantee. The Windows adapter explicitly verifies the selected profile,
  process lifetime, pipe ownership, and required tools; unsupported versions
  return an error rather than falling back to backend metadata or file edits.
- Native sidebar membership and navigation were confirmed by Desktop's own
  running tool implementation. Pixel capture returned `FrameArrived timed out`;
  visual placement is not screenshot-verified.
- macOS was not tested. Its signed-peer checks and launch topology need separate
  validation. This integration changes the Windows path only.
- The original POC did not run full Mallard integration/E2E suites. Production
  verification is recorded separately below; no native screenshot is claimed.

## Earlier connection-only diagnostic

## Scope and method

The [probe](../scripts/poc-codex-desktop-project-placement.mjs) starts the installed
app-tools server directly using Node. It reuses the bundled connector instead of
implementing its private desktop pipe protocol. No dependencies were added.

The run supplied an empty temporary directory as `CODEX_HOME` in both modes:

1. Simulate a standalone environment by removing Codex app-tools connection and
   executor variables from the child process.
2. Supply only the inherited desktop endpoint, without a calling chat or turn.

Each child performs MCP initialization and capability discovery. If available,
it requests `list_projects` without executor metadata. The probe never invents
an executor identity and stops at the connector's rejection. Requests have a
10-second process deadline and an 8 MiB total output limit. Reports contain only
capability names, booleans, and bounded error categories.

This is a read-only connector diagnostic, not a native assignment or sidebar
E2E test. The installed desktop was contacted only for capability discovery;
project reading was rejected by the connector before dispatch. No move operation
was sent. The temporary provider directory remained empty and was removed.
No real provider-home files or chat memberships were changed, and no app was
restarted. Product code and the paused test suites were not run or changed.

## Observed results

| Check | Result |
| --- | --- |
| Standalone MCP initialization | Succeeded; this only initializes the connector. |
| Standalone capability discovery | `desktop_endpoint_missing`: Codex did not supply `CODEX_APP_TOOLS_PIPE_PATH`. |
| Capability discovery with inherited endpoint | Advertised `list_projects`, `list_threads`, and `move_thread_to_project`. |
| Project read without executor metadata | `executor_context_required`. |
| Selected home passed to connector | Confirmed for both child processes. |
| Connected desktop profile matches selected home | Unverified; the move tool advertises no home argument. |
| Native assignment and visible sidebar placement | Not attempted or proved. |

Passing `CODEX_HOME` to this connector does not itself establish which profile
its inherited desktop endpoint serves. An advertised move tool is not proof that
a standalone client can invoke it.

The probe exits zero when it completes its observations, including expected
negative results. `automaticPlacementProven` remains false. Unexpected process,
timeout, or protocol failures produce a nonzero exit code; raw provider errors
and response bodies are suppressed.

## Reproduce the connection diagnostic

Set `appToolsServer` to the absolute path of the installed app-tools `server.mjs`
and `temporaryCodexHome` to an existing empty temporary directory. Then run:

```powershell
node scripts/poc-codex-desktop-project-placement.mjs --server $appToolsServer --codex-home $temporaryCodexHome
```

The inherited-endpoint observation is available only when Codex has supplied
that endpoint to the parent process. A normal shell still records the standalone
result and reports that no endpoint was inherited. The script does not scan for
private pipes or read saved conversations.

## Follow-up source investigation

Inspected the installed Windows Desktop package `26.1007.2314.0`, its bundled
JavaScript inside `app.asar`, the installed connector, current official App
Server documentation, and upstream reports. These are source observations,
not a successful native placement test. No installed files were modified.

### Why the current assignment can succeed without placing the chat

Mallard's `providers/codex/project_assignment.rs` confirms the result of
`thread/metadata/update` from a separate App Server process. The installed
Desktop has another membership layer used by its sidebar:

- The notification policy explicitly marks `thread/project/updated` and
  `project/changed` as disabled. The App Server initialization request includes
  that policy in `optOutNotificationMethods`.
- Adoption of project IDs from observed threads requires
  `localProjectTaskMembership` and a completed thread-assignment migration.
  Pending assignments and newly created threads have additional guards.
- The existing-chat deep link only navigates to the chat. It does not consume
  `path` or `projectId` as an assignment request. Those inputs belong to the
  new-chat route. Opening the project route first does not assign an old chat.

This explains why backend readback is an insufficient success condition. It
does not establish why the migration or feature is inactive for every affected
profile. Changing hidden feature flags is not a proposed fix.

A closely matching macOS report describes valid backend assignments, missing
Desktop membership, and an incomplete assignment migration. This supports a
cross-platform synchronization problem; it is a user report, not our macOS
reproduction. See [upstream issue #48728](https://github.com/openai/codex/issues/48728).

### The native operation exists; missing context is not proof of a model requirement

The installed `move_thread_to_project` implementation resolves the actual
sidebar item and invokes the same placement function used by the sidebar.
It validates the host and destination and retains the existing consent flow.
It does more than write backend metadata.

The normal MCP dispatch path checks that the calling thread can be read. Its
execution-claim check deduplicates call IDs; it is not an active-turn check for
this operation. The connector itself can generate request/turn correlation IDs.
These observations do not make its endpoint a supported standalone API, but
they correct the earlier inference that the missing-metadata response alone
requires an active model turn.

There is also a documented `mcpServer/tool/call` App Server operation. Current
upstream code loads the requested thread, checks whether direct input is
allowed, and supplies its real `threadId` in MCP metadata. No model prompt is
part of that request. The installed Desktop client uses this API too. See the
[App Server documentation](https://learn.chatgpt.com/docs/app-server) and
[request processor](https://github.com/openai/codex/blob/main/codex-rs/app-server/src/request_processors/mcp_processor.rs).

The request shape to investigate, using discovered server/project identities
and a real isolated fixture thread, is:

```json
{
  "method": "mcpServer/tool/call",
  "params": {
    "threadId": "<loaded-fixture-thread-id>",
    "server": "<discovered-desktop-app-tools-server>",
    "tool": "move_thread_to_project",
    "arguments": {
      "source": "codex",
      "hostId": "local",
      "threadId": "<same-fixture-thread-id>",
      "projectId": "<desktop-project-id>"
    }
  }
}
```

This is a candidate call, not a tested Mallard connection. It needs a loaded
thread and a configured connector that reaches the intended Desktop instance.
Starting another App Server with the same home does not provide that connection
or inherit Desktop's connector endpoint.

### Routes checked

| Route | What source inspection established | Decision |
| --- | --- | --- |
| Existing-chat deep link with project query parameters | No assignment handler on that route. | Keep it for navigation after placement. |
| Backend metadata update or a shared backend alone | Sidebar adoption is gated and project-change notifications are disabled in this build. | Backend success cannot establish placement. |
| Native app-tools connector | Calls the sidebar move operation, but requires Desktop endpoint discovery and thread context. | Strongest operation to prove in a second POC. |
| Cross-client IPC | Inspected handlers coordinate thread owners, turns, read state, and notifications; no project-move request was found. | No verified placement route. |
| `taskProject/assign` on the native pipe | Validates a ready Work executor environment and the thread's membership in that environment. | Not a general local Codex-chat API. |
| Shared managed daemon | The installed automatic local-daemon branch excludes Windows. A WebSocket override exists, but does not by itself supply the Desktop connector environment. | Too broad to adopt as the smallest fix. |
| Editing Desktop global-state JSON | Desktop keeps in-memory state and writes it back. | Do not implement live file edits or forced restarts. |

Endpoint inheritance and signed-process checks are also reported as failures
when macOS Desktop uses a separately managed daemon. That makes the process
topology part of the experiment, not just the request payload. See
[upstream issue #47425](https://github.com/openai/codex/issues/47425). Do not
disable peer verification or rely on process-chain workarounds.

### Preserving the selected home needs an instance check

Passing `CODEX_HOME` to a child connector changes that child's environment; it
does not retarget the Desktop endpoint. On packaged Windows, a second Desktop
launch normally forwards its arguments to the existing instance. Its new
environment is not a profile-switch operation.

App Server's initialization response includes its effective `codexHome`, and
the installed Desktop connection stores that value as `initializedCodexHome`.
Use this response when connected to Desktop's actual backend to compare its
canonical home with the selected home. A response from a separate backend only
identifies that backend; it does not identify a separately supplied native
Desktop endpoint. See the [initialization response schema](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/typescript/InitializeResponse.ts).

The installed bootstrap supports explicit `CODEX_ELECTRON_USER_DATA_PATH` for
isolated Desktop state. Its shell-environment loader also explicitly preserves
the supplied `CODEX_HOME` when that user-data override is present. These are
useful controls for a future isolated experiment, not proof that the existing
production launcher selects the correct already-running profile. A fixture must
isolate both directories; an empty provider home alone is insufficient.

### Local evidence anchors

These names identify the inspected installed build; they are not dependencies
for production code. Do not copy its bundled implementation into Mallard.

| Bundled file | Anchor |
| --- | --- |
| `.vite/build/src-CP7XXlpZ.js` | Notification policy `ll`, excluded list `dl`, export `bn`. |
| `.vite/build/src-BJv5Mk7n.js` | `optOutNotificationMethods`, `callMcpTool`, cross-client request registry `De`. |
| `.vite/build/main-BklS_2Y2.js` | `observeThreads`, `setThreadAssignmentsEnabled`, `writeThreadAssignment`, native pipe `Rae`/`Vae`, execution claim `h_e`. |
| `.vite/build/bootstrap-487kJHw_.js` | Deep-link parsers `qE`/`JE`, instance policy `iD`, user-data resolver `oD`. |
| `.vite/build/application-network-startup-CfAQmSYg.js` | WebSocket selection `Yo`, local-daemon platform/configuration checks. |
| `.vite/build/startup-requirements-C6B1x9gL.js` | Explicit-home preservation around shell-environment loading. |
| `webview/assets/app-shared-737655e1fb23.js` | Native tool eligibility `Sli`, dispatch `Cli`/`bli`. |
| `webview/assets/execution-e02e1de98e60.js` | Native project move `Gu`, sidebar placement call `Ja`. |

SHA-256 of the inspected main bundle:
`5cf1544463be9497b85b69051aa5ad32320f9ebdca063ec2ab41abb75b6cadf7`.

## Integration decision after the original POC

The native operation is now demonstrated. The smallest production change must
replace backend-only assignment with a connection bound to the selected Desktop
profile, exact native project resolution, the native move, membership readback,
and opening the same chat. It must reject a profile mismatch before mutation.

Establish the production discovery contract before integrating: a Desktop-owned
companion could supply a profile-bound connection, or Codex could expose a narrow
assign-and-open endpoint. MCP supports explicit environment forwarding; see the
[MCP configuration documentation](https://learn.chatgpt.com/docs/extend/mcp).
Do not add backend/file-edit fallbacks or restart the user's running app to force
placement.

### Desktop-owned companion integration attempt (2026-10-10)

An additional isolated Windows experiment used Desktop's own App Server, rather
than the external WebSocket backend. A temporary MCP registration started a
small helper using the runnable Node installed by Desktop. The helper inherited
the native endpoint and `CODEX_HOME`; an authenticated loopback connection
checked that home before sending the selected chat and project folder.

The cold-launch case passed: exact native project lookup, native move, navigation,
and native membership readback all succeeded without a model turn. The installed
`list_threads` tool accepts at most 50 non-pinned results; requesting 100 fails.

This did **not** establish a usable production integration:

- Registering the helper while Desktop was running did not start it. Reopening
  the chat, opening the project route, and opening Connections settings did not
  make that attempt succeed.
- A persistent-service variant started on cold launch, but its listener was
  subsequently unavailable after MCP discovery cancelled the helper service.
  A later placement request failed with `ECONNREFUSED`.
- These checks ran on Windows only. The intended macOS process chain is
  Desktop → its bundled Core → its bundled Node; it has not been tested on macOS.

The attempted production integration was removed. The existing staged launch
changes and selected provider homes were preserved. Only disposable fixture
profiles were changed, and the fixture Desktop was stopped afterward. No native
Mallard end-to-end success is claimed by this experiment.

That Windows attempt left a reliable, profile-bound connection to a running
Desktop unresolved. The Windows integration below avoids the MCP helper
lifecycle. The later macOS integration is described at the end of this document.

## Windows production integration

The Windows adapter replaces backend-only assignment with the native Desktop
move. It uses the existing Tokio and windows-sys dependencies, with no helper
installation, provider configuration edits, or changes to the selected home.

1. Start or focus the registered Desktop without a deep link. A second launch
   can ignore its environment, so launching with `CODEX_HOME` is not proof of
   the running profile.
2. Use Windows [Restart Manager's resource query](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmgetlist)
   to identify processes holding ordinary `state_<version>.sqlite` files in the
   selected home. No shutdown or restart API is called.
3. Require a live Core process whose creation time matches the resource owner,
   whose parent is the registered Desktop executable, and whose parent started
   before it. Keep query handles open to guard against PID reuse. Reject
   multiple matching Desktop instances.
4. Connect to a native pipe only when Windows reports that same Desktop PID as
   its server. Perform `tools/list` before the first tool call; the installed
   Desktop initializes its dispatcher during this read-only handshake. Require
   project listing, native move, and native navigation capabilities.
5. Resolve one exact local project folder, send one native move, validate the
   returned chat/host/project identity, then navigate through the same connection.
   Desktop's native move hydrates old chats; production does not require them to
   appear within `list_threads`' 50-item recency window.

The project must already be registered in Desktop. A different profile, missing
project, ambiguous match, unavailable capability, or unconfirmed operation fails
explicitly. The adapter does not use inherited endpoint variables and never
retries a move. A Desktop using a separately managed/external backend cannot
satisfy this process-parent contract and is refused.

After the original POC creates a populated temporary fixture, run:

```powershell
node scripts/poc-codex-native-placement.mjs --phase=integration --root=<temporary-poc-root>
```

This phase launches isolated Desktop with its own backend, invokes the Rust
production assignment code, and verifies native membership. It reproduces the
backend-only failure, opens the same chat twice, rejects another home containing
a copied chat, checks complete history preservation, and repeats after a fixture
Desktop restart. It saves a sanitized `integration-report.json` and stops only
its recorded fixture processes. The normal app launch helper is covered
separately; this is not a full Mallard UI or screenshot acceptance test.

Validation on the installed Windows Desktop (`26.1007.2314.0`): the production
integration phase passed both runs, including the cold restart. The infrastructure
library suite passed 851 tests, with 8 opt-in tests ignored in that suite; the
native placement test was run explicitly by the integration phase.
The Tauri backend library suite also passed: 571 tests, 0 failures, 15 opt-in
tests ignored. JavaScript syntax, Rust formatting for the placement modules,
and the whitespace diff check passed.

## macOS production integration

The macOS app (`26.1007.30912`, bundled CLI `0.162.0-alpha.17.2`) rejects a direct
Mallard connection with `untrusted-process-ancestry`. A standard MCP server
started by Desktop's own Core can use the native endpoint. The
[upstream report](https://github.com/openai/codex/issues/47425) describes the same
process ancestry restriction.

Mallard registers one `mallard_project_placement` MCP server through Codex's
config API with a version check. Desktop runs its script with the bundled Node. The
script and private connection records stay under the selected Codex home. The
connection checks the canonical home and a random token. It permits project
listing, native membership readback, moving the selected chat, and navigation
to that chat. It exposes no tools to the model.

The adapter uses the existing `assign_with_request` flow. It resolves one exact
local project, sends the move once, checks the reply, and opens the same chat.
It does not edit native membership files or backend project IDs.

Registration precedes the macOS `open -n` launch. A loaded chat does not reliably
reload a new MCP entry. The early prototype replaced the fixture's complete
configuration, which also triggered Desktop's own configuration reconciliation;
that result did not prove that adding one entry to a running app was enough.
The production test preserves the existing configuration and registers before
starting the isolated Desktop.

MCP discovery can close a temporary client while a placement is in progress.
On stdin closure, the helper stops accepting connections and allows an existing
placement connection to finish within 30 seconds. It exits when that connection
closes. SIGTERM and SIGINT stop it immediately. It never detaches from Core. Stale
connection records are removed only after their process is gone and their
content digest still matches.

```sh
node scripts/poc-codex-macos-placement.mjs
```

The native test passed on the installed macOS app. It checked the initial
projectless state, the production move and navigation, repeated opens, and
membership after an isolated restart before another move. It also checked
wrong-home, wrong-token, and wrong-chat rejection, idempotent registration,
configuration preservation, and the original chat history. Desktop appended
only its normal thread-settings events. One local mock response created the
fixture; assignment required no model turn. All fixture processes were closed.
This is native adapter verification, not a full Mallard UI acceptance test.

The infrastructure library suite passed 899 tests, with 11 opt-in tests ignored.
The Tauri backend library suite passed 585 tests, with 15 opt-in tests ignored.
The macOS native test was run explicitly for the initial launch and restart.
JavaScript syntax, Rust formatting for the changed modules, and the whitespace
diff check also passed.

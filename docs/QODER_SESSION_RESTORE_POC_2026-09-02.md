# Qoder Session Restore Proof of Concept

**Date:** 2 September 2026

**Repository:** Mallard

**Result:** `POC_ONLY`

**Production decision:** No-go for Qoder session restore

**Scope:** Qoder session capture, transport, discovery, and restore mechanics

**Out of scope:** Qoder pets, production integration, credentials, settings,
memory, application-database modification, and private Qoder interfaces

**Related research:**
[Qoder support feasibility research](QODER_SUPPORT_FEASIBILITY_RESEARCH_2026-09-01.md)

## Executive result

The proof produced three useful results:

1. Mallard-controlled file mechanics are feasible. A synthetic harness captures
   and restores a complete six-file task group, preserves the source UUID,
   blocks reviewed-state conflicts, and limits path changes to approved JSON
   value tokens. All 15 tests pass.
2. The public Qoder Agent SDK can import the main transcript and subagent
   transcripts into an external `SessionStore`. All 25 offline assertions pass.
   The importer ignores `state.json`, compression state, and `task-*.json`.
3. Qoder CLI `1.1.40` can list one synthetic session from a clean, disposable
   `QODER_CONFIG_DIR`. This proves local discovery only. It does not prove
   resume, sidecar consumption, or desktop-app discovery.

These results do not prove Qoder task restore. Qoder App has a separate
application database and user-interface projections. Qoder does not publish an
app-home selector, task import contract, open-by-task-ID command, or index
rebuild contract. A safe clean-home Qoder App test was not possible on the
current macOS account.

The correct release state is:

| Capability | Result | Release state |
| --- | --- | --- |
| Synthetic consistency-group capture | Pass | PoC evidence only |
| Synthetic exact materialization | Pass | PoC evidence only |
| SDK transcript import and readback | Pass, limited | SDK runtime only |
| Qoder CLI clean-home discovery | Pass, limited | Diagnostic evidence only |
| Qoder CLI authenticated resume | Not run | Blocked pending a disposable test account |
| Qoder App clean-home discovery and continue | Not run | Blocked pending disposable macOS environments |
| Qoder App database and UI projection rebuild | Unproved | Production blocker |
| Production Qoder session restore | No-go | Keep disabled |

The harness publishes this result in machine-readable form:

```json
{
  "productionReady": false,
  "verdict": "POC_ONLY",
  "gates": {
    "activeLeafAuthority": "UNPROVED",
    "qoderAppConsumption": "UNPROVED",
    "qoderApplicationIndexRebuild": "UNPROVED",
    "crossPathIntegrity": "UNPROVED"
  }
}
```

## 1. Question and proof boundary

The proof tests this proposition:

> Mallard can restore a Qoder task when it captures one stable JSONL and
> sidecar consistency group, preserves its UUID, applies only reviewed
> structural changes, and materializes the complete group into a clean Qoder
> environment.

This proposition has two separate parts:

- **Mallard mechanics:** capture, validation, planning, collision handling,
  staging, backup, apply, deletion, and verification.
- **Qoder consumption:** discovery, branch selection, continuation, and
  application-projection rebuild by Qoder.

The local harness can prove the first part with synthetic data. Only a real
Qoder product test in an isolated environment can prove the second part.
Production restore requires both parts to pass.

## 2. Evidence and environment

The proof uses four evidence classes:

| Evidence | Use | Limit |
| --- | --- | --- |
| Official Qoder documentation | Public CLI, SDK, and app contracts | It does not define all local schemas or app import behavior. |
| Installed-product metadata | Product identity and available public entry points | It is one installed version, not a compatibility promise. |
| Synthetic filesystem harness | Mallard-controlled capture and apply mechanics | It does not execute Qoder. |
| Isolated package probes | Public CLI and SDK behavior | They do not authenticate, run a model, or start Qoder App. |

The tested environment was:

- macOS ARM64.
- Qoder App `0.1.4`.
- Qoder bundle identifier `com.qoder.app`.
- Node.js `26.7.0`.
- npm `11.19.0`.
- `@qoder-ai/qoder-agent-sdk` `1.0.32`.
- `@qoder-ai/qodercli` `1.1.40`.

No globally available `qoder` or `qodercli` command existed before the isolated
package probes. The probes did not create a global installation. Qoder App and
Qoder CLI are separate products.

## 3. Public Qoder contract

### 3.1 CLI controls

The [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference)
documents these relevant controls:

```text
qoder --config-dir <directory>
QODER_CONFIG_DIR=<directory> qoder
qoder --cwd <project-directory>
qoder --list-sessions
qoder --resume <session-id>
```

The [Qoder CLI session documentation](https://docs.qoder.com/cli/sessions)
identifies these local files:

```text
<qoder-config>/projects/<processed-project-path>/<session-id>.jsonl
<qoder-config>/projects/<processed-project-path>/<session-id>/state.json
```

The documentation describes the first file as conversation history and the
second file as session state. It does not define:

- the processed-project-path algorithm as a stable compatibility contract;
- every transcript record and sidecar schema;
- a stable sidecar membership set;
- an atomic snapshot protocol;
- `active-leaf` authority;
- UUID collision behavior;
- path-change or signature behavior; or
- compatibility with Qoder App versions.

`--config-dir` and `QODER_CONFIG_DIR` are CLI and SDK contracts. They are not a
documented Qoder App profile selector.

### 3.2 Agent SDK controls

The [Qoder Agent SDK storage documentation](https://docs.qoder.com/cli/sdk/session-storage)
defines an external `SessionStore` and this import helper:

```ts
await importSessionToStore(sessionId, store, {
  dir: "/path/to/project",
  includeSubagents: true,
  batchSize: 500,
});
```

The SDK requires the store to keep entries opaque and in order. The SDK can
load those entries when an SDK query resumes the original session ID. Each host
must use the same working directory.

This is a public cross-runtime transcript mechanism. It is not an API to
materialize a complete local Qoder task. It does not register a task in Qoder
App or rebuild the app database.

### 3.3 Qoder App controls

The [Qoder App task documentation](https://docs.qoder.com/qoder/task-management)
supports these task actions:

- Select an existing task from the sidebar.
- Search task titles and message text.
- Continue, branch, rename, pin, archive, and delete a task.
- Export task history.
- Copy a task ID for feedback or troubleshooting.
- Restore an archived task.

In this context, **restore** means unarchive. The documentation does not define
a task migration or import operation.

[Qoder App Settings](https://docs.qoder.com/qoder/settings) has a **Data
import** entry with the description “Import supported data.” The documentation
does not identify the accepted formats or state that this operation imports
resumable tasks.

No reviewed public app documentation defines:

- an app configuration-home or profile selector;
- a task import API;
- a task-ID open command or deep link;
- a database rebuild command; or
- a task-index refresh operation.

## 4. Safety boundary

The executable proof does not read or write the user's real Qoder session tree.
It does not start Qoder App. It does not read, copy, or edit:

```text
~/.qoder/**
<app-support>/main.sqlite
<app-support>/main.sqlite-wal
<app-support>/main.sqlite-shm
<app-support>/auth.v1.dat
<app-support>/auth.machine-id
<app-support>/Cookies*
<app-support>/Local Storage/**
<app-support>/Session Storage/**
```

All filesystem tests use operating-system temporary directories. The harness
requires both of these controls before it accesses a path:

- The canonical sandbox base name starts with `mallard-qoder-poc-`.
- The sandbox contains the exact `.mallard-qoder-poc-synthetic` marker.

Every source, destination, operation, staging, backup, and journal path must be
below that validated sandbox. The harness has no default Qoder path.

The SDK and CLI probes used separate temporary roots. Each process cleared the
inherited environment and used a disposable `QODER_CONFIG_DIR`. All session
content was synthetic. Package installation disabled lifecycle scripts. The
probes did not authenticate, submit a prompt, contact a model, run a resume
command, use a real Qoder path, or start Qoder App.

The packages were acquired through the user-approved npm registry. Node.js and
npm were already available. Homebrew was not required.

## 5. PoC artifacts

| File | Purpose |
| --- | --- |
| [`../poc/qoder-session-restore/qoder-session-restore.mjs`](../poc/qoder-session-restore/qoder-session-restore.mjs) | Dependency-free synthetic capture, planning, and apply harness |
| [`../poc/qoder-session-restore/qoder-session-restore.test.mjs`](../poc/qoder-session-restore/qoder-session-restore.test.mjs) | Synthetic Node.js integration tests |
| [`../poc/qoder-session-restore/README.md`](../poc/qoder-session-restore/README.md) | Short run guide and safety statement |

The harness is intentionally outside the production Rust and Tauri modules. It
must not be called by Mallard at runtime.

The tested data flow is:

```text
synthetic task group
  -> stable capture
  -> analysis
  -> reviewed plan
  -> verified staging and backup
  -> exact file apply
  -> complete-group recapture
```

## 6. Filesystem harness design

### 6.1 Allowlisted consistency group

The synthetic fixture contains these six members:

```text
<bucket>/<session-id>.jsonl
<bucket>/<session-id>/state.json
<bucket>/<session-id>/compression-v2/state.json
<bucket>/<session-id>/subagents/agent-<kind>-<id>.jsonl
<bucket>/<session-id>/subagents/agent-<kind>-<id>.meta.json
<bucket>/<session-id>/subagents/task-<id>.json
```

The root transcript and `state.json` are required. The other directories are
optional, but every present child must match the tested allowlist. An unknown
sidecar blocks capture.

This membership set is based on the sanitized, read-only observation in the
feasibility report. It is not a Qoder compatibility contract.

### 6.2 Stable capture

The capture flow does this work:

1. Validate the synthetic sandbox and all path components.
2. Enumerate the complete group.
3. Open each file with `O_NOFOLLOW`.
4. Require a regular file.
5. Record device, inode, mode, link count, size, modification time, change
   time, and SHA-256.
6. Read the complete group.
7. Re-enumerate membership.
8. Read the complete group a second time.
9. Re-enumerate membership again.
10. Require identical membership, identity, metadata, size, and digest across
    the reads.

The tests change the group between these passes. Addition, removal, inode
replacement, in-place mutation, and symbolic-link substitution all block.

### 6.3 UUID analysis

The harness requires one canonical UUID across:

- the root transcript file name;
- the task sidecar directory name;
- every `sessionId` field; and
- every `session_id` field.

It reports source-UUID references in opaque fields. It never replaces the
source UUID. A conflicting known UUID field blocks capture.

For a new destination mapping, the plan scans every destination project bucket
for the source UUID. A transcript or session directory collision blocks before
the apply stage. A collision in Mallard's supplied mapping set also blocks.

For an existing mapping, the plan requires the same cloud task, UUID, project
bucket, logical member set, target name, identity, size, and digest. A change
after review blocks apply.

### 6.4 `active-leaf` handling

The harness collects every observed `active-leaf` record and reports:

- file line number;
- candidate leaf UUID;
- timestamp, when present; and
- explicit flag, when present.

The result always states:

```text
authority: UNPROVED
authoritativeCandidate: null
```

The harness does not use file order, timestamp, or the explicit flag to choose
a visible branch. This is a correct fail-closed result. It does not solve the
Qoder semantic gate.

### 6.5 Structural path mapping

Path mapping uses an explicit rule with three parts:

- logical file;
- JSONL record type, when applicable; and
- exact JSON value path.

The harness parses each JSON document to locate string value tokens. It patches
only selected tokens in the original text. It does not serialize the complete
document again. Therefore, unselected keys, whitespace, escaping, and
free-form historical text keep their original bytes.

The default rule maps only:

```text
transcript/main.jsonl
record type: workspace-directories
JSON path: directories[*]
```

The tests add explicit rules for a synthetic runtime `cwd` and a synthetic
state workspace root. These extra rules are test policy, not approved Qoder
production rules.

When an accepted document contains a field with a tested integrity name, such
as `signature`, `hash`, `digest`, `replay-token`, or `server-correlation`, path
mapping blocks. Exact-byte restore can still proceed.

This blocker is a defensive PoC check. It is not proof that the field-name list
covers every Qoder integrity mechanism.

### 6.6 Plan and apply

The restore plan contains:

- a random plan ID;
- an expiry time;
- the source snapshot digest;
- the source session UUID;
- destination roots and project bucket;
- transformed output names, sizes, and SHA-256 values;
- prior destination identity and digest pins for an update;
- exact reviewed obsolete-sidecar deletions; and
- a digest over the plan body.

Apply performs these steps:

1. Recheck the plan digest, expiry, source digest, and destination state.
2. Stage and verify every output.
3. Copy and verify every existing target that can be overwritten or deleted.
4. Write a local operation journal.
5. Replace one file at a time with a same-directory atomic rename.
6. Verify each installed digest.
7. Delete only an exact reviewed obsolete sidecar.
8. Never use recursive provider-file deletion.
9. Recapture the complete destination group.
10. Require the reviewed member set and digests.

The root transcript is installed after its sidecars. This reduces the interval
in which a new transcript exists without its state files. It does not make the
group atomic to Qoder.

## 7. Filesystem test results

Run command:

```sh
node --test poc/qoder-session-restore/qoder-session-restore.test.mjs
```

Result:

```text
tests: 15
passed: 15
failed: 0
cancelled: 0
skipped: 0
```

| Test | Expected safety property | Result |
| --- | --- | --- |
| Invalid synthetic sandbox | Missing marker or unsafe base name blocks before session access | Pass |
| Complete group capture and restore | All six JSON and JSONL members survive and verify | Pass |
| Member addition during capture | Membership race blocks | Pass |
| Member removal during capture | Membership race blocks | Pass |
| In-place mutation during capture | Digest or metadata race blocks | Pass |
| Member replacement during capture | Inode replacement blocks | Pass |
| Sidecar symbolic link | No-follow policy blocks | Pass |
| UUID in another destination bucket | New mapping blocks before apply | Pass |
| Destination change after review | Stale mapped update blocks | Pass |
| UUID in an opaque future record | Hidden reference is reported; remap remains disabled | Pass |
| Structural JSON token mapping | Approved values change; free-form text remains byte-identical | Pass |
| Unproved integrity field | Path mapping blocks | Pass |
| Obsolete sidecar deletion | Exact review is required; only named files are removed | Pass |
| Conflicting UUID fields | Capture blocks | Pass |
| Machine-readable verdict | `productionReady` remains `false` | Pass |

The successful tests prove the harness behavior. They do not validate real
Qoder schemas or Qoder consumption.

## 8. Agent SDK offline probe

### 8.1 Acquisition controls

The isolated probe installed:

```text
@qoder-ai/qoder-agent-sdk 1.0.32
```

The registry reported this package integrity value:

```text
sha512-3EJ3s5CczBTWZyhf22aK6ETJs+w8P8EM5fFNVwC+lBF3P8gapQmT/pY75rGWw3FjAPk5x2HASXnOF4RfwhWgVw==
```

The install used these safety controls:

```text
temporary package root
--ignore-scripts
--no-save
--package-lock=false
separate empty npm user and global configuration files
separate npm cache
```

The install added 95 dependency packages to the temporary root. It did not
modify Mallard dependencies or lockfiles. The SDK manifest has a `postinstall`
script, but the probe did not execute it.

### 8.2 Published contract inspection

The package root exposes these relevant runtime values:

```text
InMemorySessionStore
importSessionToStore
listSessions
getSessionInfo
getSessionMessages
listSubagents
getSubagentMessages
```

The root declarations expose these relevant types:

```text
ImportSessionToStoreOptions
SessionKey
SessionStore
SessionStoreEntry
SessionStoreFlush
```

The package does not contain the conformance suite that its documentation asks
custom stores to run. It publishes no test or example directory. Mallard must
obtain the official suite from its source repository or implement equivalent
contract tests before it uses a production store.

### 8.3 Synthetic importer test

The fixture contained:

- three ordered main JSONL records;
- two ordered subagent JSONL records;
- one subagent metadata object;
- one main `state.json` sentinel;
- one compression-state sentinel; and
- one subagent task-state sentinel.

The process inherited no authentication or Qoder configuration. The test made
no Qoder CLI, model, or network call after package installation.

Result:

```text
assertions: 25
passed: 25
failed: 0
```

The probe confirmed:

- main records keep their order;
- subagent records keep their order;
- subagent metadata becomes an `agent_metadata` store entry;
- `batchSize: 2` produces the expected `2 + 1` append pattern for both main
  and child entries;
- `includeSubagents: false` excludes child transcripts;
- invalid session IDs block;
- the SDK list, info, main-message, subagent-list, and subagent-message readers
  can read the imported in-memory store; and
- the external store keys use the source session UUID.

The strongest negative result is important:

```text
state.json: ignored
compression-v2/state.json: ignored
task-*.json: ignored
```

The SDK import path is therefore a transcript import path. It is not complete
sidecar materialization and it does not prove exact Qoder App task recovery.

### 8.4 SDK conclusion

The proved path is:

```text
Qoder-compatible main and subagent JSONL
  -> importSessionToStore
  -> external SessionStore
  -> offline SDK session readers
```

The unproved path is:

```text
Mallard bundle
  -> clean Qoder home
  -> Qoder CLI resume
  -> Qoder App task and UI restoration
```

Use the SDK only if Mallard later owns Qoder execution through the SDK. Do not
use it as a hidden bridge for the current Qoder App.

## 9. Qoder CLI discovery probe

### 9.1 Package controls

The isolated probe installed `@qoder-ai/qodercli` `1.1.40` into a temporary
package tree with lifecycle scripts disabled. The registry reported:

```text
sha512-pvgHgc3WQ0dCawApq98t3xStPvudRxP/yFo+Ihm8IOM/d92vBrCCWqqqzB1IwJS+/ZYDFuSgyiOIDX/sYbtckA==
```

The install added 11 packages, including the matching macOS ARM64 ripgrep
package. It did not create a global installation.

The package `postinstall` script was inspected and was not run. It invokes a
hidden `configure-path` command that writes an entry under the default Qoder
home and changes user path configuration. This behavior is outside the
temporary package tree. Running this script would have violated the proof
boundary.

The probe invoked the published `bundle/qodercli.js` file directly. It did not
invoke the npm `qoder` dispatcher because that dispatcher can search for and
start the installed desktop product.

### 9.2 Disposable session

The disposable Qoder configuration root contained one synthetic transcript at
the processed key for the disposable project. The session used this ID and
title:

```text
ID: 44444444-4444-4444-8444-444444444444
title: MALLARD_SYNTHETIC_SESSION
```

The probe ran only these commands:

```text
qodercli --version
qodercli --help
qodercli --list-sessions
```

Each command used a cleared environment, a minimal executable path, a
disposable `QODER_CONFIG_DIR`, and the disposable project as its working
directory.

Result:

```text
version exit code: 0
reported version: 1.1.40
help exit code: 0
list exit code: 0
listed sessions: 1
listed title: MALLARD_SYNTHETIC_SESSION
listed UUID: 44444444-4444-4444-8444-444444444444
```

The transcript command did not change the synthetic transcript modification time.

### 9.3 CLI writes and limits

The CLI wrote installation identity, empty authentication transaction state,
machine identity, settings, security resources, logs, telemetry state, and
endpoint and DNS caches under the disposable configuration root. It did not
write a real Qoder profile.

Because endpoint and DNS caches were created, the proof does not claim that
session listing is network-free. It proves only that no prompt, model query,
login command, resume command, or real credential was used.

The session-list output is formatted for humans. The help output does not
advertise a list-specific JSON format. Mallard must not parse this text as a
stable production contract without vendor confirmation or a proved structured
output mode.

This result proves that Qoder CLI can discover a synthetic JSONL transcript by
project key and UUID. It does not prove that the CLI consumes `state.json`,
selects the correct active leaf, or resumes the task.

## 10. Gate matrix

Result terms:

- **Pass — synthetic:** the local harness proves the stated Mallard behavior.
- **Pass — product:** a public Qoder product performed the narrow stated task.
- **Partial:** one layer passes, but a required product contract is absent.
- **Unproved:** no authoritative rule or product result exists.
- **Blocked:** the test cannot run safely in the current environment.

| Gate | Result | Evidence | Production consequence |
| --- | --- | --- | --- |
| Stable multi-file capture | Pass — synthetic | Two reads and three membership views; add, remove, replace, mutate, and symlink tests | Reimplement with Rust handle-relative traversal and limits. |
| Complete sidecar materialization | Pass — synthetic | Six reviewed members stage, apply, and recapture | The real Qoder sidecar set is still version-specific. |
| Source UUID preservation | Pass — synthetic | File names, directories, known fields, plan, and mapping keep one UUID | Do not add UUID remapping. |
| New-mapping collision detection | Partial | Existing transcript or task directory blocks | A process-wide or provider-coordinated uniqueness lock is still required. |
| Existing mapped update | Pass — synthetic | Complete identity, size, and digest pins block a stale target | Integrate Mallard revision, digest, expiry, and approval pins. |
| `active-leaf` discovery | Pass — synthetic | All candidates are reported | Do not select a visible branch. |
| `active-leaf` authority | Unproved | No public Qoder rule | Production branch-aware restore remains blocked. |
| Structural-only path mapping | Partial | Selected tokens change and free-form history does not | Real Qoder field allowlists and cross-path integrity remain unproved. |
| Known integrity-field blocker | Pass — synthetic | Synthetic `signature` blocks path mapping | The name list is not a complete integrity schema. |
| SDK transcript import | Pass — product | 25 offline assertions | Valid only for an SDK-controlled session store. |
| SDK sidecar import | Does not pass | State, compression, and task sidecars are ignored | SDK import cannot replace complete task materialization. |
| Clean Qoder CLI list | Pass — product | CLI `1.1.40` lists one synthetic session | Human output is not a stable production inventory contract. |
| Clean Qoder CLI resume | Unproved | Authentication and model execution were outside this proof | Requires a disposable account and recorded continuation test. |
| Clean Qoder App task discovery | Blocked | No supported app-home selector or disposable macOS environment | Required before production release. |
| Qoder App continuation | Blocked | No isolated task can be opened safely | Required before production release. |
| Qoder App projection rebuild | Blocked | No documented index rebuild or import contract | Required product decision and acceptance test. |

## 11. Why the desktop test did not run

No supported Qoder App profile selector was found. These mechanisms are not
acceptable proof methods:

- A temporary `HOME` value.
- `QODER_CONFIG_DIR` applied to Qoder App.
- An undocumented Electron `--user-data-dir` argument.
- An undocumented `qoder:` or `qoder-app:` URL.
- Direct private app-binary arguments.
- The current user's Qoder profile.

These methods do not prove isolation of application support, cookies, Keychain
access, helper processes, or single-instance routing. They can also route the
test into the existing app process.

A trustworthy Qoder App test needs two disposable macOS virtual machines. Two
disposable macOS user accounts are a lower-isolation alternative. No configured
macOS virtual machine or disposable account was available in this workspace.

The Qoder App was not launched during this proof.

## 12. Required clean-home product test

Run this test when two isolated macOS environments are available:

1. Install the same Qoder App version in source and destination environments.
2. Create the same synthetic project at the same absolute path in both
   environments.
3. Use disposable test accounts if Qoder requires sign-in.
4. Create one synthetic task and one branch in the source.
5. Record the source task UUID and the visible branch.
6. Start the destination before any copy. Confirm that account or cloud sync
   did not already add the task.
7. Inspect **Settings -> Data import**. Use it first only when Qoder documents a
   task-capable format.
8. If Data import is not task-capable, fully quit Qoder and all helper
   processes in both environments.
9. Capture the allowlisted JSONL and sidecar group with the stable group
   protocol.
10. Restore exact bytes to the same project path. Do not map paths in the first
    test.
11. Do not copy settings, authentication, cache, plugin data, installation
    identity, browser state, or SQLite files.
12. Start Qoder normally in the destination.
13. Confirm that the task appears in the expected workspace or folder.
14. Open the task and confirm the expected branch.
15. Add one harmless follow-up message that depends on earlier context.
16. Restart Qoder and confirm that the task remains usable.
17. Record which title, archive, search, sidebar, recap, highlight, image,
    workspace, and worktree projections Qoder rebuilt.
18. Recapture the destination task. Confirm a coherent JSONL and sidecar group.
19. Repeat with an intentional UUID collision. Mallard must block before any
    write.
20. Repeat with a different project path only after the exact-path test passes.

The desktop gate passes only when all these conditions are true:

- The destination control did not contain the task.
- Qoder discovered the restored task without a database copy or edit.
- Qoder selected the expected branch.
- Qoder continued the task with prior context.
- Qoder persisted the continuation after restart.
- Every required UI projection either rebuilt correctly or was explicitly
  removed from Mallard's product promise.
- The source UUID remained unchanged.
- A collision caused no destination write.
- The test passed on every supported Qoder version and operating system.

## 13. Production gaps

### 13.1 Qoder contract gaps

Production remains blocked until Qoder or a disposable product test defines:

1. Supported task import or task registration.
2. Complete task membership and sidecar versioning.
3. Project-key encoding and path-change behavior.
4. The authoritative `active-leaf` rule.
5. UUID collision behavior.
6. Signature, hash, replay, and server-correlation behavior.
7. Application-database and sidebar rebuild behavior.
8. Compatibility between Qoder CLI, Agent SDK, and Qoder App.

Ask Qoder for a documented import or registration interface. Ask for written
confirmation that Mallard can read, store, path-map, and restore user-owned
local task files.

### 13.2 Mallard engineering gaps

The Node.js harness shows policy and data flow. Production Rust code still
needs:

- Handle-relative traversal, such as `openat`, to close intermediate-directory
  replacement races.
- Bounded file, byte, line, record, nesting, and directory counts.
- A provider-version and transcript-dialect gate.
- A provider-owned exact member classifier.
- A destination-wide UUID lock or equivalent serialization.
- Mallard's normal revision, digest, expiry, trust, and explicit-approval
  checks.
- Durable file and parent-directory synchronization.
- Crash recovery that can continue or roll back an incomplete journal.
- A defined response to failure during final complete-group verification.
- An exact structural-field schema from Qoder.
- A complete integrity-field policy. A field-name denylist is insufficient.
- Provider-closed post-apply verification.
- Sanitized receipts and activity records with no local absolute paths.

File-level rename is atomic. The complete task group is not atomic. Qoder must
be stopped or must provide a snapshot/import transaction while apply runs.

The PoC checks for UUID collisions during planning and before apply. It does not
provide a global lock against another process that creates the same UUID during
the apply window.

The PoC journal records progress and supports in-process rollback for applied
files. It does not implement restart recovery after a process crash. It is not
a production transaction engine.

## 14. Recommended Mallard position

Keep Qoder portable-session capability set to `false`.

Do not merge the PoC harness into production modules. Use it as executable
evidence for the future Rust design.

Proceed in this order:

1. Obtain a documented Qoder app import or index-rebuild contract.
2. Prepare two disposable macOS environments and test the exact-path flow.
3. Prove `active-leaf` authority with a controlled branch fixture.
4. Prove the real sidecar membership set across supported Qoder versions.
5. Prove clean-home list, open, continue, restart, and recapture.
6. Prove UUID collision handling under concurrent destination activity.
7. Run a separate cross-path test and block mapping unless every structural and
   integrity rule is authoritative.
8. Reimplement the passed mechanics with Mallard's Rust safety primitives.
9. Add provider integration only after all external gates pass.

The SDK external store remains a valid future option only for a product mode in
which Mallard starts and owns Qoder SDK sessions. It does not make current
Qoder App sessions portable.

## 15. Reproduction and verification

The repository harness has no package dependency:

```sh
node --check poc/qoder-session-restore/qoder-session-restore.mjs
node --check poc/qoder-session-restore/qoder-session-restore.test.mjs
node --test poc/qoder-session-restore/qoder-session-restore.test.mjs
```

The tested result is 15 passed and 0 failed.

The SDK probe used an isolated temporary install with this command shape:

```sh
export NPM_CONFIG_REGISTRY="<approved-registry>"
npm install \
  --ignore-scripts \
  --no-save \
  --package-lock=false \
  @qoder-ai/qoder-agent-sdk@1.0.32 \
  zod
```

The CLI discovery probe used the same controls for
`@qoder-ai/qodercli@1.1.40`. It invoked the package's `bundle/qodercli.js`
directly. It did not run `postinstall` or the desktop-aware npm dispatcher.

The actual probes also supplied empty temporary npm user and global
configuration files and a separate temporary cache. Do not use a real Qoder
configuration directory for these tests.

No full Mallard build was required because the PoC adds no production source,
Tauri command, Rust type, TypeScript type, schema, or dependency.

## 16. Official sources

- [Qoder CLI session management](https://docs.qoder.com/cli/sessions)
- [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference)
- [Qoder CLI configuration reference](https://docs.qoder.com/cli/settings-reference)
- [Qoder CLI commands](https://docs.qoder.com/cli/commands)
- [Qoder Agent SDK session control](https://docs.qoder.com/cli/sdk/session-control)
- [Qoder Agent SDK external session storage](https://docs.qoder.com/cli/sdk/session-storage)
- [Qoder App task management](https://docs.qoder.com/qoder/task-management)
- [Qoder App system settings](https://docs.qoder.com/qoder/settings)
- [Qoder release notes](https://docs.qoder.com/release-notes/qoder)

## Final decision

The PoC validates a useful set of Mallard filesystem mechanics. It also
confirms that the SDK import path is narrower than a complete Qoder task. Qoder
CLI can discover a restored synthetic transcript, but resume and sidecar use
remain unproved. The remaining failures are product-contract and
clean-environment failures, not small implementation gaps.

Production Qoder session restore is not ready. Keep it disabled until a clean
Qoder App can discover, open, continue, restart, and recapture a restored task
without any application-database copy or edit.

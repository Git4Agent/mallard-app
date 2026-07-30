# Plan: Replace the Codex Expect Fork Helper with `portable-pty`

## Status

Implemented. Native Windows runtime acceptance remains pending.

- Date: July 29, 2026
- Parent design:
  [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
- Scope: preference 2 of the Codex-native fork coordinator
- Compatibility: clean cutover; no migration, compatibility decoder, legacy
  executor, or feature flag
- Test assumption: the owner will clear `~/.mallard` and the designated test
  cloud buckets before acceptance testing

This plan changes the PTY implementation only. It preserves the approved fork
order and Pull transaction:

```text
1. app-server thread/fork over stdio
2. native Rust PTY around codex fork
3. verified manual fork
```

It does not change the recommended keep-both result, cloud behavior, fork
verification, naming policy, or recovery rules.

## Outcome

Mallard will run the interactive Codex fork command inside a pseudo-terminal
owned directly by the Tauri Rust backend:

```text
codex fork --no-alt-screen -C <project-directory> <source-thread-id>
```

The backend will use the `portable-pty` crate rather than materializing and
running `scripts/codex-fork-and-exit` through `/usr/bin/expect`.

The result is:

- no installed Expect, Tcl, Python, Node, `script`, `unbuffer`, or WSL
  dependency;
- one in-process implementation for macOS, Linux, and supported native
  Windows;
- Unix PTY on macOS/Linux and Windows ConPTY through the crate's native
  backend;
- no executable helper extracted from the application bundle;
- no helper hash or helper path in reviewed plans and recovery state;
- no parsing, retained buffering, logging, or pattern matching of Codex TUI
  content; and
- one authoritative Rust path for child discovery, ancestry verification,
  ambiguity handling, termination, and recovery.

`portable-pty` is a build dependency compiled into Mallard. It does not need
to be installed on the user's machine.

## Verified constraints

This plan is based on the following current behavior:

1. `codex fork` is a stable top-level CLI command, but it enters the
   interactive Codex TUI.
2. `--no-alt-screen` changes TUI rendering only; it does not make the command
   headless.
3. The installed `codex-cli 0.145.0` has no `codex exec fork`; its
   non-interactive `exec` surface exposes `resume` and `review`.
4. Codex documents `/exit` and Ctrl+C as ways to close an idle CLI session.
5. App-server `thread/fork` remains the preferred machine-readable fork
   operation.
6. App-server `process/spawn` can own a PTY, but it is experimental and shares
   the same app-server availability and failure domain as preference 1. It is
   not an independent preference-2 fallback.
7. `portable-pty` exposes argv-based spawning, working-directory and
   environment configuration, PTY readers and writers, non-blocking child
   status checks, waiting, and termination.
8. `portable-pty::native_pty_system()` selects the Unix PTY implementation on
   Unix and `ConPtySystem` on Windows.
9. Microsoft ConPTY requires Windows 10 version 1809 or newer, or Windows
   Server 2019 or newer.
10. PTY reads are blocking, so the implementation must continuously drain
    output on a dedicated worker thread.

The current prototype also establishes two useful facts:

- Codex persists the native fork before Mallard needs to close the idle TUI.
- The Rust coordinator already performs the authoritative post-execution
  child scan and verification. It does not need the script to identify a
  candidate by parsing raw JSONL text.

## Why the Expect prototype is removed

The prototype was appropriate for proving that a one-command CLI fork could
be automated, but its responsibilities now overlap the Rust coordinator:

- the fixed `/usr/bin/expect` path is a platform/runtime dependency;
- the Tcl helper scans only the active `sessions` tree;
- it globs the full tree and regexes the first JSONL line rather than using
  Mallard's bounded structural rollout parser;
- it returns the first matching rollout instead of proving an exact delta;
- TUI error patterns can inspect conversation text rendered by Codex;
- any `codex login status` failure is reported as an authentication failure;
- the Rust caller does not consume the emitted `forked_thread_id`; it scans
  native children again;
- killing the outer Expect process on timeout can bypass its cleanup and
  leave the spawned Codex process uncertain; and
- helper extraction, permissions, hashing, and release packaging add
  implementation surface without strengthening the final proof.

The production executor should therefore own only PTY lifecycle. Existing
Rust code should remain responsible for provider data and identity.

## Scope

### In scope

- Add and lock `portable-pty`.
- Replace the Expect-backed preference-2 executor with an in-process Rust
  executor.
- Support native macOS, Linux, and Windows through
  `portable_pty::native_pty_system()`.
- Drain and discard TUI output without retaining conversation bytes.
- Send the documented idle exit command, then apply bounded termination
  escalation.
- Reuse the existing source pins, native-child inventory, structural rollout
  scanner, exact-delta verifier, naming path, journal, and recovery policy.
- Remove all helper artifact, helper hash, materialization, and POSIX-only
  capability code.
- Rename the executor and UI terminology from `PTY helper` to `native PTY`.
- Add deterministic fake-CLI PTY tests and opt-in real-Codex smoke tests using
  disposable homes.

### Out of scope

- Changing preference 1 from app-server `thread/fork`.
- Calling experimental app-server `process/spawn` as another fallback.
- Adding a prompt or starting a model turn in the fork.
- Parsing Ratatui screen output or depending on TUI text.
- Building a general-purpose terminal emulator.
- Displaying the hidden TUI to the user.
- Copying or synthesizing Codex rollout files to create a fork.
- Editing Codex SQLite or `session_index.jsonl` directly.
- Using WSL for a native Windows profile.
- Renaming the fork through TUI keystrokes.
- Keeping the Expect executor as a compatibility fallback.
- Changing portable bundle schema 1.

## Native PTY contract

### Inputs

The native PTY executor receives the same reviewed inputs as the other fork
executors:

- canonical pinned Codex executable;
- reviewed `CODEX_HOME`;
- reviewed project directory;
- source provider thread ID;
- source rollout SHA-256, lineage digest, and semantic digest;
- bounded fingerprints of every pre-existing direct native child;
- desired fork name for the later naming step; and
- operation deadline and termination grace periods.

The PTY executor does not receive a shell command string. It constructs an
argument vector from validated values.

### Preconditions

Immediately before creating the PTY, Mallard must:

1. Revalidate the selected Codex executable and its reviewed fingerprint.
2. Revalidate the source rollout and title expectation.
3. Prove the source is idle and unchanged.
4. Re-scan and compare the complete bounded direct-child inventory.
5. Hold the coordinator's single-flight guard for the affected
   `(CODEX_HOME, source-thread-id)`.
6. Confirm the project directory and `CODEX_HOME` are canonical directories
   and are the same values reviewed by the user.
7. Run a bounded `codex login status` preflight with the pinned executable and
   reviewed `CODEX_HOME`. Report a nonzero status as a failed prerequisite,
   not as proof of one specific authentication cause, and do not log its
   output verbatim.
8. Perform a side-effect-free `openpty` capability probe as part of the real
   invocation. Failure before spawning Codex is a definitive no-fork result
   after one complete child rescan.

Planning may advertise native PTY on compiled Unix and Windows targets.
Runtime `openpty` remains authoritative because Windows versions and host
policy can make ConPTY unavailable.

### Command construction

Use `portable_pty::CommandBuilder` directly:

```text
program: <canonical pinned codex executable>
argv:
  fork
  --no-alt-screen
  -C
  <reviewed project directory>
  <source provider thread ID>
cwd:
  <reviewed project directory>
environment overrides:
  CODEX_HOME=<reviewed Codex home>
  TERM=xterm-256color  # Unix only when inherited TERM is absent or dumb
```

Requirements:

- never invoke `sh -c`, `cmd.exe /C`, PowerShell, or another shell;
- never interpolate a path or ID into one command-line string;
- never pass an initial prompt;
- use the exact pinned executable rather than resolving `codex` from a
  mutable `PATH` at Apply time;
- preserve the normal inherited environment except for reviewed, explicit
  overrides; and
- do not expose credentials or environment contents in activity logs.

### Spawn and output drain

1. Open a PTY at a fixed bounded size such as 40 rows by 120 columns.
2. Obtain the master reader and the single master writer.
3. Spawn the Codex command on the slave.
4. Drop Mallard's slave handle immediately after a successful spawn.
5. Start one dedicated blocking reader thread.
6. Read into a fixed-size byte buffer until EOF and discard every byte.
7. Retain only a saturating byte count and terminal reader status for
   diagnostics; retain no bytes, lines, screen cells, or excerpts.
8. Never send the drain stream to stdout, stderr, tracing, activity history,
   panic text, or a receipt.

The reader must continue draining even after a diagnostic threshold is
crossed. Stopping at an output cap could fill the PTY buffer and deadlock the
Codex TUI. Runtime, not retained output, is the resource bound.

The PTY worker must run off the Tauri command/UI thread. A blocking Rust worker
or `spawn_blocking` boundary owns the session until the child is reaped and
the reader reaches EOF.

### Candidate observation

The PTY executor never reads a thread ID from terminal output.

While the child is alive, the coordinator polls the existing bounded
structural scanner. A candidate is interesting only when it is:

- absent from the pinned pre-existing direct-child map;
- a direct native child of the reviewed source;
- inside the canonical active or archived tree;
- structurally readable;
- semantically and linearly identical to the reviewed local branch; and
- the only new direct child.

Seeing a rollout path is not success. The common verifier remains the sole
acceptance authority.

Before requesting TUI exit, observe a structurally complete candidate and a
short bounded stability interval. The final post-exit scan is still required;
the pre-exit observation only establishes that it is safe to close the idle
TUI.

### Graceful exit and escalation

After one exact candidate is structurally observable:

1. Wait a short bounded composer-readiness grace period.
2. Write `/exit\r` through the PTY master writer.
3. Poll `try_wait` for a bounded graceful-exit interval.
4. If still running, write byte `0x03` to represent Ctrl+C.
5. Poll for a second bounded interval.
6. If still running, call the PTY child termination handle.
7. Always wait/reap the child.
8. Close the writer/master handles.
9. Join the reader thread after the PTY reaches EOF.
10. Run the complete source, child, and inventory verification again.

The primary success condition is the verified native child, not whether
`/exit` returned exit code zero. A forced termination after a durable exact
fork may still be accepted only when the final common verifier succeeds and
the source is unchanged.

If platform testing finds that `portable-pty` termination can leave a Codex
descendant, add a narrowly scoped Unix process-group or Windows Job Object
cleanup adapter. Do not assume process-tree cleanup without a contract test.
At this point `codex fork` is idle and must not be allowed to start agent shell
commands.

## Outcome classification

The executor must preserve the coordinator's no-duplicate fallback rule.

| Observation | Result |
| --- | --- |
| PTY creation or Codex spawn fails before invocation; complete scan shows no new child | definitive no-fork; manual may follow |
| Child exits before creating a child; complete scan shows no new child | definitive no-fork; manual may follow |
| Exactly one new child passes the common verifier | verified fork |
| Exactly one new child exists but fails ancestry/content/path verification | ambiguous; recovery required |
| More than one new direct child exists | ambiguous; recovery required |
| Deadline or forced termination occurs and final scan finds one exact child | verified fork |
| Deadline or forced termination occurs and final scan finds none | ambiguous; recovery required |
| Reader, writer, wait, or cleanup state is uncertain after spawn | rescan; accept only one exact child, otherwise recovery required |
| Source or a pre-existing child changes during execution | ambiguous; recovery required |

Mallard never invokes the manual path after an outcome that may already have
created a fork.

## Data-model cutover

Because there are no deployed users or state to preserve, replace the current
representation directly:

### Rename

```text
CodexForkExecutor::PtyHelper  -> CodexForkExecutor::NativePty
"pty_helper"                 -> "native_pty"
"PTY helper"                 -> "native PTY"
```

Do not add serde aliases for `pty_helper`.

### Remove

- `PTY_HELPER_BYTES`;
- `scripts/codex-fork-and-exit`;
- `materialize_pty_helper`;
- `pty_helper_sha256`;
- helper-directory inputs used only for extraction;
- `/usr/bin/expect` capability checks;
- `#[cfg(unix)]` exclusion of preference 2;
- helper stdout parsing and size limits;
- helper file permission and fingerprint validation;
- helper release-resource or sidecar assumptions; and
- tests that expect Windows to skip directly to manual solely because it is
  Windows.

Remove `pty_helper_sha256` from:

- divergence review DTOs;
- machine-local plan actions;
- journal records;
- execution requests;
- stale-plan comparison;
- `src/types.ts`; and
- frontend fixtures.

No replacement helper fingerprint is needed. The implementation is part of
the running Mallard binary, while the external Codex executable remains
explicitly fingerprinted.

### Keep

- `CodexForkExecutor::AppServerStdio`;
- `CodexForkExecutor::Manual`;
- ordered capability validation;
- Codex executable fingerprint;
- pre-existing direct-child fingerprints;
- source and cloud pins;
- exact common candidate verification;
- executor-attempt journaling;
- ambiguity and recovery rules;
- short-lived app-server naming after a PTY fork; and
- optional App restart after `LocalCommitted`.

## Dependency and packaging

Add `portable-pty = "0.9"` to `src-tauri/Cargo.toml` with no optional features
and commit the resulting `src-tauri/Cargo.lock` update. The lockfile pins the
resolved dependency graph used for release builds.

Before implementation is accepted:

- verify the crate's MIT license is compatible with Mallard's Apache-2.0
  distribution;
- run the repository's normal dependency/license review, if configured;
- compile every advertised release target;
- verify the macOS application and DMG contain no extracted helper;
- verify Windows packaging contains no Expect/Tcl runtime or WSL assumption;
  and
- keep the Codex CLI as a user-installed executable rather than bundling it.

`portable-pty` uses OS PTY facilities. It is library code, not a Tauri sidecar
or application resource.

## Code changes

| File or area | Change |
| --- | --- |
| `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` | add and lock `portable-pty` |
| `codex_native_fork.rs` | replace script materialization and `Command::new("/usr/bin/expect")` with the native PTY lifecycle |
| `codex_native_fork.rs` | add a small testable PTY session/spawner boundary, drain worker, exit escalation, and final-rescan classification |
| `domain.rs` | rename `PtyHelper` to `NativePty`; delete `pty_helper_sha256` from plan and journal DTOs |
| `commands.rs` | stop building helper paths and hashes; pass the existing Codex/source pins to the native executor |
| `conversation_materialization.rs` | remove helper fingerprint comparisons and request plumbing |
| `src/types.ts` | mirror the clean Rust DTO and executor-value cutover |
| Pull review fixtures and models | replace `pty_helper` values and remove helper hash fields |
| `RestorePlanView.tsx` | display `native PTY` and cross-platform availability |
| `scripts/codex-fork-and-exit` | delete |
| release tests/config | prove no helper or Expect runtime is packaged |
| divergence design docs | replace the POSIX-helper/Windows-deferral design with this native PTY plan |

Keep filesystem parsing and verification out of the PTY abstraction. The PTY
layer should expose only spawn, writer, reader-drain status, child status,
termination, and wait operations.

## Test plan

All tests use temporary provider and Mallard homes. They must never read,
write, copy authentication from, or execute against a real `~/.codex`,
`~/.mallard`, or production bucket.

### Pure coordinator tests

- Executor order remains app-server, native PTY, manual.
- App-server success never opens a PTY.
- Definitive app-server no-fork invokes native PTY.
- Ambiguous app-server result never invokes native PTY.
- Definitive native-PTY no-fork enters manual wait.
- Ambiguous native-PTY result never enters manual wait.
- Exact child after a forced PTY termination is adopted once.
- Recovery after a persisted exact child never spawns another PTY.
- Changed source or pre-existing child makes the result ambiguous.

### Command construction tests

- The exact pinned executable is argv element zero.
- Arguments exactly match the reviewed source and project.
- No prompt argument is present.
- No shell is invoked.
- `CODEX_HOME` and cwd match reviewed canonical values.
- Paths containing spaces and non-ASCII characters remain single argv
  elements on Unix and Windows.
- Environment diagnostics never serialize credentials.

### Deterministic fake-Codex PTY tests

Use a test subprocess executable, not a shell script. The subprocess should:

- assert that stdin/stdout are attached to a terminal;
- accept the same `fork` argv shape;
- write a structurally valid fixture rollout into a disposable
  `CODEX_HOME`;
- optionally emit arbitrary terminal bytes, including text resembling source
  conversation and error messages;
- wait for `/exit`, Ctrl+C, or forced termination; and
- expose controlled early-exit, hang, wrong-parent, changed-content, and
  multiple-child modes.

Run cases for:

- exact fork plus graceful `/exit`;
- `/exit` ignored, Ctrl+C succeeds;
- `/exit` and Ctrl+C ignored, forced termination and exact fork;
- forced termination with no child, producing ambiguity;
- early child exit with no fork, producing definitive no-fork;
- large continuous TUI output without memory growth or pipe deadlock;
- reader EOF and reader error;
- wrong parent;
- changed semantic digest;
- two new children;
- source mutation during execution;
- existing child mutation during execution;
- no retained output bytes in error, activity, receipt, or panic text;
- child reaped and PTY reader joined on every branch; and
- a second invocation after recovery does not create another child.

The same fake executable strategy must run on macOS, Linux, and Windows CI.
On Windows, it proves that native ConPTY is used and that no WSL executable is
launched.

### Real Codex smoke tests

Keep real-Codex tests opt-in and outside the default suite. Require an
explicit disposable `CODEX_HOME` configured for the smoke environment, and
reject the normal user home.

Each advertised platform verifies:

- `codex fork` receives a real PTY;
- one child is persisted;
- the child ID is new;
- `forked_from_id` equals the source;
- messages, lineage, and semantics exactly match;
- no prompt, response, or interruption marker is added;
- `/exit` closes the idle TUI or bounded escalation reaps it;
- no Codex process remains after the test;
- the planned name is applied later through app-server; and
- only the disposable directory is changed.

Native Windows smoke support requires Windows 10 1809+/Server 2019+.

### Handoff checks

Run:

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:release-config
```

Also run native release builds on macOS and Windows. Linux requires at least
the fake-Codex PTY integration test and release-target compilation.

## Implementation order

1. Add `portable-pty` and a minimal internal PTY lifecycle abstraction.
2. Add the cross-platform fake-Codex subprocess and failing lifecycle tests.
3. Implement argv/env/cwd construction and PTY spawn.
4. Implement the dedicated drain thread with zero retained output.
5. Integrate existing child scans and the common verifier.
6. Implement `/exit`, Ctrl+C, terminate, wait, reader join, and final rescan.
7. Apply the clean DTO and enum cutover to `NativePty`.
8. Delete helper extraction, helper hashes, `/usr/bin/expect`, and the Tcl
   script.
9. Update frontend labels, fixtures, the parent design, and the readable
   design.
10. Run unit, integration, release, and platform matrix checks.
11. Run opt-in real-Codex smoke tests only against disposable provider homes.

Do not leave a phase where both Expect and native PTY are production
executors. Intermediate commits may use compiler errors to drive the DTO
cutover, but the finished tree contains one preference-2 implementation.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| PTY output blocks Codex | dedicated blocking reader drains continuously to a fixed buffer and discards |
| Conversation text leaks through diagnostics | never retain or parse PTY bytes; test with sentinel content |
| TUI text changes across Codex versions | filesystem structure and common verifier are authoritative; no screen matching |
| Exit is sent before the composer is ready | wait for a complete stable child plus a bounded readiness grace; final rescan |
| Codex persists a fork but Mallard loses lifecycle state | final child scan and existing journal recovery adopt exactly one exact child |
| Timeout produces a duplicate fallback fork | timeout with no exact proof is ambiguous; never proceed to manual |
| Windows lacks ConPTY | runtime `openpty` fails before spawn, complete scan proves no fork, then manual path |
| Windows process cleanup differs | cross-platform fake test plus real smoke; add Job Object adapter only if proven necessary |
| Unix process cleanup leaves descendants | close PTY, terminate/wait direct child, contract-test process table; add process-group adapter if needed |
| Blocking worker freezes Tauri UI | run complete PTY ownership on a blocking worker and emit bounded progress |
| Dependency or release regression | Cargo lock, license review, target compilation, macOS/Windows package inspection |
| Codex later adds a documented headless fork | re-evaluate preference 2 and delete PTY when a stable equivalent is available |

## Acceptance criteria

1. The ordered fork policy is app-server, native PTY, verified manual.
2. Preference 2 has no runtime dependency beyond the installed native Codex
   CLI and OS PTY support.
3. Native PTY works on macOS, Linux, and supported Windows without WSL.
4. Mallard never retains, logs, parses, or pattern-matches Codex TUI output.
5. Mallard never submits a prompt or starts a turn while creating the fork.
6. Exactly one structurally and semantically verified direct child is required
   before the Pull transaction can replace the original thread.
7. A possibly successful PTY attempt never falls through to manual.
8. Every process path closes the PTY, reaps the Codex child, and joins the
   drain worker.
9. The final package contains no Tcl/Expect helper or extracted executable
   resource.
10. `PtyHelper`, `pty_helper`, `pty_helper_sha256`,
    `PTY_HELPER_BYTES`, `/usr/bin/expect`, and
    `scripts/codex-fork-and-exit` have no production references.
11. No migration, compatibility alias, legacy executor, feature flag, or dual
    reader/writer remains.
12. Bundle schema 1 and cloud state are unchanged.
13. Automated tests never touch real Mallard, Codex, or cloud state.

## References

- Codex app-server protocol, including `thread/fork`:
  <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>
- Codex CLI command reference, including `codex fork`, `--no-alt-screen`,
  `/exit`, and Ctrl+C:
  <https://learn.chatgpt.com/docs/developer-commands?surface=cli>
- Current Codex non-interactive command enum:
  <https://github.com/openai/codex/blob/main/codex-rs/exec/src/cli.rs>
- `portable-pty` API:
  <https://docs.rs/portable-pty/latest/portable_pty/>
- `portable-pty` native Unix/Windows selection:
  <https://github.com/wezterm/wezterm/blob/main/pty/src/lib.rs>
- Microsoft `CreatePseudoConsole` requirements:
  <https://learn.microsoft.com/en-us/windows/console/createpseudoconsole>

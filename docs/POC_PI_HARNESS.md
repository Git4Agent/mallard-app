# Pi harness feasibility POC

Test date: 2026-10-15. Scope: **experiment only**.

**Pi session portability is feasible. Adding Pi to the registry alone is not
sufficient for production support.** The experiment captures a session through
Mallard's real provider pipeline, relocates it using Mallard's path mapper, and
opens the result with Pi's own session manager. Production publication, Pull,
conflict handling, UI, and Library support remain unimplemented.

The target is the Pi coding agent from [earendil-works/pi](https://github.com/earendil-works/pi),
tested with `@earendil-works/pi-coding-agent@1.3.0` and session schema 3.
No older session formats or migrations are implemented.

## Reproduce

From the repository root, with the existing Rust toolchain, Node.js 22.19+,
and npm available:

```sh
# Offline fixture and boundary checks, once Rust dependencies are available.
cargo test -p mallard-infrastructure --lib pi_poc

# Native Pi -> Mallard -> native Pi experiment.
node scripts/run-pi-harness-poc.mjs
```

The runner downloads the pinned Pi package into a fresh temporary directory
with npm lifecycle scripts disabled. It creates synthetic sessions in isolated
homes, invokes one otherwise ignored Rust test, checks the result with Pi,
and removes its temporary files. Package installation requires network access;
there are no model requests, API credentials, or live provider homes involved.
The Rust test uses explicitly supplied fixture roots. Node child processes
receive an environment allowlist with temporary homes.

The only existing source-file change is a `#[cfg(test)]` module declaration in
`crates/mallard-infrastructure/src/providers/mod.rs`. The experimental adapter
lives in `pi_poc.rs`, never enters `AgentProviderRegistry::production()`, and
does not ship in the application. No application dependencies, schemas, DTOs,
commands, settings, or UI are changed.

## Observed results

The native runner passed with Pi 1.3.0 and Node.js 24.14.0 on macOS:

| Check | Result |
| --- | --- |
| Register an adapter using the existing trait | Passed, in a test registry only |
| Discover by the session header's exact project directory | One selected session; one other project's session excluded |
| Capture through `discover_full` and `capture_from_discovery` | One session file captured |
| Relocate the project directory and paths inside session records | Passed using `SessionPathMapper` |
| Find the installed session using native `SessionManager.list` | Passed in Pi's destination project bucket |
| Preserve entries and the complete tree | All 17 entries preserved, including the abandoned branch |
| Rebuild model context using native `buildSessionContext` | Equal to the source context with fixture project paths relocated |
| Preserve compaction, context edits, tool calls/results, thinking, labels, title, and extension data | Passed |
| Navigate to the abandoned branch | Passed |
| Append a user/assistant pair and reopen | Passed; new records persisted with the expected parent |
| Leave the source transcript intact | Byte-identical after the experiment |
| Production Push/Pull, remote storage, actual model turn, TUI | Not exercised |

The five non-ignored Rust tests also pass. They cover byte-identical capture,
exact ownership (including exclusion of project subdirectories), exclusion of
auth/settings/project configuration from this session-only capture, invalid
IDs and ancestry, unsupported schema versions, truncated records, byte/count
bounds, unsafe targets, linked-fork refusal, and symlinks on Unix.

Verification completed:

- `node scripts/run-pi-harness-poc.mjs`: passed the native round trip.
- `cargo test -p mallard-infrastructure --lib --quiet`: 918 passed, 12 ignored.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib --quiet`: 586 passed,
  15 ignored.
- Rust formatting, Node syntax, and `git diff --check`: passed.

The native round-trip Rust test is one of the infrastructure suite's ignored
tests; the Node runner executes it explicitly. No frontend or Tauri UI was
changed, so browser/UI verification was outside this POC.

The experiment bounds a file to 2 MiB, a parsed session to 4,096 records, and a
discovery walk to 4,096 entries at depth two. These are POC limits, not proposed
product defaults. The parser validates structural identity and parentage; it
is not a complete semantic validator for every Pi entry or extension payload.

“Continuation” here means Pi's session manager appended and reloaded native
records. It does not mean a CLI or LLM completed a turn. Both simulated machines
are disposable directories on one macOS host; cross-OS continuation is untested.

## Format and integration findings

Pi's default agent home is `~/.pi/agent`, configurable with
`PI_CODING_AGENT_DIR`. Sessions normally sit under
`sessions/--<encoded-working-directory>--/`. The encoding is lossy, so the POC
checks the header's canonical `cwd` instead of trusting a directory name.
The CLI also supports `--session-dir`, `PI_CODING_AGENT_SESSION_DIR`, and a
`sessionDir` setting. This experiment only scans `sessions/` inside its bound
agent home. External roots would need explicit binding and validation; settings
must not silently grant access outside the home.
Sources: [Pi configuration](https://pi.dev/docs/latest/configuration),
[settings](https://pi.dev/docs/latest/settings),
[CLI](https://pi.dev/docs/latest/cli).

A Pi file contains a session header followed by entries linked through
`id`/`parentId`. Branches, compaction, context edits, and extension state are part
of that file. Keeping only visible messages or the active branch would lose
information. The POC preserves every raw record, including unknown-to-Mallard
payload fields, through the relocation. Pi itself serves as the oracle for
tree structure and reconstructed context.
Sources: [session format](https://pi.dev/docs/latest/session-format),
[SDK session management](https://pi.dev/docs/latest/sdk).

The POC's experimental lineage codec accepts only a single linear entry chain.
It returns `Unknown(NonLinearParentage)` for the native branched fixture. This
is a deliberate boundary: successful archival/copying does not prove safe
automatic reconciliation. A production codec must account for every branch
and state entry, not merely fingerprint the active conversation. It must also
handle entry-ID collisions across concurrent writers. The experiment does not
exercise a live in-memory cursor moved without a subsequent persisted entry.

Forked files can name an absolute `parentSession` path. Relocation explicitly
refuses these; it does not silently drop or preserve a dangling source-machine
link. Production needs a selected-session identity map for fork links and a
clear policy when the parent was not selected. Project-path remapping alone
does not solve provider-home paths, out-of-project paths, or extension-specific
references. The POC's simple linear fingerprints are not a production portable
codec and are not certified invariant across relocation or credential redaction.

## Work required before product support

| Area | Existing seam and remaining work |
| --- | --- |
| Discovery and profiles | `SessionAdapter`, `ProviderDescriptor`, and the registry support Pi. Add bounded production scans, duplicate-ID handling, explicit root policy, and CLI discovery. |
| Publication | `build_conversation_publication` in `src-tauri/src/project_sync/service.rs` still uses Codex reconstruction or Claude-specific publication/tail encoding. Provide an explicit adapter-owned Pi preparation path. |
| Pull | `crates/mallard-infrastructure/src/pull_transcript.rs` assumes Claude wire semantics when `reconstructs_rollout_graph()` is false. Pi requires its own verified decode, identity remapping, and checkpoint preparation. |
| Reconciliation | Specify and test a versioned whole-tree/event-log contract, divergent copies, forks, compaction, context edits, and repush after relocation. Do not flatten the native tree. |
| Review and transcript UI | Implement Pi summary, timeline, expanded details, and explicit actions through the existing trait. Unsupported POC methods currently return an error or offer no action. |
| Safety | Apply existing redaction, path-origin tracking, digest/revision/expiry checks, immutable publication, and reviewed materialization. Test credential-bearing custom state and general non-Git `.pi` project-content exclusion separately. |
| Library and provider-home state | Design separate catalog adapters for any chosen skills/prompts/extensions. Do not import project-sync DTOs into the Library. This POC exports none of these resources. |

Pi's agent home can contain global instructions, prompt replacements, skills,
prompts, extensions, model configuration, and credentials. These do not form
a single portable “memory” object. A session-first feature should declare its
resource scope explicitly. Authentication and trust state remain machine-local;
executable extensions and package intent need separate review and installation
semantics. Project `.pi` configuration must not become provider-state capture.
See [Pi's configuration inventory](https://pi.dev/docs/latest/configuration).

The recommended next implementation slice, if separately authorized, is
session-only discovery/history plus verified whole-session Push/Pull with
explicit divergent-copy behavior. Keep native trees intact and add CLI resume
only after a real local-mock CLI continuation test. Broader harness resources
are a separate scope. The POC stops before this implementation.

Research also inspected upstream revision
[`4954d760504c11df9b2cbdc040346107b6a8153d`](https://github.com/earendil-works/pi/blob/4954d760504c11df9b2cbdc040346107b6a8153d/packages/coding-agent/src/core/session-manager.ts).
The executed oracle is the npm 1.3.0 package; that upstream revision is a
research reference, not a claim about the package's build commit.

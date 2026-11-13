# Pi harness feasibility POC

Test date: 2026-11-12. Scope: **experiment only**.

**Pi session portability is feasible. Adding Pi to the registry alone is not
sufficient for production support.** The experiment captures a session through
Mallard's real provider pipeline, relocates it using Mallard's path mapper, and
opens the result with Pi's own session manager. Production publication, Pull,
conflict handling, UI, and Library support remain unimplemented.

The target is the Pi coding agent from [earendil-works/pi](https://github.com/earendil-works/pi),
tested with `@earendil-works/pi-coding-agent@1.5.2` and session schema 3.
No older session formats or migrations are implemented.

## Reproduce

From the repository root, with the existing Rust toolchain, Node.js 22.19+,
and either an installed Pi 1.5.2 package or npm available. The installed CLI
probe additionally uses Python 3's standard library:

```sh
# Offline fixture and boundary checks, once Rust dependencies are available.
cargo test -p mallard-infrastructure --lib pi_poc

# Use the existing package behind the installed Pi launcher; no download.
# Replace the example directory with the package containing package.json and dist/.
node scripts/run-pi-harness-poc.mjs --installed-sdk /path/to/pi-coding-agent

# Exercise the installed `pi` command over RPC; no SDK dependency or download.
python3 scripts/run-pi-storage-rpc-poc.py

# Alternatively, download the pinned SDK for the portability experiment.
node scripts/run-pi-harness-poc.mjs
```

The SDK runner reads an explicitly supplied installed package, or downloads
the pinned Pi package into a fresh temporary directory with npm lifecycle
scripts disabled. It creates synthetic sessions in isolated
homes, invokes one otherwise ignored Rust test, checks the result with Pi,
and removes its temporary files. Package installation requires network access;
there are no model requests, real API credentials, or live session homes involved.
The Rust test uses explicitly supplied fixture roots. Node child processes
receive an environment allowlist with temporary homes. The separate RPC probe
executes the installed Pi CLI with offline mode and resource/tool discovery
disabled. Its model metadata points at an unreachable loopback address using
a synthetic key; it sends only history, state, and clone commands.

The experimental adapter lives in `pi_poc.rs`, compiled only through a
`#[cfg(test)]` module declaration in
`crates/mallard-infrastructure/src/providers/mod.rs`. It never enters
`AgentProviderRegistry::production()` and does not ship in the application.
This refresh updates the SDK pin and synthetic checks in the runner; no
application dependencies, schemas, DTOs, commands, settings, or UI are changed.

## Observed results

The native runner passed with Pi 1.5.2 and Node.js 24.22.0 on macOS:

| Check | Result |
| --- | --- |
| Register an adapter using the existing trait | Passed, in a test registry only |
| Discover by the session header's exact project directory | One selected session; one other project's session excluded |
| Capture through `discover_full` and `capture_from_discovery` | One session file captured |
| Relocate the project directory and recognized paths inside session records | Passed using `SessionPathMapper`; exact roots and descendant paths in the fixture |
| Find the installed session using native `SessionManager.list` | Passed in Pi's destination project bucket |
| Preserve entries and the complete tree | All 19 entries preserved, including the abandoned branch |
| Rebuild model context using native `buildSessionContext` | Equal with recognized fixture paths relocated and ambiguous prose preserved |
| Preserve compaction, context edits, tool calls/results, thinking, labels, title, usage, and extension data | Passed |
| Preserve system prompt sections and tool declarations through compaction | Passed; Pi rebuilt the system checkpoint and tool loadout |
| Navigate to the abandoned branch | Passed |
| Reopen after moving only the in-memory branch cursor | Pi reopens at the last persisted entry; the transient cursor is not in the file |
| Remap a root followed by a sentence period | Not remapped by the existing conservative path grammar; explicitly checked as unchanged |
| Append a user/assistant pair and reopen | Passed; new records persisted with the expected parent |
| Leave the source transcript intact | Byte-identical after the experiment |
| Production Push/Pull, remote storage, actual model turn, TUI | Not exercised |

The five non-ignored Rust tests also pass. They cover byte-identical capture,
exact ownership (including exclusion of project subdirectories), exclusion of
auth/settings/project configuration from this session-only capture, invalid
IDs and ancestry, unsupported schema versions, truncated records, byte/count
bounds, unsafe targets, linked-fork refusal, and symlinks on Unix.

Verification completed for this refresh:

- `node scripts/run-pi-harness-poc.mjs`: passed the native round trip.
- `node scripts/run-pi-harness-poc.mjs --installed-sdk ...`: passed using the
  locally installed Pi 1.5.2 package, including the storage probes below.
- `python3 scripts/run-pi-storage-rpc-poc.py`: passed against the installed
  Pi 1.5.2 CLI, with no model requests.
- `cargo test -p mallard-infrastructure --lib pi_poc`: 5 passed, 1 ignored.
- `node --check scripts/run-pi-harness-poc.mjs` and `git diff --check`: passed.

The native round-trip Rust test is one of the infrastructure suite's ignored
tests; the Node runner executes it explicitly. Production Rust and frontend
code were unchanged, so full application suites and browser/UI verification
were not rerun for this script/document refresh.

The Rust capture experiment bounds a file to 2 MiB, a parsed session to 4,096
records, and a discovery walk to 4,096 entries at depth two. These are POC
limits, not Pi limits or proposed product defaults. The larger native storage
probe deliberately exceeds them and does not pass through Mallard capture.
The parser validates structural identity and parentage; it is not a complete
semantic validator for every Pi entry or extension payload.

“Continuation” here means Pi's session manager appended and reloaded native
records. The installed CLI also reads and clones fixture sessions and reopens
a clone, but no CLI or LLM completes a generated turn. Both simulated machines
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
Sources: [environment variables](https://pi.dev/docs/latest/environment-variables),
[CLI](https://pi.dev/docs/latest/cli).

A Pi file contains a session header followed by entries linked through
`id`/`parentId`. Branches, compaction, context edits, and extension state are part
of that file. Keeping only visible messages or the active branch would lose
information. The POC preserves every raw record, including unknown-to-Mallard
payload fields, through the relocation. Pi itself serves as the oracle for
tree structure and reconstructed context. Current system messages also store
prompt sections and tool declarations; compaction checkpoints carry their
replayed state. These are transcript contents even when configuration files
are excluded, and must pass the same reviewed export and redaction checks.
Sources: [session format](https://pi.dev/docs/latest/session-format),
[SDK session management](https://pi.dev/docs/latest/sdk).

The POC's experimental lineage codec accepts only a single linear entry chain.
It returns `Unknown(NonLinearParentage)` for the native branched fixture. This
is a deliberate boundary: successful archival/copying does not prove safe
automatic reconciliation. A production codec must account for every branch
and state entry, not merely fingerprint the active conversation. It must also
handle entry-ID collisions across concurrent writers. The native test also
confirms that moving the branch cursor without appending an entry does not
persist that cursor. File sync can preserve the stored tree and last persisted
leaf, but cannot recover this transient UI state from the JSONL file.

Forked files can name an absolute `parentSession` path. The Rust relocation
experiment still explicitly refuses these, but the deeper storage probe below
establishes that native forks carry their own history: the parent link is
ancestry metadata, not a missing-history segment that Pi must load. Production
needs a portability/privacy policy for this metadata, such as mapping selected
parents and explicitly representing unavailable parents. It does not need a
Codex-style recursive history resolver for that link. Project-path remapping
alone does not solve provider-home paths, out-of-project paths, or
extension-specific references. The POC's simple linear fingerprints are not a
production portable codec and are not certified invariant across relocation
or credential redaction.

The expanded fixture exposed an existing path-mapper boundary: a bare root
followed by a period stays literal. The runner now checks both relocation of
an exact root in a system-prompt section and preservation of that ambiguous
prose spelling. Its independent oracle only replaces the fixture's exact
roots and descendant paths; it no longer assumes unrestricted string
replacement. This boundary limits portability:
arbitrary prose or extension payloads can retain source-machine path text.
Production needs an explicit policy and representative tests for Pi's system
sections, not a promise that every path-looking substring is rewritten.

## Does Pi store paginated sessions like Codex?

**Pi 1.5.2 stores a session's native history in one JSONL file. It has
incremental RPC reads, but no Codex-style linked physical history segments or
bounded page API in the inspected implementation.** This conclusion comes
from tracing the reader and writer, then executing the installed package and
CLI. It is scoped to built-in session storage, not arbitrary extension files.

Three separate concepts matter:

| Layer | Pi behavior | Codex comparison relevant to Mallard |
| --- | --- | --- |
| Physical history | `_persist()` appends to the current JSONL file. Branching with `/tree` changes entry parentage within it. | Mallard's Codex `history_base` resolver can follow multiple physical rollout segments, enforcing byte and ordinal cutoffs. A paginated mode label alone does not mean a particular thread has multiple files. |
| Reading history | `loadEntriesFromFile()` reads with a 1 MiB buffer, accumulates all parsed entries, and builds an in-memory ID index. Buffered reads do not provide bounded-memory history pages. | Codex's public app-server exposes `thread/turns/list` and `thread/items/list` with limits and cursors, independently of the physical file layout. |
| Model context | Compaction appends a summary/checkpoint. Context reconstruction selects a branch and retained range; original entries remain in the file. | Compaction and pagination solve different problems. A reduced model context is not evidence that older session files or records can be discarded. |

Sources: Pi's tagged [writer](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/session-manager.ts#L1173),
[reader](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/session-manager.ts#L628),
[branch/context guide](https://pi.dev/docs/latest/sessions),
[official OpenAI app-server documentation](https://learn.chatgpt.com/docs/app-server),
and Mallard's implementation in
`crates/mallard-infrastructure/src/codex_history_graph.rs` and
`crates/mallard-infrastructure/src/codex_rollout.rs`.

`get_entries` returns all raw entries in append order, including inactive
branches and pre-compaction history. Supplying `since` returns every entry
after that ID. Its declared request has no `limit` and its response has no
next-page cursor; this is incremental retrieval, not bounded pagination.
`get_messages` instead returns the current agent messages, which can omit
older compacted history and abandoned branches. `get_tree` returns the full
entry tree. `SessionManager.list()` streams files to build metadata and
reports progress, but ultimately returns a session array rather than a
cursor-based page. These APIs should not be confused with storage segments.
Sources: [RPC commands](https://pi.dev/docs/latest/rpc-commands#get_entries),
[RPC implementation](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/modes/rpc/rpc-mode.ts#L656),
[request types](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/modes/rpc/rpc-types.ts#L65).

Forking creates another independent history file. `createBranchedSession()`
copies the selected ancestry path and reconstructs its labels; the runtime
uses it for ordinary `/fork` and `/clone`. `forkFrom()` copies all source
entries for a cross-project fork. Both write a `parentSession` reference, but
opening the result does not read that parent to rebuild context.
Sources: [branch-file creation](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/session-manager.ts#L1643),
[cross-project fork](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/session-manager.ts#L1825),
[runtime fork](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/agent-session-runtime.ts#L262).

Observed with the installed Pi 1.5.2 on macOS:

| Probe | Result |
| --- | --- |
| 5,200 messages followed by compaction | 5,201 entries plus the header, 4,263,279 bytes, one JSONL file |
| Compare bytes before/after compaction | Original prefix unchanged; older messages still present after reopen |
| Reconstruct compacted model context | Three messages: summary plus the retained user/assistant pair |
| SDK branch and cross-project forks, parent file moved away | Both reopen with the same context and accept persisted continuation records |
| Installed CLI `get_entries` / `get_entries(since)` | Nine raw entries / five entries after the supplied ID |
| Installed CLI `get_messages` on the same file | Three context messages; raw history still contains the inactive branch |
| Installed CLI clone, then a new CLI process with parent file unavailable | Clone reopens with identical context |
| Open a valid JSONL fixture missing its final newline | Pi adds the newline; native session opening is not a read-only inventory operation |

The large-file fixture is evidence for this size, not a maximum-size or
performance benchmark. The source trace establishes the storage design;
there is no rollover branch in the inspected writer. No generated turn,
real credential, or personal session is used by either probe.

For Mallard, use one native Pi transcript as the capture unit, retain every
entry, and implement paged transcript viewing with Mallard's own bounded
index. Specify event-log reconciliation while preserving Pi's tree edges.
Handle fork ancestry as portable metadata instead of treating it as Codex
`history_base`. Keep a strict read-only scanner rather than using Pi's loader
for inventory: its newline repair, legacy migration, and permissive parsing
have different responsibilities from Mallard's capture boundary. The native
large-file result also shows why the 2 MiB / 4,096-record POC limits cannot
silently become product limits.

The same reader/writer structure and RPC behavior were checked on upstream
[`d36266e06669c6991a7c9509d1da04d18d30378d`](https://github.com/earendil-works/pi/blob/d36266e06669c6991a7c9509d1da04d18d30378d/packages/coding-agent/src/core/session-manager.ts),
the inspected `main` head on 2026-11-12. The executed tests use the installed
1.5.2 release, not an unbuilt main checkout.

## Work required before product support

| Area | Existing seam and remaining work |
| --- | --- |
| Discovery and profiles | `SessionAdapter`, `ProviderDescriptor`, and the registry can host a Pi adapter. Add bounded production scans, duplicate-ID handling, explicit root policy, and CLI discovery. |
| Publication | `build_conversation_publication` in `src-tauri/src/project_sync/service.rs` uses Codex reconstruction or Claude-specific publication/tail encoding, and explicitly refuses a provider with no conversation wire builder. Provide an explicit adapter-owned Pi preparation path. |
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

The recommended first production slice is session-only discovery/history plus
verified whole-session Push/Pull with explicit divergent-copy behavior.
Keep native trees intact and add CLI resume
only after a real local-mock CLI continuation test. Broader harness resources
are a separate scope. The POC stops before this implementation.

Version-specific research checked Pi's tagged
[`v1.5.2` session manager](https://github.com/earendil-works/pi/blob/v1.5.2/packages/coding-agent/src/core/session-manager.ts)
and [message types](https://github.com/earendil-works/pi/blob/v1.5.2/packages/ai/src/types.ts).
The executed oracle is the npm 1.5.2 package. The version tag is a research
reference, not a claim about the package's build commit.

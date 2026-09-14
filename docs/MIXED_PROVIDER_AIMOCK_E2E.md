# Mixed provider E2E test procedure

The default `mixed-aimock-e2e` test creates exactly **25 Codex and 15 Claude
source sessions**. The original 100-Codex/20-Claude test is now
[`mixed-aimock-e2e-xlarge`](MIXED_PROVIDER_AIMOCK_E2E_XLARGE.md).
Both profiles use the same test harness and performance limits.

This test uses real Codex and Claude Code processes with local mock API
responses. Mallard operations use the production `ProjectSyncService`.
It tests service behavior. It does not measure the React UI or Tauri IPC.

## Requirements

- Node 20.15 or later. Use one exact version for benchmark comparisons.
- Codex CLI 0.154.x. Reports record the exact patch version.
- Claude Code 2.1.266.
- aimock 1.39.0, installed by `npm ci`.
- The repository Rust toolchain and native build dependencies.
- A working native Codex sandbox. A failed preflight fails the run.

Use temporary provider homes only. The runner removes inherited API keys,
provider endpoints, proxies, and process injection settings from native child
environments. Codex uses no API key. Claude uses the fixed mock placeholder.
The mocks have no upstream or recording mode. Model retries are disabled.
Claude uses explicit options to disable settings discovery, hooks, skills,
plugins, and MCP discovery. It does not use `--bare`, which skips awaiting the
initial transcript recording operation in Claude Code 2.1.266. Missing user or
assistant records still fail the test; failure reports include safe transcript
states and counts.

On macOS, run outside a nested execution sandbox if the native Codex sandbox
cannot start. Local commands set loopback model endpoints. They do not prove
that the host blocks all external traffic. The dedicated Linux execution
command supplies that network restriction.

## Commands

```sh
npm run test:aimock-servers
npm run test:mixed-aimock-harness
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e
```

For a debug build with the same 40 sessions:

```sh
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e -- --debug
```

The default build is release. This profile has a fixed 25/15 session count.
Use `test:mixed-aimock-e2e-xlarge` or `bench:mixed-aimock-e2e-xlarge` for
the original 100/20 fixture. Only xlarge accepts `--scale=10`, `50`, or `100`.
The default profile runs all six graph fault cases.

## Fixed data

The full fixture has 316 authored source turns and 1,744 KiB of assistant
bodies before native metadata and inherited prefixes. It uses deterministic
1 KiB and 8 KiB bodies. Each body has checked path spans, literal destination
text, literal path-map markers, and non-working credential canaries.
Twelve Codex sessions read a fixture file through `exec_command`. Four Claude
sessions use the real `Read` tool. Tool continuation requests count separately.

| Codex labels | Shape |
| --- | --- |
| C001–C005 | Five independent roots |
| C006–C013 | A chain from C001, with eight base edges |
| C014–C015 | Two children of C002, forked after turn 2 |
| C016–C020 | Five children of C002, forked after turn 4 |
| C021 | A child of C003 |
| C022–C023 | Two children of C021 |
| C024–C025 | Two more levels below C022 |

Codex C020 and C025 each have 24 source turns. C013, C018, and C019 each have
12 turns. Other Codex sessions have four turns. Claude L001–L008 each have
four turns, L009–L012 each have 12 turns and a Read operation, and L013–L015
each have 24 turns. Four-turn sessions use 1 KiB bodies. Other sessions use
8 KiB bodies.

The source graph has exactly 20 direct edges and a maximum depth of eight.
Native IDs are private runtime values. Stable fixture labels identify expectations. Injected exclusion and
fault records do not count as native sessions.

## Test steps

| Step | Action | Check |
| --- | --- | --- |
| 1 | Create private A, B, and C replicas and a local-folder store. Check CLI pins and the native sandbox. | Each provider and Mallard repository has a separate temporary home. B and C paths have spaces, Unicode, and different lengths. |
| 2 | Generate all valid sessions and Codex forks through native processes. | Full response bodies and ordered history match the local mock. Raw source records contain the expected canaries. Every batch consumes each expected request once. |
| 3 | Read the durable Codex graph. | Parent IDs, global ordinals, byte cutoffs, prefix digests, and resolved segment order match the independent graph. |
| 4 | Inject a Codex internal-subagent control and a foreign Claude transcript in a colliding project bucket. | Neither control enters comparison or Push review. Remove the controls after the check. |
| 5 | Publish C013, C014, C016, C020, C022, C023, C024, C025, L009, and L015. | Ten cloud conversations are selected. The eight selected Codex sessions have 19 distinct required physical nodes. Unselected bases do not become extra conversations. |
| 6 | Pull the selection on B and resume each session in a disposable snapshot. | The native client sends the complete expected history to the mock. B does not need A's physical base files. Source file digests stay unchanged during Push and snapshot verification. |
| 7 | Publish and Pull the complete set. Read every history page and resume every B session. | There are 40 conversations. Event IDs do not repeat. Prompt and response markers remain in order. Complete assistant bodies have the expected paths and typed redactions. |
| 8 | Save the accepted checkpoint. Repeat Push and Pull. | The immutable manifest and mutable head remain unchanged. Inspection timestamps are excluded from this comparison. |
| 9 | Append distinct turns on A and B to C006, C020, L009, and L013. Publish A, then approve Keep both on B. | Four preserved branches retain their local native aliases. Their new logical IDs and cloud mappings match the receipts. Both replicas reach 44 conversations. |
| 10 | Reverse the conflict direction with C007, C025, L010, and L014. | Both replicas reach 48 conversations: 29 Codex and 19 Claude. |
| 11 | Pull all histories on C. Append fixed path-bearing turns to C025 and L015 at C, A, and B, with reviewed Push/Pull at each hop. | Old path spans and new spans map correctly. Text that originally named B stays literal. Path-map tags do not accumulate. |
| 12 | Resume all final A and B histories in snapshots. Repeat no-op Push/Pull. | All branches remain resumable. Verification changes no authoritative transcript. No new revision is published. |
| 13 | Try an expired Pull plan. On fresh accepted copies, inject a missing Codex base, bad byte cutoff, bad ordinal cutoff, cycle, a base shorter than its declared cutoff, and symlink. | The invalid operation fails. Independent graph roots still resolve where the fault permits an independent lookup. |
| 14 | Scan every portable storage object and the export report. Stop owned processes and remove temporary data. | Known canaries are absent. Only sanitized reports remain. |

The 40 source sessions are native sessions. The two conflict rounds add eight
preserved branches. Control and graph fault fixtures do not add source sessions.

A resume snapshot copies transcripts and portable native indexes. It omits the
Codex SQLite index because that index contains absolute source rollout paths.
Codex rebuilds it within the snapshot. Benchmark checkpoints keep the SQLite
index because replay restores the original test paths.

## Performance procedure

Generate and validate native data once. Save private checkpoints for initial
publication, empty-destination Pull, accepted state, one changed session, ten
changed sessions, a Claude append, and Keep both. Native appends occur before
timing. Restore all provider, Mallard, and storage state before each sample.
Start a fresh service worker for every independent sample.

```sh
# A reviewed full product commit ID is required for a comparison.
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e -- --baseline-ref="$BASELINE_REF"
```

The harness uses the current test worker against both product revisions.
It alternates candidate-first and baseline-first order. Each operation has two
discarded iterations and at least 20 measured samples. Use `--samples=40` to
increase the sample count. Use `--workers=1` or `--workers=4` to fix the worker
setting. Debug benchmark runs are rejected.

Measured operations include cold and settled warm comparison, one native
append for each provider, Push review, changed Push review, full Push/Pull,
one-session and ten-session updates, Pull review on empty and accepted state,
Keep both, first-page history, complete history traversal, and no-op Push/Pull.
The history samples target C013, C025, and L015. There are 23
operation groups.

Cold means a new process. It does not mean an empty operating-system cache.
Warm comparison waits for a bounded, confirmed quiet point. It must have zero
transcript, head, probe, append, selected-capture, and full-graph work. Push
review must not capture selected payloads. Pull phase output must retain the
current two semantic analyses per prepared transcript. CPU and peak RSS refer
to the Mallard worker. Peak RSS includes that worker's setup and preparation.
Provider and mock startup times are outside operation timing. Each full-Pull
benchmark variant also resumes all restored histories once, outside timing.

The report records raw samples, median, nearest-rank p95, CPU time, peak RSS,
phase times, and provider-root work counters. Metrics arrays list Codex first
and Claude second. Native generation and complete
job duration are separate. The test does not claim per-process provider RSS,
local-store request counts, or queue-byte limits when those metrics are not
exposed by the reused interfaces.

| Measure | Allowed increase over baseline |
| --- | --- |
| Comparison/history median | Greater of 20% or 5 ms |
| Comparison/history p95 | Greater of 25% or 10 ms |
| Push/Pull median | Greater of 20% or 100 ms |
| Push/Pull p95 | Greater of 25% or 250 ms |
| Maximum worker RSS | Greater of 15% or 32 MiB |
| Correctness and exact work limits | No tolerance |

A timing or RSS breach starts one further full paired batch. A repeated breach
is `regression`. A breach that does not repeat is `unstable`. Both are nonzero
results. Correctness failures are not retried. Missing, incompatible, or
incomplete benchmark data is `invalid`. Without a baseline, status is
`uncalibrated` and exit code is 2. No baseline is selected or replaced
implicitly.

Qualify the first baseline with five stable jobs on the fixed runner. Review
the baseline commit and thresholds before setting the repository variable.
Changing the profile, fixture, CLI pins, toolchain, build, hardware, or worker
count requires a matching baseline. Mixed and xlarge reports have distinct
fixture identities and cannot be compared with each other. Local measurements
do not approve a CI baseline.

## Periodic runner

The workflow is `.github/workflows/mixed-aimock-e2e.yml`.

1. Provide a dedicated Linux runner with labels `self-hosted`, `linux`, and
   `mallard-mixed-aimock`. Keep personal provider data off this host.
2. Install the exact native tools, Rust dependencies, `unshare`, and `ip`.
   Permit nested user namespaces required by the native Codex sandbox.
3. Set `MIXED_AIMOCK_BASELINE_REF` to the reviewed 40-character product commit.
4. Dispatch the `mixed` profile for the 25/15 test. Select `xlarge` only for
   the larger fixture. The scale input applies to xlarge only.
5. Check the sanitized artifacts and job exit status.

The workflow prepares both builds before network isolation. Test execution
uses a private network namespace with loopback only. It does not change the
host firewall. Cargo uses offline mode during execution. The nightly run is
09:00 UTC and uses the 25/15 `mixed` profile. The weekly xlarge run is Sunday
at 11:00 UTC and covers Codex scales 10/50/100
with worker counts 1 and 4. These six cases run as separate serial jobs.
One concurrency group prevents overlap. Each case has a 90-minute job
watchdog. Artifacts expire after 90 days.

The workflow takes effect after it is on the default branch and a runner and
baseline are configured. This implementation does not provision that host,
change repository variables, or approve a baseline.

## Reports and supporting tests

Reports go to `output/mixed-aimock/<run>/`:

- `summary.md`: result and test counts.
- `results.json`: safe metadata, measurements, assertions, and benchmark result.
- `mock-audit.json`: per-provider batch counts and fixture labels.
- `candidate.json` and `baseline.json`: comparator inputs when available.

Never upload temporary homes, transcripts, bundles, raw mock journals, or
backups. The runner has bounded native calls, a job watchdog, signal cleanup,
and an export privacy check. Failure reports use fixture labels, fixed error codes, and diagnostic digests.
They do not export raw process error text. All credentials in the test data are non-working canaries.

Run the existing provider suites and Rust library tests with this test when
changing shared mock or sync behavior:

```sh
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

These suites retain the detailed source-change isolation, redaction-rule,
path-origin, storage CAS, and local S3-stub cases. The mixed benchmark uses
local-folder storage. It does not add a full-size S3 benchmark or UI timing.
The original design remains in
[the planning document](PLAN_MIXED_PROVIDER_AIMOCK_SCALE_E2E.md).

## Local validation record

Validated on 2026-09-07 with a release build on macOS:

- Source: 25 Codex and 15 Claude sessions; 316 source turns.
- Graph: 20 edges and maximum depth eight. All six graph fault cases passed.
- Final set: 29 Codex and 19 Claude conversations, including eight preserved
  branches. Native resume, path remapping, redaction, and no-op checks passed.
- Performance: all 23 operation groups completed 20 measured samples each,
  for 460 samples. Each group also completed two discarded iterations.
- Complete job: 9 minutes 50 seconds, including build and native validation.
- No baseline was supplied. Status was `uncalibrated`, with the required exit
  code 2. These measurements do not establish a regression result.
- All 40 mock and harness tests passed. They also verify the original xlarge
  counts, tiers, graph, dependency selection, and profile separation.
- Workflow YAML, shell syntax, and nightly/manual/weekly case selection passed
  local checks.

| Operation | Median ms | p95 ms |
| --- | ---: | ---: |
| Cold comparison | 48.782 | 52.641 |
| Warm comparison | 8.980 | 9.668 |
| Full Push | 1,403.520 | 1,520.619 |
| Full Pull | 2,108.436 | 2,210.805 |
| Keep both | 434.317 | 474.091 |

The local report is under
`output/mixed-aimock/2026-09-07T20-51-57.070Z/`. Reports are ignored by Git.
The prior 100/20 measurements are in the xlarge procedure. They use a different
fixture and sample count and must not serve as the 25/15 performance baseline.

The Linux runner still requires qualification on the dedicated host. No
periodic run has been activated from this working tree.

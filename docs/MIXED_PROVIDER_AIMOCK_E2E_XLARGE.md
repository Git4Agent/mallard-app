# Xlarge mixed provider E2E test procedure

This is the renamed 100-Codex/20-Claude test. Use
[the default mixed test](MIXED_PROVIDER_AIMOCK_E2E.md) for 25 Codex and 15 Claude
sessions. The full xlarge graph and test steps are unchanged.

This test uses real Codex and Claude Code processes with local mock API
responses. Mallard operations use the production `ProjectSyncService`.
It tests service behavior. It does not measure the React UI or Tauri IPC.

## Requirements

- Node 20.15 or later. Use one exact version for benchmark comparisons.
- Codex CLI 0.152.x. Reports record the exact patch version.
- Claude Code 2.1.266.
- aimock 1.39.0, installed by `npm ci`.
- The repository Rust toolchain and native build dependencies.
- A working native Codex sandbox. A failed preflight fails the run.

Use temporary provider homes only. The runner removes inherited API keys,
provider endpoints, proxies, and process injection settings from native child
environments. Codex uses no API key. Claude uses the fixed mock placeholder.
The mocks have no upstream or recording mode. Model retries are disabled.

On macOS, run outside a nested execution sandbox if the native Codex sandbox
cannot start. Local commands set loopback model endpoints. They do not prove
that the host blocks all external traffic. The dedicated Linux execution
command supplies that network restriction.

## Commands

```sh
npm run test:aimock-servers
npm run test:mixed-aimock-harness
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e-xlarge
```

For a small development run:

```sh
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e-xlarge -- --debug --scale=10
```

The default build is release. The available Codex scales are 10, 50, and 100.
Claude adds 2, 10, or 20 sessions. Each subset includes all required parents.
The 10-session Codex subset has no forks. Use scale 50 or 100 for graph faults.

## Fixed data

The full fixture has 1,048 authored source turns and 6,424 KiB of assistant
bodies before native metadata and inherited prefixes. It uses deterministic
1 KiB and 8 KiB bodies. Each body has checked path spans, literal destination
text, literal path-map markers, and non-working credential canaries.
Twelve Codex sessions read a fixture file through `exec_command`. Six Claude
sessions use the real `Read` tool. Tool continuation requests count separately.

| Codex labels | Shape |
| --- | --- |
| C001–C020 | 20 independent roots |
| C021–C032 | A chain from C001, with 12 base edges |
| C033–C056 | 24 children of C002; half fork after turn 2, half after turn 4 |
| C057–C084 | Four nested trees under C003–C006 |
| C085–C092 | Eight further forks from nested leaves |
| C093–C100 | Eight children of C085–C092 |

The source graph has exactly 80 direct edges. Native IDs are private runtime
values. Stable fixture labels identify expectations. Injected exclusion and
fault records do not count as native sessions.

## Test steps

| Step | Action | Check |
| --- | --- | --- |
| 1 | Create private A, B, and C replicas and a local-folder store. Check CLI pins and the native sandbox. | Each provider and Mallard repository has a separate temporary home. B and C paths have spaces, Unicode, and different lengths. |
| 2 | Generate all valid sessions and Codex forks through native processes. | Full response bodies and ordered history match the local mock. Raw source records contain the expected canaries. Every batch consumes each expected request once. |
| 3 | Read the durable Codex graph. | Parent IDs, global ordinals, byte cutoffs, prefix digests, and resolved segment order match the independent graph. |
| 4 | Inject a Codex internal-subagent control and a foreign Claude transcript in a colliding project bucket. | Neither control enters comparison or Push review. Remove the controls after the check. |
| 5 | Publish C032, C033, C045, C060, C067, C092, C099, C100, L011, and L017. | Ten cloud conversations are selected. The eight selected Codex sessions have 31 distinct required physical nodes. Unselected bases do not become extra conversations. |
| 6 | Pull the selection on B and resume each session in a disposable snapshot. | The native client sends the complete expected history to the mock. B does not need A's physical base files. Source file digests stay unchanged during Push and snapshot verification. |
| 7 | Publish and Pull the complete set. Read every history page and resume every B session. | There are 120 conversations. Event IDs do not repeat. Prompt and response markers remain in order. Complete assistant bodies have the expected paths and typed redactions. |
| 8 | Save the accepted checkpoint. Repeat Push and Pull. | The immutable manifest and mutable head remain unchanged. Inspection timestamps are excluded from this comparison. |
| 9 | Append distinct turns on A and B to C007, C033, C060, C099, L011, and L017. Publish A, then approve Keep both on B. | Six preserved branches retain their local native aliases. Their new logical IDs and cloud mappings match the receipts. Both replicas reach 126 conversations. |
| 10 | Reverse the conflict direction with C008, C045, C067, C100, L012, and L018. | Both replicas reach 132 conversations: 108 Codex and 24 Claude. |
| 11 | Pull all histories on C. Append fixed path-bearing turns to C099 and L020 at C, A, and B, with reviewed Push/Pull at each hop. | Old path spans and new spans map correctly. Text that originally named B stays literal. Path-map tags do not accumulate. |
| 12 | Resume all final A and B histories in snapshots. Repeat no-op Push/Pull. | All branches remain resumable. Verification changes no authoritative transcript. No new revision is published. |
| 13 | Try an expired Pull plan. On fresh accepted copies, inject a missing Codex base, bad byte cutoff, bad ordinal cutoff, cycle, a base shorter than its declared cutoff, and symlink. | The invalid operation fails. Independent graph roots still resolve where the fault permits an independent lookup. |
| 14 | Scan every portable storage object and the export report. Stop owned processes and remove temporary data. | Known canaries are absent. Only sanitized reports remain. |

Small runs use two conflicts per direction. Their final count is 16. Scale 50
uses the same small conflict groups and ends with 64 conversations.

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
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e-xlarge -- --baseline-ref="$BASELINE_REF"
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
The full-size history samples target C032, C099, and L020. There are 23
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
Changing the fixture, CLI pins, toolchain, build, hardware, or worker count
requires a matching baseline. Local measurements do not approve a CI baseline.

## Periodic runner

The workflow is `.github/workflows/mixed-aimock-e2e.yml`.

1. Provide a dedicated Linux runner with labels `self-hosted`, `linux`, and
   `mallard-mixed-aimock`. Keep personal provider data off this host.
2. Install the exact native tools, Rust dependencies, `unshare`, and `ip`.
   Permit nested user namespaces required by the native Codex sandbox.
3. Set `MIXED_AIMOCK_BASELINE_REF` to the reviewed 40-character product commit.
4. Select `xlarge` when dispatching this suite. Qualify scale 10, then 100.
   The default `mixed` dispatch runs 25 Codex and 15 Claude sessions.
5. Check the sanitized artifacts and job exit status.

The workflow prepares both builds before network isolation. Test execution
uses a private network namespace with loopback only. It does not change the
host firewall. Cargo uses offline mode during execution. The nightly run is
09:00 UTC and uses the default 25-Codex/15-Claude fixture. The weekly xlarge run is Sunday at 11:00 UTC and covers scales 10/50/100
with worker counts 1 and 4. These six cases run as separate serial jobs.
One concurrency group prevents overlap. Each case has a 90-minute job
watchdog. Artifacts expire after 90 days.

The workflow takes effect after it is on the default branch and a runner and
baseline are configured. This implementation does not provision that host,
change repository variables, or approve a baseline.

## Reports and supporting tests

Reports go to `output/mixed-aimock-xlarge/<run>/`:

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

Before the rename, validated on 2026-09-07 with release builds on macOS.
The rename preserves this fixture. These results are historical; they are not
measurements of the new 25-Codex/15-Claude test:

- Full fixture: 100 Codex and 20 Claude sessions; 1,048 source turns.
- Graph: 80 edges and maximum depth 12.
- Final set: 108 Codex and 24 Claude conversations.
- All six injected graph fault cases passed.
- All 1,874 model requests and 356 Claude probes were accounted for.
- The paired benchmark passed all 23 operation groups, with 920 measured
  samples in total. The complete job took 40 minutes 28 seconds.
- The reference product commit was
  `f9628b039a988371713b6970ee99b198464ecb3b`. This checked the comparison
  mechanism against the current product. It did not approve a CI baseline.
- The intermediate 50-Codex/10-Claude run passed with 520 source turns and
  64 final conversations. A candidate-only small benchmark returned the
  required `uncalibrated` status.
- Both existing native provider E2E suites passed. All 276 Rust library
  tests and all 37 mock/harness tests passed.
- Final failure-report guards passed the harness tests. The explicit Claude
  retry setting passed a separate native Read and snapshot-resume check.
- The isolated shell wrapper and workflow YAML passed syntax checks.

The Linux runner and its network namespace still require qualification on the
dedicated host. Five stable calibration jobs and baseline review remain
operator steps. No periodic run has been activated from this working tree.

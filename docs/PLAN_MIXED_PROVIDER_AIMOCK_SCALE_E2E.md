# Plan: Mixed Claude and Codex E2E Test and Performance Checks

Status: implemented as `mixed-aimock-e2e-xlarge`. Use [the xlarge test procedure](MIXED_PROVIDER_AIMOCK_E2E_XLARGE.md) for commands and implemented coverage. The dedicated runner and first reviewed baseline still require qualification. This file retains the original design scope.
Date: 2026-09-06.

## 1. Required result

Add one repeatable test that uses real Codex and Claude Code processes with
local mock API responses. Exercise the production Mallard setup, review,
Push, Pull, history, and restore paths with both providers in one project.

Use **100 sync-eligible Codex sessions plus 20 Claude sessions**. This is the
default interpretation of the requested 100-session Codex test. Control
fixtures and branches created during conflict tests have separate counts.

The test must prove:

- Correct history for deep forks, wide forks, nested forks, and shared bases.
- Correct selection when a selected Codex leaf depends on unselected bases.
- Complete native resume after transport to a different project path.
- Correct path conversion and credential redaction for both providers.
- Correct conflict review, retained branches, and repeated synchronization.
- Stable performance against a reviewed baseline on a fixed runner.

This is a service E2E test with real provider CLIs. It does not measure React
rendering or Tauri IPC. Keep frontend integration checks as a separate gate.
Use the full Tauri application for any later UI acceptance work.

## 2. Current code to reuse

The following facts were checked in the working tree. Some files have pending
edits. Check these contracts again when implementation starts.

| Existing code | Reuse and required extension |
| --- | --- |
| [Codex E2E target](../src-tauri/tests/live_codex_aimock_e2e.rs) | Real CLI turns, native paginated `thread/start` and `thread/fork`, isolated homes, reviewed Push/Pull, history resume, and privacy checks. Extend the five-node graph to 100 sessions. |
| [Claude E2E target](../src-tauri/tests/live_claude_aimock_e2e.rs) | Real `Read` tool results, fake credentials, path relocation, Keep both, native resume, and request audits. Extend to 20 sessions. |
| [Codex mock](../scripts/live-codex-aimock-server.mjs) and [Claude mock](../scripts/live-claude-aimock-server.mjs) | Strict loopback fixtures and bounded audits. Both currently allow 64 scenarios and 64 expected history items. Text responses currently contain short markers. |
| [Metrics](../crates/mallard-infrastructure/src/sync_metrics.rs) | Root-scoped read, parse, hash, capture, graph, and cache counters. `MALLARD_SYNC_METRICS=1` also logs operation deltas. |
| [Pull preparation](../crates/mallard-infrastructure/src/pull_transcript.rs) | Worker analysis counts and phase times. An existing ignored benchmark measures synthetic transcript preparation only. |
| [Failure isolation](../src-tauri/tests/support/codex_failure_isolation.rs) | Source-change injection, one source check per attempted update, per-thread failure, and preserved history checks. |
| [Path conversion](../crates/mallard-infrastructure/src/session_path_map.rs) and [redaction](../crates/mallard-infrastructure/src/credential_redaction.rs) | Current path spelling tags, local origin records, typed replacements, and ruleset fingerprints. |

Current test pins are Codex `0.152.x`, Claude Code `2.1.260`, and aimock
`1.39.0`. The runners require Node 20.15.0 or newer. These are repository
contracts, not claims about the latest provider releases. Record the exact
Codex patch version in each benchmark baseline.

The current product excludes internal Codex subagent sessions from sync.
The dependency graph in this plan is the native `history_base` graph. A native
fork has one immediate history base. Many selected leaves can share that base.
Do not invent multiple-parent histories or require subagent transport.

## 3. Test environment and mock contract

Create replicas A, B, and C under one private temporary test root. Each replica
has a separate process home, Codex home, Claude home, Mallard repository, and
project directory. Bind both provider profiles to the same Mallard project.
Use different project path lengths. Include spaces and Unicode in B and C.

Use a local-folder store for the required test. Use a disposable local S3 stub
for a separate weekly transport run. Neither run uses a remote model or a real
storage account.

1. Resolve installed executables and check pinned versions in isolated homes.
2. Remove inherited provider credentials, provider endpoints, proxy settings,
   and provider-home settings from child environments. Set test homes in the
   child environment only. Do not change the caller's home variables.
3. Start provider-specific mocks on OS-assigned `127.0.0.1` ports. Keep strict
   matching, no recording, no upstream, no retries, and bounded shutdown.
4. Keep Codex keyless and reject its Authorization header. Give Claude only
   the existing fixed, non-secret API-key placeholder. Reject other headers.
5. Audit Codex Responses calls and Claude Messages calls separately. With the
   current Claude pin, count its credential-free `HEAD /api/hello` probes
   separately from model calls.
6. Permit only the fixture tools. Reuse Claude `Read`. For Codex, retain the
   reviewed patch command and add a strictly validated fixture-file read if
   needed. Do not add a general shell-command response mode.
7. Run the execution phase on a runner that denies non-loopback network
   access. Install dependencies before this phase. Endpoint configuration and
   request audits alone do not prove the absence of other outbound traffic.
8. On timeout, stop owned child processes, close servers, and delete temporary
   state. Export only the sanitized reports defined below.

Never read real provider homes, `.env.local`, login files, or real Mallard
state. Use the existing Codex sandbox preflight. A preflight failure is an
invalid run with a fixed error message; it is not a passed or skipped test.

### Bounded scale support

Compile the scenario list before execution. Split it into provider-specific
batches of at most 64 model requests. Finish and audit each batch before the
next one starts. Keep both legs of a tool round in the same batch. Close the
Codex app-server at batch boundaries to flush native files.

Extend the test-only scenario format to carry a marker plus a bounded body.
Validate the body length and digest as well as the marker sequence. Update
both existing targets and mock unit tests to use one current format.

Proposed batch bounds are 4 MiB for the scenario document, 256 expected
history items per request, and 1,024 catalog markers. Retain finite request,
output, journal, and audit limits. Calculate their required values from the
compiled batch. Fail before spawning if a bound is exceeded. Do not disable
limits or increase production transcript limits to fit this test.

The scenario compiler must emit exact expected model, tool-leg, probe, native
fork, version-probe, and process counts. Unexpected, missing, duplicate, or
reordered calls fail the run. Mallard review and Apply must make zero model
requests and start zero provider processes.

## 4. Fixed data set

Use a versioned seed, such as `mixed-100-v1`, and stable test labels. Native
provider IDs remain provider-generated. Record their mapping to test labels
only in private runtime state. Never identify sessions by discovery order or
display title.

### 4.1 Codex graph: exactly 100 sessions

An arrow means “the child uses a prefix of the parent's physical history.”

| Labels | Count | Construction |
| --- | ---: | --- |
| C001–C020 | 20 | Independent native roots. |
| C021–C032 | 12 | One chain: C001 → C021 → … → C032. Maximum depth is 12 edges. |
| C033–C056 | 24 | Children of C002. Fork 12 after its early prefix and 12 after its later prefix. |
| C057–C084 | 28 | Four seven-child trees under C003–C006. Each root has one child, then two grandchildren, then four leaves. |
| C085–C092 | 8 | Forks of C060–C063 and C067–C070, in that order. |
| C093–C100 | 8 | One further fork of each C085–C092. |
| Total | **100** | **20 roots and 80 direct base edges.** |

For each seven-child tree, number the single child first, its two children
next, and the four leaves last. This fixes every edge without runtime choices.

Create all valid sessions and forks through the installed Codex CLI or
app-server. Do not write valid rollout JSONL by hand. Complete a child's first
turn before advancing its parent. After orderly shutdown, verify the durable
byte cutoff, global ordinal cutoff, parent ID, and prefix digest.

Keep an independent expected history for each label. Later parent or sibling
turns must not enter an earlier fork. Portable histories and pulled histories
must contain the complete selected prefix and tail. Pulled histories must be
standalone; they must not need A's physical base files. Do not require B's
physical file layout or provider IDs to match A's.

### 4.2 Claude data: exactly 20 sessions

| Labels | Count | Construction |
| --- | ---: | --- |
| L001–L010 | 10 | Independent sessions with several native resumes. |
| L011–L016 | 6 | Sessions with real `Read` calls and checked tool results. |
| L017–L020 | 4 | Long histories with repeated native resumes and paged detail checks. |

Give selected Codex and Claude sessions the same display titles and body
markers. Check identity by provider and binding. Include a foreign Claude
project whose encoded bucket name collides with the selected project's name.
Its transcript `cwd` must keep it out of the selected project.

### 4.3 Content volume

These sizes describe newly added assistant bodies. They exclude inherited
prefixes, native metadata, user messages, and tool records.

| Tier | Codex count | Claude count | New turns per session | Assistant bytes per turn |
| --- | ---: | ---: | ---: | ---: |
| Short | 60 | 10 | 4 | 1 KiB |
| Medium | 30 | 6 | 12 | 8 KiB |
| Long | 10 | 4 | 24 | 8 KiB |
| Total | **100** | **20** | **1,048 new turns** | **6,424 KiB of assistant bodies** |

Codex short labels are C001–C006, C021–C032, C057–C084, and C085–C098.
Medium labels are C007–C020 and C033–C048. Long labels are C049–C056 and
C099–C100. Claude tiers follow the three ranges above. Fork C002's early
children after turn 2 and its later children after turn 4.

Use deterministic paragraphs, JSON, multiline tool output, repeated strings,
path references, and fake credential cases. Add at least one tool round to
12 Codex sessions and the six Claude tool sessions. Tool continuation calls
are additional requests, not additional logical user turns.

Measure actual physical bytes, flattened bytes, record counts, and maximum
resolved history size. Cap each resolved fixture body at 256 KiB initially.
The provider preflight must prove full history reaches the mock with no
unexpected compaction or truncation. If the pin cannot support that size,
revise the documented fixture version before establishing a baseline.

Do not copy one native transcript 100 times or pad files after generation.
Keep native timestamps and IDs out of the fixture identity hash. Include the
seed, graph, body specification, codecs, and exact provider pins in that hash.

## 5. E2E test steps and expected results

Run these steps in order. Snapshot checks use private test data. A snapshot
must include native indexes, Mallard state, and storage state, not only JSONL.

| Step | Action | Required result |
| --- | --- | --- |
| 1 | Run environment and mock preflight. Create A, B, C, and the local store. | Exact version and route contracts pass. All roots are temporary. No external connection succeeds. |
| 2 | Finish setup on A with both provider profiles. Generate the full data set through native processes. | Exactly 100 eligible Codex and 20 Claude sessions. All 80 Codex base edges and all body digests match the scenario. |
| 3 | Add separate foreign-project and internal-subagent controls. Refresh inventory and comparison. | Controls are excluded. Both provider counts are exact. Equal titles do not merge sessions. |
| 4 | Select C032, C033, C045, C060, C067, C092, C099, C100, L011, and L017 for Push review. | Exactly ten selected conversations. Review captures no selected payload and makes no provider call. Provider files and remote content stay unchanged. Local review metadata may be saved. |
| 5 | Approve that exact selection and Push. Link B and review its Pull. | The eight Codex histories resolve through 31 distinct physical graph nodes. Exactly eight Codex and two Claude conversations are published. Unselected ancestors are not extra cloud conversations. Revision and digest pins remain enforced. |
| 6 | Approve Pull on B. Resume all ten histories in disposable B snapshots. | Correct complete history reaches the mock. Native resume uses B's paths and local IDs. A's graph files are not required. Authoritative B files do not change during verification. |
| 7 | Review and publish the complete selection on A. Pull the remaining content on B. | A and B converge on 120 cloud identities. Already accepted content is not duplicated. Provider and machine-local IDs map to the correct cloud IDs. |
| 8 | Read every page of Activity and Session Detail. Resume all 120 B histories in disposable snapshots. | No missing or duplicate history events across page boundaries. Ordered prompts, responses, tool calls, tool results, and body digests match independent expectations. |
| 9 | Save the validated 120-session checkpoint for performance samples. | The checkpoint includes the A native graph and the B standalone histories. No verification append is present in authoritative state. |
| 10 | Append different native turns on A and B to the first conflict group below. Publish A's side. Approve Keep both on B, publish its six preserved branches, and converge. | Exactly six conflicts produce six new branches. Both replicas now have 126 logical conversations. Unchanged sessions remain unchanged. |
| 11 | Repeat with the second, disjoint conflict group. Publish B's side, approve Keep both on A, publish the preserved branches, and converge. | Exactly 12 new branches exist across both rounds: 108 Codex plus 24 Claude, or 132 total. Every branch retains its local-only messages and correct cloud mapping. |
| 12 | Perform A → B → C → A path round trips for the 132 histories with explicit plans and approvals. At each hop, append a fixed native turn to C099 and L020 before publishing. | Each hop exercises encoding and decoding. Retained text returns to its original spelling on its original binding except for credential replacements. New turns match the scenario. Path tags do not accumulate. |
| 13 | Resume all 132 final histories from A and B snapshots. Stop source processes before each snapshot copy. | All 264 native resumes pass. Use copied homes because a native resume can append even in an ephemeral mode. |
| 14 | Repeat reviewed Push and Pull with no content change. Restart Mallard and repeat comparison. | No new storage revision, new logical branch, or provider payload write. Cached and restarted comparisons have the same semantic result. |
| 15 | Finish mock audits and scan all portable revisions and exported reports. | Exact request accounting passes. No known fake credential or forbidden mapped path is exported. Child processes and temporary files are removed. |

The first conflict group is C007, C033, C060, C099, L011, and L017. The second
is C008, C045, C067, C100, L012, and L018. Freeze this order in the scenario.
Do not resolve counts from whatever the product returns. Track logical
conversations, native aliases, retained physical bases, and cloud rows as
separate counts. A retained physical base is not automatically a new session.

Use fault fixtures only for controls and deliberate invalid-data cases.
Label these as injected fixtures. They do not count toward the 120 native
sessions and do not count as proof of a native subagent workflow.

## 6. Required path and credential assertions

### Path conversion

Test the following in both providers: transcript `cwd`, display text, nested
JSON strings, tool arguments, tool results, and supported file-path keys.

| Case | Required result |
| --- | --- |
| Absolute in-project path; root itself; nested file | Converts to the selected destination root. |
| Binding alias, canonical spelling, and supported `~/` spelling | Preserves the spelling tag and restores it on return to the same binding. |
| `file://` URI and percent-encoded URI | Preserves the supported URI form and suffix. |
| Spaces, Unicode, quotes around a path in text, and JSON escapes | Remains valid text or JSON and maps only the intended path span. |
| Relative path, external path, and similar prefix such as `project-extra` | Does not receive an in-project rewrite. External fixture files remain unchanged. |
| Literal path-map markers and text already naming B's root on A | Survives repeated transport as literal text. Local origin records prevent a later accidental rewrite. |
| Old prefix plus one new native turn after Pull | Re-encodes the accepted prefix without drift and maps the new turn once. |
| Missing or stale referenced local path-origin data | Blocks the affected thread with a bounded error. Does not silently rescan old text under a different interpretation. |

Use the current POSIX root contract. Rejected Windows-style roots and invalid
roots belong in negative tests. A project alias test must not permit symlink
transcript traversal or weaken no-follow checks.

Portable manifests must contain no machine-local paths, bindings, origin
files, plans, receipts, or trust state. Conversation bodies may retain
intentional external paths and literal examples. Apply path assertions to the
declared spans; do not ban every absolute path in conversation text.

### Credential redaction

Generate non-working canaries for supported provider-key patterns,
credential key/value pairs, Authorization text, credentialed URIs, connection
strings, and high-entropy secret cases. Use the current redaction rule tests
to fix the supported cases and expected typed replacements.

Place canaries in user text, assistant text, nested JSON, and real tool output
in both providers. Include repeated values, unique values, and new values in
post-Pull turns. Add controls for ordinary text, UUIDs, digests, and existing
replacement labels.

1. Prove each canary is present in its source record before Push. This prevents
   a false pass when a provider dropped the text.
2. Check every stored revision, object, manifest, and index for the known
   canaries, including fixture-declared JSON and URI encodings.
3. Check exact replacement labels in pulled content. Check that ordinary
   content, message order, tool call pairing, and path spelling remain intact.
4. Check a repeated Push/Pull cycle and redaction idempotence. Keep the
   ruleset fingerprint check and the ruleset-mismatch failure.
5. Confirm Push does not rewrite source transcripts. Local safety backups can
   contain original fake values; they must remain local and never be exported.
6. Scan sanitized logs, mock audits, failures, and benchmark artifacts before
   export. Emit case labels, offsets, and error codes, not matching text.

The independent expected-result table must contain fixed outcomes. Do not
compute all expected redactions or paths with the production codec under test.
Known-canary checks test specified rules; they do not prove universal secret
detection. Preserve existing content-warning acknowledgements.

## 7. Fault cases on copies of the validated data

Run each case from a fresh copy. Keep these cases out of timed samples.

- Remove a shared Codex base. Fail exactly the selected roots that need it;
  retain independent Codex and Claude successes.
- Corrupt a base prefix digest, byte cutoff, or global ordinal cutoff. Reject
  the affected history. Never import a shortened or empty replacement.
- Inject a cycle, an over-depth chain, malformed JSONL, a truncated record,
  an oversized input, or a symlink source. Prove bounded failure.
- Create a native fork after review but before Pull Apply. Preserve its base
  bytes when the tracked Codex history gets a fresh alias. In a separate case,
  append to a retained alias and prove that the branch becomes visible.
- Change an existing source between preparation and its Apply check. Fail
  only that thread and preserve successful siblings. Check the existing
  once-per-source rule. Do not claim protection against writes after that check.
- Change the remote head, plan revision, digest, or expiry after review. Reject
  the stale approval. Require a new review; do not retry Apply automatically.
- Reject missing warning approval, a wrong redaction fingerprint, or invalid
  path-origin data without partial changes to the affected thread.
- Corrupt a stored payload and inject a storage write/CAS failure. Preserve
  immutable published objects and accepted checkpoints for successful work.
- Reject an unknown mock route, wrong header, duplicate fixture call, or
  history mismatch. Confirm a nonzero result with a sanitized error.

Keep deletion disabled in the main flow. Existing storage-reset coverage can
remain in the smaller provider tests. This plan does not need a new destructive
cleanup path in the product.

## 8. Performance benchmark

### 8.1 Measurement boundary

Build the release test binary before timing. Generate native data once per
job. Measure fixture generation, mock startup, CLI startup, native resume, and
privacy scans separately from Mallard operations. Run only one benchmark job
on the runner at a time. Keep the operation order and worker setting fixed.

Restore the validated checkpoint at the same test paths before each mutable
sample. Stop all processes before restoration. Restore provider indexes,
Mallard state, and storage together. Start a fresh worker process for each
independent sample so watchers and global caches cannot leak across samples.

For warm samples, prime the state and wait for confirmed watcher quiescence
before timing. Bound that wait. A cold-process sample means empty process
caches; it does not mean an empty OS file cache. Do not purge the host cache.

Use monotonic wall time. Collect CPU time and peak RSS for the Mallard worker,
with explicit platform units. Report mock and provider process memory
separately. A sum of worker phase times is not operation wall time.

### 8.2 Operations

| Metric | Fixed sample |
| --- | --- |
| Cold discovery and comparison | Fresh process against the 120-session checkpoint; report provider subtotals. |
| Warm unchanged comparison | Same accepted data after cache and watcher priming. |
| Append comparison | One native leaf append for Codex and one for Claude, in separate samples. Prepare each append before timing. |
| Push review | Full unchanged selection and a fixed changed subset, measured separately. |
| Push Apply | First publication of all 120 sessions; then incremental publication of one and ten changed sessions. Reset state per sample. |
| Pull review | Empty destination and accepted destination, measured separately. |
| Pull Apply | Full 120-session restore; then one and ten updates; then the fixed Keep both subset. |
| Session Detail | First page and complete traversal of C032, C099, and L020. Fix page size and expected event counts. |
| No-op Push/Pull | Accepted checkpoint with no source mutation. |
| Full E2E duration | All correctness steps, including native processes. Report as a separate job-duration metric. |

Use root-scoped `MetricsSnapshot` deltas for both providers. Collect available
Pull preparation phase times, analysis counts, source-check work, queue peaks,
and source-byte reservations. Add test-only structured output for missing
metrics; do not add a product DTO for benchmark output.

Also record physical input bytes, resolved prefix bytes, flattened wire bytes,
output bytes, and storage operations/bytes. Instrument actual local store
operations if counts are needed; report unavailable metrics as unavailable.

### 8.3 Work limits that fail immediately

- At a settled warm fixed point, comparison does no full transcript read,
  selected capture, or full Codex graph resolution. Check the current quiet
  cache rules for both providers, including zero head/probe/append work.
- Push review performs no selected payload capture. Do not confuse metadata
  discovery with Apply capture.
- For a supported append-only native tail, comparison reads the append plus
  permitted probe work. Unchanged sibling histories receive no full capture.
  Structural native appends have separate expectations; do not call them an
  append fast path if they require resolution.
- Pull preparation performs the current two semantic analyses per prepared
  transcript. Each existing source gets the current single Apply check.
- Pull admission respects the configured worker cap and source-byte budget.
  Keep the current oversized-single-job rule. The source-byte budget is not
  a total RSS limit.
- No-op operations publish no new revision or payload. Safety checks still
  run where the production path requires them.

Report graph read amplification against both unique physical bytes and the
sum of selected history prefixes. Shared prefixes can be read for more than
one selected conversation. Do not assert one read per unique base unless the
implementation has that contract.

### 8.4 Baseline and regression rules

Use two discarded warm-up iterations and at least **20 measured samples per
operation and cache mode**. Report the median, nearest-rank p95, maximum, and
raw samples. Each sample must pass correctness checks. Do not calculate a
gating p95 from fewer samples.

Choose the first baseline only after this harness is implemented and passing.
Build the baseline revision and candidate revision with the same toolchain.
Keep the harness, fixture specification, and metric definitions fixed for the
comparison. Replay the same generated checkpoint for both. Alternate
baseline-first and candidate-first order across pairs. This limits host-load
and cache bias.

Identify a baseline by product and harness commits, fixture hash, exact
provider versions, aimock, Node, Rust toolchain, OS, CPU architecture/model,
RAM, filesystem/storage mode, build profile, and worker count. Record payload
sizes and graph statistics. Baselines require clean reviewed commits. Label
local candidate edits with a dirty flag and a source-change digest.
Reject mismatched environments or incomplete data as **invalid**, not passed.
Use dependency-closed 10/50/100 Codex subsets with 2/10/20 Claude sessions for
the weekly scaling report. Record their actual tier and graph mix; timing
ratios alone do not prove linear complexity when those mixes differ.

Initial proposed regression thresholds:

| Measure | Fail candidate when increase exceeds |
| --- | --- |
| Median, comparison and page reads | Greater of 20% or 5 ms over paired baseline median. |
| p95, comparison and page reads | Greater of 25% or 10 ms over baseline p95. |
| Median, Push/Pull operations | Greater of 20% or 100 ms over paired baseline median. |
| p95, Push/Pull operations | Greater of 25% or 250 ms over baseline p95. |
| Peak worker RSS | Greater of 15% or 32 MiB over the same baseline measure. |
| Exact work limits and correctness | Any violation. No timing tolerance applies. |

These are proposed limits, not measured performance claims. Use five stable
calibration jobs to check noise and approve the first baseline and thresholds.
Until then, report **uncalibrated** and retain measurements. Do not report a
performance pass without a valid baseline.

On a timing or RSS breach, run one further full paired batch. A repeated breach
is a regression. A non-repeated breach is an unstable result that still needs
review; do not silently turn it green. Never rerun to hide a correctness or
work-limit failure. Never replace the baseline automatically after a failure.

Keep baseline updates explicit. Record the reason, before/after data, and
review approval. Test provider-version upgrades separately before changing
the pinned periodic job. Do not mix debug and release results.

## 9. Commands, reports, and periodic runs

The following existing commands can validate the smaller local mock suites:

```sh
npm run test:aimock-servers
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

The new commands below are implemented. Both use one new runner
and the same scenario code. The benchmark command also runs correctness checks.
The runner owns gates, isolated homes, mock batches, and cleanup.

```sh
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e
```

Write reports under the existing ignored `output/` directory:

- `summary.md`: status, counts, failing step, and baseline comparison table.
- `results.json`: versions, fixture identity, graph sizes, raw timing samples,
  percentiles, RSS, CPU, work counters, and result status.
- `mock-audit.json`: per-provider counts, scenario labels, and mismatch codes.

Do not export native homes, raw mock journals, transcripts, prompt bodies,
credentials, local absolute paths, bundles, backups, or Mallard state. On a
failure, keep the seed, failing test label, sanitized code, and reproduction
command. Export is permitted only after report privacy checks pass.

Proposed schedule, to implement after approval of this plan:

| Trigger | Run |
| --- | --- |
| Pull request affecting sync or provider code | Mock unit tests, relevant integration tests, and a dependency-closed small mixed correctness run. |
| Nightly at 09:00 UTC | Full 100 Codex + 20 Claude correctness test and paired local-folder benchmark. |
| Weekly on Sunday at 11:00 UTC | 10/50/100 scaling, worker counts 1 and 4, and full local S3-stub correctness/transport measurements. |
| Before release or provider pin update | Full correctness, fault cases, benchmark, and pin qualification. |
| Manual dispatch | Same commands, fixture seed, pins, and baseline selection. |

Use a dedicated runner with both CLIs installed and no user provider data.
Use one concurrency group per benchmark host. Retain sanitized reports for
90 days and baseline metadata for the life of that baseline. Surface failed,
invalid, unstable, and missed scheduled runs through the CI job status.

Start with a 90-minute job watchdog and bounded per-process timeouts. This is
a hang limit, not a performance target. Record whole-job duration during
calibration and set a separate reviewed limit once measurements exist.

The implementation adds `.github/workflows/mixed-aimock-e2e.yml`. Scheduled
runs require that workflow on the default branch, a configured dedicated
runner, and a reviewed baseline. No local app automation is created.

## 10. Implementation order and completion checks

1. Extract only shared test helpers from the two existing live targets. Keep
   provider-specific process setup and native history checks separate. Keep
   both smaller suites passing.
2. Extend the strict mocks for bounded bodies and batches. Add boundary,
   history-mismatch, duplicate-call, and report-privacy tests.
3. Add a small mixed-provider flow. Prove setup, selected-leaf closure, Push,
   Pull, and native resume with both providers before increasing volume.
4. Add the exact 100/20 generator, graph oracle, body oracle, path matrix,
   canaries, conflict ledger, and fault copies.
5. Add structured benchmark results, checkpoint replay, paired sampling, and
   the comparator. Test the comparator with fixed sample vectors, including
   baseline mismatch, insufficient samples, and an injected slowdown.
6. Run calibration. Review the baseline and thresholds. Then add periodic CI.

Proposed file boundaries:

- `src-tauri/tests/live_mixed_aimock_e2e.rs`: scenario orchestration and gates.
- `src-tauri/tests/support/`: shared isolation, provider drivers, data
  generation, independent expected results, and benchmark reporting modules.
- `scripts/run-mixed-aimock-e2e.mjs`: preflight, run modes, and exit status.
- Existing provider mock scripts and their tests: one strict current format.
- `scripts/compare-mixed-aimock-benchmark.mjs`: result validation/comparison.
- `.github/workflows/mixed-aimock-e2e.yml`: manual and periodic execution.
- `package.json` and `README.md`: commands and operator instructions.

Completion requires all native histories to resume, all graph/path/redaction
checks to pass, every model request to be accounted for, fault isolation to
pass, reports to pass privacy checks, and the calibrated performance gate to
pass on the designated runner. A successful benchmark cannot replace any
correctness or safety check. Keep machine-local and portable schemas at 1.

At implementation handoff, run the mock tests, both existing live mock suites,
the new mixed suite, and relevant Rust tests. Run frontend build/integration
checks if a frontend or shared contract changes. Record any check that could
not run. This planning change requires document validation only.

**Claude owned-child sync: verification**

Date: 2026-09-08. Scope: direct Claude children and shared child evidence.
See [the implemented design](PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md).
Local state and cloud manifests remain at schema **1**. This is a clean
cutover. There is no migration, compatibility reader, or feature flag.

**Implemented behavior**

- One parent result record supplies the child key and native agent ID.
  Filter inherited references before exact-path lookup. Keep the first key
  when an agent resumes. No pending-call map or nested discovery is added.
- Validate session ID, agent ID, and `isSidechain` before child projection.
  The main parser continues to exclude sidechain records.
- Store each child's local and normalized state in the existing compact maps.
  Both providers use neutral child labels and the existing comparator/fold.
- Push reviews membership and captures current bytes at Apply. A new member
  makes review stale. A source change after capture fails that parent.
- Pull validates and restores exactly the listed files. Claude Create local
  and Keep both allocate a new parent ID and retain native agent IDs. An
  in-place update keeps installed identities and verifies reviewed digests.
- A structured agent reference inside a child omits all children. Push shows
  the existing information icon. Missing or unsafe files do not become omissions.
- A missing accepted local child keeps comparison Unknown and blocks Push.
  Valid cloud evidence permits reviewed Keep both recovery for both providers.
  Apply rechecks absence and retains the remaining local files.

No new durable metadata table is added. Cloud file entries, parent binding
installation maps, accepted child maps, and observation caches carry the state.
Only temporary review evidence adds a missing-child key set.

**Performance audit**

Removed repeated work:

1. Separate whole-document child redaction and certification passes. The
   required parse now performs gated redaction or verifies wire redaction.
2. A second JSON parse to calculate cold main and child cursor watermarks.
   The existing parse feeds the watermark.
3. An unused full raw-range hash and repeated content fold during child seeding.
4. A cold-child stamp read before the required snapshot read.
5. Recovery-map copies on rows without missing children, an unused cached main
   stamp, and a clone of captured child source maps.
6. A full main-chain recomputation just to rebuild cold child membership.
   The reference scan uses the existing main cursor checks. On cursor failure,
   full analysis reuses the bytes already read.
7. Provider-specific compact child labels and duplicate continuation checks.
   Native parsers retain their own contracts; comparison uses the shared rule.

Required checks remain: raw child identity before projection, exact-path
ownership, source stamps, object size/SHA-256, wire and output content evidence,
reviewed target digests, no-clobber writes, and journal/revision validation.
They check different boundaries. Removing them would change the safety contract.

The cache tests prove zero full transcript or append reads on a warm unchanged
Refresh. A child append does not read its unchanged sibling. Refresh reads
HEAD/manifest metadata and makes no child object download. New direct children
are found while parsing parent records; there is no scan of all Claude projects.

**Native Claude verification**

Command: `node scripts/run-claude-owned-sync-native.mjs`.
Claude version: **2.1.263**. The script uses temporary homes and projects,
a loopback mock, placeholder credentials, and explicit tools.

| Case | Parents | Direct children | Result |
| --- | ---: | ---: | --- |
| Synchronous | 2 | 3 per parent | Push/Pull and cold resume passed |
| Asynchronous | 2 | 3 per parent | Push/Pull and cold resume passed without source output artifacts |

Each restored child resumed through a fresh CLI process. The mock observed
its prior durable response. Source transcripts remained byte-identical.
The async test removed source external output artifacts before destination
resume. It does not claim to transfer a running process or temporary output.

Synthetic tests also cover inherited references, repeated results, identity
mismatch before projection, unsupported nesting, unsafe paths, incremental
thinking records, non-linear fallback, child-only updates, stale membership,
first-contact prefix proof, parent-scoped agent IDs, missing-child recovery,
and Clear storage removal of child objects while native files remain intact.

**Release performance**

Three alternating runs compare the checked-in revision `f9628b0` with the
current workspace. Each run uses 15 cold/warm samples and 7 samples for each
Apply/append stage. Each transcript has 128 added records with 2,048 text
bytes per record, about 0.3 MB per file. Storage is a local folder.

The baseline has the same main-only benchmark and fixture shape. It is the
checked-in revision, not an exact snapshot before this task; other existing
workspace changes can affect the comparison. These results measure the full
service path. The Rust test harness uses its deterministic `ManualScanner`,
which scans fixture contents on Refresh. That cost is included in elapsed
time, outside provider-read counters. These are not production-watcher or
remote-storage latency claims. No compiler or native QA run ran during samples.

Each table cell is **median / p95 in ms**. Each value is the median of the
three runs' corresponding statistic, not a pooled percentile.

The table below is the follow-up run that adds **8 children**. All four
counts were rerun three times with the same fixture and sample counts. All
benchmark assertions passed. The earlier paired baseline check follows
separately; it was not rerun for this extension.

| Stage | No children | 1 child | 8 children | 16 children |
| --- | ---: | ---: | ---: | ---: |
| Cold Refresh | 18.02 / 23.14 | 22.23 / 25.35 | 31.94 / 34.12 | 47.83 / 52.09 |
| Warm Refresh | 9.50 / 10.67 | 8.66 / 10.32 | 8.53 / 10.66 | 10.59 / 12.98 |
| Append + Push Review | 42.60 / 48.36 | 43.69 / 50.82 | 50.08 / 55.30 | 61.80 / 73.89 |
| Push Apply | 116.28 / 123.26 | 118.20 / 130.46 | 182.60 / 188.81 | 261.88 / 269.00 |
| Pull Apply | 104.71 / 107.55 | 126.84 / 137.01 | 222.94 / 273.25 | 347.50 / 371.92 |

Append + Push Review changes one child, or the main in the zero-child case,
and prepares the next review. It is not an append-only parser microbenchmark.
Pull Apply measures Create local with a fresh destination parent namespace.

**Earlier paired main-only check**

| Main-only stage | Baseline median, ms | Current median, ms | Change |
| --- | ---: | ---: | ---: |
| Cold Refresh | 17.24 | 16.69 | -3.19% |
| Warm Refresh | 8.85 | 9.05 | +2.20% |
| Append + Push Review | 43.04 | 43.53 | +1.13% |
| Push Apply | 108.54 | 113.95 | +4.99% |
| Pull Apply | 111.98 | 112.38 | +0.36% |

The paired aggregate meets the 5% main-only target. Small differences remain
subject to scheduling and filesystem noise. Before the last cursor change,
main-only cold Refresh was 10.1% slower in the paired aggregate. The final
path removes that repeated chain analysis without new persistent metadata.

Reproduce the current benchmark with:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --lib benchmark_claude_owned_subagents -- --ignored --nocapture --test-threads=1
```

**Automated checks**

- Backend library: 286 passed, 5 opt-in tests ignored in the default run.
- Domain library: 85 passed.
- Infrastructure library: 510 passed, 5 opt-in tests ignored.
- Frontend integration: 291 passed.
- Frontend build: passed. The existing Vite chunk-size warning remains.
- `git diff --check`: passed.

The opt-in native bridge and performance benchmark were run separately.

**Native Mallard UI check**

The temporary QA project had one accepted parent with two children, then one
local child was removed. Pull review exposed the missing key and selectable
Keep both action. Apply completed. The resulting fixture had two parent files
and three child files: the original surviving child and both restored children.
The original missing path remained absent. Review and result screenshots were
captured in the implementation task. The QA app was then closed.

The test found and fixed a UI filter that hid selectable recovery rows because
their comparison status was Unknown. Frontend coverage now tests recovery rows
with and without an activity-history row.

Isolation note: the first QA launch used normal Mallard state because the app
launcher dropped the temporary environment. It was closed before any UI action
or sync. A wrapper then pinned all homes on every launch. All later UI actions,
Rust tests, native Claude tests, and benchmarks used synthetic temporary data.

# Measurements: Sync Path Performance

- Status: measurements. Two findings have since been implemented — the
  dependency-only dev profile and the fused single parse (D7); both are marked
  below with their verified results. The rest remain proposals.
- Measured: July 31 – August 1, 2026
- Scope: Push review, thread refresh, and Pull planning in `ProjectSync` —
  provider discovery, local capture, thread classification, and storage reads
- Method: temporary read-only instrumentation over an anonymized local
  benchmark corpus and isolated local-directory storage. All machine-specific
  paths and identifiers are scrubbed; the instrumentation has been removed.
- Related: [DESIGN_FAST_PUSH_PULL.md](DESIGN_FAST_PUSH_PULL.md) (proposed, not
  implemented — it already targets most of what is measured here),
  [DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md](DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md)

## Summary

The reported 19.8s Push review is **~93% an artifact of running an unoptimized
dev build**. The same work costs ~1.5s in a release build. The algorithmic
findings below are real and worth fixing, but they are a scaling concern, not
the cause of today's latency.

**The unbounded capture set is not only a performance issue** — it terminates in
a hard failure at 512 MB that breaks refresh, Push review, and Pull planning
simultaneously. See "The 512 MB cliff" below. The measured bindings sit at 14.5%
and 14.7% of that ceiling, roughly six months out at the observed growth rate.

Ranked by payoff per unit of effort:

| # | change | effect | cost | state |
| - | --- | --- | --- | --- |
| 0 | bound the capture set (see the cliff) | prevents a total sync outage | design work | proposed |
| 1 | `[profile.dev.package."*"] opt-level = 1` | 3.9× faster capture | none measurable | **landed** |
| 2 | single-pass secret scan | ~27% off capture (1.5× on ~80% of per-byte cost) | contained, one function | proposed |
| 3 | fuse the double JSONL parse (D7) | −36% to −60% of analysis; ~10–15% of capture | contained, one function | **landed** |
| 4 | memoize derivation (D2) | skips analyze **and** the secret scan for unchanged files — ~87% of per-byte work | new on-disk store | proposed |
| 5 | hoist the chain digest out of the candidate loop | removes the 4.1s Push-review classify phase | a few lines | proposed |
| — | whole-crate `opt-level = 1` in dev | 13× faster capture | 8.6× slower rebuilds | rejected: rebuild cost |

End-to-end effect of the two landed changes: the reported capture fell from
15.652s to ~3.3s and refresh from ~18.4s to ~3.5s, about 4.5×.

## The 512 MB cliff

`MAX_CAPTURE_BYTES = 512 * 1024 * 1024`
([provider_capture.rs:26](../src-tauri/src/project_sync/provider_capture.rs#L26)).
`capture_from_discovery` accumulates `total_bytes` across every selected
resource and returns `Err("selected resources exceed 536870912 bytes")` the
moment it crosses. There is no warning band, no partial result, and no
degradation path.

Because the capture set is unbounded (finding 3 below), this ceiling is
approached monotonically, and all three callers of the full-fan capture fail
together — thread refresh, Push review, and Pull planning. The user-visible
result is not slowness but an inability to see or sync any thread.

Exact capture volume per anonymized project binding, measured by running
discovery and capture for each binding:

| fixture | profile | conversations | captured | share of ceiling |
| --- | --- | --- | --- | --- |
| A | default profile | 7 | 74.2 MB | 14.5% |
| B | alternate profile | 10 | 75.4 MB | 14.7% |

Both bindings target the same anonymized checkout but use different Codex
profiles, so they capture disjoint session sets.

Growth: fixture B accumulated its entire 75.4 MB during its first measured
month. At ~75 MB/month the ceiling is roughly six months away, not imminent —
but nothing in the code slows the approach, and the failure when it arrives is
total rather than gradual.

A separate 288.5 MB / 61-conversation figure appears elsewhere in this document.
That came from a **synthetic** capture request pointed at the `mallard-app`
directory, which is not a registered project. It is useful as a scaling data
point and as a preview of what a long-lived project looks like, but it does not
describe either fixture's registered sync state.

Three properties make the failure harder to escape than it appears:

1. **Archiving does not reduce the set.** `discover_codex` walks both
   `sessions` and `archived_sessions` and captures from both, so archiving a
   Codex session leaves the captured total unchanged.
2. **Recipe deselection does not reduce it either.** The review path builds its
   selection from every discovered Codex conversation and ignores the recipe
   (the measured bindings hold 7 and 14 recipe entries against 61 discovered
   conversations in the measured scope). Push is recipe-scoped and would stay
   small — but the review that must precede it fails first.
3. **Nothing prunes.** No window, no LRU, no cap-with-degradation. The only
   remedy available to a user today is deleting session files.

Per-file limits are not close: `MAX_CONVERSATION_FILE_BYTES` is 100 MB and the
largest transcript measured is 26.7 MB.

## Environment measured

- App under test at the time of the original measurements:
  `src-tauri/target/debug/mallard`, an unoptimized dev build — no `[profile]`
  section existed in `src-tauri/Cargo.toml`, so dev builds ran at
  `opt-level = 0`. `scripts/tauri-build.mjs` builds `release`, so **shipped
  users never saw this**; it was a dev-environment cost only. A
  `[profile.dev.package."*"] opt-level = 1` section has since been added, so
  figures labelled `opt-level = 0` are historical.
- Benchmark session corpus: 2.36 GB across 522 `.jsonl` files, with 126 files /
  378 MB matching the synthetic `mallard-app` scope.
- Registered-fixture setup: two anonymized bindings targeting one
  **local-directory** test store under an isolated temporary root. No network
  is involved in any measurement below.

## The build profile dominates everything

Identical work — capture of 61 conversations / 288.5 MB — across four profiles.
"Rebuild" is a steady-state incremental `cargo build --lib` after touching
`commands.rs`.

| dev profile | capture | s/MB | rebuild | verdict |
| --- | --- | --- | --- | --- |
| `opt-level = 0` (current) | 36.3s | 0.126 | 9.9s | today |
| deps only at `1` | 9.25s | 0.032 | 9.35s | **strictly better — nothing gets slower** |
| whole crate at `1` | 2.76s | 0.0096 | 84.8s | 13×, at 8.6× slower edits |
| `--release` | 2.78s | 0.0096 | — | what ships |

Two conclusions:

1. `profile.dev.package."*".opt-level = 1` optimizes dependencies only. Deps
   compile once and are not rebuilt on edits, so the 3.9× is free.
2. `opt-level = 1` matches `--release` exactly. `3` is unnecessary.

The remaining 3.4× between deps-only and whole-crate lives in Mallard's own
code — line splitting, per-record identity extraction, and chain assembly in
`analyze_codex_transcript`. `std` already ships optimized, so the secret scan
was never affected by the profile.

Neither option requires a `Cargo.toml` commitment to experiment:

```
cargo <cmd> --config 'profile.dev.package."*".opt-level=1' --target-dir <dir>
```

## Thread refresh — measured

Refresh (`get_project_thread_sync_comparison_with_progress`) against the
anonymized local benchmark state, deps-optimized build, two bindings:

```
  load config and binding               0.005s       0.001s
  discover local state                  0.088s       0.012s
  inspect remote head                   0.006s       0.002s
  capture local conversations           4.217s       3.838s   <- 97-99%
  classify threads and build actions    0.013s       0.014s
  TOTAL                                 4.329s       3.868s
                                       (7 threads)  (10 threads)
```

Scaling this by the measured deps→`opt-level 0` factor of 3.9× gives ~16.5s of
capture, against the **15.652s actually observed** in the app log, for a run
that also reported 7 threads. The model reconciles with the reported numbers.

These two runs capture 74.2 MB and 75.4 MB respectively (exact figures above).

### Per-MB cost is not a constant

The same code path at the same optimization level:

| scope | capture | s/MB |
| --- | --- | --- |
| fixture A | 4.217s / 74.2 MB | 0.0568 |
| fixture B | 3.838s / 75.4 MB | 0.0509 |
| `mallard-app` (synthetic) | 9.251s / 288.5 MB | 0.0321 |

A 1.8× spread, so per-MB rates in this document are order-of-magnitude
indicators, not a formula to extrapolate with. Record shape drives it: cost
tracks record count and per-file overhead as much as raw bytes. The
profile-ratio comparisons (13×, 3.9×) are unaffected — those compare the same
workload against itself.

Refresh cost for this project across profiles:

| profile | refresh total |
| --- | --- |
| `opt-level = 0` (today) | ~16–18s |
| deps only at `1` | 4.3s |
| whole crate at `1` / release | ~1.2s |

Note that refresh's classify phase is **0.013s**, not the 4.1s seen in Push
review. Refresh passes `build_push_actions = false` and skips binding recovery
entirely.

## Push review — attributed

Observed phases (`opt-level = 0`):

```
  inspect remote head                    0.004s
  capture local conversations           15.652s
  classify threads and build actions     4.119s
  thread comparison and push plan       19.783s
  project files inventory                0.000s
```

### Capture (15.652s)

`capture_from_discovery` selects **every** discovered Codex conversation for the
project, not the threads the review displays. Per-byte stages, measured
separately over the same payloads at `opt-level = 0`:

| stage | s/MB | share |
| --- | --- | --- |
| `analyze_codex_transcript` — `serde_json::Value` per JSONL record | 0.089 | 57% |
| `resource_digest` — SHA-256 over file bytes | 0.055 | 35% |
| `detect_secret_material` — lowercase copy + substring scans | 0.010 | 6% |

Deep-cloning the whole captured set costs 0.05s, so the per-file `bytes.clone()`
is a memory concern (~2× peak residency), not a time one.

### Classify (4.119s)

`recover_unique_compatible_binding_from_meta`
([conversation_sync.rs:346](../src-tauri/src/project_sync/conversation_sync.rs#L346))
iterates every cloud manifest candidate per unbound local conversation, and each
iteration calls `classify_thread_refresh`, which begins by recomputing
`conversation_chain_digests` over the **entire** local chain — a value identical
for every candidate.

Measured cost of one such call (`opt-level = 0`):

```
 26.7 MB / 3075 messages   74.8 ms
 21.4 MB / 1965 messages   39.4 ms
 18.1 MB / 1680 messages   40.7 ms
 17.9 MB /  991 messages   16.9 ms
 17.4 MB / 1619 messages   34.5 ms
 16.4 MB / 1312 messages   29.8 ms
                    mean   39.4 ms
```

4.119s ÷ 39 ms ≈ **~105 calls**. Hoisting the candidate-independent digest out
of the loop is a pure win with no semantic change, and is not covered by any
existing design document.

## Pull — not measurable in this fixture

Pull planning could not be measured against the benchmark state: the isolated
storage directory contains only `_storage.json` and a lock file.
`inspect_from_storage` reports that the anonymized bundle does not exist in
8–10 ms for both bindings, even though both registrations record a
`recipe_bases` entry at generation 1 — the bundle was published at some point
and the storage has since been cleared.

This also explains the 0.004–0.006s "inspect remote head": a local-directory
storage returning not-found, not a fast network round trip.

`plan_bundle_restore_with_progress` was deliberately **not** run end-to-end: it
calls `repository.acquire_host_operation()` and `repository.save_restore_plan()`,
so benchmarking it would contend with the running app for the host-operation
lock and write plan state.

What can be said structurally — its phases are:

```
  inspect remote head and manifest
  build file restore plan
  classify and build pull actions
  finalize and save review        <- writes
```

Its local half is the same `discover_full` + capture-everything as refresh
([commands.rs:7251](../src-tauri/src/project_sync/commands.rs#L7251)), so it
inherits the same ~4.2s / ~16.5s capture cost, plus manifest and transcript
reads proportional to what it can offer to materialize.

To measure Pull for real, either push once so a bundle exists, or build a
synthetic bundle in an isolated temp home and plan a restore against it.

## Structural findings

### 0. Measurement method — read this before quoting any per-MB number

Figures in the first pass of this document were **single-run**: each file was
read and then immediately analyzed, so its bytes were hot in cache. A repeated
benchmark that cycles the whole corpus five times gives materially different
absolute numbers for identical code:

| `analyze_codex_transcript`, same corpus, same profile | s/MB |
| --- | --- |
| single-run, cache-warm (original method) | 0.0122 |
| 5-run min / max (repeated method) | 0.0192 / 0.0246 |

The single-run method understates cost by 1.6–2×. Ratios between stages held up
better than absolutes, but **per-MB figures elsewhere in this document carry
that weakness** and should be treated as indicative. Comparisons between two
code versions are only meaningful when both sides are measured by the same
method in the same session — see the D7 verification below for the format that
works.

### 0b. D7 (fused parse) — verified

The fused single-parse change was implemented against the 37% estimate below.
Measured like-for-like — `HEAD` in a detached worktree versus the working tree,
identical benchmark, same session, 5 runs each, 148.1 MB corpus:

| | before D7 | after D7 |
| --- | --- | --- |
| standalone structure driver | min 0.0065 / max 0.0073 | min 0.0044 / max 0.0121 |
| `analyze_codex_transcript` | min 0.0192 / max 0.0246 | **min 0.0076 / max 0.0157** |

**Min-to-min −60%, max-to-max −36%**, against a predicted −37%. The change
exceeds its estimate. Variance on the new code is wider, so the steady-state
saving sits somewhere in that band rather than at a single figure.

An intermediate result claiming D7 made analysis **11% slower** was recorded
during this investigation and is **withdrawn** — it compared a single run of the
new code against a single run of the old taken hours earlier. There was no
regression.

### 0c. Where capture time goes now (deps-optimized + D7)

Measured on the anonymized benchmark corpus, 88.0 MB over 18 transcripts, min
of 3 runs, with a secret scan carrying **10 synthetic markers** — the shipped
set is ~13, so its real cost is at least this:

| stage | s/MB | share | s/MB at `opt-level = 0` | speedup |
| --- | --- | --- | --- | --- |
| secret scan | 0.0301 | **~77%** | 0.010 | 0.3× |
| analyze | 0.0067 | ~17% | 0.089 | 13× |
| sha256 | 0.0023 | ~6% | 0.055 | 24× |

This reconciles with the end-to-end figure: 0.0391 s/MB of per-byte work plus
file reads is close to the 0.0435 s/MB implied by capture at 3.28s for 75.4 MB.

The ordering has inverted. The secret scan was 6% of per-byte cost at
`opt-level = 0` and is now the dominant stage — not because it regressed, but
because it is the only one that did not benefit from the profile change. `sha2`
and `serde_json` are dependencies and are now optimized; `detect_secret_material`
is `from_utf8` + `to_ascii_lowercase` + `contains`, all of which live in `std`,
which already shipped optimized.

**Caution on an earlier figure:** a first pass at this table used a 2-marker
stand-in and reported the secret scan at 0.0079 s/MB / 46.7%. Marker count
drives the cost almost linearly, so that understated it by 3.8×. Any future
measurement of this stage must use the full marker set.

#### What a faster scan is actually worth

Three variants over the same corpus, verdicts verified identical on every
transcript (0 mismatches; no transcript in this corpus contains a marker, so
every scan runs to completion — the worst and most common case):

| variant | s/MB | vs current |
| --- | --- | --- |
| A — current: lowercase copy + 10 `contains` | 0.0301 | 1.0× |
| B — no allocation, 10 `windows().eq_ignore_ascii_case()` passes | 0.1495 | **0.2× (5× slower)** |
| C — single pass, 256-entry first-byte table, no allocation | 0.0152 | **2.0×** |

The obvious fix is the wrong one: **B is five times slower**. `str::contains`
uses an optimized substring search, so ten of those over a lowercase copy beat
ten naive byte-wise window scans easily. The allocation is not the bottleneck,
and the current implementation is less naive than it appears.

The realistic ceiling is C's ~2× on a stage worth ~70% of capture — about **35%
off capture time**, from a contained change that needs no new dependency and
leaves the marker semantics untouched. C builds its dispatch table per call
here; hoisting it to a `const` would improve on that slightly.

Consequence for D2: the derivation store as designed holds the **secret-scan
verdict** alongside the chain and digests, so a cache hit on an unchanged file
skips both analyze and the secret scan — 87% of per-byte work — leaving only the
read-and-hash that keys it. That is a larger share than it was when parse
dominated, not a smaller one.

### 1. Every transcript is JSON-parsed twice

`analyze_codex_transcript`
([conversation_codec.rs:88](../src-tauri/src/project_sync/conversation_codec.rs#L88))
calls `analyze_codex_rollout_structure` — a full pass that parses every record
into a `serde_json::Value` — then runs its own second full pass over the same
lines. Measured over 148 MB of the largest transcripts, deps-optimized:

```
structure pre-pass alone     0.672s   (0.0045 s/MB)
analyze_codex_transcript     1.803s   (0.0122 s/MB)  [includes the pre-pass]
pre-pass share:              37%
```

A fused single pass recovers about 37% of analysis time. This is the largest
slice of the 3.4× that dependency optimization cannot reach.
`DESIGN_FAST_PUSH_PULL.md` already names this: D2 removes "O(threads) JSONL
parses per operation, ×2 per Push".

### 2. The full-fan capture repeats across entry points

Three call sites capture **every** discovered Codex conversation regardless of
what the caller needs:

- `get_project_thread_sync_comparison_with_progress` — every page load, binding
  revision change, storage switch, and post-push/pull refresh
- `plan_bundle_restore_with_progress` — Pull planning
- `prepare_pull_conversation_actions` via `capture_local_codex_conversations`

`push_bundle_reviewed_with_progress` is **not** in this list — it calls
`capture_recipe_from_discovery`, which is scoped to the push recipe.

No cache exists on this path. One does exist —
[`cached_transcript_analysis`](../src-tauri/src/project_sync/commands.rs#L829),
keyed by transcript sha256 — but its own comment records that its sole caller is
Pull planning's stored-subagent refusal.

### 3. The capture set has no upper bound

`discover_codex`
([provider_capture.rs:1125](../src-tauri/src/project_sync/provider_capture.rs#L1125))
takes every session whose cwd sits under the project root, with no time window.
Discovery itself is cheap — a bounded header scan, 0.4–0.7s for 522 files — but
capture then reads all of them in full, forever, growing with history.

By contrast the chat-history view bounds itself to 30 days
(`DEFAULT_HISTORY_WINDOW_DAYS`, clamped 1–90). Two features read the same tree
with very different scoping discipline.

This is what terminates in the 512 MB cliff above.

### 3a. The capture cost is paid on UI navigation, not only on state change

`ProjectChatHistoryPage` reloads its comparison on
`[project.local_project_id, binding?.revision, activeStorageId, refreshEpoch,
comparisonOverride]`, and `loadComparison` calls `setComparison(null)` before
each load, so the previous result is discarded rather than shown stale while the
new one computes. There is no cache above it. Each of these re-pays the full
capture — 4.3s optimized, ~16s at `opt-level = 0`:

- switching storage on the same project
- switching projects and returning
- opening and closing storage settings, or the new-project setup: the panel
  renders under `activeProject && !newProjectSetup && !inlineStorageReview`
  ([ProjectLinksWorkspace.tsx:1419](../src/components/project-sync/ProjectLinksWorkspace.tsx#L1419)),
  so a side trip **unmounts** it and returning remounts and refetches
- any `binding.revision` bump
- the four `setHistoryRefreshEpoch` sites — project load, post-push,
  post-pull-apply, post-conversation-apply

The post-push one is notable: the Push review already computed a fresh
comparison, and the epoch bump immediately discards and recomputes it.

The Push review path itself is well behaved — `PushResourceWorkspace` passes
`comparisonOverride={threadComparison}`
([PushResourceWorkspace.tsx:601](../src/components/project-sync/PushResourceWorkspace.tsx#L601))
and `loadComparison` short-circuits on it, so mounting the history view inside a
review reuses the review's comparison instead of running a second capture.

None of this is incorrect — it is the correctness-first design of recomputing
from bytes. It is the *when* dimension that
[PLAN_PROJECT_LEVEL_REFRESH_PERFORMANCE.md](PLAN_PROJECT_LEVEL_REFRESH_PERFORMANCE.md)
was written for, and it compounds with the *what it costs* dimension measured
here. With a derivation cache (D2) in place, a remount would cost a stat and a
hash rather than a full parse.

### 3b. One capture per page load, three tree walks

Checked and ruled out: opening a project does **not** multiply the capture cost.
`get_project` and `get_bundle_readiness` perform no discovery, and
`get_bundle_inventory` calls `discover_full` without capturing. Exactly one
full-fan capture runs per page load, from the thread comparison.

The bounded header walk over the 522-file tree does happen three times per page
load, though — inventory discovery, comparison discovery, and the chat-history
scan — at roughly 0.4–0.7s each for the first two.

### 4. The history path already has the cache the sync path lacks

[`chat_history.rs`](../src-tauri/src/project_sync/chat_history.rs#L700) keeps a
persistent `chat_history_cache.json` in `~/.mallard` (339 KB currently) with two
maps keyed by `codex_home + path`:

- `ownership` → `{size, modified_nanos, identity}` — project membership without
  opening the file
- `entries` → `{size, modified_nanos, parsed}` — the full parsed rollout

Both validate `size` and `modified_nanos` before use. This is why the history
scan takes ~3s over the same tree that costs capture 15.6s.

It is a working precedent for persistence format and invalidation, but it is
**not directly transferable**: `DESIGN_FAST_PUSH_PULL.md` explicitly rejects
stat-only keys for the sync path, because there a stale entry would decide a
divergence, whereas in history it only decides what is displayed. Hence D2's
content-addressed key. `DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md:130` pre-sanctions
the sync-path key as `(transcript path, size, file sha256)` with the rule "file
identity is a cache key, never a divergence witness".

### 4b. Publish and payload reads — measured

Seven anonymized benchmark transcripts published into temporary local storage,
then read back. Two runs, chosen so one payload set fits
`OBJECT_CACHE_MAX_BYTES` (256 MB) and one does not:

| | 73.0 MB bundle | 558.3 MB bundle |
| --- | --- | --- |
| publish (Push upload half) | 1.292s (56.5 MB/s) | 8.579s (65.1 MB/s) |
| inspect head + manifest | 0.001s | 0.001s |
| read all payloads, cold | 0.336s (217 MB/s) | 2.579s (216 MB/s) |
| read all payloads, cache warm | **0.000s (3362×)** | **2.563s (1×)** |

Manifest reads are free and payload reads run at disk speed, so on a
local-directory storage the Pull read half is not a cost centre — capture
dominates Pull exactly as it dominates refresh. On S3 these become network
transfers and the balance inverts.

**The object cache is all-or-nothing.** Below 256 MB of payloads it is a 3362×
win; above it, LRU eviction thrashes and the benefit is exactly zero — the
second pass misses on every object and pays insert-and-evict overhead for
nothing. There is no gradual degradation.

Two consequences worth noting:

- The capture ceiling is 512 MB but the object cache is 256 MB, so there is a
  256–512 MB band where sync still functions but every payload read is uncached.
  On a remote store that means re-downloading the whole bundle on each pass.
- The measured projects (74 MB, 75 MB) sit comfortably under the cache limit.

The 558 MB sample is the seven largest complete-lineage transcripts in the whole
Codex tree, not a real project bundle. Note in passing that publishing it
succeeded: `publish` does not route through `capture_from_discovery` and so
never sees the 512 MB capture ceiling.

### 4c. S3 operation counts — measured against the stub

Counted with `StubS3`'s request log, publishing a **fresh** bundle at three
sizes and then reading every conversation payload back:

| threads | Push ops | Push GET | Push PUT | Pull inspect | Pull read GETs |
| --- | --- | --- | --- | --- | --- |
| 1 | 10 | 4 | 6 | 3 | 2 |
| 5 | 12 | 3 | 9 | 3 | 6 |
| 20 | 27 | 3 | 24 | 3 | 21 |

Reading the scaling:

- **Push GETs are constant at 3** — they do not grow with bundle size.
- **Push PUTs are ≈ N + 4** — one object per conversation plus fixed overhead.
- **Inspect is a flat 3 operations** at any size.
- **Pull payload reads are ≈ N + 1 GETs** — one per object plus one.

Nothing here is quadratic, and per-object costs are near the theoretical
minimum. The cost that remains is **sequencing**: those N + 1 payload GETs are
issued one at a time through `byte_source.get()`, which is exactly the
`get()`-loop pattern D6 targets. Counts are latency-free against an in-process
stub, but against a real endpoint at 30–50 ms per round trip, 21 sequential
GETs is 0.6–1.0s of pure waiting for a 20-thread bundle, where a prefetched
batch at 16-way concurrency would be a couple of round trips.

**What this does not establish:** every publish here targeted a fresh bundle
(`PublishExpectation::Absent`, no previous manifest), so no retained-entry
verification ran. D4's "O(bundle) GETs per Push" concerns exactly that retained
path on an existing bundle, which these numbers never exercise. Treat D4 as
unverified rather than contradicted.

### 4d. The S3 store must be called from a blocking thread

`S3BundleObjectStore::run` calls `runtime.block_on` and wraps it in
`catch_unwind`, converting the panic into: *"S3 bundle-store {op} cannot block an
async runtime worker; run the enclosing bundle-engine operation with
tokio::task::spawn_blocking"*
([s3_store.rs:101](../src-tauri/src/project_sync/s3_store.rs#L101)).

Production satisfies this through `run_blocking`. It is worth recording because
the constraint is enforced at runtime rather than by the type system, and it is
invisible on a local-directory store — a refactor that moved engine calls onto
async paths would compile, pass every local-storage test, and fail only against
S3. D6's concurrency work touches exactly this boundary.

`StubS3` records every request with method, key, conditional flag and status, so
round-trip properties are expressible as assertions, not just measurements. If
D4 and D6 land, "publish issues no read-back GETs" and "a known object set goes
through `get_many`" can both be locked in as tests.

### 5. Object cache is memory-only; transfer concurrency is 6

[`ObjectCache`](../src-tauri/src/project_sync/bundle_engine.rs#L525) is a
`Mutex<HashMap<…, Arc<Vec<u8>>>>` capped at 256 MB in a process-level
`OnceLock` — it evaporates on restart, so review → quit → apply re-reads
everything. `PREFETCH_BATCH_OBJECTS`, `UPLOAD_BATCH_OBJECTS`,
`BULK_GET_CONCURRENCY` and `BULK_PUT_CONCURRENCY` are all 6. D5 and D6 target
both; neither is implemented. Neither matters for the latency reported here —
this storage is a local directory.

## Relationship to the existing design

`DESIGN_FAST_PUSH_PULL.md` (proposed, reviewed the same day) already covers most
of the structural findings:

| finding | decision |
| --- | --- |
| double parse | D2 — "One derivation per byte string… ×2 per Push" |
| repeated full-fan capture | D2 + D3 |
| ~2× byte residency from per-file clones | D3 — "capture holds handles, not bytes" |
| no parallelism across 61 independent transcripts | D6 — "local derivation runs across cores" |
| unbounded capture set | its stated goal — cost proportional to what changed |
| memory-only object cache | D5 |
| concurrency 6 | D6 |

Nothing of it is in the code. Two ideas that surfaced during measurement are
already considered and rejected there, with reasons worth preserving:
prefix-anchored incremental parsing (deferred — saves ~1 parse per operation,
costs a cache-format commitment to `analyze_codex_transcript`'s fold state) and
stat-only cache keys (rejected — a rewrite preserving size and nanosecond mtime
would let a stale chain decide a divergence).

The one finding **not** covered by any design is the hoistable chain digest in
`recover_unique_compatible_binding_from_meta`.

What these measurements add to that design is sequencing: at release speed the
entire 288 MB capture costs 2.8s, so D2/D3/D6 are worth roughly 1–2s per
operation today, growing linearly with history. The profile change is 13× for
near-zero effort and should land first — otherwise every future measurement
taken in dev is distorted by a large constant.

## Reproducing

All benchmarks were temporary `#[test] #[ignore]` functions appended to the
crate and removed afterwards. To repeat:

- Capture stages: call `discover_full` + `capture_from_discovery` with a
  `CaptureRequest` for the project, timing `analyze_codex_transcript`,
  `resource_digest`, and `detect_secret_material` over the captured payloads.
- Refresh: call `get_project_thread_sync_comparison_with_progress` with a
  `LocalStateRepository::from_home_dir(<real home>)` and print the `on_phase`
  callback. Read-only.
- Profiles: `--config 'profile.dev.package."*".opt-level=1'` and
  `--config 'profile.dev.opt-level=1'` with a separate `--target-dir`, so the
  normal debug cache is untouched.

## Corrections made during this investigation

Recorded so the reasoning can be audited:

- An initial claim that Push re-captures everything after review was **wrong** —
  `push_bundle_reviewed_with_progress` is recipe-scoped.
- An initial 288 MB / 61-conversation figure came from a synthetic capture
  request pointed at the `mallard-app` directory. The registered fixtures
  capture ~130 MB. The code path is identical; the scope is not.
- A first "deps-only rebuild = 63s" measurement was contaminated by profile
  switching in a shared target directory. Steady state is 9.35s.
- The 4.1s classify phase applies to Push review only, not refresh.
- The capture ceiling was first reported as "56% consumed, roughly a month of
  runway". That described the synthetic `mallard-app` scope. The registered
  projects are at **14.5% and 14.7%**, with roughly six months of runway at the
  observed rate. The cliff is real; its imminence was overstated.
- Per-MB capture cost was initially treated as a constant and used to
  extrapolate byte volumes from timings. Measured rates vary 1.8× across scopes,
  so byte volumes in this document are measured directly rather than derived.
- A shell header-grep suggested the benchmark corpus held no target-project
  sessions. It scanned only the first 3 KB of each file and was wrong;
  discovery finds seven. Byte and count figures here come from the full
  benchmark discovery and capture.
- Per-MB timings were originally single-run and cache-warm, understating cost by
  1.6–2×. The 37% double-parse share that motivated D7 came from two such
  numbers; its direction held, its precision did not. See section 0.
- A claim that D7 regressed analysis by 11% was withdrawn: it compared single
  runs across sessions. Measured properly, D7 is a 36–60% improvement.

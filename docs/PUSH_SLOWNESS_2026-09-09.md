# Push performance — 2026-09-09

Audit of one instrumented Push: mallard-app → myconfstorage (local
directory storage), 41 selected conversations, 41.122 s total. Working tree
at f9628b0 + uncommitted changes. Audit only — nothing in the repository was
changed; every number below comes from the app's own activity log or from a
throwaway probe built outside the repository against the real transcripts.

The compile profile is out of scope by request. All probe numbers are
release-profile numbers unless a column says otherwise, so they describe the
design floor, not the debug binary the app ran.

## 1. Reported phases

Millisecond timestamps from `~/.mallard/logs/activity-2026-09-09-000.jsonl`
(the Push started at 02:19:29.493).

| Phase (log label) | Time | Share | Notes |
| --- | ---: | ---: | --- |
| Push review (5.5 s earlier) | 0.126 s | | cached walk 0.042 s, comparison 0.029 s |
| load config and binding | 0.007 s | 0.0% | |
| discover local state | 2.326 s | 5.7% | full provider walk; the review's cached walk took 0.042 s |
| inspect remote head | 0.017 s | 0.0% | |
| reconcile selection and recipe | 0.000 s | 0.0% | |
| capture selected payloads | 2.812 s | 6.8% | read + digest + full analysis, 4 workers |
| *(untimed)* destination check + apply-time thread review | 3.877 s | 9.4% | between the capture timer and the next timer |
| build conversation actions | 0.000 s | 0.0% | timer covers only plan-id minting |
| build publish request | 30.978 s | 75.3% | the conversation wire builders |
| publish (read head … swing head) | 1.078 s | 2.6% | upload 0.953 s at 658 MB/s, local disk |
| persist push receipts and config | 0.021 s | 0.1% | |
| **Total** | **41.122 s** | | |

Two things the log hides: the review that ran 5.5 s before the Push cost
0.126 s because it used the watch-service walk and the observation cache;
the Push then re-did both from disk (2.3 s + 3.9 s). And the build phase
is one timer around `build_publish_request`; its inner stages are not
timed, which is why the probe below exists.

## 2. Workload

- 41 conversations: 29 Codex, 12 Claude. No sidecars, no subagent children
  shipped (the `Uploading 41 objects` line is exactly the 41 transcripts).
- Local transcript bytes: 537.0 MB (397.8 MB Codex leaves, 139.2 MB
  Claude). That is 512.1 MiB — the selection meter's 512 MiB cap, filled.
- Wire bytes uploaded: 598.0 MB. Codex wire is 1.16× its local leaves
  because three threads are resume forks and the standalone artifact
  flattens their ancestors into each fork: 12.9 → 59.6 MB, 23.9 → 33.0 MB,
  0.6 → 7.7 MB. Claude wire equals its source within 1%.
- Revision 1 → 2 on this bundle: 101 conversations (616 MB) were retained
  from the previous manifest and not re-uploaded; the object-level
  retention works.
- Redaction was on (`settings.json`, `credential_redaction_enabled: true`).
- Checkpoint generations after the Push: 40 of the 41 threads are at
  generation 0, i.e. no cursor checkpoint existed for them when the Push
  ran. That decides which review path they took (§4.3).
- The app binary was the debug build (`target/debug/mallard`, built 02:18,
  Push at 02:19).

## 3. What ran between the timers

`push_bundle_reviewed_with_progress` (`src-tauri/src/project_sync/service.rs:9705`):

1. `discover_full` — a fresh walk of both provider homes
   (`service.rs:9758`), although `session_watch_service().discovery_for`
   served the identical walk to the review 5.5 s earlier from cache
   (`session_watch_service.rs:710`). The Push request has no project
   content, which is exactly the cacheable case.
2. `capture_recipe_from_discovery` — per selected file: `read_snapshot`,
   SHA-256 of the bytes (`resource_digest`), and the adapter's
   `analyze_and_summarize`, a full parse (`provider_capture.rs:506-640`),
   4 workers.
3. Untimed: `classify_push_destination` (one manifest read on local disk)
   and `thread_review(ReviewPurpose::PushPlan)` over the 41 selected
   threads. Plan-purpose reviews never take observations from the cache
   (`service.rs:5176`: only `Observe` calls `take_reusable_observations`),
   so every thread is re-observed from disk. With no checkpoint yet, both
   providers take their first-contact path: Claude reads the file again and
   runs `analyze_main` over all of it (`service.rs:4448-4470`); Codex reads
   the leaf again and runs `resolve_captured_codex_root` — a full parse of
   the leaf plus its ancestors (`service.rs:4040-4120`). 4 workers.
4. `build_publish_request` — per thread, through
   `ordered_preparation::for_each_prepared` (4 workers, 64 MiB byte budget,
   in-order consumption): the provider wire builder, then
   `store_path_origins` (an atomic write with fsync under the persistence
   lock) in the serial consumer.
5. `publish_with_progress` — retention firewall, manifest, upload of the
   41 new objects, commit, head swing. 1.08 s including the 0.95 s upload.

The build verifies freshness on its own at the end of every thread:
`stamp.verify` on every captured source and `observed_tail == pins.tail`
(`service.rs:8858-8892`). The apply-time review in step 3 is therefore a
second freshness proof over the same bytes, taken from disk rather than from
the captured payload the process already holds.

## 4. Where the build's 31 s goes

### 4.1 Method

A throwaway cargo project outside the repository with path dependencies on
`crates/mallard-infrastructure` and `crates/mallard-domain`, calling the
same public entry points the app calls, on the same 41 files (paths from
`conversation_bindings.json`), single-threaded, sequential, release
profile, no writes. Per thread it times each stage of the capture, review
and build pipelines and additionally each component of the wire builders
by re-running the public pieces (`encode_record_with_origins`,
`redact_jsonl_text`, `verify_wire_record`, `verify_wire_value`,
`analyze_*_transcript`, `PayloadBytes::from`). Redaction cost is measured
as (builder with redaction on) − (builder with redaction off); the off run
is the identity on `RedactionRun` and does no scanning. A second binary was
compiled with a profile mirroring the app's dev profile (workspace crates
at opt-level 0, dependencies at opt-level 1). Sampling profiles come from
macOS `sample` on the running release probe.

A first release run overlapped with another session's 4-worker benchmark
(load average 20 on 11 cores) and is used only for ratios; the numbers in
this section are from the re-run on a quiet machine.

### 4.2 Serial cost of every pass (release, quiet machine, one thread)

Totals over the 41 threads. "Local" = 139.2 MB Claude / 397.9 MB Codex
leaves; Codex ancestors add 74 MB of source to the builder's input.

| Stage | Claude (12) | Codex (29) | Note |
| --- | ---: | ---: | --- |
| capture: read | 0.05 s | 0.14 s | |
| capture: resource digest (SHA-256) | 0.31 s | 0.87 s | |
| capture: `analyze_and_summarize` (parse 1) | 0.57 s | 0.86 s | |
| apply review, first contact (parse 2) | 1.07 s | 1.29 s | Claude read+analyze+seed; Codex read+resolve |
| build: `seed_claude_root` (parse 3 + range SHA-256) | 0.50 s | — | |
| build: `resolve_captured_codex_root` (parse 3) | — | 1.15 s | same resolve the review just did |
| build: wire builder, redaction on | 10.42 s | 46.10 s | `encode_main_transcript` / `encode_codex_tree_member` |
| build: wire builder, redaction off | 2.68 s | 6.19 s | identity run, no scanning |
| **⇒ credential detector** | **7.74 s** | **39.91 s** | 81% of all build work |
| · parse source (parse 4) | 0.13 s | 0.25 s | |
| · path-map encode + exact-restoration gate | 1.23 s | 2.31 s | encode walk, decode walk, replay, two deep compares, per record |
| · serialize wire | 0.08 s | 0.21 s | |
| · detector, cold pass | 6.28 s | 30.54 s | `redact_jsonl_text` / `redact_record_gated` |
| · detector, second pass with warm cache | 1.22 s | 1.19 s | SHA-256 lookups for strings ≥ 1 KiB, cold below |
| · parse wire (parse 5) | 0.12 s | 0.29 s | |
| · path-map decode+replay on the wire | 0.38 s | 0.51 s | |
| · wire re-analysis (incl. its parse) | 0.55 s | 0.95 s | |
| · payload SHA-256 | 0.31 s | 1.01 s | |
| build: wire seed / range SHA-256 | 0.31 s | — | |
| build: `summarize_bytes` for the title fallback (parse 7) | 0.21 s | — | runs even when the descriptor has a title |
| build: path origins serialize + hash | 0.02 s | 0.02 s | plus one fsync'd write per thread in the app |
| **build work, serial** | **11.5 s** | **47.3 s** | 58.7 s together |

The detector is 47.6 s of the 58.7 s. Everything the design stacks on top
of the detector — three to seven parses per byte, the path-map round
trips, the wire re-analysis, the hashes — is 11.1 s serial for 598 MB.

Under a profile mirroring the app's dev profile the same serial build work
is 107 s (Claude 24.5 s, Codex 82.9 s): the detector grows only 1.3× (it
lives in the optimized `regex` dependency), the workspace passes 2–26×.
The app's own per-thread durations (§4.5) match the release column within
25%, so whatever profile the running binary has, the compile profile is
not where this Push's time went.

### 4.3 Inside the detector

`sample` over the release probe while it built the Codex threads
(≈35,000 samples): 83% in `regex_automata::…::PikeVM::which_overlapping_imp`
plus `is_word_unicode`/`is_word_character`; 8% `sha2::compress256` (cache
keys and payload digests); the entropy candidate scan, aho-corasick path
matching, serde and allocation share the remaining 9%.

`which_overlapping_imp` is the multi-pattern search behind
`RegexSet::matches`, i.e. `detect_vendor`'s prefilter
(`credential_redaction.rs:532-541`). 26 of the 27 `VENDOR_RULES` use `\b`,
which in the `regex` crate is a *Unicode* word boundary. The lazy DFA
cannot evaluate a Unicode boundary once the haystack contains a
non-ASCII byte; the search is redone by the PikeVM over the whole string
with all 27 patterns. These transcripts are 39–57% non-ASCII by string
bytes (CJK text, box-drawing characters in tool output).

Micro-benchmark on the real string leaves, same rules, release profile:

| Input (string bytes) | Vendor set, Unicode `\b` (shipped) | Same rules, `(?-u:\b)` | Full detector `redact_str` |
| --- | ---: | ---: | ---: |
| Codex 01a060af, 51.2 MB, 57% non-ASCII | 8.11 s (6.3 MB/s) | 0.10 s (527 MB/s), **83×** | 11.21 s (4.6 MB/s) |
| Codex 01a05fbc, 4.7 MB, 49% non-ASCII | 0.64 s (7.2 MB/s) | 0.01 s (474 MB/s), **65×** | 0.91 s (5.1 MB/s) |
| Claude eb31b1c5, 24.8 MB, 39% non-ASCII | 2.84 s (8.7 MB/s) | 0.05 s (463 MB/s), **53×** | 3.83 s (6.5 MB/s) |

Same hit counts in every pair. On the ASCII-only strings the shipped set
already runs at 450–515 MB/s; on the non-ASCII strings it runs at
3.5–3.7 MB/s — a 140× cliff decided by one byte. The other layers on the
same 51.2 MB: entropy candidates 561 MB/s, SHA-256 cache keys 465 MB/s,
`credentialed-uri`/`jdbc`/`database-url`/`password-assignment` ≥ 700 MB/s,
`keyword-dsn` 132 → 1571 MB/s and `semicolon-conn` 86 → 1088 MB/s with
ASCII boundaries (11.9× and 12.6×).

ASCII boundaries are also the safer semantics here: a Unicode `\b` treats
a CJK character as a word character, so `密钥AKIA…` has no boundary and
the AWS key is *not* matched; `(?-u:\b)` matches it. The secrets these
rules describe are ASCII.

### 4.4 The scheduler

`ordered_preparation::for_each_prepared` (`ordered_preparation.rs:57-75`)
admits a job while `admitted.len() < workers` and the reserved bytes fit
the 64 MiB budget, then blocks until job `next` is ready and consumes in
index order. `admitted` holds finished-but-unconsumed jobs, so every
completed result behind a long head-of-line job occupies a worker slot and
its byte reservation until that job finishes. Jobs are in thread-id order,
i.e. random with respect to size.

Reproduction: the real builders through the real `for_each_prepared` on
the real selection, release profile, 11-core machine:

| Configuration | Wall | Serial | Speedup | Peak reserved |
| --- | ---: | ---: | ---: | ---: |
| shipped: 4 workers, 64 MiB, thread-id order | 41.0 s | 60.5 s | 1.48× | 66 MB |
| 4 workers, no byte budget | 37.7 s | 60.7 s | 1.61× | 125 MB |
| 4 workers, no budget, largest-first | 21.4 s | 63.4 s | 2.96× | 227 MB |
| 8 workers, no budget, largest-first | 16.1 s | 72.4 s | 4.49× | 345 MB |
| shipped, redaction off | 7.0 s | 10.8 s | 1.54× | 66 MB |

The byte budget is nearly irrelevant (1.48× → 1.61×); the slot accounting
and the order are the loss. The process already holds the 537 MB capture
plus every wire copy in memory, so a 64 MiB reservation buys little.

### 4.5 The app's own timeline

`store_path_origins` runs in the serial consumer right after each thread is
consumed, so the mtimes of the 41 files it wrote under
`~/.mallard/path_origins/` are the app's consumption timeline (seconds
after the build timer started; probe = release serial cost of that job):

| t | job consumed | local MB | probe |
| ---: | --- | ---: | ---: |
| 0.42–0.48 | three small jobs | 5.9, 4.2, 1.2 | 0.63, 0.45, 0.10 s |
| 4.95 | Claude eb31b1c5 — it ran alone: nothing else fit the 64 MiB budget next to it | 54.7 | 4.58 s |
| 6.45–7.18 | eight jobs admitted at 4.95, all done by 7.18 | | |
| 10.87 | Codex 01a060af (the 6.5 MB/s file) | 57.4 | 9.51 s |
| 13.05–16.67 | eighteen jobs | | |
| 18.31 | Codex 01a06a0c | 42.1 | 4.10 s |
| 22.10 | Codex 01a05f85 | 51.5 | 1.90 s |
| 26.99 | Codex 01a05fbc | 63.4 | 1.66 s |
| 30.96 | Codex 01a0644f, a 12.9 MB fork whose flattened ancestor makes it 70 MB — admitted last because thread-id order put it last | 12.9 | 9.62 s |

The phase ends on the most expensive job because it was admitted last, and
the four jobs above 40 MB each held the pipeline alone. The app's per-job
durations bound from this table track the release probe (sometimes a
little faster, e.g. 3.66 s probe vs ≤ 2.16 s for 01a06958, whose ancestor
has retained path origins from revision 1), which is why the app finished
in 31 s where the release reproduction of the same policy took 41 s.

## 5. Findings

Ranked by measured cost on this Push. The first two account for the build
phase; the next two for the untimed and discover phases.

1. **P1 — Unicode word boundaries make the vendor detector a PikeVM
   scan of every non-ASCII string.** `\b` in 26 of 27 `VENDOR_RULES` (and
   in `KEYWORD_DSN_PATTERN`, `SEMICOLON_CONN_PATTERN`) at
   `credential_redaction.rs:122-372, 488-506`. 47.6 s of the 58.7 s
   serial build; 83% of detector samples. Replace `\b` with `(?-u:\b)`
   (ASCII boundary; strictly more matches on CJK-adjacent secrets):
   53–84× on the set, identical findings on 600 MB of real transcripts.
   With that change the build's serial work is about the redaction-off
   column plus the entropy layer: ~13 s, i.e. the shipped scheduler's 7 s
   wall in §4.4 plus a little. A literal prefilter (the rules' fixed
   prefixes: `AKIA`, `ghp_`, `sk-`, `xox`, `-----BEGIN`, …) in front of the
   set would remove most of what is left.

2. **P1 — `for_each_prepared` runs the batch at 1.5× on 4 workers.**
   Finished results hold worker slots and byte reservations until in-order
   consumption; jobs arrive in thread-id order. Measured: 41.0 s wall for
   60.5 s of work; 21.4 s with largest-first order and no reservation
   (4 workers), 16.1 s with 8. Count only running jobs against `workers`,
   sort the batch by source bytes (leaf plus ancestors) descending, and
   either drop the 64 MiB reservation or make it a real memory bound that
   counts the captured payload it duplicates. Determinism does not need
   in-order *execution*: `operations` are sorted by `operation_id` after
   the loop anyway (`service.rs:9323`).
   Fixed 2026-09-09 by `docs/PLAN_PREPARATION_SCHEDULER.md`
   (`bounded_preparation`: running-only reservations, completion-order
   consumption, largest first, one 256 MiB budget).

3. **P2 — The apply-time review re-parses the selection from disk
   (3.9 s, untimed).** `ReviewPurpose::PushPlan` bypasses the observation
   cache by design ("executable plans read local state"), and a thread
   without a checkpoint (40 of 41 here) takes the first-contact path: a
   third read and a full parse of every byte, after capture already parsed
   the same bytes seconds earlier and before the builder parses them
   again and proves freshness itself (`stamp.verify`, `observed_tail ==
   pins.tail`, `service.rs:8858-8892`). Observe from the captured payload,
   or accept the cached observation when the source stamp is unchanged;
   the builder's checks already carry the fail-closed guarantee.

4. **P2 — Discover walks both provider homes again (2.3 s).**
   `provider_capture::discover_full` at `service.rs:9758` where the review
   used `session_watch_service().discovery_for` (0.042 s) with the same
   request. The comment says one walk feeds the project-file review-token
   check and capture; that check only needs a walk when
   `include_project_content` is true, which it was not.
   Fixed 2026-09-09: the Push now takes its discovery from
   `session_watch_service().discovery_for`, which serves the review's cached
   walk while the watcher proves the home clean and still walks fresh for a
   project-content Push (the cache never serves those).

5. **P2 — The wire pipeline verifies by re-parsing its own output, per
   layer.** Release, serial, both providers: path-map exact-restoration
   gate on every record 3.5 s (encode, decode, replay, two deep compares —
   a codec-invertibility proof re-run on every Push for 71,520 records, of
   which 43% carry a path at all), path-map decode+replay again on the
   wire 0.9 s, wire re-analysis 1.5 s, source and wire parses 0.8 s,
   payload digests 1.3 s, `seed_claude_root`'s watermark parse 0.5 s,
   `summarize_bytes` for a title fallback 0.2 s. That is 8.7 s of the
   11.1 s non-detector build work; capture's own parse and digest (2.8 s
   serial) come on top. Codex additionally resolves the graph twice (review
   and build, 1.15 s each) and deep-clones every source record (`local =
   value.clone()`, `codex_standalone_builder.rs:673`). Keep one parse per
   byte per form: verify the in-memory `Value` you just produced instead
   of re-reading the text you serialized from it, gate the restoration
   proof on records the mapper actually changed, and let capture's
   analysis feed the builder (Claude already passes it to the seed; Codex
   discards it).

6. **P3 — Small redundancies on the same path.** 41 fsync'd
   `store_path_origins` writes under the persistence lock inside the
   serial consumer; nine locked state loads per Push
   (`load_project_state_snapshot`, `load_app_settings`, two
   `load_project_activity_state_snapshot` per provider, `load_config`,
   `load_conversation_bindings`, `load_or_create_host_identity`,
   `resolve_project_binding`); every Codex ancestor read is SHA-256'd into
   a `PayloadBytes` whose digest nothing consumes
   (`codex_history_graph.rs:257`); the detection cache keys every string
   ≥ 1 KiB by SHA-256 on every pass (1.2 s per warm pass per provider);
   `RolloutLookup` walks `~/.codex/sessions` twice per Push (17 ms each).

Design note, no action: Codex forks publish a flattened standalone
artifact each, so ancestor bytes are encoded, scanned and uploaded once per
fork (1.16× here, 4.6× and 12× on two threads). Deliberate, and small on
this selection.

## 6. What is not the problem

- The compile profile, for this Push: the detector dominates and lives in
  an optimized dependency; the app's own per-job durations match the
  release probe.
- Upload and storage: 0.95 s for 598 MB on local disk; retention skipped
  the 101 previously published conversations correctly.
- The second and third detector passes the 09-04 report blamed: after the
  09-05 rescans removal they are SHA-256 cache lookups (1.2 s per provider),
  not detector work. The 09-04 attribution of ~90% to the entropy layer
  does not hold on this content either — the entropy candidate scan runs
  at 560 MB/s; the cost is the vendor set's word boundaries.
- The 09-05 origin-hash serialization: fixed; serialization is 0.3 s
  serial in release (2–5.6 s under the dev profile).

## 7. Limits

- Probe numbers are serial release-profile costs of the same functions on
  the same bytes, not app timings; the app timeline in §4.5 is the app.
- The HorrorGame pushes in the same log could not be reproduced: their
  session files are not under the local provider homes.
- The mallard-app bundle was wiped from the storage during this audit; the
  workload facts were read from its manifest before that.
- Nothing in the repository changed. Probe sources are in this session's
  scratchpad, not the tree.

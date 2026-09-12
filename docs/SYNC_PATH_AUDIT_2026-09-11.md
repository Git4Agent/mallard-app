# Push / Pull / Compare audit - 2026-09-11

Scope: the conversation sync paths (Push review and apply, Pull review and
apply, the Activity refresh comparison) at `0c6813e` plus the working tree.
Focus: performance, redundant checks, over-engineering. Audit only; nothing
in the repository changed.

Evidence used:

- The app's own activity logs (`~/.mallard/logs/activity-2026-09-08..11`):
  76 comparison runs, 4 Push reviews, 6 Pushes, 3 Pull applies, all with
  per-phase timings. The app runs the dev profile (workspace crates at
  opt-level 0), so absolute CPU numbers are pessimistic; the ratios hold.
- A release-profile probe (scratchpad, path dependencies on the crates) run
  on a copy of the real `~/.mallard` (566 KB `sync_config.json`, 208 KB
  `conversation_bindings.json`, 177 KB `materializations.json`, 2.8 MB
  `observation_cache.json`) and on three of this project's own Claude
  transcripts (45 / 57 / 138 MB). Load average was 16 while it ran.
- Code reading of `service.rs`, `bundle_engine.rs`, `pull_transcript.rs`,
  `codex_standalone_builder.rs`, `providers/claude/publication.rs`,
  `session_watch_service.rs`, `persistence.rs`, `conversation_materialization.rs`.

## What the flows cost today (from the logs)

| Operation (HorrorGame, 39 threads, ~136 MB) | Total | Where it goes |
| --- | --- | --- |
| Compare, 424 threads, warm cache | 0.09-1.0 s, median 0.37 s | `compare conversations` 0.04-0.7 s |
| Compare, first contact / big file changed | 2-58 s | full transcript resolves |
| Push review, local storage | 0.10 s | nothing dominant |
| Push review, Dropbox | 0.89 s | head + manifest read 0.76 s |
| Push apply, local storage | 5.19 s | build publish request 3.44 s, capture 1.23 s, publish 0.30 s |
| Push apply, Dropbox | 30.5 s | upload 18.2 s, head/commit/swing 6.5 s, build 3.56 s, capture 1.22 s |
| Pull review, Dropbox | 0.86 s | head + manifest 0.78 s |
| Pull apply, local, 09-09 02:27 and 02:43 (before the scheduler fix) | 19.9 / 24.7 s | prepare + install 15.9 / 21.4 s |
| Pull apply, Dropbox, 09-09 18:30 (current scheduler) | 31.0 s | apply-time review incl. download 22.0 s, prepare + install 7.2 s |

Per-thread Pull CPU, summed over 39 threads (serial worker time):

| Pull apply run | verify cloud | reconstruct | verify output | prepare total | verification share |
| --- | --- | --- | --- | --- | --- |
| 09-09 02:27 | 3.11 s | 8.29 s | 3.75 s | 15.15 s | 45% |
| 09-09 02:43 | 3.79 s | 12.62 s | 3.78 s | 20.20 s | 38% |
| 09-09 18:30 | 3.07 s | 9.94 s | 3.79 s | 16.80 s | 41% |

Release probe (numbers a design can rely on):

| Item | Cost |
| --- | --- |
| `load_project_state_snapshot` | 1.4 ms |
| `load_project_activity_state_snapshot` | 1.9 ms |
| `load_config` / `load_app_settings` | 1.0 ms / 0.04 ms |
| `write_json_atomic`, any size up to 566 KB | 8 ms (fsync floor) |
| `write_json_atomic`, 2.8 MB observation cache | 14 ms |
| detector full pass (`redact_document`) on 45 / 57 / 138 MB Claude transcripts | 118 / 323 / 273 MB/s |
| `analyze_claude_transcript` on the same files | 253 / 312 / 295 MB/s |
| SHA-256 of the same files | ~480 MB/s |

## Findings

Ranked by measured cost on the current tree.

### 1. P1 - The Push builder proves its own output by re-parsing it (66% of a local Push)

`build publish request` is 3.44 s of the 5.19 s local Push and 3.56 s of the
Dropbox one, for 136 MB that capture already parsed, hashed, and (for
sidecars) scrubbed 1.2 s earlier. Inside the builder every byte is derived
again three to four times, and the extra passes exist to re-verify
deterministic in-process transformations:

- Path mapping decodes every encoded record and deep-compares it with the
  source before accepting it (`session_path_map.rs:365`), on every Push, for
  every record, including the ~57% that carry no path at all.
- Codex redacts a record and then runs the detector again on the result
  (`codex_standalone_builder.rs:678-681`), runs it a third time over a
  projection that is never published (`:377`), and re-analyzes the finished
  output text (`:480`, `analyze_standalone_output`).
- Claude redacts the encoded text and then `verify_main_wire` re-parses the
  whole wire document, re-running the path-map and detector checks per
  record (`providers/claude/publication.rs:64-66, 78`).
- Codex resolves the rollout graph again in the builder
  (`service.rs:8502`) although the apply-time review just resolved it and
  handed the builder its stamps.

This is finding 5 of `docs/PUSH_SLOWNESS_2026-09-09.md`, still open. On
that CJK-heavy corpus it was 8.7 s of 11.1 s non-detector work; on this
ASCII corpus the detector is fast (118-386 MB/s in the probe), so these
passes are what is left. Keep the boundary proofs (payload digests, the
`RedactedWireBytes` certificate, the head CAS); verify the in-memory value
the builder just produced, gate the path-map restoration proof on records
the mapper actually changed, and let capture's analysis feed the builder.

### 2. P1 - Pull verifies each transcript twice around the reconstruction, then re-hashes what it just wrote

`prepare_pull_transcript_with_identities` runs a wire verification before
decoding and a full re-analysis of the decoded bytes after
(`pull_transcript.rs:195-203` and `:287` for Claude; Codex does the same in
`decode_codex_tree_member`, `codex_standalone_builder.rs:480`, plus a
projection fixpoint pass at `:489`). Across the three logged 39-thread
Pulls the two verification phases are 6.9-7.6 s of 15.1-20.2 s of worker
CPU (37-45%). The output pass re-parses bytes the decoder serialized a
millisecond earlier; its purpose (the local-form fold and record count)
can be computed from the decoded values in the same loop.

After the scheduler fix the serial consumer is the Pull's critical path:
`apply conversation` sums to 4.7 s of the 7.2 s prepare-and-install wall
in the 09-09 18:30 run, and `stage and commit` alone is 2.8 s. Inside it,
`commit_one_target` reads the staged transcript back from disk and hashes
it (`conversation_materialization.rs:803`) although it was written from an
immutable `PayloadBytes` whose digest is already in the operation: 15-34 MB
per thread re-read and re-hashed serially, before the three fsync'd
document rewrites.

### 3. P2 - Dropbox pays for redundant round trips

One Dropbox round trip costs 0.3-0.8 s in these logs. Per Push the head is
read three times: the review's `inspect_optional` (`service.rs:7715`), the
apply's `inspect_optional_with_token` (`:10052`), and `publish_with_progress`
reading it again (`bundle_engine.rs:733`, measured 0.78 s) even though the
caller passed the observed head token in `PublishExpectation::Match` and the
CAS at `:1075` already detects a moved head. `publish: swing head` is 3.45 s:
the head CAS plus `publish_tag` (`:1084`, `:1783`), which does a metadata
read and a CAS write of a derived summary. A Dropbox refresh reads the
head twice sequentially, once for Activity (`service.rs:1477`) and once
for the comparison (`:6900`), because the page awaits Activity before it
starts the comparison (`ProjectActivityPage.tsx:2274-2275`). That is
finding 4 of `docs/AUDIT_REFRESH_PULL_PUSH_2026-09-06.md`, still open; the
comparison's own `inspect remote head` phase measures 0.27 s on Dropbox.

### 4. P2 - The Push review classifies remote continuity twice, and the apply captures before it checks the destination

`prepare_push_review_with_progress` calls `classify_remote_continuity`
(`service.rs:7725`) and then `classify_push_destination` (`:7730`), which
calls `classify_remote_continuity` again (`:7451`), each with its own
`continuity_probe`. In the "remote ahead" arm both probes download the base
manifest, and the probe is deliberately uncached
(`bundle_engine.rs:1214-1240`, `witness = true`), so that arm costs two
manifest reads. The comment above it says "at most one probe read". The
same shape repeats in the apply: `capture_recipe_from_discovery`
(`service.rs:10109`, 1.2 s) runs before `classify_push_destination`
(`:10117`), so a destination that moved after the review wastes the whole
capture. Finding 6 of the 09-06 audit, still open.

### 5. P2 - The Pull apply-time review parses every selected Codex cloud transcript once more for one boolean

`cached_transcript_is_internal_subagent` (`service.rs:1039`, called at
`:6190`) reads each offerable Codex transcript and runs
`analyze_codex_transcript` on it to decide whether Pull may offer it, keyed
in a process-global map; the preparation worker then parses the same bytes
again in `resolve_standalone_codex_bytes` (`pull_transcript.rs:111`). On a
first Pull that is one extra full parse per Codex thread inside `build
conversation actions` (2.8-3.7 s on local storage for 39 threads). Flagged
in the 09-06 audit; still open. Carry the flag on the manifest row, or
refuse such rollouts at publish, and let the worker's parse decide.

### 6. P2 - Claude Pull runs the detector over the whole cloud transcript before decoding

`pull_transcript.rs:195` certifies the downloaded wire bytes with the full
detector although the object was digest-verified against the manifest and
the row carries the publisher's redaction stamp. On this project's
transcripts the probe measures 0.15-0.36 s per 45-138 MB; on the CJK-heavy
corpus of the 09-09 report the same detector ran at 6-9 MB/s, so the pass
would cost 5-20 s per large transcript. It defends against a publisher
that stamped without certifying. That is a policy choice; the cost is
content-dependent and should be a conscious one.

### 7. P3 - Locked state loads are numerous but cheap

Per operation: compare 4 + P loads (P = bound providers), Push review
4 + P, Push apply 8 + 2P, Pull apply 5 + 2P + K (K = installed threads).
Measured at 1.0-1.9 ms each in release, so 10-30 ms per operation. This
closes A3 of `docs/BACKEND_AUDIT_2026-09-04.md` as "not worth a redesign".
The one gratuitous case is `get_project_with_repository` at the end of
`prepare_push_review_with_progress` (`service.rs:7814`), which reloads the
snapshot already held in `state` to build the same `ProjectDetail`.

### 8. P3 - Per-thread fsync'd writes

Every `write_json_atomic` costs 8 ms regardless of size. Push writes one
`store_path_origins` per thread inside the serial consumer
(`service.rs:9451`; 41 threads = 0.3 s). Pull commits three documents per
installed thread (`persistence.rs:237-285`; 39 threads = 0.9 s). Both are
durability choices per thread rather than bugs; batching per N threads
would recover most of it if the consumer is reworked for finding 2.

### 9. P3 - Invariants re-proven on every read

`validate_config_storage_isolation` (`service.rs:13776`) canonicalizes
every local storage root against every binding root and profile path on
every engine construction (`:13746`), five to seven times per Push plus
refresh cycle, to re-prove a property established when the config was
written. Microseconds today. The Pull review's non-blocking readiness call
re-opens the engine and re-reads head and manifest right after the plan
did (`pullReviewFlow.ts:137`, `service.rs:13170-13190`).

## What is not a problem

- State loads: see finding 7.
- Post-Push and post-Pull comparisons. A Push or Pull bumps every
  checkpoint generation, which invalidates the cached observation for each
  thread it touched, but the next observation rides the checkpoint cursor
  (a tail-window read), so the miss is cheap: every comparison within
  minutes of a logged Push or Pull took 0.06-0.48 s.
- The warm comparison itself: 0.1-0.4 s for 424 threads in the dev profile,
  with nothing structural left to remove. The slow tail (2.4 s, 2.7 s,
  8.7 s, 16.6 s, 20.6 s, 58.2 s) is first-contact or full-resolve work on
  large transcripts (Claude files up to 138 MB, Codex rollouts up to
  434 MB on this machine) and scales with bytes, not with the flow.
- The detector on this corpus: 118-386 MB/s. The Unicode word-boundary
  finding of the 09-09 report applies to CJK-heavy content, not these
  sessions.
- Discovery and the observation caches: `discover local state` is
  0.014-0.14 s warm; the 2.8 MB observation cache rewrite costs 14 ms and
  only happens when something was re-derived.

## The pattern behind findings 1, 2, 4 and 5

Proofs are re-derived at each layer instead of carried forward. A pulled
transcript is checked in five layers: the review (Observe), the apply-time
review (PullPlan), the prepare pins, the worker (digests, refold, decode,
re-analysis), and the install (target re-hash, revision checks). A pushed
one in four: the review, the apply-time review with stamp reuse, the
builder (full re-encode and re-verify), and publish (scope pins, CAS). The
boundary checks earn their keep: digests at transfer boundaries, the head
CAS, the pins, first-contact adoption. The internal re-parses of bytes the
same process produced a moment earlier do not.

## Limits

- App timings come from the dev profile; the probe from release. Neither
  is a release-profile end-to-end run.
- No Dropbox call was made by this audit; Dropbox numbers are the app's own
  logged phases from 09-09 and 09-10.
- The two local-storage Pull applies predate the scheduler change of
  2026-09-09; the Dropbox one is on the current scheduler.
- Nothing in the repository changed. The probe lives in the session
  scratchpad, not in the tree.

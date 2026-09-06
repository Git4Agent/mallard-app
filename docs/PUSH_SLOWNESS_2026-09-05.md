# Push performance — 2026-09-05

The reported Push took 35.292 s. Request preparation took 27.378 s
(77.6%). Upload took 0.152 s (0.4%).

The main measured cost in the current preparation code is repeated JSON
serialization for path-origin hashes. The running app uses the debug build.
Workspace crates have compiler optimization disabled in that build.

## Reported phase costs

| Phase | Time |
| --- | ---: |
| Discover local state | 1.949 s |
| Capture selected payloads | 2.916 s |
| Build publish request | 27.378 s |
| Publish conversations and receipts, including upload | 1.144 s |
| Other work and timing gaps | 1.905 s |
| Total | 35.292 s |

Upload is a child phase of publication. Do not add it to the publication
time again. The code also performs thread review between capture and the
next phase timer. Thus, the small `build conversation actions` time does
not include that review.

## Measurement method

- Copy the 38 selected transcripts to a private temporary directory.
- Use the current Rust preparation functions in a temporary source copy.
- Add nested timers with exclusive accounting. Keep all existing gates.
- Resolve and seed Codex roots, encode both providers, verify output, and
  write path origins to a temporary Mallard home.
- Share one credential-redaction cache across the batch, as Push does.
- Run each compiler configuration twice, with no concurrent probe builds.

The snapshot contained 35 Codex threads and 3 Claude threads. Its 38
physical files contained 145,579,101 bytes. Each run prepared all 38
threads without a failure. Each produced 133,251,638 wire bytes and
2,324,076 origin-metadata bytes.

The probe isolates preparation. It omits Tauri scheduling, review, cloud
writes, and receipt transactions. It also includes Claude input analysis,
which the application normally obtains during capture. These are probe
times, not measurements of a complete Push.

## Results

| Work | Current debug profile | Workspace `opt-level = 1` |
| --- | ---: | ---: |
| Complete preparation probe | 22.66–22.96 s | 6.24–6.31 s |
| Serialize records for origin hashes | 9.54–9.58 s | 0.320–0.335 s |
| Compute those SHA-256 hashes | 1.596–1.609 s | 1.507–1.555 s |
| Credential redaction and verification | 1.87–1.90 s | 1.44–1.45 s |
| Write 38 origin files | 0.314–0.324 s | 0.264–0.325 s |

The two Codex display projections together used 0.14 s or less in the
current debug profile, including their nested credential check. They did
not explain the main delay.

The mean preparation time fell from 22.806 s to 6.274 s: a 3.64-fold
speed increase. Only the compiler profile changed between these runs.
Dependencies remained at their existing optimization level.

## Cause in the code

`build_publish_request` in `src-tauri/src/project_sync/service.rs` prepares
conversations in a serial loop. Costs therefore add across the 38 threads.

For a normal new record, the path-origin code performs these operations:

1. Encode the record and hash its source form for retained origins.
2. Decode the wire form for the exact-restoration gate. Hash that result.
3. Replay the newly captured origins. Hash the same decoded result again
   before replay.
4. After redaction, verify the emitted wire record. Its decode captures
   another hash.
5. The verification replay hashes that decoded record again.

`record_digest` in
`crates/mallard-infrastructure/src/session_path_origins.rs` uses
`serde_json::to_vec` before SHA-256. Each call allocates and serializes a
complete record. The probe counted **54,139 calls and 701,138,112 serialized
bytes**, about 4.8 times the physical source size. Large inline payloads
pass through these operations too.

The gates are in `session_path_map.rs`, in `encode_record_with_origins`,
`decode_record_with_origins`, and `verify_wire_record`. Codex invokes the
last gate again when it emits primary records in
`codex_standalone_builder.rs`. The Claude service path does the same after
redaction.

The earlier report, `PUSH_SLOWNESS_2026-09-04.md`, measured older code.
Several repeated credential scans have since been removed. Its finding
that credential scans dominate does not describe this measurement.

## Actions

1. Set `opt-level = 1` under `[profile.dev]`. This is the smallest measured
   improvement for the running development app. It can increase Rust
   rebuild time. It does not change the release profile.
2. Implemented below: remove repeated serialization and hashing of records that the current
   call has just constructed and has not changed. Keep strict hash checks
   for retained origins loaded from storage. Keep exact restoration,
   inverse-edit replay, post-redaction wire validation, and credential
   gates.
3. Add preparation subphase timing. The current 27-second label hides the
   expensive steps.

The initial investigation changed only this report. The implementation and
its separate measurements follow. Compiler settings remain unchanged in
the repository; actions 1 and 3 are still recommendations.

## Implemented origin-hash reduction

`session_path_map.rs` now decodes and verifies fresh inverse edits without
creating a temporary origin hash. `decode_record_with_origins` computes
one hash after that gate, when its caller needs origins to retain.
`session_path_origins.rs` shares the inverse-edit operation between fresh
decoding and stored-origin replay. Stored replay still verifies its saved
hash first. Pointer, range, token, and key-collision checks remain common.

The common Push path now hashes a primary source record once instead of
five times. Exact source restoration and the post-redaction wire gate both
remain. Credential checks, payload hashes, and origin-file integrity hashes
are unchanged. Schema remains 1. There is no migration or alternate codec.

### Separate measurement

The comparison used a new temporary copy of the same 38 source files,
with the same byte totals. Both versions used the same surrounding code,
instrumentation, and compiler profile. Within each profile, run order was
before, after, after, before. Builds and tests finished before measurement.

| Profile | Mean before | Mean after | Time saved | Reduction |
| --- | ---: | ---: | ---: | ---: |
| Current debug, workspace `opt-level = 0` | 23.27 s | 14.15 s | 9.11 s | 39.2% |
| Temporary workspace `opt-level = 1` | 6.17 s | 4.76 s | 1.41 s | 22.8% |

| Origin-hash work per batch | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Record serialization/hash calls | 54,139 | 10,921 | 79.8% |
| Bytes serialized for these hashes | 701,138,112 | 145,877,206 | 79.2% |

All eight runs prepared all 38 threads without failure. Wire payload and
stored origin-file SHA-256 values matched for every thread across all runs.
The probe excluded the extra comparison-hash time from its reported total.
These remain preparation measurements, not complete Push times. The
optimized result isolates this code change; it does not include a compiler
setting change in the measured before/after pair.

### Validation

- Focused path tests: 11 passed; 1 manual timing test ignored.
- Infrastructure library: 441 passed; 5 manual tests ignored.
- Backend library: 273 passed.
- Live Codex aimock scenario: passed in 71.86 s, including 3 → 1 → 1
  Push/Pull isolation and 5 → 2 → 1 Pull source checks.
- TLC: all 7 positive configurations passed, covering 287,841 distinct
  states; all 6 negative controls failed their expected invariant.
- Z3: 10 positive checks returned `unsat`; 4 negative controls returned
  `sat`. The added pair checks fresh-hash removal and rejects bypass of
  stored-hash mismatches in the abstract model.

Rust tests assert the exact hash-attempt counts and rejection of changed
records, damaged hashes, wrong inverse edits, malformed wire tokens, and
key collisions. Formal checks cover the stated algebra and model bounds;
they do not prove the complete Rust implementation or SHA-256.

The live test required execution outside the outer macOS sandbox so Codex
could start its own sandbox. Tests used temporary provider and Mallard
homes. Temporary transcript copies and generated local state were removed
after measurement.

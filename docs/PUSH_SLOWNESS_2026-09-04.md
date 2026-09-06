# Push slowness trace — 2026-09-04

Baseline: branch `extensible-providers`, `HEAD` a5a47a1 + working tree.
Subject: one instrumented Push, HorrorGame → myconfstorage2, 29.318 s wall.
Scope: **performance only.** Nothing in the repo was changed by this investigation.

---

## 1. Verdict

The push is not I/O bound, not network bound, and not hash bound. **All of the
data movement it performs has a measured floor under one second.**

`build publish request` costs 23.357 s (79.7 % of the push) because the
**credential-redaction detector is run three to four times over the same
136 MB**, and each run is dominated by an entropy scanner that re-derives
findings the previous run already proved. On top of that the whole binary is a
dev build in which the workspace crates — the ones that contain this code — are
compiled at `opt-level = 0`.

| | |
|---|---|
| Payload | 37 conversation transcripts (35 Codex + 2 Claude), 141.6 MB source / 136.2 MB published |
| Project files in this push | **zero** — this is a pure conversation-history push |
| Measured floor for all data movement | **< 1 s** (read 0.35–0.55 s, sha256 0.12–0.39 s, upload 0.173 s) |
| Redaction engine share of the 23.357 s | **≈ 12–14 s (53–60 %)** |
| Dev-profile penalty on that code | **5.7× measured** (41× on the serialize pass) |

The single most important number: reading all 37 files and hashing them takes
about **0.7 s**. The push spends **23.4 s** deciding they contain no secrets.

---

## 2. What this push actually was

Established by reading the published revision-2 manifest and stat-ing the real
sources under `~/.codex/sessions` and `~/.claude/projects`.

- **35 Codex rollouts**, 102.2 MB published (`build_standalone_codex_root` path).
- **2 Claude transcripts**, 34.06 MB published (the JSONL wire path).
- The HorrorGame working tree (185 MB of `node_modules`, `.git`, build output)
  contributes **0 bytes**. Any hypothesis about gitignore matching, tree walking
  or project-file capture explains a cost that is not in this run.

Two properties of the payload matter for the fix:

- **68.6 MiB (50 % of the whole push) is inline `data:` URI image bytes** that
  the redactor explicitly skips (`credential_redaction.rs:1332`). They are read,
  hashed, shipped and walked, and carry no secret risk.
- **Five of the 35 Codex threads are resume-forks of one conversation**, all 549
  messages, 15.30 MB each — 76.5 MB of near-identical content encoded, redacted,
  certified and stored five separate times. Across the whole payload, 129.70 MiB
  of string values are scanned but only **42.70 MiB are distinct (3.04×
  redundancy)**.

---

## 3. Phase accounting

| Phase | Measured | What it is |
|---|---|---|
| load config and binding | 0.004 s | — |
| discover local state | 2.310 s | re-walk of both provider homes; **the cache was bypassed** |
| inspect remote head | 0.006 s | — |
| reconcile selection and recipe | 0.000 s | — |
| capture selected payloads | 2.184 s | read + hash + analyze 136 MB |
| build conversation actions | 0.001 s | — |
| **build publish request** | **23.357 s** | **the bug** |
| publish: read head → swing head | 0.199 s | includes the 136.2 MB upload at 0.173 s |
| publish conversations and receipts | 1.425 s | 111 atomic writes / 222 fsync barriers |
| persist push receipts and config | 0.013 s | — |
| **Total** | **29.318 s** | |

---

## 4. Root cause — the detector runs 3–4× over the same bytes

### 4.1 The Codex arm (102.2 MB, 35 actions)

Every Codex rollout passes through the credential detector **three times**, and
nothing mutates the bytes between passes:

| # | Call site | Measured |
|---|---|---|
| 1 | `redact_record_gated` — `codex_standalone_builder.rs:322` | 3.366 s |
| 2 | `verify_wire_value` — `codex_standalone_builder.rs:194` | 4.779 s |
| 3 | `certify_wire_document` — `service.rs:7981` | 3.83 s |

Pass 2 re-proves the fixpoint pass 1 just established one record earlier.
Pass 3 re-splits the finished 102 MB artifact on `\n`, re-parses **every line**
back into a `serde_json::Value` (`credential_redaction.rs:1149`) and re-runs the
full regex battery over every string leaf — on a document whose every record was
individually certified two steps earlier. Its only real product is the
`RedactedWireBytes` newtype that publish paths require: **the redundancy is a
type-system artifact, not a semantic one.**

> Verification correction worth keeping: pass 3 is not *purely* waste — the
> `impl Into<PayloadBytes>` conversion computes the sha256 the manifest and
> object key genuinely need. Only the per-line parse and the detector traversal
> are removable.

### 4.2 The Claude arm (34.06 MB, 2 actions)

Seven sequential whole-transcript passes, measured at 1.761 s idle
(≈3.3 s at the instrumented rate):

`encode_jsonl_line_gated` 0.160 s → `redact_jsonl_text` 0.417 s → per-line
`from_str` + `verify_wire_record` self-check 0.143 s → `certify_wire_document`
0.484 s → `analyze_claude_transcript` 0.127 s → `seed_claude_root` ×2 0.189 s →
sha256 0.220 s.

`verify_wire_record` (`session_path_map.rs:582`) is itself decode + re-encode +
a deep `Value` equality, per line.

### 4.3 Inside the detector

- **The L1 entropy detector is ~90 % of every pass** —
  `credential_redaction.rs:1381`, a `[A-Za-z0-9+_=-]{10,}` regex followed by a
  256-bin Shannon loop per candidate.
- **`detect_vendor` is 85 % of detector time** and collapses to 1.7–2.6 MiB/s on
  ordinary source text. Strings in the 4 K–64 K band are only 8.3 % of the
  payload but consume **79.8 %** of the detect pass (the 16 K–64 K band alone is
  65.9 %) — a RegexSet alternation over ~45 vendor patterns thrashing the lazy
  DFA cache.
- `first_value_finding_inner` (`credential_redaction.rs:880`) builds a
  `format!` JSON-pointer `String` **for every key at every depth**, on the happy
  path, purely for an error message that is almost never emitted.

### 4.4 The build profile multiplies all of it

`Cargo.toml:28` sets `[profile.dev.package."*"] opt-level = 1`. The wildcard
**excludes workspace members**, so `mallard-infrastructure` — where every one of
the passes above lives — compiles at `opt-level = 0`.

Measured on the exact 37 files of this push: the same capture work costs
**8.243 s at the app's dev profile vs 1.447 s at opt-level 3 (5.7×)**. A single
serialize pass costs 6.783 s vs 0.165 s (41×).

**A meaningful fraction of the 29 s does not exist in a shipped release build.**
Re-time with `--release` before optimizing anything, or the numbers will lie.

---

## 5. The other three phases

**discover local state — 2.310 s.** The push re-walks both provider homes with
`discover_full` (`service.rs:8542`), bypassing the watch-service cache that
served the *identical* walk for the review 2.8 s earlier in **0.054 s**
(the app's own log line: `Push review phase — discover local state: 0.054s`).
Within the walk, Claude metadata scanning falls back to whole-file streaming for
118 of 127 large transcripts — **1.042 GiB read and 248 k JSON records parsed**,
of which **96.8 % of the bytes belong to other projects**. Codex discovery opens
all 2006 rollouts to read one record each; 1971 are foreign. A `git rev-parse`
subprocess (`service.rs:2765`) is spawned inside the timed phase to decide
project-file eligibility that this push never needed. Separately, the phase runs
at **macOS background QoS**, a measured **2.78× wall-clock multiplier** — about
0.83 s of work presented as 2.31 s.

**capture selected payloads — 2.184 s.** `detect_secret_material`
(`provider_capture.rs:2256`) allocates a full lowercase copy of every UTF-8
payload and makes up to **13 substring searches**: ten over the copy and three
over the original text. A 100 MiB payload requires a 100 MiB temporary copy.
The existing scan accounted for ~2.7 s of serial CPU in the measured run,
roughly half the phase. Removing the allocation can reduce temporary memory,
but fewer scan passes do not establish a speed improvement. Measure a replacement
on representative transcripts in a release build before claiming time savings.
Every transcript is fully JSON-parsed **twice** (`analyze` then
`summarize_bytes`, `provider_capture.rs:601`). Every byte is SHA-256'd **twice**
(`PayloadBytes::from` then `resource_digest`) for a digest the conversation path
never uses — 0.53 s. No fsync, chmod or temp-file dance here; the budget gate
re-stats but never re-reads.

**publish conversations and receipts — 1.425 s.** The 37-operation loop
(`conversation_materialization.rs:214`) performs **111 whole-document atomic
writes = 222 `F_FULLFSYNC` barriers** where 3 writes would do. Replayed at the
real document sizes on the same APFS volume: **0.821 s, versus 0.020 s batched.**
Per-write cost is 5.6–6.0 ms and flat across 28 KB–115 KB — entirely barrier
bound. `conversation_bindings.json` is loaded **77 times** in one push and
`sync_config.json` read, parsed and validated **~164 times**, each under a
process-global mutex.

---

## 6. Minimal fix

### 6.1 The one-line change (do this first, before anything else)

```toml
# Cargo.toml, in [profile.dev]
opt-level = 1
```

The existing `[profile.dev.package."*"]` bump deliberately excludes workspace
members, which is exactly where this code lives. Measured effect: the serialize
pass 6.783 s → 0.095 s; the modelled 6-pass pipeline over the same 141.6 MB
**18.3 s → 6.0 s**.

Cost: slower incremental rebuilds of workspace crates. If that is unacceptable
for daily edits, scope it to the one crate instead:

```toml
[profile.dev.package.mallard-infrastructure]
opt-level = 1
```

This buys the largest measured win for the least risk — but note it is a
*development* win. It changes nothing for users of a release build, so do 6.2 as
well.

### 6.2 Remove repeated Codex credential scans

Implemented as a clean cutover. Machine-local and bundle schemas remain 1.

- Source records still pass through `redact_record_gated` before projection.
- The projector checks each inserted record when it is created. The head is
  checked beside its final construction. The full projected-body credential
  verification loop is removed. Path and conversation-structure checks remain.
- `encode_standalone_codex_root` returns an artifact containing immutable
  `RedactedWireBytes`. Push consumes this certificate directly. The final
  Codex `certify_wire_document` parse and scan are removed.
- `decode_standalone_codex_root` returns local `PayloadBytes` and keeps its
  incoming-wire checks. The old public mode-selected builders are removed.
- `PayloadBytes` computes the output content SHA-256 once. `artifact_digest`
  reuses its binary digest inside the existing framing. The content digest
  and artifact digest retain their distinct meanings.

Projection checks remain necessary for newly created content. XML entity
unescaping can reveal a credential, and source redaction must occur before
projection removes or changes JSON field context. These checks run at
construction, without another scan of all output records.

The earlier timings describe the original run. They do not establish the
savings from this implementation. The current code also shares a bounded
`RedactionRun` cache across conversations. No new performance measurement is
part of this change.

Implementation and verification details are in
`docs/PLAN_REMOVE_CODEX_REDACTION_RESCANS.md`.

### 6.3 The rest of the ladder, by measured value

| Fix | Where | Win |
|---|---|---|
| Reuse the watch-service cache instead of `discover_full` | `service.rs:8542` | 2.310 s → 0.054 s |
| Batch the 111 atomic writes into 3 | `conversation_materialization.rs:214` | 0.821 s → 0.020 s |
| Memoize `detect()` by content hash for one publish | `credential_redaction.rs:952` | 129.7 → 42.7 MiB scanned |
| Skip `shannon` for candidates ≤ 22 bytes; iterate only bytes seen | `credential_redaction.rs:522-546` | kills 34.5 % of entropy calls |
| aho-corasick prefilter over vendor literal heads | `credential_redaction.rs:552` | the 85 % `detect_vendor` share |
| Remove the file-sized lowercase allocation from the secret scan | `provider_capture.rs:2256` | Reduces temporary memory; speed improvement is not established. |
| Externalise inline `data:` image bytes at capture | `credential_redaction.rs:1332` | ~50 % of every downstream pass |
| Drop the second `analyze` (`summarize_bytes`) | `provider_capture.rs:601` | 0.5 s serial |
| Use the library serializer for canonical JSON | `conversation_codec.rs`, `canonical_json` | Implemented: removes intermediate strings and repeated subtree copies. No new measurement. |
| Reuse the artifact's last canonical message ID | `service.rs`, `conversation_thread_from_capture` | Implemented: removes the second analysis of the finished Codex output. |
| Guard the unconditional `fchmod` | `persistence.rs:254` | noise, but a known-decided cleanup |

Explicitly **not** worth chasing for this bug: sha2's software backend
(~0.3 s, < 2 %), the redundant leaf re-reads (measured 0.065 s — *refuted* as a
cause), and the background-QoS multiplier (immaterial once the work is removed).

The canonical JSON helper now sorts object keys with `Value::sort_all_objects`
and uses `serde_json::to_vec` to write into one growing buffer. Sorting does no
work with the current dependency features because JSON maps already use
`BTreeMap`. Codex and Claude share this helper. Synthetic fixtures pin the
canonical bytes and SHA-256 values produced before the change, including
nested key order, escaped text, Unicode, numeric limits, and both providers'
semantic fingerprints.

The standalone builder still analyzes its output and verifies the canonical
chain. It retains the last canonical message ID in the artifact. Publication
now uses that ID directly. A Push test checks that a paginated thread with an
empty leaf publishes the last message ID from its inherited history.

Machine-local and bundle schemas remain 1. No migration or legacy execution
path was added. The former `4.9× allocation, runs twice` entry was a historical
claim, not a measured result of these changes. No performance measurements
were run for this implementation.

Verification passed: 415 infrastructure library tests, 267 Tauri library
tests, and `cargo check --workspace --all-targets`. The benchmark and two
tests that read real Claude data remained ignored. Live-test targets were
compiled but not run.

---

## 7. Method and confidence

Nine parallel readers over the timed phases and cross-cutting lenses, then
three adversarial verifiers per surviving hotspot (arithmetic / control-flow /
attribution), majority-refute to kill. 75 raw findings → 60 distinct → 5
survived all three lenses; 1 was refuted outright.

Numbers marked MEASURED come from throwaway probe crates built **outside** the
repo with path dependencies on `mallard-infrastructure` and `mallard-domain`,
run against the real corpus on the same APFS volume as `~/.mallard`, several
reproducing the app's exact dev profile. Nothing in the working tree was
modified, and no `cargo` build was run inside the repo.

One claim was refuted and should not be repeated: *"each Codex leaf is read from
disk four times, and that is the cost."* The re-reads are real
(`codex_history_graph.rs:168`, `service.rs:7753`,
`codex_standalone_builder.rs:397`) but measure **0.065 s — 0.28 %** of the phase.
Warm reads run at 3122 MB/s here; disk is not the problem.

See also `docs/BACKEND_AUDIT_2026-09-04.md`. Its headline O(n²) item A1
(`bundle_engine.rs:787`) is **not** this bug — that code ran in 0.009 s.

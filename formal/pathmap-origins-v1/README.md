# Lossless path references, schema 1

The implemented codec preserves conversation text on return to the original
project binding. Provider IDs and record layout can change. Credential
redaction remains an explicit exception. Redacted text stays redacted.

This is a clean cutover. Local state, manifests, plans, and receipts keep
**schema 1**. The codec name stays `mallard.pathmap.v1`. Clear the old test
state before use. There is no migration or old-format reader.

The guarantee covers valid UTF-8 text in supported provider transcripts,
including controls represented through JSON escapes. File size, provider
format, path, and record-count limits still apply. Exact path spelling on
return requires the same project root, canonical alias, and home binding.
Unicode text is not normalized.

## Encoding and local metadata

The old Pull gate rescanned decoded text under the destination root. It
changed literal destination paths into references and rejected those records.
It also lost tilde, alias, and URI spellings. The new gate replays the exact
inverse edits from decoding. It does not scan for destination paths.

A reference has a terminated spelling tag:

| Tag | Source prefix |
| --- | --- |
| `b` | Binding root, absolute path |
| `c` | Canonical alias, absolute path |
| `bt` | Binding root, tilde path |
| `ct` | Canonical alias, tilde path |
| `bu` | Binding root, raw file URI |
| `cu` | Canonical alias, raw file URI |
| `be` | Binding root, percent-encoded file URI |
| `ce` | Canonical alias, percent-encoded file URI |

For example, `~/project/src` becomes `/__mallard_root__/bt;/src`.
URI tags replace the full URI prefix. Literal `/__mallard_root__` and
`/__mallard_escape__` text becomes `/__mallard_escape__/r` and
`/__mallard_escape__/e`. Complete reference-looking source text is also
literal data. Neither pass scans its own output. Unknown tags and malformed
wire escapes fail validation. Unrecognized URI spellings remain literal.

A destination can render two stored meanings as the same text. Thus local
text alone cannot determine the next wire value. The implementation retains
a transformation complement, as described in
[Symmetric Lenses, section 2](https://www.cis.upenn.edu/~bcpierce/papers/symmetric.pdf).

`LocalConversationBinding.path_origins_ref` is required and nullable. `null`
means the physical transcript has not yet had a path capture. A reference
contains `sha256` and `size_bytes`. It names an immutable, private local file
at `path_origins/<sha256>.json` under the Mallard home. The bound is 128 MiB.

Each file contains schema 1, the codec, its host/replica/project/profile and
physical thread owner, and origin entries by physical source ID and nonempty
record ordinal. Each entry has:

- A strict SHA-256 of the parsed local JSON record, including final IDs and
  ordinals. JSON whitespace can change; text cannot.
- A `derived` bit for rebuildable native display/layout records.
- Inverse path edits: a [JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901.html),
  a key/value flag, a UTF-8 byte range, and a closed wire atom.

The file stores no transcript copy. Pointer keys can contain local text, so
origins remain private local state. They never enter a portable manifest.
Zero-edit records still have coverage. Missing, corrupt, stale, swapped,
truncated, or misaligned referenced data fails the affected thread.

Push checks retained record hashes and replays their edits. It scans only
new records. Native forks use physical parent coordinates; metadata updates
are mapped before folding. Pull captures origins after redaction and typed
identity changes. Keep both retains the source reference and gives the new
physical branch its own reference. Metadata reset retains origin-bearing
bindings; host identity rotation stages newly owned files before its existing
rotation transaction. Removing local origin state discards the inverse data.

Adoption uses saved origins when they exist. A first capture of native data
uses source encoding. The resulting wire fold must match the mapped cloud
pin. Adoption never repairs a missing or damaged referenced file by rescanning
local text. An unproven adoption remains blocked. It does not add a cloud
transcript download.

Codex wire transcripts retain source records. Pull builds redundant native
display records after decoding. This keeps HTML entity decoding and file-URI
formatting outside the path parser. Those generated records have strict local
hashes and are omitted on later Push. A retained primary record is not newly
classified as redundant merely because two stored values render alike.

Origin files are staged before publication or installation. Their references
enter the binding after-state. Accepted checkpoints advance after provider
installation and binding persistence. Current Apply is best effort. A late
write failure can leave installed bytes with the old checkpoint or origin
reference. There is no rollback or automatic replay. A stale reference fails
its strict record check. The user must Compare and Apply again.

Payload digests, cloud CAS, approvals, revision checks, size limits, file-type
checks, occupied destinations, shared indexes, and real JSON key collisions
remain checked. Pull still performs one live source update check. Origin
checks use captured or prepared records, not a second live transcript read.

## Cost sample

CPU work is linear in transcript bytes plus edits. Origin loading and saving
adds local metadata I/O. No cloud request is added to normal Push or Pull.
Each raw-path tag adds three or four wire bytes. URI tags can be shorter than
the old URI form. Source-only Codex wire records remove redundant payloads.

Origin hashes are now computed only when capturing origins to retain, or
when checking stored origins against a source record. The common Push path
hashes each primary source once. Temporary decode checks replay fresh inverse
edits directly and still compare the result with the complete wire record.
They do not create or check a temporary hash. Pull captures one hash when it
returns origins; final metadata changes can still require a new hash.
Stored origins always pass their hash check before inverse replay.

A manual **debug-build** sample before this reduction used 10,000 records,
one path edit in 25% of records, and 10,528,890 bytes of JSON.
Serialized origins used 1,387,524 bytes,
excluding the small file envelope. First encoding took 1,090 ms; retained
encoding took 649 ms; decoding took 412 ms. This is a local sample, not a
production benchmark or a measured speed change against the old codec.
Parsed records and origin vectors also use memory. No peak-memory claim is
made from serialized size alone.

```bash
cargo test -p mallard-infrastructure --lib path_origin_cost_sample -- --ignored --nocapture
```

## Formal checks

The checks used [TLC 1.7.4](https://github.com/tlaplus/tlaplus/releases/tag/v1.7.4),
Java 17, and one worker. The tool JAR SHA-256 was:

```text
936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
```

All seven positive configurations passed:

| Configuration | Bound and property | Distinct states |
| --- | --- | ---: |
| `source` | 11 atoms, length through 4; replay and cross-machine meaning | 16,105 |
| `continuation` | Source length 3, append length 2; retained prefix | 194,712 |
| `wire` | 13 atoms, length through 4; escapes and replay | 30,941 |
| `guards` | Source length 3, six record/origin conditions | 8,784 |
| `pair` | Pairs through length 2; origin distinction and key collision guard | 17,689 |
| `spelling` | 26 atoms, length through 3; eight styles and return to A | 18,279 |
| `commit` | Three threads; staged origins, separate writes, failure and crash | 1,331 |

Total: **287,841 states**. All six negative controls produced their expected
invariant failure: blind rescanning, treating zero edits as new history,
merging keys, dropping spelling on first Push, dropping spelling on later
Push, and accepting a transcript before saving its origin reference.
Tool errors do not count as successful negative controls.

[Z3 5.1.0.0](https://pypi.org/project/z3-solver/5.1.0.0/) checked the empty,
literal, and reference constructor cases of text restoration and inverse
replay. It uses [arbitrary byte sequences](https://microsoft.github.io/z3guide/docs/theories/Sequences/)
for literal chunks. Nine counterexample queries returned `unsat`; three
deliberate faults returned `sat`. These constructor checks have no literal
length or alphabet bound.

The hash-reduction check models hashing as an arbitrary deterministic
function. It proves that omitting `H(record) == H(record)` leaves the replay
gate unchanged while the captured record is immutable. A negative control
finds a counterexample when a stored hash mismatch is ignored. The current
script therefore has **10 positive checks and 4 negative controls**. These
two additional checks do not prove SHA-256 or the Rust implementation.

```bash
python3 formal/pathmap-origins-v1/run_checks.py --jar /path/to/tla2tools.jar
python3 formal/pathmap-origins-v1/check_algebra.py
```

These results apply **after correct lexical decomposition**. They do not
prove the Rust scanner, Unicode slicing, provider parsing, projection,
redaction, SHA-256, filesystem operations, or the entire application.
The commit model permits partial writes. It does not assert rollback.

Rust tests check the concrete scanner, all eight spellings, literal marker
syntax, Unicode and control text, exact returned message text, nested keys,
zero-edit history, appends, native forks, metadata folding, hook projection,
origin corruption and ownership, metadata reset, and host rotation. Existing
service and live tests cover thread isolation, Keep both, in-place updates,
approvals, shared indexes, destination collisions, and credential redaction.

## Application validation

The initial implementation passed these checks with temporary provider and Mallard
homes:

| Check | Result |
| --- | --- |
| Domain library | 77 passed |
| Infrastructure library | 435 passed; 5 manual tests ignored |
| Tauri backend library | 272 passed |
| Offline Codex aimock fixtures | 7 passed |
| Live Codex aimock scenario | Passed, 69.20 s |

The live scenario covered native pagination, storage reset and republish,
Library integration, and 20 disposable history resumes. Its failure-isolation
results were:

| Attempt | Selected | Push succeeded | Pull succeeded | Remaining | Pull source checks |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 5 | 3 | 3 | 2 | 5 |
| 2 | 2 | 1 | 1 | 1 | 2 |
| 3 | 1 | 1 | 1 | 0 | 1 |

Pull's source checks read 361,253 bytes in total and took 1.696 ms in that
run. These are the existing one-check measurements, not total Apply time.
The fake API's exact request-count assertions passed.

After the origin-hash reduction, the infrastructure suite passed 441 tests
(5 manual tests ignored), and the backend suite passed 273 tests. The live
Codex aimock scenario passed again in 71.86 s, with the same 3 → 1 → 1
Push/Pull isolation outcomes. Focused tests assert one hash per retained
origin capture or stored-origin check, and zero hashes for temporary wire
checks. Changed records, damaged hashes, wrong inverse edits, malformed
tokens, and key collisions still fail. The TLC checks above and all 14
current Z3 checks passed again.

Measured preparation costs are in
[the Push performance report](../../docs/PUSH_SLOWNESS_2026-09-05.md).

```bash
cargo test -p mallard-domain --lib
cargo test -p mallard-infrastructure --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

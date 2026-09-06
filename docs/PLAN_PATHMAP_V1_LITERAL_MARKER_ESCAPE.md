# PLAN: Replace `mallard.pathmap.v1` reserved-marker failure with reversible escaping

Status: code complete and verified in isolated tests (2026-09-03). The
clean-state native smoke test remains an operator step.

This change replaces the `mallard.pathmap.v1` string grammar in place.
It lets conversations contain Mallard's root marker without blocking Push.
It also preserves the existing project-path remap, canonical-source gate, and
target-collision gate.

Compatibility (user-set): **clean cutover; no migration and no legacy
execution path**. Bundle schema remains `1`. The path-map codec remains
`mallard.pathmap.v1`. The Codex marker and Claude stamp retain their current
shapes. Before retesting, all old `~/.mallard` state and all old cloud or
local-folder bundle data are removed. There are no customer artifacts to
preserve.

Implementation result:

- `SessionPathMapper` now escapes literal `R` and `E` values in one pass.
- Wire validation rejects unknown or truncated `E` productions.
- Path repair discovery ignores marker-only content.
- Codex and Claude regression tests cover literal markers and cross-root
  decode.
- A Rust conformance test checks 7,381 bounded source strings against the
  grammar properties.
- The complete backend integration command passes.
- The repository TLA+ models and their mutation controls pass as specified in
  section 8.
- Bundle schema `1`, codec `mallard.pathmap.v1`, provider declarations, and
  DTO shapes are unchanged.
- No migration, legacy decoder, fallback, or feature flag was added.

The implementation did not access real provider homes, `~/.mallard`, or
production storage. The clean cutover and native Push/Pull/Re-Push smoke test
must use the reset environment described in section 9.

## 1. Problem

Before this change, Push rejected every source string that contained the
literal token:

```text
/__mallard_root__
```

The rejection was unconditional. It ran before project-path matching. A
conversation therefore became unpublishable when a command printed the token,
even when the token was only test data or discussion text.

Two observed failures had this form:

- A command result printed a test line that contained
  `file:///__mallard_root__/readme`. The same text appeared in `stdout`,
  `aggregated_output`, and `formatted_output`.
- A `git diff` printed path-map test strings that contained
  `/__mallard_root__`.

The failing records were valid provider records. The path-map encoder rejected
their content because it had no way to distinguish these two meanings:

1. A root control token created by Mallard.
2. The same bytes written literally by a user, command, diff, or model.

Changing only the root token would move the collision. Conversations can print
any fixed token. The wire grammar needs a reversible escape rule.

## 2. Fixed decisions

The implementation must keep these values and shapes:

| Item | Required value |
|---|---|
| Bundle schema | `1` |
| Path-map codec | `mallard.pathmap.v1` |
| Codex marker field | `__mallard_path_map__` |
| Codex marker value | `{"codec":"mallard.pathmap.v1"}` |
| Claude path-map stamp | Existing codec-only `wire_path_map` |
| Codex declaration | Existing in-band declaration |
| Claude declaration | Existing out-of-band declaration |

The implementation must not add:

- A schema bump.
- A codec bump.
- A model fingerprint.
- A manifest migration.
- A legacy encoder or decoder.
- A fallback to the old reserved-namespace error.
- A feature flag that selects between old and new grammars.

The implementation replaces the old grammar completely. Old portable data is
out of scope and must not enter the new test environment.

## 3. Replacement wire grammar

Define two fixed tokens:

```text
R = /__mallard_root__
E = /__mallard_escape__
```

`R` remains the project-root control token. `E` is the escape prefix for a
literal control token.

The two escape productions are:

```text
E/r = /__mallard_escape__/r   # literal R
E/e = /__mallard_escape__/e   # literal E
```

The complete source-to-wire mapping is:

| Source lexical item | Portable wire item | Pull result |
|---|---|---|
| Source project path | `R` | Target project path |
| Source project file URI | `file://R` | Target project file URI |
| Literal `R` | `E/r` | Literal `R` |
| Literal `E` | `E/e` | Literal `E` |
| All other text | Unchanged | Unchanged |

`R` begins with `/`. Therefore, `file://R` is the concrete string:

```text
file:///__mallard_root__
```

### 3.1 Examples

Project-path remap:

```text
source: /machine-a/project/src/main.rs
wire:   /__mallard_root__/src/main.rs
pull:   /machine-b/project/src/main.rs
```

Literal root marker:

```text
source: /__mallard_root__/readme
wire:   /__mallard_escape__/r/readme
pull:   /__mallard_root__/readme
```

Project file URI:

```text
source: file:///machine-a/project/readme
wire:   file:///__mallard_root__/readme
pull:   file:///machine-b/project/readme
```

Literal file URI that contains the marker:

```text
source: file:///__mallard_root__/readme
wire:   file:///__mallard_escape__/r/readme
pull:   file:///__mallard_root__/readme
```

Literal escape prefix followed by a root tag:

```text
source: /__mallard_escape__/r
wire:   /__mallard_escape__/e/r
pull:   /__mallard_escape__/r
```

The last example is the recursion test. The decoder emits the literal `E` and
does not scan that emitted value again.

### 3.2 One-pass rule

Encode and decode must each use one left-to-right scan. Replacement output must
never return to the input stream.

This rule is mandatory. Repeated string replacement is order-sensitive and can
turn an escaped literal back into a control token.

At one source position, the encoder classifies one lexical item:

1. A boundary-valid source project URI or path.
2. A literal `R`.
3. A literal `E`.
4. Ordinary text.

The existing leftmost-longest behavior remains. Project-root validation keeps
`R` and `E` out of root spellings, so a path pattern cannot contain a control
token.

At one wire position, the decoder classifies one lexical item:

1. `file://R` as a project URI control.
2. `R` as a project path control.
3. `E/r` as a literal `R`.
4. `E/e` as a literal `E`.
5. Ordinary text.

### 3.3 Canonical wire validation

Every `E` occurrence in wire data must be followed immediately by `/r` or
`/e`. Pull must reject:

```text
/__mallard_escape__
/__mallard_escape__/x
```

Every unescaped `R` occurrence must be a boundary-valid root control. A
boundary-invalid raw `R` is not canonical wire. The existing
`encode(decode(wire)) == wire` gate must reject it.

Generated wire must satisfy the same rules before publication. This check keeps
Push and Pull symmetric.

## 4. Required invariants

### 4.1 Total source encoding

Every valid JSON string and object key is encodable, including strings that
contain any number or arrangement of `R`, `E`, `E/r`, and `E/e`.

The old reserved-token source error no longer exists.

### 4.2 Same-root identity

For a canonical source record `s` and its machine root `A`:

```text
decode_A(encode_A(s)) == s
```

For tilde, canonical-alias, and percent-encoded URI spellings, the right side
is the existing deterministic canonical source respelling.

### 4.3 Cross-root rebase

For source root `A`, target root `B`, and source record `s`:

```text
decode_B(encode_A(s)) == rebase_A_to_B(s)
```

`rebase_A_to_B` changes only boundary-valid project paths and project file
URIs. It preserves literal `R`, literal `E`, and ordinary text.

### 4.4 Injectivity

Two different canonical source strings must not produce the same wire string.
In particular, these three source items must remain distinct:

```text
source project path
literal R
literal E/r
```

Their wire forms are, respectively:

```text
R
E/r
E/e/r
```

### 4.5 Target collision detection

The existing Pull gate remains:

```text
encode_B(decode_B(wire)) == wire
```

It must accept canonical wire when ordinary source content does not become a
project path on machine B. It must reject a target-root collision when that
ordinary content does become a project path on machine B.

Literal control markers must not cause a false target collision. They decode as
literals and re-encode through `E`.

### 4.6 Record traversal

The grammar applies to:

- Every JSON string value.
- Every JSON object key.
- Strings inside arrays and nested objects.
- JSON text embedded inside a JSON string.

Escaping is injective, so it must not create object-key collisions. Existing
path canonicalization and target-root collision checks still fail closed if
two keys converge for another reason.

### 4.7 Out-of-root content

Ordinary out-of-root content remains unchanged. Literal `R` and `E` occurrences
are the only wire-level exception: they are escaped during transport and
restored exactly during Pull.

This qualifies the old byte-identity statement. It does not broaden project
path remapping.

## 5. Implementation design

### 5.1 Replace the source matcher

Change `crates/mallard-infrastructure/src/session_path_map.rs`.

Keep:

- `PATH_MAP_CODEC_V1`.
- `PATH_MAP_FIELD`.
- `ROOT_TOKEN`.
- `URI_ROOT_TOKEN`.
- Current source path spellings.
- Current boundary logic.
- Current canonical-root and file-URI normalization.

Add constants for:

- `ESCAPE_TOKEN`.
- `ESCAPED_ROOT_TOKEN`.
- `ESCAPED_ESCAPE_TOKEN`.

Replace the undifferentiated encode pattern with a typed pattern. The pattern
kind must distinguish:

- Project path.
- Literal root marker.
- Literal escape marker.

Project patterns use the current boundary test. Literal-marker patterns match
at every occurrence. The scan emits two outputs as it does now:

- Canonical wire.
- Canonical source.

For literal markers, canonical source is the original literal marker. For a
project path, canonical source remains the current canonical local spelling.

### 5.2 Delete the obsolete source rejection

Delete:

- `RESERVED_NAMESPACE`.
- The unconditional `input.contains(...)` error in `encode_str`.
- Tests that require the reserved-token error.
- Source comments that state there is no escaping.

Do not keep the rejection as a fallback.

### 5.3 Replace decode handling

Extend the decode grammar with `E/r` and `E/e`.

`decode_str` should return a typed error for malformed escape syntax. If its
signature changes from `Option` to `Result<Option<_>, _>`, propagate the error
through value and object-key traversal.

The decoder must:

- Match full file-URI controls before their embedded raw-root form.
- Decode root controls to the target root.
- Decode `E/r` to literal `R`.
- Decode `E/e` to literal `E`.
- Preserve ordinary text.
- Never scan emitted replacement text again.

Keep the record-level wire round-trip gate after decode.

### 5.4 Keep repair discovery path-only

`find_source_match` must continue to report only a source project-path match.
It must ignore literal `R` and `E` patterns.

If one automaton serves both encoding and repair discovery, repair discovery
must skip literal-marker pattern kinds and continue its search. A marker-only
conversation must not become a path-repair candidate.

### 5.5 Strengthen root validation

Continue to reject project roots that contain the root namespace. Also reject
project roots that contain the escape namespace:

```text
__mallard_root__
__mallard_escape__
```

This keeps the path patterns disjoint from the control grammar.

### 5.6 Preserve provider declarations and schema

No domain or DTO shape change is required.

Keep `ConversationWirePathMap` codec-only. Keep
`PathMapDeclaration::InBand` for Codex and
`PathMapDeclaration::OutOfBand` for Claude. Keep the Codex marker payload
codec-only.

The existing Codex standalone builder and Claude JSONL facade receive the new
behavior through `SessionPathMapper`.

## 6. File scope

Primary implementation:

- `crates/mallard-infrastructure/src/session_path_map.rs`

Provider regression tests:

- `crates/mallard-infrastructure/src/codex_standalone_builder.rs`
- `crates/mallard-infrastructure/src/providers/claude/codec.rs`

Active documentation:

- `docs/CURRENT_METADATA_LAYOUT.md`

Formal artifacts:

- `formal/pathmap-v1/PathMapCodecV1.tla`
- `formal/pathmap-v1/PathMapCodecV1-source.cfg`
- `formal/pathmap-v1/PathMapCodecV1-pair.cfg`
- `formal/pathmap-v1/PathMapCodecV1-wire.cfg`
- `formal/pathmap-v1/PathMapCodecV1-negative.cfg`
- `formal/pathmap-v1/PathMapCutoverV1.tla`
- `formal/pathmap-v1/PathMapCutoverV1.cfg`
- `formal/pathmap-v1/PathMapCutoverV1-negative.cfg`
- `formal/pathmap-v1/README.md`

Do not change these contracts:

- `BUNDLE_SCHEMA`.
- `ConversationWirePathMap`.
- `ConversationThread`.
- `PathMapDeclaration`.
- `src/types.ts`.
- Storage object keys.

Do not commit the TLC jar, generated states, or generated trace files.

## 7. Test plan

### 7.1 String grammar tests

Add cases for:

- One literal `R`.
- One literal `E`.
- Repeated `R` and `E`.
- Adjacent `R` and `E`.
- `E/r` and `E/e` as source text.
- `E/e/r` round-trip behavior.
- A literal marker before and after a project path.
- A marker inside a project-path suffix.
- Raw project paths and file URIs.
- Percent-encoded file URIs.
- Tilde and canonical-root aliases.
- Boundary-invalid project-root prefixes.
- Unicode text adjacent to markers.
- Quotes, backslashes, and embedded JSON text.

### 7.2 Wire rejection tests

Pull must reject:

- A bare `E`.
- `E/x`.
- A truncated escape at the end of a string.
- A boundary-invalid raw `R`.
- A valid root control plus ordinary content that collides with the target
  root.

### 7.3 JSON record tests

Cover markers in:

- String values.
- Object keys.
- Nested arrays and objects.
- Embedded JSON strings.

Use one object with these distinct keys:

```text
source project path
R
E/r
```

Verify that the wire keys stay distinct and that Pull restores the literal
keys.

### 7.4 Codex regression

Replace the current Codex test that expects a reserved-marker failure.

Build a synthetic command-result record whose `stdout`, `aggregated_output`,
and `formatted_output` all contain both `R` and `file://R`. Build another
record that contains a diff with the same text.

Verify:

- Standalone build succeeds.
- The marker literals use `E/r` on the wire.
- Real source project paths still use `R`.
- Decode on another root restores marker literals.
- Re-encode on the target reproduces the same wire record.

### 7.5 Claude regression

Run the same cases through `encode_jsonl_line_gated` and
`decode_jsonl_line_verified`.

A marker-only line is a changed line. It can receive canonical JSON
serialization because the wire bytes must contain the escape form. Its decoded
JSON value must equal the source value.

### 7.6 Model-conformance test

Add a deterministic Rust test that enumerates a small alphabet of actual text
fragments through length four. Include:

- Ordinary text.
- Source raw and URI paths.
- Target raw and URI paths.
- Literal `R` and `E`.
- `/r` and `/e` suffix text.

Check the same properties as the formal model. This test links the abstract
TLA+ model to the Rust implementation. The formal model alone does not verify
the Rust scanner or its boundary code.

## 8. Formal model and completed checks

Two TLA+ models are stored in `formal/pathmap-v1/`:

1. `PathMapCodecV1` models encoding, decoding, escape parsing, cross-root
   rebasing, injectivity, malformed wire, stray root controls, and the target
   collision gate.
2. `PathMapCutoverV1` models the required clean-cutover order.

The run used the official TLA+ command-line tools from
<https://github.com/tlaplus/tlaplus/releases>.

Tool identity:

```text
TLC2 build: 2026.09.03.193042
tla2tools.jar SHA-256:
16b8cd970e07147ff91f126baecba7edd98202e5ab33220a42f8f4358ee94b2b
```

Model identities:

```text
PathMapCodecV1.tla SHA-256:
3ef73f04e20dcbefb04996c50dfb2ee6faba469d9f9f52e43f59bafc11c8880b

PathMapCutoverV1.tla SHA-256:
01a5442848bbf85696781bccb336425572b3146658d306a3dd8a63382ac4783a
```

Completed positive checks:

| Model check | Exhaustive bound | Distinct states | Result |
|---|---:|---:|---|
| Source identity, rebase, well-formed wire, target gate | All 9-symbol source words through length 4 | 7,381 | Pass |
| Codec injectivity | All pairs of source words through length 3 | 672,400 | Pass |
| Valid and malformed wire grammar | All 9-symbol wire words through length 5 | 66,430 | Pass |
| Clean-cutover state machine | Complete four-state graph | 4 | Pass |

TLC reported no property violation in these runs.

The checked codec properties were:

- Same-root source identity.
- Cross-root rebase.
- Well-formed generated wire.
- Exact target-gate acceptance for collision-free canonical wire.
- Codec injectivity over the modeled canonical source alphabet.
- Rejection of unknown and truncated escape forms.
- Rejection of boundary-invalid raw root markers.

The checked cutover properties were:

- Sync implies no old binary is active.
- Sync implies no old local state exists.
- Sync implies no old cloud data exists.
- Sync implies the new binary is installed.

Mutation controls also ran:

- A codec mutation that emitted literal `R` as a root control produced a
  counterexample on the one-item source `literal R`.
- A cutover mutation that enabled sync before the reset produced a
  counterexample on the first unsafe transition.

These are bounded exhaustive model checks. They are not an unbounded proof.
The separate Rust conformance test connects the bounded source grammar to the
implemented scanner.

## 9. Clean cutover

The schema and codec identifiers do not distinguish old and new grammar. The
reset order is therefore a required system precondition.

Use this order:

1. Stop every old Mallard process on every test machine.
2. Remove old `~/.mallard` state.
3. Remove all old Mallard objects from each cloud or local-folder storage.
4. Install the same new Mallard build on every test machine.
5. Start Mallard and recreate storage and project bindings.
6. Push a synthetic conversation that contains `R`, `E`, `E/r`, and project
   paths.
7. Pull it to a different temporary project root.
8. Re-Push it and confirm identical portable wire.
9. Start normal retesting.

Do not enable sync between steps 1 and 5. Do not reuse an old bundle. If the
new implementation fails during retesting, stop it, fix forward, clear state
and storage again, and restart the cutover. Do not add a legacy reader as a
recovery mechanism.

## 10. Implementation order

1. Add the source, wire, JSON-key, Codex, and Claude regression tests. Confirm
   that the current implementation fails the literal-marker cases.
2. Add the formal model files to `formal/pathmap-v1/` and reproduce the
   completed planning checks.
3. Replace the encoder and decoder in `session_path_map.rs`.
4. Remove the reserved-namespace rejection and obsolete tests.
5. Add the implementation conformance test.
6. Update active metadata documentation.
7. Run focused Rust tests.
8. Run the complete backend test suite.
9. Run the formal models again.
10. Perform the clean-cutover native Push/Pull/Re-Push smoke test.

Each step must leave the relevant smaller test set passing before work moves to
the next step.

## 11. Verification commands

Focused checks:

```bash
cargo test -p mallard-infrastructure session_path_map
cargo test -p mallard-infrastructure codex_standalone_builder
cargo test -p mallard-infrastructure providers::claude
```

Required backend handoff:

```bash
npm run test:backend-integration
```

Formal checks:

```bash
java -XX:+UseParallelGC -jar tla2tools.jar -workers auto \
  -config PathMapCodecV1-source.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar tla2tools.jar -workers auto \
  -config PathMapCodecV1-pair.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar tla2tools.jar -workers auto \
  -config PathMapCodecV1-wire.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar tla2tools.jar -workers auto \
  -config PathMapCutoverV1.cfg PathMapCutoverV1.tla
```

Use temporary provider homes, Mallard homes, projects, and storage roots. Do
not run tests against real `~/.codex`, `~/.claude`, `~/.mallard`, or production
storage.

## 12. Acceptance criteria

The replacement is complete only when all statements are true:

- Both observed record shapes Push without a reserved-token error.
- A source literal `R` becomes `E/r` on the wire.
- A source literal `E` becomes `E/e` on the wire.
- Pull restores both literal markers exactly.
- Project paths and project file URIs still rebase to the target root.
- `E/e/r` decodes once to literal `E/r`.
- Unknown and truncated escape forms fail closed.
- Boundary-invalid raw `R` fails wire verification.
- Target-root collision protection still blocks unsafe Pull.
- JSON object keys remain distinct or fail with the existing collision error.
- Marker-only content does not trigger path repair discovery.
- Codex retains its in-band codec marker.
- Claude retains its codec-only out-of-band stamp.
- Bundle schema remains `1`.
- The codec string remains exactly `mallard.pathmap.v1`.
- No legacy codec branch, migration, fallback, or feature flag remains.
- Focused tests, the backend suite, the formal checks, and the native
  Push/Pull/Re-Push smoke test pass.

## 13. Main risks and controls

| Risk | Required control |
|---|---|
| Old and new data share schema and codec identifiers | Stop all old binaries and clear all local and remote data before retesting |
| Escape output is decoded more than once | One-pass, non-recursive decoder plus the `E/e/r` regression |
| Escape patterns alter path repair behavior | Filter repair discovery to project-path patterns only |
| Marker escaping hides a target-root collision | Preserve and test `encode(decode(wire)) == wire` |
| Formal model and Rust scanner diverge | Add exhaustive Rust model-conformance tests |
| Out-of-root byte-identity claim becomes inaccurate | Document literal control namespaces as the sole wire-level exception |

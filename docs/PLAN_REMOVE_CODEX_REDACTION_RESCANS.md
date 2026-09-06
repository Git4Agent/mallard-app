**Plan: Remove the repeated Codex credential scans**

Date: 2026-09-04. Status: implemented and verified.

Remove pass 2 and pass 3 as complete credential scans during Codex Push
apply. Keep source-record redaction. Check new records when projection creates
them. Return certified bytes directly from the Encode builder. Compute the
output content hash once.

Use a clean cutover. Keep machine-local schema 1 and bundle schema 1. Add no
migration, compatibility adapter, feature flag, or legacy execution path.
The user will remove local Mallard state and cloud data for retesting. Data
deletion is not part of this implementation.

**Code basis**

| Current operation | Relevant source | Required change |
|---|---|---|
| Redact source records before projection | `crates/mallard-infrastructure/src/codex_standalone_builder.rs`, `collect_flattened_non_metadata_records` | Keep this order and the existing redaction gate. |
| Verify every projected record for credentials | Same file, Encode output loop | Remove the credential calls from this loop. Keep its path checks. |
| Parse and certify the complete Codex output again | `src-tauri/src/project_sync/service.rs`, `build_publish_request` | Consume the Encode builder's certificate directly. |
| Hash the output in both `artifact_digest` and `PayloadBytes` construction | Builder and `crates/mallard-infrastructure/src/payload.rs` | Construct one immutable payload and reuse its raw digest. |

Projection creates content after source redaction. For example,
`parse_hook_prompt_fragment` decodes XML entities, and the MCP mapping moves
fields into a new object. The detector uses both text and JSON field context.
Check these new records at construction. A blanket removal of their first
check would reduce detection coverage.

Retain redaction before projection. Moving all redaction after projection
would change which source context is available when credentials are removed.

**1. Remove pass 2 from the output loop**

Change `codex_paginated_projection.rs` to accept a fallible callback for each
record it inserts. Keep this callback independent of credential-redaction
types. Route all actual insertions through it: ordinary item projections,
reasoning projections, implicit turn starts, and implicit turn completions.

The Encode builder supplies a callback that calls `verify_wire_value` on the
new record. A detected credential stops the build. The callback does not
redact or alter the new record. Run it only for records that enter the output,
not temporary records used to compare an adoption candidate.

Preserved records and adopted projections retain their source-record proof.
Their only projection mutation is `payload.thread_id`, which the current
detector excludes. Top-level ordinal removal also introduces no text. Add a
regression test for this boundary.

Move the final head credential check next to head identity rewriting and
path-map marker insertion. This checks one constructed metadata record.
Delete the later head credential check and the full projected-body credential
loop. Keep source redaction and its verification of changed records.

Keep every `mapper.verify_wire_record` check. Keep record-layout validation,
projection validation, canonical-chain validation, tail folding, and prefix
witness checks. These establish different properties.

Decode and projection-only structural verification use the same projector
without a credential callback operation. This is a distinct operation purpose,
not an alternate Push path. Decode keeps all checks on incoming wire records
before local path expansion.

**2. Bind the output hash once**

Construct `PayloadBytes` from the final output buffer after the existing
output checks. Its constructor computes one SHA-256. Retain the binary
32-byte digest and the existing lowercase hexadecimal digest from that one
calculation. Provide a crate-private binary-digest accessor.

Change `artifact_digest` to accept this payload. Keep its current framing:
domain, file count, file role, provider ID, byte length, and binary content
digest. Feed the stored binary digest into the outer hash. Do not hash the
hexadecimal spelling and do not replace the content hash with the artifact
hash.

The two digest values remain distinct. Both keep their current meaning.
Add no constructor that accepts arbitrary bytes and a caller-supplied hash.
Keep shared payload allocation and cheap cloning.

**3. Return certified Encode output and remove pass 3**

Give Encode and Decode separate public entry points:

```rust
// Existing artifact metadata stays in one structure.
StandaloneConversationArtifact<RedactedWireBytes> // Encode result
StandaloneConversationArtifact<PayloadBytes>     // Decode result
```

Use a payload type parameter only; add no general transformation framework.
Share the internal reconstruction algorithm. Replace the public
mode-selected builder functions with `encode_standalone_codex_root` and
`decode_standalone_codex_root`. The Encode entry point accepts the caller's
`RedactionRun`, so Push keeps one cache for the operation.

Add a crate-private `RedactedWireBytes::from_verified_payload` constructor.
Use it only at successful Encode completion, after the source, constructed
record, path, and output checks. It consumes the immutable payload without
parsing, scanning, or hashing it again. Document this internal proof boundary.
Expose no public unchecked constructor or mutable byte access.

In `build_publish_request`, consume or cheaply clone the certified Encode
payload. Remove the Codex `certify_wire_document` call and the
`mem::take` of raw artifact bytes. Keep artifact metadata available for
descriptors, checkpoints, and receipts.

Update all Rust callers in the service, adoption checks, history-graph tests,
provider-codec tests, continuation tests, and both Codex live-test targets.
Delete the old public builder entry points. Decode never returns a wire
certificate for locally expanded paths.

Keep document certification for Claude, sidecars, and other inputs that do
not arrive from this certified Codex encoder. Keep detector rules, codec
identifiers, and their fingerprint unchanged.

**4. Verify the change with synthetic fixtures**

Before editing runtime code, pin expected output bytes and digests for small
synthetic fixtures through the current builder. Use fixed identities and
temporary provider homes. Keep only expected fixtures, not an old executable
implementation.

| Case | Required result |
|---|---|
| Clean and credential-bearing source records | Expected output bytes, content hash, artifact hash, tails, and witnesses match. Source credentials become the same placeholders. |
| Existing and adopted `item_completed` records | Identity normalization preserves the redaction proof and record order. |
| New display and reasoning records; implicit turns | Every insertion path invokes the construction check. |
| A synthetic credential assembled by XML entity decoding | Projection is rejected before certification or publication. |
| MCP projection removes source credential context | The copied result retains the placeholder produced before projection. |
| Path markers, escaped literals, and generated file URIs | Existing path round-trip checks still pass or reject as specified. |
| Images, encrypted content, and excluded ID fields | Existing detector exclusions remain unchanged. |
| Push, Pull, append, and republish | Conversation content, identities, and tail coordinates remain valid. Tampered incoming wire content is rejected on Pull. |
| Local-folder and S3-stub publication | Object names and manifest content hashes match the exact uploaded bytes. |

Use full document certification as a test oracle for generated fixtures.
Keep this oracle outside the production Codex Encode path. Add a compile-fail
documentation test that external callers cannot use the internal certificate
constructor.

Run the relevant focused tests during implementation. Then run:

```sh
cargo test -p mallard-infrastructure --lib
cargo test -p mallard-infrastructure --doc
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --workspace --all-targets
```

Use temporary homes and fixture stores. Compile the live-test targets to
check the API cutover; do not execute them against real provider homes.
Do not run performance probes or claim elapsed-time savings.

**Completion criteria**

- Codex Push has no credential verification loop over the complete projected
  output and no final document certification scan.
- Source records retain their first redaction gate. New projection records
  and the final metadata head are checked at construction.
- The Encode result carries immutable certified bytes through publication.
- The output content hash is computed once and reused for the artifact hash.
- Schema values remain 1. No migration or old builder execution path exists.
- The fixture checks pass. Update section 6.2 of
  `docs/PUSH_SLOWNESS_2026-09-04.md` to describe the implemented control flow,
  without new timing claims.

**Verification results**

| Check | Result |
|---|---|
| `cargo test -p mallard-infrastructure --lib` | 412 passed; 3 ignored. |
| `cargo test -p mallard-infrastructure --doc` | 1 passed. External callers cannot use the internal certificate constructor. |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 267 passed. |
| `cargo check --workspace --all-targets` | Passed, including the live-test targets. |
| Formatting and diff whitespace checks for changed files | Passed. |

The ignored tests are one performance benchmark and two tests that read real
Claude data. Live tests were compiled but not run. No performance measurements
were made. Synthetic fixtures preserve the expected output bytes, content and
artifact hashes, tail coordinates, and prefix witnesses.

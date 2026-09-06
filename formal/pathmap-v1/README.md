# `mallard.pathmap.v1` formal checks

These models describe the earlier bare-root grammar. They are design history.
The active codec and verification results are in
[Lossless path references](../pathmap-origins-v1/README.md). The application
has no reader for this older grammar.

Fixed contract:

- Bundle schema remains `1`.
- Codec remains `mallard.pathmap.v1`.
- The Codex marker and Claude stamp shapes remain unchanged.
- Old local and remote data must be removed before the new build enables
  synchronization.

## Tool

Use Java 11 or later and the official `tla2tools.jar` from
<https://github.com/tlaplus/tlaplus/releases>.

The completed design verification used:

```text
TLC2 build: 2026.09.03.193042
tla2tools.jar SHA-256:
16b8cd970e07147ff91f126baecba7edd98202e5ab33220a42f8f4358ee94b2b
```

Set `MALLARD_TLA_JAR` to the absolute jar path. Verify the jar before use:

```bash
shasum -a 256 "$MALLARD_TLA_JAR"
```

Do not add the jar or TLC-generated data to the repository.

## Positive checks

Run these commands from this directory:

```bash
java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCodecV1-source.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCodecV1-pair.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCodecV1-wire.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCutoverV1.cfg PathMapCutoverV1.tla
```

Expected result for each command:

```text
Model checking completed. No error has been found.
```

The checked bounds are:

| Check | Bound | Distinct states |
|---|---:|---:|
| Source identity, rebase, wire form, target gate | Source words through length 4 | 7,381 |
| Injectivity | Every pair of source words through length 3 | 672,400 |
| Valid and malformed wire | Wire words through length 5 | 66,430 |
| Clean cutover | Complete state graph | 4 |

## Mutation controls

The negative configurations prove that the invariants detect the target
faults. They must fail with exit status `12`.

```bash
java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCodecV1-negative.cfg PathMapCodecV1.tla

java -XX:+UseParallelGC -jar "$MALLARD_TLA_JAR" -workers auto \
  -config PathMapCutoverV1-negative.cfg PathMapCutoverV1.tla
```

Expected counterexamples:

- `BrokenSourceIdentity` fails on a literal `/__mallard_root__` source atom.
- `NoLegacyExecution` fails when synchronization starts before the reset.

## Scope

The codec model treats the string scanner as a sequence of lexical atoms. The
Rust bounded conformance test exercises the concrete strings and connects the
model to `SessionPathMapper`. These checks are bounded exhaustive model checks,
not an unbounded theorem proof.

# Qoder fork mechanisms PoC

## Result

This PoC proves the authentication-free, standalone `forkSession()` behavior
in Qoder Agent SDK `1.0.32`. It uses only synthetic state in a guarded temporary
home.

The result is `TRANSCRIPT_ONLY_POC`. It is not proof of a complete Qoder task
fork.

The probe proves these behaviors:

- The source main transcript stays unchanged.
- The SDK generates a new target session UUID.
- The SDK selects the active historical message chain.
- It excludes an unselected main branch.
- It removes sidechain and progress records from the emitted target.
- It remaps copied message UUIDs, parent links, and content replacements.
- It adds `forkedFrom` origin data.
- It neutralizes a copied `model_refusal_fallback` system record.
- It removes provider-runtime ownership fields.
- `upToMessageId` is inclusive.
- A local fork creates one new main JSONL file with mode `0600`.
- A local standalone fork does not copy the session sidecar or the referenced,
  observed-shape file-history checkpoint object.
- An external-store fork calls `load(source)` and then `append(target)`. It
  does not read subkeys or make a separate target preflight or claim. The
  store's `append` operation owns collision policy.

The PoC also proves three SDK `1.0.32` graph-integrity defects:

- A retained record's `logicalParentUuid` can point to the generated UUID of
  a removed progress record.
- A retained content replacement can point to the generated UUID of a removed
  progress record.
- If the selected lineage contains a sidechain parent, sidechain filtering can
  keep its descendant and detach that descendant to a null root.

Normal `parentUuid` traversal through a removed progress record does not have
the first problem in the tested case. Treat all three cases as failed
production gates.

The probe does not call `query()`, start Qoder CLI, authenticate, or request a
model response. It guards Node network and child-process interfaces before the
SDK loads. No call reached a guarded interface.

## Run

Supply the canonical SDK `1.0.32` entry point. Use a clean environment:

```sh
env -i \
  PATH="<minimal-path>" \
  QODER_SDK_PATH="/absolute/package/path/dist/index.js" \
  node --test \
  poc/qoder-fork-mechanisms/qoder-sdk-fork-probe.test.mjs
```

The test skips when `QODER_SDK_PATH` is absent. It rejects a different package
name, package version, bundle SHA-256, indirect path, symbolic link, relative
path, or inherited credential variable.

## Safety boundary

- The probe creates one operating-system temporary root.
- The root name and marker file must match before deletion.
- `HOME`, `QODER_CLI_HOME`, `QODER_CONFIG_DIR`, `TMPDIR`, and the working
  directory stay in that root.
- The test child receives an allowlisted environment.
- The probe does not accept a credential.
- It verifies the exact SDK package name, version, and bundle SHA-256.
- The preload guard rejects network and child-process calls.
- The JSON result contains no UUID-shaped values, absolute paths, fixture
  markers, transcript content, or raw SDK output.
- Cleanup removes only the validated temporary root.

The preload guard is a JavaScript PoC control. It is not an operating-system
network sandbox. Use a disposable VM or equivalent operating-system policy for
an authenticated product test.

## Production boundary

Do not use this result to enable Qoder task fork or session restore in Mallard.
The standalone SDK helper does not copy task state, compression state, subagent
sidecars, file-history checkpoints, or Qoder application-database projections.
The external `SessionStore` interface delegates target collision behavior to
`append`; it requires no separate immutable-create or compare-and-swap target
claim.

An authenticated CLI or SDK `query()` fork test is still required. That test
must prove source immutability, target collision handling, model-context
continuation, checkpoint behavior, sidecar behavior, restart behavior, and
clean-home discovery.

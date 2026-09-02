# Qoder session restore PoC

## Result

This PoC proves offline capture and materialization mechanics with synthetic files.
It does not prove production Qoder session restore.

The machine-readable verdict is:

```json
{
  "productionReady": false,
  "verdict": "POC_ONLY",
  "gates": {
    "activeLeafAuthority": "UNPROVED",
    "qoderAppConsumption": "UNPROVED",
    "qoderApplicationIndexRebuild": "UNPROVED",
    "crossPathIntegrity": "UNPROVED"
  }
}
```

## Run

Use Node.js 20 or a later compatible release. The PoC has no package dependency.

```sh
node --test poc/qoder-session-restore/qoder-session-restore.test.mjs
```

The tests generate all data below operating-system temporary directories. They
remove those directories after each test.

Current result: 15 tests passed and 0 failed.

## Safety boundary

The harness refuses a source, destination, or operation path unless all of these
conditions are true:

- The sandbox base name starts with `mallard-qoder-poc-`.
- The sandbox has the exact `.mallard-qoder-poc-synthetic` marker.
- The sandbox path is canonical and is not a symbolic link.
- Every used path stays below that sandbox.

The harness has no default Qoder path. It does not read or write `~/.qoder` or
an application-support directory. It does not open Qoder, call a Qoder binary,
edit SQLite, or use a private Qoder interface.

## Proved mechanics

The test harness implements these mechanics:

1. It enumerates one allowlisted transcript and sidecar group.
2. It opens regular files with `O_NOFOLLOW`.
3. It records file identity, size, mode, link count, modification time, change
   time, and SHA-256.
4. It reads the complete group twice.
5. It re-enumerates membership between passes.
6. It rejects an addition, replacement, mutation, symbolic link, special file,
   or metadata change.
7. It captures JSON and JSONL sidecars without converting their format.
8. It checks the root file name, session directory, `sessionId`, and
   `session_id` against one source UUID.
9. It reports source-UUID references in fields that have no typed rule.
10. It lists every observed `active-leaf` record as a candidate. It does not
    select a candidate.
11. It rewrites path values only at an explicit logical-file, record-type, and
    JSON-path allowlist.
12. It patches only the selected JSON string tokens. Other string tokens and
    free-form historical text keep their original bytes.
13. It blocks path rewriting when it finds an unproved integrity field such as
    a signature, hash, replay token, or server-correlation field.
14. It preserves the source session UUID.
15. It blocks a create when that UUID exists in any destination bucket or
    mapping.
16. It permits an update only for the same cloud task, destination UUID, and
    project bucket.
17. It pins the complete existing member set by logical name, target name,
    identity, size, and digest.
18. It stages and verifies all new bytes before apply.
19. It backs up and verifies every existing target before overwrite or delete.
20. It records prepared, applying, per-file, rollback, and committed journal
    state.
21. It uses file-level atomic replacement and verifies the final complete group.
22. It deletes an obsolete sidecar only when the reviewed name, identity, size,
    and digest are exact. It does not use recursive provider-file deletion.

## Synthetic tests

The suite covers:

- Missing or invalid synthetic-sandbox boundary rejection.
- Complete transcript, state, compression, subagent JSON, and subagent JSONL
  capture and restore.
- File-set addition, removal, file replacement, and in-place mutation between
  snapshot passes.
- Sidecar symbolic-link rejection.
- New-materialization UUID collision.
- Existing-mapping stale update.
- A source UUID hidden in an unknown record.
- Structural path mapping with byte-identical free-form path text.
- An unproved signature field that blocks path mapping.
- Exact reviewed obsolete-sidecar deletion.
- Conflicting `sessionId` and `session_id` values.
- The explicit non-production verdict.

## Not proved

This PoC does not prove these Qoder contracts:

- Which `active-leaf` record Qoder treats as authoritative.
- Whether Qoder accepts an offline restored transcript and all sidecars.
- Whether Qoder rebuilds its application database and user-interface
  projections from restored files.
- Whether a cross-path change preserves Qoder signatures, hashes, replay state,
  or server correlation.
- Whether the observed sidecar allowlist is stable across Qoder versions.
- Whether Qoder can consume the result in a clean application home.

The installed desktop app has no proved public home or profile selector.
`QODER_CONFIG_DIR` and `--config-dir` apply to the public Qoder CLI contract,
not to this desktop-app PoC. Do not launch the desktop app by replacing `HOME`
or by supplying private Electron flags.

A clean-home consumption and resume test is a separate external gate. Run it
only in a disposable macOS account or virtual machine, or with a future
vendor-supported app-home selector. The documented Data import UI can be a
manual vendor-supported probe, but its task-import scope must be confirmed.

## Companion product probes

Two isolated product probes add narrow evidence:

- Qoder Agent SDK `1.0.32` passed 25 offline transcript-import and store-reader
  assertions. Its importer ignored session state, compression state, and task
  sidecars.
- Qoder CLI `1.1.40` listed one synthetic transcript from a disposable
  `QODER_CONFIG_DIR`. Resume was not run.

Neither probe started Qoder App, authenticated, submitted a model request, or
used a real Qoder profile. See the
[detailed PoC report](../../docs/QODER_SESSION_RESTORE_POC_2026-09-02.md).

## Production work still required

The Node.js code demonstrates policy and data flow. Production Mallard code
must use the Rust safety primitives and add these controls:

- Handle-relative traversal, such as `openat`, to close intermediate-directory
  replacement races.
- Bounded file, byte, line, record, and directory counts.
- Durable directory synchronization where the operating system requires it.
- A common Mallard plan with revision, digest, expiry, trust, and approval pins.
- Crash recovery that replays or rolls back an incomplete journal.
- Version and transcript-dialect gates.
- Qoder vendor approval for reading and restoring user-owned task files.
- A clean-home Qoder list, open, continue, branch, and re-capture test without
  an application-database edit.

Do not enable production session restore until every external gate passes.

## Files

- `qoder-session-restore.mjs` contains the dependency-free harness.
- `qoder-session-restore.test.mjs` contains synthetic integration tests.

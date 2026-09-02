# Qoder CLI resume PoC

## Result

This PoC gives a narrow result for Qoder CLI `1.1.40`:

- The CLI lists a restored synthetic session from an isolated configuration
  directory.
- The guarded restored-synthetic resume invocation stops at Qoder
  authentication.
- List and unauthenticated resume preserve the six-member set, sizes, and
  SHA-256 content.
- A user-operated test created a fresh native session and resumed its exact
  UUID in a second authenticated CLI process. The response recovered the
  source-only marker, and the same transcript changed and grew.
- Direct UUID lookup from a second unrelated working directory did not find
  the source-path bucket.
- Moving the source bucket to a verified backup and making a byte-exact copy
  under the destination-path key let a new process resume the same UUID and
  recover the marker. No structural path rewrite was applied.
- A session-only copy from isolated home A to an independently authenticated,
  initially empty home B also resumed the same UUID and recovered the marker.
  Authentication and machine state did not move with the session.
- The authenticated result does not prove a physical two-machine transfer,
  the real Mallard transport, sidecar consumption, active-leaf authority, or
  Mallard production restore.

The production verdict is `POC_ONLY`. Do not enable Mallard Qoder session
restore from this result.

## Run the authenticated native-resume smoke test

If you can authenticate Qoder CLI, run the separate user-operated smoke test
in [AUTHENTICATED_RESUME.md](AUTHENTICATED_RESUME.md). It creates and resumes
one fresh session in an isolated temporary Qoder configuration. A pass proves
native CLI semantic resume only. It does not prove Mallard session restore.

The 2 September 2026 user-operated run returned
`AUTHENTICATED_NATIVE_RESUME_PASS`. The source and resume commands exited with
structured success, the CLI returned the same session ID, the second response
recovered the marker, and the transcript changed and grew. Cleanup completed,
and no temporary profile was retained.

The first same-path run displayed a macOS Keychain availability dialog because
the runner used `QODER_FORCE_FILE_STORAGE=1`. CLI `1.1.40` checks for the exact
string `true`. The corrected cross-path run used
`QODER_FORCE_FILE_STORAGE=true`, put a blocking `security` command first in
`PATH`, and observed no command attempt or Keychain dialog.

The complete cross-path and App results are in the
[focused PoC report](../../docs/QODER_CROSS_PATH_AND_APP_RESUME_POC_2026-09-02.md).

The same report includes the clean-home transfer result
`AUTHENTICATED_CLEAN_HOME_CROSS_PATH_TRANSFER_PASS`. It is the closest PoC for
the Mallard machine-A push and machine-B pull flow.

The guarded product run has `ok: false`. This result is expected. The CLI
attempted background network and process operations through intercepted
interfaces. The harness blocked and recorded the operations. The result is not
a harness malfunction.

A separate read-only Agent SDK `1.0.32` probe proves branch projection and
duplicate-UUID behavior. It does not authenticate or resume through the CLI.

## Observed product result

The 2026-09-02 run used an isolated npm package bundle for Qoder CLI `1.1.40`.
It produced this evidence:

| Check | Result |
| --- | --- |
| Synthetic group restore | 6 of 6 members verified |
| `--list-sessions` exit | `0` |
| Session ID in list output | Yes |
| Synthetic title in list output | Yes |
| Session content manifest after list | Matches member set, size, and SHA-256 |
| `--resume` exit | `1` |
| Authentication gate | Observed |
| Session content manifest after resume | Matches member set, size, and SHA-256 |
| Credential finding | `0` |
| Blocked network or process requests | `60` to `62` across three product runs |
| Temporary-root cleanup | Complete |

The request count is diagnostic. It can change between runs. The important
result is that the CLI tried these operations while the guard denied them.
The observed operation classes included DNS, Fetch, HTTP, HTTPS, TCP, and a
child process.

The resume output included the synthetic session ID and an authentication
requirement. It did not include the earlier assistant marker or resume prompt.
Therefore, the authentication gate can occur before evidence of transcript
consumption.

## Synthetic session group

The harness restores these files under one computed CLI project bucket:

```text
<session-id>.jsonl
<session-id>/state.json
<session-id>/compression-v2/state.json
<session-id>/subagents/agent-explore-0123456789abcdef.jsonl
<session-id>/subagents/agent-explore-0123456789abcdef.meta.json
<session-id>/subagents/task-fedcba9876543210.json
```

The transcript has these four record types:

1. `workspace-directories`
2. `user`
3. `assistant`
4. `active-leaf`

The workspace record uses `directories`. The active-leaf record uses
`explicit: true`. These fields align with the Qoder CLI `1.1.40` validators
observed during this study.

The source, staging, and restored files use the same SHA-256 manifest. The
harness rechecks all six files after list and resume. It reports a missing,
unexpected, or content-changed member separately.

The sidecar payloads are synthetic. Listing validates enough of the main
transcript to show its UUID and title. It does not validate the schema or use
of root state, compression state, subagent metadata, or task state.

## Project bucket

The harness implements the function observed in Qoder CLI `1.1.40`. The helper
requires an already-canonical absolute project path:

1. Replace each character outside `[A-Za-z0-9]` with `-`.
2. Return the result if its length is 200 characters or less.
3. For a longer result, keep the first 200 characters.
4. Add `-` and the absolute DJB2-XOR hash in base 36.

For example:

```text
/tmp/a b/project -> -tmp-a-b-project
```

This algorithm is product observation. It is not a documented stable Qoder
contract. The result reports only the computed bucket digest, length, and
verification status. It does not expose the path-derived bucket text.

## CLI operations

The list operation is equivalent to:

```text
node qodercli.js \
  --config-dir <synthetic-config> \
  --cwd <synthetic-project> \
  --list-sessions
```

The resume operation uses:

```text
--bare
--resume <synthetic-session-id>
--print
--tools ""
--permission-mode dont_ask
--no-session-persistence
--disable-builtin-skills
--strict-mcp-config
--mcp-config {"mcpServers":{}}
--max-turns 1
--output-format json
```

The resume prompt is a fixed synthetic marker. The harness does not accept a
prompt from the caller.

## Safety boundary

The harness applies these controls:

- It creates one exact operating-system temporary root.
- The root name and marker file must match before use and deletion.
- `HOME`, `QODER_CONFIG_DIR`, the working directory, temporary directories,
  and XDG directories stay under that root.
- It creates the child environment from an allowlist. It does not inherit the
  caller environment.
- It does not accept a Qoder PAT, API key, password, or other credential.
- It verifies the package name and exact Qoder CLI `1.1.40` version.
- It forces Qoder file credential storage into the isolated configuration.
- It sets proxy URLs to loopback port 1 and supplies a preload guard.
- The guard denies Node.js network functions and child-process functions
  before the Qoder bundle loads.
- It bounds standard output and standard error to 1 MiB each.
- It stores only output byte counts, SHA-256 values, and known signal codes.
  It does not return raw CLI output.
- It scans output and regular files for a credential canary and credential
  value shapes.
- It records created, deleted, and modified entries by relative path, type,
  mode, size, content SHA-256, and symbolic-link target SHA-256.
- It records symbolic links without following them.
- It removes only the validated temporary root.

The preload guard is a JavaScript PoC control. It is not an operating-system
network sandbox. Production testing needs an isolated VM or account with an
operating-system egress policy.

The result replaces the path-derived bucket segment in mutation paths with
`<project-bucket>`. It also omits the temporary root, Qoder bundle path, raw
output, and credential canary.

## Run offline tests

The tests use a mock caller-supplied CLI bundle. They do not use the network or
a credential.

```sh
node --test \
  poc/qoder-cli-resume/qoder-cli-resume.test.mjs \
  poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs
```

Current result: 22 passed, zero failed, and zero skipped.

The 10 tests cover:

- short and long project-bucket computation;
- the six-member synthetic fixture;
- isolated list discovery;
- authentication-gate classification;
- create, modify, and delete recording for the measured tree fields;
- session content-manifest preservation;
- raw-output suppression;
- credential-canary rejection;
- transcript-mutation rejection;
- network-attempt blocking and recording;
- CLI symlink rejection;
- relative CLI-path rejection; and
- Qoder CLI package-version rejection.

The 12 authenticated-runner tests use a mock CLI. They cover separate
source and resume processes, marker-channel isolation, environment filtering,
exact transcript growth, exact file-storage configuration, native Keychain
command rejection, direct cross-path execution, destination-bucket
materialization, a session-only two-clean-home transfer, bundle pinning,
interrupt cleanup, retained-state opt-in, and forced termination after excess
output.

## Run the read-only SDK probe

The SDK probe requires a caller-supplied absolute canonical path to Agent SDK
`1.0.32`. Use a clean environment:

```sh
env -i \
  PATH="<minimal-path>" \
  QODER_SDK_PATH="/absolute/package/path/dist/index.js" \
  node --test \
  poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.test.mjs
```

Current result: one passed, zero failed, and zero skipped.

The probe proves these SDK reader behaviors:

- The last matching active-leaf record in file order selects the branch.
- File order wins even when that record has an older timestamp.
- A missing active-leaf record selects the latest main leaf.
- A missing or foreign leaf UUID falls back to the latest main leaf.
- Explicit project scope resolves duplicate UUIDs correctly.
- Unscoped list and get operations can select different copies of one UUID.

The probe tracks all seven synthetic regular files. Their paths, hashes, sizes,
and modification times stay unchanged.

## Run the guarded product probe

Use the package's `bundle/qodercli.js` file. Do not use its desktop-aware npm
dispatcher. Supply an absolute, canonical, regular-file path:

```sh
node poc/qoder-cli-resume/qoder-cli-resume.mjs \
  --cli-bundle /absolute/package/path/bundle/qodercli.js \
  --timeout-ms 30000
```

The harness does not install Qoder CLI. If an isolated package is required,
install it with lifecycle scripts disabled. The package postinstall operation
can change the user's path configuration.

The command can return `ok: false` with
`offline_guard_intercepted_operation`. Read the evidence and mutation records.
Do not disable the guard to make the result green.

## Not proved

This PoC does not prove:

- the same transfer on two physical machines;
- the real Mallard push, storage, review, and pull pipeline;
- that resume reads `state.json`, compression state, or subagent sidecars;
- which active-leaf record authenticated Qoder CLI `1.1.40` treats as
  authoritative;
- that the Qoder App discovers or continues the CLI session;
- that the bucket algorithm stays compatible in another Qoder version; or
- that JavaScript interception blocks every native egress path.

## Required next gate

Run the clean-home authenticated gate in a disposable operating-system
account or VM with enforced egress recording and a disposable Qoder account.
Capture these facts:

1. The exact files read before the first model request.
2. The selected active leaf.
3. The first continued user and assistant records.
4. The session UUID before and after continuation.
5. All sidecar changes.
6. All application-database projection changes.
7. Every network destination and credential boundary.

Do not copy authentication from the real Qoder home. Do not test against the
real `~/.qoder`.

## Files

- `AUTHENTICATED_RESUME.md` is the user-run authenticated smoke-test guide.
- `qoder-cli-authenticated-resume.mjs` creates and resumes one native session
  in an isolated Qoder configuration.
- `qoder-cli-authenticated-resume.test.mjs` tests the guided runner without a
  credential or network request.
- `qoder-cli-resume.mjs` contains the dependency-free harness.
- `qoder-cli-resume.test.mjs` contains mocked offline tests.
- `qoder-sdk-active-leaf-probe.mjs` contains the isolated read-only SDK probe.
- `qoder-sdk-active-leaf-probe.test.mjs` runs the version-pinned SDK product
  test.

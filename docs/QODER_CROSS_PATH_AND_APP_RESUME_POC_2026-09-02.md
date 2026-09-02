# Qoder cross-path CLI resume and App resume PoC

**Status:** complete, version-pinned PoC

**Date:** 2026-09-02

**Scope:** Qoder CLI cross-path and clean-home transfer resume, and Qoder App
resume

## 1. Decision

The cross-path CLI gate passes with an important condition. Mallard must place
the session transcript and its sidecar directory in the project bucket for the
destination working directory. Qoder CLI does not find the session across two
unrelated project buckets by UUID alone.

The clean-home transfer gate also passes in a two-home machine simulation. A
session created in an isolated machine-A Qoder home at `/A/project_a` continued
in an independently authenticated, initially empty machine-B Qoder home at
`/B/project_B`. The transfer contained only the session group. It did not
contain authentication or machine state.

The Qoder App resume gate does not pass in the safe test configuration. A clean
Qoder App database did not create a projection for the valid CLI session. The
App had no task to open or continue.

These results do not make production Qoder session restore ready.

| Gate | Previous result | PoC result | Meaning |
| --- | --- | --- | --- |
| Cross-path CLI resume | Unproved | **Pass, limited** | Direct lookup from an unrelated path failed. A byte-exact copy in the destination project bucket resumed the same UUID and recovered prior semantic context. |
| Clean-home CLI transfer | Unproved | **Pass, limited** | A session-only push from home A and pull into independently authenticated clean home B resumed the same UUID and recovered prior semantic context. |
| Qoder App projection rebuild | Unproved | **Does not pass in the isolated App test** | The App created a clean schema-64 database, but `chat_sessions` stayed empty. |
| Qoder App discovery | Unproved | **Does not pass in the isolated App test** | The App did not register the restored CLI UUID. |
| Qoder App continuation | Unproved | **Not reached** | Continuation requires an App database projection. |
| Production restore | Not ready | **Not ready** | A physical second machine, the real Mallard transport, complete sidecar rules, destination-wide collision control, and packaged-App behavior remain open. |

`ok: true` in a probe result means that the probe completed and produced a
valid observation. It does not mean that the tested product gate passed. The
App probe therefore returned `ok: true` with the negative verdict
`QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS`.

## 2. Tested builds

| Component | Tested identity |
| --- | --- |
| Qoder CLI | `@qoder-ai/qodercli` `1.1.40` |
| Qoder CLI bundle SHA-256 | `a5b9d9f116c16c512fd3000a62cb87e6d7e72ff4edbb196e24773834cef13eba` |
| Qoder App | `0.1.4` |
| Qoder App main-process SHA-256 | `c8b0230de432f9dbe6c027884005504e1348119ee6b3632fbe04714bb2a3ef19` |
| Qoder Agent SDK in the App | `1.0.30` |
| Qoder CLI version declared by the App | `1.1.35` |
| Test Electron runtime | `43.1.1` |
| Node.js | `20.19.4` |
| Host | macOS `26.6.1`, arm64 |

The App test used files extracted from the installed Qoder App package. It ran
those files with the matching generic Electron runtime so that Qoder's internal
unpackaged E2E mode could select a disposable App profile. The packaged Qoder
App was not run against its normal profile.

## 3. Cross-path CLI resume

### 3.1 Question

Can Qoder CLI continue one native session when the resumed process uses a
different working directory?

Two cases were necessary:

1. Resume by UUID from a second path while the session stays in the source
   project bucket.
2. Materialize the complete observed session bucket under the second path's
   project-bucket key, then resume by UUID.

The paths were two different empty, non-Git directories in one disposable test
root. The source and destination were not Git worktrees of the same repository.

### 3.2 Source session

The authenticated harness did the following work:

1. Created a fresh Qoder configuration root.
2. Authenticated through Qoder's browser device flow.
3. Created one persistent session in source path A.
4. Asked the model to remember a random marker.
5. Verified the marker response and the native JSONL transcript.
6. Kept the test root only for the two remaining PoCs.

The result did not print the marker, session UUID, prompt, model output,
credential value, or local test path.

### 3.3 Direct lookup from path B

The second Qoder process used all of these controls:

- the same isolated Qoder configuration root;
- `--cwd` set to destination path B;
- `--resume <source-uuid>`;
- a new operating-system process;
- no tools;
- no MCP servers;
- no built-in skills;
- one model turn; and
- a resume prompt that did not contain the random marker.

Qoder rejected the session UUID. Its version-pinned diagnostic said that it
searched the current project and worktrees of the same repository. It searched
the path-B bucket, not the unrelated path-A bucket.

Result:

```text
AUTHENTICATED_CROSS_PATH_RESUME_NOT_PROVED
resume_failed
```

This observation is stronger than the prior `Unproved` result:

- UUID lookup is project-scoped for unrelated directories in CLI `1.1.40`.
- Mallard cannot leave the transcript only under the source path's bucket and
  expect `--resume <uuid>` from an arbitrary destination path to find it.

The result does not cover Git worktrees. Qoder explicitly identified
same-repository worktrees as another search scope.

### 3.4 Destination-bucket materialization

The materialization probe then changed only the retained disposable Qoder
configuration:

1. It moved the active source project bucket to a recoverable backup in the
   same disposable root.
2. It verified every source member by size and SHA-256.
3. It copied every member to the project bucket computed for destination path
   B.
4. It verified that the destination was byte-exact before resume.
5. It did not change any JSON or JSONL value.
6. It started Qoder in another process with destination path B and the source
   UUID.
7. It verified the returned UUID, the recovered marker, and transcript growth.
8. It verified that the protected source backup stayed unchanged.

The observed source bucket contained:

- the session JSONL transcript; and
- the session directory with `state.json`.

The machine-readable probe counts directories and files. It therefore reported
three source members: one session directory and two files.

After resume, Qoder added `compression-v2/state.json` under the session
directory. The resulting tree had two directories and three files. The
original `state.json` stayed byte-identical.

The destination transcript grew from 5,425 bytes to 10,549 bytes. Its complete
historical prefix stayed byte-identical to the protected source transcript.
New records used destination path B as their `cwd`.

The final result was:

```json
{
  "ok": true,
  "verdict": "AUTHENTICATED_MATERIALIZED_CROSS_PATH_RESUME_PASS",
  "productionRestoreReady": false,
  "source": {
    "sourceBackupUnchanged": true,
    "sourceSessionUuidPreserved": true
  },
  "materialization": {
    "destinationBucketCreated": true,
    "memberCount": 3,
    "memberCountAfterResume": 5,
    "byteExactBeforeResume": true,
    "structuralPathMappingApplied": false
  },
  "resume": {
    "sameSessionId": true,
    "markerRecovered": true,
    "transcriptGrew": true,
    "cliWorkingDirectoryChanged": true,
    "transcriptStorage": "destination_project_bucket"
  }
}
```

### 3.5 Cross-path conclusion

Cross-path resume works for this native session when the session bucket is
materialized for the destination path.

No internal path rewrite was necessary for semantic continuation. This does
not show that keeping historical source paths is correct for production.
Historical transcript records still contain path-A metadata. New records
contain path-B metadata. A future production design must decide whether to keep
this mixed history or map approved structural path fields.

The smallest valid Mallard rule is:

```text
source session bucket
        |
        | verified, complete materialization
        v
destination path bucket
        |
        | qoder --cwd <destination> --resume <same-uuid>
        v
same session continues
```

Mallard must not implement a global transcript search as a substitute for this
rule. Qoder owns the project-bucket lookup behavior.

### 3.6 Clean-home machine-A to machine-B transfer

#### 3.6.1 Question

Can Qoder CLI continue this flow?

```text
machine A: /A/project_a
    |
    | Mallard push of CLI session state only
    v
portable session bundle
    |
    | Mallard pull into the bucket for /B/project_B
    v
machine B: /B/project_B
    |
    | qoder --resume <same-uuid>
    v
continued session
```

Direct cross-path lookup does not need to pass for this flow. The pull
operation changes the storage bucket. It does not ask Qoder to search the old
path-A bucket from path B.

#### 3.6.2 Isolation model

The live test used one physical macOS host and two separate disposable machine
profiles. This configuration isolated all state that is relevant to the tested
CLI lookup and resume flow:

| Boundary | Machine A | Machine B |
| --- | --- | --- |
| `HOME` | Separate empty directory | Separate empty directory |
| `QODER_CONFIG_DIR` | Separate empty directory | Separate empty directory |
| Project path | Temporary equivalent of `/A/project_a` | Temporary equivalent of `/B/project_B` |
| Authentication | Qoder browser device flow A | Separate Qoder browser device flow B |
| Session state before pull | One new native session | None |
| Keychain | Disabled and guarded | Disabled and guarded |

The two login flows had separate device challenges and separate machine
identifiers. Both flows used the same user-selected Qoder account. No
authentication file moved from A to B.

This is a clean-home and separate-profile test. It is not proof from two
physical computers.

#### 3.6.3 Push boundary

The source turn created one native session and asked the model to remember a
random marker. The push operation then did the following work:

1. Found the transcript by the UUID returned by Qoder.
2. Read only the source project bucket.
3. Accepted only `<uuid>.jsonl`, `<uuid>/`, and members below `<uuid>/`.
4. Rejected symlinks, special files, extra top-level members, and size-limit
   violations.
5. Calculated the size and SHA-256 digest of each file.
6. Verified the complete source tree again before the copy.
7. Made a byte-exact portable copy.
8. Verified the source and portable copies again after the copy.

The observed portable group had three members:

- `<uuid>.jsonl`;
- `<uuid>/`; and
- `<uuid>/state.json`.

The portable group did not contain auth data, a machine identifier, settings,
logs, caches, or App database data.

#### 3.6.4 Pull boundary

Before the pull, machine B had an empty Qoder configuration. The test then
authenticated machine B independently. It verified that the destination
project bucket did not exist after login.

The pull operation computed the Qoder bucket key from path B and copied the
portable group into that new bucket. It used exclusive file creation and
verified all sizes and SHA-256 digests. It did not edit JSON or JSONL values.
The UUID did not change.

#### 3.6.5 Resume result

A new Qoder process used the machine-B home, the machine-B configuration, path
B, and `--resume <source-uuid>`. The resume prompt did not contain the marker.

The live result was:

```json
{
  "schemaVersion": 1,
  "ok": true,
  "verdict": "AUTHENTICATED_CLEAN_HOME_CROSS_PATH_TRANSFER_PASS",
  "productionRestoreReady": false,
  "authentication": {
    "destinationAuthenticatedIndependently": true,
    "credentialValueReturned": false,
    "authenticationCopied": false
  },
  "source": {
    "nativeTranscriptFound": true,
    "stableCapture": true,
    "unchangedAfterDestinationResume": true
  },
  "transfer": {
    "sessionOnlyBundle": true,
    "authenticationCopied": false,
    "machineStateCopied": false,
    "memberCount": 3,
    "byteExactPush": true,
    "byteExactPull": true,
    "destinationCollisionChecked": true,
    "sourceSessionUuidPreserved": true,
    "structuralPathMappingApplied": false
  },
  "resume": {
    "exitZero": true,
    "sameSessionId": true,
    "markerRecovered": true,
    "transcriptChanged": true,
    "transcriptGrew": true,
    "sourceAndDestinationPathsDiffer": true,
    "sourceAndDestinationConfigRootsDiffer": true,
    "transcriptStorage": "destination_project_bucket"
  },
  "isolation": {
    "cleanDestinationConfigBeforeLogin": true,
    "separateDisposableMachineHomes": true,
    "nativeKeychainCommandAttempted": false
  },
  "evidence": {
    "memberCountAfterResume": 5
  }
}
```

Qoder returned the source UUID and the exact source-only marker. The
destination transcript kept the complete source transcript as an unchanged
prefix and added the resume records and an active-leaf record. New records used
path B. Qoder also added `compression-v2/state.json`. The protected source and
portable copies did not change.

The harness removed both disposable profiles after verification. It did not
retain either profile's file-backed login state or either transcript.

#### 3.6.6 Answer

**Yes. Qoder CLI `1.1.40` can continue the pulled session in this tested
machine-A to machine-B model.** Mallard must do these operations:

1. Push the complete, stable session group from the source path bucket.
2. Do not push authentication or machine state.
3. Require Qoder authentication on machine B.
4. Compute the project-bucket key from the canonical path on machine B.
5. Pull the unchanged session UUID and complete session group into that bucket.
6. Start Qoder at path B with `--resume <same-uuid>`.

The test does not require direct cross-path lookup. It also does not require a
path rewrite for basic semantic continuation.

## 4. Qoder App resume

### 4.1 Question

Can a clean Qoder App profile discover and continue the valid materialized CLI
session without copying or editing the App database?

Discovery is a required precondition for continuation. The Qoder task guide
describes opening an existing task from the App task list or search, then
continuing it in the composer. A task that has no App projection cannot enter
that flow.

### 4.2 Safe App test configuration

The installed App has no reviewed public command-line selector for a separate
App database root. Running the packaged App could therefore touch the normal
Qoder App profile.

The PoC used Qoder `0.1.4`'s internal unpackaged E2E smoke mode instead. The
installed main process contains this mode and rejects it when `app.isPackaged`
is true. The probe supplied:

- a new App profile root;
- a new home directory;
- a new temporary directory;
- the isolated CLI configuration through `QODER_CONFIG_DIR`;
- Qoder's internal fake E2E user;
- a fake local runtime used only for startup checks;
- demo-data mode to disable live collaboration work;
- no inherited token or API-key environment variables; and
- no normal Qoder home path.

The App created its own `main.sqlite`. The probe did not copy, seed, edit, or
replace that database. It stopped the App after startup and queried the database
read-only with SQLite.

### 4.3 Database result

| Check | Result |
| --- | --- |
| App started | Pass |
| Internal fake authentication initialized | Pass |
| Clean App profile used | Pass |
| `main.sqlite` created by Qoder | Pass |
| Database schema `user_version` | `64` |
| `chat_sessions` table exists | Pass |
| Total `chat_sessions` rows | `0` |
| Rows matching the native CLI UUID | `0` |
| CLI transcript unchanged | Pass |
| App discovered the restored session | No |
| App continuation attempted | No; discovery failed first |

The machine-readable result was:

```json
{
  "ok": true,
  "verdict": "QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS",
  "productionRestoreReady": false,
  "app": {
    "name": "Qoder",
    "version": "0.1.4",
    "executionMode": "unpackaged_e2e_smoke"
  },
  "source": {
    "nativeCliTranscriptFound": true,
    "transcriptUnchanged": true
  },
  "database": {
    "initialized": true,
    "chatSessionsTableFound": true,
    "totalSessions": 0,
    "sourceSessionRows": 0,
    "schemaUserVersion": 64,
    "projectionRebuilt": false
  },
  "resume": {
    "appDiscoveredSession": false,
    "continuationAttempted": false,
    "qoderAppResumeProved": false
  }
}
```

### 4.4 App source evidence

Static inspection of the same installed build agrees with the runtime result:

- the App task list reads `chat_sessions` from `main.sqlite`;
- opening a task expects an App-registered session;
- creating a provisional App task inserts a `chat_sessions` row;
- the reviewed migration sources are Qoder IDE Quest and QoderWork databases;
  and
- no reviewed startup path imports current Qoder CLI project transcripts into
  `chat_sessions`.

Static inspection is supporting evidence only. The clean-database runtime
result is the primary PoC result.

### 4.5 App conclusion

The App resume gate does not pass for Qoder App `0.1.4` in the isolated E2E
mode. A valid CLI transcript is not sufficient to rebuild the App projection.

The PoC does not claim that every packaged-App path fails. The packaged App was
not tested because it has no safe, reviewed disposable-profile selector. The
internal E2E mode can also differ from production startup behavior. Therefore:

- do not add Qoder App session restore to Mallard;
- do not copy or edit `main.sqlite` as a workaround;
- keep App resume disabled until Qoder documents an import or projection-rebuild
  contract, or a safe packaged-App test proves one; and
- do not treat CLI resume as proof of App resume.

## 5. Authentication and Keychain controls

The user required that the PoC not save credentials to the macOS Keychain.

### 5.1 CLI controls

The CLI probe:

- set `QODER_FORCE_FILE_STORAGE=true`;
- placed a fail-closed `security` command before the system command in `PATH`;
- stored browser-login state only under the isolated Qoder configuration;
- used separate file-backed login state for machine A and machine B;
- did not copy login state, auth files, or machine identifiers from A to B;
- detected no `security` command attempt;
- did not inherit personal-access-token, Tuzi, or OpenAI credential variables;
  and
- contained no runner path or operation that reads or copies the normal
  `~/.qoder` directory.

The retained CLI test root existed only long enough to feed the App probe. It
was removed after verification because it contained isolated login state.

### 5.2 App controls

The App probe:

- used Qoder's internal fake E2E user;
- returned from authentication initialization before persisted credential
  loading or saving;
- started Electron with `--use-mock-keychain`;
- started Electron with background networking and media-route discovery
  disabled;
- placed a fail-closed `security` command first in `PATH`;
- observed no `security` command attempt;
- found no `auth.v1.dat` in the disposable App profile; and
- removed the disposable App profile after the query.

Chromium documents `--use-mock-keychain` as a test switch that avoids the real
macOS Keychain path. The test also verified Qoder's own persisted-auth file
boundary.

These environment and command controls are not a complete operating-system
filesystem or system-call audit. The result does not claim that every opaque
native operation was observed.

## 6. PoC implementation

| File | Purpose |
| --- | --- |
| [`../poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs`](../poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs) | Authenticated same-path, direct cross-path, destination-bucket materialization, and two-clean-home transfer probes |
| [`../poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs`](../poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs) | Offline process, two-home isolation, session-only transfer, Keychain-guard, path, and materialization tests |
| [`../poc/qoder-app-resume/qoder-app-projection-probe.mjs`](../poc/qoder-app-resume/qoder-app-projection-probe.mjs) | Clean-profile App launch and read-only projection check |
| [`../poc/qoder-app-resume/qoder-app-projection-probe.test.mjs`](../poc/qoder-app-resume/qoder-app-projection-probe.test.mjs) | Offline App-profile, database, symlink, sanitization, and unsafe-root tests |

No Mallard production source file changed for this PoC.

## 7. Reproduction

### 7.1 Dependencies

The test used the user-specified npm registry:

```sh
export NPM_CONFIG_REGISTRY="https://artifacthub-iad.oci.oraclecorp.com/api/npm/npmjs-registry"
export NPM_CONFIG_USERCONFIG="/dev/null"
```

The pinned Qoder CLI package was installed without lifecycle scripts. The App
source was extracted from the installed `app.asar` into an operating-system
temporary directory. Electron `43.1.1` was installed into another temporary
directory. npm succeeded, so no Homebrew fallback was used.

Do not extract these dependencies into the repository. Do not commit App
packages, Qoder configuration, transcripts, databases, or credentials.

### 7.2 Offline tests

```sh
node --test \
  poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs \
  poc/qoder-cli-resume/qoder-cli-resume.test.mjs \
  poc/qoder-app-resume/qoder-app-projection-probe.test.mjs
```

### 7.3 Direct cross-path test

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --cross-path \
  --keep \
  --cli-bundle /absolute/temp/path/bundle/qodercli.js \
  --timeout-ms 600000
```

This command is expected to fail for two unrelated directories in CLI
`1.1.40`. Keep the root only when the materialization test follows immediately.

### 7.4 Materialized cross-path test

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --materialize-retained-root /absolute/validated/retained-root \
  --cli-bundle /absolute/temp/path/bundle/qodercli.js \
  --timeout-ms 600000
```

The command moves the source bucket to a recoverable backup inside the same
validated disposable root. It then creates and verifies the destination bucket.
Do not point this option at a normal Qoder home.

### 7.5 Clean-home transfer test

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --clean-home-transfer \
  --cli-bundle /absolute/temp/path/bundle/qodercli.js \
  --timeout-ms 600000
```

This command creates two empty disposable homes and two different project
paths. It starts a separate browser device-flow login for each home. It pushes
only one new native session group from A, pulls that group into the path-B
bucket, and resumes the UUID from B. It removes both homes by default.

Do not use `--keep` unless a failed run needs inspection. A retained root can
contain file-backed login state for both profiles and session data.

### 7.6 App projection test

```sh
node poc/qoder-app-resume/qoder-app-projection-probe.mjs \
  --electron /absolute/temp/path/Electron \
  --app-source /absolute/temp/path/extracted-qoder-app \
  --app-resources /Applications/Qoder.app/Contents/Resources \
  --qoder-config /absolute/validated/retained-root/qoder-config \
  --timeout-ms 60000
```

The Qoder App test can show a Qoder Demo window for a few seconds. It does not
use the normal Qoder App profile.

## 8. Verification completed

The committed offline suites passed:

```text
qoder-cli-authenticated-resume.test.mjs: 12 passed, 0 failed
qoder-cli-resume.test.mjs:               10 passed, 0 failed
qoder-app-projection-probe.test.mjs:      4 passed, 0 failed
Total required offline checks:           26 passed, 0 failed
```

The full wildcard run also found the optional SDK active-leaf test. It skipped
that test because `QODER_SDK_PATH` was not set.

The live product probes produced:

```text
Direct unrelated-path lookup:  AUTHENTICATED_CROSS_PATH_RESUME_NOT_PROVED
Materialized destination path: AUTHENTICATED_MATERIALIZED_CROSS_PATH_RESUME_PASS
Clean-home A-to-B transfer:     AUTHENTICATED_CLEAN_HOME_CROSS_PATH_TRANSFER_PASS
Clean App projection:          QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS
```

## 9. Remaining production gates

The two PoCs close only the named questions. They do not close these gates:

- the same transfer on two physical machines;
- the real Mallard push, storage, review, and pull pipeline;
- capture and materialization of every production session member;
- proof that `state.json` and compression state are consumed correctly;
- authoritative `active-leaf` selection;
- source-UUID collision control and destination-wide serialization;
- a product rule for historical source-path fields;
- a no-follow, bounded, atomic Mallard restore implementation;
- a supported Qoder App import contract;
- a safe packaged-App disposable-profile selector;
- packaged-App discovery and continuation; or
- a complete operating-system access audit.

The correct current product boundary is:

- Qoder CLI cross-path resume can be treated as **technically possible with
  destination-bucket materialization**;
- Qoder CLI transfer to an independently authenticated clean home can be
  treated as **technically possible for the tested native session**;
- Qoder App session restore remains **disabled**; and
- `productionRestoreReady` remains `false`.

## 10. References

- [Qoder CLI commands and parameters](https://docs.qoder.com/cli/cli-reference)
- [Qoder CLI task execution](https://docs.qoder.com/cli/run-tasks)
- [Qoder CLI configuration and file paths](https://docs.qoder.com/cli/settings-reference)
- [Qoder App task management](https://docs.qoder.com/qoder/task-management)
- [Chromium macOS test switches](https://chromium.googlesource.com/chromium/src/+/110.0.5481.77/docs/mac_build_instructions.md)

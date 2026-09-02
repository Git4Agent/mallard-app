# Qoder CLI Resume Proof-of-Concept Study

**Date:** 2 September 2026

**Repository:** Mallard

**Result:** `POC_ONLY`

**Production decision:** No-go for raw Qoder CLI session restore

**Scope:** Qoder CLI session discovery, resume, continue, fork, project identity,
authentication, active conversation lineage, collision behavior, sidecars, and
clean-home proof requirements

**Out of scope:** Qoder App modification, Qoder pets, production Mallard
integration, credential export or inspection, and changes to the user's normal
Qoder data

**Related research:**
[Qoder support feasibility research](QODER_SUPPORT_FEASIBILITY_RESEARCH_2026-09-01.md),
[Qoder session restore PoC](QODER_SESSION_RESTORE_POC_2026-09-02.md),
[Qoder CLI custom-provider and Tuzi PoC](QODER_CUSTOM_PROVIDER_POC_2026-09-02.md),
and [Qoder fork mechanisms research and PoC](QODER_FORK_MECHANISMS_RESEARCH_2026-09-02.md)

## Executive result

Qoder CLI has public session-resume functions. The tested CLI discovers a
restored synthetic session in a clean configuration directory. A separate
user-operated product run also proves semantic resume of one fresh native
session in a second authenticated CLI process. However, the PoC does not prove
successful continuation from copied local files.

The raw-copy product result is:

```text
Qoder CLI 1.1.40
  -> clean synthetic HOME and QODER_CONFIG_DIR
  -> verified six-file session materialization
  -> --list-sessions finds the UUID and title
  -> --resume <UUID> reaches Qoder authentication
  -> six-file member set, sizes, and SHA-256 content stay unchanged
  -> no credential is supplied and no live model response completes
```

The separate native-session result is:

```text
Qoder CLI 1.1.40
  -> isolated HOME, QODER_CONFIG_DIR, and empty exact cwd
  -> interactive authentication in the isolated profile
  -> one fresh persistent source session
  -> second process uses --resume <source UUID>
  -> same UUID and source-only marker returned
  -> source transcript changes and grows
  -> temporary profile cleanup completes
```

The first authenticated run displayed a macOS Keychain availability dialog.
The runner supplied `QODER_FORCE_FILE_STORAGE=1`, but CLI `1.1.40` checks for
the exact string `true`. The semantic-resume evidence remains valid. The run
does not prove that Qoder avoided Keychain access. The corrected runner uses
`true` and blocks the observed `security` command probe. It requires one more
user run to close that narrow check.

In CLI `1.1.40`, normal headless startup attempts authentication before source
resolution. However, it can retain the login error and continue through source
resolution and local fork preparation before it returns the final error.
Therefore, failed authentication is not a safe no-read or no-write boundary.
It still does not prove sidecar consumption, intended branch selection, or
semantic continuation.

The result is not a production restore contract. Qoder documents CLI resume,
but it does not document raw local-file copy as a migration interface. The
Qoder Agent SDK external `SessionStore` is the supported portable history
interface. It does not carry file checkpoints, task sidecars, authentication,
or Qoder App database projections.

### Decision summary

| Question | Result | Evidence strength |
| --- | --- | --- |
| Does Qoder CLI support resume? | Yes | Public Qoder contract |
| Can the CLI use a separate configuration directory? | Yes | Public contract and product run |
| Can the CLI list a copied synthetic transcript? | Yes | Product run on CLI `1.1.40` |
| Did the CLI complete an authenticated native resume? | Yes, limited | User-operated product run with one fresh native session |
| Did the auth-gated invocation change the restored content manifest? | No | Member-set, size, and SHA-256 checks for all six members |
| Did the six-file invocation prove that Qoder used the copied transcript for model context? | No | The authenticated run used a new native session, not the copied six-file fixture |
| Can source resolution occur despite failed authentication? | Yes, limited | Package trace and differential product probe |
| Did the invocation prove sidecar use? | No | No sidecar-specific effect was observed |
| Is the local project-bucket algorithm stable? | No guarantee | Version-specific package observation |
| Is duplicate UUID selection safe for Mallard? | No | The CLI and SDK can select one duplicate implicitly |
| Is active-leaf behavior documented as a raw-file contract? | No | SDK projection is documented; raw encoding is not |
| Is the SDK external store a complete task backup? | No | It stores history, not checkpoints or application state |
| Is production Qoder session restore ready? | No | Critical product and clean-home gates remain open |

Keep Qoder session restore disabled in Mallard.

### User-run authenticated smoke test

The repository includes a guided authenticated test for this narrow gate. See
[Authenticated Qoder CLI native-resume smoke test](../poc/qoder-cli-resume/AUTHENTICATED_RESUME.md).

The runner uses a new temporary `HOME`, `QODER_CONFIG_DIR`, empty project, and
interactive Qoder login. It creates one native persistent session, starts a
second CLI process with `--resume <source-id>`, and checks all of these facts:

- the resumed result has the source session ID;
- the resumed model returns a random marker that is absent from its prompt;
- the expected source JSONL transcript changes and grows; and
- the ordinary project directory stays empty.

The user ran the test with the pinned Qoder CLI `1.1.40` package. The sanitized
result was:

| Check | Result |
| --- | --- |
| Verdict | `AUTHENTICATED_NATIVE_RESUME_PASS` |
| Source command | Structured success |
| Resume command | Structured success in a second process |
| Source session ID preserved | Yes |
| Source-only marker recovered | Yes |
| Source transcript changed | Yes |
| Source transcript grew | Yes |
| Exact working directory reused | Yes |
| Empty project stayed empty | Yes |
| Temporary-root cleanup | Complete |
| Keychain-free execution | Not proved; a Keychain availability dialog appeared |

This result proves native CLI semantic resume for one fresh session. It does
not prove Mallard capture, copied-session restore, sidecar consumption,
active-leaf authority, cross-path mapping, Qoder App projection, or Tuzi
routing.

### Keychain dialog diagnosis

Static inspection of the hash-pinned CLI `1.1.40` bundle found this
version-specific flow:

1. Qoder reads `QODER_FORCE_FILE_STORAGE`.
2. This storage path compares the value with the exact string `true`.
3. If the comparison is false, Qoder initializes native credential storage.
4. Native initialization performs a temporary write, read, and delete probe.
5. If native storage is unavailable or the probe is canceled, Qoder falls
   back to its encrypted file store.

The original runner passed `1`. This value is accepted by some other Qoder
boolean parsers, but it is not accepted by this credential-storage comparison.
The Keychain dialog was therefore a runner defect, not a requirement for CLI
resume.

The corrected runner applies two controls:

- It passes `QODER_FORCE_FILE_STORAGE=true` in a child environment that it
  builds from an allowlist.
- It places a test-owned `security` executable first in `PATH`. This executable
  records and rejects an invocation. The runner checks the record after login,
  source, and resume. Any invocation returns
  `native_keychain_command_attempted` and makes the result fail.

The interactive credential file stays below the disposable Qoder configuration
root. Default cleanup removes the validated root after success or failure. The
result does not return any credential value.

These controls cover the credential branch and command form observed in the
exact bundle with SHA-256
`a5b9d9f116c16c512fd3000a62cb87e6d7e72ff4edbb196e24773834cef13eba`.
They are not a public Qoder contract or an operating-system Keychain audit. An
absolute command path, a new native implementation, or a changed bundle needs
a new review. The runner rejects a changed bundle before authentication.

## 1. Study question

The study tests this proposition:

> Mallard can materialize a captured Qoder CLI session into a clean Qoder
> configuration directory and use Qoder CLI to continue the exact conversation.

This proposition has five separate claims:

1. Qoder can find the copied session for the selected project.
2. Qoder can load the copied transcript and required sidecars.
3. Qoder selects the intended active conversation lineage.
4. Qoder continues with the prior semantic context.
5. Qoder persists the new turn without corrupting the source identity or state.

The current PoC proves claim 1. A differential product probe also proves that
source resolution can occur before the final login error. It does not prove
complete claim 2 or claims 3 through 5.

## 2. Evidence levels

This report keeps four evidence levels separate.

| Level | Meaning | Authority |
| --- | --- | --- |
| Documented | Qoder publishes the behavior | Public compatibility contract for the stated behavior |
| Product-proved | The version-pinned product executable completed the stated test | Evidence for the tested version and environment |
| Package-observed | Static inspection found the behavior | Diagnostic, version-specific evidence |
| Synthetic | A Mallard PoC or mock completed the behavior | Evidence for Mallard mechanics only |

Do not convert package-observed behavior into a production contract. Qoder can
change minified internal code without a compatibility notice.

## 3. Tested environment

The product and package study used:

- macOS ARM64.
- Qoder App `0.1.4` for installed-product identification only.
- Qoder CLI package `1.1.40`.
- Qoder Agent SDK package `1.0.32`.
- Node.js `26.7.0`.
- npm `11.19.0`.

The CLI package was in an isolated temporary package tree. Its lifecycle
scripts did not run. The PoC invoked `bundle/qodercli.js` directly. It did not
invoke the desktop-aware npm dispatcher.

The guarded raw-copy product run configured Qoder to use isolated home and
configuration directories. It copied no data from the normal Qoder home. That
run did not:

- copy Qoder login files;
- use a Qoder personal access token;
- complete a live model request or response;
- use the Tuzi credential;
- start Qoder App; or
- edit a Qoder application database.

The harness passed one fixed synthetic prompt to the CLI. No evidence shows
that the prompt reached a live model service.

The later user-operated native-resume run used interactive authentication in a
new isolated profile. It completed one source model turn and one resumed model
turn. The runner did not return the credential value, the session ID, the
marker, raw model output, or the temporary path. It removed the temporary
profile after success. This run used a fresh native session. It did not use the
copied six-file fixture or the Tuzi provider.

Node.js and npm were already available. npm installation succeeded through the
approved registry. Homebrew was not required.

## 4. Public Qoder CLI contract

### 4.1 Resume controls

The [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference),
[session guide](https://docs.qoder.com/cli/sessions), and
[task guide](https://docs.qoder.com/cli/run-tasks) define these controls:

| Control | Documented function |
| --- | --- |
| `qoder -c` or `qoder --continue` | Resume the most recent session for the current project. |
| `qoder -r <id>` or `qoder --resume <id>` | Resume a historical session by ID. |
| `qoder --resume` | Open the session selector when no ID is supplied. |
| `qoder -r <id> --fork-session` | Create a new session from the resumed history and preserve the source. |
| `qoder --list-sessions` | List historical sessions. |
| `qoder --delete-session <index>` | Delete the session at a human-list index. |
| `qoder --cwd <directory>` | Select the working directory. |
| `qoder --config-dir <directory>` | Select the user configuration directory. |
| `qoder -p` or `qoder --print` | Run without the interactive user interface. |
| `qoder -p --no-session-persistence` | Do not persist that print-mode session. |

The public reference states that `--continue` and `--resume` conflict with each
other and with remote-session modes. The public task guide also shows that
`-r` can be used in print mode.

### 4.2 `--session-id` ambiguity

The public pages are inconsistent about `--session-id`:

- The CLI reference describes it as a way to resume a specified ID.
- The session guide places it under caller-selected session creation.
- The Agent SDK contract uses `sessionId` alone for a new session ID. With a
  fork, it identifies the new fork.

The PoC uses `--resume <source-id>` as the source selector. It does not use
`--session-id` as a source selector. CLI `1.1.40` package logic treats
`--session-id` as the target during fork and rejects it for ordinary resume or
continue. That behavior still needs a version-pinned product contract test.

### 4.3 Project and configuration selectors

Qoder documents these selectors:

```text
--config-dir <absolute-directory>
QODER_CONFIG_DIR=<absolute-directory>
--cwd <absolute-project-directory>
QODER_WORKING_DIR=<absolute-project-directory>
```

The default configuration root is `~/.qoder`. The default working directory is
the process working directory.

Sessions are project-scoped. Qoder's external-store documentation requires
every host to use the same `cwd`. The official sample uses the same canonical
absolute workspace path on both hosts.

Qoder does not publish:

- a project-key override;
- the local processed-path algorithm as a stable contract;
- cross-path resume support;
- path remapping rules;
- case or Unicode normalization rules; or
- symlink identity rules.

The supported baseline is therefore an exact canonical path match.

### 4.4 Authentication

The [Qoder authentication guide](https://docs.qoder.com/cli/authentication)
states that authentication is required before use. It supports:

- interactive sign-in;
- browser sign-in;
- a pasted Qoder token; and
- `QODER_PERSONAL_ACCESS_TOKEN` for automation.

The environment PAT has precedence over saved interactive credentials.

Authentication is not session data. A portable session must not include Qoder
authentication. A target host must authenticate independently.

### 4.5 Documented local files

The session guide documents these two artifacts:

```text
<qoder-config>/projects/<processed-project-path>/<session-id>.jsonl
<qoder-config>/projects/<processed-project-path>/<session-id>/state.json
```

The JSONL file is the conversation log. `state.json` is session state.

The public guide does not define:

- the JSONL record schema;
- the state schema;
- a format version;
- a complete sidecar set;
- a stable snapshot operation;
- the active-leaf algorithm;
- UUID collision behavior;
- checkpoint storage;
- torn-copy failure behavior; or
- compatibility across CLI versions.

### 4.6 No public local-session import

Qoder documents `/export` for an interactive session. It does not document the
export format as resumable. It also does not document a CLI `/import` command
or startup import flag for local sessions.

Therefore, raw file placement is an empirical compatibility path. It is not a
published migration interface.

## 5. Supported SDK portability boundary

The [Qoder Agent SDK external-session storage guide](https://docs.qoder.com/cli/sdk/session-storage)
defines `SessionStore`:

```ts
type SessionKey = {
  projectKey: string;
  sessionId: string;
  subpath?: string;
};

type SessionStoreEntry = {
  type: string;
  uuid?: string;
  timestamp?: string;
  [key: string]: unknown;
};
```

The required methods are `append` and `load`. Optional methods add listing,
deletion, and subagent discovery.

Qoder requires a store to:

- keep entries opaque;
- preserve append order;
- return complete history;
- isolate keys;
- make appends idempotent;
- serialize concurrent writers for one session; and
- return only safe relative subkeys.

The SDK can resume by ID from that store. Qoder still writes a local copy.
External mirror writes are best effort. A conversation can complete after a
mirror failure. Consumers must treat `mirror_error` as a failed portability
operation.

The store contains history only. It does not contain:

- Qoder authentication;
- application settings;
- file checkpoints;
- task-state JSON;
- retention policy; or
- Qoder App database projections.

The SDK can also fall back to a local session when an explicit store resume
does not find the ID. A valid clean-host test must prove that the target has no
local source session before resume.

### 5.1 Official clean-host pattern

Qoder publishes an
[external-session-storage sample](https://github.com/QoderAI/qoder-agent-sdk-samples/tree/main/typescript/external-session-storage).
The sample:

1. Creates a session on Host A.
2. Mirrors it to Redis or PostgreSQL.
3. Deletes the Host A local transcript.
4. Starts Host B without that local transcript.
5. Uses the same absolute working directory.
6. Resumes by the original session ID.
7. Asks for a unique value from the earlier turn.
8. Verifies that the model returns the value.

This is the strongest current vendor-supported portability proof. It proves
conversation history continuity. It does not prove a full local task restore.

## 6. Qoder CLI 1.1.40 package observations

This section records static observations. These observations are not public
contracts.

### 6.1 Project-bucket function

The tested CLI computes a local project bucket as follows:

1. Canonicalize the project root.
2. Replace each character outside `[A-Za-z0-9]` with `-`.
3. Use the result directly when it has 200 characters or less.
4. For a longer result, keep 200 characters and add an absolute DJB2-XOR hash
   in base 36.

Example:

```text
/tmp/a b/project -> -tmp-a-b-project
```

The SDK uses the same observed project-key function. Canonical path resolution
uses `realpath(resolve(cwd))` when possible, with `resolve(cwd)` as a fallback.

Do not persist this function as a version-independent Mallard rule. Gate it by
Qoder CLI version if a future experimental adapter uses it.

### 6.2 Resume lookup

The CLI accepts a package-specific safe selector matching
`^[A-Za-z0-9][A-Za-z0-9._-]*$`, except `..`. A nonnumeric selector can name a
session; it is not required to be UUID-shaped. The search includes the current
project and discovered same-repository worktrees. If more than one bucket
contains the same identity, the tested CLI selects the newest match.

A positive decimal first matches an exact listed identity. If no exact match
exists, Qoder interprets it as a one-based index in the newest-first list. A
list index is unsuitable for a portable identity because list order can change.

This lookup behavior creates a collision hazard. A copied session UUID can
silently select another transcript if Mallard does not reject duplicates
before publication and apply.

### 6.3 SDK lookup inconsistency

The tested SDK has two different duplicate-ID behaviors:

- `listSessions()` deduplicates by UUID and keeps the newest modification time.
- `getSessionInfo()` and `getSessionMessages()` without `dir` return the first
  filesystem-enumerated match.

Calls with an explicit `dir` use the exact project bucket.

Mallard must never use an unscoped get-by-ID operation. The portable identity
must include source project identity and session UUID.

### 6.4 Session members found in the package

The package uses more than the two publicly documented files:

```text
projects/<project-key>/<session-id>.jsonl
projects/<project-key>/<session-id>/state.json
projects/<project-key>/<session-id>/compression-v2/state.json
projects/<project-key>/<session-id>/subagents/agent-<id>.jsonl
projects/<project-key>/<session-id>/subagents/agent-<id>.meta.json
projects/<project-key>/<session-id>/subagents/task-<id>.json
file-history/<session-id>/<checkpoint-object>
```

Main JSONL records can refer to file-history snapshots. The checkpoint payloads
are outside the project session directory. A six-file materialization is not a
complete session when file checkpoints exist.

The SDK `SessionStore` materializer copies the main transcript and safe
transcript subpaths. It does not materialize task JSON or file-history objects.
Therefore, the external store cannot replace a complete checkpoint-aware
backup.

### 6.5 Record classes

The tested CLI recognizes metadata records that include:

```text
summary
custom-title
ai-title
last-prompt
tag
workspace-directories
runtime-config
mode
content-replacement
file-history-snapshot
token-stats
active-leaf
relocated
worktree-state
```

This list is version-specific. Mallard must capture transcript entries as
opaque ordered bytes unless Qoder publishes a schema.

### 6.6 Active conversation lineage

The tested package keeps the last matching `active-leaf` record for each
session as the persisted candidate.
The record includes:

```text
sessionId
leafUuid
explicit
timestamp
optional rewound
optional compactBoundaryUuid
```

The observed resolver behaves as follows:

- A valid same-session persisted leaf selects its parent-linked lineage. It can
  select a sidechain, team, or metadata record.
- A missing or invalid persisted leaf can fall back to a computed default.
- A persisted missing, foreign-session, or agent-mismatched active-leaf
  candidate warns and falls back, even when `explicit` is true.
- A separate caller-supplied resolver `leafUuid` can reject an invalid leaf.
- Default discovery excludes sidechains but includes team and metadata
  candidates.
- Default discovery prefers a stable or complete assistant candidate, then
  parsed timestamp, with file order as a tie-breaker.
- Runtime view stops at the applicable compact boundary.
- Historical view crosses the boundary and hides compaction implementation
  records.

Persisted-leaf authority uses file order. Fallback selection uses the separate
candidate policy above. Do not treat these rules as equivalent.

These observations explain why Mallard must preserve record order and every
parent UUID. They do not establish a stable raw-file contract.

### 6.7 Resume mutation behavior

After successful initialization, a nonfork resume keeps the same session UUID
and reopens the source JSONL writer. A subsequent turn can append metadata and
conversation records.

The tested interactive path can write:

- workspace-directory metadata;
- runtime configuration metadata;
- active-leaf metadata; and
- new user and assistant records.

Subagent hydration can change an orphaned `running` task record to
`interrupted`. Thus, resume is not a read-only validation operation.

### 6.8 Fork behavior

The tested package resolves one historical active lineage, serializes it into
a new transcript, and creates a fresh minimal target `state.json`. It does not
clone source task, compression, or subagent sidecars.

Checkpoint copying is a later resume-initialization operation. When file
checkpointing is enabled, headless fork uses strict mode: it removes the target
checkpoint directory, hard-links referenced objects or falls back to ordinary
nonexclusive `copyFile`, and removes that directory on copy failure.
Interactive and ACP paths do not use strict copy. They can accept an existing
object without byte verification and continue after individual copy failures.
Hard-linked source and target objects share an inode, so independent later
mutation is unproved.

The checkpoint set is filtered to the resolved chain but not re-filtered after
a message-level cut. Live `/branch` uses the live checkpoint manager's full
snapshot set without another proved chain cut. A fork can therefore receive
file state from after its selected message.

Core fork creation has separate exclusive-create checks for the target
transcript and state. It has no atomic destination-wide claim. An explicit
target can receive an early transcript check, but a generated target relies on
the later exclusive create. The transcript can be created before a state
collision fails and triggers rollback. Late headless failures have no proved
complete fork rollback.

The complete fork study is in
[Qoder fork mechanisms research and PoC](QODER_FORK_MECHANISMS_RESEARCH_2026-09-02.md).

### 6.9 Message-level resume

The package contains hidden CLI controls named `--resume-session-at` and
`--resume-drops-turn`. The TypeScript SDK documents similar options.

The observed implementation keeps the chain through the specified entry UUID.
The dropped-turn value acts as a stale-view guard. It rejects the operation if
the discarded suffix starts at another prompt or contains unrelated user,
queued, or task work.

The normal public CLI reference does not list these flags. Mallard must not use
hidden CLI controls as a production contract.

### 6.10 Authentication order

The normal CLI performs configuration and authentication initialization before
it resolves `--resume`. It can retain the authentication error, continue source
resolution and local preparation, and return the login error later. There is
no supported credential-free semantic resume dry run.

The SDK query path also requires authentication before it can continue a
conversation. Some store materialization can occur before the child runtime
continues, but that does not make semantic resume credential-free.

### 6.11 Fake and recorded responses

The package contains hidden `--fake-responses` and `--record-responses`
controls.

- Record mode records completed live responses. It is not offline.
- Fake mode can intercept model transport in test-oriented paths.
- The ordinary CLI startup path does not provide a clear public auth bypass.
- ACP has a separate fake-auth exception for tests.

Qoder does not document fake responses as a public restore-test interface.
This PoC does not use them.

## 7. Executable PoC

### 7.1 Files

| File | Purpose |
| --- | --- |
| [`../poc/qoder-cli-resume/qoder-cli-resume.mjs`](../poc/qoder-cli-resume/qoder-cli-resume.mjs) | Dependency-free isolated CLI list and auth-gate harness |
| [`../poc/qoder-cli-resume/qoder-cli-resume.test.mjs`](../poc/qoder-cli-resume/qoder-cli-resume.test.mjs) | Mocked offline safety and behavior tests |
| [`../poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.mjs`](../poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.mjs) | Isolated read-only SDK active-leaf and duplicate-UUID probe |
| [`../poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.test.mjs`](../poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.test.mjs) | Version-pinned product test for the read-only SDK probe |
| [`../poc/qoder-cli-resume/README.md`](../poc/qoder-cli-resume/README.md) | Short run guide and limitations |

The harness is a research artifact. It is not part of the Mallard Tauri or Rust
runtime.

### 7.2 Synthetic session group

The harness materializes six files:

```text
<session-id>.jsonl
<session-id>/state.json
<session-id>/compression-v2/state.json
<session-id>/subagents/agent-explore-0123456789abcdef.jsonl
<session-id>/subagents/agent-explore-0123456789abcdef.meta.json
<session-id>/subagents/task-fedcba9876543210.json
```

The main transcript contains:

1. `workspace-directories` with `directories`.
2. One user record with a synthetic title.
3. One assistant record with a prior-context marker.
4. One `active-leaf` record with `explicit: true`.

The harness verifies one SHA-256 manifest at source, staging, and destination.
It verifies the complete six-member group again after list and resume.

This fixture does not contain file-history checkpoint payloads. It is not a
complete representation of every Qoder session.

The sidecar payloads are synthetic. Successful listing validates enough of the
main transcript to display its UUID and title. It does not validate the schema
or consumption of root state, compression state, subagent metadata, or task
state.

### 7.3 Isolation controls

The harness:

- creates one exact operating-system temporary root;
- requires an exact root-name prefix and marker file before use or deletion;
- places `HOME`, `QODER_CONFIG_DIR`, `cwd`, XDG paths, and temporary paths below
  that root;
- builds the child environment from an allowlist;
- does not accept or inherit credentials;
- verifies the CLI package name and version, and validates the CLI bundle and
  Node executable as canonical regular files;
- does not follow session-group symbolic links;
- limits each synthetic member to 4 MiB;
- limits the observed tree to 10,000 files and 128 MiB;
- limits each output stream to 1 MiB;
- returns output hashes, byte counts, and known signals, not raw output;
- scans output and files for credential-shaped values;
- records tree changes without following symbolic links;
- replaces the path-derived project bucket in the result; and
- deletes only the validated temporary root.

The child preload guard intercepts common Node.js DNS, TCP, TLS, HTTP, HTTPS,
HTTP/2, UDP, Fetch, and child-process functions. It also sets proxy URLs to
loopback port 1.

This is a JavaScript guard. It is not a complete operating-system egress
sandbox. Use a disposable VM with operating-system network controls for an
authenticated test.

### 7.4 Test command

```sh
node --test poc/qoder-cli-resume/qoder-cli-resume.test.mjs
```

Result:

```text
tests: 10
passed: 10
failed: 0
```

The tests cover:

- short and long project-bucket computation;
- the six-file synthetic fixture;
- isolated list discovery;
- authentication-gate classification;
- tree-change recording for relative path, type, mode, size, content SHA-256,
  and symbolic-link target SHA-256;
- session content-manifest preservation;
- raw-output suppression;
- credential-canary rejection;
- transcript-mutation rejection;
- network-attempt blocking;
- CLI symbolic-link rejection;
- relative CLI-path rejection; and
- Qoder CLI package-version rejection.

### 7.5 Guarded product command

The command shape is:

```sh
node poc/qoder-cli-resume/qoder-cli-resume.mjs \
  --cli-bundle /absolute/package/path/bundle/qodercli.js \
  --timeout-ms 30000
```

The harness performs these exact product operations:

```text
qodercli --config-dir <clean-config> --cwd <exact-cwd> --list-sessions

qodercli --config-dir <clean-config> --cwd <exact-cwd> \
  --bare --resume <synthetic-id> --print --tools "" \
  --permission-mode dont_ask --no-session-persistence \
  --disable-builtin-skills --strict-mcp-config \
  --mcp-config <empty-config> --max-turns 1 \
  --output-format json <synthetic-prompt>
```

The implementation also disables built-in Skills and uses a minimal startup
mode. These are test controls. They are not session portability requirements.

### 7.6 Guarded product result

| Check | Result |
| --- | --- |
| Qoder CLI version | `1.1.40` |
| Synthetic restore | 6 of 6 files verified |
| List exit code | `0` |
| Session UUID in list output | Yes |
| Synthetic title in list output | Yes |
| Restored group after list | Member set, sizes, and SHA-256 content match |
| Resume exit code | `1` |
| Authentication requirement | Observed |
| Prior assistant marker in resume output | No |
| Resume prompt in resume output | No |
| Restored group after resume | Member set, sizes, and SHA-256 content match |
| Credential findings | `0` |
| Calls permitted through intercepted JavaScript interfaces | `0` |
| Guarded network attempts | `57` to `59` across three product runs |
| Guarded subprocess attempts | `3` in each product run |
| Temporary cleanup | Complete |

Three product runs produced totals from 60 to 62 denied operations. The exact
attempt counts are diagnostic. They can change across runs.

The list operation created 49 non-session entries in the disposable tree.
These entries included identity, empty auth transaction state, settings,
security resources, logs, telemetry state, and a helper entry. The resume
operation created 7 more non-session entries. For the tracked properties, the
measured snapshot reported one modification: the disposable `logs/latest`
symbolic link.

The harness returns `ok: false` because the guard intercepted product
operations. This is the intended fail-closed result. The useful sub-results are
the successful list, the authentication gate, the matching session content
manifest, the zero credential finding, and complete cleanup.

### 7.7 Correct interpretation

The product run proves:

- the tested CLI accepts the clean configuration and working-directory
  selectors;
- the tested CLI lists the copied synthetic transcript;
- an unauthenticated resume reaches a Qoder authentication requirement;
- list and auth-gated resume preserve the six-member set, sizes, and SHA-256
  content; and
- the test configured isolated Qoder profile directories and copied no real
  profile data.

It does not prove:

- that the six-file run parsed every transcript record;
- that resume loaded `state.json`;
- that resume loaded compression or subagent state;
- that resume selected the expected leaf;
- that a model received the earlier context;
- that a new turn can append safely; or
- that Qoder App can use the restored session.

The package trace changes the earlier authentication interpretation. Normal
headless startup attempts authentication first, but it can defer the error and
continue local resume or fork preparation.

A separate differential product probe used two clean synthetic sources. A
mode-`000` source returned the invalid-session path with exit `42`. A readable
source reached the final structured error with exit `1`, `is_error: true`, the
source session ID, zero usage, and `Not logged in · Please run /login`. The
readable source main transcript kept its size and SHA-256 and gained no sibling
session artifacts during the nonfork resume. Other disposable configuration
files were created. Thus, the product can access the source before it returns
the login error, but no model used the restored semantic context.

### 7.8 Tuzi provider recheck

A bounded direct Tuzi API recheck ran on 2 September 2026. It used the existing
mode-`0600` local credential input and did not display its values.

| Check | Result |
| --- | --- |
| `GET /v1/models` | HTTP `200` |
| Model identifiers | `836` |
| `deepseek-chat` present | Yes |
| `POST /v1/chat/completions` | HTTP `200` |
| Choices | `1` |
| Non-empty content choices | `1` |
| Credential leak findings | `0` |
| Disposable-root cleanup | Complete |

This direct request did not start Qoder or resume a session. The Tuzi key
authenticates the model endpoint only. Qoder requires separate authentication
for semantic continuation and provider routing. Normal headless source
resolution can still occur while that error is deferred. The later
authenticated native-resume run used Qoder's normal service path, not Tuzi.
Thus, the Tuzi recheck does not upgrade copied-session context consumption,
active-leaf authority, sidecar consumption, or Qoder-to-Tuzi routing.

## 8. Active-leaf study

### 8.1 Why this is a separate gate

A Qoder JSONL file can contain a graph, not only a linear conversation. Parent
UUIDs, sidechain markers, compaction boundaries, and active-leaf records can
change the visible lineage.

A byte-complete restore can still open the wrong branch when:

- record order changes;
- one active-leaf record is missing;
- a parent UUID changes;
- the selected leaf belongs to another session;
- a compact boundary is incomplete; or
- an invalid leaf causes fallback selection.

Mallard must not calculate or replace an active leaf from its own assumptions.

### 8.2 Current evidence

The SDK publicly describes `getSessionMessages()` as following the persisted
active leaf and falling back to the latest main leaf for legacy transcripts.
The package trace confirms the same broad algorithm for the tested version.

The dedicated runtime probe used the public read-only SDK session APIs. It
created four branched transcripts in a clean synthetic configuration root. It
did not call `query()`, authentication, a mutation API, or a network-facing SDK
API.

Result for Agent SDK `1.0.32`:

| Case | Returned lineage | Result |
| --- | --- | --- |
| Two matching active-leaf records | Root, then the branch named by the last record in file order | Pass |
| Last record has an older timestamp | Last file-order record still wins | Pass |
| No active-leaf record | Root, then the latest main leaf | Pass |
| Active leaf names a missing UUID | Fallback to the latest main leaf | Pass |
| Active leaf names a node with another session ID | Discard the foreign candidate and fall back to the latest main leaf | Pass |

This proves that the tested SDK reader uses file order, not active-leaf
timestamp order. The last matching record wins even when its timestamp is
older.

The probe tracked seven synthetic regular files. Paths, sizes, SHA-256 values,
and modification times were unchanged after all read operations. The test
passed with one test and zero failures.

### 8.3 Evidence limit

This result proves the SDK `1.0.32` reader projection. The CLI `1.1.40` package
trace also shows warning and fallback for a missing, foreign-session, or agent-
mismatched persisted active leaf, including a persisted record with
`explicit: true`. A separate resolver-level caller `leafUuid` is a hard
constraint. Do not confuse it with persisted active-leaf metadata.

The probe does not test compaction, persisted sidechain, team, or metadata
leaves, the fallback candidate preference, or file-history rewind.

### 8.4 Required production rule

Even when the SDK projection test passes, Mallard must preserve:

- all opaque entries in original order;
- every UUID and parent UUID;
- every session ID;
- active-leaf records;
- compact boundaries and compression state; and
- all referenced task and checkpoint state.

Do not path-map free-form history. Do not rewrite graph identifiers.

## 9. UUID collision study

### 9.1 Collision classes

Mallard must distinguish these collisions:

| Collision | Risk |
| --- | --- |
| Same UUID in the target project bucket | Restore can overwrite or append to an unrelated local session. |
| Same UUID in another bucket | CLI search can select the newest duplicate. |
| Same UUID in a related worktree | Resume can select a related but incorrect transcript. |
| Same target fork UUID | Fork creation can fail or target existing checkpoint state. |
| Same SDK store key | Import can append to existing opaque history. |

### 9.2 Read-only collision result

The SDK probe placed the same synthetic session UUID in two project buckets.
Project B had the newer transcript modification time.

With an explicit `dir`, all three read-only APIs selected the correct project:

- `listSessions()`;
- `getSessionInfo()`; and
- `getSessionMessages()`.

Without `dir`, the APIs did not form one consistent projection:

- `listSessions()` reduced the duplicate UUID to one item and selected newer
  project B.
- `getSessionInfo()` selected project A in the observed run.
- `getSessionMessages()` selected project A in the observed run.

The two single-session readers use project-directory enumeration order. That
order is not a stable selection contract. The test does not assert which
project must appear first.

This runtime result confirms that an unscoped UUID is unsafe. Always provide
the exact project scope for SDK reads. Still reject duplicate UUIDs before a
Mallard restore because CLI resume can search more than one bucket.

### 9.3 Required Mallard policy

For an exact restore, reject the operation when the source UUID exists in any
reachable destination bucket, task directory, checkpoint directory, mapping,
application projection, or SDK store key.

For a fork, claim the new target UUID atomically across the transcript, session
directory, checkpoint directory, plan namespace, Mallard mapping, application
projection, and external store. Serialize this claim with all Mallard and Qoder
target writers. A destination-wide preflight without a claim is still racy.

For an update, require all of these values to match the reviewed state:

- Mallard cloud task ID;
- provider ID;
- source session UUID;
- destination session UUID;
- canonical project identity;
- complete member manifest; and
- expected destination digests.

Do not use Qoder's “newest duplicate wins” behavior as conflict resolution.

## 10. `--continue`, `--resume`, and fork risk

| Operation | Source selection | Expected writes after success | Mallard use |
| --- | --- | --- | --- |
| `--continue` | Newest session for project | Appends to selected source | Do not use for deterministic restore. |
| `--resume <id>` | Exact safe selector, or numeric list index after exact-match lookup; subject to bucket search | Appends to source session | Use only after global collision rejection. |
| `--resume <id> --fork-session` | Same source-selector rules | Creates a new transcript, fresh minimal state, and can later copy checkpoints | Test target-ID, sidecar, checkpoint, collision, and rollback behavior first. |
| `--session-id <id>` | Public meaning is inconsistent; CLI `1.1.40` uses it as a fork target | Package path can create a caller-selected fork target | Do not use as source selector. Version-gate target use. |
| hidden resume-at controls | Entry UUID within history | Can slice context and affect continuation | Do not use in production. |

`--no-session-persistence` reduces the write surface of this auth-gate probe.
It cannot prove persistent continuation. The authenticated production gate must
enable persistence and then verify every append and sidecar change.

## 11. Complete-session membership

The prior six-member PoC is useful, but it is not a final allowlist.

Production capture must account for:

1. The main JSONL transcript.
2. Root session state.
3. Compression state.
4. Subagent transcripts.
5. Subagent metadata.
6. Task-state JSON.
7. File-history snapshot records.
8. Every referenced checkpoint object.
9. Future provider-defined sidecars.

Qoder does not publish an atomic snapshot API. A production capture needs a
closed membership scan and two stable reads while Qoder is stopped, or a
vendor-supported snapshot transaction.

File-level atomic replacement is not group atomicity. Qoder must not read the
destination during a multi-file apply.

## 12. Path mapping

The documented SDK baseline requires the same canonical `cwd`. This result
supports exact-path restore only.

Do not enable cross-path restore because a path can occur in:

- workspace-directory metadata;
- runtime configuration;
- relocation records;
- worktree state;
- state sidecars;
- task state;
- file-history metadata;
- tool results; and
- free-form conversation text.

String replacement is unsafe. If Qoder later publishes a structural schema,
Mallard can map only typed path-value tokens. It must preserve all free-form
history bytes. It must reject mapping when signatures, digests, replay tokens,
server correlations, or unknown path-bearing fields are present.

The CLI fork trace shows limited path reinterpretation, not general mapping.
Relative file-history keys can resolve under the target checkpoint manager's
working directory. Absolute keys outside it can continue to name source paths.
Historical message `cwd` values remain in copied records, while the source
`workspace-directories` transcript record is omitted and resolved directories
can enter fresh target state. These rules are not a portable cross-path
contract.

The first authenticated PoC must use the same exact canonical path. Cross-path
behavior is a later and separate test.

## 13. Qoder App boundary

Qoder CLI and Qoder App are separate product surfaces. The CLI configuration
selector does not prove a supported Qoder App profile selector.

Qoder App keeps separate application-database projections. Public Qoder App
documentation does not define:

- a session import API;
- an open-by-session-ID command;
- a database rebuild operation;
- a sidebar-index refresh operation; or
- a clean application-home selector.

A successful CLI resume would not prove that Qoder App discovers, displays, or
continues the session. Keep the application-projection gate separate.

## 14. Gate matrix

| Gate | Result | Evidence | Release consequence |
| --- | --- | --- | --- |
| Public CLI resume function | Pass | Qoder documentation | Resume is a supported user function. |
| Separate CLI configuration root | Pass | Documentation and product run | Safe isolated CLI testing is possible. |
| Exact-path project discovery | Pass, version-pinned | CLI `1.1.40` list result | Diagnostic only. |
| Six-file materialization | Pass, synthetic | SHA-256 manifest at source, stage, destination | File-history objects are still absent. |
| CLI list after materialization | Pass, product | ID and title found | Does not prove resume. |
| Authentication boundary | Pass, product | Resume exits with auth requirement | A disposable account is required. |
| Authenticated native semantic resume | Pass, limited | User-operated source and second-process resume recovered a source-only marker under the same UUID | Fresh native session only; copied restore is not proved. |
| Command-level native Keychain avoidance | Unproved after correction | First user run displayed a Keychain availability dialog because the runner used the ignored value `1` | Rerun with exact `true` value and blocking `security` guard. |
| Session-content manifest through login-failure run | Pass, product | Member set, sizes, and SHA-256 content match for all six files | Does not cover every file-system attribute, sidecar use, or later authenticated writes. |
| Direct Tuzi API compatibility | Pass, live, limited | Model-list and one chat-completions request passed | Provider prerequisite only; no Qoder or session evidence. |
| Authenticated semantic resume through Tuzi | Blocked | The authenticated Qoder run did not use Tuzi; no Qoder-to-Tuzi request ran | Production blocker. |
| Persistent continued turn, fresh native session | Pass, limited | Same transcript changed and grew after second-process resume | Does not cover copied restore or sidecars. |
| Persistent continued turn from copied restore | Blocked | No authenticated copied-session resume | Production blocker. |
| Active-leaf projection | Pass, SDK reader only | SDK `1.0.32` read-only branch probe | Do not extrapolate the result to authenticated CLI resume. |
| Compression behavior | Unproved | Package trace only | Production blocker. |
| Subagent resume behavior | Unproved | Package trace only | Production blocker. |
| File-history restore | Does not pass | Checkpoint objects not in six-file group or SDK store | Production blocker. |
| Global UUID collision protection | Does not pass | Product can select duplicates | Mallard must reject collisions. |
| Exact-path restore | Partial | Discovery passes; continuation is blocked | Do not ship. |
| Cross-path restore | Unproved | No vendor contract | Keep disabled. |
| Qoder App projection rebuild | Unproved | No public interface | Keep disabled. |
| Production raw-session restore | No-go | Required gates are incomplete | Capability remains false. |

## 15. Authenticated raw-restore clean-home PoC

The preliminary native-session marker test passed. The raw-copy test in this
section is still required. Run it only with a disposable Qoder account and a
disposable operating-system account or VM. The corrected native runner must
also pass once without a Keychain dialog before the no-Keychain check is
closed.

### 15.1 Preconditions

- Pin the Qoder CLI version.
- Use two fresh Qoder configuration directories.
- Use one fresh exact absolute project directory.
- Use a disposable Qoder PAT.
- Put the PAT in the child environment only.
- Do not copy saved-login files.
- Disable all tools, Skills, plugins, hooks, agents, and MCP servers.
- Allow only documented Qoder service destinations.
- Record operating-system network egress.
- Record every file-system read and write below both disposable homes.
- Use a one-turn limit.
- Confirm that no source or target path is a symbolic link.

### 15.2 Source phase

1. Start Host A with clean configuration A and the exact project path.
2. Submit one no-tool prompt that asks Qoder to remember a random marker.
3. Capture the returned session UUID from structured result output.
4. Submit a second prompt that confirms the marker.
5. Close Qoder cleanly.
6. Confirm that no Qoder process has the transcript open.
7. Capture the transcript, complete sidecar directory, and referenced
   file-history objects as one stable group.
8. Record hashes, sizes, identities, modes, and timestamps.
9. Scan every UUID and path-bearing structural field.

### 15.3 Destination phase

1. Start with clean configuration B.
2. Confirm that B has no local session with the source UUID.
3. Authenticate B independently with the disposable PAT.
4. Materialize the complete captured group at the same exact project path.
5. Verify the complete destination manifest.
6. Run `--list-sessions` and verify the UUID and source title.
7. Run `--resume <source-id>` with a prompt that asks for the random marker.
8. Verify that the response returns the exact marker.
9. Verify that the result session UUID equals the source UUID.
10. Verify that new user, assistant, and active-leaf records append to the
    expected transcript.
11. Verify every sidecar and checkpoint change.
12. Close and resume the session a second time.
13. Verify the marker again.

### 15.4 Fork phase

1. Hash the complete source group.
2. Fork the restored source session.
3. Obtain the fork UUID from structured output.
4. Verify that the fork has the prior marker.
5. For headless persisted fork, verify that the source group is byte-identical
   to the pre-fork group.
6. For interactive fork, record source transcript flushes and task-status
   changes separately.
7. At the immediate core-fork boundary, verify that target root state is fresh
   and minimal. After runtime initialization, compare it with the expected
   post-initialization state instead of requiring revision `1`.
8. Verify that task, compression, and subagent sidecars are absent or rebuilt
   through a documented mechanism.
9. Verify every target checkpoint and plan artifact after each later phase.
10. Repeat with every target collision class. Require Mallard's atomic claim to
    reject it before Qoder writes; do not rely on native rollback as preflight.
11. Inject a late checkpoint failure and require complete cleanup or a
    recoverable journal.

### 15.5 Negative tests

The test set must include:

- wrong working directory;
- missing main JSONL;
- missing root state;
- missing compression state;
- missing subagent transcript;
- missing task state;
- missing referenced checkpoint object;
- truncated final JSONL record;
- invalid parent UUID;
- missing active-leaf record;
- invalid active-leaf UUID;
- active leaf from another session;
- two active-leaf records with different order;
- duplicate source UUID in another project bucket;
- duplicate source UUID in a related worktree;
- existing target checkpoint directory;
- source transcript changed during capture;
- source sidecar added during capture;
- destination changed after review; and
- CLI version mismatch.

### 15.6 Acceptance criteria

The raw-copy PoC passes only when all conditions are true:

- Qoder loads the target copy, not a local fallback.
- The model returns the unique source marker.
- The selected branch is the intended branch.
- The session UUID is stable for nonfork resume.
- The new turn persists and survives process restart.
- Controlled headless fork keeps the complete persisted source group
  byte-identical.
- Interactive fork accounts for allowed source writer flushes and task-status
  changes without corrupting source history.
- All referenced sidecars and checkpoint objects are present.
- Mallard's atomic destination claim rejects collisions before Qoder writes;
  native Qoder rollback is not accepted as the claim.
- No credential appears in output, logs, transcript, bundle, receipt, or
  application database.
- The test passes for every supported Qoder CLI version and operating system.

One successful test is empirical evidence for one version. It does not create
a vendor compatibility guarantee.

## 16. Supported SDK PoC option

Mallard can run a separate PoC with the public external `SessionStore`.

Use the official two-host pattern:

```text
Host A query
  -> external opaque store
  -> delete or isolate Host A local transcript
  -> clean Host B
  -> same canonical cwd
  -> resume by UUID
  -> verify unique prior marker
```

This option is appropriate only if Mallard starts and owns Qoder SDK sessions.
It does not make existing Qoder App sessions portable.

Before production use, an adapter must also pass:

- the Qoder store conformance suite;
- concurrent-writer serialization tests;
- retry and idempotency tests;
- mirror-error fail-closed policy tests;
- retention and encryption review; and
- local-fallback exclusion tests.

Do not claim file restore, task restore, checkpoint restore, or Qoder App
restore from an external-store success.

## 17. Mallard architecture consequences

### 17.1 Keep current production schemas unchanged

Do not add Qoder sessions to portable schema 1 from this PoC. Do not add an
enabled Qoder session provider adapter.

No production Rust, Tauri, TypeScript, manifest, persistence, or storage
contract changes are justified yet.

### 17.2 If the gates pass later

Use a provider-specific adapter with these responsibilities:

- detect an exact supported CLI version;
- resolve the canonical project identity;
- discover the complete session and checkpoint member set;
- capture a stable closed group;
- keep transcript bytes and graph identifiers opaque;
- plan an exact-path restore;
- reject destination-wide UUID collisions;
- stage and verify all files before apply;
- stop Qoder during apply;
- use Mallard revision, digest, expiry, trust, and approval pins;
- journal every step;
- verify the complete group after apply; and
- record a sanitized receipt with no local path or credential.

The adapter must remain separate from generic bundle transport. Local folder
and S3/R2 transport must not interpret Qoder transcript fields.

### 17.3 Do not use CLI human output as a contract

`--list-sessions` is human-formatted in the tested path. Mallard must use a
vendor-supported structured interface or the SDK session APIs. Do not parse
column positions, colors, truncation, or list indices.

### 17.4 Do not treat resume as validation

Authenticated resume can append to the source transcript and change sidecars.
Use read-only SDK inspection for diagnostics. Use a disposable copy for resume
tests. Never validate a reviewed production source by opening it for resume.

## 18. Reproduction

### 18.1 Repository tests

```sh
node --check poc/qoder-cli-resume/qoder-cli-resume.mjs
node --check poc/qoder-cli-resume/qoder-cli-resume.test.mjs
node --check poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs
node --check poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs
node --test \
  poc/qoder-cli-resume/qoder-cli-resume.test.mjs \
  poc/qoder-cli-resume/qoder-cli-authenticated-resume.test.mjs
```

Expected result:

```text
19 passed
0 failed
```

Nine of these tests exercise the authenticated runner with a mock CLI. They
include an attempted native Keychain command and require the runner to block
the command, fail closed, and remove the isolated root.

The complete Qoder PoC suite also passed after this correction:

```text
50 passed
0 failed
0 skipped
```

### 18.2 Read-only SDK branch probe

Use a clean environment and an absolute canonical SDK `1.0.32` entry path:

```sh
env -i \
  PATH="<minimal-path>" \
  QODER_SDK_PATH="/absolute/package/path/dist/index.js" \
  node --test \
  poc/qoder-cli-resume/qoder-sdk-active-leaf-probe.test.mjs
```

Expected result:

```text
1 passed
0 failed
0 skipped
```

The probe rejects another SDK version. It accepts no credential. It does not
call a network-facing SDK API or a session mutation API.

### 18.3 Isolated package acquisition

If the exact package is not already available, use a temporary package root.
Disable lifecycle scripts:

```sh
export NPM_CONFIG_REGISTRY="<approved-registry>"
npm install \
  --ignore-scripts \
  --no-save \
  --package-lock=false \
  @qoder-ai/qodercli@1.1.40
```

Do not run the package postinstall script for this PoC. Do not create a global
installation. Do not use the desktop-aware dispatcher.

### 18.4 Guarded product run

```sh
node poc/qoder-cli-resume/qoder-cli-resume.mjs \
  --cli-bundle /absolute/package/path/bundle/qodercli.js \
  --timeout-ms 30000
```

An `offline_guard_intercepted_operation` result is expected for the tested CLI.
Inspect the structured evidence fields. Do not remove the guard to obtain a
green result.

## 19. Official sources

- [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference)
- [Qoder CLI session management](https://docs.qoder.com/cli/sessions)
- [Qoder CLI task and print-mode guide](https://docs.qoder.com/cli/run-tasks)
- [Qoder CLI authentication](https://docs.qoder.com/cli/authentication)
- [Qoder CLI configuration reference](https://docs.qoder.com/cli/settings-reference)
- [Qoder CLI commands](https://docs.qoder.com/cli/commands)
- [Qoder CLI slash commands](https://docs.qoder.com/cli/slash-reference)
- [Qoder Agent SDK session control](https://docs.qoder.com/cli/sdk/session-control)
- [Qoder Agent SDK external session storage](https://docs.qoder.com/cli/sdk/session-storage)
- [Qoder Agent SDK TypeScript reference](https://docs.qoder.com/cli/sdk/references-typescript)
- [Official Qoder Agent SDK samples](https://github.com/QoderAI/qoder-agent-sdk-samples)
- [Official TypeScript external-session-storage sample](https://github.com/QoderAI/qoder-agent-sdk-samples/tree/main/typescript/external-session-storage)

## Final decision

Qoder CLI resume is technically available. Exact-path local discovery also
works for the tested synthetic session. The clean-home product evidence proves
that source access can occur before the final login error, but it does not prove
model-context continuation, complete sidecar use, or safe persistence. The
local-file format, complete member set, active-lineage contract, collision
rules, and cross-path behavior remain unsuitable for a production compatibility
promise.

The Qoder Agent SDK external store is the supported history-portability path.
It is still narrower than Mallard's full session-and-project-state goal.

Keep production Qoder session restore disabled. Complete the authenticated
raw-copy clean-home marker test in a disposable environment before any Mallard
provider design or schema change.

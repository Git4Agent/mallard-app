# Qoder Fork Mechanisms Research and Proof of Concept

**Date:** 2 September 2026

**Repository:** Mallard

**Result:** `TRANSCRIPT_ONLY_POC`

**Production decision:** No-go for production Qoder task fork or exact session
restore

**Scope:** Qoder CLI startup fork, interactive `/branch`, Agent SDK runtime
fork, Agent SDK standalone history fork, external `SessionStore`, Qoder Quest
and Editor fork, identity, lineage, sidecars, checkpoints, collisions, rollback,
and Mallard product impact

**Out of scope:** Qoder pets, production code, real Qoder credentials, the real
Qoder home, Qoder application-database changes, and unsupported UI automation

**Related research:**
[Qoder support feasibility research](QODER_SUPPORT_FEASIBILITY_RESEARCH_2026-09-01.md),
[Qoder session restore PoC](QODER_SESSION_RESTORE_POC_2026-09-02.md),
[Qoder CLI resume PoC](QODER_CLI_RESUME_POC_2026-09-02.md), and
[Qoder custom-provider and Tuzi PoC](QODER_CUSTOM_PROVIDER_POC_2026-09-02.md)

## Executive result

Qoder supports conversation fork. It does not have one common fork mechanism.
The CLI, Agent SDK, and Qoder app expose different entry points and different
state guarantees.

The main result is:

```text
Qoder Agent SDK 1.0.32 standalone forkSession()
  -> synthetic source transcript
  -> isolated temporary Qoder configuration
  -> active historical chain selected
  -> new target UUID generated
  -> copied message UUIDs and parent graph remapped
  -> source transcript and source sidecars unchanged
  -> one target main transcript created
  -> no target task, compression, subagent, or checkpoint sidecar created
```

The PoC passed one product test with zero failures. It proves a
**transcript-only history fork**. It does not prove a complete Qoder task fork.

Qoder CLI `1.1.40` has a broader internal fork path. It creates a target
transcript and a fresh minimal `state.json`. It can later rehydrate referenced
file-history objects. It still does not clone compression, task, or subagent
sidecars. Its checkpoint and cleanup behavior differs between headless,
interactive, and ACP modes. These findings are package observations, not a
public compatibility contract.

The Qoder app can fork Quest and completed Editor conversations. This is a
user-interface feature. Qoder does not document an app task-import API,
application-index rebuild API, or headless app fork API.

### Decision summary

| Question | Result | Evidence |
| --- | --- | --- |
| Does Qoder support conversation fork? | Yes | Public Qoder contract |
| Does CLI support fork from resume? | Yes | Public Qoder contract |
| Does interactive CLI have a branch action? | Yes | Public `/branch` contract |
| Can SDK runtime fork and choose a target UUID? | Yes | Public SDK contract |
| Can SDK history fork run without Qoder authentication? | Yes | Product-proved on SDK `1.0.32` |
| Does standalone SDK fork preserve the source main transcript? | Yes, in the synthetic test | SHA-256 and entry checks |
| Does standalone SDK fork clone a complete task? | No | Product-proved and package-observed |
| Does standalone SDK fork copy subagent transcripts? | No | Product-proved through a rejecting `listSubkeys` probe |
| Does standalone SDK fork copy file checkpoints? | No | Product-proved |
| Does CLI core fork copy root state? | No | It creates fresh minimal state; package-observed |
| Can CLI later copy referenced checkpoints? | Yes, conditionally | Package-observed |
| Is CLI fork one atomic transaction? | No complete transaction was found | Package-observed |
| Is external-store target collision protection guaranteed? | No | Product-proved call sequence and public store contract |
| Does fork rebuild Qoder app projections? | Unproved | No public contract or disposable app test |
| Did Qoder fork and then continue through Tuzi? | No | Native resume passed later, but no authenticated fork or Qoder-to-Tuzi continuation ran |
| Is production Mallard Qoder fork ready? | No | Critical identity, state, continuation, and app gates remain open |

Keep exact Qoder session restore disabled. Do not use fork to relabel a failed
exact restore as successful. A future **Fork Qoder history** function must be a
separate operation with a new destination identity and narrower guarantees.

## 1. Evidence model

This report uses four evidence classes.

| Class | Meaning | Authority |
| --- | --- | --- |
| Documented | Qoder publishes the behavior | Public contract for the stated surface |
| Product-proved | The version-pinned package completed the isolated test | Evidence for the tested version and fixture |
| Package-observed | Static inspection found the behavior | Diagnostic evidence only |
| Unproved | No applicable contract or completed product test exists | Production blocker when required |

Qoder CLI, Agent SDK, Quest, Editor, and the desktop application database are
different surfaces. A contract for one surface does not define another
surface.

The package trace uses names and control flow from minified published bundles.
Those names can change without notice. Mallard must use version gates and
product tests before it depends on package-observed behavior.

### 1.1 Tested baseline

The study used:

- macOS ARM64;
- Qoder App `0.1.4` for installed-product identification only;
- `@qoder-ai/qodercli` `1.1.40` for the static CLI trace;
- `@qoder-ai/qoder-agent-sdk` `1.0.32` for the static trace and product PoC;
- Node.js `26.7.0`; and
- npm `11.19.0`.

The inspected published bundle fingerprints were:

| Package member | SHA-256 |
| --- | --- |
| `@qoder-ai/qodercli/bundle/qodercli.js` | `a5b9d9f116c16c512fd3000a62cb87e6d7e72ff4edbb196e24773834cef13eba` |
| `@qoder-ai/qoder-agent-sdk/dist/index.js` | `acf8c7f1d2abe7c6b7799b3e1dce43165a1d83b7e9e1de10d13c216afbfcccd3` |

The packages were held in disposable package roots. Installation did not run
lifecycle scripts. Node.js and npm were already available, so Homebrew was not
required. The report does not record the private package-registry URL.

## 2. Fork identity and terminology

Qoder calls persisted CLI and SDK history a **session**. Qoder app pages often
call the user-visible object a **task**. This report uses these terms:

- **Source session:** The existing history used to create a fork.
- **Target session:** The new fork identity.
- **Fork:** A new session with history derived from the source.
- **Resume:** Continued writes under the existing session identity.
- **Cut point:** The last source history entry retained in a fork.
- **Active lineage:** The parent-linked path selected from a branching
  transcript.
- **Sidecar:** State outside the main `<session-id>.jsonl` transcript.
- **Projection:** Qoder app state derived into its separate application
  database.

A Qoder session UUID is not a complete local identity. Local and external
history is project-bound. Mallard needs this minimum identity:

```text
provider + canonical project identity + session UUID
```

Fork intentionally creates a different target UUID. It cannot satisfy a
requirement to preserve the source UUID at the destination.

## 3. Qoder fork surface map

| Surface | Entry point | Source | Target | Cut point | Authentication | Machine result | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| CLI startup | `qoder -r <source> --fork-session` | Resume selector | Generated, or package-specific `--session-id` | Full active history; a hidden CLI flag can select an entry | Attempted before source resolution; success is required for model use, but not necessarily before local fork files | Public output schema does not guarantee fork ID | Public command; package-observed details |
| CLI interactive | `/branch` | Current session | Generated, or a picker-supplied package-specific target | Current resolved history | Interactive Qoder session | No documented machine interface | Public action; package-observed target detail |
| SDK runtime | `query({ resume, forkSession: true })` | `resume` | New ID; `sessionId` can select it | `resumeSessionAt` exists in TypeScript | Required for query execution | `system/init.session_id` and result `session_id` | Public SDK contract |
| SDK standalone | `forkSession(sourceId, options)` | Local main transcript or store main key | Always generated by the helper | `upToMessageId`, inclusive in SDK `1.0.32` | Not required for history transform | Typed `{ sessionId }` | Public API plus product proof |
| Python external store | `fork_session_via_store(...)` | Store main key | Generated in the inspected implementation | `up_to_message_id` | Not an auth argument | `ForkSessionResult` shape is implementation evidence | Public API name; target and result details need a version gate |
| Qoder Quest or Editor | Fork action | Existing app task | New app task | Any Quest turn; completed Editor session | App account | UI result only | Public UI contract |

These are not aliases. In particular, SDK standalone fork and CLI runtime fork
produce different transcript shapes in the inspected versions.

## 4. Public Qoder contracts

### 4.1 CLI startup fork

Qoder documents this command:

```text
qoder -r <source-session-id> --fork-session
```

Qoder states that the command creates a new branch from restored history and
keeps the original session intact. See
[CLI session management](https://docs.qoder.com/cli/sessions#forking-sessions).

The [CLI reference](https://docs.qoder.com/cli/cli-reference#session-management)
defines:

- `--resume`, or `-r`, to restore a historical session;
- `--continue`, or `-c`, to continue the most recent project session;
- `--fork-session` to fork from a resumed session;
- `--session-id` to use a specified session ID;
- `--cwd` to select the working directory; and
- `--config-dir` to select the Qoder user-data directory.

The public CLI pages do not define these fork details:

- destination-ID allocation;
- a stable machine-readable fork-ID field;
- active-lineage resolution;
- cut-point semantics;
- message UUID handling;
- compaction handling;
- complete sidecar membership;
- checkpoint copy behavior;
- collision scope;
- transaction boundaries; or
- rollback after a later startup failure.

The CLI pages are also ambiguous about whether `--session-id` selects a source
or destination when combined with fork. CLI `1.1.40` treats it as the target in
that combination. That is package-specific evidence. Mallard must not infer
this behavior for another CLI version.

### 4.2 Interactive `/branch`

The [slash-command reference](https://docs.qoder.com/cli/slash-reference#sessions-and-conversations)
defines `/branch` as creating a new session branch from the current session.
The public page does not define arguments, a target UUID, a cut point, output
JSON, file-state behavior, or rollback.

`/branch` is a TUI action. Qoder does not document it as a supported headless
automation protocol. The CLI release notes also show recent corrections for
branch permission errors and post-compaction historical branching. See the
[Qoder CLI release notes](https://docs.qoder.com/release-notes/qoder-cli).

Mallard must not drive `/branch` by terminal scraping.

### 4.3 Agent SDK runtime fork

The [SDK session-control guide](https://docs.qoder.com/cli/sdk/session-control#forking-sessions)
defines this runtime fork:

```ts
query({
  prompt: "Explore a different direction",
  options: {
    auth: accessTokenFromEnv(),
    resume: "source-session-id",
    forkSession: true,
  },
});
```

The documented contract is:

| Option | Meaning during fork |
| --- | --- |
| `resume` | Source session ID |
| `forkSession` | Fork instead of appending to the source |
| `sessionId` | Optional selected target session ID |

Qoder says that the new session gets a new ID and that the original session is
unaffected. The SDK stream identifies the active target through
`system/init.session_id`. Result messages also contain `session_id`. Mallard
must verify both values when it later runs an authenticated fork.

The TypeScript options also include `resumeSessionAt`. Qoder describes it as
resuming from a specified message UUID. The public documentation does not fully
define how this option combines with fork, whether its point is inclusive, how
it behaves after compaction, or how it affects file state. See the
[TypeScript SDK reference](https://docs.qoder.com/cli/sdk/references-typescript#options).

The SDK runtime path requires Qoder authentication. A Tuzi key supplies the
model provider credential only. It does not replace Qoder authentication.

### 4.4 Agent SDK standalone history fork

The TypeScript SDK also exports a local history-management function:

```ts
forkSession(
  sourceSessionId,
  {
    dir,
    sessionStore,
    upToMessageId,
    title,
  },
): Promise<{ sessionId: string }>;
```

The [TypeScript SDK reference](https://docs.qoder.com/cli/sdk/references-typescript#session-management-functions)
describes it as creating an independent session from existing history. It can
read local Qoder session storage or an application-supplied `SessionStore`.
The `dir` option identifies the project. When omitted, the helper uses the
process working directory. This value controls the local project bucket or the
external-store project key; it is not only an output location.

The Python SDK exposes the external-store form as
`fork_session_via_store`. It requires store `load` and `append` operations. See
the [Python SDK reference](https://docs.qoder.com/cli/sdk/references-python#external-session-functions).
The generated-target and `ForkSessionResult` details in the surface map are
implementation evidence, not a complete published result-shape contract.

This API has no authentication parameter. SDK `1.0.32` completed the isolated
PoC without a Qoder credential. A later runtime query against the generated
fork still requires Qoder authentication.

### 4.5 Qoder Quest and Editor

The [Quest task-management guide](https://docs.qoder.com/user-guide/quest/task-management#fork-conversation)
states that a user can:

1. Open the session More menu.
2. Select Fork.
3. Get an independent new session with the source context and history.
4. Continue the source and target independently.

Quest can fork from any conversation turn. Editor offers Fork after a session
finishes. The current Qoder task page also documents creating a branch task
from a specific Agent response. See
[Qoder task management](https://docs.qoder.com/qoder/task-management).

`Start Side task` is not the same mechanism. Qoder documents a side task as
temporary state that disappears when Qoder closes. It is not a durable fork
candidate for Mallard.

The app contract does not define:

- the target CLI session UUID;
- app task-ID and CLI UUID equivalence;
- raw transcript membership;
- database rows or index rebuild;
- checkpoint or worktree behavior;
- collision handling;
- a task-import API; or
- a headless fork API.

The documented app flow starts from an existing app task. Qoder publishes no
raw-history import or application-projection rebuild contract. Therefore, the
UI flow is not evidence that Qoder can discover or register Mallard-restored
raw files.

## 5. Mechanisms that are not conversation fork

Do not combine these operations under one product control.

| Operation | Effect | Why it differs |
| --- | --- | --- |
| Resume | Continue the existing identity | Writes can append to the source |
| Conversation rewind | Select an earlier conversation state | Can change the active lineage without a new session |
| File rewind | Restore tracked working files | Does not roll back conversation by itself |
| Git branch | Change source-control lineage | Does not define conversation identity |
| Qoder Worktree mode | Select or create an execution worktree | Does not imply a conversation fork |
| Cloud Agent session threads | Coordinator and child-agent execution | Not a documented user-created local conversation fork |

Qoder documents file checkpoint and rewind separately. External
`SessionStore` cannot be combined with file checkpointing. See
[SDK checkpoint and rewind](https://docs.qoder.com/cli/sdk/checkpoint).

## 6. Standalone SDK fork PoC

### 6.1 Artifacts

| File | Purpose |
| --- | --- |
| [`../poc/qoder-fork-mechanisms/qoder-sdk-fork-probe.mjs`](../poc/qoder-fork-mechanisms/qoder-sdk-fork-probe.mjs) | Product probe against SDK `1.0.32` |
| [`../poc/qoder-fork-mechanisms/qoder-sdk-fork-network-guard.cjs`](../poc/qoder-fork-mechanisms/qoder-sdk-fork-network-guard.cjs) | Denies network and child-process interfaces |
| [`../poc/qoder-fork-mechanisms/qoder-sdk-fork-probe.test.mjs`](../poc/qoder-fork-mechanisms/qoder-sdk-fork-probe.test.mjs) | Version-pinned integration test |
| [`../poc/qoder-fork-mechanisms/README.md`](../poc/qoder-fork-mechanisms/README.md) | Run guide and safety boundary |

The PoC stays outside Mallard's production Rust, Tauri, and React modules.

### 6.2 Isolation

The probe applies these controls:

- It requires the exact `@qoder-ai/qoder-agent-sdk` name, version `1.0.32`,
  and bundle SHA-256.
- It requires a caller-supplied canonical package entry path.
- It creates one exact operating-system temporary root.
- The root name and marker file must match before deletion.
- It sets `HOME`, `QODER_CLI_HOME`, `QODER_CONFIG_DIR`, `TMPDIR`, and the
  working directory below the temporary root.
- The child receives an allowlisted environment.
- The probe rejects inherited Qoder, OpenAI, and Anthropic credential
  variables.
- It invokes `forkSession()` only. It does not invoke `query()`.
- A preload guard blocks Fetch, HTTP, HTTPS, TCP, TLS, DNS, and child-process
  functions.
- The result contains no UUID values, paths, prompts, transcript content, or
  raw package output.
- Cleanup removes only the validated temporary root.

The probe does not read or write the user's Qoder home.

The preload guard is a JavaScript PoC control. It is not an operating-system
network sandbox. A later authenticated test needs a disposable VM or an
equivalent operating-system egress policy.

### 6.3 Synthetic source

The source transcript contains:

- workspace-directory metadata;
- a linear user and assistant history;
- one progress record between two retained messages;
- one unselected main branch;
- one sidechain message;
- one `model_refusal_fallback` system record;
- three content-replacement references, including one unknown message;
- relocation metadata;
- a custom title; and
- an explicit active-leaf record.

The local fixture also contains synthetic root state, compression state,
subagent history, and an observed-shape file-history snapshot record with its
referenced checkpoint object. The test inventories the complete synthetic
transcript, sidecar, and checkpoint roots before and after each fork. These
files let it prove which fixture members the standalone fork does not copy.

The external-store fixture implements `load` and `append`. Its `listSubkeys`
method fails the test if the SDK calls it.

### 6.4 Product result

The command completed with one passed test, zero failures, and zero skips.

| Gate | Result | Meaning |
| --- | --- | --- |
| Exact SDK package | Pass | Package name, version, and bundle SHA-256 matched |
| Synthetic-home boundary | Pass | No real provider home was used |
| Credential isolation | Pass | No credential variable entered the child |
| External-operation guard | Pass | No call reached the guarded network or child-process interfaces |
| Source main transcript | Pass | Source digest stayed unchanged |
| Source sidecars | Pass | State, compression, subagent, and checkpoint digests stayed unchanged |
| New target UUID | Pass | Target differed from source and from the second fork |
| Full source branch | Pass, synthetic | The selected active historical chain was transformed |
| Unselected main branch | Pass, synthetic | Records outside the selected lineage were absent |
| Cut point | Pass, synthetic | `upToMessageId` retained that entry inclusively |
| Sidechain removal | Pass, synthetic | The sidechain record was absent |
| Progress removal | Pass, synthetic | The progress record was absent and its child parent was rewired |
| Message UUID remap | Pass, synthetic | Each retained message received a new UUID |
| Parent-graph remap | Pass, synthetic | Parent and logical-parent references used new UUIDs |
| Logical parent through removed progress | Does not pass | The output referenced a generated UUID with no emitted target record |
| Replacement through removed progress | Does not pass | A retained replacement referenced a generated UUID with no emitted target record |
| Selected sidechain parent | Does not pass | Filtering removed the selected sidechain parent and detached its retained descendant to a null root |
| Origin record | Pass, synthetic | Each copied message had `forkedFrom` source identity |
| Content replacements | Pass, synthetic | Retained references were remapped; unknown and cut references were removed |
| Refusal fallback | Pass, synthetic | The retained system entry had `neutralizedByFork: true` |
| Runtime ownership fields | Pass, synthetic | Team, agent, session-kind, slug, and source-tool fields were removed |
| Active-leaf copy | Pass, negative | No source `active-leaf` record was copied |
| Local target mode | Pass | New JSONL mode was `0600` |
| Local target sidecar | Pass, negative | No target session directory was created |
| Local target checkpoint | Pass, negative | The referenced observed-shape checkpoint object was not copied |
| External store calls | Pass | Exact order was source `load`, then target `append` |
| External subkey read | Pass, negative | `listSubkeys` was not called |
| External target claim | Does not pass | The SDK made no separate target preflight or claim; store `append` owned collision behavior |
| Complete task fork | Does not pass | Only the target main transcript was produced |

### 6.5 Observed standalone transform

SDK `1.0.32` performs this transform:

```text
source main entries
  -> resolve historical active lineage
  -> keep user, assistant, attachment, system, and progress classes
  -> remove sidechain entries
  -> apply optional inclusive cut point
  -> remove progress output records
  -> generate target session UUID
  -> generate new UUID for each copied message
  -> remap parent and logical-parent references
  -> add forkedFrom to each copied message
  -> remap retained content replacements
  -> append relocation when present
  -> append target title
  -> write or append target main transcript
```

When a removed progress record sits in a parent chain, the child is connected
to the nearest retained non-progress ancestor. A logical parent is remapped
directly through the pre-filter UUID map.

This order creates two progress-reference defects in SDK `1.0.32`. When a
retained record's `logicalParentUuid` points to a progress record, the progress
record is removed but its generated target UUID can remain in
`logicalParentUuid`. A retained content-replacement reference can fail in the
same way. No emitted target record owns either UUID. The PoC proves both cases.

The normal `parentUuid` traversal repairs the tested progress edge. Direct
logical-parent edges between retained records also remap correctly. Mallard
must still reject the standalone target until every parent and logical-parent
reference resolves to an emitted target record or an allowed null root.

Sidechain filtering creates a separate defect. In the negative probe, the
selected lineage contains a sidechain parent and a retained non-sidechain
descendant. The helper removes the selected parent and emits the descendant as
a new null root. Thus, filtering can change ancestry even when the selected
lineage itself includes the removed record.

The helper does not copy an active-leaf metadata record. Later Qoder readers
use fallback lineage selection until another active-leaf record is written.
The ordinary fixture is linear after transformation, but the selected-
sidechain negative fixture proves that this is not guaranteed.

The helper sets the last copied message time to the current time. Other copied
message times remain from the source. It removes these package-specific
runtime fields when present:

```text
teamName
agentName
sessionKind
slug
sourceToolAssistantUUID
```

These details are package observations plus the stated synthetic checks. They
are not a stable raw-transcript schema.

### 6.6 Local target publication

The standalone local helper writes:

```text
<project-bucket>/<generated-target-uuid>.jsonl
```

SDK `1.0.32` uses exclusive create and file mode `0600`. An existing exact
target file causes `EEXIST`. The helper does not retry with a second UUID.
Because it generates the UUID internally, the caller cannot deterministically
inject a target collision through this API.

The helper writes no target:

```text
<target-uuid>/state.json
<target-uuid>/compression-v2/state.json
<target-uuid>/subagents/**
file-history/<target-uuid>/**
```

### 6.7 External-store publication

For an external store, the helper performs this sequence:

```text
load({ projectKey, sourceSessionId })
append({ projectKey, generatedTargetSessionId }, transformedEntries)
```

It does not:

- call `listSubkeys`;
- load the target;
- make a separate target preflight or claim;
- require immutable create;
- compare an expected revision;
- use compare-and-swap; or
- roll back a partial append.

The random target UUID makes an accidental collision unlikely. It does not
create a collision contract. Qoder's reference `InMemorySessionStore.append`
can merge entries into an existing key. The public interface delegates target
collision behavior to the store's `append` implementation. A production store
must add an atomic immutable target claim and serialize it with all writers.

Qoder's [external session-storage guide](https://docs.qoder.com/cli/sdk/session-storage#operations)
requires ordered, complete, isolated, idempotent storage and serialized writers.
It also states that the store contains session history, not authentication,
application configuration, file checkpoints, or retention policy. Qoder does
not supply a production store implementation.

The same public guide adds four important limits:

- every host must use the exact same `cwd`, because it derives the project key;
- the external copy is a mirror in addition to Qoder's local transcript;
- mirror writes are best-effort, and final failure is reported as
  `system/mirror_error` without stopping the conversation; and
- when an explicitly resumed store session is absent, the SDK can fall back to
  a local session with the same ID.

Therefore, a `SessionStore` does not, by itself, prove one authoritative source
or one collision-free target.

### 6.8 Standalone source resolution and validation

SDK `1.0.32` requires the standalone source ID and optional cut-point ID to be
UUID-shaped strings. This is stricter than CLI source selection, which also
accepts package-specific safe names and list indexes.

With `dir`, the local helper checks the exact derived project bucket. Without
`dir`, it scans Qoder project buckets and returns the first nonempty transcript
with that UUID. It does not report all duplicate candidates. Directory
enumeration order is not a safe source-selection policy.

For an external store, the helper canonicalizes `dir`, or the process working
directory when `dir` is absent, and derives the store `projectKey` from it.
Every host must use the same project key.

The local parser ignores individual lines that are not valid JSON. It can then
fork the remaining history. This is unsuitable for Mallard integrity checks.
Mallard must reject a malformed source record before it calls Qoder.

The helper uses normal filesystem resolution and does not provide Mallard's
no-follow traversal contract. A future integration must validate the exact
source and destination with Mallard's own canonical-path, regular-file,
symbolic-link, size, count, and digest controls.

The PoC always supplies an exact canonical `dir`, uses regular files only, and
does not test unscoped source search.

## 7. SDK standalone fork and CLI fork are different

| Property | SDK standalone `forkSession()` `1.0.32` | CLI runtime fork `1.1.40` |
| --- | --- | --- |
| Authentication for preparation | None | Attempted during normal headless startup; a deferred failure can permit local fork artifacts before model use fails |
| Target UUID | Generated internally | Generated or caller-selected through runtime options |
| Message UUIDs | Regenerated | Preserved |
| Parent UUIDs | Remapped | Preserved |
| Progress-linked logical parent | Can dangle after progress removal | Relationships are preserved only after parser and resolver normalization; raw top-level `progress` is not a recognized persisted record type |
| Progress records | Removed after lineage selection | Only recognized records that survive parsing and chain resolution are copied |
| Sidechains | Removed; a retained child can detach | Default discovery excludes sidechains, but a persisted active leaf can select one; copied records are forced to `isSidechain: false` |
| Source active-leaf metadata | Not copied | Not copied |
| Target root `state.json` | Not created | Fresh minimal state created |
| Compression sidecar | Not copied | Not copied |
| Task sidecars | Not copied | Not copied |
| Subagent sidecars | Not copied | Not copied |
| File-history objects | Not copied | Can be copied later during resume initialization |
| External-store barrier | Not applicable to direct helper | Paired CLI/runtime capability handshake for supported store-backed query fork |
| Typed target result | `{ sessionId }` | SDK stream has target session identity |

Mallard must not prepare a standalone SDK fork and assume that it is
byte-equivalent to a CLI-created fork.

## 8. CLI `1.1.40` package trace

This section records version-specific package behavior. It does not replace an
authenticated product test.

### 8.1 Option validation and target identity

The package applies these rules:

- `--fork-session` requires `--resume` or `--continue`.
- `--resume-session-at` requires `--resume`.
- `--resume-drops-turn` requires `--resume-session-at`.
- `--session-id` is rejected for an ordinary resume or continue.
- During fork, `--session-id` is the target ID.
- A target must match Qoder's UUID format.
- The target cannot equal the source ID.
- Without `--session-id`, startup uses a process-generated UUID.
- The interactive picker uses a supplied target when present and otherwise
  generates one. `/branch` and ACP fork generate a new UUID.

The public CLI text does not fully define these rules. They need a
version-pinned contract probe before production use.

### 8.2 Source selection

`--continue` selects the newest session across the current Git repository's
known worktree buckets.

`--resume <selector>` accepts a package-specific safe name that matches
`^[A-Za-z0-9][A-Za-z0-9._-]*$` and is not `..`. Thus, the source selector does
not have to be a UUID. A positive decimal can match an exact listed ID first
and then a one-based newest-first list index.
Exact lookup can search the current bucket and related worktree buckets. When
the same ID exists more than once, the package selects the copy with the newest
`lastUpdated` value.

This is unsafe as a Mallard collision policy. Mallard must always bind the
source to an exact canonical project identity and reject duplicate reachable
UUIDs.

### 8.3 Active lineage and compaction

The package resolves the persisted source with:

```text
consistency = flush-pending
view = historical
sessionId = exact source
```

It copies one resolved active lineage. It does not copy every branch in the
source JSONL.

The last matching `active-leaf` record in file order becomes the persisted
candidate. A same-session candidate can select a sidechain, team, or metadata
record because the persisted-leaf path does not apply the default candidate
policy. The fork then forces all copied entries to `isSidechain: false`; this
can promote a selected sidechain into the target mainline.

A missing, foreign-session, or agent-mismatched persisted leaf produces a
warning and falls back, even when the record has `explicit: true`. A separate
resolver-level `leafUuid` supplied by a caller is a hard constraint, but that
is not the ordinary persisted active-leaf path.

Default discovery excludes sidechains but includes team and metadata records.
It prefers stable or complete assistant candidates, then orders by parsed
timestamp, with file order as the tie-breaker. Persisted-leaf file-order
authority and fallback ordering are different rules.

Historical traversal can cross compaction boundaries. At a compact boundary,
the resolver follows `logicalParentUuid` before `parentUuid` when applicable.

The copied chain can contain compaction implementation records. Runtime message
projection hides those records. The source active-leaf metadata record itself
is not copied, and core creation does not write a target active-leaf record.
Until later writer activity, target selection uses default discovery.

Before core creation, the parser and resolver can omit malformed, unknown, or
invalid-shape records; remove transparent attachments; diagnose duplicate
UUIDs, broken parents, or cycles; repair or replace parent relationships;
normalize sidechain flags; and reduce the graph to one usable chain. The fork
can still publish when the normalized result contains a usable user or
assistant record. Native fork is not a byte-preserving clone or a strict
transcript validator.

The separate SDK active-leaf PoC proves the last-record rule for SDK `1.0.32`
on a synthetic branching transcript. It does not make this encoding a public
Qoder compatibility contract.

### 8.4 Message-level cut

The hidden `--resume-session-at <entry-uuid>` retains the resolved chain
through the last matching entry. Thus, the observed cut is inclusive.

`--resume-drops-turn <user-uuid>` is a stale-view guard. It checks the discarded
suffix. Unrelated user, queued, or task work blocks the operation.

Content replacements are filtered to retained message UUIDs. File-history
snapshots are not filtered by this cut in CLI `1.1.40`. A discarded-turn
checkpoint can therefore remain in the snapshot set that later rehydrates.
This mismatch needs a negative product test.

The normal CLI reference does not publish these flags. Mallard must not use
them as a production interface.

### 8.5 Target transcript

Core fork creation requires:

- a nonempty resolved chain; and
- at least one user or assistant record.

For each entry in the chain after Qoder parser and resolver normalization, CLI
`1.1.40`:

- preserves its record UUID;
- preserves its parent and logical-parent UUIDs;
- preserves its timestamp and payload;
- changes top-level `sessionId` to the target;
- forces `isSidechain: false`; and
- writes `forkedFrom: { sessionId: source, messageUuid: entry.uuid }`.

These statements do not mean that raw source relationships and payloads are
preserved before normalization. It then appends target relocation and
worktree-state metadata. It conditionally
adds mode, tag, runtime configuration, custom title, and filtered content
replacements.

It does not clone source summary, AI title, last-prompt, token statistics,
workspace-directory transcript metadata, or active-leaf metadata. `/branch`
generates a unique `(Branch)` title. A headless startup title depends on later
name persistence.

Transcript publication uses exclusive create. The parent directory uses mode
`0700`, and the file uses mode `0600`. A write failure removes the partial
target file.

### 8.6 Target state and sidecars

CLI core fork creates a fresh target `state.json` with:

- target session ID;
- revision `1`;
- new timestamps;
- optional source workspace directories;
- empty data; and
- empty items.

It does not clone source state items.

No core copy was found for:

```text
compression-v2/state.json
subagents/*.jsonl
subagents/*.meta.json
subagents/task-*.json
```

Interactive hydration can read source subagent and task sidecars for display.
It can change a stale source task from `running` to `interrupted`. Thus, the
public statement that the source is unaffected is a semantic conversation
contract. It is not proof that every source byte remains unchanged.

Active `/branch` captures through the live transcript writer. Its read barrier
can commit already-pending source transcript writes before it captures the
fork snapshot.

The headless startup path does not use that interactive sidecar hydration.

### 8.7 File-history checkpoints

Core `createFork` does not copy checkpoint files. It returns the resolved
source snapshot records to later resume initialization.

When file checkpointing is enabled, later rehydration can:

1. Find every checkpoint object referenced by the source snapshots.
2. Create the target checkpoint directory.
3. Hard-link each object to the target when possible.
4. Fall back to an ordinary, nonexclusive `copyFile` operation.
5. Restore the snapshot set in memory.
6. Append target file-history snapshot records when source and target differ.

When file checkpointing is enabled and a checkpoint manager participates,
headless fork sets strict restore. It removes the target checkpoint directory
before the copy. If one copy fails, it removes that target directory and
throws. A headless fork without file checkpointing has no checkpoint copy.

Interactive picker, `/branch`, and ACP do not use strict copy. They accept
existing files, can log individual copy failures, and can continue with an
incomplete or mixed target checkpoint tree.

Checkpoint object names contain a truncated hash of the source path and a
version suffix. They are not content digests. The copy path does not pin source
bytes or verify source and target digests. In non-strict mode, `EEXIST` accepts
an existing object without checking its bytes. A target created between a
failed hard-link attempt and the fallback copy can be overwritten. A successful
hard link also makes both names share one inode; later mutation through either
name can affect both sessions unless another contract makes the object
immutable.

Persisted-source snapshots are filtered to the fully resolved chain, but the
message-level cut does not re-filter them. Live `/branch` passes the live file-
history manager's snapshot set without another proved chain cut. Thus,
checkpoint authority and cut-point alignment remain unproved.

The headless path also makes a best-effort copy of plan state. It first checks
for a free target slug and later writes without exclusive create. A concurrent
writer can take the slug and the copy can overwrite it. Read and write errors
are suppressed, and no target digest or durable copy result is returned. Plan
copy is outside core fork publication.

### 8.8 Collision scope

With an explicit `--session-id` and no remote selector, CLI `1.1.40` checks the
target main transcript in the current target project bucket before full
startup. An auto-generated target has no such early check and relies on the
later core exclusive transcript create. Core exclusive writes separately
protect the main transcript and `state.json`.

The early check does not cover:

- related project or worktree buckets;
- a target session directory that lacks `state.json`;
- compression, task, or subagent sidecars;
- the target checkpoint directory;
- copied plan state;
- Qoder app database projections; or
- external mappings.

If old target sidecars exist without `state.json`, core fork can combine new
state with those old files. If only a target checkpoint directory exists,
strict headless rehydration removes it. Interactive paths can merge with it.

If only target `state.json` exists, the transcript create can succeed before
the state create fails. Core rollback then tries to remove the transcript and
state. Therefore, collision rejection is not always pre-write even when
rollback later succeeds.

Mallard needs an atomic, destination-wide target UUID claim before Qoder runs.
A preflight scan is still racy unless Mallard and Qoder serialize all target
writers. A main JSONL existence check is not sufficient.

### 8.9 Rollback scope

If core transcript creation succeeds and core state creation fails, Qoder tries
to delete the new transcript and state file. It reports cleanup failures.

The reusable core cleanup covers only:

```text
<target>.jsonl
<target>/state.json
```

It does not remove the whole target session directory, compression/task/subagent
sidecars, checkpoint directory, or copied plan.

Interactive picker, `/branch`, and ACP invoke fork cleanup when later target
initialization fails. No fork-specific outer cleanup was found in the headless
startup path. A late strict checkpoint or model-startup failure can leave the
target transcript and state. Later failures can also leave checkpoint or plan
artifacts.

Interactive and ACP cleanup target the original fork transcript path and the
original-project state path. If target initialization relocates the transcript
to a worktree or fallback bucket before a later failure, the trace does not
prove that cleanup removes the relocated transcript.

This is not an atomic complete-task transaction.

### 8.10 Worktree and path behavior

Source lookup can span same-repository worktree buckets. The target transcript
always goes into the current target project bucket.

Interactive picker and `/branch` can attach current worktree metadata to the
new session. They do not create a Git worktree as part of conversation fork.
Headless and ACP core fork write a null worktree-state in the traced path. A
later headless writer can append the current target runtime's worktree state.
It does not copy that state from the selected source.

Historical message `cwd` values remain in copied chain records. The source
`workspace-directories` transcript record is not copied. Resolved source
directories can instead enter fresh target state and can later be emitted by
target-writer initialization. A new relocation record points to the target
working directory.

File-history restore also performs limited path reinterpretation. Relative
tracked-backup keys can resolve under the target checkpoint manager's working
directory. Absolute keys outside that directory continue to refer to source
absolute paths. This is not a general structural path map and does not prove
cross-path portability.

### 8.11 Authentication order

For headless startup:

1. CLI validates option combinations.
2. With an explicit local target ID, it checks the target main transcript.
3. It loads configuration and authenticates.
4. It can retain the authentication error and continue local preparation.
5. It resolves and consumes the source.
6. It can create the fork before returning the final login error.

Authentication is attempted first, but failure is deferred in the normal
headless path. Authentication success is required for model continuation. It
is not a reliable boundary before local source reads or target writes.

The differential clean-home probe supports this distinction. A mode-`000`
source failed as an invalid session. The readable control reached the final
structured `Not logged in` result. The non-fork readable source transcript
kept its size and SHA-256, and no sibling session artifacts appeared. Other
clean-home configuration files were created. This proves source access through
the login-failure path; it does not prove a completed authenticated fork.

ACP fork explicitly authenticates before source resolution and fork creation.
Interactive source resolution occurs in an authenticated session-browser path,
but exact timing can vary with deferred authentication.

Do not describe one authentication order as universal for all Qoder modes.

## 9. SessionStore runtime fork barrier

The supported SDK query path can mirror transcripts to an external
`SessionStore`. A store-backed fork needs coordination between source
materialization, target creation, runtime writes, and mirror writes.

SDK `1.0.32` uses a paired CLI/runtime capability handshake named
`session_store_fork_barrier_v1` for this path. Daemon-backed queries also check
daemon support. The mirror holds target writes until fork history is ready,
flushes the history, then releases later writes.

This barrier solves write ordering for a running SDK fork. It does not provide:

- source multi-key snapshot atomicity;
- atomic target UUID claim;
- target immutable create;
- task-sidecar copy;
- file-history copy;
- application projection; or
- cross-path project identity.

The standalone `forkSession()` helper does not use this barrier.

## 10. Source-preservation meaning

Public Qoder documentation states that the original conversation remains
unaffected. This is a semantic contract. Mallard needs stronger evidence.

| Property | Status |
| --- | --- |
| Source conversation remains usable | Documented |
| Target has independent identity | Documented |
| Target history derives from source | Documented |
| Standalone PoC source fixture stayed byte-identical | Product-proved, synthetic fixture |
| Headless CLI source stays byte-identical after full fork | Unproved |
| Interactive source task sidecars stay byte-identical | Does not hold in all traced paths |
| Source and target checkpoint objects are inode-isolated | Unproved; native copy can use hard links |
| Source app database rows stay unchanged | Unproved |
| Source snapshot is consistent during concurrent writes | Unproved |

Mallard must separately verify transcript bytes, sidecar bytes, checkpoint
objects, plans, and app projections. “Original unaffected” is not enough for a
backup and restore integrity claim.

## 11. Tuzi result and fork

The Tuzi API recheck passed separately:

- model listing returned HTTP `200`;
- `deepseek-chat` was available;
- chat completion returned HTTP `200`; and
- the response had a nonempty choice.

This does not change the fork result.

The standalone SDK fork is a local history transform and does not call a model.
The live SDK query and CLI model-continuation paths require Qoder authentication
before a Qoder-to-Tuzi request can complete. Normal headless CLI can still read
the source and create local fork artifacts while a login failure is deferred.
No disposable Qoder PAT was available. The user's real Qoder login was not
copied or used.

A complete authenticated test must provide two separate credential classes:

```text
disposable Qoder authentication
+ disposable or test Tuzi provider credential
```

Mallard must never capture either credential as session content.

## 12. Mallard impact

### 12.1 Exact restore remains blocked

| Exact-restore requirement | Fork result |
| --- | --- |
| Preserve source UUID | Does not pass; fork creates a target UUID |
| Preserve all source branches | Does not pass; fork selects one lineage |
| Preserve task sidecars | Does not pass |
| Preserve compression state | Does not pass |
| Preserve subagent state | Does not pass |
| Preserve file checkpoints | Only conditional in CLI runtime; absent in standalone SDK |
| Preserve app projections | Unproved |
| Support structural-only cross-path mapping | Unproved |
| Avoid destination collisions | Partial and surface-dependent |
| Complete clean-home continuation | Unproved |

Fork is not an alternative proof for exact restore.

### 12.2 Possible future product operation

A future operation can use this explicit label:

```text
Fork Qoder history
```

It must state:

- A new Qoder session ID will be created.
- One selected conversation lineage will be copied.
- Source and target can diverge.
- Task, subagent, compression, checkpoint, plan, and app metadata can differ.
- The operation is not exact restore.

Mallard should keep this operation disabled until the authenticated gates pass.
If Mallard later owns Qoder execution through the Agent SDK, prefer the SDK
runtime fork because it has a typed target identity and store-fork barrier.
Do not parse interactive `/branch` output or automate the app menu.

### 12.3 Required identity record

If Mallard later enables history fork, record this mapping as machine-local
state:

```text
portable bundle session identity
source provider and project identity
source Qoder session UUID
selected source leaf and cut entry UUID
destination canonical project identity
destination Qoder session UUID
Qoder CLI and SDK version
fork method
approved input manifest digest
result manifest digest
```

Do not put destination paths, credentials, collision inventories, or app
database identifiers in the portable manifest.

## 13. Production gates

| Gate | Current result | Required proof |
| --- | --- | --- |
| Fork surface contract | Partial | Select one supported API and version policy |
| Stable source capture | Partial | Quiescent complete multi-file snapshot or vendor snapshot API |
| Active-lineage selection | Partial | Explicit selected leaf and compaction corpus |
| Parent-graph integrity | Does not pass | Resolve the SDK progress/logical-parent and selected-sidechain detachment defects through a supported contract |
| Content-reference integrity | Does not pass | Reject every replacement that points to a removed or absent target record |
| Cut-point semantics | Partial | First, middle, last, invalid, foreign, and compacted entry tests |
| Source byte immutability | Partial | Full authenticated headless and interactive manifests |
| Target UUID allocation | Partial | Requested and generated target tests |
| Destination-wide collision claim | Does not pass | Atomic claim across transcript, sidecars, checkpoints, plans, mappings, and app state |
| Transcript publication | Partial | Crash and torn-write tests |
| Root state behavior | Package-observed | Product test for fresh minimal state and restart |
| Compression state | Does not pass | Defined copy, rebuild, or deliberate omission contract |
| Task sidecars | Does not pass | Defined copy, rebuild, or deliberate omission contract |
| Subagent sidecars | Does not pass | Defined copy, rebuild, or deliberate omission contract |
| Checkpoint copy | Partial | Mode-specific, cut-point, missing-object, digest, hard-link isolation, race, collision, and rollback tests |
| Plan copy | Partial | Exclusive publication, digest, failure, and cleanup contract |
| External store target claim | Does not pass | Immutable create or compare-and-swap |
| Store subagent fork | Does not pass | Multi-key copy and atomic visibility |
| Qoder model context | Unproved | Target returns a unique source-only marker |
| Tuzi through Qoder | Blocked | Disposable Qoder auth plus Tuzi model request |
| Restart and resume | Unproved | Close, restart, resume target, recall marker |
| Qoder app discovery | Blocked | Supported disposable app profile lists target |
| App projection rebuild | Blocked | Vendor contract or clean-home proof without database edits |
| Cross-path portability | Unproved | Structural mapping with free-form history unchanged |

Production status remains no-go.

## 14. Next PoC plan

### 14.1 Standalone SDK negative corpus

Extend the safe authentication-free test with:

1. First, middle, and last cut points.
2. Missing, malformed, and foreign-session message UUIDs.
3. Multiple active leaves in different file orders.
4. Compact boundaries and compact summaries.
5. Attachment records.
6. Broken parent and logical-parent links.
7. Progress records used as direct logical parents and replacement targets.
8. Selected sidechain parents with retained non-sidechain descendants.
9. An empty active chain.
10. Source append during fork load.
11. External append failure after partial target storage.
12. A store that already contains the generated target, using an injected UUID
    factory in a test-only wrapper if Qoder provides one.

### 14.2 Authenticated CLI and SDK runtime corpus

Use a disposable Qoder account, disposable Tuzi credential, two clean homes,
and an operating-system network policy.

1. Create a source session with a random semantic marker.
2. Add a branch, compaction, task, subagent, plan, and file checkpoint.
3. Close all Qoder processes and capture the complete source manifest.
4. Restore the source to clean home B at the same canonical project path.
5. Fork with a caller-selected target UUID through SDK runtime.
6. Verify `system/init.session_id` and result `session_id`.
7. Ask the fork for the source-only semantic marker through Tuzi.
8. Verify the source transcript and all source sidecars.
9. Verify the exact target transcript, state, sidecar, plan, and checkpoint set.
10. Restart and resume the target.
11. Ask for the marker again.
12. Repeat through CLI startup without SDK store.
13. Repeat with a historical cut before and after compaction.
14. Repeat with each destination collision class.
15. Inject checkpoint copy, plan copy, transcript write, and state write
    failures.
16. Verify complete cleanup or a precise recoverable journal.
17. Test persisted sidechain, team, and metadata active leaves.
18. Test an invalid persisted `explicit` leaf followed by fallback.
19. Verify that no target active-leaf exists at the core-fork boundary.
20. Test a cut before a later file-history snapshot.
21. Race target creation between hard-link failure and fallback copy.
22. Put different bytes under an existing checkpoint object name.
23. Mutate a source and target object after hard-link copy and prove the
    required isolation policy.
24. Fail after transcript relocation and inventory every target bucket.

### 14.3 App corpus

Use a vendor-supported disposable app profile or a disposable macOS account.

1. Create and fork one Quest task from the latest turn.
2. Fork the same task from an earlier turn.
3. Create a branch task from one specific Agent response.
4. Verify separately that `Start Side task` disappears after app restart and
   is not treated as a durable fork.
5. Fork one completed Editor task.
6. Record target task and session identities.
7. Inspect only sanitized projection field names and relationships.
8. Close and restart Qoder app.
9. Verify both source and target remain independently usable.
10. Restore a CLI or SDK fork into a clean app profile.
11. Determine whether the app discovers and projects it without database edits.

Do not automate the user's normal Qoder profile for these tests.

## 15. Final conclusion

Qoder fork is feasible as a conversation-history feature. The standalone SDK
PoC proves that Qoder can create a new, isolated main transcript from a selected
historical lineage without authentication.

The same PoC also proves the limiting fact: standalone fork is not a complete
task clone. It omits root state, compression, task, subagent, checkpoint, and
application-projection state. The broader CLI fork creates only fresh minimal
root state and performs checkpoint handling later with mode-dependent cleanup.

Therefore:

1. Keep production Qoder exact restore disabled.
2. Keep production Qoder fork disabled until authenticated continuation and
   restart pass.
3. Model fork as a separate operation with a new target identity.
4. Prefer a typed SDK runtime result if Mallard later owns Qoder execution.
5. Require destination-wide collision protection and journaled cleanup.
6. Do not claim Qoder app support until clean-home projection rebuild passes.

This result does not change the existing Tuzi conclusion. Tuzi works directly,
but Qoder authentication remains a separate prerequisite for a live Qoder fork
and continuation test.

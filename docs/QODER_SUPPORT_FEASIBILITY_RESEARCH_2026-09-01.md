# Qoder Support Feasibility Research

**Prepared for:** Mallard maintainers

**Research date:** 1 September 2026

**Scope:** Qoder sessions, user skills, and plugin install intent

**Out of scope:** Qoder pets, memory, rules, commands, hooks, connectors, subagents as independent resources, and implementation

**Decision status:** Proceed with a bounded prototype; do not claim production session restore yet

**Session restore PoC:**
[Qoder session restore proof of concept](QODER_SESSION_RESTORE_POC_2026-09-02.md)

**CLI resume PoC:**
[Qoder CLI resume proof-of-concept study](QODER_CLI_RESUME_POC_2026-09-02.md)

**Custom-provider PoC:**
[Qoder CLI custom-provider and Tuzi proof of concept](QODER_CUSTOM_PROVIDER_POC_2026-09-02.md)

**Fork-mechanisms PoC:**
[Qoder fork mechanisms research and proof of concept](QODER_FORK_MECHANISMS_RESEARCH_2026-09-02.md)

## Executive decision

> **Recommendation:** Add Qoder in stages. First, add the safe provider capability gate and local Activity adapter. Make standalone user Skills the first portable-resource candidate, after the installed Qoder app passes a Skill-root probe. Add plugin install intent only when a separately installed Qoder CLI passes an exact contract probe. Do not release Qoder session restore until a disposable two-home test proves that Qoder can discover, open, and continue a restored task without Mallard copying or editing Qoder's application database.

Qoder support is technically possible, but the three requested resource classes do not have the same readiness.

| Capability | Feasibility | Release position | Main reason |
| --- | --- | --- | --- |
| Standalone user skills | Conditional for the Qoder app | First portable-resource candidate | Qoder CLI and IDE document a `SKILL.md` user directory. Mallard already has the safe file-tree model. The installed Qoder app has not yet proved direct discovery or alternate-home support. |
| Qoder app marketplace skills | Conditional | Do not claim full parity yet | The installed Qoder app has no standalone user-skill root in the observed sample. One observed Skill belongs to a plugin. Qoder does not document how every app-installed Skill maps to disk. |
| Plugin install intent | Conditional | Require a public Qoder CLI contract | Qoder CLI documents JSON inventory, marketplaces, install, update, enable, and disable commands. The installed Qoder app does not provide that CLI on this machine. |
| Local session discovery and Activity | Medium | Prototype after the safe sync-capability gate | Qoder writes structured JSONL transcripts. The observed layout and record set differ from the detailed Qoder IDE example. |
| Session capture as a non-restorable backup | Medium | Omit unless a separate backup product is approved | File capture is possible. It is not session portability until Qoder can consume the restored data. |
| Raw session resume in Qoder CLI | Partial, version-pinned | Keep disabled | CLI `1.1.40` lists a restored synthetic session, and a separate fresh native session resumes successfully. Authenticated continuation from copied files is not proved. |
| Fork Qoder history | Transcript-only PoC | Keep disabled and separate from exact restore | SDK `1.0.32` can fork a synthetic main transcript without authentication, but it omits broader task state and the negative corpus found dangling progress references and sidechain-parent detachment. |
| Session restore and resume in the Qoder app | Unproved | Block production release | The app also keeps task references in an application database. No public task-import interface is documented. |
| Open an existing task in the Qoder app | Unproved | Omit the action | The reviewed public deep-link and CLI documentation does not define a Qoder app action that opens an existing task by ID. |
| Qoder pets | Excluded | No work | Pet support was removed from this scope. |

The first production release must not present Qoder as equivalent to Codex or Claude until session restore passes the release gates in this report.

## 1. Scope and terms

Qoder calls a persisted conversation a **task** in the Qoder app and a **session** in Qoder CLI and Agent SDK documentation. Mallard should use **session** in its provider-neutral domain and show **Qoder task** in Qoder-specific user text.

This report assesses these flows:

1. Discover project-owned Qoder sessions.
2. Show Qoder session summaries and a bounded Activity timeline.
3. Capture and restore a reviewed Qoder session through ProjectSync.
4. Capture and apply standalone Qoder user skills through the Library.
5. Capture and apply Qoder plugin install intent through the Library.

This report does not propose support for Qoder pets. Qoder pets must not be added to the Catalog capability matrix, portable resource kinds, inventory, or UI.

This report also does not include Qoder memory. The observed `memory` directories remain outside the proposed resource set.

## 2. Research method and limits

The assessment uses three evidence classes:

- Current Mallard source code and active schemas.
- Public Qoder documentation for the Qoder app, Qoder IDE, Qoder CLI, and Qoder Agent SDK.
- A read-only structural inspection of the installed Qoder app and the user's Qoder state, with the user's explicit permission.

The local inspection recorded file names, directories, JSON field names, versions, and relationships. It did not record prompts, responses, tool output, credentials, account identifiers, actual project paths, or session identifiers. The inspection did not decompile a binary.

The local evidence is a point-in-time sample, not a Qoder compatibility contract. The observed baseline is:

- Qoder app version `0.1.4` on macOS arm64.
- Qoder runtime metadata version `1.1.35` in observed task records.
- Provider-status schema version `1`.
- Plugin-registry schema version `2`.
- Compression-sidecar version `2`.
- Application database `PRAGMA user_version` value `64`.
- Default user configuration directory `~/.qoder`.
- No separately usable `qoder` CLI in the installed app's public wrapper.

These values are diagnostic format gates for the disposable fixture corpus. They are not public compatibility promises. The database value is diagnostic only and must not authorize database parsing or mutation.

Qoder, Qoder IDE, Qoder CLI, and Qoder Agent SDK are separate product surfaces. A contract documented for one surface must not be assumed for another surface. This rule is important because the documented Qoder IDE transcript path differs from the installed Qoder app path.

## 3. Public Qoder contracts

### 3.1 Configuration directory and scope

Qoder CLI documents `QODER_CONFIG_DIR` as the user configuration directory, with `~/.qoder` as the default. The CLI also supports `--config-dir`. Qoder settings use these scopes: [Q4] [Q5]

| Scope | Documented location | Mallard treatment |
| --- | --- | --- |
| User | `<QODER_CONFIG_DIR>/settings.json` | Machine-local configuration; do not copy the complete file. |
| Project | `<project>/.qoder/settings.json` | Project content or Git content; not provider-home state. |
| Local project | `<project>/.qoder/settings.local.json` | Local project configuration; outside this proposal. |

Qoder app Hook settings read the same default user settings file and use compatible Qoder CLI Hook types. This is evidence of partial app and CLI configuration sharing. It is not proof that both products share every session and plugin index. [Q3]

Mallard must keep its current product boundary: provider-home resources come from the bound Qoder configuration directory. Mallard must not capture agent configuration from inside the project directory. Project `.qoder` content remains outside the provider adapter.

### 3.2 Sessions

Qoder app documentation states that each conversation is a task. Coding tasks are organized by workspace. Users can continue, branch, rename, archive, restore, delete, export history, and copy a task ID. The documentation does not describe an import operation for an exported history. [Q1]

Qoder CLI documents these session operations: [Q11]

- Continue the most recent session.
- Resume by session ID.
- Fork a resumed session.
- Assign a session ID and name.
- List sessions.
- Delete a session by list index.

Qoder Agent SDK defines a session as persisted history that includes context, tool calls, and compaction boundaries. The SDK can resume or fork by UUID and can select the user data directory through `QODER_CONFIG_DIR`. [Q12]

Qoder IDE Hook documentation exposes `transcript_path` and describes an append-only JSONL transcript. The example path is:

```text
<QODER_CONFIG_DIR>/projects/<encoded-project>/transcript/<session-id>.jsonl
```

The documented common record types are `session_meta`, `user`, `assistant`, and `progress`. Tool inputs and tool results can contain file content and absolute paths. [Q2]

The installed Qoder app did not use this exact path or this exact record set. Therefore, the Qoder IDE example can guide a parser, but it cannot define the Qoder app codec.

### 3.3 Skills

Qoder CLI documents a Skill as a directory with a required `SKILL.md` file and optional supporting files. The documented user root is:

```text
<QODER_CONFIG_DIR>/skills/<skill-name>/SKILL.md
```

The documented project root is `<project>/.qoder/skills`. Mallard must include only the user root in the storage Catalog because project-root agent configuration is outside the Catalog contract. [Q6]

Qoder app documentation supports marketplace installation, local creation, and upload from a ZIP or `SKILL.md`. It tells users to inspect instructions, scripts, and referenced resources as executable content. [Q7]

The Qoder CLI and Qoder IDE Skill pages disagree about whether user or project Skills take precedence when names collide. Mallard does not need to resolve this discrepancy. Each Library resource has a provider and root-class identity. Mallard must not calculate Qoder's runtime precedence or merge user and project Skills. [Q6] [Q8]

### 3.4 Plugins

Qoder defines a plugin as a package that can contain commands, agents, Skills, Hooks, output styles, executables, workflows, and MCP servers. The recommended manifest is `.qoder-plugin/plugin.json`. A marketplace has a `marketplace.json` manifest. [Q9] [Q10]

Qoder CLI documents these relevant commands: [Q9]

```text
qoder plugins list --json
qoder plugins marketplace list --json
qoder plugins marketplace add <source>
qoder plugins marketplace update [name]
qoder plugins install <plugin>
qoder plugins update <plugin>
qoder plugins enable <plugin>
qoder plugins disable <plugin>
```

Installed marketplace plugin IDs use `name@marketplace-name`. Local plugin IDs use `name@local`. Enablement is written to `enabledPlugins` in the applicable `settings.json`. Plugin marketplace commands can be disabled by a feature flag. Qoder also documents security settings that can block Git extensions or allow only selected source patterns. [Q9] [Q10]

These commands map well to Mallard's existing plugin-install-intent model. Mallard must still prove the JSON schemas, exit behavior, selected configuration directory, app visibility, and post-install inventory before implementation.

### 3.5 Agent SDK external session storage

Qoder Agent SDK provides a public external session-store interface. The SDK mirrors opaque transcript entries to application-controlled storage and can resume them on another host. It also provides an import helper for an existing local session. [Q13]

This interface is useful evidence that Qoder sessions are designed to be portable in an SDK-controlled execution model. It is not a direct solution for Mallard's current product:

- The store is supplied to SDK queries. The normal Qoder app does not document a setting that attaches this store.
- The SDK still writes local history.
- Every host must use the same working directory for the same project key.
- The store does not contain authentication, application configuration, file checkpoints, or retention policy.
- The store cannot be combined with file checkpointing and some transports.
- Mirroring is best-effort and can fail without stopping the task.

Mallard should not add the Agent SDK as a first-release session bridge. That design would create a second Qoder execution path and would not prove that a task appears in the user's Qoder app. Reconsider the SDK only if Mallard later launches and owns Qoder tasks through the SDK.

## 4. Installed Qoder structure

### 4.1 Relevant roots

The installed Qoder app uses two observed local roots:

```text
~/.qoder/
<user-library>/Application Support/com.qoder.app.stable/
```

This report uses `<observed-qoder-home>` for the first root and `<app-support>` for the second root in the local evidence below. In this sample, `<observed-qoder-home>` is `~/.qoder`. `QODER_CONFIG_DIR` is a documented Qoder CLI and Agent SDK contract. The inspection does not prove that the Qoder app honors a custom `QODER_CONFIG_DIR`. Block non-default-home app operations until the Phase 0 probe proves this behavior.

The configuration directory contains task transcripts, per-task state, plugins, plugin data, settings, caches, logs, memory, and authentication-related state. The application-support directory contains an application database and other app state.

Mallard must use an allowlist. It must not copy either root as a directory.

### 4.2 Observed session layout

The installed Qoder app uses this main-task layout:

```text
<observed-qoder-home>/projects/<encoded-project>/<session-id>.jsonl
<observed-qoder-home>/projects/<encoded-project>/<session-id>/state.json
<observed-qoder-home>/projects/<encoded-project>/<session-id>/compression-v2/state.json
<observed-qoder-home>/projects/<encoded-project>/<session-id>/subagents/agent-<kind>-<16-hex-id>.jsonl
<observed-qoder-home>/projects/<encoded-project>/<session-id>/subagents/agent-<kind>-<16-hex-id>.meta.json
<observed-qoder-home>/projects/<encoded-project>/<session-id>/subagents/task-<16-hex-id>.json
```

Some optional paths were not present for every task. The listed sidecar names are observations from one Qoder app version, not a stable external interface. Unknown names must fail closed until a fixture proves their role.

In two observed macOS project roots, the encoded key equaled the absolute working directory with each `/` replaced by `-`. This observed mapping can collide. Behavior for spaces, existing hyphens, Unicode normalization, case differences, other volumes, Windows paths, and later Qoder versions is unproved. Mallard must not use the directory name as proof of project ownership.

The observed main transcript contained these record types:

- `workspace-directories`
- `runtime-config`
- `attachment`
- `user`
- `assistant`
- `active-leaf`
- `last-prompt`

Across the observed `user` and `assistant` record variants, fields included a session ID, record UUID, parent UUID, time, working directory, message, permission mode, origin, and related request identifiers. These fields were not present in every variant.

The transcript contained multiple `active-leaf` records. Their `leafUuid`,
`explicit`, timestamp, and session fields make them branch-tip candidates. The
initial home inspection did not prove selection. A later SDK `1.0.32` reader
probe proved that the last matching record in file order wins for that SDK
projection. The CLI `1.1.40` package trace adds separate persisted-candidate
and fallback rules. No public raw-file contract or authenticated branch product
test exists. Treat the structure as a branch graph and keep active-lineage
presentation blocked.

Sanitized attachment metadata indicated Skill inventory, Hook, system-context, and agent-listing categories. Attachment bodies were not recorded. Attachments can contain executable-resource details and system context. They must receive the same privacy, size, and warning treatment as message and tool records.

The main `state.json` file has a revisioned object with task-specific items. Some item fields are compact and undocumented. Mallard must treat this file as an opaque optional sidecar. Mallard must not infer semantics from compact keys.

The optional compression and subagent files create one consistency group with the main transcript. A session snapshot cannot be considered complete when only the root JSONL file is stable.

### 4.3 Application database

The Qoder application-support directory contains `main.sqlite`, `main.sqlite-wal`, and `main.sqlite-shm`. The same session IDs appear in the database and in configuration-directory transcripts. The observed JSONL record count did not equal the related database message-row count. This means the two stores are not proved to be interchangeable.

The database also contains account, authentication, custom-model, plugin, worktree, and other application state. Mallard must never copy, synchronize, or replace the complete database. Mallard must not modify undocumented database tables to register a restored task.

The database also owns app projections that can include highlights, context usage, recaps, search indexes, sidebar placement, workspaces, managed worktrees, generated images, archive state, deletion state, and titles. A JSONL-only restore can lose these projections even when Qoder can continue the core transcript. A successful transcript resume is therefore necessary but not sufficient for full task parity.

Mallard must never copy or edit `main.sqlite`, `main.sqlite-wal`, or `main.sqlite-shm`.

This dual-store design is the main session-restore blocker. The release prototype must answer one question: does a clean Qoder app rebuild the required task index from a restored session file set? If the answer is no, Mallard needs a documented Qoder import API or vendor-supported registration command.

### 4.4 Snapshot instability

The inspected task tree changed during the read-only inspection. Root transcripts appeared and disappeared, and the number of root task files changed. This is direct evidence of concurrent app writes or cleanup.

Mallard must not capture a Qoder session with one unchecked read. A safe capture needs one of these contracts:

1. The complete Qoder app and its helper processes are closed, and all session files are quiescent.
2. Qoder supplies a stable export or snapshot API.
3. Mallard reads the complete session file set and proves with a second metadata and digest pass that no member changed.

The first prototype must require the user to close the complete Qoder app and its helper processes. Closing one task is not proved to make that task quiescent. After process exit, Mallard must still run the complete two-pass file-set, metadata, and digest check. A later live-capture path can be added only after append and sidecar consistency tests pass.

### 4.5 Observed skills and plugins

No standalone `<observed-qoder-home>/skills` directory existed in the inspected sample. Absence does not mean the documented CLI and IDE user-skill root is unsupported. It means the first test must create a disposable custom user Skill and confirm that the Qoder app and Mallard see it.

One observed `SKILL.md` belonged to an installed plugin and matched the app's Skill inventory. Mallard must not publish that Skill twice. Plugin-owned Skills remain part of plugin install intent, not standalone Skill resources.

Observed plugin state included:

```text
<observed-qoder-home>/plugins/installed_plugins_v2.json
<observed-qoder-home>/plugins/cache/
<observed-qoder-home>/plugins/data/
<observed-qoder-home>/settings.json
```

The plugin registry used schema version 2. The one observed registry key used a qualified `name@source` identity. The settings file used the same key in `enabledPlugins`. The observed plugin was an app-bundled, non-user-visible component. This one sample does not prove the key form for every plugin scope and source. Mallard must exclude bundled and non-user-visible components.

The observed plugin manifest contained a name, display name, version, and description. Its source qualifier came from installation state, not from the minimal manifest. Mallard must not use manifest name alone as portable identity, and it must not infer source identity from a cache directory.

The local disk registry was not a complete list of app extension installations. Application state also represented an inline extension that had no disk-registry entry. This discrepancy supports the decision to use a public CLI contract and to exclude inline, built-in, hidden, managed, or provider-required extensions.

Plugin cache entries can contain native executables for the local architecture. Plugin data is plugin-defined and can contain sensitive configuration. The observed sample contained runtime, endpoint, process, workspace, and index state. Mallard must synchronize install intent only. It must never copy plugin cache or plugin data.

The observed registry files are not a public Qoder compatibility contract. Production plugin inventory must use the documented Qoder CLI JSON commands. Registry files can be used only in a disposable compatibility test or as evidence for a typed blocker. They must not become the primary adapter interface.

## 5. Current Mallard architecture

Qoder crosses two independent Mallard subsystems. The project-session provider registry and the storage Catalog have different provider types and different extension work.

### 5.1 Project sessions

ProjectSync already has an open validated `mallard_domain::ProviderKey`. The frontend mirrors this as `string`. Setup, bindings, Activity, remote visibility, and many DTOs are provider-driven. [M1]

`SessionAdapter` is the provider boundary for:

- Provider descriptors and default profile directories.
- Session and sidecar discovery.
- Bounded identity scanning.
- Transcript analysis and normalized lineage.
- Provider destination target proposals.
- Activity summaries and timeline detail.
- Provider-specific per-session actions. [M2]

The production registry currently registers Codex and Claude. A Qoder adapter would be a compile-time module and registry entry, not a runtime plugin. [M3]

However, full session sync is not adapter-only today. ProjectSync service code still has Codex-or-Claude branches for:

- Tail cursor parsing and observation.
- Portable wire encoding.
- Portable wire decoding.
- Session ID allocation.
- Structural wire-path mapping.
- Provider-index path, planning, and mutation.
- In-place versus copy-on-write materialization policy.
- Cloud-only Pull eligibility analysis.
- Provider-specific post-Pull app refresh.
- Project-level new-session and resume commands. [M4]

Ordinary transcript destination proposals and per-session Activity actions are already adapter-driven. The service still owns the closed policies and mutations listed above.

The session watch service also creates roots only for Codex and Claude. [M5]

This hidden two-provider contract must be removed before Qoder session sync. Do not add a third series of `if qoder` branches. Move the remaining wire, cursor, ID, and path-map behavior behind explicit provider capabilities or typed adapter methods. This work is necessary for the requested third provider and for long-term provider modularity.

### 5.2 Storage Catalog

The storage Catalog is intentionally separate from ProjectSync. It has a closed `ProviderKey` enum with `Codex` and `Claude`, a closed `AgentSkillScope`, and a closed provider capability matrix. [M6]

Catalog plugin adapters, default profile roots, inventory provider arrays, the TypeScript provider union, and display labels also list only Codex and Claude. [M7] [M8]

The Catalog already has the correct resource models:

- Standalone Skill content is copied after bounded, no-follow validation.
- Plugin payload is not copied. Only install intent is portable.
- Pull apply is plan-based, reviewed, revision-checked, and journaled.
- Plugin operations use structured argument arrays, timeouts, bounded output, and post-operation inventory verification. [M9] [M10]

Qoder Catalog support can extend these models, but it needs one narrow capability and inventory refactor. Current Skill inventory calls native plugin discovery first, every Catalog provider is treated as owning both Skills and plugin intent, and the adapter selection is exhaustive. Therefore, a missing Qoder CLI would block Qoder Skill scanning unless these concerns are separated.

Add a Qoder Catalog adapter during the Skill phase, even when plugin operations are unavailable. Make provider ownership explicit per resource class. Let the exact standalone Skill root scan run without Qoder CLI plugin inventory. Advertise plugin intent only in the later CLI-backed phase. Keep both pet kinds Codex-only.

## 6. Proposed Qoder session design

### 6.1 Product capability levels

Use explicit capability levels. Do not register a Qoder adapter that appears fully syncable but fails during Push or Pull.

| Level | User-visible behavior | Required proof |
| --- | --- | --- |
| `activity_only` | Discover local tasks and show bounded Activity. No Push or Pull selection. | Safe discovery, ownership, summary, timeline, and live-file handling. |
| `backup_only` | A separate product and bundle path captures a reviewed file set. UI states that Qoder restore is not supported. | New non-restorable-backup product contract, stable snapshot, and verified encoding. |
| `portable` | Push, Pull, materialize, open, and continue restored tasks. | Complete two-home round trip and app-index reconstruction or public import. |

The current `ProviderDescriptor` does not express these levels. Add the smallest explicit adapter capability needed by review generation. For example:

```rust
fn supports_portable_conversation_sync(&self) -> bool {
    false
}
```

The safe default must be `false`. Codex and Claude must return `true`. Qoder must return `false` until its complete restore contract passes. The gate must stop portable review before cursor seeding, cloud-transcript eligibility reads, plan construction, or publication. Review generation must return a clear codec-unsupported decision and must not offer Push or Pull. Activity, detail, and safe provider actions can still use the registered adapter.

Current ProjectSync cannot produce a non-restorable conversation backup through adapter registration. A `backup_only` product would need a separate resource, manifest, review, retention, export, and recovery design. The boolean portable-sync capability does not express it. Do not call `backup_only` session portability. If Mallard does not intentionally add that separate product, omit this level and keep Qoder sessions at `activity_only` until the portable gate passes.

### 6.2 Adapter module

Add a built-in module under `crates/mallard-infrastructure/src/providers/qoder/` with separated concerns:

```text
qoder/
  mod.rs
  codec.rs
  discovery.rs
  history.rs
  materialization.rs
  actions.rs
```

Use these initial descriptor values:

| Field | Value |
| --- | --- |
| Provider key | `qoder` |
| Display name | `Qoder` |
| Profile label | `.qoder profile` |
| Default profile directory | `.qoder` |
| Setup default | `false` |
| CLI launcher | Present only when the public `qoder` CLI contract is installed and verified |

Do not call an internal app runtime or executable from the app bundle. Use only a public installed CLI resolved through Mallard's existing binary resolver.

### 6.3 Discovery and ownership

Discovery must start at the canonical bound Qoder configuration directory and stay below its `projects` directory. Apply the existing entry, file-count, record-count, line-size, and byte limits.

For each candidate main transcript:

1. Reject symlinks and special files.
2. Validate the file name as a UUID session ID.
3. Read a bounded metadata region.
4. Require all observed session IDs to agree with the file name.
5. Select the transcript dialect through a positive discriminator.
6. Use only that dialect's proved authoritative ownership record.
7. Canonicalize the recorded project root.
8. Require exact equality with the bound Mallard project root.
9. Treat per-turn `cwd` values as activity context, not ownership authority.
10. Discover only sidecars inside the matching session directory.
11. Reject unsafe relative names, traversal, symlinks, and special files in every sidecar tree.

For the observed app dialect, the probe must identify the authoritative `workspace-directories` record. For a documented IDE dialect, prove its separate ownership rule, such as the `session_meta` working directory. Do not apply the app rule to IDE records. Block every dialect or mixed record set that lacks both a positive discriminator and one authoritative ownership rule.

The disposable probe must also define which ownership entry is authoritative for multi-root workspaces. Until this rule is proved, multi-root tasks must fail closed with a clear blocker.

The encoded project directory name is only a location hint. It must not grant ownership because the observed encoding can collide.

### 6.4 Codec and lineage

Create a distinct codec key, such as `qoder.lineage.v1`. Do not reuse the Claude codec because some JSON field names look similar.

The codec must:

- Preserve every accepted JSONL record in file order.
- Parse only the fields needed for identity, ancestry, timestamps, event presentation, and structural path mapping.
- Treat unknown record types and unknown fields as opaque data.
- Reject malformed JSON, conflicting session IDs, duplicate UUIDs, cycles, self-parenting, and invalid branch references.
- Treat `active-leaf` records as branch-tip candidates. Version-gate the SDK
  file-order rule and the different CLI fallback policy.
- Follow `parentUuid` from the selected leaf only after the complete branch,
  compaction, sidechain, team, and metadata corpus passes.
- Keep non-active branches for exact restore, but exclude them from the default active timeline.
- Give branch and incomplete-result warnings without inventing missing parentage.
- Increment the codec version when Qoder changes structural semantics.

The first codec must recognize the observed app records and the documented IDE records as separate dialects under one Qoder provider only if each dialect has a positive discriminator. If there is no safe discriminator, use separate codec keys and block unknown mixtures.

Subagent files must remain sidecars of the main task in the first release. They must not become independent ProjectSync sessions. This keeps task identity and delete behavior coherent.

### 6.5 Portable file set

One Qoder session resource should contain an allowlisted file set. These names are provider-internal role labels, not Mallard bundle paths:

```text
transcript/main.jsonl
state/state.json
state/compression-v2/state.json
subagents/<safe-relative-name>
```

Map the main role to Mallard's canonical `state/conversations/<provider>/<cloud-thread>/transcript.jsonl` path. Map each state or subagent role below a validated `state/conversations/<provider>/<cloud-thread>/sidecars/<kind>/...` path. Do not add a new `ConversationFileRole` enum value unless the serialized-grammar impact is reviewed. The existing transcript role plus typed sidecar roles can represent the proposed files.

The logical bundle path must not expose Qoder's encoded local project directory or local absolute path. Every file must have an individual digest.

Current publication replaces the capture-time resource digest with a transcript-only digest. Do not assume that `content_sha256` protects the complete Qoder file set. The smallest safe design is to pin the existing scope digest, validate the complete expected member list, and carry every transcript and sidecar digest into Pull planning and apply. Add a serialized per-conversation aggregate digest only if the action protocol cannot prove the same invariant with these existing pins.

Current Pull does not fetch or materialize conversation sidecar bytes. It keeps sidecar names, selects the transcript target, and validates a generic sidecar as one JSON document. This is not sufficient for Qoder JSONL subagent files. Full Qoder portability requires all of these changes:

- Let the provider define sidecar encode, decode, and verification behavior.
- Fetch and digest-pin every reviewed cloud sidecar.
- Create a journaled target payload for every planned sidecar.
- Support both JSON and JSONL sidecar codecs.
- Rewrite only proved session-ID and structural-path fields.
- Remove an obsolete destination sidecar only with an exact expected digest and an explicit reviewed action.
- Fail when the restored member set differs from the reviewed member set.

Current capture can also apply generic credential redaction to sidecars. Do not call an undocumented Qoder sidecar opaque and then transform it with a generic JSON rule. A Qoder sidecar is resumable only when a provider-owned codec proves each transformation. Otherwise, preserve exact bytes and treat any detected unsafe content as a blocker.

Do not include:

- Any exact excluded path in section 10.
- Complete settings files.
- Plugin registry, cache, or data.
- Memory.

### 6.6 Structural path mapping

Qoder transcripts can contain old absolute paths in structural fields and in free-form message or tool text. Mallard must rewrite only declared structural fields.

Initial structural candidates are:

- `workspace-directories.directories[]`
- Root record `cwd`
- Known tool input path fields, after the Qoder tool schema is proved
- Known sidecar path fields, after their semantics are proved

Mallard must not run a global string replacement over prompts, model responses, tool output, Hook output, or attachments. A free-form old path can be part of historical evidence and must remain unchanged.

Reuse the scalar matching and round-trip proof ideas from Mallard's `session_path_map` code, but do not use its current whole-record facade for Qoder. That facade recursively rewrites every string value and object key. It cannot enforce Qoder's structural-only rule. Add a Qoder typed field walker, or extend the path-map API so the provider selects fields. Define whether the Qoder codec declares the map in-band or out-of-band, and update conversation validation for that declaration. Require decode, re-encode, and semantic-digest checks before a rewritten transcript becomes resumable. If a field can contain both a path and arbitrary text, do not rewrite it without a typed parser.

Observed message content can contain a `signature` field, but the inspection did not establish what it protects. Any transcript mutation can invalidate a Qoder signature, hash, replay token, or server correlation. Prefer exact-byte restore to the same working directory until Qoder documents a remapping contract. A disposable cross-path test must resume the remapped task. If Qoder rejects or alters the history, keep cross-path restore blocked.

### 6.7 Stable snapshot

Treat the main transcript, state files, and subagent files as one snapshot group.

A first implementation should use this sequence:

1. Enumerate the bounded allowlisted file set.
2. Open each member without following links and read its identity, size, and modification time from the open handle.
3. Read every member to a private captured buffer and calculate its first digest.
4. Read metadata from the same open handle again. Require unchanged identity, size, and modification time.
5. Enumerate the file set a second time and require the same names.
6. Open each member again without following links and record pre-read metadata from the new handle.
7. Read every member again, calculate its second digest, and record post-read metadata from the same handle.
8. Require stable metadata within the second read.
9. Require identical names, file identities, sizes, modification times, and digests between both passes.
10. Reject the snapshot when any file was added, removed, replaced, or changed.

This protocol cannot live only inside `QoderSessionAdapter::discover()`. Discovery returns paths before the common capture executor reads them. Extend `provider_capture.rs` with a snapshot-group protocol, or let it call provider-owned pre-read and post-read hooks around the actual reads. The capture executor must re-enumerate membership after reading. It must use no-follow file opens and compare file identities during the read, not only during discovery. Addition, removal, replacement, or in-place mutation of any transcript or sidecar must fail the complete group.

The UI should tell the user to close Qoder and retry after an instability blocker. Do not silently retry an active task until it happens to look stable.

Do not enable a tail cursor merely because JSONL is normally appended. The observed task tree also adds and removes sidecars, and `active-leaf` changes branch selection. Enable cursor capture only after provider-specific tests prove a complete incremental state machine.

### 6.8 Materialization and restore

Qoder materialization must remain disabled until all restore gates pass.

When enabled, the adapter must:

1. Preserve the source session UUID by default.
2. Classify the plan as a new local materialization or an update to an existing Mallard mapping.
3. Encode the destination project bucket without trusting the source bucket name.
4. Write only below the destination Qoder configuration directory.
5. For a new mapping, block when the source UUID already exists in any destination path or mapping.
6. For an update, require that the same cloud task maps to the exact destination UUID.
7. For an update, require that every existing target and the complete member set match the reviewed expected digests and file identities.
8. Back up and journal each exact target before an approved overwrite.
9. Apply path mapping only to proved structural fields.
10. Materialize the complete file set atomically as far as the platform permits.
11. Never edit `main.sqlite`, `main.sqlite-wal`, or `main.sqlite-shm`.
12. Start Qoder only after all files and digests are complete.
13. Verify that Qoder lists and resumes the task.

Never treat an unbound, pre-existing session with the same UUID as an update target. Never repurpose a UUID that belongs to another cloud task. A stale digest, changed file identity, or unexpected member must block the plan and require a new review.

The observed session UUID occurs in the root file name, session directory name, root records, `state.json`, subagent transcripts, subagent metadata, task metadata, path fields, and both `sessionId` and `session_id` fields. It also appears in application database projections. A replacement UUID is not a file rename. Do not allocate one until a versioned typed reference map proves every rewritable occurrence and proves that unknown records cannot contain hidden references. Until then, a UUID collision during new materialization is a hard blocker.

If Qoder does not rebuild its application index, stop. Do not add a SQLite registration layer. Request a public import API or documented registration command from Qoder.

### 6.9 Actions

The public Qoder CLI can resume a session by ID, but the inspected machine does not have that CLI. Qoder app documentation does not define a stable deep link to open an existing task by ID.

Initial action policy:

- `activity_only`: no provider action.
- Public CLI installed and verified: offer **Open in Terminal with Qoder** and pass `--config-dir`, `--cwd`, and `--session-id` as structured arguments.
- Qoder app only: optionally offer **Open Qoder** without a session target, if the app launch is stable.
- Never label a generic app launch as **Open this task**.

Do not use the older Qoder IDE deep-link contract as a Qoder app task contract.

## 7. Proposed Qoder Skill design

### 7.1 Resource boundary

Add `QoderUser` to the closed Catalog `AgentSkillScope` and map it to provider `qoder` and root class `provider/qoder`. Resolve the root as:

```text
<QODER_CONFIG_DIR>/skills
```

This root is a documented Qoder CLI and IDE contract. It is not yet a proved custom-home contract for the installed Qoder app. Phase 0 must prove creation, discovery, and loading in two app homes before Phase 2 implementation starts. If only Qoder CLI passes, describe the result as Qoder CLI Skill support, not Qoder app Skill support.

Do not scan `<project>/.qoder/skills`. Do not add a shared `.agents/skills` root for Qoder unless Qoder documents support and a later requirement asks for it.

### 7.2 Reuse the existing Skill tree

Reuse the existing Skill-tree scanner for:

- Required `SKILL.md`.
- Bounded `SKILL.md` content and optional leading `name:` extraction.
- Bounded file count, file size, and total bytes.
- Regular files only.
- No symlinks or special files.
- Validated relative paths.
- Deterministic file ordering and digest.

Reuse the Catalog service and apply flow for:

- Secret and executable-content warnings.
- Explicit warning acknowledgement before apply.
- Plan-based destination replacement and backup.

The current scanner does not fully parse YAML front matter or validate its description. Its generic resource-name rule is also wider than Qoder's documented rule. Add a Qoder-specific metadata validator for required front matter, directory and declared-name agreement, lowercase letters, digits, hyphens, the 64-character name limit, and the documented description limit. Keep the original files unchanged after validation.

### 7.3 Ownership and duplicate prevention

Standalone Skill discovery must start only at the configured user Skill root. Plugin-owned Skills outside that root are not candidates.

If a future Qoder CLI inventory reports a plugin install root that overlaps the user Skill root, use the existing plugin-ownership exclusion before scanning Skills. Do not publish one Skill both as content and as plugin intent.

### 7.4 Prerequisite Skill probe and acceptance test

Run this probe in Phase 0, before the Qoder Catalog implementation. Use two temporary `QODER_CONFIG_DIR` values for Qoder CLI tests only. Do not pass this variable to the Qoder app until a vendor-supported app-home selector is proved. If such a selector exists, use it with two temporary app homes. If it does not exist, run the app test in two isolated OS accounts or disposable virtual machines, each with its own default home. If neither method is available, keep Qoder app Skill support blocked.

Create one harmless Skill with a support file and a script. Prove all of these:

1. Qoder lists or can invoke the Skill.
2. Mallard discovers the Skill under a Qoder Catalog profile.
3. Mallard pushes the exact reviewed tree.
4. A second isolated Qoder home pulls the Skill.
5. Qoder loads the restored Skill.
6. Mallard warns about the script.
7. A symlink, special file, oversize file, unsafe name, and malformed front matter fail closed.

## 8. Proposed Qoder plugin-intent design

### 8.1 Required public interface

Add `QoderCatalogAdapter` in the Skill phase with plugin operations unavailable. Enable its plugin inventory and apply methods only after an installed public Qoder CLI passes a contract test. Use:

```text
QODER_CONFIG_DIR=<selected-home> qoder plugins marketplace list --json
QODER_CONFIG_DIR=<selected-home> qoder plugins list --json
```

The process runner must set the selected home explicitly, use structured arguments, set the home as the working directory, bound stdout and stderr, apply the existing timeout, and redact all returned errors.

Fix the common output-bound contract before Qoder uses it. The current runner reads at most the configured byte limit, so a later `length > limit` check cannot prove truncation. Read one byte beyond the limit or return an explicit truncation flag. Reject truncated output before JSON parsing.

The currently installed Qoder app is not sufficient for this path. Its public wrapper reports that Qoder CLI is not installed. Mallard must show a typed requirement such as **Install Qoder CLI to manage Qoder plugins**. Mallard must not locate or invoke an internal runtime from the app bundle.

### 8.2 Inventory mapping

The CLI adapter must map only user-scope, user-visible, installed marketplace plugins to portable intent.

For each entry, require:

- Qualified plugin identity.
- Marketplace identity.
- Installed state.
- User scope.
- Source type and source location from the configured marketplace inventory.
- Optional observed version.
- Enablement state as machine-local inventory observation only.

The contract probe must prove where marketplace source identity comes from and how Qoder resolves duplicate names across marketplaces. Do not use manifest `name` alone. Do not derive source identity from the cache path. If the public inventory cannot provide a stable qualified identity, plugin intent remains blocked.

Exclude:

- Built-in and app-bundled plugins.
- Non-user-visible plugins.
- Project and local scopes.
- Local directory plugins that have no portable source.
- Uninstalled marketplace listings.
- Plugin cache and plugin data.
- Credentials, connector authentication, and Hook runtime state.

If Qoder CLI JSON does not expose the source or scope needed for this mapping, return `NativeContractUnverified`. Do not infer a Git URL from a cache path or internal registry.

### 8.3 Portable intent and resolution

Reuse Mallard's `PluginInstallIntentResource`. Use exact source rules:

| Qoder source | Mallard resolution |
| --- | --- |
| Git marketplace with validated portable location | Automatic install intent can be offered. |
| HTTPS `marketplace.json` | Manual-only until Mallard's portable source model explicitly supports and validates this form. |
| Local marketplace directory | Manual-only; never store the local absolute path. |
| Direct local plugin | Manual-only. |
| npm, Git object, or other marketplace plugin source | The marketplace remains the install source; do not copy plugin payload. |
| Unknown or app-bundled source | Exclude or block. |

Do not copy source enablement or include `enabledPlugins` in portable intent. Mallard must not issue enable or disable commands. Qoder can apply its native default during install or update. Report the resulting state, but do not compare it with the source as an apply condition. If enablement later becomes a requirement, add a separate explicit, reviewed, digest-pinned state and action contract. Other user settings remain local.

### 8.4 Apply mapping

For a verified Git marketplace, the likely native plan is:

```text
qoder plugins marketplace add <source>
qoder plugins marketplace update <marketplace>
qoder plugins install <plugin-name>
```

For overwrite or update:

```text
qoder plugins update <plugin-name>
```

The exact arguments are not final until the contract probe confirms how Qoder resolves duplicate plugin names across marketplaces and how `--scope user` is applied. Prefer a fully qualified ID when Qoder accepts one. Otherwise require that the selected marketplace resolves the name uniquely.

After every apply, rerun both JSON inventory commands. Success means the expected qualified plugin ID, source, scope, and installed state are present. Process exit code alone is not success.

### 8.5 Plugin safety

Qoder plugins can execute Hooks, commands, binaries, agents, and MCP servers. A Pull review must state this consequence. Preserve Mallard's warning acknowledgement and native-contract blockers.

Do not automatically authenticate connectors. Do not copy the plugin-data tree. Do not override the destination Qoder security policy. A native Qoder policy rejection is a final typed blocker, not a reason to modify settings.

## 9. Mallard change surface

### 9.1 Project-session foundation

| File or module | Phase | Required change |
| --- | --- | --- |
| `crates/mallard-infrastructure/src/providers/mod.rs` | Activity | Export the Qoder adapter module. |
| `crates/mallard-infrastructure/src/providers/registry.rs` | Activity | Register Qoder only after the safe sync-capability gate and Activity adapter are complete. |
| `crates/mallard-infrastructure/src/providers/traits.rs` | Activity | Add `supports_portable_conversation_sync()` with a safe `false` default. |
| `crates/mallard-infrastructure/src/providers/codex/mod.rs` | Activity | Return `true` for the portable-sync capability. |
| `crates/mallard-infrastructure/src/providers/claude/mod.rs` | Activity | Return `true` for the portable-sync capability. |
| `crates/mallard-infrastructure/src/providers/qoder/` | Activity | Add discovery, codec analysis, Activity history, and safe actions. Keep materialization blocked. |
| `src-tauri/src/project_sync/service.rs` | Activity | Gate portable review before cursor seeding, cloud transcript reads, cloud-only eligibility analysis, action planning, or publication. |
| `src-tauri/src/project_sync/session_watch_service.rs` | Activity or later | Derive watch roots from registered, bound provider adapters. Activity can use uncached discovery before this performance change is complete. |
| `src/types.ts` | Activity | Mirror a capability DTO only if the frontend needs it. The project provider key remains `string`. |
| `src/components/project-sync/` | Activity | Render Qoder from descriptors and disable unavailable sync and session actions. Avoid provider-name checks. |
| `src/components/project-sync/pullReviewModel.ts` | Activity | Parse validated provider session resource IDs generically. Remove the fixed `codex` and `claude` resource-ID check. Keep frontend provider-key validation aligned with Rust. |
| `src-tauri/src/sync_tests/provider_extensibility.rs` | Activity | Prove that Qoder Activity remains visible while Push and Pull are blocked before publication. |
| `crates/mallard-infrastructure/src/providers/traits.rs` | Portable sync | Add typed provider operations for first-contact adoption, tail observation, wire encode and decode, sidecar codecs, session-ID policy, structural path selection, index behavior, materialization policy, and post-apply behavior. |
| `crates/mallard-domain/src/conversation_tail.rs` | Portable sync | Add Qoder codec constants and pin validation only if the proved Qoder design uses tail cursors. |
| `crates/mallard-domain/src/lib.rs` | Portable sync | Add explicit Qoder binding-codec, wire-stamp, tail, path-map declaration, and complete file-set validation. Do not apply Claude rules to every non-Codex provider. |
| `crates/mallard-domain/src/tests.rs` | Portable sync | Prove Qoder binding, wire-stamp, tail, path-map, and file-role rules. |
| `crates/mallard-infrastructure/src/provider_capture.rs` | Portable sync | Add a common snapshot-group protocol with no-follow reads, member re-enumeration, file-identity checks, and post-read digests. |
| `crates/mallard-infrastructure/src/session_path_map.rs` | Portable sync | Reuse scalar matching but add provider-owned structural-field selection. Prove free-form strings and keys remain unchanged. |
| `crates/mallard-infrastructure/src/conversation_materialization.rs` | Portable sync | Fetch, decode, verify, journal, and apply JSON and JSONL sidecar payloads. Preserve common ownership, digest, no-clobber, and exact-deletion checks. |
| `src-tauri/src/project_sync/service.rs` | Portable sync | Remove Codex-or-Claude fallthroughs. Dispatch wire, cursor, decode, ID, path map, index, materialization, sidecar payload, and project launch behavior through the adapter. Prevent a Qoder receipt from producing a Codex refresh. |
| `src-tauri/src/sync_tests/provider_extensibility.rs` | Portable sync | Extend the generic provider test from discovery to a complete Push and Pull round trip. |

### 9.2 Storage Catalog

| File or module | Required change |
| --- | --- |
| `src-tauri/src/storage_catalog/domain.rs` | Add `ProviderKey::Qoder` and `AgentSkillScope::QoderUser`. Make provider ownership explicit per resource class. Keep pet ownership Codex-only. |
| `src-tauri/src/storage_catalog/adapters.rs` | Add a Qoder adapter in the Skill phase with plugin operations unavailable. In the plugin phase, add CLI inventory and plans, set `QODER_CONFIG_DIR`, and add explicit output-truncation detection. Test the exact capability matrix. |
| `src-tauri/src/storage_catalog/profiles.rs` | Resolve the Qoder default home through the project provider descriptor. Add the Qoder user Skill root and exhaustive provider cases. |
| `src-tauri/src/storage_catalog/inventory.rs` | Separate standalone Skill scanning from plugin-first inventory. Include Qoder according to its per-resource capabilities. Never include Qoder in pet inventory. |
| `src-tauri/src/storage_catalog/skill_tree.rs` or a focused Qoder validator | Add Qoder front-matter, name, description, and directory-agreement checks without weakening the shared bounded tree scanner. |
| `src-tauri/src/storage_catalog/apply.rs` | Reuse and regression-test the provider-neutral journaled apply path. Add no Qoder plugin file-copy path. |
| `src-tauri/src/storage_catalog/service.rs` | Remove fixed provider loops and exhaustive Codex-or-Claude app and terminal launch matches. Test automatic Qoder default profiles. |
| `src/components/storage-catalog/types.ts` | Extend `CatalogProviderKey` with `qoder`. Do not change pet resource kinds. |
| `src/components/storage-catalog/model.ts` | Add the Qoder label without a binary conditional. |
| `src/components/storage-catalog/AddCatalogScopeWorkspace.tsx` | Use the returned capability matrix for provider availability and text. Remove “Codex and Claude” fixed copy. |
| `tests/frontend/storage-catalog.integration.test.tsx` | Add closed-provider-union, Qoder capability, profile, label, Skill, plugin blocker, and no-pet coverage. |
| `src-tauri/tests/storage_catalog_e2e.rs` | Add an isolated two-home Skill round trip. Add the stub-Qoder-CLI plugin-intent round trip only in the plugin phase. |

### 9.3 Schemas

Project provider keys are already open. Activity-only Qoder registration does not change the bundle schema.

Portable Qoder conversations must update schema-1 conversation validation for the required wire stamps, tail contract, and path-map declaration. Current validation names only Codex and Claude for some rules and forbids the Claude-style out-of-band path map for other providers. A numeric schema bump is a separate version-policy decision. Do not assume that an opaque provider key and generic sidecar role make Qoder portable without these validation changes.

The Catalog provider enum is serialized in machine-local and portable documents. Adding Qoder changes the accepted schema-1 value set. Under Mallard's clean-cut version policy, a numeric schema bump is a product decision, not an automatic requirement. Update Rust and TypeScript together and update all schema fixtures. Record the downgrade consequence: an older Mallard build cannot deserialize a Catalog profile document that contains the new closed `qoder` enum value.

Do not add a pet format, pet resource kind, or pet schema value for Qoder.

### 9.4 Required regression surfaces

Update these existing test surfaces as each phase lands:

- `src-tauri/src/sync_tests/provider_extensibility.rs`: production provider list, Activity visibility, and the early portable-review gate.
- `crates/mallard-domain/src/tests.rs`: Qoder binding codec, wire stamp, tail, path-map declaration, and conversation file-role rules.
- `crates/mallard-infrastructure/src/provider_capture.rs`: member add, remove, replace, and in-place mutation between discovery and capture.
- `crates/mallard-infrastructure/src/conversation_materialization.rs`: JSON and JSONL sidecars are fetched, decoded, target-mapped, journaled, verified, restored, and removed only by exact reviewed action.
- `src-tauri/src/project_sync/service.rs`: generic new and resume command dispatch; a Qoder receipt cannot trigger a Codex restart; free-form path text remains byte-identical while declared structural fields move.
- `src-tauri/src/storage_catalog/adapters.rs`: exact Qoder Skill, plugin-intent, and no-pet capability matrix; CLI truncation blocks before parse.
- `src-tauri/src/storage_catalog/service.rs`: automatic Qoder default profiles and fixed provider-loop removal.
- `tests/frontend/storage-catalog.integration.test.tsx`: closed frontend provider union, Qoder capabilities, and pet invariants.
- `src-tauri/tests/storage_catalog_e2e.rs`: isolated two-home Qoder Skill flow and later stub-CLI plugin flow.

## 10. Security and privacy requirements

Qoder session files can contain prompts, responses, source fragments, tool arguments, tool results, Hook output, absolute paths, repository metadata, and Skill inventory. Treat the complete session file set as sensitive conversation content.

All existing Mallard safety controls apply:

- Explicit project and provider profile binding.
- Exact canonical project ownership.
- Bounded reads, lines, records, files, and total bytes.
- No-follow traversal and symlink rejection.
- Special-file rejection.
- Private staging and SHA-256 verification.
- Immutable publication.
- Reviewed Pull plans with revision, digest, and expiry checks.
- Exact target backups before overwrite.
- No recursive deletion.
- Sanitized activity logging.
- No provider-home or checkout paths in portable manifests.

Add these Qoder-specific controls:

- Never copy or edit `main.sqlite`, `main.sqlite-wal`, or `main.sqlite-shm`.
- Never copy a complete Qoder settings file. The proposed plugin intent does not need a settings field. If a later resource needs one, add a separate reviewed allowlist for that non-secret scalar.
- Exclude bundled and non-user-visible plugins.
- Treat all plugin code and Skill scripts as executable.
- Require a stable multi-file session snapshot.
- Require explicit structural path maps. Never replace free-form text.
- Fail closed when Qoder version, transcript dialect, sidecar layout, or CLI JSON does not match a tested contract.
- Test only with temporary Qoder homes and synthetic sessions. Never run destructive tests against a real Qoder home.

Use this exact exclusion baseline for the observed app layout:

```text
<observed-qoder-home>/.auth/**
<observed-qoder-home>/mcp-router.json
<observed-qoder-home>/.qoder-app-status.json
<observed-qoder-home>/installation_id
<observed-qoder-home>/ipc/**
<observed-qoder-home>/logs/**
<observed-qoder-home>/shell-snapshots/**
<observed-qoder-home>/tmp/**
<observed-qoder-home>/.cache/**
<observed-qoder-home>/.models/**
<observed-qoder-home>/memory/**
<observed-qoder-home>/settings.json
<observed-qoder-home>/plugins/installed_plugins_v2.json
<observed-qoder-home>/plugins/cache/**
<observed-qoder-home>/plugins/data/**
<app-support>/auth.v1.dat
<app-support>/auth.machine-id
<app-support>/Cookies*
<app-support>/Local State
<app-support>/Network Persistent State
<app-support>/Local Storage/**
<app-support>/Session Storage/**
<app-support>/main.sqlite
<app-support>/main.sqlite-wal
<app-support>/main.sqlite-shm
```

Apply this denylist below exact validated parent roots. Do not expand it from an unresolved path. The observed MCP router schema has an API-key field. The app-status file has identity fields. The installation file is machine identity. Application support contains authentication, cookies, and browser storage. The database contains account, custom-model credential, MCP OAuth credential, remote-host, scheduled-task, and plugin tables. Plugin data contains workspace bindings and runtime, endpoint, process, and index state. These observations identify data classes only; no value was recorded.

## 11. Terms and vendor approval gate

Qoder's Terms of Service were last updated on 29 April 2026. Section 3.2.9 prohibits reverse engineering, disassembly, or decompilation. Section 3.2.10 broadly restricts scraping, mining, or distilling service information and content by manual or automated means. Section 5.3 states that users are responsible for redundancy and backup of data they require. [Q14]

Qoder also publicly documents transcript paths for Hooks, task history export, local session persistence, CLI session management, and an Agent SDK external session store. These documents support user backup and integration use cases, but they do not explicitly authorize a third-party desktop product to parse and rewrite Qoder app task files.

This is a contract ambiguity, not a technical conclusion. This report is not legal advice.

Before production release:

1. Ask Qoder for written confirmation that Mallard can read, store, path-remap, and restore user-owned local task transcripts for portability.
2. Ask for a documented Qoder app import or task-registration interface.
3. Ask whether the Qoder app officially shares plugin and Skill state with Qoder CLI under `QODER_CONFIG_DIR`.
4. Record the supported versions and compatibility policy.
5. Review Qoder's current Terms and privacy policy again at release time.

Until this gate is complete, use only documented files, public CLI or SDK interfaces, and synthetic disposable state. Do not inspect binaries, call private services, automate Qoder UI scraping, or depend on undocumented database tables.

## 12. Implementation sequence

### Phase 0 — Contract and disposable proof

**Goal:** Resolve the technical and product blockers before production code depends on Qoder state.

1. Obtain the vendor and legal answer described in section 11.
2. Install the public Qoder CLI separately in a test environment.
3. Create two temporary `QODER_CONFIG_DIR` values for CLI and SDK probes and two temporary project roots.
4. Identify a vendor-supported Qoder app-home selector. Do not assume that `QODER_CONFIG_DIR` controls the app.
5. If no app-home selector exists, use isolated OS accounts or disposable virtual machines for separate default app homes. If that is not possible, block app two-home acceptance.
6. Create a standalone Skill in each isolated app home and prove app discovery before Phase 2 code starts.
7. Generate synthetic app and CLI sessions with no private repository content.
8. Record only sanitized schemas and behavior.
9. Run the session, Skill, and plugin probes in section 13.

**Exit condition:** Every release-blocking question has a pass, fail, or explicit product reduction.

### Phase 1 — Qoder local Activity and safe registration

**Goal:** Discover and inspect Qoder tasks safely without enabling session sync.

1. Add the safe-default portable-sync adapter capability.
2. Make Codex and Claude return `true` and Qoder return `false`.
3. Short-circuit portable review before cursor seeding, cloud transcript reads, eligibility analysis, planning, or publication.
4. Add Qoder discovery and metadata validation.
5. Add the Qoder codec. Show active-branch lineage only after the complete
   versioned lineage corpus and authenticated branch test pass.
6. Add bounded summaries, timeline indexes, pages, and event detail.
7. Add snapshot-instability blockers.
8. Register Qoder as `activity_only`.
9. Use uncached Activity discovery until Qoder watch roots are adapter-driven.

**Exit condition:** Mallard shows only exact project-owned synthetic tasks. Qoder has no Push or Pull action, and Qoder data cannot enter the wire publication path.

This phase also makes the registered Qoder descriptor available to the Catalog's default-profile resolver. Do not hard-code `.qoder` independently in two subsystems.

### Phase 2 — Qoder standalone user Skills

**Goal:** Add the lowest-risk portable Qoder resource.

**Prerequisite:** The Phase 0 app-root and two-home Skill probe passed for the supported Qoder app version.

1. Add the Qoder Catalog provider and `QoderUser` Skill scope.
2. Add a Qoder Catalog adapter whose plugin operations are explicitly unavailable.
3. Split standalone Skill scanning from plugin-first inventory so a missing CLI does not block Skills.
4. Advertise the exact capability set: Qoder Skills only; no plugin intent and no pets.
5. Resolve the default profile through the registered Qoder descriptor.
6. Reuse the existing Skill scanner and apply engine with Qoder metadata validation.
7. Add backend and frontend integration tests.

**Exit condition:** A synthetic Skill moves between two isolated Qoder homes and Qoder loads it.

### Phase 3 — Qoder plugin install intent

**Goal:** Add provider-native plugin recovery without payload copying.

1. Add the CLI contract fixture tests.
2. Fix common CLI output-truncation detection.
3. Add Qoder plugin and marketplace JSON parsers.
4. Enable Qoder plugin intent in the explicit capability matrix.
5. Add user-scope and built-in exclusion.
6. Add native install and update plans. Do not copy source enablement or call enable or disable commands; Qoder applies its native default.
7. Add post-apply verification.
8. Show a typed CLI-required blocker when the public CLI is absent.

**Exit condition:** A synthetic Git marketplace plugin is captured as intent, installed into a second temporary home, and verified by the public CLI and Qoder app.

### Phase 4 — Third-provider session foundation

**Goal:** Remove hidden two-provider logic before Qoder portable sync.

1. Move wire encode and decode behind the provider contract.
2. Move first-contact adoption and cloud-only eligibility behind the provider contract.
3. Move tail behavior behind the provider contract.
4. Move session-ID preservation, collision policy, and any future coordinated allocation behind the provider contract.
5. Add provider-owned transcript and sidecar encode, decode, and verification behavior.
6. Fetch, pin, journal, materialize, and verify every reviewed sidecar.
7. Move materialization policy and path-map declaration behind the provider contract.
8. Add structural-field selection instead of whole-record path rewriting.
9. Move index planning and post-apply behavior behind the provider contract.
10. Derive watcher roots and project launch commands from registered providers.
11. Prove that Codex and Claude behavior and schemas do not change.

**Exit condition:** The existing test provider can exercise complete generic dispatch without being parsed as Claude or Codex.

### Phase 5 — Qoder session portability

**Goal:** Enable Push and Pull only after Qoder consumes restored files.

1. Add multi-file portable encoding and structural path mapping.
2. Add destination targets, source-ID preservation, collision blocking, and exact backups.
3. Add the common stable snapshot-group capture protocol.
4. Run the two-home matrix across supported Qoder versions.
5. Verify exact-byte same-path restore before cross-path mutation.
6. Verify app listing, open, continue, branch, app-projection behavior, and later re-capture.
7. Promote Qoder from `activity_only` to `portable` only after all gates pass.

**Exit condition:** Every session acceptance criterion in section 14 passes without database edits.

## 13. Disposable probe matrix

All probes must use temporary user homes, temporary Qoder configuration directories, temporary projects, synthetic content, and test accounts when authentication is required. App probes must use a proved vendor-supported home selector or separate disposable OS accounts or virtual machines. Do not assume that the app reads `QODER_CONFIG_DIR`.

### 13.1 Session probes

| Probe | Expected result |
| --- | --- |
| Create one task | One stable session ID maps to the main transcript and allowed sidecars. |
| Continue task | Existing session ID continues and the snapshot change is understood. |
| Rename task | Determine which public file or API owns the display name. |
| Branch from response | Confirm the version-pinned file-order rule, persisted-candidate validation, fallback ordering, sidechain/team/metadata leaves, compaction, and restart behavior. |
| Fork task | Original and fork have distinct valid identities and shared ancestry is handled. |
| Compact task | Compression sidecar and main transcript remain a consistent restorable unit. |
| Run subagent | All required subagent sidecars are discovered and restored. |
| Archive and restore | Determine whether archive state is file-based, database-only, or public-API controlled. |
| Qoder running during capture | Unstable snapshot fails with a clear retry message. |
| Qoder closed during capture | Snapshot is stable and repeatable. |
| Move to a different project path | Structural paths map to the destination; free-form history does not change. |
| Resume after path remap | Qoder accepts the modified transcript; signatures, hashes, replay state, and server correlation remain valid. |
| Restore to clean Qoder home | Qoder lists, opens, and continues the task without database changes. |
| Re-capture restored task | Mallard sees one task with stable identity and no duplicate. |
| New-mapping session-ID collision | Restore blocks without overwriting or allocating a replacement UUID. |
| Existing mapped-session update | Update proceeds only for the same cloud task and exact destination UUID after all reviewed target digests, identities, and member names still match. |
| Unknown record contains source session ID | Restore blocks any future ID remap because an opaque record can hide an old reference. |
| Unknown record and field | Raw data survives; Activity warns or omits only the unknown presentation. |
| Malformed or oversized record | The task fails closed without hiding other safe tasks. |
| Colliding encoded project names | Ownership comes from validated transcript metadata, not the bucket name. |
| Project path variants | Test spaces, boundary hyphens, Unicode normalization, case-only changes, external volumes, symlink roots, Windows drive letters, and Windows separators. |
| App database absent in clean home | Record which title, archive, search, sidebar, recap, highlight, generated-image, workspace, and worktree projections Qoder rebuilds. |
| Retention cleanup | Mallard reports disappearance and never restores or deletes without review. |

Test the current supported Qoder app version and at least one later release before production. Qoder changes rapidly, so a single-version corpus is insufficient.

### 13.2 Skill probes

| Probe | Expected result |
| --- | --- |
| Standalone user Skill | Qoder and Mallard both discover it. |
| Two isolated app homes | Through a supported selector or isolated OS environments, the Qoder app loads only the Skill in the applicable home. |
| CLI-only custom home | Report CLI Skill support only; do not claim Qoder app support. |
| Project Skill | Catalog ignores it. |
| Plugin-owned Skill | Catalog does not publish it as standalone content. |
| Same name in user and project roots | User resource identity remains stable; Mallard does not choose runtime precedence. |
| Script or executable | Review requires warning acknowledgement. |
| Symlink or special file | Capture blocks. |
| Changed destination after plan | Apply rejects the stale plan. |

### 13.3 Plugin probes

| Probe | Expected result |
| --- | --- |
| CLI missing | Typed requirement; no internal runtime fallback. |
| JSON output exceeds limit | Inventory blocks. |
| Unknown JSON field | Entry parser remains forward-tolerant. |
| Missing identity, scope, or source | Entry is skipped with warning or inventory blocks according to safety impact. |
| User Git marketplace plugin | Portable install intent is created. |
| Same plugin name in two marketplaces | Public inventory supplies a stable source-qualified identity and install selects the intended source. |
| Minimal manifest without source | Manifest name alone is rejected as portable identity. |
| Local plugin | Manual-only; no local path becomes portable. |
| Bundled non-user-visible plugin | Excluded. |
| Project or local-scope plugin | Excluded from the global Catalog. |
| Destination security policy blocks source | Native blocker is shown; settings are not relaxed. |
| Install exits zero but inventory is absent | Apply fails verification. |
| Concurrent inventory change | Revision or post-state verification rejects the stale action. |
| Plugin contains native code | Payload is never copied; destination obtains its own build from the source. |

## 14. Production acceptance criteria

### 14.1 Sessions

Qoder session portability is releasable only when all criteria pass:

1. Mallard discovers only sessions owned by the exact bound project.
2. Every captured session is a stable, bounded, no-follow file set.
3. The codec proves session identity, the authoritative active-leaf rule, active lineage, and structural path fields for the accepted version.
4. Unknown records survive an exact encode and decode cycle.
5. Free-form message and tool text remains unchanged.
6. The portable manifest contains no provider-home or checkout path.
7. Exact-byte same-path restore succeeds before cross-path mutation is enabled.
8. A cross-path restore preserves Qoder signatures, hashes, replay state, and server correlation.
9. A clean destination receives only allowed session files.
10. A new mapping preserves the source session UUID and blocks any destination collision without overwrite or unproved remapping.
11. Qoder discovers the restored task without Mallard reading or editing application database tables.
12. Qoder opens and continues the restored task by the same approved identity.
13. Required app projections rebuild, or the product explicitly excludes them without fabricating state.
14. Branches, compaction, and subagents remain valid.
15. A second Mallard capture does not create a duplicate or false divergence.
16. Qoder running during an unsafe snapshot causes a blocker.
17. Unsupported Qoder versions and diagnostic format values fail closed.
18. An existing mapped-session update applies only to the same cloud task and destination UUID after all reviewed target pins still match.
19. The vendor and terms gate is complete.

If criterion 11 fails, production session restore remains blocked.

### 14.2 Skills

Qoder Skill support is releasable when:

1. Qoder's public user root is proved for the supported product and version.
2. The existing Skill safety tests pass for `QoderUser`.
3. Plugin-owned and project Skills are excluded.
4. A two-home round trip loads in Qoder.
5. Rust and frontend Catalog capability tests include Qoder Skills and exclude Qoder pets.

### 14.3 Plugins

Qoder plugin intent is releasable when:

1. A public Qoder CLI is installed and version-probed.
2. CLI inventory returns stable qualified IDs, scopes, and marketplace sources.
3. Only user-visible user-scope plugins become resources.
4. Local and unknown sources remain manual-only.
5. Apply uses only public native commands.
6. Post-apply inventory proves the result.
7. No plugin payload, data, credential, or Hook state becomes portable.
8. The Qoder app sees the CLI-installed plugin in the selected supported home.

## 15. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Qoder app path differs from Qoder IDE documentation | Parser or discovery can miss or corrupt tasks | Versioned dialects and fixture corpus; fail closed on unknown layout. |
| Application database is required for task listing | Restored files do not become usable tasks | Require index rebuild proof or public import; never edit the database. |
| App database owns task UI projections | Transcript resumes but titles, archive state, search, sidebar, recaps, highlights, images, or worktrees are absent | Define required product parity and verify every required projection after clean-home restore. |
| Session file group changes during capture | Mixed transcript, state, and subagent generations | Quiescence requirement and two-pass group snapshot. |
| Current Pull ignores sidecar payloads | Restored task is incomplete or cannot resume | Fetch, pin, journal, verify, and materialize every reviewed sidecar through provider codecs. |
| Source UUID exists for an unbound destination task | New materialization overwrites a task or creates mixed references | Preserve the source UUID and block the new mapping until a complete typed remap is proved. |
| Mapped destination changes after review | An update overwrites unreviewed local state | Require the same cloud-task mapping plus exact member, identity, and digest pins; then back up and journal each overwrite. |
| Encoded project key collides | A task can be assigned to the wrong project | Exact canonical ownership from authoritative transcript metadata. |
| `active-leaf` selection changes | Activity shows the wrong branch | Versioned branch probe for file order, timestamp, `explicit`, restart, cycle, and missing-parent behavior. |
| Path mapping invalidates an unknown signature | Qoder rejects or changes restored history | Prefer exact-byte same-path restore; require a disposable resume test for every structural mutation. |
| Unknown attachments contain secrets or executable context | Sensitive data moves unexpectedly | Treat all session content as sensitive; review, warnings, limits, and verified storage. |
| Public Qoder CLI is absent | Plugin inventory and apply cannot use a supported interface | Typed CLI requirement; no app-runtime fallback. |
| App and CLI use different inventories | CLI success does not change the Qoder app | Cross-product contract probe and post-install app verification. |
| Plugin cache has platform-specific native code | Copied plugin fails or creates supply-chain risk | Install intent only; destination fetches through Qoder. |
| Qoder Skill UI uses plugin-backed resources | Mallard could duplicate or miss Skills | Limit first release to the documented standalone user root and state this scope. |
| Qoder updates change local formats | Previously supported sessions become unsafe | Version corpus, codec versioning, and typed unsupported-version blocker. |
| Terms do not clearly cover third-party portability | Product or account risk | Written vendor clarification and legal review before release. |

## 16. Open questions for Qoder

1. Is the installed Qoder app task JSONL and sidecar layout a supported external interface?
2. Can the Qoder app rebuild its task index from restored files in a clean configuration directory?
3. Is there a public command, URL, or API to import a task and open it by task ID?
4. Does the Qoder app honor `QODER_CONFIG_DIR` or another supported profile-home override?
5. Are Qoder app, Qoder IDE, and Qoder CLI transcripts guaranteed to be compatible?
6. Which file is authoritative for task title, archive state, pin state, and workspace grouping?
7. Is `workspace-directories` the immutable task-ownership record for the Qoder app?
8. Which `active-leaf` record is authoritative when several records exist, and what does `explicit` change?
9. Which session-ID fields and signatures must change during a supported fork or cross-path import?
10. What is the supported snapshot protocol while Qoder is running?
11. Does the Qoder app officially share CLI plugin marketplace and install state?
12. Does every app-created standalone Skill materialize under `<QODER_CONFIG_DIR>/skills`?
13. Can Mallard parse, path-remap, store, and restore user-owned local transcripts under Qoder's Terms?
14. What compatibility and deprecation period does Qoder provide for local formats and CLI JSON?

## 17. Verification after implementation

Run the smallest relevant checks during each phase. Run the complete handoff set after a Tauri contract or provider behavior changes:

```text
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --test storage_catalog_e2e
npm run build
npm run test:frontend-integration
```

Also run the disposable Qoder corpus and two-home tests for every supported Qoder transcript dialect and public CLI version. These tests must use injected temporary roots. They must not read or write the user's real Qoder configuration directory or application-support database.

Before release, run:

```text
npm run test:integration
```

For a Qoder UI or native-action change, test the complete Tauri application on the supported operating systems. Include a screenshot or recording in the pull request. Record the tested Qoder app and CLI versions in the pull request.

## 18. Final conclusion

Qoder is a viable Mallard provider, but support must be split by resource class.

- **Land the safe Activity capability gate first.** This lets Mallard register Qoder without exposing a broken Push or Pull path.
- **Use standalone user Skills as the first portable-resource candidate.** The file-tree model has a small change surface, but Qoder app support remains conditional on the Phase 0 app-root and two-home discovery probe.
- **Implement plugin install intent only through the public Qoder CLI.** The app-only installation on the inspected machine does not provide that interface. Do not parse internal registry files as the production contract.
- **Prototype sessions, but do not release restore yet.** Qoder JSONL is suitable for bounded discovery and Activity. The separate application database and undocumented sidecars prevent a safe portability claim until the two-home test passes.
- **Refactor the remaining Codex-or-Claude session branches before Qoder sync.** Qoder must use a real provider contract, not a third set of service fallthroughs.
- **Do not add Qoder pet support.** Pets remain outside this plan.

The decisive next action is Phase 0. A small disposable proof can determine whether Qoder session portability is possible without an unsupported database integration. Separate probes determine whether Mallard can ship Qoder app Skills, CLI-backed plugin intent, and read-only Activity.

## Sources

### Qoder public sources

- [Q1] [Qoder task management](https://docs.qoder.com/qoder/task-management)
- [Q2] [Qoder IDE Hooks and transcript format](https://docs.qoder.com/extensions/hooks)
- [Q3] [Qoder app Hooks](https://docs.qoder.com/qoder/hooks)
- [Q4] [Qoder CLI configuration scope](https://docs.qoder.com/cli/config-scope)
- [Q5] [Qoder CLI settings, environment variables, and paths](https://docs.qoder.com/cli/settings-reference)
- [Q6] [Qoder CLI Skills](https://docs.qoder.com/cli/Skills)
- [Q7] [Qoder app Skills](https://docs.qoder.com/qoder/skills)
- [Q8] [Qoder IDE Skills](https://docs.qoder.com/extensions/skills)
- [Q9] [Qoder CLI Plugins](https://docs.qoder.com/cli/plugins)
- [Q10] [Qoder CLI Plugin Reference](https://docs.qoder.com/cli/plugins-reference)
- [Q11] [Qoder CLI commands and parameters](https://docs.qoder.com/cli/cli-reference)
- [Q12] [Qoder Agent SDK Session Control](https://docs.qoder.com/cli/sdk/session-control)
- [Q13] [Qoder Agent SDK External Session Storage](https://docs.qoder.com/cli/sdk/session-storage)
- [Q14] [Qoder Terms of Service](https://qoder.com/product-service)

### Mallard source anchors

- [M1] [`ProviderKey` and project domain contracts](../crates/mallard-domain/src/lib.rs)
- [M2] [`SessionAdapter` and provider descriptors](../crates/mallard-infrastructure/src/providers/traits.rs)
- [M3] [Project provider registry](../crates/mallard-infrastructure/src/providers/registry.rs)
- [M4] [ProjectSync orchestration](../src-tauri/src/project_sync/service.rs)
- [M5] [Session watch service](../src-tauri/src/project_sync/session_watch_service.rs)
- [M6] [Storage Catalog domain](../src-tauri/src/storage_catalog/domain.rs)
- [M7] [Storage Catalog provider adapters](../src-tauri/src/storage_catalog/adapters.rs)
- [M8] [Storage Catalog frontend types](../src/components/storage-catalog/types.ts)
- [M9] [Storage Catalog Skill tree](../src-tauri/src/storage_catalog/skill_tree.rs)
- [M10] [Storage Catalog apply engine](../src-tauri/src/storage_catalog/apply.rs)

# Plan: CLI Git-Hook Checkpoint Capture

**Status:** proposed; feasibility research complete; provider contract spike
pending; implementation not started

**Date:** 2026-08-07

**Research baseline:** Entire CLI `caa0c9be90261fb2b64bf6cfc7147ee3981494db`
and v0.9.0; Git 2.54 hook configuration; current Codex hook manual; GitHub
webhook documentation

**Scope:** exact local Git commit-to-session links for Codex and Claude, with
review-first portability as a later phase

**Repository baseline:** the current 2026-08-07 working tree, including the
packaged storage CLI and in-progress provider-neutral Activity/session-detail
contracts; older plan status labels are not treated as implementation truth

## 1. Decision summary

Mallard can integrate its packaged CLI with hooks, but the core mechanism
should be **local Git hooks**, not GitHub repository webhooks.

The recommended design borrows Entire's two-layer model:

1. provider lifecycle events identify the active Codex or Claude session; and
2. local Git events identify a commit or rewrite boundary.

Mallard should not copy Entire's Git-backed transcript store, working-tree
snapshots, hidden checkpoint refs, or automatic `pre-push` publication. A Git
hook should only append a small, bounded, machine-local association. No hook
may upload a transcript, publish a bundle, restore content, or contact storage.

The initial feature is therefore:

- opt-in per clone and explicitly reviewed before installation;
- `post-commit` capture plus `post-rewrite` repair;
- provider leases narrow the candidate set, while an exact link requires an
  explicit selection or a separately proven provider commit event;
- unresolved capture when zero or multiple sessions are plausible, never a
  timestamp guess labeled as exact;
- existing successor-commit inference retained for historical sessions;
- no commit-message trailer in the default mode; and
- no data leaving the machine until a later normal Mallard Push review selects
  portable checkpoint metadata.

An optional durable-trailer mode may be evaluated later for teams that want
the association to survive arbitrary Git history movement:

```text
Mallard-Checkpoint: <opaque-checkpoint-id>
```

That mode must be a separate opt-in because it changes commit messages and the
opaque marker travels wherever the Git commit is pushed.

### Feasibility

| Capability | Assessment | Main condition |
| --- | --- | --- |
| Observe local commits offline | High | Install a local Git hook |
| Show many checkpoints per session | High | Add an authoritative checkpoint-link store |
| Attribute a commit to a session exactly | Medium | Require manual selection or prove a provider commit event |
| Preserve links through amend/rebase | High | Consume bounded `post-rewrite` mappings |
| Share links through Mallard storage | High, later | Define a reviewed portable schema contract |
| Use GitHub webhooks for local capture | Poor fit | GitHub cannot see unpushed commits or local provider sessions |

## 2. Terminology: three different kinds of hook

The phrase "GitHub hooks" can refer to unrelated mechanisms.

| Mechanism | Runs where | What it can know | Role in this plan |
| --- | --- | --- | --- |
| Local Git hook | Developer machine during `git commit`, rewrite, push, and other Git operations | Repository, worktree, local commit | Core Git boundary |
| Provider/agent hook | Codex or Claude lifecycle | Provider session and lifecycle event | Core attribution boundary, after capability validation |
| GitHub webhook | GitHub server after a hosted event | Pushed repository/ref/commit | Optional later reconciliation or PR annotation |

A GitHub webhook arrives only after a GitHub-side event. It requires a hosted
HTTPS receiver and repository or GitHub App permission, and it cannot inspect
a developer's local Codex/Claude home. It cannot replace either of the first
two layers.

## 3. Research findings

### 3.1 What Entire does

`entire enable` installs five local Git hooks. It detects agent providers and
offers their hook integrations; provider hooks are optional and can be managed
separately with `entire agent add/remove`.

| Hook | Entire's purpose |
| --- | --- |
| `prepare-commit-msg` | For matching sessions with new content, prompt, annotate an editor message, or auto-link according to commit mode/policy, then insert an `Entire-Checkpoint` trailer |
| `commit-msg` | Maintain valid commit-message behavior around that trailer |
| `post-commit` | Condense a valid trailer-linked session into a permanent checkpoint; otherwise advance active session base state |
| `post-rewrite` | Update active session bases and migrate affected shadow state after amend/rebase |
| `pre-push` | Drain queued checkpoint refs or synchronize the shared checkpoint branch |

Entire resolves the active hook directory through Git, handles linked
worktrees, optionally uses an absolute CLI path for GUI clients with a sparse
`PATH`, backs up and chains existing hooks, and specially replays
`post-rewrite` stdin to both handlers. Its stable commit trailer exists because
ordinary Git pushes do not automatically push Git notes and commit object IDs
change during rewrites.

Entire also owns substantially more storage behavior than Mallard needs here:

- active session files in the Git common directory;
- temporary shadow branches containing session metadata and working-tree
  snapshots;
- a supported shared checkpoint branch, retained as the config-less fallback,
  or the per-checkpoint Git-ref backend recommended for new configured setups;
- a local publication queue for the Git-ref backend; and
- `pre-push` synchronization to the selected Git remote.

The useful pattern is the combination of provider identity and Git identity.
The unsuitable part is moving source snapshots and transcripts into Git-owned
storage before Mallard's normal review flow. Entire's own security material
notes that temporary snapshots can contain sensitive source and that
checkpoint content pushed with a public repository becomes visible to readers
of that repository.

The current Entire Codex adapter merges handlers into `.codex/hooks.json` for
`SessionStart`, `UserPromptSubmit`, `Stop`, and `PostToolUse`. Its current
source says hooks are enabled by default beginning with Codex 0.124.0 and does
not write `.codex/config.toml`, while some live Entire documentation still
describes the older feature-flag flow. This drift is a reason to
capability-probe supported provider versions instead of copying a configuration
recipe into Mallard.

### 3.2 What Git supports

Traditional Git hooks are executable files resolved from the configured hook
directory. `core.hooksPath` can redirect that directory, and linked worktrees
make assumptions such as `<root>/.git/hooks` incorrect.

Git 2.54 added configuration-defined, named hooks. Multiple named hooks can
subscribe to the same event, and the traditional hook-directory program runs
after configured hooks. This is the preferred coexistence mechanism because
Mallard does not need to replace Husky, Lefthook, pre-commit, Entire, or a
user-owned script.

The Apple Git in the research environment is 2.50.1, so Mallard cannot require
the new mechanism. Installation must feature-probe `git hook list`; parsing a
version string alone is insufficient.

The relevant hook semantics are:

- `post-commit` runs after the commit exists and cannot change that commit;
- `post-rewrite` receives old/new object IDs on stdin for supported amend and
  rebase operations, although not every history-rewriting tool invokes it;
- `prepare-commit-msg` can edit the message and cannot be skipped with
  `--no-verify`;
- `commit-msg` can edit or abort and can be skipped with `--no-verify`; and
- `pre-push` can abort the whole push.

The first release uses only the two post-event hooks. It does not place Mallard
on the critical path before a commit or push succeeds.

### 3.3 Current Codex hooks make the provider half feasible

The current official Codex hook contract is stronger than the older surface
used by the inspected Entire adapter. Codex documents `SessionStart`,
`SessionEnd`, `UserPromptSubmit`, `PostToolUse`, `Stop`, and other lifecycle
events. Every command hook receives a `session_id`, `cwd`, event name, and an
optional transcript path. The transcript format is explicitly not a stable
hook interface, so Mallard should use the identity and cwd fields and continue
to read conversations through its provider adapter.

Codex discovers hooks in user and project configuration and from plugins.
Matching hooks from multiple sources all run, while command handlers for the
same event can run concurrently. New or changed non-managed commands are
skipped until the user reviews and trusts their exact hash through `/hooks`.
Mallard must preserve that trust step and must not depend on execution order
relative to another hook.

This establishes a credible Codex lease adapter using user-level hooks that
no-op for unbound projects. Phase 0 still pins the packaged Codex versions and
tests CLI, app, worktree, resume, compaction, and shutdown behavior. Claude's
corresponding event contract remains an explicit exit gate rather than an
assumption.

The documented Codex shape does not include a dedicated successful-Git-commit
event with a full object ID. `PostToolUse` can expose a Bash command and result,
but parsing shell text is not proof of authorship. Codex can therefore provide
good lease candidates now; automatic `exact` links still require a future
structured event, explicit user confirmation, or the separately reviewed
trailer/policy modes.

### 3.4 Why GitHub webhooks are a separate project

A real GitHub webhook path would require:

- a public or relayed HTTPS receiver;
- a GitHub App or repository webhook with appropriate permission;
- `X-Hub-Signature-256` verification and delivery-ID deduplication;
- durable asynchronous processing and failed-delivery reconciliation; and
- a join from a pushed commit back to machine-local provider data.

It would be useful later for remote reconciliation, status checks, or a PR link
to already-reviewed Mallard material. It would not improve local attribution.

## 4. Fit with Mallard's current architecture

The repository already has most of the read and packaging path:

- [`crates/mallard-cli`](../crates/mallard-cli) is a real packaged CLI with
  stable `--json` output, sanitized errors, and storage commands. It does not
  yet have project, session, Git, or hook commands.
- [`mallard-application`](../crates/mallard-application/src/lib.rs) is the
  shared use-case seam used by CLI and Tauri adapters. Hook workflows belong
  here rather than in either presentation adapter.
- [`LocalStateRepository`](../crates/mallard-infrastructure/src/persistence.rs)
  already provides bounded, atomic machine-local persistence, no-follow file
  handling, mode protection, and a cross-process lock under `~/.mallard`.
- [`get_mallard_cli_path`](../src-tauri/src/project_sync/commands.rs) resolves
  the signed CLI next to the packaged desktop executable.
- [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs)
  already combines provider-neutral session summaries with bounded,
  first-parent Git history.
- [`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs)
  already emits checkpoint events and the frontend contract already accepts a
  vector of checkpoint jumps.

The existing association is intentionally inferred. It maps a session to its
first successor commit using branch, recorded SHA, and end time. Session detail
similarly derives zero or one checkpoint and labels it `inferred`. A hook-backed
store can add exact, many-to-many links without deleting that useful historical
fallback.

Important gaps are:

- no authoritative commit-to-session document;
- no active provider-session lease shared with a Git hook;
- no Git-hook installer or CLI commands;
- session detail currently holds only one optional derived checkpoint;
- timeline cursors do not include a checkpoint-store revision; and
- the app's configuration watcher does not observe checkpoint-link changes.

Portable bundle scope digests are canonical contracts. Adding a structured
checkpoint collection to `BundleScope` would change that contract even if a
Serde default made old JSON deserialize. Portability must not silently
reinterpret schema 1.

## 5. Settled product and safety decisions

### D1 — Installation is explicit and plan-based

Mallard never installs executable hooks merely because a project is opened,
pulled, or bound. The user requests an installation plan, reviews the selected
method and conflicts, and applies that exact plan by digest. A pulled bundle
can never enable or trust a hook.

### D2 — The default mode does not modify commit messages

The core flow uses `post-commit` and local state. It does not require a trailer,
Git notes, hidden branches, or custom refs. This keeps ordinary Git history
unchanged and keeps checkpoint visibility inside Mallard's review boundary.

Trailer mode is a later, separate capability with an explicit warning that the
marker becomes part of the commit and may become public.

### D3 — Git observation alone is not exact attribution

A Git hook knows the repository and commit, not which session produced it.
Multiple Codex/Claude sessions, a human commit, or two bound profiles can all
be plausible at once.

An association is `exact` only when one of these evidence paths succeeds:

- the user explicitly links a bounded session candidate to the commit; or
- a versioned provider event directly identifies that session's successful
  commit and Mallard independently resolves the same full object ID.

A unique active lease is useful evidence, but a human can commit while an agent
session is active. Lease or timestamp correlation therefore remains
`inferred` by default. Zero or multiple leases create an unresolved
commit-boundary record for later review. A separately reviewed auto-link policy
may accept a sole fresh lease, but its evidence remains `policy_linked` rather
than being silently presented as user-confirmed.

### D4 — Hooks are local metadata writers only

A hook may validate Git context and atomically append a small event. It must
not:

- read or hash a full transcript;
- snapshot staged or working-tree files;
- read file contents to infer ownership;
- contact GitHub, a Git remote, or Mallard storage;
- invoke Push, Pull, restore, or dependency apply;
- write a Git ref, branch, note, index, or source file; or
- persist raw hook stdin, environment variables, remote URLs, prompts, tool
  payloads, or credentials.

### D5 — Hook failures fail open

Mallard-owned capture failures must never block Git. Every installed command
goes through a stable fail-open launcher that supervises the CLI, handles a
missing executable, nonzero exit, crash, signal, and deadline expiry, and
returns zero after recording only a bounded local diagnostic. A timeout inside
the CLI cannot supervise its own hang, so Phase 0 must prove an external,
cross-platform watchdog before hook installation ships. Failure is exposed
later by `status`/`doctor` and the desktop UI.

The hook path uses a dedicated short-lived checkpoint lock or append journal,
not the host-operation lock used by publication and restore.

### D6 — Exact links win; inference remains visible

Activity and session detail read exact records first. A session may link to
many commits, and one commit may link to many sessions. Inference is used only
when no exact record covers that session/commit relationship and stays visibly
`inferred`.

Exact means the association followed a user-confirmed or provider-commit
evidence path. It does not mean the provider or commit is cryptographically
authenticated.

### D7 — Provider hooks require a versioned capability gate

Phase 0 records, for each supported Codex and Claude version, whether lifecycle
events reliably supply:

- native session identity;
- canonical or provable project cwd;
- worktree identity;
- event time and lifecycle state; and
- enough information to resolve Mallard's profile, binding revision, and
  logical `ThreadId`; and
- if automatic exact attribution is claimed, a directly matchable successful
  commit event and full object ID rather than a parsed shell-command guess.

Mallard merges with existing provider configuration rather than overwriting
it. Provider hook commands, absolute paths, trust choices, and install receipts
remain machine-local and are never captured in a portable bundle. If a
provider only supports project-file configuration, the first release refuses
automatic installation: that file may be Git-tracked and leave through an
ordinary Git push. Any later project-file option is an explicit source-file
modification outside the core checkpoint flow.

For Codex, prefer the active profile's user-level hook layer so Mallard can
no-op for unbound projects without writing `.codex/` into the checkout. The
install plan must tell the user to review the resulting command through
Codex's `/hooks` trust UI; Mallard neither bypasses nor fabricates that trust.
Use `SessionStart`/`SessionEnd` to open and expire a lease and bounded turn/tool
events only to refresh it. Do not read the unstable `transcript_path` format in
the hook handler.

### D8 — Worktrees share installation but not active-session state

Installation is keyed by Git common-directory identity and reference-counted
across bound worktrees. Runtime attribution is keyed by the canonical current
worktree. Removing one binding must not disable a shared hook needed by another
worktree.

Submodules are independent repositories. Bare repositories are unsupported.
Mallard never assumes `.git` is a directory.

### D9 — No `pre-push` in the capture feature

Mallard already has an immutable, reviewed Push path. A Git push must not
silently publish Mallard data or be blocked because checkpoint storage is
unavailable. A later advisory-only push warning can be proposed separately,
but is not needed for capture.

### D10 — Portability is a normal reviewed resource change

Phase 1 links are machine-local. A later portable checkpoint projection is
selected, reviewed, published, planned, and applied through the same Push/Pull
contracts as other Mallard state. It contains no executable hook configuration
or local trust decision.

### D11 — Checkpoint state participates in machine lifecycle transactions

Host rotation, provider/profile removal, binding replacement, project removal,
worktree removal, and startup recovery must account for leases, boundary
events, links, plans, and shared installation references. Extend the existing
host-rotation/recovery journal to rebase the new identities, or quarantine the
checkpoint ledger until an explicit repair can do so. Never leave a hook
pointing at a deleted binding, and never uninstall a common-repository hook
while another bound worktree still references its receipt.

## 6. Target flow

```text
Provider lifecycle hook                  Local Git hook
          |                                   |
          | update bounded lease              | resolve full HEAD
          v                                   | snapshot lease IDs/revisions
   ~/.mallard active leases  ---------------->|
                                              v
                              append exactly one boundary event
                                              |
                                      background replay
                                              |
                            +-----------------+----------------+
                            |                                  |
                      proposed/unresolved               proven association
                            |                                  |
                      user review/policy                       |
                            +-----------------+----------------+
                                              v
                                   checkpoint read index
                                              |
                          +-------------------+------------------+
                          |                                      |
                    Activity/detail                       later Push review
                    evidence-aware view                  portable metadata
```

For normal capture:

1. A provider hook writes or refreshes a bounded active-session lease after
   re-proving provider, session, project, and worktree ownership.
2. `post-commit` invokes a fixed, installer-owned launcher with only an opaque
   installation-receipt ID.
3. The handler resolves the common repository, current worktree, and full
   `HEAD` against that receipt using a sanitized Git subprocess environment.
4. It ignores repositories/worktrees that are not currently bound in Mallard.
5. It appends exactly one self-contained boundary event containing the commit
   OID, hook event ID, matching lease IDs and revisions as observed at that
   boundary, and no transcript/source content. It does not synchronously
   update both an inbox and a link index.
6. Background replay turns the captured evidence into a proposed, unresolved,
   or proven association. The app resolves proposals through explicit user
   selection or a separately reviewed auto-link policy.
7. `post-rewrite` drains stdin to EOF, validates a bounded set of old/new
   object-ID edges, and appends one rewrite observation. Background replay
   handles the duplicate `post-commit` plus `post-rewrite` sequence from amend
   idempotently and preserves a bounded many-to-many rewrite graph.

Unsupported rewrite tools can leave an unreachable link. `doctor` reports it;
Mallard never guesses a replacement from timestamp alone.

## 7. Domain and persistence contracts

Names are illustrative; implementation should preserve existing naming and
module boundaries.

### 7.1 Validated identifiers

- `CheckpointId`: opaque, bounded, URL-safe, and generated by Mallard.
- `GitObjectId`: an explicit `sha1 | sha256` algorithm plus a full normalized
  lowercase 40-hex or 64-hex object ID.
- `GitCommonRepositoryId`: machine-local digest derived from canonical Git
  identity, never a portable path.
- `GitWorktreeId`: machine-local identifier distinct for each linked worktree.

Never accept an arbitrary filesystem path as a hook-state destination.

### 7.2 Authoritative local link

```text
CheckpointLink {
  checkpoint_id
  local_project_id
  bundle_id
  replica_id
  binding_revision
  provider_profile_id
  provider
  thread_id
  git_common_repository_id
  git_worktree_id
  commit_oid
  observed_at
  provider_watermark?
  evidence: manual | provider_commit | policy_linked
  store_revision
}
```

The idempotency key includes the binding revision, provider/profile, logical
thread, common repository, worktree, and commit OID. Derive `resource_id` from
`provider + thread_id` and validate it wherever a DTO carries both; do not keep
two independently trusted identities. This disambiguates Mallard's supported
case where one checkout has multiple active projects through different
provider profiles.

Subject and commit time are presentation data re-read from the local Git object
where possible, not trusted portable assertions. A commit does not contain a
branch. Store the bounded symbolic ref observed at capture as evidence, and
derive current branch placement from reachability on the selected rail; a
commit may be reachable from multiple branches or none.

`manual` and `provider_commit` project as `exact`. `policy_linked` remains
visibly policy-derived and does not silently become exact. A Git-supplied
rewrite preserves the original evidence and records bounded rewrite lineage;
the rewrite itself is not a new attribution source.

### 7.3 Supporting local documents

- Active session leases: keyed by provider profile, installation receipt,
  binding revision, and worktree; short TTL and bounded per provider.
- Checkpoint ledger: the single authoritative append-only history of boundary,
  manual decision, policy decision, unlink, and many-to-many rewrite-edge
  events. Each record has an event ID, schema, size cap, checksum, and captured
  lease revisions. A partial final record is recoverable without inventing an
  association.
- Checkpoint read index: a disposable, revisioned projection rebuilt from the
  ledger. The hook never mutates this index.
- Hook installation receipt: installation ID, common-repository identity,
  provider/profile and binding identity, method, owned config keys or wrapper
  digests, selected Git executable/version, CLI path, and worktree references.
- Installation transaction: staged intended writes, before/after digests, and
  completion markers for deterministic rollback or roll-forward when Git
  config, wrapper/provider config, and the receipt cannot change atomically.

All live documents use schema 1 independently, enforce record/segment/total
size limits, use no-follow regular-file checks, reject unsafe hard links, use
mode `0600` where applicable, and recover deterministically. Reaching an
authoritative ledger limit stops new capture and surfaces `capacity_exceeded`
through `doctor`; it never silently evicts a live link. Sealed-segment
compaction is a later format migration, not an excuse for the first release to
split authority between an inbox and a compacted index.

### 7.4 Portable projection

Do not make links portable in the first implementation slice. Before the
portable phase, choose one of these through a contract test spike:

1. use an existing forward-compatible envelope if one can preserve old
   canonical scope digests exactly; or
2. perform a clean bundle/schema cutover that adds a bounded checkpoint map to
   provider scopes.

Do not add a field to `BundleScope` under schema 1 merely because
`#[serde(default)]` accepts missing JSON. Older and newer digest views would
not hash the same contract.

Portable metadata may contain logical thread/resource identity, commit object
ID, opaque checkpoint ID, observed time, and evidence kind. It must exclude:

- absolute checkout or provider-home paths;
- profile IDs and machine-local project IDs;
- native hook commands or executable paths;
- installation/trust receipts;
- prompts, transcript text, tool payloads, staged paths, and environment data;
- remote URLs or credentials; and
- automatic instructions to install or run a hook.

Bundle SHA-256 proves object integrity, not who asserted a session/commit
relationship. On Pull, remote `manual` or `provider_commit` evidence becomes
`portable_asserted` until Mallard verifies the local commit and the normal Pull
plan explicitly accepts it. Portable evidence never acquires local hook trust
or silently renders as locally exact.

## 8. CLI and installation design

### 8.1 Public commands

```text
mallard-cli git-hooks plan enable --project <local-project-id>
mallard-cli git-hooks plan repair --project <local-project-id>
mallard-cli git-hooks plan disable --project <local-project-id>
mallard-cli git-hooks apply --plan-id <id> --digest <sha256>
mallard-cli git-hooks status [--project <local-project-id>]
mallard-cli git-hooks doctor [--project <local-project-id>]

mallard-cli checkpoints list [--project <local-project-id>]
mallard-cli checkpoints plan link --project <local-project-id> \
  --resource-id <resource-id> --commit <oid>
mallard-cli checkpoints plan unlink --project <local-project-id> \
  --checkpoint <checkpoint-id>
mallard-cli checkpoints apply --plan-id <id> --digest <sha256>
```

Public commands preserve the current global schema-1 `--json` envelope and
stable sanitized exit codes. Destructive or executable changes are
revision/digest checked.

Every persisted plan carries `plan_id`, action, repository/provider/binding
identity, observed configuration and store revisions, expected target digests,
creation/expiry times, and its canonical digest. Apply loads and revalidates
that exact plan. A digest cannot be replayed for another action, repository,
project, provider profile, or revision.

Manual link planning resolves the supplied object through the receipt's
sanitized Git adapter, proves that it is a commit in the bound common
repository, expands it to the full storage-format OID, records the SHA-1 or
SHA-256 algorithm, and re-proves resource ownership. A 40- or 64-character
string alone is not sufficient evidence.

Internal commands are stable implementation details used only by
installer-owned entries:

```text
mallard-cli hook git post-commit --receipt <opaque-installation-id>
mallard-cli hook git post-rewrite --receipt <opaque-installation-id>
mallard-cli hook provider <provider> <event> --receipt <opaque-provider-id>
```

Internal commands accept only the documented Git/provider event shape and
resolve all state from the real Mallard home. They do not expose a general
`--state-dir`, `--command`, or arbitrary output-path option.
The opaque receipt ID is a routing handle, not authorization; the handler still
re-proves repository, worktree, binding, profile, and session ownership.

### 8.2 Install plan contents

The plan shows:

- repository and all affected worktrees in local display form;
- the absolute Git executable/version used for the probe and whether known GUI
  clients may use a different embedded Git;
- the Git hook capability probe result, including supported `git hook list`
  exit `0`/`1` versus unsupported-command exit `129`;
- effective configuration across system, global, local, worktree, and included
  files;
- effective `core.hooksPath`, including config origin and scope, `/dev/null` or
  other disabling targets;
- event/friendly-name `enabled` values, friendly-name collisions, and
  worktree-specific overrides;
- selected native-config or legacy method;
- every exact config key/file that would change;
- existing hook managers, programs, symlinks, or special-file conflicts;
- whether the target is shared outside the repository;
- packaged CLI path and executable check; and
- provider-hook capability, user-level config/trust steps, or the reason a
  project-file-only provider is refused.

The apply command re-proves every expectation before mutation.

Enable, repair, and disable use the installation transaction journal because
Git config, a legacy launcher, provider config, and the Mallard receipt cannot
be replaced atomically as one unit. Startup, `status`, and `doctor` detect a
partial install and deterministically roll forward or roll back only when all
recorded before/after digests still match; otherwise they stop and request a
new reviewed repair plan.

### 8.3 Git 2.54 and newer

Prefer named, repository-local config entries such as Mallard-owned
`hook.<name>.command` and repeated `hook.<name>.event` values. Use the actual
feature probe, not just the Git version. Record the exact key/value/origin in
the receipt and remove only a matching Mallard-owned entry.

For the probe, `git hook list <event>` exit `0` or `1` means the subcommand is
supported (with or without a match); legacy unknown-subcommand exit `129`
means it is unavailable. Record the absolute Git executable and version.
`doctor` warns if they change or if a GUI/IDE is known to use an older embedded
Git, because that client can ignore named hooks even though the installer probe
succeeded.

### 8.4 Legacy Git fallback

Resolve paths with:

- `git rev-parse --show-toplevel`;
- `git rev-parse --git-dir`;
- `git rev-parse --git-common-dir`; and
- `git rev-parse --git-path hooks`.

For the first release, write an atomic, marked Mallard wrapper only when the
target hook is absent and the effective directory is an ordinary repository
hook directory. If an existing hook, custom/shared `core.hooksPath`, hook
manager, symlink, or special file owns the target, refuse automatic mutation
and show a reviewed manual integration command.

Do not change `core.hooksPath`; doing so can shadow every existing hook. Do not
silently adopt Entire's backup-and-chain behavior. Chaining may be added later
only with explicit approval and tests that preserve arguments, stdin, order,
output, mode, and exit status. `post-rewrite` specifically requires exact stdin
replay.

Uninstall removes only the exact owned config or wrapper digest from the
receipt. If either the installed entry or an expected backup has changed,
leave it intact and require review.

### 8.5 Executable path

Do not point Git directly at a movable application bundle. Each installed entry
uses a minimal, per-receipt fail-open guard and a stable launcher under
`~/.mallard/hook-launchers/`. The launcher supervises the packaged absolute CLI
path already exposed by the app, which avoids relying on the sparse `PATH` of a
GUI Git client. It contains no credential, provider path, or arbitrary command.

The guard returns zero if the launcher is missing. The launcher returns zero
after a missing/moved CLI, nonzero exit, crash, signal, or externally enforced
timeout, while preserving a bounded diagnostic for `doctor`. A reviewed repair
updates the receipt and launcher atomically after an app move/update. Phase 0
must prove the launcher and watchdog on macOS, Windows, and Linux; copying the
signed CLI sidecar is not assumed and would require a separate signing,
quarantine, and upgrade analysis.

### 8.6 Git subprocess boundary

Hook processes inherit ambient Git variables that can redirect repository,
object, index, and configuration resolution. Structured argv and `git -C` are
not sufficient by themselves.

After using the opaque receipt to select an expected worktree, invoke the
receipt's absolute Git executable with an explicit canonical worktree and a
sanitized environment. Remove ambient `GIT_DIR`, `GIT_WORK_TREE`,
`GIT_CONFIG_*`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`,
`GIT_ALTERNATE_OBJECT_DIRECTORIES`, and related repository overrides; set a
fixed locale, disable the pager and optional locks, and bound stdin, stdout,
stderr, execution time, and child processes. Never bypass Git's ownership or
`safe.directory` checks. Compare the independently resolved common repository
and worktree identities with the receipt before recording an event.

## 9. Read path and UI integration

### Activity

- Load the bounded checkpoint index alongside existing Git history.
- Model checkpoint occurrences separately from the session's current tail, and
  attach every reachable exact occurrence to its commit.
- Preserve many-to-many mapping and resource-scoped selection behavior.
- For a session with activity after its last checkpoint watermark, retain an
  explicit uncheckpointed tail; do not hide it merely because an earlier exact
  link exists. The historical successor rule can place a visibly inferred
  tail when its evidence applies.
- Run the current whole-session successor inference only when there are no
  exact occurrences or checkpoint watermarks to segment the session.
- Label unresolved and unreachable links without placing them on an unrelated
  commit.

### Session detail

- Change checkpoint support from one optional checkpoint to a bounded
  collection.
- Build a composite merged index instead of appending one inferred event after
  the provider page. Order by placement timestamp, event-kind precedence,
  provider ordinal, and checkpoint ID; use `observed_at`/provider watermark for
  placement because Git author/committer dates are user-controlled.
- Apply event filters before pagination, assign stable composite ordinals and
  anchors, and preserve cursor behavior when several checkpoints share a
  timestamp.
- Include checkpoint-store revision/digest in the timeline snapshot ID so a
  new hook event invalidates stale cursors.
- Preserve `exact`, `inferred`, `lower_bound`, and `unsupported` quality.
- Re-read commit presentation from the repository; a missing commit remains a
  portable/unreachable marker rather than trusted display text.

### Settings and status

Add an `Exact Git checkpoints` surface only after the CLI contract is stable.
It shows Disabled, Healthy, Needs repair, Conflict, or Provider unsupported,
plus the installation method and bound worktrees. Enable, repair, and disable
always display the same reviewed plan used by the CLI.

The running app should watch only the checkpoint store revision and invalidate
Activity/session detail. It must not consume raw hook payloads in the frontend.

### Push/Pull review

When portability lands, checkpoint links appear as their own metadata rows and
are selectable. A Push that does not select them leaves them local. Pull plans
show additions, remaps, conflicts, and removals without installing hooks.

Deletion remains explicit. Removing a portable link does not delete a Git
commit or conversation transcript.

## 10. Delivery phases

### Phase 0 — Contract and capability spike

- Pin and test supported Codex and Claude lifecycle payloads.
- Build a provider capability matrix and prove project/session/worktree
  ownership.
- Probe named Git hooks on 2.54+ and legacy behavior on the packaged platform
  Git versions, including exit `0`/`1`/`129`, the current Apple Git 2.50 path,
  and a newer installer Git paired with an older GUI Git.
- Prototype ordinary repository, linked worktree, submodule, and custom
  `core.hooksPath` resolution.
- Prove the stable fail-open launcher and external watchdog on every packaged
  platform.
- Benchmark and crash-test the authoritative append ledger under concurrent
  hook/CLI/app access.
- Decide the future portable schema approach without changing schema 1.

**Exit gate:** no automatic exact attribution unless at least one supported
provider supplies a stable session identity plus a directly matchable commit
event, and Mallard can re-prove worktree ownership and the full object ID.
Otherwise proceed with lease-assisted candidate capture and manual linking.

### Phase 1 — Local domain, persistence, and manual links

- Add validated checkpoint, Git object, lease, and installation types.
- Add the authoritative ledger, disposable index, limits, replay, and partial
  tail recovery.
- Add the shared `mallard-application` service.
- Add action-bound `checkpoints` plans/apply and JSON contracts with Git,
  binding, and resource proofs.
- Integrate exact-first, many-checkpoint Activity and session detail reads.
- Add revision invalidation and app watcher support.
- Integrate host rotation and profile/binding/project/worktree removal.

This phase makes exact links useful without installing executable hooks.

### Phase 2 — Git hook lifecycle

- Add action-bound plan/apply plus status/doctor CLI commands for enable,
  repair, and disable.
- Implement native Git 2.54 configured hooks.
- Implement the conservative empty-target legacy wrapper.
- Add journaled installation recovery and effective-config diagnostics.
- Add sanitized, receipt-bound Git execution, one-event `post-commit` capture,
  and EOF-draining, many-to-many `post-rewrite` observation.
- Package and verify the internal hook commands on macOS and Windows.
- Add the desktop reviewed settings surface.

### Phase 3 — Provider lifecycle adapters

- Merge supported Codex and Claude lifecycle hooks without overwriting user
  configuration.
- Maintain bounded, expiring active-session leases.
- Turn unique leases into reviewable candidates; promote only confirmed or
  directly proven provider-commit associations to exact.
- Keep ambiguous boundaries unresolved and label an explicitly enabled
  auto-link policy as `policy_linked` evidence.
- Add conflict/repair UX and provider-version diagnostics.

Phase 2 and Phase 3 may be developed together, but exact automatic attribution
is not released until both are proven for a provider.

### Phase 4 — Review-first portability

- Finalize the portable contract or clean schema cutover.
- Add capture, reconciliation, plan, apply, receipt, and tombstone behavior.
- Add selectable Push/Pull review rows.
- Prove that hooks, paths, trust, and executable data never enter a bundle.

### Phase 5 — Optional extensions

- Explicit durable commit-trailer mode with `prepare-commit-msg`,
  `commit-msg`, and rewrite tests.
- Advanced reviewed legacy-hook chaining.
- Advisory checkout/merge invalidation hooks.
- A GitHub App/webhook for remote reconciliation or PR annotation, never local
  session capture.

## 11. Verification matrix

All tests use temporary homes, temporary provider trees, and temporary Git
repositories. They never read real `~/.codex`, `~/.claude`, or `~/.mallard`.

### Git behavior

- normal commit, empty-message failure, amend, rebase, merge, squash,
  cherry-pick, detached HEAD, unborn branch, and unreachable commit;
- `post-rewrite` old/new pairs, malformed and oversized stdin, partial write,
  EOF draining after the processing limit, amend's duplicate post-event pair,
  many-to-one mappings, duplicate delivery, and a rewrite tool that does not
  invoke the hook;
- SHA-1 and SHA-256 repositories;
- ordinary clone, linked and moved worktrees, submodule, and bare-repository
  rejection; and
- simultaneous events from two worktrees.

### Installation safety

- Git 2.50 legacy behavior, Git 2.54 named hooks, later Git, probe exits
  `0`/`1`/`129`, and a newer installer Git with an older GUI Git;
- default, relative, absolute, local, global, and shared `core.hooksPath`;
- included config, worktree overrides, disabled event/friendly hooks,
  friendly-name collision, and `core.hooksPath=/dev/null`;
- absent, executable, non-executable, modified, symlinked, hard-linked, and
  special-file hooks;
- Husky, Lefthook, pre-commit, Entire, and unknown managers;
- spaces, quotes, Unicode, shell metacharacters, and Windows drive paths;
- sparse GUI `PATH`, moved/missing CLI, read-only target, interrupted apply,
  partial-install roll-forward/rollback, repeated enable, repair, and disable;
- launcher missing, child nonzero, crash, signal, and hang past the external
  watchdog deadline; and
- uninstall leaving every pre-existing hook untouched.

### Attribution and data behavior

- no lease, one lease, stale lease, multiple sessions, multiple providers,
  human commit during a lease, and manual resolution;
- manual link to an abbreviated ID, blob/tree/tag, foreign repository commit,
  unreachable commit, and valid full SHA-1/SHA-256 commit;
- one session to many commits and one commit to many sessions;
- idempotent repeated hook delivery and rewrite remapping;
- exact occurrences taking precedence without erasing an uncheckpointed
  session tail;
- cursor invalidation after a new checkpoint; and
- same-timestamp composite ordering and pagination;
- multi-branch/unreachable placement using observed ref plus reachability; and
- unreachable/portable assertions represented without invented local trust.

### Safety and privacy

- no network call, prompt, transcript parse, or source read in a Git hook;
- no absolute path, native provider-home path, remote URL, credential, prompt,
  transcript, tool payload, staged path, or environment dump in portable data,
  logs, JSON output, or errors;
- spoofed cwd/repository, unsafe Git config, path traversal, symlink swap,
  poisoned `GIT_*` environment, unsafe hard link, partial ledger tail, full
  ledger capacity, concurrent append/replay, and index rebuild;
- host rotation and provider/profile/binding/project/worktree removal with an
  installed common-repository hook;
- pulled data never installing, trusting, or invoking a hook; and
- hook failure returning promptly without blocking the user's Git operation.

### Performance budgets

- provider lease update: target p95 under 50 ms;
- `post-commit`/`post-rewrite` validation plus one ledger append: target p95
  under 100 ms;
- no network I/O and no full transcript parse on either path; and
- external launcher-enforced hard timeout with later repair through `doctor`.

## 12. Acceptance criteria

The feature is ready when:

1. every hook or attribution mutation requires an action-, target-, revision-,
   expiry-, and digest-bound reviewed plan;
2. Mallard coexists with named Git hooks and refuses unsafe legacy conflicts;
3. a supported provider session can acquire multiple exact commit checkpoints;
4. ambiguous or absent provider evidence never becomes an exact association;
5. amend/rebase mappings are idempotently repaired when Git supplies them;
6. the stable launcher makes commits and pushes succeed if Mallard is missing,
   moved, crashed, nonzero, or hung;
7. no capture hook performs network, transcript, source, bundle, or Git-ref
   writes;
8. Activity and session detail prefer exact links and clearly retain inferred
   history;
9. each Git hook appends at most one authoritative ledger event and background
   replay builds every disposable read index;
10. uninstall removes only unchanged Mallard-owned material and shared
    worktree references survive correctly; and
11. no checkpoint or provider-hook data leaves the machine without a normal
    explicit Mallard Push review.

## 13. Primary sources

### Entire

- [Entire overview](https://docs.entire.io/overview)
- [Entire quickstart](https://docs.entire.io/quickstart)
- [Entire CLI FAQ](https://docs.entire.io/cli/faq)
- [Entire security model](https://docs.entire.io/security)
- [Entire Git-hook installer at the inspected commit](https://github.com/entireio/cli/blob/caa0c9be90261fb2b64bf6cfc7147ee3981494db/cmd/entire/cli/strategy/hooks.go)
- [Entire Codex-hook adapter at the inspected commit](https://github.com/entireio/cli/blob/caa0c9be90261fb2b64bf6cfc7147ee3981494db/cmd/entire/cli/agent/codex/hooks.go)
- [Entire session/checkpoint architecture at the inspected commit](https://github.com/entireio/cli/blob/caa0c9be90261fb2b64bf6cfc7147ee3981494db/docs/architecture/sessions-and-checkpoints.md)
- [Entire per-checkpoint ref architecture at the inspected commit](https://github.com/entireio/cli/blob/caa0c9be90261fb2b64bf6cfc7147ee3981494db/docs/architecture/ref-checkpoint-backend.md)
- [Entire known limitations at the inspected commit](https://github.com/entireio/cli/blob/caa0c9be90261fb2b64bf6cfc7147ee3981494db/docs/KNOWN_LIMITATIONS.md)

### Git

- [Git hooks](https://git-scm.com/docs/githooks)
- [Git 2.54 hook command](https://git-scm.com/docs/git-hook/2.54.0)
- [Git 2.54 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.54.0.adoc)
- [`core.hooksPath`](https://git-scm.com/docs/git-config/2.54.0.html#Documentation/git-config.txt-corehooksPath)
- [Git worktrees](https://git-scm.com/docs/git-worktree)
- [Git path resolution](https://git-scm.com/docs/git-rev-parse)

### Codex

- [Official Codex hooks documentation](https://developers.openai.com/codex/config-advanced#hooks)

### GitHub

- [About GitHub webhooks](https://docs.github.com/en/webhooks/about-webhooks)
- [Webhook types and permissions](https://docs.github.com/en/webhooks/types-of-webhooks)
- [Validating webhook deliveries](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries)
- [GitHub webhook best practices](https://docs.github.com/en/webhooks/using-webhooks/best-practices-for-using-webhooks)

# Exact-home ownership: each directory is a project

2026-08-22. **IMPLEMENTED same day — see §9 for deltas.** Clean cutover:
no migration, no compatibility paths; `~/.mallard` and the cloud buckets
are wiped before the next live test.

## 1. The review finding, evaluated

The finding ("working directories are counted as roots") is **upheld as a
defect and rejected as a remedy**.

Upheld: `inspect_codex_structural_cwds`
(src-tauri/src/project_sync/commands.rs:16536) pools the authoritative
`session_meta.cwd` and every `turn_context.cwd`
(crates/mallard-infrastructure/src/codex_rollout.rs:155–185) into one set,
filters out paths under the *current* root, and the audit calls
`len() > 1` "multiple project roots" (commands.rs:15897). A moved project
whose transcript once ran a turn in `<old-root>/frontend` is blocked as
"ambiguous" even though the one-root mapper provably repairs it
(commands.rs:19548 exercises exactly that).

Rejected: the finding's implied fix — collapse nested stale cwds under
the outermost old root by containment — imports the containment model we
already ruled out for Claude ("a subdir launch is another project",
PLAN_CLAUDE_IN_ROOT_PATH_REMAP §11). It also leaves the *worse* sibling
defect standing:

- A thread whose home **equals** the current root but which once ran a
  turn in a foreign directory yields `stale_cwds == {foreign}`, i.e.
  exactly one stale cwd — so the audit offers a **repair that would
  rewrite the foreign directory's prefix into the project root** across
  the whole transcript (commands.rs:15906 uses that foreign cwd as
  `recorded_cwd`). One confirm click from corrupting a healthy thread.
- The screenshot's immediate cause — false thread ownership — comes from
  the same root error at the assignment layer: `path_belongs_to_project`
  (commands.rs:16490) is containment, and threads are claimed through
  turn-level evidence (`thread-writable-roots`, `thread-workspace-root-hints`,
  commands.rs:15674–15701), so a thread homed in `/A` that merely held a
  writable grant or hint into `/B` becomes `/B`'s thread — and then,
  via the defect above, gets a repair offer mapping `/A → /B`.

All three defects are one error: **structure (identity, roots) is being
derived from a bag of turn-level evidence instead of the provider's
single authoritative record.**

## 2. The law

> **Each absolute directory is its own project. A session belongs to
> exactly one directory: its authoritative home. Ownership is exact
> canonical path equality — containment never confers ownership. A
> subdirectory of a project root is a different project.**

- Authoritative home, per provider:
  - **Codex**: the active thread's authoritative `session_meta.cwd` —
    the first `session_meta`, already exposed as
    `CodexRolloutStructure::current.cwd` (codex_rollout.rs:45).
  - **Claude**: the launch cwd of the active leaf session ID (the in-flight
    uncommitted work: `identity.leaf_identity()` in providers/claude/mod.rs —
    copied history is ignored, a new session ID resets the home, and later
    per-turn cwd changes within that ID do not move it).
- Everything else is **content, not identity**: `turn_context.cwd`,
  sandbox `writable_roots`, desktop workspace hints, copied ancestor
  `session_meta` records from forks. Content under the owning home
  remaps with the home (mapper is uniform); content outside it is
  preserved byte-identical (the standing HARD INVARIANT,
  PLAN_CODEX_IN_CWD_PATH_REMAP §3.1).

This is already the engine's law — `verify_provider_payload` requires
`analysis.source_cwd` to equal the binding root or its canonical alias
exactly (conversation_materialization.rs:1299), and the Claude push gate
keeps exact launch-cwd equality. Discovery and the Codex Desktop audit are
the two outliers still on containment; this plan brings them in line.
(Today's incoherence is real: a subdir Codex session is discoverable and
pushable, but no machine can ever materialize it — the pull gate rejects
its cwd. Ownership it can't round-trip is not ownership.)

## 3. Part A — discovery: exact match, `relative_cwd` deleted

`owned_relative_cwd` (provider_capture.rs:1784) becomes an exact-match
predicate: canonicalize the session's home, own it iff it equals the
canonical project root. Consequences, applied as deletions in the same
change:

- **`relative_cwd` is deleted end-to-end.** It can only ever be `"."`
  now. Falls out: `DiscoveredSession.relative_cwd` (traits.rs:103),
  `DiscoveredProviderState.relative_cwd` (traits.rs:133),
  `ResourceDescriptor.relative_cwd` + its validate arm
  (mallard-domain/src/lib.rs:880, 908–922), both `types.ts` DTO fields
  (143, 979), the activity presentation plumbing
  (project_activity.rs:98, 301, …), capture plumbing
  (provider_capture.rs:62, 543–546, 770–786, 881, 1144, 1194),
  `relative_cwd_key` (provider_capture.rs:2332), the `state_target`
  `relative_cwd` parameter (traits.rs:569, bundle_engine.rs:3266,
  claude/mod.rs:191) and claude/materialization.rs's subdir arm.
  No shipped UI reads `relative_cwd` (only mockups set it to null).
- **`excluded_roots` (session side) is deleted as vacuous.** Its whole
  job was to stop a parent claiming a nested bound project's sessions
  under containment; with exact equality a nested project's sessions
  have a different home by construction. Falls out:
  `SessionDiscoveryContext.excluded_roots` (traits.rs:97),
  `CaptureRequest.excluded_project_roots`, the nested-root validation in
  `discover_full` (provider_capture.rs:1006–1019), and the supplier
  (commands.rs:13317–13326). It also dissolves a live inconsistency: the
  watch service passes no exclusions today
  (session_watch_service.rs:1035), so watch-path and command-path
  ownership disagreed for nested projects.
- **Content capture keeps its nested exclusion.** Files are not
  sessions: a nested bound project's *tree* must still be excluded from
  the parent's project-content walk. The supplier's nested-binding list
  moves into `excluded_content_roots`
  (discover_generic_project_content already merges that list,
  provider_capture.rs:1227–1236); file behavior is unchanged.
- **Claude provider state (memory)** is discovered only from the
  adapter's canonical bucket for the binding root (the munged root
  directory), still gated on that bucket containing at least one owned
  session (munging is lossy; the session proof stays). Foreign buckets
  never contribute state, even if a resumed-into-root session lives in
  one — their memory belongs to their own directory-project.
  `state_component` becomes the constant `root`
  (resource id `claude:memory:root`), and the logical suffix drops the
  key segment: `memory/<rel>` instead of `memory/_root/<rel>`.

## 4. Part B — Codex Desktop audit & repair: nominate by evidence, classify by home

The audit (commands.rs:15526) keeps its shape — desktop state nominates
threads, the audit classifies them — but the two roles are separated
cleanly. `path_belongs_to_project` is replaced by an exact canonical
equality helper.

**Nomination** (from `.codex-global-state.json`):

| Channel | Rule | Class |
|---|---|---|
| `local-projects.rootPaths` | any root == binding root (canonical, exact) | selects desktop projects |
| `thread-project-assignments` | `projectKind == local` and `projectId` in a selected project | **assigned** |
| `thread-writable-roots`, `thread-workspace-root-hints` | any listed path == binding root (exact) | **aux** |
| legacy cwd fallback (assignment with no `projectId`) | **deleted** | — |

Real-state scan (this machine, types only): 302/302 assignments carry
`projectId` (290 local, 12 remote) — the legacy fallback matches nothing
and goes. 55 threads are writable-roots-only and 16 hints-only, so the
aux channels stay as nominators; 87/274 writable-roots lists have more
than one entry, which is why aux evidence must never *classify*. No
desktop project has multiple rootPaths and none nests under another.

**Classification** — one rule per nominated thread, from the rollout's
authoritative home (`analyze_codex_rollout_structure(...).current.cwd`,
canonicalized when it still exists):

| Authoritative home | Assigned | Aux only |
|---|---|---|
| == binding root | matching | matching |
| == another live registered Codex Desktop root | visible as unavailable for sync; never captured or repaired | silently skipped — not this project's thread |
| strictly under binding root, without a registered-root match | warning: separate directory project — bind that directory to manage it; never repaired (no flattening) | same warning |
| anywhere else | **repair issue**, `recorded_cwd` = home | silently skipped — not this project's thread |

An explicitly assigned thread homed at a different live registered Desktop
root remains visible in Activity with an unavailable-for-sync status, but it
never enters capture or repair. The observed assigned conflict lands in that
second row; an auxiliary-only foreign-home thread remains silently skipped. A
healthy thread with a foreign turn cwd lands in "matching". The
nested-frontend transcript from the finding is a plain single-home repair.

**Deleted:** `inspect_codex_structural_cwds`, the multi-root blocker and
its message (commands.rs:15892–15905), `path_belongs_to_project`.
`structural_cwds` itself stays — the repair invariant check still maps
every structural cwd through the codec pair (commands.rs:16624) and that
remains correct: in-root cwds move, out-of-root cwds must be identical.

**Repair** is mechanically unchanged (§6.5 stands): `mapper_old` is
built from `recorded_cwd`, which is now the authoritative home rather
than an inferred lone stale cwd; the pinned wire-equality gate, the
unpinned collision pre-scan, backups, and checkpoint rebase all stay.
The pre-repair TOCTOU recheck (commands.rs:16059–16068) re-reads the
home and requires it to still equal `issue.recorded_cwd`. Cross-directory
fork ancestry (an ancestor `session_meta` recording a different home) is
out-of-root content: preserved byte-identical, exactly per the hard
invariant — such rollouts are now repairable instead of blocked.

**Documented residual:** a thread that was a *subdir launch under a
moved parent* and is desktop-assigned to the parent project repairs to
the parent's new root — the old root/suffix split is unrecoverable from
the evidence (only the home string survives; the desktop project records
only its current root). If that distinction matters, reassign the thread
in Codex Desktop before repairing. Rare, user-confirmed, and every
affected rollout is backed up first.

## 5. What does not change

Wire form and path-map grammar, folds/tails/checkpoints and the
classifier, `BUNDLE_SCHEMA = 1` and every schema constant, credential
redaction, the storage catalog (cloud listing is storage-scoped, not
cwd-scoped), the repair math and gates of §6.5, and the ProjectSync
confirm copy — "Only conversations explicitly assigned to this project by
Codex Desktop will be changed" becomes *more* true, not less.

UI surface only shrinks: one blocker string ("records N distinct
out-of-project roots") disappears, warnings gain the
subdir-separate-project case, `ready`/`can_repair` semantics are
untouched (commands.rs:16438). No new error states to classify.

## 6. Tests

Retargets (assertions move, cover is conserved —
[[dead-code-deletion-verify-test-halves]]):

- `nested_fork_path_repair_remaps_matcher_scope_under_one_old_root`
  (commands.rs:19548): the `stale.len() == 2` audit half becomes a
  home-based assertion (home = old root ⇒ one repair issue); the
  multi-root fixture (active `/tmp/old-active`, parent `/tmp/old-parent`)
  now asserts repairability with the ancestor home preserved verbatim,
  instead of asserting the blocker.
- The ownership fixture (commands.rs:19227): `stale_thread` moves from
  the writable-roots channel to an explicit `thread-project-assignments`
  entry (that is what repair now requires); `matching_thread` stays on
  writable-roots and must still count as matching; `unrelated_thread`
  must be invisible.
- `provider_capture.rs:2579`: `apps/web` sessions are no longer owned —
  assert exclusion instead.
- The uncommitted Claude leaf-identity tests (fixtures use
  `project/original`, `project/resumed` subdir cwds) retarget their cwds
  to the root / disjoint dirs; the distinct-identity point survives
  unchanged.
- codex_rollout.rs structural tests keep `structural_cwds` as-is.

New: aux-nominated foreign-home thread is skipped; an assigned foreign home
that is another live Desktop project remains visible but unavailable for sync;
other assigned foreign homes yield a repair; assigned subdir-home threads
yield the warning and no issue; matching threads with a foreign
`turn_context.cwd` are matching and get no repair offer.

## 7. Sequencing & cutover

The uncommitted Claude leaf-cwd identity work is a prerequisite (the law
cites leaf identity); land it first or in the same change. Then one
cutover commit for Parts A+B: no migration, no dual readers, schemas
untouched; wipe `~/.mallard` and the cloud buckets before live retest
(ResourceDescriptor and state logical suffixes change shape).

Full bar to hold green: domain + infra + src-tauri + frontend suites,
clippy `-D warnings`, fmt.

## 8. Decisions taken in this design

- **D1 — aux channels survive as nominators only** (never classify,
  never trigger repair). Alternative — deleting them — would hide the 55
  writable-only + 16 hints-only real threads from "matching" counts for
  no safety gain.
- **D2 — legacy cwd assignment fallback deleted** (corpus: zero
  `projectId`-less assignments). The camelCase/snake_case *spelling*
  tolerance for desktop keys stays — that is external-format tolerance,
  not a compatibility path of ours.
- **D3 — Claude memory comes from the canonical root bucket only**,
  still session-proven; foreign buckets never contribute state.
- **D4 — subdir-home threads are never repaired into the parent root**,
  regardless of channel: flattening a child directory project into its
  parent is precisely what the model forbids.

## 9. Implementation deltas (2026-08-22)

Landed with the full bar green (domain 51, infra 316, src-tauri 235 +
storage_catalog_e2e, frontend 236, clippy `-D warnings`, fmt, tsc).
Differences from the letter of the design:

- **Aux threads are fully silent unless classifiable.** Beyond never
  blocking, aux-only nominations also skip the oversized/missing-rollout
  warnings (those messages speak of desktop assignment); the
  subdirectory warning fires for both channels as tabled. The oversized
  test fixture moved to the assignment channel accordingly.
- **Claude root bucket is found by self-consistency, not by munging the
  binding root**: a bucket qualifies when it holds an owned session
  whose directory name equals the munge of that session's *own recorded
  home*. This keeps firmlink spellings (`/tmp` vs `/private/tmp`)
  working without consulting the binding string. Multiple qualifying
  spellings emit one memory resource (first in order) plus a warning
  naming the ignored buckets.
- Memory identity landed as `claude:memory:root` with logical suffixes
  `memory/<rel>` (the `_root/` key segment is gone);
  `relative_cwd_key` deleted.
- New helpers: `session_home_is_project_root` (provider_capture, shared
  by all three adapters), `codex_authoritative_home` and
  `path_is_project_root` (commands.rs). `owned_relative_cwd`,
  `inspect_codex_structural_cwds`, `path_belongs_to_project`, the
  multi-root blocker, and the legacy cwd assignment fallback are gone.
- The capture supplier folds nested active binding roots into
  `excluded_content_roots`; `CaptureRequest.excluded_project_roots` and
  `SessionDiscoveryContext.excluded_roots` no longer exist.
- The repair TOCTOU recheck compares the re-read authoritative home to
  `issue.recorded_cwd` exactly.
- The prerequisite Claude leaf-cwd work was already in the working tree
  and shipped in the same change; its discovery-level test was
  retargeted to prove ownership follows the resumed leaf home while
  copied foreign history neither grants nor blocks it.

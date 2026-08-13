# Plan: Codex session watcher acceleration — backend follow-up

- Status: Deferred follow-up
- Date: August 10, 2026
- Depends on:
  [PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
- Scope: Advisory local Codex session watching, bounded dirty-state caching,
  refresh scheduling, and backend tests
- Excluded: Conflict resolution, standalone reconstruction semantics, schema
  definitions, cloud metadata, Push/Pull authorization, React UI, and any
  legacy migration or translation path
- Cutover contract: Optional runtime acceleration built only on the active
  schema-1 product; checkpoint-only execution remains the complete behavior
  and no legacy reader or writer is supported

This follow-up is intentionally not part of the minimal conversation conflict
resolver cutover. Implement it only after the portable-tail checkpoint and
incremental append-range refresh path are stable. Removing or disabling every
watcher component must leave refresh, Push, Pull, recovery, and cloud results
unchanged.

## 1. Objective

Use filesystem events to schedule local Codex refresh work earlier and avoid
reparsing unchanged rollout metadata. The main benefit is lower latency for
no-op refreshes, passive status updates, and large profiles with many unchanged
rollouts.

This is a second-level optimization. `FixedPaginatedTail` partitions and local
rolling scan cursors remove the dominant inherited-prefix rehashing cost.
`WholeThread` retains its authoritative full comparison. Watching does not
eliminate:

- bounded reconciliation of known root/history-source paths and unknown
  candidates needed to discover new roots during an authoritative refresh or
  Push/Pull planning pass; recognized subagent paths are ignored;
- authoritative reads and hashes of every pending append range;
- standalone reconstruction and complete immutable-object hashing;
- remote manifest/head reads; or
- Push compare-and-swap and Pull transaction verification.

Conflict identity is root-only. Subagent rollout bytes are incidental files in
the complete conversation snapshot: they may be copied and SHA-256-verified
when a root action already consumes bytes, but they never participate in the
root tail value or the `P`/`L`/`C` relation. The watcher must preserve
that boundary. It may make later artifact capture cheaper; it must not invent
an independent subagent synchronization path.

Filesystem events may be coalesced, dropped, reordered, delayed, or lost while
Mallard is stopped or the machine sleeps. A missing event is never proof that a
rollout is unchanged.

## 2. Safety and clean-cutover contract

The watcher is an advisory scheduler and cache accelerator, not a correctness
witness.

1. Portable tail layouts, checkpoint generations, physical scan cursors,
   source-range pins, apply-generated complete output hashes, journal phases,
   cloud revisions, and CAS remain the only durable authority.
2. Watcher state is bounded and process-local. It never enters the active
   machine-local schema 1, portable bundle schema 1, reviewed plan, journal,
   receipt, or cloud object. The follow-up adds no alternate schema version or
   legacy interpretation.
3. Startup failure, backend failure, event loss, overflow, restart, and
   sleep/wake fall back to the checkpoint-driven path without changing a
   relation, action, receipt, or recovery decision.
4. Absolute provider-home and rollout paths never enter frontend events,
   activity logs, portable data, or diagnostics exposed to users.
5. The callback performs no parsing, hashing, cloud access, blocking I/O, or
   sync decision. It only scopes events and updates a bounded queue/generation.
6. No filesystem event authorizes Push/Pull or promotes an accepted cloud
   version, cumulative tail state, physical cursor, or checkpoint generation.
7. A known subagent-only event is ignored. It never changes watcher state,
   dirties the root conflict relation, or creates an advisory attachment
   state. A later root-driven byte-consuming Push/Pull discovers and captures
   the complete bounded subagent closure authoritatively without relying on a
   watcher hint.

## 3. Required foundation

The follow-up consumes, but does not redefine, these contracts from the main
backend plan:

- destination-scoped accepted complete conversation versions and portable
  cumulative tail states;
- `ConversationCheckpoint` and its generation;
- provider-local physical-source descriptors;
- `FollowEof` and `FixedHistoryCutoff` logical segments;
- newline-safe append-range discovery, raw pins, and canonical tail
  accumulator extension;
- the root-only portable partition kind/start and observation;
- the single `Detached` physical-mapping invalidation after provider-alias
  changes; and
- transaction receipts that can seed a newly materialized checkpoint directly.

If any of these are unavailable, keep the watcher disabled rather than adding
a second refresh or conflict-classification implementation.

`FollowEof` and `FixedHistoryCutoff` are machine-local physical source-segment
roles for reconstructing the root. All `history_base` ancestor segments
flatten into the one root transcript and the portable tail has one fixed root
start. A watcher event may dirty several physical consumers, but it must never
create a second conflict value. Subagent rollouts remain incidental
snapshot files and have no portable-tail entry.

## 4. Watch scope

Maintain one process-lifetime registry entry per exact canonical bound
`CODEX_HOME`/provider profile. Multiple projects and storage links using that
profile share the operating-system watcher but retain independent reconciliation
state.

Do not recursively watch all of `~/.codex`. Watch only:

- `<CODEX_HOME>/sessions/` recursively when it exists;
- `<CODEX_HOME>/archived_sessions/` recursively when it exists; and
- the canonical `CODEX_HOME` non-recursively, filtered to those directory names
  and `session_index.jsonl`, to observe tree creation/replacement and atomic
  index replacement.

Never follow symlinks. Continue to require canonical containment, no-follow
regular-file checks, and bounded path components before an event becomes a
candidate rollout. A `session_index.jsonl` event invalidates only presentation
or title caches; it does not dirty canonical conversation content.

Mallard already depends on `notify = "8.2"`. Model the process lifetime and
managed-state ownership on `project_sync/config_watcher.rs`. The removed fork
watcher may be read only as an implementation reference for bounded queue and
`need_rescan` handling. Do not import its executor, reuse its persisted state,
or expose a legacy execution path.

## 5. Runtime model

Keep only bounded in-memory state:

```rust
struct CodexWatchEpoch {
    provider_profile_id: LocalProviderProfileId,
    epoch_id: u64,
    event_generation: u64,
    reliability: WatchReliability,
    path_generations: BoundedMap<PathBuf, u64>,
    dirty_thread_keys: BoundedSet<ThreadCheckpointKey>,
    inventory_generation: u64,
}

enum WatchReliability {
    Reliable,
    ReconcileRequired(WatchInvalidationReason),
    Disabled,
}

struct WatchCleanStamp {
    epoch_id: u64,
    checkpoint_generation: u64,
    reconciled_path_generations: BoundedMap<PathBuf, u64>,
    reconciled_inventory_generation: u64,
}
```

Names are illustrative; use the repository's validated bounded types.

Store each `WatchCleanStamp` only in the runtime registry, keyed by the exact
local consumer scope `(provider profile, project, storage link, scope, thread)`.
It is not a field of the durable `ConversationCheckpoint`.

Change generations are monotonic and are never globally consumed. Each
destination-scoped observation has its own `WatchCleanStamp`. Refreshing
storage A cannot make storage B clean when both reference the same rollout at
different cursors.

Maintain a one-to-many reverse index from a physical rollout path to every
physical source-segment consumer owned by a canonical root. A paginated
ancestor may serve many roots, and one physical rollout may participate in
more than one segment role. An event therefore dirties all dependent root
observations. A
`FixedHistoryCutoff` consumer may later prove that an append occurred strictly
beyond its cutoff and remain logically unchanged; a `FollowEof` consumer stays
dirty.

Do not track known subagent paths or generations. Their events are outside the
watcher's conflict-acceleration model. If a root action later requires full
snapshot bytes, the ordinary bounded capture freshly discovers all retained
subagents and verifies the final artifact hashes.

Bound all of the following:

- watched homes;
- consumers per home;
- callback queue depth and event paths;
- path-generation entries;
- dirty-thread keys;
- reverse-index fanout;
- clean-stamp entries; and
- retained generations.

Failure to record or retain any required entry atomically marks the complete
profile `ReconcileRequired`. If the watcher cannot be rearmed within bounds,
mark it `Disabled`. Never evict evidence and leave any consumer partially
clean. Increment generations with checked arithmetic; exhaustion starts a
fresh `ReconcileRequired` epoch and never wraps a generation.

## 6. Watcher lifecycle

### Registry creation and profile changes

Every new registry entry starts `ReconcileRequired`. Watcher creation failure
selects `Disabled` immediately. Reconcile registry entries whenever configured
provider profiles or project bindings change; stop and drop entries with no
live consumers. If an existing profile identifier resolves to a different
canonical `CODEX_HOME`, tear down the old watch and start a fresh
`ReconcileRequired` epoch for the new exact path.

### Watch-before-scan baseline

Avoid the initial scan race:

1. Create the root watcher and attach every existing narrow session-tree watch.
2. Record the current event generation.
3. Run ordinary bounded discovery and checkpoint refresh.
4. Drain/coalesce events and verify that the generation stayed stable.
5. Publish a clean watermark only when the scan and generation agree.
6. Retry a bounded number of times; otherwise remain
   `ReconcileRequired`/`Busy`.

A reliable epoch is never persisted across process restart.

### Loss, replacement, and rearming

Queue overflow, `Event::need_rescan()`, backend error/disconnect, excessive
paths, watched-tree removal or replacement, app restart, and sleep/resume mark
the complete profile `ReconcileRequired`.

Add an explicit backend lifecycle input for sleep/resume when the platform
provides one. Otherwise use a conservative application-resume/focus or
heartbeat-gap signal to request reconciliation; do not claim that watcher
delivery alone detects time spent suspended. Correctness remains independent
because every authoritative pass performs the reconciliation below.

After backend error/disconnect, tree replacement, or failed rearm, tear down
the obsolete operating-system watcher. Recreate the narrow watches, rebuild the
reverse index, assign a fresh epoch, and repeat watch-before-scan. Do not return
to `Reliable` while using an obsolete watch handle or before the new baseline
finishes.

## 7. Event classification and background work

Event handling is conservative:

- modify, create, or rename-to of a known root/history-source rollout
  increments its path generation and dirties every dependent root
  observation;
- removal, rename-from, or source-identity change invalidates affected physical
  observations instead of representing an empty pending range;
- an unknown rollout or directory event increments `inventory_generation` and
  requires bounded provider discovery only to determine whether it is a new
  root or a history source segment; a discovered subagent is discarded from
  watcher state;
- create, modify, remove, or rename of a known subagent rollout is ignored;
- a root append that mentions a subagent is still a root append, so the root
  file event dirties the root. Canonical refresh may then prove the append is
  excluded subagent bookkeeping and leave the root tail unchanged; it
  does not register or inventory the child;
- `session_index.jsonl` updates affect presentation state only; and
- incomplete trailing JSONL remains dirty until a complete newline-safe
  boundary exists.

Debounce and coalesce per profile. Background work may pre-read a candidate
append range and cache its raw hash/canonical result, but it does not advance a
durable physical cursor, accepted cloud version, or accepted cumulative tail
state. Do not issue a cloud request for every local
filesystem event; normal refresh/status cadence performs remote comparison.
Background preprocessing must call the existing bounded, no-follow,
newline-safe incremental reader rather than introduce a second parser. Discard
its result if the watcher epoch, path generation, checkpoint generation, or
host-operation generation changes before publication; it must never delay
Push/Pull quiescence.

## 8. Authoritative operation integration

Every user-invoked refresh and Push/Pull planning pass still performs bounded
root inventory reconciliation: check known root/history-source paths, detect
unknown/moved/removed root candidates, and perform cheap filesystem metadata
checks. Recognized subagent paths are ignored; unknown candidates that parse
as subagents are discarded immediately. This does not re-read every rollout's
structural `session_meta`. The watcher avoids JSONL metadata reparsing and
prioritizes root-content work; it does not establish root-inventory
completeness. Full subagent closure discovery happens only after a root
relation has independently selected a byte-consuming operation.

For each destination-scoped root observation:

1. If it is dirty/unreconciled or has a pending append range, run the existing
   incremental refresh path. Every authoritative refresh or Push/Pull planning
   pass re-reads and hashes that pending range because watcher silence plus
   unchanged length cannot exclude a same-length edit.
2. If the raw range hash and exact boundaries reproduce a background
   observation, reuse its canonical parse/rolling result. Otherwise parse and
   canonicalize the range again.
3. Allow the no-content-read fast path only for `FixedPaginatedTail` when the
   pending append range is empty and every observed end equals its saved scan
   cursor. `WholeThread` still reruns its complete canonical comparison. Probe
   every physical
   source for canonical path/file identity, regular-file kind, expected length,
   and safe boundary. Any mismatch takes the ordinary incremental or
   invalidation path.
4. Advance only that destination's `WatchCleanStamp`, conditional on the
   watcher epoch/path generations and checkpoint generation observed at the
   start. A concurrent event or checkpoint promotion leaves it dirty.
5. Continue remote head/manifest refresh and call the same shared
   `P`/`L`/`C` tail-value classifier used by Refresh, Push, and Pull;
   local watcher state says nothing about cloud movement and never creates an
   operation-specific comparison path.

Subagent watcher generations are deliberately absent from these five steps.
They do not cause a root reclassification or a byte-consuming action. When a
root action independently requires complete snapshot bytes, the ordinary
capture/materialization path enumerates, bounds, copies, and SHA-256-verifies
the root's retained subagent rollouts. A manifest-only `Sync` or
`LocalOutstanding` action neither publishes nor installs a subagent-only
change. Mallard stores no accepted subagent tail and offers no
independent subagent reconciliation.

The optimization is intentionally asymmetric:

| Operation | Watcher assistance | Verification retained |
| --- | --- | --- |
| Refresh/status | Identify dirty roots, precompute advisory append-range observations, and reuse `FixedPaginatedTail` cumulative state when no append is pending after metadata probes. | Rehash every pending fixed-tail range and every `WholeThread`, reconcile unreliable epochs, read the remote manifest, and never promote accepted state. Subagent events do not change status. |
| Push review | Skip fixed-tail content reads when no append is pending and detect a relevant post-plan event early. | Rehash and pin every pending fixed-tail range and every `WholeThread` range, read the remote manifest, and call the shared accepted-pin/local/cloud-tail classifier. |
| Push apply | Fail early when the epoch changed. | Stream the exact planned ranges once, verifying identity/EOF/raw pins while reconstructing; hash/verify each output once, check the head, and CAS. |
| Pull review | Skip fixed-tail content reads for roots with no pending append and prioritize dirty roots. | Rehash pending fixed-tail ranges and every `WholeThread`; call the shared accepted-pin/local/cloud-tail classifier. Materializing review pins immutable manifest/object metadata and fixed IDs/paths only; it downloads, reconstructs, hashes, and stages nothing. |
| Pull apply | Report local interference early. | Preserve the journal-free atomic SyncConfig write for zero-target actions. For materialization, download/verify pinned objects once, reconstruct/reidentify the one remote target once while validating typed invariants and serializing/hashing emitted bytes once, journal the apply-generated stage, then preserve provider quiescence, no-clobber writes, retained-source pins, recovery, and checkpoint finalization. Divergent C reuses P-local. |

An early “watch epoch changed since Push review” check may use only an
ephemeral in-process `plan_id -> watch snapshot` entry. Do not serialize watcher
fields into a reviewed plan. After restart, eviction, or a missing hint, skip
the early check and run the ordinary complete apply preflight.

### Mallard-generated writes

Materializing Pull apply knows the exact files and verified bytes it has just
generated. Seed the destination checkpoint directly from the transaction
receipt; do not wait for an event. Do not suppress self-generated events by path
or a time window. Keep the watcher stamp dirty until the journaled after-state
is durable and a post-write reconciliation observes stable path generations.
This prevents suppression of an external append racing with Mallard's write.

The same transaction may install subagent snapshot files because the selected
root action already consumes the complete artifact. Those writes require no
watcher update and create no accepted subagent state, extra checkpoint
relation, or follow-up subagent synchronization.

In Divergent Pull, reassigning the existing P-local binding from T to C does
not write provider files and therefore produces no watcher event. Only the one
new P-remote target is a Mallard-generated provider write.

When a materializing Pull changes a provider alias, apply the main plan's journaled
invalidation to every other destination checkpoint that references the old
alias. Set the one `Detached` physical-mapping state while preserving each
destination's independent accepted pin. Rebuild watcher reverse-index consumers
only after that durable mutation; watcher state does not replace it.

## 9. Ownership and modules

Add provider-format logic under:

```text
crates/mallard-infrastructure/src/providers/codex/watch.rs
```

This module owns pure event scoping/classification, bounded generation
accumulators, and dirty-snapshot/acknowledgment types. It does not own Tauri
handles, parse complete conversation graphs, or make sync decisions.

Add process-lifetime integration under:

```text
src-tauri/src/project_sync/codex_session_watcher.rs
```

This module owns `notify` watchers, debouncing, registry/profile lifecycle, and
background work scheduling. `src-tauri/src/lib.rs` keeps the registry alive as
managed state. Existing refresh/Push/Pull orchestration consumes an internal
snapshot and conditionally updates clean stamps.

Keep watcher DTOs out of `mallard-domain`. If the frontend receives passive
status invalidation, expose at most an opaque provider-profile identifier and
generation—never absolute paths or transcript content.

## 10. Implementation sequence

1. Land and stabilize the checkpoint-only conflict-resolver plan, including
   incremental range tests and generation-CAS behavior.
2. Record checkpoint-only refresh/Push/Pull measurements for unchanged root,
   root append, paginated ancestor, and incidental subagent-inventory cases.
3. Implement pure event scoping, bounded generations, reverse-index fanout,
   overflow behavior, and deterministic unit tests.
4. Add the Tauri-managed per-profile registry and lifecycle reconciliation.
5. Implement watch-before-scan, fresh-epoch rearming, and checkpoint-only
   fallback.
6. Feed dirty candidates into the existing incremental refresh path and add
   conditional destination-specific clean stamps.
7. Add passive background preprocessing and the no-pending-append metadata fast
   path without changing authoritative operation results.
8. Integrate early Push/Pull staleness and safe handling of Mallard-generated
   writes.
9. Run race/fault tests and compare all watcher-enabled outcomes against
   checkpoint-only execution.
10. Re-measure performance and keep the feature only where it materially
    improves latency or foreground I/O.

## 11. Backend test plan

All tests use temporary Codex, Mallard, project, and storage roots. They never
watch a real user home.

### Lifecycle and event tests

- watcher arming precedes baseline discovery, and an event during baseline
  prevents a false clean watermark;
- append, create, rename-to/from, removal, sessions-to-archive movement,
  session-root creation, and custom/multiple `CODEX_HOME` profiles produce the
  expected scoped generations;
- `session_index.jsonl` changes invalidate only presentation/title state;
- unknown rollout creation dirties inventory and requires discovery/edge
  validation;
- incomplete trailing JSONL remains dirty until a complete safe boundary;
- debounce, coalescing, and reordering never lose dirtiness;
- queue/path/reverse-map/clean-stamp overflow, `need_rescan`, backend error,
  disconnect, root replacement, restart, and sleep/wake force profile-wide
  reconciliation with no partially clean consumer; and
- disconnect/root replacement tears down the obsolete watcher and cannot
  produce `Reliable` until a new epoch and baseline complete.

### Shared-graph and destination tests

- one physical ancestor used by multiple roots dirties every dependent
  checkpoint;
- adding or invalidating P0/P1-style physical segment consumers changes
  watcher dirtiness only; it does not add a conflict input or change any root
  tail layout;
- storage A and B keep independent reconciled generations, so refreshing A
  never consumes B's evidence;
- appends beyond a `FixedHistoryCutoff` can reconcile as irrelevant for that
  segment while a `FollowEof` consumer remains dirty;
- a physical source used in several root segment roles keeps distinct
  consumers;
- different local/cloud subagent rollout bytes with identical root tails leave
  `P`, `L`, `C`, and the resulting `Sync` relation unchanged;
- a known subagent-only event changes no watcher or sync state and never
  creates an independent plan action or a durable accepted subagent
  tail value; and
- provider-alias replacement invalidates every affected durable checkpoint
  before rebuilding runtime reverse indexes.

### Correctness-equivalence and race tests

- a clean hint still performs no-follow metadata/cursor checks;
- simulated missed append, truncation, rename, symlink replacement, or file
  identity change enters ordinary invalidation;
- a watcher-clean pending append range is authoritatively re-read, and a missed
  same-length mutation changes its raw pin;
- only a `FixedPaginatedTail` with no pending append can take the metadata-only
  content fast path; its cumulative portable tail may still be nonempty, while
  `WholeThread` remains a complete comparison;
- a newer event or checkpoint promotion prevents a stale clean-stamp update;
- a `FollowEof` event after Push planning reports early staleness, but no event
  ever skips Push apply's exact range/EOF/output/CAS verification;
- Pull receipts seed checkpoints directly, self-generated events remain dirty
  until stable post-commit reconciliation, and an external racing append is not
  suppressed;
- a byte-consuming root Push/Pull includes and SHA-256-verifies retained
  subagent snapshot files, while a manifest-only root action ignores a
  subagent-only difference;
- watcher-disabled and forced-loss execution produce the same relations,
  plans, receipts, cloud heads, local bindings, and recovery fixed points as
  watcher-enabled execution; and
- frontend events and logs contain no provider-home path or transcript content.

### Performance tests

Measure at least:

- an unchanged large profile;
- one appended root among many unchanged roots;
- one append to a shared paginated ancestor;
- one new or extended incidental subagent snapshot, proving zero root
  reclassification work;
- watcher-disabled fallback; and
- overflow followed by full reconciliation.

Report directory entries enumerated, metadata records reparsed, transcript
bytes read/hashed, canonical records parsed, foreground wall time, and
background work. Do not claim that watcher assistance removes mandatory
publication or materialization I/O.

## 12. Acceptance criteria

1. Watcher-enabled and checkpoint-only execution produce identical sync
   relations, plans, receipts, cloud heads, local bindings, and recovery
   outcomes.
2. No event, clean stamp, or background cache can authorize Push/Pull, promote
   trusted state, replace a source/output hash, or replace CAS.
3. A healthy reconciled epoch can reuse `FixedPaginatedTail` cumulative state
   when no append is pending after cheap no-follow metadata probes; every
   pending fixed-tail range and every `WholeThread` remains authoritatively
   re-read and hashed before use.
4. Event loss, overflow, restart, sleep/wake, and backend failure converge to
   bounded checkpoint reconciliation without a partially clean profile.
5. Shared physical sources and multiple storage destinations never consume one
   another's dirty evidence.
6. Pull/materialization does not wait for watcher delivery and cannot suppress
   a racing external append.
7. Watcher state remains bounded, process-local, provider-path private, and
   absent from every durable or portable schema.
8. Subagent rollout state is never a conflict input: equal root tails classify
   `Sync` despite different subagent bytes, and there is no subagent-only sync
   relation or action. Subagents travel only inside an already byte-consuming
   complete root snapshot.
9. Disabling or removing the watcher changes no schema-1 document and leaves
   the checkpoint-only product fully supported; there is no migration or
   compatibility path.
10. Performance measurements show a material improvement in at least no-op
   refresh or sparse-change foreground work before the follow-up ships.

## 13. Verification commands

```text
cargo fmt --all -- --check
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Run watcher integration tests only against explicitly created temporary
provider homes.

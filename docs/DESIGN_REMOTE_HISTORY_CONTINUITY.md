# Design: Remote history continuity

- Status: Design history. The continuity states are current; their
  classification is superseded by section 9 of
  [DESIGN_CAS_FREE_STORAGE_REFACTOR.md](DESIGN_CAS_FREE_STORAGE_REFACTOR.md),
  and their gates, their banners and the explicit-unlink rule are superseded by
  [PLAN_CLEARED_STORAGE_PUSH_AGAIN.md](PLAN_CLEARED_STORAGE_PUSH_AGAIN.md)
- Last reviewed: September 20, 2026

> **Superseded classification.** Since the CAS-free storage cutover there is
> no stored head and no probe read. Continuity is decided by identity and
> ancestry over the selected chain of publication records, never by revision
> order and never by a store read of the base manifest — `Advanced` means the
> base is an ancestor of the selected publication, and `Replaced` means it is
> on no selected chain (a cleared or recreated stream; a push that is
> outranked fails or is rolled back, and never reads `Replaced`). Read rows 1
> to 8, the probe, and its cache rules as design history.

> **Superseded gates and recovery.** `RemoteMissing` and `Replaced` no longer
> block a Push, and nothing asks for an unlink, a relink or a reset. A sync
> record that names a publication off the selected chain is stale: it is read
> as absent and never deleted at a refresh. A cleared storage is pushed again;
> a recreated one is pulled first only when another computer wrote into an
> area this computer writes. The Push states `remote_history_missing` and
> `remote_history_replaced`, the scope state `replaced`, the "Storage history
> changed" banners and their Unlink verb are removed. One standing note
> replaces them: "Another computer cleared <storage>. Push again." or
> "… Pull, then push again." Read the gates, the banners and the
> explicit-unlink rule below as design history.

- Scope: repository-level continuity classification between a replica's
  recorded sync base and the observed bundle head, computed in the shared
  plan phase and surfaced by Refresh, Pull review, and Push review
- Compatibility: clean cutover; no migration or legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner
  will clear `~/.mallard` and the test cloud buckets before acceptance testing
- Related:
  - [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md) — this design
    extends its shared plan step 1; workload and commit are unchanged
  - [DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md](DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md)
    — the per-thread table is unchanged; this is its repository-level analog
  - [DESIGN_FAST_PUSH_PULL.md](DESIGN_FAST_PUSH_PULL.md) — D5 cache
    interaction (the probe must bypass it) and D6 batching (the probe joins
    an existing fetch round)
  - [DESIGN_STREAMED_OBJECT_DOWNLOADS.md](DESIGN_STREAMED_OBJECT_DOWNLOADS.md)
    — the probe's required manifest-size coordinate and staged read contract

## Motivating incident

Two replicas, one storage, and a wiped repository directory:

1. Replica A opens Push review: head absent, base recorded →
   `RemoteHistoryMissing`, and the UI shows the "Remote history is missing —
   unlink and link again" banner. Unlink removes A's base
   ([commands.rs:1386](../src-tauri/src/project_sync/commands.rs#L1386));
   after relink A classifies fresh and the push **recreates the repository
   under the same `bundle_id`** — the bundle id belongs to the project
   registration, not the link, so relinking reuses it.
2. Replica B still holds base `revision 1` with the old manifest digest.
   The head now reads `revision 1` with a new digest. Today's classifier
   ([classify_push_destination](../src-tauri/src/project_sync/commands.rs#L5476))
   has no replaced-history state, so its catch-all arm reports
   `RemoteAdvanced` — "remote bundle advanced since this replica's base" —
   and the review UI renders a banner only for `remote_history_missing`
   ([PushResourceWorkspace.tsx:402](../src/components/project-sync/PushResourceWorkspace.tsx#L402)),
   so B first learns anything from a push-preflight failure that
   misdescribes a wipe as a routine advance.

Row 5 of the table below classifies B's case as `Replaced` with **zero
additional requests**, and every direction now surfaces it.

## Decision

The shared plan phase classifies remote history continuity from the
observed head, the recorded base, and — in exactly one arm — a store-read
probe of the base manifest. Store reads use exact size and content-digest
coordinates; no clock, timestamp, or cache participates in a verdict.

| # | condition | continuity |
| - | --- | --- |
| 1 | no head, no base | `Fresh` |
| 2 | no head, base recorded | `RemoteMissing` |
| 3 | head present, no base | `Unreviewed` |
| 4 | revisions equal, digests equal | `InSync` |
| 5 | revisions equal, digests differ | `Replaced` |
| 6 | head revision < base revision | `Replaced` |
| 7 | head revision > base revision, probe hits | `Advanced` |
| 8 | head revision > base revision, probe misses or size/hash-mismatches | `Replaced` |

**The probe** fetches `_manifests/{base.revision}-{base.commit_id}.json`
**from the store — never from the object cache** — and stages it against
`base.manifest_size` plus `base.manifest_sha256` before parsing. It is
consulted only in rows 7–8.

**Placement.** The base manifest key is known from local state before any
remote call, and the head answers the probe in the steady state:

- head digest equals the base digest → the head just *named* our base
  manifest as current; rows 4 applies and no probe request exists.
- head moved → the probe GET joins the same concurrent round as the head
  manifest fetch. One parallel round of two GETs replaces today's serial
  two, so replaced-history detection adds zero round trips anywhere.

## Definitions

- **Base** — the per-storage `RecipeBase` on the project
  (`recipe_bases[storage_id]`,
  [domain.rs:782](../src-tauri/src/project_sync/domain.rs#L782)): revision,
  exact manifest size, manifest digest, commit ID. Written by every push receipt
  ([commands.rs:7051](../src-tauri/src/project_sync/commands.rs#L7051)) and
  every pull apply
  ([commands.rs:8169](../src-tauri/src/project_sync/commands.rs#L8169));
  removed by unlink. `commit_id` becomes **required** — both writers already
  populate it; `validate()` enforces it; the unprobeable edge ceases to
  exist. `manifest_size` is likewise required and nonzero; there is no
  fallback inference for pre-cutover bases.
- **Head** — the mutable bundle head, read live on every operation, never
  cached (fast doc rule, unchanged).
- **Probe** — the row-7/8 store read defined above.

## Soundness

- **Rows 4 and 7** — the store demonstrably holds the exact manifest this
  replica recorded: row 4 because the inspected head names our digest, row
  7 because the staged bytes reproduce the recorded size and hash. The
  witness is our own recorded coordinate, so no trust is placed in response
  length metadata.
- **Row 5** — the commit CAS linearizes publishes and every publish
  increments the revision, so one lineage commits a given revision
  exactly once. Two different manifests at one revision cannot share a
  lineage.
- **Row 6** — revisions are monotonic in a lineage; a regressed head is
  not a descendant of the base.
- **Row 8** — manifests are immutable and nothing in the store deletes
  them (no delete API exists); absence of the base manifest proves the
  repository does not retain this replica's history.
- **The cache-bypass rule** is the remote-presence corollary of "a cache
  is never a safety witness": the base manifest usually sits in the local
  object cache, and a local hit proves nothing about a wiped remote. Probe
  reads go to the store; the response may populate the cache, never be
  served from it.
- **Known proxy edge.** A force-reset that rewrote history but *retained*
  old files leaves the base manifest present: rows 5–6 still classify
  `Replaced` (their witnesses are terminal and a surviving file must not
  upgrade them), but row 7 would read that edge as `Advanced`. The
  misreading degrades to the safe remedy — pull, protected by pins and
  apply-time verification — never to loss. Exact lineage membership via
  the commit-record ancestry chain is recorded as a TODO in *Considered
  and deferred*, with the worked rsync-restore example.

## What each direction does with the verdict

- **Refresh** — continuity becomes part of the storage-level status:
  advisory, blocks nothing, and explains why threads suddenly reclassify
  after a wipe-and-recreate.
- **Pull review** — `Replaced` shows an informational banner ("adopting a
  replaced history; local content is protected by pins and backups") and
  the pull proceeds: pull *is* the remedy, and its receipts rewrite the
  base, returning continuity to `InSync`.
- **Push review** — `Replaced` joins `RemoteMissing` as a blocking banner
  with the same shape as the existing missing-history warning: what
  happened, that content existing only in the old repository is not
  recoverable from here, and two actions — "Pull and review" (primary) and
  "Unlink storage…". `Advanced` and `Unreviewed` keep their requires-pull
  gate but now render at review time; today they surface only as a
  preflight failure toast. The push-side `BindingChanged` overlay is
  policy, not continuity, and keeps its current precedence.
- **Push preflight** — same classification re-run, message updated to
  match, and the failure log line gains base and head revision + digest
  (today's log carries no numbers).

## Failure handling

- **Probe transport error** — continuity is `Unknown`: refresh surfaces a
  storage error chip; push fails closed with a storage error exactly as a
  manifest fetch failure does today. An unreachable store is never
  interpreted as `Advanced` or `Replaced`.
- **Head fetch failure** — unchanged from today.
- A `Replaced` verdict never mutates anything by itself; it is display and
  gating only. The base is rewritten exclusively by a completed pull or
  push receipt, as today.

## Work budget

| state | remote reads today | after |
| --- | --- | --- |
| steady (head unmoved) | 1 head GET | 1 head GET — row 4/5/6 need no probe |
| head moved, genuine advance | head GET + serial manifest GET | head GET + one parallel round {head manifest, base probe} |
| wiped and recreated (rows 5–6) | same as advance, misdiagnosed | same reads, correct verdict, zero probes |

## Cutover

1. Make `RecipeBase.commit_id` and `RecipeBase.manifest_size` required;
   enforce the ID and nonzero size in `validate()`.
2. Extract `classify_remote_continuity(head, base)` and run it in the
   shared plan phase; Push maps continuity onto its destination status and
   keeps the `BindingChanged` overlay. Delete the ambiguous catch-all arm —
   per the no-parallel-paths policy, `RemoteAdvanced` can no longer be
   produced without a row-7 witness.
3. Wire the probe into the moved-head fetch round with cache bypass.
4. Render a destination banner for every non-ready status in Push review;
   add the pull and refresh surfaces.
5. Update the preflight messages and the push failure log line.

No migration or fallback reader. Pre-cutover local state and storage are
cleared; every base written by the supported path is complete.

## Test plan

- One fixture per table row (1–8), asserting the exact continuity.
- **The incident regression, end to end** — replica A: missing → unlink →
  relink → push recreates; replica B: `Replaced` at review on all three
  surfaces, push blocked, pull heals the base, continuity returns
  `InSync`, push then succeeds.
- **Warm-cache wipe tripwire** — remote wiped while the local object cache
  still holds the base manifest: classification must be `Replaced`. This
  fails the moment a probe read is served from cache.
- **Work counts** (`CountingStore`) — steady state: one head GET, zero
  probes; row 5: zero probes; row 7/8: exactly one probe in the same round
  as the manifest fetch.
- **Probe error degradation** — a store that fails only the probe key
  yields `Unknown` and a storage error, not a verdict.
- **Probe coordinate checks** — short, overlong, and wrong-hash historical
  manifests miss; no body is exposed to the continuity classifier.
- **`BindingChanged` precedence** — a binding-revision mismatch still wins
  over continuity on the push side.
- **Pull heals** — after a pull from a replaced repository, the rewritten
  base classifies `InSync` and push is offered again.

## Considered and deferred

- **TODO — commit-record ancestry walk for the retained-files force-reset
  edge.** The one case the probe misreads. Worked example: a repository at
  revision 2 (lineage `c1 ← c2`; replica B based at `c2`, digest `M2`)
  is rewound by restoring a revision-1 snapshot **without deleting newer
  files** — `rsync -a` without `--delete` over the storage directory, or
  an S3 versioning rollback that removes only `_head`'s latest version.
  `_head` returns to revision 1 while `_manifests/2-c2.json` survives as
  an orphan. New pushes then grow a second lineage past the base's
  revision (`c1 ← d2 ← d3`, head at revision 3, and
  `_commits/2-d2.json` records `previous: c1`, not `c2`). B lands in the
  revision-ahead arm, probes `_manifests/2-c2.json`, finds its own
  recorded digest, and reads `Advanced` — though the head's ancestry
  (`d3 → d2 → c1`) never contains `c2`. Had the new lineage stopped at
  revision 2, row 5 would have caught it for free; the misread requires
  the rewrite to overtake the base's revision. Degradation is safe —
  pull is offered, pins and apply-time verification prevent loss, and
  abandoned-lineage content resurfaces as locally outstanding — but the
  diagnosis is wrong: B is told teammates advanced when history was
  rewound, gets no replaced-history warning, and may innocently re-push
  content the reset meant to purge (e.g. a leaked secret). Exact fix when
  this edge matters: `CommitRecord.previous_commit_id`
  ([bundle_engine.rs:808](../src-tauri/src/project_sync/bundle_engine.rs#L808))
  gives lineage membership by walking head back to the base's revision
  at O(chain) GETs — row 7 then requires *ancestry*, not *presence*.
- **Repository epoch nonce in the head** — a random id minted at repo
  creation would detect recreation with zero probe reads, but adds a
  schema field, cannot distinguish the retained-files reset either, and
  duplicates what the probe already proves from existing state.
- **Manifest pruning** — the row-8 witness rests on `_manifests/` never
  being pruned. If GC over manifests ever arrives, the probe needs a
  retention floor or the ancestry walk replaces it. Recorded here so that
  design cannot land without revisiting this one.
- **Pull-apply surfacing** — apply already fails closed via its pinned
  revision, commit ID, manifest size, and manifest digest checks; a
  continuity re-check there would duplicate an existing witness.

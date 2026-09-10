**Plan: Sync Codex subagents owned by a task**

Status: Implemented. Verification results and open validation limits are in
[the verification record](CODEX_OWNED_SUBAGENT_SYNC_VERIFICATION.md).
“Today” columns retain the pre-change design baseline.
Date: 2026-09-08.

Implemented settings amendment: [provider child-Push controls](PLAN_CHILD_SYNC_SETTINGS.md).
It uses the cloud child stamp for Compare and the local setting for Push.
Use that amendment for the current Push and Compare scope rules.

Mallard will include a task's owned Codex child rollouts in Compare, Push,
and Pull. The selected parent remains the sync unit. Children have no
separate task row, selection, cloud revision, binding, or conflict action.
A change confined to an included child can change the parent's sync result.
Apply keeps the existing failure behavior. Completed writes can remain after
an error. This feature adds no rollback for the parent or its children.

**Compatibility is a clean cutover.** Keep local state, manifests, and storage
at schema 1. Use one implementation of the new contract. Add no migration,
compatibility reader, feature flag, or legacy execution path. Required new
fields must be present; permitted empty or null values must be explicit.
Enforce presence for nullable fields too; a missing field is not null.
The user will remove Mallard local state and cloud data before retesting.
Do not add reset code or change native provider homes for the cutover.

This is the Codex child-sync design. It supersedes
[closure transport](PLAN_CODEX_SUBAGENT_CLOSURE_TRANSPORT.md),
[tree sync](PLAN_CODEX_SUBAGENT_TREE_SYNC.md), and the Codex transport and
verification scope of [the local mock plan](PLAN_LOCAL_MOCK_MULTI_SUBAGENT_E2E.md).
Claude direct-child support is defined separately in
[PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md](PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md).
That implementation also replaces the derived child evidence labels with one
neutral pair for both providers.
It keeps Codex's native parser contracts, main codecs, and descendant scope.
The missing-local-child recovery amendment below is implemented for both providers.

The earlier decision, “a missed subagent is acceptable; a delayed or blocked
user is not,” is replaced. Included children are required evidence. Missing
required evidence can block the affected parent. Known unsupported child
history instead permits main-only sync. These rules apply through the same
pipeline and do not preserve the earlier best-effort implementation.

**The terms below define the contract.**

| Term | Meaning |
| --- | --- |
| Rollout | A native JSONL file with task metadata, messages, and activity records. A child has its own file. Follow-up questions can append to that file. |
| Parent | The selected user task. An owning parent can also be an included child that creates a descendant. |
| Fork | A new task created from an earlier task's history. Copied messages do not transfer ownership of children. |
| Local state | Canonical content evidence in the current machine's native form. |
| Normalized state | Canonical content evidence after portable ID, path, and credential transformations. |
| Accepted pair | Local and normalized states saved together after cloud publication or local installation. A later metadata or receipt write can still fail. Refresh does not advance the pair. |
| Binding | Local identity and installation data for the parent and its installed child files. |
| Checkpoint | Accepted content evidence for one parent on one storage link. |
| Observation | Rebuildable local content and discovery cache data. |
| Manifest | The cloud task and file inventory. Each listed file refers to an immutable object verified by SHA-256. |
| Unavailable | The UI result for invalid or unprovable evidence, represented by Unknown in the comparator. |

**Ownership excludes inherited children.**

~~~text
P creates C                    C's spawning parent is P
|
+-- fork --> T                 T inherits P's history, not ownership of C
             |
             +-- creates D     D's spawning parent is T
                    |
                    +-- creates E
                               E's spawning parent is D

Sync P: P and its owned children, including C
Sync T: T + D + E
        A later update to C does not add C to T's sync set.
~~~

Selection and payload content have different purposes. Select only children
owned by the selected task or its admitted descendants. Keep the complete
resume history in each transported rollout, including copied context.
Do not trim the main or child payload to its own messages.

A structured activity reference and the child's authoritative metadata must
agree. The child metadata must contain the referenced native ID and
`source.subagent.thread_spawn.parent_thread_id` must identify the task that
supplied the reference. If top-level `parent_thread_id` is present, it must
agree. Use the child's first authoritative session metadata, not copied
ancestor metadata. Exclude valid references to another parent's child,
another user task, or a provider helper.

Read both native activity forms. `agent_path` can be a string or an array.
Codex 0.152.0 emitted strings in the native test. These examples show the relevant fields:

~~~json
{
  "type": "event_msg",
  "payload": {
    "type": "sub_agent_activity",
    "event_id": "start-D",
    "kind": "started",
    "agent_thread_id": "child-native-id",
    "agent_path": ["worker"],
    "occurred_at_ms": 1
  }
}
~~~

~~~json
{
  "type": "event_msg",
  "payload": {
    "type": "item_completed",
    "item": {
      "type": "SubAgentActivity",
      "id": "start-D",
      "kind": "started",
      "agent_thread_id": "child-native-id",
      "agent_path": "/root/worker"
    }
  }
}
~~~

`payload.event_id` and `payload.item.id` are the same creation ID. Preserve
its exact value through projection and native ID changes. The existing
[paginated projector](../crates/mallard-infrastructure/src/codex_paginated_projection.rs)
already maps these fields. It can retain the source record and emit its
projection; deduplicate those representations by owner and event ID.
Conflicting representations are invalid evidence.

The activity kinds are `started`, `interacted`, `interrupted`, and `completed`.
Only `started` defines a new child key. The `item_completed` envelope does
not imply activity kind `completed`. Correct the stale `spawned` test fixture
during implementation; do not add it as an alias. A legacy activity record
in a main task does not imply a legacy child payload.

**Discovery uses ownership and local installation cutoffs.**

1. For a native paginated main, scan its current physical segment. Do not
   open `history_base` ancestors solely to discover children.
2. For a native legacy-fork main, scan its local records, including copied
   history. Verify each referenced child's spawning parent. A child owned
   by an ancestor is excluded. No exact fork marker is needed for this test.
3. For a task installed by Pull, obtain known children from its binding map.
   Re-verify their IDs and owning parents. Discover new children only in
   records after that task's stored `installed_end` byte offset.
4. Resolve candidate UUIDs in the same bound Codex home through the existing
   provider inventory. Batch unresolved IDs into one request-scoped filename
   lookup. Read bounded child headers to verify ownership and source support.
5. Repeat for each admitted child. Deduplicate references and reject
   ownership conflicts, cycles, unsafe identifiers, and exceeded bounds.
   Fold content and discover descendants in the same required body read.

Legacy-fork mains with `forked_from_id` but no `history_base` are eligible
if their main content and all required ownership evidence validate. Do not
infer a fork boundary from metadata order or copied message text. This
feature does not require parsing `forked_from_ordinal_exclusive`.
If a referenced file is missing, ownership cannot be proved. That parent is
unavailable, even if the missing file could have been an inherited child.

A known unsupported owned child selects main-only scope for the entire
parent. Preserve the agreed source support rule:

| Evidence found during source discovery | Parent scope |
| --- | --- |
| Every owned child has supported paginated history and a valid native child boundary | Main and all owned children |
| Any owned child uses legacy or unsupported history mode | Main only; omit every child and descendant |
| Any owned paginated child has a missing or invalid native child history boundary | Main only; omit every child and descendant |
| A valid foreign or inherited child is unsupported | Exclude that reference; it does not restrict the parent's own tree |
| A required file is missing or unreadable, or ownership evidence conflicts | Unavailable |

A legacy child's missing `subagent_history_start_ordinal` does not prove
that it has no copied context. Do not substitute the position after its
metadata. Native child boundary validation is a source-format check; it is
not a new Mallard wire boundary or the test for ownership. Native metadata
needed for correct resume remains the Codex codec's responsibility.

Once an unsupported owned child is found, stop child work and discard the
provisional child set. Do not enumerate the omitted descendants or hash
remaining siblings. Main-only scope still requires a valid main rollout.
Omission does not delete native files or historical cloud objects.

Scope is a result of the current support check, retained in the temporary
review. It is not a stored mode on the cloud row, binding, or checkpoint.
An empty manifest child inventory means that the snapshot carries only the
main. It does not claim that no native child ever existed.

**Pull cutoffs stay local.** A cutoff is the byte length of the complete local
file emitted by Pull. It is not a message count or provider record ordinal.
Record it only after a successful installation. It excludes all imported
references from new-child discovery, including inherited and omitted ones.
Known installed children remain discoverable through the binding map.

~~~text
Installed main T
  [ imported records ................ ][ new native records ....... ]
  0                             installed_end                      EOF
  |                                      |
  +-- known children: binding map         +-- discover new children
      re-verify native ownership             verify native ownership

Each installed child has the same arrangement for its descendants.
~~~

Each child map value therefore needs its own cutoff beside its native UUID.
A main-only cutoff cannot locate new grandchildren in an installed child.
This adds one integer per installed child, in the same map. No separate
ownership graph or cursor database is needed.

The cutoff controls discovery only. Hash the complete required content.
Keep existing file-identity, append, and cursor validation. A truncated or
replaced file, invalid cutoff, or conflicting map is unavailable; never
silently restart discovery at zero. An observation cache can be rebuilt
from a valid binding. Loss of the binding is not a supported recovery path
for pulled copies. Obtain a fresh Pull; do not guess installation metadata.

**Child keys use creation IDs directly.** A direct child's key is its
`started` activity ID. A nested key is `<owner-key>/<creation-id>`.
The cloud parent task scopes the first segment. For example, D is `start-D`
and E is `start-D/start-E`. The key prefix supplies E's owning child key.

Validate each segment as nonempty ASCII letters, digits, `_`, or `-`, up to
the provider's existing 256-byte identifier limit. Also apply the existing
component and complete logical-path limits, including the file extension.
Reject unsafe or oversized values; do not hash, escape, or truncate them.
Preserve creation IDs when native task UUIDs change. Require a unique
creation event for each admitted child.

Use the existing parent file inventory with child logical paths such as
`state/conversations/codex/<cloud-task>/subagents/<child-key>.jsonl`.
During encoding, construct a temporary native-ID map for the main and
included children. During Pull, map portable references to the allocated
native IDs. Validate structural references in both directions.

References to inherited, foreign, or omitted children keep their source
UUIDs, as in today's main-only Pull. Do not allocate replacement UUIDs for
those references. Ownership checks and installed cutoffs prevent Mallard
from attaching them to the imported tree. Native resume and control behavior
must pass the shared-home isolation test before release.

**Each child has two content states and one existing object digest.**

~~~rust
struct SubagentContentState {
    message_count: u64,
    sha256: String,
}

struct AcceptedSubagentState {
    local: SubagentContentState,
    normalized: SubagentContentState,
}
~~~

Fold the whole canonical child conversation, including its copied context.
Use the same record admission, message ID, predecessor, and content rules
as the main. The algorithm is defined in
[conversation_tail.rs](../crates/mallard-domain/src/conversation_tail.rs) and
[conversation_codec.rs](../crates/mallard-infrastructure/src/conversation_codec.rs).
Do not create another hash algorithm or a combined tree hash. Preserve the
existing main codec, partition, start, and accepted-tail rules.

The cloud stores the child's normalized content state. Its existing file
entry separately stores the SHA-256 of the exact object bytes and their
size. The object digest proves transfer integrity; it is not comparison
evidence. After Pull, compute the local state from the emitted local file
and accept it with the verified normalized state from that snapshot.

**Metadata changes stay in the existing records.**

| Location | Change | Purpose |
| --- | --- | --- |
| Cloud `ConversationThread.files` | Add `SubagentTranscript { child_key, normalized }` to `ConversationFileRole` | List each included child and its normalized content evidence |
| Cloud `BundleScope.files` | Use the existing file entry for each child | Object key, exact SHA-256, size, and existing file metadata |
| Local `conversation_bindings.json` | Add required `installed_end` and `installed_subagents` fields to the parent binding | Identify local installed files and exclude their imported references from discovery |
| Local `sync_config.json` | Add required `accepted_subagents` to the parent's storage-link checkpoint | Accept the child's local and normalized states together |
| Local `observation_cache.json` | Extend the parent observation with child states, membership, and existing cursor data | Reuse validated observations and process appends |
| Temporary Push review | Retain child membership, source support result, and displayed sizes | Define the reviewed selection without capturing upload bytes |

`ConversationFileRole` currently has `Transcript`, `ProviderIndexFragment`,
and `Sidecar`. `SubagentTranscript` is a new variant. Add no child role to
ordinary user-task discovery and no independent cloud child conversation.
The cloud parent row, HEAD, and object format need no new mode or boundary.

The two binding fields have the following shape:

~~~rust
installed_end: Option<u64>, // null for a native source not installed by Pull
installed_subagents: BTreeMap<SubagentKey, InstalledSubagent>,

struct InstalledSubagent {
    provider_thread_id: String,
    installed_end: u64,
}

// On the existing per-storage-link parent checkpoint:
accepted_subagents: BTreeMap<SubagentKey, AcceptedSubagentState>,
~~~

A native source starts with explicit null and an empty installation map.
A successful Pull of K children stores K installation map entries and K
accepted child entries. A successful main-only sync stores an empty
accepted child map.
The installation map and accepted map have different purposes: a later
main-only Push does not erase the local installation metadata.

Store no `subagent_sync_mode`, `accepted_subagent_sync_mode`, or Mallard
`subagent_start_ordinal`. Add no separate metadata file, global child index,
per-child checkpoint, or duplicate parent graph. Reuse the existing parent
path-origin ledger for included child transformations.

~~~text
CLOUD SNAPSHOT                         LOCAL MACHINE
  parent T                               parent binding
    main file                              existing main native UUID
    child file entries                     installed_end
      key -> normalized state              key -> child UUID + installed_end
      key -> existing object entry
                                         storage-link checkpoint
                                           existing accepted main pair
                                           key -> accepted local/normalized

                                         observation cache
                                           current main and child evidence
                                           validated discovery/read cursors
~~~

| Operation result | Durable metadata update |
| --- | --- |
| Refresh or Review | Observation cache only; no accepted pair or installation change |
| Successful Push | Accept main and included child pairs together on the selected storage link; leave installation fields unchanged |
| Successful main-only Push | Accept main and clear that link's accepted child map; publish an empty child inventory |
| Successful Pull | Install main and listed children, then save bindings, accepted pairs, and receipts in the existing write order |
| Push parent rejected before publication | Preserve that parent's previous cloud row and accepted states; other valid parents can publish |
| Push fails after HEAD changes | Keep the published cloud version; completed local metadata writes remain |
| Pull parent fails before file installation | Install no rollout; do not save new binding or checkpoint state |
| Pull parent fails during installation or metadata saving | Report failure; installed files and completed metadata writes remain |
| Keep both | Retained local tree keeps its installation data; clear its acceptance for the selected storage link only. The imported tree gets distinct UUIDs, cutoffs, and pins |

**Apply failure does not undo completed writes.** Keep the existing
[materialization behavior](../crates/mallard-infrastructure/src/conversation_materialization.rs)
and [metadata write order](../crates/mallard-infrastructure/src/persistence.rs).
Push publishes cloud HEAD before saving local metadata. Pull installs files
before saving bindings, checkpoints, and receipts. Each metadata document
write is atomic; the complete operation can partially finish.

Stop the failed parent at the error. Use existing staging cleanup and retain
existing backups. Do not restore backups, remove installed rollouts, or
revert cloud HEAD automatically. Report the failure and any known completed
writes through the existing result flow. Other parents can continue while
the shared-state checks permit it. Reload durable metadata after a write
error as the current flow requires. A retry starts with a new review of
current state. Add no rollback journal or recovery state for this feature.

**Today's Refresh compares only the main.** The source of truth is
[compare_conversation_tail](../crates/mallard-domain/src/conversation_tail_relation.rs).
The comparison uses complete five-field tail values. It does not use a
lineage/semantic hash-pair prefix rule for normal accepted comparisons.

1. Read cloud HEAD and its referenced manifest. Verify metadata and reuse
   the snapshot across tasks. These are separate objects; the number of
   network requests depends on the existing cache and transport. Download
   no cloud transcript objects during Refresh.
2. Observe the local main. Reuse a valid observation, process a valid append,
   or read and fold a bounded snapshot. Unreadable evidence is Unknown.
3. Load the accepted local and normalized pair. For first contact, the
   existing bounded proof can normalize the local chain through the cloud
   end and compare it with the manifest. It can supply a temporary pair;
   it does not save acceptance or download a cloud transcript.
4. Construct `VerifiedTailEvidence`. Invalid content, an incomplete pair,
   incompatible layout, an expected missing side, or unproved overlap is
   Unknown. With no pins, confirmed one-sided presence is MissingCloud or
   MissingLocal; confirmed absence on both sides is MissingBoth.
5. Call `compare_conversation_tail`. When both current sides and pins exist:
   - Local equals accepted local and cloud equals accepted normalized:
     Synchronized.
   - Cloud equals accepted normalized and local's current message end is
     greater than accepted local's message end: LocalOutstanding.
   - Local equals accepted local and cloud's current message end is greater
     than accepted normalized's message end: CloudOutstanding.
   - Otherwise: Divergent. This includes changes on both sides and a changed
     state whose message end is equal to or below its accepted end.
6. Return the main result. Save observations only.

An increased count alone proves no relation. The comparator also requires
the opposite side to match its full accepted state. It does not prove an
unchanged prefix on the advancing side. Do not add a second prefix or
“rewrite” classification. Codex local and normalized codec labels differ;
they are compared only with their corresponding accepted form.

**Planned Refresh uses that comparator for every included rollout.**

1. Read and validate the same HEAD and manifest snapshot as today. Child
   normalized states come from child file entries. Fetch no child objects.
2. Observe the main and discover its current owned children. Reuse valid
   child observations. If the source support check selects main-only scope,
   exclude all child content and membership from this comparison.
3. Obtain accepted pairs for the main and each included child. Extend the
   existing first-contact proof to overlapping children when no child pair
   exists. A matching main cannot prove child acceptance.
4. Wrap each compact child state as a `PortableConversationTail`: the
   implemented form uses the neutral local or normalized child codec label,
   `whole_thread`, start 0, end `message_count`, and tail state `sha256`.
   The Claude plan's shared amendment changes only these derived child
   labels to the neutral pair. Construct `VerifiedTailEvidence` and call
   `compare_conversation_tail` per rollout.
   Do not persist these derived wrapper fields on each child.
5. Fold the valid results once at parent level. A new local-only child
   contributes local outstanding work. A new cloud-only child contributes
   cloud outstanding work. Any Divergent result, or work on both sides,
   makes the parent Divergent. Work on one side selects that side's action.
6. Return one parent result with its reviewed scope. Save observations only.

Membership additions require confirmed absence and no accepted entry for
that key. A missing referenced or installed child is invalid evidence.
An accepted child that unexpectedly disappears from either side is also
Unknown under `VerifiedTailEvidence`; it is not a deletion request. Do not
bypass that validator or substitute an accepted hash for a missing file.
Main-only scope deliberately excludes the entire child comparison; it never
claims to have accepted omitted child content.

**Recovery: Keep both when an accepted local child is missing.**

Before this amendment, Unknown blocked Pull. Keep the result Unavailable and keep Push
blocked, but permit a reviewed Keep both Pull when confirmed absence of
accepted local child files is the only evidence failure. Require a valid
main, binding, remaining local evidence, and cloud manifest that lists the
missing accepted keys. Missing cloud files, unreadable local files, unsafe
paths, and identity conflicts do not qualify.

Pin and validate the cloud snapshot through normal Pull. Revalidate the
missing-child cause at Apply. Restore its entire listed tree under fresh
UUIDs and reuse Keep both to preserve the remaining local files and unsaved
work. The tracked cloud task receives the restored binding and accepted
main/child pairs. The preserved copy retains its installation data and clears
acceptance for the selected link as normal. Other tasks are unaffected.

The restored copy can sync. The preserved copy remains unavailable until
the missing native file is restored from another device or backup and the
user refreshes. State this in review; do not promise recovery of unpublished
work. No storage-wide Clear is required.

Extend the shared action policy and plan validation, not the comparator.
Permit this action only with the verified missing-child cause; do not turn
Unknown into Divergent or introduce a generic override. Reuse the detailed
recovery contract in the [Claude plan](PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md).
Add no stored recovery mode or separate child action.

~~~text
COMPARE TODAY                          COMPARE PLANNED
  main T only                            main T + owned D + owned E
       |                                 C is excluded by ownership
       |                                      |
       |                                 unsupported owned child?
       |                                   yes -> main T only
       |                                   no  -> T, D, E
       +------------------+-------------------+
                          |
                          v
           Validate each included rollout's evidence
             invalid / expected missing -> Unavailable
             no pins + one side absent  -> Missing on that side
                          |
                          v
           SAME compare_conversation_tail FOR EACH ROLLOUT X

           L = current local X      A = accepted local X
           C = current cloud X      B = accepted normalized X
           Equality includes all five fields.

           L == A and C == B?
             yes -> Synchronized
             no  -> C == B and L.end > A.end?
                      yes -> LocalOutstanding
                      no  -> L == A and C.end > B.end?
                               yes -> CloudOutstanding
                               no  -> Divergent

           end = canonical message end, not file size or time
                          |
          +---------------+----------------------------+
          |                                            |
     TODAY: return T                           PLANNED: fold results
                                                       |
                              any Divergent? ----------+-> Divergent
                              else local AND cloud work -> Divergent
                              else local work ----------> LocalOutstanding
                              else cloud work ----------> CloudOutstanding
                              else ---------------------> Synchronized

  New local-only child: MissingCloud contributes LOCAL work.
  New cloud-only child: MissingLocal contributes CLOUD work.
  Parent absent on one side: retain its existing Missing status/action.
  Divergent -> Keep both. Local work -> Push. Cloud work -> Pull.
  Recovery: eligible missing-local-child Unavailable -> Keep both Pull.
~~~

Examples: D grows locally while T is unchanged, so Push T with its children.
D grows locally and E grows in the cloud, so Keep both. D changes content
without increasing its accepted message count, so Keep both. A change only
to inherited C adds no child work to T. An unchanged main and child set
remain synchronized.

**Review fixes selection; Apply captures bytes.**

| Operation and stage | Today | Planned |
| --- | --- | --- |
| Push Review | Observe main and cloud metadata; build the reviewed selection and size estimates | Also fix owned child membership and the include/omit decision; retain child size estimates |
| Push Apply | Capture selected main bytes, encode, verify sources, and publish | Capture latest main and included child bytes; validate the reviewed scope; verify captured sources and publish valid parents |
| Pull Review | Read cloud metadata and plan the main restore | Plan the main plus exactly the child entries in that manifest |
| Pull Apply | Fetch, verify, decode, and install the main with a fresh native UUID | Fetch and validate all listed rollouts; allocate K + 1 UUIDs; install files, then save metadata |

Review may read transcript content for a cold observation. It does not pin
upload bytes or a `SourceStamp`. Displayed sizes are estimates, not exact
staleness tests. Appends to already reviewed rollouts before Apply capture
are allowed and included. Apply rechecks current size limits and existing
content-warning requirements.

**Push Apply publishes each valid parent's complete selected scope.**

1. Validate the existing review token, revisions, expiry, warnings, selection,
   and expected cloud HEAD. Retain the current conflict-action guards.
2. Reobserve the reviewed sources and capture their latest bounded snapshots
   with the existing `read_snapshot` helper. It checks source stability
   during the read and returns bytes with a `SourceStamp`. Include any
   ancestor source needed to reconstruct a selected main payload.
3. Derive the owned child set from the captured rollouts. Check source
   support with their metadata and the bounded headers used for omission.
   The set and scope must match Review. A membership or include/omit change
   fails this parent and requires a new review.
   An ordinary append to an existing member does not fail this step.
4. Encode from the captured bytes. Compute local and normalized states,
   perform path conversion and redaction, and build child file entries.
   Reuse unchanged immutable objects through existing content checks.
   Do not reopen sources for another encode pass.
5. At the final publication gate, call `SourceStamp::verify` once for each
   captured source. An append, rewrite, truncation, or replacement detected
   since capture fails that parent. Do not publish through an older cutoff.
6. Publish valid parents through the existing immutable-object and HEAD CAS
   path. Preserve the previous row and pins for each parent rejected before
   publication. After publication succeeds, save accepted main and child
   pairs and receipts through the existing local metadata writes. If a
   later write fails, report failure; keep the published cloud version and
   any completed local writes.

The existing stamp checks file metadata and identity; it does not lock the
provider through network publication. Keep the current publication ordering
and integrity checks. The new rule removes the older verified-cutoff
exception and does not add a second Review snapshot layer.

Use this exact message for a source change or stale child selection:

> This task changed during Push. Push it again to include the latest work.

~~~text
PUSH REVIEW
  cloud metadata + local observations
  fix parent selection, child keys, include/omit decision, size estimates
                         |
                  user reviews
        ordinary appends to known members are allowed
                         |
                         v
PUSH APPLY: each selected parent
  capture latest main + included children + required source dependencies
                         |
  revalidate reviewed child set and support decision
                         |
       mismatch ---------+-> fail this parent; request another Push
                         |
  encode bytes + compute pairs + prepare immutable objects
                         |
  final SourceStamp verification for captured sources
                         |
       changed ----------+-> fail this parent; request another Push
                         |
                         v
  publish valid parents with HEAD CAS
  parents rejected before publication keep their previous rows and pins
                         |
  save local bindings, accepted pairs, and receipts
                         |
       error ------------+-> report failure; completed writes remain
                         |
                         v
  report success for parents whose required writes completed
~~~

Omitted child payload appends are outside a main-only capture. Changes that
alter the reviewed scope still require a new review. A batch can succeed
for some parents and fail for others. If no valid selected work remains,
do not create an empty revision. If HEAD publication is not confirmed, save
no new accepted state for that publication. Uploaded immutable objects can
remain unreferenced. An uncertain publication result requires a fresh read
of HEAD; it does not prove that the cloud stayed unchanged.

**Pull restores exactly the manifest-listed files.**

1. Review validates the selected immutable manifest and plans the main plus
   its listed child entries. An empty inventory plans the main only.
   Local unsupported children do not narrow a nonempty cloud inventory.
2. Apply revalidates the existing plan and destination guards. Fetch the
   listed objects and verify their exact sizes and SHA-256 digests.
3. Resolve each wire graph once. Reuse its parse for normalized content
   states, child keys, owning references, native boundaries, payload formats,
   and complete declared membership. Reject a missing or invalid
   listed child. Never silently omit it or switch to main-only restore.
   References left as native source UUIDs are outside the declared inventory;
   do not fetch them or classify them as missing portable payloads.
4. Extend `allocate_codex_thread_ids_local` from its current count of 1 to
   K + 1, where K is the number of listed children. Reuse its occupied-ID
   checks and bounded collision handling. Allocate during Apply.
5. Decode and stage all local rollouts with the allocated native IDs.
   Preserve child spawning-parent metadata and rewrite only included
   structural references. The builder validates each emitted native payload
   once and computes its local content state. Retain that immutable verification
   result through Apply. Compute each output file's `installed_end`.
6. Match verified results to the planned child identities, ownership, accepted
   pairs, project, and target bytes. Check staged file digests at commit; do not
   repeat transcript analysis after staging. Install through the existing
   materialization path. Save
   the binding map and cutoffs, accepted pairs, and receipt only after all
   required rollout files are installed. Use the existing metadata write
   order and one receipt and conflict action for the parent tree. Stop on
   an error and report failure. Completed file or metadata writes remain;
   do not roll back the parent or its children.

A main-only cloud snapshot is complete for its declared inventory. Pull
must not search the destination provider home for omitted historical
children. No omission mode, source scan, or fresh dangling UUID is needed.
For Keep both, preserve the local tree and install the listed cloud tree
under distinct native IDs. Do not share mutable child files between them.
Validate and stage all required rollouts before installation starts. This
keeps content failures before installation; a later write error can still
leave part of the imported tree. A new Codex rollout can remain even if its
binding or receipt was not saved. Existing source rollouts remain intact.

~~~text
PULL REVIEW
  pin selected manifest and normal restore guards
  plan T + exactly its listed children {D, E}; no child entry means T only
                         |
                         v
PULL APPLY
  fetch listed objects -> verify bytes and normalized states
                         |
  validate keys, payloads, ownership, and declared reference closure
                         |
       invalid ----------+-> fail parent before file installation
                         |
  allocate K + 1 native UUIDs -> decode and stage all listed rollouts
                         |
  compute local states + byte cutoffs from emitted files
                         |
  install staged files one by one
                         |
       error ------------+-> fail parent; installed files remain
                         |
  save binding map/cutoffs, accepted pairs, and receipt in existing order
                         |
       error ------------+-> fail parent; completed writes remain
                         |
  report success only after all required writes complete
                         |
                         v
NEXT REFRESH
  known installed children: binding map + ownership checks
  new children: records after each installed_end + ownership checks
~~~

**The only new omission notice is in Push Review.** Put one icon on the parent
row when its source support check selects main-only scope. Use accessible
label “Child rollouts excluded” and show this tooltip on hover or focus:

> Only this task will sync. All child rollouts are excluded because their history cannot be verified.

Add no banner, child selector, extra confirmation, or omission notice in
Compare, Pull, or results. Omission remains explicit in comparison scope
and Pull's validated file inventory. Synchronized refers to that scope.
Main-only success is a normal parent success. Existing result UI must list
successful and failed parents in a mixed batch.

**Performance depends on changed bytes and owned children.** There is no
fixed “100 directories” cost. Codex can store rollouts in multiple dated
session directories; children need not share a directory. Reuse inventory
and batch unresolved filenames into one pass per operation, not one pass
per child. Header reads verify owners; body reads supply content evidence.

| Case | Added work over today's main-only operation |
| --- | --- |
| Parent with no child references | Scan for activity while processing already-required main records; no child file opens |
| Cold source with K owned children | Resolve filenames once; read candidate headers; fold each included child body once, including descendant discovery |
| Native legacy fork | Also read headers for copied references to prove foreign ownership; no inherited child body hashing |
| Warm unchanged source | Reuse valid membership, hashes, and cursors; no full child body read |
| One child append | Validate that child's cursor, read its suffix, update its state, and inspect new activity; unchanged siblings need no full read |
| Push Apply | Capture and encode the included child bytes; verify one stamp per captured source |
| Pull Apply | Transfer and decode the listed child bytes; allocate and store K child identities |

Cloud metadata and durable accepted state grow with K. Capture and hashing
work grows with the included bytes. Cold legacy discovery also depends on
the number of inherited references. Refresh downloads zero cloud transcript
objects. No latency or throughput claim is made before measurement.

Use existing byte, file, record, path, traversal, and aggregate capture
bounds. Children share the parent's operation budget. Bound ownership depth
and reject cycles; do not give each child a new independent capture budget.
Preserve no-follow reads, special-file rejection, immutable publication,
redaction, canonical paths, and atomic writes of each metadata document.

The release performance gate compares the baseline and new code in release
builds on the same isolated fixtures: no children, wide children, nested
children, legacy forks, warm refresh, one child append, Push, and Pull.
Record median and p95 time, peak memory, and existing `sync_metrics` work
counts. Accept only if unchanged child bodies are not reread, filename
resolution is batched, discovery shares required parsing, and Refresh
fetches no transcript objects. Investigate a repeatable no-child slowdown
before release. Retain measurements with the test result, not estimates in
this design.

**Implementation and verification use the existing modules.**

1. Establish native fixtures before changing transport. Use Codex 0.152.0,
   `features.multi_agent_v2=true`, two temporary provider homes, temporary
   Mallard state, dummy credentials, and a deterministic loopback model.
   Verify both activity forms, copied child context, native boundary support,
   and actual cold resume. Never test against real provider homes.
2. Add the child role, compact states, binding fields, and accepted map in
   [the domain](../crates/mallard-domain/src/lib.rs). Add wrappers and the
   parent fold beside the existing sole comparator. Align affected Rust and
   TypeScript review contracts. Reject obsolete schema-1 documents.
3. Add focused ownership and observation support in the infrastructure layer.
   Reuse native parsing, history resolution, canonical folding, bounded
   reads, and [the observation cache](../src-tauri/src/project_sync/session_watch_service.rs).
   Keep filesystem and provider work out of the domain comparator.
4. Extend [the standalone builder](../crates/mallard-infrastructure/src/codex_standalone_builder.rs),
   Codex capture, and native materialization for included child payloads and
   identity maps. Add no Mallard boundary transport. Complete local-folder
   Push, Pull, and Keep both through the existing Apply paths.
5. Add per-parent failure handling and the Push review icon in the existing
   [project sync flow](../src-tauri/src/project_sync/service.rs). Verify the
   same bundle behavior with mock object storage, then run native and
   performance gates. Update current product and metadata documentation
   only when implementation is complete.

Required acceptance cases:

- P creates C, then forks T. T creates D and D creates E. T includes D and
  E only. Follow-ups append to the intended existing child. Repeat after
  Push, Pull, a native restart, a cache reset, and another Push. Creation
  keys and unchanged normalized states remain stable.
- A legacy-fork main with copied metadata and no marker syncs through
  ownership proof. A missing candidate makes it unavailable. A supported
  paginated main does not open ancestor sources solely for discovery.
- One legacy child or one paginated child with an invalid or missing native
  boundary omits the entire child tree. A foreign unsupported child does
  not. Main-only Push stores no child entries or accepted child states.
- Pull installs exactly its manifest inventory. A listed invalid child
  fails the parent. A main-only snapshot transfers no child object. Missing
  accepted evidence remains Unknown; no implicit child deletion is added.
- Proposed recovery amendment: remove an accepted child in an isolated home.
  Confirm Unavailable, blocked Push, and an eligible Keep both Pull. Confirm
  that the restored copy is Synchronized, surviving local work is preserved,
  and other tasks are unchanged. Reject recovery for other evidence failures
  or a reappearing child after Review.
- Cache reconstruction uses installed UUIDs and per-file cutoffs. New
  descendants after Pull are found. Imported omitted references remain
  excluded. Invalid cutoffs and lost installation metadata are not guessed.
- Validate duplicate activity projections, all four activity kinds, event
  ID preservation, owner-scoped nested keys, unsafe IDs, both parent fields,
  cycles, symlinks, partial records, and all existing operation bounds.
- Compare local-only and cloud-only child growth, new membership, equal-count
  content changes, mixed main/child changes, and changes to different children
  on opposite sides. Reuse the comparator tests; add parent-fold coverage.
  First-contact proof must cover overlapping children without cloud objects.
- Allow known-member appends between Review and Apply capture. Fail changed
  membership or scope. Fail any captured-source change detected at the final
  stamp check, including display-only or partial-record appends. Verify the
  exact retry message, mixed parent results, all-failed batches, and CAS loss.
- Verify byte tampering, normalized hash tampering, object reuse, per-link
  acceptance, and metadata write order. Invalid payloads must fail before
  installation. Review never advances accepted state.
- Inject failures between child installations and between metadata writes.
  Verify that completed writes remain, staging cleanup runs, and the parent
  reports failure. Also fail local metadata saving after Push changes HEAD;
  the published cloud version must remain. Do not expect unchanged pins
  after a metadata write error. Check shared-state reload before another
  parent proceeds and fresh review before retry. Add no rollback expectation.
- In the native client, resume imported T, D, and E after a cold restart and
  send child follow-ups. Test repeated Pull and Keep both. In a shared home,
  omitted source UUID references must not control or modify another tree.
  A displayed child or a result copied into T is not proof of native resume.
- Verify the complete Tauri review flow and icon keyboard access. Keep a
  screenshot with UI validation. Run the work-count and timing gate above.

Run the Rust library suite for backend changes. Run the frontend build and
integration suite for DTO or UI changes. All fixtures use temporary homes
and local or mock storage.

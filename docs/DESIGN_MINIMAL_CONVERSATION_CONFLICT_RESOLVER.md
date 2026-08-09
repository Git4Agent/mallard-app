# Design: Minimal conversation conflict resolver

- Status: Proposed
- Date: August 9, 2026
- Scope: ProjectSync conversation Push/Pull conflicts
- Compatibility: Clean cutover; no migration or legacy execution path

## Decision

Keep the cloud `thread_id` as the stable shared identity. A machine-local
`provider_thread_id` is only an alias for that cloud thread and may change
after a conflict.

The first compare-and-swap Push wins. A stale Push never overwrites or forks
cloud state automatically; it is blocked until the machine pulls. Pull then
preserves the losing local history as a new independent thread and keeps the
winning cloud history bound to the original cloud `thread_id`.

This requires no `family_id`, machine ID, or new portable metadata.

## Minimal durable schema

The portable `ConversationThread` schema is unchanged. In particular, the
resolver needs only its existing `thread_id`, message count, and
lineage/semantic digests. Provider IDs and optional fork provenance are not
matching keys.

The existing machine-local binding already contains the required state:

```json
{
  "thread_id": "T",
  "provider_thread_id": "P-B",
  "last_observed_lineage_digest": "<lineage-sha256>",
  "last_observed_semantic_digest": "<semantic-sha256>",
  "source_revision": 12,
  "source_manifest_sha256": "<manifest-sha256>",
  "revision": 7
}
```

The full record retains its existing project, replica, and provider-profile
scope. Those fields prevent local alias collisions; they do not participate
in cross-machine thread identity.

The digest pair is the accepted content base. The revision and manifest hash
are the accepted remote CAS base. Plans and journals additionally pin all
source bytes, target absence/digests, generated IDs, and operation phases, but
that recovery state is temporary and never portable.

## Resolver rules

Let `B` be the last accepted content base, `L` the current local history, and
`R` the current cloud history.

| Condition | Result |
| --- | --- |
| `L = R` | No-op; refresh the binding pins. |
| `L = B`, `R != B` | Fast-forward Pull into the provider thread bound to `T`. |
| `L != B`, `R = B` | Push `L` to `T`, guarded by head CAS. |
| `L != B`, `R != B`, `L != R` | Divergence; preserve local, then adopt cloud. |
| Ancestry or digests cannot be proved | Block and require review; never guess. |

A Push also compares the current remote head with the binding's accepted
revision and manifest hash. If they differ, Push returns `require_pull`
without publishing anything.

## Pull divergence transaction

For cloud thread `T` bound locally to provider thread `P-local`:

1. Pre-mint an independent Mallard thread ID `C` and fresh provider IDs in the
   reviewed plan. Retries reuse exactly those IDs.
2. Reconstruct the complete local dependency graph behind `P-local` as a
   standalone provider thread `P-preserved`, then bind `C -> P-preserved`.
3. Verify that the preserved copy has the same canonical message history and
   is independently readable and resumable.
4. Materialize cloud `T` as a standalone local provider thread `P-remote`.
5. Atomically switch the binding to `T -> P-remote` and pin the accepted cloud
   revision, manifest, and content digests.
6. Commit only after both provider materializations and both bindings are
   recoverable. On failure, retain the original local thread and resume from
   the journal.

The preserved `C` is an ordinary independent thread. It has no required
family or parent relationship with `T`. If the user later pushes `C`, it is
published under `C`; it never overwrites cloud `T`.

For Codex paginated histories, every newly constructed standalone rollout
must contain the entire reachable dependency history and end with:

```json
{
  "history_mode": "paginated",
  "history_base": null
}
```

Its authoritative `session_meta` and all provider/session references must be
rewritten consistently to the new provider ID. Subagent relationships must
remain valid, but they do not create Mallard thread-family metadata.

## Example

```text
cloud T:       m1 -> m2 -> m3
A: T -> P-A    m1 -> m2 -> m3
B: T -> P-B    m1 -> m2 -> m3
```

A adds `m5` and wins the Push CAS. B has independently added `m4`, so B's
next Push is blocked. B pulls and resolves:

```text
B: C -> P-C    m1 -> m2 -> m3 -> m4   (preserved, independent)
B: T -> P-B2   m1 -> m2 -> m3 -> m5   (tracks cloud T)
```

B can add `m6` to `P-B2` and Push it to cloud `T`. A then receives `m6` by
pulling the same thread `T`. If A has also added `m7`, A goes through the same
divergence transaction and its losing branch is decoupled under another new
independent ID.

## Invariants

- Cloud `T` has one mutable head protected by compare-and-swap.
- A stale Push cannot overwrite a remote winner.
- Conflict resolution never destroys either branch.
- A logical cloud ID and a provider-native ID are never assumed equal.
- A newly minted provider thread is standalone before its source may be
  replaced or removed.
- IDs are random, plan-persisted, collision-checked, and retry-stable.
- No resolver decision depends on title, `family_id`, machine ID, or fork
  provenance.

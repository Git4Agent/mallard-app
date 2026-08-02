# Design: Acknowledge Codex forks from filesystem events

- Status: Accepted
- Last reviewed: August 1, 2026
- Compatibility: clean cutover

## Decision

Mallard observes a native-PTY fork with a short-lived filesystem watcher.
The durable and runtime order is:

1. persist a `Pending` attempt;
2. start a recursive watch on the active Codex `sessions/` tree;
3. launch `codex fork` in the native PTY;
4. accept only a definite create or rename-to event for a rollout-shaped
   path;
5. retry bounded reads of that observed path while Codex finishes writing it,
   then verify it twice across the stability window; and
6. persist the exact provider ID, canonical path, and SHA-256 as `Verified`.

The event is only a locator. The file read proves canonical containment,
regular no-follow file type, stable file identity across the opened handle and
persisted path, bounded size, interior thread identity, parent, project cwd,
lineage, semantics, and copied history. The reviewed source is also re-read
from its exact persisted path.

## Unacknowledged attempts

After Codex has been launched, any timeout, missed event, watcher loss or
overflow, process-lifecycle uncertainty, or verification failure produces
`Unacknowledged`. Mallard does not scan for, adopt, bind, rename, or delete a
possible fork from that attempt. It also does not fall through to another
fork executor.

A durable `Pending` found after restart is changed directly to
`Unacknowledged`, without reading the profile. A crash before any executor was
durably marked `Pending` rolls back staging and never starts a provider during
startup recovery. The user may explicitly retry an unacknowledged operation.
Retrying may create another Codex fork; any unacknowledged fork remains
ordinary, unbound Codex state.

An explicit retry begins again at the unacknowledged executor. If that
executor proves it did not invoke a fork, the same user-authorized retry may
continue to a later executor; another uncertain result still stops
immediately.

Failures before the PTY child is spawned are definitive no-fork outcomes and
may use the next reviewed executor.

## Watcher failure policy

The native watcher is bounded and scoped to one attempt. If `sessions/` does
not yet exist, Mallard watches only the `CODEX_HOME` directory entry
non-recursively and arms the recursive `sessions/` watch when it appears; it
never recursively watches the whole profile. Events are filtered to rollout
paths before entering the bounded queue. Transcript bytes are never retained.
Queue overflow, a backend rescan request, callback error, or channel
disconnect ends observation as `Unacknowledged`.

There is no polling watcher, all-dirty mode, SQLite fallback, baseline walk,
or final recovery scan. A dropped event can therefore cause an
unacknowledged attempt, but cannot cause Mallard to adopt the wrong thread.

## Removed profile-wide discovery

Fork execution no longer contains `scan_rollout_paths`,
`scan_native_threads`, child-inventory pinning, by-ID profile discovery, or
manual recovery adoption. Exact-file reads are verification, not scans.

Features that intentionally list imported provider threads retain their own
bounded provider-capture traversal; it is not used by PTY execution, timeout
handling, crash recovery, or retry.

## App-server path

When app-server is available, `thread/fork` requests metadata only and uses
the returned thread ID and rollout path for the same exact-file proof. Failure
before the fork RPC is invoked may fall back to native PTY. Every error,
timeout, missing path, or unverifiable result after invocation is
unacknowledged and never falls through automatically.

## Required tests

- watcher is armed before the PTY child is spawned;
- create and rename-to events can identify one exact fork;
- modification of an old rollout cannot promote it to a candidate;
- partial creation is accepted only after stable exact-file verification;
- unrelated profile files do not affect the attempt;
- no event, overflow, disconnect, timeout, or stale durable `Pending` becomes
  unacknowledged without a scan;
- unacknowledged stops executor fallthrough; and
- only an explicit retry starts another attempt.

**Child Push settings: verification**

Date: 2026-09-09. Implementation:
[Control child inclusion in Push](PLAN_CHILD_SYNC_SETTINGS.md).

**Behavior**

- Settings has separate Codex and Claude child Push switches. Both default On.
- Every cloud task requires `wire_children: "on" | "off"`. Off rejects child
  entries. Missing, null, and unknown stamps fail validation.
- Compare uses the cloud stamp. Push uses the local setting. A setting change
  does not create a false content difference or bypass a conflict.
- Pull restores the listed files and keeps local settings. On with zero
  children is valid, including a later Push of an Off snapshot.
- Review pins outgoing membership, omission, and stamp. Apply captures bytes.
  A stale parent fails separately; other valid parents can publish.
- Missing-child Keep both recovery remains available. Push source checks do
  not change Pull's recovery capability.
- Local state and cloud schemas remain 1. No migration or legacy reader was added.

**Metadata and performance audit**

Local settings gain two booleans. The cloud task gains one child stamp. The
existing temporary Push selection also carries that stamp. Child comparison
pins and outgoing selection are separate. No binding mode, checkpoint stamp,
observation-scope cache, child codec, or per-child settings read was added.

Observation remains independent of settings. A failed child attachment keeps
proven main evidence; an incomplete child observation is not cached as a
successful tree. Cloud Off can use the main evidence. An outgoing On Push
still requires child source evidence.

Child publication now uses each captured child's mtime and the existing
Retained/Captured placement. Retention compares identity, digest, size, mtime,
and mode with the verified previous entry. Payload SHA-256 is already cached
in `PayloadBytes`; retention adds no content hash pass, filesystem stat, or
remote object-existence request. On still captures and encodes child content.
Off skips child payload capture and encoding.

**Measured transfer results**

Synthetic zero- and eight-child fixtures use temporary provider homes and a
local-folder store. The same produced wire files are also replayed through
the production placement rule, bundle publisher, and Dropbox adapter against
its loopback API. The server records received upload-session body hashes and
sizes. No live storage service or personal provider home is used.

Parent append, eight unchanged children:

| Provider | Before: submitted objects / bytes | After: submitted objects / bytes | Child HTTP body bytes after retention |
| --- | ---: | ---: | ---: |
| Codex | 9 / 7,483 | 1 / 2,467 | 0 |
| Claude | 9 / 7,122 | 1 / 3,194 | 0 |

One changed child, seven unchanged children, unchanged parent:

| Provider | Submitted objects / bytes | Child HTTP bodies received |
| --- | ---: | ---: |
| Codex | 1 / 815 | 1 / 815 bytes |
| Claude | 1 / 691 | 1 / 691 bytes |

Submitted counters cover payload objects, including attempts that a store
could report as AlreadyPresent. HTTP checks identify child payload bodies by
hash and exclude metadata. These checks measure transfer work, not internet
latency. The loopback replay checks transport separately from service capture.

Empty On-stamp publication after an Off Pull, one debug-build run:

| Provider | Review + Apply, fresh empty task | Review + Apply, task that previously had eight children | Submitted payloads |
| --- | ---: | ---: | --- |
| Codex | 153 ms | 151 ms | 0 objects in both cases |
| Claude | 134 ms | 168 ms | 1 main object in each case |

Both cases publish a new stamp through the normal path. These single-run
elapsed times are not CPU benchmarks or stable latency estimates. Opposite
settings on two machines can make a task eligible for another reviewed Push.
Content can still compare Synchronized.

**Checks**

- Frontend build passed. Vite reports the existing large-chunk advisory.
- Frontend integration: 294 passed.
- Domain library: 86 passed.
- Infrastructure library: 514 passed; 5 optional tests ignored.
- Backend library: 290 passed; 6 optional tests ignored.
- Targeted lifecycle checks cover both providers, zero and eight children,
  all four stamp/setting combinations, child-only changes, Off adoption,
  fresh and previously populated receiving tasks, and empty On publication.
- Mixed-provider review checks confirm a stale Codex selection leaves the
  Claude parent publishable. Missing/corrupt child and corrupt main fixtures
  verify the comparison and outgoing-source error boundaries.
- The unsupported-Claude-child check confirms a stamp change makes Review
  stale even when both settings produce the same empty, omitted selection.
- Whitespace check passed.

**Native UI check**

The debug Tauri app used temporary Mallard state, temporary provider homes,
and a synthetic project with eight children per provider. Both switches saved
independently through the native backend. The saved settings document kept
schema 1. Returning to the project refreshed the Push decision. Review showed
one omitted-child icon, the specified tooltip, and the outgoing file estimate.
The QA app was closed after the check.

- [Settings screenshot](images/child-sync-settings.png)
- [Push review tooltip screenshot](images/child-sync-push-review.png)

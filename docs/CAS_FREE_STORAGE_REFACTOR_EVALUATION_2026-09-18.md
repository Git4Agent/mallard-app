# Evaluation: CAS-free storage model refactor

- Evaluated: [DESIGN_CAS_FREE_STORAGE_REFACTOR.md](DESIGN_CAS_FREE_STORAGE_REFACTOR.md) (untracked, 825 lines, design only)
- Date: September 18, 2026
- Tree: `460edb6` plus uncommitted work (25 Rust files, +820 / −914, in the same
  files this refactor rewrites)
- Constraints given by the owner: desktop app only today; clean cutover; no
  migration or legacy execution path; every schema number stays `1`;
  `~/.mallard` and cloud data are wiped for retesting; no customers
- Method: the design's claims were checked against the working tree, the real
  `~/.mallard` and local storage on this machine, and the Managed Cloud service
  in `mallardInternal/cloudflare/managed-cloud`. Nothing was built or run. Line
  numbers are working-tree lines.

## 1. Verdict

The direction is sound and the design's section 2 description of the current
code is accurate on all nine rows. The motivation is documented in the
repository: the provider research blocks Google Drive, Aliyun OSS, Huawei OBS
and Tencent COS on compare-and-swap, and the mainland China assessment itself
recommends evaluating an append-only publication protocol.

As written it does not fit the constraints:

1. **It is not a desktop-only change.** Managed Cloud cannot follow without a
   service and website redesign (section 2.1).
2. **Three protocol rules turn a rare fork into a late, large reselection**
   (section 2.2).
3. **It regresses two things the current engine does**: rebasing a Push onto
   the apply-time head, and the one-read steady refresh (sections 2.3, 3.1).
4. **It carries work the constraints make unnecessary**: legacy rejection
   paths, a durable outbox, archive browsing (section 4).

The design bundles two refactors. Deriving the head from immutable records is
what removes compare-and-swap. Opaque locators, the snapshot envelope, the
payload table and the UID-qualified root are a transport change that only an
identifier-addressed provider needs. The second half is what changes the
portable manifest and breaks Managed Cloud. Section 6 describes the smaller cut.

## 2. Decisions needed before starting

### 2.1 "Desktop only" is not possible for Managed Cloud under this design

Measured in `mallardInternal/cloudflare/managed-cloud`:

- **The service validates the manifest shape** at upload creation and again at
  commit. `src/validate/manifest.ts:148` rejects any file entry whose
  `object_key` is not `_objects/<sha256>`; `:83-86` require `schema_version 1`,
  `revision >= 1` and a `commit-<hex>` id. The inventory must equal the
  manifest's payload digests, and an extra blob is refused
  (`src/routes/uploads.ts:68-92`). A snapshot envelope or a publication record
  cannot be uploaded at all.
- **The website parses the manifest in the browser**
  (`web/src/content.ts:40-50`, `:101-114`).
- **The schema is single-head.** The head is a pointer column,
  `repositories.private_publication_id` (`migrations/0001_init.sql:93-99`).
  `publications` has `UNIQUE (repository_id, scope, revision)` and no parent
  column (`:193-215`). The commit batch compares the repository revision, the
  private revision and the expected commit id (`uploads.ts:427-447`), and the
  same pins run at upload creation (`:190-194`). Two siblings of one parent can
  never both be finalized.
- **The cleanup job deletes what the design requires the service to keep.**
  Every five minutes it deletes `ready` objects that no `available` publication
  references (`src/jobs/cleanup.ts:47-59`), and the commit batch retires the
  previous head (`uploads.ts:489`). The service does not retain mainline
  history payloads today, only the head's. Design sections 12.1 and 13 forbid
  exactly this.
- **Public release is a server-side approval pinned to the private head**
  (`src/routes/releases.ts:142-262`). An approval act has no earliest-sibling
  equivalent. The design does not mention it.
- **Session views are carried forward from the single private head**
  (`uploads.ts:473-488`).
- **Open public repositories accept a push from any signed-in account**
  (`src/db/tx.ts:28-36`). Design 6.3 assumes trusted writers inside the storage
  ACL. With client timestamps, any contributor could backdate a sibling of the
  first publication and displace the whole mainline.
- **The service's own test pins the opposite behaviour**: two collaborators
  push side by side, the first commit wins, the second gets 409 `head_changed`
  and pushes again on top (`test/uploads.test.ts:288-383`).
- **Blobs cannot be uploaded before a manifest exists.** Every object must be
  declared in an upload's inventory first (`uploads.ts:319-323`), the reverse of
  the design's publish order. The desktop hides this today by buffering objects
  in memory.
- Desktop side: the managed module is 6,316 lines. Of the 921 production lines
  in `managed_cloud/store.rs`, 482 are head, CAS and synthetic-key emulation
  that disappear and 317 more are rewritten (87% in total). `stub.rs`
  (1,336 lines) emulates the same protocol and must be rewritten in lockstep
  with the real service.

Options:

| Option | Consequence |
| --- | --- |
| A. Unregister `managed_cloud` until the service ships a publications API | Parks about 6,300 recently written lines and the sign-in lane; they stop compiling once `ObjectStore` goes |
| B. Add the service and website to the scope | Not "desktop only"; staging and production are live |
| C. Put the seam at the publication level, so Managed Cloud keeps its server-arbitrated commit **(recommended)** | No service change today. Requires that the portable manifest shape does not change, which section 6 also wants |

Option C contradicts design section 12.1 ("rejection because a different
publication already advanced HEAD is not acceptable"). That rule buys
uniformity only. A provider that refuses a stale parent never produces a fork,
and the selector still works over its linear log. The engine handles the
refusal as an ordinary error with today's "compare again" outcome. No
capability flag is needed.

### 2.2 A late-written record with an old timestamp displaces the mainline

The rule is earliest `(created_at_ms, publication_id)` per parent, the timestamp
is frozen before the final append (design 6.2), and an unconfirmed record is
resent with identical bytes (design 8.3). Put together:

1. Machine B freezes record XB under parent H0, then loses the network or goes
   to sleep before the append lands. A laptop lid closing mid-push is enough.
2. A and C publish H0 → XA → XA2 → … for three days.
3. B wakes and resends the identical bytes. `rank(XB) < rank(XA)`, so XB is
   selected and the whole XA subtree becomes archive, on every device.

A listing is read-after-write consistent on S3 and R2 (documented), on a local
folder and on D1. Dropbox behaves that way in practice; the new setup probe
(append, then the listing must show it) should confirm it. So on the current
providers "discovered later" can only mean "written later", and this path is
the realistic one, not an adversarial one. Three cheap rules close it:

- Scan again immediately before the final append. If the parent already has a
  child, stop with `ReviewStale`. The fork window shrinks from the payload
  upload (18 s to 99 s measured on Dropbox) to one list plus one put.
- Bound the freeze-to-append gap (for example 60 s on a monotonic clock).
  Past it, freeze a new timestamp and scan again. Nothing was sent, so the
  frozen-bytes rule is not broken.
- Never resend an unconfirmed record when a complete scan shows the parent has
  another child and does not show the record. Abandon it.

With those rules the remaining forks are near-simultaneous, and the timestamp
adds little over the random id as a tie-break. Keep it only for the
"first writer wins" reading.

### 2.3 Push apply regresses: today it rebases onto the apply-time head

Design 8.1 step 3: "Build the complete next snapshot from that exact parent …
Do not combine data from a newly selected branch without another review."

Today Push apply reads the head again (`service.rs:10116`), runs
`classify_push_destination` per writable scope (`service.rs:7607-7709`, doc
comment: "A remote change in an unbound or unknown provider scope is carried,
not blocking"), and the engine carries every other scope forward byte for byte
(`bundle_engine.rs:861-864`, firewall `:978-1009`). A teammate's Claude push
between review and apply does not void a Codex push. The Library does the same
by design: the new set is "freshly observed manifest + reviewed upserts — never
a replace" (`storage_catalog/service.rs:1794-1800`).

Scopes are `project` plus one per provider (`mallard-domain/src/lib.rs:621-624`),
not per machine, so two machines on Codex share `provider:codex` and are gated
today as well. The regression is the disjoint-scope case and every Library push.

Keep the current rule: the parent is the head selected at apply time, and the
gate is per-scope content continuity against the reviewed base.

### 2.4 Continuity under forks is not specified, and revision ordering gives wrong answers

Design 10.1 says to replace "global revision assumptions with immutable
references" and stops there. This is the centre of the desktop change. Sites
that order remote revisions today:

- `classify_remote_continuity` rows 6 to 8 (`lib.rs:999-1044`)
- `classify_scope_continuity` `<` and `>` (`lib.rs:923-958`)
- `repository_base_from_link` = `max_by_key(revision)` (`service.rs:13583-13588`)
- `scope_revision.max(...)` (`service.rs:12231`, `:13243-13244`)
- the Library's `observed.revision < accepted.revision` → "history was rewound"
  (`resource_engine.rs:81-83`)

Under forks a scope revision is a per-branch counter. A sibling branch with a
higher counter classifies as `Advanced`, and Pull fast-forwards onto a snapshot
that does not descend from the base.

The mapping the design should state:

| Today | After |
| --- | --- |
| base equals head | `InSync` |
| head revision > base, probe hits | base is an ancestor of the selected head → `Advanced` |
| revision regressed, same revision with another digest, probe misses | base is not an ancestor → `Replaced` (this is also the reselection case) |
| per scope | digest equal → `InSync`; otherwise inherit the repository verdict |

The index already holds the graph, so the row 7 and 8 probe and its extra
witness read go away. It also closes the "commit-record ancestry walk" TODO in
DESIGN_REMOTE_HISTORY_CONTINUITY.md. The existing `Replaced` flow (Push blocked,
"Pull and review", then Push again) then becomes the losing device's recovery
path with no new UI.

**Bases advance on upload today.** Project Push advances every writable scope
base right after publish returns (`service.rs:10445-10563`) and stamps the
accepted conversation checkpoints (`conversation_materialization.rs:160-200`).
That is sound today because the CAS arbitrates before the local commit. Design
10.2 says "never update accepted or installed pins simply because an upload
completed", which contradicts both paths. With ancestor-based continuity the
project side needs no stored-versus-selected split: a base that later loses is
simply not an ancestor of the selected head, so it reads `Replaced` and the
Pull-then-Push flow repairs it. Say so in the design, or specify the split.

Two Library gaps sit under this heading:

- The per-resource base also advances at upload
  (`storage_catalog/service.rs:1868-1885`). If that publication later loses, the
  row derives `RemoteMissing`, which is not selectable for Push
  (`resource_planning.rs:258-262`). The archived pusher cannot push again
  through the existing review. The reset that storage clear already performs
  (`accepted_head = None` plus cleared `accepted_resources`,
  `storage_catalog/persistence.rs:546-558`) would reopen those rows when the
  accepted head is not an ancestor.
- Library apply takes no local lock (`storage_catalog/service.rs:1700-1897`).
  Today a double Apply is stopped by the CAS. Design 8.3 says to "keep"
  single-flight; for the Library there is none to keep, so a double-click forks
  the stream.

## 3. Costs the design understates

### 3.1 Refresh goes from one GET to a listing of a storage-wide collection

- A steady refresh today is **one** remote read: the head. The manifest comes
  from the in-process object cache (`service.rs:7041-7073`). There is no
  background polling; remote reads happen per user action (project open 2,
  refresh 1, Push 2, Pull 3).
- Production code lists the store in exactly two places today: the flat `_tags/`
  prefix and storage clear. The design moves listing onto every one of those
  user actions.
- One Dropbox round trip measures 0.3 to 0.8 s (SYNC_PATH_AUDIT_2026-09-11.md).
  `list_folder` returns 1,000 entries a page. The current Dropbox `enumerate`
  walks the whole subtree again for every Mallard page, with hard caps of 200
  pages, 100,000 entries, 32 MiB and 120 s (`dropbox_store.rs:38-40`). The local
  `list` takes the storage-wide lock, walks the tree, and hashes every listed
  file to produce its etag (`local_store.rs:376-419`).
- All streams share one `publications` collection (design 4.4), so every
  project's refresh and every Library page pays for the history of every
  stream. There is no garbage collection (design 13), and hitting a budget
  yields `Incomplete`, which blocks new operations (design 9). A long-lived
  storage eventually cannot push.

Use a collection per stream. On key-addressed stores a prefix costs nothing and
needs no exclusive creation; `_commits/` already is one. Names that sort by
time let S3 resume with `start-after`, and Dropbox has a native listing cursor.
The design allows both as optimizations.

### 3.2 Bundle listing goes from a 0.3 KB tag to one snapshot per stream

`list_remote_bundles` reads one small tag per bundle. It feeds project setup,
the links workspace, the link flow and sibling status. Design 9 derives
summaries "from publications and the selected feature snapshot". The real
manifest on this machine is 118.8 KB for 39 objects and 154 MB of sessions. Put
the `BundleTag` fields inside the publication record so a listing needs records
only.

### 3.3 The Library gets a new cost class

The Library never lists. There are six coordinates in the production registry
(`storage_catalog/registry.rs:305-315`), repository ids are derived from the
coordinate, and a page reads two or three fixed keys per capability. Under the
design it scans the global collection. Other consequences the design does not
work through:

- `_tag.json`, which binds the truncated id back to a readable coordinate, has
  no successor.
- Streams for coordinates this build does not register become visible, and
  design 14.2 forbids skipping unknown records.
- The planned Managed Cloud item adapter holds one item per service repository
  (CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md:712-736), so
  `library/<catalog_repository_id>` is not a unique stream there.

One gain the design does not claim: reusing references from the parent snapshot
removes the full verified GET per reused payload that every Library push pays
today (`resource_engine.rs:319-343`).

## 4. What to cut, given the constraints

| Design item | Why it can go |
| --- | --- |
| 14.1 and 14.2 legacy rejection: the "reconnect/reinitialize-required" result and the three acceptance rows for an old marker, old pins and an old layout | No old data exists after a wipe. AGENTS.md:3-4 says not to keep compatibility paths, and design 14.3 itself removes "legacy fixtures". The affected structs already use `deny_unknown_fields`, so an old shape fails to parse. Library plans live in memory only, so "invalidate legacy pending plans" does nothing there. Keep the new format string; it is one constant |
| 8.3 durable per-operation outbox | It contradicts "Mallard keeps no status between pushes". Project Push and Library Push are both stateless today. With consistent listings a fresh scan resolves an unknown outcome: the record is there or it is not. Defer it to the first eventually-consistent provider |
| 11 archive browsing and "Restore as a copy" | The loser's content is still on the loser's disk, and the `Replaced` flow already walks it through Pull then Push. Ship the role computation, a count and diagnostics first |
| 6.3 integrity-conflict state ("same id, different bytes" blocks the stream) | No recovery path is defined and records cannot be deleted. Make the record hash the identity, or name records `<id>-<sha256>`, and the state cannot occur |
| `publication_id` | The manifest already mints a random `commit_id` per publication. One id is enough |

The UI work that remains (outcome states, the reselection notice) should go
through a static specimen first, as usual.

## 5. Gaps

- **Joining from a second device.** The root key is not computable without the
  UID, so a joiner must list `.mallard/v1/` and read each descriptor. The
  discovery backend in the tree reads the fixed key: `classify_storage_marker`,
  `dropbox_auth.rs:486`, `r2_setup/discovery.rs:125`. One destination may now
  hold several roots, while `DiscoveryCandidate` and
  `ensure_destination_available` assume one. Two concurrent Creates give a
  silent split-brain; today the second writer joins the first. `StorageConfig`
  has no field for the pinned descriptor, and settings are locked for life
  (`storage_setup.rs:2396-2404`). Nothing in setup provisions a root today: the
  marker is created by the first open. None of this is in the design's change
  map.
- **Clear storage.** Today clear keeps the marker and resets link state so the
  same destination continues in place (`storage_core/mod.rs:127-129`,
  `storage_catalog/persistence.rs:546-558`; the catalog e2e republishes at
  revision 1 after a clear). Design 13 requires a new UID, which kills every
  other device's binding. That is a product change. It also affects the retest
  routine: wiping cloud data but not `~/.mallard` leaves a binding to a missing
  root, and wiping `~/.mallard` but not the cloud needs the join flow.
- **Foreign files in the feed.** Design 14.2: "an unknown record … must not be
  silently skipped". Every transport's listing fails closed today: Dropbox
  aborts on any non-canonical or duplicate path (`dropbox_store.rs:794-805`),
  S3 on a key that does not parse (`s3_store.rs:842`), local on any symlink
  (`local_store.rs:386-391`). That only breaks the link chooser and clear now.
  Once listing is the discovery path, one stray file stops sync for every
  device: Finder writes `.DS_Store` into a browsed folder, and Dropbox creates
  "conflicted copy" names. The Dropbox duplicate check also contradicts design
  5.1, which allows duplicate-producing listings. The adapter needs a name
  grammar; names outside it are ignored and reported, and only a well-named
  invalid record blocks.
- **Listing mechanics.** All three transports use the last key string as the
  cursor and rebuild the full sorted set per page; only S3 resumes
  incrementally. A lexical cursor cannot answer "what is new" once names are
  random. Dropbox's native cursor is discarded today
  (`dropbox_store.rs:830-836`). The Dropbox listing is recursive, so the
  publications prefix must never contain blobs. `finish_batch_v2` is not atomic
  across entries, so "record last" costs Dropbox a separate final commit of two
  more requests.
- **Names.** "Fresh random physical names per attempt" makes every retry leave
  garbage that is never collected. A key-addressed adapter may derive the name
  from the content (`blobs/<sha256>`, `publications/<id>-<sha256>`). Writing
  identical bytes to the same key needs no exclusivity, retries leave nothing
  behind, and common code still treats the locator as opaque.
- **Base addressing.** `RepositoryBase` is also the address of the base
  manifest that the three-way project-files compare re-reads
  (`service.rs:2347-2353`, `:2712-2720`). `RemoteSnapshotPin` carries no
  locator, so that read depends on the index still holding the record.
- **Re-push cost.** A losing device's payloads are already uploaded. Unless its
  own archived publication is a reuse source, the second push uploads them
  again.
- **An advantage the design does not claim.** A local folder inside a file-sync
  product (iCloud Drive, a Dropbox or OneDrive desktop folder, Syncthing)
  becomes safe for several machines, because immutable uniquely named files
  never conflict. The research document rules that setup out today because the
  only exclusion is a machine-local lock file (`local_store.rs:93-103`), so the
  head CAS gives no protection across machines there.
- **Housekeeping.** "Supersedes DESIGN_APPEND_ONLY_MAINLINE_ARCHIVE.md" names a
  file that is not in `docs/`. The baseline is `460edb6`, but the tree has about
  1,700 changed lines in the files to be rewritten, and other sessions share
  it. Commit first.

## 6. A smaller cut that reaches the same goal

`CommitRecord` (`bundle_engine.rs:435-449`) is already an immutable per-publication
record with a parent pointer. It is written on every publish and never read.

| Design publication record | `CommitRecord` today |
| --- | --- |
| `stream_id` | `bundle_id` |
| `publication_id` | `commit_id` |
| `parent` | `previous_commit_id` (no hash) |
| `created_at_ms` | `created_at` (seconds) |
| `origin_id` | `published_by_origin` |
| `snapshot {locator, size, sha256}` | `manifest_key`, `manifest_sha256` (no size) |

Design 8.2 says `_commits/` must not be reinterpreted. That holds for existing
files, and after a wipe there are none.

1. Promote the commit record to the publication point: add the manifest size,
   the parent's record hash, a millisecond timestamp and the tag summary, and
   write it **last**. Delete `_head.json` and `_tags/`.
2. The head is the selector over a listing of that stream's `_commits/` prefix.
3. Keep `_objects/<sha256>`, `_manifests/<revision>-<commit_id>.json` and the
   portable manifest schema exactly as they are. `revision` becomes depth and is
   display only. Every local pin keeps its shape (`RepositoryBase`,
   `MaterializationRecord`, `ConversationApplyReceipt`, `RestorePlan`,
   `ConversationRevisionPins`, 14 TypeScript types); only ordering comparisons
   change, per section 2.4.
4. `ObjectStore` loses `compare_and_swap`, and `put_immutable` loses its
   no-overwrite promise, because every name is unique or content-addressed.
   That removes about 200 lines plus the 410-line capability probe from
   `s3_store.rs`, about 130 lines plus most of `probe.rs` from Dropbox, and
   about 108 lines from `local_store.rs`.
5. Managed Cloud goes behind the publication-level seam of section 2.1 with no
   service change, because the manifest it validates is unchanged.

This gives up opaque locators. An identifier-addressed adapter would resolve
names by listing a folder and caching name to id. That is safe because names are
unique by construction and every read is hash-verified. If the first such
adapter shows that is too slow, add locators then. The design already schedules
Feishu as a later adapter-only project.

## 7. Size of the design as written

| Area | Measured |
| --- | --- |
| Transport and fakes | `object_store.rs` 362, `local_store.rs` 677, `s3_store.rs` 2,536 (about 1,190 are tests with a fake S3 server that emulates `If-Match`), Dropbox 3,172 across five files, `storage_core/mod.rs` 716. Eight `impl ObjectStore` blocks, three of them test wrappers |
| Engines | `bundle_engine.rs` 6,238 (61 head or CAS references, 26 tests); `resource_engine.rs` 754, a parallel implementation with no shared publication code, about two thirds rewritten |
| Managed Cloud desktop | 6,316 lines; `store.rs` 87% deleted or rewritten; `stub.rs` rewritten with the service |
| Callers | 20 production engine call sites in `service.rs` plus 1; three continuity classifiers; 5 persisted record types; 6 head-shaped types that cross IPC; 14 TypeScript types |
| New subsystems | selector, persisted publication index, discovery scanner with budgets, outbox, root create and join plus discovery changes, archive UI |
| Library side | 7 of 20 Rust files touched; the other 13 (8,369 lines, including `apply.rs`) have no head, CAS or etag reference. Estimate, not measured: 1,000 to 1,400 Rust lines, about 200 lines of the e2e flow, 150 to 300 frontend lines, before the shared engine |
| Tests | 1,148 Rust test functions in total. 48 name a CAS-era symbol in their body (56 with `put_many`, `HEAD_FILE` and similar): Dropbox store 15, `service.rs` 5, `storage_parity.rs` 5, managed store 4, `storage_core/mod.rs` 4, engine 3, S3 3, local 3. This is a lower bound: every test that publishes through an engine exercises the head CAS through a helper, and the six R2 capability-probe tests assert CAS through the probe API. Library: 25 of 129 touched, 12 of them assert head, CAS, revision or on-disk layout, and the catalog e2e threads a numeric revision through 20 pushes. Two fake S3 servers (`s3_store.rs` tests, `sync_tests/stub_s3.rs`) and the 632-line Dropbox mock emulate conditional writes |
| Outside the desktop | service schema, commit batch, cleanup job, release flow, views carry-forward, manifest validator, website manifest parsing |

## 8. Suggested order

1. Commit the in-flight work.
2. Decide section 2.1. Option C is recommended.
3. Add the rules from 2.2 to 2.4 to the design. They apply to either cut.
4. Land the selector and the ancestor-based continuity in the domain crate with
   the permutation tests from design section 17. It is pure code and touches no
   I/O.
5. Cut the local store over first, then S3 and Dropbox, then the Library engine.
6. Leave the outbox, the archive UI and locators for when a provider needs them.

## 9. Follow-up: the owner's four answers (September 18, 2026)

| Answer | Verdict | Condition |
| --- | --- | --- |
| 1. Desktop first, fix Managed Cloud later | Workable | Once `ObjectStore` goes, the `managed_cloud` adapter no longer compiles, and the live service refuses the new manifest shape. On this branch the provider is unavailable, or its adapter is written against the new contract and tested only against a rewritten `stub.rs`. Ship the service change before the desktop build that needs it; the reverse order breaks every older desktop, because `client.rs` has no version negotiation |
| 2. A limit for late-written records | Works, as one writer rule | A reader-side time limit ("ignore a record older than N when it is discovered") does not work: it depends on each reader's clock and discovery time, so two devices compute different heads. See 9.1, which replaces the two-limit plan first written here |
| 3. Refresh compares with the final publication; rebase uses the publication version | Yes to both | Comparing is free; finding the final publication is the cost. See 9.2. For rebase the version answers "did the remote move, and forward?"; the scope digest still answers "was my scope touched?" |
| 4. Forks and pins use the publication version, not the snapshot | Yes | The record hash pins the snapshot transitively. See 9.3 |

### 9.1 The limit: list, then write

The first version of this section proposed two limits: a 60-second publish
deadline on the writer, and a "built-on" selection rule on the reader. The plan
below replaces both. It is now section 8.3 of the design.

**Why the first plan was the wrong shape.** The hazard has one cause: a writer
that prepares a record, keeps it, and sends it later. Revision 1 of the design
required that (persist the frozen record, resend identical bytes after a
restart). A deadline and a second arbitration rule treat the symptom. The
built-on rule also cost a guarantee: an excluded publication could be revived by
building on it.

**The rule.** Every attempt to write a publication record is the second half of
one step: list the stream; continue only if the record's parent is the selected
version and has no child; send the record once.

- Every attempt lists first. A retry is a new attempt, never a resend.
- The listing must be at most ten seconds old on the wall clock when the write
  starts. Rust's `Instant` on macOS reads `CLOCK_UPTIME_RAW`
  (`library/std/src/sys/time/unix.rs:62-67` in the 1.95 toolchain), which stands
  still during sleep.
- The write is one request with no transport retry. The S3 client is built with
  the SDK defaults (`s3_store.rs:1277-1292`, no retry policy set, so three
  attempts), and tokio timers also pause during sleep, so without this the SDK
  could resend a record after a wake. The Dropbox client already sets
  `retry(reqwest::retry::never())` (`dropbox_store.rs:338`); its record write
  should be one `files/upload` call, not a session start plus a batch commit
  with Mallard's own code in between.
- Nothing is kept. The record lives in memory for one operation. This removes
  the durable outbox and keeps Push stateless.

**The limit it gives.** A record is written only when a listing at most ten
seconds old showed its parent childless. Any sibling it can compete with was
written after that listing, or too recently for the listing to show it:

```text
fork window = adapter listing lag + 10 s listing freshness + one record request
```

The listing lag is zero on Local, S3/R2, Dropbox and D1, so the window is a few
seconds in practice and about a minute if a request stalls (the provider drops
an idle upload). Outside the window selection is final. The bound does not
depend on any clock being right, on the age of the record's timestamp, or on how
long a machine slept: a laptop that slept three days lists, sees a child, and
stops.

**What stays as designed.** The selector is unchanged (earliest sibling), so
"an excluded publication stays excluded" still holds and nothing can be revived.
No timestamp floor and no deadline constant are needed.

**What it asks of an adapter.** A completed publication append is visible to
every later listing of its stream, or the adapter states its worst-case lag,
which widens the window by that amount. If a future provider's lag proves large,
the built-on rule can be added then as a new `publication_policy` value; it is
not needed for the providers that exist.

### 9.2 Refresh and rebase

Without a head pointer, the final publication is only known after looking at the
stream's records. Two conditions make that one request:

1. **List one stream, not the storage.** `publications/<stream>/…`, never the
   shared collection.
2. **The listing carries the graph.** Each entry returns `publication_id`,
   `parent_id`, `created_at_ms` and `depth`; a key-addressed adapter puts them
   in the object name. Selection then needs no record reads. Without this a new
   device reads every record once (500 records at 0.3 to 0.8 s each on
   Dropbox), and a persisted local index is needed to avoid repeating it.

| Refresh | Requests |
| --- | --- |
| Selected version unchanged | 1 listing per 1,000 records (today: 1 head GET) |
| Selected version changed, or a cold start | listing + record + snapshot (today: 2) |
| Head ahead of this replica's base | no extra read (today: 1 uncached probe on every refresh) |

No persisted index, cursor or scan state is required. A mutable "latest" pointer
written without CAS does not help: a crash after the record write leaves it
stale, so a safe reader lists anyway.

Rebase at Push apply:

| Fresh selected version versus the reviewed one | Action |
| --- | --- |
| Same | publish on it |
| Reviewed version is an ancestor | each writable scope's digest still equals this replica's base digest: publish on the new one and carry the other scopes; otherwise Pull first |
| Not an ancestor, or the stream is empty | `Replaced` or `RemoteMissing`; stop |

### 9.3 Publication version as the only remote coordinate

`PublicationVersion { publication_id, record_sha256, depth }`; identity is the
first two fields, `depth` is the number the UI prints.

- It replaces the head coordinates in `RepositoryBase`, `ScopeBase.reviewed_in`,
  `MaterializationRecord`, `ConversationApplyReceipt`,
  `AcceptedConversationVersion`, `LibraryHeadCoordinate`, and in the IPC types
  `RestorePlan`, `PullReview` and `ConversationRevisionPins`.
- Continuity: same version is `InSync`; base is an ancestor of the selected
  version is `Advanced`; anything else is `Replaced`. Verify the hash link once
  by reading the record one level above the base.
- Two siblings share a depth, so "Version 12" is not unique. Never compare the
  number, and never compare `scope_revision`.
- The version says nothing about content. Two publications can carry the same
  scope bytes, so the per-scope verdict still starts with the scope digest.
- Re-reading the base snapshot for the three-way project-files compare becomes
  two reads (base record, then its snapshot) instead of one. Both are immutable
  and cacheable in process.

## 10. Follow-up: removing the storage UID

Verdict: it can be removed, and the design gets simpler. Nothing in Revision 2's
limit, refresh, or version pins depends on it. Code facts below were read from
the working tree; nothing was run.

### 10.1 What the UID does in the design, and what covers it without one

| Job in the design | Without a UID |
| --- | --- |
| A fresh namespace `.mallard/v1/<storage_uid>/`; "two independent Creates produce two different roots" (4.1) | The descriptor becomes constant bytes at a fixed key, so every Create writes the same bytes and needs no exclusivity. Two Creates at one destination converge on one storage, which is what happens today. With a UID they split silently, and the second device must list `.mallard/v1/` and pick a root |
| Root identity: the descriptor's locator and SHA-256 pinned in a local root binding (4.1, 10) | Every provider already has a provider-assigned destination identity, and the code uses it: `continuity_coordinates` is R2 jurisdiction, account and bucket (`r2.rs:568-578`), the Dropbox `namespace_id` (`dropbox.rs:148-155`), the canonical local directory (`local.rs:181-191`), Managed Cloud origin and account (`managed_cloud.rs:144-150`). `ensure_destination_available` refuses a second storage at the same destination (`storage_setup.rs:1231-1253`), and the Library pins a hash of the same coordinates (`resource_engine.rs:30-42`). An identifier-addressed provider has a folder id |
| Detect a recreated root "rather than silently continuing another incarnation" (17) | Publication versions already detect it per stream. Ids are random and pins carry the record hash, so a base from the old incarnation is never listed in the new one: `Replaced`, or `RemoteMissing` when the stream is empty (design 10.2). The UID adds detection only for streams this device holds no base in, where nothing needs protecting |
| `storage_uid` inside every record and envelope (6.1, 6.2, 6.4) | A whole-folder copy carries the UID with it, so the field never detected a clone. It only rejects files hand-copied from another storage into the same stream, which the trusted-writer rule of 6.3 already excludes |
| A cleared storage becomes a new incarnation with a new UID (13) | Clear stays in place, as today: it keeps the marker and resets link state (`storage_core/mod.rs:127-129`, `storage_catalog/persistence.rs:546-558`). Other devices read `RemoteMissing` or `Replaced` and recover through Pull and Push |
| Isolation from the old layout (14.1) | A wipe, plus the distinct `format` string. See 10.2 for why the fixed key isolates better than the UID path |

This is the same conclusion the project reached once before.
DESIGN_REMOTE_HISTORY_CONTINUITY.md considered "a repository epoch nonce" and
deferred it because it "adds a schema field" and "duplicates what the probe
already proves from existing state". Ancestry over publication versions proves
more than that probe did.

### 10.2 Conditions

1. **Keep the descriptor at today's marker key, `.mallard/_storage.json`, with the
   new `format`.** An old build then reads it and refuses ("storage marker format
   … is not supported", `marker.rs:33-39`). Under the UID layout the descriptor
   moves to `.mallard/v1/<uid>/storage.json`; an old build finds nothing at the
   old key and silently creates an old-layout storage beside the new one,
   because `ensure_storage_marker` creates on first open (`marker.rs:59-75`).
   The new `blobs/` and `publications/` folders do not collide with the old
   `repositories/` and `library/`.
2. **Version-based continuity is the only recreation detector**, so the
   `RemoteMissing` and `Replaced` rows of design 10.2 need a "cleared, then
   pushed again" fixture each.
3. **Clear deletes publications before blobs, deepest first, and loops until the
   listing is empty** (it already loops on the first page). An interrupted clear
   then leaves a valid older history whose blobs still exist, never a record
   with a missing parent or snapshot. A writer racing the clear is stopped by
   list-then-write: its parent is gone, so it ends with `RemoteChanged`, or it
   leaves an orphan the next pass deletes.

### 10.3 What removing it simplifies

- The second-device join problem from section 5 disappears. The discovery
  backend that exists today reads the fixed key once per bucket or namespace
  (`r2_setup/discovery.rs:115-139`, `dropbox_auth.rs:472-512`,
  `classify_storage_marker`). It needs a new format string, not a root listing,
  a root picker, or a `(destination, uid)` candidate shape.
- No root binding in `StorageConfig`, so no `sync_config` change, and settings
  stay locked for life.
- No UID provisioning step and no setup-journal entry for it.
- Clear storage keeps today's behaviour and UI, including the Library path.
- Records and envelopes lose a field, 4.3 becomes "each stream", and the
  acceptance row "Root re-creation or wrong binding" is covered by the
  continuity rows.
- Managed Cloud has no storage-root concept at all (identity is origin plus
  account; data lives in repositories), so the proposed
  `/v1/storage-roots/{root}/…` routes can stay repository-scoped in stage 5.

### 10.4 What is given up

- **A first publication that races a clear.** A record with `parent = null`,
  written in the one-request gap right after a clear finishes, can reference
  blobs the clear already deleted. The stream then selects a record whose
  snapshot is missing and reads `Unavailable` until the storage is cleared
  again. With UID namespaces that record would land in the dead root. The clear
  dialog already says to stop sync on all machines first.
- **Hand-merged storages.** If someone merges two storages' folders and both
  hold the same bundle, the two histories meet under one stream, the earlier
  first publication wins, and the other history becomes an archive. Nothing is
  deleted, and Pull then Push recovers.
- An incarnation id to print in diagnostics.

## 11. Follow-up: Revision 3 of the design (no UID, the smaller cut)

The owner chose to remove the storage UID (section 10) and to go with the
smaller cut (section 6). The design was rewritten as Revision 3 on
September 18, 2026. Earlier sections of this evaluation describe Revisions 1
and 2 and are kept as the record of how the design got here.

How the smaller cut was reconciled with the four earlier answers:

| Earlier answer | In Revision 3 |
| --- | --- |
| Desktop first, cloud later | Unchanged. The smaller cut keeps the manifest shape, `_objects/<sha256>` and `commit-<hex>` ids, so the service's validator, the website's manifest parsing, views and the release flow stay valid. Stage 5 shrinks to: store the record with the upload, add `previous_commit_id` and `created_at_ms`, list a repository's records, and adapt the desktop bridge. Who arbitrates on Managed Cloud is left as an open stage-5 decision, with "the service keeps its commit guard" recommended |
| The limit: list, then write | Unchanged, applied to the commit record through a new single-request `put_record` |
| Refresh compares with the final publication | One listing of `repositories/_commits/<bundle_id>/`. The graph facts are in the record's name, which shared code builds and parses, as it builds every key today. Steps 4 and 5 yield the same `BundleSnapshot` as `inspect_optional` |
| Forks use the publication version | `commit_id` is the identity and `revision` is the printed depth. Pins keep today's shape, `(revision, commit_id, manifest_sha256, manifest_size)`, because the smaller cut's point is that no persisted, IPC or TypeScript type changes. What changes is every comparison: identity and ancestry, never revision order. The link above a pinned base is verified with `previous_commit_id` plus `previous_manifest_sha256`, both already in the pin, so no record hash has to be pinned |

Points specific to the smaller cut:

- Project records move to one flat namespace, `repositories/_commits/<bundle_id>/`,
  in the position `_tags/` holds today. One listing of it replaces the tag index
  for bundle discovery without ever listing a payload key, and the tag's facts
  move into the record as `summary`. Library records stay inside each
  repository, because the Library never enumerates repositories.
- `BundleHead` and `LibraryResourceHead` survive as in-memory views of the
  selected record, so `snapshot.head.*` callers (27 in `service.rs`) are
  untouched.
- `ObjectStore` loses `compare_and_swap` only. `put_immutable` keeps its
  signature and loses its no-overwrite promise, which is safe because every key
  is content-addressed or contains a random `commit_id`. The existing stores may
  keep their native no-overwrite mechanics.
- The stores' listings must skip and report a name they cannot represent instead
  of failing, because a stream listing is now on every refresh.
- Clear stays in place and deletes records first.
- Deferred, stated in design section 18: opaque locators, the snapshot envelope,
  the payload table, `AppendStore`, and identifier-addressed providers.

Still unanswered from section 4 of this evaluation: the legacy-rejection paths,
deferring the archive UI, and the "same id, different bytes" blocked state.
Revision 3 keeps the design's original position on each.

## 12. Follow-up: the clean plan (September 19, 2026)

The owner answered the last three open points, and the design was rewritten as
one implementation plan with no revision history or rejected alternatives in
it. Code locations moved to a pointer table at its end.

| Answer | In the plan |
| --- | --- |
| Remove the legacy-rejection paths | No detection, message, conversion or test for data written by an older build. The marker's `format` constant still changes to `mallard-append-storage`, because it is part of the new format and costs one constant; nothing else knows an older format existed |
| Defer the archive UI; keep the loser's content | Unselected publications stay in the storage and nothing lists, restores or counts them. The selector returns the mainline only; there are no role enums. The existing `Replaced` flow (Pull and review, then Push) is the recovery. The user-visible change is three notices in existing surfaces: `RemoteChanged`, stored-but-not-current, and outcome unknown |
| Name records `<id>-<sha256>` | A record's name contains the SHA-256 of its bytes, and a parent is named by that same token. The same `commit_id` with different bytes is two publications, ranked by `(created_at_ms, commit_id, record_sha256)`, so no state blocks a stream. Names are self-certifying, and the ancestor walk is hash-linked without reading a record, so `previous_manifest_sha256` from Revision 3 became `previous_record_sha256` and the "link above the base" read disappeared. A name is at most 223 bytes |

One ordering fix made while writing the work plan: `managed_cloud` is
unregistered in the bundle-engine step, not the store step, because its bridge
commits on the head write that stops happening there.

## 13. Follow-up: the cost of `previous_record_sha256` (September 19, 2026)

Asked whether the parent digest hurts performance. It does not: the value comes
from the parent's name in a listing the Push already made, so it costs no request
and no read. Measured with quick scripts on this machine; the listing sizes are
estimates from representative entries, and no live provider was called.

| Item | Result |
| --- | --- |
| SHA-256 of a record for its own name | 0.66 microseconds at 1 KB, 22 microseconds at the 64 KiB cap |
| Name length | 223 bytes with the parent digest, 158 without |
| Listing growth from the parent digest | +65 bytes per record on S3/R2, +195 on Dropbox, whose JSON repeats the name three times |
| A full 1,000-record listing | S3/R2 486 KB instead of 421 KB (about 10 ms at 50 Mbps); Dropbox 1,137 KB instead of 942 KB (about 31 ms). A stream of 100 records pays 1 to 3 ms |

The digest stays in the name: every parent link stays exact, and the saving
would be a few milliseconds.

Two larger listing costs turned up. The owner adopted the first only:

1. **Adopted.** The local store's `list` reads and hashes every listed file to
   fill an ETag: 41.8 ms for 1,000 one-kilobyte records, against 1.2 ms for names
   alone. The plan now has `list_keys`, a keys-only listing that every record
   listing uses. Its default forwards to `list`, which is free on S3/R2 and
   Dropbox, and the local store overrides it to open no listed file. Storage
   clear keeps `list`, because it deletes exactly the ETag it listed.
2. **Not adopted.** Dropbox responses are uncompressed, because `reqwest` is
   built without its `gzip` feature. On 1,000 realistic names gzip would shrink a
   listing from 1,137 KB to 131 KB. It needs one live check that `list_folder`
   honours `Accept-Encoding`.

## 14. Follow-up: one listing against one head read (September 19, 2026)

Measured from this machine on September 19, 2026. The S3 figures are against a
public AWS bucket (`noaa-goes16`), anonymous, one reused connection, requests
interleaved, median of eight. R2 and Dropbox were not measured, because that
needs the owner's credentials.

| Store | 1 small read | List 10 | List 100 | List 1,000 |
| --- | --- | --- | --- | --- |
| AWS S3, public bucket | 101 ms (300-byte range GET) | 93 ms (3.6 KB) | 99 ms (32 KB) | 142 ms (314 KB), worst 401 ms |
| Local folder, names only (Python) | 0.05 ms | 0.05 ms | 0.15 ms | 1.3 ms; 13.8 ms at 10,000 |
| Dropbox | not measured; the app's logs give 0.27 s for the head read and 0.3 to 0.8 s per request | one request | one request, about 114 KB | one request, about 1.1 MB |

- Up to a few hundred records a listing costs the same as a head read: both are
  one round trip, and the round trip dominates.
- At 1,000 records the listing is about 1.4 times a head read on S3. The plan's
  entries are about 486 bytes against 314 in that bucket, so expect about 1.6
  times. Every further 1,000 records adds one sequential request.
- Request price: a listing is an R2 Class A operation and a read is Class B,
  12.5 times dearer per call. At 300 refreshes a day that is about 4 cents a
  month, inside R2's free allowance.

What this means for refresh under the plan:

| Refresh | Against today |
| --- | --- |
| Nothing changed | The same up to a few hundred records; about 1.5 times at 1,000; one more request per further 1,000 |
| Remote ahead of this replica's base | Cheaper: today's extra uncached probe read on every refresh is gone |
| A new remote publication, or the first refresh after launch | One request dearer: listing, record, manifest instead of head, manifest. About +0.1 s on an S3-like store and +0.3 to 0.8 s on Dropbox, from the logged per-request times |
| After this computer's own Push | No reads: the record and manifest it just wrote are in memory |

A way to win the extra request back, not adopted: read the record and the
manifest in one parallel round. The manifest's key follows from the record's
name, and its digest would be checked when the record arrives, which needs a
staged read that verifies after the transfer instead of during it.

## 15. Correction: the Library does not rebase at apply (September 19, 2026)

The plan's Push gate row read "every writable scope (or reviewed Library entry)
still has this replica's base digest", which implied that a Library Push rebases
onto a newer publication at apply. It does not today: `observed_matches` requires
the head at apply to equal the reviewed head and otherwise returns "the storage
head changed after review" (`src-tauri/src/storage_catalog/service.rs:1685-1697`,
`:1744-1751`). Its "observed set plus reviewed upserts" merge is therefore always
against the reviewed publication. The plan now says so: the scope gate is for
projects only, and the Library keeps its stricter rule.

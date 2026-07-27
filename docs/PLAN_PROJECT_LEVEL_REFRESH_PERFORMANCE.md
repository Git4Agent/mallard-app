# Project-level refresh performance plan

- Status: proposed, source-reviewed, not implemented
- Date: 2026-07-27
- Baseline: the current generation-history working tree on top of `ccd2275`

This is a clean replacement for the earlier refresh plan. It incorporates the
current generation-history model and closes the safety, cache-lifetime, memory,
and request-budget gaps found during review.

The generation-history work is treated as existing baseline:

- stored Codex titles are portable `ConversationThread.display_title` metadata;
- manifests carry `lineage_status`, `lineage_codec`, `head_message_id`,
  `message_count`, `lineage_digest`, and `semantic_digest`;
- manifests carry per-resource `resource_content_sha256`;
- mutable heads identify the current generation and pin immutable digests; and
- generation comparison and selected-version restore already consume immutable
  manifests and generation records.

No new cloud metadata is required for this refresh optimization.

## Decision summary

Implement the refresh as five coordinated changes:

1. Make History reconciliation fingerprint-based instead of forcing every
   transcript to be reconsidered.
2. Add a comparison-specific local conversation index so comparison does not
   run authoritative Push capture.
3. Classify conversations from manifest metadata only when the metadata proves
   the exact canonical relationship; otherwise retain the existing payload
   classifier.
4. Share bounded remote metadata and payload caches under a race-safe storage
   scope that changes when namespace or credentials change.
5. Stop ordinary project-page mount and post-Push refresh from starting
   unrelated inventory and readiness work.

Push remains authoritative and unchanged in its safety role: it still reopens,
validates, reads, hashes, and reclassifies every selected source.

## Scope

The primary target is the circular refresh action on the active project's
**Git & sessions** page. The same work also removes duplicate discovery and
remote inspection from a fresh project-page mount.

The normal refresh must:

- find new, changed, archived, deleted, and replaced local sessions;
- observe the selected branch and current linked-storage head;
- show stored thread titles without downloading transcripts;
- keep existing rows visible while revalidation runs;
- isolate local History errors from linked-storage comparison errors;
- coalesce duplicate requests only within the exact same UI context; and
- remain read-only.

The normal refresh must never publish, restore, materialize, delete, modify the
reviewed recipe, or use a display cache as publication authority.

## Non-goals

- changing the portable bundle schema;
- changing the top-level machine-state schema;
- adding a legacy schema or compatibility path;
- persisting transcript bytes, message text, or chat-derived titles in a new
  cache;
- changing the 30-day History presentation window;
- weakening root/subagent exclusion, secret detection, or collision checks;
- changing Pull planning or approval semantics;
- changing generation reachability rules; or
- optimizing unrelated application-shell loading.

## Current source-verified bottlenecks

### History

`ProjectChatHistoryPage` currently starts History and comparison separately.
The circular refresh calls History with forced revalidation, so a valid
privacy-clean `chat_history_cache.json` entry cannot provide the intended warm
path.

Within `chat_history.rs`:

- both `sessions` and `archived_sessions` are walked;
- `session_index.jsonl` is reopened and reparsed;
- an unindexed chat-derived title prevents full warm reuse because the
  persisted cache deliberately strips title text; and
- the bounded first-parent Git rail is obtained three times: branch discovery,
  thread mapping, and older-content detection.

### Local conversation comparison

`get_project_thread_sync_comparison_with_repository` currently:

1. calls `provider_capture::discover_full`;
2. performs bounded transcript-prefix reads for identity and classification;
3. calls `capture_from_discovery` for the selected local conversations;
4. rereads complete transcript bytes;
5. runs secret detection, SHA-256 hashing, and canonical lineage analysis; and
6. builds local versions from captured resources.

That is appropriate for Push authority, but excessive for a read-only status
comparison.

### Stored conversation comparison

`stored_thread_entries` currently obtains and analyzes a cloud transcript
before branching on whether the thread exists locally. This means a
storage-only thread can trigger an immutable object GET even though
`LocalMissing` is already known from the two inventories.

The current canonical relationship classifier compares:

- message IDs;
- parent IDs; and
- semantic fingerprints.

A match on IDs and parents alone is therefore insufficient. In particular,
`CloudPrefixOfLocal` can enable `FastForwardCloud`, so metadata must not claim
that relationship unless both lineage and semantic prefixes match.

### Remote reads and cache lifetime

`BundleEngine::open` validates the storage marker. Comparison then reads the
mutable head, current manifest, and sometimes a reviewed-base manifest.

The existing immutable `ObjectCache` is keyed by storage ID, bundle ID, and
content digest. Invalidating by storage ID after a configuration edit is not
enough: an in-flight request against the old namespace can finish after
invalidation and reinsert old data under the reused key. A credentials-only
change also needs a new cache lifetime even when namespace coordinates stay
the same.

Generation comparison already batches the two manifest misses through one
`get_many` call. A shared manifest cache must preserve that cold-path batching
instead of turning it into serial GETs.

### Project-page mount

`ProjectSyncV3.loadProjectData` eagerly loads project detail, provider
inventory, and readiness. History comparison then starts its own provider
discovery and remote inspection.

Inventory is authoritative only inside Push preparation. Readiness is needed
inside the Pull workflow for the inspected snapshot; it is not required to
render the project workspace or enable the Push/Pull entry points.

## Cross-cutting contracts

### 1. Source fingerprints

Use one internal `SourceFingerprint` for local History and comparison caches:

```text
SourceFingerprint
  canonical_profile_identity
  canonical_source_path
  logical_path
  size
  modified_nanos
  safe_mode
  platform_file_identity?
  platform_change_time?
```

On Unix, include device, inode, and change time when available. On platforms
without those fields, use the portable fields and preserve the explicit forced
revalidation path described below.

Fingerprint construction must use no-follow metadata and retain the existing
canonical-root, ancestor, symlink, special-file, hard-link, size, record-count,
and cumulative-byte checks.

For any full content read:

1. fingerprint before opening;
2. read and validate the bounded content;
3. fingerprint again;
4. accept only if the two fingerprints match; and
5. retry once after a change, then report the source unavailable.

### 2. Complete-scan reconciliation

Every profile walk returns both its entries and a completion state.

- Evict unseen file-cache entries only after a complete, successful, bounded
  walk of the relevant profile roots.
- A transient directory read error is not evidence that previously cached
  files were deleted.
- After an incomplete walk, retain unseen cache entries for a future retry but
  do not synthesize current History/comparison rows from entries that were not
  rediscovered.
- Continue surfacing per-source unavailability where safe.
- Invalid roots, collection-limit violations, and other global safety failures
  still fail the command.

This prevents both false deletion and ghost rows.

### 3. Explicit recovery mode

Normal circular refresh is cache-aware. Keep a separate recovery mode:

```text
RefreshValidationMode
  Normal
  ForceRevalidate
```

History already has a force-revalidation input. Add the equivalent internal
input to thread comparison and its typed frontend wrapper.

`ForceRevalidate` must bypass, for that operation:

- persisted History parse reuse;
- session-index and History display caches;
- conversation identity and local-version caches;
- shared canonical transcript analysis;
- marker, generation-record, manifest, and immutable payload caches.

The existing explicit **Reload app data** or diagnostic recovery path invokes
both commands in `ForceRevalidate` mode. A successful recovery may repopulate
the caches with newly validated entries.

This mode changes only cache use. It does not relax or expand publication,
restore, reachability, or approval authority.

### 4. Frontend refresh context

Capture one immutable context for a refresh generation:

```text
ProjectRefreshContext
  local_project_id
  binding_revision
  codex_profile_id
  codex_profile_revision
  active_storage_id?
  config_revision
  selected_branch
  refresh_epoch
```

The context is frontend-only and is part of the History and comparison
single-flight keys. It is not a Tauri DTO.

- Calls with the same context share one in-flight promise.
- A new click increments `refresh_epoch`.
- A branch, project, binding, profile, storage, or configuration change creates
  a different key.
- Each response is applied only if its captured context still equals the
  component's current context.
- Completed promises are evicted so a later refresh checks freshness.

### 5. Storage cache scope

Use one scope for every cache whose value came from a storage backend:

```text
StorageInstanceKey
  storage_id
  storage_kind
  namespace_fingerprint

StorageCacheScope
  instance_key
  storage_cache_epoch
```

`namespace_fingerprint` is SHA-256 over normalized, non-secret coordinates:

- local storage: canonical storage directory;
- S3-compatible storage: normalized endpoint, account ID, bucket, and region.

`storage_cache_epoch` is a process-local, monotonically increasing generation.
Advance it only after a successful change to:

- storage kind or transport;
- namespace coordinates;
- credentials or other authentication material; or
- storage removal and recreation.

The epoch is advanced even when the non-secret namespace fingerprint is
unchanged. Compare credential configuration only to decide that the epoch must
advance; never put a credential, credential hash, bearer token, or secret in a
key, diagnostic, or log.

Keep the latest per-storage epoch as a process-lifetime tombstone after
removal. Recreating the same storage ID, even with identical coordinates, must
receive a later epoch.

Every remote-derived cache key includes the captured
`StorageCacheScope`:

- marker: `(scope)`;
- generation record: `(scope, bundle_id, generation_record_sha256)`;
- manifest: `(scope, bundle_id, manifest_sha256)`;
- immutable object:
  `(scope, bundle_id, resource_content_sha256)`.

An in-flight read retains the scope captured when it started. If storage is
reconfigured while that read is running, its result can only populate the old
scope; a new request cannot observe it. Old-scope entries may be eagerly
invalidated and are also harmless under bounded eviction.

Canonical transcript analysis remains keyed by verified transcript SHA-256
only because it is a pure derivation from bytes that have already passed the
calling path's size and digest checks.

### 6. Aggregate cache budget

Add one process-wide `RefreshCacheRegistry` for the new process-local caches.
Use LRU eviction and estimated heap accounting, not entry count alone.

Initial hard resident-cache budgets:

| Cache area | Budget |
| --- | --- |
| All new History, identity, analysis, and remote-metadata caches combined | 128 MiB estimated heap |
| Canonical conversation analysis within that total | 200,000 canonical messages/records |
| All new cache entries combined | 4,096 entries |
| Existing immutable `ObjectCache` | retain its separate 256 MiB byte budget |

The resulting cache capacity is bounded to 384 MiB: 128 MiB for new refresh
caches plus the existing 256 MiB immutable-object cache. Existing per-file,
record-count, and cumulative-read bounds still constrain transient work.

Every cache entry implements a conservative `estimated_heap_bytes` calculation
that includes owned `String`, `Vec`, map, and canonical-message storage.

Use one shared `TranscriptAnalysisCache` for:

- local comparison;
- verified stored transcripts; and
- Push's optional pure parse reuse after authoritative byte hashing.

The local identity/version cache stores the transcript SHA and scalar metadata,
not a second lineage vector. A command obtains an
`Arc<TranscriptAnalysis>` from the shared analysis cache for its lifetime.
If that analysis was evicted, the caller safely rereads/revalidates the source
or verified object and rebuilds it. Oversize entries are usable for the current
bounded operation but are not admitted to the cache.

Do not store transcript bytes, chat text, semantic message text, credentials,
or absolute paths in diagnostics. Eviction may reduce performance only; it
must never change a classification or authorize an action.

## Target refresh architecture

History and thread comparison remain separate backend commands. This preserves
local History when linked storage is unavailable. The frontend coordinates
their lifecycle with one `ProjectRefreshContext`.

### A. Incremental History

#### Persisted and process-local cache behavior

Keep `chat_history_cache.json` privacy-clean. Increment its internal
`CHAT_CACHE_SCHEMA` and add only:

```text
has_derived_title: boolean
```

Do not persist the derived title.

Add:

- `SessionIndexCache`, keyed by the index `SourceFingerprint`, containing the
  normalized title/update map; and
- `HistoryDisplayCache`, keyed by transcript `SourceFingerprint`, containing
  the process-local chat-derived display title.

Title resolution remains:

1. indexed title;
2. first real user-message-derived title;
3. `Codex thread <short-id>`.

Warm reuse rules:

- indexed title: use the persisted parse plus current index metadata;
- no indexed title and `has_derived_title = false`: use the persisted parse and
  generic fallback;
- no indexed title, `has_derived_title = true`, display-cache hit: use the
  process title without opening the transcript;
- display-cache miss: parse the transcript once and repopulate the process
  cache.

Track whether the persisted cache changed. Rewrite it only after schema rebuild,
insertion, replacement, authoritative deletion reconciliation, or ownership
metadata change.

#### One Git rail

Retain one bounded first-parent commit rail for the command and reuse it for:

- the visible commit page;
- session-to-commit mapping;
- `next_cursor`; and
- the older-content decision.

Do not invoke `git log --first-parent` again inside the same History request.

#### History algorithm

1. Capture the project/profile/branch context.
2. Fingerprint and parse `session_index.jsonl` only on a cache miss.
3. Walk `sessions` and `archived_sessions` with existing bounds.
4. Reuse privacy-clean parses and process display titles by fingerprint.
5. Parse only changed, uncached, or force-revalidated transcripts.
6. Reconcile deletions only after a complete walk.
7. Load one first-parent Git rail and build the response.
8. Save the persisted cache only if its content changed.

Apply the 30-day presentation window after complete reconciliation, as today.

### B. Comparison-specific local discovery

Do not call `discover_full` or `capture_from_discovery` from
`get_project_thread_sync_comparison_with_repository`.

Add a read-only provider path such as:

```text
derive_local_conversation_versions
  -> local versions
  -> unavailable resources
  -> warnings
  -> scan completion state
```

The per-request result is:

```text
LocalConversationVersion
  resource_id
  source_thread_id
  display_name
  transcript_sha256
  source_mtime
  record_count
  analysis: Arc<TranscriptAnalysis>
  secret_warning?
```

`transcript_sha256` is exactly `SHA256(transcript_bytes)` and must equal the
manifest transcript file entry's `sha256`. It is not
`CapturedResource.content_sha256` or `provider_capture::resource_digest`,
which also incorporate resource framing such as logical path or mode.

The long-lived local cache entry contains:

- `SourceFingerprint`;
- provider thread ID and source cwd identity;
- root/subagent classification;
- transcript SHA-256 when derived;
- source mtime and record count;
- secret-warning classification; and
- the SHA key for shared transcript analysis.

It contains no transcript bytes, message text, or duplicate lineage vector.

Algorithm:

1. Walk the provider roots with existing no-follow and collection bounds.
2. Build each current `SourceFingerprint`.
3. On an identity hit, evaluate cached project ownership and root/subagent
   class without opening the prefix.
4. On an identity miss, run only the bounded prefix classifier. Cache identity
   for unrelated-project and subagent rollouts too.
5. Do not read full content for unrelated-project or subagent rollouts.
6. For a project-owned root rollout, reuse shared analysis by transcript SHA
   when both fingerprint and cache entry are valid.
7. Otherwise read once, validate, hash, derive canonical analysis, and recheck
   the fingerprint.
8. Reconcile vanished paths only after a complete profile walk.

Preserve the existing failure taxonomy:

- an unreadable, oversized, hard-linked, repeatedly changing, or individually
  invalid rollout becomes unavailable where safely attributable;
- a subagent is excluded from user-visible local resources;
- invalid roots, collection-limit failures, and storage errors fail the
  command;
- collisions remain global unless both sources can be safely isolated; and
- Push still fails closed if a source changes classification.

### C. Exact metadata-first relationship classification

Restructure `stored_thread_entries` so inventory presence is decided before any
cloud transcript lookup.

#### Storage-only branch

If a cloud thread has no local counterpart:

- relationship is `LocalMissing`;
- UI state is `StorageOnly`;
- no `_objects` GET and no transcript analysis occurs.

Derive `cloud_safely_readable` and destination resumability from the already
verified manifest:

- the transcript resource entry and its bounded digest metadata must be valid;
- `lineage_status` must be `Complete`;
- `lineage_codec` must be supported by the current client; and
- the complete lineage fields must satisfy their domain invariants.

When lineage is incomplete, unknown, or uses an unsupported codec, keep the
thread visible as storage-only and expose only the safe StoreOnly capability.
Do not download the payload merely to improve the storage-only card.

#### Both-sides-present branch

Apply this decision order:

1. If local transcript SHA-256 equals the manifest transcript resource SHA-256,
   classify `Identical`. No payload GET and no canonical comparison is needed.
2. Otherwise, attempt the strict manifest-prefix proof below.
3. If the proof succeeds and local canonical length equals stored
   `message_count`, classify canonical `Identical`. The raw bytes are a
   structural rewrite of the same canonical conversation.
4. If the proof succeeds and local canonical length is greater than stored
   `message_count`, classify `CloudPrefixOfLocal`.
5. In every other both-sides-present case, fetch the verified cloud transcript,
   analyze it through the shared SHA cache, and invoke the existing canonical
   payload classifier.

The strict manifest-prefix proof is eligible only when all conditions hold:

- stored `lineage_status` is `Complete`;
- stored `lineage_codec` equals the current supported local codec/version;
- `head_message_id`, `message_count`, `lineage_digest`, and `semantic_digest`
  are present and pass domain validation;
- `message_count` equals the length of the local prefix ending at the located
  stored head;
- the stored head is the final message of that exact prefix;
- the recomputed local prefix `lineage_digest` equals the stored value; and
- the recomputed local prefix `semantic_digest` equals the stored value.

Do not prove `CloudPrefixOfLocal` from head ID and lineage digest alone.
Matching parent structure with different semantic fingerprints must fall
through to the payload classifier and may become `Unknown`.

The payload fallback is required for possible local-prefix-of-cloud, divergence,
semantic mismatch, missing/unknown lineage, unsupported codec, inconsistent
message count, or any other unproved relationship.

This preserves the existing action mapping, including the safety boundary that
only a proven `CloudPrefixOfLocal` may offer `FastForwardCloud`.

### D. Push authority

Keep `discover_full` plus `capture_from_discovery` as the only publication
authority.

Push must still:

- reopen every selected source;
- perform before/after fingerprint validation;
- reread and SHA-256 hash authoritative bytes;
- enforce size/count bounds;
- run secret detection and warning acknowledgement;
- verify logical-path and resource-ID collision rules; and
- revalidate project ownership and root/subagent classification.

After Push has read and hashed the authoritative bytes in the current call, it
may reuse pure `TranscriptAnalysis` by that transcript SHA. This avoids a
second parse but does not allow local comparison or History cache hits to
authorize publication.

### E. Remote metadata and immutable-object reuse

Use three bounded cache families under `StorageCacheScope`:

- marker validation;
- intrinsic immutable metadata: generation records and manifests;
- immutable payload bytes via the existing `ObjectCache`.

Always reread and validate the mutable head for normal refresh.

The mutable head pins `generation_record_sha256`; it does not authenticate that
the record exists or that its contents are valid without reading and verifying
the record. Ordinary current-head refresh needs the head and current manifest,
so it does not need a `_versions` read. Generation listing, selected-generation
restore, comparison, and reachability validation must continue reading and
verifying the required generation records.

#### Manifest cache validation levels

Name the shared entry an intrinsically validated parsed manifest, not a generic
"verified manifest." Cache admission requires:

- bounded bytes;
- expected manifest SHA-256;
- successful parse and schema validation;
- intrinsic manifest digest/resource-map consistency; and
- matching bundle identity.

Every consumer must still apply its own external pin:

- current-head read: head generation, commit, manifest SHA, and snapshot digest;
- `BundleVersionRef`: reachable generation record plus generation, commit,
  manifest SHA, generation-record SHA, and snapshot digest;
- reviewed-base read: its pinned generation, commit, manifest SHA, and snapshot
  digest.

A cache hit must never bypass reachability, reviewed-plan binding, generation,
digest, expiry, or CAS validation.

#### Batch cache misses

Provide bulk helpers that:

1. resolve all requested manifest keys;
2. take cache hits immediately;
3. collect all misses;
4. issue one `get_many` call for the misses;
5. validate each result independently; and
6. restore results to the caller's requested order.

Route `read_manifest_pair` through this helper. A cold two-generation compare
must remain one bulk store call, not two serial manifest fetches. The same
helper may serve future multi-generation views.

#### Remote refresh request behavior

For current-head comparison:

1. capture the current `StorageCacheScope`;
2. construct/open storage from the same configuration;
3. validate the marker on the first use of that scope;
4. always GET and validate the mutable head;
5. load the current manifest through the scoped intrinsic cache;
6. load a reviewed-base manifest only if the comparison needs it; and
7. fetch cloud transcript payloads only for both-sides-present relationships
   that the strict metadata proof cannot decide.

Any corrupt, oversized, hash-mismatched, or coordinate-mismatched entry fails
closed and removes the affected scoped entry before returning the error.

After warm-up, an unchanged comparison performs exactly one mutable-head GET
and no marker, manifest, generation-record, base-manifest, or transcript GET.

### F. Project-page mount

Narrow `ProjectSyncV3.loadProjectData` to the project detail required to render
the workspace.

- Do not eagerly call `get_bundle_inventory`.
- Do not eagerly call `get_bundle_readiness`.
- Let `prepare_push_review` own authoritative inventory and discovery.
- Let Pull review support own readiness for its already inspected snapshot.
- Keep post-Pull readiness work inside the Pull workflow.
- Narrow `refreshAfterPush` to config, project/link state, and History refresh;
  do not start a standalone readiness inspection.

Do not add an inventory/readiness request to `ProjectLinksWorkspace`.
Provider/resource counts remain absent or unknown outside the workflow that
loads them. This first implementation does not back-propagate Push's temporary
inventory into page-load state.

Expected cold mount: project detail, one incremental History request, and one
thread-comparison request. No parallel inventory discovery or readiness
inspection.

## Implementation phases

### Phase 1: Measurements and cache foundations

Add sanitized per-command measurements:

- files visited;
- cache hits/misses/evictions and estimated resident bytes;
- prefix/full transcript files and bytes read;
- session-index bytes parsed;
- Git subprocess count and duration;
- provider walk and local-version derivation duration;
- marker/head/manifest/generation-record/object request counts;
- metadata-only versus payload-based relationship counts; and
- total duration.

Emit at most one throttled aggregate event per command, following the existing
`push.capture_complete` pattern. Log counts, byte totals, and durations only.
Never log paths, locators, object keys, IDs, titles, credentials, or content.

Implement:

- shared `SourceFingerprint`;
- scan completion state;
- `RefreshCacheRegistry` and LRU accounting;
- shared SHA-keyed `TranscriptAnalysisCache`;
- `StorageCacheScope` and epoch lifecycle; and
- `Normal`/`ForceRevalidate` behavior.

### Phase 2: Incremental History and frontend coordination

- stop circular refresh from forcing History revalidation;
- add session-index and display caches;
- add `has_derived_title` to the internal persisted cache format;
- avoid unchanged cache rewrites;
- reconcile deletion only after complete walks;
- reuse one Git first-parent rail; and
- add context-scoped History/comparison single-flight and stale-response
  rejection.

### Phase 3: Local conversation comparison

- add comparison-specific provider discovery;
- cache identity before prefix reads;
- derive exact raw transcript SHA and shared canonical analysis;
- preserve typed source/global failures;
- add the storage-only no-download branch;
- implement the strict lineage-plus-semantic metadata proof; and
- retain payload fallback for every unproved both-sides-present relationship.

### Phase 4: Scoped remote reuse

- route marker, generation-record, manifest, and `ObjectCache` keys through
  `StorageCacheScope`;
- advance the epoch after successful namespace/auth/removal/recreation changes;
- preserve per-consumer manifest validation;
- preserve batched cold manifest reads; and
- add race and request-budget coverage.

### Phase 5: Remove duplicate mount work

- remove inventory/readiness from ordinary project loading;
- retain inventory in Push preparation;
- retain readiness in Pull support;
- narrow post-Push reconciliation; and
- verify Push/Pull entry points remain available.

## File-by-file change map

### Frontend

`src/components/project-sync/ProjectChatHistoryPage.tsx`

- build and capture `ProjectRefreshContext`, including selected branch;
- make normal circular refresh cache-aware;
- coordinate one refresh epoch across History and comparison;
- retain stale rows during refresh;
- keep local and storage errors independent; and
- reject obsolete responses.

`src/components/project-sync/api.ts`

- include the frontend-only context in both single-flight keys;
- add typed comparison `forceRevalidate` support;
- evict completed promises; and
- keep the context out of Tauri arguments.

`src/components/project-sync/ProjectLinksWorkspace.tsx`

- pass branch, active storage ID, config revision, and provider-profile
  revision needed for the context;
- do not add inventory or readiness loading.

`src/components/project-sync/ProjectSyncV3.tsx`

- pass the complete refresh context inputs;
- remove inventory/readiness from ordinary project loading;
- keep authoritative inventory in Push preparation;
- keep readiness in Pull support; and
- narrow post-Push reconciliation.

No serialized frontend response DTO change is expected.

### Backend

`src-tauri/src/project_sync_v3/source_fingerprint.rs` and `mod.rs`

- define filesystem-only fingerprint and scan-completion helpers;
- preserve no-follow and platform-specific safety fields; and
- keep Tauri, provider parsing, and cloud concerns outside the helper.

`src-tauri/src/project_sync_v3/chat_history.rs`

- add sanitized scan statistics;
- increment `CHAT_CACHE_SCHEMA` for `has_derived_title`;
- add bounded session-index and display caches;
- use shared fingerprint and recovery mode;
- skip unchanged persisted writes;
- reconcile only after complete walks; and
- reuse one Git rail.

`src-tauri/src/project_sync_v3/provider_capture.rs`

- add comparison-specific cached discovery;
- cache identity before prefix reads;
- derive exact transcript SHA and shared analysis;
- preserve source/global failure boundaries; and
- leave authoritative Push discovery/capture intact.

`src-tauri/src/project_sync_v3/commands.rs`

- consume derived local conversation versions;
- branch on local presence before any stored transcript fetch;
- implement strict lineage and semantic prefix proof;
- expose normal/forced comparison behavior;
- own or access the shared refresh-cache registry;
- assign/advance storage cache epochs after successful configuration changes;
- preserve metadata-only stored titles; and
- emit sanitized comparison statistics.

`src-tauri/src/project_sync_v3/bundle_engine.rs`

- scope marker, generation-record, manifest, and object caches;
- separate intrinsic manifest validation from consumer pin validation;
- always reread mutable heads;
- batch manifest misses through `get_many`; and
- preserve all size, SHA-256, schema, snapshot, reachability, and coordinate
  checks.

`src-tauri/src/project_sync_v3/s3_store.rs`

- keep the transport contract unchanged;
- extend the loopback harness for request counts, delayed reads, and
  reconfiguration races.

## Safety and schema impact

Portable bundle schema impact: none.

Top-level machine-state schema impact: none.

Internal cache impact:

- `CHAT_CACHE_SCHEMA` increments;
- old private History cache files are discarded and rebuilt;
- all new title, comparison, transcript-analysis, marker, generation-record,
  and manifest caches are process-local and bounded; and
- storage epochs are process-local, non-secret lifecycle values.

The cache registry is an optimization boundary only. It is not portable bundle
input and cannot authorize Push, Pull, restore, deletion, or CAS updates.

## Verification plan

### Frontend integration

Extend `tests/frontend/project-chat-history.integration.test.tsx`:

- circular refresh uses `Normal`, not forced History revalidation;
- one context shares one History/comparison promise;
- branch, binding, profile, active storage ID, config revision, or
  refresh-epoch changes do not share;
- obsolete responses are ignored;
- rows remain rendered while refreshing;
- History can succeed when comparison fails and vice versa; and
- explicit recovery invokes forced behavior for both commands.

Extend `tests/frontend/project-v3-cutover.integration.test.tsx`:

- ordinary project mount calls neither inventory nor readiness;
- Push preparation still obtains authoritative inventory;
- Pull support still obtains readiness;
- Pull remains available after the first Push; and
- post-Push refresh does not perform standalone readiness.

### History tests

Add or extend Rust tests for:

- unchanged indexed transcripts reuse persisted parses;
- unchanged `session_index.jsonl` is neither reopened nor reparsed;
- generic fallback needs no transcript parse;
- a derived title parses once after a cold display-cache miss and reads zero
  transcript bytes on the next refresh;
- persisted cache contains no title, summary, or message text;
- unchanged refresh does not rewrite the persisted cache;
- one changed file alone reparses;
- replacement and metadata-preserving rewrite invalidate where platform
  identity/change time is available;
- changed-during-read retries once, then becomes unavailable;
- complete walks reconcile archive/deletion;
- incomplete walks neither delete cache entries nor render unseen ghost rows;
- one History command runs one first-parent `git log`; and
- forced recovery bypasses persisted, index, and display caches.

### Local comparison and relationship tests

Add or extend Rust tests for:

- unchanged comparison reads zero prefix and full local transcript bytes while
  its analysis remains cached;
- analysis eviction causes a safe reread/reparse and the same result;
- one changed project-owned rollout alone is reread;
- unrelated-project and subagent misses read bounded prefixes only;
- `transcript_sha256` equals raw transcript SHA and the manifest file SHA;
- it is not confused with framed captured-resource digest;
- local derived lineage matches authoritative capture;
- equal raw transcript SHA classifies `Identical` with zero object GETs;
- complete compatible metadata with matching count, lineage digest, and
  semantic digest classifies canonical `Identical` when lengths are equal;
- the same proof classifies `CloudPrefixOfLocal` only when local is longer;
- matching IDs/parents with a semantic mismatch does not prove a prefix;
- incomplete lineage, unsupported codec, wrong count, or wrong head does not
  prove a prefix;
- remote-ahead and divergent threads use payload fallback and match the
  existing canonical classifier;
- any number of storage-only threads produces zero `_objects` GETs;
- unknown-lineage storage-only threads remain visible with StoreOnly capability
  and zero `_objects` GETs;
- Push analysis reuse produces the same manifest as uncached analysis while
  still rereading/re-hashing bytes; and
- Push rejects a source that changes or becomes a subagent.

### Storage scope, batching, and request tests

Add loopback and unit tests for:

- after warm-up, unchanged comparison performs one head GET only;
- a changed head fetches one verified current manifest plus only required
  ambiguous-thread payloads;
- a lagging reviewed-base manifest is fetched once and reused;
- refresh, generation comparison, and selected-version restore share intrinsic
  manifest bytes but each repeats its required external-pin validation;
- current-head refresh does not read `_versions`;
- generation listing/restore still reads and verifies required generation
  records and reachability;
- cold `read_manifest_pair` submits all misses in one `get_many` call;
- warm `read_manifest_pair` performs no manifest GET;
- changing namespace under the same storage ID cannot reuse old entries;
- changing credentials with the same namespace advances the epoch;
- removal/recreation advances the epoch;
- a slow old-scope GET, followed by reconfiguration and old completion, cannot
  populate or satisfy the new scope;
- corrupt, oversized, missing, or digest-mismatched metadata fails closed;
- cache keys cannot cross scope or bundle boundaries; and
- failed/incomplete profile walks do not authoritatively evict entries.

### Cache-budget tests

Use synthetic maximum-sized analyses and mixed cache populations to verify:

- total new resident estimated heap stays at or below 128 MiB;
- cached canonical messages/records stay at or below 200,000;
- entry count stays at or below 4,096;
- LRU order is deterministic under the test clock;
- local and stored analysis users receive the same shared allocation;
- the local version cache does not own a duplicate lineage vector;
- oversize entries are used but not admitted;
- eviction changes only read/parse counts, never results or available actions;
  and
- secrets, transcript bytes, and chat text are absent from cache diagnostics.

### Handoff checks

Run:

```sh
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Then use only temporary project, provider-home, Mallard-home, and storage
fixtures in the complete Tauri application:

1. record a cold mount;
2. refresh twice without changes;
3. append one fixture Codex record and refresh;
4. archive and delete fixture sessions and refresh;
5. add many cloud-only fixture threads and verify zero object reads;
6. publish a new test generation and refresh;
7. edit namespace and credentials while retaining the storage ID;
8. complete a deliberately delayed old-scope read after reconfiguration;
9. force cache-budget eviction and repeat comparison;
10. make storage unavailable and confirm local History remains usable; and
11. record only sanitized timings, byte counts, cache totals, Git counts, and
    storage request counts.

Never test against real `~/.codex`, `~/.claude`, `~/.mallard`, or production
storage.

## Acceptance criteria

### Correctness and safety

- one refresh reconciles new, changed, archived, deleted, and replaced fixture
  sessions;
- one refresh observes a new remote generation and manifest title;
- storage-only threads never require transcript downloads;
- no lineage shortcut ignores semantic fingerprints or message count;
- unknown metadata falls back safely without inventing ancestry;
- storage reconfiguration and in-flight old reads cannot cross cache scopes;
- branch/binding/profile/storage context changes cannot apply old responses;
- local and storage failures remain independently visible;
- forced recovery bypasses all refresh-status caches; and
- Push retains authoritative reread, hash, secret, collision, ownership, and
  root/subagent validation.

### Warm-path work

- History reads zero unchanged transcript and session-index bytes;
- History does not rewrite an unchanged persisted cache;
- comparison reads zero local prefix/full transcript bytes while entries remain
  cached;
- identical, canonical-identical, proven local-ahead, and every storage-only
  thread require zero `_objects` GETs;
- ambiguous both-sides-present threads alone may fetch payloads;
- one History request runs one first-parent Git query;
- an unchanged linked-storage refresh performs one mutable-head GET and no
  other storage request after warm-up; and
- duplicate frontend calls are coalesced only within the exact same complete
  refresh context.

### Performance targets

For a representative one-project fixture:

- warm local History refresh: under 500 ms;
- warm R2-linked project refresh: under 1.5 seconds at typical network latency;
- one changed small session: under 2 seconds;
- unchanged transcript bytes read: zero; and
- storage-only cloud transcript object reads: zero.

These are engineering targets, not product guarantees. Keep sanitized
measurements until repeatable fixture runs show stable headroom.

## Suggested commit sequence

1. `Measure project refresh work`
   - sanitized phase metrics and request/cache counters.
2. `Add scoped refresh cache foundations`
   - source fingerprints, complete-scan reconciliation, aggregate LRU,
     shared analysis, storage scope/epoch, and forced recovery.
3. `Make project history refresh incremental`
   - privacy-safe History caches, one Git rail, complete frontend context, and
     History tests.
4. `Cache local thread comparison metadata`
   - comparison discovery, storage-only early branch, exact metadata proof,
     payload fallback, and Push analysis reuse.
5. `Reuse scoped remote bundle metadata`
   - intrinsic manifest cache, consumer-specific validation, `get_many`
     batching, object scope, race tests, and request budgets.
6. `Defer project inventory and readiness`
   - narrow mount and post-Push reconciliation with frontend coverage.

Each commit should pass the smallest relevant checks. Run the full handoff set
after the final commit.

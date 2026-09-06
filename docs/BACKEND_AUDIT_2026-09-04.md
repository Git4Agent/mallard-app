# Mallard backend audit — performance, over-engineering, redundant checks

- **Audit date:** 2026-09-04
- **Baseline:** branch `extensible-providers`, `HEAD` = `a5a47a1`, plus the 89
  modified/untracked Rust files in the working tree
- **Scope:** all production Rust in the workspace — `mallard-domain`,
  `mallard-infrastructure`, `mallard-application`, `mallard-cli`, and the
  `src-tauri` shell (`project_sync/`, `storage_catalog/`). ~35k production
  lines out of ~147k total.
- **Focus, as requested:** performance, over-engineering, redundant checks.
  Correctness and security were not re-audited; see
  [`BACKEND_AUDIT_2026-09-01.md`](BACKEND_AUDIT_2026-09-01.md) for that.
- **Method:** static call-graph tracing, then **release-mode probes that link
  the real crates** to measure the paths in question. Every headline number
  below was measured on this host, not estimated. Probe sources are in the
  appendix.
- This is **audit only** — no source file was modified.

---

## Verification baseline

| Command | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | exit 0, 25.4 s |
| `cargo test --workspace --lib` | **776 passed, 0 failed, 3 ignored** |
| `cargo clippy --workspace --all-targets` | exit 0, **2 warnings** (see O7) |

The tree builds and every test passes. The two clippy warnings are new since
2026-09-01 and are recorded as O7. Everything else in this report is work the
compiler and the suite cannot see.

## Measurement environment

arm64 macOS 26.6.0, APFS internal SSD. All filesystem probes were run on the
**same volume as `~/.mallard`** (an earlier run under `/private/tmp` reported
`symlink_metadata` at 15 µs against 0.8 µs on the real volume; those numbers
were discarded). Absolute microseconds are dev-host figures — use the
**ratios and the operation counts**, which are machine-independent.

Reference costs measured on this host:

| Operation | Cost |
| --- | ---: |
| `symlink_metadata` on a warm path | **0.8 µs** |
| `read_json_bounded` shape (3 stats + open + read) | **37 µs** |
| `persistence_guard` shape (open + fstat + `fchmod`) | **54 µs** |
| `sync_directory` — one directory `fsync` | **47 µs** |
| `write_json_atomic` of a 33 KiB document (2 `fsync`s) | **6.37 ms** |

The last row is the number that matters most: **an atomic JSON write costs
about 6.4 ms**, and several of them happen on paths that had nothing to write.

---

## Executive assessment

The backend's *algorithms* are mostly fine and its *safety* posture is
genuinely careful. What it lacks is thrift about the boundary between the
in-memory world and the disk. Three patterns account for nearly all of the
avoidable cost:

1. **State documents are re-read, re-validated, and re-locked per call site
   rather than per operation.** One Activity refresh performs **eight**
   separately locked state loads — four of them the same `sync_config.json` —
   when a batched snapshot API already exists in the codebase and is used by
   nothing but tests. Measured: **3.28 ms of pure ceremony per refresh on a
   state root with no documents on disk at all** (P2).
2. **Durability barriers on read paths and unconditional writes on unchanged
   data.** Every identity load `fsync`s the state directory; every Activity
   refresh rewrites the whole local activity cache whether or not anything
   changed (**6.4 ms**, and the cache is allowed to reach 32 MiB) (P1/P2).
3. **One genuinely quadratic loop on the Push apply path.** The retained-file
   mutation firewall scans the previous scope linearly per retained file.
   Measured **1.14 s at 20,000 files against 5.7 ms for the indexed form —
   201×** — and the domain cap is 100,000 files (P1).

On over-engineering: the codebase is disciplined about most seams — the
provider registry, the catalog adapters, and the storage-provider trait all
have two or more real implementations and earn their indirection. The
exception is `portable_resources/`, a four-axis extensible contract registry
whose three principal traits have **exactly one production implementation
each**, and two of whose eight query methods have no callers anywhere (O1).

Five `pub fn`s are dead with zero references in the entire tree, and a further
set — including the batched loader in point 1 — is reachable only from tests.

On redundant checks: most repeated validation is deliberate defence in depth
and worth keeping (the redaction fixpoint gate is correctly gated and should
stay). Three are strictly redundant: an unconditional `fchmod` that doubles
the persistence-guard cost, a `parent.exists()` fully subsumed by the
`ensure_real_directory` call two lines later, and a duplicate `stat` of a path
`ensure_no_symlinks` just walked.

### Headline numbers

| | |
|---|---|
| Findings | **20** (3 P1, 8 P2, 9 P3) |
| Avoidable cost per Activity refresh, measured | **≈ 10 ms** before any real work |
| Locked state loads per Activity refresh | **8** (4 of them the same document) |
| Push retention firewall at 20k files | **1.136 s → 5.7 ms** (201×) |
| `pub fn` with zero references in the tree | **5** |
| Traits with exactly one production impl | **3** (all in `portable_resources/`) |

---

## Priority key

- **P1** — can dominate a normal workflow, or grows superlinearly inside a
  supported bound.
- **P2** — repeated work on a routine path; fixed cost paid every time.
- **P3** — waste worth removing when the surrounding code is next touched.

---

# A. Performance

## A1 (P1) — The Push retention firewall is quadratic

**[`bundle_engine.rs:787-799`](../crates/mallard-infrastructure/src/bundle_engine.rs#L787)**

```rust
for (path, entry) in &input.retained_files {
    entry.validate()?;
    let was_published = previous_scope.is_some_and(|scope| {
        scope.files.values().any(|candidate| {
            candidate.object_key == entry.object_key
                && candidate.sha256 == entry.sha256
                && candidate.size == entry.size
        })
    });
```

For every retained file it linearly scans every file in the previous scope,
comparing two `String`s and a `u64` per pair. `retained_files` and
`previous_scope.files` are the same order of magnitude — the published file
set — so this is **O(n²)** with a heavy constant. `MAX_FILES` is
**100,000** ([`lib.rs:37`](../crates/mallard-domain/src/lib.rs#L37)).

Measured (release, this host):

| Files in scope | Current (linear scan) | Indexed set | Speedup |
| ---: | ---: | ---: | ---: |
| 1,000 | 6.48 ms | 0.47 ms | 13.8× |
| 5,000 | 71.3 ms | 1.28 ms | 55.6× |
| 10,000 | 276 ms | 2.29 ms | 121× |
| 20,000 | **1.136 s** | **5.65 ms** | **201×** |

Extrapolating quadratically to the 100,000-file cap: roughly **28 seconds**,
inside a Push the user is watching.

**Fix.** Build the identity set once per scope, then look up:

```rust
let published: BTreeSet<(&str, &str, u64)> = previous_scope
    .map(|scope| scope.files.values()
        .map(|c| (c.object_key.as_str(), c.sha256.as_str(), c.size)).collect())
    .unwrap_or_default();
// then: published.contains(&(entry.object_key.as_str(), entry.sha256.as_str(), entry.size))
```

Semantics are identical — the check is "was this exact object published
anywhere in this scope", and a set membership test answers exactly that.
Low risk; it is a pure hoist.

## A2 (P1) — The local activity cache is rewritten on every refresh, changed or not

**[`project_activity.rs:471-486`](../crates/mallard-infrastructure/src/project_activity.rs#L471)**

`scan_local_session_views` ends with an unconditional `write_json_atomic` of
the whole cache document. On a warm refresh where every entry was a cache hit
and nothing was inserted or retained away, the file is still serialized,
written to a temp file, `fsync`ed, renamed, and the directory `fsync`ed again.

- Measured cost at the **real** current size of `~/.mallard/activity_cache.json`
  (33,242 bytes): **6.37 ms per refresh**, two `fsync`s.
- The document is permitted to reach **32 MiB / 50,000 entries**
  ([`project_activity.rs:38-39`](../crates/mallard-infrastructure/src/project_activity.rs#L38)),
  at which size the same unconditional write is far worse.
- The cache is **global across projects** but rewritten per-project scan, so a
  refresh of project A rewrites project B's entries too.

**Fix.** Track whether the scan inserted, updated, or retained-away anything;
skip the write when it did not. (This is PERF-037 from the 2026-09-01
re-audit, still open, now with a size and a measurement attached.)

## A3 (P1) — Eight locked state loads per Activity refresh; the batched API exists and is unused

Traced call graph for one `get_project_activity`:

| # | Caller | Loads |
| --- | --- | --- |
| 1–2 | [`resolve_activity_project`](../crates/mallard-infrastructure/src/project_activity.rs#L234) | `load_config`, `load_bindings` |
| 3–4 | [`capture_request_for_resolved_binding`](../src-tauri/src/project_sync/service.rs#L11740) | `load_bindings`, `load_config` |
| 5–7 | [`build_project_activity`](../crates/mallard-infrastructure/src/project_activity.rs#L1180) | `load_conversation_bindings`, `load_materializations`, `load_config` |
| 8 | [`project_storage_sync`](../crates/mallard-infrastructure/src/project_activity.rs#L1296) | `load_config` |

`sync_config.json` is read, parsed, and **fully structurally validated** four
times. Load #8 is the starkest: `project_storage_sync` is called *from*
`build_project_activity`, which already holds a validated `config` in scope
and simply does not pass it down.

Each load additionally acquires `persistence_guard` (a process mutex, plus an
`open`/`fstat`/`fchmod`/`flock` on `persistence.lock`), and each of the four
identity-bearing loads `fsync`s the state directory (see A4).

Measured against the real `LocalStateRepository`, on a **completely empty**
state root — the cheapest case that exists, no documents to parse:

```
load_config()                 240 µs
load_bindings()              1.84 ms
load_conversation_bindings()  662 µs
load_materializations()      1.31 ms
load_project_state_snapshot() 1.31 ms   <- batched, one guard
4 separate loads             4.06 ms
ratio separate / snapshot    3.09x

one Activity refresh's 8 state reads: 3.28 ms
```

`LocalStateRepository::load_project_state_snapshot`
([`persistence.rs:580`](../crates/mallard-infrastructure/src/persistence.rs#L580))
already loads config, bindings, and materializations under **one** guard and
is **3.09× cheaper** than doing it piecemeal — and no production code calls
it. Its only references are two tests.

**Fix.** Thread one snapshot through the Activity path: take
`load_project_state_snapshot` once at the command boundary and pass borrowed
references down. `build_project_activity` should pass its `config` to
`project_storage_sync` immediately — that one is a two-line change.

## A4 (P2) — Every state read `fsync`s the state directory

**[`persistence.rs:1440-1446`](../crates/mallard-infrastructure/src/persistence.rs#L1440)**

```rust
fn load_or_create_host_identity_unlocked(&self) -> Result<HostIdentity, String> {
    self.recover_host_rotation_unlocked()?;
    match self.load_host_identity_document_classified_unlocked() {
        Ok(Some(identity)) => {
            sync_directory(&self.root)?;   // <- fsync on the happy READ path
            Ok(identity)
        }
```

On the branch where the identity document simply exists and parses, nothing
has been written — yet the directory is `fsync`ed. This runs on every
`load_bindings`, `load_conversation_bindings`, `load_materializations`, and
every `mutate_*`. Measured **47 µs each**; with four identity-bearing loads
per Activity refresh that is ~190 µs of pure barrier per refresh, and it
serializes against other filesystem activity on the volume.

The rotation path already syncs where it needs to
([`persistence.rs:1853`](../crates/mallard-infrastructure/src/persistence.rs#L1853)
and `finish_host_rotation_cleanup_unlocked`), so this barrier publishes
nothing.

**Fix.** Move the `sync_directory` to the branches that actually write
(`auto_register_host_identity_unlocked`, the quarantine path). Keeping it
would need a documented reason; there does not appear to be one.

## A5 (P2) — The two largest state documents are parsed twice

**[`persistence.rs:2044-2048`](../crates/mallard-infrastructure/src/persistence.rs#L2044)** and
**[`persistence.rs:2060-2066`](../crates/mallard-infrastructure/src/persistence.rs#L2060)**

```rust
let Some(value) = read_json_bounded::<Value>(&self.root, &path, MAX_BINDINGS_BYTES)? else { ... };
let bindings: MachineProjectState = serde_json::from_value(value)...
```

`machine_projects.json` (8 MiB cap) and `materializations.json` (**64 MiB**
cap) are deserialized into a `serde_json::Value` tree and then converted.
`read_conversation_bindings_unlocked`
([`persistence.rs:2071`](../crates/mallard-infrastructure/src/persistence.rs#L2071))
does the same job with a direct typed read, so the `Value` hop is not a
migration requirement — it is incidental.

Measured on documents of the same shape:

| Document size | via `Value` | direct | overhead |
| ---: | ---: | ---: | ---: |
| 1.5 MiB | 3.08 ms | 1.96 ms | 1.57× |
| 7.5 MiB | 15.3 ms | 9.84 ms | 1.55× |
| 29.9 MiB | 67.6 ms | 40.6 ms | 1.67× |

On top of the ~1.6× time, the intermediate `Value` tree is a transient
allocation typically 2–4× the document size — a 64 MiB materializations
document briefly costs well over 100 MiB of heap.

**Fix.** `read_json_bounded::<MachineProjectState>` / `::<Materializations>`
directly. The error message even improves: `read_json_bounded` already wraps
parse failures with the path, and a direct parse keeps serde's line/column.

## A6 (P2) — The discovery cache deep-clones its whole inventory under a global mutex

**[`session_watch_service.rs:831`](../src-tauri/src/project_sync/session_watch_service.rs#L831)** and
**[`session_watch_service.rs:868-875`](../src-tauri/src/project_sync/session_watch_service.rs#L868)**

On a cache **hit** the service holds `self.inner.lock()` and returns
`(cached.discovery.clone(), cached.views.clone())`. `Discovery` is eight
collections of owned data; each `LocalSessionView` carries a
`ResourceCandidate` with roughly a dozen heap allocations. A profile with
5,000 sessions therefore performs on the order of 60,000 allocations per
refresh, inside the lock. When `titles_dirty` is set the path clones the
discovery, rebuilds the views, and clones again — three full copies.

Two further problems on the same lock:

- **Filesystem I/O is performed while holding it** —
  `refresh_session_titles` ([:786](../src-tauri/src/project_sync/session_watch_service.rs#L786))
  reads the provider's title index, and the dirty-root loop
  ([:798](../src-tauri/src/project_sync/session_watch_service.rs#L798))
  calls `symlink_metadata` per dirty path.
- The cache is keyed `(provider, home, local_project_id)`, so N projects
  sharing one provider home each retain a **full independent copy** of the
  same home's discovery. (PERF-026, still open; the memory duplication is the
  part not previously recorded.)

**Fix.** Store `Arc<Discovery>` and `Arc<[LocalSessionView]>` in the cache and
hand out clones of the `Arc`. That removes the deep copies, shrinks the
critical section to a map lookup, and lets the per-project entries share one
allocation. Hoist the title refresh and the stat loop out of the lock.

## A7 (P2) — `provider_display_name` rebuilds a full descriptor per row

**[`project_activity.rs:589-594`](../crates/mallard-infrastructure/src/project_activity.rs#L589)**

```rust
fn provider_display_name(registry: &AgentProviderRegistry, provider: &ProviderKey) -> String {
    registry.get(provider).map(|adapter| adapter.descriptor().display_name)...
}
```

`descriptor()` constructs a `ProviderDescriptor` with six `String`
allocations and a `Vec`
([`claude/mod.rs:249`](../crates/mallard-infrastructure/src/providers/claude/mod.rs#L249)),
and everything but `display_name` is dropped immediately. It is called twice
per local row and twice per remote row in `merge_activity_sessions_with_state`,
plus once per provider row.

Measured: **302 ns** per lookup against **6 ns** for a memoized map — 50×.
For a 2,000-session project (~4,000 lookups): **1.21 ms → 24 µs**.

**Fix.** Build a `BTreeMap<ProviderKey, String>` of display names once at the
top of the merge, or add `fn display_name(&self) -> &'static str` to
`SessionAdapter`. Five lines.

## A8 (P2) — The session timeline cache reads and rewrites the whole document per page

**[`session_detail.rs:998-1095`](../crates/mallard-infrastructure/src/session_detail.rs#L998)**

- A **cache hit** still loads and parses the entire
  `session_timeline_cache.json` (cap **32 MiB / 50,000 entries**) to read one
  entry, then clones that entry's `index` and `summary`.
- A **miss** rewrites the whole document — measured **6.37 ms** at the
  current real 17 KiB size, worse as it grows.
- [`:1078`](../crates/mallard-infrastructure/src/session_detail.rs#L1078)
  clones `fresh_entry` on every insert purely so the over-capacity branch can
  reuse it; the common path pays for the rare one.
- Over-capacity handling is `cache.entries.clear()`
  ([`:1080`](../crates/mallard-infrastructure/src/session_detail.rs#L1080)) —
  at 50,000 entries every session's timeline is discarded at once and must be
  rebuilt. The activity cache has the identical cliff
  ([`project_activity.rs:477`](../crates/mallard-infrastructure/src/project_activity.rs#L477)).

**Fix.** Split the cache into per-entry files, or keep the document but
maintain an in-process memo so a hit does not re-parse. Replace both
`clear()` cliffs with eviction of the oldest entries. (Related to PERF-028 and
PERF-036, both still open.)

## A9 (P2) — Catalog journal: full read, two validations, and a full write per phase transition

**[`storage_catalog/persistence.rs:410-451`](../src-tauri/src/storage_catalog/persistence.rs#L410)**

`mutate_operations` loads the whole operations document (16 MiB cap),
`validate()`s it, applies the mutation, prunes, `validate()`s it **again**,
and rewrites the whole file. `apply.rs` calls it **seven times** in one apply,
so a single catalog apply pays seven full read-validate-validate-write cycles
— at least **~45 ms** of `fsync`-bound work at the measured 6.4 ms per write,
independent of how much was actually applied.

The double `validate()` is the redundant half: the document was validated on
load, and the mutation closure is the only thing that changed it. One
post-mutation validation is sufficient and is the one that matters.

**Fix.** Drop the pre-mutation `validate()` (the load already rejects an
invalid document on the read path). Batch the phase transitions that occur
within one apply into a single write where the crash-recovery contract allows
it. (PERF-044, still open.)

## A10 (P3) — `prefetch` dedupes against a `Vec`

**[`bundle_engine.rs:328`](../crates/mallard-infrastructure/src/bundle_engine.rs#L328)**

`if pending.iter().any(|(current, _)| current == path)` inside the loop that
builds `pending` — O(n²) in the prefetch batch size, with `LogicalPath`
comparison. Cheap per comparison, but free to fix: track a
`BTreeSet<&LogicalPath>` alongside.

## A11 (P3) — Observation cache eviction is O(k·n); the write clones every record

**[`session_watch_service.rs:323-346`](../src-tauri/src/project_sync/session_watch_service.rs#L323)**

`while entries.len() > MAX { min_by_key(stored_at); remove }` re-scans the
whole map per evicted entry. Bounded at 4,096 entries so it is not urgent, but
one sort-and-drain is both simpler and cheaper. Two lines later
`entries.values().cloned().collect()` copies every record purely to serialize
it; a borrowed serialization wrapper avoids the copy.

## A12 (P3) — `merge_activity_sessions_with_state` walks the binding list twice with an identical predicate

**[`project_activity.rs:888-932`](../crates/mallard-infrastructure/src/project_activity.rs#L888)**

`preserved_threads` and `cloud_to_local` are built by two separate passes over
`bindings.bindings` under a **byte-identical** four-clause filter, and that
filter itself does a linear `profile_ids.values().any(...)` per binding. One
pass producing both maps is strictly less code and half the work.

## A13 (P3) — `key_belongs_to_project` allocates a format string per check

**[`project_activity.rs:494-498`](../crates/mallard-infrastructure/src/project_activity.rs#L494)**

Called from `cache.entries.retain(...)`, so it runs per cache entry per bound
provider and builds a fresh `format!` prefix each time. Precompute the
prefixes once before the retain.

---

# B. Redundant checks

Most repeated validation in this codebase is deliberate and should stay. Three
are strictly redundant.

## B1 (P2) — The persistence guard `fchmod`s the lock file on every read

**[`persistence.rs:2316-2317`](../crates/mallard-infrastructure/src/persistence.rs#L2316)**

`file.set_permissions(0o600)` runs unconditionally on every guard acquisition
— that is, on every state read in the process. It is a metadata **write** on a
read path.

Measured decomposition of the guard:

| Shape | Cost |
| --- | ---: |
| `open` only | 29.5 µs |
| `open` + `fstat` | 28.6 µs |
| `open` + `fstat` + **unconditional `chmod`** | **55.9 µs** |
| `open` + `fstat` + chmod **only if the mode is wrong** | **33.5 µs** |

The unconditional `chmod` **roughly doubles** the guard. Guarding it with
`if metadata.permissions().mode() & 0o777 != 0o600` preserves the invariant
exactly — the file is still forced to `0600` whenever it is not already — and
returns 40% of the guard cost. At eight guards per Activity refresh that is
~180 µs.

## B2 (P3) — `parent.exists()` is fully subsumed by the call two lines later

**[`persistence.rs:2472-2478`](../crates/mallard-infrastructure/src/persistence.rs#L2472)**

```rust
if !parent.exists() {
    return Err(format!("parent directory '{}' does not exist", parent.display()));
}
ensure_real_directory(parent)?;
```

`ensure_real_directory` calls `symlink_metadata` and reports a missing parent
itself, with a message that is at least as good. The `exists()` call is an
extra `stat` on every atomic write that can only produce an error the next
line would also produce. Additionally `ensure_no_symlinks` at
[:2469](../crates/mallard-infrastructure/src/persistence.rs#L2469) already
walked and stat'ed that same parent — so the parent is stat'ed **three times**
per write.

## B3 (P3) — `read_json_bounded` re-stats the leaf `ensure_no_symlinks` just walked

**[`persistence.rs:2394-2395`](../crates/mallard-infrastructure/src/persistence.rs#L2394)**

`ensure_no_symlinks(root, path)` walks every component **including the leaf**
and discards the metadata; the next line immediately calls
`fs::symlink_metadata(path)` again for `len()` and `is_file()`. Having
`ensure_no_symlinks` return the leaf metadata removes one `stat` per read
(~0.8 µs — small, but it is free and it makes the intent clearer).

## B4 — Deliberate redundancy that should stay

Recorded so it is not "cleaned up" by a later pass:

- **The redaction fixpoint gate.**
  [`credential_redaction.rs:923-933`](../crates/mallard-infrastructure/src/credential_redaction.rs#L923)
  runs the detector again over the redacted record and diffs the spans. That
  is a triple pass — and it is correctly gated behind `if changed`, so it only
  costs anything on records that actually contained a secret. Keep it.
- **`validate()` on every document load.** Cheap for the small documents this
  app has today and the thing that makes a hand-edited `~/.mallard` fail
  closed. The problem is not that it runs, it is that A3 makes it run four
  times per refresh on the same document.
- **`ensure_no_symlinks` itself.** A full-component walk per access is the
  correct shape for a no-follow guarantee; do not memoize it.

---

# C. Over-engineering and dead weight

## O1 — `portable_resources/`: four extensibility axes, one implementation each

**[`crates/mallard-infrastructure/src/portable_resources/`](../crates/mallard-infrastructure/src/portable_resources/)** (~1,070 lines)

A registry over four axes — resource kinds, apply policies, provenance
codecs, tombstone target codecs — each entry an `Arc<dyn Trait>` in a
`BTreeMap` keyed by a versioned coordinate, with duplicate-registration
refusal and version negotiation.

What it actually holds: **5 resource kinds, 5 apply policies, 2 provenance
codecs, 1 tombstone codec**. And the trait implementations are:

| Trait | Production impls | Test-only impls |
| --- | ---: | ---: |
| `ResourceKindAdapter` | **1** (`SimpleKind`) | 1 |
| `ApplyPolicyAdapter` | **1** (`SimplePolicy`) | 1 |
| `TombstoneTargetCodec` | **1** (`TombstoneCodec`) | 1 |
| `PortableContractResolver` | **1** (the registry) | 1 |
| `ProvenanceCodec` | 2 | 1 |

`SimpleKind` and `SimplePolicy` are plain data holders — a key, a primitive,
a label — so the whole `dyn`-dispatch layer resolves to a static table read
through three levels of indirection. Two of the registry's eight query
methods (`require_apply_policy`, `apply_policy_descriptors`) have **zero
references anywhere in the tree**.

The contrast is instructive: `CatalogProviderAdapter` (2 real impls),
`CatalogFormatCodec` (3 real impls), `SessionAdapter` (3), `ObjectStore` (5),
and `StorageProviderAdapter` (2) all carry real polymorphism and are fine.
`portable_resources/` is the one seam where the abstraction has not yet been
paid for by a second implementation.

**Recommendation.** Not a rewrite. Delete the two unused methods, and when the
next portable contract lands, check whether it needs the trait or just a row
in the table — if it is another `SimpleKind`, collapse the axis to a static
table and drop `Arc<dyn>`.

## O2 — Five `pub fn`s with zero references in the entire tree

Verified: each of these appears exactly once in the workspace — its own
declaration.

| Item | Location |
| --- | --- |
| `tombstone_map` | [`mallard-domain/src/lib.rs:3757`](../crates/mallard-domain/src/lib.rs#L3757) |
| `canonical_sha256` | [`mallard-domain/src/bounded.rs:402`](../crates/mallard-domain/src/bounded.rs#L402) (macro-generated, dead across every expansion) |
| `require_apply_policy` | [`portable_resources/registry.rs:131`](../crates/mallard-infrastructure/src/portable_resources/registry.rs#L131) |
| `apply_policy_descriptors` | [`portable_resources/registry.rs:176`](../crates/mallard-infrastructure/src/portable_resources/registry.rs#L176) |
| `note_tombstone` | [`portable_resources/test_contracts.rs:40`](../crates/mallard-infrastructure/src/portable_resources/test_contracts.rs#L40) |

Before deleting, apply the rule from
`dead-code-deletion-verify-test-halves`: check that no test's only coverage of
a live symbol runs *through* these.

## O3 — `merge_activity_sessions` is a production-shaped function that only tests call

**[`project_activity.rs:854-875`](../crates/mallard-infrastructure/src/project_activity.rs#L854)**

It loads bindings, materializations, and config and delegates to
`merge_activity_sessions_with_state`. Production never calls it —
`build_project_activity` does its own loading and calls the `_with_state` form
directly. Its only three references are tests.

This is worse than dead code: **the tests exercise a different loading path
than production does**, so a regression in how `build_project_activity`
assembles its state is invisible to them. Either delete it and have the tests
call `_with_state` with explicit fixtures, or make production use it.

## O4 — The batched state loader nobody uses

**[`persistence.rs:580-595`](../crates/mallard-infrastructure/src/persistence.rs#L580)**

`load_project_state_snapshot` is exactly the fix for A3 — one guard, one
identity load, config + bindings + materializations together, measured
**3.09× cheaper** than the piecemeal calls — and its only two references are
tests. Someone built the right thing and it was never wired in.

## O5 — `project_sync/service.rs` is a 24,890-line module

**[`src-tauri/src/project_sync/service.rs`](../src-tauri/src/project_sync/service.rs)**

5,811 production lines and **19,080 test lines** in one file. The production
half is not unreasonable for the orchestration it does; the test half makes
the file impractical to navigate, and it is where the parallel-session edit
conflicts recorded in this project keep landing.

`mallard-domain/src/lib.rs` has the same shape at 5,568 lines.

**Recommendation.** Split the tests into `service_tests/` submodules by
concern (push, pull, comparison, restore). Mechanical, behaviour-preserving,
and it makes the production half readable. This is the cheapest large
maintainability win available.

## O6 — `SyncConfig::project()` is a linear scan on a hot lookup

**[`mallard-domain/src/lib.rs:1345-1349`](../crates/mallard-domain/src/lib.rs#L1345)**

`self.projects.iter().find(...)` — fine at today's project counts, but it is
called from many paths and `SyncConfig::validate` already builds a
`BTreeMap<LocalProjectId, &LocalProjectRegistration>` internally
([:1266](../crates/mallard-domain/src/lib.rs#L1266)) and throws it away.
Worth noting, not worth changing until project counts grow.

---

## What is already good

Stated plainly, because a report that only lists problems misrepresents the
codebase:

- **Bounded concurrency on transfers is done correctly.** `stage_many_verified`
  and `put_many` ([`s3_store.rs:989`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L989),
  [:1057](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1057))
  use a semaphore, preserve result order, and abort-and-drain on first failure.
- **App settings are cached in atomics**, not re-read per call — 66 call sites
  pay one relaxed load each ([`app_settings.rs:135-161`](../crates/mallard-infrastructure/src/app_settings.rs#L135)).
- **The credential detector is well-engineered** — a `RegexSet` prefilter
  before the per-rule regexes, an entropy gate, and a skip-mode check before
  any of it ([`credential_redaction.rs:505-518`](../crates/mallard-infrastructure/src/credential_redaction.rs#L505)).
- **`redact_jsonl_text` splices** rather than re-serializing, so untouched
  lines stay byte-identical.
- **The provider and catalog registries are honest seams** with real multiple
  implementations, clearly documented as compile-time and not a plugin ABI.
- **PERF-033 (pretty-printed machine caches) is resolved** — a compact writer
  exists and `to_string_pretty` survives only in two error-formatting paths.
- **The watcher's dirty-root tracking is genuinely good**: a clean profile
  serves cached views with **no `stat` at all**, and only dirty roots are
  re-examined.

## O7 — Two clippy `too_many_arguments` warnings, both in the newest `service.rs` code

`cargo clippy --workspace --all-targets` exits 0 but is **no longer
warning-clean**: two warnings, both in this working tree's uncommitted
changes. The 2026-09-01 audit recorded zero, so these are new.

| Location | Arity |
| --- | ---: |
| [`service.rs:9049` `build_restore_plan_for_apply`](../src-tauri/src/project_sync/service.rs#L9049) | 8/7 |
| [`service.rs:9226` `prepare_pull_conversation_actions`](../src-tauri/src/project_sync/service.rs#L9226) | 9/7 |

Worth more than a lint suppression: in both signatures the same cluster
(`repository`, `resolved`, `snapshot`, `storage_id`, `engine`) travels
together and is already assembled by the caller. A borrowed context struct —
the shape `ProjectLocalRead` and `ProjectRemoteRead` already use elsewhere in
this file — removes the warning and the positional-argument hazard at once.
This is the same altitude problem as O5, showing up at function scale.

---

## Recommended remediation order

**Milestone 1 — the measured wins (roughly a day, all low-risk):**

| Finding | Change | Expected effect |
| --- | --- | --- |
| A1 | Index the retention firewall | 201× at 20k files; removes a ~28 s worst case |
| A2 | Skip the activity-cache write when unchanged | −6.4 ms per refresh |
| A3 | One state snapshot per command; pass `config` to `project_storage_sync` | −3.09× on state reads, −3.3 ms per refresh |
| A4 | Move `sync_directory` off the read path | −190 µs per refresh, fewer barriers |
| B1 | `chmod` only when the mode is wrong | −40% of every guard |
| A7 | Memoize provider display names | −1.2 ms on a 2,000-session project |

That set is about **10 ms per Activity refresh** and turns the Push firewall
from superlinear to linear.

**Milestone 2 — allocation and ownership:** A5 (direct typed parse), A6
(`Arc` the discovery cache and shrink the critical section), A8 (timeline
cache), A9 (catalog journal).

**Milestone 3 — hygiene:** O2 (delete the five dead items), O3/O4 (wire the
batched loader in, retarget the merge tests), O5 (split `service.rs`'s
tests), O7 (context structs for the two wide signatures), then A10–A13 and
B2/B3 opportunistically.

**Not recommended:** rewriting `portable_resources/`. Trim its dead methods
and re-evaluate when a second real contract arrives.

## Relationship to prior audits

Re-confirmed as **still open** at this baseline, now with measurements:
PERF-026 (per-project inventory cache — plus the memory duplication),
PERF-028 (entry-count vs byte-limit eviction), PERF-036 (session-detail cache
state), PERF-037 (warm refresh still writes the full cache), PERF-044
(catalog journal rewrites).

**Newly found here:** A1 (quadratic retention firewall), A3/O4 (eight locked
loads and the unused batched loader), A4 (`fsync` on the read path), A5
(`Value` double-parse), A7 (descriptor per row), B1 (unconditional `chmod`),
O1–O3, O7.

**Confirmed resolved:** PERF-033.

---

## Appendix — probe sources and raw output

All probes were release builds. The two that link the real crates depend on
`crates/mallard-infrastructure` and `crates/mallard-domain` by path from an
external workspace; **no repository file was modified**.

```
scratchpad/probe/src/main.rs           A1 retention scan; A5 Value double-parse
scratchpad/statebench/src/main.rs      A3 real LocalStateRepository load costs
scratchpad/statebench/src/bin/breakdown.rs  guard / fsync / read / write costs
scratchpad/statebench/src/bin/guardsplit.rs B1 chmod decomposition
scratchpad/statebench/src/bin/descr.rs      A7 descriptor() cost
scratchpad/deadpub.py                  O2/O3/O4 pub-fn reference analysis
scratchpad/nested.py                   linear-scan-inside-loop detector
```

```
retention n=  1000  linear=     6.483ms  indexed=   470.667µs  speedup=  13.8x
retention n=  5000  linear=    71.318ms  indexed=     1.283ms  speedup=  55.6x
retention n= 10000  linear=   276.183ms  indexed=     2.290ms  speedup= 120.6x
retention n= 20000  linear=      1.136s  indexed=     5.651ms  speedup= 201.0x

double-parse  1.50 MiB  via Value= 3.079ms  direct= 1.962ms  overhead=1.57x
double-parse  7.48 MiB  via Value=15.278ms  direct= 9.842ms  overhead=1.55x
double-parse 29.92 MiB  via Value=67.560ms  direct=40.567ms  overhead=1.67x

persistence_guard shape (open+fstat+fchmod) :   54.065µs
sync_directory(root)  -- on the READ path   :   46.907µs
one symlink_metadata (the redundant stat)   :  816.000ns
read_json_bounded shape (3 stats+open+read) :   37.176µs
write_json_atomic 33 KiB (2 fsyncs)         :    6.367ms

open only                            :  29.537µs
open + fstat                         :  28.585µs
open + fstat + UNCONDITIONAL chmod   :  55.918µs
open + fstat + chmod IF WRONG        :  33.541µs

descriptor()-per-lookup : 302ns  (4000 rows => 1.208ms)
memoized lookup         :   6ns  (4000 rows =>  24.000µs)

LocalStateRepository, empty state root:
  load_config()                 240.251µs
  load_bindings()               1.842435ms
  load_conversation_bindings()  661.569µs
  load_materializations()       1.314318ms
  load_project_state_snapshot() 1.313726ms
  4 separate loads              4.058573ms   ratio 3.09x
  one Activity refresh's state reads (8 calls): 3.283643ms
```

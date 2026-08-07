# Thread Variant Sync Requirements — Readable Version

## Main promise

**Pull never silently switches the conversation a user is continuing.**

- A stays on **(a, b, c1)**.
- B stays on **(a, b, c2)**.
- Both versions remain available.
- The user chooses when to open or promote another version.

## Symbols

| Symbol | Meaning |
| --- | --- |
| **L** | Local version |
| **C** | Cloud version |
| **L = C** | Identical |
| **C < L** | Local extends cloud |
| **L < C** | Cloud extends local |
| **L ⋈ C** | Versions diverged |
| **∅** | Missing |
| **V1**, **V2** | Portable variant IDs |
| **T1**, **T2** | Local provider thread/session IDs |

Message IDs and parent links decide whether one version extends another.
Matching text alone is not enough.

## Push and Pull

| State | Push | Pull |
| --- | --- | --- |
| **L = C** | No-op | No-op |
| **C < L** | Cloud → L | Keep L |
| **L < C** | Block; Pull first | Local → C |
| **L ⋈ C** | Keep C; add L | Keep L; add C |
| Ancestry unknown | Treat as diverged | Treat as diverged |

Default behavior always preserves both branches. Replacing a version or
changing the primary requires explicit approval.

## Three separate choices

- **Cloud primary**: the version cloud presents first.
- **Local preferred**: the version Mallard presents first on one machine.
- **Active thread**: the version the user actually opens in Codex or Claude.

These are independent:

~~~text
cloud primary  ≠  local preferred  ≠  active thread
~~~

Ordinary Push/Pull does not change any of them. Pull also does not launch,
focus, or switch Codex or Claude.

## A/cloud/B example

Cloud contains:

~~~text
V1 → (a, b, c1)
V2 → (a, b, c2)
~~~

Before Pull:

~~~text
A: V1 → A-T1
B: V2 → B-T2
~~~

After Pull:

~~~text
A: V1 → A-T1   V2 → A-T2   preferred = V1
B: V1 → B-T1   V2 → B-T2   preferred = V2
~~~

Result:

- A continues **c1** through unchanged ID **A-T1**.
- B continues **c2** through unchanged ID **B-T2**.
- A can explicitly open **c2** through **A-T2**.
- B can explicitly open **c1** through **B-T1**.
- Pushing from **V1** updates only **V1**; pushing from **V2** updates only
  **V2**.

On a fresh machine, Pull creates both versions with different local provider
IDs. Cloud primary may appear first, but nothing is opened automatically.

## Identity rules

Each conversation version uses three identities:

~~~text
conflict_group_id → conversation
variant_id        → branch
provider_id       → local resume handle
~~~

- **conflict_group_id** and **variant_id** are portable.
- **provider_id** is local to one provider profile.
- The same variant may use different provider IDs on A and B.
- Two visible variants in one profile must never share a provider ID.
- An existing local thread keeps its ID.
- An incoming version gets a new ID when its proposed ID is occupied.
- Repeat Pull reuses the saved mapping and creates no duplicates.

Creating a new provider ID must update all structural filenames, metadata,
sidecars, indexes, and **cwd** references without changing messages or
parentage. If this cannot be verified safely, keep the version stored-only.

## Missing and deletion

| Local | Cloud | Default |
| --- | --- | --- |
| **∅** | Present | Pull creates local version when safe |
| Present | **∅** | Push adds it; Pull keeps it |
| **∅** | **∅** | No-op |

Missing does not mean deleted. Deletion requires an explicit tombstone. A
delete/modify race keeps the modified version as a conflict.

## Safety requirements

- Keep cloud bundles immutable and SHA-256 verified.
- Use compare-and-swap for cloud updates and primary changes.
- Recheck cloud revision, digests, local target, provider ID, binding
  revision, and plan expiry before apply.
- Back up content before an explicitly approved replacement.
- Reject symlinks, special files, stale reviews, and ID collisions.
- Stage and verify provider changes; commit all changes or roll back all.
- Keep credentials, absolute local paths, preferences, plans, backups,
  receipts, and local bindings out of portable storage.
- Use a new bundle schema if schema-4 clients could misread variant data.

## Done when

- The full **=**, **<**, **⋈**, unknown, and **∅** matrix is tested.
- A stays on **c1** and B stays on **c2**.
- Both can continue either version.
- Fresh and repeated Pulls create no ID collisions or duplicates.
- Stale and partial operations fail safely.
- Every supported provider passes ID-rewrite and continuation tests.

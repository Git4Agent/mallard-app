# Plan: Distinguish Codex forks and preserved local conversations in Activity

Status: Proposed
Date: August 12, 2026

## 1. Outcome

Activity must stop collapsing unrelated Codex tasks, native forks, and
Mallard-created divergence copies under the same first-user-message title.

For example, instead of five rows named `Good morning`, Activity should show
the provider's latest native titles and relationship labels:

```text
Respond to greeting

Respond to greeting                 [Fork]
Forked from Respond to greeting

Respond to greeting                 [Preserved local]
Forked from Respond to greeting
```

The exact provider title remains available in the row details or title
tooltip:

```text
Respond to greeting — fork · 2026-08-05T09:50:00-07:00
Respond to greeting — local before Pull · 2026-08-05T09:59:00-07:00
```

This is presentation only. Titles and relationship labels never participate
in conversation identity, Push/Pull comparison, conflict classification, or
cloud CAS.

## 2. Metadata impact

The first implementation does **not** change persisted metadata.

| Surface | Change |
| --- | --- |
| Codex rollout JSONL | Read existing authoritative `forked_from_id`; never write |
| Codex `session_index.jsonl` | Read the latest existing `thread_name`; never write |
| `~/.mallard` Activity cache | No field or schema change |
| `~/.mallard` bindings, sync config, checkpoints, plans, journals, and receipts | No change |
| Portable `ConversationThread` manifest entry | No new field and no changed write behavior |
| Cloud objects and bundle schema | No change; remain schema 1 |
| Activity Rust/Tauri response and `src/types.ts` | Add presentation-only runtime fields |
| React/CSS | Render the runtime fields |

The Activity response is serialized across the Tauri boundary, so its DTO
shape changes. That is an application API change, but it is not stored sync
metadata and does not modify either provider home, `~/.mallard`, or cloud
storage.

Activity already updates its existing bounded `activity_cache.json` during a
scan. This plan does not eliminate that existing write; it adds no cache key,
schema change, or additional persistence caused by fork presentation.

The richer relationship UI must not add fields to `SessionFileSummary`,
because that type is serialized in the machine-local Activity cache. Extract
the relationship during normal discovery and carry it only through ephemeral
discovery and Activity response values.

## 3. Existing authoritative inputs

### 3.1 Native title

Codex stores task names in the append-only `session_index.jsonl`. The latest
valid row for a task wins. Mallard already follows that rule in
`parse_session_index_titles` and places the result in
`DiscoveredSession.display_name`.

Activity currently loses it by preferring `SessionFileSummary.title`, which is
derived from the first meaningful user message. Reverse that precedence:

1. latest non-empty native Codex title when it differs from the provider ID;
2. first meaningful user-message title;
3. `Codex session <short-id>`.

Because discovery rereads the native index while the cache stores only the
transcript summary, a native rename appears on the next Activity scan without
an Activity-cache migration or invalidation.

### 3.2 Native fork relationship

The authoritative first `session_meta` record may contain
`payload.forked_from_id`. It identifies the immediate source task of a native
fork. Mallard's Codex structural parser already reads and validates this
field.

Use only this first authoritative value. Do not infer a fork from:

- an equal title;
- equal transcript content;
- `history_base`, which describes a physical inherited-history prefix;
- `parent_thread_id`, which describes subagent topology; or
- timestamps or discovery order.

Nested forks point to their immediate parent. Missing or out-of-window parents
do not invalidate the child.

### 3.3 Existing portable presentation

For a cloud row, use the existing `ConversationThread.display_title` and
existing presentation-only `ConversationThread.forked_from` when they are
already present.

This plan does not start copying provider-native IDs into portable metadata
and does not change how `ConversationThread.forked_from` is populated. A raw
Codex task ID is machine/provider identity and must not be introduced into the
portable manifest as ancestry metadata.

Consequently, a remote-only or reidentified task may retain its exact minted
display title while lacking a relationship badge. Making native ancestry
portable would be a separate metadata design and is outside this plan.

## 4. Presentation model

Add a small provider-neutral Activity presentation value:

```rust
#[serde(rename_all = "snake_case")]
enum SessionThreadVariant {
    Main,
    Fork,
    PreservedLocal,
}

struct SessionThreadPresentation {
    variant: SessionThreadVariant,
    // The exact latest provider title, including a Mallard-minted suffix.
    native_title: Option<String>,
    // A resolved parent title or bounded short-ID fallback. Display only.
    parent_label: Option<String>,
}
```

Add `thread_presentation: SessionThreadPresentation` to
`ProjectSessionSummary` and mirror it in `src/types.ts`.

These are response fields only. Do not add them to `SessionFileSummary`,
`ConversationThread`, `LocalConversationBinding`, or any sync document.

`ProjectSessionSummary.title` remains the compact primary title. It uses the
native title as its source but removes only a recognized Mallard-owned variant
suffix for row presentation. `native_title` retains the exact string.

## 5. Classification policy

The Codex adapter, not generic React code, interprets Codex and Mallard-owned
native naming conventions.

| Evidence | Variant | Compact row title |
| --- | --- | --- |
| No `forked_from_id` | `main` | latest native title, otherwise first-user fallback |
| `forked_from_id` and exact native suffix ` — local before Pull · <timestamp>` | `preserved_local` | title before the suffix |
| `forked_from_id` and exact native suffix ` — fork · <timestamp>` | `fork` | title before the suffix |
| `forked_from_id` with any other title | `fork` | complete native title |

The suffix parser must be strict and bounded. A matching suffix without
authoritative fork ancestry does not turn an unrelated root into a fork.

`preserved_local` is a presentation convention, not durable divergence
provenance. Current sync state does not retain a separate authoritative
"preserved by Pull" marker, and this plan deliberately does not add one. If
the product later requires that label to survive provider rename,
reidentification, and cross-machine materialization, that would require a
separately approved persisted-metadata change.

If the user later renames a fork, the latest native name wins. Its relationship
remains `fork` from `forked_from_id`, but it becomes `preserved_local` only
while the exact Mallard-owned preserved-local title is still present. This is
the intentional limit of making no persisted provenance change.

Do not assign sibling ordinals such as `Fork 1` and `Fork 2`; they are unstable
under paging, filtering, and later discovery.

## 6. Data flow

### 6.1 Codex discovery

Extend the existing bounded head metadata result with an optional immediate
fork source ID:

```rust
struct SessionFileMetadata {
    session_id: String,
    cwd: PathBuf,
    is_internal_subagent: bool,
    forked_from_session_id: Option<String>,
}
```

Carry it through `DiscoveredSession` and `LocalSessionRecord`. These structs
are ephemeral and are not written to the Activity cache.

Do not add another transcript read. The existing discovery metadata scan
already parses the authoritative `session_meta` needed to select the session.

The same discovery pass reads the latest native index title and runs a pure
Codex presentation classifier, producing:

- the exact native title;
- a compact base title;
- `main`, `fork`, or `preserved_local`; and
- the optional immediate parent provider task ID for later resolution.

### 6.2 Activity merge

During `merge_activity_sessions`:

1. build the existing provider-session-to-logical-thread/resource map;
2. select the title using the precedence in section 3.1;
3. resolve the optional immediate parent to another visible Activity resource;
4. resolve its title when present;
5. otherwise display `Forked from session <short-parent-id>`; and
6. emit only presentation DTOs, without mutating bindings or portable records.

Two roots with the same title remain two main tasks and receive no relationship
label.

### 6.3 Remote rows

For remote Activity rows:

- use the existing portable `display_title` as the title source;
- use an existing logical `forked_from` only when it resolves safely;
- never reconstruct native ancestry from content or physical history; and
- never write missing presentation evidence back to the manifest as a side
  effect of viewing Activity.

## 7. UI behavior

The compact row shows:

- base title as the primary label;
- a `Fork` or `Preserved local` badge when applicable;
- the normal task timestamp in its existing right-side position; and
- a secondary `Forked from <parent title>` line when ancestry is known.

The exact `native_title` appears in the row's existing details surface and as
the primary-title tooltip. This exposes the full minted timestamp without
duplicating it in the compact row.

The relationship label is informational. It must not change selection,
grouping, sorting, resource identity, action eligibility, or Push/Pull state.

## 8. Implementation locations

Expected implementation files:

- `crates/mallard-infrastructure/src/providers/traits.rs`
- `crates/mallard-infrastructure/src/providers/codex/history.rs`
- `crates/mallard-infrastructure/src/providers/codex/discovery.rs`
- `crates/mallard-infrastructure/src/codex_rollout.rs`
- `crates/mallard-infrastructure/src/project_activity.rs`
- `src/types.ts`
- `src/components/project-sync/ProjectActivityPage.tsx`
- `src/App.css`
- focused Rust and frontend integration tests

No persistence, bundle, Push, Pull, binding, or storage module should change.

## 9. Implementation sequence

1. Reverse Activity title precedence and add regression coverage for native
   titles and index-only renames.
2. Add a pure Codex title/variant classifier with strict minted-suffix tests.
3. Carry `forked_from_id` through ephemeral discovery state.
4. Add the presentation-only Activity response fields in Rust and TypeScript.
5. Resolve visible direct parents during Activity merge.
6. Render badges, relationship text, exact-title details, and tooltip.
7. Add no-write tests around discovery and Activity scanning.

## 10. Tests and acceptance criteria

### Backend

- Native index title wins over an identical first-user title.
- Repeated native index rows use the latest valid name.
- An index-only rename appears despite a transcript-summary cache hit.
- A root and child with the same first message render as `main` and `fork`.
- Two unrelated roots with the same first message both remain `main`.
- A nested fork names its direct parent.
- An unavailable parent falls back to its bounded short ID.
- `history_base` without `forked_from_id` remains `main`.
- A subagent `parent_thread_id` does not create a task-fork relationship.
- Exact ` — fork · <timestamp>` classifies as `fork` only with ancestry.
- Exact ` — local before Pull · <timestamp>` classifies as
  `preserved_local` only with ancestry.
- A renamed preserved-local task falls back to the generic `fork` variant.
- Activity scanning adds no provider-home or cloud write and no new
  `~/.mallard` field or write beyond its existing Activity-cache update.
- Serialized cache, binding, sync-config, and manifest fixtures are unchanged.

### Frontend

- Main, fork, and preserved-local rows are visibly distinct.
- The full native minted title is available in details and tooltip.
- Relationship text uses the parent title when visible and a short ID fallback
  otherwise.
- Badges do not alter row selection, ordering, or actions.
- Existing remote rows without ancestry continue to render normally.

### Release gate

- Bundle schema remains 1.
- Machine-local schema remains 1.
- No migration or backward-compatibility path is added.
- No raw provider ancestry ID is added to portable metadata.

## 11. Version-pinned Codex references

The adapter behavior is grounded in the installed Codex 0.147.0 source:

- [`Thread.forked_from_id` and `Thread.name`](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs#L185-L245)
- [append-only native name index](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/rollout/src/session_index.rs#L24-L54)
- [latest matching index row wins](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/rollout/src/session_index.rs#L123-L153)
- [Codex's own `Thread forked from ...` presentation](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/tui/src/chatwidget/session_flow.rs#L198-L223)

# Project Activity and Codex Session History

## Implemented behavior

A completed project row is the single navigation target for project activity. The separate Git action has been removed. Each bound completed row now has a compact, non-interactive `Git Based` or `Non-Git Based` indicator; it communicates repository type but is not another navigation target. Setup drafts remain non-project rows and cannot call history APIs. Project and Storage Settings stay dedicated pages; closing them returns to the selected project activity page. A successful Pull increments the existing refresh epoch so restored sessions are rescanned.

The title uses `projectLabel(project)`: a machine-local alias is primary and the shared repository name is secondary. The compact header shows the canonical project directory, bound Codex configuration path, and per-storage last Pull/Push timestamps. Git projects add Branch and Refresh controls; non-Git projects omit them.

```text
Project Name          Local alias or repository name
Directory             /canonical/local/path
Codex configuration   /project/profile/.codex                  [Settings]

Storage sync
  Storage 1           Last Pull …                 Last Push …

Uncommitted Changes
  Session title                         [Open in Codex] [Open in Terminal]
  Started …  Ended …  User rounds …  Tokens …  Agent messages …  Tool calls …
  Appears under 3 commits
  [› Show chat details]

abc1234  Jul 18, 10:00 PM  Commit subject
  same compact session card, ordered by last activity
```

The `Uncommitted Changes` label means no overlapping commit and no qualifying follow-up commit on the recorded branch. It does not describe current `git status`.

## Session and mapping semantics

Each rollout JSONL file is one Codex session. Mallard streams the complete file and derives authoritative start/end dates from its first and last timestamped records. It counts genuine `user_message` rounds, visible `agent_message` events, tool calls, and the maximum reported cumulative token total. Oversized records are selectively projected for relevant metadata and visible text; malformed records are skipped without stopping the file, make the session partial, and produce diagnostics in Sync Log.

For a selected first-parent branch:

1. A session is attached to every commit timestamp inclusively inside its start/end range.
2. With no overlap, it is attached once to its successor commit, with no time cap. When the session's recorded SHA sits on the branch's first-parent rail, the successor is the first of that commit's first-parent descendants committed after the session ended — commits below the recorded checkout never qualify, whatever their timestamps. Without a usable recorded SHA, the successor is the earliest commit after session end.
3. With no successor, it appears under `Uncommitted` together with the reason.

One session spanning three commits is one unique session and three commit occurrences. It intentionally renders under all three commits; all occurrences share metrics, launch target, and a frontend detail cache keyed by thread ID.

The initial response is the latest exclusive 30-day window. `Load previous 30 days` appends the next complete `[before − 30 days, before)` window. Boundary commits carrying a mapped session are returned as needed and merged by SHA/thread ID so cross-window sessions remain visible at every occurrence.

## Chat details and privacy

`Show chat details` lazily loads genuine User and Codex message previews, 50 at a time, chronologically. Each preview is normalized and capped at 240 characters. System/developer content, injected context, reasoning, and raw tool payloads are excluded. Full chat text and derived metadata stay local and never enter bundle manifests. The on-disk cache under `~/.mallard/chat_history_cache.json` contains only parsed metadata/metrics, keyed by profile, rollout path, size, and modification time.

## Oversized-record recovery

The rollout reader retains its 1 MiB bounded fast path for ordinary JSONL records. When a record exceeds that limit, Mallard seeks back to the record boundary and stream-deserializes only the fields needed for session discovery, metrics, and chat details. Visible User and Codex text remains recoverable without retaining embedded image data, compaction bodies, reasoning, or raw tool payloads.

This behavior does not add image display or change the chat-details contract. Details remain chronological, paginated 50 at a time, and normalized to 240-character previews. Successfully parsed oversized records that contain no relevant metadata or visible text are ignored silently; malformed oversized records may still make the session partial and produce a deduplicated diagnostic.

## Main-branch compatibility

The work preserves the main changes already merged through `2edc284`, including the relevant `826d890`, `60e0ea5`, and `d4d794a` work: global `~/.mallard` persistence, resumable setup, optimistic revisions, resizable sidebar, local aliases, and dedicated settings layouts. The landed `Git Info` fallback is replaced in place; no parallel `links | chat-history` state is introduced.

## Design decisions made autonomously

- Kept the existing dense desktop theme tokens and outline icons; added no gradients, badges, fonts, palettes, or icon dependencies.
- Used a neutral metric grid and explicit Started/Ended values instead of prompt extracts or mapping-confidence labels.
- Kept repeated cards visually identical and explained repetition with subdued occurrence text.
- Used semantic disclosure buttons, visible focus rings, per-occurrence DOM IDs, and browser viewport containment for long lists.
- Put scan diagnostics and exact launch commands in Sync Log instead of a page-wide technical warning.
- Superdesign CLI 0.6.0 was reachable on 2026-07-19, but `auth status --json` returned `authenticated: false`; no unapproved substitute export was placed in `assets/`.

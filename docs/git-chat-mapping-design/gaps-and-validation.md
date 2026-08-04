# Gaps and Validation

## Assumptions and intentional limitations

- V1 remains Codex-only and local/restored-only. Claude and remote-only sessions are deferred.
- Each rollout JSONL file is treated as one Codex session; duplicate thread IDs keep the latest complete parsed instance.
- `Uncommitted Changes` is temporal classification, not a claim about current working-tree modifications.
- Recorded SHA is context only and never proves authorship or controls attachment.
- First-parent history is intentionally not the full Git DAG. Rebases, amended/cherry-picked commits, shallow clones, deleted branches, clock skew, and active sessions can alter or remove temporal relationships.
- Commit correlation remains bounded to the newest 10,000 first-parent commits to prevent an unbounded Git subprocess result. Rollout files and session counts themselves are not capped.
- JSONL records over 1 MiB leave the bounded fast path and are selectively streamed for metadata and visible text while image data and other large irrelevant fields are discarded. Malformed oversized records are skipped, mark the session partial, and produce one diagnostic per rollout scan.
- The session index still has a defensive 16 MiB cap. It affects preferred titles only, not rollout discovery, dates, metrics, details, or pagination.
- Token totals are maximum Codex-reported cumulative usage, not billing estimates.
- Restored sessions may be visible before Codex rebuilds its own index.
- Windows/Linux terminal automation, Claude launch actions, full-text search, causal attribution, remote merge semantics, and sync-schema conversation content are out of scope.

## Known validation gaps

- `codex://threads/<uuid>` remains unconfirmed by public Codex documentation. The implemented app command follows the project-scoped `CODEX_HOME` requirement and must be tested against the installed desktop build.
- macOS is the only implemented automated launch platform.
- The Superdesign CLI was validated at version 0.6.0 but was not authenticated. An interactive `superdesign login` is required before generating the one approved remote export. `assets/` intentionally contains no fabricated design.
- Manual Tauri verification is still required for real long rollouts, a profile path/project path containing quotes and spaces, both themes, launch actions, shallow/worktree repositories, and a Pull-restored task.

## Oversized-record recovery validation

Automated coverage demonstrates that:

- an oversized User record containing small input text plus a multi-megabyte image returns the text without retaining the image;
- an oversized assistant record returns eligible Codex output text while ignoring image or attachment data;
- oversized compaction snapshots and tool-output bodies create no visible turns or warnings;
- oversized tool calls still contribute their counter without retaining arguments;
- a malformed oversized record produces one deduplicated warning, marks metrics partial, and does not hide later valid messages;
- recovered turns preserve chronological pagination, stable ordinals, duplicate suppression, and the 240-character preview cap; and
- the ordinary line buffer remains bounded to 1 MiB while ignored oversized strings are streamed without allocation proportional to their payload size.

## Automated evidence

Focused Rust coverage includes:

- parsing beyond 256 records with first/last timestamp precedence;
- malformed and oversized individual records continuing as partial;
- injected context excluded from user metrics/details;
- user, agent, tool, token, active-session, fallback, and summary behavior;
- single successor attachment (earliest commit at or after last activity) through the recorded SHA's first-parent descendants, time-based fallback without a usable recorded SHA, uncommitted classification (including sessions with activity after the newest commit), and branch isolation;
- first-parent Git pagination and hostile input validation;
- detail pagination and role filtering;
- UUID, quoting, project-scoped `CODEX_HOME`, app command, and Terminal command construction;
- backward-compatible RecipeBase sync timestamps.

Frontend integration coverage includes alias/repository presentation, project/profile/storage metadata, neutral metrics, explicit dates, absence of mapping badges/raw extracts, Git/non-Git flows, launch actions, removal of the sidebar Git action, and the replacement non-interactive repository-type indicator. Rows use stable SHA/thread keys and shared detail data in implementation.

Latest verified during implementation:

- `npm run test:frontend-integration` — 6/6 passed.
- focused `project_sync_v3::chat_history::tests` — 29/29 passed, including oversized image-text recovery, silent compaction/tool-output projection, malformed-record warning deduplication, recovered-turn pagination, a rollout larger than 16 MiB, timestamp precedence under partial parsing, injected-message filtering, duplicate rollout selection, and private cache persistence.
- schema-3 command integration tests — 28/28 passed, including successful/failed Pull timestamp semantics.
- frontend integration tests — 6/6 passed.
- `cd src-tauri && cargo check` — passed.
- `npm run build` — passed (the existing Vite large-chunk advisory remains).

## Design-skill notes

`frontend-design` kept commit structure, timestamps, and session metrics as the visual hierarchy instead of generic dashboard decoration. `ui-ux-pro-max` informed the dense desktop spacing, disclosure semantics, focus treatment, one-line previews, accessible loading/error announcements, and viewport containment. Existing light/dark tokens and the outline icon component remain the only visual system.

Local Superdesign context in `.superdesign/init/` was refreshed for the activity destination, removal of the sidebar Git action, profile/storage metadata, neutral session cards, repeated occurrences, lazy details, and 30-day paging. Remote export generation is blocked solely by authentication; after login, generate one compact variant from the implemented page and retain only that approved export in `assets/`.

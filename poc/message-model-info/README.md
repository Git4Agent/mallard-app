POC result: **Codex and Claude Code can supply model and thinking-level metadata for each message.** The available data depends on the client version and the saved records.

Verified on 2026-10-06:

| Client | Model source | Thinking-level source |
| --- | --- | --- |
| Codex CLI 0.156.0 | `turn_context.payload.model` | `turn_context.payload.effort` |
| Claude Code 2.1.291 | `assistant.message.model` | `assistant.effort` |

The Claude field is on the outer assistant record. It is not inside `message`. In both native test turns, `perTurnEffort` was `null`.

The native test used a local mock. It started one conversation per provider, then resumed each conversation with a different model and effort. Each input and reply had the expected metadata:

| Provider | Turn | Message roles | Model | Thinking level |
| --- | --- | --- | --- | --- |
| Codex | 1 | User and assistant | `gpt-5.6-luna` | `low` |
| Codex | 2 | User and assistant | `gpt-5.6-sol` | `high` |
| Claude | 1 | User and assistant | `claude-opus-4-6` | `low` |
| Claude | 2 | User and assistant | `claude-sonnet-4-6` | `high` |

The test compared the saved fields with the outgoing API request fields. It checked four requests and eight message rows. This proves client persistence and extraction. It does not test real model inference or backend routing.

A user input has no generating model. For a user row:

- Codex uses the settings of the matching `turn_id`.
- Claude follows `parentUuid` through attachments and tool results to link replies to the input. The user row is marked `inferred`.
- If replies have different settings, the row has status `conflict` and lists the values. A UI can show `Mixed`.
- If metadata or a parent link is missing, the row stays `unknown`. A UI can show `Unknown`.

The extractor groups Claude blocks by `message.id`. It excludes tool-result inputs, local command strings, synthetic errors, and Codex replay duplicates. It emits model metadata and row numbers. It does not emit message text, reasoning text, native message IDs, or source paths.

“Thinking level” means the configured effort. Claude's adaptive thinking mode is a separate setting. A thinking block or token budget does not establish the effort level. See [Claude effort and thinking](https://platform.claude.com/docs/en/build-with-claude/effort#effort-with-thinking). Codex permits model and effort overrides per turn; see [Codex turn settings](https://learn.chatgpt.com/docs/app-server#turns).

Run from the repository root:

```sh
node --test poc/message-model-info/model-info.test.mjs
node poc/message-model-info/run-native.mjs
```

The native probe requires `codex` and `claude` on `PATH`, plus the existing `@copilotkit/aimock` dependency. It uses temporary homes and a loopback mock server. It clears inherited credentials, disables hooks and MCP servers, and removes temporary state when it finishes. It saves a metadata report to `output/message-model-info-poc/native-report.json`. That directory is ignored by Git.

To inspect one prepared JSONL fixture:

```sh
node poc/message-model-info/model-info.mjs codex fixture.jsonl
node poc/message-model-info/model-info.mjs claude fixture.jsonl
```

The file must be a regular file with one hard link. Its resolved path must be canonical and contain no symlinks. Limits are 16 MiB per file, 1 MiB per record, and 10,000 records. The reader rejects a changed file. Tests use temporary fixtures only.

POC limits:

- Codex support covers the current `item_completed` stream and explicit turn IDs. The reported model is the recorded turn setting. A server-side reroute can use a different model.
- Claude support covers the saved assistant model and effort fields verified above. Non-null `perTurnEffort` is unverified and produces `Unknown` for thinking level.
- Older records can lack these fields. Current profile settings cannot recover missing historical values.
- The POC reads one file. It does not resolve retained Codex history, rollbacks, active Claude branches, or child transcripts. A production change must use Mallard's existing source and ownership resolution.

UI mockup: run `npm run dev`, then open [the preview](http://localhost:1420/mockups/message-model-mockup.html). Each message shows `model · level`. Click the label for Model, Thinking, and Source. Use the Codex and Claude buttons to switch providers. The messages are static samples.

Optional preview parameters: `provider=claude`, `scene=missing`, `scene=mixed` (Claude only), and `theme=dark`.

POC code is in this directory and `mockups/message-model-mockup.{html,tsx,css}`. Mallard's production parsers, DTOs, UI, and storage schemas are unchanged by this POC. The existing integration points are `providers/codex/history.rs`, `providers/claude/history.rs`, and `providers/traits.rs` under `crates/mallard-infrastructure/src`, plus `src/types.ts`.

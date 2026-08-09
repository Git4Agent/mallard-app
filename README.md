# Mallard — GitHub for AI-agent work

Git records what changed. Mallard preserves the Codex sessions that explain
why—and makes that project knowledge portable across teammates and machines.

Mallard was built for [OpenAI Build Week](https://openai.devpost.com/), a
global week of building with Codex and GPT-5.6. It is a review-first desktop
app for syncing selected project-scoped agent sessions and independent global
Library resources without copying an entire provider home or credential
store.

**[Project website](https://mallard-ai.com/)** ·
**[Devpost submission](https://devpost.com/software/agentgithub)** ·
**[Watch the demo](https://youtu.be/5Agu6OuaQLg)**

## How Codex was used

Codex was used end to end: Sol for product design and technical planning,
ImageGen 2 for design prototyping, and Terra for implementation and iteration.
Mallard is itself a feature for Codex, making project-scoped sessions and
resources portable across teammates and machines.

## What Mallard does

- Maps Codex sessions onto a Git branch's first-parent commit history, so a
  team can explore the reasoning behind a diff.
- Parses session summaries and conversation previews locally, with metrics for
  start and end times, user turns, tokens, agent messages, and tool calls.
- Syncs selected project conversations and provider-home resources — memory
  and session state — through a local folder or Cloudflare R2.
- Compares local and stored resources before Push or Pull, including
  local-only, storage-only, ahead, synchronized, and conflict states.
- Publishes immutable, SHA-256-verified bundle revisions and presents an
  explicit restore plan before applying changes. Pulls create backups and
  retain apply receipts.
- Optionally syncs selected ordinary files and folders for non-Git projects.
  Git-managed project content continues to travel through Git.
- Manages user-global Agent Skills, provider-native plugin install intent, and
  Codex pets as independent Library resources. The Catalog menu starts empty
  and grows one row per **(provider home × resource kind)** the user adds;
  inside a row, each resource links to storages individually and then exposes
  per-link Push, Pull, and Refresh plus local Fork for skills and pets.

The first Library catalog supports Codex and Claude skills/plugins and Codex
pet v1/v2. Pet formats are separate resources — and separate menu rows — and
the user chooses the exact format to operate on. Skill discovery is limited to
`<CODEX_HOME>/skills`, `$HOME/.agents/skills` (default `~/.codex` config
only), and `<CLAUDE_CONFIG_DIR>/skills`; project-local skill directories are
not scanned.

Library resources use a clean cutover: there is no backward compatibility,
migration, legacy execution path, or parallel storage-wide catalog.

## Quick Start for judges

### 1. Install Mallard

The fastest path is the packaged Apple Silicon DMG from the
[v0.1.2 release](https://github.com/Git4Agent/mallard/releases/tag/v0.1.2).
The app is ad-hoc signed and unnotarized, so macOS may require you to approve
it under **System Settings → Privacy & Security** before the first launch.

You will also need Git and a local Codex profile. The default profile is
normally stored at `~/.codex`.

To run the desktop app from source instead, install Node.js with npm and a
stable Rust toolchain, then run:

```sh
npm install
npm run tauri dev
```

`npm run dev` starts only the Vite frontend and does not provide the Tauri
filesystem and storage backend used by these test flows.

### 2. Connect the shared Cloudflare R2 storage

The bucket location and credentials are included in the private submission
details. Do not add them to this repository, a project file, or an issue.

1. In Mallard, open storage settings and add a **Cloudflare R2** storage.
2. Paste the supplied **S3 API URL**. Mallard derives the R2 account, endpoint,
   and bucket location from this URL; confirm the populated values.
3. Enter the supplied **Access Key ID** and **Secret Access Key**.
4. Save the storage.

Credentials and machine-specific paths are stored only in Mallard's local
metadata under `~/.mallard/`; they are never uploaded inside a portable
project bundle.

### Agent-assisted R2 setup and packaged CLI

Builds produced from this source tree package `mallard-cli` inside both the
macOS app and the Windows per-user installer. In **New storage → Cloudflare
R2**, **Set up with an agent** copies the exact packaged executable path and a
credential-free instruction. The installer does not modify PATH.

The CLI can connect existing credentials, provision a new bucket and
exact-bucket credential, verify a configured storage, and explicitly recover
an interrupted setup:

```sh
mallard-cli storage r2 --help
mallard-cli storage list
mallard-cli storage verify <storage-id>
mallard-cli storage r2 cleanup <setup-id>
```

Secret Access Keys and Cloudflare bootstrap tokens are accepted only through
a hidden prompt or a bounded stdin option, never through command arguments or
environment variables. See
[PLAN_AGENT_ASSISTED_R2_SETUP.md](./docs/PLAN_AGENT_ASSISTED_R2_SETUP.md) for
the security, recovery, and packaging contracts.

### Test Flow 1: Pull the public demo project

This flow demonstrates how an existing Mallard project is discovered and
pulled onto a new machine.

1. Clone the public demo repository:

   ```sh
   git clone https://github.com/Git4Agent/mallard_test_demo.git
   ```

2. In Mallard, choose **Add project** and select the cloned
   `mallard_test_demo` folder.
3. Select the local Codex profile and the R2 storage configured above.
4. Under **Repository**, choose the existing remote repository marked
   **Git match** instead of creating a new repository.
5. Choose **Finish & review**, inspect the Pull plan, approve the intended
   resources, and apply the Pull.
6. Open **Project History** and verify that the project shows:
   - its Git commits and branch timeline;
   - mapped sessions plus any intentionally unmapped sessions;
   - always-visible session summaries;
   - session metrics for activity time, turns, tokens, messages, and tools;
   - expandable conversation previews.

   ![Mallard project history after pulling the public demo project](./docs/images/test-flow-1-pulled-project-history.png)

7. Click **Open ChatGPT Desktop** for a session and verify that the restored
   conversation opens in the desktop app.

   ![Restored conversation open in ChatGPT Desktop](./docs/images/test-flow-1-open-chatgpt-desktop.png)

### Test Flow 2: Push and transfer your own project

This flow works with a new or existing Git project, or with a non-Git folder.

1. Use the project from Codex so it has at least one project-scoped session.
   For the clearest Git mapping, make one or more commits after or during that
   work.
2. In Mallard, choose **Add project**, select the folder and Codex profile,
   choose the configured R2 storage, and select **New repository**.
3. Finish setup and open **Project History**. Git projects show the selected
   branch's commit-to-session mapping; both Git and non-Git projects show
   summaries, conversation previews, and session metrics.
4. Choose **Push** and review **Git & sessions** and the final **Review**
   step.
5. For a non-Git project, optionally open **Project files**, run
   **Scan files**, and select the ordinary files and folders that
   should travel with the bundle. This step is unavailable for Git-managed
   content.
6. Complete the Push to publish the reviewed bundle to R2.
7. On another machine, configure the same R2 storage, clone the Git repository
   or create an appropriate local folder, add it to Mallard, select the
   existing remote repository, and complete a reviewed Pull.

## How session-to-commit mapping works

Mallard parses Codex JSONL archives locally to recover session metadata,
activity windows, branch information, and recorded commit references. For a
Git project, it places sessions on the selected branch's first-parent history
when the evidence is strong enough: a commit can occur during the session or
be the nearest follow-up commit within 24 hours. Ambiguous sessions remain
visible as unmapped rather than being assigned to a commit without evidence.

## Push, Pull, and project-file safety

- Every Push and Pull is reviewed by resource before it changes shared or
  local state.
- Mallard captures only provider-home state; it never captures configuration
  or instruction files from inside a Git-managed project directory. That
  content travels with Git.
- Non-Git project-file scans are explicit and read-only. `.gitignore`,
  `.ignore`, and `.mallardignore` are honored.
- VCS data, build output, Codex and Mallard metadata, credential filenames,
  links, special files, and private-key material are excluded or blocked.
- Credential-shaped or executable content requires an explicit warning
  acknowledgement.
- Pull deletions begin unselected. Approved file deletion verifies the local
  digest and creates a backup; approved directory deletion removes only an
  exact empty directory and is never recursive.
- Absolute checkout paths, Codex-home paths, storage credentials, restore
  plans, receipts, backups, and local preferences stay on the machine.

## Architecture

Mallard is a Tauri 2 desktop application. React, TypeScript, and Vite provide
the review-focused interface; Rust handles discovery, validation, hashing,
safe filesystem operations, local persistence, and S3/R2 transport.

Local-folder and R2 storage share the same versioned bundle model. A mutable
head points to immutable manifests and uploaded objects, with SHA-256 checks
and compare-and-swap publication protecting concurrent updates.

See [TECHNICAL_REPORT.md](./TECHNICAL_REPORT.md) for the current schemas,
storage layout, sync semantics, and implementation boundaries.

## Development

Requirements: Node.js with npm, a stable Rust toolchain, and Git.

```sh
npm install
npm run dev          # frontend only at http://localhost:1420
npm run tauri dev    # complete desktop app
```

Configuration is created through the app; no `.env` file is required.

## Verify

```sh
npm run build
npm run test:frontend-integration
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:cli
```

The Rust suite includes tests that bind localhost for the stub S3 transport;
restricted environments may require permission for that test server.

## Code map

- `src/components/project-sync/` — project setup, Activity, storage, Push/Pull
  review, project files, and status UI.
- `src/components/project-sync/api.ts` and `src/types.ts` — Tauri command and
  DTO contracts.
- `crates/mallard-application/` — storage setup and Cloudflare provisioning
  use cases shared by desktop and CLI adapters.
- `crates/mallard-cli/` — the packaged storage CLI, hidden-input handling,
  stable output schema, and process exit contract.
- `crates/mallard-infrastructure/src/providers/` — the compile-time provider
  registry and the Codex and Claude adapters (profiles, discovery, history,
  codecs, materialization, and session actions).
- `crates/mallard-infrastructure/src/project_activity.rs` — provider-neutral
  Activity composition, session details, metrics, and Git mapping.
- `crates/mallard-infrastructure/src/provider_capture.rs` — registry-driven
  resource discovery and no-follow project-content scanning.
- `crates/mallard-infrastructure/src/bundle_engine.rs` — scoped publish,
  fetch, integrity checks, restore planning, backups, and apply.
- `crates/mallard-infrastructure/src/storage_core/` — the shared object-store
  transport, including the local-folder and S3/R2 stores.
- `crates/mallard-infrastructure/src/r2_setup_journal.rs` — bounded,
  non-secret recovery state for interrupted R2 probes and token cleanup.
- `src/components/storage-catalog/` and `src-tauri/src/storage_catalog/` — the
  independent scope-rowed Library UI, provider adapters, plans, persistence,
  per-resource remote protocol, and journaled apply behavior.

## License

Licensed under the [Apache License 2.0](./LICENSE).

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
  and grows one row per **(provider home × resource kind)** the user adds. A
  row's page works like a project's: one storage chosen in the path bar, one
  list of resources each wearing the same status badge a thread wears, and
  Push and Pull as page verbs with a review. Pairing a resource with a storage
  is a side effect of those verbs, not a step the user takes; Refresh, Unlink,
  and local Fork stay behind the individual resource.

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

Install Cloudflare Wrangler and make sure `wrangler` is available on `PATH`.
The copied agent instruction checks this first. If Wrangler is missing, it
stops, asks for approval, and shows `npm install --global wrangler@latest`;
it never installs the dependency without explicit approval. Mallard invokes
`wrangler` directly, so a project-local-only install is not sufficient.
Mallard uses Wrangler's authenticated Cloudflare session; the setup flow can
open the system browser if sign-in or consent is required.

1. In Mallard, open storage settings and add a **Cloudflare R2** storage.
2. Either enter the five connection fields directly, or choose a storage name,
   select **Set up with an agent**, and paste the copied instruction into the
   agent.
3. For agent setup, the agent may need separate one-time execution approval
   before it starts the exact Mallard CLI setup command. This approval lets
   that command write the current user's `~/.mallard` state and launch the
   system browser; it does not authorize a shell, a broader command prefix,
   or browser control. If approval is declined, setup stops instead of using
   another home, temporary state, `sudo`, or direct configuration edits.
4. In the system browser, personally review and complete Cloudflare login,
   MFA, and consent. The agent does not control or inspect the browser and
   never receives credentials.
5. Wait for the CLI to fill the open Mallard setup draft. Review or edit the
   populated connection values in Mallard, optionally choose **Test
   connection**, and then personally choose **Finish setup**. The CLI does not
   test or commit the storage. Mallard loads the Secret Access Key only for the
   exact draft opened in the editor, where the existing **Show** control lets
   you review it; background draft listings and committed storage snapshots
   remain redacted.

Manual **Finish setup** and **Save storage** use the same exact R2 capability
probe before committing connection changes. Leaving the Secret Access Key
blank while editing an existing storage preserves the backend-held value.

Credentials and machine-specific paths are stored only in Mallard's local
metadata under `~/.mallard/`; they are never uploaded inside a portable
project bundle.

### Agent-assisted R2 setup and packaged CLI

Builds produced from this source tree package `mallard-cli` inside both the
macOS app and the Windows per-user installer. In **New storage → Cloudflare
R2**, **Set up with an agent** copies the exact packaged executable path and a
credential-free instruction. The installer does not modify PATH.

R2 has one setup command. The copied instruction first saves the exact open
schema-1 setup draft and includes its non-secret ID and expected revision in
the command. The CLI uses the current Wrangler login, selects the only
available account, creates or reuses a deterministic bucket with a readable
slug derived from the storage name, opens Cloudflare Pipelines' hosted
credential-consent broker in the system browser,
receives the callback on loopback, and revision-safely fills that same draft:

```sh
mallard-cli storage r2 --help
mallard-cli storage r2 setup --name "Team backup" \
  --draft-id "storagedraft-…" --expected-revision 3
```

The user—not the agent—reviews browser login, MFA, and consent. Credentials
never appear in a command argument or environment variable, and the CLI does
not print them. If Wrangler exposes multiple accounts, setup stops instead of
guessing which account to use. When the command succeeds, the running app
uses the redacted background update only as a signal, exact-loads that one
draft into the editor, and leaves every field—including the revealable Secret
Access Key—editable. The user may test the connection and is the only person
who finishes the setup; that final in-app action performs the capability probe
and commits the storage. The agent stops at the CLI success or failure result;
it does not invoke Computer Use, take screenshots, automate the browser, or
inspect Mallard's window to verify the draft.
Discarding the Mallard draft removes only the local draft. It does not revoke
an R2 access key or delete a bucket that the user approved in Cloudflare; if
the setup is abandoned, remove those resources from the Cloudflare dashboard.

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

Application configuration is created through the app; normal development does
not require an `.env` file. The opt-in paid live test has a separate local
connection input described below. The additional aimock live tests are
entirely local and do not use that connection file.

## Verify

```sh
npm run build
npm run test:frontend-integration # strict test type-check plus frontend tests
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:backend-integration  # all Rust targets plus loopback aimock tests
npm run test:integration          # all committed standard suites
```

The backend suite includes Rust and aimock server tests that bind localhost.
Restricted environments may require permission for these test servers. The
aimock server tests use fixed local responses and require no credentials.

### Live Codex API end-to-end test

The API-backed live Codex test is deliberately separate from
`npm run test:integration`. Its five source-conversation turns use an
OpenAI-compatible Responses provider; its twenty final-history reopen checks
use a bounded loopback Responses recorder and make no remote model request.
Every phase uses only fresh temporary Codex homes, projects, Mallard
repositories, and local-folder storage. The test never reads a user Codex
home, requires a Codex login, or reads, copies, or creates `auth.json` or
another legacy credential file. The ignored `.env.local` described below is
the one intentional credential input.

Create an ignored `.env.local` at the repository root with the provider
connection document:

```json
{
  "_type": "newapi_channel_conn",
  "key": "<provider-api-key>",
  "url": "<https-provider-base-url>"
}
```

On Unix, restrict it to the current user before running the test:

```sh
chmod 600 .env.local
```

The runner accepts exactly those three JSON fields, validates the HTTPS base
URL, and fails closed for a missing, malformed, symlinked, oversized, or
over-permissive file. `.env.local` is covered by the repository's `*.local`
ignore rule and must never be committed.

The current rollout fixtures have a hard `codex-cli 0.147.x` prerequisite. The
test rejects another CLI version before making a model request; update the
current-format fixture in place before moving the gate to another version.

Run it only when you intend to spend API tokens:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The explicit paid-use gate remains required even when `.env.local` is valid.
For each of the five provider-backed conversation processes, the harness keeps
user configuration disabled, passes a closed set of non-secret custom-provider
settings through repeated Codex CLI configuration overrides, and exposes the
API key only in that child process's dedicated environment variable. It does
not write the key into a Codex config, prompt, command argument, rollout,
snapshot, Mallard object, or test log. The twenty loopback children receive no
real provider URL or credential. All command shapes set
`features.plugins=false`: Codex marketplace synchronization is outside this
scenario, while the skill and pet phases use Mallard's isolated filesystem
fixtures directly.

The test runs exactly 25 Codex CLI turns with no retry path: five real,
provider-backed `gpt-5.6-luna` conversation turns at low reasoning effort and
twenty deterministic loopback headless resumes. The count is deliberately a
CLI-turn count; the revision-1 tool-using turn can require more than one
provider HTTP exchange. The scenario exercises two isolated machine homes
through project revision 10, including paginated dependency histories,
reviewed Push/Pull, stale-plan rejection, divergence in both directions, Keep
Both, and a no-op fixed point. The same topology also shares and updates nested
Codex skills and fake v1/v2 pets through the storage Library without additional
provider-backed turns.

For all ten final aliases in both homes, the test first resolves and pages the
exact history through the production Codex history and Session Detail paths.
It then runs public `codex exec --json resume --ephemeral` against a one-request
HTTP server bound to literal `127.0.0.1`. The recorder validates the complete
ordered history Codex sent for that exact alias before returning a fixed SSE
response. Each verifier operates on a disposable snapshot because Codex 0.147
appends a turn even with `--ephemeral`; both authoritative homes must remain
byte-identical.

The exact API gate passed on August 19, 2026: all five provider-backed turns
and twenty loopback resumes completed in one 75.91-second Rust test run.

See [`docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md)
for the API topology, POC evidence, revision matrix, and safety assertions.

### Live Claude Code Tuzi end-to-end test

The paid Claude target uses the same ignored repository-root `.env.local`
connection document, but requires its HTTPS URL to be the exact Tuzi site
root (an input ending in `/v1` is normalized back to the root because Claude
Code appends `/v1/messages`). It is separate from `npm run test:integration`
and from the credentialless AIMock target.

Run it only after checking the Tuzi key's quota and consumption log and when
you intend to pay for three Sonnet 5 turns:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
```

The runner requires Node.js 20.15.0 or newer and exactly Claude Code 2.1.235.
Before Cargo starts, it validates `.env.local` as a bounded, no-follow,
mode-`0600` three-field JSON document and performs a bounded authenticated
catalog lookup for exact model ID `claude-sonnet-5`. It never passes the key
through Cargo's environment. The Rust target independently reopens and
validates the connection file, then exposes the key only to each direct
Claude child as `ANTHROPIC_API_KEY`.

The test makes exactly three Claude model-turn processes with automatic
retries disabled, no fallback model, no tools, `--effort low`, a 1,024-token
output limit, and a USD 0.05 Claude client cap per process. It creates a real
session on replica A, publishes revision 1, pulls and reidentifies it onto
replica B, resumes the relocated transcript through Tuzi, and publishes
revision 2. Replica A then resumes its stale base independently. The test
requires Push to be blocked, reviews a divergent Pull offering exactly one
Keep Both action, preserves A's local continuation byte-for-byte, publishes
both lineages as revision 3, and reconciles exact bindings, comparison rows,
provider UUIDs, and root-level Claude JSONLs on both replicas. It finishes by
applying reviewed empty Pull plans and proving no-op Pushes and byte-identical
storage.

All Claude homes, projects, process homes, Mallard repositories, and storage
are private temporary roots. The test uses fixed synthetic markers and sends
no repository source or personal conversation. It validates the production
Claude adapter, paged Session Detail history, transcript structural rewrites,
semantic lineage preservation, credential absence, portable-manifest path
privacy, and full-depth no-follow state scans. The USD caps are client-side
estimates; Tuzi's final consumption log remains authoritative, including for
failed or disconnected calls.

The final gate passed on August 19, 2026 with `1 passed, 0 failed`; the Rust
body completed in 13.47 seconds and the complete package command in 18.14
seconds. During implementation, one earlier execution completed only its
first paid turn before a local plan-action assertion failed; the final
three-turn execution then passed. Neither execution automatically retried.
Check the Tuzi consumption log for the four development turns before running
the paid gate again.

See [`docs/PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md)
for the research, one-turn POC evidence, three-revision implementation, cost
boundary, and exact limitations.

### Keyless aimock Codex end-to-end test

The additional aimock suite drives the same installed Codex CLI and Mallard
two-replica behavior against a deterministic
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not open `.env.local`, needs no
API account, key, Codex login, GPU, or local model, and makes no external model
request. It is a separate test transport, not a fallback or compatibility path
inside Mallard.

Install dependencies and run it with its independent heavyweight-test gate:

```sh
npm install
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Run the gate from an ordinary Terminal. A development agent already enclosed
by macOS Seatbelt may need permission for the outer test process so Codex can
initialize its nested sandbox; the test still keeps Codex itself at
`workspace-write` with only the reviewed additional directory. The runner
detects the known nested-Seatbelt failure with a bounded, no-provider preflight
before starting Cargo and prints a fixed ordinary-Terminal instruction.
Implementation, protocol POCs, and the complete ignored live gate are verified.
The exact keyless gate passed on August 19, 2026 with `1 passed, 0 failed, 7
filtered` in 90.48 seconds. Every model-shaped response came from the one local
aimock child; the run used no provider account, credential, remote model, GPU,
or external model request.

The Rust test owns the mock lifecycle, supplies a keyless custom provider with
`requires_openai_auth=false`, strips inherited provider/proxy state, disables
plugins and request compression, and permits no retries or Authorization
header. Strict fixture matching rejects unknown, repeated, reordered, or
history-incomplete requests; no proxy, recorder, remote fixture, or upstream
is configured.

The aimock scenario adds native Codex pagination coverage. One bounded
`codex app-server --stdio` process creates P0-P4 with paginated
`thread/start`/`thread/fork` operations, which must consume no Responses
request. Five source turns, five native app-server graph turns, seven later
graph-mutation turns, and twenty final history-resume turns make 37
deterministic turns and exactly 38 local `/v1/responses` requests. R1 uses two
intentional scenario legs with the same prompt marker: an ordinary
`exec_command` function call invokes the reviewed `apply_patch` command, then
an exact tool-result call ID selects the fixed final response. Every other
prompt marker is unique, and every fixed response is selected only after the
mock validates the complete ordered marker history.
Mallard planning may additionally start bounded, ephemeral Codex app-server
children only to allocate native UUIDv7 thread IDs. Those children perform no
turn, disable plugins, run under disposable provider/process homes, inherit no
provider or proxy credentials, and are excluded from deterministic turn and
Responses-request accounting.

Codex `MaterializeCloud` is always copy-on-write. A native fork can begin
referencing an unchanged parent between review and apply, so neither a digest
pin nor a dependency rescan can make replacement of that parent atomic with
Codex's external writer. Pull instead assigns the standalone history a fresh,
reviewed-absent alias, preserves the prior rollout bytes, and inserts the new
index entry without clobbering the old one. Binding/receipt history suppresses
no content by itself: it derives a domain-separated retained-branch identity,
and an immutable whole-file checkpoint hides the displaced physical alias only
while its exact bytes still match the reviewed source. Missing evidence, a
read failure, or any later append fails open and surfaces that retained branch
in review, Activity, and Session Detail; publishing it creates a direct
binding. Revision 10 proves this for a referenced root, revision 7 proves the
same copy-on-write rule for an unreferenced P2 control, and a focused backend
regression exercises a changed retained branch plus repeated R1-to-R2
displacement. Providers without Codex's physical rollout graph retain their
existing in-place update behavior.

The native POC also proved that
`history_base.end_ordinal_exclusive` is a global top-level rollout boundary,
not a physical JSONL row count: the child `session_meta` starts at that ordinal
and subsequent rows are contiguous. Mallard now enforces those current
paginated ordinals and decodes portable Codex display titles on Pull in the
single schema-1 path. Neither correction adds a fallback, migration, old-format
path, acceptance of ordinal-less paginated data, or schema change. Codex's
upstream `history_mode: "legacy"` remains its separate current ordinal-less
source mode, not a Mallard legacy execution path.

Both suites exercise the current schema-1 product path. Adding the local mock
does not add a migration, legacy execution branch, or provider fallback to the
application. Copy-on-write materialization likewise changes only the current
execution path: schema 1 remains unchanged, with no migration or compatibility
path.

See [`docs/PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md)
for the aimock topology, protocol POCs, exact accounting, and safety contract.

### Credentialless aimock Claude Code end-to-end test

The Claude Code aimock suite is a separate ignored target for the real Claude
CLI and Mallard's production Claude adapter. It runs against one strict
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not read `.env.local`, a user
Claude home, or `.credentials.json`, and it needs no provider account, login,
real API key, remote model, GPU, or paid token.

Install dependencies and run the explicitly gated target with:

```sh
npm install
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

The current native-format gate requires exactly Claude Code `2.1.235`,
`@copilotkit/aimock` `1.39.0`, Node.js 20.15.0 or newer, and the mock model
identifier `claude-sonnet-4-6`. The runner strips inherited provider,
provider-home, endpoint, and proxy state before Cargo starts. Each Claude
turn uses `--bare`, `--permission-mode dontAsk`, and a one-tool `Read` allow
list. Claude requires a non-empty `ANTHROPIC_API_KEY` for this custom endpoint,
so the child receives the fixed non-secret placeholder
`mallard-local-aimock-placeholder`. The mock requires that exact value in
`X-Api-Key`; `Authorization` and alternate credential headers are forbidden.
This is credentialless local-mock testing, not a headerless or keyless flow.

The scenario creates five real source turns across two isolated Claude homes
and five Mallard revisions. Revision 1 performs a real `Read` tool round;
later turns exercise Pull relocation and reidentification, native resume on a
second canonical project path, divergence in both directions, both Keep Both
flows, three final cloud lineages, paged Session Detail history, and reviewed
revision-5 no-op Push/Pull. Six additional snapshot resumes verify every final
lineage from both replicas without changing either authoritative Claude home.

The accounting is exact: 11 Claude model-turn processes make 12 authenticated
`POST /v1/messages?beta=true` requests, because the revision-1 tool round uses
two Messages calls. Claude Code 2.1.235 also makes one credential-free
`HEAD /api/hello` compatibility probe per process. The raw boundary therefore
requires exactly 23 requests: 12 model requests plus 11 probes. The Messages
requests are separately reconciled with all 12 semantic aimock scenarios. Any
duplicate scenario, other route or method, extra request, upgrade, wrong key,
credential-bearing probe, history mismatch, or exposed tool makes the audit
fail. No upstream, proxy, recorder, remote fixture, or retry path is
configured.

The package runner and Rust target also perform their own bounded, isolated
`claude --version` preflights. Those are not model turns, start before the mock
traffic, and make no request to aimock; the exact process budget above is
deliberately the model-turn budget.

The fully hardened gate passed on August 19, 2026: `1 passed, 0 failed`; the
Rust test body completed in 12.91 seconds (14.80 seconds for the package
command), with all 11 Claude model-turn processes, 12 model requests, and 11
native hello probes accounted for.

See [`docs/PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md)
for the initial POC evidence, five-revision matrix, final history oracle,
implementation notes, and safety contract.

## Code map

- `src/components/project-sync/` — project setup, Activity, storage, Push/Pull
  review, project files, and status UI.
- `src/components/project-sync/api.ts` and `src/types.ts` — Tauri command and
  DTO contracts.
- `crates/mallard-application/` — the verified, atomic storage setup use case
  shared by the desktop state and CLI adapter.
- `crates/mallard-cli/` — the packaged storage CLI, Wrangler/browser R2
  onboarding, stable output schema, and process exit contract.
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
  non-secret recovery state for interrupted R2 capability probes.
- `src/components/storage-catalog/` and `src-tauri/src/storage_catalog/` — the
  independent scope-rowed Library UI, provider adapters, plans, persistence,
  per-resource remote protocol, and journaled apply behavior.

## License

Licensed under the [Apache License 2.0](./LICENSE).

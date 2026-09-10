# Plan: local mock E2E for multiple subagents

Date: 2026-09-06.
Status: Codex transport and E2E scope superseded on 2026-09-07 by
[PLAN_CODEX_OWNED_SUBAGENT_SYNC.md](PLAN_CODEX_OWNED_SUBAGENT_SYNC.md).
Claude transport and E2E scope superseded on 2026-09-08 by
[PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md](PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md).

The owned-child plan is the sole Codex feature and verification authority.
Its role, identity, ownership, comparison, omission, and cutover rules replace
the Codex proposals below, including the Sidecar representation and identity
reservations in Review. Codex scenarios below are historical test ideas;
reuse them only after they conform to the owned-child plan.

The Claude child plan is the sole Claude feature and verification authority.
Its scope is direct children only. It replaces the Claude proposals below,
including the Sidecar role, Review identity reservations, and stale-review
byte rules. The remaining text is design history; use test ideas only under
the current provider plans.

Extend both local mock E2E commands. Create parent threads with multiple
subagents, Push them through Mallard, Pull them into another provider profile,
and open the restored threads in the native Codex and Claude desktop clients.
The user selected native-client verification. Mallard Session Detail is not
the final visibility check.

Use the [local mock POC](POC_LOCAL_MOCK_SUBAGENTS.md) as protocol evidence.
Use current source code as the implementation baseline. Apply the Codex
and Claude supersession above.

| Current code | Required change |
| --- | --- |
| Codex discovery excludes internal child rollouts. Codex materialization rejects sidecars. | Attach proven child rollouts to the selected parent and restore them with it. |
| Claude discovery captures root transcripts, `todos`, and `file-history`. It does not capture nested subagent transcripts. | Capture and restore the parent's referenced `subagents` files. |
| Stock `aimock` 1.39.0 drops Codex tool namespaces and `agent_message` records. | Preserve the native Responses fields in the repo mock handler. |
| The Claude source runner uses `--bare` and a `Read` tool list. | Add an isolated Agent-enabled run mode for the new phase. |
| Current reopen checks verify parent history through headless CLI resumes. Snapshot helpers copy parent data. | Include restored children and verify their native detail views. |
| The Codex scenario document already has 62 requests; its limit is 64. | Give the new phase a separate mock instance and request budget. |

The transport changes are prerequisites. A test that only finds child result
text in the parent transcript does not meet this request.

1. **Fix the test scope and native-reader contract.**

   Use the following matrix for each provider:

   | Item | Test value |
   | --- | --- |
   | Source | Profile A with its own provider home and project path |
   | Destination | Profile B with a different profile ID, provider home, and project path |
   | Wrong-profile control | An unselected profile C with sentinel data |
   | Parent threads | Two, named P1 and P2 |
   | Children | Three direct children per parent; six distinct children per provider |
   | Storage | A fresh local-folder store shared by A and B |
   | Lifecycle | All children complete before Push review |

   Give every child a unique task marker and result marker. The final parent
   reply contains only a parent completion marker. Keep a test-owned expected
   table of parent, child, task, status, and result. Do not build expected
   membership from the pulled files.

   First extend the POC from one child to three. Confirm that the installed
   native clients can open all three source child detail views. Record CLI,
   desktop-client, and bundled runtime versions separately. Confirm that
   isolated app state and provider homes work before developing the UI gate.
   Do not assume that the desktop runtime matches the CLI on PATH.

2. **Extend the mock protocols.**

   For Codex, use the POC's `gpt-5.4` mock label and enable
   `features.multi_agent_v2`. Serve native `function_call` items with separate
   `namespace` and `name` fields. Preserve `agent_message` input items. Add
   bounded scenario variants for spawn, child response, wait, and parent
   completion. Validate the advertised collaboration tools. Keep existing
   text and file-command cases working through the same Responses handler.
   Do not retain a second lossy Responses path.

   For Claude, add an Agent-enabled mode without `--bare`. Use fresh process
   and provider homes, empty settings sources, disabled hooks, an empty MCP
   configuration, and explicit tool lists. Parent scenarios allow `Agent`;
   the first child fixtures require no tools. Existing `Read` scenarios keep
   their closed tool contract. The paid Tuzi path remains outside this work.

   Match requests by parent marker, child marker, call ID, and stage. Child
   completion order must not choose the fixture. Return parent completion
   only after the runtime delivers all three actual child results. Add a
   bounded wait schedule and reconcile every request with that schedule.
   Determine exact phase totals from the three-child protocol probe; do not
   multiply the single-child POC totals. Count child model requests separately
   from top-level CLI processes and Claude hello probes.

   Mounted Codex requests bypass the normal aimock request journal. Extend
   the raw and semantic audits so these requests cannot escape accounting.
   Keep fixed loopback endpoints, no upstream configuration, bounded reads,
   strict misses, and sanitized audit output.

3. **Transport children as parent attachments.**

   Keep each parent as the review and sync unit. Children must not become
   independent Activity rows, root conversations, or separately selected
   Push/Pull actions. Preserve the exclusion of internal agents as roots.

   Use the existing `ConversationFileRole::Sidecar` representation with a
   closed `subagent-transcript` kind in each provider adapter. Keep native
   path rules and metadata parsing inside the provider modules. Extend the
   adapter contract for validation and transformation of child transcripts;
   do not send sidechain data through a codec that only accepts roots.

   Codex must prove both the parent's reference and the child's parent link.
   Claude must prove the parent tool-result agent ID, child filename, session
   identity, and sidechain metadata. Capture only proven children of the
   selected parent. Do not copy a whole provider home or project bucket.

   Define the parent-and-child identity map before implementing Pull. Store
   destination identity reservations in the reviewed local plan. Rewrite
   native structural references and filenames together. Preserve ordinary
   conversation text. Never use a global string replacement for IDs. Verify
   that parent reidentification cannot create a link to a source-profile
   child or an unrelated destination child.

   Include child content and membership in artifact change detection and
   integrity checks. Keep root message counts and root lineage separate from
   child history. Prove that encode/decode and destination identity changes
   do not cause false divergence or a new Push immediately after Pull.
   Reuse the current file and byte limits. Keep identity mappings, profile
   IDs, local paths, and restore state out of portable manifests. Update the
   codec contract if its canonical form changes; do not add a legacy decoder
   or migration path.

   Stage and verify the complete family before publication. Commit child
   files before the parent becomes available. Use the existing journal,
   expected-digest checks, no-clobber writes, and recovery rules. A missing,
   invalid, or changed child must not produce a successful complete-family
   receipt.

4. **Add the profile-transfer phase to each E2E entry point.**

   Add one ignored Rust test to each existing integration target:
   `live_codex_aimock_subagent_profile_transfer` and
   `live_claude_aimock_subagent_profile_transfer`. Put the new flow helpers in
   focused modules under `src-tauri/tests/support/`. Reuse the production
   `ProjectSyncService` setup, review, Push, and Pull operations.

   Update each npm runner to invoke an explicit list of exact local test
   names: the current flow, then the new subagent flow. Each flow gets its
   own mock and temporary storage. Do not use an unfiltered `--ignored`
   command: the Claude integration target also contains a paid test.

   Execute this sequence in each new flow:

   - Register A, B, and the unselected control profile C.
   - Create P1 and P2 through the real provider CLI. Wait for all six children
     to finish and for their transcript writes to settle.
   - Verify source membership and native transcript structure.
   - Review and Push the two parents from A. Verify that the manifest contains
     both parents and exactly their six child attachments.
   - Select B explicitly in Pull review. Apply the reviewed plan through
     production code to B's different project path.
   - Verify both restored families, destination IDs, paths, and link maps.
     Verify that C's sentinel data is unchanged and that P1 cannot resolve a
     child from P2.
   - Verify no-op Push/Pull before opening the clients. Opening a history
     view must not create new child conversations.

   The destination must be populated only through Pull. Tests must not copy
   child files from A to B to make reopen succeed. Snapshot-based headless
   checks may copy B's restored family; they must not read A for repair.

5. **Open the pulled parents in both native desktop clients.**

   Add a local native-UI verification mode that keeps the new phase's mock
   server and temporary profiles alive until evidence is saved. Use a fresh
   native app instance and isolated app state. Resolve each parent through
   B's local conversation binding; do not launch with the source session ID.
   Reuse Mallard's provider action path for Open in Codex and Open in Claude.
   Confirm that the launch selected B's home and project.

   For each of P1 and P2, open the parent, open its subagent activity, and
   enter all three child detail views. Verify the child identity or task
   label, completed state, and full expected child result. All three must be
   reachable from the correct parent. A parent summary, a tool-call count,
   a successful CLI resume, or files on disk is insufficient evidence.

   Use native UI automation and accessibility text, with screenshots or a
   recording. Record the actual navigation for the pinned clients during
   the source probe. Do not invent a history API or assume a terminal view
   proves desktop visibility. Headless provider reads are intermediate
   checks only.

   Keep a separate result for transport and native visibility. Without a
   working desktop session, report native verification as `not_run` or
   `blocked`, never `passed`. If an app cannot use an isolated local profile,
   resolve that test-launch issue without switching to the user's real home
   or credentials. Stop only test-owned processes and remove temporary
   profiles after evidence capture.

6. **Add failure checks that can detect incomplete restoration.**

   | Check | Required outcome |
   | --- | --- |
   | Missing child in a fresh copy of B after successful Pull | The child-open verification fails even if the parent summary remains. |
   | Corrupt child object in storage | Digest verification rejects the affected family before it is reported complete. |
   | Child changes after review | Existing revision and digest checks reject stale approval. |
   | Wrong parent ID, duplicate child identity, or unrelated child | Discovery or apply rejects the invalid association. |
   | Wrong destination profile or an occupied target | No write to C and no overwrite of unrelated data. |
   | Symlink or path traversal in a child path | No traversal or write outside the selected profile. |
   | Repeat no-op Push/Pull | No duplicate children and no false divergence. |

   Keep small boundary tests beside the provider helpers. Keep full
   multiple-child creation, transfer, and reopen checks in the local E2Es.

7. **Verify and record the result.**

   Run the mock server tests and the affected Rust tests while implementing.
   Run the full Rust library tests before handoff. Run both existing gated
   local E2E commands; their runners must include the new phase. Complete the
   native UI gate for both providers and both parents. Run frontend checks
   only if a shared DTO or application UI changes.

   Save a sanitized result manifest with tested versions, per-phase request
   counts, parent/child counts, profile-isolation results, and separate native
   visibility status. Attach native screenshots or recordings that contain
   only fixture data. Update README with the local UI verification command
   and its desktop-session requirement.

| Work area | Main files |
| --- | --- |
| Mock protocol and audit | `scripts/live-codex-aimock-server.mjs`, `scripts/live-claude-aimock-server.mjs`, and their tests |
| Exact test dispatch and isolation | `scripts/run-live-codex-aimock-e2e.mjs`, `scripts/run-live-claude-aimock-e2e.mjs` |
| E2E flow and expected results | `src-tauri/tests/live_codex_aimock_e2e.rs`, `src-tauri/tests/live_claude_aimock_e2e.rs`, and focused support modules |
| Provider child capture and restore | `crates/mallard-infrastructure/src/providers/{codex,claude}/`, `provider_capture.rs`, and `providers/traits.rs` |
| Family identity, hashing, and apply | `codex_standalone_builder.rs`, provider codecs, `conversation_materialization.rs`, and `src-tauri/src/project_sync/service.rs` |
| Native open and evidence | Provider `actions.rs`, a local UI-verification runner, and the pinned native clients |

Completion requires four restored parent threads across the two providers,
with all 12 expected children visible through their correct parent in the
native clients. Passing only the mock protocol or transport checks does not
complete this plan. Nested delegation, active-child continuation, paid APIs,
and a new Mallard subagent UI are outside this first scope.

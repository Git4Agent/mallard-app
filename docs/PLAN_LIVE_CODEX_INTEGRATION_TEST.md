# Plan: Live Codex Project-Sync Integration Test

Status: complete. Revisions 1–5 were live-verified on 2026-08-17. The
paginated-chat, headless-reopen, skill, and pet expansion was implemented,
offline-verified, and live-verified with 25 Luna-low requests on 2026-08-18.

## 1. Outcome

Mallard now has an opt-in integration test that makes exactly twenty-five real
model requests with an installed, authenticated Codex CLI and drives the
production project-sync and storage-Library workflows end to end. Five requests
create conversation turns. Twenty headless requests reopen every final history
from both replicas through disposable Codex-home snapshots. The test covers
project setup, paginated dependency histories, local-folder storage, reviewed
Push publication, Pull planning and apply, stale plans, divergence in both
replica directions, Keep Both reconciliation, shared skills and fake pets, and
final convergence.

The normal fixture suites remain the default CI gate because they are fast,
offline, and deterministic. The live suite is deliberately separate because it
makes authenticated, paid requests and verifies that the rollout format emitted
by the installed Codex CLI still crosses Mallard's provider boundary.

The implementation retains the current product formats unchanged:

- machine-local Mallard state remains schema 1;
- portable bundle manifests remain schema 1; and
- no schema migration is required or introduced.

This is a clean cutover to the dedicated production harness. There is no
legacy test path, compatibility wrapper, migration path, or fallback path. The
temporary child-module POC under `commands.rs` was removed after the dedicated
test passed.

## 2. Production application seam

The reusable, non-Tauri application boundary is
`src-tauri/src/project_sync/service.rs`. `ProjectSyncService` accepts an
explicit `LocalStateRepository` and exposes the production setup, Push review
and publication, Pull planning and apply, comparison, and bundle-inspection use
cases.

Tauri commands call this same service for the supported project-sync path. The
standalone integration test injects two isolated repositories directly, so it
does not duplicate the workflow or depend on desktop runtime state. Progress
callbacks remain available to the Tauri adapter while the test uses the same
underlying orchestration without UI concerns.

## 3. Fresh isolated topology

Every run starts from a new temporary directory. It does not reuse a real
Mallard home, real project, prior test repository, or source Codex sessions:

```text
temporary root
├── machine1/              isolated Mallard/process home for replica A
├── machine2/              isolated Mallard/process home for replica B
├── myconf1/auth.json      protected copy used only by Codex A
├── myconf2/auth.json      protected copy used only by Codex B
├── project1/              temporary cwd for replica A
├── project1_copy/         temporary cwd for replica B
├── process-home/          isolated HOME during setup
└── mystorage1/            shared temporary local-folder object store
```

The repositories model two independent machines. They share one storage ID,
one bundle ID, and one local-folder object store, but use distinct host IDs,
replica IDs, project bindings, provider profiles, project roots, Codex homes,
and Mallard repositories. `TempDir` removes the topology when the test ends.

Only `auth.json` is copied from the selected source Codex home. No existing
sessions, configuration, plugins, skills, logs, rules, or global state enter
the test fixtures.

## 4. Dedicated live target and runner

The production scenario lives in the ignored integration target:

```text
src-tauri/tests/live_codex_e2e.rs
```

Its exact test name is:

```text
live_codex_mallard_setup_push_pull_divergence
```

The dedicated runner is `scripts/run-live-codex-e2e.mjs`, exposed as:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The source profile defaults to `~/.codex`; callers can select another source
with `MALLARD_LIVE_CODEX_SOURCE_HOME`. The runner verifies basic prerequisites
without printing the source path or authentication data, then invokes exactly
one ignored Rust test with `--exact`, `--nocapture`, and `--test-threads=1`.

The test is protected by both `#[ignore]` and the explicit
`MALLARD_RUN_LIVE_CODEX_E2E=1` gate. It is not included in
`npm run test:integration`, so normal builds and offline test runs cannot spend
tokens or require credentials.

## 5. Symmetric revision 1–5 scenario

The implemented scenario uses exactly five model requests.

### Revisions 1–3: replica A becomes stale

1. Replica A sets up `project1` as a new repository, creates a real Codex
   thread under `myconf1`, and publishes revision 1.
2. Replica B sets up `project1_copy` against the existing bundle, Pulls
   revision 1 into `myconf2`, and verifies that the materialized provider
   thread has a distinct provider ID and the correct temporary cwd.
3. Replica B resumes its thread and publishes revision 2.
4. Replica A independently resumes its revision-1 provider thread.
5. Replica A's Push is gated as `RequiresPull(RemoteAdvanced)`; the direct
   publication attempt is rejected and its logical thread is classified as
   divergent.
6. Replica A reviews a Pull whose conversation action is Keep Both, applies
   it, verifies the preserved logical/provider identities, and publishes
   revision 3.

### Revisions 4–5: replica B becomes stale

7. Replica B Pulls revision 3 and verifies that the stable primary plus the
   first preserved branch materialize without changing their identities.
8. Starting from the shared revision-3 primary, both replicas independently
   resume their provider aliases.
9. Replica A publishes first, producing revision 4.
10. Replica B's Push is now gated as divergent in the reverse direction.
11. Replica B reviews and applies Keep Both. Its local branch moves to a fresh
    preserved logical identity while replica A's revision-4 branch becomes the
    stable primary.
12. Replica B publishes both accepted branches as revision 5 without making a
    sixth model request.

### Fixed point

13. Replica A Pulls revision 5.
14. Both replicas verify that the primary and two preserved branches retain
    three distinct provider identities, every comparison row is synchronized,
    and no divergent, local-only, or storage-only work remains.
15. Another Pull has no applicable actions, and a reviewed Push from either
    replica reports zero changed resources while the remote head remains at
    revision 5.

## 6. Cost and process controls

Every request explicitly uses:

```text
--model gpt-5.6-luna
--config model_reasoning_effort="low"
--ignore-user-config
--ignore-rules
--sandbox read-only
```

Prompts prohibit tool use and request one exact short response. The harness
uses argv-only process execution, null stdin and stderr, bounded stdout,
bounded prompt and argument sizes, and a hard timeout. It removes inherited
OpenAI and Codex API-key environment variables so the copied `auth.json`
boundary is the one under test.

The test runs alone because it temporarily sets process-global `HOME` and
`USERPROFILE` while exercising setup. Each Codex child receives its own
temporary `CODEX_HOME`, machine home, and project cwd.

## 7. Credential, privacy, and cleanup assertions

The live target verifies all of the following against the real generated
rollouts and published local-folder storage:

- the source auth file is a regular, non-symlinked file within a strict size
  bound;
- copied auth files and their parent directories use restrictive Unix modes;
- source and copied auth metadata and content digests are unchanged after all
  five requests;
- the source Codex sessions inventory is unchanged;
- no portable manifest, storage object, or object key contains `auth.json`, an
  auth digest, or credential-derived test needles;
- no portable manifest or object key contains a Codex-home path, a Mallard-home
  path, a project path, the source-profile path, or the temporary root;
- stored objects are bounded regular files and the storage traversal does not
  follow symlinks; and
- all test-created state is contained under the fresh temporary root and is
  removed automatically.

Credentials, checkout paths, provider-home paths, restore plans, receipts,
backups, and trust decisions therefore remain machine-local, preserving the
schema-1 product safety contract.

## 8. Verification record

The dedicated production command completed successfully with five live Codex
requests on 2026-08-17:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

That passing run covered setup on both isolated replicas, revisions 1 through
5, Push gating in both stale-replica directions, both reviewed Keep Both
operations, privacy assertions, source-auth immutability, and the final no-op
fixed point. The prior three-request child-module POC was then removed.

The committed offline suites remain credential-free and continue to validate
the deterministic product behavior independently of the live provider check.

## 9. Expansion POC record

The expansion POC ran on 2026-08-17 against the installed `codex-cli 0.147.0`.
It used an isolated temporary Codex home, a protected copy of `auth.json`, a
temporary project, and a directly written two-segment paginated rollout. The
leaf referenced a marker that existed only in its base segment.

The leaf was resumed with exactly one request using:

```text
--model gpt-5.6-luna
--config model_reasoning_effort="low"
```

Codex returned the exact inherited marker:

```text
PAGINATED-BASE-7K3
```

Reported usage was 8,545 input tokens, 14 output tokens, and zero reasoning
tokens. This demonstrates that a directly written current-format paginated
dependency is resolved into model context, not merely parsed by Mallard.

The POC also established two fixture requirements for Codex 0.147:

- `session_meta` must include current inner fields such as `timestamp`,
  `originator`, `model_provider`, and `base_instructions`; and
- every top-level record in a paginated rollout must have a monotonically
  increasing `ordinal`.

A synthetic `thread_settings_applied` record produced a warning because its
current `thread_settings` payload was absent. The committed fixture will omit
that optional record or emit the current schema-correct form. It will not add a
legacy representation or compatibility fallback.

The copied authentication file remained byte-identical to its source. No real
Mallard state, cloud storage, source Codex session, or project file was
modified. The disposable POC scratch directories were deleted after
verification.

A follow-up headless-resume POC verified the complete ordered user/assistant
marker history through `codex exec resume --json --output-schema`. It also
proved that Codex 0.147 appends the verification turn to the resumed rollout
even when `--ephemeral` is present: the scratch rollout grew from 7 to 21
records and its digest changed. The committed test therefore never resumes a
session directly inside the authoritative `myconf1` or `myconf2`. It resumes a
fresh disposable snapshot and verifies that the authoritative home remains
byte-for-byte unchanged.

## 10. Expansion goals and cost contract

The expansion adds four capabilities to the existing live scenario:

1. current-format paginated chats with a nontrivial dependency DAG;
2. Push, Pull, stale reviewed Push, and divergent Pull coverage for those
   chats;
3. headless Codex CLI resume checks that prove the complete old history is in
   context for every final materialized conversation in both Codex homes; and
4. storage Library sharing, update, divergence, preservation, and convergence
   for Codex skills and fake v1/v2 pets.

The project-sync scenario continues to use the five requests already required
by revisions 1–5. Verifying old history through the public headless CLI needs
one resumed model request per final provider alias because
`codex exec resume --json` reports the new turn rather than dumping the
previous transcript.
The full ten-history-by-two-home matrix therefore adds twenty verification
requests, for a total recurring live-test budget of twenty-five requests.
Every request uses `gpt-5.6-luna` with low reasoning effort. Direct rollout
generation, skills, and pets add no requests.

The existing ignored test remains the single live orchestrator so every phase
shares the same fresh storage, projects, Codex homes, Mallard repositories,
and provenance map. Deterministic fixture and Library behavior also have
offline tests so failures are caught before a paid run.

## 11. Current-format paginated rollout support

Add a test-support `PaginatedRolloutFixture` that writes native current Codex
JSONL with:

- canonical UUID provider and session IDs;
- complete current `session_meta`;
- `history_mode: "paginated"` and a validated `history_base` where required;
- contiguous top-level ordinals;
- `task_started`, user and assistant response items, current
  `item_completed` lifecycle records, and `task_complete`;
- explicit parent-message IDs, including parents inherited from another
  physical segment;
- a final newline; and
- a matching `session_index.jsonl` title record.

The builder must calculate `end_byte_offset` and
`end_ordinal_exclusive` from the exact serialized prefix. It must never use
handwritten offsets. A companion helper will append complete turns while
preserving ordinal and parent-message continuity.

Before Mallard sees a generated conversation, the test will validate it with
the production rollout structure analyzer, history resolver, standalone
builder, and paginated projection fixpoint verifier. Assertions include the
resolved segment IDs, canonical message sequence, history partition, fixed
tail start, final message ID, and exact cutoff evidence.

The live test will probe `codex --version` before copying authentication or
starting a model request. The initial fixture supports the current 0.147.x
format. A different format fails with an actionable error; it does not select
an older writer. When Codex changes its native contract, the fixture is updated
in place and the obsolete format is removed.

Offline failure coverage will include:

- missing base segment;
- dependency cycle;
- invalid byte or ordinal cutoff;
- noncontiguous ordinals;
- truncated JSONL;
- shared-parent fan-out;
- inherited-history-only leaf;
- prefix mutation; and
- projection fixpoint failure.

## 12. Paginated dependency graph

After the existing revision-5 fixed point, replica A writes five native
paginated rollouts under `myconf1`:

```text
P0 shared root
├── P1 left parent       history_base = P0 after P0 turn 1
│   ├── P3 left leaf     history_base = complete initial P1
│   └── P4 empty leaf    history_base = same P1 cutoff; no own messages
└── P2 right parent      history_base = P0 after P0 turn 2
```

This graph covers depth, fan-out, two different cutoffs into the same parent,
cross-segment message parents, and a leaf whose visible history is entirely
inherited.

The initial visible assistant markers are:

| Conversation | Expected assistant markers |
| --- | --- |
| P0 | `GRAPH_BASE_1`, `GRAPH_BASE_2` |
| P1 | `GRAPH_BASE_1`, `GRAPH_LEFT_PARENT` |
| P2 | `GRAPH_BASE_1`, `GRAPH_BASE_2`, `GRAPH_RIGHT_PARENT` |
| P3 | `GRAPH_BASE_1`, `GRAPH_LEFT_PARENT`, `GRAPH_LEFT_LEAF` |
| P4 | `GRAPH_BASE_1`, `GRAPH_LEFT_PARENT` |

Frozen-prefix assertions ensure that P1, P3, and P4 never acquire
`GRAPH_BASE_2`; later P0 appends never leak into descendants; and later P1
appends never leak into P3 or P4.

## 13. Paginated revision 6–10 scenario

The expanded project-sync flow continues from revision 5:

### Revision 6: publish and materialize the graph

1. Replica A writes P0–P4 and publishes revision 6.
2. Replica B Pulls all five conversations.
3. Every B artifact has a fresh provider ID, the B project cwd, contiguous
   ordinals, the expected fixed tail, and a standalone paginated history with
   an explicit null base.

### Revision 7: mixed local, storage, and divergent state

4. Starting at revision 6, A appends `GRAPH_A_LOCAL_ONLY` to P1 and
   `GRAPH_A_DIVERGENT` to P3, then prepares but does not publish a reviewed
   Push.
5. B appends `GRAPH_B_CLOUD_ONLY` to P2 and `GRAPH_B_DIVERGENT` to P3, then
   publishes revision 7.
6. Applying A's stale reviewed Push fails because its reviewed storage head no
   longer matches.
7. A's fresh comparison reports exactly:
   - P1 `LocalAhead` and Push-eligible;
   - P2 `StorageAhead` and Pull-materializable; and
   - P3 `Diverged`, held from Push, and eligible for Keep Both.
8. A applies the reviewed Pull. P2 updates, P3 preserves A's local branch
   before installing B's storage branch, and P1 remains unchanged.

### Revision 8: publish the surviving A work

9. A publishes its P1 update and the preserved P3 branch as revision 8.
10. B Pulls revision 8 and verifies both identities and histories.

### Revision 9: reverse the divergence direction

11. A and B independently append to P4: A adds `GRAPH_A_REVERSE`, while B adds
    `GRAPH_B_REVERSE`. B also appends `GRAPH_B_ROOT_LOCAL` to P0 and prepares a
    reviewed Push pinned to revision 8.
12. A publishes P4 first as revision 9.
13. B's stale reviewed Push fails. A fresh review holds divergent P4 while P0
    remains Push-eligible.
14. B applies Keep Both for P4. Its local branch is preserved, A's branch
    becomes primary, and B's P0 change is not overwritten.

### Revision 10 and fixed point

15. B publishes its P0 update and preserved P4 branch as revision 10.
16. A Pulls revision 10.
17. Both replicas report revision 10, another Pull has no applicable mutation,
    another reviewed Push changes zero resources, and the storage head and
    object count remain unchanged.

The final logical histories are:

| Logical history | Expected assistant markers |
| --- | --- |
| P0 primary | base 1, base 2, B root-local |
| P1 primary | base 1, left-parent, A local-only |
| P2 primary | base 1, base 2, right-parent, B cloud-only |
| P3 primary | base 1, left-parent, left-leaf, B divergent |
| P3 preserved | base 1, left-parent, left-leaf, A divergent |
| P4 primary | base 1, left-parent, A reverse |
| P4 preserved | base 1, left-parent, B reverse |

After revision 10, A's native descendants are resolved again. Replacing or
appending to P0 must retain every referenced prefix byte-for-byte. An invalid
child cutoff is treated as a product failure, not normalized away by the test.

## 14. Headless Codex CLI old-history verification

The test verifies every final provider alias with the public headless CLI.
Because Codex 0.147 persists a resumed turn even with `--ephemeral`, each
invocation receives a fresh bounded, no-follow snapshot of the relevant
isolated Codex home. The authoritative `myconf1` and `myconf2` trees are never
passed to the verifier as `CODEX_HOME`.

The command against each disposable snapshot is:

```sh
codex exec resume \
  --ephemeral \
  --json \
  --model gpt-5.6-luna \
  --config 'model_reasoning_effort="low"' \
  --ignore-user-config \
  --ignore-rules \
  --output-schema <history-schema.json> \
  <provider-session-id> \
  <verification-prompt>
```

Before each invocation, the test records a bounded digest inventory of the
authoritative Codex home, copies only controlled regular files without
following symlinks, and confirms that the target provider ID exists in the
snapshot. Each child receives the snapshot as `CODEX_HOME`, temporary `HOME`
and `USERPROFILE`, target project cwd, read-only sandbox, null stdin, and no
inherited API-key environment variables. Argument sizes, stdout, stderr,
JSONL events, and wall time are bounded, and the process is forcibly stopped
on every timeout or malformed response.

The verification prompt prohibits tool use and asks Luna to return every test
history marker visible before the verification prompt, in exact order and with
its original role. The response schema is closed and minimal:

```json
{
  "type": "object",
  "properties": {
    "history": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "role": { "enum": ["user", "assistant"] },
          "marker": { "type": "string" }
        },
        "required": ["role", "marker"],
        "additionalProperties": false
      }
    }
  },
  "required": ["history"],
  "additionalProperties": false
}
```

The fixture gives every historical user and assistant message a unique,
short, machine-checkable marker. The test compares the returned array against
the complete expected sequence, not merely one canary. It therefore detects:

- a missing inherited or local turn;
- reversed or otherwise incorrect order;
- an incorrect user/assistant role;
- duplicate history;
- leakage from a sibling dependency branch; and
- failure to load a preserved branch.

After a successful response, the test confirms that the snapshot's target
rollout changed, proving that the CLI actually resumed that provider session.
It then requires the complete authoritative `myconf1` or `myconf2` digest
inventory to remain unchanged and discards the entire snapshot. The test never
tries to edit, truncate, or clean a verification turn out of an authoritative
rollout. `--ephemeral` remains enabled to minimize unrelated state, but it is
not treated as a non-persistence guarantee.

The existing revision-1–5 histories remain part of the matrix:

| Logical history | Expected markers |
| --- | --- |
| Primary | `MALLARD_BASE`, `MALLARD_REMOTE_BRANCH`, `MALLARD_REVISION_FOUR` |
| First preserved branch | `MALLARD_BASE`, `MALLARD_LOCAL_BRANCH` |
| Second preserved branch | `MALLARD_BASE`, `MALLARD_REMOTE_BRANCH`, `MALLARD_SECOND_LOCAL_BRANCH` |

Together with the seven final graph histories, this yields ten logical
histories in each of two homes: twenty headless CLI resume checks. Provider IDs
come from Mallard materialization receipts and provenance; the test does not
manufacture aliases.

This verifies old history at Codex's resumed model-context boundary using only
the public CLI. It does not claim to test terminal pixels or TUI layout. The
expansion POC already demonstrated the same mechanism for an inherited
paginated base marker.

## 15. Storage Library service seam

Add `src-tauri/src/storage_catalog/service.rs`, analogous to the existing
project-sync service. `StorageCatalogService` owns or receives the explicit
machine-local state root, catalog state, storage transport, plugin process
runner, and OS user home. It exposes the production operations needed to:

- save a Codex resource profile;
- create a Library scope;
- discover and list scope resources;
- link local or storage-only resources;
- inspect current link state;
- prepare and publish a reviewed Push;
- prepare and apply a reviewed Pull; and
- inspect a Library repository and head.

The existing Tauri commands delegate to the same service/core operations.
Publication, CAS, planning, and apply behavior are not copied into the test.

Skill inventory invokes Codex plugin commands to classify plugin ownership.
Deterministic Library tests inject a bounded runner that accepts only:

```text
codex plugin marketplace list --json
codex plugin list --json
```

It returns empty plugin ownership and asserts that the target home is exactly
`myconf1` or `myconf2`. Capture, planning, object publication, reviewed apply,
and filesystem materialization still use the production implementation.

## 16. Shared and updated skills

Create two nested skill packages:

```text
myconf1/skills/mallard-alpha/
├── SKILL.md
├── references/contract.md
└── examples/input.txt

myconf2/skills/mallard-beta/
├── SKILL.md
├── references/contract.md
└── examples/input.txt
```

The ordinary update path keeps directory and frontmatter `name:` stable while
changing the skill body and at least one nested file.

The symmetric workflow is:

1. A publishes alpha v1; B discovers, links, and installs it.
2. B publishes beta v1; A discovers, links, and installs it.
3. A publishes alpha v2; B applies `UpdateSkill`.
4. B publishes beta v2; A applies `UpdateSkill`.
5. Both edit alpha from the same accepted base. A publishes first. B's stale
   reviewed Push fails; a fresh Push is `ChangedBoth`, held, and non-selectable.
6. B Pulls with default `PreserveSkillAsCopyAndOverwrite`, naming the copy
   `mallard-alpha-b-local-copy`.
7. Repeat with B winning the beta race and A preserving
   `mallard-beta-a-local-copy`.
8. Link and publish both preserved copies as independent Library resources,
   then cross-Pull them so both homes contain all four skills.
9. Perform another ordinary update on each primary and reach a fixed point on
   all four repositories.

Assertions cover exact tree bytes and regular-file modes, stable primary
resource and repository IDs, content digest changes, stale-plan rejection,
nondestructive preservation, rewritten copy `name:`, new copy identities,
symlink and special-file rejection, and zero-revision no-op Push/Pull.

## 17. Shared and updated fake pets

Create two format-distinct fake pets:

```text
myconf1/pets/mallard-duck-v1/
├── pet.json
└── duck.webp

myconf2/pets/mallard-owl-v2/
├── pet.json
└── owl.webp
```

`mallard-duck-v1` uses the v1 manifest, omits `spriteVersionNumber`, and has a
1536×1872 atlas. `mallard-owl-v2` sets numeric `spriteVersionNumber: 2` and has
a 1536×2288 atlas. A compact deterministic VP8L fixture produces parser-valid
WebP headers with those dimensions. Updates change both the description and a
payload marker while retaining the format and dimensions.

The workflow is:

1. A publishes duck; B installs it with `InstallPet`.
2. B publishes owl; A installs it.
3. Each origin publishes an update and the opposite replica applies
   `UpdatePet`.
4. Each formerly receiving replica publishes another update, proving that
   either linked machine may become publisher.
5. Diverge duck on both machines, prepare the losing Push before the winner
   publishes, and assert stale-plan rejection followed by fresh `ChangedBoth`
   Push gating.
6. Pull with default `PreservePetAsCopyAndOverwrite`.
7. Repeat for owl with winner and loser reversed.
8. Link and publish both preserved copies, cross-Pull them, and reach a fixed
   point on all four pet repositories.

Assertions cover canonical `pet.json`, exact atlas bytes and checksums, stable
primary IDs, distinct v1/v2 identities, preserved format, renamed copy ID and
display name, exact pre-conflict local content in the copy, format and
dimension validation, and zero model or provider-CLI calls during pet
inventory. The same pet ID is never reused across formats because both formats
materialize to the same directory namespace.

## 18. Safety, privacy, schema, and fixed-point assertions

The combined test verifies that:

- the project bundle converges at revision 10;
- Library activity cannot change the project bundle head;
- each skill and pet has an independent immutable Library repository;
- stale reviewed publication fails its head pin;
- a fresh divergent Push remains held until Pull reconciliation;
- divergent Pull defaults to Keep Both;
- destructive overwrite still requires explicit acknowledgement;
- no-op Push and Pull create no revision, object, or filesystem mutation;
- source rollouts change only at intentional direct-append points;
- headless verification mutates only disposable snapshots and leaves the
  authoritative rollout and session-index data unchanged;
- source and copied authentication remain unchanged;
- portable control documents and object keys contain no authentication content
  or absolute machine, project, Codex-home, or Mallard-home path;
- every Library object is likewise credential- and path-free; provider-native
  project rollout payloads may retain their captured `cwd`, but those paths
  never enter portable control metadata or object keys; and
- all child processes and temporary directories are bounded and cleaned up.

Machine-local state, project bundles, and Library/catalog documents remain
schema 1. Storage continues to use `.mallard/v1/library/<repository-id>`. The
expansion adds no migration, legacy reader, legacy execution path, fallback
writer, portable schema field, or compatibility layer.

## 19. Implementation and verification order

Implementation was completed in working layers:

1. add the current-format paginated fixture builder and offline corruption
   tests;
2. add the bounded headless `codex exec resume` verifier, closed output schema,
   disposable-home snapshots, and authoritative digest non-mutation checks;
3. add `StorageCatalogService`, make Tauri wrappers delegate to it, and add
   offline skill/pet service tests;
4. extend the existing live test through revisions 6–10;
5. add the twenty-alias reopen matrix;
6. add skill and pet phases to the same temporary topology;
7. add final privacy, fixed-point, and cleanup scans; and
8. run one paid live verification only after all offline checks pass.

The verification sequence is:

```sh
cargo fmt --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_e2e --no-run
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
npm run build
npm run test:frontend-integration
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The expansion is complete when the existing five Luna-low requests still pass,
all twenty Luna-low headless verification requests return complete exact
branch-safe histories while mutating only discarded snapshots, the project
replicas converge at revision 10, both homes contain the two primary and two
preserved skills, both homes contain the two primary and two preserved pets,
post-conflict updates converge, no-op operations are revision-free, portable
control metadata and object keys plus every Library object are credential- and
path-free, and every schema constant remains 1.

## 20. Expansion implementation and verification record

The expansion uses the same production topology and one ignored live
orchestrator. The principal implementation artifacts are:

- `src-tauri/tests/live_codex_e2e.rs` for the revision-1-through-10 project
  flow, native paginated DAG, disposable headless resume matrix, exact project
  namespace invariance, and total request-budget gate;
- `src-tauri/src/storage_catalog/service.rs` for the non-Tauri production
  Library seam used by both desktop commands and isolated integration tests;
- `src-tauri/tests/support/catalog_flow.rs` for the reusable two-replica skill
  and pet scenario; and
- `src-tauri/tests/storage_catalog_e2e.rs` for deterministic, credential-free
  Library coverage in the normal backend integration suite.

The following offline checks passed on 2026-08-18:

```text
cargo fmt --all -- --check
npm run build
npm run test:integration
cargo test --manifest-path src-tauri/Cargo.toml \
  --test live_codex_e2e \
  current_codex_paginated_graph_fixture_passes_production_validators \
  -- --exact
```

The integration suite passed 187 Rust library tests, the storage-Library E2E,
the frontend integration suite, release configuration checks, and all CLI
tests. The dedicated paginated fixture also covers the remote-materialization
shape: a native P0→P2 dependency is encoded, decoded into a standalone P2
rollout, appended, and revalidated with its full inherited history but no
machine-local base-file dependency.

The complete production command then passed on 2026-08-18:

```text
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e

test live_codex_mallard_setup_push_pull_divergence ... ok
test result: ok. 1 passed; 0 failed
finished in 201.91s
```

The passing run made exactly 25 requests, all with `gpt-5.6-luna` and low
reasoning effort: five conversation requests and twenty headless history
verification requests. Before that passing run, the first expansion attempt
stopped after the five conversation requests because a test-only final
assertion incorrectly expected a remotely materialized standalone P2 rollout
to retain its native P0 file dependency. No product mutation failed. The
assertion was corrected only after adding the offline encode/decode regression
described above; the complete live gate was then rerun once and passed.

The final implementation keeps every machine-local, project-bundle, and
Library schema at 1. It introduces no migration, legacy reader or writer,
fallback execution path, or compatibility layer, and the deterministic Library
test rejects creation of the obsolete `.mallard/v1/catalog` namespace.

## 21. Proposed expansion: file paths in Session Details

This phase extends the existing live test without adding another conversation
or changing the revision graph. The existing revision-1 Codex request becomes
the file-producing request. It runs from `myconf1` with `project1` as its
working directory, creates deterministic files both inside and outside the
project, and still finishes with the existing `MALLARD_BASE` assistant marker.

The required end-to-end contract is:

```text
source rollout in myconf1
  project1/generated/session-paths/...       project1_external/...
                    |                                |
                    | Push encode                    | unchanged
                    v                                v
portable conversation object
  /__mallard_root__/generated/session-paths/...      project1_external/...
                    |                                |
                    | Pull decode                    | unchanged
                    v                                v
target rollout in myconf2
  project1_copy/generated/session-paths/...          project1_external/...
```

Session Details intentionally presents a normalized view rather than exposing
the active checkout root. On both replicas, in-project occurrences therefore
render as `./generated/session-paths/...`, while the outside absolute path
renders unchanged. The raw target rollout separately proves that Pull changed
the physical project prefix from `project1` to `project1_copy` before Session
Details normalized it.

`affected_paths` keeps its existing safety contract: it contains only clean,
project-relative paths. Outside absolute paths remain visible in the bounded
Arguments or Result section, but they must never enter the dedicated Paths
list. This phase does not weaken that boundary or add an absolute-path DTO.

## 22. Development POC and remaining preflight

A disposable, copied-auth POC was run against `codex-cli 0.147.0` with
`gpt-5.6-luna`, low reasoning, `codex exec`, `workspace-write`, and one exact
`--add-dir` directory. The successful request used a file-edit operation to
create one nested in-project file and one outside-project file. It proved that:

- the child Codex sandbox can write both allowed locations without
  `danger-full-access`;
- both files are created with exact requested bytes as regular files;
- the durable rollout contains a successful `patch_apply_end` record;
- the record's `changes` object contains both absolute path keys.

The first development request was blocked by the enclosing desktop sandbox;
the same bounded command was then rerun outside that enclosing layer while the
child Codex process retained `workspace-write` and the exact `--add-dir`.
Thus the development POC used two Luna-low requests, one blocked and one
successful. Its copied credential and complete temporary tree were removed
after inspection. These development-only calls do not change the committed
live-test budget.

The POC intentionally stopped at the raw provider record. Before modifying the
paid live flow, add a credential-free fixture with the observed current 0.147
`custom_tool_call` plus `patch_apply_end` shape and pass it through the
production Codex history and Session Detail projection. If the current
code-mode completion does not attach its in-project `changes` keys to the
durable tool event, extend only the current-format projection so those paths
reach `affected_paths`. Do not add a legacy parser or a conditional assertion
that silently accepts a missing Paths row.

## 23. Deterministic fixture topology

Create two nested files in `project1`:

```text
project1/generated/session-paths/inside-alpha.txt
project1/generated/session-paths/nested/inside-beta.json
```

Create two files in a sibling directory that is outside the project but whose
name deliberately shares the project-root byte prefix:

```text
project1_external/outside-alpha.txt
project1_external/nested/outside-beta.json
```

The prefix-lookalike is intentional. For a project root ending in
`/project1`, `/project1_external/...` is not a descendant because `_` is not a
path boundary. The wire mapper already handles this correctly. The fixture
also exposes a current Session Detail bug where plain `str::replace` can render
that outside path as `._external/...`.

All four files use small fixed contents. Immediately after revision 1, assert:

- the selected project spelling and its canonical alias resolve to the same
  no-follow directory, while the canonical external root is disjoint;
- every fixture is a no-follow regular file with exact bytes, digest, and Unix
  mode;
- the project-root tree changed only by the two expected relative files;
- the external tree contains only the two expected files; and
- the provider rollout contains all four absolute paths and no wire token.

Only the exact external fixture directory is passed to `--add-dir`; never pass
the system temporary root, the whole live-test root, either provider home,
Mallard state, or storage.

## 24. Bounded Codex execution policy

Replace the test runner's hard-coded access mode with an explicit closed enum:

```rust
enum CodexRunAccess<'a> {
    ReadOnly,
    WorkspaceWrite { additional_dir: &'a Path },
}
```

Only revision 1 receives `WorkspaceWrite`. All revision-2-through-5 chats and
all twenty headless history checks remain read-only. The write-capable command
uses this shape:

```text
codex --ask-for-approval never exec
  --json
  --ignore-user-config
  --ignore-rules
  --skip-git-repo-check
  --sandbox workspace-write
  --add-dir <exact-project1_external-directory>
  --model gpt-5.6-luna
  --config model_reasoning_effort="low"
  --cd <project1>
  <bounded deterministic prompt>
```

The prompt requires a file-edit operation, exact relative paths and contents,
no reads, no network, no other filesystem changes, and the existing final
`MALLARD_BASE` response. The runner retains its closed environment, null
stdin, bounded prompt/output sizes, timeout, exact version gate, and request
counter. The test asserts outcomes from filesystem state and production
Session Details rather than accepting arbitrary shell output as proof.

## 25. Session Detail production seam and boundary fix

The external live test must use the same orchestration path as the desktop UI.
Today, project-bound session discovery is private to the Tauri commands while
`ProjectSyncService` stops at bundle inspection. Extract synchronous,
repository-parameterized cores for:

```text
get_project_session_detail
get_project_session_timeline_page
get_project_session_event_detail
```

Make both the Tauri wrappers and `ProjectSyncService` delegate to those cores.
Reuse the existing request and response DTOs, ownership checks, provider
registry, watcher-backed discovery, snapshot IDs, pagination cache, and error
types. Do not construct `LocalSessionView` values in the integration test and
do not add a test-only parser.

Replace Session Detail's plain project-root string replacement with the same
conservative exact-root-or-descendant text boundary used by the path mapper.
A slash may prove descent, while punctuation and non-ASCII bytes that can
continue a path component must not turn a prefix-lookalike sibling into a
match. Carry both the configured and canonical spellings of a validated bound
root so macOS `/var` and `/private/var` aliases normalize identically. Preserve
the stronger transcript and provider-home redactions. Add focused regressions
proving:

- `/tmp/.../project1/file` becomes `./file`;
- `/tmp/.../project1_external/file` remains byte-identical;
- URI, JSON-string, and object-key occurrences use the same boundary decision;
- provider-home and transcript paths remain redacted; and
- no absolute or escaping value can enter `affected_paths`.

This is a current-path correctness fix. It adds no alternate codec, fallback,
legacy execution path, or serialized field.

## 26. Copy-before-sync sequence

Place the new work around the existing A-to-B revision-1 flow:

1. Finish setup for `project1` and `myconf1`.
2. Run the enhanced revision-1 Luna-low request.
3. Verify the four files, source rollout, and replica-A Session Details before
   Push.
4. Copy only the two in-project files into `project1_copy` at the identical
   relative paths. Use bounded, no-follow regular-file checks; create only the
   expected parent directories; preserve exact bytes and Unix modes.
5. Snapshot the two outside files' identities, bytes, digests, and modes. They
   are not copied and Mallard must never open them for mutation.
6. Push revision 1 to `mystorage1` only after the explicit project-file copy
   has completed and passed its assertions.
7. Finish setup for `project1_copy` and `myconf2`.
8. Pull revision 1 through the existing reviewed Pull path.
9. Verify the materialized target rollout, replica-B Session Details, copied
   files, and unchanged outside files.
10. Continue revisions 2 through 10, divergence, Keep Both, Library flow, and
    the existing headless reopen matrix unchanged.

Because every revision-1-derived primary or preserved conversation contains
the original file event, repeat the read-only Session Detail path assertions
for every final materialized R1 descendant in both provider homes. This adds
coverage without another model call.

## 27. Three-layer path assertions

The test must prove all three representations rather than inferring remapping
from the normalized UI response:

| Surface | In-project path | Outside-project path |
| --- | --- | --- |
| `myconf1` raw rollout | absolute `project1/generated/...` | original absolute `project1_external/...` |
| portable rollout object | `/__mallard_root__/generated/...` | byte-identical original absolute path |
| `myconf2` raw rollout | absolute `project1_copy/generated/...` | byte-identical original absolute path |
| Session Details on A | `generated/...` in Paths and `./generated/...` in visible payloads | unchanged absolute path in Arguments or Result |
| Session Details on B | same portable display as A | same unchanged absolute path in Arguments or Result |

For each replica, load the header, retain its `snapshot_id`, request timeline
pages with limit 1 until `next_cursor` is absent, and expand every relevant
tool event. Assert that:

- both in-project relative paths occur in `affected_paths` for the required
  file-edit event;
- every `affected_paths` entry is clean and relative;
- neither external absolute path enters `affected_paths`;
- the aggregate full bounded Arguments and Results contain all four visible
  paths;
- Session Details contains no boundary-valid active absolute project root and
  contains neither provider home nor transcript path;
- the B rollout contains `project1_copy` descendants and no boundary-valid
  `project1` descendant;
- the B rollout contains no `/__mallard_root__` token or portable path-map
  marker; and
- the outside path spelling and bytes are identical at source, wire, and
  target.

Add a frontend integration case that renders the same DTO contract: two
relative Paths entries, normalized `./...` in Arguments, unchanged external
absolute paths in Arguments or Result, and no external path in the Paths
section.

An outside absolute path is intentionally provider-native conversation
content in this scenario. It is permitted only in the expected immutable,
provider-native conversation object bodies. Portable manifests, tags, commits,
object keys, Library objects, credentials, and project-control documents must
remain free of that path. The storage assertions should identify those expected
conversation bodies rather than broadly exempting every object body.

## 28. Request budget, verification, and completion gate

Enhancing revision 1 instead of creating a sixth conversation keeps the
successful committed gate at exactly 25 requests:

```text
5 conversation requests, including the enhanced revision 1
20 disposable-snapshot headless history verification requests
```

All requests remain `gpt-5.6-luna` with low reasoning. Session Detail reads,
file verification, manual copying, codec inspection, and Push/Pull operations
make no model calls.

Implement and verify in layers:

1. add the sanitized observed-record fixture and production projection tests;
2. fix boundary-aware Session Detail display sanitization;
3. add the non-Tauri Session Detail service seam and make Tauri delegate;
4. add the frontend rendering regression;
5. add the test-local bounded write policy and deterministic revision-1
   prompt;
6. add copy-before-Pull and source/wire/target assertions;
7. compile the ignored live target and run all offline suites; and
8. run the paid live gate once only after every offline check is green.

Verification remains:

```sh
cargo fmt --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_e2e --no-run
npm run build
npm run test:frontend-integration
npm run test:backend-integration
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

This phase is complete only when the real source and pulled sessions display
the expected paths through the production Session Detail API, the target raw
rollout proves physical remapping, the copied in-project files are exact, the
outside files and path spellings are unchanged, every R1-derived final alias
passes, the gate uses exactly 25 Luna-low requests, and every machine-local,
project-bundle, and Library schema remains 1. No migration, legacy reader or
writer, fallback execution path, compatibility layer, or obsolete namespace
is added.

## 29. Implemented path-remapping result

The phase-21 expansion is implemented. Revision 1 now makes one bounded
code-mode call to `await tools.apply_patch(...)` under `workspace-write`, with
only the exact sibling outside directory granted through `--add-dir`. The
prompt preserves the user-selected project spelling instead of silently
canonicalizing it. This is material on macOS, where the selected temporary
root can use `/var/...` while its canonical spelling is `/private/var/...`.
The production Session Detail projection carries both validated spellings and
normalizes either spelling without treating a prefix-lookalike sibling as an
in-project path.

The completed test proves all of the following in one isolated two-replica
topology:

- revision 1 creates exactly two nested files in `project1` and two nested
  files in the outside `project1_external` sibling;
- the runner observes exactly one successful `patch_apply_end`, its complete
  four-key `changes` map, and exactly one final `MALLARD_BASE` response before
  accepting the filesystem result;
- only the two in-project files are copied into `project1_copy`, at identical
  relative paths, before the first Push;
- source, portable wire, and pulled target rollouts respectively contain the
  selected `project1` spelling, `/__mallard_root__/...`, and the remapped
  `project1_copy` spelling;
- both outside paths remain byte-identical and are never admitted to the
  dedicated relative `affected_paths` list;
- the production Session Detail header, limit-one pagination, and expanded
  event APIs display both in-project paths as relative paths on both replicas,
  while retaining the expected outside absolute paths in bounded provider
  arguments/results;
- all three final revision-1-derived logical conversations are checked through
  both provider homes, for six production Session Detail aliases total;
- the existing Push, Pull, divergent Pull, Keep Both, paginated graph,
  cross-machine skill/pet, and twenty disposable headless-resume checks still
  run in the same scenario; and
- the source authentication, copied authentication, source session inventory,
  three fixture trees, complete project repository, and portable privacy
  boundaries remain unchanged where required.

The runner now captures stdout and stderr concurrently under independent
64-KiB limits and parses every non-empty stdout line as object-valued JSON. Its
failure synopsis uses an explicit allowlist of public lifecycle labels and
fixed stderr category flags; provider messages, arbitrary error codes, paths,
credentials, and model-written fixture bytes are never printed. The offline
test covers malformed JSONL, the event-count limit, selected-versus-canonical
prompt spelling, credential-shaped error codes, and content-free diagnostics.

The first path-expansion gate attempt stopped immediately after its first
Luna-low revision-1 request because no fixture files were present. It had not
performed a Push, Pull, Library operation, or any of the remaining requests.
That version discarded the provider event evidence, so the failure could not
be attributed conclusively. Before retrying, the prompt was made explicit
about the current code-mode tool call, selected-path spelling was preserved,
and bounded rollout/output evidence was made mandatory. No blind retry was
performed.

After the offline target passed, the complete gate passed on 2026-08-18:

```text
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e

live Codex phase: conversation revisions 1-5
live Codex phase: paginated graph revisions 6-10
live Codex phase: storage Library skills and pets
live Codex phase: twenty disposable headless history resumes
live Codex phase: complete (25 Luna-low requests)

test live_codex_mallard_setup_push_pull_divergence ... ok
test result: ok. 1 passed; 0 failed
finished in 204.00s
```

The successful gate made exactly 25 requests: five conversation requests and
twenty disposable-snapshot headless history checks. The aborted preflight made
one request, so this implementation gate phase consumed 26 Luna-low requests
in total. The two development POC requests are recorded separately in section
22. Every request used `gpt-5.6-luna` with low reasoning.

After the passing gate, a final review found two failure-log-only hardening
opportunities: an equality assertion could have debug-printed unexpected
fixture bytes, and an arbitrary short provider error code could have entered a
diagnostic synopsis. Both were replaced with content-free comparisons and an
explicit diagnostic allowlist, then the complete non-paid live target was
rerun:

```text
cargo fmt --all -- --check
git diff --check
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_e2e

test result: ok. 3 passed; 0 failed; 1 ignored
```

Those final edits affect only failure formatting and preserve the exact
successful execution and assertions exercised by the paid gate, so another 25
model requests were intentionally not spent.

All machine-local, project-bundle, and Library schema constants remain 1. The
Codex and Claude timeline parser-version increments only invalidate derived
machine-local caches after current-format projection changes; they are not
serialized schema migrations. The implementation adds no legacy reader,
legacy writer, migration, fallback execution path, compatibility layer, or
obsolete storage namespace.

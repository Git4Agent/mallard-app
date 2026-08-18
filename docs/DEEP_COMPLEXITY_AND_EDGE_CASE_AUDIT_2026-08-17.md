# Mallard deep complexity, over-detection, and edge-case audit

**Audit date:** 2026-08-17  
**Baseline:** `dae5bd64b604303aae6bf0c2c20dee1de4ee9904` plus the existing staged,
unstaged, and untracked working-tree changes  
**Scope:** active conversation reconciliation, provider discovery and history
adapters, Activity/session detail, project-file ignore handling, Library
inventory, frontend workflow state, and tests  
**Companion:** [codebase simplification audit](./CODEBASE_SIMPLIFICATION_AUDIT_2026-08-17.md)

## Executive assessment

Mallard has real safety complexity: it moves conversations and user files,
rewrites provider-native state, and must recover from crashes without
clobbering concurrent changes. The canonical-path checks, bounded reads,
immutable publication, review/apply pins, compare-and-swap, no-clobber writes,
and recovery journals are justified.

The overengineering sits around that safety core:

1. **Detection is broader than the supported formats.** The shared session
   scanner recursively searches arbitrary JSON, can combine an ID and working
   directory from different records, and treats substring matches as provider
   metadata. That creates false-positive ownership paths rather than merely
   accepting harmless spelling variants.
2. **The conversation resolver stores the same conclusion several ways.** A
   relation, UI state, presence flags, Push decision, Pull decision, action,
   and reason can disagree. The serialized action has 36 fields, while its
   request and live revalidation context manually mirror most of them.
3. **Frontend concurrency protection is extensive but incomplete.** The root
   controller has 75 state cells and 159 references to one workflow epoch, yet
   project switching, outcome follow-ups, and transfer progress still have
   reachable stale-response cases.
4. **Several read paths detect much more than the requested object.** Session
   detail rediscovers the whole provider home per request; Activity and sync
   comparison run separate discovery stacks for one screen; a single Library
   resource can trigger a full Library inventory and provider CLI probes.
5. **The test suite over-detects source spelling while missing state
   transitions.** One third of frontend tests read production source directly,
   but the concrete request-order races found in this audit remain untested.
6. **The executable formal model is already a false-green signal.** It passes
   its own three codec vectors, but those vectors no longer match the Rust
   codec vectors that also pass.

The right simplification is not to remove fail-closed checks. It is to narrow
what gets detected, represent each decision once, group the existing safety
pins, and test behavior at asynchronous and format boundaries.

## Method and false-positive controls

The existing knowledge graph was used to trace the relevant architectural
subgraph (2,875 nodes, 4,751 edges, eight layers), then every reported path was
validated against the current source. Direct counts include production and
test code only where stated.

A candidate is called accidental complexity here only when at least one of
these is true:

- two writable representations encode the same fact;
- a type exposes combinations that production cannot construct;
- a detector accepts fields outside an authoritative provider shape;
- a single-object operation scans unrelated objects that can invalidate it;
- a test binds to source spelling instead of observable behavior; or
- an optimization or compatibility path has no explicit support boundary.

Theoretical combination counts below measure the size exposed by the types,
not the number of reachable product states. They are used to locate invalid
state space, not to claim every combination occurs.

## Findings register

| ID | Priority | Category | Finding |
| --- | --- | --- | --- |
| DEEP-001 | P1 | Over-detection / ownership | Session metadata can be synthesized from unrelated nested fields or records |
| DEEP-002 | P1 | Edge-case correctness | Claude rewrite accepts mixed working directories and can leave old paths behind |
| DEEP-003 | P1 | Verification drift | The bounded formal model passes obsolete codec vectors |
| DEEP-004 | P1 | State-space explosion | Conversation reconciliation has duplicate truth tables, unreachable axes, and four presentation forms |
| DEEP-005 | P1 | Async state / races | Manual frontend request guards are large but miss project, outcome, and progress races |
| DEEP-006 | P1 | Test over-detection | 66 of 204 frontend tests inspect production source directly |
| DEEP-007 | P2 | Repeated detection | Activity, sync comparison, capture, and session detail repeat provider work |
| DEEP-008 | P2 | Full-surface invalidation | One Library resource scans and pins unrelated Library state |
| DEEP-009 | P2 | Reimplemented semantics | The custom ignore matcher is smaller than Git-ignore semantics it claims to honor |
| DEEP-010 | P2 | External compatibility | Provider-format tolerance is implicit, unversioned, and open-ended |
| DEEP-011 | P2 | Hidden mutation / path guessing | Profile list operations write defaults and profile probes silently change the selected path |

Priority is recommended remediation order. P1 findings include correctness or
verification risks. P2 findings mainly increase latency, stale-plan frequency,
and maintenance cost.

## Detailed findings

### DEEP-001 — Session metadata can be synthesized from unrelated JSON

The provider-neutral scanner at
[`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L1586)
reads up to 128 records and 1 MiB. For every record it recursively searches all
nested objects and arrays for generic keys through `find_string_for_keys` at
[line 1689](../crates/mallard-infrastructure/src/provider_capture.rs#L1689).

It then saves the first fallback ID and first fallback working directory
independently at
[lines 1605-1662](../crates/mallard-infrastructure/src/provider_capture.rs#L1605).
They need not come from the same record or object. For example, a nested tool
payload with an `id` in one record and a later diagnostic object with a `cwd`
can become one invented session identity.

The provider predicates widen the match further:

- Codex treats any type containing `session_meta` as authoritative and accepts
  five ID spellings at
  [`providers/codex/mod.rs`](../crates/mallard-infrastructure/src/providers/codex/mod.rs#L32).
- Claude treats any type containing `system` or `init`, or any record with a
  `sessionId`, as authoritative at
  [`providers/claude/mod.rs`](../crates/mallard-infrastructure/src/providers/claude/mod.rs#L34).
- Session-index filtering recursively accepts a matching ID anywhere inside a
  row at
  [`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L1743).
- Claude sidecars are attached to the first proven session whose ID is merely a
  substring of the relative path at
  [`providers/claude/discovery.rs`](../crates/mallard-infrastructure/src/providers/claude/discovery.rs#L77).

The resulting metadata drives `owned_relative_cwd`, which decides whether a
session belongs to the selected project. A false positive therefore affects
project ownership and stable resource identity, not just a display label.

The committed current-format fixtures are much narrower: Codex uses an exact
top-level `session_meta` envelope, while Claude records carry `sessionId` and
`cwd` together at the top level. No focused test covers split-record fallback,
nested tool IDs, near-match event types, or ambiguous sidecar IDs.

**Recommendation:** replace the recursive generic search with provider-specific
typed metadata records. Require an exact authoritative discriminator, ID, and
working directory in the same record and at named field paths. Keep only
aliases backed by a committed external-format fixture. Parse the Claude
sidecar filename grammar and reject ambiguity instead of choosing the first
substring match.

### DEEP-002 — Claude rewrite accepts mixed working directories

Claude analysis rejects conflicting session IDs, but records only the first
working directory it sees at
[`providers/claude/codec.rs`](../crates/mallard-infrastructure/src/providers/claude/codec.rs#L72).
Later `cwd` values are not compared with it.

The rewrite path compounds that gap:

1. Semantic fingerprints remove every top-level `cwd` at
   [line 254](../crates/mallard-infrastructure/src/providers/claude/codec.rs#L254).
2. Rewriting changes only fields equal to the expected first working directory
   at [line 306](../crates/mallard-infrastructure/src/providers/claude/codec.rs#L306).
3. Post-validation analyzes the result and again checks only the first observed
   working directory at
   [line 361](../crates/mallard-infrastructure/src/providers/claude/codec.rs#L361).

A transcript can therefore contain `/old/project` on later records, be judged
resumable, rewrite successfully, retain those old paths, and still pass the
semantic round-trip check because `cwd` is excluded from the semantic digest.

**Recommendation:** analysis should require every structural `cwd` to equal
the first. Rewrite should fail if any structural path differs from the reviewed
source, and post-validation should prove that every structural path equals the
destination. Add a mixed-working-directory regression fixture before changing
the implementation.

### DEEP-003 — The formal model passes obsolete codec vectors

The Python model describes itself as an executable bounded model of the current
dual-pin schema-1 resolver at
[`FORMAL_VERIFICATION_PORTABLE_TAIL_CONFLICT_RESOLVER.md`](./FORMAL_VERIFICATION_PORTABLE_TAIL_CONFLICT_RESOLVER.md#L1).
It is not part of any package script or CI gate.

Running it succeeds:

```text
codec golden vectors: 3
classifier cases: 8
push: 10 scenarios, 102 states, 96 transitions
pull: 14 scenarios, 180 states, 216 transitions
total: 282 states, 312 transitions
PASS: no bounded counterexample in the simplified model
```

However, its empty-whole-tail vector is
`85ca60a860dfac615f75141b865e6191feea154a72dac2b1007c8d5dcdedf5fd` at
[`verify_portable_tail_conflict_model.py`](../scripts/verify_portable_tail_conflict_model.py#L861),
while the passing Rust golden test pins
`267171df350b0854b2a599f63c7c6929e3ab9461da4bf037b953dce7f6bb7758` at
[`mallard-domain/src/tests.rs`](../crates/mallard-domain/src/tests.rs#L1365).
The Python framing still includes a codec field and tuple count that the current
Rust accumulator at
[`conversation_tail.rs`](../crates/mallard-domain/src/conversation_tail.rs#L147)
does not hash.

The model also has five relation outcomes, while production has nine effective
outcomes after splitting `Sync(Current)` and `Sync(Advanced)` and including
three absence states plus `Unknown`. Its documented simplifications are
reasonable, but a self-consistent stale model should not be presented as
current verification evidence.

**Recommendation:** make Rust emit a machine-readable vector and decision
fixture consumed by the Python explorer, then run both in the aggregate test
gate. If independent implementation is the purpose, compare its results to
that shared fixture. If the model will not gate releases, move it to historical
design evidence and remove the current-status claim.

### DEEP-004 — Conversation reconciliation is overdetermined

The semantic relation is compact, but the exposed state space multiplies:

| Shape | Exposed combinations |
| --- | ---: |
| Four `TailSideEvidence` values plus four proof flags | `3^4 × 2^4 = 1,296` |
| `ConversationCapabilities` | `3 × 2^7 = 384` |
| Direction × relation × capabilities | `2 × 9 × 384 = 6,912` |
| Direction × action × reason × relation | `2 × 14 × 17 × 9 = 4,284` before pins |

Much of this is not real production state:

- The four proof gates on `TailComparisonEvidence` at
  [`conversation_tail_relation.rs`](../crates/mallard-domain/src/conversation_tail_relation.rs#L69)
  are all hardcoded `true` at the sole production classifier construction in
  [`commands.rs`](../src-tauri/src/project_sync/commands.rs#L5551).
- `destination_codec_resumable` is assigned the same value as
  `cloud_safely_readable`, and `provider_quiescent` is always `true` at
  [line 5580](../src-tauri/src/project_sync/commands.rs#L5580).
- `ProviderNotQuiescent` is therefore test-only; `CorruptionBlocked` has no
  construction site; `StalePlan` is only used in a test fixture.

The same decision is then represented several times:

- the canonical direction/relation policy at
  [`conversation_sync.rs`](../crates/mallard-infrastructure/src/conversation_sync.rs#L76);
- a second production relation/action table in the 240-line action validator
  at [`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L3224);
- a 17-value `reason_code` stored independently, but never checked against the
  action or relation by that validator;
- an eight-value `ThreadSyncState` derived separately at
  [`commands.rs`](../src-tauri/src/project_sync/commands.rs#L4158);
- optional relation, two presence flags, Push decision, and Pull decision in
  the 19-field `ThreadSyncEntry` at
  [line 482](../src-tauri/src/project_sync/commands.rs#L482).

The frontend never reads `ThreadSyncEntry.relation`. A concrete semantic split
already exists: `MissingBoth` maps to UI state `Synced`, while its decisions are
`NotPushable` and `NotFound`.

The plan plumbing is similarly flat:

| Type | Fields / variants |
| --- | ---: |
| `ConversationPlanActionRequest` | 35 fields |
| `ConversationPlanAction` | 36 fields |
| `ConversationApplyContext` | 27 fields |
| `ConversationStalePin` | 27 variants |

The pins are legitimate TOCTOU protection. Their manual copying through three
flat shapes is not. The 1,255-line `provider_thread_review` function beginning
at [`commands.rs`](../src-tauri/src/project_sync/commands.rs#L4812) gathers
evidence, observes local state, classifies, builds a presentation row, reserves
provider identities, and constructs Push or Pull plans. Two Booleans select its
mode even though production uses only three of four combinations.

Only six of the 14 action variants are selectable mutations. Nevertheless,
Push and Pull reviews serialize informational and blocked rows through the same
36-field action shape. This makes the executable plan double as a view model.

**Recommendation:**

1. Construct `VerifiedTailEvidence` only after its proofs pass; represent a
   failed proof as a typed blocker instead of four caller-set Booleans.
2. Move the single decision policy into the domain layer. Derive reason text
   from relation plus blocker rather than serializing a freely variable reason.
3. Produce one `ClassifiedThread`, then project it through `present_thread`,
   `plan_push`, and `plan_pull`. Replace two mode Booleans with
   `ReviewPurpose::{Observe, PushPlan, PullPlan}`.
4. Persist only executable actions. Keep informational review rows in a small
   review DTO.
5. Group the existing pins into shared nested values such as context, tail
   evidence, target, and revision pins. Implement one typed diff that retains
   precise stale diagnostics.

### DEEP-005 — Frontend request guards are extensive but incomplete

Current `ProjectSync` metrics:

| Metric | Count |
| --- | ---: |
| Lines | 3,571 |
| `useState` calls | 75 |
| `useEffect` calls | 19 |
| `useRef` calls | 15 |
| References to `hostWorkflowEpoch.current` | 159 |
| References to `shellRequest.current` | 23 |
| References to `restoreRequest.current` | 16 |
| Setter calls in `clearHostScopedWorkflows` | 41 |

The independent request scopes are justified: host lifetime, shell refresh,
project selection, Pull review, logs, and detail pagination are not the same
lifetime. The repetition still fails to cover three reachable cases.

#### Stale project response

`selectProject` changes the active ID and calls `loadProjectData` at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L1303).
`loadProjectData` validates only the host-wide epoch at
[line 1121](../src/components/project-sync/ProjectSync.tsx#L1121). Switching
project A → B does not advance that epoch, and the sidebar remains selectable
during `loading` because it is disabled only by `busy` at
[`ProjectSidebar.tsx`](../src/components/project-sync/ProjectSidebar.tsx#L570).

If B completes first and A completes later, A can overwrite `detail`,
`inventory`, `readiness`, and `loading` while B remains the active project.
The project list then treats the global inventory as belonging to whichever ID
is active at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L761).

#### Stale Pull outcome action

`runOutcomeAction` awaits readiness, confirmation, and native app restart at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L2238)
without capturing a host epoch, plan ID, or outcome identity. A late completion
can repopulate state after `clearHostScopedWorkflows`, or combine readiness for
one plan with the mutable `pullOutcomeInputs.current` from another.

#### Unscoped transfer progress

`BundleFetchProgress` carries `phase`, `storage_id`, and `bundle_id` at
[`types.ts`](../src/types.ts#L788), but the global listener ignores all three at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L641).
A late event can replace or clear the progress shown for a different operation.

Navigation is another manual state machine. At least eight values encode
mutually exclusive routes, and every navigation function clears sibling state
at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L1381).
Rendering resolves accidental combinations by precedence at
[line 3388](../src/components/project-sync/ProjectSync.tsx#L3388).

Pull bookkeeping has unreachable/transient work as well. Once
`applyPullReview` returns a non-null result, `applyReview` closes the review and
returns at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L2831).
The subsequent skipped-action branch therefore runs only when no restore result
exists. Several completed-ID states are set immediately before the close path
resets them.

**Recommendation:** use a small request-generation primitive that returns
typed tokens for independent scopes, and put those tokens on reducer events.
Store project data atomically as `{projectId, detail, inventory, readiness}`.
Use one discriminated `WorkspaceRoute` instead of mutually exclusive flags.
Replace global `busy` and progress with operation descriptors keyed by scope and
token. Keep the independent lifetimes; centralize their stale-event rejection.

### DEEP-006 — Frontend tests over-detect implementation spelling

The 12 frontend files contain 204 tests. A source-block inventory found:

- 66 test blocks (32.4%) call `readFileSync` on production files;
- 10 of 12 files contain at least one such test;
- 133 `readFileSync` calls, 127 of them against production/configuration files;
- 925 `assert.match` and 334 `assert.doesNotMatch` calls overall; and
- 85 tests use `renderToStaticMarkup`, which does not execute effects or state
  transitions.

Some source checks are valuable, especially negative cutover guards. Others pin
variable names, function order, CSS pixels, and exact hook implementation. For
example, the project cutover suite slices named functions out of
[`ProjectSync.tsx`](../tests/frontend/project-sync-cutover.integration.test.tsx#L53),
and Activity tests require exact JSX and local-variable expressions at
[`project-activity.integration.test.tsx`](../tests/frontend/project-activity.integration.test.tsx#L249).

The suite asserts the exact `hostWorkflowEpoch.current` pattern repeatedly, but
does not execute A → B → late-A, host reset during outcome recheck, or late
progress events. Both races above therefore coexist with green source tests.
The companion audit also found that these tests are not typechecked and contain
substantive DTO drift.

**Recommendation:** keep source inspection only for deliberate absence and IPC
surface boundaries. Test extracted route/workflow reducers with deferred
promises. Add explicit out-of-order resolution tests. Use syntax-aware checks
for CSS and command contracts instead of regex over source, and typecheck every
fixture before esbuild bundles the suite.

### DEEP-007 — Provider work is repeated across adjacent read paths

Mallard has three overlapping local-session cache/index mechanisms:

- the Activity cache at
  [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs#L279);
- the timeline cache at
  [`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L151);
- the 996-line watcher service with inventory, observation, and candidate-pin
  maps at
  [`codex_watch_service.rs`](../src-tauri/src/project_sync/codex_watch_service.rs#L162).

They do not share one discovery witness.

On the Activity screen, two effects with the same dependency set independently
request Activity and sync comparison at
[`ProjectActivityPage.tsx`](../src/components/project-sync/ProjectActivityPage.tsx#L1920).
Both discover local sessions and read the selected remote bundle. The frontend
then joins their different row identities using resource, logical-thread, and
provider-session fallbacks.

Session detail repeats even more work. `resolve_local_session` performs full
provider discovery and then scans the chosen transcript metadata again at
[`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L671).
It is called separately for initial detail, every timeline page, and every
event-detail request at lines 1483, 1597, and 1751.

Selected capture also reads and parses a transcript twice. `capture_resource`
reads the bytes and calls `adapter.analyze`, then calls path-based
`adapter.summarize_file`, which opens and parses the same file again at
[`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L329).
The code comment explicitly acknowledges the re-read. A transcript may be up
to 100 MiB.

**Recommendation:** create one profile-scoped session index/discovery witness
consumed by sync, Activity, and detail. Detail requests should revalidate only
the selected no-follow path and its identity, not enumerate every session.
Return summary/presentation from the captured byte snapshot through
`summarize_bytes` or a combined analyze/summarize adapter operation. Activity
and comparison may remain separate projections, but should share the scan and
remote snapshot through one backend orchestration request.

### DEEP-008 — One Library resource pins the entire Library surface

`resolve_resource_workspace` always asks for
`CatalogInventoryFilter::Everything` at
[`resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs#L1396).
Inspection, Push, Pull, and their apply-time revalidation all use this helper.
Fork planning performs another full inventory at line 2114, even though a
kind-scoped filter already exists and is used by the scope list.

The full inventory stamp includes every retained resource, every blocked
candidate, and the complete plugin inventory at
[`inventory.rs`](../src-tauri/src/storage_catalog/inventory.rs#L585). A Push
plan stores that global stamp at
[`resource_planning.rs`](../src-tauri/src/storage_catalog/resource_planning.rs#L270),
and apply rejects the single-resource plan if anything in that broad surface
changed at
[`resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs#L1700).

Consequences:

- an unrelated pet, skill, plugin, or blocked directory can stale a reviewed
  action;
- Codex skill inventory can launch two plugin CLI processes even for a resource
  whose own bytes did not change;
- prepare and apply repeat the broad scan; and
- the detail UI launches one inspection per storage link at
  [`StorageCatalogPage.tsx`](../src/components/storage-catalog/StorageCatalogPage.tsx#L562).

Those link inspections run concurrently, and each sets one global `busy`
Boolean true and then false. With multiple links, the first completion can
re-enable controls while other inspections are still running. The domain
allows up to 128 links per resource.

`force_revalidate` does not bypass an inspection cache—there is none. It only
discards saved plans before performing the same full scan at
[`resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs#L1534).

**Recommendation:** inventory and stamp only the selected kind/resource plus
the ownership, profile, and root evidence that can change its classification.
Use one batch link-inspection command that scans local state once and reads each
remote head independently. Track a request token or pending count rather than a
shared Boolean. Rename or remove `force_revalidate` unless it has a real cache
contract.

### DEEP-009 — The ignore matcher reimplements an incomplete Git dialect

Non-Git project sync promises to honor `.gitignore`, `.ignore`, and
`.mallardignore` in the current README at
[`README.md`](../README.md#L183), while the implemented design says
“Git-style matching” at
[`PLAN_NON_GIT_PROJECT_FILE_SYNC.md`](./PLAN_NON_GIT_PROJECT_FILE_SYNC.md#L393).

The implementation is a custom 140-line parser and recursive dynamic-programming
glob matcher at
[`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L1445).
It implements `*`, `**`, and `?`, but not the full advertised grammar: bracket
classes, escaped leading `#`/`!`, escaped spaces, and Git's trailing-space rules
are not represented. Backslashes are converted to `/`, so escape semantics are
lost entirely.

Committed coverage writes only:

```text
ignored/
*.tmp
```

at
[`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L2722).
There is no executable coverage for the documented precedence, negation,
nested-pattern, escaping, or character-class cases.

This is both overengineering and under-detection: Mallard maintains a custom
matcher yet still cannot make the compatibility claim users infer from
`.gitignore`.

**Recommendation:** replace the custom grammar with an established Git-ignore
matcher and add conformance fixtures for the three-file precedence policy.
Keep Mallard's hard exclusions, symlink/special-file rejection, and secret
warnings outside the matcher so ignore syntax can never weaken safety.

### DEEP-010 — External provider compatibility has no explicit support window

External format tolerance is not obsolete Mallard schema compatibility. The
repository's own removal plan explicitly says to retain known external formats
at
[`PLAN_REMOVE_BACKWARD_COMPAT.md`](./PLAN_REMOVE_BACKWARD_COMPAT.md#L43).
The problem is that current support is inferred from permissive parsing rather
than named and bounded.

Seven Codex portability/history modules total 10,003 lines. The Codex timeline
parser alone is 2,617 lines with 73 functions, nine serde aliases, 30
alias-helper call sites, and more than 60 normalized legacy event
discriminators at
[`providers/codex/history.rs`](../crates/mallard-infrastructure/src/providers/codex/history.rs#L1).

`normalized_type` removes all punctuation and case at
[line 1089](../crates/mallard-infrastructure/src/providers/codex/history.rs#L1089),
so undocumented spellings collapse to supported ones. Timeline format choice
depends on whether any canonical item was observed; the function builds
canonical and legacy state in one scan and selects at the end at
[line 1805](../crates/mallard-infrastructure/src/providers/codex/history.rs#L1805).
Claude has a similar normalization path. The two history adapters contain 24
same-named functions for bounded reads, safe paths, query matching, preview
bounds, and tool summaries.

Codex Desktop ownership detection has the same shape. It unions top-level and
`electron-persisted-atom-state` envelopes, three assignment sources,
camel/snake aliases, and `legacy_cwd_matches` at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L13936). If both
envelopes exist, `state_object_maps` returns both at line 14762 rather than
selecting one authoritative layout.

**Recommendation:** publish a small support matrix of provider/application
format generations. Select a named adapter from exact envelope evidence, keep a
fixture for every supported generation, and reject unknown or conflicting
layouts through the existing degraded/blocked path. Extract only genuinely
neutral duplicated history helpers. Do not delete known provider formats,
Codex pet v1/v2, or the path-mapped wire codec.

### DEEP-011 — Profile reads mutate state and path probes guess intent

Project profile listing calls `ensure_default_provider_profiles` at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L2929). For every
present default provider directory, that function probes the path and then
calls creation, which probes it again at lines 2879-2903 and 2832. Listing can
therefore write profile records and repeat config/binding/overlap checks.

The independent Library profile list repeats default-directory detection and
persistent creation at
[`storage_catalog/resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs#L361).
The same physical Codex or Claude home can consequently be represented by two
separate registries and two generated IDs.

Project probing also changes the requested path. If the chosen directory has a
child named after the provider's default directory, the probe silently selects
that child at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L2729). The frontend
reads `detected_child` but does not ask for confirmation before calling create
with the original path at
[`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L1318); the
backend re-probes and persists the child.

**Recommendation:** make list operations read-only. Run default discovery once
through an explicit bootstrap/setup action. Treat a detected child as a
suggestion that must be accepted, not as an alternate interpretation of the
selected path. Reuse an accepted probe result while revalidating its exact
canonical target and safety boundaries before persistence.

## Edge-case test backlog

These tests have the highest value because they exercise boundaries current
source-regex tests cannot:

| Area | Test |
| --- | --- |
| Session metadata | Nested tool `id` plus unrelated `cwd` must not form a session |
| Session metadata | ID in record 1 and cwd in record 2 must be rejected |
| Session metadata | `not_session_meta` / `initialize_tool` must not be authoritative |
| Claude rewrite | Conflicting later `cwd` must make analysis and rewrite fail |
| Claude sidecars | Two overlapping IDs or an incidental filename substring must block attachment |
| Project switching | A starts, B starts, B resolves, A resolves: B remains rendered |
| Outcome follow-up | Host reset or plan replacement during recheck/restart drops the late result |
| Progress | Late event for bundle A cannot clear bundle B's progress |
| Library detail | Two concurrent link inspections keep controls busy until both settle |
| Library plan | Unrelated pet/plugin/skill change does not stale a resource-scoped plan |
| Ignore files | File precedence, negation, `**`, bracket classes, escaped `!/#`, and trailing spaces |
| Formal model | Python vectors and relation fixture must equal the current Rust fixture |

## Complexity that should remain

| Mechanism | Verdict | Boundary to preserve |
| --- | --- | --- |
| Canonical/no-follow path validation | Keep | Prevent provider, project, and storage-root escape |
| Bounded reads/counts/depth | Keep | Prevent unbounded provider or hostile file input |
| Dual accepted wire/local tails | Keep | Required because path-mapped wire and local bytes differ |
| Review/apply pins | Keep, regroup | Exact revisions, digests, targets, provider state, expiry, and quiescence checks |
| `Unknown` / blocked evidence | Keep, type the cause | Fail closed when ancestry or ownership is unproven |
| Journals, backups, no-clobber, CAS | Keep | Required crash and concurrency behavior |
| Secret-shaped/executable warnings | Keep | Best-effort warning plus explicit acknowledgement |
| Codex watcher | Keep and share | Measured pre-cache refresh cost was material; watcher is bounded and not apply authority |
| Provider registry and separate adapters | Keep | Codex and Claude have genuinely different current formats |
| Project sync vs Library protocols | Keep | Separate bounded contexts and remote repository identities |

The watcher should not be removed merely because it is large. Its design is
correctness-neutral, process-local, bounded, and backed by measured multi-second
capture costs. The simplification is to share its discovery witness and put its
manual test backend behind `#[cfg(test)]`.

## Recommended sequence

### 1. Add correctness fences before refactoring

- Add the split-record/nested-metadata and conflicting-Claude-cwd fixtures.
- Add deferred-promise tests for project switching, outcome actions, and late
  progress.
- Make the Python/Rust vector mismatch fail one aggregate command.

### 2. Narrow detection and invalidation

- Replace recursive session metadata search with exact provider records.
- Scope Library inventory and source stamps to the selected resource/kind.
- Batch link inspection and share Activity/detail discovery.
- Derive summary from captured bytes instead of reopening the transcript.

### 3. Collapse duplicate state representations

- Introduce `ClassifiedThread` and one domain decision policy.
- Separate review rows from executable mutations.
- Group plan pins without removing any comparison.
- Replace free proof/capability Booleans with constructed evidence/blockers.

### 4. Make frontend workflows explicit

- Add independent typed request tokens and reducer-based stale-event rejection.
- Replace navigation flags with one route union.
- Key project data and progress by operation identity.
- Remove transient Pull states and compute the Push review view model once.

### 5. Bound external compatibility and harden tests

- Name every supported Codex/Claude/Codex Desktop layout and retain one fixture
  per generation.
- Remove permissive aliases not justified by those fixtures.
- Keep only cutover/absence source assertions; move workflow and presentation
  checks to behavior, syntax-aware, and typechecked tests.

## Verification performed

- Read and traced the existing knowledge graph metadata and relevant one-hop
  subgraph.
- Ran `python3 scripts/verify_portable_tail_conflict_model.py`: passed 282
  modeled states and 312 transitions, while exposing the stale vectors above.
- Ran `cargo test -p mallard-domain tail_codec_golden_vectors_are_pinned -- --nocapture`:
  passed the current Rust vectors.
- Recounted hooks, request guards, plan fields, test source reads, parser aliases,
  and active call sites against the current working tree.

No production code was changed for this audit.

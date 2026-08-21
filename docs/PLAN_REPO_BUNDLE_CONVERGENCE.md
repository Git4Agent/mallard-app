# Plan: Repo Bundle Convergence (registration claims + sibling discovery)

Status: **Implemented** — 2026-08-21 (proposed and implemented same day; see
Implementation notes at the end for deltas)
Depends on: storage-scoped catalog (shipped), cloud thread id decoupling
(shipped 2026-08-19)

Two registrations of the same repository against the same storage must end up
in the same bundle regardless of who pushes first — and when they don't, the
app must say so and offer the repair. Today neither holds when the first
project links a storage but has not pushed yet.

## 1. The regression as observed (2026-08-21, wiped state)

Reproduced order, from `~/.mallard/sync_config.json`,
`machine_projects.json`, the activity log, and the storage tree:

1. **HorrorGame (Claude)** (`~/Desktop/project/HorrorGame`, default homes)
   registered and **linked Local storage 1, no push** → bundle
   `b751caac…` exists only in local config. The storage is not touched:
   linking writes nothing, so the bundle has no head, no manifest — no
   storage-side existence at all.
2. **HorrorGame (myconf2_claude)** (`~/Desktop/project/demo/HorrorGame`,
   simulated second machine) ran setup against the same storage. The setup
   page's repository discovery lists **published bundles only**
   (`list_remote_bundle_snapshots` → `engine.inspect` per head); the storage
   had none, so the repository section stayed silent and the draft kept its
   default `DraftRepositoryChoice::New` → second bundle `19e9afda…`.
3. The second project **pushed revision 1** (1 Claude + 1 Codex
   conversation, `repository_fingerprint` identical: `045a58ec…`).
4. The first project's **Refresh** compares only `link.bundle_id`
   (`b751caac`), which still does not exist in the storage → storage side
   empty → all 35 sessions LocalOnly, "Push 35", and no hint that a sibling
   bundle carries the same repository.

Nothing mis-executed; the state machine has a hole. Three causes compose:

- **C1 — an unpushed registration is invisible.** The bundle exists nowhere
  outside the creator's local config until the first push, so no later setup
  on any machine can converge on it.
- **C2 — the setup repository choice is silent.** The section renders only
  when a fingerprint match was found and auto-selected
  (`ProjectSetupWorkspace.tsx:401` auto-select, `:569 showRepositoryChoice`);
  with no matches the draft silently stays `New`. The user is never shown
  "this will create a new repo in the storage" as a decision.
- **C3 — no post-setup convergence signal.** After the split exists, only a
  failing Pull (`beginProjectRestore` → plan error `contains("does not
  exist")` → connect dialog, `ProjectSync.tsx:2387`) ever surfaces the
  sibling. Refresh — the thing users actually do — never looks
  (`find_remote_bundle_matches` has no UI caller; `api.ts:191` is orphaned).

## 2. Decisions

### D1 — Linking a storage publishes a registration claim

**Finishing setup with a storage selected — and linking a storage to an
existing project — publishes the bundle immediately as an empty
"registration" manifest: revision 1, zero scopes, carrying `bundle_id`,
`display_name`, `kind`, and `repository_fingerprint`.**

- The claim is an ordinary bundle head. Every existing reader works
  untouched: `list_remote_bundles` lists it, `inspect` returns it,
  `bundle_snapshot_summary_from_snapshot` reports fingerprint + 0 resources,
  the Library shows an empty repo, setup's existing match-and-default logic
  finds it. No new object kinds, no reader changes.
- `BundleManifest::validate` already accepts zero scopes (domain untouched);
  only the engine's publish request path requires writable scopes
  (`bundle_engine.rs:544`). A dedicated engine entry point
  `publish_registration(...)` builds and commits the empty manifest through
  the same object/commit/head machinery (same origin stamping), **refusing
  when any head already exists** — it can only create revision 1, never
  advance history. Callers treat "head exists" as success (idempotent).
- Integration points: the setup finalize flow (storage selected, repository
  `New`) and `save_project_link_with_repository` (commands.rs:13869). The
  connect flow needs none (it adopts an existing head by definition).
- Failure is loud: if the claim cannot be written, Finish/link fails with
  the storage error. A storage you cannot write is a storage you cannot
  push to; discovering that at link time is correct. No deferred/retry
  claim state.
- First push atop a claim advances revision 1 → 2 with absent previous
  scopes (the engine's existing "creating an absent scope" path); no
  RequiresPull is triggered by the registration itself (stage test).

Rejected alternatives:

- *Claim sidecar file* (`_claim.json` beside `_head.json`): every reader —
  listing, inspect, summaries, Library, setup matching — must learn a second
  existence mode. The empty manifest gives the same information through the
  paths that already exist.
- *Defer bundle minting to first push, join at push time*: turns push into a
  discovery/merge point and re-orders its base-revision semantics; far
  larger blast radius than making registration visible.
- *Local-registry lookup at setup* (see the sibling project in
  `machine_projects.json`): fixes only the same-machine simulation, not real
  two-machine order; and D1 covers the simulation case too.

### D2 — The setup repository section is always an explicit choice

**When a draft selects a storage, the repository section always renders and
always states what will happen.** Two forms:

- Matches exist (fingerprint-equal bundles in the storage, claims included):
  a choice list, fingerprint match pre-selected — "Use the existing repo
  ‘HorrorGame’ (same repository · 2 items · updated Aug 21)" vs "Create a
  new repo in this storage". Non-matching bundles with the same display name
  are listed as selectable candidates, never pre-selected, labeled
  "different repository" (connect-with-mismatch stays acknowledgeable, as
  today's dialog does).
- No matches: a single static line — "This creates a new repo ‘HorrorGame’
  in Local storage 1." No silent default; the user has seen the claim they
  are about to make.
- Projects without a git fingerprint never auto-default to a match; name
  matches appear as candidates only.
- The silent auto-select (`repoAutoSelected` ref) is deleted in the same
  change — the pre-selection moves into the always-visible section (clean
  cutover, no second path).

### D3 — Refresh-adjacent sibling detection with a Connect affordance

**When a linked bundle has no published head (or only a registration head),
the app checks the storage for same-fingerprint sibling bundles and surfaces
a banner-zone notice on the links workspace: "This storage already has this
repository in another repo (‘HorrorGame’, 2 items, updated …) — Connect."**
The action opens the existing `PendingBundleConnection` dialog (new reason
`"sibling"`) and runs the existing `connect_project_to_remote_bundle`;
`api.ts findRemoteBundleMatches` finally gets its caller.

- **Observation discipline holds**: detection never writes. Claims are
  published only by link/setup/push flows (D1); refresh only reads. (This is
  also why D3 alone repairs claimless legacy states — including the live
  split from §1.)
- Trigger precondition keeps it cheap: the sibling scan runs only when the
  own-bundle head is absent or registration-empty — i.e. only while
  unconverged, at most one scan per refresh, cached per
  `(storage_id, project)` until the storage's bundle listing changes. The
  scan reuses `find_remote_bundle_matches_with_repository` (10,000-bundle
  inspection cap already enforced). For converged links (own head has
  content) nothing runs — steady-state refresh cost is unchanged.
- States are classified before any UI is built (compact-sync-flow rule):
  `sibling_with_content` (notice + Connect), `sibling_registration_twin`
  (concurrent-claim race: notice names the deterministic survivor, D4),
  `none` (no banner). The Pull-failure dialog (`reason: "missing"`) stays as
  the last-resort path for states detection cannot see (deleted bundles,
  replaced storages).

### D4 — Connect may abandon an empty registration; twins tie-break

- `connect_project_to_remote_bundle_guarded` currently refuses whenever the
  own bundle has any head ("refusing to abandon published history",
  commands.rs:13997). Relaxation: **abandoning is also allowed when the own
  head is a pure registration** — zero resources, files, directories,
  tombstones, and conversations across all scopes. Any content keeps the
  refusal (that is a real divergence, resolved by the existing pull/Keep
  Both machinery, not by connect).
- Concurrent registration race (both machines claim before seeing each
  other): deterministic survivor = **lexicographically smaller bundle_id**.
  Both sides' notices point at the survivor; the loser connects and its
  registration husk remains in storage.
- Husks (abandoned empty registrations) are a deliberate non-goal: the
  engine has no bundle deletion, and an empty repo row in the Library is
  honest. A catalog-side cleanup affordance is future work (§6).
- While relaxing the guard, resolve the adjacent dead code: the
  `if false`-disabled "reviewed sync base" guard (commands.rs:13968) either
  becomes a real condition or is deleted — no disabled branches left behind.

## 3. Lifecycle after this plan

| Order of events | Today | With D1–D4 |
|---|---|---|
| A links (no push) → B sets up | B sees nothing, silently mints a second bundle | A's claim is in the storage; B's setup **asks**, defaulting to A's repo |
| A sets up + pushes → B sets up | Works (existing match auto-select) | Same, but the choice is explicit |
| Both set up before either links/pushes (race) | Permanent silent split | Both claim; both refreshes flag the twin; loser connects to the survivor (lossless — both empty) |
| Split already exists (e.g. §1, pre-plan states) | Only a failing Pull reveals it | Refresh shows the sibling notice → Connect → pull |
| Storage unwritable at link time | Link "succeeds", push fails later | Link fails loudly at the claim write |

## 4. Stages

Every stage lands green on the standing bar: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, domain + infra +
src-tauri lib suites; frontend stages additionally keep the frontend
integration suite green. UI stages build the static HTML specimen first and
get sign-off on it before touching components (standing rule).

**S1 — sibling detection backend.**
A `sibling_bundle_status` computation on the links/refresh read path gated by
the own-head-absent/registration-only precondition, reusing
`find_remote_bundle_matches_with_repository`; result cached per storage
listing generation. DTO carries the classified state (D3) plus the match
summaries the dialog already consumes. Tests: absent-head + content sibling;
absent-head + no sibling; fingerprint mismatch excluded; converged link runs
no scan (metrics-counted, the cursor-parity discipline); cache invalidates
when the storage listing changes.

**S2 — links-workspace sibling notice (UI).**
Specimen first (banner zone, both themes, catalog page context), then port:
notice rendering from the S1 state, Connect opens the existing
`PendingBundleConnection` dialog with `reason: "sibling"`; connect path
unchanged. Frontend integration tests for both sibling states and for
none-state absence. *S1+S2 alone repair the live split from §1.*

**S3 — registration claims.**
Engine `publish_registration` (refuses when a head exists; same commit/origin
machinery; domain untouched). Wire into setup finalize (storage selected)
and `save_project_link_with_repository`; link/Finish fail loudly on claim
write failure. Tests: claim manifest round-trips (`inspect`, summary shows
fingerprint + 0 resources); idempotence (existing head → no-op success);
push after claim advances 1→2 with no RequiresPull; `plan_bundle_restore` on
a claim yields an empty plan (no "does not exist"); Library lists the claim
as an empty repo.

**S4 — connect-guard relaxation + twin tie-break.**
Guard allows abandoning a pure-registration head (D4), still refuses any
content; `sibling_registration_twin` state resolves to the smaller
bundle_id on both sides; the `if false` guard is resolved. Tests: twin race
converges from both directions; content-bearing own head still refuses;
post-connect the adopted link pulls normally.

**S5 — setup repository section (UI).**
Specimen first (both forms of D2, name-collision and mismatch labels), then
port `ProjectSetupWorkspace.tsx`: always-rendered section, explicit
default, silent auto-select deleted. Frontend tests: claim visible at
second setup (the §1 order, fixed); no-match shows the create statement;
no-fingerprint never auto-defaults.

**S6 — lifecycle regression tests.**
Lib-level, both orders end-to-end in one home (two profiles, one storage —
codifying the config-dir two-machine simulation as a supported topology;
replica identities already distinct per binding): (a) the §1 order —
A links (claim) → B setup adopts → B pushes → A refresh shows StorageAhead →
A pulls; (b) legacy claimless split → S1 notice state → connect → pull.
Extend the aimock live e2e with variant (a) if its runtime budget allows;
otherwise the lib tests carry it.

## 5. Compatibility

- **No schema changes.** The claim is an instance of the existing
  `BUNDLE_SCHEMA` manifest (empty scopes already validate); the writable-
  scopes requirement is an engine publish-request rule, relaxed only for the
  new dedicated entry point. Wire, checkpoint, and domain schemas untouched.
- **No migration, no wipe required.** Legacy claimless storages and the
  current split state are handled by detection (D3) — that path needs no
  claims to exist. Digests, folds, and cursors are untouched by this plan.
- Two local projects sharing one bundle on one host becomes a test-pinned
  supported topology (S6); per-link scope_bases/checkpoints and per-binding
  replica ids already keep their state disjoint.

## 6. Out of scope

- Bundle deletion / husk GC and a Library "remove empty repo" affordance
  (engine has no delete today; husks are visible and inert).
- Auto-connect without a user click (all convergence flows end in an
  explicit user action; only the setup default is pre-selected).
- Repository-fingerprint drift (history rewrite changing the fingerprint) —
  the existing mismatch-acknowledge flow remains the escape hatch.
- The `open -n` / Desktop-import hazards tracked in
  `PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md` §10.

## 7. Implementation notes (landed 2026-08-21, deltas from the plan text)

All stages landed; the standing bar is green — fmt, clippy `-D warnings`,
workspace suites 213 (src-tauri) / 303 (infrastructure) / 50 (domain) / 26 /
8, storage-catalog e2e, frontend integration 224. Deltas and decisions made
during implementation:

- **Every first push moved from revision 1 to revision 2** — the link-time
  claim consumes revision 1. This rippled through ~20 test expectations
  (absolute revisions, `Published revision N` message text,
  `Ready { revision }`, `RemoteHistoryMissing/Replaced` bases,
  `RemoteContinuity`), all bumped rather than special-cased: the claim is
  real history, not an annotation. Tests that hand-publish stored states
  with `PublishExpectation::Absent` now derive the expectation from the
  observed head (`observed_publish_expectation` test helper).
- **Two "push published nothing" assertions became "only the registration
  stands"** (`is_registration_only` on the inspected manifest) — rejected
  pushes are still proven contentless, just no longer head-less.
- **Unlink + relink republishes a fresh claim**, so the
  missing-remote-history recovery now shows `Ready { revision: Some(1) }`
  before the republish instead of `None` — the relink itself re-registers.
- **The sibling DTO** serializes as `state: "none" | "sibling_found" |
  "registration_twin"` (plan §D3's classifications, with `sibling_found`
  covering both the content case and the claim-only case — the banner copy
  differentiates via `registration_only` on the match). The twin's survivor
  is boxed (clippy `large_enum_variant`).
- **Detection runs uncached** inside `loadProjectData`'s workspace load:
  the own-head precondition already limits it to unconverged links, and the
  fetch happens only on user-facing workspace loads — a listing-generation
  cache added state for no measured cost. Failures are silent (the banner
  is advisory, never evidence).
- **Pending storages list read-only** (`list_pending_storage_bundle_
  snapshots`): a local folder that is not already a Mallard storage returns
  an empty listing without creating the storage namespace inside it — the
  draft must not write into a folder the user may still discard.
- **Claims require an active binding** (origin tokens derive from the
  replica); a link saved without one publishes nothing and stays
  discoverable through detection. Setup's Finish always has a binding.
- **The setup empty-storage statement renders as the section's own option
  card** — the selected "New repository" card carrying the creation
  statement as its subtitle — after design review of the first floating-
  text draft. Specimen: `convergence-mockup.html` (dev server), covering
  both Repository states and all three banner states with shipped CSS and
  the shipped `SiblingRepoNotice` component.
- The two new commands joined the frozen registration list
  (`sync_tests/registration_freeze.rs`).

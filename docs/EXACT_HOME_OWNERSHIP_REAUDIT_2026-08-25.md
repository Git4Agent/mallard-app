# Exact-home ownership — re-audit

2026-08-25. Audit of the **shipped** exact-home ownership feature and its
Claude leaf-cwd prerequisite, against `docs/PLAN_EXACT_HOME_OWNERSHIP.md`.
No code was changed by this audit.

## 0. Subject, method, status

**Subject.** The law from `PLAN_EXACT_HOME_OWNERSHIP.md` §2: *each absolute
directory is its own project; a session belongs to exactly one directory —
its authoritative home — by exact canonical path equality; a subdirectory is
a different project.*

**Where it lives.** Implemented in `e9a0d0d` (2026-08-22), not uncommitted as
the plan's §9 and the project memory both stated. Two later commits touched
the same files (`5b706b2`, `e2e7322`); audited at `d78e3c5`.

**Method.** Eleven independent review lenses over the current code, each
finding put through two adversarial verifiers (a refuter and a reproduction
engineer), then merged and ranked; 41 raw findings, 35 survived verification,
6 refuted. Every finding below that is marked **[measured]** was then
re-confirmed by direct measurement against the live `~/.codex` and
`~/.claude` corpora on this machine, or by reading the cited code directly.

**Bar.** Re-run at `e2e7322`: `mallard-domain` 52, `mallard-infrastructure`
337, `src-tauri` 241 plus integration binaries (`storage_catalog_e2e`
included), frontend integration 241/241, `tsc --noEmit`, `cargo clippy
--workspace --all-targets -D warnings`, `cargo fmt --check` — all green, zero
failures. §9's counts (51/316/235/236) are lower only because later commits
added tests.

**Verdict.** The mechanical cutover is clean, complete, and verified. Two
defects are live production bugs — one silently hides 12 of this machine's
77 mallard-app transcripts, the other turns a bind into an unclearable
Push/Pull dead-end. Both stem from the same root cause the plan itself
identified and then reintroduced in a new place: **structure inferred from
per-turn state instead of a stable session property.**

## 1. What is correct

Verified by direct inspection, so it is not re-litigated later:

- **Every promised deletion happened.** `owned_relative_cwd`,
  `inspect_codex_structural_cwds`, `path_belongs_to_project`, the multi-root
  blocker, the legacy cwd assignment fallback, `relative_cwd` end-to-end
  (Rust and `types.ts`), `relative_cwd_key`, `CaptureRequest.excluded_project_roots`,
  and `SessionDiscoveryContext.excluded_roots` are all gone. `grep` for
  `relative_cwd` across `crates/`, `src-tauri/`, `src/`, `tests/` returns zero.
- **The nested-content exclusion survived correctly.** Nested active binding
  roots now flow into `excluded_content_roots`
  (`src-tauri/src/project_sync/commands.rs:13249`), so a parent project's
  content push still cannot swallow a child project's tree. This was the
  highest-stakes deletion risk in §3 and it is intact.
- **The repair's structural-identity invariant is form-correct.** The
  `expected_cwds` comparison maps every pre-repair structural cwd through
  `encode_old`/`decode_new`; foreign cwds are unchanged on both sides, so a
  rollout carrying cross-directory fork ancestry passes rather than
  false-blocking. This was the design's stated risk and it holds.
- **The repair TOCTOU recheck is exact** — `codex_authoritative_home` is
  re-read and compared to `issue.recorded_cwd` by string equality
  (`commands.rs:15991`), as §9 claims.
- **The moved-project case is handled** in classification: `fs::canonicalize`
  falls back to the literal path when the old root no longer exists
  (`commands.rs:15827`), which is exactly the state a moved project is in.
- **Aux-only nominations really are silent** for the oversized and
  missing-rollout warnings, and the classification table's six cells are
  implemented arm-for-arm (`commands.rs:15740-15855`).
- **The inventory cache is not stale after a rebind.**
  `session_watch_service.rs:822` compares `cached.request == *request` and
  `CaptureRequest` carries `project_root`, so narrowing ownership cannot
  serve a previous root's session set.

## 2. Findings

Ranked by real user impact.

---

### F1 — Claude ownership is decided by the last record's cwd, which is per-turn shell state **[measured]**

**Defect.** `ClaudeSessionIdentityTracker::observe` overwrites `leaf_cwd` on
*every* chain record
(`crates/mallard-infrastructure/src/providers/claude/codec.rs:204-208`).
`session_home_is_project_root`
(`crates/mallard-infrastructure/src/provider_capture.rs:1752-1757`) then
demands exact equality against the project root. So whichever directory the
session's **last tool call** happened to run in owns the entire transcript.

**Measured on this machine.** In
`~/.claude/projects/-Users-hequ-Desktop-project-mallard-app` (77 files):

| Observation | Count |
|---|---|
| Files whose chain cwd changes at least once mid-session | **37 / 77** (one flips 41 times) |
| Files whose leaf cwd is now `.../mallard-app/src-tauri` ⇒ **not owned** | **5** (`0fd0e569`, `463d9194`, `58e064f3`, `b805b821`, `cb592c1d`) |
| Files dropped by the filename-stem gate | 7 |
| **Transcripts invisible to the bound project right now** | **12 / 77** |

Every one of the 37 *starts* at the project root; only the tail wanders.

**Consequence.** Ownership is not stable over time. The transcript is
append-only and live: a scan at T0 (leaf cwd = root) discovers the session;
one more `cd src-tauri` tool call at T1 drops it. Its cloud copy then flips
to "only in storage" between two consecutive comparison runs, and further
appends silently stop being pushable. Nothing tells the user
(`claude/discovery.rs:99` drops a non-owned session with a bare `continue`,
unlike its sibling arms at `:88` and `:119`).

**Root cause.** §2's Claude bullet assumes cwd is a property of the session
("resumed sessions are identified by their leaf record"). It is not — it is
a property of each turn. The design's own thesis (identity must come from a
stable authoritative record, not a bag of per-turn evidence) applies here
and was not applied.

**Fix.** Keep leaf-*session* selection exactly as it is — that is what
correctly makes a resume move the home. Take the home from the **first** cwd
of the leaf sessionId's own chain records, resetting the recorded home only
when `leaf_session_id` changes, instead of overwriting at `codec.rs:206-207`.
Verified: this resolves all 5 excluded files back to
`/Users/hequ/Desktop/project/mallard-app`, which is also what Claude's own
bucket name says. Amend §2 to "the launch cwd of the leaf session" and strike
the premise that cwd changes only on resume.

---

### F2 — A live foreign home is classified as a stale path, producing an unclearable Push/Pull dead-end **[measured]**

**Defect.** The classifier's third arm (`commands.rs:15883-15898`) treats any
assigned thread whose home ≠ binding root as "the project moved." It never
asks whether that home still exists, or whether it is itself a registered
directory-project. `finalize_conversation_path_audit` (`commands.rs:16421`)
then sets `can_repair = true` without running the collision pre-scan that
the repair itself applies at `commands.rs:16060-16080`.

**Measured on this machine.** Thread
`019f7c76-7ceb-71b2-a4dd-86c3672764d7` is Codex-Desktop-assigned
(`projectKind: local`) to project `16be32da…` = `mallardInternal`
(root `/Users/hequ/Desktop/project/mallardInternal`). Its authoritative
`session_meta.cwd` is `/Users/hequ/Desktop/project/mallard-ai-web` — a
different directory that **currently exists** and is itself a Codex Desktop
project. The 10.5 MB rollout contains **574 occurrences** of the
`mallardInternal` root string and 92 of its own home.

**Consequence, on binding `mallardInternal`:**

1. Thread nominated (assignment channel, exact root match) →
2. home ≠ root and not under root → **repair issue** →
3. `ready = false` → `require_codex_conversation_paths_ready`
   (`commands.rs:16388`) refuses **both Push and Pull** →
4. `can_repair = true` → the notice renders a Repair button →
5. Repair aborts on the unpinned collision pre-scan (574 mentions of the new
   root) → **the button always errors.**

There is no in-app exit. The only escapes are reassigning the thread inside
Codex Desktop or unbinding the project.

**The unguarded variant is worse than the dead-end.** An assigned
foreign-home thread whose transcript never mentions the binding root passes
the pre-scan. `mapper_old` is then built from the *live* sibling project's
path, and every occurrence — structural cwds and prose alike — is rewritten
into the binding root. The pre-scan is a **collision** guard, not a
**staleness** guard; it protects the observed case by coincidence.

**Fix.** Two changes:
(a) In the classifier, before emitting an issue, test whether `resolved_home`
still exists and whether it is a registered local project root. A live
directory-project is not a moved project — emit a manual-review blocker
naming both projects.
(b) In `finalize_conversation_path_audit`, run the same new-root collision
predicate the repair uses and exclude failing issues from `can_repair`, so
the notice states the real reason instead of offering a button that cannot
work. Give the user a reachable exit (an explicit "exclude this thread from
ownership verification").

---

### F3 — Claude memory read bucket and write bucket are computed by different rules **[measured]**

**Defect.** Capture picks the bucket by self-consistency with the session's
*own recorded* cwd (`claude/discovery.rs:104-114`). Restore picks it by
munging `binding.project_root` — the user-selected spelling —
(`bundle_engine.rs:3258` → `claude/materialization.rs:95`). The asymmetry is
visible inside one file: `project_content_target`, twelve lines later at
`bundle_engine.rs:3270`, correctly uses `binding.canonical_project_root`.

**Trigger.** Any binding whose picked spelling differs from canonical — a
project reached through a symlinked ancestor, which is a first-class modelled
state. Memory is captured from `-private-tmp-proj/memory/` and pulled into
`-tmp-proj/memory/`, a directory Claude never opens.

**Consequence — silent lost update, not a repeating prompt.** The plan marks
the row `UpToDate` by inspecting the digest of the path *it computed*
(`bundle_engine.rs:1404-1412`), so after one apply the row vanishes. The pull
reports success, Claude never sees the memory, and the next Push republishes
the stale local copy over the peer's — on a `merge_apply: true` resource.

**Blast radius exceeds memory.** The same raw `binding.project_root` feeds
transcript buckets at `commands.rs:6515` and `:11739`, so a pulled Claude
conversation can land where `claude --resume` will not list it.

**Fix.** Pass `binding.canonical_project_root` at `bundle_engine.rs:3258`
(and `commands.rs:6515`/`:11739`), matching `project_content_target`. Better:
carry the proven bucket from discovery into the plan so the halves cannot
drift. §9's second bullet is true of capture only; say so.

---

### F4 — The memory bucket is picked lexicographically and memory-blind **[measured]**

**Defect.** `claude/discovery.rs:176-186`: `root_buckets.into_iter().next()`
takes the `BTreeSet`'s first path; the "ignored Claude memory in X; using Y"
warnings fire at `:179-184` **before** any memory check; and
`memory.exists()` at `:185` tests only the winner. `context.project_root` is
in scope (used at `:99`) and never consulted.

**Consequences.** (a) When the winning bucket has no `memory/`, **zero**
memory resources are emitted even though a qualifying bucket has memory —
and the warning affirmatively claims a duplicate was ignored "using" the
winner. (b) When both have memory, byte order decides, not the bucket the
write side targets.

**Trigger.** The app's own pull path reliably produces a second qualifying
bucket: pull writes the transcript into `encode(binding.project_root)` *and*
rewrites its cwd to `binding.project_root` (`commands.rs:11659`, `:11739`),
making that bucket self-consistent. A case-only rename on APFS produces the
same shape and deterministically prefers the stale bucket (`'P' < 'p'`).

**Fix.** Filter `root_buckets` to those whose `memory/` exists and is
non-empty *before* selecting; warn only about buckets actually dropped;
prefer the bucket `state_target` writes to. **§9's "one memory resource
(first in order) plus a warning" is false** whenever the first bucket lacks
`memory/`.

---

### F5 — Checkpoint rebase is committed before verification; rollback restores only files

*Reported and twice-verified by the audit; traced but not independently
re-measured here.*

**Defect.** `commands.rs:16273` durably writes the rebased checkpoints. The
final audit at `:16352` and the `!final_audit.ready` arm at `:16371` both
call `rollback(pending.len())` (`:16137-16146`), which only rewrites
transcript bytes. The config keeps `accepted_local_tail`, `observed_tail`,
`root_mapping`, `raw_range_pin.range_sha256`, `prefix_witness`, and a bumped
`generation` derived from bytes that no longer exist — under a message that
says "changes were rolled back."

**Trigger.** Codex Desktop rewrites `.codex-global-state.json` or rotates a
session file between the first write at `:16147` and the final audit's read.

**Consequence.** Fails **closed**: `refresh_codex_tail` returns
`Invalid{Replaced|Truncated}`, `observe_codex_local` returns `seeded: None`,
and the thread becomes uncapturable — not Divergent, so no spurious
republish. A later successful Repair self-heals it; the damage is durable
only if the user resolves the mismatch another way, after which Repair
disappears and the thread is stranded with no re-baseline affordance.

**Fix.** Move `mutate_config` after the final audit, or snapshot the
pre-rebase checkpoints and restore them inside `rollback`. The
`rebase_outcome` Err arm at `:16333` is not affected — `mutate_config`
returns before `write_json_atomic`.

---

### F6 — `session_detail`'s ownership revalidation still confers ownership by containment **[measured]**

**Defect.** `crates/mallard-infrastructure/src/session_detail.rs:930` —
`resolved == project_root || resolved.starts_with(project_root)`.
Byte-identical to the pre-change version. This is the gate behind
`session_detail.rs:862`'s "The session no longer belongs to the selected
project": the app's own ownership re-check, still on containment after the
change that abolished containment.

**Consequence.** §2's "Discovery and the Codex Desktop audit are the two
outliers still on containment" (`PLAN_EXACT_HOME_OWNERSHIP.md:71`) is an
incomplete inventory — there were three. Concretely, a live Claude session
that appends a `<root>/src-tauri` record mid-request is rejected by discovery
(F1) but **accepted** here, so Session Detail serves a full header and
timeline for a session that now belongs to a different directory-project,
with `project_root_aliases` relativizing and redacting every path against the
wrong root. Given F1's measured 37/77 oscillation rate, the window is not
theoretical.

**Fix.** Drop the `starts_with` disjunct; better, delete the local helper and
call `provider_capture::session_home_is_project_root` at the `:862` call site
so discovery and revalidation cannot drift. No test currently touches this
gate.

---

### F7 — Repair's consent dialog misdescribes what it rewrites **[measured]**

**Defect.** `src/components/project-sync/ProjectSync.tsx:1734` promises
"update structural cwd fields **without changing messages or history**."
`remap_codex_conversation_paths` passes every record through
`SessionPathMapper::encode_record`/`decode_record`, rewriting every
boundary-valid occurrence in every string value and object key — message text
included. The function's own doc comment says so
(`commands.rs:16520-16528`), and a test asserts it (`commands.rs:19588`).

**Provenance.** The copy predates this change; §5 reaffirmed it rather than
introducing the mismatch. But exact-home **widened exposure** by deleting the
multi-root blocker, so strictly more rollouts now flow through the same
inaccurate consent text — on a destructive, file-rewriting operation.

**Fix.** Name the real scope: "rewrite every mention of the old project path,
including inside message text, tool output, and shell commands." Do not
narrow the backend; the full scope is deliberate. Pin the string with a
frontend assertion. Note the existing test at `commands.rs:19309-19312` is
vacuous on this point — its fixture uses a trailing *space*, which
`boundary_valid` rejects.

---

### F8 — Two directory-projects that munge to one Claude bucket each export the other's memory

**Defect.** `encode_claude_project_path`
(`claude/materialization.rs:12-19`) maps every non-alphanumeric byte to `-`,
so `/Users/x/proj/a` and `/Users/x/proj.a` share one bucket and the
self-consistency check passes for **both**. The comment at
`claude/discovery.rs:103-107` and §3/D3 ("foreign buckets never contribute
state") claim this is prevented; it is not. Sessions still separate correctly
by leaf cwd at `:99` — only the memory resource is affected.

**Scope.** Claude already commingles the two directories' memory in one
physical tree before Mallard sees it. Mallard's new harm is exporting that
tree into two independent bundles, so a pull onto a machine where the roots
do **not** collide mixes them there.

**Fix.** Prove the bucket *exclusive*, not merely self-consistent: record
every distinct recorded home that munges to each candidate bucket and emit
`claude:memory:root` only when that set is exactly `{project_root}`.
Gate the pull side the same way. Correct the comment and §3/D3.

---

### F9 — Ownership admits cwd spellings the materialization gate rejects

**Defect.** `provider_capture.rs:1752` owns by *canonical* equality;
`conversation_materialization.rs:1299-1306` recognises only two *literal*
strings (`project_root`, `canonical_project_root`). Discovery ownership is
strictly wider than round-trippable ownership.

**Trigger.** A case-only rename of the project root on case-insensitive
APFS: `realpath` case-folds, `SessionPathMapper` matches case-sensitively.
Old sessions stay owned, push ships their cwd byte-literally, and every
materialization dies with "staged codex transcript has the wrong project
cwd." Because the Codex audit uses the same canonicalizing comparison
(`commands.rs:15871`), the unpullable thread is classified **healthy** and
the repair that would fix it is never offered.

**Fix.** Canonicalize `source_cwd` in the materialization gate, or add the
recorded spelling to the mapper at capture time.

---

### F10 — The audit's `warnings` channel has no reader **[measured]**

**Defect.** No file under `src/` reads `audit.warnings`,
`assigned_thread_count`, or `matching_thread_count`, though all three are
declared (`types.ts:301-302`) and populated.

**Consequence.** The remedy the classifier computes for the subdirectory case
(`commands.rs:15877-15880`, "bind that directory as its own project to manage
it") — the entire middle row of §4's table — is discarded. A thread Codex
Desktop says belongs to the project silently never syncs, `ready` stays true,
and nothing renders. §5's "warnings gain the subdir-separate-project case"
treats `warnings` as a UI surface that does not exist.

**Honest framing.** The dead channel predates this change (four push sites,
no reader then either). What this change adds is a **new class of
silently-excluded thread** routed into it.

**Related.** `assigned_thread_count` silently changed meaning — it is now
`nominated_thread_ids.len()` (`commands.rs:15605`), i.e. assigned **plus**
aux-only. Undocumented, and the name is now wrong.

---

### F11 — Repair executes a freshly re-audited issue set, not the one the user approved

`ProjectSync.tsx:1733` names `audit.issues.length` from the cached client
audit; `repair_codex_conversation_paths_with_repository` re-audits
server-side (`commands.rs:15865`) and loops over whatever it finds. No count,
thread list, or token crosses the IPC boundary. The user can approve "repair
2 conversations" and have a third rewritten.

**Fix.** Return a stable issue-set identity from the audit, send it with the
repair call, and repair only the intersection — or fail with "the audit
changed; review and confirm again."

---

### F12 — Aux-only threads can still block the audit **[measured]**

§9 and the loop's own comment (`commands.rs:15779-15780`) promise "auxiliary
threads never block the audit." Four sites break it:
`commands.rs:15630` and `:15639` (nomination hands `&mut audit.blockers` to
`insert_safe_assigned_thread`), and `:15751` / `:15757` (walk-phase blockers
pushed for any *nominated* thread, not just assigned).

Only `:15751` is reachable in practice — a TOCTOU when Codex Desktop archives
an aux-only rollout between `scan_metadata` and `fs_canonicalize` — and it
transiently pauses Push, Pull, **and** Repair for the whole project with a
message that calls an unassigned thread "assigned." (`:15757` is unreachable:
`follow_links(false)` + `is_file()` + pre-canonicalized `codex_home`. The
malformed-key path is unreachable with real data: all 639 thread-map keys on
this machine are bare UUIDs.)

**Fix.** Gate `:15751` on `assigned_thread_ids.contains(...)`, matching
`:15772`/`:15795`; give the two nomination sites a discarded blocker sink.

---

### F13 — Six assigned rollouts are permanently unauditable and unrepairable **[measured]**

`commands.rs:15724-15731` drops any rollout over
`conversation_file_limit_bytes()` into `oversized_thread_ids` **before**
classification; `:15784-15790` emits a warning and continues.

Measured for `/Users/hequ/Desktop/project/mallard-app`: of 172 assigned
threads, **6 exceed the 100 MiB default** — 320, 320, 285, 278, 262 and
107 MB. They are never classified, so they can never become a repair issue.
If the project moves, their `session_meta.cwd` keeps the stale root
**forever**: raising the limit past 320 MB also raises the per-file sync
limit for everything else. The only signal is a `warnings` string, which F10
established nothing reads.

**Fix.** Classify oversized threads from the bounded head scan —
`scan_metadata` already yields `meta.cwd`, which *is* the authoritative home —
and surface them as a blocked row rather than an unread warning.

---

### F14 — A duplicated rollout is a buttonless, permanent Push/Pull block (latent) **[measured]**

`commands.rs:15805-15810` blocks any assigned thread with more than one
rollout file. `transcript_paths` collects from **both** `sessions/` and
`archived_sessions/`. The other half of this feature explicitly declares that
shape normal and tolerates it —
`providers/codex/discovery.rs:76-90`: *"Codex can briefly leave the same
transcript in both … while an archive operation settles"* — keeping the
active copy plus a warning.

A **blocker** (unlike an issue) sets both `ready=false` and
`can_repair=false`, and `ConversationPathRepairNotice.tsx` renders the blocked
branch with `action={undefined}` — verified: **no Repair button at all**.
Push and Pull are off with no in-app remedy.

**Status: latent on this machine.** The duplicate-file condition exists
(`019f8305-a963-7602-81e1-cd3f38fe8051` is in both trees and has been since
2026-07-20), but that thread is unassigned, and no *nominated* mallard-app
thread currently has more than one rollout. It fires the moment an assigned
thread is archived.

**Fix.** Apply discovery's rule — prefer the `sessions/` copy, warn about the
`archived_sessions/` one — instead of blocking. No test covers this; the
string `"rollout files"` appears once in the repo, at the emitting line.

---

### F15 — The audit re-reads 1.33 GB on every Push, Pull, and shell load **[measured]**

`commands.rs:15839` reads each nominated, non-oversized rollout **whole**
(`fs::read`) and JSON-parses it, unconditionally and before classification,
on every call — and the audit is invoked from Push (`:10065`), Pull
(`:10524`), and the frontend's `loadConversationPathAudits` at shell load.

Measured for mallard-app: 174 nominated threads / 2.91 GB on disk; 6 skipped
as oversized; **168 files, 1.33 GB, fully read and parsed per call**, plus a
`scan_metadata` head read of all 1,554 rollouts during the walk.

Two guards that exist elsewhere are absent here: repair has an aggregate
ceiling (`MAX_CONVERSATION_REPAIR_BYTES`), discovery has an inventory cache
(`session_watch_service.rs:820-869`). The audit has neither.

**Fix.** Cache the audit keyed on the state file's stat plus each rollout's
`(size, mtime)`, and classify from the bounded head scan instead of reading
whole files — the authoritative home is the *first* `session_meta`.

Related, Claude side: `claude/mod.rs:52-88` reads every transcript to EOF
before the ownership filter, ~762 MB / 183k records per uncached pass, with
per-file bounds but no profile-level ceiling like Codex's
`MAX_PROFILE_SESSION_FILES`. This full read is **mandated** by the
leaf-identity design (a head scan cannot compute the leaf), so it is a cost
the design incurred, not a reverted optimization — but the aggregate ceiling
its sibling has is still missing. The filename-gate mismatch also emits its
warning into whichever project happens to be scanning, since `mod.rs:101-103`
discards the leaf cwd it already holds: 9 of 105 files trip it, so every
project sees 9 cross-project warnings on every pass, permanently.

---

### F16 — An oversized Claude transcript vanishes; an oversized Codex rollout stays visible

`claude/mod.rs:45-50` returns `Err` on the size gate, so `discovery.rs:88-98`
warns and continues — the session never reaches `discovery.sessions` and
never gets the `blocked_reason` row that exists for exactly this case at
`:119-124`. Codex has no size check in `scan_metadata`, so its oversized
rollouts list as blocked rows. This is a regression: before the change,
Claude used the bounded-head scanner and oversized transcripts surfaced as
blocked rows.

At the 100 MiB default nothing trips (largest Claude file is 53 MB); choosing
25 MB in Settings drops 8 transcripts corpus-wide, 5 in mallard-app.
Fail-closed is defensible — Claude ownership genuinely cannot be determined
without the leaf — but it should produce a size-attributed blocked row, not a
free-text warning nothing renders.

---

### F17 — The clean cutover has no enforcement point **[measured]**

§7 says to wipe `~/.mallard` and the cloud buckets because "ResourceDescriptor
and state logical suffixes change shape." That instruction is the only thing
standing between the user and a stale bundle:

- `BUNDLE_SCHEMA` is still `1` (`crates/mallard-domain/src/lib.rs:21`), so a
  pre-change bundle passes the schema gates.
- `ResourceDescriptor` has **no** `#[serde(deny_unknown_fields)]` — 25 other
  structs in that same file do — and `relative_cwd` plus its validate arm
  were deleted.

So an old manifest carrying `relative_cwd: "apps/web"` deserializes cleanly,
the field is silently discarded, the transcript materializes at the project
root, and the pull dies at `conversation_materialization.rs:1299-1305` with
"staged codex transcript has the wrong project cwd" — a message that names
neither the cutover nor the wipe. Since ownership moved from containment to
equality, subdir sessions are exactly the population most likely to be in a
stale published bundle.

**Fix.** Bump `BUNDLE_SCHEMA`, or add `deny_unknown_fields` so a stale bundle
is rejected with a message that names the cause. A doc line is not an
enforcement point.

---

### F18 — The two "exact equality" helpers disagree on a missing directory

`commands.rs:16428` falls back to the raw string when `canonicalize` fails;
`provider_capture.rs:1753-1756` fails closed. §2 presents them as one law and
§9 lists them side by side as the new helpers; nobody compared them.

The consequence lands on **nomination**. After a project moves, Codex
Desktop's `local-projects` entry still records the old `rootPath`, which no
longer exists → `canonicalize` fails → the raw old string is compared against
the canonical new binding root → no match → `desktop_project_ids` is empty →
early return with `assigned_thread_count = 0` and **`ready = true`**. The
entire §4 repair flow is invisible in exactly the scenario §1 opens with.

Every fixture builds the desktop project at a path that exists, so this arm
is untested in both directions. All 22 real `local-projects` rootPaths
currently resolve, so the state is not present today — but it is the
definitional post-move state. Whether Codex Desktop rewrites its `rootPath`
on a move decides whether this fires; that needs a live check.

**Fix.** Decide deliberately: if a stale desktop rootPath should still
nominate (it should — that is the moved project), match lexically against the
binding's recorded `project_root` as well as its canonical form, and say so
in §4's nomination table.

---

## 3. Design fidelity

**Structurally faithful.** Every deletion, both new helpers, and all six
cells of §4's classification table are implemented as designed (§1 above).

**Two substantive infidelities:**

1. **§2's containment inventory is wrong.** It names two outliers; there were
   three. `session_detail.rs:930` is untouched and unmentioned (F6).
2. **§2's Claude premise is false against the corpus.** "Resumed sessions are
   identified by their leaf record" presumes cwd changes only on resume. It
   changes 41 times in one file on this machine. The law as written cannot
   produce stable Claude ownership (F1).

**§9 is candid but three claims are untrue as written:**

| §9 claim | Status |
|---|---|
| "Aux threads … never blocking" | **False** at four sites, one reachable (F12). The companion claim that aux-only skips oversized/missing-rollout warnings **is** true. |
| "Multiple qualifying spellings emit one memory resource (first in order) plus a warning" | **False** when the first-ordered bucket has no `memory/` — zero resources, and the warning misdescribes the outcome (F4). |
| "Keeps firmlink spellings working without consulting the binding string" | True of capture, **false end-to-end** — the write half consults exactly that string (F3). |

Also: §3/D3's "foreign buckets never contribute state" is false under a munge
collision (F8); §5's "warnings gain the subdir-separate-project case" names a
UI surface that does not exist (F10).

**One doc-only correction.** §3 justifies deleting session-side
`excluded_roots` partly by claiming "the watch service passes no exclusions
today (`session_watch_service.rs:1035`), so watch-path and command-path
ownership disagreed." At `31bcc5d`, line 1035 is inside `mod tests`. The
inconsistency never existed; the exact-equality argument in the preceding
sentence is self-sufficient. Strike the clause.

## 4. Test coverage

- **D3 has no protecting assertion.** Deleting the self-consistency guard at
  `claude/discovery.rs:108-114` and reverting to an unconditional
  `root_buckets.insert(...)` ships the whole bar green. `claude:memory`
  appears exactly once in all of `crates/`, `src-tauri/`, `tests/`. All four
  memory fixtures are either self-consistent by construction or have an empty
  `memory/`. The two fixtures that *did* cover a non-self-consistent bucket
  before the change were rewritten to self-consistent ones — that is the
  retarget that went vacuous. This matters concretely: binding
  `.../mallard-app/src-tauri` yields owned sessions living in the parent's
  bucket, whose `memory/` holds 59 real files, and the guard is the only
  thing keeping the parent's `MEMORY.md` out of the child's Push.
- **F6, F14 and F18 have no test at all** (the session-detail gate, the
  duplicate-rollout blocker, the missing-desktop-root nomination arm).
- **F7's existing test is vacuous** on the claim it appears to cover.
- **What is well covered:** the §6 retargets landed. The ownership fixture
  (`commands.rs:19076+`) now exercises four nominated threads and asserts one
  matching, one issue with the right `recorded_cwd`, and a subdir warning that
  is *not* an issue — a faithful test of §4's table.

## 5. What could not be verified by reading

1. **Whether the macOS folder picker can produce
   `project_root != canonical_project_root`.** The severity of F3, F4 and F9
   hinges on it. The fields are `readOnly`, the only writer is
   `open({directory:true})`, and it is the default in the crate's own macOS
   tempdir tests — but a live run with a symlinked ancestor is the only proof.
2. **The `mallardInternal` dead-end end-to-end.** Every link is proven
   statically (assignment, home, 574 mentions, `can_repair` formula, pre-scan
   abort); nobody bound the project and clicked Repair.
3. **Whether F5's rebase race actually fires** — the window is real; the
   deterministic routes to a failing final audit are pre-filtered.
4. **Whether Codex Desktop rewrites `rootPaths` when a project moves** —
   decides whether F18 fires.
5. **Wall-clock cost** of F15's 1.33 GB read on a real Push.

## 6. Recommended fix order

1. **F1** — Claude leaf-home rule. It is silently hiding 12 of this machine's
   77 mallard-app transcripts today, and it is a two-line change to the
   tracker plus a §2 correction.
2. **F2** — staleness guard in the classifier + `can_repair` running the
   collision predicate. Prevents both the dead-end and the
   rewrite-a-live-sibling corruption variant.
3. **F6** — delete the `starts_with` disjunct in `session_detail`; call the
   shared predicate. Completes the law.
4. **F7** — correct the consent copy. One string, and it is currently false
   on a destructive action.
5. **F3 + F4 + F8** — the Claude memory bucket cluster, together with the
   missing D3 test.
6. **F12, F13, F14, F10** — the blocker/warning plumbing: make aux silent,
   classify oversized threads from the head scan, tolerate the duplicate
   rollout, and give `warnings` a reader.
7. **F17** — an enforcement point for the cutover.
8. **F15** — cache the audit; classify from the head scan.
9. **F5, F9, F11, F16, F18** — as capacity allows.

## Appendix — reproducing the measurements

All numbers above were taken at `d78e3c5` against the live corpora.

- Claude ownership (F1): replay `is_chain_record` + leaf selection over
  `~/.claude/projects/-Users-hequ-Desktop-project-mallard-app/*.jsonl`,
  comparing the final chain-record `cwd` to the project root.
  → 77 files, 37 oscillating, 5 leaf-cwd `src-tauri`, 7 stem-gated.
- Cross-project thread (F2): `thread-project-assignments` in
  `~/.codex/.codex-global-state.json` for thread `019f7c76…`; first
  `session_meta.cwd` of its rollout; `grep -o` counts of both root strings.
  → assigned to `mallardInternal`, home `mallard-ai-web`, 574 / 92.
- Audit read cost and oversized threads (F13, F15): resolve assigned + aux
  thread ids for the mallard-app root, locate each rollout, sum sizes.
  → 172 assigned + 2 aux; 2.91 GB total; 6 over 100 MiB; 168 files / 1.33 GB read.
- Duplicate rollout (F14): intersect thread ids present in both
  `~/.codex/sessions` and `~/.codex/archived_sessions`.
  → `019f8305…`, unassigned, so latent.
- Desktop state shape: 22 local-projects (none with >1 rootPath, none
  nested), 302 assignments (290 local / 12 remote, all with `projectId`),
  274 writable-roots (87 multi-entry), 27 hints; 55 writable-only and 16
  hints-only threads; no `electron-persisted-atom-state` nesting.

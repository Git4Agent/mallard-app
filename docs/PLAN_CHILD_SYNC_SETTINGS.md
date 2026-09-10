**Plan: Control child inclusion in Push**

Status: Implemented. Date: 2026-09-09.
Verification: [test and performance record](CHILD_SYNC_SETTINGS_VERIFICATION.md).

Add two independent Settings switches: **Push Codex children** and
**Push Claude children**. Both default to On. Each applies to all projects,
storages, and profiles for that provider on this computer. Pull restores
exactly the files listed in the cloud and keeps local settings unchanged.

**Decision:** keep the `wire_children` On/Off stamp and the cloud-stamp
comparison rule. The cloud stamp selects Compare scope. The local setting
selects the next Push output and stamp. Pull uses the stored stamp and files.
An On export with zero children remains distinct from an Off export.

Keep schema **1** for settings, local state, and manifests. Add no migration,
legacy execution path, or compatibility layer. The user will clear Mallard
state and cloud data before retesting. Native provider files remain intact.

The implemented [Codex design](PLAN_CODEX_OWNED_SUBAGENT_SYNC.md) and
[Claude design](PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md) retain ownership, child
formats, hashes, conflict rules, and complete Pull validation. Codex supports
its existing owned descendants. Claude supports direct children only.
The parent remains the review, selection, publication, and result unit.

**Scope and publication**

Add required `wire_children: "on" | "off"` to each cloud conversation row.
Off requires an empty child file inventory. On permits zero or more children;
it records that inclusion was enabled, not that child files exist. Existing
unsupported-format omission rules still apply to an On export. Children have
no separate stamp. Keep this stamp independent of `wire_redaction`.

Reject missing, null, or unknown stamps and Off rows with child entries during
manifest validation, before fetching child payloads. Do not infer a stamp from
the file inventory. A missing cloud row follows existing presence rules;
CreateCloud uses the local setting. A row with an invalid stamp is not a
missing row.

| Cloud stamp | Local Push setting | Compare scope | Approved Push output |
| --- | --- | --- | --- |
| Off | Off | Parent only | Parent without children; Off stamp |
| Off | On | Parent only | Parent and supported owned children; On stamp |
| On | Off | Parent and supported owned children | Parent without children; Off stamp |
| On | On | Parent and supported owned children | Parent and supported owned children; On stamp |

Cloud On uses the existing child comparison and unsupported-format omission
rules. When child comparison is active, use the union of local, cloud, and
accepted child keys. Cloud Off excludes all child evidence and accepted child
pairs from the fold, including pins retained from an earlier On publication.
Do not erase accepted data during Compare. Main transcript changes always
count, including parent messages about children.

Use the existing [comparator and fold](../crates/mallard-domain/src/conversation_tail_relation.rs)
and [action policy](../crates/mallard-domain/src/conversation_reconciliation.rs).
Extend the existing publication condition:

~~~text
export_settings_changed = existing redaction stamp change
                       OR cloud wire_children != outgoing child stamp
~~~

Generalize the existing `redaction_changed` gate to `export_settings_changed`
in the decision, action validation, and publication record. It permits Publish
for an otherwise synchronized task with a different export stamp, including
an empty On export. Use the same cloud task ID. Keep normal LocalOutstanding
and CreateCloud actions. CloudOutstanding, Divergent, and Unavailable retain
their existing conflict actions. A stamp mismatch is a Push reason; it does
not manufacture a content difference or bypass a conflict.

Saving a setting publishes nothing. It makes otherwise eligible tasks with a
different cloud stamp available for reviewed Push, even without new messages
or child files. After publication, an unchanged Push with the same settings
is a no-op.

~~~text
REFRESH / REVIEW
  Read and validate cloud stamp; use existing local observations
       |
       +-- Off -> existing parent-only fold; ignore ALL child pins
       +-- On  -> existing main + supported-child fold
       |
  Existing action policy + export-stamp-change publication condition
       |
PUSH: review outgoing selection -> capture approved scope -> publish
      Local On: main + supported children + On stamp
      Local Off: main only + Off stamp

PULL: validate stored stamp -> restore every listed file
      Keep local settings; the next Push uses those settings
~~~

A-Off -> B-On example: A publishes the parent without children and stamps
Off. B pulls that content and keeps its local setting On. Compare uses cloud
Off and checks only the parent. B's next reviewed Push uses On and publishes
an On stamp, even if B has no children. Pull cannot recreate omitted files.
Once the cloud stamp is On, supported child-only changes enter Compare.

**Settings and review**

Add `push_codex_children` and `push_claude_children` to Rust `AppSettings` and
the TypeScript DTO. Use serde defaults of `true`, consistent with the existing
partial-settings contract and credential-redaction field. A missing field or
settings file means On. Invalid field types still fail validation.

Use the existing Sync section, backend save-on-change path, loading state,
and inline save error. Keep two switches. Use one publication reason label:
**Export settings changed**. Keep the current information icon for omitted
children. Its Push tooltip says: “Child conversations are excluded from this
Push. Pull restores the children stored in the cloud.” Estimates include only
the outgoing files. Add no confirmation dialog or per-child selection.

Update Push decisions from existing observations after a save. Compare scope
continues to depend on the cloud stamp. Reuse the existing operation settings
snapshot and host lock. Keep cache purge through `tuning_differs` unchanged;
these switches do not purge caches. Add no settings reads per child or new
settings command.

**Reuse the existing omission and approval paths**

At the consumer in [service.rs](../src-tauri/src/project_sync/service.rs),
apply parent-only comparison before building child pins when the cloud has
an Off stamp. Reuse the existing omission return. If children are required for
Compare, retain their evidence even when outgoing Push is Off. Cloud Off with
local On still requires valid child source evidence for an On Push. Cloud On
with local Off retains existing child conflict checks before an Off Push.

For outgoing Off, use the existing selection: `omitted = true`, empty keys,
and zero child bytes. Keep comparison pins and outgoing selection distinct;
cloud children can participate in Compare while the approved export omits
them. Route this selection to the two provider capture sites and bypass child
reconstruction, encoding, and payload preparation when it is omitted.

Record the outgoing stamp once in the existing per-parent Push approval.
Include it in the Review-to-Apply selection check beside keys and `omitted`.
An On export can omit unsupported children, so `omitted` alone cannot detect
a change to the approved stamp. A changed stamp or selection fails only the
affected parents; changing one provider's switch does not fail the other.
Reuse the existing retry path. Keep byte capture at Apply, final source checks,
and mixed-result handling. For a source change after capture, retain:
“This task changed during Push. Push it again to include the latest work.”

Always retain the current setting-independent observation path and cache
format. Do not add an observation-scope gate, coverage protocol, or a new
meaning for `subagents: None`. Main parsing already collects creation records
and links. Off still pays existing observation costs; it saves child work in
Push payload preparation and transfer.

Preserve validated main evidence if attaching a child observation fails.
During Compare, apply that failure only when child comparison is required.
Main-source failures must still block both scopes. Do not cache a failed child
observation as a complete successful tree; retry it through the existing refresh path.
This error boundary is needed because the current child attachment can return
an error before the consumer reaches its omission check.

**Metadata and Pull behavior**

The cloud conversation row gains only `wire_children`. Mirror it in the Rust
domain and TypeScript DTO. Add no child codec, binding mode, checkpoint stamp,
or cache key. The manifest digest already covers the stored stamp. Reuse the
current compact child states and local/normalized hashes.
`accepted_subagents` is already built from captured children. An omitted
export therefore accepts an empty map. `commit_conversation_publication`
already removes the old row's file entries before installing its replacement.
Reuse these paths; retain native files and installed child mappings on Push.

Pull validates the stored stamp and restores every listed file. On restores
the listed children; Off restores none. First-contact adoption uses the stored
stamp and child inventory to reproduce cloud-form evidence and references.
Outgoing Push uses the local setting. Keep existing redaction validation,
native reference handling, installation cutoffs, and installation-map checks.
A local switch change does not invalidate a Pull plan or change adoption.

If another machine publishes Off after this machine accepted children, Compare
ignores the retained child pins. No new missing-cloud-child recovery is needed
for that change. With cloud On, retain the existing missing-child validation
and supported Keep both recovery. Push Off removes child entries from the new
cloud snapshot; it does not delete native files or old immutable bundles.

**Retain unchanged child uploads**

The current child publication path assigns the parent's `source_mtime` to
all children. Correct it to the captured child's source mtime. Use the existing
snapshot metadata; add no extra stat or metadata cache. For a Codex child,
use its leaf file's timestamp; its full source set still supplies content proof.

That correction alone does not avoid child uploads. The `can_retain` check
in `build_publish_request` serves generic captured resources. Children are
added later as captured sidecars, and `commit_conversation_publication`
places them in `captured_files`. The
[bundle engine](../crates/mallard-infrastructure/src/bundle_engine.rs) queues
those captures without comparing their digests with the previous publication.

Reuse the existing Retained/Captured file placement for child entries too.
After required capture and encoding, compare each output with its previous
verified file entry. Retain it only when identity, digest, size, and required
file metadata agree. Preserve retained child entries when replacing the row.
A changed child still enters the captured path. Do not trust mtime instead of
content evidence, skip source checks, or add an object-existence request per
child. This saves repeat uploads; it does not remove child capture or encoding.

Measure a parent append with eight unchanged children, then one changed child,
using synthetic fixtures and the S3 or Dropbox adapter with a test server.
Record publication object/byte counters and transport request-body bytes.
Publication counters count submitted objects, including AlreadyPresent;
they are not newly stored bytes. Existing-object verification can also read
bodies back. A conditional PUT is not evidence of zero network traffic.

Acceptance: unchanged retained children submit zero child object bodies; a
changed child submits its new body. Check the normalized bytes and accepted
states against the existing full-capture result. Report CPU preparation and
transport changes separately. Measured results are in the verification record.

Retaining the stamp has a separate cost. A stamp change requires the existing
capture and publication path even when content bytes stay equal. Machines
with opposite settings can make an unchanged task eligible for Push after
each other's publication. Content can still compare Synchronized. Measure
this empty-export case separately; add no special stamp-only publication path.

**Implementation and checks**

1. Measure the unchanged-child upload path. Correct child mtime and add retained
   child placement with the focused transport check above.
2. Add the two settings and switches using existing defaults and save behavior.
3. Add the required stamp and validation. Apply cloud-stamp scope at the
   consumer. Extend the shared publication gate and approval check, and route
   outgoing omission to capture.
4. Reuse existing lifecycle fixtures for four focused checks per provider:
   the four cloud-stamp/local-setting combinations; Review-to-Apply changes;
   A-Off -> B-On with fresh and previously accepted B; and removal of existing
   cloud children without new messages. Include zero and eight children,
   invalid stamps, mixed-provider results, and the child-error scope boundary.
   Check that stamp-only Push preserves Synchronized content status and Pull
   keeps local settings. Reuse the existing child parser, restore, ownership,
   and native-resume suites.

Use temporary homes and fixture storage only. Use the affected Rust tests, frontend build and integration suite, and the
complete Tauri Settings/Push review check as the verification gate. Existing
[Codex](CODEX_OWNED_SUBAGENT_SYNC_VERIFICATION.md) and
[Claude](CLAUDE_OWNED_SUBAGENT_SYNC_VERIFICATION.md) benchmarks remain the
reference for local-folder timings; they do not measure remote upload savings.
The switches control export scope and volume. Do not promise a fixed per-child
speedup from those fixture timings.

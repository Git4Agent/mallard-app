# Mallard Cloud comparison and Push plan

Status: plan only. Implementation changes were reverted at the user's request.
No implementation work is active.

## Required behavior

Select the comparison source from the current repository visibility.

| Repository | Comparison and Pull source | Push content source | Push destination |
| --- | --- | --- | --- |
| Public | Current published version | Published version plus reviewed changes | Replace the unpublished private snapshot |
| Private | Latest private version | Latest private version plus reviewed changes | New current private version |

For a public repository, an upload does not establish public synchronization.
A session is synced only when it matches the published session, using verified
local and portable content evidence. Owner approval remains a separate release
operation. Admin approval remains required where the release rules require it.

Planning assumption: “append” means retain existing sessions and apply reviewed
additions or updates. It does not mean selectable history of every private
version. This plan uses this meaning.

For public Push, replacement means rebuild the draft from published content
and reviewed changes. Do not carry old draft-only content into the replacement
without review. Show draft-only content that will be removed. Keep published
content unless an approved action changes it.

## Behavior verified before this change

The cloud service uses the same private snapshot replacement process for public
and private repositories. Each committed upload creates a private publication,
advances the private version, and retires the previous private publication.
It does not change the published version. It invalidates open release requests.

Before this change, the desktop carried previous session content forward for both
repository types. Thus, replacing the private snapshot pointer does not imply
that all previous draft-only content is removed.

Private snapshots are not permanent version history. Cleanup can delete payloads
referenced only by retired publications. It retains a bounded set of retired
private manifests for continuity checks. The history endpoint lists public
publications.

Source locations:

- Desktop: `src-tauri/src/project_sync/service.rs`,
  `build_thread_sync_comparison` and the publication builder.
- Desktop: `crates/mallard-infrastructure/src/managed_cloud/store.rs`,
  `head_bytes` and `commit_head`.
- Service: `cloudflare/managed-cloud/src/routes/uploads.ts`, `commitUpload`.
- Service: `cloudflare/managed-cloud/src/routes/content.ts`, `getHistory`.
- Service: `cloudflare/managed-cloud/src/jobs/cleanup.ts`.

Verification completed: the existing upload, release, and cleanup suites passed
in an isolated temporary copy: 3 files, 22 tests. These are local service tests.
They do not verify the deployed service or the native desktop application.

## Metadata changes

The fields below describe the proposed changes. Schema versions stay at 1.
No new stored local fields or Cloud database columns are proposed.

### 1. Cloud response: explicit comparison and write targets

Replace the desktop head response's ambiguous `head` with:

| Field | Purpose |
| --- | --- |
| `comparison_head` | Exact version used by status, inventory, and Pull |
| `write_head` | Current private version that Push can replace; only return it to authorized callers |
| `private_revision` | Existing private version used for the write conflict check |
| `link` | Existing visibility, permissions, release state, and repository revision |

Reuse the existing head fields: commit ID, manifest revision, manifest digest,
manifest size, scope, and public or private version number. Keep manifest
revision distinct from the public release number.

The server must read the targets and repository revision consistently. A public
comparison must not fall back to the private draft. A missing or unreadable
published target must produce an explicit unavailable or absent result, as
appropriate to the repository state.

Keep release inspection's access to private content. Do not change a shared
read helper globally if that would change Owner or Admin review behavior.

### 2. Local metadata: keep the existing fields

Do not add `managed_cloud_draft` or a stored comparison-source field.
Use the existing `scope_bases` and `conversation_checkpoints` maps.

For public Push, keep the published comparison bases and checkpoints. An upload
must not replace them with evidence for an unpublished snapshot. Keep the
existing atomic update of session identity, path mapping, and upload receipts.

When the published content changes, use the existing encode-and-compare path
to verify local content against its portable tail. This path accounts for path
mapping and credential redaction. Use new evidence only when verification
succeeds. If it fails, keep the existing evidence and safety checks.

The evidence is computed for the current review. It is not another stored
metadata field. A later approved Pull can update the existing checkpoints.
Private Cloud and ordinary storage keep their existing checkpoint updates.
The existing scope safety gate remains: after a new public release, a Pull
review can be required before the next Push, even when session content matches.

### 3. Review and apply: reuse existing revision checks

Comparison reads the latest published version for a public repository. It does
not need a new saved public-version counter or a separate public-version check.
Use the snapshot's existing commit ID, manifest revision, digest, and size for
content verification.

Carry a temporary `managed_cloud_review_token` with the Push review. It contains
the existing repository revision, private revision, and private commit ID.
The verified snapshot and its head token carry the comparison content.
Together they preserve:

- Comparison basis and repository visibility.
- Comparison commit ID, manifest revision, digest, and size.
- Expected private commit ID and private version.
- Expected repository revision.

Use the published snapshot to build public Push content. Use the private target
for the write conflict check. Validate retained objects against the reviewed
content source, not against an unrelated draft manifest.

At commit, use the existing repository revision check to detect a release or
visibility change. Use the existing private version and commit check to detect
a concurrent Push. Keep digest checks, immutable object writes, and atomic
compare-and-swap behavior.

The service already stores repository revision and private revision on uploads.
Publication and visibility changes advance repository revision. Preserve the
repository revision observed with the reviewed published snapshot through upload
creation and commit. Do not replace it with a newer revision fetched during
upload setup. No additional published-version guard or database column is needed.
Verify all mutation routes follow this revision rule before relying on it.

Example: review against public A at repository revision 5. Another user publishes
B, advancing repository revision to 6. The existing expected revision 5 check
rejects the stale operation. The user reviews again against B.

### 4. Frontend comparison metadata

Reuse the Cloud status visibility and the existing selected revision in the
comparison DTO. Use precise status details:

- Public: “Matches published version.”
- Private: “Matches latest private version.”

Add a temporary `unpublished_changes` list to the Push review. Show content
that differs from the published source and must be selected again to keep it.

Keep release state separate from session state. A successful upload may show an
upload result, but must not overwrite the comparison result with synced rows.

### 5. Metadata that stays unchanged

- Local and portable schema version numbers: 1.
- Portable bundle manifest format and session identity fields.
- Cloud database public/private publication pointers and version counters.
- Release approval states and role requirements.
- Credentials and local path data remain machine-local.

The stored local JSON and portable manifest formats stay unchanged. The head
API and temporary review DTO change directly. There is no migration, automatic
backfill, old-contract reader, or legacy execution path.

## Implementation sequence

1. Define the explicit targets in the cloud API. Update service fixtures and
   the Rust client/stub together. Remove the old desktop head contract.
2. Separate comparison, content construction, and write-conflict inputs in the
   Cloud adapter and publication flow. Do not add a fallback to draft comparison.
3. Keep existing local metadata; adjust checkpoint selection and atomic receipt persistence.
   Public upload success must leave published comparison evidence intact.
4. Apply the selected source to activity rows, remote inventory, Pull preview,
   Pull apply, and refresh. Keep Owner/Admin draft inspection available.
5. Make public Push replace the reviewed draft. Show draft-only removals in the
   review. Preserve existing private-repository session retention behavior.
6. Update frontend DTOs, status text, tests, and active documentation. Remove
   superseded code in the same cutover.

Main desktop files: `src-tauri/src/project_sync/service.rs`,
`crates/mallard-infrastructure/src/bundle_engine.rs`,
`crates/mallard-infrastructure/src/managed_cloud/{client,store,stub}.rs`,
`src/types.ts`, and the project-sync comparison/review components.

Main service files: repository head routes, upload routes, access/query helpers,
and the corresponding tests under `cloudflare/managed-cloud`.

## Acceptance tests

| Case | Required result |
| --- | --- |
| Public A, local B; Push B | Published A stays selected; B remains outstanding |
| Approve and publish B | Local B becomes synced after verified refresh |
| Public A, local A, draft B | Local A is synced with published A |
| Reject B | Local B remains different from published A |
| Replace draft B with C | Public A stays unchanged; reviewed draft-only content is removed; old request is invalidated |
| Private A; Push reviewed B changes | New latest version retains existing sessions and applies B |
| Repeat an identical Push | No duplicate sessions or unnecessary version; public comparison stays correct |
| Another user pushes during review | Stale apply fails; no silent overwrite |
| Release or visibility changes during review | Stale apply fails; new review selects the correct target |
| Restart after public Push | Draft upload does not become published comparison evidence |
| Draft-only session is absent publicly | Report unpublished/local-only content; do not treat it as lost cloud history |
| Public target is missing or unreadable | No fallback to the draft and no false synced result |
| Local bytes differ from portable encoding | Verified paired-tail comparison remains correct |
| Codex/Claude child-session or export settings change | Status and Push decisions preserve existing safety checks |

Run service type checks, build, and affected tests. Run desktop frontend build,
frontend integration tests, and Rust library tests. Verify public and private
flows in the complete Tauri application and capture screenshots.

## Cutover and scope

The user will remove local Mallard state and cloud data for retesting. This plan
does not authorize or perform that deletion. The implementation was reverted.
No deployment or data reset was performed by this task. Existing unrelated
work must remain intact.

If “append” means permanent selectable private history, revise this plan first.
That requires explicit payload retention, history listing, and restore rules;
the current retired-manifest mechanism does not provide that feature.

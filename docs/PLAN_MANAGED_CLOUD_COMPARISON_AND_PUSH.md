# Mallard Cloud comparison and Push plan

Status: plan only. Implementation changes were reverted at the user's request.
No implementation work is active. This revision uses the existing Cloud API
response and limits implementation to the desktop.

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

## Current behavior

The cloud service uses the same private snapshot replacement process for public
and private repositories. Each committed upload creates a private publication,
advances the private version, and retires the previous private publication.
It does not change the published version. It invalidates open release requests.

The current desktop carries previous session content forward for both
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

### 1. Reuse the existing repository-details response

Use `ManagedCloudClient::get_repository(repository_id)`, which calls
`GET /v1/repositories/{id}` and returns the existing `RepositorySummary`.
Do not change the website, Cloud routes, or response contracts.
Do not add `comparison_head` or `write_head` fields to the API.

Select the source in the desktop adapter from one repository-details response:

| Existing field | Desktop use |
| --- | --- |
| `visibility` | Select public or private comparison |
| `public_head` | Comparison, inventory, Pull, and Push content source for a public repository |
| `private_head` | Comparison and content source for a private repository; Push destination for both types |
| `revision` | Existing repository revision for the write conflict check |
| `private_revision` | Existing private version for the write conflict check |
| `public_summary` / `private_summary` | Match inventory summaries to the selected source |

The selected head already contains the commit ID, manifest revision, digest,
size, and scope. Reuse these fields to fetch and verify the exact manifest.
Keep manifest revision distinct from the public release number.

Use visibility, both heads, and revision values from the same response. Do not
combine a cached public head with newer write revisions. A missing or unreadable
public head must give an explicit absent or unavailable result. Do not use the
private snapshot as a fallback for public comparison.

Keep the existing `/head` response unchanged. Its current request can remain
for the toolbar's release status and permissions. Its writer-selected `head`
must no longer select comparison or Pull content. Separate status reads must
not replace the revision values captured with the selected content source.

Reuse the existing client method and DTO. Add only the desktop logic needed
to select the source and retain the existing write checks. Owner and Admin
draft inspection continues to use the existing service behavior.

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

Carry the observed write values through the existing prepared-review path.
If the desktop IPC needs a token, use one temporary `managed_cloud_review_token`
with the existing repository revision, private revision, and private commit ID.
Do not add a stored token or a new service-side counter.
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

Reuse the existing review warnings and result components. Show unpublished
content that must be selected again to keep it. Do not introduce a separate
review data model or redesign the dialog for this change.

Keep release state separate from session state. A successful upload may show an
upload result, but must not overwrite the comparison result with synced rows.

### 5. Metadata that stays unchanged

- Local and portable schema version numbers: 1.
- Portable bundle manifest format and session identity fields.
- Cloud API endpoints, response fields, and website behavior.
- Cloud database public/private publication pointers and version counters.
- Release approval states and role requirements.
- Credentials and local path data remain machine-local.

The stored local JSON, portable manifests, and Cloud API stay unchanged.
Change the desktop comparison path directly. Add no migration, automatic
backfill, compatibility branch, or legacy execution path.

## Minimal implementation sequence

1. Reuse the existing repository-details call in the desktop Cloud adapter.
   Select `public_head` for public repositories and `private_head` for private
   repositories. Use the matching inventory summary.
2. Use that selected snapshot for comparison, Pull, and Push content. Retain
   `private_head`, `private_revision`, and `revision` from the same response
   for the existing write conflict checks.
3. Keep local metadata unchanged. A public upload must not replace published
   comparison checkpoints. Reuse content verification after a release.
4. Build public Push from published content plus reviewed changes. Reuse the
   existing review warnings for draft replacement. Preserve private Push
   retention and the existing approval checks.
5. Adjust only the necessary status text and Push result. Add focused desktop
   tests. Update the existing Cloud stub to test the current details response;
   do not create a new service contract.

Expected implementation scope:

- `crates/mallard-infrastructure/src/managed_cloud/store.rs`: source selection
  and write conflict inputs.
- `src-tauri/src/project_sync/service.rs`: comparison and publication flow.
- `crates/mallard-infrastructure/src/bundle_engine.rs` and
  `conversation_materialization.rs`: only the changes required for draft
  replacement and preservation of published comparison evidence.
- Existing review/status components and DTOs: only necessary temporary review
  data and text changes.
- Existing desktop unit, integration, and Cloud-stub tests.

No Cloud service implementation, website UI, database, dependency, or version
change is planned. Do not refactor unrelated storage or review code.

## Acceptance tests

| Case | Required result |
| --- | --- |
| Existing repository-details response | Public/private selection works without a new API field |
| Toolbar status read differs from the content read | Status does not replace the content source or its write revision values |
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

Run the desktop frontend build, frontend integration tests, and relevant Rust
library tests. Cover the unchanged repository-details contract in the desktop
Cloud stub. Verify public and private flows in the complete Tauri application.
No service deployment is required by this plan.

## Cutover and scope

The user will remove local Mallard state and cloud data for retesting. This plan
does not authorize or perform that deletion. The implementation was reverted.
No deployment or data reset was performed by this task. Existing unrelated
work must remain intact.

If “append” means permanent selectable private history, revise this plan first.
That requires explicit payload retention, history listing, and restore rules;
the current retired-manifest mechanism does not provide that feature.

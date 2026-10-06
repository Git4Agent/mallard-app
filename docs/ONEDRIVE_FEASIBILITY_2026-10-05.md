# OneDrive support feasibility

Research date: 5 October 2026. Scope: research and an isolated POC.

OneDrive is a feasible storage-provider candidate for current Mallard. This
is an engineering assessment, not live qualification. The proposed first
scope is one Microsoft account across computers. Check personal and work or
school accounts separately. Shared team storage needs a separate access test.

The previous mutable-head blocker no longer applies. Current Mallard uses
immutable publication records. No production code or portable schema change
was made during this research. Live Microsoft Graph tests remain pending.

The follow-up
[shared OneDrive and SharePoint assessment](ONEDRIVE_SHARED_SHAREPOINT_FEASIBILITY_2026-10-05.md)
covers multiple users, restricted library access, guest sign-in, and the
`sharedWithMe` deprecation. It recommends a dedicated SharePoint library for
organization storage and a shared OneDrive folder for personal-account groups.

The existing POC passes 17 local tests. The new probe passes 16 local tests.
All tests use synthetic data and an injected HTTP mock. They do not use real
provider homes, account tokens, or remote storage.

## Current Mallard requirements

The checked source defines these requirements:

- [`ObjectStore`](../crates/mallard-infrastructure/src/storage_core/object_store.rs)
  requires immutable writes, bounded metadata reads, verified payload reads,
  and complete bounded listings.
- [`publication.rs`](../crates/mallard-infrastructure/src/storage_core/publication.rs)
  publishes a record after its objects. Selection uses record names. A claim
  and a record each have one write attempt. An unknown result requires a
  read-only check, with no automatic resend.
- A provider can enable `supports_claims()` only after its server-side
  create-only operation is proved. This operation must reject competing
  bytes in the write request itself. A client-side existence check is
  insufficient. A store can use the existing protocol without claims, with
  its existing displaced-publication behavior. Do not promise equivalent
  arbitration until the claim tests pass.
- Conditional delete is a separate capability. Keep storage clearing
  unavailable until its revision checks are qualified.
- The
  [production registry](../crates/mallard-infrastructure/src/storage_providers/registry.rs)
  contains Local, R2, Dropbox, Feishu, and Mallard Cloud. It has no OneDrive
  adapter. The current provider configuration is extensible. There is no
  need to redesign all storage types or change portable schema 1.

The old
[September POC](ONEDRIVE_USER_APP_FOLDER_POC_2026-09-02.md)
tested stale replacement of a mutable head. Its result cannot establish or
block the current immutable-record protocol.

## Account and permission scope

Microsoft documents App Folder for personal OneDrive and work or school
OneDrive. Delegated `Files.ReadWrite.AppFolder` limits the application to its
folder. `GET /me/drive/special/approot` resolves that folder and creates it
if absent. The data uses the account's storage quota. The user can change or
delete it. App Folder is not an immutable vault.
[Microsoft App Folder guidance](https://learn.microsoft.com/en-us/graph/onedrive-sharepoint-appfolder).

There is a documentation gap. The delegated permission still has a preview
label. Generic upload permission tables list `Files.ReadWrite`, but omit
`Files.ReadWrite.AppFolder`. Test both small PUT and upload sessions with only
the App Folder grant. Do not treat broader-scope success as proof of this
restricted mode.
[Permission reference](https://learn.microsoft.com/en-us/graph/permissions-reference#filesreadwriteappfolder),
[small upload permissions](https://learn.microsoft.com/en-us/graph/api/driveitem-put-content?view=graph-rest-1.0),
[upload-session permissions](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0).

| Proposed mode | Assessment | Required evidence |
| --- | --- | --- |
| Personal account across computers | Best initial scope | App Folder consent, refresh, same namespace, and storage tests |
| Work or school account across computers | Feasible, with tenant restrictions | Independent business-account tests and tenant consent behavior |
| Shared folder or SharePoint library | Separate product scope | Exact shared drive and folder, two users, permissions, and revoked-access behavior |

For shared storage, do not assume each user's `/me/drive/special/approot`
resolves the same location. A later design must pin the shared drive and
folder IDs. Assess explicit shared-file or selected-resource permissions.
The selected-resource permission model needs separate administrative setup.
[Graph permissions](https://learn.microsoft.com/en-us/graph/permissions-reference#filesselectedoperationsselected).

## Sign in design

Use a Microsoft Entra public-client application. Configure the audience for
work or school accounts and personal Microsoft accounts. Use the system
browser, authorization code, PKCE S256, and a registered loopback callback.
A native public client must not embed a client secret. Request `offline_access`
for refresh. This permits direct desktop access without a Mallard-hosted
token service; that is a proposed architecture.
[Account audiences](https://learn.microsoft.com/en-us/entra/identity-platform/single-and-multi-tenant-apps),
[authorization-code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow).

Register the exact callback during app setup. Microsoft documents a manifest
step for an HTTP `127.0.0.1` redirect. Do not copy Dropbox's registration
assumptions. Work-account administrators can restrict user consent even when
a delegated permission does not require administrator consent by default.
[Redirect rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url),
[tenant consent policy](https://learn.microsoft.com/en-us/entra/identity/enterprise-apps/configure-user-consent).

Keep refresh credentials in a machine-local connection document. Persist
the connection reference, account context, drive ID, drive type, and App
Folder ID in local configuration. Never place these in portable manifests.
Test account change, token renewal, revoked consent, and reconnect. Reuse
Mallard's loopback receiver and connection-health model where applicable.

## API mapping

| Mallard operation | Proposed Microsoft Graph operation | Qualification needed |
| --- | --- | --- |
| Resolve storage root | `GET /me/drive/special/approot` | Same application and account resolve the same root |
| Create small object or claim | Path-based `PUT ...:/content?@microsoft.graph.conflictBehavior=fail` | One winner under a race; existing bytes remain unchanged |
| Upload large object | `POST ...:/createUploadSession`, with conflict behavior `fail` | Conflicting final commit, lost response, and verified content |
| Read object | Metadata plus `GET .../content` | Bound size and verify SHA-256 locally |
| List records | Folder `children`, followed through every `@odata.nextLink` | Complete observed cross-client listing; page and item bounds |
| Clear object | `DELETE .../items/{id}` with `If-Match` | Reject a changed revision; no recursive folder deletion |

The DriveItem contract permits `fail`, `replace`, and `rename`; PUT defaults
to `replace`. Send `fail` explicitly. A silent rename would change a
Mallard key. The new probe tests the one-request small-file route because
claims must not depend on an upload-session preparation sequence.
[DriveItem conflict behavior](https://learn.microsoft.com/en-us/graph/api/resources/driveitem?view=graph-rest-1.0#instance-attributes).

Simple upload supports up to 250 MB. Upload sessions support sequential
fragments smaller than 60 MiB, with fragment sizing based on 320 KiB. The
session documentation describes a final `409` if another file takes the
name during upload. Test that create conflict, rather than the former
mutable-head replacement race.
[Small uploads](https://learn.microsoft.com/en-us/graph/api/driveitem-put-content?view=graph-rest-1.0),
[upload sessions](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0).

Downloads can redirect to a short-lived URL. Do not forward the Graph bearer
to that URL. A listing returns direct children; traverse only the required
Mallard prefix and consume all pages. The listing article does not establish
a cross-client snapshot guarantee. A successful finite test is an
observation, not such a guarantee.
[Downloads](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0),
[folder listings](https://learn.microsoft.com/en-us/graph/api/driveitem-list-children?view=graph-rest-1.0).

Graph supports eTag-conditional deletion. Test a stale eTag before enabling
storage clearing. Graph can return `429` with `Retry-After`. Respect the delay
for retryable reads. Do not let a generic SDK retry claims or publication
records; preserve Mallard's one-attempt rule.
[Conditional delete](https://learn.microsoft.com/en-us/graph/api/driveitem-delete?view=graph-rest-1.0),
[throttling](https://learn.microsoft.com/en-us/graph/throttling).

## Path and performance limits

The current
[record-name format](../crates/mallard-domain/src/publication.rs)
uses up to 223 ASCII characters. The longest normal project and Library
record keys are 290 and 293 characters before the storage-root prefix.
OneDrive's cloud path limit is 400 decoded characters. The adapter must
validate the full destination path, including its root. Addressing the root
by ID does not remove the storage path limit.
[Path limits](https://support.microsoft.com/en-us/onedrive/what-are-file-path-length-limits).

Generated record names use lowercase letters, digits, periods, and hyphens.
These avoid the documented reserved characters. Still test the actual
`.mallard` directory, the 223-character record name, both key namespaces,
case aliases, and empty objects. Encode each path segment.
[Graph path rules](https://learn.microsoft.com/en-us/graph/onedrive-addressing-driveitems).

Expected engineering costs are folder traversal, one request per small file,
pagination, and upload-session overhead. No throughput claim was measured.
Use request results within one operation and bounded concurrency. Do not add
a persistent listing cache as a substitute for publication refresh.

## Probe and verification

The new
[`immutable-storage-poc.mjs`](../poc/onedrive-user-app-folder/immutable-storage-poc.mjs)
tests these conditions with two delegated tokens:

1. Both clients resolve the same App Folder and cannot read the drive root.
2. Two different payloads compete for one generated name. Exactly one write
   succeeds; the other returns `409`.
3. The second client verifies the winning bytes and revision.
4. Another write to the same name returns `409` and preserves the winner.
5. A zero-byte object can be stored and read.
6. The second client sees both files through a paginated listing.
7. Cleanup checks known content and uses conditional file deletion.

Local fault tests reject overwrites, missing entries, repeated or redirected
page links, and changed cleanup content. A lost write response is not resent.
The existing credential reader rejects non-private files and symbolic links.
The current live harness does not support Windows credential-file ACL checks.
That is a POC limit, not a OneDrive platform limit.

Run the local suites from the repository root:

```sh
node --test poc/onedrive-user-app-folder/*.test.mjs
```

Result on this research run: 33 tests passed, including the historical 17.
No application build was required because no application source changed.

For the first live run, supply two independently acquired, short-lived
tokens for one dedicated application and account. Use the private credential
file described in the
[probe instructions](../poc/onedrive-user-app-folder/README.md).
The tokens and credentials must remain outside the repository and chat.
The command writes two synthetic files and attempts bounded file cleanup.
The App Folder lookup itself can create the app's folder.

```sh
node poc/onedrive-user-app-folder/immutable-storage-poc.mjs live \
  --credentials /absolute/path/to/onedrive-poc.json
```

No test application or account tokens were supplied in this task. No live
command was run. Distinct tokens do not prove distinct physical devices.

## Implementation sequence

1. Run the small-object probe on personal and business test accounts.
   Resolve the App Folder permission gap before adding setup UI.
2. Extend live qualification to the actual nested key layout, long names,
   large upload-session conflicts, interrupted transfers, quota exhaustion,
   and sustained throttling. Exercise pagination beyond the default page
   size. Test visibility from independently signed-in computers.
3. Add the OneDrive auth and connection modules, Graph `ObjectStore`, and
   storage-provider adapter in `mallard-infrastructure`. Use existing HTTP,
   hashing, URL, temporary-file, and loopback dependencies. Keep transport
   code out of the domain layer.
4. Add the application setup flow, Tauri commands and DTOs, registry entry,
   and a focused OneDrive sign-in component. Keep ProjectSync and Library
   on the shared object-store implementation.
5. Run transport conformance, publication races, unknown-result recovery,
   ProjectSync and Library parity, and native setup tests. Qualify each
   account mode before exposing it as supported.

This is a medium-size provider addition, not a publication-protocol rewrite.
The estimate is qualitative. The unresolved service tests determine the
release scope. Start with one account across computers; qualify shared
storage as a separate mode.

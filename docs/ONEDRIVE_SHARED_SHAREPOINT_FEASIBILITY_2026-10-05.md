# Shared OneDrive and SharePoint support

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

**15 October follow-up:** The isolated
[Microsoft 365 shared-folder POC](../poc/microsoft-shared-folder/README.md)
is now runnable for two-user inspection and opt-in synthetic upload tests.
Its local fixtures validate the harness; live Microsoft behavior and the
wider qualification matrix below remain untested.

Research date: 5 October 2026. Status: documentation and source review.
Live access, uploads, and permission changes were not tested.

Both services are feasible candidates for shared Mallard storage. For a
Microsoft 365 team, use a dedicated SharePoint document library. For a few
people with personal Microsoft accounts, a shared OneDrive folder is the
simpler candidate. These are design recommendations, not qualified support.

SharePoint supports team ownership and administration. A shared OneDrive
folder remains in an individual's drive. This makes SharePoint the better
long-term home for organization data.
[Microsoft collaboration guidance](https://support.microsoft.com/en-us/sharepoint/legacy/collaborating-with-teams-sharepoint-and-onedrive).

| Storage mode | Proposed authorization | Main tradeoff |
| --- | --- | --- |
| Personal OneDrive shared folder | Delegated `Files.ReadWrite` | Simple setup; access extends beyond the chosen folder |
| Work OneDrive shared folder | Delegated `Files.ReadWrite.All`, or an explicitly granted folder scope | Broad consent is simpler; restricted access needs resource setup |
| Dedicated SharePoint library | Delegated `Lists.SelectedOperations.Selected` with a library `write` grant | Restricted access with administrator setup |
| Folder in an existing SharePoint library | Delegated `Files.SelectedOperations.Selected` with a folder `write` grant | Smaller boundary; permission inheritance needs care |
| Dedicated SharePoint site | Delegated `Sites.Selected` with a site `write` grant | Site-wide access; covers more than one folder |

Microsoft explicitly says that personal-account `Files.ReadWrite` includes
files shared with the user. `Files.ReadWrite.All` covers files the user can
access. The Selected scopes above require administrator consent. Do not use
old `Files.ReadWrite.Selected`; it is a separate file-handler permission,
not a scope for direct Graph calls.
[Graph permission reference](https://learn.microsoft.com/en-us/graph/permissions-reference).

Delegated access remains limited by the user's own permissions. It does not
give Mallard access to every file in the organization. Each collaborator
uses their own sign-in and machine-local tokens.
[Delegated access model](https://learn.microsoft.com/en-us/graph/permissions-overview).

## Restricted SharePoint access

Selected scopes support delegated access. Consent alone gives no resource
access. The application also needs an explicit resource grant, and its token
must contain the scope. Effective access is the intersection of the
application grant and the user's permissions. The resource grant names the
application; routine sync can still use a delegated user token.

Grant at one library or storage folder and let children inherit it. Do not
grant every Mallard object separately. Grants below the site level break
permission inheritance and consume unique permission scopes. A lower-level
grant does not authorize reads of parent resources.
[Selected permission model](https://learn.microsoft.com/en-us/graph/permissions-selected-overview).

An authorized setup operator can assign `write` to Mallard using:

```text
Library: POST /sites/{siteId}/lists/{listId}/permissions
Folder:  POST /drives/{driveId}/items/{folderId}/permissions
Site:    POST /sites/{siteId}/permissions
```

These calls grant application access. Users still need their own access.
Provision the grant separately from routine sign-in. Keep grant-management
privileges out of the normal sync token. Current endpoint tables and the
Selected overview differ on some grant-management requirements. Verify the
operator's grant call in the test tenant; do not assume ordinary user consent
can provision it.
[Library grant API](https://learn.microsoft.com/en-us/graph/api/list-post-permissions?view=graph-rest-1.0),
[folder grant API](https://learn.microsoft.com/en-us/graph/api/driveitem-post-permissions?view=graph-rest-1.0),
[site grant API](https://learn.microsoft.com/en-us/graph/api/site-post-permissions?view=graph-rest-1.0).

Use an administrator-provided connection descriptor for a selected library
or folder. Do not depend on searching all sites or listing inaccessible
parents. Site search documents broad read scopes and explicitly excludes
`Sites.Selected` application permission. Its table does not establish
Selected delegated search. Do not add broad access solely for a picker.
[Site search contract](https://learn.microsoft.com/en-us/graph/api/site-search?view=graph-rest-1.0).

## Connecting a shared folder

Proposed flow for broad delegated access:

1. The owner creates a dedicated folder and grants collaborators edit access
   through OneDrive or SharePoint.
2. Each collaborator signs in to Mallard with their own Microsoft account.
3. The collaborator pastes the folder's sharing link and presses Connect.
4. Mallard resolves the link, validates the canonical folder, and reads the
   storage marker. Creating a new storage remains a separate action.
5. Normal storage requests use the saved drive and folder IDs.

Graph resolves a sharing URL through
`GET /shares/{encodedSharingUrl}/driveItem`. The encoding is `u!` followed by
unpadded base64url of the URL. This endpoint documents delegated
`Files.ReadWrite`; successful link resolution does not prove every later
direct write will succeed with that scope. Test the full flow.

`Prefer: redeemSharingLink` can grant durable access. It is an access change,
even though the method is GET. Bind it to the user's Connect action.
`redeemSharingLinkIfNecessary` only guarantees access for that request, so it
does not prove a reusable connection. Treat the original link as sensitive
setup input and keep it out of logs.
[Shared item API](https://learn.microsoft.com/en-us/graph/api/shares-get?view=graph-rest-1.0).

If a shortcut has a `remoteItem` facet, use
`remoteItem.parentReference.driveId` and `remoteItem.id` as the storage
identity. Do not save the shortcut's local ID as the shared root. Proposed
Mallard boundary: resolve shortcuts only at setup; reject remote shortcuts
inside the managed storage namespace.
[Remote item identity](https://learn.microsoft.com/en-us/graph/api/resources/remoteitem?view=graph-rest-1.0).

Do not implement discovery with `/me/drive/sharedWithMe`. Microsoft marks it
deprecated. It operates in a degraded state until November 2026 and then
stops returning data.
[Deprecation notice](https://learn.microsoft.com/en-us/graph/api/drive-sharedwithme?view=graph-rest-1.0).

## SharePoint roots and guests

A SharePoint document library is a Graph drive with
`driveType: documentLibrary`. A site can expose several drives through
`GET /sites/{siteId}/drives`. Do not assume the default drive is the selected
library. Restricted folder access should use its configured root directly.
[Drive types](https://learn.microsoft.com/en-us/graph/api/resources/drive?view=graph-rest-1.0),
[library enumeration](https://learn.microsoft.com/en-us/graph/api/drive-list?view=graph-rest-1.0).

Proposed local configuration keeps `connection_id`, `drive_id`, and
`root_item_id`. Organization storage also records its resource tenant for
sign-in. Site and list IDs can support setup. Both users must resolve the
same root and marker; their connections and credentials differ. Keep these
connection facts out of portable manifests.

For a B2B guest, authenticate against the tenant that owns the resource.
Default sign-in can select the user's home tenant instead. Test two members
of one tenant first, then test a member and guest separately.
[Guest sign-in guidance](https://learn.microsoft.com/en-us/entra/architecture/authenticate-applications-and-users).

External sharing depends on organization and site settings. The more
restrictive setting applies. Browser access alone does not prove Graph
access from Mallard.
[External sharing rules](https://learn.microsoft.com/en-us/sharepoint/external-sharing-overview).

## App Folder as a shared root

Microsoft documents App Folder on SharePoint drives through
`/drives/{driveId}/special/approot`. A shared library App Folder is therefore
a candidate experiment. But `/me/.../approot` resolves each user's drive.
The current probe only tests that user root.

The App Folder article does not establish the complete two-user, guest, and
restricted-upload behavior required here. Keep this candidate separate from
the Selected-scope design until tested.
[App Folder contract](https://learn.microsoft.com/en-us/graph/onedrive-sharepoint-appfolder).

## Required live POC

The checked
[`ObjectStore`](../crates/mallard-infrastructure/src/storage_core/object_store.rs)
and
[`publication`](../crates/mallard-infrastructure/src/storage_core/publication.rs)
contracts still apply. One Graph transport can serve shared OneDrive and
SharePoint roots. Setup, identity, and permissions differ. The portable
bundle format does not need to change.

SharePoint approval settings can hide new files from other users. A
successful upload can therefore leave another Mallard user unable to see its
record. All participants must see new storage objects. If library policy
prevents this, reject that configuration. Do not silently change the policy.
[Approval and visibility](https://support.microsoft.com/en-us/sharepoint/lists/documents-and-library/require-approval-of-items-in-a-list-or-library).

The existing
[immutable POC](../poc/onedrive-user-app-folder/immutable-storage-poc.mjs)
cannot run these cases unchanged. Its
[credential and root validation](../poc/onedrive-user-app-folder/onedrive-app-folder-poc.mjs)
requires App Folder scope, rejects broader file scopes, accepts only personal
or business drives, and requires drive-root access to fail. The prior 33
local tests do not qualify shared users or SharePoint.

The shared POC needs an explicit root, a declared permission mode, and two
independently signed-in users. Use validated account identity evidence.
Different token strings alone do not prove different users. Do not infer
identity by decoding an opaque Graph access token.
[Access token guidance](https://learn.microsoft.com/en-us/entra/identity-platform/access-tokens).

| Test | Required result |
| --- | --- |
| Owner and collaborator resolve the root | Same drive ID, folder ID, and marker |
| A writes; B lists and reads; then reverse | Complete listings and matching SHA-256 digests |
| Two users create different bytes at one claim key | Exactly one accepted create; winning bytes preserved |
| Large upload conflicts and lost final responses | No replacement, silent rename, or automatic record resend |
| Nested keys, long record names, and many pages | Both repository and Library namespaces work |
| Selected-mode request outside the grant | Denied; broad consent must not mask the result |
| Remove user access, then remove the app grant | Each removal stops new Graph access after it takes effect |
| Root rename or cross-drive move | Preserve verified identity or require reconnect |
| Approval, draft visibility, and guest policy | No hidden records or false empty-storage result |
| Cleanup races | Delete only synthetic content with the expected revision |

Test revocation with fresh Graph requests. An issued download URL can remain
valid after file permissions change. Do not promise immediate invalidation
of these URLs.
[Download URL lifetime](https://learn.microsoft.com/en-us/graph/api/resources/driveitem?view=graph-rest-1.0).

Run personal shared-folder, work shared-folder, SharePoint member, and
SharePoint guest cases as separate results. A finite pass observes behavior;
it does not prove a general listing-consistency guarantee.

A new mock cannot resolve these access questions. The useful next POC is a
live run with a dedicated shared root and two test users. This follow-up
changed documentation only. No tenant, sharing permission, or production
adapter was changed.

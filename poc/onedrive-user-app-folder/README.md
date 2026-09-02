# OneDrive user App Folder PoC

## Result

This harness tests whether one OneDrive user App Folder can preserve Mallard's
mutable-head safety contract across two access tokens for the same account.
The two tokens represent two device clients. The operator must establish the
device origin of each token. The harness cannot prove physical device identity.
It does not change Mallard's production schema, storage adapters, commands, or
UI.

The offline protocol tests pass. A real Microsoft Graph run is still required.
Do not add OneDrive as a Mallard storage type from the offline result.

## Decisive live test

The probe creates two random files directly under the calling application's
`approot`. It runs the following sequence on each file:

1. Client A creates a 640 KiB baseline file and records eTag E0.
2. Client A creates an update upload session with `If-Match: E0`.
3. Client A uploads the first 320 KiB range. The range is not final.
4. Client B replaces the file.
5. Client A sends its final 320 KiB range.
6. The final range must return `404`, `409`, or `412`.
7. A metadata read and byte download must retain Client B's eTag and bytes.
8. A new upload session with E0 must return `412`.

Client B uses an `If-Match` upload session in the first case. Client B uses the
small-file `/content` PUT in the second case. Acceptance of either stale final
range is a hard failure. An unrelated error, a transport timeout, or a read-back
mismatch is also a failure. Such a result does not prove compare-and-swap.

## Other live checks

The probe also checks these conditions:

- Both authorization results report the delegated
  `Files.ReadWrite.AppFolder` scope.
- Neither authorization result reports another `Files.*` or `Sites.*` scope.
- A read outside App Folder must return `403` for both tokens.
- The access tokens are distinct.
- Both authorizations resolve the same drive ID, App Folder item ID, and drive
  type.
- Client B can read the metadata and content that Client A creates.
- Upload and download URLs receive no `Authorization` header.
- Downloads match the expected size, SHA-256 digest, eTag, and bytes.
- Cleanup deletes only known files whose current content matches one of the
  test payloads. It uses the current eTag.
- Cleanup does not delete a directory or unrelated App Folder content.
- Upload-session cancellation must return `204` or a status read must prove
  that the session is absent.

The `granted_scopes` value must be copied from each authorization result. The
harness does not decode Microsoft Graph access tokens. Microsoft says that
clients must treat access tokens as opaque. The reported scope plus the
outside-root denial form the permission probe. They are not cryptographic proof
of the token grant. Use a dedicated application and retain the external
authorization evidence.

## Microsoft application setup

Use a dedicated Microsoft Entra test application. Configure it as a public
client. Do not create or embed a client secret. Use the authorization-code flow
with PKCE to acquire delegated tokens.

Request these scopes:

```text
openid
profile
offline_access
https://graph.microsoft.com/Files.ReadWrite.AppFolder
```

Do not request another `Files.*` or `Sites.*` scope for this test. Additional
file or site permission would invalidate the least-privilege result.

Acquire two short-lived access tokens through independent authorization
sessions. Use the same application client ID, Microsoft account, and tenant
context. For the required two-device test, acquire one result on each device.
Copy only the short-lived access tokens and the returned granted-scope strings
to the credential file on the machine that runs the probe. Distinct token text
does not by itself prove that two devices supplied the tokens.

The harness does not use refresh tokens. Revoke the test grants or let the
short-lived access tokens expire after the run.

## Credential file

Create the credential file outside the repository:

```json
{
  "client_a": {
    "access_token": "<FIRST_SHORT_LIVED_ACCESS_TOKEN>",
    "granted_scopes": "openid profile Files.ReadWrite.AppFolder"
  },
  "client_b": {
    "access_token": "<SECOND_SHORT_LIVED_ACCESS_TOKEN>",
    "granted_scopes": "openid profile Files.ReadWrite.AppFolder"
  }
}
```

Use the exact `scope` value returned with each access token. The value can use
the fully qualified Graph scope name.

Set private file permissions:

```sh
chmod 600 /absolute/path/to/onedrive-poc.json
```

The reader rejects relative paths, symbolic links, broad file permissions,
unknown JSON fields, duplicate tokens, invalid scope sets, oversized files, and
files that change during the read. The live harness fails as unsupported on
Windows because it does not verify Windows file ACLs.

Do not send this file in chat. Do not place it in the repository. Delete it
after the live test.

## Run offline

The mock suite makes no network request:

```sh
node --test poc/onedrive-user-app-folder/onedrive-app-folder-poc.test.mjs
```

## Run against Microsoft Graph

```sh
node poc/onedrive-user-app-folder/onedrive-app-folder-poc.mjs live \
  --credentials /absolute/path/to/onedrive-poc.json
```

A successful result has this form:

```json
{
  "verdict": "PASS",
  "driveType": "personal",
  "permissionProbe": "reported_scope_and_outside_root_denial",
  "sameAccountNamespace": true,
  "distinctAccessTokens": true,
  "finalCommitCas": {
    "upload_session": "stale_final_rejected",
    "simple_upload": "stale_final_rejected"
  },
  "cleanup": {
    "status": "complete"
  },
  "stages": [
    "app_folder_access_and_scope_isolation",
    "same_account_namespace_two_tokens",
    "stale_final_rejected_after_session_replace",
    "stale_final_rejected_after_simple_replace"
  ]
}
```

Normal output excludes tokens, granted scopes, account IDs, drive IDs, item
IDs, eTags, remote paths, upload URLs, download URLs, response bodies, request
IDs, and error messages.

If cleanup cannot complete, the result contains `cleanup.status: "required"`
and the generated probe ID. Inspect only these exact generated file names:

```text
mallard-onedrive-poc-<probe-id>-session.bin
mallard-onedrive-poc-<probe-id>-simple.bin
```

Do not delete the App Folder or use an unbounded or recursive deletion command.

## Required test matrix

Run the live probe separately for:

- OneDrive for home (`driveType: personal`); and
- OneDrive for work or school (`driveType: business`).

One result does not approve the other account mode. Tenant consent policy,
Conditional Access, OneDrive provisioning, and service behavior can differ.

This PoC does not test a SharePoint document library. That mode needs a
separate selected-drive and team-access PoC.

## Documentation gap

Microsoft's App Folder article says that the folder supports upload, list, and
sharing operations with `Files.ReadWrite.AppFolder`. The current generic
upload-session and small-upload permission tables do not list that scope. The
permissions reference also marks the delegated App Folder permission as
preview.

The upload-session document defines `If-Match` when the session starts. It does
not state that automatic final-range commit rechecks that eTag. The small-file
upload document does not list conditional headers. The live permission and
concurrency checks are therefore release gates, not optional smoke tests.

## Limits

This is a provider capability probe. It does not test:

- OAuth UI, PKCE implementation, refresh, revocation, or reconnect;
- validated ID-token account identity;
- automatic proof that each token came from a different physical device;
- operating-system credential storage;
- managed-tenant approval or Conditional Access remediation;
- quota exhaustion, sustained throttling, or files larger than 640 KiB;
- complete ambiguous-write recovery outside the tested setup-file case;
- directory listing or pagination behavior;
- zero-byte and maximum-path objects;
- a Rust `ObjectStore` adapter;
- storage-marker, BundleEngine, Library, Push, or Pull parity; or
- packaged macOS, Windows, or Linux behavior.

## Official references

- [Using App Folder in OneDrive and SharePoint](https://learn.microsoft.com/en-us/graph/onedrive-sharepoint-appfolder)
- [Files.ReadWrite.AppFolder permission](https://learn.microsoft.com/en-us/graph/permissions-reference#filesreadwriteappfolder)
- [Get a special folder](https://learn.microsoft.com/en-us/graph/api/drive-get-specialfolder?view=graph-rest-1.0)
- [Create an upload session](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0)
- [Upload or replace small-file content](https://learn.microsoft.com/en-us/graph/api/driveitem-put-content?view=graph-rest-1.0)
- [Download DriveItem content](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0)
- [Delete a DriveItem](https://learn.microsoft.com/en-us/graph/api/driveitem-delete?view=graph-rest-1.0)
- [Microsoft identity access tokens](https://learn.microsoft.com/en-us/entra/identity-platform/access-tokens)
- [Authorization-code flow with PKCE](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)

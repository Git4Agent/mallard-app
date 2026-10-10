# OneDrive user App Folder proof of concept

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

**5 October update:** This document tests the former mutable-head protocol.
Use the [current OneDrive assessment](ONEDRIVE_FEASIBILITY_2026-10-05.md) for
today's immutable-record requirements and the new create-only probe.
The stale update-session test below is no longer a product blocker.

**Date:** 2026-09-02  
**Scope:** Same-account, two-client OneDrive user App Folder capability gate  
**Production status:** Not integrated  
**Offline status:** Pass, 17 tests  
**Live Microsoft Graph status:** Not run; two independent delegated test
tokens for one account and application are required

## Decision

The offline harness implements the required fail-closed test. It does not prove
the OneDrive service behavior. Keep OneDrive user App Folder support blocked
until the live probe passes separately on the selected account modes.

The isolated harness is in
[`poc/onedrive-user-app-folder`](../poc/onedrive-user-app-folder/README.md).
It does not change `StorageConfig`, the `ObjectStore` dispatch, Tauri commands,
frontend DTOs, the storage UI, or portable schema 1.

## Research result

Microsoft documents App Folder as a least-privilege, application-specific
folder that works across a user's devices. The delegated permission is
`Files.ReadWrite.AppFolder`. Microsoft currently marks that delegated
permission as preview.

The current documentation does not prove Mallard's compare-and-swap contract:

- Upload-session creation supports `If-Match` and returns `412` for a mismatch.
- Range PUT requests use a preauthenticated upload URL and do not carry the
  original `If-Match` header.
- The final range automatically commits the file when `deferCommit` is false.
- Microsoft documents a final name conflict. It does not state that an update
  session rechecks its starting eTag after another client replaces the item.
- The small-file `/content` PUT request-header table lists authorization and
  content type. It does not list `If-Match`.

The permission documents also conflict. The dedicated App Folder article says
that upload and list operations work with the App Folder permission. Current
generic method permission tables often list only broader file permissions.
This creates two independent gates: effective permission and commit-time
concurrency.

## Implemented live sequence

The harness accepts two distinct short-lived delegated access tokens. The
operator must copy the granted `scope` string from each authorization result.
The harness requires `Files.ReadWrite.AppFolder` and rejects every other
`Files.*` or `Sites.*` scope. For example, it rejects `Files.ReadWrite`,
`Files.ReadWrite.All`, and `Sites.ReadWrite.All`. It also calls
`GET /me/drive/root?$select=id` with each token. That request must return `403`.
This negative functional check detects broader effective file access.

The harness calls `GET /me/drive/special/approot` with each token. It requires
the same drive ID, App Folder item ID, and drive type. It creates two random
files directly under that root. Client B reads the metadata and exact content
that Client A creates. This is the functional same-namespace check. It does not
prove human account identity, authorization-session independence, or physical
device identity. The operator must acquire one token on each device and retain
that external evidence.

The decisive race runs twice:

| Case | Client A | Client B | Required final state |
| --- | --- | --- | --- |
| Session versus session | Start update session with E0; upload bytes `0-327679` of a 655,360-byte body | Start another update session with E0; commit B's full body | A's range `327680-655359` fails; metadata and download retain B |
| Session versus simple upload | Start update session with E0; upload bytes `0-327679` of a 655,360-byte body | Replace through `/content` PUT | A's range `327680-655359` fails; metadata and download retain B |

The harness accepts only `404`, `409`, or `412` as the stale final-range
rejection. It treats any `2xx` result as a hard failure. It also treats a
transport error, unrelated error, changed item ID, unchanged winner eTag,
unexpected size, digest mismatch, or byte mismatch as failure.

After the stale final request, the harness creates a new update session with
E0. That request must return `412`. This separate check proves the documented
session-start precondition while the late-final check tests the undocumented
boundary.

## Safety controls

The PoC follows the existing live-provider harness boundary:

- Tokens come only from an opt-in external credential file.
- The credential path must be absolute, canonical, private, regular, and not a
  symbolic link.
- The reader uses `O_NOFOLLOW` where the platform supplies it and detects a
  file change during the read.
- The Graph host is fixed to `https://graph.microsoft.com/v1.0`.
- Preauthenticated upload and download URLs must use HTTPS.
- Preauthenticated URLs never receive the Graph bearer token.
- Requests, JSON, downloads, item counts, page counts, retries, and timeouts
  are bounded.
- Normal output contains only fixed classifications and boolean results.
- Downloads are verified with local SHA-256 and exact byte comparison.
- Probe file names are recorded before the first write. This permits cleanup
  after a lost successful response.
- A known file is deleted only when its current content matches a test payload.
  Cleanup uses the exact item ID and current eTag.
- Cleanup never deletes a directory or unrelated App Folder content.
- Upload-session cancellation accepts `204`, `404`, or `410`. For another
  result, a status read must prove `404` or `410`. An unresolved session makes
  cleanup required.

If cleanup cannot complete, the output includes only the generated probe ID and
a fixed reason. This permits manual inspection without printing a token, item
ID, eTag, URL, provider response, or request ID.

## Offline verification

Command:

```text
node --test poc/onedrive-user-app-folder/onedrive-app-folder-poc.test.mjs
```

Result:

```text
tests 17
pass 17
fail 0
```

The mock transport covers:

1. Non-production status.
2. Required App Folder scope and rejected additional file or site scopes.
3. Two distinct access tokens.
4. Private canonical credential-file reads.
5. Symbolic-link rejection.
6. Both successful fail-closed cross-mode races.
7. Detection of a stale overwrite after a session winner.
8. Detection of a stale overwrite after a simple-upload winner.
9. Rejection of an unrelated final-range service error as CAS evidence.
10. Cleanup after a lost successful setup-file response.
11. Rejection of different App Folder roots.
12. Sanitized permission denial.
13. Rejection when an alleged narrow token can read outside App Folder.
14. Failure when upload-session cancellation cannot be proved.
15. Non-destructive handling of unrelated App Folder content.
16. Non-destructive handling of unexpected content in a known file.
17. Absence of authorization on upload URLs.

The passing mock is not provider evidence. The two negative stale-write mocks
are the main harness test: each deliberately lets Client A overwrite B, and the
probe returns `FAIL`.

## Required live evidence

Follow the PoC
[`README`](../poc/onedrive-user-app-folder/README.md). Use one dedicated public
client application with delegated `Files.ReadWrite.AppFolder` only. Acquire one
token on each of two devices for the same account and application. The harness
checks that the tokens differ. Device provenance remains an external
precondition.

A usable result must have all of these fields:

- `verdict: "PASS"`
- `permissionProbe: "reported_scope_and_outside_root_denial"`
- `sameAccountNamespace: true`
- `distinctAccessTokens: true`
- both `finalCommitCas` values set to `stale_final_rejected`
- `cleanup.status: "complete"`

Run this gate separately on OneDrive for home and OneDrive for work or school.
Record the returned `driveType`, Microsoft application configuration, requested
scope, tenant consent state, date, and sanitized result. Do not record tokens,
account IDs, drive IDs, item IDs, eTags, or preauthenticated URLs.

A pass for `personal` does not approve `business`. A pass for either user-drive
mode does not approve a SharePoint document library.

## Production boundary after live passes

Live passes would establish the narrow provider behavior needed for a user App
Folder adapter. They would not complete production integration. That work still
requires:

1. A tagged machine-local OneDrive destination model and authorization
   profile.
2. Authorization code with PKCE in the system browser.
3. Validated ID-token identity for stable account and tenant binding.
4. Refresh-token rotation and operating-system credential storage.
5. A direct-HTTP Rust adapter with bounded streaming and error redaction.
6. The provider-neutral conformance suite, including absent creation,
   ambiguity recovery, listing, deletion, quota, throttle, and path tests.
7. ProjectSync and storage Library parity tests.
8. Separate SharePoint drive selection, consent, and two-user tests for team
   storage.

Portable bundle schema 1 and the closed `.mallard/v1` key model need no change
for this PoC.

## Official sources

- [Using App Folder in OneDrive and SharePoint](https://learn.microsoft.com/en-us/graph/onedrive-sharepoint-appfolder)
- [Files.ReadWrite.AppFolder permission](https://learn.microsoft.com/en-us/graph/permissions-reference#filesreadwriteappfolder)
- [Get a special folder](https://learn.microsoft.com/en-us/graph/api/drive-get-specialfolder?view=graph-rest-1.0)
- [Create an upload session](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0)
- [Upload or replace DriveItem content](https://learn.microsoft.com/en-us/graph/api/driveitem-put-content?view=graph-rest-1.0)
- [Download DriveItem content](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0)
- [Delete a DriveItem](https://learn.microsoft.com/en-us/graph/api/driveitem-delete?view=graph-rest-1.0)
- [OneDrive error responses](https://learn.microsoft.com/en-us/onedrive/developer/rest-api/concepts/errors?view=odsp-graph-online)
- [Microsoft identity access tokens](https://learn.microsoft.com/en-us/entra/identity-platform/access-tokens)
- [Authorization-code flow with PKCE](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)

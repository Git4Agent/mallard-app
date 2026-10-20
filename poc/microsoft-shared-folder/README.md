# Microsoft 365 shared-folder POC

Prepared on 15 October 2026. **Standalone POC only; live Microsoft access has
not been tested.** The local tests use the real MSAL library with synthetic
identity responses and a stateful Graph fixture. They validate the probe,
not Microsoft's service behavior.

This replaces the runnable part of the removed October 5 authentication
experiment for this specific investigation. It adds the missing two-user
shared-folder checks. It does not restore the old App Folder or mutable-head
experiments. Mallard's application, storage providers, schemas, and root
dependencies do not import this directory.

## What runs

| Mode | Behavior | Meaning of `PASS` |
| --- | --- | --- |
| Default inspect | Two browser sign-ins, forced renewal, identity verification, shared-link resolution, direct folder access, bounded listings | Both users could read the same canonical folder during this run |
| `--write-probe` | Inspect plus synthetic uploads, SHA-256 verification by the other user, two competing upload sessions, rejection of an existing filename, and paginated listings by both users | The tested transport operations behaved as required in this run |

Both modes request delegated **`User.Read` and `Files.ReadWrite.All`**.
Inspect limits Graph calls to GETs; it still requests write consent so the
same application registration and permissions can be exercised before the
write run. This is broad delegated file access within each user's rights,
not folder-scoped consent. The POC does not request site-search permissions.

The scope is the public Microsoft cloud, a SharePoint document-library folder
or a work OneDrive shared folder hosted on `*.sharepoint.com`, and two users
in the resource tenant. Start with two tenant members. A guest run is a
separate experiment and requires access in that same resource tenant.
Personal OneDrive and Selected permissions are outside this POC.

## Prepare Microsoft

1. In the [Entra admin center](https://entra.microsoft.com/), create a test
   app registration in the tenant that owns the folder. A single-tenant
   registration is sufficient for the first two-member test.
2. Under Authentication, add a **Mobile and desktop applications** redirect:
   `http://localhost/microsoft-shared-folder/callback`.
   Keep the path exact. The probe uses an ephemeral localhost port, which
   Microsoft ignores when matching a registered localhost redirect.
   [Redirect URI rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url).
3. Add Microsoft Graph **delegated** `User.Read` and `Files.ReadWrite.All`.
   Obtain any consent or assignment required by the tenant. Do not create a
   client secret or enable implicit or password flows.
   [Permission reference](https://learn.microsoft.com/en-us/graph/permissions-reference),
   [user consent policy](https://learn.microsoft.com/en-us/entra/identity/enterprise-apps/configure-user-consent).
4. Create a dedicated, empty test folder in the chosen SharePoint library.
   Give both test users edit access and copy its folder-sharing link. Each
   user should open the link in Microsoft's UI first, accepting any sharing
   invitation there. Do not point the write probe at existing Mallard storage.
5. Keep the public application client ID, resource tenant UUID, and folder
   link in the local config below. Tokens and client secrets are not config
   fields. The sharing link is sensitive setup data even though the IDs are
   not credentials.

Selecting a folder does not narrow OAuth permissions. The POC does not
provision a tenant, license, application grant, sharing invitation, or
library policy. It also does not send `Prefer: redeemSharingLink`, which can
grant durable access even on a GET. Existing direct access is required.
[Shared-link API](https://learn.microsoft.com/en-us/graph/api/shares-get?view=graph-rest-1.0).

## Install and run

Use Node 22 or newer. In this directory:

```sh
npm ci --ignore-scripts
npm test
cp config.example.json config.local.json
chmod 600 config.local.json
```

Edit `config.local.json` with the three setup values. On Windows, use a
user-private directory and ACL; the POSIX mode check is skipped. Local
`*.local.json` files and `node_modules` are Git-ignored. Config must be a
regular file, not a symlink, at most 16 KiB, with no extra fields.

First inspect access:

```sh
npm start -- --config ./config.local.json
```

Complete the first browser sign-in as user A and the second as user B.
Each has a three-minute deadline. Choose a different account when prompted;
two sessions or tokens for the same user are rejected. Microsoft can require
MFA, consent, or administrator involvement. The probe verifies the MSAL
account against Graph `/me` before and after forced renewal. It does not
decode Graph access tokens to infer identity.

Then, when ready to create synthetic test files:

```sh
npm start -- --config ./config.local.json --write-probe
```

Each invocation starts fresh sign-ins. A successful write run leaves exactly
three synthetic files with one random `mallard-ms-poc-<run-id>-` prefix:

- `a.bin`: A uploads; B reads and verifies SHA-256.
- `b.bin`: B uploads; A reads and verifies SHA-256.
- `claim.bin`: two 640 KiB payloads compete for one filename; exactly one
  final upload must succeed and the other must return HTTP 409. Both users
  verify the winner. Another create at that name must return 409, with the
  winning bytes and revision unchanged.

Both competing sessions are opened before either commits, and each uploads
its first 320 KiB before the final ranges run concurrently. Upload sessions
explicitly request `conflictBehavior: fail` and automatic final-range commit.
The probe never uses the default replace behavior or accepts a renamed file.
[Upload-session contract](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0).

Both users then list all probe files through every returned page. The request
asks for one item per page; if the service does not paginate, that gate fails
as unexercised. A pass is a finite observation, not a general listing
consistency guarantee.

## Output and boundaries

Stdout contains fixed result fields, counts, and the synthetic probe prefix.
Progress goes to stderr. Failure exits with status 1. `productionReady` is
always `false`; an inspect pass does not prove write access. Error responses,
account IDs, folder IDs, links, tokens, and signed transfer URLs are never
printed or persisted by the POC.

The MSAL Node dependency is pinned to
[7.1.0](https://github.com/AzureAD/microsoft-authentication-library-for-js/releases/tag/msal-node-v7.1.0).
MSAL owns PKCE generation, token exchange, nonce checking, renewal, and its
in-memory cache. The POC retains a bounded, one-use form-POST listener for
the exact callback path, Host and state checks, and the overall sign-in
deadline. MSAL's bundled interactive listener does not expose these bounds.
The code clears both token caches when the probe ends; this does not revoke
Microsoft consent or sign the browser out.

Graph and identity hosts are fixed. Pagination must stay in the same folder.
Responses, downloads, pages, item counts, and requests are bounded; redirects
are rejected. The Graph bearer is never sent to preauthenticated transfer
URLs. Those URLs must be HTTPS on `*.sharepoint.com` or `*.1drv.com` and are
used immediately. A different Microsoft transfer domain fails explicitly
and needs investigation, rather than silently broadening the allowlist.
[Download contract](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0).

No write is retried. A lost or rejected upload response fails the run. When
a filename is pending, the probe makes one read-only metadata check and
reports whether an item was observed, without treating that observation as
a successful write. An absent item does not establish that the server will
never commit it. Local tests cover loss after a successful commit; this is
not a live fault-injection result.

There is **no automated deletion or upload-session cancellation**. Three
files remain after a successful write run, and a failed run can leave fewer,
unexpected conflict copies, or pending sessions. The result's synthetic
prefix helps locate that run in the dedicated test folder. Inspect the
remote result before manually cleaning up. The probe never writes `.mallard`
markers, publication records, or actual claims.

## Verification and remaining gates

Local verification on 15 October 2026: `npm test` passed 32 tests. The suite
covers real-MSAL PKCE and nonce behavior, callback validation and timeout,
renewal, distinct users, canonical shared roots, request boundaries,
cross-user digests, concurrent creation, replacement and rename failures,
missing listings, pagination escape/loops/limits, lost responses, private
config files, and sanitized CLI output.

Live Microsoft sign-in, consent, shared access, and upload behavior remain
**not run**. The next step is to fill the local config and run inspect with
two tenant members, then the write probe in the dedicated folder. Keep those
results separate from the local fixture results.

Production qualification would additionally need Selected-scope grants and
outside-grant denial, guest and revocation behavior, rename/move handling,
approval/draft visibility policies, actual repository and Library paths,
long record names, larger uploads, zero-byte objects, and durable handling
of ambiguous publication outcomes. These remain outside this runnable POC;
the [shared-storage research matrix](../../docs/ONEDRIVE_SHARED_SHAREPOINT_FEASIBILITY_2026-10-05.md)
records that wider investigation.

# Browser sign-in for SharePoint

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

**15 October follow-up:** A new isolated
[Microsoft 365 shared-folder POC](../poc/microsoft-shared-folder/README.md)
prepares two-user browser sign-in, shared-root inspection, and opt-in
synthetic upload checks. Live Microsoft validation remains pending. The
October 5 experiment below remains historical.

Research date: 5 October 2026.

Yes. Mallard can offer **Connect SharePoint → Microsoft sign-in → choose a
destination → review**. End users need no app ID, client secret, token, or
authorization code fields. Microsoft may still require a password, MFA,
consent, or an administrator. This is setup without manual credential
fields; it is not a promise of passwordless sign-in.

## Comparison with Dropbox

The current
[`dropbox_auth.rs`](../crates/mallard-infrastructure/src/dropbox_auth.rs)
contains a public app key, S256 PKCE, a loopback callback, and refresh-token
renewal. [`DropboxSetup.tsx`](../src/components/project-sync/DropboxSetup.tsx)
then confirms the account, finds or creates a destination, and prepares a
reviewable draft. SharePoint can follow that product flow.

Microsoft supports authorization code with PKCE for public desktop clients.
The app ID can ship with the application. A desktop client must not depend
on a secret embedded in its binary. One Mallard registration can support
multiple organizations.
[Public client model](https://learn.microsoft.com/en-us/entra/msal/msal-client-applications),
[authorization code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow).

| Concern | Dropbox today | Proposed SharePoint flow |
| --- | --- | --- |
| App identity | Public key in Mallard | Public client ID in Mallard |
| Sign-in | Browser and PKCE | Browser and PKCE |
| Token entry | None | None |
| Renewal | Local refresh chain | Local account-bound token cache |
| Destination | Dropbox namespace discovery | Site/library picker or preconfigured root |
| Organization approval | Provider and account policy | Entra consent, assignment, and tenant policy |

## Consent choices

| Mode | Delegated permissions | Consequence |
| --- | --- | --- |
| Read-only POC | `User.Read`, `Sites.Read.All` | Can verify identity and discover site/library candidates |
| General sync candidate | Add `Files.ReadWrite.All` | Can request file access across resources the user can access |
| Restricted library | `Lists.SelectedOperations.Selected` plus library grant | Needs administrator consent and a resource grant |

The current Graph reference marks delegated `Sites.Read.All` and
`Files.ReadWrite.All` as **AdminConsentRequired: No**. It marks delegated
`Lists.SelectedOperations.Selected` as **Yes**. Broad delegated file access
still depends on the signed-in user's rights. These are different from
application permissions, which run without a user.
[Permission reference](https://learn.microsoft.com/en-us/graph/permissions-reference).

A tenant can restrict user consent to approved permissions or verified
publishers, disable it, or require user assignment. Thus Mallard cannot
promise administrator-free setup in every organization. Publisher
verification helps distribution but does not override tenant policy.
[User consent policy](https://learn.microsoft.com/en-us/entra/identity/enterprise-apps/configure-user-consent),
[publisher verification](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview).

Selected access requires consent, an explicit application grant on the
resource, and the user's own permission. Selecting a folder in a picker
does not create this resource grant. An organization can distribute a
connection descriptor so users do not type tenant, drive, or folder IDs.
That descriptor's design is separate from the authentication POC.
[Selected permission model](https://learn.microsoft.com/en-us/graph/permissions-selected-overview).

## Destination selection

Microsoft File Picker v8 supplies a hosted picker and returns item and drive
IDs. It operates with delegated access. Its setup also involves SharePoint
resource permissions and token requests, so it is more work than a Graph
sign-in alone. Evaluate it if a familiar Microsoft picker is required.
[File Picker v8](https://learn.microsoft.com/en-us/onedrive/developer/controls/file-pickers/).

The smaller alternative is a Mallard site/library picker using Graph. Site
search requires `Sites.Read.All` or higher delegated permission. Treat its
results as candidates; do not infer a complete inventory or an access grant
from search alone. Avoid a design that requires typing a site URL for the
normal member-account flow.
[Site search](https://learn.microsoft.com/en-us/graph/api/site-search?view=graph-rest-1.0).

Do not use `sharedWithMe`; Microsoft has deprecated it. Also do not rely on
`followedSites` for correctness: Microsoft documents incorrect results as a
known issue. Neither API is a sound basis for declaring that storage is
absent.
[Shared item discovery notice](https://learn.microsoft.com/en-us/graph/api/drive-sharedwithme?view=graph-rest-1.0),
[followed sites limitation](https://learn.microsoft.com/en-us/graph/api/sites-list-followed?view=graph-rest-1.0).

For guests, use the resource tenant for sign-in. Registering an Entra app
does not itself provision SharePoint or grant access to a library.
The live test needs a tenant with working SharePoint access.
[Guest sign-in](https://learn.microsoft.com/en-us/entra/architecture/authenticate-applications-and-users).

## POC and evidence

The isolated [POC](../poc/sharepoint-browser-auth/README.md) uses MSAL Node
7.0.1, a Microsoft release published on 2 October 2026. It runs outside the
production application. The registration guide includes the exact native
callback `http://localhost/sharepoint/callback`; Entra ignores the port when
matching localhost redirects.
[MSAL release](https://github.com/AzureAD/microsoft-authentication-library-for-js/releases/tag/msal-node-v7.0.1),
[redirect matching](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url).

The probe receives the code through a form POST, validates state, and lets
MSAL generate PKCE and perform code exchange and nonce validation. It then
forces silent renewal, verifies the same Graph user, and reads bounded
site/library candidate lists. MSAL keeps tokens in memory. The final output
contains only fixed status fields and counts. No Graph writes are sent.

Validation: **16 local tests passed**. They use the real MSAL library with
synthetic identity responses and loopback HTTP. They verify the local
protocol and request boundaries. They do not prove Microsoft consent,
licensing, tenant policy, or live SharePoint access.

The first live gate is sign-in, forced renewal, and read-only discovery with
the new app registration. The next gate is a two-user shared-storage test.
That test must cover immutable writes, verified reads, listing visibility,
revocation, and library approval settings before a production adapter is
added. See the
[shared-storage test matrix](ONEDRIVE_SHARED_SHAREPOINT_FEASIBILITY_2026-10-05.md).

No production provider, schema, Tauri command, or storage UI was changed.

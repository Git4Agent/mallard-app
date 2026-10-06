# SharePoint browser sign-in POC

Status: 16 local tests passed on 5 October 2026. Live Microsoft sign-in and
SharePoint access remain untested.

This probe tests a desktop public client. Users complete Microsoft sign-in
in a browser. They do not copy tokens, secrets, or authorization codes. The
public app ID is developer configuration; a production Mallard build would
include it, as the current Dropbox build includes its public app key.

The probe uses MSAL Node 7.0.1. It runs outside Mallard. It adds no production
provider, frontend command, or runtime dependency.

## Register the test application

1. Open the [Entra admin center](https://entra.microsoft.com/).
2. Open **Entra ID → App registrations → New registration**.
3. Use **Mallard** as the name.
4. Select **Multiple Entra ID tenants → Allow all tenants**. This is the
   current portal label for work and school accounts in other tenants.
5. Select **Public client/native (mobile & desktop)**. Set the redirect URI
   to `http://localhost/sharepoint/callback`.
6. Register the application. Microsoft requires acceptance of its platform
   policies at this step.
7. In **API permissions**, add Microsoft Graph **delegated** `Sites.Read.All`.
   Keep delegated `User.Read`.
8. Copy the **Application (client) ID** from Overview. It is public.
9. Use the existing Mallard icon under **Branding & properties** if needed.
   The repository's `src-tauri/icons/128x128@2x.png` is a square PNG.

Do not create a client secret. Do not enable implicit token grants or
password-based flows. The registered native redirect identifies this as a
public client. The POC uses authorization code with PKCE.

For a single-tenant test registration, pass that tenant's UUID with
`--tenant`. For a guest account, use the UUID of the resource tenant. The
default `organizations` authority is for the multitenant member-account
case. The POC supports the public Microsoft cloud only.

## Run

Use Node 22 or later. From this directory:

```sh
npm ci --ignore-scripts
npm test
node browser-auth.mjs --client-id PUBLIC_APP_ID
```

The probe opens the system browser. Finish sign-in within three minutes.
Microsoft can require a password, MFA, or administrator consent. No manual
credential entry is required in the probe itself.

To test a specific tenant:

```sh
node browser-auth.mjs --client-id PUBLIC_APP_ID --tenant RESOURCE_TENANT_ID
```

The default mode requests only `User.Read` and `Sites.Read.All`. To test the
future sync consent screen, first configure delegated `Files.ReadWrite.All`
on the registration, then run:

```sh
node browser-auth.mjs --client-id PUBLIC_APP_ID --mode sync-consent
```

That mode requests write permission but still sends no Graph writes. It does
not prove upload access or qualify a storage provider.

## Probe behavior

1. Receive a bounded, one-use loopback callback with state validation.
2. Exchange the code once through MSAL, with PKCE and nonce validation.
3. Call Graph `/me` to verify the access token can read the signed-in user.
4. Force silent renewal through MSAL. Verify the account and tenant stay the
   same and `/me` still succeeds.
5. Search site candidates and enumerate document libraries in at most three
   returned sites. Bound each listing to 20 pages and 1,000 items.
6. Print counts and fixed status fields. Clear the in-memory MSAL cache.

Search results are candidates, not a complete inventory. No folder picker,
storage marker read, remote write, sharing change, or connection save is
performed. An empty search result does not prove the account has no sites.

No token, user name, account ID, site ID, or remote URL is printed or saved.
The callback accepts form POSTs so the code does not appear in its URL.
Identity and Graph responses are bounded. Redirects and foreign pagination
URLs are rejected. SDK internal retries are disabled. Clearing the cache
does not revoke the consent granted at Microsoft.

The local tests use the real MSAL SDK with synthetic identity responses.
They cover PKCE, nonce failure, renewal, callback rejection, timeout,
pagination boundaries, safe output, and cache cleanup. They do not test a
real Microsoft account, tenant policy, SharePoint license, or live listing
behavior.

See the [research and design comparison](../../docs/SHAREPOINT_BROWSER_AUTH_POC_2026-10-05.md).

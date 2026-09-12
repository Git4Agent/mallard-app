# Existing storage discovery POC

Mallard can discover existing storage in R2 and Dropbox. Both live checks
passed on 2026-09-10. This POC is separate from the application.

See the [Claude handoff](../../docs/POC_EXISTING_STORAGE_DISCOVERY_FOR_CLAUDE.md)
for verified results, code references, and proposed app integration steps.

The current app accepts a known R2 destination or Dropbox namespace. Its
agent setup creates new storage. The CLI's `storage list` command lists
local connections. It does not search a provider account.

## Live result

| Provider | Scope checked | Candidates | Valid Mallard markers | Missing markers | Read errors |
| --- | --- | ---: | ---: | ---: | ---: |
| R2 | One account, default jurisdiction | 15 | 6 | 9 | 0 |
| Dropbox | One user, accessible shared folders | 8 | 2 | 6 | 0 |

Both final scans returned `SCAN_COMPLETE`. Both Dropbox matches were mounted
folders with owner access. The scans did not write storage objects or Mallard
configuration. No credentials, destination IDs, names, or paths were saved
in this report.

The first Dropbox run stopped after consent because its permission check
rejected the returned scope set. The final run requested and verified all
four read scopes below. It passed. The first rejected scope set was not
retained, so its exact mismatch was not established.

Live coverage does not include another account, an unmounted Dropbox share,
a Dropbox team space, or an R2 jurisdiction other than `default`. Local tests
cover those discovery branches. Discovery does not prove write access or
validate stored bundles.

## Detection rule

List provider candidates. Then read only `.mallard/_storage.json` at each
candidate root. A supported marker has this content:

```json
{"format":"mallard-storage","layout_version":1}
```

The marker read limit is 4 KiB, as in the Rust storage engine. A missing,
invalid, or unsupported marker has a separate result. A denied read is not
reported as a missing marker. A name such as `mallard-…` or `Mallard …` is
not proof of storage. Renamed storage can still be found.

A newly provisioned bucket or folder can have no marker until Mallard
initializes its storage layout. Such a candidate is not a confirmed Mallard
storage. The POC does not create a marker to test it.

### R2

1. Use the current Wrangler OAuth login, or an explicit Cloudflare API token.
2. List buckets in one selected account and jurisdiction. Follow cursors.
3. Get the marker through the Cloudflare REST object endpoint.
4. Keep the account, jurisdiction, bucket, and S3 URL in memory as the exact
   destination for a future connection review.

The existing saved S3 key can lack bucket-list permission. Discovery needs
account access that permits bucket listing and marker reads. Cloudflare's
Admin Read only token permission is sufficient for these operations.
Wrangler may already have broader rights; the POC sends only GET requests.
See the [R2 token permissions](https://developers.cloudflare.com/r2/api/tokens/),
[bucket list API](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/list/),
and [object read API](https://developers.cloudflare.com/api/resources/r2/subresources/buckets/subresources/objects/methods/get/).

The default scan covers only `default`. Use an explicit jurisdiction for EU,
US, or FedRAMP. The POC reports US candidates but marks them unsupported by
the current app adapter. It does not scan all Cloudflare accounts or regions
automatically.

### Dropbox

1. Obtain a user token with `account_info.read`, `files.metadata.read`,
   `files.content.read`, and `sharing.read`.
2. Read account root metadata. Exclude the personal home namespace. Add a
   distinct team root as a candidate when the account reports one.
3. Call `sharing/list_folders` and `sharing/list_mountable_folders`. Follow
   each route's continuation cursor.
4. Remove duplicate namespace IDs. Read each marker with
   `Dropbox-API-Path-Root` set to that namespace.

Mallard's current setup scope string does not include `sharing.read`.
Discovery therefore needs consent for that scope. The POC uses Mallard's
public app ID and registered callback. It requests a short-lived token and
does not request write scopes or a refresh token.

Unmounted shares are candidates. A failed namespace read stays visible as
an unreadable candidate; the POC never mounts the folder. A viewer can have
a valid marker without permission to Push. Discovery covers the authorized
user's accessible shares and distinct team root. It does not enumerate every
namespace across a Dropbox team or search personal file trees. See the
[sharing API specification](https://github.com/dropbox/dropbox-api-spec/blob/main/sharing.stone),
[file access guide](https://developers.dropbox.com/dbx-file-access-guide),
and [OAuth scope guide](https://dropbox.tech/developers/customizing-scopes-in-oauth-flow).

## Run

Use Node 20.19 or later. No package installation is required. Run from the
repository root.

```sh
node poc/storage-discovery/storage-discovery.mjs mock
node --test poc/storage-discovery/storage-discovery.test.mjs
```

The mock uses synthetic API responses. The authorization test uses a real
loopback callback with a mock token exchange. Tests do not use real provider
homes or Mallard state.

For live R2, use an existing Wrangler OAuth login:

```sh
node poc/storage-discovery/storage-discovery.mjs r2 --wrangler
node poc/storage-discovery/storage-discovery.mjs r2 --wrangler --jurisdiction eu
```

If Wrangler reports more than one account, set
`MALLARD_DISCOVERY_CF_ACCOUNT_ID` to select one. The POC verifies membership
in Wrangler's account list. It never selects an arbitrary account or starts
login automatically. Wrangler manages its own login cache.

For live Dropbox, complete consent in the system browser:

```sh
node poc/storage-discovery/storage-discovery.mjs dropbox --browser
```

The callback uses `127.0.0.1:53682/dropbox/callback`. The helper stops if that
port is in use. It does not change the callback port for a live run.
Browser launch supports macOS and Linux. The environment-token route can
also run on Windows. To use a different registered public app ID, set
`MALLARD_DISCOVERY_DROPBOX_APP_KEY`.

Tokens can instead come from the process environment:

| Command | Required environment variables |
| --- | --- |
| `r2 --env` | `MALLARD_DISCOVERY_CF_ACCOUNT_ID`, `MALLARD_DISCOVERY_CF_TOKEN` |
| `dropbox --env` | `MALLARD_DISCOVERY_DROPBOX_TOKEN` |

Keep tokens out of command arguments, chat, source files, and shell history.
The environment route uses the token supplied by the operator; its grant can
have broader rights than the POC's browser grant.

## Output and limits

The internal result contains names and exact destinations for a future
picker. CLI output contains only counts, temporary references, access roles,
mount state, and fixed errors. References such as `r2-1` are labels for that
scan; they are not storage identities or saved connection IDs.

- `SCAN_COMPLETE`, exit 0: enumeration and marker checks finished. Zero
  matches is a valid result.
- `SCAN_PARTIAL`, exit 2: a list page, candidate read, or scan limit prevented
  a complete result. Earlier matches remain available.
- `SCAN_FAILED`, exit 1: authorization or local setup did not complete.

Limits are 100 pages per listing route, 1,000 unique candidates per scan,
1 MiB per list response, 4 KiB per marker, and 16 KiB per error response.
At most four marker reads run together. Each request, including its body,
has a 15-second limit. A scan has a 120-second deadline. Browser consent has
a separate 180-second limit. Repeated cursors stop the list. There are no
automatic API retries or redirects.

No create, upload, mount, share, delete, connection-save, or restore command
is available. Production schemas, commands, dependencies, and UI are unchanged.

## App integration decision

Add **Find existing storage** beside **Create new storage**. Use this flow:

1. Authorize discovery and select the account or jurisdiction when required.
2. Show verified candidates, access limits, and any incomplete scan status.
3. Let the user select an exact bucket or namespace.
4. Obtain the credentials needed for that destination. Prepare the existing
   setup draft with the normal revision checks.
5. Run the current connection checks. Let the user select **Finish setup**.

Use the same marker and portable layout. The marker has no storage display
name, unique storage ID, credentials, or local trust state. It cannot restore
another machine's connection record. Match existing local connections by
provider destination. Do not auto-connect because a marker was found.

For production, move discovery into the Rust provider modules and use the
current authorization and marker types. The Node POC remains separate.

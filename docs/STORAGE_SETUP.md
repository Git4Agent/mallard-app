# Storage setup

How each storage is connected, what it holds, and how it behaves. The short
version is on the website: [Add a storage](https://mallard-ai.com/docs/add-a-storage).

## Cloudflare R2

1. In Mallard, add a **Cloudflare R2** storage and select **Set up with Wrangler**.
   Mallard runs its packaged setup CLI directly. No agent is required.
2. If Wrangler is missing, install it with:

   ```sh
   npm install --global wrangler@latest
   ```

   The setup row provides **Copy command**, **Copy agent message**, and
   **Try again**. The agent message asks only to install and verify Wrangler.
   Mallard does not install dependencies automatically. Node.js and npm are
   required; a project-only Wrangler install is not sufficient.
3. Check the Cloudflare account shown in Mallard. If Wrangler returns several
   accounts, select one. **Change login** provides a fresh login link. Copy it
   into the browser profile or private window for the required user. Select **Continue** to create the
   bucket and complete browser consent. Both browser steps provide **Copy link**
   and **Open browser**; neither opens automatically. Keep Mallard running and
   use a browser on the same computer. Account checks do not create buckets.
   A draft that has started provisioning stays tied to its original account;
   use a new draft for a different account. Mallard uses Wrangler's OAuth session in memory. It does not
   print or save that OAuth token. Wrangler must support `auth token --json`.
4. If authorization expires, select **Try again**. Setup reuses a confirmed
   bucket. If the saved login is unusable, select **Sign in again**. Each
   attempt opens a new authorization flow.
5. Review the connection values in Mallard, optionally select **Test
   connection**, then select **Finish setup**. Setup prepares only the draft.
   It does not test or commit storage. The Secret Access Key is loaded only
   for the exact draft opened in the editor. Background draft listings and
   committed storage snapshots remain redacted.

Manual connection entry remains available. Setup can also prepare new
credentials for an existing R2 destination without replacing the bucket.

To reconnect a Mallard bucket on another machine, choose **Use existing**,
then **Find in my account**. Confirm the Wrangler account, or select
**Change login** and copy the login link into another browser profile on the
same computer. The scan lists buckets in that account's **default
jurisdiction** and reads only `.mallard/_storage.json`. Use manual entry for
other jurisdictions. Results distinguish Mallard storage, missing or invalid
markers, unsupported layouts, and read failures. Already-connected buckets
cannot be selected again.

Select a Mallard bucket, then choose **Use selected**. Mallard checks the
bucket's creation date and marker before and after browser authorization.
**Copy link**, **Open browser**, and **Cancel setup** work as
in the create flow; no browser opens automatically. Cancellation stops the
local helper and callback listener. **Try again** repeats the stopped step;
**Back to setup** clears the temporary scan and returns to **Find in my
account**, keeping the draft. Finish by reviewing the prepared connection
and selecting **Finish setup**. This flow does not create a bucket or write
a storage marker.

Select **Cancel setup** to stop an active setup. Mallard stops the setup helper
and its child processes before it enables retry. The draft and any bucket
already created remain. Close the authorization tab before you retry.

Manual **Finish setup** and **Save storage** use the same exact R2 capability
probe before committing connection changes. Leaving the Secret Access Key
blank while editing an existing storage preserves the backend-held value.

## Dropbox

For **Dropbox**, choose **Create new** or **Use existing**. Mallard signs in
through Dropbox directly. Each authorization step shows a selectable link,
**Copy link**, **Open browser**, and **Cancel**. Complete consent within three
minutes, or retry with a fresh link. Confirm the returned account, or select
**Switch account**. No agent, Dropbox CLI, or pasted access code is required.

For a new folder, edit the proposed name and select **Create folder**.
For existing storage, Mallard lists accessible shared folders and team roots,
then reads their storage markers without changing them. Select **Use folder**
for a supported Mallard marker. Review the prepared connection, test it, and
select **Finish setup**. Full Dropbox permission covers more than the chosen
folder. Access renews automatically; **Connected** means a request succeeded.

If Dropbox rejects the saved authorization, Mallard says so once, with the
route, HTTP status, Dropbox error identifier, and request ID it can prove,
and stops presenting that credential until it changes. The sidebar and the
Storages index mark the storage **Reconnect**; the storage page, and a Push or
Pull that failed on it, offer **Reconnect Dropbox**, which opens the same
storage's settings. Select **Reconnect Dropbox** there, or paste a
replacement token, then **Save credential**. Reconnection keeps the namespace,
the storage ID, and the project links, and creates no folder. A manual token
that expired is reported the same way and needs a replacement. Access that
merely ran out under a renewable credential is renewed and the read repeated
once; a write is never repeated. Connection health is runtime state; it is not
stored and does not change any schema.

Manual Dropbox setup accepts **Namespace ID** and **Access token** for an
existing shared folder or team namespace. Use a user-linked token from a Full
Dropbox app with `account_info.read`, `files.metadata.read`,
`files.content.read`, and `files.content.write`. Replace a manual token when
it expires. The namespace is fixed after setup. Each machine must connect to
the same namespace; creating another folder creates another storage.

## Feishu Drive

For **Feishu Drive**, your organization's Feishu admin installs Mallard's
app once ([the guide](FEISHU_APP_SETUP_GUIDE.md)); nothing is typed.
Mallard runs no service on the Feishu path: sign-in and every file transfer
happen between the computer and Feishu, Mallard can always fetch from
Feishu with nothing of Mallard's in the way, and Mallard Cloud holds nothing
about it. Choose **Create new** or **Use existing**, select **Connect
Feishu**, sign in in a browser you choose, and confirm the account. Create
new then makes a `Mallard <word>` folder in My Space under a name you can
change; Use existing takes the link of a folder shared with you. There is no
manual setup: a Feishu storage needs the sign-in only the setup mints.
Feishu's consent covers every Drive folder you
can reach, not only that folder; the sign-in stays on this computer, under
`~/.mallard/feishu_connections/`, and renews by itself. Feishu requires a
fresh sign-in within a year of consent; the storage page prints the date.
Two computers share a storage by sharing the folder in Feishu and connecting
each with its own sign-in. If Feishu rejects the saved sign-in, the storage
reads **Reconnect**: **Reconnect Feishu**, then **Save credential**. A release
build carries Mallard's Feishu app (`MALLARD_FEISHU_APP_ID` and
`MALLARD_FEISHU_APP_SECRET` in the build environment; `npm run tauri` also
takes them from a git-ignored `.env.feishu` at the repository root, and a
development build reads them at run time); a build without it says so on
the Sign in line,
and an older build whose app secret was rotated says "update Mallard"
without asking for a new sign-in.

Feishu holds no claims, like a local folder: two computers that push the
same version within seconds can displace one another, and the next Compare
reports the displaced version. **Clear storage data** is unavailable on
Feishu. One request is in flight at a time, 200 ms apart, within Feishu's
documented 5 uploads or downloads per second and 10,000 per day; objects up
to 20 MiB are one request, larger ones up to **100 MiB** go in parts. A Push
that stops at a limit resumes on the next Push, which skips every object
already present. Below the storage folder, payloads, manifests, and records
each get one shard folder so no Drive folder outgrows Feishu's 1,500
children; the logical layout is unchanged. See the
[Feishu storage design](DESIGN_FEISHU_DRIVE_STORAGE.md).

## Upload limits and claims

Dropbox uses the same `.mallard/` layout. Each uploaded object must be at most
**150 MiB (157,286,400 bytes)**. This Dropbox upload limit does not change
Local, R2, capture, or download limits. Dropbox transfers objects concurrently
through upload sessions and commits each bounded batch. See the
[Dropbox storage design](DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md)
for setup and recovery details. The authorization and folder-creation POC
passed with one live account. Full transport and two-account live checks
remain pending.

On R2 and Dropbox a Push claims the version it builds on before it writes
its record, with one create-only write that the storage refuses for a second
computer, so a version listed there has one child and never loses while the
storage's claims are intact. A push that claimed a version and never wrote
its record stops the next Push onto that version with "A push to <storage>
did not finish"; nothing finishes it by itself, and the Push review offers
**Finish that push**, which writes that record once. A Push sends one claim
and one record request and never retries or resends them. A local folder
writes no claim.

## Clear storage data

**Clear storage data** is available for local-folder, R2, and Dropbox storage,
never for Feishu. The
confirmation names the storage and requires you to confirm that sync has
stopped on all machines. It deletes all synced repositories and Library
resources under `.mallard/v1/repositories/` and `.mallard/v1/library/`,
claims first, then records, then everything else.
Mallard validates and retains `.mallard/_storage.json`. It preserves unrelated
objects, local files, provider sessions, and the storage connection.

Local-folder and Dropbox clear check each listed object revision. R2 clear uses
unconditional deletion because R2 does not support revision-bound
`DeleteObject`. Normal object deletion still requires a revision check;
R2 does not provide that operation. Keep sync stopped until clear completes.
A configuration change after the confirmation opens stops the operation.
Local sync metadata resets only after remote deletion succeeds. If clear
fails, the error reports confirmed or possible partial deletion. Use
**Clear storage data** again to delete the remaining objects. There is no
automatic retry of the whole clear operation. Other computers need no unlink
and no reset afterwards: they push again, after a Pull when another computer
already pushed into an area they write. Schema and storage layout versions
remain 1.

Credentials and machine-specific paths are stored only in Mallard's local
metadata under `~/.mallard/`; they are never uploaded inside a portable
project bundle.

## Direct setup and packaged CLI

Builds package `mallard-cli`. The installer does not modify PATH. The desktop
spawns it for R2 setup (`storage r2 setup|accounts|discover`, below), and it
sets up every provider on its own: `storage add local --path <dir>` saves a
folder storage, `storage add r2|dropbox|feishu|cloud` sign in through the
browser (the link is printed, never opened), prepare the draft, probe the
destination and save it in one run, and `storage reconnect <storage>` renews a
lost credential. Open drafts are `storage draft list|show|test|commit|discard`.
See [CLI.md](CLI.md). The desktop's own Dropbox, Feishu and Mallard Cloud setup
pages run in the desktop backend, as before.

Mallard includes its public Dropbox app key and uses PKCE with a registered
loopback callback. Creation adds `sharing.write` to the four storage scopes.
Discovery adds `sharing.read`; reconnection and refreshed access request only
the storage scopes. Setup creates no invitations or public links.

Setup keeps no recovery state. A folder whose creation or sharing was
interrupted stays in the account, and the next attempt with the same name
reports it taken: choose another name, or connect the folder through
**Find in my Dropbox**. Switching accounts is allowed until the draft holds
its namespace. See [Dropbox direct setup](PLAN_DROPBOX_DIRECT_SETUP.md).

Mallard proposes `mallard-<word>` for a new R2 bucket and `Mallard <word>` for
a new Dropbox folder, selecting from 1,155 distinct short words in the
`petname` library's small English noun, adjective, and adverb lists. The
display name does not select the destination.

A storage is named after its destination once that is known: `R2 <bucket>`,
`Dropbox <folder>`, `Feishu <folder>`, `Mallard Cloud <GitHub login>`, and a
local folder's last path component. Until then a draft carries a placeholder,
`<stem> draft <n>`. A typed name always wins. A committed name is unique: a
duplicate takes the next free number, so a second `myconfstorage` becomes
`myconfstorage 2`.

For R2 the proposed name is shown in the setup lane's Bucket line before
anything is created; the user can rename it under Cloudflare's rule (3 to 63
lowercase letters, numbers, or hyphens, beginning and ending with a letter or
number) or ask for another proposal. Setup passes the exact name to the CLI
(`--bucket`). A name the account already has creates nothing and returns to
the Bucket line as `bucket_name_taken`, with the name back in the field.
Nothing pins the account, jurisdiction, or name between attempts: a renamed
request simply asks for that name. A creation Cloudflare did not confirm is
reported once; the next attempt finds the bucket and reports the name taken.
Permission errors never change the name.

For Dropbox, a confirmed name conflict returns to the name field. Edit the
name or select **Suggest**, then approve creation again. Mallard
never adopts a conflicting folder or silently changes the approved name.
All schemas remain at `1`. There is no migration or legacy setup path.

R2 setup saves the exact open schema-1 draft and passes its ID and revision
to the CLI. `storage r2 accounts` lists the current Wrangler accounts;
`--login` opens login again. The desktop waits for account selection and
Continue before calling setup with `--account-id`, `--jurisdiction`, and the
bucket name confirmed in the lane as `--bucket`. The CLI checks that the
selected account is still available, creates the bucket through the R2 management API, and provides
a link to Cloudflare Pipelines' hosted credential-consent broker.
For a saved destination, it prepares credentials for that same bucket.
It receives the callback on loopback and fills that same draft with a revision
check. Without `--bucket`, the CLI proposes a name itself:

```sh
mallard-cli storage r2 --help
mallard-cli storage r2 setup --name "Team backup" --bucket team-backup \
  --draft-id "storagedraft-…" --expected-revision 3
```

The user reviews browser login, MFA, and consent. Credentials
never appear in a command argument or environment variable, and the CLI does
not print them. Terminal setup requires `--account-id` when Wrangler exposes
multiple accounts. Without it, setup uses the only available account. When the command succeeds, the running app
uses the redacted background update only as a signal, exact-loads that one
draft into the editor, and leaves every field—including the revealable Secret
Access Key—editable. The user may test the connection and is the only person
who finishes the setup; that final in-app action performs the capability probe
and commits the storage.
Discarding the Mallard draft removes only the local draft. It does not revoke
an R2 access key or delete a bucket that the user approved in Cloudflare; if
the setup is abandoned, remove those resources from the Cloudflare dashboard.

R2 setup emits an `authorization_link` JSON line while browser authorization
is pending, followed by the existing final result. Mallard sends these links
to the active setup row over an IPC channel. Raw Wrangler output is not
forwarded. Authorization links are not saved in the draft or activity log;
they disappear when the step ends or is cancelled. Bucket links retain the
existing single-use loopback exchange. If authorization expires, retry to
get a fresh link. Wrangler login uses `--browser=false`.

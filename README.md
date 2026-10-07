# Mallard — GitHub for AI-agent work

Git records what changed. Mallard preserves the Codex sessions that explain
why—and makes that project knowledge portable across teammates and machines.

Mallard was built for [OpenAI Build Week](https://openai.devpost.com/), a
global week of building with Codex and GPT-5.6. It is a review-first desktop
app for syncing selected project-scoped agent sessions and independent global
Library resources without copying an entire provider home or credential
store.

**[Project website](https://mallard-ai.com/)** ·
**[Devpost submission](https://devpost.com/software/agentgithub)** ·
**[Watch the demo](https://youtu.be/5Agu6OuaQLg)**

## How Codex was used

Codex was used end to end: Sol for product design and technical planning,
ImageGen 2 for design prototyping, and Terra for implementation and iteration.
Mallard is itself a feature for Codex, making project-scoped sessions and
resources portable across teammates and machines.

## What Mallard does

- Maps all project sessions onto any selected Git branch's first-parent commit
  history, so a team can explore the reasoning behind a diff.
- Parses session summaries and conversation previews locally, with metrics for
  start and end times, user turns, tokens, agent messages, and tool calls.
- Syncs selected project conversations and provider-home resources — memory
  and session state — through Mallard Cloud (sign in with GitHub; private
  snapshots, collaborators, and a website that shows released sessions and
  Library items), a local folder, Cloudflare R2, Dropbox, or Feishu Drive.
  Every storage holds the same immutable layout, and Mallard Cloud runs the
  same publication protocol as R2, with claims.
- Answers the website's links (`mallard://open?address=<a page's address>`).
  A repository page's **Open in Mallard** opens one sheet that says what it
  found — asking each Mallard Cloud account on the page's host — and where
  that stands on this computer, with one verb: the project when it is here,
  otherwise New project on From Mallard Cloud, filled in. A session page's
  **Pull in Mallard** opens the same sheet for its repository with *Pull
  session…*: a Pull review of that session alone, whose **Pull 1** brings it
  here, opens its page, and leaves the project's reviewed base, Pull badge
  and Push gate where they were. A skill, plugin or pet page's **Pull in
  Mallard** opens the item's page on its own Pull review, after Add Catalog
  resource when the item is not in the Catalog yet. The listing's **Open
  Mallard** (Share yours) asks
  nothing and goes straight to Projects, or to the Catalog when a Skills,
  Plugins or Pets chip was on; the sheet appears only when no storage signs
  in to that service. A link only reads, and its verb only opens a page or a
  review; nothing is connected, cloned, added, pulled or pushed until that
  page's own verb is pressed. While an operation,
  a review or a dialog is open, the link waits under the bar instead.
- Compares local and stored resources before Push or Pull, including
  local-only, storage-only, ahead, synchronized, and conflict states.
- Publishes immutable, SHA-256-verified bundle revisions after an explicit
  comparison review. Push and Pull capture and build current actions only in
  Apply; they persist no executable plan or conversation recovery journal.
  Pulls create backups and retain result receipts.
- Optionally syncs selected ordinary files and folders for non-Git projects.
  Git-managed project content continues to travel through Git.
- Manages user-global Agent Skills, provider-native plugin install intent, and
  Codex pets as independent Library resources. The Catalog menu starts empty
  and grows one row per **(provider home × resource kind)** the user adds. A
  row's page works like a project's: one storage chosen in the path bar, one
  list of resources each wearing the same status badge a thread wears, and
  Push and Pull as page verbs with a review. Storage setup returns to the
  originating Catalog row. On every storage an item's name opens its page,
  made like its page on Mallard Cloud: a skill's SKILL.md and files, a
  plugin's own words and the provider CLI lines that install it, a pet playing
  on its desk — read from this Mac's copy, or the storage's when only it has
  the item — beside what only this Mac knows (edited, size, warnings, and the
  launch actions); its path bar carries Pull and Push for that item alone. On
  Mallard Cloud each item is its own repository, and the bar adds the Cloud
  chip (status, release, draft). An item with no repository shows the private one its first
  Push creates, and Change… opens the project's connect dialog: create it now,
  or choose one by address or invite code. A connected item keeps its
  repository. The row's + adds someone else's item from Mallard Cloud, and so
  does Add Catalog resource's From Mallard Cloud door; nothing is pulled on its
  own. An item two repositories hold waits on its page for a choice. Push
  reviews name each item's repository, new ones included.
  Push and Pull show preparation, per-item progress, and persistent results;
  a partial Push keeps successful items and offers a fresh review of the rest.
  Repository bindings are machine-local schema-1 state; portable schemas stay
  at 1. Refresh, Unlink, and local Fork remain available on each Catalog row.
  Add Catalog resource is New project's two steps: the agent home, chosen as
  the project chooses its agents (one card per provider, the saved profile
  preselected when the machine has exactly one, Custom path… and Browse for
  any other), then that home's setup page — one kind as the Kind cards with
  what the home holds on each, the storage, Finish — making one row per
  pass. Setup pages list storages Mallard Cloud first, then the other
  clouds, then local folders, and open on the first; a Catalog row opens on
  the storage a row used last.

The first Library catalog supports Codex and Claude skills/plugins and Codex
pet v1/v2. Pet formats are separate resources — and separate menu rows — and
the user chooses the exact format to operate on. Skill discovery is limited to
`<CODEX_HOME>/skills`, `$HOME/.agents/skills` (default `~/.codex` config
only), and `<CLAUDE_CONFIG_DIR>/skills`; project-local skill directories are
not scanned.

Library inventory reads local provider files. Listing Skills or Plugins,
linking storage, Push, Pull review, and skill Pull/Fork do not require a
provider executable. Codex plugin presence uses local config or remote-install
markers plus cached manifests; cache-only packages are not listed as installed.
Remote plugin enablement remains unknown when it is absent from local config.
Plugin installation, overwrite, or uninstall runs the provider CLI after approval,
with local state checked before and after execution. Schemas remain version 1.

Local item cards and item pages have a trash button. **Delete local skill** and
**Delete local pet** prepare a removal review, count the complete local folder,
and ask for confirmation. Deletion checks file digests and removes only empty
directories. **Uninstall local plugin** uses the selected provider home; Claude
uninstalls only the user scope. Changed or expired reviews must be prepared
again. Removal keeps storage copies and links, so a stored item can be pulled
again. Interrupted removal is reported and is never replayed automatically.

Library resources use a clean cutover: there is no backward compatibility,
migration, legacy execution path, or parallel storage-wide catalog.

## Quick Start for judges

### 1. Install Mallard

The fastest path is the packaged macOS DMG (Apple Silicon or Intel) from the
[latest release](https://github.com/Git4Agent/mallard-app/releases/latest).
The app is Developer ID signed and notarized, so it opens without a security
override. The first **Open in Terminal** or Codex App refresh asks for
Automation permission; allow it. Mallard finds the Codex and Claude CLIs the
way your terminal does, including the `PATH` your shell sets: it asks your
login shell once per launch, in the background, with
`MALLARD_RESOLVING_ENVIRONMENT=1` set so a startup file can skip slow work
when it sees it. If a machine keeps a CLI somewhere else, pick the executable
under Settings › Agent CLIs, where **Test** runs its `--version`. Mallard
downloads newer releases in the
background; **Update** in the sidebar's version mark restarts into one, and
quitting installs it for the next launch. **Automatic updates** can be
turned off under Settings › Privacy, where **Check for updates** always
works.

You will also need Git and a local Codex profile. The default profile is
normally stored at `~/.codex`.

To run the desktop app from source instead, install Node.js with npm and a
stable Rust toolchain, then run:

```sh
npm install
npm run tauri dev
```

`npm run dev` starts only the Vite frontend and does not provide the Tauri
filesystem and storage backend used by these test flows.

### 2. Connect the shared Cloudflare R2 storage

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

For **Feishu Drive**, your organization's Feishu admin installs Mallard's
app once ([the guide](docs/FEISHU_APP_SETUP_GUIDE.md)); nothing is typed.
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
[Feishu storage design](docs/DESIGN_FEISHU_DRIVE_STORAGE.md).

Dropbox uses the same `.mallard/` layout. Each uploaded object must be at most
**150 MiB (157,286,400 bytes)**. This Dropbox upload limit does not change
Local, R2, capture, or download limits. Dropbox transfers objects concurrently
through upload sessions and commits each bounded batch. See the
[Dropbox storage design](docs/DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md)
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

### Direct setup and packaged CLI

Builds package `mallard-cli` for R2 setup. The installer does not modify PATH.
Dropbox setup runs directly in the desktop backend. There is no Dropbox CLI
setup command or storage agent launch path.

Mallard includes its public Dropbox app key and uses PKCE with a registered
loopback callback. Creation adds `sharing.write` to the four storage scopes.
Discovery adds `sharing.read`; reconnection and refreshed access request only
the storage scopes. Setup creates no invitations or public links.

Setup keeps no recovery state. A folder whose creation or sharing was
interrupted stays in the account, and the next attempt with the same name
reports it taken: choose another name, or connect the folder through
**Find in my Dropbox**. Switching accounts is allowed until the draft holds
its namespace. See [Dropbox direct setup](docs/PLAN_DROPBOX_DIRECT_SETUP.md).

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

### Test Flow 1: Pull the public demo project

This flow demonstrates how an existing Mallard project is discovered and
pulled onto a new machine.

1. Clone the public demo repository:

   ```sh
   git clone https://github.com/Git4Agent/mallard_test_demo.git
   ```

2. In Mallard, choose **Add project**, type or paste the path of the cloned
   `mallard_test_demo` folder (or pick it with **Browse…**), and choose
   **Use this folder**.
3. Select the local Codex profile and the R2 storage configured above.
4. Under **Repository**, choose the existing remote repository marked
   **Git match** instead of creating a new repository.
5. Choose **Finish & review**, inspect the Pull plan, approve the intended
   resources, and apply the Pull.
6. Open **Project History** and verify that the project shows:
   - its Git commits and branch timeline;
   - mapped sessions plus any intentionally unmapped sessions;
   - always-visible session summaries;
   - session metrics for activity time, turns, tokens, messages, and tools;
   - expandable conversation previews.

   ![Mallard project history after pulling the public demo project](./docs/images/test-flow-1-pulled-project-history.png)

7. Click **Open ChatGPT Desktop** for a session and verify that the restored
   conversation opens in the desktop app.

   ![Restored conversation open in ChatGPT Desktop](./docs/images/test-flow-1-open-chatgpt-desktop.png)

### Test Flow 2: Push and transfer your own project

This flow works with a new or existing Git project, or with a non-Git folder.

1. Use the project from Codex so it has at least one project-scoped session.
   For the clearest Git mapping, make one or more commits after or during that
   work.
2. In Mallard, choose **Add project** and give it the folder: type its path,
   pick it under **Where your agents have worked**, or use **Browse…**. Then
   select the Codex profile, choose the configured R2 storage, and select
   **New repository**.
3. Finish setup and open **Project History**. Git projects show all project
   sessions against the selected branch's commit history; both Git and non-Git
   projects show summaries, conversation previews, and session metrics.
4. Choose **Push** and review **Git & sessions** and the final **Review**
   step.
5. For a non-Git project, the review scans the folder as it opens. A file
   already in storage travels when it changed here. A new file waits: open
   **Project files** and tick the ordinary files and folders that should
   travel with the bundle. This step is unavailable for Git-managed content.
6. Complete the Push to publish the reviewed bundle to R2.
7. On another machine, configure the same R2 storage, clone the Git repository
   or create an appropriate local folder, add it to Mallard, select the
   existing remote repository, and complete a reviewed Pull.

### Test Flow 3: Get a project from a Mallard Cloud link

1. Choose **Add project** › **From Mallard Cloud**. Select a connected
   account, or add one and choose **Finish setup** to return here with it
   selected. Setup started from **Storages** opens the new storage's page.
   Or, in an installed build, choose **Open in Mallard** on the repository's
   page on the website and then **Set up project…** on the sheet: this page
   opens with the account ticked and step 2 done.
2. Paste the repository's page address, such as `mallard-ai.com/owner/name`,
   and choose **Look up**, or pick it from the list. A private repository you
   have not joined takes its invitation code.
3. Check **Folder**. Under **New folder** it is the repository's name,
   beside your other projects, and **Browse…** picks the parent and keeps the
   name. Already have the project here? Choose **Existing folder** (or **Use
   it as it is** when the path holds files) and pick the folder itself. It is
   connected as it is; a folder whose Git remote isn't the repository's asks
   for one tick first.
4. Choose **Clone and connect** (**Connect** for an existing folder). A
   repository linked to GitHub is cloned with `gh`, then `git`. If cloning
   fails, Mallard initializes an empty Git repository and shows how to fetch
   the source code. A missing GitHub URL also creates an empty Git repository.
   Git must be installed. Only non-Git projects get a plain folder.
   Nothing is pulled: choose **Pull**
   on the project when ready.

### Test Flow 4: A Mallard Cloud repository removed, and a draft discarded

1. Push a project to Mallard Cloud. On the website, open the repository's
   **Settings** and remove it. In Mallard, refresh: the project reads **Being
   deleted**, then **Repository gone**, every session keeps its own state,
   and Push is open.
2. Choose **Push**. The repository is created again at the same address, as
   version 1, and the note ends. A second computer of the same account reads
   **Created again**, pulls, and pushes.
3. Release a version, change a session, add one, and push again. As the
   Owner, open the Mallard Cloud chip in the path bar and choose
   **Discard draft…**, then
   **Discard**. The project reads **Draft discarded**: the changed session
   is ahead, the added one is only on this computer, and the rest are in
   sync with the public version. Pull, then Push, to start a new draft.

## Claude widgets in session detail

Mallard displays complete SVG widgets from Claude's `mcp__visualize__show_widget`
tool in session detail. The Replies filter includes widgets. Other tool calls
remain under Tool calls. Each widget has a Source control with a copy button.

The widget source travels inside the Claude transcript through Push and Pull.
Mallard Cloud displays it from the stored session preview when that preview
contains the complete arguments. No extra widget file or bundle schema is needed.

SVG widgets load as images with local presentation styles. Their scripts and
external document resources do not run. HTML widgets, incomplete input, and
malformed SVG show a message and retain the available source. Widget interaction
state and external dependencies are not synced.

## How session-to-commit mapping works

Mallard parses provider session archives locally to recover session metadata,
activity windows, recorded branch provenance, and recorded commit references.
A session's interval runs from its first user prompt to its last agent reply.
For a Git project, every selected local branch shows the same project-session
set for the loaded time range, and each session is placed on exactly one
commit of that branch's first-parent history: the closest commit at or after
the session's **last user input**, read from the provider's JSONL transcript.
A session with no commit after its last input is listed under
**Uncommitted**. The session's timestamp tooltip shows the time of that last
input.

Placement uses time only. A provider-recorded branch or commit never controls
visibility, Push, or Pull.

The mapping is rebuilt from `git log` whenever Activity loads: when the app
opens, on **Reload app data**, on the refresh button, and when the open
project's `HEAD`, branch refs, or `packed-refs` change outside Mallard, for
example after a checkout or a commit in a terminal. Reloads follow the
checked-out branch unless another branch was picked in the branch menu. See
[docs/DESIGN_GIT_SESSION_MAPPING.md](docs/DESIGN_GIT_SESSION_MAPPING.md).

## Push, Pull, and project-file safety

- Every Push and Pull is reviewed by resource before it changes shared or
  local state.
- A conversation that fails during Push or Pull fails alone: the other
  selected conversations still publish or apply, and the result names the
  failed one with its reason.
- Mallard captures only provider-home state; it never captures configuration
  or instruction files from inside a Git-managed project directory. That
  content travels with Git.
- Non-Git project-file scans are explicit and read-only. `.gitignore`,
  `.ignore`, and `.mallardignore` are honored.
- VCS data, build output, Codex and Mallard metadata, symlinks, and special
  files are excluded or blocked. Size and path limits still apply.
- Non-Git project files can include credential files, private keys, and
  hard-linked regular files. Credential filenames, detected secrets, and
  executable content require an explicit warning acknowledgement.
- Provider-home capture keeps its credential-file and hard-link exclusions.
- Pull deletions begin unselected. Approved file deletion verifies the local
  digest and creates a backup; approved directory deletion removes only an
  exact empty directory and is never recursive.
- Absolute checkout paths, Codex-home paths, storage credentials, restore
  plans, receipts, backups, and local preferences stay on the machine.
- A Mallard Cloud repository that was removed on the website, is being
  deleted, or is out of the account's reach reads as empty: every session
  shows its own state, Push and Pull stay open, and a Push creates the
  repository again at the address it had. Nothing is unlinked, reset or
  relinked, and a refresh saves nothing; a Push or a Pull saves the
  repository it used on the link. Any other answer, such as a suspended
  account, a rejected sign-in or a service that cannot be reached, never
  reads as empty and keeps Push and Pull closed.
- The Owner or an Admin of a Mallard Cloud repository, private or public, can
  discard its draft from the Mallard Cloud chip's panel (**Discard draft…**),
  after a confirmation: every push after the latest version is removed from
  Mallard Cloud and the draft is that version again; with no version, every
  push goes and the repository stays, empty. The row stays in view and is
  off, with its reason, while there is nothing to remove. Nothing on any
  computer is deleted. The website's Versions tab offers the same act.

Settings > Sync has separate **Push Codex children** and **Push Claude children**
switches. Both default to Off and control the next Push. Each cloud task carries
an On/Off child stamp: Compare uses the cloud stamp, while Push uses this
computer's setting. Cloud Off compares only the parent. Cloud On includes
supported owned children in Compare. A stamp difference permits reviewed Push
for otherwise synchronized content, including a task with no children.

Pull restores exactly the child files listed in the cloud and keeps local
settings unchanged. Claude supports direct children only and retains agent
IDs inside the destination parent directory. Inherited children stay outside
the task's sync scope. With Push enabled, one unsupported owned child omits
all children and shows an information icon in review. A missing required
child blocks Push. If that accepted child exists in storage and all other
evidence is valid, Pull offers Keep both to restore the cloud copy and retain
the local files.

A Claude task is updated in place, so Pull can meet child files on this
computer: those an earlier Pull installed, and those Claude wrote here. Pull
replaces such a file only when the cloud lists that child, and only in the
state the review saw. Any other file in the place of a listed child stops
that task. A child that Claude wrote here stays a child of the task when the
cloud version does not list it. A copy from an earlier Pull does not.

Child changes use the same comparison rules as main changes, and a change on
this computer beside a change in the cloud reads as diverged. One case is
different. A child that only this computer has stands against nothing in the
cloud when the task is updated in place, which is so for Claude: the task
reads as behind the cloud, Pull installs what the cloud holds and leaves that
child where it is, and the next Push stores it. So the children that two
computers made of one task come together in one task. A main conversation or
a child that was continued on this computer would be replaced by that Pull:
beside a change in the cloud it reads as diverged, and Pull offers Keep both.
A Codex update is written under a new identity and would leave the child
behind, so for Codex every such case reads as diverged.

The session page shows a card for each call of a child agent. It leaves out
the card of a child that came with a Pull without its transcript. A child
that was made on this computer has its card, also in a task that was pulled
without its children.

Review fixes child membership; Apply captures the latest bytes. A source
change after capture fails that task with a request to Push again. Completed
writes remain after an Apply error; there is no rollback of the task tree.

This is a schema-1 clean cutover with required new fields. Use fresh Mallard
state and empty test storage. See the [owned-child design](docs/PLAN_CODEX_OWNED_SUBAGENT_SYNC.md)
and [verification results](docs/CODEX_OWNED_SUBAGENT_SYNC_VERIFICATION.md). Claude scope and validation are in
the [Claude design](docs/PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md) and
[Claude verification](docs/CLAUDE_OWNED_SUBAGENT_SYNC_VERIFICATION.md).

Refresh reads a session from the place Mallard saved in its file, and only
what was added after it. The saved place is used only on the same file, and
only while that file is untouched or has grown. A session file that was
rewritten, replaced, or shortened is read in full and judged by its content:
an edited session shows as diverged, and Pull offers Keep both. A pulled
session, and a session whose file was copied, is read in full each time it
changes until it is next pushed.

Refresh cannot see an edit that arrives together with new turns, because it
reads only the new turns. Push reads the whole session. It stops that task
once, says that earlier messages changed, and switches the saved place off.
The next Refresh reads the session in full and shows it as diverged. An edit
is a change to an earlier message. A rewind is not an edit, and neither is
the rewrite the Claude desktop app makes when it imports a session. Child
session files are read from their own saved place, under the first rule.

## Architecture

Mallard is a Tauri 2 desktop application. React, TypeScript, and Vite provide
the review-focused interface; Rust handles discovery, validation, hashing,
safe filesystem operations, local persistence, and S3/R2 transport.

Every storage shares the same versioned bundle model, and a storage needs
only three operations: write bytes at a key, read a key, list a prefix. A Push
writes immutable payload objects, an immutable manifest, and last an immutable
publication record. There is no mutable head: the current version is selected
from the names of those records, the same way on every computer, with SHA-256
checks on everything read. When two computers push on the same version within
seconds, the earlier record is selected. The other Push counts as not made:
it fails, or, if it had already been confirmed, the next refresh rolls it
back and says so. Pushing again publishes the same content, and the losing
record is kept in the storage and never selected.

A storage that another computer cleared never asks for an unlink or a reset.
What this computer wrote down about its last sync names a history the storage
no longer has, so Mallard ignores it without deleting it and compares directly.
Push is open when nobody has pushed since, and one Push recreates the storage
under the ids the sessions already had. When another computer pushed first into
an area this computer also writes, a Pull comes first. A session both computers
hold is matched from its content: an equal copy is marked as synced, an earlier
stored version is updated by Push, and a longer or a different stored copy is
never replaced. Pull then keeps both copies. The activity page says which case
applies: "Another computer cleared <storage>. Push again." or "… Pull, then
push again."

See [TECHNICAL_REPORT.md](./TECHNICAL_REPORT.md) for the current schemas,
storage layout, sync semantics, and implementation boundaries.

## Development

Requirements: Node.js with npm, a stable Rust toolchain, and Git.

```sh
npm install
npm run dev          # frontend only at http://localhost:1420
npm run tauri dev    # complete desktop app
```

Application configuration is created through the app; normal development does
not require an `.env` file. The opt-in paid live test has a separate local
connection input described below. The additional aimock live tests are
entirely local and do not use that connection file.

## Verify

```sh
npm run build
npm run test:frontend-integration # strict test type-check plus frontend tests
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:backend-integration  # all Rust targets plus loopback aimock tests
npm run test:integration          # all committed standard suites
```

The backend suite includes Rust and aimock server tests that bind localhost.
Restricted environments may require permission for these test servers. The
aimock server tests use fixed local responses and require no credentials.

### Live Codex API end-to-end test

The API-backed live Codex test is deliberately separate from
`npm run test:integration`. Its five source-conversation turns use an
OpenAI-compatible Responses provider; its twenty final-history reopen checks
use a bounded loopback Responses recorder and make no remote model request.
Every phase uses only fresh temporary Codex homes, projects, Mallard
repositories, and local-folder storage. The test never reads a user Codex
home, requires a Codex login, or reads, copies, or creates `auth.json` or
another legacy credential file. The ignored `.env.local` described below is
the one intentional credential input.

Create an ignored `.env.local` at the repository root with the provider
connection document:

```json
{
  "_type": "newapi_channel_conn",
  "key": "<provider-api-key>",
  "url": "<https-provider-base-url>"
}
```

On Unix, restrict it to the current user before running the test:

```sh
chmod 600 .env.local
```

The runner accepts exactly those three JSON fields, validates the HTTPS base
URL, and fails closed for a missing, malformed, symlinked, oversized, or
over-permissive file. `.env.local` is covered by the repository's `*.local`
ignore rule and must never be committed.

The current rollout fixtures have a hard `codex-cli 0.156.0` prerequisite. The
test rejects another CLI version before making a model request; update the
current-format fixture in place before moving the gate to another version.
The resume checks accept Codex's unrecognized-configuration warning and the
exact missing-metadata warning for the selected model only before the model
turn starts. They still reject other error items and tool events. This does
not change machine-managed configuration.

Run it only when you intend to spend API tokens:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The explicit paid-use gate remains required even when `.env.local` is valid.
For each of the five provider-backed conversation processes, the harness keeps
user configuration disabled, passes a closed set of non-secret custom-provider
settings through repeated Codex CLI configuration overrides, and exposes the
API key only in that child process's dedicated environment variable. It does
not write the key into a Codex config, prompt, command argument, rollout,
snapshot, Mallard object, or test log. The twenty loopback children receive no
real provider URL or credential. All command shapes set
`features.plugins=false`: Codex marketplace synchronization is outside this
scenario, while the skill and pet phases use Mallard's isolated filesystem
fixtures directly.

The test runs exactly 25 Codex CLI turns with no retry path: five real,
provider-backed `gpt-6-luna` conversation turns at low reasoning effort and
twenty deterministic loopback headless resumes. The count is deliberately a
CLI-turn count; the first content tool-using turn can require more than one
provider HTTP exchange. The scenario exercises two isolated machine homes
through project revision 11, including paginated dependency histories,
reviewed Push/Pull, changed-after-comparison rejection, divergence in both
directions, Keep Both, and a no-op fixed point. It then clears the local-folder
storage through the production operation on both replicas, proves both Codex homes are
unchanged, republishes the same ten conversations as fresh revision 1, and
reaches a new fixed point before the Library and resume phases. The same
topology also shares and updates nested Codex skills and fake v1/v2 pets
through the storage Library without additional provider-backed turns.

For all ten final aliases in both homes, the test first resolves and pages the
exact history through the production Codex history and Session Detail paths.
It then runs public `codex exec --json resume --ephemeral` against a one-request
HTTP server bound to literal `127.0.0.1`. The recorder validates the complete
ordered history Codex sent for that exact alias before returning a fixed SSE
response. Each verifier operates on a disposable snapshot because Codex 0.152
appends a turn even with `--ephemeral`; both authoritative homes must remain
byte-identical.

The Codex 0.154.0 API gate passed on September 12, 2026: all five provider-backed
turns and twenty loopback resumes completed in one 49.99-second Rust test run.

See [`docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CODEX_INTEGRATION_TEST.md)
for the API topology, POC evidence, revision matrix, and safety assertions.

### Live Claude Code Tuzi end-to-end test

The paid Claude target uses the same ignored repository-root `.env.local`
connection document, but requires its HTTPS URL to be the exact Tuzi site
root (an input ending in `/v1` is normalized back to the root because Claude
Code appends `/v1/messages`). It is separate from `npm run test:integration`
and from the credentialless AIMock target.

Run it only after checking the Tuzi key's quota and consumption log and when
you intend to pay for three Sonnet 5 turns:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
```

The runner requires Node.js 20.15.0 or newer and exactly Claude Code 2.1.266.
Before Cargo starts, it validates `.env.local` as a bounded, no-follow,
mode-`0600` three-field JSON document and performs a bounded authenticated
catalog lookup for exact model ID `claude-sonnet-5`. It never passes the key
through Cargo's environment. The Rust target independently reopens and
validates the connection file, then exposes the key only to each direct
Claude child as `ANTHROPIC_API_KEY`.

The test makes exactly three Claude model-turn processes with automatic
retries disabled, no fallback model, no tools, `--effort low`, a 1,024-token
output limit, and a USD 0.05 Claude client cap per process. It creates a real
session on replica A, publishes content revision 2 after setup registration
revision 1, pulls and reidentifies it onto replica B, resumes the relocated
transcript through Tuzi, and publishes revision 3. Replica A then resumes its
stale base independently. The test
requires Push to be blocked, reviews a divergent Pull offering exactly one
Keep Both action, preserves A's local continuation byte-for-byte, publishes
both lineages as revision 4, and reconciles exact bindings, comparison rows,
provider UUIDs, and root-level Claude JSONLs on both replicas. It applies
reviewed empty Pull plans and proves no-op Pushes, then clears storage through
the production operation on both replicas. Replica A republishes the same two
conversations as fresh revision 1, replica B pulls them, and the test verifies
unchanged Claude homes, semantic digests, lineage digests, identities, and
cross-replica topology.

All Claude homes, projects, process homes, Mallard repositories, and storage
are private temporary roots. The test uses fixed synthetic markers and sends
no repository source or personal conversation. It validates the production
Claude adapter, paged Session Detail history, transcript structural rewrites,
semantic lineage preservation, credential absence, portable-manifest path
privacy, and full-depth no-follow state scans. The USD caps are client-side
estimates; Tuzi's final consumption log remains authoritative, including for
failed or disconnected calls.

The Claude Code 2.1.266 gate passed on September 12, 2026 with `1 passed, 0
failed`; the Rust body completed in 19.89 seconds with explicit launch isolation. No execution automatically
retried.
The Tuzi consumption log remains authoritative for the earlier development
turns and the current three-turn certification run.

See [`docs/PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CLAUDE_TUZI_INTEGRATION_TEST.md)
for the research, one-turn POC evidence, three-revision implementation, cost
boundary, and exact limitations.

### Keyless aimock Codex end-to-end test

The additional aimock suite drives the same installed Codex CLI and Mallard
two-replica behavior against a deterministic
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not open `.env.local`, needs no
API account, key, Codex login, GPU, or local model, and makes no external model
request. It is a separate test transport, not a fallback or compatibility path
inside Mallard.

Install dependencies and run it with its independent heavyweight-test gate:

```sh
npm install
MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 npm run test:live-codex-aimock-e2e
```

Run the gate from an ordinary Terminal. A development agent already enclosed
by macOS Seatbelt may need permission for the outer test process so Codex can
initialize its nested sandbox; the test still keeps Codex itself at
`workspace-write` with only the reviewed additional directory. The runner
detects the known nested-Seatbelt failure with a bounded, no-provider preflight
before starting Cargo and prints a fixed ordinary-Terminal instruction.
Implementation, protocol POCs, and the complete ignored live gate are verified.
The current native-format gate requires Codex CLI `0.156.0`. The exact keyless
gate passed on September 22, 2026 with `1 passed, 0 failed, 10 filtered` in 51.61
seconds. Every model-shaped response came from the one local
aimock child; the run used no provider account, credential, remote model, GPU,
or external model request.

The Rust test owns the mock lifecycle, supplies a keyless custom provider with
`requires_openai_auth=false`, strips inherited provider/proxy state, disables
plugins and request compression, and permits no retries or Authorization
header. Strict fixture matching rejects unknown, repeated, reordered, or
history-incomplete requests; no proxy, recorder, remote fixture, or upstream
is configured.

The aimock scenario adds native Codex pagination coverage. One bounded
`codex app-server --stdio` process creates P0-P4 with paginated
`thread/start`/`thread/fork` operations, which must consume no Responses
request. Five source turns, five native app-server graph turns, seven later
graph-mutation turns, and twenty final history-resume turns make 37
deterministic turns and exactly 38 local `/v1/responses` requests. The first
content turn uses two intentional scenario legs with the same prompt marker:
an ordinary `exec_command` function call invokes the reviewed `apply_patch`
command, then an exact tool-result call ID selects the fixed final response.
Every other prompt marker is unique, and every fixed response is selected only
after the mock validates the complete ordered marker history.
Pull allocates Codex thread IDs locally during Apply. It uses the existing
random version-7 format, validation, and bounded collision checks. ID allocation
starts no Codex process and makes no API request. The schemas remain at 1.

Pull prepares cloud transcripts in a bounded worker queue. It uses the existing
local worker setting, which defaults to four, and a 64 MiB source-byte budget.
Running jobs and completed results keep their reservations until installation
finishes. One transcript above that budget runs alone if the existing file
limit permits it. Installation follows action order. Source checks, provider
index changes, bindings, checkpoints, and receipts stay in that serial step.

Codex preparation uses downloaded bytes directly. It does not write and read
an intermediate transcript file. Both providers analyze cloud content and the
final local content once each. An immutable result binds the local bytes to
their digest, provider, codec, thread ID, project root, lineage, and semantic
digest. Materialization checks those facts against the current operation and
binding. It verifies the staged SHA-256 without another semantic analysis.
The output layout, path-map, redaction, shared-index, and destination checks
remain in place. A preparation or installation failure affects that thread.

Settings > Privacy controls **Credential redaction** for this machine. It is
Off by default. On replaces detected credentials in exported sessions, child
sessions, sidecars, provider-state content, and titles. Off preserves that
content. Exported content can then include API keys, tokens, and passwords.
Credential-file exclusions and diagnostic sanitization remain active.

Push review shows the selected mode. A setting change before apply requires a
new review. A save attempted during Push apply returns busy; it does not change
the saved mode or the active Push. Retry the save after Push completes.

A selected current session with a different stored mode publishes a complete
new revision under its existing cloud ID. Pull and first-contact comparison use
the stored revision's mode, independent of this machine's outgoing setting.
A mode change does not purge comparison caches. On cannot remove content from
older immutable Off revisions. Off cannot recover text already replaced with
placeholders. All schemas remain at version 1; there is no migration path.

Conversation path conversion preserves literal marker text and destination
paths. Tagged references retain absolute, alias, tilde, and URI spellings.
On return to the same bindings, conversation text is unchanged except for
credential redaction when enabled. Restore IDs and record layout can change.

Local bindings reference immutable path-origin files. These files hold strict
record hashes and inverse edits. Retained history uses those edits on later
Push; only new records are scanned. Missing or stale referenced data fails
that thread. Origin files stay in the Mallard home and never enter a portable
manifest. Metadata reset retains this local history data. The format remains
schema 1 and requires a clean test-data reset for this cutover.
See [codec details and formal checks](formal/pathmap-origins-v1/README.md).

Apply phase logs report cloud verification, reconstruction, output verification,
and serial stage/commit time. The preparation total includes its three worker
phases. Worker durations can overlap each other and serial installation; do
not add them to estimate wall time. The pipeline phase reports wall time and
peak admitted jobs and source bytes. The source-byte budget is not a total
memory limit: parsed records, output bytes, and existing capture state use
additional memory.

The ignored `pull_transcript::tests::preparation_benchmark` test measures eight
synthetic Codex transcripts with fixed destination IDs. Set
`MALLARD_PULL_BENCH_WORKERS=1` or `4` and run it with `--ignored --nocapture`.
It reports preparation wall time, summed worker time, input/output bytes,
queue bounds, and temporary transcript reads. It uses a temporary fixture
directory. The normal tests compare serial and parallel bytes and checkpoints
for both providers, check queue reservations, and verify the analysis count.

The same command also runs five-thread failure isolation on a separate pair
of temporary replicas. Push selects five threads, then retries the two failed
threads, then the last failed thread. Its completion counts are 3, 1, and 1.
Pull also completes 3, 1, and 1 threads. Each selected local source is checked
once during Apply. A detected update fails only that thread, with no automatic
retry. This includes an approved Keep both action. The final attempt writes a
new local message after the check and confirms that Apply still completes.
Service phase callbacks make real Codex turns append
fixed fake-API responses after capture. The checks cover exact failed
identities, unchanged completed content, accepted checkpoints, and the next
review's remaining work. Pull retries approve Keep both for divergent local
history and verify that the preserved branches keep every local message.
Seven disposable history resumes verify the five final stored histories and
two preserved branches. This adds 24 turns and requests.

A native revert flow retains the initial rollout and creates a suffixed
replacement with the same thread ID. It warms discovery and Session Detail
before the revert, checks that Mallard selects one current resource, and
rejects the old timeline cursor. Paged Session Detail before Push and after
Pull must show Turn 1 and the replacement Turn 2, with the discarded Turn 2
excluded. A real Codex CLI resume on the destination verifies the exact restored
history without the old source file. The old rollout remains byte-identical.
This flow adds four requests, for exactly 66 local Responses requests in the
complete suite.

The retry flow reports each attempt's elapsed time. Pull also reports the
source check count, byte count, and elapsed time.
It verifies two semantic analyses per prepared transcript and reports summed
preparation time, stage/commit time, and pipeline wall time.
Per-source read and hash counters verify that each attempt checks each source
once, including a failed check.
All fixtures remain temporary; the local and portable schemas stay at 1.

Pull uses the local snapshot captured at the start of the Apply attempt as its
fixed comparison baseline. Preparation uses that captured digest. After cloud
verification, one bounded source read and SHA-256 comparison runs before each
existing thread is installed. A changed, missing, or unreadable source fails
that thread. Cloud-only creation and metadata-only adoption need no source check.

The rule covers normal updates and approved Keep both actions. In-place
replacements can overwrite a change made after the check. Their backups use
the bytes returned by that check, with no further live transcript read.
Cloud and staged payload verification, destination collision checks, path,
file-type and size checks, approval checks, and identity and binding revision
checks remain. Shared provider files, including the Codex session index, retain
their digest checks through commit. A later write or shared-file conflict can
still fail the thread. Only successful installation advances its accepted
checkpoint; successful sibling threads remain applied.

Source-check cost is linear in the total bytes read. The check adds at most
one transcript buffer. Checked bytes are kept only for the backup or the
retained-source checkpoint, then released. There is no additional cloud
request, wait loop, or automatic retry. Internal checked bytes are not
serialized; schemas remain at 1 without a migration or legacy execution path.

Codex `MaterializeCloud` is always copy-on-write. A native fork can begin
referencing an unchanged parent between review and apply, so neither a digest
pin nor a dependency rescan can make replacement of that parent atomic with
Codex's external writer. Pull instead assigns the standalone history a fresh,
reviewed-absent alias, preserves the prior rollout bytes, and inserts the new
index entry without clobbering the old one. Binding/receipt history suppresses
no content by itself: it derives a domain-separated retained-branch identity,
and an immutable whole-file checkpoint hides the displaced physical alias only
while its exact bytes still match the reviewed source. Missing evidence, a
read failure, or any later append fails open and surfaces that retained branch
in review, Activity, and Session Detail; publishing it creates a direct
binding. Revision 11 proves this for a referenced root, revision 8 proves the
same copy-on-write rule for an unreferenced P2 control, and a focused backend
regression exercises a changed retained branch plus repeated R1-to-R2
displacement. Providers without Codex's physical rollout graph retain their
existing in-place update behavior.

After the revision-11 fixed point, the live flow clears the whole disposable
storage through the production storage-reset operation. It proves that both
replicas forget remote acceptance without changing local Codex state, then
republishes all ten conversations at a fresh revision 1 and converges again.

The native POC also proved that
`history_base.end_ordinal_exclusive` is a global top-level rollout boundary,
not a physical JSONL row count: the child `session_meta` starts at that ordinal
and subsequent rows are contiguous. Mallard now enforces those current
paginated ordinals and decodes portable Codex display titles on Pull in the
single schema-1 path. Neither correction adds a fallback, migration, old-format
path, acceptance of ordinal-less paginated data, or schema change. Codex's
upstream `history_mode: "legacy"` remains its separate current ordinal-less
source mode, not a Mallard legacy execution path.

Both suites exercise the current schema-1 product path. Adding the local mock
does not add a migration, legacy execution branch, or provider fallback to the
application. Copy-on-write materialization likewise changes only the current
execution path: schema 1 remains unchanged, with no migration or compatibility
path.

See [`docs/PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CODEX_AIMOCK_INTEGRATION_TEST.md)
for the aimock topology, protocol POCs, exact accounting, and safety contract.

### Credentialless aimock Claude Code end-to-end test

The Claude Code aimock suite is a separate ignored target for the real Claude
CLI and Mallard's production Claude adapter. It runs against one strict
[CopilotKit/aimock](https://github.com/CopilotKit/aimock) server on an
ephemeral literal-IPv4 loopback port. It does not read `.env.local`, a user
Claude home, or `.credentials.json`, and it needs no provider account, login,
real API key, remote model, GPU, or paid token.

Install dependencies and run the explicitly gated target with:

```sh
npm install
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e
```

Select the repeated Desktop-import scenario with:

```sh
MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 npm run test:live-claude-aimock-e2e -- --scenario desktop-import
```

This separate scenario has 16 main conversation rounds, four simulated imports,
and three further rounds for visible-edit refusal and Keep both. The mock emits
thinking and redacted-thinking blocks through the real CLI. A test helper removes
those blocks, deletes empty assistant rows, leaves parent links unchanged, and
replaces the transcript atomically. Round 6 also constructs explicit mixed and
consecutive thinking-only rows before Push. Each import must leave the session
Synced; the next turn must become Local ahead and Push back to Synced. The test
checks exact request history, the Read exchange, immutable stored objects, and
both replicas after Pull. It reports time and byte counts without timing limits.
It simulates the file rewrite; it does not launch or validate Claude Desktop.
The import after Pull is a required assertion, including the installed-cutoff
boundary. No padding or metadata reset hides that failure.

The current native-format gate requires exactly Claude Code `2.1.266`,
`@copilotkit/aimock` `1.39.0`, Node.js 20.15.0 or newer, and the mock model
identifier `claude-sonnet-4-6`. The runner strips inherited provider,
provider-home, endpoint, and proxy state before Cargo starts. Each Claude
turn disables settings discovery, hooks, skills, plugins, and MCP discovery
through explicit options. It uses `--permission-mode dontAsk` and a one-tool
`Read` allow list. The test avoids `--bare`: Claude Code 2.1.266 does not await
the initial transcript recording operation in that mode. Claude requires a
non-empty `ANTHROPIC_API_KEY` for this custom endpoint,
so the child receives the fixed non-secret placeholder
`mallard-local-aimock-placeholder`. The mock requires that exact value in
`X-Api-Key`; `Authorization` and alternate credential headers are forbidden.
This is credentialless local-mock testing, not a headerless or keyless flow.

The scenario creates five real source turns across two isolated Claude homes.
Setup publishes a registration-only revision 1, and the source turns publish
content revisions 2 through 6. The first content turn performs a real `Read`
tool round; later turns exercise Pull relocation and reidentification, native
resume on a second canonical project path, divergence in both directions,
both Keep Both flows, three final cloud lineages, paged Session Detail history,
and reviewed revision-6 no-op Push/Pull. Six additional snapshot resumes
verify every final lineage from both replicas without changing either
authoritative Claude home. The flow then clears the whole disposable storage,
proves both homes unchanged, republishes all three conversations at a fresh
revision 1, and verifies one incremental cursor append.

The accounting is exact: 12 Claude model-turn processes make 13 authenticated
`POST /v1/messages?beta=true` requests, because the first content tool round
uses two Messages calls. Claude Code 2.1.266 also makes one credential-free
`HEAD /api/hello` compatibility probe per process. The raw boundary therefore
requires exactly 25 requests: 13 model requests plus 12 probes. The Messages
requests are separately reconciled with all 13 semantic aimock scenarios. Any
duplicate scenario, other route or method, extra request, upgrade, wrong key,
credential-bearing probe, history mismatch, or exposed tool makes the audit
fail. No upstream, proxy, recorder, remote fixture, or retry path is
configured.

The package runner and Rust target also perform their own bounded, isolated
`claude --version` preflights. Those are not model turns, start before the mock
traffic, and make no request to aimock; the exact process budget above is
deliberately the model-turn budget.

The fully hardened gate passed on August 31, 2026: `1 passed, 0 failed`; the
Rust test body completed in 19.27 seconds, with all 12 Claude model-turn
processes, 13 model requests, and 12 native hello probes accounted for.

See [`docs/PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md`](./docs/PLAN_LIVE_CLAUDE_AIMOCK_INTEGRATION_TEST.md)
for the initial POC evidence, five-revision matrix, final history oracle,
implementation notes, and safety contract.

## Code map

- `mockups/` — standalone mock pages, browser fixtures, and their source files.
  Run `npm run dev`, then open `/mockups/<page>.html`. These files use the
  shared app components and styles, but are not app entry points.
- `src/components/project-sync/` — project setup, Activity, storage, Push/Pull
  review, project files, and status UI.
- `src/components/project-sync/api.ts` and `src/types.ts` — Tauri command and
  DTO contracts.
- `crates/mallard-application/` — the verified, atomic storage setup use case
  shared by the desktop state and CLI adapter.
- `crates/mallard-cli/` — the packaged storage CLI, Wrangler/browser R2
  onboarding, stable output schema, and process exit contract.
- `crates/mallard-infrastructure/src/providers/` — the compile-time provider
  registry and the Codex and Claude adapters (profiles, discovery, history,
  codecs, materialization, and session actions).
- `crates/mallard-infrastructure/src/project_activity.rs` — provider-neutral
  Activity composition, session details, metrics, and Git mapping.
- `crates/mallard-infrastructure/src/provider_capture.rs` — registry-driven
  resource discovery and no-follow project-content scanning.
- `crates/mallard-infrastructure/src/bundle_engine.rs` — scoped publish,
  fetch, integrity checks, restore planning, backups, and apply.
- `crates/mallard-infrastructure/src/storage_core/` — the shared object-store
  transport, including the local-folder and S3/R2 stores.
- `src/components/storage-catalog/` and `src-tauri/src/storage_catalog/` — the
  independent scope-rowed Library UI, provider adapters, plans, persistence,
  per-resource remote protocol, and journaled apply behavior.

## License

Licensed under the [Apache License 2.0](./LICENSE).

### Mixed provider E2E and performance test

The default mixed test creates **25 Codex sessions and 15 Claude sessions**
with real provider CLIs and local mock responses. It checks 20 Codex fork
edges, selected-history Push/Pull, native resume, path remapping, credential
redaction, Keep both, and three project paths. The two conflict rounds add
eight branches, for 48 final conversations. All test homes and storage are
temporary.

The original 100-Codex/20-Claude fixture is now **xlarge**. It retains its
80-edge graph and 132 final conversations.

```sh
npm run test:mixed-aimock-harness
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e -- --debug
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e -- --baseline-ref="$BASELINE_REF"
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run test:mixed-aimock-e2e-xlarge
MALLARD_RUN_MIXED_AIMOCK_E2E=1 npm run bench:mixed-aimock-e2e-xlarge -- --baseline-ref="$BASELINE_REF"
```

Set `BASELINE_REF` to a reviewed full product commit ID. The benchmark builds
both revisions with the same test worker. It alternates their samples against
the same checkpoints. It uses two discarded iterations and at least 20 measured
samples per operation. Without a baseline, it records measurements and exits 2
with status `uncalibrated`. Debug builds cannot produce benchmark results.

Reports are in the ignored `output/mixed-aimock/` directory, or
`output/mixed-aimock-xlarge/` for xlarge. They contain test
counts, mock audits, operation times, worker CPU and peak RSS, work counters,
and regression results. They do not contain transcripts or provider homes.

The periodic workflow requires a dedicated Linux runner with the
`mallard-mixed-aimock` label and the repository variable
`MIXED_AIMOCK_BASELINE_REF`. The 25/15 test runs nightly at 09:00 UTC. The Sunday
11:00 UTC run tests xlarge scales 10, 50, and 100 with one and four comparison
workers. The
execution phase uses a private network namespace with loopback only.

See [the test procedure](docs/MIXED_PROVIDER_AIMOCK_E2E.md) for pins, test steps,
benchmark limits, baseline qualification, and runner setup. See the
[xlarge procedure](docs/MIXED_PROVIDER_AIMOCK_E2E_XLARGE.md) for the larger fixture.


R2 setup emits an `authorization_link` JSON line while browser authorization
is pending, followed by the existing final result. Mallard sends these links
to the active setup row over an IPC channel. Raw Wrangler output is not
forwarded. Authorization links are not saved in the draft or activity log;
they disappear when the step ends or is cancelled. Bucket links retain the
existing single-use loopback exchange. If authorization expires, retry to
get a fresh link. Wrangler login uses `--browser=false`.

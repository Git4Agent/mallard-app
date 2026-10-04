# Plan: Pull this session — a link from a session page into Mallard

**Status (2026-10-04): proposed**, nothing built. Layer 2 of
`docs/PLAN_DEEP_LINK_DESTINATIONS.md` (session links), on the same scheme and
sheet as repository links and `PLAN_DEEP_LINK_SHARE_YOURS.md`. Sessions are
the main case; *Non-Git projects* and *Skills, plugins and pets* below say
what differs for them. The choices are collected at the end. Two mocks:
`mockups/session-pull-mockup.html` + `.tsx` is the specimen in the app's own
parts (`npm run dev`, then `/mockups/session-pull-mockup.html`; `?scene=`,
`?dir=b|a`, `?stage=`, `?theme=dark`, `?shot=on`): the shipped sidebar, path
bar, connect-dialog frame, Pull review, session page, New project and Add
Catalog resource on a stand-in backend, with the plan's additions dotted.
`mockups/session-pull-link-mockup.html` is the flow and the choices on one
static page (no build step).

## Where things stand

- The website (`mallardInternal`) shows one session at
  `/{owner}/{name}/sessions/{resource_id}?commit=…` (R2 architecture §8).
  `{resource_id}` is the manifest row's id, `<provider>:conversation:cloudthread-…`.
  The repository page has *Open in Mallard*; the session page's Pull menu
  still leads with the steps by hand.
- The desktop reads a session page's link as its repository: the tail after
  `{owner}/{name}` is dropped (`managed_cloud/app_link.rs`,
  `address.rs`). The link then opens the project, not the session.
- Pull is whole-project. `plan_bundle_restore` compares every thread and
  file against the storage's current version, the review ticks rows, Apply
  takes action ids, and a complete apply advances the link's reviewed base
  and the cloud pin. A session that is only in storage is a row under *Only
  in {storage}* with no page of its own; it comes here through that Pull.
- Push has a scope (`PushScope::Session`, `PLAN_SESSION_DETAIL_PUSH.md`):
  one session, enforced in Rust, no Git capture, the saved recipe untouched.
  Pull has no mirror of it.

## What changes for the person

On the website's session page, one button: **Pull in Mallard**. Mallard
comes forward with the sheet, *Open from Mallard Cloud*: the repository it
found, that the link names one session of it, and one verb. The verb opens a
Pull review of **that session alone**; **Pull 1** brings it here and lands
on its page in Mallard.

| The session, on this computer | The sheet's verb | It opens |
| --- | --- | --- |
| Not here (project connected) | Pull session… | The one-row review, ticked. Pull 1 → the session's page |
| Here, up to date | Pull session… | No review: *Already here and up to date.* → the session's page |
| Here, newer in Mallard Cloud | Pull session… | The one-row review: *Update* |
| Here, changed on both sides | Pull session… | The one-row review: *Keep both* |
| Project not on this computer | Set up project… | New project › From Mallard Cloud, as today. The link is used again after setup |
| Not in the storage's current version | — | *That session isn't in {address} any more.* |

The stops are the repository link's: no account on the host, another host,
no account can see it, the service could not be asked, not a link. A link
waits under the bar while an operation, a review or a dialog is open, as
today.

Between the click on the website and a write on disk there are two presses
in Mallard: the sheet's verb, and Pull.

## The link

`mallard://open?address=https://mallard-ai.com/{owner}/{name}/sessions/{resource_id}`

- The page's own address, as the repository and listing links are: one verb,
  `open`; one parameter, `address`. Nothing else is read.
- A tail of exactly `sessions/{id}` names a session. `{id}` is read
  percent-decoded and must parse as a cloud conversation resource id
  (`parse_cloud_conversation_resource_id`); one that does not is **not a
  link** — the website never sends one, so it is a forged or broken link,
  not a repository. Any other tail (`/release`, a trailing slash) stays the
  repository, as today.
- The query (`commit=`, `scope=`) is the page's and is dropped. The app pulls
  the version the account reads now, which is what the website shows at the
  head; see decision 5 for a historical view.
- A link still cannot name a verb, a folder, an account or a storage.

## The rule, in order

1. **Read the link**, in Rust. `AppLink::Session { host, address, session }`
   where `session` is the provider and cloud thread id. Not one → *Mallard
   can't open that link.*
2. **Find the accounts** on the host and **ask the service** for the
   repository, exactly as a repository link does (`resolve_app_link`,
   `repository_by_name`). The service is not asked about the session: the
   repository summary has no sessions, and the storage read that would
   confirm it is the Pull review's own.
3. **The project is not here** → *Set up project…*, as today. The session is
   not carried through setup in this layer (decision 8).
4. **The project is here** → the sheet's row is the repository (mark,
   standing), its second line *One session of this repository* and *On this
   computer: {project}*; the verb **Pull session…**.
5. **The verb** runs the shipped Pull path with a scope:
   `beginSessionRestore(projectId, storageId, session)` → the binding check
   `beginProjectRestore` makes → `planRestore` with
   `scope: { kind: "session", resource_id }`.
6. **The plan** answers one of three ways: the row is synced → no review,
   the session's page opens with the line *Already here and up to date.*;
   the row has a Pull action (`create_local`, `materialize_cloud`,
   `keep_both`) → the review opens with that one row ticked; the row is not
   in the current version → the error above, nothing opens.
7. **Pull 1** applies the one action. On success the project reloads and the
   session's page opens, found by its `cloud_thread_id` in the refreshed
   comparison. The after-Pull offers (a desktop app restart, say) are shown
   as after any Pull.

## The one-session Pull

The mirror of the session Push, with the same boundary in Rust.

- `PullScope { Project, Session { resource_id } }` (`service.rs`, beside
  `PushScope`), a parameter of `plan_bundle_restore` and a field of
  `RestorePlan`/`PullReview`; `apply_bundle_restore` re-reads it from the
  review. Not persisted beyond the plan.
- A session plan has **no file actions**: no provider state, tools or
  project files; `project_content_eligibility` is absent so the review has
  no Files step. `conversation_review` holds the one row. Discovery is
  narrowed to the thread's local twin through `thread_review`'s
  `resource_filter`, as the session Push narrows it, so a large project's
  other threads are not read.
- The row's id is matched in the pinned manifest (`find_conversation`).
  Absent → *That session isn't in {storage} any more.* (removed, a draft
  discarded, or a public reader after a release without it).
- **Apply refuses** any action id outside the row and any selection that is
  not the plan's scope, before it reads anything.
- A session Pull **never advances the link's reviewed base or the cloud
  pin**: the rest of the storage was not reviewed. Its materialization
  record is written `Partial` by scope, not from its receipts, and
  `reviewed_base_advanced` is absent from its result. The project's Pull
  badge and Push gate stay exactly as they were, as a session Push leaves
  the project's Git history alone. The row's own acceptance is recorded as a
  project Pull records it, so the session reads synced and can be pushed
  from its page.
- Automatic metadata acceptance (`canAcceptPullMetadata`) runs scoped: a
  session whose only difference is storage metadata is accepted under the
  sheet's verb and its page opens.
- Pins, digests, the one-day expiry, the apply limit and the stopped-Pull
  rule are unchanged. Nothing is deleted.

## Non-Git projects

The same link, route and session id: a non-Git project's repository has the
same session pages. What differs is the folder and the files.

- **Setup from the link.** *Set up project…* lands on New project › From
  Mallard Cloud as for a Git project, but the verb reads **Create folder and
  connect** (or **Connect** for an existing folder): nothing is cloned, the
  folder starts empty, and the project's files arrive only through its Pull
  (`README.md`, Test Flow 3; `CloudProjectSource.tsx`).
- **The session scope carries no project files.** A session Pull writes the
  session and nothing else, on a non-Git project too. A session pulled alone
  into a fresh folder can therefore mention files that are still only in
  storage. The session's page says so when the project's comparison holds
  pending project files: *Project files for this folder are still in Mallard
  Cloud. Pull the project to bring them.* The project's Pull badge already
  counts them.
- **Path remap** of a pulled Codex session to the local folder is the
  project Pull's today and is unchanged.
- Git-managed source never travels through Mallard; a Git project's files
  come with the clone. Neither kind of project gets files from a session
  link.

See decision 9 for the alternative that brings the files along.

## Skills, plugins and pets

On the website a Catalog item is its own repository, and its page is the
repository page, `/{owner}/{name}` — the page that already has *Open in
Mallard*. There is no sub-page to link, so the item's pull link **is the
repository link**, and what changes is the button's word and the sheet's
verb.

Where things stand (`PLAN_DEEP_LINK_DESTINATIONS.md`,
`PLAN_CATALOG_CLOUD_LIKE_PROJECTS.md`):

- Saved in a Catalog row → *Open in Catalog*: the item's page, whose own
  **Pull** opens the row's Pull review narrowed to that item
  (`StorageCatalogPage.tsx`, `openPull`). The one-item Pull exists; the link
  does not press it.
- Not in the Catalog → *Add to Catalog…*: Add Catalog resource › From
  Mallard Cloud. **Add** saves the binding and lands on the item's page
  reading *Only in Mallard Cloud. Pull to download it here.* with `Pull 1`;
  nothing is pulled on its own (owner, 2026-10-01: Connect no longer opens
  the Pull review).
- Yours, with no row of its kind → *Add to Catalog…* with the Kind chosen.
- On Mallard Cloud each item has its own repository, version and source
  (`item_heads`, `sources`, `reads: draft | public`); Apply takes one choice
  per item and leaves the rest *Skipped; local content was left unchanged*.

What changes:

| The item, on this computer | The sheet's verb | It opens |
| --- | --- | --- |
| Saved in a row | Pull {kind}… | The item's page with its one-item Pull review open, as its Pull button opens it. Up to date → no review: *Already here and up to date.* |
| Not in the Catalog | Add to Catalog… | Add Catalog resource › From Mallard Cloud, as today. After **Add**, the item's page opens **with its Pull review open** (decision 11) |
| Yours, no row of its kind | Add to Catalog… | As today; after Add, as above |

- The sheet's row keeps the pickers' words (*Public · Contributor · v5*) and
  the second line names the row (*In your Catalog: Pets v2 · Default Codex*).
  Whether the item is up to date is the review's to say, as for a session:
  the sheet makes no storage read.
- **Scope.** The item's page narrows the review in the window and sends one
  choice; that is the shipped pet page and it is enough for layer 1. A
  backend `only` filter on `prepare_catalog_scope_pull`, so a row of forty
  pets reads one repository instead of forty, is decision 12.
- **Version.** The Pull reads the draft for a collaborator and the public
  version for anyone else, as the Catalog's Pull does; the link does not
  pin a version.
- **Public items** a signed-in non-member adds are recorded on Mallard Cloud
  (`catalog-addition`), as Add does today.
- Each kind pulls what its Catalog Pull pulls today: a skill's files, a
  plugin install intent, a pet. Nothing kind-specific is added.
- **The project repository page** keeps *Open in Mallard*: a project has no
  one thing to pull, and *Pull this project* by the same scope is layer 3.

## Safety

- **A link only reads** until the sheet's verb, and the verb only opens a
  review. The gate test on `followAppLink` gains one route,
  `beginSessionRestore`, which is the shipped Pull path with a scope; the
  hook's four reads do not change.
- **Untrusted text.** The raw link stops in Rust. The window receives the
  host, `owner/name`, a provider key and a cloud thread id, each validated
  by its own grammar; the page address is re-read before the service is
  asked, as today.
- **The token goes only to its own service.** Unchanged.
- **The scope is enforced where the bytes move.** A session Pull cannot write
  a file or another session, whatever the window sends.
- **No base or pin moves** for a partial review. Tests demonstrate the
  boundary (below). No invariant in `AGENTS.md` is relaxed.

## Website (`mallardInternal`)

1. **The button.** The session page's path bar ends with **Pull in Mallard**
   (primary), using `openLink` from `src/appLaunch.ts` and
   `appLink(pageAddress)` from `pull.ts` with the page's own address. On a
   phone or tablet (`canOpenApp`) it is not drawn.
2. **No app answered.** The 1.5 s focus test; the band turns to *Didn't
   open?* with *Get Mallard*, as the listing's does.
3. **A historical view** (`?commit=` behind the head): the button's tooltip
   says *Pulls the current version of this session* (decision 5).
4. **A Catalog item's page** (a skill, plugin or pet): the existing button
   reads **Pull in Mallard**; its link is unchanged. The project repository
   page keeps *Open in Mallard* (decision 10).
5. **Tests** (`web/test/pull.test.ts`): the session link; the repository
   link unchanged; the page's markup on desktop and on a phone; the item
   page's label.

## Desktop (`mallard-app`)

1. **Reading** (`managed_cloud/app_link.rs`): `AppLink::Session`; `address.rs`
   gains `parse_session_page_address`, sharing `parts` and `repository_in`.
2. **Arrival** (`project_sync/app_link.rs`): `AppLinkArrival::Session { host,
   address, provider, cloud_thread_id, resource_id }`. `resolve_app_link` is
   unchanged; the session rides the arrival.
3. **Scope** (`service.rs`): `PullScope`; `plan_bundle_restore_with_progress`
   takes it; the session branch narrows discovery, drops file actions, finds
   the row; `apply_bundle_restore_with_progress` checks it and skips the
   base and pin advance. `commands.rs`, `lib.rs` unchanged in names.
4. **Contract**: `PullScope` and `PullReview.scope` in `src/types.ts`;
   `projectSyncApi.planRestore(storageId, bundleId, binding, scope)`.
5. **Routing** (`app-link/appLink.ts`): `LinkDestination` gains
   `{ kind: "pull_session", projectId, storageId, resourceId }`; the verdict
   for a session link reuses the repository join and names the verb. The
   `catalog_item` landing gains `pull: true` and its verb reads *Pull
   {kind}…*; `add_catalog_item` and `add_catalog_row` carry `pullAfterAdd`.
6. **Shell** (`ProjectSync.tsx`): `beginSessionRestore` beside
   `beginProjectRestore`; `planRestore` passes the scope; the synced and
   not-found answers; after Apply, `openSessionDetail` for the row's thread.
7. **Review** (`RestorePlanView.tsx`): with a session scope the row starts
   ticked, the steps bar is hidden, the header reads *Pull session · {title}*,
   the primary reads **Pull 1**, and *Mark as reviewed* is not offered.
8. **Outcome** (`syncOutcome.ts`): *Pulled “{title}”.* No Push-hold sentence
   for a session scope; the project's badge already says what else is in
   storage.
9. **Waiting line**: *Link · {title or owner/name} is waiting.*
10. **Catalog** (`StorageCatalogPage.tsx`): a landing with `pull` opens the
    item's page and calls its `openPull`; a synced item shows *Already here
    and up to date.* instead of an empty review. Add Catalog resource's
    From Mallard Cloud door passes `pullAfterAdd` through to the landing.
11. **Session page** (`SessionDetailPage.tsx`): the pending-project-files
    line for a non-Git project, read from the comparison the page already
    has.

## Tests

- `managed_cloud::app_link`: the website's session link verbatim, with raw
  and percent-encoded colons; a malformed id refused; `/release` and a
  trailing slash still the repository; every existing refusal.
- `project_sync::app_link`: the arrival serialized as the window reads it.
- `sync_tests` (Codex and Claude, fixture homes): a session plan has one row
  and no file actions while the project has other changes; apply refuses an
  outside id; the link's reviewed base, cloud pin and the other rows'
  acceptance are byte-identical after a session Pull; the session then
  reads synced and its Push is accepted; a row not in the manifest errors
  before anything is read; a keep-both session Pull preserves as a project
  Pull does.
- `tests/frontend/app-link.integration.test.tsx`: the table above, the
  stops, the verb's gate with the one route added, the waiting line.
- `tests/frontend/pull-review.integration.test.tsx`: the scoped review's
  rows, header, primary and the absent Files step; the synced and not-found
  answers; the landing on the session's page.
- `sync_tests`, non-Git: a session Pull into a non-Git project writes the
  session and no project file, and the project's files step still lists
  them afterwards.
- Catalog (`cloud_flow_tests.rs`, `tests/frontend/`): a one-item Pull on a
  Mallard Cloud row leaves the row's other items' heads and content
  unchanged; the item landing with `pull` opens the review on that item
  alone; Add with `pullAfterAdd` lands on the review; a synced item says so.

## Why this shape (what others do)

- **A link carries an identifier, never an instruction, and lands on the
  app's own confirmation surface.** GitHub Desktop's
  `x-github-client://openRepo/<url>` opens the Clone dialog pre-filled when
  the repository is not on the computer and switches to it when it is; VS
  Code's `vscode://vscode.git/clone?url=` asks for a folder before cloning;
  Steam's `steam://install/<id>` opens the install dialog. None performs the
  write from the link. The sheet, then the review, is Mallard's version.
- **A link to something already here navigates.** Slack, Linear, Figma and
  Notion links name one page by id. *Already here and up to date* opens the
  session's page and pulls nothing.
- **A private-use scheme is untrusted input.** RFC 8252 §7.1 and §8 note any
  app can claim a scheme and any page can fire one. The raw link stops in
  Rust, only validated parts cross, and the token goes only to its own
  service. Verified `https` links (Apple Universal Links, Android App Links)
  remain the unplanned layer.
- **The page cannot know whether the app opened.** Browsers prompt (*Open
  Mallard?*) and remember the site; the website's 1.5 s focus test and
  *Didn't open?* band are the standard fallback. Deferred deep linking
  (install, then land) is a later layer.
- **One link per page, the page's own address.** Keeps the parser to one
  form, the website to one helper, and the refusal list intact.

## Decisions for the owner

1. **The link's form.** Recommended: **A**, the session page's own address
   under `open`; the app decides. Alternatives: **B**, a second verb,
   `mallard://pull?address=…`, explicit on the wire but a second form to
   read and refuse-test, and two links per page; **C**, a query flag on the
   page address, which the parser drops today.
2. **The website's button.** Recommended: **Pull in Mallard**, saying what
   happens. Alternative: **Open in Mallard**, uniform with the repository
   page, and the sheet's verb says Pull.
3. **Sheet or no sheet.** Recommended: **sheet first**, as repository links,
   since there is something to say (the account, the project, two accounts).
   Alternatives: straight to the review, the sheet only for the stops; or
   skip the sheet only when the project is connected. Sub-choice: the sheet
   names the repository and *one session* (recommended, no extra read) or
   reads the manifest for the session's title (one storage read before the
   verb; layer 2).
4. **Where the scope lives.** Recommended: **`PullScope` in Rust**, as the
   session Push: plan and apply refuse anything outside the row, no base or
   pin moves. Alternative: the whole-project plan filtered in the window,
   which compares every thread, leaves enforcement to the window, and would
   advance the base when nothing else is ticked.
5. **`?commit=` on a historical view.** Recommended: **pull the current
   version**, the button's tooltip says so. Alternatives: hide the button on
   historical views; honor the commit and pull that version (the planner
   observes the current snapshot only; a later layer).
6. **After Pull.** Recommended: **open the session's page**. Alternative:
   stay on the project's activity list with the row highlighted.
7. **Already here and up to date.** Recommended: **no review**, open the
   session's page with one line. Alternative: show the empty review.
8. **Project not on this computer.** Recommended: **Set up project…, then
   the link again** (the sheet says so). Alternative: carry the session
   through setup and open its review when the project connects (layer 3;
   the set-aside mechanism exists for the account case).

9. **A non-Git project's files.** Recommended: **the session alone**, and
   the session's page says the folder's files are still in Mallard Cloud.
   Alternatives: a session Pull on a non-Git project also shows the Files
   step with the stored files unticked (two scopes in one review, and the
   files' own warning acknowledgements come with it); or, for a non-Git
   project set up from the link, the verb opens the project's first Pull
   with the session ticked instead of a session Pull.
10. **The item page's button.** Recommended: **Pull in Mallard** on a
    skill, plugin or pet page; the project repository page keeps *Open in
    Mallard*. Alternative: one label everywhere.
11. **After Add from a pull link.** Recommended: **open the item's Pull
    review** on its page, since the button said Pull; Add alone still
    pulls nothing, and the review is the second press. Alternative: land
    as today, on the page with `Pull 1`, and leave the press to the person
    (the owner's 2026-10-01 rule for Connect, kept for a link too).
12. **The Catalog's scope.** Recommended for layer 1: **the window narrows
    the review and sends one choice**, as the item's page does today.
    Later: an `only` filter on `prepare_catalog_scope_pull`, so one item's
    Pull reads one repository.

## Layers

1. This.
2. The session's title in the sheet, from the manifest; the Catalog's
   backend `only` filter.
3. Carry the session through project setup; honor `commit=`; a *Pull this
   project* link on the repository page by the same scope; a non-Git
   session Pull that offers the files.
4. Not planned: verified `https` links, sign-in by link, invitation links.

## Not checked

- The website's session page code is in `mallardInternal`, not read here:
  whether its Pull menu is the repository page's `PullMenu.tsx`, and whether
  the id in its address is sent raw or percent-encoded. The parser accepts
  both.
- The owner's live pass: a real click from the session page into an
  installed build, Safari, Chrome and Firefox, with Mallard closed and open,
  and with a review open.
- That a one-item Catalog Pull on a Mallard Cloud row leaves the other
  items' accepted heads where they were (`item_heads` suggests so; the test
  above proves it).

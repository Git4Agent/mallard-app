# Plan: Open in Mallard — where a link from the website lands

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

**Status (2026-10-02): implemented**, uncommitted. The owner picked on
`mockups/deep-link-mockup.html`: **B · Sheet first**; each account on the link's host
is asked; a link waits while an operation, a review or a dialog is open;
session links are a later layer (built since: `PLAN_DEEP_LINK_SESSION_PULL.md`).

Bars: `cargo test --lib` 524/0 (15 ignored), the infrastructure crate's
address and link tests 6/6, frontend integration 544/544, release-config
10/10, `tsc` clean, clippy clean on the touched Rust. Driven end to end in
the shipped shell on a stand-in backend (`/mockups/deep-link-live.html`).

**Not run:** the system handing a real link to an installed build. macOS
takes links only in an installed bundle, and installing one here would claim
`mallard://` on this computer and open the real `~/.mallard`. See *Live pass*.

## Where things stood

- The website already sends the link. Its repository page has *Open in
  Mallard* (`mallardInternal`, `web/src/components/PullMenu.tsx`), which hands
  the browser `mallard://open?address=<the page's address>`
  (`web/src/pull.ts`, `appLink`). For an item, a signed-in reader who is not a
  member is put on the Catalog first (`PUT …/catalog-addition`).
- No desktop answered it: no scheme, no deep-link plugin.

## What changes for the person

Clicking *Open in Mallard* brings Mallard forward with one sheet, *Open from
Mallard Cloud*: the repository it found, where that stands on this computer,
and one verb.

| Page on the website | On this computer | The sheet's verb | It opens |
| --- | --- | --- | --- |
| A project | not here | Set up project… | New project › From Mallard Cloud, account ticked, address looked up |
| | connected to a project | Open project | that project, on that storage |
| A skill, plugin or pet | not in the Catalog | Add to Catalog… | Add Catalog resource › From Mallard Cloud, item found, row chosen |
| | saved in a Catalog row | Open in Catalog | the item's page in that row |
| | yours, a row of its kind exists | Open in Catalog | that row, on the item |
| | yours, no row of its kind | Add to Catalog… | Add Catalog resource, its Kind chosen |

Cancel, ×, or Esc lets the link go. The link itself presses nothing: the
landing pages still wait for their own verb.

## The link

`mallard://open?address=<absolute http(s) address of a repository page>`

- One verb, `open`; one parameter, `address`. Any other verb, a path under
  it, a second parameter, a fragment, userinfo, a port, or more than 2048
  characters is not a link (`managed_cloud/app_link.rs`).
- The address is the page's own; its tail and query are dropped, so a
  session page's link opens its repository until layer 2.
- A link cannot name a verb, a folder, an account or a storage.

## The rule, in order

1. **Read the link**, in Rust. Not one → the sheet says *Mallard can't open
   that link.* and shows none of its text.
2. **Find the accounts**: the Mallard Cloud storages whose saved service is
   the address's host. None, and the host is this build's service → *Sign in
   to Mallard Cloud to open maya/portal.* with **Set up Mallard Cloud**; the
   link then waits under the bar, and a new Mallard Cloud account opens its
   sheet again. None on any other host → *No account here for {host}.* A link
   never introduces a service, and no request is sent.
3. **Ask the service** as each of those accounts. Nobody sees it → *Rika321
   can't see acme/billing.* (*No account here can see …* with several). An
   account that could not be asked, when none sees it → its failure and **Try
   again**.
4. **It is here → Open.** A project whose storage link on that host saves the
   repository (or its bundle); two projects: the first in the project list.
   An item a Catalog row saved (`find_catalog_cloud_repository`).
5. **It is not → the page that adds it.** One account sees it: that one is
   ticked. Several do: New project ticks none and its hint reads *Choose an
   account for maya/portal.*; the pick looks it up. The page's own lookup
   runs, so its checks still apply.

## When the sheet opens

At once, unless the shell is in the middle of something: a tracked operation
is running, a Push or Pull review is open or being prepared, or a dialog is
open. Then the link waits under the bar as *Link · maya/portal is waiting.*
with **Open** (off while an operation runs) and the usual ×. Setup pages do
not hold a link. One link is held at a time: a newer one replaces it, and
takes over an open sheet. Nothing about a link is saved.

Started by a link, the app finishes its startup first.

## Safety

- **A link only reads.** It never signs in, connects, clones, creates a
  folder, joins, pulls or pushes. The sheet's verb only navigates, by the
  routes a sidebar click uses; a gate asserts the list of calls.
- **The token goes only to its own service.** A storage is asked only when
  its saved origin's host equals the link's; the test sends a canary token on
  another origin and counts the requests.
- **Untrusted text.** Any page can fire the scheme. The raw link stops in
  Rust (`receive`); the window gets `host` and `owner/name`, re-read as a page
  address before anything is asked. The window holds no deep-link permission
  and the app ships no deep-link JS package.
- **Nothing is interrupted** by a link: see the waiting rule.
- No invariant in `AGENTS.md` is relaxed.

## What was built

1. **Scheme.** `tauri-plugin-deep-link` (`plugins.deep-link.desktop.schemes =
   ["mallard"]`), read from Rust (`project_sync::app_link::listen`: the link
   that started the app, then `on_open_url`). `tauri-plugin-single-instance`
   with its `deep-link` feature on Windows and Linux only, registered first:
   there a link starts a second process, which hands its arguments over and
   exits. macOS delivers to the running app, and the plugin there would end a
   development build started beside an installed one. Both are held to the
   2.4 line; 2.5 needs Tauri 2.12 and the app is on 2.11.
2. **Reading**, `managed_cloud/app_link.rs`: `parse_app_link`. The address
   parser is split so the link shares its checks (`parse_page_address`); its
   behaviour is unchanged.
3. **Answering**, `project_sync/app_link.rs`: `AppLinkInbox` (the newest
   link), `take_app_link`, `resolve_app_link` (each account on the host, by
   the existing by-name read), the `app-link` event, one activity-log line
   with the host and address only.
4. **Catalog**: `find_catalog_cloud_repository(storage_id, repository_id)`
   reads the row's saved connections.
5. **Routing**, `src/components/app-link/appLink.ts`: `linkVerdict` (pure) and
   `linkWaits`. `useAppLink` takes, holds and answers; `AppLinkSheet` is the
   connect dialog's frame around the pickers' found row.
6. **Shell**, `ProjectSync.tsx`: the waiting line, the sheet, and
   `followAppLink`, which calls `openProject`, `beginAddProject`,
   `beginAddCatalogScope` and the Catalog row's landing.
7. **Pages take an address**: `initialAddress` on `NewProjectFolderStep` →
   `CloudProjectSource` → `RepositoryAddressPicker`; `initialCloud` on
   `AddCatalogScopeWorkspace` → `useCloudRepositoryChoice`.

## Contract changes

- `tauri.conf.json` (`plugins.deep-link`), two crates, no capability change.
- Commands `take_app_link`, `resolve_app_link`, `find_catalog_cloud_repository`;
  DTOs `AppLink*` (`src/types.ts`) and `CatalogCloudPlace` (the Catalog's
  `types.ts`).
- No schema change.

## Tests

- `managed_cloud::app_link`: the website's link verbatim; every refusal; a
  look-alike host stays its own host.
- `project_sync::app_link`: reading and the inbox; a project and an item
  described; against the stub service, each account on the host asked once,
  a signed-out account still listed, another origin's token never sent, and
  no request for a host with no account.
- `storage_catalog` cloud flow: a saved repository is found by row and item.
- `tests/frontend/app-link.integration.test.tsx`: the routing table, the
  stops, the waiting rule, the sheet, the pages opened with an address, and
  what the hook and the verb are allowed to call.
- `tests/release-config.test.mjs`: the scheme and the plugins' wiring.

## Live pass (the owner's)

Install a build, then, signed in to Mallard Cloud:

1. Website › a repository's page › *Open in Mallard*: Mallard comes forward
   with the sheet. Repeat with Mallard closed.
2. Each row of the table above; a private repository of another account;
   Safari, Chrome and Firefox.
3. With a Push review open: the link waits; **Open** shows the sheet.
4. Windows: with Mallard running and not; a second launch does not open a
   second window.

## Not done / open

- The Windows and Linux branch (single-instance, `register_all`) compiles
  only on those targets and was not built here.
- A Catalog row's own Push or Pull review is not visible to the shell, so a
  link's sheet opens over it; the sheet's verb then leaves it, as a sidebar
  click does.
- The website: *Didn't open?* fires 1.5 s after the click unless the page
  lost focus, which a browser's own *Open Mallard?* prompt may outlast; and
  the Pull guide still leads with the steps by hand.
- A development build and an installed one both claim `mallard://`; the
  system picks one.

## Layers

1. **Repository links**: this.
2. **Session links.** `/owner/name/sessions/{id}` → the session's page when
   it is here, the project's *Only in Mallard Cloud* rows when it is not.
   Needs the website's button and its own plan.
3. **Not planned:** sign-in by link, invitation links, `https://` universal
   links.

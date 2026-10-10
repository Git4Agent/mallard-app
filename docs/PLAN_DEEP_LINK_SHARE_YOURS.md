# Plan: Share yours — a link from the listing into Mallard

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

**Status (2026-10-03): implemented on both sides**, uncommitted and not
deployed. The owner took all four recommendations at the end: straight to
the page, the kind chip steers, **Open Mallard** is the primary, and the band
says *Didn't open?* when no app answers. It builds on
`docs/PLAN_DEEP_LINK_DESTINATIONS.md` (repository links, sheet first).

Bars. Desktop: the infrastructure crate's address and link tests 7/7,
`project_sync::app_link` 3/3, frontend integration 559/559, `tsc` clean,
clippy clean on the touched Rust. Website: `vitest` 236/236, `tsc` clean.
Driven in the shipped shell on a stand-in backend (`/mockups/deep-link-live.html`:
`listing`, `listing-pets`, `listing-staging`, a dialog open, `?accounts=none`)
and on the local website (`localhost:5173/repositories`), where no app claims
the scheme, so a click ends at *Didn't open?*.

**Not run:** a real click from the website into an installed build (the
first plan's live pass covers it), and neither side is deployed.

Mocks, kept as the design record:

- The website's band: `mallardInternal` `web/share-yours-mockup.html` +
  `landing-mock/share-yours-mock.{tsx,css}` (`?theme=`, `?answers=yes|no`,
  `?kind=`, `?downloads=soon|out`).
- The desktop's arrival: `mockups/share-yours-mockup.html` +
  `mockups/share-yours-mockup.tsx` (`?scene=projects|catalog|welcome|no-account|
  other-host|waiting`, `?theme=dark`).

## What was built

Desktop:

- `managed_cloud/app_link.rs`: `AppLink` is a repository or a listing
  (`ListingKind`, the website's four `type` words). The listing is exactly
  `/repositories`; a `type` that is not one of the four, or given twice, is
  not a link; the listing's other filters are dropped.
- `project_sync/app_link.rs`: `AppLinkArrival::Listing { host, listing_kind,
  can_set_up }`. No command is added and the service is not asked.
- `app-link/appLink.ts`: `listingVerdict` (a stop, or straight on),
  `listingPage`, and `serviceHost`, which spells a storage's host as the
  backend spells a link's. The window counts the accounts itself, from the
  saved config.
- `useAppLink`: a listing link with an account on its host goes through
  `go` without a sheet; without one the sheet shows the stop. It waits like
  any link.
- `ProjectSync.tsx`: `goToLinkDestination` gains `projects` and `catalog`
  (`openProjectsIndex`, `openCatalogIndex`). A link set aside for a Mallard
  Cloud account is let through when the storage setup just saved is opened
  (`appLinkLandingRef`, as `cloudLandingRef` returns New project): the
  listing's goes to its page instead of the storage's; a repository's opens
  its sheet over it.

Website:

- `src/appLaunch.ts`: the hand-over and the 1.5 s focus test, moved out of
  `PullMenu.tsx`, which now uses it. `components/GetMallard.tsx` likewise.
- `pull.ts`: `listingAppLink(origin, type)`. A `type` that is not one of the
  four chips is left out rather than sent.
- `components/ShareYours.tsx`, mounted by `HomePage.tsx` with the chip that
  is on. One rule added to `styles.css`.

One thing differs from the plan below: `accounts` is not carried on the
arrival. The window already holds the storages, so it counts them.

## Where things stand

- The website's listing (`mallardInternal`, `web/src/pages/HomePage.tsx`,
  `.listing-share`) ends its banner with *Share yours — Push a project, skill
  or pet from the Mallard desktop app.* and one button, **How to push →**,
  which opens the docs page `push-and-pull`. It is drawn for everyone, signed
  in or not, at `/` and `/repositories`.
- The desktop answers one link, `mallard://open?address=<a repository's
  page>`. The listing's address is refused: *Mallard can't open that link.*
- The app has a page for each half of that sentence: **Projects** (one card
  per project; empty, the welcome with its two doors) and the **Catalog**
  (one row per home and kind). Push is a verb on a project's or a row's own
  page.

## What changes for the person

On the listing, the band gets a second button, **Open Mallard**. It brings
Mallard forward on **Projects**: pick a project, then **Push**. With one of
the listing's Skills, Plugins or Pets chips on, it opens the **Catalog**
instead. Nothing is pushed by the link.

## The link

`mallard://open?address=https://mallard-ai.com/repositories[?type=…]`

- The same form as today: one verb, one parameter, the page's own address.
  The listing is the second page the app knows.
- `type` is the listing's own filter (`project`, `agent_skill`,
  `plugin_install_intent`, `desktop_pet`), sent only when a chip is on. Any
  other query key is dropped; any other value makes it not a link.
- The website always sends `/repositories`, also from `/`, so the app reads
  one spelling.

## Where it lands

| The link's `type` | Mallard opens |
| --- | --- |
| none, or `project` | Projects |
| a Catalog kind | Catalog |

Before that, in order:

1. Not this form → *Mallard can't open that link.* (the sheet, as today).
2. No Mallard Cloud storage signs in to the link's host. This build's
   service → the sheet: *Sign in to Mallard Cloud to share.* with **Set up
   Mallard Cloud**; a new account then lands it. Any other host → *No account
   here for {host}.* Both are decided on this computer; nothing is asked of
   the service.
3. The shell is in the middle of something (an operation, a Push or Pull
   review, a dialog) → it waits under the bar: *Link · Projects is waiting.*
   with **Open**.
4. Otherwise it goes to the page.

### Sheet or no sheet (decision 1)

A repository link opens a sheet because there is something to say: what the
service found and where it stands here. The listing's link has nothing to
find.

- **A · Straight to the page (recommended).** The sheet appears only when
  the link cannot land (rules 1 and 2). One rule: *the sheet is for a link
  that needs an answer*.
- **B · Sheet first, as repository links.** *Open from Mallard Cloud*, a line
  naming the page, one verb (**Open Projects** / **Open Catalog**). Uniform,
  and one more click each time for a sheet with nothing on it.

A third way was weighed and dropped: a sheet listing every project and
Catalog row to pick from. It is the Projects page and the Catalog index drawn
again in a dialog.

## Website (`mallardInternal`)

1. **The band** (`HomePage.tsx`): **Open Mallard** as the primary button,
   **How to push →** beside it as the plain one. On a phone or tablet (no
   desktop app; `canOpenApp` in `PullMenu.tsx`) the band stays as it is.
2. **No app answered.** The page kept focus for 1.5 s (the test `PullMenu`
   already makes): the band's line becomes *Didn't open?* with the *Get
   Mallard* link (*soon* until `LAUNCH.downloads` is set) and **How to
   push →** stays. No popover.
3. **Shared launch.** `openLink` and the focus wait move out of
   `PullMenu.tsx` into one helper both buttons use; `appLink` in `pull.ts`
   takes a page address, and the repository page passes its own.
4. **Tests** (`web/test/pull.test.ts`): the listing link with and without
   `type`; the repository link unchanged; the band's markup on desktop and on
   a phone.

The band is on production today. The button can ship before a build that
answers it is out: it falls to *Didn't open?*, as the repository page's
button does now.

## Desktop (`mallard-app`)

1. **Reading** (`managed_cloud/app_link.rs`): `AppLink` becomes a repository
   or a listing (`host`, `kind: Option<…>`). The listing is exactly
   `/repositories`; `type` is read against the four values. `/docs`,
   `/reviews`, `/invitations`, `/privacy` and a profile stay refused.
2. **Arrival** (`project_sync/app_link.rs`): `AppLinkArrival::Listing { host,
   kind, accounts, can_set_up }`, where `accounts` is how many storages sign
   in to the host, read from the config. No request, so `resolve_app_link` is
   not called for it.
3. **Routing** (`app-link/appLink.ts`): the listing's verdict is a stop (rule
   2) or a destination, `projects` or `catalog`. `followAppLink` gains two
   cases, `openProjectsIndex()` and `openCatalogIndex()`; nothing else in the
   shell changes.
4. **Waiting**: the same `linkWaits`; the line names the page.

No command is added, no schema changes, and the link still only reads and
navigates.

## Tests

- Rust: the listing with and without `type`, each bad `type`, the other
  top-level pages refused; arrival counts accounts on the host and sends
  nothing.
- Frontend: the two destinations, the two stops, the waiting line's words,
  and the gate on what `followAppLink` may call, with the two routes added.
- Website: above.

## Layers

1. This: Projects, or the Catalog by kind.
2. Later, if wanted: a Catalog kind opens that kind's row when there is
   exactly one, instead of the Catalog index.

## Decisions for the owner

1. **Sheet or no sheet** for this link. Recommended: A, straight to the
   page, the sheet only for the two stops.
2. **Does the kind chip steer?** Recommended: yes — Skills, Plugins or Pets
   on opens the Catalog; otherwise Projects. The alternative is Projects
   always.
3. **The band's buttons.** Recommended: **Open Mallard** primary, **How to
   push →** plain. The alternative keeps *How to push* primary and adds
   *Open Mallard* as the plain one.
4. **When no app answers.** Recommended: the band's line turns to *Didn't
   open?* with *Get Mallard*. The alternative sends the reader to the docs
   page.

## Not checked

- Whether the repository page's *Open in Mallard* button is on production;
  only the band was seen there.
- The 1.5 s focus test against a browser's own *Open Mallard?* prompt: still
  open from the first plan's live pass.

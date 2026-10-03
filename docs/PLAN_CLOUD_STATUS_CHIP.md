# Plan: the Mallard Cloud status chip

Owner decisions, 2026-09-27, made on the specimens `mockups/cloud-status-mockup.html`
(the choices) and `mockups/cloud-status-b-mockup.html` (the picks):

- **B**: the path bar's Cloud chip shows a composite mark (the cloud with a
  badge) and the state's word, and above a 960px bar content width a chevron.
  At 960px and below it folds to the mark; the word stays in its tooltip and
  its accessible name.
- **Cloud with a badge**: one mark per state, drawn in the icon set.
- **Split rows**: every link row in the chip's panel opens its page, and the
  button at the row's end copies its address.
- **Every state on the chip**, including a repository that is gone or being
  deleted, a sign-in that is needed, a refused account and a service that
  cannot be reached. The workspace notice under the bar stays as it is.

## What the user sees

The chip is a button. It opens a panel under it with:

1. the mark, the word, and the repository (`owner/name`), or the storage's
   name when the status carries no repository;
2. one line saying what the state means;
3. for a ready repository, four facts: Draft, Public, Review, You. A
   contributor also reads that the Owner decides what is released;
4. the verbs: Request public release (sent at once, as before), or View release request while a
   request waits or was turned down; View draft; View published version; then
   Discard draft… in danger ink. The first three rows copy their link
   (release, draft, published) from a button at their end, and the button
   says Copied for two seconds;
5. for a status with no repository, the notice's one control (Retry or
   Storage settings) or none (gone, being deleted).

The project menu loses View draft, View published version, Request public
release and Discard draft. Its caption is then the agent profile alone.

Before the first status arrives, the chip is present and inert ("Checking…").
This was offered in the picks specimen as a proposal. It keeps the bar from
shifting when the answer lands.

### Words, in order of precedence

The chip shows the first that applies.

| # | Status | Word | Mark | Tone |
|---|---|---|---|---|
| 0 | no status yet | Checking… | cloud | inert |
| 1 | `sign_in_required` | Sign in | cloud-user | warning |
| 2 | `inaccessible` | No access | cloud-alert | warning |
| 2 | `unavailable` (incl. a failed comparison) | Can't check | cloud-alert | warning |
| 2 | ready, publication unavailable | Can't compare | cloud-alert | warning |
| 3 | `deleting` | Being deleted | cloud-off | warning |
| 3 | `gone` | Gone | cloud-off | warning |
| 4 | release rejected | Rejected | cloud-x | danger |
| 5 | waiting for the Owner or an Admin | In review | cloud-clock | info |
| 6 | public, draft newer than the public version | Draft v*N* | cloud-dot | plain |
| 7 | public, draft is the public version | Published v*N* | cloud-check | success |
| 8 | private | Private | cloud-lock | plain |
| 9 | nothing pushed | Not pushed | cloud | plain |

Rows 2 through 9 cover the ready states. The notice under the bar and
`cloudSyncBlocked` are unchanged.

## Design

- `managedCloudLink.ts` owns the words: `cloudHeadline(status)` (the table
  above), `cloudReviewWord(link)`, `cloudFacts(status)`, `cloudReleaseUrl(link)`
  (`{website_url}?tab=release`, the website's Release tab), and
  `cloudVerbs(status)`, which gains `viewRelease`. `releaseChip` and
  `cloudStatusChip` are removed.
- `SyncPathBar.tsx` gains `PathBarPopover`: a button in the bar's actions
  whose panel shares the bar's single open-menu state. A crumb menu and the
  panel are never open together. It also shares the bar's outside click,
  Escape (consumed, so a Push/Pull review stays open) and focus return. The
  bar stays domain-free: the caller hands it the face, the names and the
  panel.
- `ManagedCloudChip.tsx` (project-sync) renders the chip and its panel from
  the status and the workspace's handlers. It copies through the shipped
  `writeClipboardText`. Each split row is two buttons, so the panel's verbs
  are a group the Tab key walks, not a `role="menu"`.
- `Icons.tsx` gains `cloud-dot`, `cloud-check`, `cloud-lock`, `cloud-clock`,
  `cloud-x`, `cloud-alert`, `cloud-user` and `cloud-off`. Each is the shipped
  cloud with a badge bitten out of its corner, or a slash for `cloud-off`,
  measured at 31–47 cells of ink. The badge path takes `--icon-badge`
  (currentColor by default), so the chip tints it by tone.
- `ProjectLinksWorkspace.tsx`: the Cloud rows leave the project menu. The chip
  renders for every Mallard Cloud project and passes the same handlers the
  rows had (open, request release, discard), plus Retry and Storage
  settings.

## Safety

UI only. The copied addresses are the repository's website pages, the same
ones View draft and View published version already open. No credential,
path or token reaches the clipboard. Discard keeps its system-dialog
confirmation, and Request public release keeps its revision check.

## Tests

- `cloudHeadline` for every row of the table, and its precedence.
- `cloudVerbs.viewRelease`, `cloudReleaseUrl`, `cloudFacts` and
  `cloudReviewWord`.
- The panel's static markup:
  - a copy button per link, with the right name and address;
  - the release row switching between Request and View;
  - the storage name and control for a status with no link;
  - no copy button on Discard.
- The project menu no longer carries the Cloud verbs, and the chip renders
  for a non-ready status.
- `PathBarPopover` shares the bar's open state (source and markup).
- `npm run build` and `npm run test:frontend-integration`.

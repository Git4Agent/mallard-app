# Filter the activity list by session status

- Status: the owner picked **C, the scope bar**, on 2026-10-03 ("Go with C").
  Implemented the same day, uncommitted. Open: a pass in the running Tauri
  app. The plan below is kept as written; *As built* at the end records what
  landed and where it differs.
- Scope: the standing activity list of a project (`ProjectChatHistoryContent`
  outside a review), Git and non-Git. Push and Pull reviews, the Files face,
  the Library and the path bar are not changed.
- Visual contract: `mockups/session-filter-mockup.html`
  (`?dir=all|a|b|c|d|e&kind=git|flat&theme=light|dark`). The list in every
  frame is the shipped page on static data. C's frames are now the shipped
  page whole, with its own scope bar; in the other four the search row and
  the control are drawn by the specimen.
- Screens: [all five side by side](images/session-status-filter/compare-all.png)
  ([dark](images/session-status-filter/compare-all-dark.png)), then every
  state of [A](images/session-status-filter/a-glyph-chips.png),
  [B](images/session-status-filter/b-named-chips.png),
  [C](images/session-status-filter/c-scope-bar.png),
  [D](images/session-status-filter/d-status-menu.png) and
  [E](images/session-status-filter/e-show-bar.png).

## The problem

Every row leads with a status glyph, and the list can only be narrowed by
name. To find the sessions that still need a Push, or the two the path bar's
attention chip counts, the reader scans the glyph column row by row.

## What the references ask of a list filter

| Reference | What it asks | What this plan does |
|---|---|---|
| [Apple HIG, search fields](https://developer.apple.com/design/human-interface-guidelines/search-fields) | A scope bar narrows a search among clearly defined categories. Open on the broad scope and let people narrow it. | C is a scope bar. Every direction opens on everything. |
| [Material 3, chips](https://m3.material.io/components/chips/guidelines) | Filter chips are short words in a compact row; several can be on at once; a selected chip is marked. One set is single-select or multi-select, never both. | B. A is the same set without the words. |
| [Primer, filter pattern](https://primer.style/product/scenario-patterns/filter/) | Size the control to the task: a menu or a few exposed options for a small known set. Put it at the top of what it governs, show the selection on the trigger, pair each count with a label, never mark "active" by colour alone, give an empty result a way back, and announce the new count. | Counts sit beside names. The active scope carries an underline and heavier ink, not only a colour. The empty box offers *Clear filter*. The row's live count reads "3 of 12 sessions". |
| [Carbon, filtering](https://carbondesignsystem.com/patterns/filtering/) | Apply at once when one category is filtered. While a filter is in force, show it and let it be cleared in one step. Start all-off or all-on by whether people mostly include or exclude. | No Apply step. *All* (C) or × (A, B) clears. A to D start all-off; E starts all-on. |
| [NN/g, applying filters](https://www.nngroup.com/articles/applying-filters/) | Instant filtering suits a list that answers within a second; each option should say how many results it holds. | The loaded list is in memory. Every option carries its count. |
| [ARIA APG, button](https://www.w3.org/WAI/ARIA/apg/patterns/button/) | A toggle is a button with a pressed state whose label does not change when pressed. | C is a radio group. A and B are `aria-pressed` buttons. |

Products that ship each form:

| Form | Seen in |
|---|---|
| A, glyph toggles with tallies | [VS Code Problems](https://code.visualstudio.com/docs/getstarted/userinterface), the [Chrome DevTools Console](https://developer.chrome.com/docs/devtools/console/reference) sidebar |
| B, named chips | [Gmail search chips](https://workspaceupdates.googleblog.com/2020/02/gmail-search-chips-ga.html), [Jira quick filters](https://support.atlassian.com/jira-software-cloud/docs/show-or-hide-issues-on-your-board/) |
| C, scope bar | Mail and Finder scope bars, the Open / Closed switch heading [GitHub issue lists](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/filtering-and-searching-issues-and-pull-requests) |
| D, one button and a check menu | [Mail's Filter button](https://support.apple.com/guide/mail/mlhl1f6cf15a/mac), [Linear filters](https://linear.app/docs/filters) |
| E, all on, press to hide | DevTools log levels, VS Code's Show Errors / Warnings / Infos |

## The five directions

| | Control | Rule | Costs | Cannot |
|---|---|---|---|---|
| A | A chip per status in the search row: glyph and count | Press to narrow; several at once; × clears | No extra line at normal widths | Name a status without a hover |
| B | The same chips, named, on a line under the field | Same as A | One 37px line | — |
| **C** | A scope bar under the field: All, To push, To pull, Attention, Up to date | One at a time; All is the way back | One 36px strip | Combine scopes |
| D | One *Status* button closing the search row, with a check menu | Tick to narrow; *Clear filter* in the menu | Nothing until opened; a click to look, a click to change | Show counts at rest |
| E | The session page's Show bar: every status on | Press to hide | One 44px box | Reach one status in fewer presses than there are other statuses |

**Recommendation: C.** A row has exactly one status, so the statuses split the
list into parts, and a scope bar is the standard control for choosing one
part. Its words are the path bar's own (Pull, Push, and the attention chip),
so the bar and the list say the same thing. It has one rule and no separate
clear control. What it gives up is combining: "everything except up to date"
needs B, which is the runner-up for that reason.

## What the user sees (C)

- **The strip.** Directly under the search row, in the review's underline
  strip: `All 12 · To push 3 · To pull 2 · Attention 2 · Up to date 5`. Each
  scope shows the row glyph it stands for and its count. *Attention* shows
  the path bar's warning triangle and groups the four statuses that chip
  already groups: changed on both sides, waiting, can't match, can't sync.
- **One at a time.** Pressing a scope keeps only its rows. *All* brings the
  list back. Nothing combines.
- **With the name search.** Both apply. The count reads "3 of 12 sessions"
  whenever either is in force.
- **The rail while narrowed.** The same as under a name search: a commit with
  no matching session leaves the rail, folds open and stop folding, and the
  empty-rail card is not drawn. *Load earlier* stays at the foot.
- **Nothing left.** If the chosen scope has no rows (after a Push, the three
  rows under *To push* become up to date), the scope stays, at 0, and the
  list shows the shipped "No matching loaded sessions." with a *Clear filter*
  button. The button appears only when clearing would bring rows back; a name
  that matches nothing keeps the shipped box as it is.
- **When there is no strip.** One status across every loaded row (a strip
  would narrow nothing), no comparison yet, a comparison that failed, no
  linked storage, and inside a Push or Pull review. A scope chosen before the
  strip went away is not applied while it is hidden.
- **Rows the comparison has no entry for** take a sixth scope, *Not
  compared*, only when they sit beside rows that have a verdict.

### Scope: the loaded rows

The strip counts and narrows the rows that are loaded, exactly as "Search
loaded sessions" does. The path bar counts the whole comparison plus files,
so the two can differ: in the specimen the bar says Push 9 and the strip says
To push 3, with twenty-four sessions behind *Load earlier*. The whole span
for one verb is already a page: the Push or Pull review. Narrowing past the
loaded window would mean loading every page first, and is not part of this.

## Implementation (C)

The other session's uncommitted work touches `ProjectActivityPage.tsx` (a
`reviewResourceIds` prop in the review filter). Every edit below is a
targeted one on top of it.

1. **One resolver for a row's status.** `SessionCard` picks its badge in
   three steps: `sync_blocked_reason`, then the comparison entry, then the
   page fallback. Lift that into `sessionRowPresentation(session, entry,
   storageName, fallback)` in `ProjectActivityPage.tsx` and call it from the
   card's non-review branch and from the filter, so a row's scope is by
   construction the badge it draws.
2. **One mapping from status to scope.** `statusScope(status)` returns
   `push | pull | attention | synced | unknown`. `activityHeaderCounts`
   switches on the same statuses to build the bar's attention count; rewrite
   that switch to call `statusScope`, so the chip and the scope cannot
   disagree.
3. **State.** `const [scope, setScope] = useState<StatusScope>("all")` beside
   `localQuery` in `ProjectChatHistoryContent`. It lives as long as the
   search text: the component is keyed on project, storage and review mode.
   The scope in force is `"all"` whenever the strip is not drawn.
4. **Narrowing.** Add the scope test to the `sessions` memo, after the name
   test. Introduce `narrowing = searching || scope !== "all"` and use it
   where `searching` now decides how the list is drawn: the two unmapped
   folds, each commit's fold and its `disabled`, the `commits` memo (bare
   commits drop), the commit section's condition, the flat list's empty
   line, and the "No matching loaded sessions." box.
5. **The strip.** New `src/components/sync-review/StatusScopeBar.tsx`: a
   `role="radiogroup"` of `role="radio"` buttons on the shipped
   `.sync-project-tabs` strip, with arrow-key movement copied from
   `SyncReviewTabs`. It takes `{ id, label, count, glyph }[]` and the active
   id, and imports no feature DTOs.
6. **The count.** `NameSearchField` reads "N of total" only while a query is
   typed. Add a `narrowed` prop so it reads that way whenever the page says
   the list is narrowed.
7. **Placement.** The strip follows the search row in both layouts: after
   `searchField` in a Git project, and after the `.sync-list-head` row in a
   non-Git one (ahead of the notice zone, where that row already sits).
8. **CSS**, beside `.sync-name-search` in `App.css`: the `.sync-status-scopes`
   rules from the specimen (wrap, 34px buttons, readable resting ink, count
   style), the hand-over of the field's rule to the strip
   (`:has(+ .sync-status-scopes)`, already used elsewhere in the file), and
   the empty box's button spacing.

No Rust, no DTO and no Tauri command changes.

### If another direction is picked

- **B or A:** state is a `Set` of statuses instead of one scope; steps 1, 2,
  4 and 6 are the same. `NameSearchField` gains a `filters` slot (B: a line
  under the field, inside its rule; A: beside the count, wrapping with it).
  New chip CSS from the specimen, and a × that clears while narrowed.
- **D:** B's state, in a `trailing` button on the search row, with the path
  bar's menu and `menuitemcheckbox` rows.
- **E:** the set holds what is hidden; the bar is `SessionDetailToolbar`'s
  markup with a status glyph for the tick box.

## Tests

In `tests/frontend/project-activity.integration.test.tsx`:

- no strip with one status, with no comparison, with a failed comparison, and
  in a Push or Pull review;
- each scope's count equals the number of rows drawing that scope's badges,
  including a `sync_blocked_reason` row under *Attention*;
- *Attention* over the loaded rows equals `activityHeaderCounts(...).attention`
  for the same entries;
- choosing a scope narrows the rows, drops bare commits, opens the folds, and
  reads "3 of 12 sessions";
- scope and name both apply;
- a chosen scope with no rows stays at 0, and the box offers *Clear filter*
  only when clearing brings rows back;
- a scope chosen before the comparison went away is not applied.

CSS is checked by computed style in the mounted page, not by source text:
strip height, the field's rule handed to the strip, and text contrast at
rest and active in both themes (the strip's text measured 5.4:1 or better in
the specimen).

Handoff: `npm run build` and `npm run test:frontend-integration`.

## Not in this change

- **The path bar's attention chip selecting the *Attention* scope.** It is
  the natural next layer, and needs the scope lifted to
  `ProjectLinksWorkspace`.
- The Files face and the Library, which could take the same strip later.
- Narrowing beyond the loaded window, and remembering the scope across
  restarts.

## Open questions

1. ~~Which direction.~~ C, picked 2026-10-03.
2. The scope names: *To push*, *To pull*, *Attention*, *Up to date*, *Not
   compared*. Shipped as written; not confirmed by the owner.

## As built (2026-10-03)

| Step | Landed in |
|---|---|
| One resolver for a row's badge | `sessionRowPresentation` in `ProjectActivityPage.tsx`; `SessionCard` and the scope bar both call it |
| One mapping from status to scope | `sessionStatusScope`, exported; `activityHeaderCounts` counts push, pull and attention through it |
| State and narrowing | `ProjectChatHistoryContent`: `narrowing = searching \|\| statusScope !== null` replaces `searching` wherever the rail keyed on the name search |
| The strip | `src/components/sync-review/StatusScopeBar.tsx`, a radio group on the `.sync-project-tabs` strip; arrows, Home and End move the choice with focus |
| The count | `NameSearchField` takes `narrowed` |
| CSS | `App.css`, after `.sync-name-search-hidden` |
| Tests | four in `project-activity.integration.test.tsx` (576 of 576 pass) |

Differences from the plan:

- **The scope can be held by the host.** `ProjectChatHistoryContent` takes
  `statusScope` and `onStatusScopeChange`, and holds the scope itself when
  they are absent, the same shape as `searchQuery`. The suite renders static
  markup and cannot press a control, so the tests choose a scope through the
  prop; the specimen presets its C frames the same way. The page wrapper
  does not pass them, so the app's scope is the list's own state.
- **The strip's rule takes the field's tone** (`--border`), not the review
  strip's darker one, so the head's line does not change when the bar
  appears.
- ***Clear filter* moves focus to the search field** before the box it sits
  in leaves, as the field's own clear does.
- **CSS was checked by computed style in the specimen's shipped frames**, not
  by a committed test, because the suite has no DOM: strip 36px, 12px to the
  list, the field's rule handed over in both layouts, two lines and no
  overflow at 580px with six scopes, text 5.4:1 or better in both themes.
  Scope presses, arrow keys, the name search, and *Clear filter* were driven
  on the same frames.
- The specimen lost its *Before* direction: the page it mounted now has the
  bar.

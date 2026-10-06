# Filter the Files face by status

- Status: the owner picked **F1** and **deleted files count as To pull** on
  2026-10-06. Implemented the same day, uncommitted. *As built* at the end
  records what landed. Open: a pass in the running Tauri app.
- Scope: the Files face of a non-Git project (`ProjectFilesActivityView`) and
  the path bar's file tallies. The Sessions face, the Push and Pull reviews,
  and the Library get no new control.
- Visual contract: `mockups/files-filter-mockup.html`
  (`?theme=light|dark&only=files|spacing|refs`). Every frame is the shipped
  page on static data.
- Screens: [Files face](images/files-status-filter/files.png)
  ([dark](images/files-status-filter/files-dark.png)),
  [spacing fix](images/files-status-filter/spacing.png).

## Why the Files face has no filter

The gap is deliberate, not a regression. `PLAN_SESSION_STATUS_FILTER.md` lists
"The Files face and the Library, which could take the same strip later" under
*Not in this change*. `StatusScopeBar` was only ever wired into
`ProjectChatHistoryContent`, and `git log -S StatusScopeBar` finds nothing in
`ProjectFilesActivityPage.tsx`.

## Decisions (2026-10-06)

- **F1: the Sessions face's scope bar, with folders kept.** The strip is the
  same: All, To push, To pull, Attention, Up to date, plus Not compared when
  a file has no verdict. One scope is active at a time. A folder tile with no
  file in the scope leaves the list. The cloud tile still comes first. Tile
  facts count the files shown.
- **A file deleted here is a file to pull.** This is the `missing` state:
  synced once, deleted on this computer, still in storage. It keeps the Pull
  arrow and its place in the cloud tile. It joins *To pull*, and the bar's
  Pull count now includes it. In the specimen, the bar goes from Pull 3 to
  Pull 4.
- **Declined:**
  - F2: one merged list while a scope is in force;
  - F3: one scope shared by both faces;
  - deleted files under Attention;
  - a separate glyph for deleted files.

## Spacing fix (landed)

The Files page spaces its children with a 12px flex gap and already strips the
search field's margin. When the field sits inside a `.sync-list-head`, the
head kept its own 12px margin on top of that gap. The `.sync-list-head` wraps
the field when the face's view switch or the review's Rescan sits beside it.
The result was that the first tile sat 24px under the search rule, on both the
Files face and the Push/Pull files step, against 12px on Sessions.

The fix extends the existing rule:
`.sync-project-files-page > .sync-name-search, .sync-project-files-page > .sync-list-head { margin-bottom: 0; }`.
Computed style in the specimen now reads 12px on the Files face, on the review
step, and on Sessions. `npm run build` passes, and
`npm run test:frontend-integration` passes 591 of 591.

## Implementation

1. **One mapping.** Add `fileStatusScope(entry)` beside
   `projectFileSyncCounts` in `ProjectFilesActivityPage.tsx`:

   | Entry | Scope |
   |---|---|
   | blocked (`blocked_reason` or `blocked`) | attention |
   | `local_only`, `local_ahead` | push |
   | `storage_only`, `storage_ahead`, `missing` | pull |
   | `diverged` | attention |
   | `synced` | synced |
   | anything else | unknown |

   Rewrite `projectFileSyncCounts` to count through `fileStatusScope`, and
   update its doc comment. Two counts change:
   - a deleted-here file now counts as a Pull;
   - a blocked file no longer counts as a Push.
2. **One vocabulary.** Move `SESSION_STATUS_SCOPES` out of
   `ProjectActivityPage.tsx` into a resource-neutral `STATUS_SCOPES` in
   `sync-review/`. Move `SessionStatusScope` with it and rename it
   `StatusScope`.
3. **Narrowing.** Change the signature to
   `buildProjectFilesActivityProjection(fileEntries, query, scope)`. Apply the
   scope before the name test and before `PROJECT_FILE_CARD_RENDER_LIMIT`.
   Change the limit line to "Search or choose a status to narrow the list."
4. **The strip.** In `ProjectFilesActivityView`, render
   `<StatusScopeBar label="Show files by status" noun={fileNoun} …>` after
   `NameSearchField`, and pass `narrowed`. Follow the Sessions face's rules:
   - show the strip with two or more scopes, or while a scope is chosen;
   - a chosen scope stays at 0;
   - offer *Clear filter* only when clearing brings rows back.

   The scope is the view's own state, so it resets when the face unmounts,
   the same way its search does. Like Sessions, the view also takes an
   optional `statusScope` and `onStatusScopeChange`, so static-markup tests
   can preset a scope.
5. **CSS.** Add `.sync-project-files-page > .sync-status-scopes.sync-project-tabs { margin: -12px 0 0; }`
   beside the spacing rule. The strip pulls up flush under the field whose
   rule it takes over, and the page gap spaces it from the first tile. The
   specimen measures 0px and 12px.
6. **Confirm Pull restores a deleted-here file.** The code reads that way:
   - the bundle engine plans a `WriteProjectFile` for every project file in
     the snapshot;
   - the Pull review reads a write with no `expected_target_sha256` as
     "Add file";
   - `recommendedPullSelection` pre-ticks every add.

   Pin it with a backend test: delete a synced file locally, plan a Pull, and
   expect one `WriteProjectFile` with no expected target digest.

No DTO or Tauri command changes.

## Tests

In `tests/frontend/files-activity.integration.test.tsx`:

- no strip when every file is in one scope, or when the inventory is not
  eligible;
- each scope's count equals the number of rows it draws, including a blocked
  file under Attention and a deleted-here file under To pull;
- the push and pull scopes equal `projectFileSyncCounts` for the same
  inventory, with the deleted-here file in Pull;
- a scope reaches files past the card budget;
- scope and name both apply, and tiles with no match leave;
- a chosen scope with no rows stays at 0, with *Clear filter* only when
  clearing brings rows back.

Rust: the Pull-planning test in step 6.

Handoff: `npm run build`, `npm run test:frontend-integration`, and
`cargo test --manifest-path src-tauri/Cargo.toml --lib`.

## As built (2026-10-06)

| Step | Landed in |
|---|---|
| One mapping | `fileStatusScope` in `ProjectFilesActivityPage.tsx`; `projectFileSyncCounts` counts through it |
| One vocabulary | `src/components/sync-review/statusScopes.tsx` (`StatusScope`, `STATUS_SCOPES`); the Sessions face imports it |
| Narrowing | `buildProjectFilesActivityProjection(fileEntries, query, scope)` |
| The strip | `ProjectFilesActivityView`, with the optional `statusScope` / `onStatusScopeChange` pair |
| CSS | `App.css`: the list-head rule beside `.sync-project-files-page > .sync-name-search`, and the strip rule after the `.sync-status-scopes` block |
| Pull restores a deleted-here file | `a_file_deleted_here_is_planned_back_by_pull_as_an_add` in `service.rs` tests: one `WriteProjectFile` for the deleted file, no expected target digest |
| Tests | `files-activity.integration.test.tsx`: the tally test (a deleted-here file in Pull, a blocked new file out of Push), the scope reaching past the card budget, and three strip tests |

Differences from the plan:

- The limit line names the status only while the strip is drawn: "Search or
  choose a status to narrow the list." with the strip, "Search to narrow the
  list." without it.
- The strip is not drawn for an empty folder, even with a scope chosen.

Verified on the shipped component in the specimen: scopes narrow and tiles
leave; a name that matches nothing keeps *Clear filter* away; switching faces
resets the scope; *Clear filter* moves focus to the search field; strip
margins compute to −12px top and 0 bottom, giving 0px under the field and
12px to the first tile.

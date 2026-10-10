# Push footer refusal

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: the owner picked **C · Warning pill**, and it is implemented
(uncommitted, 2026-10-02). What shipped is listed under "As built (C)" at the
end; the sections before it are the plan as proposed.

Specimen: `mockups/push-blocker-mockup.html` + `mockups/push-blocker-mockup.tsx`
(`npm run dev`, then `/mockups/push-blocker-mockup.html`). It uses the real header,
tabs, file cards, counts, gauge and Push button on the shipped CSS; only the
proposed rules are mock CSS in the page's `<style>`. The cards are live, so
ticking a row or pressing *Mark reviewed* changes the footer.

## Problem

In the owner's screenshot (non-Git project, Project files tab, 7 files to add)
the Push footer shows "1 selected item can't be captured — exclude it to
push." squeezed into a column about 70px wide and five lines tall. Push is
disabled.

## Findings (verified in code)

1. **Why the column is narrow.** The sentence is
   `<span role="status"><small>` in `PushResourceWorkspace`'s footer, and
   `.sync-inline-action-footer > span` (App.css) gives it `flex: 1`, which
   means a flex-basis of 0. Next to it, `.sync-project-file-commit-impact` is
   `flex: 1 1 360px; margin-right: auto`. At a 970px review the free space is
   about 140px, split between the two, so the sentence gets about 70px.
   Measured in the specimen: 68×73px, and the footer is 82px instead of 44px.
   At 414px the footer is 117px, and 214px when the review is also over the
   limit.
2. **Running notes have the same bug.** The running note in `PUSH_RUNNING_NOTE`
   (Push, and `SessionPushReview`) and "Pull cannot be cancelled…" in
   `RestorePlanView` use the same span next to the same counts block. The
   Catalog footers use the same span too. They have no counts block, so they
   aren't squeezed, but the text sits far to the left of the button it
   explains.
3. **The copy names the wrong fix.** `pushReviewBlockers` gives the kind
   `local` to two different causes:
   - an item the capture can't read (`unavailable`, `blocked`, `unknown`);
   - a selected file with an unacknowledged scanner warning
     (`warning_unacknowledged`, from *Executable* or *Possible secret*).

   The second is fixed by *Mark reviewed* on the file's card. The footer tells
   you to exclude the file instead. With 7 files on the Files tab, the second
   cause is the likely one in the screenshot. That's an inference: the
   screenshot can't prove it.
4. **The review can open already blocked.** `selected_after_scan` (in
   `service.rs`) is "newly discovered, not blocked, not excluded". Scanner
   warnings don't opt a file out, so a new `run.sh` is ticked by default and
   the review opens with Push disabled.
5. **Two strings disagree.** The footer builds its own sentence.
   `pushReviewDisabledReason` returns "Exclude the items that can't be
   captured", and that string is the title of both the footer's Push and the
   path bar's Push.
6. **Zero counts take space.** "0 Replace · 0 Delete files · 0 Delete
   folders" says nothing, and the block reserves 360px (a whole row at ≤720px)
   whether or not it needs it. Dropping only the zeros frees nothing; the
   block also has to be sized to its content.

## Directions

All three use the same copy (one helper), and the measurements below include
the compact counts.

- **A · Join the refusal row** (recommended). The blocker and the running
  note move into the row the over-limit sentence already uses: its own row
  above the button cluster, right-aligned so the sentence ends over Push.
  The blocker line is warning-coloured with a triangle, and keeps
  `role="status"`.
- **B · A plus Show.** The sentence ends with *Show file* / *Show session*,
  which opens the tab holding the first blocker and scrolls its card into
  view. It's the only direction that helps when the blocker sits on the tab
  you aren't on; A relies on that tab's warning dot.
- **C · Warning pill.** No sentence row. A pill such as "⚠ 1 needs review"
  sits in the cluster, shaped like the gauge pill, and pressing it does B's
  Show. The full sentence goes in its title and in a visually hidden status.
  The running note still uses A's row.

Proposed copy (also Push's title in both places):

- one flagged file: `run.sh is executable — mark it reviewed or exclude it.`
  (or `.env.local may hold a secret — …`)
- several: `3 files need review — mark them reviewed or exclude them.`
- one unreadable item: `“Fix login redirect” can’t be captured — exclude it to push.`
- both causes at once: the two sentences, one after the other.

Footer height in px (Before / A / B / C), measured in Chromium:

| Scenario | 970 review | 684 (1100 window) | 414 (800 window) |
|---|---|---|---|
| Opened blocked (screenshot) | 82 / 67 / 67 / 44 | 66 / 67 / 67 / 44 | 117 / 109 / 109 / 85 |
| Blocker on the other tab | 82 / 67 / 67 / 44 | 66 / 67 / 67 / 44 | 117 / 109 / 109 / 85 |
| Over limit + flagged file | 90 / 91 / 91 / 67 | 88 / 91 / 91 / 67 | 214 / 133 / 133 / 109 |
| Flagged file + deletions | 82 / 67 / 67 / 44 | 79 / 103 / 103 / 79 | 135 / 149 / 149 / 125 |
| Pushing | 96 / 67 / 67 / 67 | 66 / 67 / 67 / 67 | 117 / 109 / 109 / 109 |

At 684px, Before's 66 hides a three-line squeezed sentence. Without the
compact counts, A measures 89 there, because the 360px counts slot claims a
row of its own. Warning ink on the footer measures 5.12:1 in light and
9.60:1 in dark.

## Recommendation

**A with compact counts.** It's the smallest change that fixes the layout,
the copy and the two disagreeing titles. It reuses the row grammar the footer
already has for the over-limit refusal, and it puts the instruction right
above the button it explains. The footer goes back to 44px as soon as the
file is marked reviewed.

The cost is height. A adds a line for the sentence, so with the deletion
approval or an over-limit sentence it is up to about 24px taller than Before
(103 vs 79 at 684). C is the only direction that is never taller than Before.
Pick **C** if a footer that never grows matters more than reading the
instruction without hovering. **B's Show** can be added on top of A later; it is its own
layer (see below).

## Implementation (A)

1. **One sentence.** Add `pushBlockerSentence(blockers, threadComparison,
   projectContentInventory)` to `PushResourceWorkspace.tsx`, beside
   `pushReviewBlockers`. It splits `warning_unacknowledged` from the
   unreadable states and names a single item:
   - a file: the basename of `relative_path`, plus its warning code
     (`executable` → "is executable", `possible_secret` → "may hold a
     secret", anything else → "has a scan warning");
   - a session: `ThreadSyncEntry.display_name`.

   `pushReviewDisabledReason` returns it in place of "Exclude the items that
   can't be captured", so the path bar's Push title follows through
   ProjectSync. Delete `localBlockerSentence`.
2. **One footer note.** Add a `ReviewFooterNote` primitive in
   `src/components/sync-review/`. It renders
   `<p className="sync-review-footer-note {tone}">` with an optional
   `role="status"` and an optional severity icon, and replaces every review
   footer sentence:
   - `PushResourceWorkspace`: over-limit, blocker, running note;
   - `SessionPushReview`: refusal, running note;
   - `RestorePlanView`: over-limit, running note;
   - `StorageCatalogPage`: the four spans in its Push and Pull footers. These
     aren't squeezed today; the swap keeps one pattern. **Not mocked:** this
     moves the Catalog sentences from the left of the footer onto their own
     row above the button.

   Keep each line's current announce behaviour: over-limit and the Catalog's
   "Acknowledge the warnings…" stay silent, the blocker and the running notes
   stay `role="status"`.
3. **CSS.** Rename `.sync-review-budget-sentence` to
   `.sync-review-footer-note` (`flex: 1 0 100%; order: -1`, right-aligned,
   `--type-xs`, tone modifiers for danger, warning and neutral). Then delete
   `.sync-inline-action-footer > span` and the footer `small` rule if nothing
   else uses them (grep first).
4. **Compact counts.** `ProjectFileCommitImpact` prints only non-zero counts.
   That also applies to Pull, which shares the component. Its block becomes
   `flex: 0 1 auto`. It keeps the 360px slot, and the full row at ≤720px,
   only while it shows the deletion approval: a modifier class from the
   component, rather than `:has`. Otherwise the step button separates from
   the gauge and Push. The specimen hit exactly that before the rule was
   added.
5. **Tests** (in `push-budget.integration.test.tsx` and
   `project-files.integration.test.tsx`):
   - a flagged file is named, and the sentence says "mark it reviewed";
   - the footer line and both Push titles read the same string;
   - acknowledging the warning clears the line and re-enables Push;
   - an unreadable session keeps "can't be captured";
   - the refusal-markup assertions move to the new class (push-budget 246
     and 634, pull-review 952);
   - zero counts are absent;
   - a source gate: no review footer renders `<span role="status"><small>`.
6. **Verification.**
   - `npm run build` and `npm run test:frontend-integration`.
   - Computed-style measurement of the real footers at 970, 684 and 414 in a
     harness, because source-text tests miss cascade bugs.
   - A live Tauri pass with a non-Git project holding a new executable file.

No Rust, DTO or schema change. No safety invariant is relaxed: warning
acknowledgement still blocks Push, and only its wording changes.

## B and C deltas

- **B:**
  - Cards carry `data-resource-id` (`ProjectFilesReviewPage` `renderCard`
    and the session rows).
  - A `focusResourceId` handoff from `PushResourceWorkspace` selects the
    blocker's tab and scrolls the card into view.
  - If the page's own search hides the card, or it falls past
    `PROJECT_FILE_CARD_RENDER_LIMIT` (500), the search must be set to the
    file's path. The search state is internal to the page today, so it would
    need lifting or a prop.
- **C:** a `.sync-review-blocker-pill` button in the cluster, with the same
  focus handoff as B. The sentence goes in its title and in a visually hidden
  `role="status"`. The running note uses A's row.

## Open question (separate from this fix)

Should newly discovered files with a scanner warning start unticked? That
would mean changing `selected_after_scan` in `service.rs`. It would stop the
review opening blocked, but it changes the default selection rule (a Rust
change, plus a backend test). It's the owner's call.

## As built (C)

- `pushBlockerSentence` and `pushBlockerPillLabel` in
  `PushResourceWorkspace.tsx`. The sentence is the pill's title, the
  footer's visually hidden status (always mounted, empty when nothing
  blocks), and `pushReviewDisabledReason`, so both Push buttons' titles
  read it. "Exclude the items that can't be captured" and the footer's own
  count sentence are gone.
- `.sync-review-blocker-pill` sits between the step button and the gauge.
  Pressing it selects the first blocker's tab (one whose row the review lists
  wins) and hands `ProjectFilesReviewPage` a `reveal` request. The page:
  - sets the search to the card's path if the search or the 500-card limit
    hides it;
  - scrolls the card into view (`block: "nearest"`);
  - focuses *Mark reviewed*, or the row's tick if there's no warning;
  - washes the card in the warning tint for 1.6s.

  Changing tabs by hand drops the request.
- `.sync-review-footer-note` (renamed from `.sync-review-budget-sentence`,
  `--type-xs`, a `danger` modifier) holds the over-limit refusal and the
  running notes in Push, session Push and project Pull. Catalog footers are
  unchanged.
- `ProjectFileCommitImpact` prints non-zero counts only. It is sized to
  content; `with-approval` keeps the 360px slot, and a full row at ≤720px.
  The redundant `:has` wrap rule is gone.
- Real-component footer heights in Chromium (970 / 684 / 414):
  - opened blocked: 44 / 44 / 85;
  - with deletions: 44 / 79 / 123;
  - over the limit: 66 / 66 / 108;
  - pushing: 66 / 66 / 108.
- Measured contrast: pill ink 5.09:1 (light) and 6.88:1 (dark).
- Known gap, not fixed: a selected file in the `unknown` state is hidden
  from the Push files list but still blocks Push. The sentence now names
  it, but there is no row to exclude, so the pill can only switch tabs.

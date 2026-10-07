# Plan: delete the draft of a Mallard Cloud repository, on both sides

- Status: picked on 2026-10-06 ("Go with A1 and B, keep Discard, implement
  both") and implemented the same day in both trees, uncommitted. *What was
  built* at the end says where the code differs from this text.
- Owner's words (2026-10-06): "I want both the mallard cloud and desktop to
  be able to delete a draft, plan and mock for both desktop and cloud"; then,
  shown the specimens, "Go with A1 and B, keep Discard, implement both".
  Decisions 1, 3 and 6 were not named; the recommended answers stand (R10 as
  what the act leaves, the Owner and Admins by the service's one rule, the
  website asks in place).
- Scope: who may remove a draft, in which states each side offers it, what
  the control says, how it asks, and what it leaves. One flag in the service's
  answers; the desktop's Cloud chip panel; the website's repository page.
  Project repositories and Catalog items alike. No schema changes anywhere.
- Specimens, both uncommitted: the app's `mockups/delete-draft-mockup.html`
  (screens in `docs/images/delete-draft/`) and the website's
  `mallardInternal/cloudflare/managed-cloud/web/delete-draft-mockup.html`
  (screens in `mallardInternal/output/delete-draft-mock-2026-10-06/`).
- Related: `PLAN_CLOUD_VERSIONS.md` (R10: what a discard keeps),
  `PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md` (part B: the discard as built,
  R10 to R16), `PLAN_CLOUD_STATUS_CHIP.md` (the panel that holds the verb),
  `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` sections 4.3, 4.5 and 5.7,
  `mallardInternal/…/STORE_API.md` section 3.

## Words

| Word | Meaning |
|---|---|
| Draft | The repository's latest push. It moves with every Push. As today. |
| Discard draft (or Delete draft: decision 2) | The one act this plan is about: the draft's pushes leave Mallard Cloud. What stays is decision 1. |
| Nothing to remove | The draft is the latest version, or nothing is pushed. The act has no work to do. |
| Kept version | The version the draft returns to: the latest one. |

## What is there today

Read in both trees on 2026-10-06. Nothing was run. The versions work of
2026-10-05 is committed in both trees and not deployed.

| # | Fact | Where |
|---|---|---|
| 1 | The service has the route. The Owner and Admins may call it. It returns the draft to the latest version, whatever the visibility; with no version it removes every push and the repository stays, empty (R10 of the versions plan). It answers `400 invalid_request` "There is no draft to discard." when the draft is the latest version or nothing is pushed. | `src/routes/content.ts` `discardDraft`; `STORE_API.md` section 3 |
| 2 | The status the desktop reads carries `can_publish_version`, computed by the service. Nothing of the kind for the discard: the desktop decides by role and visibility on its own. | `src/routes/store.ts` `streamStatus`; `managedCloudLink.ts` `canDiscardDraft` |
| 3 | The website offers it in Settings, to the Owner and Admins, as a panel of its own between Visibility and Remove repository. The panel exists only while there is something to remove; otherwise nothing on the page says the act exists. It asks in place with a second click, and says "Draft discarded. The draft is version 2 again." or "… The repository is empty." | `web/src/pages/RepositoryPage.tsx` `SettingsTab`; `web/src/discardDraft.ts` |
| 4 | The desktop offers it in the Cloud chip's panel as the row **Discard draft…**, to the Owner alone, on a public repository alone, while the draft is past the latest version. It asks in the system's dialog. Afterwards the standing notice says "Draft discarded — Mallard Cloud holds version 2. Pull, then push again." or "— Push creates a new draft." | `managedCloudLink.ts` `canDiscardDraft`, `discardDraftQuestion`; `ManagedCloudChip.tsx`; `clearedStorageNotice.ts` |
| 5 | Those gates are the owner's words of 2026-09-26: "if there is only private snapshot A1, no public repo A, then user has to go to website to discard"; "Admin can only discard on the website". The versions plan changed what is kept and left the gates as they were. | `PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md` R15, R16; `PLAN_CLOUD_VERSIONS.md` *Desktop* |
| 6 | Neither side shows the act when there is nothing to remove: no row, no panel, no reason. Publish, in the same two places, stays in view and off with its reason ("Nothing new since version 3"). | `ManagedCloudChip.tsx`; `RepositoryPage.tsx` `publish` |
| 7 | The Library's items read the same status and draw the same panel, and their website page has the same Settings tab. | `StorageCatalogPage.tsx`; `RepositoryPage.tsx` |
| 8 | After a discard the desktop needs no new behavior: its bases are stale, the next comparison reads them as replaced or missing, and the standing notice says so. Pull brings the kept version; Push creates a new draft. | `clearedStorageNotice.ts`; `PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md` part B |

### Where the ask is not met today

| The repository | Website | Desktop |
|---|---|---|
| Private, nothing published, a draft | Settings: Discard draft, every push | nothing |
| Private, the draft past version 2 | Settings: Discard draft, back to 2 | nothing |
| Public, the draft past version 3 | Settings | the Owner's row; an Admin has nothing |
| Private or public, the draft is the latest version | nothing, and no word why | nothing, and no word why |
| Nothing pushed | nothing | nothing |

Every repository is private until its first release, so the desktop offers
the act in none of the common states. The website has it in a tab three
clicks from the draft, and hides it in the one state that most often follows
a Publish. That is what "neither side can delete a draft" looks like from a
project that was just published as version 3.

## The rules

- **R1. One rule, the service's.** The status the desktop reads and the
  summary the website reads both carry `can_discard_draft`: the account is
  the Owner or an Admin, something is pushed, there is something to remove
  (no version, or the draft past the latest version), and no removal is
  pending. Each side offers the act when the flag is true and never
  otherwise. The desktop's own test by role and visibility goes.
- **R2. Offered wherever the draft is.** Private or public, with or without
  a version, a project or a Catalog item.
- **R3. In view, off, with the reason.** Whoever is the Owner or an Admin
  sees the control in every state. While there is nothing to remove it is
  off and its title says why: "Nothing to remove: the draft is version 3",
  or "Nothing pushed yet". As Publish does on both sides.
- **R4. What it leaves is R10 of the versions plan** (decision 1): the
  draft returns to the latest version; with no version every push goes and
  the repository stays, empty. Every version keeps its content.
- **R5. It asks once, in the words of what goes and what stays**, with
  Cancel and the verb, never OK or Yes. The desktop asks in the system's
  dialog, as its fourteen other destructive verbs do; the website asks in
  place (decision 6).
- **R6. Afterwards each side says what happened, as today.** The website's
  notice; the desktop's standing notice under the bar. Nothing is rolled
  back; Pull brings the kept version; Push creates a new draft.
- **R7. One word on both sides** (decision 2), in the row, the question,
  the button and the notices.

## Metadata changes

None. Nothing portable, nothing in `~/.mallard`, nothing in D1.

## API changes

| Route | Change |
|---|---|
| `GET …/store?prefix=<the stream>` (the desktop's status) | `link` gains `can_discard_draft` (R1), beside `can_publish_version`. |
| `GET /v1/repositories/{id}` and by-name | The summary gains `can_discard_draft`, for whoever may read the draft. |
| `POST …/private-snapshot/discard` | Unchanged. |

**Desktop commands**: unchanged. `discard_managed_cloud_draft` stays as it
is; only who sees the row changes.

## What the user sees

### App: direction A1 of the specimen

The Cloud chip and its headline do not change. In the panel:

| State | Row today | Row after |
|---|---|---|
| Private, nothing published, a draft | — | **Discard draft…** · every push |
| Private, the draft 3 pushes past version 2 | — | **Discard draft…** · back to version 2 |
| Private, the draft is version 3 | — | Discard draft… off · "Nothing to remove: the draft is version 3" |
| Private, as an Admin (decision 3) | — | as the Owner |
| Public, the draft 1 push past version 3 | **Discard draft…** | the same |
| Public, the draft is version 3, which is public | — | off · "Nothing to remove: the draft is version 3" |
| In review, the draft asked for | — | **Discard draft…**; the question adds "The open release request is withdrawn." |
| Nothing pushed | — | off · "Nothing pushed yet" |
| A Member, a contributor | — | — |

The question, in the system's dialog, titled **Discard draft?**:

> Discard the draft of "rika/portal"?
>
> Removes every push after version 2 from Mallard Cloud. The draft returns
> to version 2. Every version stays.
>
> Nothing is deleted on this computer. This cannot be undone.
>
> Cancel · Discard

With no version the middle paragraph reads "Removes every push from Mallard
Cloud. There is no version to return to: the repository stays, empty." With
an open request that names the draft, "The open release request is
withdrawn." follows the first sentence.

Afterwards, the notice under the bar as today: "Draft discarded — Mallard
Cloud holds version 2. Pull, then push again." or "Draft discarded — Push
creates a new draft."

### Website: direction B of the specimen

The act moves to where the draft is listed: the Versions tab's draft line,
at the end, beside Publish. Settings loses its panel (decision 5).

| State | The draft's line, after |
|---|---|
| Private, nothing published | `Draft · No version yet` … **Publish version 1** · Discard draft… |
| The draft 3 pushes past version 2 | `Draft · 3 pushes since version 2` … **Publish version 3** · Discard draft… |
| The draft is version 3 | `Draft · Same as version 3` … Publish off · Discard draft… off, "Nothing to remove: the draft is version 3" |
| Nothing pushed | `Draft · Nothing pushed yet` … Publish off · Discard draft… off, "Nothing pushed yet" |
| A Member, a contributor, a visitor | no control |

Pressing it asks in place, in a warning notice under the panel's head:
"**Discard the draft?** Removes every push after version 2 and its unused
cloud content. The draft returns to version 2. Every version and local files
stay. Push is blocked until deletion finishes." with **Discard** and
*Cancel*. Afterwards the success notice: "Draft discarded. The draft is
version 2 again." or "… The repository is empty.", and the draft's line reads
`Same as version 2`.

## Service

- `streamStatus` (`src/routes/store.ts`): `can_discard_draft` =
  `(access.role === "owner" || access.role === "admin") && pushed &&
  (latest === null || ahead !== 0) && removing === null`. It is what
  `discardDraft` would accept, minus the revision it has not seen.
- `repositorySummary` (`src/db/queries.ts`): the same flag from the same
  facts, for whoever may read the draft; false otherwise.
- Nothing else. `discardDraft` already refuses everyone and everything the
  flag excludes.

## Website (`web/src`)

- `discardDraft.ts`: `draftDiscard(summary)` answers for the Owner and
  Admins in every state: `{ on, hold, keptVersion, sentence, done }`, `on`
  from the flag, `hold` the reason while off. Null for everyone else.
- `components/VersionList.tsx`: a `discard` slot beside `publish` on the
  draft's line, and the ask under the head.
- `pages/RepositoryPage.tsx`: the control and the ask on the Versions tab;
  the Settings panel goes; the success notice stays where it is shown now.
- `types.ts`: `can_discard_draft` on `RepositorySummary`.
- `versions.css`: the ghost danger button in `.ver-end`; the ask row in the
  notice under the head.
- `versions-harness.tsx`: the stub answers the flag.

## Desktop

- `src/components/managed-cloud/types.ts`: `can_discard_draft` on
  `ManagedCloudLink`.
- `managedCloudLink.ts`: `canDiscardDraft` reads the flag (and
  `cloudPinMoved`, as Publish does). `cloudVerbs.discardDraft` becomes the
  row's two facts: offered (Owner or Admin) and on. `discardDraftHold` gives
  the reason while off. `discardDraftQuestion`: the no-version sentence ends
  "the repository stays, empty."
- `ManagedCloudChip.tsx`: the row draws off with its title, as Publish does.
- `crates/mallard-infrastructure/src/managed_cloud/client.rs`: the field on
  `ManagedCloudLink`; `stub.rs` answers it from its state.
- `clearedStorageNotice.ts`: unchanged under decision 2a; the two titles
  change under 2b.

## Documents

- `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` 4.3 ("The desktop offers the
  discard to the Owner alone, and only over a public version") and 5.7:
  one rule, the service's flag.
- `PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md`: a line under R15 and R16
  saying this plan supersedes them.
- `STORE_API.md` section 3: the status's `link` fields.
- The website's docs pages where Settings' panel is named.

## Tests

Service (vitest on Miniflare):

1. The stream's status answers `can_discard_draft` for each row of the
   app's table above: true on a private repository with no version and with
   the draft ahead, for the Owner and for an Admin; false while the draft is
   the latest version, with nothing pushed, for a Member, for a contributor,
   and while a removal is pending.
2. The summary answers the same flag; a visitor's summary has it false.
3. An Admin's discard on a private repository is accepted (today's test
   covers a Member's refusal and a contributor's).

Website (vitest, and the harness):

4. `draftDiscard` for the Owner and an Admin in every state: on with the
   sentence, or off with the hold; null for a Member and a contributor.
5. The Versions tab draws the control on the draft's line for the Owner,
   off with its title when there is nothing to remove, and not for a Member;
   the ask appears under the head and Cancel removes it.
6. Settings has no Discard draft panel.

Desktop:

7. `canDiscardDraft` follows the flag alone: true on a private repository
   for an Admin; false when the flag is false whatever the role and
   visibility; false while another repository is read in the link's place.
8. `cloudVerbs` offers the row to the Owner and an Admin in every ready
   state, on or off; never to a Member or a contributor.
9. The panel's markup: the row off carries the hold as its title; the three
   question texts.
10. The Rust parity test over the stub reads the flag for each state, and
    the stub refuses what the flag excludes.

## Decisions

Each is the owner's. The recommendation is marked.

1. **What the act leaves.**
   - (a) **Recommended.** As today, R10 of the versions plan: back to the
     latest version; with no version, nothing. The service does not change.
     While the draft is the latest version the control is off: the version
     already holds what the draft holds, and Pull brings it.
   - (b) Every push goes and the versions stay, so that a draft that is
     version 3 can be removed too. It costs a rule the model does not have:
     a version whose push is gone from the draft's history. `draftAhead`,
     the discard's "not part of the draft's history" refusal, the draft's
     line on both sides and the Release tab's choices all read that chain.
     And no desktop can Pull a version yet (*Later* in the versions plan),
     so after it the versions are readable on the website alone until
     someone pushes. Not recommended.
2. **The word.**
   - (a) **Recommended.** *Discard draft*, as both sides say today. Under
     1a the act is a return to the latest version, which is what discard
     means (Gmail's Discard draft, Primer's "discard changes"); the notices
     "Draft discarded" and the documents stay as they are.
   - (b) *Delete draft*, the owner's word (Apple Mail's Delete Draft,
     GitHub's Delete this release). Right under 1b. Under 1a it says more
     than happens: the draft's line then reads "Same as version 3". The
     specimen draws both; the change is a word in the row, the question,
     the button and the two after-notices on each side.
3. **Who, on the desktop.**
   - (a) **Recommended.** The Owner and Admins, one rule with the website
     and the service (R1). This reverses the owner's 2026-09-26 word "Admin
     can only discard on the website", which was said when the act removed
     everything on a private repository; under the versions model it
     returns the draft to a version, and the website already lets an Admin.
   - (b) The Owner alone on the desktop. The flag still travels; the
     desktop adds `role === "owner"`.
4. **Desktop: where and how.**
   - **A1, recommended.** The panel's row, on every repository, off with its
     reason; the system's dialog asks.
   - A2: the row, and a sheet of the app's own with the facts as rows.
   - A3: the row turns into the question inside the panel.
   - B: a pill in the project's menu, beside Delete all local sessions.
   - C: the row opens the website.
5. **Website: where.**
   - **B, recommended.** The draft's line on the Versions tab, beside
     Publish; the ask under the head; Settings' panel goes.
   - A: Settings, as today, shown always and off with the reason.
   - C: an overflow menu on the content panel's head.
   - D: inside Settings' red Danger zone.
   - F: the rail's Draft card.
   - B and A both: two homes for one act. Not recommended.
6. **Website: how it asks.**
   - (a) **Recommended.** In place, a second click, as Settings asks today
     and as GOV.UK's two-step wants.
   - (b) E: a confirmation dialog, as Primer, Carbon and GitHub's release
     page do.

## Later

- The Owner removes a version. Today a Super Admin does, through the API.
  Its own plan.

## What was built (2026-10-06)

Everything under *Service*, *Website*, *Desktop* and *Documents* above, with
these differences from the text:

- The summary's `can_discard_draft` reads no removals table: a removal in
  flight is the discard's own refusal (`repository_discarding`), and the page
  reloads after it. The stream status reads it, as Publish's flag does.
- The desktop keeps `cloudVerbs().discardDraft` as one boolean (the row is
  offered: the Owner or an Admin) and adds `discardDraftHold` beside
  `canDiscardDraft`, mirroring Publish's three functions. A status without
  its publication offers the row, off, with no title.
- The website's success and error notices sit where "Published version N"
  sits, above the tabs; the ask sits under the panel's head in a warning
  notice. Settings lost its panel. The versions harness answers the discard
  and the flag for the Owner (it has no Admin viewer).
- The docs page *Manage a repository* names the Versions tab and both roles.

Checks run: the service's typecheck and 221 tests; the website's typecheck
and 273 tests; the desktop's `tsc`, `cargo check --tests`, the frontend
integration suite and `cargo test --lib`; the website flow on the harness in
the browser pane (ask, Discard, "Draft discarded. The draft is version 2
again.", both buttons off with their reasons); the real `ManagedCloudPanel`
over the ten states of the app specimen (on, off with the reason, or absent).
Open: a live pass against a deployed service, which the versions work still
waits for too.

## Steps

0. The owner's picks on decisions 1 to 6. Done: A1, B, Discard; 1, 3 and 6
   as recommended.
1. Service: the flag in the status and the summary; tests 1 to 3;
   `STORE_API.md`.
2. Desktop: the flag, the rule, the row, the question; tests 7 to 10. Build,
   the frontend suite, the Rust suites.
3. Website: `draftDiscard`, the Versions tab, Settings; tests 4 to 6; the
   harness.
4. Documents named above. Live pass with the versions deployment the owner
   still has to run; this plan needs no reset of its own.

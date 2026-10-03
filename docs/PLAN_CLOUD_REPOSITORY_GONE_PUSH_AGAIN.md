# Mallard Cloud: a repository gone or being deleted, a draft discarded — push again, no relink

- Status: implemented on 2026-09-26, on the owner's word ("Go and implement
  PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md. Use your pick for the UI
  change"), after "DON'T IMPLEMENT YET" earlier the same day. Steps 1 to 11
  are done, in the desktop tree and in the service tree. Step 12 is the
  owner's and is open: reset and deploy staging, the live pass, production.
  *Implementation notes* at the end list what differs from the text as it
  was written.
- Date: 2026-09-26
- Owner's words (2026-09-26): "if repo is gone, user should allow to repush
  instead of relink"; "if repo is gone or being deleted"; "there is no
  customer"; "I also want user to be able to discard private snapshot too …
  public repo A, private snapshot A1, user can discard A1"; "Both side should
  be able to discard"; "if there is only private snapshot A1, no public repo
  A, then user has to go to website to discard"; "still only owner can
  discard"; "D7 is not good, After the discard A1 or A1 published, session
  should be compared to A"; "Admin should be able to discard too"; "D8 run
  immediately"; "Admin can only discard on the website"; of the Pull before
  a Push after a discard, "it's fine"; of a repository made private after a
  release, "this should be treated as private repo"; of R5, after its
  examples, "I would like the rule".
- Scope, part A: the desktop's managed store, the project Push and Pull
  save, the Cloud status DTO, the workspace notices, the test stub, the
  Library's item repositories, and one rule of the service (a repository
  being deleted stops counting).
- Scope, part B: the service's discard (who may, what it leaves, when it
  takes effect), a discard control on the website and one in the desktop's
  project menu, and the desktop's wording after a discard.
- Constraints: no customers. Clean cutover on both sides: no migration, no
  compatibility between an old desktop and a new service or the reverse. The
  desktop's schemas stay 1 and the link document gains no field
  (`managed_cloud_repository_id` already exists). The service's schema is
  edited in `migrations/0001_init.sql` in place, and the remote databases are
  reset as the service's README describes. Nothing on the desktop is reset.
- Related: `DESIGN_MANAGED_CLOUD_STATUS.md`, `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`
  (the rule this plan extends to Mallard Cloud), `PLAN_MANAGED_CLOUD_CAS_FREE.md`
  (decision 8, sections 5.8, 6.5, 7, 11 and 13), `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md`
  (sections 4.3, 4.5 and 5.8), `mallardInternal/cloudflare/managed-cloud/STORE_API.md`.
  "What this reverses" lists every passage of them that changes.
- Words: the documents say "private snapshot"; the website and the desktop
  say "draft". They are the same thing: the repository's pushes, of which the
  latest is what collaborators read.

## Lineage

A project link on Mallard Cloud saves the exact repository it syncs with,
`ProjectStorageLink.managed_cloud_repository_id` (the pin). The pin was added
so a link never drifts to another repository of the same bundle. The status
design then made a vanished destination a hard stop: "Deletion or lost access
is an error, not an empty snapshot. A pinned link does not select or create a
replacement repository when its destination disappears."

On a user-owned storage the opposite rule shipped on 2026-09-22: a cleared
storage reads as empty, stale bases read as absent, nothing is reset, and the
user pushes again. Mallard Cloud is the one storage where a vanished history
still ends at Unlink.

Discard was designed with the service and built there, as a website action
that clears the draft. Its control was never built, and what it leaves on a
public repository was never looked at from the desktop.

## What happened on 2026-09-26

| Time (PDT) | Desktop status | Service state behind it |
|---|---|---|
| 8:29:38 – 8:31:04 p.m. | Being deleted | `deleting`: a collaborator's request answers `410 repository_removed` |
| 8:34:14 – 8:34:50 p.m. | Repository unavailable | `removed`: every request answers `404 not_found` |

The removal job runs on the five-minute cron, which fits the gap. In both
states Push and Pull are disabled, every session wears `?`, and the only
control is Retry, which asks the same repository again.

# Part A. Gone or being deleted

## What is wrong today

| # | Fact | Where |
|---|---|---|
| 1 | The store resolves a pinned bundle to the pin, always. The table and the by-bundle lookup are never reached. | `crates/mallard-infrastructure/src/managed_cloud/store.rs:408` (`repository_for`) |
| 2 | The engine opens with the pin of every link, on every open. | `src-tauri/src/project_sync/service.rs:14425` |
| 3 | A stream listing that answers `410` or `404` records `Deleting` or `Inaccessible` and returns an error. The comparison fails, so every row falls back to `unknown`. | `store.rs:769-785`, `client.rs:1207`, `service.rs:1534-1547`, `ProjectActivityPage.tsx:675` |
| 4 | Any Cloud status other than `ready` disables Push and Pull. The notice offers Retry, or Storage settings for a sign-in. | `ProjectLinksWorkspace.tsx:1304`, `:2473` (Pull), `:2529` (Push), `:2582-2589` |
| 5 | The store creates a repository only when the scope resolves to none. With a pin it never does, so a Push can never create the repository again. | `store.rs:588-605` (`publish_manifest`) |
| 6 | The pin is written at three places, all of them linking: save link, connect, finish setup. Push and Pull never write it. | `service.rs:14687`, `:14855-14889`, `:16578`; Push save `:10715-10817`; Pull save `:13400-13455` |
| 7 | Once removal has completed the service already supports the recreate: the create guard ignores the `removed` row, the name is free, and by-bundle lists active repositories only. | service `src/routes/repositories.ts:116-126`, `:465-481`; `migrations/0001_init.sql:135`; `src/jobs/removal.ts:110-120` |
| 8 | While the repository is `deleting` the service does not: the create guard counts it (`storage_managed_account_exists`), its name is held, it counts toward the account's limit, and the address lookup loads it. Every one of these reads `state <> 'removed'`. | service `src/routes/repositories.ts:86`, `:123`, `:133`, `:152`, `:361`; `src/db/queries.ts:90`; `migrations/0001_init.sql:135` |
| 9 | `404` is ambiguous by design: deleted, or the account lost access. `403`, `5xx` and an unreachable service are distinct. | `client.rs:608-636`, service `src/db/queries.ts:231-239` |
| 10 | By-bundle answers the account's own repository first, else an open public one of any account. | service `src/routes/repositories.ts:465-481` |
| 11 | Library item repositories have no pin but share the defect: the store never forgets an id it learned, so an item deleted on the website while the app runs fails its next refresh of that stream and its next Push, until restart. | `store.rs:401-430`, `:365-380` |

### Measured on the stub, 2026-09-26

A temporary test in `store.rs`'s `stub_tests`, removed after the run. Each
case pushes once, removes the repository from the stub, then lists and pushes
again through the same store.

| Case | Listing | Push again | Requests after the removal |
|---|---|---|---|
| Project, pinned | error "Repository unavailable. It was deleted or you no longer have access."; status `Inaccessible` | error `404 not_found`; no repository created | the listing and the manifest `PUT`, both on the old id |
| Library item, id remembered | error `404 not_found` | error `404 not_found`, at its first listing | the listing on the old id |

Both the project engine and the Library engine begin every operation with a
stream listing (`bundle_engine.rs:816`, `resource_engine.rs:300`, `:432`), so
the listing is where a gone repository is first met.

## The rule

- **R1. Gone reads as empty.** When the first page of a stream listing
  answers `404 not_found` or `410 repository_removed`, the stream reads as
  empty. For a pinned project the status is `gone` or `deleting`, and both
  behave alike. The comparison succeeds. Stale bases and stale acceptances
  read as absent, by the rules that already serve a cleared storage. No code
  path resets, rolls back or relinks.
- **R2. Nothing else is gone.** `403` stays `inaccessible`. `401`, `5xx`, a
  timeout and a malformed answer stay what they are. None of them reads as
  empty, and all of them keep Push and Pull closed.
- **R3. Push creates it again.** With nothing to adopt (R5), the manifest
  write of a Push creates the repository from the manifest, the request the
  first link made. Objects, views, manifest, claim and record go to the new
  repository. One claim request and one record request, as today.
- **R4. Push and Pull save the pin.** The transaction that saves the bases
  also saves the repository the operation used, on every link of that storage
  and bundle. A refresh saves nothing. A Push that failed saves nothing.
- **R5. Adopt only a repository this account collaborates on.** After a gone
  answer the listing asks the service once (by-bundle, by-library). The
  answer is read only when its stream listing reports a role for this account
  (`link.role` is not null): another computer of this account created it
  again. A public repository the account does not belong to is never adopted.
  The stream then reads as empty and a Push creates the account's own.
- **R6. The pin is asked first, every time.** Each first-page stream listing
  of a pinned bundle asks the pin before anything else. A pin that answers
  again is read again and the fallback is dropped. The extra request exists
  only while the pin is stale, and ends with the next Push or Pull.
- **R7. A refused create fails the Push.** After a gone answer,
  `storage_managed_account_exists` means another computer created the
  repository since the listing. The Push fails with `REMOTE_CHANGED`, "The
  storage changed while pushing. Compare again." The next listing adopts it
  (R5).
- **R8. A repository being deleted stops counting (service).** From the
  request that confirms its removal, a repository no longer blocks a create
  for its bundle or its Library item, no longer holds its name, and no longer
  counts toward the account's limit. The address belongs to the active
  repository of that name. The removal job is unchanged: it erases the old
  repository's own prefix, `repos/<old id>/`, which the new one never shares.
- **R9. The Library forgets.** An item repository the store learned that
  answers gone is forgotten. The item's stream then resolves as in a process
  that never knew it, under R5, and its Push creates the repository again.

### R5 by example

A link remembers one exact repository. R5 answers one question: when that
repository is gone, may the link follow another repository of the same
project? "Collaborates on" means the account is its Owner, an Admin or a
Member. The desktop learns it from the role the service states in the
listing's answer; no role, no collaboration.

| # | The story | Without R5 | With R5 |
|---|---|---|---|
| 1 | Rika has one computer. She removes `rika/portal` on the website. | | No other repository of this project exists. The link reads empty, "Repository gone". Push creates `rika/portal` again. |
| 2 | Rika has two computers, both linked to `rika/portal`. She removes it on the website. On computer A she pushes, which creates `rika/portal` again, a new repository. Computer B still remembers the old one. | B reads empty. Its Push tries to create the repository, and the service refuses: the account already has one for this project. B is stuck, and the way out is Unlink. | B finds that the account has a repository for this project, and that Rika is its Owner. B follows it: "Created again", A's sessions listed. Pull, then Push. |
| 3 | Rika was a Member of Tom's private `tom/portal`, and her project carries that repository's identity. Tom removes her. Her next Push creates her own private `rika/portal` (O3). Later Tom's repository becomes public. Later still Rika removes `rika/portal`. | The service is asked for a repository of this project and answers Tom's: it is public, and Rika has none of her own. The link moves to `tom/portal` without a word. Rika's next Push puts her sessions into the draft of Tom's public repository. | The answer states no role for Rika, so the link does not follow it. It reads empty, "Repository gone". Push creates `rika/portal` again, private, on her own account. |

Story 2 is why a gone pin may follow another repository at all. Story 3 is
why it may follow only one the account collaborates on. To push to Tom's
public repository Rika connects to it herself, by its address, as today.

## What the user sees

| Service answer | Status | Notice | Rows | Push | Pull |
|---|---|---|---|---|---|
| `410 repository_removed` | `deleting` | **Being deleted** — Push creates it again. | each session reads its own state; a session never pushed elsewhere reads local only | open | open, nothing to pull |
| `404`, nothing to adopt | `gone` | **Repository gone** — Deleted, or this account lost access. Push creates it again. | the same | open | open, nothing to pull |
| `410` or `404`, a repository adopted | `ready` | **Created again** — `owner/name` was deleted and created again. Pull, then push again. (The last sentence only when Push is not open.) | compared with the adopted repository | by the scopes | open |
| `403`, suspended account | `inaccessible` | unchanged | `?`, unchanged | closed | closed |
| `401`, `5xx`, unreachable | unchanged | unchanged | `?`, unchanged | closed | closed |

After the Push: the notice ends by itself, the chip reads `Private · Not
requested`, and **View draft** opens the new page. The address is the old one
when this account owned the old repository and had not renamed it on the
website. A renamed one comes back under the project's name.

While a pin is stale and a repository is adopted, the chip and **View draft**
describe the adopted repository. **Request public release** is hidden until a
Push or a Pull has saved the pin, because it names the repository by the pin.

The Library page has no notice of its own for this. An item whose repository
is gone or being deleted reads as not in the storage, and its Push creates
the repository again.

# Part B. Discard the draft

## What is there today

| # | Fact | Where |
|---|---|---|
| 12 | The service has the route, with tests. Today any collaborator may call it: Member, Admin, Owner. A contributor to a public repository may not. | service `src/routes/content.ts:196-222` (`requireMember` at `:200`, the guard at `:208`), `test/contributors.test.ts:63-69`; architecture document section 4.3 |
| 13 | It queues a removal of target `chain` and answers at once. The worker, on the five-minute cron, deletes the stream's claims, then its records: every push, the released one included. Objects, views and manifests stay. | service `src/jobs/removal.ts:93-108`, `test/store.test.ts:436-470` |
| 14 | Nothing calls the route. The website's Settings has Make private and Remove repository, and its history never held a discard control. The desktop has no verb and its client no method. | service `web/src/pages/RepositoryPage.tsx:440-466`; `managed_cloud/client.rs` |
| 15 | The website's Settings tab is shown to every collaborator, and its content to the Owner and Admins. | service `web/src/pages/RepositoryPage.tsx:657`, `:776` |
| 16 | The desktop's verbs on a Cloud repository live in the project crumb's menu: View draft, View published version, Request public release. The release request is the model: a client method, a command that names the repository by the link's pin, an `api.ts` wrapper, a menu row. | `ProjectLinksWorkspace.tsx:1318-1360`, `api.ts:324`, `managed_cloud_link.rs:299-348`, `lib.rs:210` |
| 17 | The documented home of the control is the website: "Discard and removal are website actions with confirmation". | `PLAN_MANAGED_CLOUD_CAS_FREE.md:598` (section 11), architecture document section 5.8 |
| 18 | Until the worker runs, every desktop still reads the draft. A Push that lands before the worker runs is erased with the rest. By reading, not measured. | service `src/routes/content.ts:207-220`, `src/jobs/removal.ts:50-58` |
| 19 | A desktop reads the draft's history and nothing else. The public version's files stay stored after a discard, and no desktop looks for them. | `bundle_engine.rs:816` (`observe`), service `src/routes/store.ts:411-419` |
| 20 | A release publishes the push that is the head at that moment, and the public row names it: `source_publication_id` is that push's row, which carries its record's name. While a repository is public, its public version is one push of the draft's history. | service `src/routes/releases.ts:207-216`, `:243-249`; `migrations/0001_init.sql` (`publications`) |
| 21 | The service can already point its head at any push of the stream: `headStatements` retires the rest and makes that row available again. The reconciliation walk uses it. | service `src/store/index.ts:107-131`, `src/jobs/reconcile.ts:140-153` |
| 22 | Publishing changes nothing in the stream. After a release the desktop reads `public` and `matches_published`, and compares with the same push as before. | `store.rs` test `the_pinned_status_comes_from_the_stream_listing` |

### Measured, 2026-09-26: a discard on a public repository today

The service's own tests, run today: `test/release.test.ts`,
`test/store.test.ts` and `test/cleanup.test.ts`, 21 of 21 passing. The
desktop: its twelve Cloud store tests, passing, and a temporary test on the
stub in `store.rs`, removed after the run, which pushes version 1, releases
it, pushes version 2, discards, and pushes again.

| What | Before the discard | After the discard |
|---|---|---|
| Public version, read without signing in | served | served (`release.test.ts:173-174`) |
| The repository's latest push, `private_head` | the draft | none (`release.test.ts:175`) |
| Stored objects | all | what the public version references survives cleanup; the rest is residue after a day (`cleanup.test.ts:94-137`) |
| The desktop's stream listing | selects version 2 | selects nothing |
| The desktop's status | `ready`, public, `snapshot_changed`, `unpublished_draft`, draft 2, public 1 | `ready`, public, `not_pushed`, `no_draft`, draft none, public 1 |
| The desktop's chip | `Public version 1 · Unpublished draft · Not requested` | `Public version 1 · Not pushed` |
| Sessions on the desktop | compared with the draft | compared with nothing: every one reads local only, the ones in the public version too |
| Pull | the draft | nothing, the public version included |
| Push | onto version 2 | a first version again: the manifest, one listing, the claim of the root, the record, one listing |

The desktop's note after the discard is "Another computer cleared Mallard
Cloud", read from `clearedStorageNotice.ts`, not run.

This is what the owner called not good: after a discard the sessions are
compared with nothing, where they should be compared with A.

## The owner's cases

| Case | What a discard leaves | Website | Desktop |
|---|---|---|---|
| Public repository A with draft A1 | the draft is A again | Discard draft | Discard draft… |
| Draft A1 alone, no public version | nothing; the repository stays, empty | Discard draft | no verb; the user goes to the website |
| A repository made private after a release | as any private repository: nothing; the repository stays, empty | Discard draft | no verb; the user goes to the website |
| A1 published | there is nothing to discard: the draft is the public version | no control | no verb |

Public repository A, draft A1. The Owner discards A1 from either side, or an
Admin from the website:

| Step | Website | The computer that pushed A1 | A computer that last pulled A |
|---|---|---|---|
| Discard | the public version; "latest push not released" is gone | "Draft discarded", sessions compared with A: the ones A holds by their content, the ones only A1 held local only | in sync, no notice |
| Pull | | what A holds against this computer; with nothing to install, Mark as reviewed | |
| Push | a new draft on top of A | "Push complete"; chip `Public version N · Unpublished draft · Not requested` | reads ahead, Pull |

## The rule

- **R10. Discard is the Owner's verb on both sides, and an Admin's on the
  website, with a confirmation.** A Member and a contributor may not,
  anywhere. The service accepts the Owner and Admins; the desktop offers the
  verb to the Owner alone.
- **R11. On a public repository a discard returns the draft to the public
  version.** Every push after the released one is removed from Mallard
  Cloud. The released push and every push before it stay, with their claims.
  The desktop then selects the released push and compares with it.
- **R12. On a private repository a discard removes every push.** The
  repository stays, empty. A repository that was made private after a
  release is a private repository like any other: its old public version
  plays no part, its discard is on the website, and the push that was
  released goes with the rest.
- **R13. A discard takes effect in its request.** The cron finishes only
  what a request's budget left over.
- **R14. A discard removes nothing else.** No object, view or manifest, no
  public version, nothing on any computer. What only the removed pushes
  referenced becomes residue and is cleaned up after a day, as today.
- **R15. The desktop discards only over a public version, and only for the
  Owner.** Its verb is offered when the repository is public and its draft
  differs from the public version (`publication.state` is
  `unpublished_draft`), on a pin that answers.
- **R16. The website discards both.** Its control is offered to the Owner
  and Admins while there is something to discard: on a public repository a
  draft that differs from the public version, on any other a draft.
- **R17. The desktop says what happened.** On Mallard Cloud a pin that
  answers and bases that are stale read "Draft discarded", never "Another
  computer cleared". Nothing is reset. A computer that was on the removed
  pushes compares with what Mallard Cloud holds now, and pulls before it
  pushes, as after any history it has not reviewed.
- **R18. Publishing changes nothing in the draft's history.** After A1 is
  published the draft is the public version, and the desktop compares with
  it, as today (fact 22).

## What the user sees

Website, the repository's Settings, for the Owner and Admins:

> **Discard draft**, a panel of its own between Visibility and Remove
> repository, drawn as any other panel.
> On a public repository: Removes every push after public version N from
> Mallard Cloud. The draft returns to the public version. Nothing is deleted
> on any computer.
> On any other: Removes every push from Mallard Cloud. The repository stays,
> empty. Nothing is deleted on any computer.
> `Discard draft…` → in its place, "Discard the draft?" `Discard` · `Cancel`
> → "Draft discarded. The draft is public version N again.", or "Draft
> discarded. The repository is empty."
> While a release request is open, one more sentence: The open release
> request is withdrawn.

Desktop, the project crumb's menu, after Request public release, as one
more icon pill in danger ink (the `x` mark; its tooltip reads "Discard
draft… · back to public version N"):

> `Discard draft…` → the system's confirmation, titled **Discard draft?**:
> Discard the draft of "owner/name"?
> Removes every push after public version N from Mallard Cloud. The draft
> returns to the public version.
> Nothing is deleted on this computer. This cannot be undone.
> `Cancel` · `Discard`
> While a release request is open on the draft, one more sentence: The open
> release request is withdrawn.

Both were reviewed as specimens first (`mockups/cloud-discard-mockup.html` here,
`web/discard-mockup.html` in the service tree), each with three places for
the control and two ways to ask. The owner left the pick to the author (O18).

A refusal reads in the workspace notice zone with the service's sentence, as
a refused release request does.

Desktop, in the workspace notice zone, on a computer whose last Push or Pull
was on a removed push. The last sentence shows only when Push is not open.

| What Mallard Cloud holds now | Continuity | Notice | Pull | Push |
|---|---|---|---|---|
| The public version | `replaced`, `matches_published` | **Draft discarded** — Mallard Cloud holds public version N. Pull, then push again. | open | after the Pull |
| A draft another computer pushed since | `replaced`, any other state | **Draft discarded and pushed again** — Pull, then push again. | open | after the Pull |
| Nothing | `remote_missing` | **Draft discarded** — Push creates a new draft. | open, nothing to pull | open |

# Decisions

Taken by the owner on 2026-09-26:

| # | Decision |
|---|---|
| O1 | A repository that is gone allows a Push. No unlink, no relink. |
| O2 | A repository that is being deleted allows a Push too. |
| O3 | Lost access reads as gone. A collaborator removed from a private repository sees "Repository gone", and their Push creates a private repository on their own account. |
| O4 | The service frees the name when removal is confirmed (R8), so the new repository has the old address at once. No `-2` suffix. |
| O5 | The Library is fixed in this plan (R9). |
| O6 | The copy of part A's "What the user sees", as written. |
| O7 | There are no customers: no migration, no compatibility, the remote databases are reset. |
| O8 | A user can discard the draft. |
| O9 | Both sides can discard: the website and the desktop. |
| O10 | The desktop discards only a draft over a public version. A repository that holds a draft alone is discarded on the website. |
| O11 | The Owner can discard on both sides. An Admin can discard on the website only. Nobody else can. ("still only owner can discard", "Admin should be able to discard too", "Admin can only discard on the website".) |
| O12 | Today's result of a discard is not good. After A1 is discarded, and after A1 is published, the sessions are compared with A (R11, R18). |
| O13 | A discard runs immediately (R13). |
| O14 | Nothing is implemented until the owner says so. The owner said so later on 2026-09-26. |
| O15 | After a discard a computer that was on the removed pushes pulls before it pushes. ("it's fine") |
| O16 | A repository made private after a release is treated as a private repository (R12). |
| O17 | A link whose repository is gone follows only a repository the account collaborates on (R5), after the three stories of "R5 by example". |
| O18 | The discard's control, after the specimens: on the desktop one more icon pill in the project menu, asked through the system's dialog; on the website a panel of its own in Settings, asked in place with a second click. ("Use your pick for the UI change") |

No decision is open.

# What this reverses

Three sentences of `DESIGN_MANAGED_CLOUD_STATUS.md`:

> Deletion or lost access is an error, not an empty snapshot. A pinned link
> does not select or create a replacement repository when its destination
> disappears.

> Errors stop Push and Pull in that view.

They become: "A pinned repository that answers `not_found` or
`repository_removed` reads as an empty stream, or as the repository this
account collaborates on for the same bundle. A Push creates the repository
when there is none. A Push or a Pull saves the repository it used as the pin.
A refresh saves nothing. Every other error stops Push and Pull in that view."

Of `PLAN_MANAGED_CLOUD_CAS_FREE.md`:

| Where | Today | Becomes |
|---|---|---|
| Decision 8 | "Discard clears the chain and keeps the content. … Desktops read an empty stream, which to them is a cleared storage; the next Push starts at revision 1" | On a public repository a discard removes the pushes after the released one, and desktops select the released push. On a private repository it clears the chain, as written. It keeps the content either way. |
| Section 5.8 | "queues a removal of target `chain` and answers at once" | runs the removal in its request; the worker finishes what the budget left |
| Section 6.5 | 'A website Discard reads on every desktop as "Another computer cleared <storage>. Push again."; no new copy is proposed.' | R17's three notices |
| Section 7 | the rows "Discard on the website" and "Repository removed, or access lost" | R11, R12 and part A's R1 |
| Section 11 | "Nothing on Mallard Cloud is deleted by a desktop verb. Discard and removal are website actions with confirmation, run by the worker in the documented order." | Discard is a verb of the website, for the Owner and Admins, and of the desktop, for the Owner over a public version, each with a confirmation. Removal is a website action. No other desktop verb deletes anything on Mallard Cloud. |
| Section 13 | row 8 of the staging pass | this plan's live pass, rows 9 to 15 |

Of the architecture document:

| Where | Today | Becomes |
|---|---|---|
| Section 4.3, the role table | "Discard the private snapshot; list members; leave": Member yes, Admin yes, Owner yes | Discard joins "Rename; make private; remove the repository": Admin and Owner |
| Section 4.5 | "Discard private snapshot clears the private slot in one transaction … and changes nothing public." | On a public repository it returns the private snapshot to the public version; on a private one it clears it. It changes nothing public. |
| Section 5.8 | "Members, invitations, approvals, discard, removal, and Super Admin live on the website only." | discard leaves the list: the Owner has it on the desktop too |

`STORE_API.md` section 3, the discard route: its roles, what it keeps, its
order, its answer.

# Safety

The documented invariants this plan touches, and the test that holds each:

| Invariant | After this plan | Test |
|---|---|---|
| A refresh writes nothing (`observe_linked_bundle`). | Kept. Adoption and a gone pin live in memory; only a Push or a Pull saves a pin. | 9 |
| A link never moves to a repository the user did not choose. | Relaxed to: or one this account collaborates on for the same bundle, after the pin answered gone. Never someone else's public repository. | 5 |
| Only a storage that answers reads as empty. | Relaxed to: and a pinned repository that answers `not_found` or `repository_removed`. Nothing else. | 7 |
| The desktop never deletes a claim or a record, and never finishes another writer's claim by itself. | Kept for the engine. The discard verb is one confirmed request; the service does the removing, in its own order. | 27, 28 |
| Nothing on this computer is deleted without an opt-in and a digest check. | Kept. A discard deletes nothing on any computer. | 27 |
| A publication is immutable and final while claims and history are intact. | Kept up to the public version, which no discard removes. The pushes after it are removed only by a confirmed discard, the Owner's or an Admin's. | S6, S9 |
| Recovery is a user action. | Kept. Stale bases and acceptances read as absent; nothing is rolled back; the user pulls and pushes. | 10, 21 |

# What stays

- The store routes and `STORE_API.md`'s wire contract, the discard route's
  answer aside. A `deleting` repository still answers `410
  repository_removed` and a `removed` one `404 not_found`, on every route
  that names it by id.
- Schema numbers, the link document's fields, the portable manifest, and the
  service's tables but one column: the removal row already names the push a
  discard keeps (`publication_id`), and gains `cut_record_name`, the first
  record it removes (*Implementation notes*).
- `observe_linked_bundle` writes nothing. Recovery is a user action.
- One claim request and one record request per Push, settled by read-only
  checks.
- Removal of a repository: the confirmation, the external record, the
  erasure of the prefix on the cron, the final `removed` state.
- Releases, the public face, the history of public versions.
- Unlink, for a user who wants the link gone.
- Bases, checkpoints, recipe and project-file preferences of the link, and
  the Library's accepted item heads. A gone repository or a discarded draft
  changes how they are read, never what is stored, until the next Push or
  Pull replaces the stale ones.

# Design

## Service (`mallardInternal/cloudflare/managed-cloud`)

### A repository being deleted stops counting (R8)

Six reads of `state <> 'removed'` become `state = 'active'`, and the index
follows them:

| Rule | Where |
|---|---|
| Create guard, project: the account collaborates on a repository of this bundle | `src/routes/repositories.ts:123` |
| Create guard, catalog: the same for a Library item | `:133` |
| The account's repository limit | `:152` |
| Names the Owner holds, for the suffix rule | `:86` |
| Rename: the name is taken | `:361` |
| Address lookup: the active repository of that name, else the newest one being deleted | `src/db/queries.ts:90` |
| `repositories_owner_name`: unique per Owner among active repositories | `migrations/0001_init.sql:135`, edited in place |

The account's repository list keeps showing a repository that is being
deleted (`src/routes/repositories.ts:214`); the website may show two rows of
one name for a few minutes.

### Discard (R10 to R14)

`discardPrivateSnapshot`, `src/routes/content.ts:196`:

1. **Who.** `requireOwnerOrAdmin`, and the guard's roles are the Owner and
   Admins (`:200`, `:208`). A Member's request answers `403 forbidden`.
2. **What is kept.** The route lists the stream and selects (`listStream`),
   and reads the public head's source push (fact 20).

   | The repository | The stream | Kept | Answer |
   |---|---|---|---|
   | public, its source push at depth k | longer than k | pushes 1 to k | `200` |
   | public, its source push at depth k | ends at k | | `400 invalid_request`, "There is no draft to discard." |
   | public, its source push not on the selected chain | | | `409 revision_conflict`; by construction it does not occur |
   | not public | holds pushes | nothing | `200` |
   | not public | empty | | `400 invalid_request`, the same sentence |
   | public | more than ten listing pages, so the selection cannot be trusted | | `503 temporarily_unavailable` |

3. **The batch**, as today: the guards, the removal row (target `chain`, its
   `publication_id` the kept push's row or null, its `cut_record_name` the
   name of push k+1 or null), the revision bump, the invalidated release
   request, the audit row.
4. **The removal**, in the same request: `runRemoval` of
   `src/jobs/removal.ts`, with a budget of its own (50 R2 operations).
   - The external removal record first, as today.
   - Keeping nothing: the claims until their listing is empty, then the
     records, as today.
   - Keeping pushes 1 to k:
     1. the claim of push k, when it holds the record of push k+1;
     2. the record of push k+1;
     3. every claim of a record the root no longer reaches;
     4. every record the root no longer reaches.
     Steps 1 and 2 are the cut. They run while the record of push k+1
     exists and never again, so the claim a later push writes on push k is
     left alone. Steps 3 and 4 remove what the cut orphaned, and a push
     that landed on push k since is reached from the root, so it stays.
   - The index: the rows of the records that are no longer listed become
     `removed`, and `headStatements` points the head at the selection over
     what is left: push k, a push that landed on it since, or nothing
     (fact 21).
5. **The answer**: `{ok, repository_id, removal_id, state, kept_revision}`.
   `state` is `done`, or `deleting` when the budget ran out; the cron then
   repeats the same steps, which are safe to repeat. `kept_revision` is k or
   null.

A draft of a thousand pushes costs about ten storage operations.

Why that order:

| Moment | A desktop's listing selects | A Push at that moment |
|---|---|---|
| before step 2 | the draft | lands on the draft, and steps 3 and 4 remove it with the rest |
| from step 2 on | the public version | claims push k, which step 1 freed, lands as its child, and stays |

Step 2 before step 1 would leave, for a moment, a claim of push k that holds
a record no listing holds. A desktop would read "A push onto this version
did not finish", and *Finish that push* would write the removed push back.

Known, and left as it is: a Push that listed before the discard and writes
after it stores a record whose parent is gone. Its confirming listing selects
the public version, so the Push fails with "The storage changed while
pushing. Compare again." The record is never selected and stays in the
stream, a few hundred bytes.

### Reset and deploy

Per the service README's "Resetting a remote database":
`node scripts/purge-r2.mjs <env>`, `wrangler d1 execute … --file
scripts/reset-remote.sql`, `npm run db:migrate:<env> && npm run
deploy:<env>`. Then wipe `~/.mallard` on each desktop and sign in again.

## Website (`web/src/pages/RepositoryPage.tsx`)

Settings stays the Owner's and the Admins' tab. The Discard draft panel sits
between Visibility and Remove repository, under R16's condition: the summary
has a `private_head`, and on a public repository that head is not the public
one. It posts `{confirm: true, expected_repository_revision}` and reloads the
summary. Its text depends on whether the repository is public.

## Discard from the desktop

The release request's path, step for step (fact 16):

| Layer | Addition |
|---|---|
| `managed_cloud/client.rs` | `discard_private_snapshot(repository_id, expected_repository_revision)`: one `POST`, not a read, no retry. A refusal arrives as the service's sentence: `forbidden` for a Member or a contributor, `revision_conflict` for a stale revision. The service would accept an Admin; the desktop does not offer the verb to one (R15). |
| `project_sync/managed_cloud_link.rs` | command `discard_managed_cloud_draft(local_project_id, storage_id, expected_repository_revision)`, naming the repository by the link's pin through `target()`. |
| `src-tauri/src/lib.rs` | the command's registration. |
| `src/components/project-sync/api.ts` | `discardManagedCloudDraft`. |
| `ProjectLinksWorkspace.tsx` | the menu row under R15, the confirmation (`confirm` of `@tauri-apps/plugin-dialog`, as the storage draft's discard uses it), then the activity refresh. A refusal goes to `cloudActionError`. |

The verb reads nothing and writes nothing on this computer. The link's bases
become stale when the pushes they name are removed, and the next comparison
reads them as absent.

## The desktop after a discard

No change in the engine. It already does what R11 needs, by the rules of a
cleared storage:

| What happens | By which existing rule |
|---|---|
| The listing selects the released push. | `Mainline::select` over the names that are left |
| The removed child of the released push, which this process saw earlier, is confirmed with one direct read and dropped. | `list_stream`'s confirmation read |
| Bases that name a removed push read as absent; the continuity is `replaced`. | `classify_remote_continuity`, `classify_scope_continuity` |
| Acceptances that name a removed push read as absent; sessions are compared with A by their content. | `ConversationCheckpoint::read_on` |
| Push waits for a Pull where A holds an area this computer has not reviewed. | `classify_push_destination`, `RemoteWithoutBase` |
| The Pull and the Push that follow save fresh bases, and drop the stale ones. | `drop_stale_scope_bases` |

## Store (`managed_cloud/store.rs`, `client.rs`)

- `ManagedCloudStatusState` gains `Gone`. `ManagedCloudStatus::failed` maps
  `ErrorKind::NotFound` with the code `repository_removed` to `Deleting`,
  without it to `Gone`, and `Permission` alone to `Inaccessible`.
- `Inner` gains `moved: Mutex<BTreeMap<Scope, Option<String>>>`: the scopes
  whose repository answered gone at the last first-page stream listing, each
  with the repository adopted in its place, if any. A pinned bundle stays
  there until its pin answers again or is replaced; a Library item, which
  has no pin, only while nothing is adopted, because what is adopted is what
  the table learns.
- `repository_for`: a scope in `moved` resolves to its entry, with no
  request. Otherwise the pin, then the table, then the service, as today.
- `repository_in_use(bundle)`: the same answer without a request, for the
  Push and Pull saves.
- `list_keys_async`, first page of a stream:
  1. list the repository the scope resolves to (for a pinned bundle, the
     pin itself, whatever `moved` holds); on success clear `moved` for the
     bundle and record `observed`;
  2. on `NotFound` (either code): forget the id. Take the repository `moved`
     holds for the bundle, else ask the service (by-bundle, by-library; an
     entry of none asks again, so a repository another computer created
     since is found). List its stream and adopt it only when the listing's
     `link.role` is set. For a pinned bundle record `moved` and the status
     (`observed` for an adopted repository, `Deleting` or `Gone` otherwise);
     for any other scope remember the adopted id. Return the adopted listing
     or an empty page. An adopted repository that answers gone itself is
     dropped and the stream reads empty;
  3. on any other error: today's path.
  A page with a cursor follows `repository_for`.
- `ManagedCloudStatus.repository_id` stays the link's pin.
  `link.repository_id` is the repository that was read. They differ exactly
  while a stale pin has an adopted repository.
- `create`: on success fills `moved[bundle]` when the bundle is in `moved`.
  `AccountExists` after a gone answer returns `REMOTE_CHANGED` (R7); the
  link-time fallback for a scope that never answered gone is unchanged.
- `set_pin` with a changed id clears `moved` for the bundle; with the same
  id it leaves it.
- A read of a key on a scope that resolves to none answers absent with no
  request, so `list_stream`'s confirmation reads of publications this process
  saw earlier drop them.

## Service layer (`project_sync/service.rs`)

- Push save (`:10715`) and Pull save (`:13400`): after the operation, read
  the store's `repository_in_use(bundle)` and write it to
  `managed_cloud_repository_id` of every link with this storage and bundle,
  in the same `mutate_config` (`cloud_repository_in_use`, `save_cloud_pin`).
  Other providers answer none and write nothing, and so does a bundle whose
  repository is gone with nothing adopted.
- No change to `observe_linked_bundle`, `classify_push_destination`,
  `classify_remote_continuity` or the comparison: an empty observation with
  stale bases already reads `remote_missing` with every writable scope
  `fresh`, which is `Ready`. The binding-revision overlay still applies, as
  it does on a cleared storage.

## Library (`storage_catalog/service.rs`)

No change. The refresh already reads an accepted item head of a stream the
storage no longer answers for as absent, and the Push already saves a fresh
accepted state. R9 lives in the store. A discard of an item's repository on
the website reads the same way: an accepted head on a removed push is absent.

## DTO and frontend

- `src/types.ts`: `ManagedCloudStatus.state` gains `"gone"`.
- `managedCloudLink.ts`: `cloudPinMoved(status)`, true when
  `status.link.repository_id !== status.repository_id`.
- `managedCloudLink.ts`: `canDiscardDraft(status)`, R15 in one place: the
  status is `ready`, the pin has not moved, `link.role` is `owner`, the
  repository is public, and `publication.state` is `unpublished_draft`.
- `managedCloudLink.ts` owns every other decision of the workspace, so each
  is tested without rendering it: `cloudSyncBlocked` (false for `gone` and
  `deleting`), `cloudStatusNotice` (the `gone` row, the reworded `deleting`
  row, no control on either), `cloudVerbs` (the release verb and the discard
  verb wait while `cloudPinMoved`), `discardDraftQuestion` and
  `discardCloudDraft` (ask, send the one request, report, compare again).
- `ProjectLinksWorkspace.tsx` draws them: the two buttons, the note, the
  menu rows.
- `clearedStorageNotice.ts` takes the comparison's Cloud status and owns
  every sentence of a Cloud link whose history moved:

  | Cloud status | Continuity | Publication | Sentence |
  |---|---|---|---|
  | `gone`, `deleting` | any | | none; the status notice speaks |
  | `ready`, pin moved | any | | "Created again" |
  | `ready`, pin answers | `replaced` | `matches_published` | "Draft discarded — Mallard Cloud holds public version N" |
  | `ready`, pin answers | `replaced` | any other | "Draft discarded and pushed again" |
  | `ready`, pin answers | `remote_missing` | | "Draft discarded — Push creates a new draft" |
  | any | `displaced` | | unchanged |
  | none (not Mallard Cloud) | any | | unchanged |

## Stub (`managed_cloud/stub.rs`)

`Repository` gains `state` (`active`, `deleting`, `removed`) with the
service's rules after R8: routes that name a repository by id answer `410`
to a collaborator of a `deleting` one and `404` for a `removed` one;
by-bundle, by-library, the coordinate answer, the create guard and the held
names count active repositories only. Helpers `remove(id)` and
`finish_removal(id)`.

`discard(id)` follows the service: on a public repository it keeps the
released push and the pushes before it, with their claims; on any other it
keeps nothing. The stub gains the discard route with the service's checks:
the Owner or an Admin, the current revision, `confirm: true`, something to
discard.

# Steps

Steps 1 to 11 were done on 2026-09-26, each with its suites green. Step 12
is open.

| # | Step | Done when | State |
|---|---|---|---|
| 1 | Service: R8 in the six reads and the index of `0001_init.sql`. | Service tests S1 to S5 | done |
| 2 | Service: discard. Who, what is kept, the order, in the request (R10 to R14). | Service tests S6 to S14; `npm test` green | done |
| 3 | Stub: repository states, discard as the service does it. Store: `Gone`, `moved`, R1, R2, R3, R6, R7. | Store tests 1 to 3 and 6 to 8 | done |
| 4 | Store: adoption with the role check (R5). | Store tests 4, 5 | done |
| 5 | Service layer: Push and Pull save the pin (R4). | Service-layer tests 9 to 12 | done |
| 6 | DTO, notices, buttons, and every Cloud sentence of `clearedStorageNotice.ts` (R17). | Frontend tests 13 to 16; `npm run build` | done |
| 7 | Library: R9 for item streams. | Store test 17, Library test 18 | done |
| 8 | Desktop after a discard: the tests that hold R11 and R17 on the engine as it is. | Tests 19 to 23 | done |
| 9 | Website: the Discard draft panel in Settings (R16). | Service tree builds; live pass rows 9, 12, 14 and 15 | built and type-checked; its live rows are open |
| 10 | Desktop: the discard verb (R15): stub route, client method, command, `api.ts`, the menu row and its confirmation. | Tests 24 to 30 | done |
| 11 | Documents: the reversed passages, `README.md` (Mallard Cloud and the test flows), the architecture document (sections 4.3, 4.5, 5.8 and 6.4), `STORE_API.md` section 3, the website's guides. | Review | done; the website's guides have no passage on removal or discard, and none was added |
| 12 | The owner resets and deploys staging; the live pass below; then production. | The fifteen rows pass | open |

# Tests

Service, Vitest on Miniflare:

- S1. After a confirmed removal and before the job runs, the Owner creates a
  repository of the same bundle: `201`, the same name, a new id. The same for
  a Library item.
- S2. The address resolves to the new repository; the old id still answers
  `410 repository_removed`; by-bundle answers the new id.
- S3. The job then erases `repos/<old id>/` and marks the old row `removed`;
  every object of the new repository remains.
- S4. A rename into the name of a repository being deleted is accepted; two
  active repositories of one Owner still cannot share a name.
- S5. An account at its limit with one repository being deleted creates one.
- S6. **Boundary.** A public repository with pushes 1 (released), 2 and 3.
  A discard answers `done` and `kept_revision` 1. With no job run: the
  stream lists push 1 alone, the claims hold the root's alone, the head is
  push 1 and is the public head's push, the rows of 2 and 3 are `removed`,
  every object and manifest is still stored, the public version still reads
  without signing in.
- S7. After S6 a push onto push 1 is accepted, is the head, and the status
  reads `snapshot_changed` and `unpublished_draft`.
- S8. A repository that was never released: a discard keeps nothing, the
  stream lists empty, the head is none. The same for a repository made
  private after a release: the released push goes with the rest, and its
  row in the history of public versions stays.
- S9. **Boundary.** The order, step by step under budgets of a few
  operations: before step 2 a listing selects push 3; from step 2 on it
  selects push 1; at no step does a claim hold a record that no listing
  holds while its parent is kept. With the two deletes of the cut swapped,
  the test fails on that last sentence.
- S10. **Boundary.** A Member's and a contributor's discard are refused and
  change nothing. An Admin's and the Owner's are accepted.
- S11. Nothing to discard: a public repository whose draft is the public
  version, and an empty private one, answer `400`.
- S12. Under a budget too small the discard answers `deleting`, a listing
  already selects the public version, and the next job run finishes it.
- S13. A push that listed before the discard and writes after it is stored
  and not selected; the listing still selects the public version. A push
  that lists after the cut lands on the public version, and the worker that
  finishes the discard leaves it, its claim and its row.
- S14. What only the removed pushes referenced is residue after a day; what
  the kept pushes reference is not (`cleanup.test.ts`).

Store, on the stub:

1. A removed pinned repository lists empty and reports `gone`.
2. A Push onto a gone pin creates the repository; every put lands in it; no
   write names the old id, and each listing asks the old id first (R6);
   `repository_in_use` answers the new id; the address is the old one.
3. A pinned repository being deleted lists empty and reports `deleting`; a
   Push creates the repository while the old one is still being deleted.
4. A second store with the old pin adopts the repository the first one
   created, and reads its content.
5. **Boundary.** With an open public repository of another account on the
   same bundle, a gone pin reads empty, the Push creates the account's own
   private repository, and the public repository receives no request but the
   one listing.
6. A pin that answers again is read again, and `moved` is cleared.
7. **Boundary.** `403`, `5xx` and an unreachable service never read as
   empty, and no create follows them.
8. A create refused after a gone answer fails the Push; the next listing
   adopts.

Service layer, on the stub:

9. **Boundary.** A refresh of a gone repository leaves `sync_config.json`
   byte for byte; the comparison has no error; sessions read local only.
10. A Push after removal publishes version 1 and saves the new pin and the
    new bases; recipe and preferences are untouched, and no session's record
    is dropped or started over.
11. A Pull from an adopted repository saves its pin.
12. Two links of one bundle on one storage receive the same pin.

Frontend:

13. `gone` and `deleting`: the notice, rows without `?`, Push and Pull
    enabled, no button in the notice, no cleared-storage note.
14. `inaccessible` and `unavailable`: Push and Pull disabled, Retry present.
15. Adopted: chip and **View draft** of the adopted repository, no release
    verb, the "Created again" note.
16. The three notices of R17, each on its own continuity and publication
    state, and "Another computer cleared" on a storage that is not Mallard
    Cloud.

Library:

17. An item repository removed after the store learned it lists empty, and
    the same for one being deleted. This is the measured failure above,
    turned around.
18. The item's row reads as not in the storage; its Push creates the
    repository again and saves the item's accepted head.

The desktop after a discard, on the stub:

19. On a public repository the listing selects the released push, and the
    status reads `public` and `matches_published`. The removed child costs
    one direct read, once.
20. A link whose bases name a removed push reads `replaced`, and
    `sync_config.json` is byte for byte the same after the refresh.
21. **The owner's rule.** The comparison runs against A: a session A holds
    unchanged reads in sync or asks for a Pull that installs nothing, a
    session this computer continued reads ahead, a session only the draft
    held reads local only. None reads `unknown`.
22. After the Pull review a Push lands as the child of the released push,
    and the link's bases name it.
23. On a private repository the listing selects nothing and a Push starts a
    first version, as `discard_reads_as_cleared_and_the_next_push_restarts_without_reuploading`
    holds today.

The discard verb on the desktop:

24. The Owner's discard is one `POST` with `confirm: true` and the revision
    the status showed.
25. **Boundary.** A Member's and a contributor's discard are refused with
    the service's sentence, and the stream is unchanged.
26. A discard with a stale revision is refused, and the stream is unchanged.
27. **Boundary.** The command names the repository by the link's pin,
    refuses a storage that is not Mallard Cloud, sends the one request, and
    leaves `sync_config.json` and every file of the project byte for byte.
28. **Boundary.** The store has no operation that deletes a key; the verb
    reaches the service through the client alone.
29. **Boundary.** The menu row shows for the Owner of a public repository
    with an unpublished draft, and for nothing else: not for an Admin, a
    Member or a contributor, not on a private repository, one made private
    after a release included, not when the draft is the public version, not
    while the pin has moved.
30. Cancel in the confirmation sends nothing. Discard sends the revision,
    then refreshes. A refusal reads in the workspace notice zone.

Handoff commands: in the service tree `npm test`; in this repository
`cargo test -p mallard-infrastructure`,
`cargo test --manifest-path src-tauri/Cargo.toml --lib`, `npm run build`,
`npm run test:frontend-integration`.

# Live pass (staging; two computers of one account, and further accounts for rows 7 and 14)

| # | Action | Expected |
|---|---|---|
| 1 | A: push a project. Website: remove the repository. A: refresh at once. | "Being deleted", sessions without `?`, Push open |
| 2 | A: Push, before the removal run. | "Push complete", version 1, the same address, the notice gone |
| 3 | After the removal run: website and A's refresh. | The new repository intact; the old id gone |
| 4 | Repeat 1, wait for the removal run, A: refresh, Push. | "Repository gone", then "Push complete" at the same address |
| 5 | B (old pin): refresh. | "Created again", A's sessions listed as in storage |
| 6 | B: Pull, then Push. | Both complete; B's link holds the new id |
| 7 | Second account, removed from a private repository: refresh, Push. | "Repository gone"; the Push creates a private repository on that account |
| 8 | A: push a skill from the Library. Website: remove its repository. A, without restarting: refresh the Library, Push the skill. | The skill reads as not in the storage, then "Push complete" at the same address |
| 9 | Public repository, version A released. B: Pull. A: change a session, add a session, Push (draft A1). Website, as the Owner: Settings, Discard draft. Reload; A and B refresh, at once. | Website: the public version, no "latest push not released". A: "Draft discarded — Mallard Cloud holds public version N"; the changed session reads ahead, the added one local only, the rest in sync. B: in sync, no notice |
| 10 | A: Pull, Mark as reviewed; Push. B: refresh. | A: "Push complete", nothing uploaded again; chip `Public version N · Unpublished draft · Not requested`. B: reads ahead; Pull installs A's two sessions |
| 11 | The same repository. A, as the Owner: project menu, Discard draft…, Cancel; then again, Discard. Website and B refresh, at once. | Cancel changes nothing. Then as row 9; B reads "Draft discarded — Mallard Cloud holds public version N"; nothing on A or B changed on disk |
| 12 | A private repository with a draft and no public version. A, the Owner: project menu. Website: Settings, Discard draft. A: refresh. | A's menu has no Discard draft…. The website's panel says the repository stays, empty. A reads "Draft discarded — Push creates a new draft", Push open |
| 13 | A public repository whose draft is the public version: website Settings, A's project menu. | No Discard draft on either side |
| 14 | A public repository with a newer draft, as an Admin and as a Member: the website's Settings and the desktop's project menu. | The Admin has Discard draft on the website, where it works, and not on the desktop. The Member has it on neither |
| 15 | A public repository with a newer draft. Website, as the Owner: Make private. A, the Owner: project menu. Website: Settings, Discard draft. A: refresh. | A's menu has no Discard draft…. The website's panel says the repository stays, empty. A reads "Draft discarded — Push creates a new draft" |

# Implementation notes (2026-09-26)

What the code does differently from the text as it was first written, and
why. The text above was brought in line; these are the reasons.

- **The removal row gained a column.** `removals.cut_record_name` names the
  first record a discard removes, the child of the kept push. The plan as
  written removed "every claim but…" and "every record but…" until a
  listing held no other, which erases a push that lands on the public
  version while the discard is still running, and on a resumed run deletes
  that push's claim. With the column the cut is done while that record
  exists and never again, and what follows removes only what the root no
  longer reaches. A push after the cut stays (S13).
- **The pin is asked at every first-page listing, so a Push onto a gone pin
  asks it too.** R6 as written. Test 2 had said no request names the old id
  after the listing; what holds is that no write does.
- **A `404` must be the service's own.** Only `not_found` and
  `repository_removed` read as gone (`Error::repository_gone`). A `404`
  without the service's error code, as a proxy in front of it would answer,
  reads as unavailable and keeps Push and Pull closed (test 7).
- **`moved` is keyed by scope.** A Library item has no pin. After its
  repository answered gone and nothing was adopted, its Push must create the
  account's own repository and not follow a public one of another account,
  so the item is remembered as gone until a listing adopts a repository or a
  Push creates one.
- **A refusal carries the service's sentence.** The client kept the
  service's `detail` for validation refusals only. It now keeps it for
  `403` too, and the fixed sentence for `403` no longer guesses at the
  reason. A Member's discard reads "Mallard Cloud refused this request.
  Only the Owner or an Admin can do this."
- **`inaccessible` has its own sentence.** It is `403` alone now, which the
  service answers for a suspended account: "Mallard Cloud refused this
  account. It may be suspended."
- **Checkpoint generations move.** Test 10 had said they are untouched.
  Every acceptance a Push saves advances its session's generation, on any
  storage. What holds is that no session's record is dropped or started
  over, and the test says that.
- **The discard's mark is `x`.** The icon set has no mark for it, and `trash`
  is Delete all local sessions in the same menu. A mark of its own is a
  drawing task for the icon set.
- **`scripts/reset-remote.sql` dropped two tables too few.** `guest_stars`
  and `star_secrets` were left behind, so the migrations that follow a reset
  would have failed at `0002_stars.sql`. Both are dropped now.

# Out of scope

- A service answer that tells a former collaborator "removed" apart from
  "no access". O3 makes it a matter of one sentence.
- The `?` on every row for the states that stay errors (`inaccessible`,
  sign-in, unreachable). One repository-level failure still marks every row;
  it deserves its own specimen.
- A discard that removes the latest push alone, or returns to a push other
  than the public version.
- Letting the computer that discards push without a Pull first.
- Refusing a record whose parent's record is gone (the stored, unselected
  record of a Push that raced a discard).
- A discard verb on the desktop's Library page. The website's panel serves
  a Library item's repository as it serves a project's.
- A row in the desktop's menu that opens the website's Settings when R15
  withholds the verb. **View draft** already opens the repository's page.
- Passing the picked repository's id through connect: today the pin after a
  connect is by-bundle's answer, not the row the user picked.
- Hiding a repository that is being deleted from the account's list on the
  website and in the desktop's repository picker.

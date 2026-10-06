# Plan: versions of a Mallard Cloud repository

- Status: implemented on 2026-10-05 in both trees, uncommitted and not
  deployed. *What was built* at the end says where the code differs from
  this text. It replaces `PLAN_CLOUD_PRIVATE_REVISIONS.md`, the first text
  of the same day ("every push is a revision, no draft"), which the owner
  turned down.
- Owner's words (2026-10-05): "Currently when user published a private copy
  in mallard cloud she only published in the draft, later on when she add
  another copy, the draft was overwritten … I want user to be able to publish
  several revision in private repo"; "I still would like to keep the draft
  concept"; "I like model 1"; "Version is fine"; of the website walk, "This
  looks good"; of the app, "Go with A"; then "implement both, desktop app
  and website".
- Scope: the service (`mallardInternal/cloudflare/managed-cloud`), its
  website, and the desktop's Mallard Cloud chip. Project repositories get the
  whole of it. Catalog items (skills, plugins, pets) get the data model and
  keep their pages as they are.
- Constraints: no customers, clean cutover on both sides, no compatibility
  between an old desktop and a new service or the reverse. The service's
  schema is edited in `migrations/0001_init.sql` in place and the remote
  databases are reset as its README describes. The desktop's schemas stay 1:
  nothing in `~/.mallard`, in a bundle manifest or in the link document
  changes.
- Specimens, both uncommitted: the website's
  `mallardInternal/cloudflare/managed-cloud/web/versions-mockup.html`
  (`?v=walk`, screens in `mallardInternal/output/versions-mock-2026-10-05/`)
  and the app's `mockups/cloud-versions-mockup.html` (screens in
  `docs/images/cloud-versions-app/`). Where this text and a specimen differ,
  *Differences from the specimens* says so.
- Related: `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` (sections 4.2 to
  4.7, 6.3, 7.5, 7.6, 11.1), `PLAN_MANAGED_CLOUD_CAS_FREE.md`,
  `PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md` (discard),
  `PLAN_CLOUD_STATUS_CHIP.md`, `mallardInternal/…/STORE_API.md`.

## Words

| Word | Meaning |
|---|---|
| Draft | The repository's latest push. It moves with every Push. As today. |
| Version | A frozen copy of the draft: numbered 1, 2, 3, never changed, its number never given again. It can carry a name and a note. |
| Publish | To freeze the draft as the next version. The team's act. Nothing becomes public. |
| Release | To make one version public. A request, the Owner's approval, and Mallard's review the first time. As today. |
| Public version | The version the public pointer names. A private repository has none. |

```text
Push ──► Draft  (moves with every push)
           │  Publish version
           ▼
       Version 1 ── Version 2 ── Version 3      frozen, numbered, each with its own address
                                     ▲
                                  Public: a pointer, moved by an approved release
```

## What is there today

Read in both trees on 2026-10-05. Nothing was run.

| # | Fact | Where |
|---|---|---|
| 1 | Every push stays: R2 writes are create-only, a superseded push's index row goes `retired`, and cleanup spares what it references. | `STORE_API.md` sections 3 to 5; service `src/store/index.ts`, `src/jobs/cleanup.ts` |
| 2 | A public release already is a frozen, numbered copy of a push: a `publications` row of scope `public`, numbered `public_revision + 1`, pointing at the push through `source_publication_id`, with the push's object references copied. No bytes are copied. | `publishStatements` in `src/routes/releases.ts` |
| 3 | That row is made and made public in one act, and only by an approved release. A private repository can never have one. | the same |
| 4 | A release request pins the current push. A newer push invalidates it. | `createReleaseRequest`; `G.invalidateOpenRequest` in `indexRecord` |
| 5 | The discard returns a public repository's draft to the push its public version was released from, and removes every push of any other repository. Its sweep keeps what every public row references. | `discardDraft` in `src/routes/content.ts`; `src/jobs/discard.ts` |
| 6 | The website's repository page reads the latest push or the public version. *Public releases* lists the public rows with Open. | `web/src/pages/RepositoryPage.tsx` |
| 7 | The desktop reads one status object beside the stream's listing: `no_draft`, `not_public`, `matches_published` or `unpublished_draft`, the draft's push count and the public number. The chip says `Private`, `Draft v8`, `Published v3`. | service `streamStatus` in `src/routes/store.ts`; `managedCloudLink.ts` |
| 8 | The desktop's *Request public release* row sends the request itself. | `request_managed_cloud_release` in `managed_cloud_link.rs` |
| 9 | On a public repository anyone signed in reads the draft, pushes over it and may ask for a release. | architecture section 4.3 |

Fact 2 is what makes this small: a version is today's public row without
the approval, and a release is the pointer alone.

## The rules

- **R1. Push moves the draft.** Push, Pull, the review screens, the store
  contract and a session's status (this computer against the draft) do not
  change.
- **R2. Publish freezes the draft as the next version.** A version holds
  what the draft held at that moment. It never changes. Nothing becomes
  public.
- **R3. One number line.** Versions are numbered from 1 by the service. A
  number is never given again, also after a version is removed. The draft
  carries no number: `Draft v8` is gone.
- **R4. No version of an unchanged draft.** Publish is refused while the
  draft is the latest version.
- **R5. Collaborators publish.** Owner, Admins and Members. A signed-in
  account that is not on the team does not.
- **R6. Every version opens.** Whoever may read the draft may open any
  version on the website, at the repository's address with `?version=3`.
  That address keeps meaning version 3.
- **R7. A release names a version.** The request names one version, the
  latest unless another is chosen. Approval moves the public pointer to it.
  The draft and the other versions stay as they are.
- **R8. A request may name the draft instead.** It is today's request:
  approving it publishes that push as the next version and makes it public
  in one step, and a newer push closes it. This is how someone who is not on
  the team proposes what they pushed, and how the team releases a draft that
  is ahead without a separate Publish.
- **R9. A request for a version stays open while the team pushes on.** A
  push changes the draft, not the version.
- **R10. Discard returns the draft to the latest version**, on a private
  repository and on a public one. With no version, every push goes, as
  today.
- **R11. Whoever is not signed in reads the public version and nothing
  else.** No draft, no other version, no list. Whoever is signed in reads a
  public repository's draft and versions, as they read its draft today.
- **R12. Publish and Release are two verbs everywhere.** "Published" no
  longer means public: the chip says `Public v3`, and the Owner's approve
  button says *Release now*.

## Metadata changes

Nothing portable changes and nothing on a computer changes. The service's
index changes in four tables.

| Where | Change |
|---|---|
| Bundle manifest, publication records, claims, objects, session views | None |
| R2 layout (`STORE_API.md` section 1) | None. A version references a manifest and objects that are already stored. |
| `~/.mallard` (config, links, bases, receipts), schema 1 | None |
| D1 `publications.scope` | `'private'` or `'version'`. `'public'` is gone. |
| D1 `publications`, a version row | `revision` is the version's number. `source_publication_id` is the push it froze. `commit_id`, the manifest facts and `summary_json` are the push's. `created_by` is who published. `state` is `available` or `removed`. |
| D1 `publications`, new columns | `name TEXT`, `note TEXT`: a version's own, both optional |
| D1 `publications`, dropped columns | `released_by`, `owner_approved_by`, `super_admin_approved_by`. The request already holds who asked and who decided. |
| D1 `publications`, new index | One live version per push: unique on `source_publication_id` where `scope = 'version' AND state <> 'removed'` |
| D1 `repositories.public_revision` | Dropped. In its place `version_count` (version numbers given so far, never lowered; the next version is one more) and `release_count` (how often the pointer moved; the policy's "first release" test). |
| D1 `repositories.public_publication_id` | Now names a version row |
| D1 `release_requests.private_publication_id` | Split in two, exactly one set: `version_publication_id` (R7) or `push_publication_id` (R8) |
| D1 `removals.target` | `'revision'` becomes `'version'` |
| D1 `guard_check` | One more column and code: `version_unchanged` |
| `src/validate/limits.ts` | `versionNameChars: 80`, `versionNoteChars: 500` |

## API changes

**New**

| Route | Who | Body and answer |
|---|---|---|
| `POST /v1/repositories/{id}/versions` | A collaborator | `{commit_id, name?, note?, expected_repository_revision}` answers `201 {version, commit_id, name, created_at}`. `commit_id` is the draft the caller saw. Refusals: `409 revision_conflict` (the repository or the draft moved), `409 version_unchanged` (R4; the detail names the version), `409 repository_discarding`, `403 forbidden`, `400 invalid_request` (nothing pushed, or a name or note too long). |
| `GET /v1/repositories/{id}/versions` | Whoever may read the draft | `{latest_version, next_version, public_version, versions: [{version, commit_id, name, note, created_at, published_by, sessions, files, state, is_latest, is_public, public_since, in_review}]}`, newest first. A removed version stays as a marker. |

**Changed**

| Route | Change |
|---|---|
| `POST …/release-requests` | The body names what is asked for: `version: n` (R7) or `draft_commit_id` (R8), exactly one. A draft that is already a version is taken as that version. |
| `GET …/release-requests`, and every answer that describes a request | `private_commit_id` becomes `commit_id`; new `version` (a number, or null for a request that names the draft) and `version_name`; `private_summary` becomes `summary` and `private_manifest_revision` goes |
| `POST …/release-requests/{request}/decision` | `private_commit_id` becomes `commit_id`; the answer's `public_revision` becomes `public_version` |
| `GET /v1/repositories/{id}`, by-name, and the public summary | `public_revision` becomes `public_version` (a version's number or null), with `public_since`. New for whoever may read the draft: `latest_version`, `next_version` and `draft_ahead` (pushes after the latest version; 0 when the draft is that version; null with no version). A head's `scope` is `private` or `version`, and a version head carries `version` and `name`. `open_release_request` carries `version`. `private_revision` goes: nothing prints it. Rows of `GET /v1/repositories` carry `public_version` alone. |
| `GET /v1/public/repositories` | A row's `public_revision` becomes `public_version` |
| `GET …/store?prefix=<the stream>` (the desktop's status) | `publication` becomes `{draft: "none" \| "pushed", latest_version, next_version, draft_ahead, public_version, review_version}`. `link` gains `can_publish_version`; its `release.kind` reports `waiting_owner` and `waiting_super_admin` for any open request. |
| `POST …/private-snapshot/discard` | Same route and body. What is kept follows R10. `kept_revision` becomes `kept_version`. |
| `POST /v1/super-admin/repositories/{id}/publications/{commit_id}/removal` | Becomes `…/versions/{n}/removal`: it removes a version, public or not |

**Removed**

| Route | Why |
|---|---|
| `GET /v1/repositories/{id}/history` | The versions list holds what it held |

**Unchanged**: every store route's contract (`STORE_API.md`), `GET …/pushes`,
`GET …/manifests/{commit_id}`, `…/objects/{sha256}`, `…/views/{resource_id}`,
the public read routes, members, invitations, stars, visibility, removal of a
repository.

**Desktop commands**

| Command | Change |
|---|---|
| `publish_managed_cloud_version {localProjectId, storageId, name}` | New. Answers `{repository_id, version}`. |
| `request_managed_cloud_release` | Removed. The row opens the website's Release tab, where the version is chosen. |
| `discard_managed_cloud_draft` | Answer: `kept_version` |
| `ManagedCloudPublicationStatus` (Rust `PublicationStatus`, `src/components/managed-cloud/types.ts`) | The new `publication` shape above |

## What the user sees

### Website: B + A

The specimen's walk is the specification of layout. In words:

| Place | Today | After |
|---|---|---|
| Head of the content panel | *Latest push \| Public version* on a public repository | A pop-up: `Draft`, or `Version 2`. Its menu lists the draft and every version, newest first, with *Latest* and *Public* marks. Beside it, for the draft: `3 pushes since version 2` and **Publish version 3**. |
| Tabs | … *Public releases*, Release … | … **Versions**, Release … |
| Versions tab | — | The draft's line and Publish, then one row per version: number, marks, name, who, when, sessions and files, snapshot ID, *Open*, *Copy link* |
| An opened version | — | The same page reading that version. One notice inside the panel names it, says the address keeps showing it, and offers the latest version and the draft. No Publish there. |
| Publish | — | A dialog: what the version adds since the last one, an optional name and note, "Nothing becomes public." |
| Release tab, no request | One button, *Request release* | Which version: the latest preselected, any other selectable, and first, while the draft is ahead, the draft itself (R8). The steps. **Request release of version 3**. |
| Release tab, a request | Today's review | The same, naming the version. *Publish now* becomes *Release now*. |
| Band above the tabs | "… asked to publish" | "… asked to release version 3" |
| A visitor | The public version | The same, and the panel's head says `Version 3` and since when |
| Rail | *Latest push* card; "Public version 2" | *Draft* card with the latest version; "Public: version 3" |
| Settings | *Discard draft*: back to the public version, or everything | *Discard draft*: back to the latest version; with none, everything |
| *Before you approve* | "Mallard keeps one private version. A new push replaces it." | Rewritten: the draft, and that every version stays |

Addresses: `?version=draft` (the default for whoever may read it),
`?version=3`. Today's `?version=private|public` goes. A session's page keeps
`?commit=`.

### App: A

| State | Chip today | Chip after | Panel line |
|---|---|---|---|
| Nothing pushed | `Not pushed` | `Not pushed` | Nothing pushed yet. |
| Private, no version | `Private` | `Private` | Draft only. No version yet. |
| Private, draft ahead | `Private` | `Private` | The draft is 3 pushes past version 2. |
| Private, draft is version 3 | `Private` | `Private` | Version 3. Collaborators only. |
| In review, a version asked for | `In review` | `In review` | Version 3 is waiting for the Owner. |
| In review, the draft asked for | `In review` | `In review` | The draft is waiting for the Owner. |
| Public, draft is the public version | `Published v3` | `Public v3` | Version 3 is public. The draft is the same. |
| Public, draft ahead | `Draft v8` | `Draft` | 1 push past version 3, which is public. |
| Public, version 4 not public | `Draft v8` | `Version 4` | Not public. Public is version 3. |

- Facts: Draft, Version, Public, You. Review moves to the headline, which
  already names it.
- Rows: **Publish version N…** (collaborators; off under R4), *Request
  public release* (opens the Release tab), *View release request*, *View
  draft*, **Versions** (the website's tab), *View public version*, *Discard
  draft…*. Every link row keeps its copy button.
- Publish asks in a sheet of the link sheet's frame: what the version holds,
  an optional name, and, when sessions here wait to be pushed, "2 sessions
  here are not pushed, so not in it". It warns and does not stop. After it,
  one toast: "Published version 3."
- *Discard draft…* asks with "back to version 3".

## Service

- **Publish** (`src/routes/versions.ts`, new). Outside the batch: list the
  stream and select, as the discard does; the selected push must be
  `commit_id`, be indexed (index it first by the reconciliation's own step
  when its row is missing) and hold something. One guarded batch: a
  collaborator; the repository active with no removal pending and at the
  expected revision; the push is `private_publication_id`; no live version
  of it; then the version row (`revision = latest_version + 1`, the push's
  facts, `source_publication_id`, name, note), its `publication_objects`
  copied from the push, `latest_version + 1`, `revision + 1`, an audit row
  `version.publish`. No R2 write.
- **Release** (`src/routes/releases.ts`). `publishStatements` loses its
  `restore` branch and splits by what the request pins. A version: guard it
  available, move the pointer. A push: guard it is still the draft, make the
  version row exactly as Publish does (published by whoever asked), move the
  pointer. Moving the pointer is today's update with `release_count + 1` in
  place of `public_revision + 1`.
- **A new push** invalidates only a request that pins a push
  (`G.invalidateOpenRequest` takes that condition). A discard invalidates a
  request whose push it removes.
- **First-release policy** (`src/release-policy.ts`): `release_count > 0`
  in place of `public_revision > 0`.
- **Discard** (`discardDraft`): what is kept is the push of the highest
  available version, whatever the visibility. The sweep's keep set
  (`src/jobs/discard.ts`) is every version that is not removed.
- **Removing a version** (Super Admin, today's revision removal): the row
  goes `removed` and drops its references; if the pointer named it, the
  pointer clears and the repository is private; a request that pins it is
  invalidated.
- **Status** (`streamStatus`): the new `publication` object, from the
  selected record and the version rows.
- **Reads** need no change: a version row has its own references, so the
  public routes, `publicationForCommit`, cleanup and the byte count work as
  they do for today's public rows.

## Website (`web/src`)

- `pages/RepositoryPage.tsx`: the `version` reading (`draft` or a number),
  the pop-up in the panel head, the Versions tab in place of `HistoryTab`,
  the notice over an earlier version, the rail, Settings' sentences.
- New: `components/VersionSwitcher.tsx`, `components/VersionList.tsx`,
  `components/PublishVersionDialog.tsx`. Their classes come from the
  specimen's `vm-` rules, renamed.
- `components/ReleaseTab.tsx`: the choice of what to release; the review
  names the version; *Release now*. `components/ReleaseBanner.tsx`: the
  words. `components/Disclosure.tsx`: the storage sentences.
- `content.ts`, `links.ts`, `types.ts`, `discardDraft.ts`,
  `pages/SuperAdminPage.tsx`, `pages/ReviewsPage.tsx`, `docs/pages.tsx`:
  the renamed fields and the words.

## Desktop

- `crates/mallard-infrastructure/src/managed_cloud/client.rs`:
  `PublicationStatus` in its new shape, `publish_version`,
  `DiscardedDraft.kept_version`; `request_release` goes. `store.rs` passes
  the status through. `stub.rs`: versions, the pointer, both kinds of
  request.
- `src-tauri/src/project_sync/managed_cloud_link.rs`:
  `publish_managed_cloud_version` (one stream listing gives the draft's
  commit and the repository revision; then the one request, sent once);
  `request_managed_cloud_release` goes. `lib.rs` registers the change.
- `src/components/managed-cloud/`: `types.ts`; `managedCloudLink.ts`
  (`cloudHeadline`, `cloudFacts`, `cloudVerbs`, `canPublishVersion`,
  `cloudVersionsUrl`, the discard's question); `ManagedCloudChip.tsx` (the
  rows); new `PublishVersionSheet.tsx`.
- `src/components/project-sync/`: `api.ts`; `ProjectLinksWorkspace.tsx`
  (the sheet, its busy state, the count of sessions to push from the
  comparison it already holds, the toast); `clearedStorageNotice.ts` (it
  reads `matches_published` and the public number today).
- The Library reads the same status for its items; its three uses in
  `storage_catalog/service.rs` take the new shape and print nothing new.

## Tests

Service (vitest on Miniflare):

1. After three pushes and Publish: `GET …/versions` holds version 1 at the
   third push's commit; a collaborator reads its manifest, a view and a
   payload by that commit after two more pushes.
2. Publish again with nothing pushed: `version_unchanged`, naming 1.
3. Publish with a stale `commit_id`: `revision_conflict`; by a signed-in
   account that is not on the team: `forbidden`.
4. Numbers: with versions 1 and 2, remove 2 as Super Admin and publish
   again: the new one is 3.
5. A request for version 2 stays open across a push; approval makes the
   repository public at version 2 with no new row.
6. A request for the draft is invalidated by a push; approved in time, it
   makes the next version and points at it.
7. Someone not on the team asks with `draft_commit_id` and is accepted; asks
   with nothing, refused.
8. Discard on a private repository with versions 1 and 2 and two pushes
   after: the pushes go, both versions and their content stay, the draft is
   version 2. With no version: every push goes.
9. Discard on a public repository whose public version is 2 and latest is
   3: the draft returns to 3.
10. Someone not signed in reads a public repository's public version and gets
    `not_found` for the draft, for another version's manifest and for
    `GET …/versions`.
11. A day later, after cleanup and reconciliation: every version's manifest,
    views and payloads are still there.
12. The stream's status for each row of the app's table above.

Website (vitest, and a harness page): the pop-up's menu; the Versions tab's
rows and marks; `?version=1` draws version 1 with the notice and without
Publish; Publish off under R4; the Release tab's choices for a collaborator
with the draft ahead, with the draft equal, and for someone not on the team;
the visitor's page; no page prints "Published" for a public version.

Desktop: the Rust parity test over the stub for the status of each state and
for `publish_version`'s refusals; frontend integration tests for
`cloudHeadline`, `cloudFacts` and `cloudVerbs` over the eight states, the
sheet with and without sessions to push, and the removed release command.

## Decisions

**The owner's (2026-10-05):** keep the draft; model 1; the word Version; the
website's B + A as walked; in the app, Publish in the Cloud chip's panel.

**Taken from the specimens he called good; say so if one is wrong:**

1. Versions takes the place of the Public releases tab.
2. *Request public release* stays in the header and leads to the Release
   tab.
3. An earlier version can be released, not only the latest.
4. Whoever is signed in and not on the team still reads the draft and every
   version of a public repository, as they read the draft today.
5. A private repository's chip stays `Private`; the panel says where the
   draft stands.
6. `Published v3` becomes `Public v3`, and the approve button *Release now*.
7. The draft has no number.
8. Publish with sessions waiting to be pushed warns and goes on.

**Mine. The owner was shown the four and answered "implement" (2026-10-05),
so they stand as written:**

9. Members publish, as well as the Owner and Admins (R5).
10. A request for the draft closes on the next push, as today; only a
    request for a version stays open (R8, R9).
11. The app's *Request public release* opens the Release tab and no longer
    sends the request itself.
12. A version's name and note are set at Publish and not edited after.

## Differences from the specimens

- The website's Release tab, with the draft ahead, offered "Publish version
  3 from the draft, then release it" and a button "Request release of
  version 3". Under R8 the number is given at approval, so the choice reads
  "The draft, as pushed Oct 1: becomes the next version when approved" and
  the button "Request release of the draft".
- The band for such a request says "asked to release the draft".

## Later

- Pull a chosen version into the desktop. It needs a link that keeps
  `version=`, a planner that takes a version as its source, and one guard of
  the Pull apply relaxed; its own plan.
- The Owner removes a version; a version's name and note are edited.
- The versions list on a catalog item's page.
- An offer to publish after a Push (the app specimen's C).

## Steps

0. The owner's word on decisions 9 to 12. Done: "implement".
1. Service: schema, Publish, the list, tests 1 to 4 and 11. Done.
2. Service: Release by version and by draft, the policy, discard, removal,
   the status, tests 5 to 10 and 12. Done.
3. Website: the pop-up, the Versions tab, the notice, the dialog, the
   Release tab, the words; its tests. Done.
4. Desktop: the status, the chip's words, the row and the sheet, the removed
   command, the stub; its tests. Done.
5. Documents: the architecture sections named under *Related*,
   `STORE_API.md` section 3's status and discard, the service's README, the
   website's docs pages. Done.
6. The owner's, still to do: reset and deploy staging (`scripts/purge-r2.mjs`,
   `scripts/reset-remote.sql`, migrate, deploy), wipe `~/.mallard`, a live
   pass (push twice, publish, push, publish, open version 1, ask for version
   2, approve, visit signed out, discard), then production.

## What was built

Implemented on 2026-10-05, uncommitted in both trees. Where the code differs
from the text above:

- **`version_count`, not `latest_version`, is the column.** The API's
  `latest_version` is the highest version that is still there, which a
  removal lowers; the counter never goes down. So the answers that offer
  Publish also carry `next_version`, the number it would give.
- **The service says who may publish.** The stream status's `link` carries
  `can_publish_version` beside `can_request_release`, so the app does not
  work the rule out again.
- **Publish does not index.** When the latest push has no index row yet (the
  push's own request failed after R2 had the record), Publish answers
  `temporarily_unavailable` and the reconciliation walk repairs the row, in
  place of indexing inside the request.
- **Publish follows an open request.** A request that names the draft
  becomes a request for the version when that draft is published, so it
  stays open across the next push.
- **The summary carries a little more** for the pages: `public_since`, a
  version head's `name`, and `version` on `open_release_request`.
- **Catalog items got the Versions tab too.** Their page had *Public
  releases*, which is gone, so the tab, the pop-up and Publish are the same
  on a skill, a plugin or a pet as on a project. The app's Library panel
  offers no Publish and no Versions row.
- **The Library's release request went the same way as the project's.**
  `request_catalog_cloud_release` is removed; its row opens the Release tab.
- **The Release tab draws no steps before a request exists.** Whether
  Mallard's review follows the Owner's is the service's policy, and the page
  learns it from the request. Until then one sentence says so.
- **The app offers *Discard draft…* where it did: to the Owner of a public
  repository,** now while the draft is past the latest version. The
  specimen's private states show no such row. A private repository's draft
  is discarded on the website, where the rule is R10 for both.
- **Session addresses carry `version=` in place of `scope=`** (`draft` or a
  number), so the way back from a session returns to the version it was
  opened from.
- **Removing a version is the API's alone.** The Super Admin page has no
  control for it, as it had none for a public revision.

Run on the final trees:

| Where | Check | Result |
|---|---|---|
| Service | `npm run typecheck`, `npm test` | clean; 217 passed, 10 of them `test/versions.test.ts` (the plan's tests 1 to 12) |
| Website | `npm run test:web`, `npm run build:web`, `scripts/icons.mjs check` | 269 passed; built; 302 comparisons passed |
| Website | `web/versions-harness.html`: the shipped `App` on a stub of the service | Seen as Owner, Member, a signed-in account off the team and a visitor; Publish, a request and its approval walked; 375 px and dark |
| Desktop | `npm run build`, `npm run test:frontend-integration` | built; 585 passed |
| Desktop | `cargo test … managed_cloud`, `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 63 passed; 536 passed, 15 ignored |

## Not checked

- Nothing ran against a deployed service: the service's tests run on
  Miniflare, the website's harness on a stub in the page, and the desktop's
  tests on its own stub of the service. The first run of all three together
  is step 6.
- The desktop was not run as the Tauri app. Its panel and sheet were drawn
  in a browser from the shipped components, in light mode.
- The path bar's spare width with the new words. They are as short as
  today's or shorter, except `Version 4`, one character longer than
  `Draft v8`.
- The session page of the website through the harness; its trail and links
  are covered by unit tests only.

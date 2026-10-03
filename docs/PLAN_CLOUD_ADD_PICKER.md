# Plan: one Mallard Cloud picker for pets and projects (S1 · Tabs + rows)

Picked by the owner, 2026-10-02, on `cloud-add-mockup.html` (`?style=rows`).
Round 1 settled the flow; round 2 the style. This is the cutover.

**Status (2026-10-02): implemented**, uncommitted. Bars: Rust lib
managed-cloud tests 27/27, frontend integration 524/524, `npm run build`
green, clippy clean on the touched files. Live checks: the real
`CatalogConnectDialog` and New project › From Mallard Cloud on the stubbed
harness pages (`/catalog-cloud-add-mockup.html?scene=head`, click Add from
Mallard Cloud…; `/cloud-project-live.html?accounts=one`, the second door).
Open: a pass in the packaged app against the real service.

## What changes for the person

Both places that choose a Mallard Cloud repository — the Catalog row page's
*Add from Mallard Cloud* dialog (and Add Catalog resource's door) and the
project's *Connect {project}* dialog and New project › From Mallard Cloud —
get the same picker:

- **One field.** Plain words filter the rows as you type. `owner/name` or
  the page's link looks up (verb *Look up*); a 20-letter code joins (verb
  *Join*). Placeholder: *Search, or paste a link, owner/name or an invite
  code*.
- **Three tabs** under it, in the website's order. Pets: *Shared with you ·
  Starred · Public*. Projects: *Yours · Starred · Public*. Counts on the tabs
  that have a number; Public has none until its pages are read. Public has
  *Newest · Most stars* and a *Show more* foot.
- **Rows** with a tile (the Catalog card's ink-boxed preview wearing the
  kind's mark; the pet image comes in a later layer), the address, the
  description, and role · ★ stars · age.
- **Picking a Starred or Public row** resolves it by address, exactly as a
  typed address does, so Add and Connect keep their checks (misfit, Git
  remote, acknowledgement). The found row above the tabs is shown only for
  an address that is not one of the rows.
- **Empty tabs** say what they would hold and offer *Browse Public*.
- **Adding a public pet** now records the addition on Mallard Cloud
  (`PUT /v1/repositories/{id}/catalog-addition`), so another computer's
  Library refresh lists it too. Today only a machine-local binding is saved.

## Service and website

No change. Routes and fields already exist: `GET /v1/me/starred?cursor=`,
`GET /v1/public/repositories?class=&provider=&kind=&sort=&cursor=`, the
by-name lookup, invitations, catalog-addition; rows carry `description`,
`github_description`, `star_count`, `viewer_starred`.

## Desktop

1. **Crate client** (`managed_cloud/client.rs`): `RepositorySummary` gains
   `description`, `github_description`, `star_count`, `viewer_starred`
   (serde defaults). New `ListingRow` (one shape for starred and public
   rows: address, website URL, class, catalog coordinate, visibility, role,
   GitHub URL, description, stars, revision, age), `RepositoryListing { rows,
   next_cursor }`, `list_starred(cursor)`, `list_public(filter, cursor)`,
   `add_catalog_item(id)`. The stub serves the three routes and keeps stars,
   descriptions and additions.
2. **Project side** (`project_sync/managed_cloud_link.rs`):
   `ManagedCloudRepository` gains `description`, `star_count`,
   `viewer_starred`; commands `list_managed_cloud_starred(storage_id)` and
   `list_managed_cloud_public(storage_id, sort, cursor)` return project rows.
3. **Catalog side** (`storage_catalog/service/cloud.rs`):
   `CatalogCloudRepository` gains the same three fields; commands
   `list_catalog_cloud_starred(scope_id, storage_id)` and
   `list_catalog_cloud_public(scope_id, storage_id, sort, cursor)` return the
   scope's coordinate; `connect_cloud_repository` records the catalog
   addition for a repository the account does not belong to.
4. **Frontend**: `src/types.ts` and `storage-catalog/types.ts` mirror the
   DTOs; `projectSyncApi` and `storageCatalogApi` gain the two listings; a
   new `src/components/managed-cloud/CloudScopes.tsx` holds the shared
   parts (entry kind, the scope tabs, the listing hook, the listing rows,
   the foot); `RepositoryAddressPicker` and `CloudRepositoryChoice` draw the
   tabs and rows and resolve a picked listing row through their existing
   lookup; `RepositoryLookupField` says a link is accepted and lights its
   verb only for an address or a code. CSS: `.repo-scope-tabs`, `.repo-row`
   and the small preview tile, beside the `.repo-*` block in App.css.
5. **Tests**: crate listing parse and catalog-addition against the stub;
   frontend gates for the tabs, rows, filter and placeholder.

## Not in this layer

Pet images on rows (one manifest read per row), a text search on the
service, folding the two host pickers into one component.

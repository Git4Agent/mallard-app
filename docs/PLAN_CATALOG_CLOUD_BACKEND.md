# Backend changes: Catalog items on Mallard Cloud, the way projects are

What the backend needs for
[PLAN_CATALOG_CLOUD_LIKE_PROJECTS.md](PLAN_CATALOG_CLOUD_LIKE_PROJECTS.md)
(specimen: `cloud-pet-mockup.html`). Mallard Cloud (`managed_cloud`) only:
every other storage's Library path stays as it is. Checked against the
working tree on 2026-10-01.

**Status (2026-10-01):** 1–8 are built; 9 reports each item's version, and a
moved head still stops the batch; 10 names each item's `sources` (address,
draft or public, version), and an inaccessible item still fails the prepare;
11 is not built. In 3 the union is
the address's facts (`found`, `not_found`, `not_catalog`, `wrong_service`,
`github`, `invalid`); the chooser says whether the item fits, and Connect
checks it again. In 7 a second repository blocks its row, and a refused
sign-in is the page's `cloud_status`; one refused repository still fails the
page. Shared with you is its own command, `list_catalog_cloud_shared`.

## Already there

- **One item's status:** `get_catalog_cloud_status` answers its
  `ManagedCloudStatus` once it has a repository.
- **Connect pulls nothing.** It saves the binding; "Connect and review" is the
  frontend's.
- **The first Push names the repository** the way `predictRepositoryAddress`
  predicts: a valid name as it is, `-2`, `-3` when taken.
- **Held for Pull at prepare:** `CatalogPushRow.held_for_pull`.

## Schema: stays 1

Nothing here changes a schema (owner, 2026-10-01):

- **Machine-local state** (`CATALOG_SCHEMA = 1`): the binding is already
  `cloud_repositories` in `catalog_scope_links.json`. Create, Connect, Add,
  and choosing between two repositories all write that field; the door's new
  row and link are today's scope and link documents.
- **Plans** are in memory only (`plan_store.rs`: "die with the process"), so
  push destinations and pull sources are not persisted.
- **Results** (version, address, created, held, copy name) are returned, not
  stored.
- **Portable manifests** (schema 1): unchanged. One repository per item, as
  today.
- **The Mallard Cloud API:** unchanged. A repository summary already carries
  revision, visibility, role, and its tags' `updated_at` and
  `registration_only`.

The new fields are Tauri contract changes: Rust DTOs and `types.ts`.

## Changes

| # | Change | For | Today |
|---|---|---|---|
| 1 | `list_catalog_cloud_repositories(scope, storage, resource_id)` lists the repositories holding that item. Rows: `repository_id`, `address`, `website_url`, `visibility`, `role`, `can_push`, `revision`, `updated_at`, `registration_only`, `resource_id`, `display_name` — all already in the service's repository summary. | Connect dialog's list | `{resource_id, address, link}`, filtered by kind, not item (`service/cloud.rs`) |
| 2 | The listing also answers **shared with you**: across catalog kinds, the repositories this account belongs to that no row connects, each with its kind. | Add | None |
| 3 | Resolve and join return a union — collaborator, public, not_found, wrong_service, github, invalid, not_this_kind, other_item, other_format — carrying the kind. A code for another kind still joins (the service has no invitation preview, and joining is harmless); the answer says which row it belongs to. | Field answers and errors | Plain strings. `join_cloud_repository` accepts, then fails on the kind with a sentence. |
| 4 | New `create_catalog_cloud_repository(scope, storage, resource, expected_link_revision)`: creates the `catalog` repository and saves the binding. `AccountExists` returns the existing one. | Connect on the New row | Only the first Push creates one (`managed_cloud/store.rs`) |
| 5 | **The binding is fixed.** Connect refuses an item that already has one. `disconnect_catalog_cloud_repository` is removed. | "Can't be changed later." | Connect replaces a saved binding and clears its acceptance; Disconnect is registered in `lib.rs`. |
| 6 | Add from the door creates the Catalog row and links Mallard Cloud when either is missing, then connects. | Add Catalog resource › From Mallard Cloud | Connect needs the row's link: "Connect this Catalog row to the storage first." |
| 7 | `observe_managed_items` reports per item instead of failing: a `blocked` row with its reason (refused, several repositories) and the candidates. A refused sign-in becomes a page-level `cloud_status`; the rows stay. | ⚠ count, held rows, Two repositories | "Several Cloud repositories contain this item…" (`service.rs:1215`) or one refused repository fails the page and both prepares. |
| 8 | Push plan rows gain `destination {repository_id?, address, create}`. | Push review: line two, "New repository" | No destination (`CatalogPushRow`, `domain.rs`) |
| 9 | Push results per item: `revision`, `address`, `created`, and `held` when its head moved after review. | "Push complete · Version 1"; "Pushed 1 of 2 · held for Pull" | `revision: None` on Cloud (`service.rs:2688`); each item's revision and repository are computed, then dropped; a moved head fails the batch ("the storage head changed after review"). |
| 10 | Pull plan items gain `source {address, reads: draft \| public, revision}`. An inaccessible item is `blocked`, not a failed plan. | Pull review: line two, Version N, "1 skill can't be pulled" | Revision only; one refused repository fails the prepare. |
| 11 | Pull results gain `copy_name` for a kept local copy. | "⑂ 1 kept independently" | Only inside the message: "Kept your version as '…'" (`apply.rs`) |

**Frontend only:** pass the account's `account_login` and `service_origin`
into `CatalogStorageOption` for the predicted address. The storage already
serves both.

## To decide

- **The kept copy's name.** The specimen shows `sql-review-2`; the backend
  makes `sql-review-local-copy` (`resource_planning.rs`). Recommended: keep
  the backend's and fix the specimen.

## Nice to have

- An exact `-2` prediction: the account's owned names across kinds (the
  listing is filtered by kind).
- `resource_ids` on the push and pull prepares, so another item's failure
  can't stop one item's review.
- A saved-or-learned flag on status, and addresses in the page DTO.
- `resource_id` on `finish_catalog_unfinished_push`, so the pet page finishes
  that item's push only (today it finishes every item stream in the row).

## Contract, tests, safety

- **Commands:** new `create_catalog_cloud_repository`; removed
  `disconnect_catalog_cloud_repository`; changed list, resolve, join,
  connect, the push and pull plans and results, and the scope page. Register
  them in `src-tauri/src/lib.rs`, and change the Rust DTOs and
  `src/components/storage-catalog/types.ts` together.
- **Tests:** flows in `src-tauri/src/storage_catalog/cloud_flow_tests.rs`
  (its "Several Cloud repositories" assertion flips to a blocked row);
  frontend in `tests/frontend/storage-catalog.integration.test.tsx`.
- **Safety:** nothing is relaxed. Refusing a second binding is stricter, and
  removing Disconnect deletes no data. Push stays immutable and SHA-256
  verified; Pull stays plan-based and approved.

## Order

1. The dialog and Add: 1–6.
2. The pet page and the row page: 7.
3. The reviews: 8–11.

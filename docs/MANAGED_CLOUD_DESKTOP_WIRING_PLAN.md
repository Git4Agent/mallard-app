# Managed Cloud: desktop wiring plan

Status: implemented 2026-09-14 through Phase 3 (uncommitted). Phase 4 (catalog items) is not
started; Phase 5's production deploy and the super admin grant remain the user's. Stars are out
of scope for this plan. Deltas from the plan as written are listed in the architecture doc,
section 5.8; the staging exit checks of Phase 1 and Phase 2 have not been run yet.
Companion to `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` (section 5 is the design this plan
implements). The service lives in `mallardInternal/cloudflare/managed-cloud` and is live on
staging and production.

## 1. Summary

Managed Cloud lands as a fourth storage provider behind the existing bundle engine. The Worker
already serves every route the desktop needs, so the service stays untouched until a fixture hook
in the last phase. Five phases; the first makes private Push and Pull work end to end against
staging.

## 2. Verified facts the plan rests on

- Service: desktop sign-in (`POST /v1/auth/desktop/start`, GitHub callback, `POST
  /v1/auth/desktop/exchange` to a bearer token valid 90 days), `GET
  /v1/repositories/by-bundle/{bundle_id}/head`, repository list and create, the three-step upload
  (multipart create with the raw manifest part and the inventory, `PUT …/objects/{sha256}` with
  `Content-Length` and `X-Mallard-Sha256`, `POST …/commit` with `expected_commit_id`), abandon,
  and streamed manifests, objects, and views. `Idempotency-Key` is required on create-repository,
  create-upload, and commit. The loopback callback is `http://127.0.0.1:{port}/mallard/callback`.
- Desktop: no storage-kind enum. A provider is one `StorageProviderAdapter` plus one entry in
  `StorageProviderRegistry::production()`. Drafts, test, commit, rename, remove, and reconnect are
  neutral; the Reconnect band follows `managed_secret_id`. `ObjectStore` is synchronous; the
  Dropbox store wraps reqwest with `block_on` and is the template. The Dropbox sign-in is PKCE
  with a loopback receiver on port 53682 and a fixed path (`dropbox_browser.rs`).
- Session views must come from the desktop's timeline builder (`build_timeline_index`,
  `summarize_timeline`, `read_timeline_page`). `SessionTimelineContext` needs a project root and a
  provider home; checkpoint rows come from running git in the root.
- The registry test `descriptors_follow_display_rank_then_key` asserts the exact rank vector.
- Nothing named Managed Cloud exists in the desktop apart from the `managed-cloud-dev` launch config.

## 3. Decisions

1. Provider key `managed_cloud` (not `managed`: the codebase uses that word for managed credentials
   and managed data). Display name "Mallard Cloud", default storage name "Mallard Cloud", first in
   the picker.
2. Settings `service_origin`, `account_id`, `account_login`; one secret `account_token` declared as
   the managed secret. The origin defaults to production; development builds may point at
   localhost or staging, release builds refuse any other origin.
3. Views are built inside the managed store from the wire bytes with a temporary empty project root
   and provider home. Cloud views therefore carry no checkpoint rows and no child tail facts in this
   pass. (Building them in the sync service would give checkpoints but needs a provider-specific
   branch in neutral push code.)
4. Catalog items are the last phase and may become their own plan. Until then the store answers
   Library reads as absent and refuses Library writes with `managed_catalog_uses_item_adapter`.
5. Engine parity tests run against an in-process Rust stub of the service (as R2 runs against the
   stub S3 server). Live checks run by hand against the local Worker and staging.
6. The storage card gets Open on website, the release status chip, and Request public release. The
   website already offers release requests, so that button is the first thing to drop.

## 4. Phases

### Phase 0: groundwork

- Generalize the loopback receiver into `crates/mallard-infrastructure/src/loopback_browser.rs`
  with the callback path as a parameter; Dropbox calls it with its path; `dropbox_browser.rs` is
  removed.
- Move `R2SetupPort` out of `R2Setup.tsx` into `StorageSetupStep.tsx` as a neutral
  `StorageSetupPort` imported by all three lanes.
- Add `StorageLocalSetup::ManagedCloud` (Rust) and `"managed_cloud"` to the union in
  `src/types.ts`; audit the `local_setup` branches in `ProjectLinksWorkspace.tsx` and
  `StorageSettings.tsx` so the third value renders its own label and icon.
- Update the registry ordering test.

### Phase 1: sign-in, adapter, store (private Push and Pull)

Rust, new module `crates/mallard-infrastructure/src/managed_cloud/`:

- `client.rs`: reqwest client built like `R2SetupClient` (no redirects, no retries, timeouts),
  bearer header marked sensitive, `Idempotency-Key` on the three idempotent routes, typed errors
  from the JSON error body, health mapping (401 → `TokenExpired`, 403 on the account →
  `PermissionDenied`, transport failure → `Unreachable`). Never logs the token.
- `mod.rs`: `ManagedCloudAdapter`, `ManagedCloudSettingsV1`, descriptor (`clear_mode:
  Unavailable`, no discovery, `local_setup: ManagedCloud`, `managed_secret_id: account_token`),
  validation, redacted summary, continuity coordinates `[origin, account_id]`, probe = `GET /v1/me`
  with the login checked against the pinned account, reconcile = no-op, `open`.
- `store.rs`: `ManagedObjectStore` behind `ObjectStore`, `block_on` like Dropbox.

| Engine key | Store behaviour |
| --- | --- |
| `.mallard/_storage.json` | Synthesized marker |
| `_head.json` read | `GET by-bundle/{bundle}/head`; 204 → absent; `{repository_id, head: null}` → remember the id, absent; head → synthesized `BundleHead`, etag = commit id |
| `_manifests/{rev}-{commit}.json` read | `GET /repositories/{id}/manifests/{commit}` streamed to a temp file, verified by size and digest; 404 → Missing |
| `_objects/{sha}` read | `GET /repositories/{id}/objects/{sha}`, same verification |
| `_objects/{sha}` write | Staged in memory (`Bytes`), returns Written |
| `_manifests/…` + `_commits/…` write | Resolve or create the repository; `GET /repositories/{id}` for `revision`, `private_revision`, `open_uploads`; end our own upload of the same manifest; build views (Phase 2); `POST /uploads` multipart; end our own stale uploads; `PUT` the objects listed as missing, `cloud_transfer_concurrency()` at a time; commit record discarded |
| `_head.json` compare-and-swap | `POST …/commit` with `expected_commit_id` (Match(etag) → etag, Absent → null); 200 → Written; 409 `head_changed` → Conflict; else error |
| `_tags/…` write | No-op Written |
| `_tags` list and reads | One `GET /v1/repositories?class=project` snapshot; tags synthesized from the publication summaries |
| `_uploads/…` | Rejected (probes are for user-owned stores) |
| `.mallard/v1/library/…` | Reads absent; writes refused |
| `delete`, `clear_managed_data` | Unsupported |

- Registry entry, module declarations in `crates/mallard-infrastructure/src/lib.rs`.
- Tauri: `src-tauri/src/project_sync/managed_cloud_setup.rs` with single-flight control and
  three commands: `authorize_managed_cloud_setup` (start → `AuthorizationLink` over the channel →
  loopback callback → exchange → login kept in the session), `prepare_managed_cloud_storage`
  (writes settings and the secret into the draft through `prepare_storage_draft_in_operation`;
  refuses a second storage for the same origin and account with
  `storage_managed_account_exists`), `cancel_managed_cloud_setup`. An application-layer prepare
  (`crates/mallard-application/src/managed_cloud_setup.rs`) mirrors Dropbox's. Registration in
  `src-tauri/src/lib.rs`.
- Frontend: `src/components/project-sync/ManagedCloudSetup.tsx` (hook + lane): Sign in (Copy link,
  Open browser, Cancel, 180 s countdown; links are never opened automatically) → Account (login,
  Continue, Change login) → Name and save (adopts the prepared draft, then the neutral Test and
  Save). Reconnect mode pins the account. Wrappers in `api.ts`, DTOs in `types.ts`, the lane branch
  in `ProjectLinksWorkspace.tsx`, labels in `StorageSettings.tsx`. No new CSS classes (reuse the R2
  lane classes) and no edit to the desktop `Icons.tsx`.
- Tests: `src-tauri/src/sync_tests/stub_managed_cloud.rs` (hyper) with the same guard semantics
  (bundle → repository, one open upload, missing list, commit CAS, 401/409); a parity pair in
  `storage_parity.rs`; the registry proof; adapter, store, and client unit tests
  (abandon-then-retry, conflict mapping, health mapping, redaction of summaries); moved loopback
  tests; a lane test with a fake API; `ipc` and `storage-settings` integration gates extended.
- Exit, on staging: add the storage through the lane; link a Git project and a non-Git project and
  see both repositories on the website at once (registration revision 1); push; pull from a second
  local state root; continuity rows 4, 7, and 8 classify correctly after the other replica pushes;
  revoke the session row, see the Reconnect band, reconnect, push again; a 90 MiB object round-trips.

### Phase 2: session views

- `views.rs`: for every conversation whose transcript object is in the staged upload, write the
  wire bytes to a private temp file, run the provider's index builder, summary, and page reader
  until 20,000 events or 8 MiB, then set `truncated`. Assemble the view as
  `get_project_session_detail` assembles a detail, minus checkpoints, actions, and child call
  facts. Add it to the inventory with `role: view` and the transcript digest. Unchanged transcripts
  get no view; the service carries the previous one forward.
- Spike first: confirm the Codex index builder accepts a standalone wire rollout outside the
  provider home. The Claude builder reads one file.
- Fixture generator behind a cargo feature writing valid and invalid manifests and views to the
  directory named by `MALLARD_MANAGED_CLOUD_FIXTURES_DIR`; the Worker's validator test reads it
  when set. No hardcoded cross-repo path.
- Exit: staging renders real timelines with the pusher's avatar; a redacted secret shows its
  placeholder; a second push of an unchanged session keeps its view; the truncation flag appears
  at the cap.

### Phase 3: website bridge on the storage card

- `get_managed_cloud_link(local_project_id, storage_id)` resolves the repository id and summary
  and returns the website URL (`{origin}/{owner_login}/{name}`, the repository's address) and the release state. The card shows
  Open on website and the chip: Waiting for Owner, Waiting for Super Admin, Public, Rejected,
  Snapshot changed. Refreshed on open, focus, and Refresh.
- `request_managed_cloud_release(local_project_id, storage_id)` posts a release request with the
  expected repository revision.
- Shown only for managed storages, gated like the lanes.

### Phase 4: catalog items

The item adapter of architecture section 5.5: `managed_cloud/catalog.rs`, `CatalogItemLink`, the
branch in the catalog push and pull commands. The website pages already exist. Own plan when
reached.

### Phase 5: closing

Architecture doc section 5 deltas, the service README pointer, lane disclosure copy (what
collaborators and, after release, everyone can read), memory. Production deploy only on the
user's go (migration 0002 first), and the super admin grant stays the user's command.

## 5. Size

| Piece | New lines (rough) |
| --- | --- |
| HTTP client | 300 |
| Managed object store | 900 |
| Adapter and descriptor | 300 |
| Tauri setup and app-layer prepare | 500 |
| View builder | 350 |
| Lane, wrappers, DTOs | 450 |
| Stub service and tests | 1200 |

## 6. Risks and gaps

- Discovery lists only repositories the account belongs to. A public repository the user is not a
  member of can be read by bundle id but does not appear in the connect dialog.
- Open uploads are capped per member, not per repository (section 7 below).
- Tag writes are no-ops; the website listing is the source of truth.
- The token lives in `sync_config.json` (mode 0600), as the architecture decided. No keychain.
- Removing the storage forgets the token locally without revoking the session; a logout hook needs
  a new adapter method. Deferred.
- Other sessions have uncommitted edits in `src/App.css` and `src/components/Icons.tsx`; this work
  avoids both files.

## 7. Open uploads

An upload is the service's name for a push in flight: create (manifest and inventory), put the
missing objects, commit. Uploads run side by side: the commit batch is the only compare-and-swap,
so two collaborators transfer at the same time and the later commit answers `head_changed`, a CAS
conflict (refresh, review Pull, push again). The create step refuses only a member's own 65th open
upload (`upload_in_progress`, `LIMITS.openUploadsPerMember`); another member's upload never blocks
a push.

An upload stays open until it commits, its owner abandons it, or its 24-hour lifetime ends
(`LIMITS.uploadLifetimeSeconds`). The repository summary lists them as `open_uploads`, each with
`user_id`, `commit_id`, and `stale`: a stale upload's pins no longer match the repository, so it
can never commit.

What the desktop does about them:

- Before creating a new upload it ends an own open upload of the same manifest, because the
  service claims a commit id per open upload.
- After the new upload exists it ends its own stale uploads. The new upload already references the
  objects they carried, so a push after a conflict sends only the delta.
- It never touches another account's uploads, nor its own non-stale ones (a live push from another
  machine). A commit refused with `head_changed` leaves the upload open for exactly that reuse.
- Past the cap it ends its stale uploads and retries the create once.

# Plan: Refresh in the Mallard Cloud picker

Status: A picked and implemented 2026-10-11 (uncommitted). Specimen:
`mockups/cloud-refresh/index.html?lang=zh|en&theme=light|dark&only=A,B`
(serve on the mockups Vite, port 1437); `live.html` mounts the real
connect dialog on mock IPC.

## Problem

The picker (project connect dialog, project setup's Repository band, New
project's From Mallard Cloud, and the Library's Add from Mallard Cloud)
reads Yours and Starred once when it opens and Public once per sort. A
project pushed from another machine while the dialog is open never
appears; the only way to see it is to cancel and reopen.

## Directions in the specimen

| | Where | Seen in | Against |
|---|---|---|---|
| **A** (recommended) | Icon button at the strip's end, after Public's sort | GitHub Desktop clone dialog; Cloudscape header actions; Primer / M3 icon-button tooltip rule | icon-only; beside the sort pills |
| B | Same spot, labeled "Refresh" | Mallard Files "Rescan"; Fluent command bar | three controls on one strip |
| C | Field row, after the Look-up verb | GitHub Desktop post-filter slot | reads as part of search |
| D | "Updated 2 min ago · Refresh" stamp under the list | Cloudscape last-updated + live region; WCAG 4.1.3 | below the fold on a long list; needs a clock |
| E | Dialog header, beside the close | Cloudscape header actions | far from the list; reads as window chrome |
| F | Poll while open, "N new · Show" pill | X / Instagram timelines; Cloudscape auto-refresh | polling a short-lived dialog; no manual control |

Behavior common to A–E, from the references: the current rows stay on
screen while the request runs (no loader replacing the list), the button
disables and its glyph spins, the tab count updates when the answer lands,
and a hidden `role="status"` line announces "Updated" / "Refreshing…".
Target 28×28 clears WCAG 2.5.8's 24px minimum; tooltip and accessible
name are the same words (Primer, M3).

## Implementation (A)

Desktop only; no service or website change. The backend commands do not
cache, so a refresh is a new request.

1. `src/components/managed-cloud/CloudScopes.tsx`
   - `CloudScopeTabs` gains `onRefresh: () => void` and `refreshing:
     boolean`; it draws `<button class="btn btn-ghost repo-scope-refresh"
     aria-label title aria-busy>` with `<Icon name="refresh" size={14}
     className={refreshing ? "icon-spin" : undefined} />` after the sort.
   - `useCloudListings` gains `generation: number` in its key input. A
     ready Starred listing keeps its rows during the reload: `Listing`
     gains `refreshing: boolean` on the ready state, set when the key
     changes with rows already held; the loading state is only for the
     first read. Public resets to idle on refresh (reloads if open), as
     a sort change does today.
   - `ListingStatus` shows nothing for a ready-but-refreshing listing.
2. `src/components/project-sync/RepositoryAddressPicker.tsx`
   - `useManagedCloudRepositories(storageId, localFingerprint, isActive,
     generation)`; a ready listing keeps `repositories` while reloading
     (`RepositoryListing` ready state gains `refreshing`).
   - The host holds `generation` state; `refresh = () =>
     setGeneration(g => g + 1)`; `refreshing = yours.refreshing ||
     starred.refreshing || public.state === "loading"`. Selection,
     `chosen`, `joined` and the field's words are untouched; `joined`
     drops out once the listing holds it (existing `inListing`).
   - Since `listing` is a prop from the dialog/setup host, the host that
     calls `useManagedCloudRepositories` passes `generation` down, or the
     hook moves into the picker. Prefer the latter if every host only
     forwards it.
3. `src/components/storage-catalog/CloudRepositoryChoice.tsx`: same
   `generation` for its own listing and `useCloudListings`; pass
   `onRefresh`/`refreshing` to `CloudScopeTabs`.
4. i18n: `cloud:scopes.refresh` "Refresh" / "刷新", `cloud:scopes.refreshing`
   "Refreshing…" / "正在刷新…", `cloud:scopes.updated` "Updated" / "已更新"
   (the status line).
5. `src/App.css` after `.repo-scope-sort`:
   ```css
   .repo-scope-tabs > .repo-scope-refresh { margin-left: auto; margin-bottom: 7px; width: 28px; min-height: 28px; padding: 0; }
   .repo-scope-tabs > .repo-scope-sort + .repo-scope-refresh { margin-left: 8px; }
   ```
   Icon-only: add `.repo-scope-refresh` to the no-displacement `:active`
   list beside `.session-detail-launch`.
6. Errors: a failed reload shows the existing blocking notice with its
   Try again; the previous rows are gone only when the answer was an
   error (the listing state is then `error`, as today).

## Tests

- `tests/frontend/managed-cloud-link*.integration.test.tsx`: a refresh
  re-invokes `list_managed_cloud_repositories` and
  `list_managed_cloud_starred`, not `list_managed_cloud_public` while
  Yours is open; rows stay rendered between the click and the answer;
  the selected row stays selected; the count updates; the button is
  disabled and `aria-busy` while pending.
- Catalog: the same on `CloudRepositoryChoice` with `list_cloud_*`.
- Boundary test: `CloudScopes.tsx` still imports no feature DTOs.

Handoff: `npm run build`, `npm run test:frontend-integration`.

# Managed Cloud: open push on public repositories, one invitation code per private repository

Status: implemented 2026-09-16 (service, website, desktop, documents; see
section 7 for what differs from the plan). Section numbers refer to
`CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md`; the connect-by-address
work this builds on is `DESIGN_MANAGED_CLOUD_CONNECT_BY_ADDRESS.md`.

Compatibility, as the user set it: clean cutover, no migration, no legacy
path; `migrations/0001_init.sql` is edited in place and stays schema 1;
D1, R2 and `~/.mallard` are wiped before the next test.

## 1. The contract

Today membership gates everything private (section 4.3): only a
collaborator reads the private snapshot, pushes, pulls or asks for a
release, on a private and a public repository alike, and an invitation
code admits one account, once, within seven days.

The new contract has two rules.

1. **A public repository takes pushes from any signed-in account.** No
   invitation. Anyone signed in reads its latest push, pushes over it
   (the commit is still the compare-and-swap of section 6.5, so a moved
   head answers `head_changed`), pulls it, and asks for a release. What
   becomes public still goes through the Owner and then a Super Admin.
2. **A private repository is entered by invitation, and the invitation
   belongs to the repository.** A private repository carries at most one
   invitation code. The Owner or an Admin creates it, rotates it, or
   revokes it; whoever enters it joins as a Member, for as long as the
   code stands. Tom creates the code for B and sends it to Jeff; Jeff
   enters it once; both push to B freely from then on.

"Public" keeps its meaning from section 4.4: a repository is public once a
release was approved, and only while it has a public head. Before its
first release every repository is private and rule 2 applies.

Roles keep their other powers: the Owner approves releases; the Owner or
an Admin renames, makes private, removes, manages the code and the
members; a collaborator (Owner, Admin, Member) discards the private
snapshot and can leave. A Super Admin still reads everything and pushes
nothing without membership.

### 1.1 Decisions this plan takes, to confirm

- **One standing code per repository**, reusable, no expiry, replaced by
  rotate and closed by revoke. The alternative is today's per-person,
  single-use, seven-day code kept but labelled "repository" in the UI;
  the plan reads "at the repo level instead" as the former.
- **A signed-in visitor of a public repository sees its latest push on
  the website**, the way a Member does today: the "Latest push" switch
  appears, History opens, and Request public release is offered. Anonymous
  visitors keep seeing only the released version.
- **Pushing does not make anyone a Member.** Membership is what creation
  and the code grant; contributors of a public repository are recorded as
  pushers (`pushed_by`, section 8) and nowhere else. The website's Yours
  view therefore lists memberships only; the desktop's storage card shows
  the linked repository regardless, since links are by bundle id.
- **Discard stays with collaborators.** A contributor can push over the
  latest push but cannot reset it to the public version; that is a
  management act.
- **Making a public repository private closes the door at once.**
  Contributors lose the private head on their next read (204 from
  by-bundle, `forbidden` on upload); their open uploads expire on their
  own.

## 2. Service

### 2.1 Access

`Access` (`src/db/queries.ts`) gains one flag and one derived meaning:

| Flag | Today | New |
| --- | --- | --- |
| `role` | membership role or null | unchanged |
| `writer` | (none) | `role !== null`, or the caller is signed in and the repository is open (`visibility = 'public'` with a public head) |
| `privileged` | collaborator or Super Admin | `writer` or Super Admin: may read private content |
| `publicOnly` | non-collaborator on a public repository | anonymous caller on an open repository |

`requireWriter(access)` joins `requireMember` and `requireOwnerOrAdmin`,
and a transaction-time guard `G.writer(db, repositoryId, userId)` joins
`G.memberWithRole`:

```sql
SELECT 1 WHERE EXISTS (SELECT 1 FROM repository_members WHERE repository_id = ?1 AND user_id = ?2)
   OR EXISTS (SELECT 1 FROM repositories WHERE id = ?1 AND state = 'active'
              AND visibility = 'public' AND public_publication_id IS NOT NULL)
```

Route by route:

| Route | Today | New |
| --- | --- | --- |
| `POST …/uploads`, `PUT …/objects/{sha}`, `POST …/commit`, `POST …/abandon` | `requireMember`; `G.memberWithRole(member)` in create and commit | `requireWriter`; `G.writer` |
| `POST …/release-requests` | `requireMember`; `G.memberWithRole(member)` | `requireWriter`; `G.writer` |
| `GET …/release-requests`, `GET …/history`, manifests, objects, views by commit | `privileged` | unchanged: `privileged` now includes writers |
| `GET /v1/repositories/{id}`, `GET …/by-name/{owner}/{name}` | summary when `privileged`, else public face | unchanged rule; a signed-in visitor of an open repository now gets the summary (private head, open uploads, fingerprint, open request) |
| `GET …/by-bundle/{bundle_id}/head` | a member's repository, else the public head of a public one | a member's repository, else the private head of an open one (`privileged = true`); the desktop then pulls and pushes the latest push |
| `POST …/private-snapshot/discard` | `requireMember` | unchanged |
| `GET …/members` | `privileged` | collaborators and Super Admin only (`role !== null || isSuperAdmin`) |
| `DELETE …/members/{user_id}` (leave), `PATCH …/members`, rename, visibility, removal, stars | | unchanged |

The Super Admin audit row in `readSummary` (a read of a repository they do
not collaborate on) is unchanged.

### 2.2 Invitation: one code per repository

Schema, edited in `0001_init.sql`:

- `repositories` gains `invite_code_hash TEXT UNIQUE`,
  `invite_code_set_by TEXT REFERENCES users(id)`, `invite_code_set_at
  INTEGER`, with `CHECK ((invite_code_hash IS NULL) = (invite_code_set_by
  IS NULL) AND (invite_code_hash IS NULL) = (invite_code_set_at IS NULL))`.
- The `invitations` table and its index go. The guard column
  `invitation_invalid` stays: it is the error for an unknown or revoked
  code.
- `LIMITS` drops `pendingInvitationsPerRepository`, `invitationLabelChars`,
  `invitationSeconds`, `closedInvitationRetentionSeconds`;
  `membersPerRepository` (50) stays.
- `jobs/expire.ts` drops the two invitation statements and their counters.

Routes (`src/routes/invitations.ts`, rewritten):

| Route | Who | Body → answer |
| --- | --- | --- |
| `PUT /v1/repositories/{id}/invitation` | Owner or Admin | `{expected_repository_revision}` → `{code, set_at}`; creates the code or rotates it (the old one stops working in the same batch); the code is shown once; audit `invitation.set` |
| `DELETE /v1/repositories/{id}/invitation` | Owner or Admin | `{expected_repository_revision}` → `{ok}`; audit `invitation.revoke` |
| `GET /v1/repositories/{id}/members` | collaborators | `invitation: {set_by, set_by_login, set_at} \| null` replaces `pending_invitations`; the code itself is never returned |
| `POST /v1/invitations/accept` | any signed-in account | `{code}` → the repository summary; joins as Member, or answers the summary unchanged when already a collaborator; `quota_exceeded` at 50 members; the repository must be active; bumps the revision and invalidates an open release request, as today; audit `invitation.accept` |

The code keeps today's shape (20 characters in four groups, hashed with
the pepper, `normalizeInvitationCode`). Neither route needs an
Idempotency-Key: set is revision-guarded and accept is idempotent by
construction.

### 2.3 Tests

- `test/helpers.ts`: the member-adding helper uses `PUT …/invitation` and
  `POST /v1/invitations/accept`.
- `test/collaboration.test.ts`: rewritten around the repository code:
  set, rotate closes the old code, revoke, accept twice is one membership,
  accept on a removed repository fails, the 50-member cap, Owner cannot
  leave.
- New cases: a stranger pushes to an open repository and the head moves; a
  stranger is refused on a private one; a stranger's by-bundle head is the
  latest push; a Super Admin without membership is refused an upload; a
  stranger cannot discard; making private cuts a stranger off; a stranger
  requests a release and the Owner approves it.
- `release.test.ts`, `pushed_by.test.ts`, `uploads.test.ts`: only the
  helper changes.

## 3. Website

- **Members tab** (`RepositoryPage.tsx`): the "Invitation codes" panel
  becomes "Invitation code": the state line (none, or set by whom and
  when), Create code / Rotate / Revoke, and the fresh code shown once with
  Copy. Copy text: "One code per repository. Anyone who enters it joins
  as a Member until you rotate or revoke it. A public repository needs no
  code: anyone signed in can push."
- **Invitations page**: the sentence changes to match; the field and the
  Join button stay.
- **Repository page for a signed-in visitor of a public repository**: the
  summary now carries the private head, so the existing "Latest push"
  switch, History, and Request public release appear; the About rail's
  "Your role" prints "Contributor" when `role` is null and the viewer is
  signed in. Members and Settings stay collaborator-only.
- **Home / Yours**: unchanged.
- `web/src/types.ts`: `MembersResponse.invitation`; the invitation reply
  type.

## 4. Desktop

Rust:

- `managed_cloud/client.rs`: `accept_invitation(code) ->
  RepositorySummary`. `invitation_invalid` already maps to
  `ErrorKind::Invalid` with the service's sentence as `detail`.
- `managed_cloud/stub.rs`: `Repository.invite_code`; `PUT` and `DELETE
  …/invitation`; `POST /v1/invitations/accept`; the writer rule: a
  signed-in stranger on a public repository gets the summary from
  by-name, the private head from by-bundle, and accepted uploads; a
  stranger on a private one keeps getting 404 and 204.
- `src-tauri/src/project_sync/managed_cloud_link.rs`:
  `ManagedCloudRepository.can_push: bool` (collaborator, or public);
  `ManagedCloudRepositoryResolution::Public` now means writable; new
  command `accept_managed_cloud_invitation(storage_id, code,
  local_repository_fingerprint) -> ManagedCloudRepository`, which accepts
  and then describes the repository the way `resolve` does, so the
  picker selects it. `ManagedCloudLink.role` stays for the card.

Frontend:

- `RepositoryAddressPicker.tsx`: the read-only verdict, caution and
  verbs go. A public repository's card reads "Public · anyone signed in
  can push" and the primary verb is Connect and review Pull. The
  not-found notice gains the code field: "Nothing at that address that
  you can see. If it is private, enter its invitation code." with Join;
  a resolved code selects the repository. A small "Enter an invitation
  code" toggle under the address field opens the same field without a
  failed lookup first.
- `BundleConnectionDialog.tsx`: the footer verb for `public` becomes
  Connect and review Pull; `Connect read-only` goes.
- `ProjectLinksWorkspace.tsx`: the `Read-only` chip becomes `Contributor`
  when `role` is null.
- Tests: `tests/frontend/managed-cloud-connect.integration.test.tsx` (public
  is writable; the code flow); Rust tests in `managed_cloud_link.rs` (the
  stranger after release can push; accept through the stub).

## 5. Documents

- Architecture: 4.2 (who pushes the private snapshot), 4.3 (the table
  gains "Anyone signed in, public repository"; the invitation paragraph
  becomes the repository code), 5.3 discovery note, 6.3 schema, 6.6 reads,
  7.4 routes.
- Service README: a new entry in "Decisions that refine the architecture
  document"; the roles paragraph.
- `DESIGN_MANAGED_CLOUD_CONNECT_BY_ADDRESS.md` section 2.2: the public
  row and the not-found copy.

## 6. Order and size

Service first (schema, access, routes, tests), then the website, then the
desktop stub, commands, picker, and the documents.

| Piece | Lines |
| --- | --- |
| Service: schema, access flags and guard, route edits, invitations rewrite, jobs | 260 |
| Service tests | 220 |
| Website: Members tab, Invitations page, role line, types | 120 |
| Desktop Rust: client, stub, command, tests | 260 |
| Desktop frontend: picker, dialog, card, tests | 170 |
| Documents | 90 |

After deploy: wipe D1 and R2, `migrations apply`, wipe `~/.mallard`, sign
in, seed.

## 7. As built (2026-09-16)

Everything in sections 2 to 5 is in, with these differences:

- **`RepositorySummary.writer`** was added (service, website types, Rust
  client). The website needed a way to tell a contributor's summary from
  the public face, and the desktop's `can_push` reads it directly instead of
  inferring it from `visibility`. `listRepositories` rows carry `writer:
  true`; the public face `writer: false`.
- **A Super Admin on a public repository is a writer**, like any signed-in
  account; the "Super Admin cannot push" rule holds for private
  repositories, where they are privileged readers only. The plan's decision
  5 read too broadly.
- **Everyone who may list members sees the code's state** (`invitation`:
  who set it and when), not only Owners and Admins; a Member learning that
  a code stands discloses nothing. Only Owners and Admins set, rotate, or
  revoke it.
- **Set and revoke bump `revision`** (as creating an invitation did), so a
  desktop upload opened against the old revision answers
  `revision_conflict` at commit and is retried; the answers carry
  `repository_revision`. Revoke is idempotent: a repository without a code
  answers `ok` under the same revision guard.
- **Accept skips the batch for an existing collaborator** (a pre-read of
  their role) so the summary comes back unchanged, and it runs one batch
  otherwise: guard on the code and the active row, the 50-member cap,
  `INSERT OR IGNORE`, revision + 1, open request invalidated, audit.
- **`LIMITS`** lost `pendingInvitationsPerRepository`, `invitationLabelChars`,
  `invitationSeconds`, `closedInvitationRetentionSeconds`; the 30-day
  retention of revoked sessions that borrowed the last one is now
  `revokedSessionRetentionSeconds`. `GET /v1/me` no longer reports
  `pending_invitations_per_repository`.
- **The website holds a freshly issued code in the page**, not the Members
  tab: a summary reload shows the page spinner, which unmounts the tab, so
  tab-local state would have lost the code the moment it was created. The
  Members tab has one panel: the state line, Create code / Rotate code /
  Revoke, and the code once with Copy. `content.ts` distinguishes
  `collaborator` (a role, or Super Admin: Members and Settings) from
  `privileged` (a writer, or Super Admin: the latest push, History, the
  repository content routes); the About rail prints "contributor" for a
  signed-in non-collaborator of a public repository.
- **The desktop stub** keeps the code in clear (`Repository.invite_code`,
  `StubState::set_invite_code`), answers `PUT`/`DELETE …/invitation` and
  `POST /v1/invitations/accept`, and treats every signed-in caller of a
  released repository as a writer. `accept_managed_cloud_invitation`
  returns a `ManagedCloudRepositoryResolution` (`Collaborator`, or
  `NotAProject` for a catalog code) so the picker's one switch handles it.
- **Picker copy.** The verdict for a non-collaborator reads `Public ·
  anyone signed in can push` and warns only about the Git remote; the
  found row's verb is always `Use this`; the dialog footer says "Public
  repository: anyone signed in can push" under Connect and review Pull;
  the storage card's chip is `Contributor` with a cloud mark, no longer a
  warning.

Tests: 62 service cases (new `test/contributors.test.ts`,
`test/collaboration.test.ts` rewritten around the repository code), 360
desktop frontend integration cases, and the Rust stub-backed tests in
`managed_cloud_link.rs` (an address resolves for a stranger with
`can_push`; a code joins and selects) all pass.

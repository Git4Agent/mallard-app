# Repository picker: the empty box and the stacked fields

Status: **B implemented 2026-09-27** (see section 4). The mock
(`repo-picker-mockup.html` + `src/repo-picker-mockup.tsx`) showed Before, A,
B and C across eight scenes. Its Before column drove the shipped two-field
picker, so the mock was removed once B shipped.

## 1. What is wrong

The picker has three hosts: New project › From Mallard Cloud (`mode="clone"`),
the connect dialog (`rows="choice"`), and the setup band (`offerNew`). All
three share these faults:

1. **The empty list looks like a text field.** The empty line is a `<small>`
   *inside* the `radiogroup` container. That container carries the list-box
   border, the offset card shadow and the row fill. With no rows, what is left
   is a bordered strip holding one line of small text, flush with the left
   edge. Restyling the line doesn't fix this; the box must not render.
2. **The order splits things apart.** The order is address field, the lookup's
   answer, the code field, then the list. The list has no heading, so its rows
   look like part of the code field. The empty line's "Paste an address above"
   refers to a field two rows up.
3. **The edges don't line up.** Look up and Join (or Join repository) differ in
   width, so the two inputs end at different points. Join's lock icon makes the
   disabled button look locked.
4. **"(optional)" misleads.** It reads as one more field in the same form. The
   code is really a second way to name a repository.
5. **A found repository shows twice** when it is also one of the account's
   rows: once in the answer card, once as the selected row.

## 2. Directions

- **A · Tidy in place.** Same fields, same order. The list gets a heading and
  loses its box when empty. The buttons share one width. The code label reads
  "Invitation code" plus the aside "for a private repository". The Join icon is
  `user`.
- **B · One field (recommended).** One field, "Address or invitation code". The
  button reads Look up, or Join once the text is a code, with a hint line saying
  so. Below it come the answer, then the heading and the list. The "visible
  before any lookup" intent (pinned in
  `managed-cloud-connect.integration.test.tsx`) still holds, because the label
  names the code.
- **C · Code behind a link.** The design before the field became visible:
  "Have an invitation code?" under the address, and a miss opens the code
  field. **This undoes a deliberate, tested decision.**

All three share these fixes:
- The empty state is a heading, "Your repositories", followed by the quiet line
  "None yet. Repositories you create or join appear here." No box.
- The heading shows a count when there are rows.
- A listed repository that a lookup finds is not repeated (in every host). The field shows one
  line, "In your repositories. Selected below.", and the row is selected. The
  answer card stays for anything outside the list: a public repository, or one
  just joined by code.

## 3. Implementation (for B; A and C need a subset)

Frontend only. The commands `resolve_managed_cloud_repository` and
`accept_managed_cloud_invitation` stay as they are, and nothing changes in the
service.

`src/components/project-sync/RepositoryAddressPicker.tsx`
- Replace the `address` and `code` state with one `entry`. Enter and the button
  call `runJoin` when `looksLikeInvitationCode(entry)`, and `runLookup`
  otherwise. `Lookup` and `Join` stay separate states. A join error shows under
  the one field.
- `looksLikeInvitationCode`: the entry has no `/` and no `.`, and it is
  exactly 20 characters after stripping spaces and hyphens. An address always
  has a `/`, so that is what tells the two apart. The alphabet check stays
  with the service (`normalizeInvitationCode` in `managed-cloud/src/ids.ts`),
  so a mistyped code (O, 0, I, 1) still gets the code's error from
  `accept_managed_cloud_invitation`, not "Paste a page address". This
  replaces `isInvitationCodeShaped`, which counts length only and so passes a
  20-character `owner/name`.
- On a successful join, fill `entry` with the repository's address (as today).
- The answer: when the resolved repository's `bundle_id` is among
  `listing.repositories`, render the one-line pointer instead of the found
  card. In connect mode that drops the card's Git-remote caution and its Open
  on website link. A mismatched remote is still caught by the existing
  Acknowledgement box. If that is too much, limit the rule to `clone` mode.
- Known rough edge: in the setup band, "Your repositories · 2" sits above the
  New repository row, which isn't one yet.
- The list: always render the "Your repositories" heading. When there are no
  rows and no New row, render `p.repo-list-empty` and no `radiogroup`.
- The verb button gets the `repo-verb` class. The label is "Join" in both
  modes. The icon is `user`.
- Not-found copy: "…For a private repository, paste its invitation code here
  instead."

`src/App.css`: append the mock's "proposed app CSS" block to the "Connect by
address" rules (around `.repo-connect`): `.repo-verb`, `.repo-field-label`,
`.repo-list-head`, `.repo-list-empty` and `.repo-answer-line`. Run the
css-hygiene test.

`src/components/project-sync/BundleConnectionDialog.tsx`
- Header copy: "Paste a repository address or an invitation code, or choose one
  of your repositories."
- Footer prompt: "Paste an address or a code, or select a row."

Tests
- `tests/frontend/managed-cloud-connect.integration.test.tsx`: lines ~124–130
  pin "Invitation code (optional)", "Join repository" and "No repository
  address is needed". Replace those pins with the one field, and add these
  cases:
  - an address with `/` stays Look up;
  - a 20-character `owner/name` stays Look up;
  - a code with mixed case and spaces reads Join;
  - an empty listing renders no `radiogroup`;
  - a listed repository found by address renders the pointer line, not the
    card.
- Check `tests/frontend/cloud-project.integration.test.tsx` for the clone-mode
  strings.

Docs
- `docs/DESIGN_MANAGED_CLOUD_CONNECT_BY_ADDRESS.md`: section 8, "As built".
- `docs/DESIGN_MANAGED_CLOUD_OPEN_PUBLIC_PUSH.md`: the frontend bullet on the
  code field.
- Website (`mallardInternal/cloudflare/managed-cloud`): these app shots quote
  today's wording.
  - `web/landing-mock/add-project-mock.tsx`: `CodeField`, "Join repository",
    and the "enter the invitation code and click Join" line.
  - `web/landing-mock/cloud-docs-mock.tsx`: around lines 284 and 300–302.
  Both are mocks still awaiting a pick, so update them when they are ported.

Verify: open `mockups/cloud-project-live.html` and `mockups/cloud-connect-live-mockup.html` in
the Browser pane, light and dark, and go through empty, list, found, not found,
code and join.

## 4. As built

Built as section 3 describes, with these specifics:

- `RepositoryAddressPicker.tsx` now has one `entry` state.
  - `isInvitationCodeShaped` has the new rule and keeps its name.
  - `inListing` is new and exported.
  - A failed join clears when the field is edited, or when a lookup or a row
    click happens.
  - Switching account re-runs a lookup for an address, but never joins with a
    code: joining is the person's own click.
  - "Invalid" answers now mention the code too.
  - A repository joined by code (`joined` state) is the first row until the
    listing is read again, so a join ends like a listed lookup: one line and
    a selected row. Otherwise the list would still say "None yet" under the
    repository just joined. It clears on an account switch.
- `App.css`: the block after the "Connect by address" rules adds `.repo-verb`
  (min-width 104px), `.repo-list-head`, `.repo-list-empty` and
  `.repo-answer-line`.
- `BundleConnectionDialog.tsx`: the header and footer copy from section 3.
- Tests:
  - `managed-cloud-connect.integration.test.tsx`: code rule cases (a
    20-character `owner/name`, a URL, a mistyped code), `inListing`, one
    `<input>`, the heading and count, and the empty state with no
    `radiogroup`.
  - `cloud-project.integration.test.tsx`: the field label, and Look up (no
    Join) before a code is typed.
  - `npm run test:frontend-integration`: 460/460. `tsc --noEmit` is clean.
- Checked in the live harnesses (`mockups/cloud-project-live.html`,
  `mockups/cloud-connect-live-mockup.html`), light and dark:
  - no repositories;
  - a listed repository looked up (row selected, one line, no card);
  - an account switch re-looking up to not found;
  - a mistyped code (the code's error under the field);
  - a code joined (the field shows the address, its row is selected under
    "Your repositories 1", and Project opens);
  - an account switch with a code typed (no join);
  - the dialog with a different-remote row (the Acknowledgement box still
    appears, and Connect waits for it).
- Not done: the website mocks `web/landing-mock/add-project-mock.tsx`
  (`CodeField`, `CloudJoinShot`, and the "enter the invitation code and click
  Join" line) and `web/landing-mock/cloud-docs-mock.tsx` (around lines 284
  and 300–302) still draw the two fields. Update them when those pages are
  ported.

# UI audit — storage setup, editing, health and removal (snapshot 2026-09-11 17:15 PDT)

Auditor slice: storage setup/editing/health/removal. Source: frozen snapshot of commit 0c6813e plus uncommitted work. Paths are relative to the snapshot root.

Reading order, most important first: ST-2, ST-1, ST-5, ST-7, ST-6, ST-4, ST-3, ST-11, ST-8, then the Lows ST-9, ST-10, ST-12. Counts: 0 High, 9 Medium, 3 Low.

### ST-1 · "Test connection" on an existing storage gives no success signal
- **Severity:** Medium
- **Where:** Storage page → ⋯ → Storage settings (also the Reconnect path: the danger notice's "Reconnect <provider>" button opens the same card) → card footer "Test connection"
- **Evidence:** `src/components/project-sync/ProjectLinksWorkspace.tsx:1894` — "{storageTestVerified && (… Connection verified" is rendered only inside the new-draft footer opened at `:1881` — "{creatingStorage && showStorageDetails && storageSession && (". The existing-storage footer at `:1840-1855` has only the two buttons ("Testing…" / "Test connection", "Save storage"); `:1399-1400` sets `storageTestVerified` on success but nothing on this page reads it. A failure goes to the shell error zone (`src/components/project-sync/ProjectSync.tsx:2835` — "setError(errorMessage(reason))").
- **Impact:** After pasting a replacement key or running "Reconnect with Wrangler", the user presses Test connection and sees the label flip back with nothing else — they cannot tell a pass from a no-op, which is exactly the moment they need confirmation before saving. New-storage setup shows "Connection verified", so the same verb behaves differently on the two pages.
- **Fix:** Render the same `storage-setup-verified` chip ("Connection verified") beside the buttons in the existing-storage footer (`sync-storage-editor-footer`), cleared on edit exactly as it is today.
- **Confidence:** confirmed from source

### ST-2 · A passing "Test connection" of unsaved replacement credentials clears the storage's "needs reconnection" state, though the revoked key is still the saved one
- **Severity:** Medium
- **Where:** Storage whose credential was revoked or expired → storage page danger notice → "Reconnect <provider>" (or Storages index card "Reconnect" chip) → Storage settings → paste a new key, or run "Reconnect with Wrangler" → "Test connection" → leave without "Save storage"
- **Evidence:** `crates/mallard-application/src/storage_setup.rs:1234-1237` — "if result.is_ok() { … ConfiguredObjectStore::forget(&pending.id);" runs for Test as well as commit (`:680-686` in `test_storage_setup_draft`), and an edit draft carries its storage's id (`src/components/project-sync/ProjectLinksWorkspace.tsx:736-738` — "An edit draft shares its storage's id"). `crates/mallard-infrastructure/src/storage_core/mod.rs:392-397` drops the recorded health and sends a connection event; `src/components/project-sync/ProjectSync.tsx:1353-1363` re-reads the config on "project-sync-storage-connection-changed". With `connection` gone, `connectionSuspends` is false, so the danger notice (`ProjectLinksWorkspace.tsx:1660-1676`), the index "Reconnect" chip (`StorageIndexPage.tsx:97-101`) and the red guide row all disappear; the guide row now reads "Configured. You can review or update the connection values below." (`StorageSettings.tsx:495`). Nothing was committed — the saved config still holds the refused key until "Save storage".
- **Impact:** The only visible reaction to a successful test on this page (see ST-1) is the red "needs reconnection" state going away, which reads as "fixed". A user who leaves at that point keeps the revoked credential: the index and sidebar drop their "Reconnect" marks until the next Push, Pull or listing fails again, and only a separate "Draft · not applied yet" entry (ST-3) hints that nothing changed.
- **Fix:** Forget recorded health only on commit, or only when the tested draft's credentials equal the saved ones; keep the notice up while an edit draft with a different credential is unsaved, and after a pass say "Connection verified — select Save storage to use this key."
- **Confidence:** confirmed from source (the next storage read re-records the refusal, so the false-healthy window lasts until something reads the storage)

### ST-3 · Just opening "Storage settings" leaves a duplicate "Draft · Unsaved … settings — not applied yet" entry, with no Discard on that page
- **Severity:** Medium
- **Where:** Any connected storage → ⋯ → "Storage settings" (or the "Reconnect <provider>" button) → leave without changing anything → Storages index and sidebar
- **Evidence:** `src/components/project-sync/ProjectLinksWorkspace.tsx:1041-1044` — opening settings calls "projectSyncApi.createStorageEditDraft(storage.storage_id)", which always persists a draft (`crates/mallard-application/src/storage_setup.rs:474,494` — `create_storage_edit_draft` → "let saved = save_draft("); `list_storage_setup_drafts` lists it without checking for changes. The index then shows a second card with the storage's own name: `src/components/project-sync/StorageIndexPage.tsx:125-127` — "`Unsaved ${draft.provider_label} settings — not applied yet`" plus a "Draft" badge; the sidebar adds "Continue setting up <name>" with a "Draft" badge (`src/components/project-sync/ProjectSidebar.tsx:477-482`). Hiding settings keeps the draft (`ProjectLinksWorkspace.tsx:1486-1487` — "Hide keeps the draft … and the page's own Discard is the way to drop it"), but the existing-storage footer has only "Test connection" and "Save storage" (`:1838-1857`); "Discard draft…" exists only in the new-storage footer (`:1899-1906`).
- **Impact:** A user who only looked at their settings is told they have unsaved, unapplied changes and sees every such storage twice, once as "Draft". The only way to clear it is the sidebar's trash icon, which reads like deleting the storage.
- **Fix:** Create the edit draft on the first real edit, or discard it on close when it still matches the saved storage; give the existing-storage card a "Discard changes" button when a changed draft exists.
- **Confidence:** confirmed from source

### ST-4 · The trash glyph means "Remove storage (files kept)" on the settings card but "Clear storage data (permanent)" in the ⋯ menu
- **Severity:** Medium
- **Where:** Storage page → ⋯ (Storage group) versus the Storage settings card header of a storage with no linked projects
- **Evidence:** `src/components/project-sync/StorageSettings.tsx:537` — `title="Remove storage" aria-label="Remove storage"><Icon name="trash" size={14} />` (icon only, no text); `src/components/project-sync/ProjectLinksWorkspace.tsx:1597-1600` — `label: "Clear storage data…", icon: "trash" as const, danger: true`; `:1605-1607` — `label: "Remove storage…", icon: "x" as const`.
- **Impact:** The same red trash icon stands for the one irreversible storage action (deleting every synced repository, for every machine) and for a harmless one (forgetting the connection, files kept). Someone who learned "trash = remove" on the card can reach for "Clear storage data…" expecting removal; the confirm dialog is the only safeguard.
- **Fix:** Use the menu's "x" glyph plus a text label ("Remove…") for the card's button, and reserve trash for "Clear storage data…".
- **Confidence:** confirmed from source

### ST-5 · One click on the type switch or "Reset setup" throws away a finished R2 setup, including the key Cloudflare issues once — no confirmation
- **Severity:** Medium
- **Where:** Add storage → Cloudflare R2 → Set up with Wrangler → "Ready for review." → click "Local folder"/"Dropbox" in the card header's type switch, or "Reset setup"
- **Evidence:** `src/components/project-sync/StorageSettings.tsx:529` — the type switch stays live on a new draft (`disabled={disabled || editsExisting || !onSwitchProvider}`) and calls `switchStorageProvider`, which discards the draft with no prompt (`src/components/project-sync/ProjectLinksWorkspace.tsx:1450-1466`). "Reset setup" is on the review state (`src/components/project-sync/R2Setup.tsx:732`) and runs a straight discard (`ProjectLinksWorkspace.tsx:1699-1716`); its warning is only a hover title, `R2Setup.tsx:386` — "Discards the entire local setup draft and all setup progress. Any bucket already created in Cloudflare and any connected storage are kept." The key step says `R2Setup.tsx:682` — "Issued · shown once, in the fields below". "Discard draft…" by contrast confirms first (`ProjectLinksWorkspace.tsx:1406-1409`, "external storage resources are not changed").
- **Impact:** After sign-in, account confirmation, bucket creation and browser consent, a stray click erases the issued Secret Access Key; the user must repeat every browser step, and the discarded key stays valid in their Cloudflare account without Mallard ever saying so.
- **Fix:** Confirm before discarding any draft that holds a secret or a created bucket, naming what stays in Cloudflare ("bucket <name> and its access key"); lock the type switch once a lane step has run.
- **Confidence:** confirmed from source

### ST-7 · The existing-bucket scan says "No Mallard storage in your Cloudflare account" under a bucket it lists as "Needs a newer Mallard", and offers "Create a new bucket instead"
- **Severity:** Medium
- **Where:** Second Mac: Add storage → Cloudflare R2 → "Use existing" → "Find in my account" → Buckets step results
- **Evidence:** `src/components/project-sync/R2Setup.tsx:548` — `const found = … .filter((candidate) => candidate.marker === "storage").length`; `:597-605` renders `DiscoveryEmpty` whenever `found === 0`, beside the groups drawn by `src/components/project-sync/StorageDiscovery.tsx:192-214` ("Needs a newer Mallard", "Could not be checked"). The empty box reads `StorageDiscovery.tsx:250` — "No Mallard storage in {place}." with the primary button `:254` — "Create a new {unit} instead". The backend emits "unsupported" for a marker written by a newer Mallard (`crates/mallard-infrastructure/src/storage_core/marker.rs:123-124`).
- **Impact:** A Mac running an older Mallard, or one whose marker read failed, is told there is no storage and handed a primary button to create a fresh bucket, splitting the user's history across two storages instead of prompting an update or an access fix.
- **Fix:** Show the empty box only when every candidate is "missing"; otherwise lead with "Update Mallard to use <bucket>" or "Check access to <bucket>, then Scan again", with "Create a new bucket" secondary.
- **Confidence:** confirmed from source

### ST-6 · "Clear storage data" makes the user certify "sync has stopped on all machines" — a state Mallard gives no way to reach or check
- **Severity:** Medium
- **Where:** Storage page → ⋯ → "Clear storage data…" → confirmation dialog
- **Evidence:** `src/components/project-sync/ProjectSync.tsx:2908-2909` — "Stop sync on all machines before you continue. By choosing “Sync stopped — clear”, you confirm that sync has stopped on all machines." with `okLabel: "Sync stopped — clear"`. There is no storage-wide stop or pause control; the nearest verb is per project, `src/components/project-sync/ProjectLinksWorkspace.tsx:2034-2038` — "Unlink from <project>…" titled "Stop syncing this pairing; files already in storage are kept". Push and Pull are manual, reviewed actions, so nothing is running that could be stopped. For R2 the clear is unconditional (`crates/mallard-infrastructure/src/storage_providers/r2.rs:431` — `clear_mode: … Unconditional`), so a Push from another Mac during the clear is exactly the case the warning exists for.
- **Impact:** On the only irreversible storage action, the user must click a button asserting something they cannot do or verify from the app. Most will click through; careful users will wonder whether they must unlink every project on every Mac first, and the dialog never says what those other Macs will see afterwards.
- **Fix:** Say it concretely: "Make sure no one runs Push or Pull to this storage from another Mac until clearing finishes", state what linked Macs will see afterwards, and use a plain destructive label such as "Clear all Mallard data".
- **Confidence:** confirmed from source

### ST-8 · Dropbox is introduced as "An existing shared or team namespace" — contradicting its own Create-new door — and its storages are labelled by a raw namespace ID
- **Severity:** Medium
- **Where:** Add storage… → "Choose a storage type" cards; Dropbox new-storage doors; Storages index card, crumb menu and "Switch to" list of any Dropbox storage
- **Evidence:** `crates/mallard-infrastructure/src/storage_providers/dropbox.rs:66` — `description:"An existing shared or team namespace. Uploads up to 150 MiB per object."` (printed on the picker card, `src/components/project-sync/StorageProviderPicker.tsx:71`), while `dropbox.rs:80` — "A new Mallard shared folder in your Dropbox, created after browser consent." is the Create-new door (`StorageSettings.tsx:546`). The only summary fact is `dropbox.rs:118-119` — `label: "Namespace".into(), value: typed.namespace_id`, which `storageSubtitle` (`src/components/project-sync/StorageSettings.tsx:174-178`) prints as the card location (`StorageIndexPage.tsx:80`, `LandingPicker.tsx:130`). The R2 card reads `r2.rs:422` — "An R2 bucket reached through its S3 API."
- **Impact:** A Dropbox user deciding which provider to pick is told they need an existing "namespace" (a term Dropbox's own UI never shows) and may skip a provider that would create the folder for them; afterwards every listing identifies the storage by an opaque ID instead of the folder name they approved. The picker describes transports ("S3 API", "object") rather than what each option does for them.
- **Fix:** Describe outcomes: "A Mallard folder in your Dropbox — new, or one you already use", "A Cloudflare R2 bucket — Mallard can create it with Wrangler", "A folder on this Mac, a mounted drive, or a synced folder". Show the shared folder's name as the Dropbox summary fact and keep the namespace ID as secondary detail.
- **Confidence:** confirmed from source (Dropbox copy landed today and may still change)

### ST-9 · The reconnect lanes tell the user to "Save changes", but the button is "Save storage"
- **Severity:** Low
- **Where:** Existing storage → Storage settings → "Reconnect with Wrangler" → step list, last row "Review"
- **Evidence:** `src/components/project-sync/R2Setup.tsx:693` — `fact: mode === "reconnect" ? "Test connection, then Save changes" : "Test connection, then Finish setup"` (same string in `src/components/project-sync/DropboxSetup.tsx:238`); the actual button is `src/components/project-sync/ProjectLinksWorkspace.tsx:1854` — `"Save storage"`.
- **Impact:** At the step that finally applies the new key, the instruction names a control that does not exist; together with ST-1/ST-2 this makes it easy to stop before saving.
- **Fix:** Use one verb in both places — "Test connection, then Save storage" (or rename the button "Save changes" everywhere).
- **Confidence:** confirmed from source (the Dropbox copy is in flux at snapshot; the R2 copy is settled)

### ST-10 · "Remove storage" and "Discard storage draft" confirmations fall back to a generic "Ok" button
- **Severity:** Low
- **Where:** Storage page → ⋯ → "Remove storage…" (or the card's trash button); new-storage footer → "Discard draft…"
- **Evidence:** `src/components/project-sync/ProjectSync.tsx:2952-2955` — `confirm(… "Files already written to the storage location will not be deleted.", { title: "Remove storage" })`; `src/components/project-sync/ProjectLinksWorkspace.tsx:1406-1409` — `{ title: "Discard storage draft" }`. Neither passes `okLabel`, so `@tauri-apps/plugin-dialog` `confirm` shows its default "Ok"/"Cancel" pair; the Clear dialog, by contrast, sets `okLabel: "Sync stopped — clear"` (`ProjectSync.tsx:2909`).
- **Impact:** The user confirms a removal or discard with "Ok", which does not restate the action; next to the purpose-labelled Clear dialog the three storage confirmations feel unrelated.
- **Fix:** Pass `okLabel: "Remove storage"` / `okLabel: "Discard draft"` (and `kind: "warning"`), matching the Clear dialog's pattern.
- **Confidence:** confirmed from source (default label per the installed `@tauri-apps/plugin-dialog` typings in the live repo's `node_modules`, not part of the snapshot: "Shows a question dialog with `Ok` and `Cancel` buttons")

### ST-11 · An existing R2 or local-folder storage can be silently re-pointed at another bucket or folder from its settings; only Dropbox says the destination is fixed
- **Severity:** Medium
- **Where:** Storage page → ⋯ → Storage settings → "Location path" + "Browse…" (Local folder) or "S3 API URL" / "Bucket name" / "Account ID" (R2) → "Save storage"
- **Evidence:** `crates/mallard-infrastructure/src/storage_providers/local.rs:132` and `r2.rs:434,448,463,478` — `immutable_on_edit: false`, so `src/components/project-sync/StorageSettings.tsx:573,681` leave the fields editable with no help text about what a change means (editing the S3 API URL also rewrites Bucket name and Account ID through `applyFieldDerivations`). Dropbox locks its field and says why: `dropbox.rs:69` — "Use the same namespace ID on each machine. This value cannot change after setup."; the backend refusal is `crates/mallard-application/src/storage_setup.rs:2363` (in `validate_immutable_fields`, `:2346`) — "This destination cannot change. Add a new storage connection." Only a collision with another configured storage is refused (`ensure_destination_available`, `storage_setup.rs:1148`).
- **Impact:** A user opening settings to replace a key can change the destination of a storage their projects are linked to, on this Mac only; linked projects then compare against different data than the other Macs, and nothing on the page warns that this is a different storage.
- **Fix:** Make the destination read-only on edit for every provider (as Dropbox does, with "Add a new storage to use a different bucket/folder"), or require an explicit "Move this storage" confirmation that states the effect on linked projects and other Macs.
- **Confidence:** needs runtime check (source confirms the fields are editable and accepted; what linked projects do after a re-point was not exercised)

### ST-12 · Nothing tells a second Mac how to join the same Local-folder storage
- **Severity:** Low
- **Where:** Second Mac: Storages → Add storage… → "Local folder"
- **Evidence:** `crates/mallard-infrastructure/src/storage_providers/local.rs:121` — `description: "A folder on this Mac or on a mounted drive."`; the only field has `help: None` (`:138`) and the placeholder "e.g. /Volumes/backup/mallard-storage" (`:137`); `discovery: None` (`:147`) so there is no "Use existing" door. R2 and Dropbox get "Use existing" doors (`r2.rs:506`, `dropbox.rs:81`).
- **Impact:** The product's core promise is moving sessions between Macs, but the Local folder path reads as machine-local; users are not told that both Macs must point at the same shared or synced folder, and nothing distinguishes joining an existing Mallard folder from starting a new one until repositories appear after Finish.
- **Fix:** Add help under "Location path" ("To use this storage on another Mac, pick the same folder there — for example on a shared drive or a synced folder") and, when the chosen folder already holds a Mallard marker, say "Existing Mallard storage found" before Create storage.
- **Confidence:** confirmed from source

## Prior findings status
- **Storage validation without field-level errors — worse.** The URL field's `aria-invalid` is gone: no `StorageEditor` input carries `aria-invalid` now (`src/components/project-sync/StorageSettings.tsx:674-687`); the only one in storage setup is the R2 lane's Bucket name input (`src/components/project-sync/R2Setup.tsx:626`).
- **R2 finishing without a successful test — unchanged, and safe in effect.** "Finish setup" is gated only on readiness and folder conflicts (`ProjectLinksWorkspace.tsx:1361`), but the commit re-runs the same probe (`crates/mallard-application/src/storage_setup.rs:714-720`).
- **"Add storage…" collapsing to "+" with one storage — not re-checked.** The Storages index head still renders the text label beside the plus (`src/components/project-sync/LandingPicker.tsx:79-82`); the sidebar case was not examined.
- **Agent-setup split menu (keyboard contract, focus restore) — not re-checkable.** The storage agent route was being removed at snapshot time (`setup_assistant` dropped from descriptors; `ProjectLinksWorkspace.tsx:1864` calls an undefined `setAgentNotice`). Escape focus and segmented-control items were not re-checked.

## Done well
- Secrets stay out of view after commit. An edit draft copies the committed secret and stays redacted (`crates/mallard-application/src/storage_setup.rs:356-360`), and the draft-changed event carries no values (`ProjectSync.tsx:1337-1339`). Secret inputs are password fields with `autoComplete="new-password"` and an explicit Show/Hide (`StorageSettings.tsx:704-725`). The R2 helper keeps the key in `Zeroizing` memory, runs Wrangler with log writing off and sanitizing on (`crates/mallard-cli/src/r2_browser_setup.rs:265-279`), and prints no secret to the CLI output path.
- The R2 lane leaves the user in control. It never opens a browser on its own ("Copy login link" / "Open browser", `R2Setup.tsx:452-459`), confirms the Cloudflare account before anything is created or listed (`:271-276`, `:524`), and pins each failure to the step it belongs to, with its own "Try again".
- "Connected" is claimed only after an observed success (`StorageSettings.tsx:485-495`). A refused credential shows up everywhere it matters (index "Reconnect" chip, storage-page notice, failed Push/Pull), each with one "Reconnect <provider>" route into the same settings (`StorageIndexPage.tsx:97-101`, `ProjectLinksWorkspace.tsx:1661-1676`, `:2890-2902`).
- A folder already used by another storage is refused before any verb is offered, with only two ways on: "Open <name>" and "Choose another…" (`StorageSettings.tsx:587-611`). The backend enforces the same rule (`crates/mallard-application/src/storage_setup.rs:1148`).
- Scans say they are read-only and that nothing was changed, and a partial scan warns that a storage there would not be listed (`StorageDiscovery.tsx:222-235`, `R2Setup.tsx:557-560`). Remove is blocked while projects are linked, and the menu says why (`ProjectLinksWorkspace.tsx:1609-1615`).

## Not verified
- Everything here comes from reading source; the Tauri app was not run. The ST-2 false-healthy window lasts until the next storage read, which is a runtime detail. ST-11's effect on linked projects was not exercised.
- DropboxSetup.tsx and the agent-assisted route were in flux at snapshot time (per the coordinator), so only their copy was read. Two in-flux behaviours were seen but deliberately not reported: after a successful Dropbox create, the lane still shows "The original folder and draft are kept." plus "New setup for another account" (`DropboxSetup.tsx:248-250`, `creationStarted` stays true); and its "Try again" always restarts sign-in, even when the message says "Scan again" (`DropboxSetup.tsx:43`, `:246`).
- Not checked: visual rendering, contrast and focus order of the lanes; whether the shell error zone that receives Test/Finish failures (`ProjectSync.tsx:2835`, `:2871`) stays visible when the card footer is scrolled into view; and the sidebar's "Add storage" collapse.

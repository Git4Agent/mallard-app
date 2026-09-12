# Mallard customer-facing UI audit — 2026-09-11

Date: 2026-09-11
Audited code: source snapshot taken 2026-09-11 17:15 PDT from base commit `0c6813e` plus the uncommitted working tree
Application version: `0.1.2`

## Executive result

Where Mallard's copy was designed, it is good. Destructive confirmations say what stays untouched,
the R2 setup lane never acts without the user, Settings now confirms every save, and Activity tells
stored-only sessions apart from local ones. The weak places are the paths nobody wrote copy for:
failures, zero states and settings changes. There the UI either shows internal text or says
something that is not true.

The audit found **6 High, 30 Medium and 17 Low** distinct findings, plus **3 regressions** in
fixes from earlier audits. Three of the six Highs share one theme: the screen says something false
about sync state.

1. **Pull with nothing selected reports "Pull complete"** (H02). Nothing was restored, and the
   storage is marked reviewed.
2. **Turning credential redaction back on leaves every session marked synced** (H05). The storage
   keeps the unredacted copies until the user happens to push.
3. **Non-Git projects are told to "remove it during Push"** (H06). No control for that exists.

The other three are about failure. An unreachable storage turns Push into a dead review whose only
offered fix erases local sync state (H01). Storage errors reach the screen as SDK prose on most
paths (H04). And ⌘Q quits in the middle of a sync without the warning the app shows for every other
way of closing (H03).

The 2026-08-30 and 2026-09-01 audits are in good shape: 18 of their findings are fixed and 3
surfaces are gone. Three fixes regressed when their surfaces were reworked: the Sessions/Files
switch, the Catalog Info popover and the app-actions menu. One finding the 09-01 audit closed as
fixed never was: the focus rings on status chips, and on metric chips in light, still fall below 3:1. A latent button
colour fallback that was flagged twice has also got worse (2.41:1 in dark).

## Scope and method

The audit covers what a customer sees and does: first launch, adding a project, setting up a
storage, linking it, Push and Pull, reading Activity and session detail, the Catalog, Settings and
Logs. Code quality is out of scope unless it produces a visible defect.

- **Source review of a frozen snapshot.** Several sessions were editing the live tree during the
  audit, so five auditors each read a frozen copy. They covered shell and onboarding, storage setup,
  sync flows, reading surfaces, and backend-authored copy. A sixth re-checked every prior finding.
  Their raw findings are in the `findings-*.md` files beside this report, with IDs SH, ST, SY, RD
  and CP.
- **Live verification.** All six High findings were re-verified against the live tree at about
  17:50 PDT, and their citations are live-tree lines. Entries marked *verified live* below were
  spot-checked the same way. Every other citation is a snapshot line and may have drifted by a few
  lines.
- **Runtime capture.** The production React tree and stylesheet were rendered in headless Chrome
  against a mocked backend: the real `App`, with every command it called answered by fixtures. The
  capture covers first run, the three landings, project Activity, Settings, Logs, and the Pull and
  Push reviews, at 1100 × 720 and at the 800 × 560 minimum window, in both themes. Session detail,
  storage pages, the Files view and failure scenarios were not captured. Chrome is not the
  WKWebView the app ships in, so widths can differ slightly.

Not exercised: the native Tauri shell, native dialogs, VoiceOver, real storage providers, and real
provider homes.

At snapshot time another session was midway through a storage-setup refactor, and the live tree
did not typecheck (`setup_assistant` had just been removed from the storage descriptor type). That
work in progress is not reported as a customer finding.

## Surface health

| Step | Surface | Health | Audit result |
|---:|---|---|---|
| 1 | Startup and recovery | Good with one concern | Stages are named in plain words. The failure screen is accessible now, but its title "Host identity unavailable" is internal language and Retry is its only exit. |
| 2 | Shell and navigation | Good | One icon strip with `aria-current`, one index per root. The duplicate navigation from 08-30 is fixed. |
| 3 | First run | Needs repair | The dead end is gone, but nothing says what Mallard does or that a storage is step two. Adding a storage from a project does not link it. |
| 4 | Joining from a second Mac | At risk | The repo chooser preselects nothing, and its only live action splits the histories. The R2 scan can say "No Mallard storage" beside a bucket that needs a newer Mallard. |
| 5 | Storage setup and settings | Needs repair | The R2 lane is careful and secrets stay hidden. Testing existing credentials gives no result and can clear a real warning. Opening settings leaves a duplicate draft. |
| 6 | Push | At risk | An unreachable storage produces a dead review with a destructive fix. Capped pushes, file pushes and partial failures are misreported. |
| 7 | Pull | At risk | An empty Pull reports success. Failure text is internal, and the Reset offer depends on matching that text. |
| 8 | Project Activity and session detail | Good with concerns | Stored-only sessions are clearly separated. Unreadable sessions vanish silently. Settings changes leave stale synced checks. |
| 9 | Catalog | Needs repair | The concept is never explained. One status promises a Push that is refused. |
| 10 | Settings and Logs | Good with concerns | Save receipts read well. Switches don't state their cost, Reset Mallard understates what it deletes, and log lines carry no dates. |

## What works well

1. Destructive confirmations say what is *not* touched ("The checkout and provider files stay on
   disk", "Files already written to the storage location will not be deleted"), and Clear storage
   data cannot be approved with a bare OK.
2. The R2 lane leaves the user in control. It never opens a browser on its own, confirms the
   Cloudflare account before creating or listing anything, and pins each failure to its step with
   its own Try again.
3. Secrets stay out of view. Committed secrets are redacted in edit drafts, draft events carry no
   values, and the R2 helper keeps keys in zeroised memory with Wrangler logging off.
4. Settings confirms every save: a per-row "Saving…" / "Saved" stamp, one status line in the bar,
   and a failure that jumps to its row.
5. Activity puts stored-only sessions under their own "Only in <storage>" heading, and every status
   tooltip names the action that resolves it.
6. Keep-both is explained before and after a Pull, and the size-cap refusal says how far over the
   push is and what to do.
7. Startup names each stage in plain words, including the slow one: "Finishing interrupted work…".
8. Some backend strings already set the standard the rest should follow, for example
   "Dropbox could not be reached. Check the network and try again."

## Findings

### High priority

#### H01 — When the storage can't be reached, Push opens a dead review whose only offered fix erases local sync state

A Push while offline, with an expired or revoked credential, or with the bucket unreachable does
not stop at the storage card like the other Push gates. It opens the review with every session
unselectable (0/0), a danger banner titled "Conversations couldn't be compared, so nothing can be
selected", the raw transport error as its message, and one action: "Reset local sync state". That
reset makes Mallard forget this project's reviewed sync bases, Pull results and restored-session
path mappings. It is the wrong repair for a temporary outage, and its effects outlast it.

Evidence: the Push gate handles only `requires_pull`, `remote_history_missing` and
`remote_history_replaced`
([ProjectSync.tsx:1737](../../../src/components/project-sync/ProjectSync.tsx#L1737)) and passes the
comparison error through untranslated
([ProjectSync.tsx:1815](../../../src/components/project-sync/ProjectSync.tsx#L1815)). The backend
reports the unreachable destination and copies the same error into the comparison
([service.rs:7610](../../../src-tauri/src/project_sync/service.rs#L7610),
[service.rs:7650](../../../src-tauri/src/project_sync/service.rs#L7650)). The banner offers Reset for
every comparison error, on the assumption written in its comment that stale metadata is "the usual
cause" ([PushResourceWorkspace.tsx:719](../../../src/components/project-sync/PushResourceWorkspace.tsx#L719),
[PushResourceWorkspace.tsx:730](../../../src/components/project-sync/PushResourceWorkspace.tsx#L730)).
The reset confirmation is at
[ProjectSync.tsx:2555](../../../src/components/project-sync/ProjectSync.tsx#L2555). Source: SY-3, CP-1.

Repair: treat an unavailable destination like the other gates. Do not open the review; raise the
storage notice with a translated cause and its Retry or Reconnect action. Offer Reset only for
errors recognised as stale local metadata, by code (see M22).

#### H02 — Pull with nothing selected reports "Pull complete"

The Pull review opens with nothing ticked, and its button says only "Pull". Pressing it with
nothing ticked restores nothing, marks the storage as reviewed (which unblocks Push), and reports a
green "Pull complete" with a "0 restored" stat. On a second Mac, the first Pull is exactly where a
user expects their sessions to arrive, sees them listed, and presses Pull. The Catalog's Pull, by
contrast, is disabled at zero and carries its count.

Evidence: selection starts empty
([RestorePlanView.tsx:240](../../../src/components/project-sync/RestorePlanView.tsx#L240)); the
recommended set only feeds a select-recommended button
([RestorePlanView.tsx:302](../../../src/components/project-sync/RestorePlanView.tsx#L302)); the
footer verb has no count
([RestorePlanView.tsx:584](../../../src/components/project-sync/RestorePlanView.tsx#L584)). The empty
apply is deliberate, as an acknowledge-only Pull
([pullReviewFlow.ts:78](../../../src/components/project-sync/pullReviewFlow.ts#L78)), and the path-bar
Pull applies it too
([ProjectSync.tsx:3185](../../../src/components/project-sync/ProjectSync.tsx#L3185)). The outcome is
titled "Pull complete"
([syncOutcome.ts:329](../../../src/components/project-sync/syncOutcome.ts#L329)). Source: SY-1.

Reproduced in the rendered app: the review opens at 0/3 under a green "Done · Pull review ready for
otter-api — nothing applied yet." notice, and pressing Pull ends in "Pull complete · 0 restored"
(screenshots 16 and 17 below).

Repair: keep the acknowledge-only mechanic but give it an honest verb. Put the count on the button
("Pull 3"). At zero, relabel it "Mark as reviewed" and title the outcome "Nothing pulled — storage
marked as reviewed". Consider opening with the recommended rows ticked, as Push does, if the
explicit-approval rule allows it.

#### H03 — ⌘Q quits during a Push or Pull without the "Sync in progress" warning

The app warns before closing mid-sync, and the warning itself says quitting "can leave this
project's synced state half-written, and it cannot be undone". The guard listens only for a window
close request, so it covers the red close button and ⌘W. ⌘Q and Mallard › Quit skip it.

Evidence: the only guard is a window `onCloseRequested` listener
([ProjectSync.tsx:1041](../../../src/components/project-sync/ProjectSync.tsx#L1041)). The app
registers no menu and no run-event handler
([lib.rs:183](../../../src-tauri/src/lib.rs#L183)), so Tauri 2.11.3 installs its default macOS menu
(`enable_macos_default_menu: true`, tauri `app.rs:1620`), whose Quit sends `terminate:`. The tao
0.35.3 app delegate implements only `applicationWillTerminate`, which ends the event loop directly
(`app_delegate.rs:131`, `app_state.rs:272`). No window close request and no preventable exit request
is raised, so `prevent_exit` alone cannot catch it. Source: SH-1; first recorded in the 2026-08-25
re-audit.

Repair: install an app menu whose Quit item is a custom item routed through the same guard, then
exits through the app handle. Keep ⌘Q as its shortcut.

Confidence: confirmed from source, including the platform crates. Not exercised in a native build.

#### H04 — Storage failures reach the screen as SDK and transport prose on most paths

Mallard has one translator for storage failures, and it only recognises messages that begin
"S3 get", "S3 body" or "S3 conditional put" and contain one of a few keywords
([storageError.ts:27](../../../src/storageError.ts#L27)). Everything else is shown verbatim:

- Linking a storage, or finishing project setup with one, the first thing a new user does, fails
  as "register the repo in '<storage>': S3 conditional put '…': dispatch failure…". The prefix stops
  the translator from matching
  ([service.rs:3034](../../../src-tauri/src/project_sync/service.rs#L3034)).
- The Push comparison error in H01 bypasses the translator entirely.
- R2 Test and Save with a bad key say "R2 authentication or required remote capability
  verification failed." for every cause (snapshot `crates/mallard-infrastructure/src/storage_providers/r2.rs:379`).
- Dropbox failures read like "The Dropbox request result is unknown. Refresh remote state before
  retrying." (snapshot `storage_core/dropbox_store.rs:72`).
- A storage written by a newer Mallard reads "Mallard storage layout version 2 is not supported;
  expected version 1". It sounds like damage and never says "update Mallard"
  ([marker.rs:42](../../../crates/mallard-infrastructure/src/storage_core/marker.rs#L42)).

Repair: translate by typed error kind at the command boundary: offline or timeout, rejected
credential, missing bucket or folder, missing permission, newer Mallard. Give each one sentence of
cause and one of next step, and keep the raw text in Logs. `dropbox_auth.rs` already shows the
target voice ("Dropbox could not be reached. Check the network and try again."). Do this together
with M22. Source: CP-2, CP-4, CP-5, CP-9 and pattern P1 in [findings-copy.md](findings-copy.md).

#### H05 — Turning Credential redaction back on marks nothing for re-push, and Settings never says what a change does

Changing Credential redaction, or either "Push … children" switch, changes how every synced
session is exported, so the next Push re-publishes all of them. Nothing says so. The Settings rows
describe the switch but not its consequence. On Activity, every affected session keeps its synced
check (its tooltip becomes "Export settings changed"), and the Push count leaves them out. A user
who pushed with redaction off and then turns it back on sees a clean project; the storage keeps
the unredacted copies until they happen to open Push.

Evidence: the row copy
([SettingsPage.tsx:638](../../../src/components/project-sync/SettingsPage.tsx#L638),
[SettingsPage.tsx:666](../../../src/components/project-sync/SettingsPage.tsx#L666)); a changed
export setting turns a current session into a Publish
([conversation_reconciliation.rs:170](../../../crates/mallard-domain/src/conversation_reconciliation.rs#L170),
[service.rs:6142](../../../src-tauri/src/project_sync/service.rs#L6142)); Activity keeps it
`synced` ([ProjectActivityPage.tsx:493](../../../src/components/project-sync/ProjectActivityPage.tsx#L493))
and counts only `local-outstanding` rows toward Push
([ProjectActivityPage.tsx:565](../../../src/components/project-sync/ProjectActivityPage.tsx#L565)).
Source: RD-1.

Repair: say it on the rows ("Changing this re-uploads every synced session on your next Push").
Give affected sessions their own status that the Push count includes, for example "Pushed with
your previous privacy settings — push to update".

Open question: revisions are immutable, so copies pushed with redaction off may stay readable in
the storage's history even after a redacted re-push. If so, the Off warning should say that too.

#### H06 — Non-Git projects: a file deleted on this Mac can't be removed from storage, though the UI says "remove it during Push"

This affects projects that are not Git repositories, the only ones whose ordinary files Mallard
syncs. A file deleted locally shows "Not in this folder. It remains in <storage>. Pull to download
it here, or remove it during Push." The Push review never lists it, the "Remove from storage"
control is never wired, and Pull drops the delete actions the planner produces. The deletion counts
and approval checkbox that were added at commit are therefore unreachable. The file stays in
storage and keeps being offered to other Macs.

Evidence: the sentence
([ProjectFilesActivityPage.tsx:74](../../../src/components/project-sync/ProjectFilesActivityPage.tsx#L74),
[ProjectFilesReviewPage.tsx:136](../../../src/components/project-sync/ProjectFilesReviewPage.tsx#L136));
Push lists only files present locally
([PushResourceWorkspace.tsx:89](../../../src/components/project-sync/PushResourceWorkspace.tsx#L89));
the control renders only when `onToggleRemoval` is passed, and no caller passes it
([ProjectFilesReviewPage.tsx:384](../../../src/components/project-sync/ProjectFilesReviewPage.tsx#L384));
removals are only ever reset to empty
([ProjectSync.tsx:1718](../../../src/components/project-sync/ProjectSync.tsx#L1718)); Pull filters out
file and directory deletions
([pullReviewModel.ts:180](../../../src/components/project-sync/pullReviewModel.ts#L180)). Source: SY-2.

Repair: wire the removal control on the Push files step and show Pull deletions as rows with the
existing approval. Until then, remove "or remove it during Push" from both sentences.

### Regressions and a wrongly closed finding

R01–R03 were reported by earlier audits, reworked since, and are still broken in the new form.
R05 was closed as fixed by the 09-01 audit and never was. They matter more than an unchanged
finding, because each was believed to be addressed. All five were verified live.

#### R01 — The Sessions/Files switch is still reachable only through the breadcrumb

2026-08-30 H04. For a non-Git project, the switch moved from the breadcrumb leaf into the section
crumb's "Switch view" menu
([ProjectLinksWorkspace.tsx:2281](../../../src/components/project-sync/ProjectLinksWorkspace.tsx#L2281)).
There is still no tab or segmented control, so the Files view stays hidden behind a crumb chevron.

#### R02 — The Catalog Info popover now closes when focus leaves it, so its actions can't be reached by Tab

2026-08-30 H02. Catalog Info gained `aria-controls` and a polite copy status, but it also gained an
outside-`focusin` close
([StorageCatalogPage.tsx:237](../../../src/components/storage-catalog/StorageCatalogPage.tsx#L237)).
Opening it still doesn't move focus into the portalled panel, so the first Tab closes it. File
details and the session debug popover have the same pattern.

#### R03 — Keyboard focus can no longer enter the app-actions menu

2026-08-30 M03. The ⋯ menu now reuses the path-bar menu, but with a no-op `registerRef`
([ProjectSync.tsx:2658](../../../src/components/project-sync/ProjectSync.tsx#L2658)) and items at
`tabIndex={-1}` ([SyncPathBar.tsx:201](../../../src/components/sync-review/SyncPathBar.tsx#L201)).
Nothing moves focus in, and Tab closes the menu, so a keyboard user can open it but can't reach an
item.

#### R04 — Latent: the base primary-button colour has fallen to 2.41:1 in dark

2026-09-01 R6. The base rule still sets `color: var(--text-0)`
([App.css:334](../../../src/App.css#L334)). Measured by computed style: a `.btn-primary` without the
doubled class reads **3.30:1** in light and **2.41:1** in dark, down from 3.10 before the dark
palette change. Every shipping button is masked by the doubled-class override (5.11 light, 6.38
dark), so nothing fails today. The one-line fix, `color: var(--button-primary-text)` on the base
rule, has been deferred twice while the fallback got worse.

#### R05 — Closed as fixed on 09-01, never fixed: the status-indicator focus ring measures 2.05–2.86:1, and the metric-chip ring fails in light

2026-08-30 M04. The sync-status indicator and the metric chips on Activity rows are focusable
spans that turn the outline off and draw a translucent accent ring instead
([App.css:5945](../../../src/App.css#L5945), [App.css:6116](../../../src/App.css#L6116)). The app's
opaque focus ring covers only buttons, inputs, selects, textareas and summaries, so it never
reaches them. Measured by computed style on the rendered Activity page:

| Ring | Light, row | Light, selected row | Dark, row | Dark, selected row |
|---|---:|---:|---:|---:|
| Status indicator | 2.11 | 2.05 | 2.86 | 2.79 |
| Metric chip | 2.72 | 2.61 | 3.91 | 3.67 |

A focus indicator needs 3:1, and only the metric chip in dark clears it. The 09-01 audit measured the global `--focus-ring` token instead of
these rings, which is how it came to close the finding. Repair: give these spans the global opaque
ring, or an opaque replacement of at least 3:1 in both themes, and pin the measurement in a test.

### Medium priority

Entries are grouped by the journey they sit in. Each keeps its own fix and owner.

#### First run and joining from a second Mac

**M01 — The first-run window never says what Mallard does, or that a storage is step two.** The
empty Projects landing offers "Pick a folder…" and one caption, "Sessions & memory travel with it"
([ProjectPicker.tsx:46](../../../src/components/project-sync/ProjectPicker.tsx#L46)). Nothing says
where they travel, which folder to pick, or that nothing leaves this Mac until a storage is added
and pushed. The folder dialog opens with no title. Repair: one purpose sentence and three marked
steps (pick the folder you run your agent in, add a storage, Push); a title on the folder dialog; an
"Add storage…" action beside "None yet" in setup. Source: SH-3.

**M02 — "Add storage" from a project creates the storage but never links it.** The "No storage
linked yet" card calls Add storage without the project
([ProjectLinksWorkspace.tsx:2731](../../../src/components/project-sync/ProjectLinksWorkspace.tsx#L2731)),
and Finish lands on the storage's own page
([ProjectLinksWorkspace.tsx:1382](../../../src/components/project-sync/ProjectLinksWorkspace.tsx#L1382)).
The project still says "No storage linked yet", four steps from Push. Repair: carry the project
through Add storage and link on Finish, or return to the project with the new storage preselected.
Source: SH-2. *Verified live.*

**M03 — On a second Mac, the repo chooser preselects nothing and its only live action splits the
histories.** Selection starts empty even when a row is marked "Recommended"
([BundleConnectionDialog.tsx:34](../../../src/components/project-sync/BundleConnectionDialog.tsx#L34)).
"Connect and review Pull" stays disabled until a pick, while "Create separate repo" is live from
the start ([BundleConnectionDialog.tsx:170](../../../src/components/project-sync/BundleConnectionDialog.tsx#L170)).
Rows are named by hex IDs and revision numbers. Repair: preselect the single recommended match, as
setup already does; describe rows by last push time and session count; relabel the separate path
"Keep separate — don't share sessions with <name>". Source: SH-4. *Verified live.*

**M04 — The R2 "Use existing" scan says "No Mallard storage in your Cloudflare account" beside a
bucket it lists as "Needs a newer Mallard".** The empty box shows whenever no bucket has a
current marker ([R2Setup.tsx:548](../../../src/components/project-sync/R2Setup.tsx#L548)), and its
primary button is "Create a new bucket instead"
([StorageDiscovery.tsx:254](../../../src/components/project-sync/StorageDiscovery.tsx#L254)). An older
Mallard is steered into splitting the history across two buckets. Repair: show the empty box only
when every candidate is missing; otherwise lead with "Update Mallard to use <bucket>" or "Check
access, then scan again". Source: ST-7. *Verified live.*

#### Storage setup and settings

**M05 — "Test connection" on an existing storage gives no result.** "Connection verified" renders
only in the new-storage footer; the existing-storage footer shows nothing on success (snapshot
`ProjectLinksWorkspace.tsx:1840-1894`). This is the moment after pasting a replacement key, when
the user most needs the confirmation. Source: ST-1.

**M06 — A passing test of unsaved replacement credentials clears the "needs reconnection" state.**
The test runs the same probe as a commit and forgets the recorded rejection on success
([storage_setup.rs:680](../../../crates/mallard-application/src/storage_setup.rs#L680),
[storage_setup.rs:1241](../../../crates/mallard-application/src/storage_setup.rs#L1241)), so the red
notice, the index "Reconnect" chip and the guide row all disappear. The revoked key is still the
saved one until "Save storage". Repair: forget recorded health only on commit, and after a pass say
"Connection verified — select Save storage to use this key". Source: ST-2. *Verified live.*

**M07 — Just opening Storage settings leaves a duplicate "Draft · Unsaved … settings — not applied
yet" entry.** Opening settings always persists an edit draft, which the index and sidebar list
beside the storage itself; the existing-storage card has no Discard (snapshot
`ProjectLinksWorkspace.tsx:1041`, `StorageIndexPage.tsx:125`). Repair: create the draft on the first
real edit, or discard it on close when unchanged. Source: ST-3.

**M08 — One click on the type switch or "Reset setup" throws away a finished R2 setup, including
the key Cloudflare shows only once.** Neither asks first
([R2Setup.tsx:699](../../../src/components/project-sync/R2Setup.tsx#L699),
[ProjectLinksWorkspace.tsx:1696](../../../src/components/project-sync/ProjectLinksWorkspace.tsx#L1696)),
while "Discard draft…" does. The discarded key stays valid in the user's Cloudflare account.
Repair: confirm before discarding a draft that holds a secret or a created bucket, naming what stays
in Cloudflare; lock the type switch once a lane step has run. Source: ST-5. *Verified live.*

**M09 — An existing R2 or local-folder storage can be re-pointed at another bucket or folder from
its settings, without warning.** Only Dropbox locks its destination and says why (snapshot
`storage_providers/local.rs:132`, `r2.rs:434`). Linked projects would then compare against different
data from other Macs. Repair: make the destination read-only on edit for every provider, as Dropbox
does. Source: ST-11; the effect on linked projects needs a runtime check.

**M10 — Dropbox is introduced as "An existing shared or team namespace", contradicting its own
Create-new option, and its storages are listed by a raw namespace ID** (snapshot
`storage_providers/dropbox.rs:66`, `:118`). Repair: describe each provider by outcome ("A Mallard
folder in your Dropbox — new, or one you already use") and show the folder name, keeping the ID as
secondary detail. Source: ST-8; Dropbox copy landed the day of the audit.

#### Destructive actions

**M11 — Reset, Remove and Unlink confirmations describe Mallard's internals instead of what the
user loses.** "Session bindings", "restored-session path mappings", "sync bases", "recipes" and
"derived records" name mechanisms, not effects
([ProjectSync.tsx:2555](../../../src/components/project-sync/ProjectSync.tsx#L2555)). Reset is also
the offered way out of a failed sync, so this dialog reaches users at a bad moment. Repair: say what
Mallard forgets on this Mac, what the next Push and Pull will do, and what stays untouched. Source:
SH-5.

**M12 — Reset Mallard never says it removes your storages, their saved keys, and your project
list.** The row and dialog list "sync metadata, activity logs, and the backups of files Pull
replaced" and add that "Cloud revisions and your project files are not touched"
([SettingsPage.tsx:432](../../../src/components/project-sync/SettingsPage.tsx#L432),
[SettingsPage.tsx:998](../../../src/components/project-sync/SettingsPage.tsx#L998)). The wiped folder
also holds every storage connection and its secrets (no Keychain is used), every project
registration and link, and the Catalog rows. After a reset the user must find or re-issue an R2 key
that Cloudflare shows once. Repair: list those losses in plain words in both places. Source: RD-4.
*Verified live.*

**M13 — The Clear storage data dialog uses a term the app uses nowhere else, and makes the user
certify something they cannot do.** It deletes "every synced project repository and Library
resource" (the app says Catalog everywhere else) and asks the user to confirm "that sync has
stopped on all machines" with the button "Sync stopped — clear". Mallard has no storage-wide stop,
and nothing runs in the background to stop (snapshot `ProjectSync.tsx:2908`). Repair: "Make sure no
one runs Push or Pull to this storage from another Mac until clearing finishes", name Catalog items,
and use a plain destructive label. Source: SH-6, ST-6.

**M14 — The trash icon means "Remove storage (files kept)" on the settings card but "Clear storage
data (permanent)" in the ⋯ menu** (snapshot `StorageSettings.tsx:537`,
`ProjectLinksWorkspace.tsx:1597`). Repair: reserve trash for the permanent action and give the card's
remove button a text label. Source: ST-4.

#### Push and Pull results

**M15 — A push limited by the size cap never says what was left out; it ends in a green "Push
complete".** The recommended selection silently skips sessions that no longer fit, and neither the
review nor the outcome says so. No "doesn't fit — push again" copy exists anywhere in the Push
review or its outcome, although that is the rule the capture-budget design settled on. Repair: say
it above the list ("The 38 older sessions (1.2 GB) don't fit in this push — push again after it")
and carry it into a warning-tone outcome with a "Push again" action. Source: SY-6. *Verified live.*

**M16 — The Push outcome counts only sessions, and can report every stored item as "sessions".**
When no per-session rows exist, the count falls back to `resources_changed`
([syncOutcome.ts:114](../../../src/components/project-sync/syncOutcome.ts#L114)), which is the size
of the whole published manifest
([service.rs:10256](../../../src-tauri/src/project_sync/service.rs#L10256)). A non-Git project that
pushes three files can read "Push complete · 40 sessions", and pushed files are never mentioned.
Repair: build the outcome from the reviewed selection by category. Source: SY-5. *Verified live.*

**M17 — After a partial failure, which rows failed and why is only in hover tooltips, and there is
no next step.** Row status and reason live in `data-tooltip`
([SyncOutcomeNotice.tsx:156](../../../src/components/project-sync/SyncOutcomeNotice.tsx#L156)), and a
Push outcome offers an action only for held rows. The in-flight line had promised the push "finishes
or fails whole". Repair: print status and reason on failed rows, add "Push again", and reword the
in-flight line. Source: SY-7. *Verified live.*

**M18 — A file with a scan warning is reported as "can't be captured — exclude it", when "Mark
reviewed" is the fix.** An unacknowledged warning becomes a local blocker whose footer and title
say to exclude it
([PushResourceWorkspace.tsx:416](../../../src/components/project-sync/PushResourceWorkspace.tsx#L416),
[PushResourceWorkspace.tsx:754](../../../src/components/project-sync/PushResourceWorkspace.tsx#L754)).
The actual remedy is the card's "Mark reviewed" checkbox
([ProjectFilesReviewPage.tsx:460](../../../src/components/project-sync/ProjectFilesReviewPage.tsx#L460)).
Following the instruction silently leaves the file behind. Source: SY-4. *Verified live.*

#### Reading surfaces

**M19 — A session Mallard can't read disappears from Activity without a word.** The backend returns
the reasons in `warnings` ("Skipped '…': …", "Cannot read '…'"), but no screen reads the field; they
reach only a Logs line (snapshot `project_activity.rs:410-466`, `commands.rs:564`). A truncated commit
rail is equally silent. Repair: a standing Activity notice, "2 sessions couldn't be read and aren't
listed. Show details". Source: RD-2.

**M20 — Log lines show a clock time only, across up to a year of retained history** (snapshot
`LogPanel.tsx:160`, `clock.ts:14`). Repair: day dividers, as the session timeline already has, or a
short date on every line not from today. Source: RD-3.

**M21 — A Catalog item marked "Gone from <storage>. Push to put it back." cannot be pushed.** The
badge counts it toward Push, but the push list admits only new and locally changed items
([model.ts:210](../../../src/components/storage-catalog/model.ts#L210),
[model.ts:228](../../../src/components/storage-catalog/model.ts#L228)), and the backend refuses it too.
The Push button shows "1", greyed out, with no reason. Source: RD-6. *Verified live.*

#### Systemic

**M22 — Error text doubles as control flow.** Whether the UI offers "Reset local sync state" after
a failed Pull is decided by substring-matching backend prose, "stale at semanticdigest" and
"changed lineage or semantic content"
([syncError.ts:9](../../../src/components/project-sync/syncError.ts#L9)). The customer reads
"conversation Pull action '<id>' is stale at SemanticDigest", an action ID plus a Rust enum name,
next to a reset whose reason they cannot see. Any rewording of that sentence silently removes the
button. The storage translator in H04 has the same weakness. Repair: carry a stable code beside every
user-visible error, key behaviour on the code, and only then rewrite the copy. R2 setup already works
this way. Source: SY-8, CP-3, pattern P2.

**M23 — One object, many names.** A single row is a session, a conversation, a thread and a task
depending on the string ("Publish new task", "Update this task", "This thread can't be synced
safely right now.", "Pull this conversation before pushing it."). Subagents are "children" in
Settings, "Agents" on the session page and "Child rollouts" in Push. The Catalog says resource, row,
Catalog row and, in one dialog, Library. Settings offers "Thread size limit" beside "session
transcript". Backend strings add "provider profile", "home" and "materialized". Repair: adopt one
glossary (session, agent session, storage, project, project files, agent profile, Catalog item,
Push, Pull, this Mac) and apply it in one pass across the frontend and user-facing Rust strings.
Source: SY-9, RD-5, SH-8, CP-14, pattern P5, and 2026-08-30 M07, which has grown since.

#### Backend copy

Each of these is a single Rust string shown verbatim. The rewrites are in
[findings-copy.md](findings-copy.md).

| ID | Where the user sees it | Shown today | Problem |
|---|---|---|---|
| CP-6 | Project files review | "File exceeds the 16777216 byte project-content limit" | Raw bytes; the session version already says "100 MiB" |
| CP-7 | Disabled Open buttons | "This session is not materialized locally." | Internal word; doesn't say Pull |
| CP-8 | Pull outcome on a second Mac | "Codex home is not mapped" | "Home" and "mapped" aren't the app's nouns; no place named |
| CP-10 | Rename, settings saves | "project changed (expected revision 3, current 4)" | Revision numbers; doesn't say the edit wasn't saved |
| CP-11 | Push race | "repo head CAS conflict; compare again before pushing" | Protocol term |
| CP-12 | Storage test after an interruption | "… cleanup needs manual cleanup. for setup 'setup-…'." | Setup ID, internal words, broken sentence |
| CP-13 | Storage settings, local-state failure | "…run the same command again." | CLI instruction shown in the desktop app |

### Low priority

| ID | Finding | Evidence (snapshot) |
|---|---|---|
| SH-7 | "Host identity unavailable" is internal language on a screen that replaces the whole app, and Retry is its only exit | `ProjectSync.tsx:3236` |
| SH-8 | The Catalog is never explained, and its empty landing offers "Pets v1" and "Pets v2" as separate doors | `CatalogIndexPage.tsx:66`, `storage_catalog/registry.rs:286` |
| SH-9 | One failed side read titles the shell "Project-sync backend unavailable." with internal subsystem names | `ProjectSync.tsx:3263` |
| SH-10 | "Agent profile…" opens a dialog titled "Project setup for …" under "Machine-local binding", with raw IDs | `ProjectBindingEditor.tsx:263` |
| SY-10 | Revision numbers and action IDs appear in banners and outcome stats | `RestorePlanView.tsx:420`, `syncOutcome.ts:162` |
| SY-11 | The missing-history banner's Unlink shows the generic confirmation, not the relink text written for it | `ProjectSync.tsx:2705` |
| SY-12 | A failed readiness check shows a red, assertive "ready to apply" error in a review that can still be applied | `ProjectSync.tsx:2216` |
| RD-7 | The "N agents" chip does nothing and agent rows start hidden; a subagent's run mode prints as a raw enum | `SessionDetailHeader.tsx:93`, `:109` |
| RD-8 | Catalog Pull copy says "did not advance the comparison base" and names a "Skip" that isn't on screen | `StorageCatalogPage.tsx:1153`, `:1721` |
| RD-9 | The same moment is printed three ways, sometimes in one column; "Not reported" vs "Tokens unavailable"; MB vs MiB | `clock.ts:31`, `:55` |
| ST-9 | The reconnect lane says "Save changes", but the button is "Save storage" | `R2Setup.tsx:693` |
| ST-10 | Remove storage and Discard storage draft confirm with a generic "Ok" | `ProjectSync.tsx:2952` |
| ST-12 | Nothing tells a second Mac to pick the same shared or synced folder for a Local folder storage | `storage_providers/local.rs:121` |
| CP-15 | "Another Mallard host operation is already in progress." | [findings-copy.md](findings-copy.md) |
| RT-1 | The empty sidebar rail shows wordless doors, and Cloudflare R2 and Dropbox wear the same cloud glyph, so they can't be told apart without hovering | Screenshot 04; `ProjectSidebar.tsx:42` |
| RT-2 | Opening a Pull review posts a green "Done ·" receipt whose sentence says nothing was applied | Screenshot 16 |
| RT-3 | Double-clicking the sidebar edge resets it to 298 px without checking the window; only a window resize clamps the width. In an 800 px window the workspace then drops to about 478 px, below the app's own 580 px floor, and the path-bar controls overlap until the window is resized: the branch button runs 30 px under Pull | `ProjectSync.tsx:3442` (the load path at `:166` has the same gap, but the window always opens at 1100 px); screenshots 14 and 19 |

## Status of earlier findings

Full table with evidence: [findings-prior.md](findings-prior.md).

| Status | Count | Findings |
|---|---:|---|
| Fixed | 18 | 08-30 H01 (closed Logs tab order), H03 (muted text contrast), H05 (duplicate navigation), H06 (deletion counts at commit, but see H06 above), H07 (Push and Pull labels now survive narrow widths, but see RT-3), H08 (missing-profile recovery), M09 (Finish now runs the same connection probe as Test), M15 and 09-01 L10 (fresh-install pane), L02 (Settings receipts); 09-01 R1, R2 (row contrast), R3, R4 (startup failure), H01 (nested frames), M01 (switcher search keyboard), L06 (switcher height), L09 (read-link contrast) |
| Gone | 3 | 09-01 L03, L11 (agent-setup split menu removed), L07 (capture-budget banner removed) |
| Regressed after rework | 3 | 08-30 H04, H02, M03, reported above as R01–R03 |
| Closed in error | 1 | 08-30 M04, reported above as R05 |
| Still open | 21 | Logs resizer mouse-only (08-30 M01 = 09-01 L02, L08); "log" vs "Logs" (08-30 L01); latent button fallback (09-01 R6, now R04); 'Add storage…' as a bare "+" with one storage (09-01 L05); session detail icon-only chrome (08-30 M11); custom radio groups (08-30 M02, one new instance in the R2 jurisdiction control); menus dropping focus on Escape (09-01 L04); Escape discarding a prepared Push from the Logs drawer (09-01 L01); page orientation, partly fixed (08-30 M05); terminology (08-30 M07, now M23); missing local Retry (08-30 M08); duplicate commit actions (08-30 M10); 9–10 px type (08-30 M12); zoom reflow (08-30 M13); loading not announced (08-30 M14); segmented controls (08-30 L03); "{n} storage" in a near-dead branch (08-30 L04); small pointer targets (08-30 L05); storage field validation (08-30 M06: only the new R2 bucket-name field sets `aria-invalid`; the descriptor-driven fields name nothing when Test and Finish stay disabled) |

## Recommended repair order

1. **Error codes, then error copy** (H01, H04, M22, the backend copy table). One change at the
   command boundary makes every later rewording safe, and it is what makes H01's "reset only for
   stale metadata" possible.
2. **Make the screen tell the truth about sync state** (H02, H05, H06, M15, M16, M17). Each is a
   small change to a verb, a status or a sentence.
3. **Guard Quit** (H03).
4. **Destructive dialogs** (M11–M14). Copy-only.
5. **Storage settings loop** (M05–M10), before the Dropbox lane settles on the same patterns.
6. **First run and second Mac** (M01–M04). These decide whether a new user's first two sessions
   end up in one history.
7. **Terminology pass** (M23), then the Low items.
8. **Regressions** (R01–R03) and the one-line **R04** fix, each with a behaviour test rather than a
   source-text assertion, so the next rework cannot silently undo them.

## Verification completed

| Check | Result |
|---|---|
| `npm run build`, live tree, end of audit | Pass |
| `npm run test:frontend-integration`, live tree, end of audit | Pass: 338 of 338 |
| Six High findings re-verified in the live tree | Confirmed; H02 also reproduced in the rendered app |
| Rendered capture in both themes at 1100 × 720 and 800 × 560 | First run, the three landings, Activity, Settings, Logs, Pull and Push reviews |
| Computed-style measurement | R04 button fallback, R05 focus rings, RT-3 path-bar geometry |
| Native Tauri shell, VoiceOver, real storage providers | Not run |

A green suite is not evidence against any finding here: none of these defects is covered by an
assertion. The mocked-backend harness used for the captures was removed afterwards, so the tree is
left as it was found apart from this directory.

## Runtime evidence

Every capture is the production React tree and stylesheet in headless Chrome, with a mocked
backend answering each command the app called. Images are at 2× in [screenshots/](screenshots/).

**First run.** The empty Projects landing's only explanation is its caption, and the sidebar's door
is a wordless folder and plus (M01). The Catalog offers "Pets v1" and "Pets v2" as separate doors
under "Shared through your storages" (SH-8). On Storages, R2 and Dropbox wear the same cloud glyph
in the sidebar (RT-1).

![First run, Projects](screenshots/02-first-run-projects-light.png)

![First run, Catalog](screenshots/03-first-run-catalog-light.png)

![First run, Storages](screenshots/04-first-run-storages-light.png)

**Populated screens.** On Activity, session rows print "Sep 11, 2026 at 17:20" under commit headers
that print "Sep 11, 2026, 17:30" (RD-9). The Storages index identifies the Dropbox storage by its
namespace ID (M10). Settings describes the redaction and children switches without their re-upload
cost (H05, M23). Every Logs line shows a time only (M20), under the title "LOG".

![Activity, light](screenshots/07-activity-light.png)

![Activity, dark](screenshots/13-activity-dark.png)

![Storages index](screenshots/10-storages-index-light.png)

![Settings](screenshots/11-settings-light.png)

![Logs drawer](screenshots/12-logs-drawer-light.png)

**Pull and Push.** The Pull review opens at 0/3 with a footer "Pull" and no count, and pressing it
reports "Pull complete · 0 restored" (H02). The Push review's warning reads "2 conversations held
for Pull … Selected threads still push.", two nouns for one object in one sentence (M23).

![Pull review](screenshots/16-pull-review-light.png)

![After an empty Pull](screenshots/17-pull-outcome-light.png)

![Push review](screenshots/18-push-review-light.png)

**Minimum window.** With the 298 px default sidebar in an 800 × 560 window, the state a
double-click on the sidebar edge produces, the path bar is 478 px wide and its controls overlap: the branch button runs 30 px under Pull, and Refresh overlaps both
the branch button and the section name. Session dates truncate at this width too. With the sidebar
at its 220 px minimum, the bar is 556 px and nothing overlaps (RT-3).

![Activity at 800 × 560, default sidebar](screenshots/14-activity-800x560-light.png)

![Activity at 800 × 560, dark](screenshots/15-activity-800x560-dark.png)

![Path bar with the narrowest sidebar](screenshots/19-pathbar-800x560-narrowest-sidebar.png)

**Startup failure.** The message is now the mapped sentence inside a `role="alert"` region, not a
raw JavaScript error. The internal title remains (SH-7).

![Host failure, light](screenshots/01-host-failure-light.png)

![Host failure, dark, 800 × 560](screenshots/01-host-failure-dark-800x560.png)

**Probes and measurements.**

- No captured screen has an unnamed control, and no text overflows its own box at 1100 × 720.
- Activity and Settings have no page-level heading (08-30 M05).
- The status-indicator focus ring measures 2.05–2.86:1, and the metric-chip ring fails in light (R05).
- A `.btn-primary` without the doubled class reads 3.30:1 in light and 2.41:1 in dark; every
  shipping primary button reads 5.11:1 and 6.38:1 (R04).

Still needed from a native run: ⌘Q during a Push (H03), an offline Push (H01), a redaction flip on
a synced project (H05), VoiceOver on the outcome banner (M17), and session detail, storage pages and
the Files view at 800 × 560.

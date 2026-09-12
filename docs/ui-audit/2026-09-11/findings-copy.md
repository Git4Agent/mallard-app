# Copy audit 2026-09-11 — Rust-authored strings the UI shows verbatim

Snapshot: `snapshot/` (commit 0c6813e + uncommitted work, taken 2026-09-11 17:15 PDT). Paths are relative to the snapshot.

## Worst strings

### CP-1 · Unreachable storage shows SDK prose under a "Reset local sync state" button
- **Severity:** High
- **Where the user sees it:** Push review for any project whose storage cannot be opened (offline, R2 endpoint down, bad key). Danger banner titled "Conversations couldn't be compared, so nothing can be selected", body = the Rust string, action = "Reset local sync state" (`src/components/project-sync/PushResourceWorkspace.tsx:730-740`, fed by `ProjectSync.tsx:1815`).
- **Evidence:** `crates/mallard-infrastructure/src/storage_core/s3_store.rs:500` — "S3 get '{}': {}" (also `:493` "S3 get '{}' timed out after {} ms", `:821` "S3 list '{}': {}"), carried verbatim by `src-tauri/src/project_sync/service.rs:7766` (`thread_comparison_error = Some(error.clone())`) and `:7698-7700` (`PushDestinationStatus::Unavailable { message: error.clone() }`; the comment at `:7680-7681` says "the thread comparison carries it verbatim"). The frontend's own comment quotes the result: "S3 conditional put 'bundles/…': dispatch failure: io error" (`src/storageError.ts:17`). This path never goes through `storageAccessErrorMessage`.
- **Why it fails:** A network outage is presented as a data failure: the user reads an object key plus "dispatch failure" and is offered a destructive local reset as the only action, when the real fix is to reconnect and refresh.
- **Rewrite:** "Mallard couldn't reach “{storage}”, so sessions can't be compared yet. Check your connection and the storage's settings, then refresh the review. Nothing was changed." (Don't offer Reset for transport failures.)

### CP-2 · Linking a storage / finishing setup wraps SDK prose in "register the repo in …"
- **Severity:** High
- **Where the user sees it:** Link storage on a project (`ProjectSync.tsx:1700`, `setError(errorMessage(reason))`) and Finish on project setup (`ProjectSetupWorkspace.tsx:500`), which both publish the registration claim before the first Push.
- **Evidence:** `src-tauri/src/project_sync/service.rs:3123` — "register the repo in '{}': {}" (called from `:13933` link and `:15506` setup finish), whose `{}` is the raw S3 text from `s3_store.rs:500`/`:690` ("S3 conditional put '{}': {}").
- **Why it fails:** The first thing a new customer does fails with an object key and AWS SDK wording under a lowercase code-style prefix. The prefix also stops `storageAccessErrorMessage` (anchored `^S3`, `src/storageError.ts:27`) from ever translating it.
- **Rewrite:** "Mallard couldn't write to “{storage}”, so the project wasn't linked. Check your connection and the storage's credentials in Storage settings, then try again."

### CP-3 · Pull fails with an action ID and a Rust enum name, then offers a reset
- **Severity:** High
- **Where the user sees it:** Pull apply failure notice on the storage card (`ProjectSync.tsx:2282`, `:2409` via `errorMessage`), which shows "Reset local sync state" because `syncError.ts:9-11` matches the text (`ProjectLinksWorkspace.tsx:2903`, `RestorePlanView.tsx:373`).
- **Evidence:** `src-tauri/src/project_sync/service.rs:10944` — "conversation Pull action '{}' is stale at {:?}" (renders e.g. "…action 'act-…' is stale at SemanticDigest"); sibling `crates/mallard-infrastructure/src/pull_transcript.rs:72` — "cloud conversation changed lineage or semantic content".
- **Why it fails:** Internal ID plus `Debug` output ("SemanticDigest", "lineage") tells the user nothing about what happened or why a reset is the fix. The sentence is also load-bearing: `isIrrecoverableSyncError` string-matches it, so a copy edit silently removes the Reset button. Switch the frontend to an error code before rewording.
- **Rewrite:** "A session in this Pull no longer matches what Mallard recorded on this Mac, so it can't be applied. Reset local sync state for this project, then review Pull again."

### CP-4 · R2 Test/Save with bad keys: one jargon sentence for every cause
- **Severity:** High
- **Where the user sees it:** Storage settings → R2 connection → Test or Save (`ProjectSync.tsx:2835` and `:2871`, both `setError(errorMessage(reason))`).
- **Evidence:** `crates/mallard-infrastructure/src/storage_providers/r2.rs:379-381` — "R2 authentication or required remote capability verification failed." Path: `StorageSetupError::from_adapter` (`crates/mallard-application/src/storage_setup.rs:121-133`) → `setup_error` = `error.to_string()` (`src-tauri/src/project_sync/service.rs:1078-1079`).
- **Why it fails:** This is the most common manual R2 failure (wrong secret, read-only token, wrong bucket), and the text doesn't separate those cases or say what to check. The probe records the failing stage (`s3_store.rs:63-67`: "bounded listing", "absent-key creation", "conditional overwrite", …) and the code drops it.
- **Rewrite:** "Cloudflare rejected these R2 credentials, or they can't write to “{bucket}”. Check the Access Key ID and Secret Access Key, and make sure the API token has Object Read & Write access to this bucket. Then test again."

### CP-5 · Version skew between Macs reads as corruption, with no "update Mallard" step
- **Severity:** High
- **Where the user sees it:** Any storage open after another Mac running a newer Mallard wrote to the storage: the Push review banner from CP-1, the Pull failure notice (`ProjectSync.tsx:2409`), and Link storage (`:1700`).
- **Evidence:** `crates/mallard-infrastructure/src/storage_core/marker.rs:42` — "Mallard storage layout version {} is not supported; expected version {}" and `:36` "storage marker format '{}' is not supported; expected '{}'", checked on every shared-store open and returned as-is (`storage_core/mod.rs:360-371`). `crates/mallard-domain/src/lib.rs:3988` — "unsupported bundle head schema {}" (via `read_head` → `head.validate()`, `bundle_engine.rs:547`), which the user reads as "unsupported repo head schema 2" after `terminology.ts`. Siblings: `lib.rs:4816` "unsupported bundle manifest schema {}", `:1266` "unsupported project-sync schema {} (expected {})".
- **Why it fails:** Version numbers and "not supported" suggest damaged data. The text never names the fix (update Mallard) or says the stored data is intact. In the Push review it also sits next to the Reset button, which won't help. Current builds write layout version 1 (the expected marker in `s3_store.rs:1591-1592`), so customers first see this copy at the next format change. Fix it before that ships.
- **Rewrite:** "“{storage}” was saved by a different version of Mallard. Update Mallard on this Mac and on the other Macs that use this storage, then try again. Nothing was changed."

### CP-6 · Project-file rejections: raw byte counts, internal nouns, no end punctuation
- **Severity:** Medium
- **Where the user sees it:** Push review → Project files, and the project files activity page. The UI renders "Unavailable for sync. {reason}" (`ProjectFilesReviewPage.tsx:94`, `ProjectFilesActivityPage.tsx:55`, details menu `ProjectFileDetailsMenu.tsx:55`).
- **Evidence:** `crates/mallard-infrastructure/src/provider_capture.rs:1398` — "File exceeds the {} byte project-content limit" (`MAX_FILE_BYTES` = 16 MiB at `:27`, so the user reads "File exceeds the 16777216 byte project-content limit"). Also `:1357` "Symlinks are not portable project content", `:1388` "Hard-linked files are not portable project content", `:1369` "Sockets, devices, FIFOs, and other special files are not portable", `:1377` "Known credential and private-key paths cannot be synced", `:1485` "Private-key material cannot be synced".
- **Why it fails:** The session version of the same limit already says "Session file exceeds the {} MiB per-file sync limit." (`src-tauri/src/project_sync/service.rs:3240-3243`). The file version prints eight digits of bytes and calls files "project content". No reason ends with a period, so the composed sentence doesn't either.
- **Rewrite:** "Larger than the 16 MiB limit for project files." / "Symbolic links can't be synced." / "Hard-linked files can't be synced. Replace it with a regular copy to include it."

### CP-7 · "Not materialized" on the session Open buttons
- **Severity:** Medium
- **Where the user sees it:** Session detail header and activity rows. When the session exists only in storage, Open in Claude / Open in Codex / Open in Terminal are disabled with this reason as the tooltip (`SessionDetailHeader.tsx:122`, `ProjectActivityPage.tsx:1170`).
- **Evidence:** `crates/mallard-infrastructure/src/providers/claude/actions.rs:162` and `:170`, `crates/mallard-infrastructure/src/providers/codex/actions.rs:290` — "This session is not materialized locally."; session-detail error `src-tauri/src/project_sync/service.rs:1545` — "This session is not materialized in a readable local provider profile."
- **Why it fails:** "Materialized" and "local provider profile" are internal words for a simple state: the session is only in storage. The fix, Pull, isn't named.
- **Rewrite:** "This session is only in storage. Pull it to open it on this Mac."

### CP-8 · Missing agent profile after Pull is described as an unmapped "home"
- **Severity:** Medium
- **Where the user sees it:** Pull outcome rows on the storage card. `syncOutcome.ts:262-266` renders `issue.title` and `issue.detail` as a failed row, which is what happens on a second Mac that has no profile set for the provider.
- **Evidence:** `src-tauri/src/project_sync/service.rs:13530` — "{} home is not mapped"; `:13532` — "Choose an existing {} home before restoring provider state." Sibling `:13314` — "Review the technical result, fix the reported problem, and try Pull again." (under "{count} project data changes were left unchanged", `:13308-13311`).
- **Why it fails:** "Home", "mapped" and "provider state" aren't the app's nouns (it says agent profile, sessions, project files), and the row doesn't say where to make the choice. The sibling asks the user to fix "the reported problem" without naming it.
- **Rewrite:** Title "Codex isn't set up for this project on this Mac"; detail "Choose a Codex agent profile in Project settings, then Pull again to restore its sessions." For `:13314`: "Mallard couldn't apply these project file changes. Open the details to see which files failed and why, then Pull again."

### CP-9 · Dropbox Push/Pull failures describe transport internals
- **Severity:** Medium
- **Where the user sees it:** Push review error and Pull failure notice on a Dropbox storage. `storageFailureMessage` only rewrites S3-shaped text, so these pass through unchanged (`src/storageError.ts:27-29`, `ProjectSync.tsx:2006`).
- **Evidence:** `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs:70` — "The Dropbox object changed. Refresh the review and try again."; `:72` "The Dropbox request result is unknown. Refresh remote state before retrying."; `:69` "The Dropbox object is missing."; `:73` "Dropbox returned an invalid response or a noncanonical storage path."; `:77` "The Dropbox staging file or setup journal could not be saved."; `:75` "Dropbox response or listing exceeds its safety limit." All become the IPC error through `impl From<Error> for String` (`:82-85`).
- **Why it fails:** Words like "object", "noncanonical storage path", "setup journal", "staging file" and "remote state" describe the implementation. "Missing" doesn't say which item or what to do, and "Refresh remote state" isn't a control in the app.
- **Rewrite:** (`:72`) "Mallard lost contact with Dropbox before it could confirm the last change. Refresh the review to see what's in “{storage}”, then try again." (`:70`) "Something in “{storage}” changed while Mallard was working, so nothing was overwritten. Refresh the review and try again."

### CP-10 · Revision numbers in "changed since you opened it" errors
- **Severity:** Medium
- **Where the user sees it:** Rename project (`ProjectSync.tsx:2155` → rename field error, `ProjectLinksWorkspace.tsx:1300`), Project settings save, project setup, and storage settings save, whenever the saved state changed after the form loaded (a second window, the CLI, or a background refresh).
- **Evidence:** `crates/mallard-infrastructure/src/persistence.rs:1188` — "project changed (expected revision {}, current {})" (also `src-tauri/src/project_sync/service.rs:14446`); `persistence.rs:929` "project-sync config changed (expected revision {}, current {})"; `persistence.rs:1301` "storage configuration changed (expected revision {}, current {})"; `service.rs:14163` "project binding changed (expected revision {}, current {})"; `service.rs:15488` "setup draft changed (expected revision {}, current {})".
- **Why it fails:** A lowercase fragment with two integers and internal nouns ("project-sync config", "binding", "draft"). It doesn't say whether the edit was saved or what to do next.
- **Rewrite:** "This project changed after you opened it, so your edit wasn't saved. Check the current settings and try again."

### CP-11 · Push race: "repo head CAS conflict; compare again before pushing"
- **Severity:** Medium
- **Where the user sees it:** Push review error box (`ProjectSync.tsx:2053`, `setPushError(message)`), when the recheck after a failed Push (`:2007-2050`) doesn't classify the destination as moved, or the recheck itself fails. Pull failure notice for the Pull sibling.
- **Evidence:** `crates/mallard-infrastructure/src/bundle_engine.rs:1098` — "bundle head CAS conflict; compare again before pushing" (displayed as "repo head CAS conflict…" after `terminology.ts`); `:801` "sync scope '{}' changed since review; pull and compare again before pushing" (the scope key is an internal ID); `:739` "bundle head changed: expected an absent head"; `:742` "bundle head is missing; the remote repository was removed"; Pull: `src-tauri/src/project_sync/service.rs:12252` "bundle head changed after comparison; compare again".
- **Why it fails:** "CAS", "head" and "sync scope" are protocol words, and "compare again" isn't a control in the app (it says Review, Refresh, Pull). The text also omits the reassuring fact the code guarantees: nothing was written.
- **Rewrite:** "“{storage}” changed while you were reviewing, so nothing was pushed. Refresh the review, then push again."

### CP-12 · Interrupted storage test: raw setup ID, internal words, and a sentence broken in two
- **Severity:** Medium
- **Where the user sees it:** Storage settings → Test or Save after an earlier test was interrupted, or when probe cleanup fails (same render sites as CP-4).
- **Evidence:** `crates/mallard-application/src/storage_setup.rs:148-152` — Display "{} for setup '{}'." appended to `crates/mallard-infrastructure/src/storage_providers/r2.rs:385` "R2 remote cleanup is required", `storage_setup.rs:1337` "Remote cleanup of an earlier interrupted storage probe is required", and `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs:78` "Dropbox probe cleanup needs attention. Restore access and test again; an unknown revision needs manual cleanup." That last one already ends with a period, so the user reads "…needs manual cleanup. for setup 'setup-…'." (`dropbox_store.rs:424-425` feeds it into `cleanup_required`).
- **Why it fails:** "Probe", "remote cleanup", "unknown revision" and an opaque setup ID say neither what was left behind nor how to clear it. The Dropbox variant produces a broken sentence.
- **Rewrite:** "An earlier connection test didn't finish and left a test file in “{storage}”. Test the connection again. If it keeps failing, remove the storage connection and set it up again."

### CP-13 · CLI instructions shown in the desktop app
- **Severity:** Medium
- **Where the user sees it:** Storage settings → Test / Save / edit, when Mallard can't read or write its local state (same render sites as CP-4).
- **Evidence:** `crates/mallard-application/src/storage_setup.rs:104-108` — "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again." Siblings with the same CLI/agent voice: `:1099` "…copy fresh agent setup instructions from Mallard.", `:1380` "…review the latest settings in Mallard.", `:1394` "Mallard storage configuration changed; reload it before saving storage."
- **Why it fails:** The module serves the CLI and the app. The app shows the CLI sentence ("run the same command again", "from Mallard"), which gives a GUI user no action they can take.
- **Rewrite:** "Mallard couldn't read or save its local settings. Check that your home folder has free space and is writable, then try again."

### CP-14 · Setup conflicts say "config" and "provider profile" where the app says agent profile
- **Severity:** Low
- **Where the user sees it:** Project setup → Finish (`ProjectSetupWorkspace.tsx:500`) and Project settings save.
- **Evidence:** `src-tauri/src/project_sync/service.rs:15059` — "'{}' already syncs this folder with that {} config; open it instead or pick a different config"; `:14389` — "'{}' already uses this {} config for this folder; pick a different config"; `:2114`, `:2181`, `:14769` — "choose at least one local provider profile".
- **Why it fails:** The components say "agent profile" and never call it a "config". These sentences also start lowercase and have no final period. The next step itself is good; keep it.
- **Rewrite:** "“{project}” already syncs this folder with this Codex agent profile. Open that project, or choose a different agent profile." / "Choose at least one agent profile."

### CP-15 · "Another Mallard host operation is already in progress."
- **Severity:** Low
- **Where the user sees it:** Storage settings → Test / Save while a Push, Pull or another window holds Mallard's lock (same render sites as CP-4).
- **Evidence:** `crates/mallard-application/src/storage_setup.rs:1043-1044` — "Another Mallard host operation is already in progress."; siblings `:1115` "Too many storage drafts are open; discard one first.", `:568` "The storage provider declares no secret with that identifier.", `:436` "This build has no storage provider with that key."
- **Why it fails:** "Host operation", "drafts", "secret identifier" and "provider key" are architecture words. The busy case also doesn't say to wait and retry, or what it's waiting on.
- **Rewrite:** "Mallard is finishing another Push, Pull or storage change. Wait for it to finish, then try again."

## Systemic patterns

### P1 · Transport and SDK text crosses the IPC boundary
- **Example:** `src-tauri/src/project_sync/service.rs:3123` — "register the repo in '{}': {}" wraps `crates/mallard-infrastructure/src/storage_core/s3_store.rs:690` "S3 conditional put '{}': {}". The only safety net is a frontend regex anchored at `^S3` (`src/storageError.ts:27-28`). It misses `S3 list` (`s3_store.rs:821`), every prefixed wrapper, and every surface that calls `errorMessage` instead (CP-1, CP-2). The same thing happens in `storage_core/mod.rs:201`, which appends a raw `{error}` to an otherwise careful storage-clear sentence.
- **House rule:** Storage transports return a typed error (kind, fixed code, one product sentence), the way `StorageAdapterError` already does for setup (`storage_providers/traits.rs:195-204`). No `format!("{context}: {error}")` over an SDK, io or serde error may reach IPC. The frontend maps codes, not regexes.

### P2 · Copy doubles as control flow
- **Example:** `src/components/project-sync/syncError.ts:9-11` decides whether to offer "Reset local sync state" by substring-matching `service.rs:10944` ("…is stale at {:?}") and `crates/mallard-infrastructure/src/pull_transcript.rs:72`. Rewording either sentence silently removes the recovery button.
- **House rule:** Behaviour keys off a stable code carried beside the message. R2 setup already works this way: `r2_setup.rs:124-158` maps codes and the frontend owns the copy. A string the UI or a test matches on is an identifier, not copy, and must not be shown.

### P3 · Protocol identifiers, revisions and raw units inside sentences
- **Example:** `src-tauri/src/storage_catalog/domain.rs:2496-2499` — "overlapping_targets: the active target roots overlap ({})": a machine-code prefix plus joined absolute paths, returned as the Catalog command error through `blocker.to_string()` (`storage_catalog/service.rs:649`, `:1026`, `:1593`). Same family: "path_map_codec_mismatch:" (`service.rs:8430`), "redaction_gate_failed:" (`service.rs:8675`), "wire_secret_detected:" (`credential_redaction.rs:1403`), "(expected revision {}, current {})" (CP-10), "for setup '{}'" (CP-12), `{:?}` (CP-3), "16777216 byte" (CP-6), raw IDs in "unknown storage '{}'" (`service.rs:13948`).
- **House rule:** User text never contains an internal ID, code, revision number, `Debug` output or byte count. Codes go in a separate field the UI can show under technical details. Sizes are in MiB. Paths the user chose appear in the UI's path chip, not inside the sentence.

### P4 · Log-style fragments rendered as sentences
- **Example:** `ProjectFilesReviewPage.tsx:94` builds "Unavailable for sync. ${blockedReason}" over `crates/mallard-infrastructure/src/provider_capture.rs:1357` "Symlinks are not portable project content" (no period). Similar: `pullReviewFlow.ts:95` "Readiness verification failed: ${readiness_error}". Measured outside test modules: in `service.rs`, 138 of 152 literal `Err` strings start lowercase and only 9 end with a period. In `bundle_engine.rs` it's 43/43 lowercase, 0 periods; `persistence.rs` 77/79 and 0; `mallard-domain/src/lib.rs` 251/253 and 0. These are log lines shown to users as messages.
- **House rule:** Any Rust string that can reach a screen is one or two complete sentences: capital first letter, final period, product nouns, a next step. The frontend doesn't put its own lead-in before backend text. It shows the whole backend sentence, or replaces it based on the code.

### P5 · Two vocabularies and two voices
- **Example:** `src-tauri/src/project_sync/service.rs:13259` — "1 conversation conflict still needs manual resolution" (a Pull outcome row via `syncOutcome.ts:262-266`), while the UI's own copy says sessions ("Pushed N of M sessions", `syncOutcome.ts:171`). The same drift: "provider profile"/"home"/"config" instead of agent profile (CP-8, CP-14), "project content" instead of Project files (CP-6), "materialized" (CP-7), and CLI voice ("run the same command again", CP-13). The `bundle`→`repo` bridge (`src/terminology.ts:10-13`) is a render-time regex that needs a delimiter on both sides, so compound forms get through untouched ("S3 bundle-store …", `s3_store.rs:189`; "bundle-relative object key", `storage_core/mod.rs:245`). It also only runs where the frontend calls it (`src/storageError.ts:13`).
- **House rule:** One glossary (session, storage, project, project files, agent profile, repo, Push, Pull, "this Mac"), applied where the string is written: user-facing Rust strings use these nouns, and strings shared with the CLI take the surface as a parameter. Remove `userFacingRepoTerms` once no user-facing Rust string says "bundle".

## Exemplary strings

- `crates/mallard-infrastructure/src/dropbox_auth.rs:163` — "Dropbox rejected the saved authorization. Reconnect Dropbox to restore access." Says what happened in product terms, names one action that matches a visible control, and uses no internal words (shown on the storage connection line, `StorageSettings.tsx:215`).
- `crates/mallard-infrastructure/src/dropbox_auth.rs:183` — "Dropbox could not be reached. Check the network and try again." A transport failure translated where it's produced. Use it as the model for CP-1 and CP-2.
- `crates/mallard-application/src/storage_setup.rs:861` — "This storage is still linked to a project. Unlink it before removing the storage." States the blocking fact and the exact next step in the app's nouns.
- `src-tauri/src/project_sync/service.rs:9839` — "Credential redaction changed after review. Review Push again." The model for every "changed after review" error (compare CP-11).
- `src-tauri/src/project_sync/service.rs:3241` — "Session file exceeds the {} MiB per-file sync limit." Human units and the app's nouns. The model for CP-6.

# A project from a Mallard Cloud link

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: implemented 2026-09-26, uncommitted. Specimens, kept as the record:
`mockups/cloud-project-mockup.html` (round 1, directions A/B/C),
`mockups/cloud-project-accounts-mockup.html` (round 2, several accounts),
`mockups/cloud-project-m1-mockup.html` (the chosen design, one frame per state).
Live harness of the shipped component on a stand-in backend:
`mockups/cloud-project-live.html` (`?accounts=one|several|none&git=yes|no&clone=works|fails`).

## Decisions (owner)

- **Where:** a second door on New project, **From Mallard Cloud**, beside
  **On this computer**. Finish setup of a new Mallard Cloud storage opens it
  with that account checked; Add project reaches it any time.
- **Account first (M1):** the Storage band leads and lists Mallard Cloud
  storages only. One account is checked for you; with several, none is
  checked until you pick. The address is looked up with the checked account
  only. When it can't see it, the empty state says to try another account.
  Switching accounts keeps the typed address and looks it up again.
- **Folder:** the Project band's shipped Folder field, one destination path.
  It defaults to the repository's name, beside most of your projects (home
  when there are none). **Browse…** picks the parent and keeps the name.
- **One verb, connect only:**
  - **Clone and connect**, **Create folder and connect**, or **Connect**
    (already its checkout, or an empty folder with nothing to clone).
  - It finishes like setup's Finish and lands on Activity.
  - Nothing is pulled.
- **Clone best effort:**
  - Order: gh, then git, then an empty folder, exactly as for a project
    without Git.
  - A repository off GitHub always gets an empty folder; there is no clone
    URL field.
  - After an empty-folder fallback, a warning under the path bar gives the
    reason and the clone command, with **Copy command**.
- **Few words** everywhere on this page.

## What runs

1. **Probe** (`probe_cloud_project_folder`, read-only). Uses
   `probe_project_folder`'s rules for the destination, but never runs Git:
   it reads a `.git` entry and `.git/config` instead. On a Mac without
   developer tools, the `/usr/bin/git` stub would offer to install them on
   every keystroke. The verdicts:
   - `new` when missing under a folder setup accepts (one leaf only);
   - `empty`;
   - `checkout` when it holds this repository's origin, by fingerprint;
   - `occupied`, suggesting the first free `name-N`;
   - `refused` for everything else: a file, Mallard's data, a missing
     parent, or an existing project.

   It also reports `can_clone`, which is true when git runs here. gh clones
   through git. On macOS, `/usr/bin/git` counts only when `xcode-select -p`
   succeeds. Tools are found once per app run, through `find_binary`.
2. **Folder** (`get_cloud_project_folder`, progress on a channel):
   - It probes again first, and refuses `occupied` and `refused`.
   - It clones only `ManagedCloudRepository.github_url`, and only in its
     canonical form and when it hashes to the repository fingerprint.
   - gh (`gh repo clone owner/name dest -- --progress`) runs first, then
     `git clone --progress -- https://github.com/owner/name.git dest`.
   - Clones run with prompts off: `GIT_TERMINAL_PROMPT=0`,
     `GCM_INTERACTIVE=never`, `GH_PROMPT_DISABLED=1`,
     `SSH_ASKPASS_REQUIRE=never`. The child leads its own session (`setsid`),
     so it has no terminal. gh gets the git that was found first on its PATH.
   - When nothing clones, it creates exactly one folder with `create_dir`.
   - `cancel_cloud_project_folder` sends SIGTERM to the clone's process
     group; git removes what it wrote. On Windows, `taskkill /T /F`.
3. **Connect**, from the page, all existing commands:
   - `create_setup_draft` on the folder; this resumes a draft already there.
   - `update_setup_draft` sets the storage, the agent profiles, and
     `DraftRepositoryChoice::Existing`.
   - `finalize_project_setup` finishes. A failure keeps the folder, and
     **Try again** finds it as `checkout` or `empty`.

## Safety

- Mallard deletes nothing. It creates one new folder under an existing
  parent, or lets git create it. A cancelled or failed git clone removes its
  own folder.
- It never asks for Git credentials: sign-in stays gh's and git's own.
- **Relaxed on purpose:** `mismatch_acknowledged` is set for an empty
  folder (`created`: made here, or found empty).
  - That folder is always one this flow made empty, or an empty one the
    person chose, for this repository.
  - Finish would otherwise refuse the Git repository's fingerprint against
    a folder that has none.
  - A cloned folder carries the matching remote, so it needs no
    acknowledgement.
  - Since `docs/PLAN_CLOUD_PROJECT_EXISTING_FOLDER.md` the rule is keyed on
    the folder being empty, not on it having no remote: a folder that holds
    files, connected as it is, needs setup's tick.

## Contract changes

- `ManagedCloudRepository.github_url` (Rust and `types.ts`).
- New `CloudProjectFolder*` and `CloneFailure*` types.
- Three commands, registered in `lib.rs`, with `CloudProjectControl` as
  managed state.
- `ModeDoor` is exported from `StorageSettings.tsx`.
- `RepositoryAddressPicker` gains a `mode="clone"`: no remote comparison,
  no acknowledgement, no New repository row, a Git chip naming what gets
  cloned, and short copy.

## Tests

- Rust, `project_sync::cloud_project` (9 tests):
  - destination states;
  - GitHub address binding;
  - progress, failure and command parsing;
  - a real clone from a local bare repository;
  - a failed clone leaving an empty folder;
  - no git, and no address;
  - an occupied or refused folder left untouched;
  - Cancel stopping a running clone and leaving no folder;
  - an early Cancel.
- Rust: `managed_cloud_link` asserts that `github_url` carries through.
- Frontend: `tests/frontend/cloud-project.integration.test.tsx` covers
  accounts, clone mode, Folder lines, default parent, and the door.
  `project-add-folder` and `project-sync-cutover` are updated.

## Not done / open

- Not run against live Mallard Cloud and a real GitHub clone in the
  packaged app.
- The website's Pull guide (`mallardInternal` `web/src/docs/pages.tsx`)
  still says "Clone it, then Add project".
- An "Open in Mallard" deep link from the website is a later layer.

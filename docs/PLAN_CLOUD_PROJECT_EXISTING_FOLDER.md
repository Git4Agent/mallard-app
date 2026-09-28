# An existing folder for a Mallard Cloud project

Status: implemented 2026-09-27 (B, pills, the tick kept), uncommitted.
Builds on `docs/PLAN_CLOUD_PROJECT_FROM_LINK.md` (implemented, uncommitted).
Specimen, kept as the record: `cloud-project-existing-mockup.html`
(`?show=all|today|a|b|c|<frame>&switch=pills|segmented&theme=dark`). The
shipped component runs on a stand-in backend in `cloud-project-live.html`
(folders `…/taken`, `…/fork`, `…/project`, `…/empty`, `…/checkout`).

## Problem

New project › From Mallard Cloud refuses a folder that holds files: "Holds
files; needs a new or empty folder." The only way on is `name-2`, a new folder
beside it. Someone who already has the project on this computer (a checkout,
or files they copied) must cancel, open On this computer, and pick the storage
and the repository again.

## Owner decisions (2026-09-27)

1. **Direction: B** now; C stays a later layer.
2. **Switch look: the Storage band's pills.** They are the band above's
   grammar and the form's 32 px height; the segmented pill is 26 px, with
   10 px type.
3. **Tick: kept.** Setup's Git-remote acknowledgement applies to a folder
   Mallard can't verify (see Safety).

## Directions

- **A · a link on the Folder line.** "Holds files." gains **Use it as it
  is** beside **Use `name-2`**. No new control. Browse still picks the
  parent, so a folder with another name (the screenshot's `demo` for
  `HorrorGame`) can only be typed.
- **B · New folder | Existing folder.** A two-way switch heads the Folder
  field. Browse follows it:
  - under New folder it picks the parent and keeps the name, as decided in
    round 3;
  - under Existing folder it picks the folder itself.
- **C · B, opened on a found checkout.** Agent history already lists the
  folders Codex and Claude ran in. A recent folder whose Git remote matches
  opens Existing folder on it. The rows below are On this computer's rows.

B is the smallest direction where Browse works for both jobs. C needs the
agent-folder list to carry each folder's remote, so it is a second layer.

## Behavior (B)

- **The switch** takes the Folder label's place. The label stays for screen
  readers. New folder is chosen first. Switching keeps the field's text.
- **New folder** is unchanged, except that "Holds files." now offers
  **Use it as it is**, which switches to Existing folder and keeps the text,
  beside **Use `name-N`**.
- **Existing folder** never clones, creates, or pulls. The verb is always
  **Connect**, and it finishes like setup's Finish, on Activity. The Name
  stays the repository's; the folder keeps its own name.

| Existing folder holds | Folder line | Tick |
| --- | --- | --- |
| this repository's checkout (fingerprint) | Already its checkout. Nothing to clone. | no |
| files; no Git on either side | Used as it is. | no |
| files, no Git remote; the repository has Git | No Git remote to check. | yes |
| another Git remote | Git remote: github.com/maya/HorrorGame. | yes |
| a Git checkout; the repository has no Git | A Git checkout; field-notes has no Git. | yes |
| nothing (an empty folder) | Empty. Nothing is cloned. | no, as today |
| nothing there | No folder here. | stops |
| a Mallard project already | Already a project here. **Open it** | stops |
| a file, Mallard's data, unreadable | as today | stops |

- **The tick** is the shipped `Acknowledgement` recipe under the field:
  "Can't verify this is HorrorGame. Connect it anyway." Connect waits for it.
- Existing folder ends where On this computer plus this repository would.
  The doors say where you start, not what you end with.

## What runs

1. **Probe** (`probe_cloud_project_folder`, read-only, same rules, still
   never runs Git):
   - `occupied` also carries two things:
     - `local_fingerprint`, the origin's fingerprint through
       `repository_fingerprint`;
     - `git_remote`, the origin for the line, as host and path only
       (`github.com/owner/name`, `gitlab.com/acme/portal`): no scheme, user,
       query, fragment or `.git`. A token can sit in any of those, and the
       short form reads better.
   - The page decides the tick with the shipped
     `repositoryFingerprintsRequireAcknowledgement`: the same rule as
     setup's Repository band and `finalize_project_setup`.
   - "Already a project here." becomes its own verdict,
     `project { project_id }`, so both modes can offer **Open it**.
     `CloudProjectSource` gets `onOpenProject` from `NewProjectFolderStep`.
2. **Connect** (Existing folder):
   - Probe again. If the verdict changed since the line was drawn, stop and
     show the new line.
   - Then setup's calls, as today:
     - `create_setup_draft` with the typed path. The typed spelling is kept,
       as On this computer keeps it, because providers record `cwd` in that
       spelling. It resumes a draft already on the folder.
     - `update_setup_draft` with the storage, the agents,
       `DraftRepositoryChoice::Existing`, and `mismatch_acknowledged` set
       from the tick.
     - `finalize_project_setup`.
   - `get_cloud_project_folder` isn't called. It keeps refusing `occupied`,
     and its "never touched" test stays.
3. **`mismatch_acknowledged`, in both modes:**
   - true for a folder this page made, or found empty;
   - the tick for a folder that holds files;
   - false for its checkout.

   Today it is `!draft.repository_fingerprint`. That would silently
   acknowledge a folder full of files that has no remote.

A subfolder of a checkout has a `.git` above it but no `.git/config` of its
own. It reads as "No Git remote to check." and takes the tick.

## Safety

- Mallard writes nothing into an existing folder: no clone, no folder, no
  pull. Pull stays plan-based, with nothing ticked.
- A later Pull into a folder that already had files keeps each local file
  that differs, as a `-local-copy` sibling written before the storage version
  (`plan_project_file_preservation`). With no reviewed base yet, as right
  after connecting, it fails closed to keeping them.
- **No relaxation.** A folder with files that Mallard can't verify needs
  setup's tick, exactly as on the setup page and at finalize. The shipped
  relaxation (a new or empty folder made for this repository) is unchanged,
  and is no longer keyed on "no remote".
- The remote is shown as host and path only. Nothing new is stored or sent.
- The refusals are unchanged. An existing project becomes a way on
  (Open it) instead of a dead end.

## Contract changes

- Rust: `CloudProjectFolder::Occupied { suggestion, local_fingerprint,
  git_remote }`, and a new `CloudProjectFolder::Project { project_id }` that
  replaces the refusal message. Mirrored in `src/types.ts`. `service.rs`
  gains `origin_url` (the bounded `.git/config` read `repository_fingerprint`
  already did, now shared), and `repository_fingerprint_from_remote_url` is
  visible to `cloud_project`.
- Frontend, in `CloudProjectSource`:
  - `FolderMode`, the pill switch, Browse per mode (`browsedPath`), and the
    tick (`needsTick`, bound to one folder and one repository);
  - `destinationVerdict` takes the mode;
  - `mismatchAcknowledged`, `connectFolder` (setup's three calls) and
    `connectExistingFolder` (probe again, then connect; null when the folder
    changed), exported for the tests;
  - `onOpenProject`, passed by `NewProjectFolderStep`.
- `FolderVerdict.action` becomes `actions` (`FolderAction`: new text, or a
  `FolderRun` the owner carries out). `ProjectFolderField` renders each one,
  takes a `heading` in its label's place, and `onRun` replaces
  `onAnotherProfile`. On this computer's single actions moved over.
- CSS: none (`setup-pill-row` as the Storage band has it).

## Tests

Bars at landing: Tauri lib 420 passed (0 failed), frontend 450/450,
`npm run build` green, clippy clean on the touched Rust.

- **Rust** (`project_sync::cloud_project`):
  - `occupied` carries the origin's fingerprint;
  - `git_remote` is host and path only for `https://token@host/o/n.git?x=y`,
    `git@host:o/n.git` and a GitHub remote;
  - a folder that is a project reads as `project`;
  - `get_cloud_project_folder` still refuses `occupied` (existing test).
- **Frontend** (`tests/frontend/cloud-project.integration.test.tsx`):
  - each Existing folder line, and New folder's two ways on;
  - `needsTick` and `mismatchAcknowledged`;
  - Browse picks the parent under New folder and the folder itself under
    Existing folder;
  - `connectExistingFolder` against a stubbed `projectSyncApi`: the typed
    path, never `getCloudProjectFolder`, `mismatch_acknowledged` false
    without the tick (and no call at all when the tick is needed), true
    with it and for an empty folder, false for its checkout, and nothing
    written when the folder changed;
  - `ProjectFolderField` with a heading and two ways on.
- **By hand**, the shipped component on `cloud-project-live.html`: Use it as
  it is switches the mode and keeps the text; Open it opens the project; the
  New folder path still sends `ack=false` for a clone and `ack=true` for the
  empty fallback.

## Checked while planning

- A "not pushed yet" repository, like the screenshot's, connects **when its
  registration head exists**. That is the normal case:
  - only the desktop creates repositories, and the service requires a
    `bundle_id`;
  - the desktop publishes a registration-only head at link time, and
    finalize's `engine.inspect` finds it.
- "not pushed yet" is also what the list says for a repository with **no
  head at all** (`registration_only` falls back to true without a summary):
  a claim that never published, or plausibly one after a website discard.
  That repository fails at finalize ("bundle … does not exist") from either
  door, today already. Which case HorrorGame is, isn't known from here.
- The fingerprint a repository carries is the first machine's origin. A
  checkout here matches it; anything else takes the tick.

## Not done / open

- C's detection (layer 2).
- A live pass in the Tauri app: a real checkout, and a real folder of files,
  on live Mallard Cloud, with the native Browse under each mode.

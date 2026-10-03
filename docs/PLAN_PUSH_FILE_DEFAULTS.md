# Plan: which project files a non-Git Push ticks by default

Status: owner picked A on 2026-10-02. Both layers are implemented
(uncommitted); see "As built" at the end for where the code differs from the
plan below.

Specimen: `mockups/push-file-defaults-mockup.html` + `mockups/push-file-defaults-mockup.tsx`
(`npm run dev`, then `/mockups/push-file-defaults-mockup.html`;
`?scenario=first|later|edits|big&open=history|project_files&width=w684|w970&theme=light|dark&only=before|a|b|c&shot=on`).

## What ships today

1. **The scan is lazy.** `prepare_push_review` builds a project-files inventory
   only for Git-managed projects. A non-Git folder is walked when the Files tab
   is opened (`scanPendingProjectFiles` in `ProjectSync.tsx`).
2. **Push from the Sessions step sends no files.** Without a review token the
   push does not walk the folder (`capture_request.include_project_content =
   wants_project_content`), so an edit made here to a file already in storage
   is not captured. Nothing on the Sessions step says so. Confirm the
   stored copy is kept as-is with a test before relying on it.
3. **Opening the tab ticks every new file.** `selected_after_scan` = local, not
   in storage, not in the recipe, not blocked, not left out earlier. Findings
   do not opt out, so `.env` and `run.sh` are ticked and Push turns blocked.
4. **Unticked is remembered.** After a push with a reviewed inventory, every
   local file left unticked goes to `excluded_resource_ids`.
5. **The walk reads every file** up to 16 MiB (hash and secret detection);
   larger files are blocked. `node_modules`, `target`, `dist`, `build`, VCS
   folders and `.gitignore` / `.ignore` / `.mallardignore` matches are skipped.

So today's answer to "are files ticked by default" is: none until you look,
every new one once you look. Looking at a tab changes what Push sends.

## What other tools do

Checked against vendor docs or source on 2026-10-02.

| Tool | Changed, sent before | New files | Safety net |
| --- | --- | --- | --- |
| **Review, then commit** (local, can be amended) | | | |
| [Git CLI](https://git-scm.com/docs/git-commit) | only with `commit -a` | never without `git add` | `.gitignore`, `commit --dry-run` |
| [GitHub Desktop](https://docs.github.com/en/desktop/making-changes-in-a-branch/committing-and-reviewing-changes-to-your-project-in-github-desktop) | ticked | ticked | `.gitignore`; 100 MB check; [open request](https://github.com/desktop/desktop/issues/21427) to stop ticking untracked files |
| [VS Code](https://github.com/microsoft/vscode/blob/main/extensions/git/package.json) | asks "stage all?" | in the same prompt | Yes / Always / Never; smart commit is off by default |
| [JetBrains IDEs](https://www.jetbrains.com/help/idea/commit-and-push-changes.html) | whole changelist | separate Unversioned node | that new files start unticked is implied, not stated |
| [Sublime Merge](https://www.sublimemerge.com/docs/getting_started), [Sourcetree](https://support.atlassian.com/sourcetree/kb/viewing-file-status-of-a-repository/), [GitKraken](https://help.gitkraken.com/gitkraken-desktop/staging/) | unstaged | unstaged | one Stage all control |
| **Sync or back up to a private place** (no review) | | | |
| [Dropbox](https://help.dropbox.com/sync/ignored-files), [Syncthing](https://docs.syncthing.net/users/ignoring.html) | all | all | per-file ignore, `.stignore` |
| [Time Machine](https://support.apple.com/guide/mac-help/mh15622/mac), [Backblaze](https://help.backblaze.com/hc/en-us/articles/217666298-Exclusions-Settings-Mac), [Arq](https://www.arqbackup.com/documentation/arq6docs/English.lproj/excludeFiles.html), [restic](https://restic.readthedocs.io/en/stable/040_backup.html) | all | all | exclusion lists; Backblaze ships defaults |
| **Publish to an audience** (cannot be taken back) | | | |
| [npm publish](https://docs.npmjs.com/cli/v11/commands/npm-publish) | all | all | `files` allowlist, never-include list, `npm pack --dry-run`; [npm's blog](https://blog.npmjs.org/post/165769683050/publishing-what-you-mean-to-publish) calls the allowlist the safest method |
| [Docker build](https://docs.docker.com/build/concepts/context/), [Vercel CLI](https://vercel.com/docs/deployments/vercel-ignore), [gcloud](https://docs.cloud.google.com/sdk/gcloud/reference/topic/gcloudignore) | all | all | `.dockerignore`; Vercel skips `.env.local` by itself; gcloud generates an ignore file |
| [cargo package](https://doc.rust-lang.org/cargo/commands/cargo-package.html) | all | refuses a dirty tree | outside Git it skips every dotfile; `--list` previews |
| [GitHub push protection](https://docs.github.com/en/code-security/secret-scanning/introduction/about-push-protection) | secrets blocked | secrets blocked | on by default for public repositories |

Guidance:

- [Microsoft, Windows checkbox guidelines](https://learn.microsoft.com/en-us/windows/win32/uxguide/ctrl-check-boxes):
  a checkbox starts in the safest, most private state; convenience decides only
  when safety is not in play.
- [Nielsen Norman Group](https://www.nngroup.com/articles/the-power-of-defaults/):
  people keep the defaults they are given.
- [GDPR Art. 25(2)](https://gdpr-info.eu/art-25-gdpr/): by default, personal
  data must not reach an open-ended audience without the person acting.
- [Saltzer & Schroeder](https://web.mit.edu/Saltzer/www/publications/protection/Basic.html),
  fail-safe defaults: the default is no access.

Reading: every tool sends a changed file it has sent before. They split on
files never sent, and the split follows whether a list is shown first and
whether what leaves can be taken back. A Mallard Push is reviewed on a second
tab, is published immutably, and can go to a public Mallard Cloud project. That
is the third group, where include-everything is the documented source of leaks
and the remedies are allowlists, previews and secret blocks.

## Directions

| File | Today | A | B | C |
| --- | --- | --- | --- | --- |
| Changed here, already in storage | ticked once the tab is opened; else the edit stays here | ticked | ticked | ticked |
| New, no finding | ticked once the tab is opened | unticked, counted on the step button | ticked | unticked; ticked with the switch on |
| New, executable or credential-shaped | ticked; Push blocked until reviewed | unticked | unticked | unticked |
| Left out at an earlier push | unticked | unticked, not counted | unticked | unticked |
| Push from the Sessions step | no files, no edits | edits travel; new files wait | waits until Files was shown | as the switch says |

- **A · Changed files travel, new files wait** (recommended). The step button
  on the Sessions step reads `Project files [N new] ›`. On the Files step
  **Select** ticks every file without a finding that nobody left out. Push stays
  live on both steps.
- **B · New files travel, after one look.** New files without a finding are
  ticked. While such ticks exist, Push waits until the Files step was shown
  once. Files spend the 512 MB budget before sessions do, and a large folder
  opens refused (the `big` scenario) unless files get a fit rule.
- **C · A, with a switch.** `New files · Wait / Travel` on the Files step,
  saved with the storage link. One persisted field and a second behaviour.

Why A: it matches the product's own rules (deletion is opt-in, file sync is
"explicitly reviewed", nothing ticked by default travels unseen), it needs no
fit rule and no gate, and its cost to the user is one press of Select per
batch of new files. Its risk is a first push that leaves the files behind; the
count on the step button is the answer to that.

## Plan for A, in layers

### Layer 1 — the default rule (the scan stays lazy)

- Rust `ProjectContentEntry`: replace `selected_after_scan` with `undecided`
  (same predicate: local, not in storage, not in the recipe, not blocked, not
  left out). It no longer feeds the selection. Mirror in `src/types.ts`.
- `scanPendingProjectFiles`: tick `selected_in_recipe` entries only.
  `projectDefaults` (what Select applies) gains the undecided entries without a
  `warning_code`. `projectContentReviewRow.default_selected` follows.
- `ProjectFilesReviewPage`: push rows that are undecided wear a `New` chip.
- Tests: opening the Files step ticks no new file; Select ticks the safe new
  ones and no flagged one; a changed stored file is still ticked.

This layer alone answers the default. The review no longer opens blocked.

### Layer 2 — the scan runs with the review

- `openPushChooser` starts `scanPendingProjectFiles` for an eligible project
  once the review is open. `projectContentLoading` already holds Push and the
  Files step already draws its scaffold.
- Footer step button: a count chip of undecided, unticked files.
- Changed stored files are ticked and travel without a visit, because the
  review token now always accompanies the push.
- Exclusion memory: today every unticked local file in a reviewed inventory is
  recorded as left out. With the scan at open that would record files nobody
  saw. Add one bool to the push's project-content review (the Files step was
  shown); the backend adds exclusions only when it is true.
- Measure the walk on a real folder first. It reads every non-ignored file up
  to 16 MiB on each non-Git review. If that is slow, the chip slot shows the
  scan running and Push waits, as it does today during a scan.
- Tests: a push from the Sessions step captures a changed stored file; an
  unseen new file is not recorded as left out; a seen, unticked one is.

### Not in this plan

- B's gate and a fit rule for files.
- C's switch (`ProjectContentPreferences` field, switch, tests).

## Safety and schema

- No invariant is relaxed. A is stricter than today on two points: new files
  and flagged files are never ticked for the user. Warning acknowledgement, the
  review token check and the deletion approval are unchanged.
- `selected_after_scan` is a response DTO field, not persisted. Layer 2 adds
  one request field. `excluded_resource_ids` keeps its shape; schema numbers
  stay 1.

## As built

- `ProjectContentEntry.undecided` replaces `selected_after_scan`. The scan
  (`scanProjectFiles` in `ProjectSync.tsx`) starts with the review for a
  non-Git project and ticks only what the recipe holds. The Files step no
  longer starts it; a failed walk reads "Project files were not scanned." and
  Rescan retries.
- The review stays usable during the walk. Both Push buttons read
  "Scanning files…" until it lands.
- Step button: `Project files [N new] ›`, counting undecided, unticked cards
  (`projectFileReviewCards`). Rows wear a `New` chip.
- Select offers `projectFileSelectOffer`: recipe files, new files without a
  finding, and new folders only when empty. A folder that holds entries rides
  the required-ancestor rule, so one whose only file is flagged does not
  travel empty.
- **Not in the plan: the token rule.** `pushProjectFileWork` decides whether
  the push sends the scan's review token: `send` (removal, recipe change,
  selected file differs from storage), `remember` (a new file left unticked on
  a shown list), or null. Without the token the backend does not walk the
  folder, so a push of sessions cannot be refused with "project files changed
  after Scan" while an agent is still writing there. The meter counts project
  files only when the token is sent (the backend then captures every selected
  file), and a selection of files storage already holds reads "Nothing is
  selected".
- Push request: `project_files_shown`. The backend records left-out files
  only when it is true (`ProjectContentPushReview.files_shown`).
- Verified: the backend test
  `new_project_files_wait_and_only_a_shown_list_remembers_the_ones_left_out`
  also confirms fact 2 above (an edit does not travel without the token), and
  `mockups/push-files-live.html` drives the real shell on a stand-in backend.

## Still open

1. The folder walk has not been measured on a large real folder.
2. Session defaults are computed before the scan lands. When stored files
   changed and are large, sessions plus files can open over the limit; the
   meter refuses and the user unticks.
3. A live pass in the packaged app.

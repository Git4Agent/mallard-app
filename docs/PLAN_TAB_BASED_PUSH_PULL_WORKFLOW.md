# Tab-Based Push and Pull Workflow

## Status

Design only. This document does not change the current sync behavior, storage schema, or cloud object layout.

## Goal

Replace the separate technical resource chooser with one consistent review workflow for both Push and Pull. The workflow reuses the project's existing **Git & sessions**, **Skills**, and **Plugins** tabs, then ends with a concise **Review** step.

The user should review recognizable project content instead of raw resource IDs.

## Design thesis

- **Visual thesis:** sync review is a temporary mode of the existing project workspace, not a modal or a second application surface.
- **Content plan:** destination context, Sessions, Skills, Plugins, Review, then progress and result.
- **Interaction thesis:** keep the selected storage fixed, preserve the familiar tab layout, and reveal warnings directly beside the affected resource.

## Shared workflow shell

Clicking **Push** or **Pull** on the active storage enters an inline sync-review mode below the compact storage row.

```text
Storage: Local storage 0                              Cancel Push

[ Git & sessions  7 ] [ Skills  2 ] [ Plugins  1 ] [ Review ]

Existing project content with selection and sync-state controls

7 included                                      [Next: Skills →]
```

The shell has these rules:

- The active storage is the destination/source for the entire review.
- The storage picker and storage mutations are locked until the review is closed.
- Push and Pull use the same four tabs and row components.
- The current mode is stated once: `Push to Local storage 0` or `Pull from Local storage 0`.
- Tab badges show the number selected. An amber dot indicates that the tab needs attention.
- A sticky footer contains Back, Next, the current selection summary, and the final action.
- Cancel returns to the tab that was active before the review and makes no changes.

## Shared resource states

Use one presentation model even when the backend reports slightly different names for thread and capability states.

| State | Presentation | Push meaning | Pull meaning |
| --- | --- | --- | --- |
| Synced | Green check | Retain if included; no upload change | No restore action |
| Local only | Blue upload | Safe local addition | Keep local |
| Local ahead | Blue upload | Safe local update | Keep local by default |
| Storage only | Blue download | Pull before Push; never remove silently | Safe storage addition |
| Storage ahead | Blue download | Pull before Push | Safe storage update when local is unchanged |
| Diverged | Amber warning | Push blocked until Pull review | Explicitly choose local or storage |
| Unknown | Gray question | Refresh or block final action | Refresh or block affected action |
| Blocked | Red warning | Explain the blocker | Explain the blocker |

Color is always paired with an icon, label, and accessible description.

## Push workflow

### Entry

When Push is clicked, load the current project inventory and the selected storage comparison. Initialize selections from the recipe saved on that specific project-storage link.

Checkboxes mean **included in this storage backup**, not merely **changed since the last push**. The Review step separately reports how many included resources will actually change.

### Step 1: Git & sessions

Reuse the current Git history and session UI.

- Add one inclusion checkbox to each unique session.
- Keep the session title, commit grouping, latest-update time, status indicator, details, and launch actions.
- A session appearing under multiple commits has one shared selection state.
- Remove raw UUID rows and hide the technical project-session index.
- Include required indexes automatically when conversations are included.
- Put storage-only sessions in a concise `Only in storage` section.
- Retain the existing history pagination. Selections outside the loaded date range remain selected and are counted as `included outside this view`.
- Offer filters for `All`, `Included`, `Local changes`, and `Needs review`.
- `Include all shown` affects only the currently visible filtered rows.

Storage-only, storage-ahead, and diverged sessions show `Pull required`. They block the final Push rather than implying that Push can safely overwrite them.

### Step 2: Skills

Reuse the Skills status list in selection mode.

- Project skills and standalone skills are selectable.
- Keep local/storage status and version information available in row details.
- Plugin-provided skills remain read-only and identify their parent plugin.
- New or locally updated skills can be selected normally.
- Storage-newer or diverged skills require Pull review before Push.

### Step 3: Plugins

Reuse the Plugins status list in selection mode.

- The checkbox controls portable plugin-install intent.
- Plugin payloads, caches, credentials, and machine-local state remain excluded.
- Skills supplied by a plugin remain nested beneath it and are not selected separately.
- Conflicts follow the same blocking rules as Sessions and Skills.

### Step 4: Review and Push

The Review tab summarizes intent without repeating every row.

```text
Push to Local storage 0

Sessions        7 included     3 updates
Skills          2 included     1 update
Plugins         1 included
Other           4 included

No unresolved storage changes

[Back]                                  [Push 14 resources]
```

`Other` contains project instructions, settings, MCP definitions, hooks, and other supported project resources. It is collapsed by default but remains inspectable and editable. Nothing is silently added.

The Review step must:

- Separate `included` from `will change`.
- Link each warning back to the affected tab and row.
- Disable Push while storage-newer, diverged, unknown, or blocked resources remain unresolved.
- Offer `Pull and review` when a newer storage revision blocks Push.
- Prevent an accidental empty publication. An empty revision requires a separate explicit confirmation.
- State the destination and expected next revision when known.

### Push execution

After confirmation:

1. Freeze tabs and selection controls.
2. Capture the selected local resources.
3. Publish using the existing expected-head safety check.
4. Save the per-storage recipe only after publication succeeds.
5. Refresh Sessions, Skills, Plugins, and storage comparison data.

On success, exit review mode and show a compact message such as `Pushed revision 4 · 14 resources`. Detailed events remain available in Log.

If the storage head changes during review, keep the user's selection, return to Review, and show `Storage changed while you were reviewing. Refresh before pushing.`

## Pull workflow

### Entry and immutable plan

Clicking Pull fetches the selected storage revision and creates the existing immutable restore plan. The UI may change presentation, but it must preserve the current plan alignment and approval guarantees:

- Restore and dependency plans refer to the same storage, bundle, revision, manifest, and binding revision.
- The plan expires and must be refreshed if its snapshot becomes stale.
- No local mutation occurs before the final Apply action.

### Step 1: Git & sessions

Render conversation restore actions through the same Git and session UI used by the project.

- Storage-only and storage-ahead sessions are selected by default when the restore is safe.
- Local-only sessions show `Kept on this computer` and require no action.
- Local-ahead sessions are not overwritten by default.
- Diverged sessions show an inline choice:
  - `Keep local`
  - `Use storage version`
- Choosing the storage version maps to the corresponding immutable restore action.
- Choosing local means that restore action is omitted; it does not mutate the plan.
- Session details and chat history remain lazy-loaded.

The conflict choice should be presented only for affected rows. Do not add a global conflict form.

### Step 2: Skills

Use the same selectable Skills list.

- New storage skills are selected when installation is safe.
- Overwriting a local custom skill requires explicit approval.
- State that Mallard creates a local backup before replacement.
- Skills provided by selected plugins remain attached to their plugin and cannot be restored independently.
- Missing installers or invalid skill metadata appear as row-level blockers.

### Step 3: Plugins

Use the Plugins list to approve installation intent.

- Plugin installation is never silently selected when explicit approval is required.
- Show provider, marketplace/source, observed version, and provided skills in details.
- Clearly distinguish `Restore configuration` from `Install plugin` when both actions exist.
- A failed installer can be retried without repeating successful file restores.

### Step 4: Review and Pull

Summarize the selected restore and installation actions.

```text
Pull revision 4 from Local storage 0

Sessions        3 to restore    1 keep local
Skills          2 to restore
Plugins         1 to install
Project files   4 to restore

Backups will be created before 2 local files are replaced

[Back]                                  [Apply 10 changes]
```

The Review step also contains:

- A collapsed `Project files & settings` section for instructions, settings, MCP definitions, hooks, and manual actions.
- A concise list of unresolved approvals or blockers.
- Readiness issues that will remain after the selected actions.
- A direct link back to the affected tab or resource.
- The local project root, assigned profile, source storage, and revision.

The primary action is disabled when no action is selected. Skipped optional tools do not prevent restoring safe project data, but the result must clearly state that setup remains incomplete.

### Pull execution

Keep the current ordered execution model:

1. Restore selected project data and custom skills.
2. Install selected plugins and tools.
3. Verify readiness.

During execution, keep the tabs visible but read-only and show progress inside Review. Do not move the user to another screen.

After execution:

- Mark successful rows complete across all tabs.
- Keep failed or skipped rows actionable.
- Allow retrying only failed or remaining actions.
- Show `Pull complete` only when all selected work succeeded and readiness is confirmed.
- Otherwise show a concise partial result such as `Project restored · 1 plugin needs attention`.

## Selection and persistence

| Behavior | Push | Pull |
| --- | --- | --- |
| Initial selection | Saved recipe for this project-storage link | Safe defaults from immutable restore/dependency plans |
| Persistence | Save only after successful Push | Ephemeral; completed action IDs are tracked for retries |
| Cancel | Discard edits | Discard unapplied approvals |
| Hidden/paginated rows | Preserve and count | Preserve and count |
| Conflict default | Block and direct to Pull | Keep local until explicitly approved |

## Component plan

Introduce a shared presentation shell while retaining the existing backend safety model.

- `ProjectSyncReviewWorkspace`
  - Owns mode, current step, locked storage context, footer, and review navigation.
- `ProjectWorkspaceTabs`
  - Accepts `mode: normal | push | pull`, selection counts, warning counts, and the Review tab.
- `ProjectChatHistoryContent`
  - Gains an optional selection/conflict mode keyed by thread resource ID.
- `SkillsPluginStatusContent`
  - Gains an optional selection/approval mode without duplicating the existing status rows.
- `SyncReviewSummary`
  - Shared Push/Pull review layout with mode-specific copy and actions.
- `RestorePlanView`
  - Retains plan mapping, phase execution, retry, and result logic; its resource lists move into the shared tabs.
- `PushResourceWorkspace`
  - Is replaced after the new workflow reaches parity.

Recommended review state:

```ts
type SyncReviewMode = "push" | "pull";
type SyncReviewStep = "history" | "skills" | "plugins" | "review";

interface SyncReviewSession {
  mode: SyncReviewMode;
  projectId: string;
  storageId: string;
  step: SyncReviewStep;
  selectedResourceIds: Set<string>;
  conflictDecisions: Map<string, "keep_local" | "use_storage">;
}
```

Push continues to use inventory, bundle status, thread comparison, capability status, and `push_bundle`. Pull continues to use the immutable restore plan, dependency plan, readiness report, and existing apply commands. No cloud schema change is required for the first implementation.

## Accessibility and interaction details

- Keep tabs as an accessible `tablist` with arrow, Home, and End navigation.
- Include counts and warnings in accessible tab names without making visual labels noisy.
- Use real checkboxes for inclusion and radios for two-way conflict decisions.
- Maintain visible keyboard focus in dense session lists.
- Announce plan loading, publishing, restoring, installing, verification, and results through polite live regions.
- Return focus to the originating Push or Pull button when review is cancelled.
- Respect reduced-motion preferences; transitions should be short layout fades or shared-tab movement only.

## Implementation phases

1. **Shared shell**
   - Add review mode, locked storage context, tab counts, sticky footer, and Cancel behavior.
2. **Push tabs**
   - Add session, skill, and plugin selection modes; map selections back to the destination recipe.
3. **Push safety and Review**
   - Normalize states, surface conflicts, add Other resources, stale-head handling, and final summary.
4. **Pull tabs**
   - Project immutable restore/dependency actions into Sessions, Skills, Plugins, and Other resources.
5. **Pull execution and retry**
   - Move the existing progress, partial-result, and retry behavior into the Review tab.
6. **Remove the old chooser**
   - Delete the generic Push inventory surface only after feature and test parity.

## Verification plan

Frontend integration tests should cover:

- Push and Pull enter the same tabbed review shell.
- The chosen storage stays locked throughout review.
- A thread shown under multiple commits has one selection state.
- Hidden and paginated selections are preserved.
- Skills supplied by plugins cannot be selected independently.
- Storage-newer and diverged states block Push.
- Diverged Pull rows require an explicit local/storage choice.
- Review counts match the exact Push recipe or Pull action selection.
- Cancelling either flow performs no mutation.
- A stale Push head keeps the selection and requires refresh.
- Partial Pull failures retry only remaining work.
- Keyboard tab navigation and focus restoration work.

Backend tests should retain and extend coverage for:

- Expected-head enforcement on Push.
- Per-storage recipe persistence only after successful publication.
- Restore/dependency plan snapshot alignment.
- Explicit approval filtering.
- Backups before replacement.
- Idempotent retry of completed Pull actions.

## Acceptance criteria

- Users never need to select raw resource IDs.
- Push and Pull are recognizable variants of one workflow.
- Sessions are reviewed in Git/session context.
- Skills and Plugins reuse their normal project views.
- Conflicts appear beside the affected content and cannot be bypassed accidentally.
- Every mutation is summarized before execution.
- The destination/source storage remains unambiguous.
- Existing cloud schema and backend concurrency safeguards remain intact.

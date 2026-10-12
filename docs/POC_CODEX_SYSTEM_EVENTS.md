# System Events thread-ID POC

This standalone POC uses macOS `osascript`, JXA, and System Events. It needs no
download, npm package, Rust crate, Swift compiler, or Xcode installation.
It has no production integration.

The earlier direct scripting probe returned zero Codex scriptable windows.
This probe uses System Events' Accessibility window tree instead. It sends no
JavaScript to Codex and does not use Codex's scripting dictionary.

## Selection rule

The selected window must expose exactly one eligible navigation landmark
(`AXLandmarkNavigation`). The project list's Accessibility label must match
exactly and occur once inside that landmark. Within that list, exactly one
top-level chat button must expose the requested native thread UUID through one
of these fields:

- `AXIdentifier` or `AXDOMIdentifier`: exactly the UUID or `local:<UUID>`.
- `AXURL`: exactly `codex://threads/<UUID>`, with no query or fragment.

A group or row wrapper can supply identity only if it contains exactly one chat
button. Nested action buttons and nested lists cannot identify a parent chat.
Conflicting fields, duplicate IDs, and duplicate project labels cause refusal.
Chat titles and UUID substrings cannot authorize a press.

The scan has three parts: locate navigation, locate the project list, and read
that project's rows. Navigation discovery can cross container landmarks such
as regions, banners, and complementary regions. It skips the main-content
landmark (`AXLandmarkMain`), button contents, and other English project lists
labelled `Scheduled tasks in ...`. It can pass through the outer Projects list.
It does not scan the full window's
descendants before selecting a project. Child roles are read in batches;
subroles are read only while locating navigation. Requested AX attributes are
also read in filtered batches. Element references retain their original
positions. No unrelated attribute values are fetched.

If the navigation landmark is absent or ambiguous, the script stops. It does
not revert to a full-window scan. The UI structure is version-dependent; the
user's earlier real Codex Accessibility dump established this navigation
landmark for the tested build. It did not establish UUID exposure.

## Manual Codex check

Keep Codex open with the target project list exposed. Run from the Mallard
repository root. Use the native thread UUID and the exact project list label
from Accessibility. The project label can differ with the UI language.

```sh
/usr/bin/osascript -l JavaScript scripts/poc-codex-system-events.js \
  --mode=diagnose \
  --thread-id=NATIVE_THREAD_UUID \
  '--project-list-label=EXACT_PROJECT_LIST_LABEL'
```

The probe identifies the Codex process by bundle ID and retains its unique
System Events process ID. It selects the only window or, when there are several,
the unique main window. To specify a window, add `--window-title=EXACT_TITLE`.
It retains positional UI references so equal chat titles do not alias each other.

Progress JSON goes to stderr immediately, then at each phase change and at most
once per second during a scan. Scan phase changes also print immediately.
It includes the phase, a sampled operation, and
the visited node count. It contains no project labels, chat titles, or UUIDs.
The last sampled operation is not necessarily the call that blocked.

The probe has a 15-second probe budget and a 20-second OS timer. The OS timer also
terminates a blocked synchronous call, including application discovery. If it
fires, `osascript` exits with status 142 (SIGALRM). The shell can print `Alarm
clock`. There is no final result JSON in that case; retain the progress lines.
Check the exit status immediately with `echo $?` if needed.

Stop a previous stuck command with Ctrl+C before retrying. Run diagnose first.
The earlier version had no process-level time limit and returned no progress.
Its fixture checks did not prove that System Events would respond for Codex.
The first bounded real-Codex run found one window, then returned `probe_timeout`
at `inspect`. That result established access to a window, but no thread identity.
The targeted scan replaces the expensive full-window traversal. The 15-second
probe budget and 20-second OS timer remain unchanged.

On failure, the final JSON retains `scanPhase`, `nodeCount`, `roleCounts`,
`navigationCount`, `projectListCount`, `lastOperation`, and `elapsedMs` when
available. A partial scan never authorizes a press.

The result also includes `navigationOutline`: up to 80 visited nodes, with only
their AX role, subrole, depth, and traversal decision. `navigationOutlineTruncated`
shows whether more nodes were visited. `skip_main` means that the probe did not
enter that main-content container. It does not prove that the container lacks
navigation. Labels, chat text, and identity values are excluded.

A real Codex run returned `navigation_missing` after 33 nodes in 3.1 seconds,
with the renderer exposed. The previous filter excluded every non-navigation
landmark. A fixture reproduced a failure when a region enclosed the sidebar;
the filter now permits that structure. The reported Codex result had no subrole
data, so it does not establish which branch caused that failure. Use the outline
from a new diagnostic to check that boundary. A sidebar inside `AXLandmarkMain`
remains outside this probe's search scope.

A diagnostic must report all of these fields before a press is attempted:

```json
{"ok":true,"transport":"system_events","selectorResult":"matched","exactThreadIdentityProven":true,"pressSent":false}
```

Run the same command with `--mode=press` to select the row. The script locates
the project once. It then rechecks the navigation landmark, project label, and
fresh project rows through their retained references before sending one
`AXPress`. Verification reads that same project again. It does not repeat the
sidebar search. Success reports
`code: "selected"` and `projectRowSelected: true`. An already-current row reports
`already_selected` and sends no press. An uncertain reply reports
`pressSent: "unknown"`; the script never repeats that action.
Before sending the action, progress also reports `pressSent: "unknown"`.
If a hard timeout then occurs, the action may have reached System Events. A
timer terminates this script; it cannot cancel an Apple event already sent.

Read JSON `ok`, not just the shell exit status. A completed `osascript` command
can exit with status zero while its JSON reports refusal.

| Result | Meaning |
| --- | --- |
| `accessibility_permission_required` / `accessibility_access_failed` | System Events could not use Accessibility. The latter also includes an invalid UI index. |
| `automation_permission_required` | macOS did not permit the caller to control System Events. |
| `accessibility_window_missing` | System Events returned no window for the target process. |
| `window_missing_or_ambiguous` | The window cannot be selected uniquely. |
| `navigation_missing` / `navigation_ambiguous` | The eligible sidebar navigation landmark cannot be selected uniquely. |
| `project_list_missing` | The exact labelled list is absent. Check `rendererExposed` and `roleCounts`. |
| `thread_id_not_exposed` | No supported thread identity is exposed on the project's chat rows or permitted wrappers. |
| `thread_id_not_found` | Some rows expose IDs, but none matches the requested UUID. |
| `thread_id_ambiguous` / `project_list_ambiguous` | More than one candidate matches. No press is sent. |
| `project_list_changed` / `thread_identity_changed` | The target changed before the action. No press is sent. |
| `navigation_changed` | The navigation landmark changed before the action. No press is sent. |
| `probe_timeout` | The 15-second budget expired. Partial scan diagnostics remain in the result. |
| `selection_not_verified` | One press was sent, but the project row did not become current during verification. |

The script does not set Accessibility attributes, enable renderer access, open
or activate Codex, expand projects, scroll, load more rows, pin chats, or change
sidebar preferences. It does not modify Codex files or read provider homes.
It reads list labels and identity fields, and omits their values from output.
macOS may request Automation or Accessibility permission for the calling app.

## Verification

Pure checks:

```sh
/usr/bin/osascript -l JavaScript scripts/test-poc-codex-system-events.js
```

These checks also launch owned `osascript` children with a blocking read and a
blocking press. They confirm OS timer termination and the last reported action
state. A third case checks that successful completion cancels the timer. These
children do not connect to System Events or Codex.

Native System Events checks against separate fixture windows:

```sh
/usr/bin/osascript -l JavaScript scripts/test-poc-codex-system-events.js --native
```

The native suite uses only macOS AppKit and WebKit through JXA. It launches its
own synthetic fixture processes, verifies their owned PIDs, rejects a Codex
process, and terminates only its fixtures. It checks missing identities, an
exact ID among equal titles, duplicate IDs, and duplicate project labels. An
independent fixture report verifies the click count, selected UUID, project
occurrence, and continued presence of the Recents copy.
Another fixture encloses the sidebar in an exposed `AXLandmarkRegion` and checks
that the probe crosses it before selecting the project row.
The large fixture adds over 8,000 HTML elements in main content and other project
lists. The test requires selection within the unchanged time limit and fewer
than 300 inspected Accessibility nodes. Pure tests also cover missing or
duplicate navigation, enclosing container landmarks, a changed navigation
landmark, bounded structural diagnostics, and partial timeout output.

The positive fixture deliberately exposes an HTML `id`. Codex's custom `data-*`
attribute alone does not prove equivalent Accessibility exposure. The native
fixture checks do not establish that real Codex exposes the required UUID.
The report retains `realCodexTested: false`.

The available computer-use tool excludes Codex. Real Codex commands are provided
for the user to run manually; automated tests target only the separate fixtures.

References: [Apple UI scripting](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/AutomatetheUserInterface.html)
and [Apple JXA object references and event timeouts](https://developer.apple.com/library/archive/releasenotes/InterapplicationCommunication/RN-JavaScriptForAutomation/Articles/OSX10-10.html).

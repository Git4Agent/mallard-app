# macOS scripting POC

Status: the user's real Codex diagnostic returned `scriptable_window_missing`
with `scriptableWindowCount: 0`. The app was running, but its scripting interface
returned no windows. The probe stopped before JavaScript execution or ID lookup.
It sent no click. This route is not usable in the reported app state.

The 44 local checks passed in JXA and a separate WebKit fixture. Those checks use
a simulated Apple-event transport. They do not establish real app support.
Production code is unchanged.

Reported result:

```json
{"ok":false,"mode":"diagnose","code":"scriptable_window_missing","pressSent":false,"labelsRedacted":true,"identityValuesRedacted":true,"scriptableWindowCount":0}
```

This result does not test the JavaScript-from-Apple-events preference. Changing
that preference cannot be justified by this failure. Do not run press mode for
this result.

## Route

`osascript` runs JXA. JXA uses the installed app's `execute javascript` Apple
event to inspect the front scriptable view. The probe locates the exact project
ID in the Projects section, then the exact `local:<native-thread-UUID>` row.
Press mode calls the row's normal click handler. Codex supplies the navigation
state for that project occurrence.

This route needs no npm package, Rust crate, Swift compiler, Xcode, Node, browser
driver, or Accessibility helper. macOS may request Automation access for the
Terminal app that runs the command.

The inspected desktop build, 26.1007.30912, includes a Chromium scripting
dictionary with window, tab, and `execute` commands. Its renderer includes the
project and thread attributes used here. These facts do **not** prove that the
native app view is exposed as a scriptable tab, or that JavaScript from Apple
events is enabled. This POC tests those two conditions. It does not enable them.

## Manual check

Open the target chat in Codex first. Keep its project and chat row visible under
Projects. Run from the Mallard repository root. Replace both placeholders with
UUIDs. The project ID is Codex's project ID; the thread ID is the native UUID.
Do not use Mallard resource IDs, `thread-...` IDs, paths, or chat titles.

```sh
/usr/bin/osascript -l JavaScript scripts/poc-codex-macos-scripting.js \
  --mode=diagnose \
  --project-id=CODEX_PROJECT_UUID \
  --thread-id=NATIVE_THREAD_UUID
```

The required diagnostic fields are:

```json
{"ok":true,"selectorResult":"matched","exactThreadIdentityProven":true,"pressSent":false}
```

Only then run press mode with the same IDs:

```sh
/usr/bin/osascript -l JavaScript scripts/poc-codex-macos-scripting.js \
  --mode=press \
  --project-id=CODEX_PROJECT_UUID \
  --thread-id=NATIVE_THREAD_UUID
```

Success reports `code: "selected"` and `projectRowSelected: true`. A row that is
already selected reports `code: "already_selected"` and sends no click.

Read the JSON `ok` field. A completed script can exit with status zero while its
JSON reports refusal. The output omits titles, IDs, URLs, transcript text, and
raw Apple-event error messages.

| Result | Meaning |
| --- | --- |
| `scriptable_window_missing` / `scriptable_tab_missing` | The interface does not expose the needed window or tab. |
| `app_view_missing` | JavaScript ran, but the tab has no active app sidebar. |
| `project_list_missing` / `thread_id_missing` | The specified project or chat row is not exposed under Projects. |
| `automation_denied` | macOS did not permit the Apple event. |
| `javascript_from_apple_events_disabled` | The app rejected JavaScript from Apple events. |
| `scripting_command_unavailable` / `scripting_object_unavailable` | The installed app does not expose the requested operation or object. |
| `apple_event_failed` | The event failed. The numeric error is included when available. |
| `selection_not_verified` | One click was sent, but the project occurrence did not become active during verification. |
| `pressSent: "unknown"` | The press request was sent, but its reply failed. No retry was sent. |

The POC checks only the front scriptable window and its active tab. It does not
scan other tabs or activate a different view. A missing app view or disabled
JavaScript result leaves this route unproven under the current app settings.

System Events UI scripting is a separate interface. It uses macOS Accessibility
and can query UI elements even when an app has no scriptable windows. It still
depends on the app's exposed Accessibility attributes. Chromium maps
`AXDOMIdentifier` to HTML `id`; it does not map a thread's custom `data-*`
attribute to that field. Moving the existing Accessibility selector into JXA
does not establish that Codex exposes the native thread UUID. The earlier
title-based real-app success does not prove selection by thread ID.

## Local checks

```sh
/usr/bin/osascript -l JavaScript scripts/test-poc-codex-macos-scripting.js
```

The suite uses synthetic HTML in an in-memory WebKit view with private browsing
enabled. It tests exact IDs, duplicate titles and IDs, Recents retention, nested
projects, hidden and inactive rows, permission errors, a changed view, and an
uncertain press reply. The test injects a transport adapter and shadows
`Application` to reject accidental real-app access. The report includes
`realCodexTested: false`.

## Scope

The script sends at most one row click. It repeats only read-only verification.
It does not expand projects, load more rows, pin chats, change grouping or
Recents visibility, set JavaScript preferences, modify the app bundle, write
history state, or read provider homes. It has no title or coordinate fallback.

The available computer-use tool excludes Codex. Automated checks therefore use
only the fixture; real Codex commands must be run manually. Do not treat fixture
success as proof that this route works in Codex.

References: [Apple's scripting architecture](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/HowMacScriptingWorks.html)
[Apple's UI scripting interface](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/AutomatetheUserInterface.html),
[Chromium's Accessibility ID mapping](https://github.com/chromium/chromium/blob/main/ui/accessibility/platform/ax_platform_node_cocoa.mm),
and [Chromium's JavaScript-from-Apple-events control](https://www.chromium.org/developers/applescript/).

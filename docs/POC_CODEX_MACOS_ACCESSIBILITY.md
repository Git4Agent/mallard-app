# macOS Accessibility thread-ID POC

Status: 18 pure selector checks and all five native scenarios passed in WebKit
and Electron 43.4.0. The user confirmed the earlier title-based selection in
real Codex. The current probe requires a native thread UUID. Real Codex exposure
of that UUID still needs a manual check. This is separate from production code.

## Scope

The probe reads the target project list through macOS Accessibility. It presses
one chat row only when an identity field contains the exact requested thread ID.
It does not use the chat title to authorize a press. It does not register a
project, open Codex, read provider homes, or change sidebar preferences.

The installed Codex renderer stores row identity in HTML `data-*` attributes.
Chromium maps `AXDOMIdentifier` to the HTML `id` attribute. A `data-*` attribute
alone does not prove native identity exposure.

The current selector accepts:

- An exact UUID in `AXIdentifier` or `AXDOMIdentifier`.
- An exact local link, `codex://threads/<UUID>`, in `AXURL`.

The identity must be on the chat button or on a wrapper inside the project list
that contains exactly one top-level chat button. A shared wrapper cannot identify
one of several rows. Nested action buttons cannot identify their parent chat.
Conflicting identity fields cause refusal. UUID substrings, titles, arbitrary
URLs, and links with profile or remote-host parameters do not authorize a press.
The project list label must also match exactly and occur only once.

`--chat-title` is optional. It limits diagnostic output only. The title-only
press option has been removed.

## Run the fixture POC

From the repository root on macOS with Xcode command-line tools:

```sh
node scripts/poc-codex-macos-accessibility.mjs
```

For a Chromium renderer, use a separate Electron runtime unpacked under
`/private/tmp`:

```sh
node scripts/poc-codex-macos-accessibility.mjs \
  --electron=/private/tmp/electron-fixture/Electron.app/Contents/MacOS/Electron
```

The runner compiles the Swift probe and uses a temporary fixture profile. It
prints the report path. The compiled `ax-probe` is beside that report. No package
dependency is added. The runner stops only its own fixture process.

The native scenarios are:

| Scenario | Required result |
| --- | --- |
| ID exists only in HTML `data-*` | Refuse with `thread_id_not_exposed`; zero clicks. |
| Exact HTML ID on the project button | Select the correct ID among identical titles. |
| Exact HTML ID on a single-row wrapper | Select the wrapper's chat button. |
| Same ID on two project rows | Refuse with `thread_id_ambiguous`; zero clicks. |
| Two lists with the same project label | Refuse with `project_list_ambiguous`; zero clicks. |

The positive fixtures deliberately expose an HTML ID. They test the selector,
not a claim about what Codex exposes. Fixture IDs are synthetic UUIDs.

Successful native cases omit the title from the press command. An independent
DOM report must confirm one click, the requested thread ID, and the project
location. The Recents copy must remain. Wrong or missing IDs must cause no click.
A second call for an already-current project row must also cause no click.

Pure selector checks cover wrong IDs, a target ID found only in Recents, an ID
inside a title, nested action IDs, shared wrappers, conflicting fields, renamed
chats, exact local links, invalid formats, and redacted diagnostic output.

Exit codes: `0` means the requested operation or fixture suite completed;
`1` means refusal or failure; `2` from the runner means native fixture checks
need Accessibility permission. A diagnostic can return `ok: true` while its
`selectorResult` reports that the target cannot be selected.

## Manual native check

Use the PID, bundle ID, window title, and exact project list label from the
previous successful manual check. Use the **native UUID**, not Mallard's
resource ID or its `thread-...` ID. Run this from the same Terminal shell:

```sh
"$mallard_ax_probe" --mode=diagnose \
  '--thread-id=NATIVE_THREAD_UUID' \
  '--project-name=PROJECT_DISPLAY_NAME' \
  "${mallard_ax_args[@]}"
```

Replace the uppercase placeholders before running. `mallard_ax_probe` must
point to the newly compiled probe. The old title-only binary does not support
this interface. The existing argument array may include `--chat-title`.

If the app was restarted and the renderer is absent, use the same arguments
with `--mode=enable-accessibility`. That mode sends one runtime
`AXEnhancedUserInterface = true` request and waits for a web area. It sends no
press. Chromium can return `-25208` while activation is pending; the probe checks
the observed tree rather than treating the setter code as the final result.

A positive diagnostic requires both:

```json
{"selectorResult":"matched","exactThreadIdentityProven":true,"pressSent":false}
```

Then run the separate press command:

```sh
"$mallard_ax_probe" --mode=press \
  '--thread-id=NATIVE_THREAD_UUID' \
  "${mallard_ax_args[@]}"
```

The press command rechecks identity. It needs no title-match opt-in. Success
requires `exactThreadIdentityProven: true` and `currentAfter: "page"`. The report
names the matching AX field and whether it belongs to the button or its wrapper.
It does not print field values.

If the result is `thread_id_not_exposed`, the permitted AX identity fields do not
identify a project row. A title match cannot override that result. Do not use
this POC as proof that thread-ID selection works in Codex.

Diagnostics include per-field `hasValue`, `exactThreadID`, and
`containsThreadID` flags. The last flag is diagnostic only; substring matches
never authorize a press. `identityValuesRedacted` and `labelsRedacted` are true.
No transcript text or `AXValue` is read. Identity reads are restricted to the
requested project list and its row wrappers.

## Boundaries

The probe validates the PID, bundle ID, process start time, and exact window
before selection. It bounds tree size, depth, attribute reads, and wait time.
It rechecks the same AX element and native ID before one press. It never repeats
a press after a timeout. Errors report whether a press was sent.

macOS must authorize the process responsible for the probe. Permission checks
do not prompt or change permissions. The probe does not modify the Codex bundle,
pinning, grouping, Recents visibility, or sidebar settings. It does not expand,
scroll, or load more rows. Required rows must be exposed already.

Automated tests use only separate fixture apps and temporary profiles. The
available computer-use tool excludes Codex; this POC does not bypass that
restriction. Real Codex commands are run manually by the user. Reports retain
`realCodexTested: false` for all fixture runs.

References: [Chromium AXDOMIdentifier mapping](https://github.com/chromium/chromium/blob/main/ui/accessibility/platform/ax_platform_node_cocoa.mm),
[Apple AX action API](https://developer.apple.com/documentation/applicationservices/1462091-axuielementperformaction),
and [Chromium renderer activation](https://github.com/chromium/chromium/blob/main/chrome/browser/chrome_browser_application_mac.mm).

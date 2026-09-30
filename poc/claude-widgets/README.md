# Claude widget rendering POC

This POC tests display of Claude `show_widget` content. It does not change the
Mallard session viewer, transcript codec, sync format, or production policy.

## Display paths

- **SVG:** Parse a complete `widget_code` SVG. Add local styles for Claude's
  presentation classes. Display it through an SVG image data URL. Image mode
  disables scripts and external document resources. Mallard's current policy
  permits this path.
- **HTML and JavaScript:** Load a separate document in an iframe with
  `sandbox="allow-scripts"`. The frame has an opaque origin. Its response policy
  blocks network requests, nested frames, objects, forms, and base URLs. The
  frame can report display state through checked messages. It cannot request
  a Mallard command through those messages.

The loopback server is test equipment. It is not a proposed production server.
Only the POC enables its frame origin. The production app still uses
`frame-src 'none'`.

## Run

From the repository root:

```sh
node poc/claude-widgets/server.mjs
```

The server prints its loopback URL. Open that URL to test the browser viewer.
The default diagram is a synthetic fixture based on the supplied screenshot.

To test the native SVG path with the current app policy:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example claude_widgets_poc --locked
```

To test the native HTML frame, pass the URL printed by the server:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example claude_widgets_poc --locked -- \
  --server-url http://127.0.0.1:PORT --interactive
```

Select **Interactive** to test a counter. Select **Isolation tests** to test
script execution, parent access, storage, native API access, fetch requests,
popups, top navigation, and script blocking inside an SVG image.

Both runners accept `--widget-file /path/to/widget.svg`. The input must be a
regular UTF-8 file below 1 MiB. A saved transcript must not be committed as a
fixture. Use a temporary file for real widget code. Neither runner discovers
or reads a provider home.

The native runner also accepts `--example diagram|interactive|isolation` and
`--report-path /path/to/report.json`. Reports contain only test metadata.

## Verified on 2026-09-30

| Check | Browser | macOS Tauri WebView |
| --- | --- | --- |
| Saved SVG from the supplied diagram | Rendered | Rendered under the unchanged production CSP |
| Synthetic HTML and JavaScript counter | Real click changed 1 to 2 | Real click changed 1 to 2 |
| Isolation probes | 9 of 9 passed | 9 of 9 passed |
| Parent native command control | Not applicable | Succeeded |
| Widget requests to the local canary | 0 | 0 |

The server control route was reached once. The blocked-request result therefore
uses a reachable server. It is not caused by a missing server.

Native build and check passed with the repository's locked dependencies.
JavaScript syntax checks and Rust formatting checks passed.

Screenshots and test reports are in the ignored
`output/playwright/claude-widgets/` directory. Real widget source is held in a
temporary file outside the repository. The repository POC fixture is synthetic.

These checks do not prove Windows or Linux behavior, full Claude host API
compatibility, external library support, or a Push/Pull round trip.

## Production scope

SVG display can use the existing tool arguments and CSP. Render only a complete
SVG from the expected widget tool. Keep the source view available. Reject
truncated or malformed input.

Interactive display needs a viewer origin and a command boundary that are
verified on each supported native platform. Tauri documents a Linux limitation:
it cannot distinguish iframe requests from the window's requests. Do not treat
a sandbox attribute as complete proof of native command isolation.

Remote dependencies and Claude host APIs are outside this POC. Widgets that
use these facilities need separate contracts. Interaction state remains local
to the preview and is reset when the frame is replaced.

References:

- [MDN: SVG as an image](https://developer.mozilla.org/en-US/docs/Web/SVG/Guides/SVG_as_an_image)
- [MDN: iframe sandbox](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe#sandbox)
- [Tauri: capabilities and platform limits](https://v2.tauri.app/security/capabilities/)

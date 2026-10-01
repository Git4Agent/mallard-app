# Session diagram validation

Date: 2026-09-30. Scope: production local and Cloud source. No commit or deployment.

## Automated checks

| Check | Result |
| --- | --- |
| Local `npm run build` | Passed |
| Local frontend integration | 485 passed |
| Rust library suite | 501 passed, 14 ignored |
| Isolated full-app fixture preparation | 1 passed, explicitly run with `--ignored` |
| Release configuration | 9 passed |
| Cloud typecheck and web build | Passed |
| Cloud web suite | 115 passed |
| Cloud Worker suite | 186 passed |
| Local and Cloud `git diff --check` | Passed |

The ignored native fixture is separate from the default Rust test count.
The suites include existing work in these dirty worktrees.

The portable test uses actual local-folder capture and reviewed Push. It reads
the verified published object, builds the Cloud preview, restores into a second
fixture home, and queries the receiving desktop's Replies timeline. Mermaid
text and SVG source match exactly at each step.

## Browser component checks

Checked 14 input cases on each surface: complete source, two diagrams,
malformed source, no nodes, incomplete fence, oversized source, ordinary code,
directives, frontmatter, external images, hostile labels, node limit, edge
limit, and quoted fences. Valid cases rendered. Rejected cases retained Source.
The click directive case was rejected because its SVG contained a link element.
The canary endpoint received zero requests.

Checked both light and dark page themes for the widget and flowchart on each
surface. All eight images loaded. Both builds emitted identical parser HTML,
script hash metadata, and license assets. The separate protocol test rejects
wrong windows, origins, tokens, IDs, unsafe flags, and timed-out replies.

Records are under ignored `output/mermaid-flowcharts/`:

- `production-browser-checks.json`
- `production-theme-checks.json`
- `local-widget-light.png`, `cloud-widget-light.png`
- `local-diagram-light.png`, `cloud-diagram-light.png`

The widget fixture reproduces the screenshot layout with Claude host variables.
The screenshot did not supply its original SVG. Fixed author colors are not
rewritten by theme adaptation.

## Complete desktop app

Built the standard Mallard binary with `tauri/custom-protocol`, embedded frontend
assets, and all production commands. Used a temporary home, provider homes,
project, and local store. The session detail showed the recorded widget and the
six-node flowchart. Both changed on page theme changes. Native Source showed
the complete flowchart; Copy source reported `Copied.`.

The trusted parser reports the Tauri globals absent. The parent accepts an SVG
only when that report is false, strict mode is true, and HTML labels are false.
The successful native image therefore confirms those checks on this build.
This check is not an independent raw IPC penetration test.

The first macOS launch used the normal home because `LSEnvironment` was ignored.
It was stopped before test UI actions. The verified run used an executable
launcher. Its `launch-proof.json` reported all three homes inside the fixture.
The app showed only `Session diagrams QA` before the session was opened.

Screenshots:

- `full-native-widget-light.png`
- `full-native-flowchart-light.png`
- `full-native-flowchart-dark.png`

## Complete Cloud page

Served the production Cloud build with a read-only local fixture API. The view
and manifest came from the verified fixture Push. The session page showed the
widget and flowchart with the normal timeline and filters. Both page themes
worked. Source showed the full source; Copy source reported `Copied.`.

Screenshots: `full-cloud-light.png`, `full-cloud-dark.png`, `full-cloud-source.png`.
The Worker tests separately checked the actual ASSETS response, its canonical
redirect, exact parser script hashes, the SAMEORIGIN header, and the unchanged
policy and DENY header on ordinary pages.

## Proof limits

Deployed Cloud, Windows/Linux WebViews, and the oldest supported macOS WebView
were not tested. The OS minimum was not changed. Mermaid Tiny uses browser
APIs such as `Array.at` and `Object.hasOwn`; the oldest WebView needs separate
release qualification. A parser failure keeps Source available.

The iframe blocks access and resource loading under its policy. It is not a
separate process and does not provide a hard CPU limit. Interactive HTML or
JavaScript widgets are outside this SVG and flowchart change.

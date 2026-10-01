# Session diagram verification

This harness now uses the production Mermaid and SVG widget components. It has
no Markdown injection and no separate test renderer. All session inputs are
synthetic. `supplied-flowchart.mmd` contains the user's six-node example.

## Build the two surfaces

Run `npm run build` in mallard-app. Run `npm run build:web` in the sibling
`mallardInternal/cloudflare/managed-cloud` package. Then run:

```sh
node poc/mermaid-flowcharts/build.mjs
node poc/mermaid-flowcharts/server.mjs
```

Open `http://127.0.0.1:64482/desktop.html` or `/cloud.html`. The harness checks
that both builds emit the same parser asset and script hashes. Change Example
and Theme to inspect the production components. The `widget` case reproduces
the screenshot layout with Claude host variables and semantic color classes.
It is a synthetic widget, not an extraction of the screenshot's original SVG.
Author-specified fixed SVG colors remain part of the recorded source.

## Prepare the full desktop fixture

After the builds, run:

```sh
node poc/mermaid-flowcharts/prepare-native.mjs
```

Open the app path saved in `output/mermaid-flowcharts/full-native-app-path.txt`.
The launcher sets HOME, CODEX_HOME, and CLAUDE_CONFIG_DIR in the child process.
Do not use macOS `LSEnvironment` for isolation; that setting was ignored in the
first test launch. Check `launch-proof.json` in the fixture root and confirm
that the app contains only `Session diagrams QA` before UI actions.

The complete app registers its production commands. The fixture contains one
Claude session with the widget and Mermaid message. The fixture has no recorded
time, so use **Load earlier history** to find the session. Open its details.
Check both page themes, scroll to the flowchart, and check Source and Copy source.

## Run the full Cloud page with the stored fixture preview

With the fixture prepared, run:

```sh
node poc/mermaid-flowcharts/cloud-session-server.mjs
```

Open the local session URL printed by the server. It serves the production
Cloud build and a read-only fixture API. The preview and manifest come from the
fixture's verified local-folder Push. This does not test the deployed service.

## Automated checks

Run the repository's frontend integration and Rust library suites. Run Cloud
web and Worker suites from the Cloud package. The frontend tests cover fence
completion and protocol validation. The Rust round trip checks exact Mermaid
text and widget source after Push, preview generation, Pull, and local reading.

See [VERIFICATION.md](VERIFICATION.md) for results and proof limits. Generated
assets, fixture descriptors, and screenshots stay under ignored `output/` or
the temporary fixture root.

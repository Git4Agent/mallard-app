# Mermaid flowcharts in session detail

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: implemented in the local workspace on 2026-09-30. Cloud is not deployed.

## Behavior

Render complete `mermaid` code fences that contain `flowchart` or `graph`.
Use the same behavior for Codex and Claude session messages. Keep other fences
as code. Keep Source and Copy source for valid and rejected input.

The Markdown tokenizer marks complete fences. Quotes, lists, tilde fences,
CRLF text, and incomplete messages have explicit test coverage. A fence labeled
`css` remains code, even when its body contains Mermaid source.

Both the flowchart and Claude host widget colors follow the resolved page theme.
A theme change updates the image without changing the saved source. A widget's
explicit author colors remain part of that source.

## Implementation

| Area | Source |
| --- | --- |
| Desktop Markdown hook | `src/components/project-sync/session-detail/SessionDetailEvent.tsx` |
| Cloud session hook | Sibling `web/src/session/SessionTimeline.tsx` and `web/src/components/Markdown.tsx` |
| Fence metadata, client, image, and parser | `src/components/mermaid/` and Cloud `web/src/components/mermaid/` |
| Page theme observer | `src/components/sessionTheme.ts` and its Cloud copy |
| Bundled asset and script hashes | `scripts/mermaidRendererPlugin.ts` and Cloud `web/mermaidRendererPlugin.ts` |
| Cloud parser response policy | Sibling `src/mermaid_assets.ts` and `src/index.ts` |
| Portable source proof | `src-tauri/src/project_sync/claude_widget_tests.rs` |

The Cloud package is `mallardInternal/cloudflare/managed-cloud` in the sibling
repository. Cloud enables flowcharts from session callers only.

## Parser boundary

Pin `@mermaid-js/tiny` 11.15.0. Bundle its code and license. Load it only when a
flowchart approaches the viewport. Use one opaque iframe with `allow-scripts`
and no same-origin permission. Its policy permits only the two exact bundled
script hashes and blocks network access, child frames, forms, objects, and fonts.

Use strict mode, SVG text labels, and locked host settings. Authenticate replies
by window, opaque origin, token, and request ID. Require safe parser flags.
Reject active SVG elements, links, image resources, and event attributes.
Display SVG with an image element. Do not insert it into the session DOM.

Desktop changes `frame-src 'none'` to `frame-src 'self'`. Cloud permits framing
only for the parser HTML response. Other Cloud pages retain their existing
policy and DENY header. The canonical asset redirect is covered by a Worker test.

Limits are 32 KiB of UTF-8 source, 128 nodes, 128 edges, 1 MiB of SVG, and 16
queued or cached results. Cache by source and theme. A 10-second reply timeout
reports failure. The iframe is not a separate process or a hard CPU limit.

## Portable source

Keep Mermaid in the transcript's message text. Keep widget SVG in its recorded
tool arguments. Existing bundle and preview fields carry both. No new schema,
object key, command, or migration is needed. Existing complete Mermaid text in
stored Cloud views can render after the new viewer is deployed. Do not rewrite
immutable historical previews.

The Rust test checks actual capture, verified publication, Cloud view building,
restore into a second fixture home, and the receiving desktop's timeline.

## Verification and release limits

See [the verification record](../poc/mermaid-flowcharts/VERIFICATION.md) and
[the fixture runner](../poc/mermaid-flowcharts/README.md).

The complete Tauri app and full Cloud page were checked on this Mac. The Cloud
page used a local fixture API. Deployed Cloud, Windows/Linux WebViews, and the
oldest supported macOS WebView remain unverified. The product OS minimum was
not changed. Unsupported parsers retain the source view.

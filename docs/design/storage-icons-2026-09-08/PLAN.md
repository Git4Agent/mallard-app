# Storage icon design plan

Status: plan and mock only. Application code is unchanged by this study.
Date: 2026-09-08.

Recommend C, Provider marks, for lists with different storage providers.
Keep the pixel style for Mallard navigation and local folders. Give
Cloudflare R2 and Dropbox distinct provider marks. Keep the provider name
beside each mark. This recommendation is a design judgment. It is not a
requirement from a platform standard.

## Current problems

- `Icons.tsx` calls the archive-box shape `drive`. It represents both the
  storage section and local storage. These roles need separate symbols.
- R2 and Dropbox both return the `cloud` presentation hint. They have the
  same icon in the sidebar and storage menus.
- Storage surfaces use several icon sizes, including 15, 16, 18, and 19 px.
  The current pixel renderer uses `crispEdges`. Some sizes produce uneven
  pixel widths. The component documents 12 and 24 px for 2× displays.

## Choices

| Choice | Storage section | Local folder | R2 | Dropbox | Trade-off |
| --- | --- | --- | --- | --- | --- |
| A. Pixel system | Pixel server stack | Pixel folder | Pixel cloud | Pixel cloud | Closest to the current family. Provider names remain necessary. |
| B. Soft outline | Outline hard drive | Outline folder | Outline cloud | Outline cloud | Smooth curves at small sizes. Differs from other Mallard icons. |
| C. Provider marks | Pixel server stack | Pixel folder | Cloudflare mark | Dropbox mark | Distinct provider shapes. Requires approved brand variants for application use. |

The Cloudflare mark identifies the provider. It is not an R2 product icon.
The server stack represents the storage section. It must not imply that
Mallard manages servers. Keep the visible “Storages” label.

## Industry guidance used

These sources give design guidance. They do not define one mandatory SVG
symbol for a storage feature.

- **Apple HIG:** use familiar concepts. Keep size, detail, weight, and
  perspective consistent. Adjust dimensions for visual balance.
  [Icons](https://developer.apple.com/design/human-interface-guidelines/icons).
- **Microsoft:** use clear icons that work at small sizes. Match the
  surrounding type and design language. Avoid mixed meanings.
  [Icons in Windows apps](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/icons).
- **Material Symbols:** weight, fill, and optical size are separate design
  controls. Judge icons at their intended display size. A 24-unit source
  grid is not a rule that every control must be 24 px.
  [Material Symbols guide](https://developers.google.com/fonts/docs/material_symbols).
- **WCAG 2.2 SC 1.4.11:** meaningful graphical information needs at least
  3:1 contrast against adjacent colors. A decorative icon with equivalent
  visible text does not always need this. Use 3:1 as the icon design target
  in both themes.
  [Non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
- **Pixelarticons:** use a 24×24 grid and whole coordinates. Render at
  suitable scale factors for crisp edges.
  [Source and design rules](https://github.com/halfmage/pixelarticons).

## SVG rules for the selected set

1. Use saved SVG geometry from a known icon source. Keep one meaning per
   symbol. Use folder for a local directory and cloud for a general remote
   location. The main storage icon must cover local and remote storage.
2. Keep status separate from identity. A provider icon must not contain
   Push, Pull, sync, success, or error symbols.
3. Keep a 24-unit source grid. Use `currentColor` for general UI symbols.
   Use `crispEdges` only for pixel geometry. Allow antialiasing for curved
   outlines and brand marks.
4. Compare 12, 16, 20, and 24 px in normal and active rows. For pixel icons,
   prefer 12 or 24 CSS px on 2× displays. Verify 1× displays separately.
   Do not force all icon families into the pixel renderer.
5. Keep labels visible. Hide decorative SVGs from assistive technology.
   Give any icon-only control an accessible name and a visible focus state.
6. For C, use official approved logo variants before application use.
   Do not convert brand geometry to pixel art. The mock uses unmodified
   Simple Icons paths with theme color to compare shape and weight.
   It does not establish approval for that color treatment.
   [Dropbox developer guidance](https://www.dropbox.com/developers/reference/branding-guide),
   [Cloudflare brand guidance](https://www.cloudflare.com/trademark/).

## Implementation plan after a design is selected

1. Finalize the four role mappings in the table. Keep the frontend glyph
   vocabulary separate from provider setup behavior.
2. Add the selected glyphs to `src/components/Icons.tsx`. Keep the general
   storage symbol separate from the archive symbol. Do not change unrelated
   uses of `drive` without checking their meaning.
3. For C, update the provider presentation hints in the existing local, R2,
   and Dropbox descriptors. Extend the frontend hint vocabulary for the
   selected marks. Keep provider identity in the existing descriptors;
   do not add a second provider registry in the frontend.
4. Apply the mapping to `ProjectSidebar.tsx`, `StorageIndexPage.tsx`,
   `StorageSettings.tsx`, and the storage paths in
   `ProjectLinksWorkspace.tsx` and `StorageCatalogPage.tsx`.
   Keep `SyncPathBar.tsx` free of provider logic.
5. Remove obsolete storage icon paths where no current use remains. Update
   third-party notices for any new shipped assets. Do not add a package
   only to display a few fixed SVGs.
6. Run `npm run build` and `npm run test:frontend-integration`. If provider
   descriptors change, run the relevant Rust tests and the repository Rust
   handoff check, `cargo test --manifest-path src-tauri/Cargo.toml --lib`.
   Verify the complete Tauri app in both themes. Capture the
   sidebar, storage menu, settings header, and Library storage path.

No bundle schema, storage schema, restore behavior, or credential behavior
change is planned.

## Mock files

- `index.html`: interactive comparison. Theme, size, and choice controls
  affect this page only.
- `comparison-dark.svg` and `comparison-light.svg`: standalone vector boards.
- `comparison-dark.png` and `comparison-light.png`: rendered previews.
- `assets/`: source SVGs and licenses.
- `sources.json`: source URLs and SHA-256 digests.
- `build_mock.py` and `template.html`: reproducible mock source.

Use the local Vite server to open
`/docs/design/storage-icons-2026-09-08/index.html`.

## Mock verification

- Opened the mock in the Codex browser. The page has three choices.
- Checked the light theme, the 24 px control, and choice selection.
- Rendered and inspected both SVG comparison boards.
- Application build and integration tests were not run. This study changes
  only design artifacts.

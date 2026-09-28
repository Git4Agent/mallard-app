# Application icons

`macos.png` is the source for the macOS icon. It uses the selected Porcelain
design: a dark duck on a light background. `icon.icns` is its packaged form.
The source fills the square canvas. macOS Tahoe applies the rounded outline.
Keep the source fully opaque. Transparent margins caused an extra gray frame
in the native 32-pixel icon check.

The imagegen source prompt preserves the selected smooth duck, right-facing
bill, split tail, white neck band, and subtle porcelain shading. It requests
an opaque square with no outer margin, rounded mask, border, or cast shadow.

Regenerate only the macOS bundle icon from the repository root:

```sh
mallard_icon_output=$(mktemp -d)
node node_modules/@tauri-apps/cli/tauri.js icon src-tauri/icons/macos.png --output "$mallard_icon_output"
cp "$mallard_icon_output/icon.icns" src-tauri/icons/icon.icns
```

`macos-dev.png` is the rounded source for macOS development and Windows.
It is a 1024-pixel export of the packaged icon as rendered by macOS 26.6.2.
It has rounded edges and transparent corners. `macos-dev.icns` is its macOS
development form; `icon.ico` is its Windows form. Both include the rounded
outline in the artwork.

Regenerate both forms after updating the rounded PNG source:

```sh
mallard_rounded_icon_output=$(mktemp -d)
node node_modules/@tauri-apps/cli/tauri.js icon src-tauri/icons/macos-dev.png --output "$mallard_rounded_icon_output"
cp "$mallard_rounded_icon_output/icon.icns" src-tauri/icons/macos-dev.icns
cp "$mallard_rounded_icon_output/icon.ico" src-tauri/icons/icon.ico
```

Use `npm run tauri dev`. On macOS, the launch script selects
`macos-dev.icns` through a development-only configuration override. macOS
build and bundle commands keep `icon.icns`. Windows development and packaged
builds select `icon.ico`. The other PNG files serve the other platforms.

The Windows ICO contains 16, 24, 32, 48, 64, and 256-pixel images with
transparency. Keep the 32-pixel image first for Tauri's development display.
Check the taskbar, desktop shortcut, and window icon
on Windows after an icon change.

After an icon change, check both `npm run tauri dev` and a packaged `.app`
in the Dock. Restart the development command after changing its icon setup.

Apple's [app icon guidance](https://developer.apple.com/design/human-interface-guidelines/app-icons)
describes the square source and system-applied rounded mask. Native appearance
was checked on macOS 26.6.2. Earlier macOS versions were not checked.

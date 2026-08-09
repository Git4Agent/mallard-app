# Releasing Mallard

Mallard is distributed through GitHub Releases as an Apple Silicon DMG and a
Windows x64 NSIS installer. A pushed `v*` tag starts
`.github/workflows/release.yml`. The Apple Silicon job builds
`aarch64-apple-darwin` and creates the draft release first. Only after that job
succeeds does the Windows job build `x86_64-pc-windows-msvc`, verify a temporary
installation, and upload its installer to the same draft.

## Signing and platform approval

Mallard does not currently have an Apple Developer Program certificate. Its
macOS bundle therefore uses Tauri's explicit ad-hoc identity (`-`). This gives
the Apple Silicon application a structurally valid code signature, but it is
not a Developer ID signature and the application is not notarized.

The release build validates the application before upload with:

```bash
codesign --verify --deep --strict --verbose=4 /path/to/Mallard.app
```

If that validation fails, the GitHub release draft is not created or updated.
Ad-hoc signing does not prevent Gatekeeper warnings; testers must approve the
application manually.

The Windows NSIS installer is currently unsigned. Windows may show a Microsoft
Defender SmartScreen warning, and testers must review and approve that prompt
manually. The workflow does not produce automatic-update metadata and does not
claim an Authenticode or Developer ID signature.

## Install the DMG

1. Download the Apple Silicon DMG from the GitHub Release.
2. Open the DMG and drag Mallard to Applications.
3. Attempt to open Mallard from Applications.
4. If macOS blocks it, open System Settings, select Privacy & Security, find the
   message about Mallard, and choose Open Anyway.
5. Confirm the final Open prompt.

Do not tell users that this build is notarized or signed by an identified Apple
developer.

## Install on Windows

1. Download the Windows x64 NSIS `.exe` from the same GitHub Release.
2. Run the installer and review any SmartScreen warning before choosing to
   continue.
3. Launch Mallard from the Start menu and confirm the storage workspace opens.

The installer places `mallard-cli.exe` inside Mallard's installation directory
for the app's agent-assisted workflow. It does not add the CLI to `PATH`, edit a
PowerShell profile, or modify shell startup configuration. Mallard passes an
absolute packaged path to an agent instead.

## Prepare version 0.1.2

The version must match in these files:

- `package.json`
- `package-lock.json`
- `Cargo.toml` (`[workspace.package]`; `src-tauri/Cargo.toml` inherits it)
- `src-tauri/tauri.conf.json`

Run the release checks before creating a tag:

```bash
npm run check:release-version -- v0.1.2
npm run test:integration
npm run build
cargo check --workspace
```

## Build and verify locally on macOS

Build the same Apple Silicon DMG locally before creating the release tag:

```bash
MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build -- \
  --target aarch64-apple-darwin \
  --bundles dmg
```

Inspect the generated DMG and application:

```bash
hdiutil verify \
  target/aarch64-apple-darwin/release/bundle/dmg/Mallard_0.1.2_aarch64.dmg

file \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app/Contents/MacOS/mallard

codesign --verify --deep --strict --verbose=4 \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app

codesign -dvvv \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app
```

The DMG verification must succeed, `file` must report an `arm64` executable,
and `codesign` must report a valid ad-hoc signature. The automated verifier also
runs the packaged `mallard-cli --help` from the mounted image and from a
temporary installed location. Install this local DMG and complete the manual
Gatekeeper approval flow before creating the tag.

## Build and verify locally on Windows

Run these commands from PowerShell on an x64 Windows machine:

```powershell
npm run tauri build -- `
  --target x86_64-pc-windows-msvc `
  --bundles nsis

./scripts/verify-windows-bundle.ps1 `
  -BundleDirectory "target/x86_64-pc-windows-msvc/release/bundle/nsis" `
  -TemporaryRoot $env:TEMP
```

The verifier requires exactly one generated NSIS installer, installs it
silently into a new bounded temporary directory, requires the installed
`mallard-cli.exe`, and checks that its `--help` command exits successfully with
the expected command surface. Cleanup runs the installed Tauri `uninstall.exe`
silently from a copy outside the install directory, waits for it with a bounded
timeout, and requires the temporary installation and its current-user uninstall
registry entry to be gone. The verification install passes Tauri's `/NS` flag,
so it creates no Start menu or desktop shortcuts. It also verifies that process,
user, and machine `PATH` values did not change, then removes only its own
verification directory.
The GitHub-hosted Windows machine is ephemeral if a Windows platform failure
prevents the normal cleanup. The installer itself remains in the target bundle
directory for manual inspection and upload.

## Create the GitHub draft

After the local installation succeeds, commit the release changes and push
them to the default branch. Then create and push the matching tag:

```bash
git tag v0.1.2
git push origin v0.1.2
```

Wait for both release jobs. The Windows job is deliberately ordered after the
Apple Silicon job so the draft exists before the second artifact is uploaded.
Inspect the draft release, download and install the DMG on an Apple Silicon Mac,
and install the NSIS artifact on Windows x64. Publish the draft only after both
artifacts pass the same checks. Do not replace an artifact for an existing
version; fix the problem, increment the version, and create a new release.

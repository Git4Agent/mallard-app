# Releasing Mallard

Mallard is distributed as one Apple Silicon DMG through GitHub Releases. A
pushed `v*` tag starts `.github/workflows/release.yml`; the workflow builds for
`aarch64-apple-darwin` and creates a draft release for manual inspection.

## Signing and macOS approval

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

## Install the DMG

1. Download the Apple Silicon DMG from the GitHub Release.
2. Open the DMG and drag Mallard to Applications.
3. Attempt to open Mallard from Applications.
4. If macOS blocks it, open System Settings, select Privacy & Security, find the
   message about Mallard, and choose Open Anyway.
5. Confirm the final Open prompt.

Do not tell users that this build is notarized or signed by an identified Apple
developer.

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

## Build and verify locally

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
and `codesign` must report a valid ad-hoc signature. Install this local DMG and
complete the manual Gatekeeper approval flow before creating the tag.

## Create the GitHub draft

After the local installation succeeds, commit the release changes and push
them to the default branch. Then create and push the matching tag:

```bash
git tag v0.1.2
git push origin v0.1.2
```

Wait for the single Apple Silicon job. Inspect the draft release, download and
install its DMG on an Apple Silicon Mac, and publish the draft only after that
artifact passes the same checks. Do not replace an artifact for an existing
version; fix the problem, increment the version, and create a new release.

# Releasing Mallard

Mallard is distributed through GitHub Releases as Developer ID signed and
notarized Apple Silicon and Intel DMGs and an unsigned Windows x64 NSIS
installer. A pushed `v*` tag starts `.github/workflows/release.yml`. The Apple
Silicon job builds `aarch64-apple-darwin`, signs, notarizes, verifies, and
creates the draft release first. Only after that job succeeds do the other two
jobs run in parallel: the Intel job cross-compiles `x86_64-apple-darwin` on an
Apple Silicon runner (Rosetta 2 lets the verifier run the packaged x86_64 CLI)
and the Windows job builds `x86_64-pc-windows-msvc` and verifies a temporary
installation. Both upload to the same draft only while it is still a draft.

The same workflow can be started by hand from the Actions tab (**Run
workflow**, `workflow_dispatch`). That is a dry run: every build, signing,
notarization, and verification step runs, but the artifacts are attached to
the workflow run instead of a release, and no tag or draft is touched. Use it
to debug signing without consuming a version.

## Signing and platform approval

### macOS

The release build signs `Mallard.app` and the packaged `mallard-cli` with a
`Developer ID Application` certificate, enables the hardened runtime, submits
the application to Apple's notary service, and staples the ticket. The build
wrapper then notarizes and staples the DMG itself. Gatekeeper opens the result
without any security override.

The hardened runtime blocks Apple Events unless the application carries the
`com.apple.security.automation.apple-events` entitlement
(`src-tauri/Entitlements.plist`) and declares `NSAppleEventsUsageDescription`
(`src-tauri/Info.plist`). Mallard needs both for **Open in Terminal** and the
Codex App refresh. Exercise those actions on every trusted build; a missing
entitlement fails silently with error -1743 rather than at build time.

`src-tauri/tauri.conf.json` keeps `signingIdentity: "-"` so local builds
without the certificate remain structurally valid ad-hoc builds. The Tauri CLI
reads `APPLE_SIGNING_IDENTITY` from the environment before the configuration
value, so trusted builds override it without editing the file.

Trusted builds require these GitHub Actions repository secrets:

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` (certificate and private key) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Name> (<TEAMID>)` |
| `APPLE_TEAM_ID` | the Apple Developer Team ID |
| `APPLE_API_KEY` | App Store Connect API Key ID |
| `APPLE_API_ISSUER` | App Store Connect API Issuer ID |
| `APPLE_API_KEY_CONTENT` | full text of `AuthKey_<KEYID>.p8` |

The workflow writes the API key to `$RUNNER_TEMP/AuthKey.p8` and passes its
path as `APPLE_API_KEY_PATH`. Nothing in the repository prints a secret. How
to obtain each value is in
[`PLAN_TRUSTED_DESKTOP_RELEASES.md`](./PLAN_TRUSTED_DESKTOP_RELEASES.md)
§2.1.

Two environment variables control verification in `scripts/tauri-build.mjs`:

- `MALLARD_VERIFY_MACOS_BUNDLE=1` runs `scripts/verify-macos-bundle.mjs`
  against the produced DMG.
- `MALLARD_REQUIRE_TRUSTED_MACOS=1` fails the build unless the App Store
  Connect credentials are present, and makes the verifier require a Developer
  ID Application signature with the hardened runtime on the app and the CLI,
  the Apple Events entitlement, `spctl --assess --type execute` reporting
  `source=Notarized Developer ID`, `spctl --assess --type open` accepting the
  DMG, and `xcrun stapler validate` succeeding on both. The release workflow
  sets both.

### Windows

The Windows NSIS installer is currently unsigned. Windows shows a Microsoft
Defender SmartScreen warning ("Windows protected your PC"); testers choose
**More info → Run anyway**. The workflow does not produce automatic-update
metadata and does not claim an Authenticode signature.

## Install the DMG

1. Download the DMG for your Mac from the GitHub Release: `_aarch64.dmg` for
   Apple Silicon, `_x64.dmg` for Intel. (Apple menu → About This Mac shows
   which chip you have.)
2. Open the DMG and drag Mallard to Applications.
3. Open Mallard from Applications. No security override is required.
4. The first **Open in Terminal** or Codex App refresh shows a macOS
   Automation prompt for Mallard; allow it.

## Install on Windows

1. Download the Windows x64 NSIS `.exe` from the same GitHub Release.
2. Run the installer and review the SmartScreen warning before choosing to
   continue.
3. Launch Mallard from the Start menu and confirm the storage workspace opens.

The installer places `mallard-cli.exe` inside Mallard's installation directory
for the app's agent-assisted workflow. It does not add the CLI to `PATH`, edit a
PowerShell profile, or modify shell startup configuration. Mallard passes an
absolute packaged path to an agent instead.

## Prepare version 0.1.3

The version must match in these files:

- `package.json`
- `package-lock.json`
- `Cargo.toml` (`[workspace.package]`; `src-tauri/Cargo.toml` inherits it)
- `src-tauri/tauri.conf.json`
- `tests/release-config.test.mjs`

Run the release checks before creating a tag:

```bash
npm run check:release-version -- v0.1.3
npm run test:integration
npm run build
cargo check --workspace
```

## Build and verify locally on macOS

A trusted local build needs the Developer ID certificate in the login keychain
(`security find-identity -v -p codesigning` lists it) and the App Store Connect
API key on disk. Build the same Apple Silicon DMG the workflow produces:

```bash
APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TEAMID>)" \
APPLE_TEAM_ID=<TEAMID> \
APPLE_API_KEY=<KEYID> \
APPLE_API_ISSUER=<ISSUER> \
APPLE_API_KEY_PATH=~/.private_keys/AuthKey_<KEYID>.p8 \
MALLARD_VERIFY_MACOS_BUNDLE=1 \
MALLARD_REQUIRE_TRUSTED_MACOS=1 \
npm run tauri build -- --target aarch64-apple-darwin --bundles dmg
```

Without the `APPLE_*` variables and without `MALLARD_REQUIRE_TRUSTED_MACOS`,
the same command produces an ad-hoc signed DMG for local testing only; never
upload one.

The Intel DMG builds the same way from an Apple Silicon Mac after
`rustup target add x86_64-apple-darwin`, replacing the target with
`--target x86_64-apple-darwin`. The verifier then runs the packaged x86_64
`mallard-cli` under Rosetta 2 and checks with `lipo -archs` that both the app
and the CLI contain exactly `x86_64`. Output lands under
`target/x86_64-apple-darwin/release/bundle/dmg/Mallard_0.1.3_x64.dmg`.

Inspect the generated DMG and application by hand if needed:

```bash
hdiutil verify \
  target/aarch64-apple-darwin/release/bundle/dmg/Mallard_0.1.3_aarch64.dmg

spctl --assess --type open --context context:primary-signature --verbose=4 \
  target/aarch64-apple-darwin/release/bundle/dmg/Mallard_0.1.3_aarch64.dmg

codesign -dvv \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app

spctl --assess --type execute --verbose=4 \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app

xcrun stapler validate \
  target/aarch64-apple-darwin/release/bundle/macos/Mallard.app
```

`codesign` must report `Authority=Developer ID Application: …` and
`flags=0x10000(runtime)`, and both `spctl` assessments must print `accepted`.
The automated verifier also runs the packaged `mallard-cli --help` from the
mounted image and from a temporary installed location.

Then test Gatekeeper the way a user meets it: copy the DMG to another user
account or a clean Mac through a browser download (or add the quarantine flag
by hand with `xattr -w com.apple.quarantine "0083;$(printf '%x' $(date +%s));Safari;" <dmg>`),
open it, install, and launch. No "unidentified developer" dialog may appear.
Finally run **Open in Terminal** and a Codex App refresh from the installed
build and confirm the Automation prompt appears once and the action succeeds.

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

Run the dry-run workflow first (Actions → **Release macOS and Windows apps** →
**Run workflow**) and confirm both jobs pass with the trusted verifier. Then
commit the release changes, push them to the default branch, and create and
push the matching tag:

```bash
git tag v0.1.3
git push origin v0.1.3
```

Wait for all three release jobs. The Intel and Windows jobs are deliberately
ordered after the Apple Silicon job so the draft exists before their artifacts
are uploaded. Inspect the draft release: it must hold exactly
`Mallard_<version>_aarch64.dmg`, `Mallard_<version>_x64.dmg`, and
`Mallard_<version>_x64-setup.exe`. Download and install the DMGs on an Apple
Silicon Mac and an Intel Mac, and install the NSIS artifact on Windows x64.
Publish the draft only after every artifact passes the same checks. Do not
replace an artifact for an existing version; fix the problem, increment the
version, and create a new release.

## Secret rotation

Developer ID certificates are valid for five years. If the certificate or the
API key is compromised, revoke it in the developer portal or App Store Connect,
create a replacement, and update the GitHub secrets. Already-notarized releases
stay valid.

# Plan: Trusted Desktop Releases — Developer ID macOS, Intel, and Windows

Status: Phase 1 §2.1 (Apple credentials and GitHub secrets) done 2026-09-20;
§2.2 repository changes implemented on `release-package-enhancement`.
Phase 2 pipeline changes (Intel job, architecture check, Windows dry run)
implemented on the same branch on 2026-09-20; `cargo check` for
`x86_64-apple-darwin` passes locally and a static audit found every Unix-only
API behind a `cfg` gate, but the Windows build has still never run. Awaiting
the dry-run workflow, then the v0.1.3 tag (§2.3, §3). Phase 3 implemented on
`app-updates` (2026-09-20) as designed in
[`DESIGN_APP_UPDATES.md`](./DESIGN_APP_UPDATES.md), which supersedes §4 below;
the updater keypair (§4.1) remains a manual step before the v0.1.3 tag.
Date: 2026-09-20
Application: Mallard (Tauri 2.11, `com.hequ.agent-sync`)
Supersedes: the distribution and signing sections of
[`DESKTOP_DISTRIBUTION_DESIGN.md`](./DESKTOP_DISTRIBUTION_DESIGN.md)

Decisions already taken for this plan:

- macOS is distributed as a **Developer ID signed and notarized DMG** from
  GitHub Releases. The Mac App Store is out of scope: it requires App Sandbox,
  which would block provider-home access, spawning `codex`/`claude`, Apple
  Events, and symbolic links that Mallard depends on.
- Notarization authenticates with an **App Store Connect API key**, not an
  Apple ID password.
- Intel macOS ships as a **separate `x64` DMG**, not a universal bundle.
- Windows ships as an **unsigned** NSIS installer for now. Authenticode
  signing is documented as a follow-up, not implemented.

## 1. Current state

The release pipeline is `.github/workflows/release.yml`, triggered by a pushed
`v*` tag, wrapped by `scripts/tauri-build.mjs` (`npm run tauri`), and pinned by
`tests/release-config.test.mjs`.

| Area | State | Evidence |
|---|---|---|
| Apple Silicon DMG | Works. Ad-hoc signed (`bundle.macOS.signingIdentity: "-"`), unnotarized, verified by `scripts/verify-macos-bundle.mjs`, uploaded to a draft release. | v0.1.2 draft holds `Mallard_0.1.2_aarch64.dmg`; run `29892122184` succeeded with one job. |
| Windows x64 NSIS | Job exists but has **never run**. Unsigned. Whether the workspace compiles for `x86_64-pc-windows-msvc` is unverified. | `publish_windows` and `scripts/verify-windows-bundle.ps1` were added in `96e1e41`, after the v0.1.2 tag. |
| macOS Intel DMG | Removed. | `8c6fc71` had a three-target matrix with `x86_64-apple-darwin`; `f17cef1` dropped Apple signing because no certificate existed. `scripts/tauri-build.mjs` still names `x64` DMGs. |
| Release tests | Pin the ad-hoc state: `signingIdentity === "-"`, forbid `APPLE_*` in the workflow, forbid `x86_64-apple-darwin`. | `tests/release-config.test.mjs`, test "desktop release builds verified Apple Silicon and Windows artifacts". |
| Automatic updates | Deliberately absent and enforced by test. `DESKTOP_DISTRIBUTION_DESIGN.md` §9 claiming an implemented updater is stale. | `tests/release-config.test.mjs`, test "application contains no automatic updater". |
| Versions | 0.1.2 everywhere; the v0.1.2 draft is already consumed, so the first trusted release is **0.1.3**. | `package.json`, `Cargo.toml`, `src-tauri/tauri.conf.json`. |

### 1.1 Notarization blocker found in code

Mallard sends Apple Events through `/usr/bin/osascript`:

- `crates/mallard-infrastructure/src/providers/launch.rs` —
  `launch_terminal_command` tells Terminal to run a session command.
- `crates/mallard-infrastructure/src/codex_app_refresh.rs` —
  `mac_app_is_running` and `request_mac_app_quit` query and quit the Codex App.

Notarization requires the hardened runtime (Tauri enables it by default). Under
the hardened runtime, macOS refuses Apple Events from the app and its child
processes unless the app carries the
`com.apple.security.automation.apple-events` entitlement, and the TCC prompt
needs `NSAppleEventsUsageDescription` in `Info.plist`. Without both, the
notarized build fails those actions with error -1743 while the ad-hoc build
keeps working. Phase 1 must add them and verify the paths on the notarized
build.

No other entitlement is needed: the app is not sandboxed, has no JIT or
unsigned-memory requirement, and the loopback browser in
`crates/mallard-infrastructure/src/loopback_browser.rs` needs no network
entitlement outside the sandbox.

## 2. Phase 1 — Developer ID signed and notarized Apple Silicon DMG

### 2.1 Apple account setup (manual, once)

Do these steps on the Mac that will also run local trusted builds. They need
the Apple Developer Program membership to be active and the account to be the
Account Holder (an individual membership is).

#### 2.1.1 Record the Team ID

1. Sign in at <https://developer.apple.com/account>.
2. Open **Membership details**. Copy the **Team ID** (10 characters, for
   example `A1B2C3D4E5`). It appears in the signing identity and is the
   `APPLE_TEAM_ID` secret.

#### 2.1.2 Create the certificate signing request (CSR)

On macOS 15 and later, Keychain Access is no longer in Applications →
Utilities (that folder holds the Passwords app, which has no Certificate
Assistant). Open it directly:

```bash
open "/System/Library/CoreServices/Applications/Keychain Access.app"
```

1. With Keychain Access frontmost, use the application menu (top-left):
   **Keychain Access → Certificate Assistant → Request a Certificate From a
   Certificate Authority…**.
2. Fill in:
   - User Email Address: the Apple ID email of the developer account;
   - Common Name: something recognisable, for example `Mallard Developer ID`;
   - CA Email Address: leave empty;
   - Request is: **Saved to disk**;
   - tick **Let me specify key pair information**.
3. Continue, save `CertificateSigningRequest.certSigningRequest` (Desktop is
   fine), choose **Key Size 2048 bits** and **Algorithm RSA**, Continue.
4. Keychain Access now holds a private key named after the Common Name in the
   **login** keychain. Do not delete it; the certificate is useless without
   it.

Terminal alternative (same result, no GUI):

```bash
mkdir -p ~/.private_keys && cd ~/.private_keys && \
openssl req -new -newkey rsa:2048 -nodes \
  -keyout mallard-developer-id.key \
  -out CertificateSigningRequest.certSigningRequest \
  -subj "/emailAddress=<apple-id-email>/CN=Mallard Developer ID/C=US"
```

If the CSR was made this way, the private key is a file rather than a
keychain item; step 2.1.3 item 7 then differs (see the note there).

#### 2.1.3 Create and download the Developer ID Application certificate

1. Go to <https://developer.apple.com/account/resources/certificates/list>.
2. Click the blue **+** next to Certificates.
3. Under **Software**, choose **Developer ID Application** and Continue.
   (Do **not** choose Developer ID Installer — that is for `.pkg` files —
   or any Apple Distribution/App Store type.)
4. If asked for a profile type, keep **G2 Sub-CA (Xcode 11.4.1 or later)**.
5. **Choose File** → select the `.certSigningRequest` from 2.1.2 → Continue.
6. On the confirmation page click **Download**. The file is
   `developerID_application.cer`.
7. Double-click `developerID_application.cer`. Keychain Access imports it
   into the **login** keychain and pairs it with the private key from 2.1.2.
   In Keychain Access → My Certificates you should see
   `Developer ID Application: <Name> (<TEAMID>)` with a disclosure triangle
   showing the private key underneath. If there is no private key under it,
   the CSR was generated elsewhere; repeat 2.1.2 and 2.1.3 on this Mac.

   If the CSR came from the `openssl` alternative, pair and import from the
   terminal instead (the `pkcs12 -export` password becomes
   `APPLE_CERTIFICATE_PASSWORD`, and `DeveloperID.p12` is already the file
   to encode in 2.1.5):

   ```bash
   cd ~/.private_keys && \
   openssl x509 -in ~/Downloads/developerID_application.cer -inform DER \
     -out developerID_application.pem && \
   openssl pkcs12 -export -inkey mallard-developer-id.key \
     -in developerID_application.pem -out DeveloperID.p12 && \
   security import DeveloperID.p12 -k ~/Library/Keychains/login.keychain-db \
     -T /usr/bin/codesign -T /usr/bin/security
   ```
8. If Keychain Access marks the certificate "not trusted" or `codesign`
   later reports `CSSMERR_TP_NOT_TRUSTED`, install Apple's intermediate
   certificate: download **Developer ID - G2** from
   <https://www.apple.com/certificateauthority/> (file
   `DeveloperIDG2CA.cer`) and double-click it. Current macOS versions
   already ship it; this step is usually unnecessary.

#### 2.1.4 Verify the identity

```bash
security find-identity -v -p codesigning
```

The output must contain one line such as:

```text
1) 3F1A…  "Developer ID Application: Fuyao Li (A1B2C3D4E5)"
```

The quoted string, exactly as printed, is `APPLE_SIGNING_IDENTITY`.

#### 2.1.5 Export the certificate for CI

1. In Keychain Access → login → **My Certificates**, select the
   `Developer ID Application: …` entry (the certificate row, not only the
   key).
2. Menu **File → Export Items…**, format **Personal Information Exchange
   (.p12)**, save as `DeveloperID.p12`.
3. Enter a strong password when prompted (this becomes
   `APPLE_CERTIFICATE_PASSWORD`), then the login keychain password to allow
   the export.
4. Encode it for the secret:

   ```bash
   base64 -i DeveloperID.p12 | pbcopy
   ```

   The clipboard now holds `APPLE_CERTIFICATE`.
5. After the secret is saved, delete `DeveloperID.p12` and the
   `.certSigningRequest` file. The certificate and key stay in the login
   keychain for local trusted builds.

#### 2.1.6 Create and download the App Store Connect API key

1. Sign in at <https://appstoreconnect.apple.com>.
2. **Users and Access → Integrations → App Store Connect API**.
   If this is the first key, click **Request Access** and accept the terms.
3. Under **Team Keys** click **+** (Generate API Key).
4. Name: `Mallard notarization`; Access: **Developer**. Generate.
5. Record from the table: **Issuer ID** (shown above the table, a UUID) →
   `APPLE_API_ISSUER`; **Key ID** (10 characters) → `APPLE_API_KEY`.
6. Click **Download API Key** for that row. The file is
   `AuthKey_<KEYID>.p8`. Apple lets you download it **once**; if it is lost,
   revoke the key and create a new one.
7. Keep a local copy for local trusted builds at
   `~/.private_keys/AuthKey_<KEYID>.p8` (`notarytool` looks there by
   default) with `chmod 600`.
8. The full text of the `.p8` file is `APPLE_API_KEY_CONTENT`:

   ```bash
   cat ~/.private_keys/AuthKey_<KEYID>.p8 | pbcopy
   ```

9. Confirm the key works before touching CI:

   ```bash
   xcrun notarytool history \
     --key ~/.private_keys/AuthKey_<KEYID>.p8 \
     --key-id <KEYID> --issuer <ISSUER>
   ```

   An empty history with no authentication error is a pass.

#### 2.1.7 Add the GitHub Actions secrets

Repository → Settings → Secrets and variables → Actions → **New repository
secret**, one per row:

| Secret | Value | From |
|---|---|---|
| `APPLE_CERTIFICATE` | base64 of `DeveloperID.p12` | 2.1.5 |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password | 2.1.5 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Name> (<TEAMID>)` | 2.1.4 |
| `APPLE_TEAM_ID` | the Team ID | 2.1.1 |
| `APPLE_API_KEY` | the API Key ID | 2.1.6 |
| `APPLE_API_ISSUER` | the Issuer ID | 2.1.6 |
| `APPLE_API_KEY_CONTENT` | the full text of `AuthKey_<KEYID>.p8` | 2.1.6 |

Never commit any of these values or the `.p12`/`.p8` files. Nothing in the
repository may print them; the workflow only writes the `.p8` content to
`$RUNNER_TEMP`.

### 2.2 Repository changes

1. **`src-tauri/Entitlements.plist`** (new):

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>com.apple.security.automation.apple-events</key>
     <true/>
   </dict>
   </plist>
   ```

2. **`src-tauri/Info.plist`** (new; Tauri merges it into the bundle):

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>NSAppleEventsUsageDescription</key>
     <string>Mallard opens session commands in Terminal and asks the Codex App to quit before refreshing its data.</string>
   </dict>
   </plist>
   ```

3. **`src-tauri/tauri.conf.json`** → `bundle.macOS`:

   ```json
   "macOS": {
     "signingIdentity": "-",
     "hardenedRuntime": true,
     "entitlements": "Entitlements.plist",
     "minimumSystemVersion": "12.0"
   }
   ```

   `signingIdentity: "-"` stays so local builds without the certificate remain
   structurally valid ad-hoc builds. The Tauri CLI reads
   `APPLE_SIGNING_IDENTITY` from the environment before the configuration
   value, so CI and local trusted builds override it without editing the file.

4. **`.github/workflows/release.yml`**, `publish_macos` job:
   - Add a step that writes the API key and exports its path:

     ```yaml
     - name: Install App Store Connect API key
       env:
         APPLE_API_KEY_CONTENT: ${{ secrets.APPLE_API_KEY_CONTENT }}
       run: |
         printf '%s' "$APPLE_API_KEY_CONTENT" > "$RUNNER_TEMP/AuthKey.p8"
         echo "APPLE_API_KEY_PATH=$RUNNER_TEMP/AuthKey.p8" >> "$GITHUB_ENV"
     ```

   - Pass these to the `tauri-apps/tauri-action@v1` `env`:
     `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
     `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`, `APPLE_API_KEY`,
     `APPLE_API_ISSUER`, and `APPLE_API_KEY_PATH`. With these set, the Tauri
     CLI imports the certificate into a temporary keychain, signs the app and
     the `mallard-cli` sidecar with the hardened runtime and a secure
     timestamp, submits the app to `notarytool`, and staples the ticket to
     `Mallard.app`.
   - Set `MALLARD_REQUIRE_TRUSTED_MACOS: "1"` next to
     `MALLARD_VERIFY_MACOS_BUNDLE: "1"` so the verifier fails closed if signing
     or notarization silently did not happen.
   - Replace the `releaseBody` text; it must no longer describe the macOS app
     as ad-hoc signed or unnotarized.
   - Add a **dry-run** trigger so signing and notarization can be debugged
     without consuming a version tag (`RELEASING.md` forbids replacing assets
     on an existing version):

     ```yaml
     on:
       push:
         tags: ["v*"]
       workflow_dispatch:
     ```

     On `workflow_dispatch` the macOS job runs the same build and verification
     but calls `tauri-action` without `tagName`/`releaseName`/`releaseDraft`
     (so no release is touched) and uploads the DMG with
     `actions/upload-artifact`. The version check step uses the version from
     `package.json` instead of `github.ref_name` on dispatch. The simplest
     shape is one `if: github.event_name == 'push'` release step and one
     `if: github.event_name == 'workflow_dispatch'` dry-run step sharing the
     same `env` block.

5. **`scripts/tauri-build.mjs`**: after the Tauri build, when
   `APPLE_API_KEY`, `APPLE_API_ISSUER`, and `APPLE_API_KEY_PATH` are all set,
   notarize and staple the **DMG** as well. Tauri notarizes and staples
   `Mallard.app` but not the disk image, and Apple recommends notarizing the
   image users download:

   ```bash
   xcrun notarytool submit "$DMG" \
     --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
     --issuer "$APPLE_API_ISSUER" --wait
   xcrun stapler staple "$DMG"
   ```

   Run this before the existing `MALLARD_VERIFY_MACOS_BUNDLE` step so the
   verifier checks the final artifact. Fail the build if `notarytool` does not
   report `status: Accepted`.

6. **`scripts/verify-macos-bundle.mjs`**: keep every existing check and add a
   trusted mode enabled by `MALLARD_REQUIRE_TRUSTED_MACOS=1`:
   - `codesign -dvv Mallard.app` must report
     `Authority=Developer ID Application` and `TeamIdentifier=<APPLE_TEAM_ID>`;
     the same for `Contents/MacOS/mallard-cli`.
   - `codesign -d --entitlements - --xml Mallard.app` must contain
     `com.apple.security.automation.apple-events`.
   - `spctl --assess --type execute --verbose=4 Mallard.app` must print
     `accepted` and `source=Notarized Developer ID`.
   - `xcrun stapler validate Mallard.app` must succeed.
   - `spctl --assess --type open --context context:primary-signature --verbose=4 <dmg>`
     must print `accepted`.
   - Run the app checks on both the mounted and the installed copy, as today.

7. **`tests/release-config.test.mjs`**, test "desktop release builds verified
   Apple Silicon and Windows artifacts":
   - Replace `deepEqual(tauriConfig.bundle.macOS, { signingIdentity: "-" })`
     with assertions on `signingIdentity === "-"`, `hardenedRuntime === true`,
     `entitlements === "Entitlements.plist"`, and a `minimumSystemVersion`.
   - Assert `src-tauri/Entitlements.plist` contains
     `com.apple.security.automation.apple-events` and `src-tauri/Info.plist`
     contains `NSAppleEventsUsageDescription`.
   - Replace the `doesNotMatch(/…APPLE_[A-Z_]+/)` rule with a rule that the
     seven Apple variables come from `secrets.*`, that `APPLE_API_KEY_PATH`
     points under `RUNNER_TEMP`, and that no secret value is echoed.
   - Assert `MALLARD_REQUIRE_TRUSTED_MACOS: "1"`, the `workflow_dispatch`
     trigger, and that the dispatch path contains no `tagName`.
   - Assert the verifier contains the `spctl`, `stapler`, and
     `Developer ID Application` checks and the build script contains the
     `notarytool submit` / `stapler staple` calls.
   - Keep the updater and `TAURI_SIGNING`/`latest.json` prohibitions.
   - Update the version test to 0.1.3.

8. **Docs**:
   - `docs/RELEASING.md`: rewrite "Signing and platform approval" (Developer
     ID, notarized, no Open Anyway), add the secrets table from §2.1, the
     dry-run workflow, and the local trusted build:

     ```bash
     APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TEAMID>)" \
     APPLE_TEAM_ID=<TEAMID> APPLE_API_KEY=<KEYID> APPLE_API_ISSUER=<ISSUER> \
     APPLE_API_KEY_PATH=~/.private_keys/AuthKey_<KEYID>.p8 \
     MALLARD_VERIFY_MACOS_BUNDLE=1 MALLARD_REQUIRE_TRUSTED_MACOS=1 \
     npm run tauri build -- --target aarch64-apple-darwin --bundles dmg
     ```

     Keep the "do not replace an artifact for an existing version" rule.
   - `README.md` install section: drop the Open Anyway guidance for the
     trusted release and fix the release link, which points at
     `Git4Agent/mallard` while the repository is `Git4Agent/mallard-app`.

9. **Version 0.1.3** in `package.json`, `package-lock.json` (both `version`
   fields), `Cargo.toml` `[workspace.package]`, `src-tauri/tauri.conf.json`,
   `tests/release-config.test.mjs`, and `docs/RELEASING.md`.

### 2.3 Verification and acceptance

1. `npm run check:release-version -- v0.1.3`, `npm run test:release-config`,
   `npm run test:integration`, `npm run build`, `cargo check --workspace`.
2. Local trusted build (§2.2 step 8) passes with
   `MALLARD_REQUIRE_TRUSTED_MACOS=1`.
3. Gatekeeper check: copy the DMG to a second macOS user account or a clean
   Mac through a browser download (or add the quarantine flag by hand:
   `xattr -w com.apple.quarantine "0083;$(printf '%x' $(date +%s));Safari;" Mallard_0.1.3_aarch64.dmg`),
   open it, drag to Applications, launch. It must open with no
   "unidentified developer" or "Open Anyway" prompt.
4. Apple Events on the notarized build: open a session with **Open in
   Terminal** and run a Codex App refresh. macOS must show the Automation
   prompt once (Mallard → Terminal / Codex), and the action must succeed. Check
   `System Settings → Privacy & Security → Automation` lists Mallard.
5. Dry-run workflow green on CI with the trusted verifier.
6. Push `v0.1.3`; the draft must contain `Mallard_0.1.3_aarch64.dmg`; repeat
   step 3 with the CI artifact; publish the draft.

Phase 1 is complete when a `v*` tag reproducibly produces a notarized Apple
Silicon DMG that installs on a clean Mac without security overrides and every
Apple-Events feature still works.

## 3. Phase 2 — Intel macOS and Windows

Start Phase 2 only after a trusted Apple Silicon release has been published.

### 3.1 Phase 2a — macOS Intel DMG

1. Add a `publish_macos_intel` job to `release.yml`:
   - `runs-on: macos-latest` (Apple Silicon host), `needs: publish_macos`,
     `dtolnay/rust-toolchain@stable` with `targets: x86_64-apple-darwin`.
   - Same test, API-key, and signing/notarization `env` as the Apple Silicon
     job; `MALLARD_VERIFY_MACOS_BUNDLE=1` and
     `MALLARD_REQUIRE_TRUSTED_MACOS=1`.
   - Build with `npm run tauri build -- --target x86_64-apple-darwin --bundles dmg`
     rather than `tauri-action`, then upload with `gh release upload` behind
     the same `isDraft` guard the Windows job uses. `tauri-action` creates the
     release once in `publish_macos`; later jobs only attach assets.
   - Dry-run support mirrors the Apple Silicon job.
2. `scripts/tauri-build.mjs` already computes `Mallard_<version>_x64.dmg` for
   `x86_64-apple-darwin`; no change needed for naming.
3. `scripts/verify-macos-bundle.mjs`: accept an expected architecture
   (`MALLARD_EXPECT_MACOS_ARCH=x86_64|arm64`) and check
   `lipo -archs Contents/MacOS/mallard` and `mallard-cli` against it.
   Running `mallard-cli --help` for the Intel build on the arm64 runner
   requires Rosetta 2. If the hosted image lacks it, either run
   `softwareupdate --install-rosetta --agree-to-license` in a step or move the
   job to the `macos-15-intel` runner label. Decide on the first dry run.
4. `tests/release-config.test.mjs`: allow `x86_64-apple-darwin`, assert the
   Intel job follows the Apple Silicon job, uses the draft guard, and never
   uses `--clobber`.
5. `docs/RELEASING.md`: Intel install steps and the local command:

   ```bash
   rustup target add x86_64-apple-darwin
   MALLARD_VERIFY_MACOS_BUNDLE=1 MALLARD_EXPECT_MACOS_ARCH=x86_64 \
   npm run tauri build -- --target x86_64-apple-darwin --bundles dmg
   ```

Acceptance: the notarized `x64` DMG passes the trusted verifier, and a clean
install on real Intel hardware launches and completes local-folder Push and
Pull. A Rosetta launch on Apple Silicon is only a smoke test.

### 3.2 Phase 2b — Windows x64 NSIS (unsigned)

1. **Find out whether it builds.** Run the dry-run workflow for the Windows
   job first; `npm run test:cli` compiles `mallard-cli`, `mallard-application`,
   and `mallard-infrastructure` for `x86_64-pc-windows-msvc` before the Tauri
   build. Expect failures and fix them per the AGENTS.md rule: each
   Unix-only behaviour gets an explicit `cfg(windows)` implementation or an
   explicit unsupported error; nothing may report success without running.
   Known Unix-only code to review:
   - `/bin/zsh` launches in
     `crates/mallard-infrastructure/src/providers/codex/actions.rs` and
     `crates/mallard-infrastructure/src/codex_binary.rs`;
   - `std::os::unix::fs` (permissions, `MetadataExt`, `symlink`) in
     `crates/mallard-infrastructure/src/persistence.rs`,
     `provider_capture.rs`, `storage_setup_journal.rs`,
     `crates/mallard-application/src/storage_setup.rs`, and
     `crates/mallard-cli/src/r2_browser_setup.rs`;
   - `osascript` and `/usr/bin/open` callers are already gated on
     `target_os = "macos"`.
2. Keep the existing `publish_windows` job and
   `scripts/verify-windows-bundle.ps1`; add the dry-run path
   (`actions/upload-artifact` instead of `gh release upload` on dispatch).
3. Release notes and `docs/RELEASING.md` state that the installer is
   unsigned, that SmartScreen shows "Windows protected your PC", and how to
   choose **More info → Run anyway**. Do not claim a publisher signature.
4. Add a "Windows signing (future)" section to `RELEASING.md` describing the
   intended path without implementing it: Azure Trusted Signing through
   `bundle.windows.signCommand` in `tauri.conf.json`, signing both
   `mallard.exe`, `mallard-cli.exe`, and the NSIS installer with a timestamp,
   verified by `Get-AuthenticodeSignature` returning `Valid`.
5. `tests/release-config.test.mjs`: keep the existing Windows assertions;
   add the dry-run assertions.

Acceptance: a clean Windows 11 x64 machine installs from the NSIS artifact,
launches Mallard, discovers a disposable provider home, completes a
local-folder Push and Pull, and uninstalls cleanly; the CI verifier passes.

## 4. Phase 3 — In-app update notifications

Goal: a running Mallard learns that a newer release exists, shows a
non-blocking notice, and installs it on request. This is the Tauri updater,
not Apple Push Notifications: no APNs certificate or Services certificate is
involved, and the App Store Connect API key plays no role.

History: `09929d3` added a signed updater and `af20e1a` removed it; the
"application contains no automatic updater" test in
`tests/release-config.test.mjs` and the `updater|latest.json` prohibitions in
the release test enforce that removal. Phase 3 deliberately reverses that
decision and must rewrite those assertions rather than work around them.

Start only after Phase 1 has published a notarized release: the updater
replaces the installed app with the downloaded package, and an unsigned or
unnotarized package fails Gatekeeper on the next launch.

### 4.1 One-time setup

1. Generate the updater keypair (minisign; separate from Apple signing):

   ```bash
   npm run tauri signer generate -- -w ~/.private_keys/mallard-updater.key
   ```

   Keep the private key and its password outside the repository and back
   them up; losing them means existing installations can never trust another
   update. Only the printed **public key** is committed.
2. GitHub secrets: `TAURI_SIGNING_PRIVATE_KEY` (file contents) and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

### 4.2 Repository changes

- Dependencies: `tauri-plugin-updater` and `tauri-plugin-process` (Rust and
  npm); register both in `src-tauri/src/lib.rs`; add `updater:default`,
  `process:allow-restart` to `src-tauri/capabilities/default.json`.
- `src-tauri/tauri.conf.json`: `bundle.createUpdaterArtifacts: true` and
  `plugins.updater` with the public key and the endpoint
  `https://github.com/Git4Agent/mallard-app/releases/latest/download/latest.json`.
- `release.yml`: pass the two `TAURI_SIGNING_*` secrets to every build job;
  in `publish_macos` set `uploadUpdaterJson: true` on `tauri-action`, and in
  the Intel and Windows jobs upload their `.tar.gz`/`.sig` (macOS) and
  `.exe`/`.sig` (Windows) updater assets and merge their entries into
  `latest.json` (the `tauri-action` `updaterJsonKeepUniversal`/multi-job
  pattern, or a small script that downloads, edits, and re-uploads the
  manifest under the existing `isDraft` guard). Publishing the draft is what
  makes `releases/latest/download/latest.json` resolve, so a draft never
  offers itself to users.
- Frontend: one component that checks on startup and every few hours, shows
  the available version with release notes, offers **Update and restart**
  and **Later**, shows download progress, and refuses to restart while a
  Push, Pull, or apply is in progress (reuse the existing busy state from
  `ProjectSync`).
- `tests/release-config.test.mjs`: replace the no-updater test with one that
  asserts the public key, endpoint, capabilities, secrets wiring, and that
  every release job uploads updater signatures.
- `docs/RELEASING.md`: updater key handling, the fact that only published
  releases are offered, and how to pull a bad release (unpublish → previous
  `latest.json` resolves again).

### 4.3 Acceptance

Install release N on each platform, publish N+1, and confirm: the notice
appears, **Later** defers, **Update and restart** installs N+1, the updated
macOS app launches without a Gatekeeper prompt, and a tampered `.sig` is
rejected.

## 5. Release checklist after all phases

1. Bump the version in every file from §2.2 step 9.
2. `npm run check:release-version -- vX.Y.Z`, `npm run test:integration`,
   `npm run build`, `cargo check --workspace`.
3. Run the dry-run workflow; all three jobs green.
4. Commit, push to `main`, tag `vX.Y.Z`, push the tag.
5. Inspect the draft: exactly three assets —
   `Mallard_X.Y.Z_aarch64.dmg`, `Mallard_X.Y.Z_x64.dmg`,
   `Mallard_X.Y.Z_x64-setup.exe`.
6. Clean-machine installs on Apple Silicon, Intel, and Windows.
7. Publish the draft. Never replace an asset on an existing tag; fix, bump,
   and tag again.

Secret hygiene: Developer ID certificates expire after five years; note the
date. If the API key or certificate is compromised, revoke it in App Store
Connect / the developer portal, generate replacements, and rotate the GitHub
secrets; already-notarized releases stay valid.

## 6. Risks and open items

- **Apple Events under the hardened runtime** is the most likely silent
  regression. It is only observable on a signed, notarized build, which is why
  §2.3 step 4 is mandatory before publishing.
- **Rosetta on hosted arm64 runners** decides whether the Intel verifier can
  run `mallard-cli --help`; resolved on the first Phase 2a dry run.
- **Windows compile status** is unknown until the first Phase 2b dry run; the
  size of that work is the main schedule risk of Phase 2.
- **Bundle identifier** `com.hequ.agent-sync` becomes permanent with the first
  trusted release; confirm it before tagging v0.1.3.
- **Notarization latency** is usually minutes but can be longer; the workflow
  should not add a short timeout around `notarytool --wait`.

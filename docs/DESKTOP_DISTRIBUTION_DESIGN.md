# Desktop Distribution Design

Status: Historical. Distribution, signing, and release-pipeline planning
now lives in [`PLAN_TRUSTED_DESKTOP_RELEASES.md`](./PLAN_TRUSTED_DESKTOP_RELEASES.md).
Section 9 is out of date: the application deliberately ships without an
automatic updater, and `tests/release-config.test.mjs` enforces that.
Date: 2026-07-19
Application: Mallard (Tauri 2)

## 1. Objective

Distribute Mallard as native, installable desktop packages with a download
experience similar to the MossX client page:

- macOS Apple Silicon (`.dmg`)
- macOS Intel (`.dmg`)
- Windows x64 (`-setup.exe`)
- one versioned GitHub Release containing every artifact
- a simple download page that selects the most likely installer while keeping
  explicit platform and architecture choices visible

The first release should use separate Apple Silicon and Intel DMGs. They are
smaller than a universal package and match Tauri's standard CI matrix. A single
universal DMG can be added later if a one-button macOS experience is more
important than download size.

Linux packages and app-store distribution are outside the first-release scope.

## 2. Current Repository Assessment

The repository is already close to macOS packaging readiness:

- `src-tauri/tauri.conf.json` uses the Tauri 2 schema.
- Bundling is active and currently uses `"targets": "all"`.
- macOS `.icns` and Windows `.ico` assets are present.
- The application version is `0.1.0`.
- The resolved frontend CLI and Rust Tauri versions are 2.11.x.
- `npm run build` and `cargo check --manifest-path src-tauri/Cargo.toml`
  pass on macOS.

This implementation adds:

- a tag-triggered GitHub Actions draft-release workflow;
- Tauri updater signing, runtime integration, and update UI;
- public release delivery directly through GitHub Releases; and
- a safe Windows fallback for the Unix-only Claude alias operation.

Production still requires Apple and Windows publisher credentials and
clean-machine validation on every target.

### 2.1 Windows validation gaps

Generating an installer is not enough by itself: the application must first
compile and behave correctly on Windows.

The known cross-platform compile blocker is resolved: Windows now has a
fail-closed `create_claude_alias` implementation that returns an explicit
unsupported message when a remap would require a Unix link.

It also has runtime assumptions that need Windows implementations:

- executable discovery invokes `/bin/zsh` and searches Unix install paths;
- global npm installation invokes `/bin/zsh`;
- process detection uses `pgrep`;
- the actor name reads `USER` but not Windows' `USERNAME`; and
- Claude project aliases rely on Unix symbolic-link behavior.

The initial Windows port should either implement these behaviors natively or
disable the affected features with an explicit, user-facing unsupported message.
It must not silently report success when a platform operation did not run.

## 3. Release Artifacts

Each release should publish these three primary assets:

| Platform | Architecture | Bundle | Suggested filename |
|---|---|---|---|
| macOS | Apple Silicon | DMG | `Mallard_<version>_aarch64.dmg` |
| macOS | Intel | DMG | `Mallard_<version>_x64.dmg` |
| Windows | x64 | NSIS EXE | `Mallard_<version>_x64-setup.exe` |

Optional release assets:

- SHA-256 checksum file for all installers;
- release notes;
- updater metadata and signatures after the updater is introduced; and
- a universal macOS DMG if product requirements call for one Mac download.

Windows should use NSIS rather than MSI for the first release. Tauri produces
the requested setup executable directly, and this avoids the extra WiX and
VBSCRIPT requirements associated with MSI packaging.

## 4. Product Identity and Bundle Configuration

Before the first public build, finalize these values in
`src-tauri/tauri.conf.json`:

- `productName`: use the final display name, with intended capitalization;
- `identifier`: use a permanent reverse-DNS identifier controlled by the
  publisher;
- `version`: use the public semantic version;
- publisher, copyright, and descriptions; and
- minimum supported OS versions after clean-machine testing.

The identifier should not be casually changed after release. Operating systems
and installers can treat a changed identifier as a different application,
breaking upgrades or leaving duplicate installations.

The current `"targets": "all"` setting can remain for general development, but
release commands must explicitly select `dmg` or `nsis`. Alternatively, add
platform-specific Tauri configuration files:

- `src-tauri/tauri.macos.conf.json`
- `src-tauri/tauri.windows.conf.json`

Platform-specific configuration is useful for installer branding, signing,
minimum OS versions, and Windows WebView2 policy without bloating the shared
configuration.

## 5. Local Package Generation

### 5.1 macOS, current host architecture

Run on macOS:

```bash
npm ci
npm run tauri build -- --bundles dmg
```

Expected output directory:

```text
src-tauri/target/release/bundle/dmg/
```

### 5.2 Universal macOS package

To create one DMG containing both Apple Silicon and Intel binaries:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin --bundles dmg
```

Expected output directory:

```text
src-tauri/target/universal-apple-darwin/release/bundle/dmg/
```

### 5.3 Windows x64 package

Run on native Windows or a `windows-latest` GitHub Actions runner after the
Windows portability work is complete:

```powershell
npm ci
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Expected output directory:

```text
src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\
```

Cross-compiling the Windows installer from macOS is not the release plan.
Tauri supports it with caveats, but a native Windows CI runner is simpler and
also provides the correct environment for Windows tests and code signing.

## 6. Release Pipeline

The release flow is:

1. Update and commit the application version.
2. Push a version tag such as `v0.1.0`.
3. GitHub Actions builds two macOS targets and one Windows target in parallel.
4. Platform signing happens on the native runner.
5. Tauri creates the DMG and NSIS artifacts.
6. The workflow uploads all artifacts to one draft GitHub Release.
7. A maintainer verifies installation and signatures.
8. The maintainer publishes the release.
9. The download page resolves its links from the published release.

Draft releases are preferred because they create a human review point between
automated artifact production and public distribution.

### 6.1 Implemented workflows

`.github/workflows/release.yml` builds the two macOS updater applications and
DMGs plus the Windows NSIS package, creates one draft release, validates the
generated multi-platform updater manifest, and attaches every artifact directly
to the GitHub Release. Publishing the draft makes GitHub's stable latest-release
URLs resolve to the reviewed release. Detailed operator steps are in
`docs/RELEASING.md`.

## 7. Signing and Trust

### 7.1 macOS

Internal builds may be unsigned or ad-hoc signed. Public browser downloads
should use:

1. an Apple Developer Program membership;
2. a `Developer ID Application` certificate for direct distribution;
3. Tauri code signing;
4. Apple notarization; and
5. a stapled notarization ticket.

CI secrets will include the exported certificate and its password, plus either
App Store Connect API credentials or Apple ID notarization credentials. Secrets
must remain in GitHub Actions secrets and must never be committed.

Release verification should include:

```bash
hdiutil verify path/to/Mallard.dmg
codesign --verify --deep --strict --verbose=2 /path/to/Mallard.app
spctl --assess --type execute --verbose=4 /path/to/Mallard.app
xcrun stapler validate /path/to/Mallard.app
```

### 7.2 Windows

An unsigned NSIS installer can execute, but public browser downloads will show
an unknown-publisher or Microsoft SmartScreen warning. The public release should
use Authenticode signing through a supported certificate provider or Azure
Artifact Signing.

Signing must cover both the application executable and installer produced by
Tauri. The signature should include a trusted timestamp so it remains valid
after the signing certificate expires.

Release verification should include:

```powershell
Get-AuthenticodeSignature .\Mallard_0.1.0_x64-setup.exe
```

The status must be `Valid`, and the signer subject must match the intended
publisher.

## 8. Download Page Design

The download page should provide the same clarity as the MossX page without
hiding alternate packages.

### 8.1 Page content

Show:

- current stable version;
- release date;
- recommended download based on the visitor's OS;
- explicit Apple Silicon, Intel, and Windows x64 buttons;
- minimum supported OS beside each platform;
- installer size and SHA-256 checksum;
- link to release notes;
- explicit links to every public release asset; and
- short installation guidance for macOS drag-to-Applications and Windows setup.

Do not claim minimum macOS or Windows versions until packages have been tested
on clean installations of those versions.

### 8.2 Artifact resolution

The repository and its GitHub Releases are public. The download page resolves
metadata from GitHub's public latest-release API and uses each matching asset's
`browser_download_url`. Download URLs must be accepted only when they use the
exact `github.com` origin and the repository's `/releases/download/` path.

The UI should treat operating-system detection as a recommendation, not an
access restriction. Browser architecture detection is imperfect, so all
downloads must remain visible.

GitHub serves release assets through its download infrastructure. A separate
CDN or mirror should be introduced only if future bandwidth, analytics, or
regional-performance requirements justify the added release-state complexity.

## 9. Automatic Updates

The signed updater is implemented. Mallard automatically checks after startup
and every six hours, offers the available version without interrupting work,
blocks restart while a Push, Pull, restore, or other tracked operation is busy,
shows download progress, and displays the published release notes once after a
successful restart. Choosing **Later** defers that version for 24 hours.

The implementation includes:

- `tauri-plugin-updater` and `tauri-plugin-process` in Rust and the frontend;
- a dedicated Tauri updater signing keypair, with only its public key committed;
- `bundle.createUpdaterArtifacts` and the required Tauri capabilities;
- the HTTPS endpoint
  `https://github.com/Git4Agent/mallard/releases/latest/download/latest.json`;
- a tag-triggered native build matrix that creates a draft public GitHub
  Release and attaches the installers, updater bundles, signatures, and
  `latest.json`; and
- a public website that discovers installers through GitHub's latest-release
  REST API.

The public desktop app and website need no GitHub token to read release
metadata or download release assets.

Updater signing is separate from macOS and Windows publisher signing. The
updater private key must be backed up securely: losing it prevents existing
installations from trusting later updates.

## 10. Validation Matrix

Before publishing the first release, validate at least:

| Area | macOS Apple Silicon | macOS Intel | Windows x64 |
|---|---:|---:|---:|
| Clean installation | Required | Required | Required |
| First launch | Required | Required | Required |
| Signature/trust verification | Required | Required | Required |
| Local-folder storage | Required | Required | Required |
| S3/R2 connection | Required | Required | Required |
| Codex profile discovery | Required | Required | Required |
| Claude profile discovery | Required | Required | Required |
| Push, fetch, review, and apply | Required | Required | Required |
| Upgrade over previous version | Required | Required | Required |
| Uninstall/reinstall preserves user data | Required | Required | Required |

Tests must use disposable provider homes and storage, following the repository's
existing safety guidance. Do not validate destructive restore behavior against
the developer's real `~/.codex`, `~/.claude`, or `~/.mallard` data.

## 11. Implementation Phases

### Phase 1: macOS development package

- finalize product name and identifier;
- generate an unsigned or ad-hoc DMG;
- install it on a second Mac account or clean machine; and
- verify the application can access intended profile and storage paths.

### Phase 2: Windows portability

- make the Rust backend compile for `x86_64-pc-windows-msvc`;
- add Windows executable discovery for `.exe`, `.cmd`, and `.bat` launchers;
- replace shell-based npm installation with platform-specific invocation;
- implement Windows process detection;
- decide how Claude project aliasing behaves on Windows; and
- run the integration suite on a Windows runner.

### Phase 3: automated unsigned releases

- add the three-platform GitHub Actions matrix;
- create draft releases from version tags;
- upload DMG and NSIS artifacts; and
- verify release naming and download links.

### Phase 4: trusted public releases

- configure Apple Developer ID signing and notarization;
- configure Windows Authenticode signing;
- add signature verification to release checks;
- publish the download page; and
- document installation and troubleshooting.

### Phase 5: updates

- [x] add the Tauri updater and restart flow;
- [x] publish signed update metadata as GitHub Release assets;
- [x] point the updater at GitHub's stable latest-release manifest URL;
- [x] resolve website downloads from GitHub's public release API; and
- [ ] test real upgrades and rollback/error behavior on all release targets.

## 12. Acceptance Criteria

The first public desktop release is complete when:

- a version tag reproducibly creates two signed/notarized DMGs and one signed
  NSIS setup executable;
- all three artifacts are attached to one draft GitHub Release;
- installation succeeds on clean supported systems without bypassing normal OS
  security controls;
- core sync flows pass on all three targets;
- the download page links the correct immutable release assets;
- checksums and release notes are available; and
- no certificate, token, updater private key, or local Mallard data is
  committed to the repository.

## 13. Reference Findings

The MossX download page is useful as a presentation reference because it groups
downloads by operating system and publishes support expectations. Its linked
`jetbrains-cc-gui` repository is not a desktop-installer reference, however. It
is an IntelliJ plugin project whose workflow builds a plugin ZIP on Ubuntu and
uploads that ZIP to GitHub Releases.

Use Tauri's release tooling for Mallard rather than copying that Gradle
workflow.

## 14. Sources

- [Tauri: DMG distribution](https://v2.tauri.app/distribute/dmg/)
- [Tauri: Windows installers](https://v2.tauri.app/distribute/windows-installer/)
- [Tauri: GitHub Actions pipeline](https://v2.tauri.app/distribute/pipelines/github/)
- [Tauri: macOS signing and notarization](https://v2.tauri.app/distribute/sign/macos/)
- [Tauri: Windows code signing](https://v2.tauri.app/distribute/sign/windows/)
- [Tauri: updater plugin](https://v2.tauri.app/plugin/updater/)
- [Apple: Developer ID](https://developer.apple.com/developer-id/)
- [MossX download page](https://www.mossx.ai/en/download)
- [`jetbrains-cc-gui` build workflow](https://github.com/zhukunpenglinyutong/jetbrains-cc-gui/blob/main/.github/workflows/build.yml)

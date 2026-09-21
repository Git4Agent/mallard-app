# Design: In-App Updates from GitHub Releases

Status: Implemented on `app-updates` (2026-09-20). First release carrying the
client: v0.1.3. The updater keypair and the two `TAURI_SIGNING_*` secrets are
a one-time maintainer step (§4) that must precede the v0.1.3 tag.
Date: 2026-09-20
Application: Mallard (Tauri 2.11, `com.hequ.agent-sync`)
Related: [`RELEASING.md`](./RELEASING.md) (operator steps),
[`PLAN_TRUSTED_DESKTOP_RELEASES.md`](./PLAN_TRUSTED_DESKTOP_RELEASES.md) §4
(the phase this delivers).

## 1. Objective

A running Mallard learns that a newer **published** release exists on GitHub,
says so once without interrupting work, downloads the signed package on
request, refuses to restart while a Push, Pull, or apply is in flight, and
relaunches on the new version. Users on v0.1.3 or later never need to revisit
the Releases page to stay current.

Out of scope: downgrades, delta updates, update channels (beta/stable), and
Windows Authenticode signing (still a follow-up; see §9).

## 2. History and why now

| Commit | Date | Event |
|---|---|---|
| `09929d3` | 2026-07-20 | Signed updater added (`tauri-plugin-updater`, `AppUpdater.tsx`). |
| `f17cef1` | 2026-07-21 | Apple release signing disabled: no certificate existed. |
| `e633e69`, `af20e1a` | 2026-07-21 | Updater UI and infrastructure removed; `tests/release-config.test.mjs` began enforcing the absence. |
| `83be57d`, `8b4cca3` | 2026-09-20 | Developer ID signing, notarization, Intel and Windows jobs on `release-package-enhancement`. |

The updater was removed for one reason: it replaces the installed `.app` in
place, and an unnotarized replacement fails Gatekeeper on the next launch with
no way back. Trusted releases remove that reason. The tests that enforced the
removal are rewritten to pin the new trust boundary rather than bypassed.

## 3. Trust model

Two independent signatures, each protecting a different thing:

| Signature | Protects | Where the key lives |
|---|---|---|
| Apple Developer ID + notarization + stapled ticket | The `.app` that Gatekeeper launches after the update. | `APPLE_*` GitHub secrets; local login keychain. |
| Tauri updater key (minisign, ed25519) | The bytes the updater downloads — `Mallard_<v>_<arch>.app.tar.gz` and `Mallard_<v>_x64-setup.exe` — via the `signature` field in `latest.json`. | `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub secrets; the key file backed up outside the repository. |

Only the **public** updater key is committed (`src-tauri/tauri.conf.json`,
`plugins.updater.pubkey`). The plugin rejects any download whose signature
does not verify against it, so a compromised GitHub account or a tampered
asset cannot push code to installed copies without the private key.

**Losing the private key strands every installed copy**: no later release can
be trusted by the updater, and users must reinstall by hand. Back it up before
the first tagged release that ships the client.

The endpoint is one URL, pinned in configuration:

```text
https://github.com/Git4Agent/mallard-app/releases/latest/download/latest.json
```

GitHub's `latest` redirect resolves only to a **published, non-pre-release**
release. A draft never reaches an installed copy, so the maintainer's existing
review gate — publish the draft — is exactly the action that makes an update
visible. No token, no GitHub API call, and nothing about the user or their
projects is sent; the request is a plain HTTPS `GET`.

## 4. One-time setup (maintainer)

1. Generate the keypair, outside the repository:

   ```bash
   npm run tauri signer generate -- -w ~/.private_keys/mallard-updater.key
   ```

   The July key targeted a different repository and no installed build trusts
   it; generate a fresh one. Back up the key file and its password.
2. Add the GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` (the key file's
   contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
3. Paste the printed public key into `plugins.updater.pubkey` in
   `src-tauri/tauri.conf.json`, replacing the placeholder
   `REPLACE_WITH_MALLARD_UPDATER_PUBLIC_KEY`. The release-config test only
   checks the value's shape; a placeholder ships a client that can never
   verify an update, so this step is on the release checklist in
   `RELEASING.md`.

## 5. Release assets

Per release `vX.Y.Z` the draft must hold **nine** assets before it is
published:

| Asset | Job | Notes |
|---|---|---|
| `Mallard_X.Y.Z_aarch64.dmg` | `publish_macos` | via `tauri-action`, as before |
| `Mallard_X.Y.Z_aarch64.app.tar.gz` + `.sig` | `publish_macos` | updater archive, renamed by the build wrapper |
| `Mallard_X.Y.Z_x64.dmg` | `publish_macos_intel` | as before |
| `Mallard_X.Y.Z_x64.app.tar.gz` + `.sig` | `publish_macos_intel` | renamed |
| `Mallard_X.Y.Z_x64-setup.exe` | `publish_windows` | the installer is the updater artifact on Windows |
| `Mallard_X.Y.Z_x64-setup.exe.sig` | `publish_windows` | |
| `latest.json` | `publish_updater_manifest` | written once, after all three |

Both macOS builds emit an identically named `bundle/macos/Mallard.app.tar.gz`.
`scripts/tauri-build.mjs` renames it to the DMG's `<Product>_<version>_<arch>`
stem so the second upload cannot collide with the first.

### 5.1 `latest.json`

Assembled by `scripts/build-updater-manifest.mjs` from the three `.sig` files:

```json
{
  "version": "0.1.4",
  "notes": "https://github.com/Git4Agent/mallard-app/releases/tag/v0.1.4",
  "pub_date": "2026-10-01T12:00:00.000Z",
  "platforms": {
    "darwin-aarch64": { "signature": "…", "url": "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.4/Mallard_0.1.4_aarch64.app.tar.gz" },
    "darwin-x86_64":  { "signature": "…", "url": "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.4/Mallard_0.1.4_x64.app.tar.gz" },
    "windows-x86_64": { "signature": "…", "url": "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.4/Mallard_0.1.4_x64-setup.exe" }
  }
}
```

`notes` is the release page rather than the body: the maintainer edits the
page after the manifest is written, and the client links it instead of
repeating it. The script refuses a version that is not `MAJOR.MINOR.PATCH`, a
tag that is not `v<version>`, a version that differs from `package.json`, a
missing or empty signature, an unknown platform, or a repository that is not
`owner/name`; every URL is under
`https://github.com/Git4Agent/mallard-app/releases/download/`.

## 6. Build and release pipeline

```text
tag vX.Y.Z
  └─ publish_macos (Apple Silicon)
       build → sign → notarize → verify DMG and updater archive
       tauri-action creates the DRAFT and attaches the DMG
       gh release upload  _aarch64.app.tar.gz + .sig   (isDraft guard)
       artifact: updater-sig-darwin-aarch64
       ├─ publish_macos_intel        gh release upload _x64.dmg + _x64.app.tar.gz + .sig
       │                             artifact: updater-sig-darwin-x86_64
       └─ publish_windows            gh release upload _x64-setup.exe + .sig
                                     artifact: updater-sig-windows-x86_64
            └─ publish_updater_manifest  (needs all three)
                 download the three signatures
                 build-updater-manifest.mjs → latest.json
                 isDraft guard; every referenced asset must already be attached
                 gh release upload latest.json
maintainer installs from the draft on each platform, then PUBLISHES
  └─ releases/latest/download/latest.json now resolves → clients see it
```

Design points:

- **Artifacts are a per-build request, not a committed setting.**
  `bundle.createUpdaterArtifacts` is injected by `scripts/tauri-build.mjs`
  through the same `--config` overlay that stages the CLI sidecar, and only
  when `TAURI_SIGNING_PRIVATE_KEY` is set. A local ad-hoc build without the
  key still builds. `MALLARD_REQUIRE_UPDATER_ARTIFACTS=1` (set in every
  release job) makes a missing key a build failure instead of a silent
  unsigned release.
- **The updater archive is verified like the DMG.** In trusted mode
  `scripts/verify-macos-bundle.mjs` extracts the `.app.tar.gz` and runs the
  same `codesign --verify`, `spctl --assess --type execute`
  (`source=Notarized Developer ID`), `stapler validate`, `lipo -archs`, and
  `mallard-cli --help` checks on the extracted app. Tauri tars the app after
  signing and notarizing it; this proves it, on every build.
- **One manifest, written whole.** No job uploads a partial `latest.json`
  (`tauri-action`'s `uploadUpdaterJson` stays off) and no job uses
  `--clobber`. If any build job fails there is no manifest, and a draft
  without `latest.json` must not be published.
- **The dry run covers the manifest.** On `workflow_dispatch` every job signs
  with the real key and the manifest job builds `latest.json` from the
  signatures, uploading it as a workflow artifact; nothing touches a release.

## 7. Client behaviour

Implemented in `src/components/AppUpdater.tsx` and wired into
`src/components/project-sync/ProjectSync.tsx`.

| Rule | Behaviour |
|---|---|
| **When it checks** | 8 s after startup, then every 6 hours, and on window focus if 6 hours have passed. A check is skipped while another check or an install is running. |
| **Opt-out** | Settings › Privacy › *Automatic update checks* (`update_check_enabled` in `~/.mallard/settings.json`, default on). The setting is read at the moment a scheduled check is due, so the toggle needs no event plumbing. It is not a tuning value: toggling it never purges caches. |
| **Manual check** | Settings › Privacy › *Check for updates* (dispatches `mallard:check-for-updates`). Ignores the opt-out and the interval. Reports *Checking*, *Mallard is up to date* (auto-dismisses after 4 s), or the error with *Try again*. A failed **scheduled** check stays quiet. |
| **One notice** | The updater contributes one `NoticeItem` to the shell's single notice zone (`docs/PLAN_NOTICE_SYSTEM.md`): "Note · Mallard X.Y.Z is available." with *Release notes*, *Later*, and *Update and restart*. It shares the box with any other shell message; no banner family of its own. |
| **Busy gate** | While `ProjectSync`'s `busy` is set (Push, Pull, apply, restore, setup), the message reads "Finish the current operation before restarting" and *Update and restart* is disabled. The install path re-checks `busy` after the confirm dialog. |
| **Later** | Hides that version, and only that version, for 24 h (`localStorage`). A manual check ignores the deferral. |
| **Install** | Native confirm → modal `<dialog>` with a `<progress>` bar (cannot be cancelled; Escape is swallowed) → `downloadAndInstall` (5-minute timeout) → `relaunch()`. On macOS the plugin replaces `Mallard.app` in place; on Windows it runs the NSIS installer in passive mode and exits. |
| **Failure** | The notice becomes "Error · Mallard X.Y.Z did not install." with the reason, *Try again*, and *Later*. The pending-changelog marker is cleared so a failed install never claims success. |
| **After restart** | Once: "Done · Mallard X.Y.Z installed." with *Release notes*. Keyed by version in `localStorage`, so it shows once per update. |
| **Outside Tauri** | `npm run dev` (Vite only): every hook is inert and the Settings button is disabled with a tooltip. |

The download happens in Rust (the plugin), so the webview CSP's
`connect-src 'self' ipc:` is unaffected. The release-notes link opens in the
system browser through `openExternalUrl` (`opener:default` allows https).

## 8. Failure modes and rollback

| Situation | What happens | Operator action |
|---|---|---|
| A build job fails | No `latest.json`; the draft is incomplete. | Do not publish. Fix, bump, tag again (`RELEASING.md` forbids replacing assets on a tag). |
| Draft published with a bad build | Clients on the previous version are offered it. | `gh release edit vX.Y.Z --draft --repo Git4Agent/mallard-app` immediately. `latest` falls back to the previous published release, whose `latest.json` names a lower version, so nobody else is offered it. |
| Users already updated to the bad build | The updater never downgrades. | Ship vX.Y.Z+1 with the fix; they update forward. |
| Private updater key lost | New releases cannot be verified by installed copies. | Generate a new key, commit the new public key, and tell users to reinstall once from the Releases page. There is no in-band recovery. |
| Private updater key leaked | A leaked key alone is not enough: publishing also needs release write access to `Git4Agent/mallard-app`. | Rotate: new key, new public key committed, next release signed with it. A copy verifies against the public key baked into the build it runs, so any copy that skips the rotation release cannot update past it; announce a one-time manual reinstall. |
| GitHub unreachable | Scheduled check fails silently; manual check reports the error. | None. |
| Tampered `.sig` or asset | Plugin rejects the download; notice shows the error. | Investigate the release's assets. |

## 9. Open items

- **`--bundles dmg` and the updater archive.** Confirmed locally
  (2026-09-20) that the Tauri 2.11 CLI appends the updater bundle whenever
  `createUpdaterArtifacts` is on even though `--bundles app` did not name it
  (`Finished 1 bundle … (updater)`), and that the wrapper's rename and the
  verifier's extraction work on the result. The `--bundles dmg` path could
  not be run from the same session (the DMG script needs Finder automation),
  so `scripts/tauri-build.mjs` still fails the build if the archive is
  missing and the first dry run is the final confirmation.
- **Windows SmartScreen on an updater-launched installer.** The NSIS
  installer is unsigned. A browser download carries the Mark-of-the-Web and
  triggers SmartScreen; the updater's download should not, but this must be
  confirmed on a clean Windows 11 machine before v0.1.3 is published. If it
  is blocked, the Windows notice must fall back to a *Download* button that
  opens the release page until Authenticode signing lands.
- **Placeholder public key.** `tauri.conf.json` ships
  `REPLACE_WITH_MALLARD_UPDATER_PUBLIC_KEY` until §4 is done; the v0.1.3
  checklist in `RELEASING.md` requires the real key before tagging.

## 10. Verification

Automated, on every push and in CI:

- `npm run test:release-config` — `tests/release-config.test.mjs` pins the
  endpoint, the committed public key's shape, the capabilities, the wrapper's
  key-gated artifact request and per-arch rename, the verifier's archive
  check, every job's `TAURI_SIGNING_*` wiring and `MALLARD_REQUIRE_UPDATER_ARTIFACTS`,
  the manifest job's `needs`, guards, and upload, and that nothing uses
  `--clobber` or `uploadUpdaterJson`; `tests/updater-manifest.test.mjs` covers
  the manifest builder and its CLI.
- `npm run test:frontend-integration` — `tests/frontend/app-updater.integration.test.tsx`
  covers the notice copy for every phase, the busy gate, the *Later*
  deferral, the scheduled-check switch (and that a manual check ignores it),
  the progress dialog, and the shell/Settings wiring.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` — settings default.

Manual, before publishing the first release that carries the client:

1. Dry-run workflow green on all four jobs; artifacts hold two archives,
   three signatures, and `latest.json`.
2. Install the dry-run build. Serve a hand-built `latest.json` for a higher
   dry-run version from a local HTTPS or file server and temporarily point a
   local build's endpoint at it. Confirm: the notice appears after ~8 s;
   *Later* hides it and the manual check brings it back; *Update and restart*
   is disabled during a Push; installing while idle downloads, restarts, and
   launches without a Gatekeeper prompt; the "installed" receipt shows once;
   a tampered `.sig` is rejected with an error notice. Repeat on Intel
   (Rosetta smoke at minimum) and Windows.
3. Tag v0.1.3, verify nine assets on the draft, publish. The first genuine
   in-app update is v0.1.3 → v0.1.4.

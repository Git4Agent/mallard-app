# Qoder App projection probe

This PoC checks one prerequisite for Qoder App resume: whether a clean Qoder
App database discovers a valid native Qoder CLI transcript.

The tested Qoder App `0.1.4` result is:

```text
QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS
```

Qoder created `main.sqlite` schema 64 and the `chat_sessions` table. The table
had zero rows. The App did not register the source CLI UUID, so the probe did
not attempt continuation.

The complete method, evidence, and limits are in
[`../../docs/QODER_CROSS_PATH_AND_APP_RESUME_POC_2026-09-02.md`](../../docs/QODER_CROSS_PATH_AND_APP_RESUME_POC_2026-09-02.md).

## Safety boundary

The runner accepts a CLI configuration only when it is under the exact
temporary root created by the guarded authenticated CLI probe. It rejects a
normal `~/.qoder` path.

The runner:

- uses Qoder's internal unpackaged E2E smoke mode;
- creates a separate App profile and `main.sqlite`;
- uses Qoder's fake E2E user;
- passes Chromium's `--use-mock-keychain` switch;
- does not inherit credential environment variables;
- does not copy or edit an App database;
- verifies that the CLI transcript is unchanged;
- fails if Qoder creates `auth.v1.dat`;
- fails if the observed `security` command is invoked; and
- removes its App profile by default.

This is not a full operating-system access audit. Do not use the runner with a
normal Qoder home.

## Offline tests

```sh
node --test poc/qoder-app-resume/qoder-app-projection-probe.test.mjs
```

## Product run

Use an extracted copy of the installed Qoder App and a generic Electron runtime
that matches its major version:

```sh
node poc/qoder-app-resume/qoder-app-projection-probe.mjs \
  --electron /absolute/temp/path/Electron \
  --app-source /absolute/temp/path/extracted-qoder-app \
  --app-resources /Applications/Qoder.app/Contents/Resources \
  --qoder-config /absolute/validated/retained-root/qoder-config \
  --timeout-ms 60000
```

The Qoder App can show a Qoder Demo window for a few seconds. The test does not
run the packaged App against the normal App profile.

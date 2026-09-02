# Authenticated Qoder CLI native-resume smoke test

## Purpose

Run this test yourself after you can authenticate Qoder CLI. The test checks
one narrow behavior in Qoder CLI `1.1.40`:

1. Create a native Qoder session in a new, isolated configuration directory.
2. Ask Qoder to remember a random marker.
3. Start a second CLI process.
4. Resume the exact source-session UUID.
5. Ask for the marker without putting the marker in the resume prompt.
6. Verify that Qoder returns the marker and appends to the source transcript.

The runner uses Qoder's documented authentication flow and CLI options. See
[Qoder CLI authentication](https://docs.qoder.com/cli/authentication) and the
[Qoder CLI reference](https://docs.qoder.com/cli/cli-reference).

## Scope

A pass proves all of these facts for one fresh session:

- Interactive login works with an isolated Qoder configuration.
- A persistent source turn creates the expected JSONL transcript.
- `--resume <source-uuid>` continues the same session in a new CLI process.
- The resumed model response recovers a marker that is absent from the second
  prompt.
- The CLI reports the same session UUID for the source and resumed turns.
- The transcript size increases and its SHA-256 digest changes after resume.
- The runner configures Qoder's version-specific file credential store and
  detects any attempted `security` command invocation.

A pass does not make Qoder session restore ready for Mallard.

## Prerequisites

You need:

- network access for Qoder login and model requests;
- a Qoder account that you can use for this disposable test;
- Node.js 20 or newer in `PATH`;
- this Mallard repository; and
- an absolute path to the installed Qoder CLI package file
  `bundle/qodercli.js`.

Use the CLI package bundle. Do not use a desktop launcher or an npm dispatcher.
The runner accepts only a canonical, regular, non-symbolic-link bundle for the
pinned package and version. It rejects another package, version, or bundle
digest.

If you already have the package, set the variable to its canonical bundle
path:

```sh
QODER_CLI_BUNDLE="$(realpath "/absolute/package/path/bundle/qodercli.js")"
```

If you do not have the npm package path, install the pinned package in a new
temporary directory. npm uses your current `NPM_CONFIG_REGISTRY` value, if it
is set:

```sh
QODER_CLI_PACKAGE_ROOT="$(mktemp -d -t mallard-qoder-cli)"
npm install --prefix "$QODER_CLI_PACKAGE_ROOT" \
  --ignore-scripts --no-save @qoder-ai/qodercli@1.1.40
QODER_CLI_BUNDLE="$(realpath \
  "$QODER_CLI_PACKAGE_ROOT/node_modules/@qoder-ai/qodercli/bundle/qodercli.js")"
```

Use `$QODER_CLI_BUNDLE` as the `--cli-bundle` value. The package directory does
not contain Qoder authentication, but remove it when you no longer need the
test dependency.

The test sends two short prompts to the Qoder model service. These requests can
consume account quota.

## Run the test

From the Mallard repository root, run:

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --cli-bundle "$QODER_CLI_BUNDLE"
```

The runner starts the isolated Qoder `login` command. It prints the Qoder
sign-in URL instead of opening a browser. Open that URL and complete login.
Do not start a separate Qoder process for this test.

The runner then makes the source and resume model requests. Do not type the
test marker. The runner creates and checks the marker automatically.

A successful result has this verdict:

```json
{
  "verdict": "AUTHENTICATED_NATIVE_RESUME_PASS",
  "productionRestoreReady": false,
  "authentication": {
    "credentialStorage": "isolated_encrypted_file"
  },
  "isolation": {
    "nativeKeychainBypassConfigured": true,
    "nativeKeychainCommandAttempted": false
  }
}
```

In the default cleanup mode, the JSON result contains only bounded test
evidence. It does not contain the credential, random marker, prompt text, raw
model output, session UUID, or temporary path. With `--keep`, the runner writes
the retained root path separately so that you can inspect it.

## Increase the operation timeout

The default timeout can be too short on a slow model connection. Set a timeout
in milliseconds:

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --cli-bundle "$QODER_CLI_BUNDLE" \
  --timeout-ms 600000
```

The timeout applies separately to login and each bounded model operation. The
default is `180000`. The accepted range is `30000` through `900000`.

## Keep diagnostic state

By default, the runner deletes its validated temporary root after success or
failure. Add `--keep` only when you must inspect a failed run:

```sh
node poc/qoder-cli-resume/qoder-cli-authenticated-resume.mjs \
  --cli-bundle "$QODER_CLI_BUNDLE" \
  --keep
```

The runner prints the retained root path. This directory contains the isolated
Qoder login state and transcript. Treat it as sensitive. Move only that exact
directory to Trash when the investigation is complete. First verify that its
base name starts with
`mallard-qoder-manual-resume-`.

An interrupted process can also leave its temporary root. Apply the same
validation before you remove it.

## What the runner isolates

The runner:

- creates a new operating-system temporary root with restrictive permissions;
- places `HOME`, Qoder configuration, XDG state, and temporary files below
  that root;
- uses an empty temporary project as the exact working directory for both
  turns;
- does not contain runner code that reads or copies authentication from
  `~/.qoder`;
- sets the version-specific `QODER_FORCE_FILE_STORAGE=true` control;
- places a fail-closed `security` command guard first in `PATH`;
- disables user and project setting sources;
- disables tools, MCP servers, and built-in skills for both model turns;
- limits each model call to one turn and a small output;
- uses a new Qoder-assigned session UUID and a random marker for each run;
- starts resume as a separate process; and
- validates the temporary-root marker before cleanup.

## Keychain control

Qoder CLI `1.1.40` compares `QODER_FORCE_FILE_STORAGE` with the exact string
`true`. The value `1` is not equivalent in this code path. The first
user-operated run used `1`. Qoder then displayed a macOS Keychain availability
dialog. That run proves native semantic resume, but it does not prove that the
CLI avoided Keychain access.

The corrected runner uses the exact value `true`. Qoder stores the interactive
login in its encrypted file store under the isolated configuration root. The
runner removes that root after the test unless you specify `--keep`.

The runner also puts a local command named `security` first in `PATH`. If the
pinned CLI tries its observed `security default-keychain` probe, the command is
blocked and the test fails with `native_keychain_command_attempted`. This guard
does not modify the user's Keychain.

An unexpected macOS Keychain dialog is a test failure. Select **Cancel**. Do
not select **Reset To Defaults**.

The test intentionally permits Qoder network access for login and model calls.
The runner is not an operating-system filesystem or network sandbox.
Environment and CLI selectors do not prove that the opaque Qoder process never
accesses the normal home, calls Keychain through an unobserved native path, or
accesses another machine path. Run the test in a disposable operating-system
account or VM if you need that proof.

## Failure meaning

The command exits with a failure if it cannot establish all pass conditions.
Common failure classes are:

- bundle identity or Qoder CLI version mismatch;
- incomplete login or account authorization failure;
- model-request timeout;
- an attempted native Keychain command;
- source response or resume response is not the exact marker;
- CLI-reported session UUID mismatch;
- expected transcript is missing or is not a regular file; or
- transcript size or digest does not change after resume.

A failure does not show that Qoder resume is generally broken. It shows that
this run did not produce sufficient evidence. Retry once for a transient login
or model error. Use `--keep` for the next run if you need local artifacts.

## Not proved

This smoke test does not prove:

- Mallard capture or restore of a Qoder session;
- restore into a different Qoder configuration directory or machine;
- stable capture or materialization of Qoder task sidecars;
- Qoder use of state, compression, subagent, or task sidecars;
- authoritative `active-leaf` selection rules;
- structural path mapping or continuation after a project-path change;
- source-UUID collision safety across concurrent restore operations;
- Qoder App discovery, application-database projection rebuild, or App
  continuation;
- Qoder fork behavior;
- a custom model provider or the Tuzi API; or
- production session-restore readiness.

Use this result only as evidence for native CLI semantic resume of one fresh
session. Keep the Mallard Qoder production-restore gate closed.

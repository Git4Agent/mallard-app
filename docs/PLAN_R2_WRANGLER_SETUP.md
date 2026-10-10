# Replace R2 agent setup with Wrangler setup

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: implemented and verified. Based on the successful
[local Wrangler POC](../poc/r2-local-wrangler/README.md).

## Scope and approach

Replace the R2 agent setup action with **Set up with Wrangler**. The desktop
backend will run the packaged `mallard-cli` directly. That CLI already owns
R2 authentication, bucket provisioning, browser consent, and draft preparation.
Its JSON output already reports success and fixed error codes.

Keep that CLI as the single setup implementation. Keep the current R2 API,
bucket journal, callback checks, schemas, manual entry, and final review.
Dropbox keeps its current setup action.

## User flow

1. The user selects **Set up with Wrangler** in the existing R2 setup row.
2. Mallard saves the current form and pins its draft ID and revision.
3. The helper checks that Wrangler can run. If required, the user installs it.
4. Mallard starts setup. Show **Setting up R2. Complete authorization in your
   browser.** Disable duplicate setup and conflicting form actions while busy.
5. The helper uses the current login or opens Wrangler login. It prepares the
   bucket, opens credential consent, and fills the same draft.
6. Mallard loads that exact draft. Show **Connection ready for review.**
7. The user can select **Test connection**, then **Finish setup**.

The same action can prepare credentials for an existing destination. It must
preserve that destination.

## Missing dependency

Show this inline message when Wrangler is missing:

> Wrangler is required for R2 setup. Install it, then select Try again.

Show the command with **Copy command**:

```sh
npm install --global wrangler@latest
```

Provide **Copy agent message** with this exact instruction:

> Install Cloudflare Wrangler globally with `npm install --global wrangler@latest`.
> Check `npm --version` first. If npm is missing, tell me that Node.js and npm
> are required. After installation, verify `wrangler --version`, then stop.
> I will return to Mallard to continue R2 setup.

**Try again** repeats the dependency check and continues setup if it passes.
The agent message is installation help. It contains no draft ID, credential,
or Mallard setup command. Copying it does not launch an agent or install software.

Distinguish a missing executable from one that cannot start. For the latter,
show **Wrangler could not start. Check the Node.js and Wrangler installation.**
An unsupported Wrangler command should request an update using the same install
command. A network or account error must not be reported as a missing dependency.

## Backend changes

- Add one awaited Tauri command:
  `setup_r2_storage(draftId, expectedRevision, reauthenticate)`.
  Read the saved name from the draft. Validate the R2 provider and revision.
  Release any local validation lock before starting the CLI; the CLI takes
  the existing operation lock itself.
- Reuse `packaged_cli_path` and invoke the binary with separate arguments:
  `--json storage r2 setup --draft-id … --expected-revision … --name …`.
  Keep process output bounded. Return only the known JSON result or a fixed
  error. Do not log raw output or expose arbitrary command execution.
- Put dependency checks in the existing Wrangler client so the desktop and
  CLI use the same runtime. Reuse the repository's executable lookup patterns.
  Resolve both Wrangler and its Node runtime for a desktop launch. Check
  `wrangler --version` with that same runtime. Preserve the isolated working
  directory and sanitized environment. Cover npm launchers on supported systems.
- Add an explicit R2 `--login` option for `reauthenticate: true`. It runs
  Wrangler login before the existing setup operation continues. This handles
  the expired-session failure found in the POC without guessing from stderr.
- Keep the existing provisioning journal. A retry after consent timeout must
  reuse a confirmed bucket. An unknown creation result must keep blocking a
  second create request.
- Add `cancel_r2_storage_setup(draftId, expectedRevision)`. Cancel only the
  matching setup. Stop the helper and its child processes before releasing
  the active setup guard. Keep the draft, provisioning journal, and cloud
  resources. Report a missing helper separately from a missing Wrangler.

Keep the current timeout values. On consent timeout, show **Authorization
expired. Try again.** On an unusable login, offer **Sign in again** through
the same command with `reauthenticate: true`. Each retry starts a fresh browser
flow after the previous process exits. Created cloud resources remain after
timeout, as in the current implementation.

Show **Cancel setup** while setup runs. Disable it while cancellation is in
progress. Enable retry after the setup command returns. Cancellation during
draft save prevents the helper from starting. Tell the user to close the
authorization tab before retry.

Use one active setup and an awaited command. A task registry, automatic retries, a dependency installer, and new bucket pickers
are outside this change.

## Frontend and descriptor changes

- Add one optional descriptor capability, `local_setup: "wrangler"`.
  Set it on R2 and remove R2's `setup_assistant` declaration and setup prompt.
  Update the Rust descriptor and TypeScript type together. Keep the shared
  agent APIs used by Dropbox.
- Use a small Wrangler setup component in the existing editor row. Rename
  the editor's `agentSlot` prop to `setupSlot`. The workspace keeps ownership
  of the draft, save queue, request, and result adoption.
- Expose the new command through `projectSyncApi`. After success, exact-load
  the requested draft immediately. Reuse the current identity, revision, and
  form-session checks. Completion must not depend on the file watcher event
  arriving after a listener is armed.
- Remove R2's agent launcher, copied setup prompt, and agent-waiting text.
  The install message remains available only in dependency help.
- Update the R2 setup instructions in `README.md` and related UI copy.

Primary files: `ProjectLinksWorkspace.tsx`, `StorageSettings.tsx`,
`project-sync/api.ts`, `src/types.ts`, the Tauri commands and service,
`storage_providers/traits.rs`, `storage_providers/r2.rs`, and the CLI R2
arguments, backend, and Wrangler client. Keep the launcher helper in one
small Tauri module if needed to keep process handling separate from the service.

## Acceptance and verification

- R2 setup works with no agent configured or installed.
- Missing Wrangler shows the command, agent message, and working retry action.
- Desktop launch finds an installed Wrangler and Node with a limited PATH.
- Valid login, fresh login, expired login, and delayed consent have clear results.
- Consent retry preserves the draft and confirmed bucket; uncertain creation
  cannot create a replacement bucket.
- A quick CLI result, duplicate click, form change, or navigation cannot apply
  values to the wrong form. Test this with an injected process runner and
  temporary repositories, not the user's provider homes or Mallard state.
- Credentials remain outside process output. Setup prepares a draft and never
  tests or commits storage automatically.
- Manual R2 entry and Dropbox setup retain their current behavior.

Run `npm run build`, `npm run test:frontend-integration`,
`cargo test -p mallard-cli`, `cargo test -p mallard-infrastructure --lib`, and
`cargo test --manifest-path src-tauri/Cargo.toml --lib`.
Verify the complete Tauri app with installed and missing Wrangler. Include a
screenshot of the setup and dependency states in the review.

## Implementation verification

- Frontend build and all 322 frontend integration tests passed.
- CLI: 29 unit tests and 11 integration tests passed. Infrastructure: 558 passed, 5 ignored.
- Tauri: 308 passed, 6 ignored.
- POC: 21 tests passed with `--test-threads=1`. The first parallel run
  encountered an ephemeral callback port collision.
- The production Wrangler runtime check passed with the installed Homebrew
  executable and with a minimal desktop PATH. This check used a temporary home
  and did not authenticate or contact R2.
- Native Tauri checks used a temporary home and a fixture CLI. Verified missing
  dependency help, copying the install agent message, locked form controls,
  consent timeout, explicit login arguments, and immediate draft adoption.
- The final native retry loaded the same draft and enabled review. It did not
  commit a storage. The fixture applications were removed after the checks.
- Cancellation checks covered early requests, revision matching, helper and
  child process termination, and retry. The bundled macOS app passed an
  isolated native check: Cancel stopped both fixture processes, kept the draft,
  and allowed the same draft to reach review on retry. The accessibility tree
  confirmed the states. The new screenshot capture omitted the form, so it
  is not included as visual evidence.
- Native screenshots are under `output/r2-wrangler-qa/`: `setup.png`,
  `missing-wrangler.png`, and `ready.png`. They show simulated responses.

The live Cloudflare setup and capability probe were completed in the earlier
POC. This implementation pass did not repeat cloud provisioning. Windows
executable handling has not been verified on a Windows host.


## Account confirmation

The setup row now checks Wrangler accounts before bucket creation. It shows
one account directly, or requires a selection when several accounts are
available. Continue starts provisioning for the selected account. Change
login starts Wrangler with `--browser=false` and provides a link the user can
copy into another browser profile. Cancel covers both command phases.

The desktop uses `get_r2_setup_accounts` and passes the selected account ID
to `setup_r2_storage`. The CLI checks membership again before provisioning.
Account labels are bounded; tokens and other login fields are not returned.
The existing journal still rejects a change of account after provisioning
starts. No new dependency or storage schema is added.

Account-flow verification passed the frontend build, 322 frontend tests, 29
CLI unit tests, 11 CLI integration tests, and 308 Tauri tests (6 ignored).
The native app and CLI built. Native UI automation returned
`cgWindowNotFound`, so this account-flow change has no new native screenshot
or completed visual check. The isolated QA app was closed and removed.


## Copy authorization links

Wrangler login runs with `--browser=false`. Both login and bucket consent
emit bounded `authorization_link` JSON records while waiting. The desktop
validates each URL and delivers it only to the matching active setup row.
Copy link is always available for a pending browser step; Open browser is
optional. Completion, failure, timeout, and cancellation clear the link.
The dashboard-switch step is removed because the link can be used directly
in the required browser profile. Links are not persisted or logged.

Verification: frontend build and 327 frontend tests passed; 30 CLI unit tests
and 12 CLI integration tests passed serially; Tauri passed 310 tests with 6
ignored. Tests cover streaming before process exit, URL validation, output
bounds, cancellation, and a stdout-lock regression. The installed Wrangler
returned a valid URL with `--browser=false` using a temporary home and no
authorization. A parallel CLI run had a callback-port collision; the serial
suite passed. Native visual verification remains blocked by
`cgWindowNotFound`; the isolated QA app was closed and removed.

# Dropbox agent setup POC

This POC tests creation of a new Mallard shared folder after browser consent.
It resolves the namespace ID and obtains an access token without manual
token entry. It also tests a refresh grant. It does not change the app.

The plan is [Agent setup for Dropbox](../../docs/PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md).

The local mock and live POC pass. The live run on 2026-09-07 verified one
account, shared folder creation, namespace access, and token refresh. All
40 Node tests and 17 Rust application tests pass. Production integration now
uses the Rust CLI, callback, provisioning journal, and refresh transport in
the [implementation plan](../../docs/PLAN_AGENT_ASSISTED_DROPBOX_SETUP.md).
This Node helper remains a standalone protocol POC.

## Run the local mock

Use Node 20 or later. No package installation is required.

```sh
node poc/dropbox-agent-setup/setup-poc.mjs mock
node --test poc/dropbox-agent-setup/setup-poc.test.mjs
cargo test -p mallard-application --lib dropbox_agent_poc
```

The mock uses two loopback HTTP servers: a fake Dropbox server and the real
POC callback listener. All credentials are synthetic. It makes no external
network request and does not read provider homes or Mallard state.

The Node suite checks 40 cases. It covers callback state, PKCE, duplicate
callbacks, host checks, denial, synchronous and asynchronous sharing, strict
creation, namespace identity, refresh, permission errors, lost responses,
redirect rejection, response bounds, and timeouts. The Rust test consumes the
same synthetic config fixture and prepares an exact draft in a temporary
home. It does not commit a storage.

The live run showed that sharing changes the ordinary folder ID. The final
check uses the acknowledged shared namespace and exact mount path. Tests
accept this ID change while rejecting a replacement namespace or path.

## Prepare a live test

Use the Dropbox account selected for the test. In the Dropbox
[App Console](https://www.dropbox.com/developers/apps), create a scoped app
with **Full Dropbox** access. Enable:

- `account_info.read`
- `files.metadata.read`
- `files.content.read`
- `files.content.write`
- `sharing.write`

Register this exact redirect URI:

```text
http://127.0.0.1:53682/dropbox/callback
```

Disable implicit grant if it is enabled. Use the public **App key**, not the
App secret. Do not put tokens or an app secret in chat, arguments, environment
variables, or files. The production feature will use one registered Mallard
app with its public app key packaged in the CLI. These are one-time
maintainer steps. Normal users will not need App Console setup, a copied
prompt, or any typed setup value. The planned **Run with Codex/Claude** action
starts the helper directly. Dropbox can still require login, MFA, and consent.

## Run the live POC

This command creates one remote folder named `Mallard POC <runId>` in the
authorizing user's home, then converts that exact folder into a shared
folder. This creates a separate namespace. The selected storage namespace
is not the personal home namespace. The POC does not create a managed team
folder, add members, send invitations, or create a public link.

```sh
node poc/dropbox-agent-setup/setup-poc.mjs live --app-key PUBLIC_APP_KEY
```

The helper opens the system browser on macOS or Linux. The user completes
login, MFA, and consent. The agent waits for the command result. It must not
read or control the browser. Windows browser launch is outside this POC.

The callback port must be free. If it is occupied, the helper stops before
opening the browser. It does not choose a different unregistered port.
Consent has a three-minute limit. Each Dropbox request, including its body
read, has a 15-second limit. A share job has at most 30 polls. There is no
automatic retry of a failed API request.

Successful output contains `verdict: "POC_PASS"`,
`productionGate: "INCOMPLETE"`, `mode: "live"`, and a random `runId`.
Output contains no credentials, namespace IDs, account IDs, or paths.

On failure, `remoteFolderMayExist: true` means a creation request was sent.
It does not prove that creation succeeded. The helper does not repeat an
uncertain create or share request. Use the reported `runId` to identify the
test folder in Dropbox.

If the result is `required_scope_missing`, the `missingScopes` list contains
only names from the helper's fixed permission list. Enable those permissions
in App Console, click **Submit**, then repeat the live command and consent.
This check runs before folder creation for the initial authorization.

For read-only diagnosis of an existing POC folder, use its reported run ID:

```sh
node poc/dropbox-agent-setup/setup-poc.mjs inspect \
  --app-key PUBLIC_APP_KEY --run-id REPORTED_RUN_ID
```

Inspection performs browser consent, metadata reads, token refresh, and a
namespace listing. It does not create, share, or delete a folder. Its output
contains only field-presence and identity-equality checks, plus the run ID.

## What remains after a run

The remote folder remains. The helper never deletes a folder. Remove the
empty test folder manually when you finish. Revoke the test app's connection
in Dropbox settings when it is no longer needed.

Access and refresh tokens stay in process memory and are not saved. Process
exit releases the references; JavaScript does not guarantee memory zeroing.
The generated config is checked in memory only. A live pass does not add a
Mallard storage or provide a durable connection after the POC exits.

## Scope of the result

This POC tests authorization and provisioning. The existing
[two-account storage POC](../dropbox-shared-team-namespace/README.md) tests
the storage protocol with pre-issued credentials. They test separate gates.

The live result covers the selected account and app. It does not test
second-account access, managed team policy, production refresh persistence,
a CLI-to-Tauri draft event, restart recovery, file uploads, or packaged
desktop behavior. The production Dropbox upload limit remains 150 MiB per
object. Schemas remain 1.

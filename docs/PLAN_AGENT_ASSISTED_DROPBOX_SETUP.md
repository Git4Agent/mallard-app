# Agent setup for Dropbox

Status: implemented. Frontend, CLI, and workspace library tests pass.
The authorization POC passed with one live account. Native macOS verification
uses an isolated home and a local HTTP mock. Windows and full live transport
checks remain release checks.
Updated: 2026-09-07.

## Product behavior

Dropbox uses the same setup card, agent launch controls, manual route, and
exact-draft watcher as R2. Normal agent setup needs no typed setup values.
Mallard supplies the default name, public app key, permissions, callback,
CLI path, draft ID, and revision.

1. Select **New storage → Dropbox**.
2. Select **Run with Codex** or **Run with Claude Code**. Copying the
   instruction is an optional route.
3. Complete Dropbox consent in the system browser. Dropbox can require
   login or MFA. The agent does not read or control the browser.
4. The helper creates a new Mallard shared folder and prepares the exact
   draft. Mallard shows automatic access renewal; **Connected** appears
   only after a request to Dropbox has succeeded in this session.
5. Review the connection, optionally test it, and select **Finish setup**.

The helper creates no invitations, members, public links, or managed team
space. A new folder initially has its owner only. Creating another folder
creates another storage. Machines that share data must use the same namespace.

Manual setup remains available for an existing shared or team namespace.
It accepts a namespace ID and user-linked access token. A manual token must
be replaced when it expires.

For an existing connection, the agent control reads **Reconnect with
Codex/Claude** and its instruction tells the agent it is regaining access
for the existing namespace with the same CLI command; the CLI reports
`reconnected` in its result. It creates no folder. The user selects **Save
changes** to commit that credential change. OAuth namespace fields are
read-only. A rejected or expired credential suspends the storage's shared
transport, marks it **Reconnect** in every listing, and leads to this page;
it never selects another token. See
[the reconnection plan](PLAN_DROPBOX_RECONNECTION.md).

## Implemented paths

| Layer | Implementation |
| --- | --- |
| Provider | Declares the assistant, manual access token, and managed refresh token |
| Frontend | Reuses the R2 card, direct launch, draft watcher, and review actions |
| CLI | Pins the exact draft before browser or remote work |
| OAuth | Public app key, PKCE S256, random state, registered loopback callback |
| Provisioning | Strict folder creation, sharing by acknowledged ID, bounded job polling |
| Recovery | Atomic schema-1 journal with account and acknowledged folder state |
| Transport | Manual token or coordinated refresh before request dispatch |
| IPC | Exact new-draft fetch returns the saved token for review, as in R2; other responses carry presence only |

The agent runs this command; the user does not type it:

```text
mallard-cli storage dropbox setup --name "Dropbox storage 1" \
  --draft-id <exact-id> --expected-revision <exact-revision>
```

There is no external Dropbox CLI dependency or configurable production API
host. Success returns only a fixed draft-ready result. No token, destination,
HTTP response, or browser content enters CLI output. The helper does not
commit storage. The shared application service retains revision, name,
provider, host-operation, and source-target checks.

## Authorization and credentials

The registered Dropbox app is `mallard-app`, with public app key
`ex00srwhe5f5m5m`. It uses Full Dropbox access. Its public client key
is packaged in Mallard. An app secret is not used. App Console registration,
permissions, and callback setup are one-time maintainer tasks. End users do
not create their own app or enter authorization codes.
[Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide).

New-folder setup requests:

- `account_info.read`
- `files.metadata.read`
- `files.content.read`
- `files.content.write`
- `sharing.write`

Reconnection and refreshed access request the four storage scopes without
`sharing.write`. Full Dropbox permission covers more than the new folder.
The path-root header selects a namespace; it does not restrict the grant to
that namespace. The setup card states this scope of access. A narrower access
token does not remove permissions from the underlying refresh grant.
[Dropbox namespace guide](https://developers.dropbox.com/dbx-team-files-guide).

Keep all local and portable schemas at **1**, including Dropbox config
version **1**. Settings remain `{ namespace_id }`. The existing secret map
contains exactly one credential:

| User option | Secret | Behavior |
| --- | --- | --- |
| Manual | `access_token` | Use the supplied token; report expiry |
| Agent | `refresh_token` | Obtain and cache short-lived access |

Mixed and unknown secrets fail validation. An exact new-draft fetch returns
the saved refresh token to the local editor. It uses the same Show/Hide and
replacement controls as R2, with the label Refresh token. Manual setup shows
Access token. Listings, update responses, and edit drafts of committed storage
return presence only. Tokens stay out of the clipboard, assistant prompt,
logs, and provisioning journal. A managed connection permits refresh-token
replacement and rejects access-token substitution and namespace edits.

Each open transport shares one refresh lock. Expired cache entries are
removed before refresh. A refresh failure never uses the expired entry or
another credential. Remote mutations are not replayed after an unknown
result. Namespace identity, immutable publication, SHA-256 checks, and
revision-based writes and deletion remain unchanged.

Keep the **150 MiB (157,286,400 bytes) per-object Dropbox upload limit** and
full-input preflight. The shared transport now uses bounded upload sessions
and batch commit, including setup probe writes. Session IDs are not persisted.
There is no splitting, migration, old-schema reader, or legacy execution path.
Other storage limits do not change. See the
[push performance plan](PLAN_DROPBOX_PUSH_PERFORMANCE.md).

## Provisioning recovery

One provisioning ID is derived from the exact random draft ID. Its remote
folder name is stable across retries of that draft. The private journal uses
the existing bounded, atomic, no-follow journal store in a separate directory.
It contains no credential and remains schema 1.

| Last durable stage | Next action |
| --- | --- |
| No mutation started | Create the exact folder with autorename disabled |
| Folder ID acknowledged | Verify that exact folder, then share it |
| Sharing job acknowledged | Poll the same job |
| Namespace acknowledged | Verify the exact mount path and namespace |
| Create/share outcome unknown | Stop without repeating the mutation; retain recovery state |

Dropbox can change the ordinary folder ID during sharing. The acknowledged
shared namespace is therefore the final identity. The helper checks the exact
mount path against that namespace. A same-name replacement with a different
namespace is rejected. It never claims an unknown mutation from a name match.

A different account cannot resume another account's journal. After the exact
draft is prepared, the provisioning journal is removed. A stale form edit is
preserved; its acknowledged folder can be resumed from a fresh instruction.
Cancellation or failure retains remote folders. Deletion remains opt-in.
An unknown mutation requires inspection in Dropbox before a new setup.

## Verification

- `npm run build`: passed.
- `npm run test:frontend-integration`: 290 passed.
- `cargo test --workspace --lib`: 883 passed, 5 ignored.
- `cargo test -p mallard-cli`: CLI unit and process integration tests passed.
- Native macOS build and UI: passed with an injected temporary home and local
  Dropbox mock. The initial and connected cards were inspected at default and
  expanded window sizes. The connected card keeps the credential field with
  Show/Hide and replacement controls, locks the
  namespace, and enables Test connection and Finish setup.

Native tests that patch home resolution must use a separate source tree and
a separate Cargo target directory. Do not share the normal `target` directory:
Cargo can reuse a test library in the desktop app or CLI. After rebuilding a
normal CLI, run its process tests with temporary homes to verify draft lookup.

The external agent was not launched during native UI testing. Automatic
approval review rejected the Copy instruction menu click as a possible Claude
Code launch. The local mock verified the persisted prepared state instead.

Tests cover callback state, duplicate parameters and headers, request bounds,
denial, timeouts, PKCE, scope rejection, redirect rejection, refresh expiry,
revocation, concurrent refresh, strict creation, changed folder IDs, namespace
replacement, synchronous and asynchronous sharing, restart, unknown mutation
results, stale drafts, hidden credentials, reconnection, and refreshed-token
storage probes and verified object operations.

The [Node authorization POC](../poc/dropbox-agent-setup/README.md) remains a
standalone protocol reference. Its 40 tests and one-account live run passed.
The live run verified creation, namespace access, and refresh on 2026-09-07.
It left three empty POC folders and saved no credential or committed storage.
Production code uses the Rust implementation; it does not execute the Node POC.

Before public release, complete Dropbox app approval, packaged Windows checks,
and live project/Library Push/Pull and reconnection. Shared access with a second
account and managed team policy remain separate live checks.
[Dropbox developer guide](https://www.dropbox.com/developers/reference/developer-guide).

## Research decision

Use the Mallard helper. Stock [dbxcli authorization](https://github.com/dropbox/dbxcli/blob/master/cmd/auth.go)
still asks for a pasted code. Do not copy its app identity or credential cache.
Dropbox MCP supports automatic registration for trusted clients, including
Codex and Claude Code. Its documented tools do not establish the required
shared-namespace creation and credential handoff to Mallard. This conclusion
is based on the documented tool list; no connector credentials are extracted.
[Dropbox MCP documentation](https://help.dropbox.com/integrations/connect-dropbox-mcp-server).

## Code

- [OAuth and refresh](../crates/mallard-infrastructure/src/dropbox_auth.rs)
- [Provisioning and recovery](../crates/mallard-infrastructure/src/dropbox_setup.rs)
- [Native callback](../crates/mallard-cli/src/dropbox_browser_setup.rs)
- [CLI draft preparation](../crates/mallard-cli/src/backend.rs)
- [Dropbox adapter](../crates/mallard-infrastructure/src/storage_providers/dropbox.rs)
- [Draft and IPC service](../crates/mallard-application/src/storage_setup.rs)
- [Shared setup form](../src/components/project-sync/StorageSettings.tsx)
- [Agent launch and watcher](../src/components/project-sync/ProjectLinksWorkspace.tsx)

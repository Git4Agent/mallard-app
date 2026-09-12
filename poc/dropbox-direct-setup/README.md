# Direct Dropbox setup POC

This POC calls the existing Rust Dropbox setup backend directly. It uses no
agent, Dropbox CLI, app secret, or manual code entry. The user completes
Dropbox login, MFA, and consent in the system browser.

The POC is separate from the desktop UI. It creates a draft in a temporary
Mallard home. It does not save a storage connection in the user's Mallard
configuration.

## Run

```sh
cargo test -p mallard-cli --example dropbox_direct_setup
cargo test -p mallard-infrastructure --lib dropbox
cargo run -p mallard-cli --example dropbox_direct_setup -- setup --probe
```

The live command creates one new Dropbox folder named `Mallard <word>`.
The production name allocator chooses the word. It selects another word only
after Dropbox confirms a name collision. The draft display name defaults to
`Dropbox storage 1`. No name, namespace ID, code, or token is typed by the user.

The command uses the public app key already registered for `mallard-app`.
It opens browser consent with PKCE S256 and offline access. Dropbox returns
the code to `http://127.0.0.1:53682/dropbox/callback`. The backend checks the
callback state and exchanges the code for access and refresh tokens.

The backend creates the folder, converts it into a shared folder, checks its
namespace and ownership, and verifies token refresh. It adds no members,
invitations, or public links. It then fills the exact draft with the namespace
and refresh credential. The usual review API can read that draft. Background
draft lists keep the credential value hidden.

`--probe` runs Mallard's normal storage capability check. This writes, reads,
and removes probe objects in the new folder. Omit the flag to test setup only.

## Consent delay

The current browser callback waits for 180 seconds. A user can complete
consent during that interval. On timeout, the command stops and releases the
callback port. Start a new attempt to open a fresh consent request. There is
no manual code fallback. An occupied callback port stops setup before the
browser opens.

Dropbox API calls have a 15-second limit. Folder sharing can require an
asynchronous job with up to 30 status checks. The backend does not repeat an
uncertain create or share request.

## State after a run

Successful output contains fixed status fields and booleans. It contains no
OAuth URLs, codes, credentials, account IDs, namespace IDs, or folder paths.
The POC verifies that no storage was committed, then removes its temporary
home and credential files before it reports `REVIEW_READY`.

The new remote folder and Dropbox app authorization remain. The command does
not delete the folder or revoke the app. It does not copy project or provider
data. If setup fails after consent, inspect Dropbox before starting another
creation attempt. This POC removes its temporary recovery journal on exit.

## Verification

The example reuses the production callback and application preparation. Two
tests run the complete setup with a delayed loopback callback and a local
Dropbox API fixture. They check automatic naming, PKCE, refresh, review
population, and denial before any remote operation. All fixture state uses
temporary homes.

On 2026-09-10, all 22 example tests and all 51 Dropbox infrastructure tests
passed. The example also includes existing R2 tests because it compiles the
shared CLI backend. No R2 flow runs during the live Dropbox POC.

The first live attempt on 2026-09-10 stopped with
`dropbox_authorization_timeout`. No browser callback arrived within 180
seconds. The attempt made no token exchange or folder creation request. Its
temporary state was removed. Live verification needs a fresh browser consent
attempt; no live success is claimed.

## Product change supported by this POC

The smallest integration is a **Connect Dropbox** action that calls the
bundled setup helper directly, then reloads the draft for review. The current
OAuth, folder naming, provisioning journal, refresh transport, and explicit
save step can stay in place. Dropbox needs no Wrangler installation step.

Dropbox documents desktop PKCE, redirect callbacks, and refresh tokens in its
[OAuth guide](https://developers.dropbox.com/oauth-guide). The registered app
uses Full Dropbox access. Its permission extends beyond the new folder; the
selected namespace limits Mallard's storage destination, not the OAuth grant.

## Direct desktop flow regression

Run `npm run dev` and open
`http://localhost:1420/poc/dropbox-direct-setup/flow-check.html`.
The fixture runs the production React hook with synthetic APIs. All four
checks must show **PASS**. It checks create, discovery, reconnect, double
clicks, and late consent after cancellation. No native command or Dropbox
request runs.

The draft revision check first failed with the shared save helper: each step
saved a new revision after consent. The desktop flow now saves once at login
and pins that revision through review. The three complete flows pass with
that change. A new authorization attempt saves a fresh revision.

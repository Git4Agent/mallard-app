# Feishu CAS-free POC rerun

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Date: 2026-09-23 (America/Los_Angeles). Data: synthetic only.

The local checks pass under the current CAS-free publication model.
The live Feishu Open API run remains pending. This run does not qualify
Feishu as a production storage provider. The provider is now implemented
against a loopback mock ([design](DESIGN_FEISHU_DRIVE_STORAGE.md)); its
live rows are part 6 of [the two-machine pass](LIVE_TWO_MACHINE_PASS.md).

## Scope and source

The experiment uses `list -> write record -> list`, immutable record names,
SHA-256 checks, and the current Rust `RecordName` and `Mainline` code.
The Feishu path has no parent claim, mutable head, CAS request, automatic
resend, or automatic rollback. The separate STABLE design was not tested.

Base commit: `a49c7eb697b00f018099366664027de699cfa7bd`.
Tests used the working files, including existing uncommitted publication
changes. This is not a test of the base commit alone. Node: `v22.22.0`.
Cargo: `1.96.0 (30a34c682 2026-05-25)`.

## Results

| Experiment | Result | Evidence boundary |
| --- | --- | --- |
| Feishu Node suite | 45 passed, 0 failed | Mock transport, OAuth, and desktop verifier regression |
| Rust domain publication suite | 9 passed, 0 failed | Production record parser and selection code |
| Rust infrastructure publication suite | 21 passed, 0 failed | Production publication helper; includes stores with and without claims |
| Standalone mock sequence | `PROBE_PASS`, 249 simulated HTTP requests | 11 transfer, pagination, integrity, and selection checks |
| Sibling wins before confirmation | `remote_changed`; one record request; losing bytes retained | New deterministic mock race |
| Delayed admitted writer | Confirmed selection later displaced; both clients converge | New deterministic mock race with no hidden entries |
| Live Feishu Open API | Pending direct OAuth | Waiting for hidden terminal secret entry |
| Live Feishu desktop round trip | Not rerun | The earlier eight-file result remains historical evidence |

The Node count comprises 33 transport/protocol tests, 2 token-form tests,
9 OAuth tests, and 1 desktop verifier test. The mock sequence checks the
marker, small and multipart payload hashes, manifest-before-record order,
223-character record names, all listing pages, two client objects, sibling
selection, retained losing records, duplicate bytes, and abandoned payloads.
Its client objects are not independent live users or machines.

See the [saved result summary](../poc/feishu-drive/evidence/cas-free-2026-09-23.json)
and [race tests](../poc/feishu-drive/feishu-drive.test.mjs).

## What the new race establishes

1. Writer A lists root R and prepares an earlier-ranked child A1.
2. Writer B lists R, writes B1, and confirms B1 as selected.
3. A sends A1 within the ten-second admission window.
4. Both clients now select A1. B1 remains stored with unchanged bytes.

Every completed write is visible in each later listing in this fixture.
There is no hidden record, retry, or concurrent HTTP call. Thus complete
listing alone cannot make a selected no-claim publication permanent.
The existing hidden-sibling experiment remains a separate visibility test.
Neither experiment measures live Feishu consistency.

Current `observe_linked_bundle` classifies displaced acceptance without
writing local state. Recovery requires user action. The earlier reports'
automatic-rollback statement has been corrected. This rerun inspected that
source behavior; it did not run the full Tauri recovery workflow.

## Probe and production differences

The JavaScript publication helper tests transport behavior. It calls the
production Rust parser and selector, but not production `write_record`.
Its record and manifest bodies are fixtures, not complete Mallard DTOs.

The probe fails an acknowledged record absent from a later listing with
`record_visibility_gap`. Production can retain its own acknowledged record
in memory. If a send fails and a successful settlement listing omits the
record, the probe reports `outcome_unknown`; production returns the send
error. Neither sends the record again. Production can repeat a stale
admission listing; the probe fails that case before sending.

## Reproduce

```sh
node --test poc/feishu-drive/feishu-drive.test.mjs poc/feishu-drive/desktop-fixtures.test.mjs poc/feishu-drive/browser-probe.test.mjs poc/feishu-drive/oauth-probe.test.mjs
node poc/feishu-drive/probe.mjs mock
cargo test --offline --locked -p mallard-domain publication::tests --lib
cargo test --offline --locked -p mallard-infrastructure storage_core::publication::tests --lib
```

The [live procedure](../poc/feishu-drive/README.md#prepare-a-live-run) uses a
private credential file outside the repository, or direct OAuth with a
hidden terminal prompt. It writes synthetic data to a new child folder
and retains that folder. The prior API Explorer authentication failure
was not retried. No live upload or deletion ran in this rerun.

The user selected direct OAuth. The current developer console confirms
that the existing Mallard app is enabled and its local callback is still
registered. All four Drive scopes are enabled for user identity. The
computer-use tool blocks Terminal access for safety
reasons. The user must start the supplied command and enter the App Secret
at its hidden prompt. The prior test-folder token was recovered from the
earlier run; current folder access still requires the API probe.

The official listing page returned no readable content through the web
reader; the upload page fetch failed. No new provider contract claim or
quota verification is based on those fetches. Production OAuth, live
multipart, cross-client visibility, folder races, capacity, and full
ProjectSync/Library flows remain unverified.

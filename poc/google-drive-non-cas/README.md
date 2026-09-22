# Google Drive non-CAS proof of concept

Status: local proof passed on 19 September 2026. Live Google testing is pending.

This probe tests a Drive API adapter for Mallard's current immutable
publication model. It uses a fixed folder ID, binary files, and public
metadata hashes for logical keys and prefixes. It has no mutable head or
global key-to-ID index.

Read the [research report](../../docs/GOOGLE_DRIVE_NON_CAS_POC_2026-09-19.md)
for the design, official sources, consistency limits, and release conditions.

**Run locally**

Use Node.js 22 or later and `rustc`. No npm package is required. Run these
commands from the repository root:

```sh
node poc/google-drive-non-cas/drive-poc.mjs mock
node --test poc/google-drive-non-cas/drive-poc.test.mjs
```

The suite has 28 tests. One test compiles the production Rust record parser
and selector in a temporary directory. It compares 128 generated histories
with the JavaScript model. It does not compile the complete application.

The mock command reports:

```json
{
  "evidence": "in_process_http_mock",
  "result": {
    "marker": "verified_by_second_client",
    "payload": "sha256_verified_by_second_client",
    "publication": "same_selection_after_both_writes",
    "losingSibling": "remote_changed",
    "duplicateKey": "identical_copies_read",
    "conditionalWrites": 0,
    "limitations": [
      "finite_observation_not_consistency_guarantee",
      "synthetic_records_not_bundle_engine"
    ]
  },
  "requests": 41,
  "liveGoogleDrive": "not_run"
}
```

Run the reused HTTP helper's existing tests with this suite:

```sh
node --test poc/google-drive-non-cas/drive-poc.test.mjs \
  poc/storage-discovery/storage-discovery.test.mjs
```

Result: 63 passed. These include bounded response reads, timeouts, redirect
rejection, and sanitized transport errors.

Run the production selector's own tests without the application build:

```sh
rustc --edition=2021 --test poc/google-drive-non-cas/select-mainline.rs \
  -o /private/tmp/mallard-google-drive-native-tests
/private/tmp/mallard-google-drive-native-tests
```

Result: 7 passed. The harness imports the current working-tree source at
`crates/mallard-domain/src/publication.rs`. It provides that module's small
SHA-256 string validator. It does not include the storage engine.

**Run a live probe**

The live command creates one new folder under the specified parent. It writes
six small synthetic files into that folder. It leaves the folder in place.
It does not read local provider homes, change Mallard settings, share files,
update existing files, or delete anything.

Prepare a Google Cloud OAuth app with the Drive API enabled. Obtain two
short-lived OAuth access tokens for the same app. For the first test, use
the same account and a dedicated My Drive parent folder accessible to both
tokens. Request `drive.file` and grant the app access to that parent.
See the report for the official authorization references.

Save this JSON outside the repository, in a regular file with mode `0600`:

```json
{
  "client_a_token": "<first short-lived OAuth access token>",
  "client_b_token": "<second short-lived OAuth access token>",
  "parent_id": "<exact parent folder ID>",
  "space": "drive"
}
```

Do not place tokens in command arguments or send them in chat. Use a canonical
absolute file path. The reader rejects symlinks, hard links, broad file
permissions, unknown fields, equal tokens, and files that change during read.
The token file reader supports POSIX systems. Windows ACL checks are absent.

```sh
node poc/google-drive-non-cas/drive-poc.mjs live \
  --credentials /private/tmp/mallard-drive-poc-credentials.json
```

The exact folder ID, account details, tokens, and provider error bodies do
not appear in output. The report includes the generated folder name:

```text
mallard-drive-poc-<24 hexadecimal characters>
```

If a write fails, the result can be uncertain. Inspect only this generated
folder. The PoC has no cleanup command. A retained probe folder is not an
application storage configured by Mallard.

For a Shared Drive experiment, add `shared_drive_id` and use a parent folder
in that drive. For an app-data experiment, use `space: "appDataFolder"` and
an accessible app-data parent. Query construction for these modes has local
coverage. Their live permissions, folder handling, and visibility are unproved.
App-data content cannot be inspected or removed through the normal Drive UI.

Two distinct tokens do not prove two physical machines or different users.
The probe does not inspect token scopes or validate an ID token. Record those
conditions separately. A successful live run proves only that run's finite
observations.

**What the tests establish**

- Exact binary and zero-byte round trips with local SHA-256 verification.
- Duplicate logical keys with equal bytes, and rejection of unequal bytes.
- Isolation by root folder and logical prefix.
- Project and Library stream separation.
- Complete pagination or an explicit error.
- One record POST per attempt, with another listing before a retry.
- Lost-reply recovery, stale-parent rejection, and unknown outcomes.
- Deterministic selection against the production Rust selector.
- Two counterexamples: hidden remote records and a delayed earlier sibling.

The delayed-sibling test passes when it reproduces a change in selection
after an earlier reported success. It records a limitation. It is not a
claim of permanent publication acceptance.

**Scope and files**

| File | Purpose |
| --- | --- |
| [drive-store.mjs](drive-store.mjs) | Real Google API request construction; bounded put, get, and list |
| [publication-model.mjs](publication-model.mjs) | Executable model of record selection and list-before-write |
| [mock-drive.mjs](mock-drive.mjs) | In-process HTTP simulation and fault controls |
| [drive-poc.mjs](drive-poc.mjs) | Mock and live commands; private credential file reader |
| [drive-poc.test.mjs](drive-poc.test.mjs) | Adapter, protocol, and native parity tests |
| [select-mainline.rs](select-mainline.rs) | Isolated harness for the current production selector |

Limits: 4 MiB per payload, 64 KiB per record, 32 copies per key, 1,000 pages,
100,000 listed entries, 25 key components, and 15 seconds per HTTP request.
This PoC buffers payloads. Production must stage larger reads to disk.

The application does not import these files. No provider registry entry,
OAuth UI, resumable upload, persistent ID cache, bundle manifest processing,
restore, or storage clear is implemented. Local tests do not prove Google
search consistency or authorization behavior.

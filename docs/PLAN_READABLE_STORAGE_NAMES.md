# Readable storage names

Status: implemented and verified locally. Date: 2026-09-10.

Replace generated R2 bucket names and Dropbox folder names with one readable
word. After 10 word conflicts, add a numeric suffix. Use an existing library
for the word list. Detect name conflicts during creation and advance the name
when the provider confirms a conflict.

## Product decision

| Provider | Format | Example |
| --- | --- | --- |
| Cloudflare R2 | `mallard-<word>` | `mallard-otter` |
| Dropbox | `Mallard <word>` | `Mallard otter` |

The storage display name remains a separate, editable label. It does not select
the remote destination. A new setup draft selects a new candidate. A retry uses
the candidate saved in that draft's setup journal.

One word is sufficient for the expected small number of storage locations per
account. The word is a readable label, not a security identifier. Creation and
identity checks prevent accidental reuse of another destination.

## Clean cutover

- Keep every schema version at `1`, including local state, configuration,
  drafts, setup journals, and portable manifests.
- Change affected setup-journal fields directly. Add no migration, old-format
  fallback, or legacy execution path.
- Remove the R2 hash-based naming function and the Dropbox draft-hash folder
  naming rule. Remove tests that require those names.
- Support reconnect for storage created by the new implementation through its
  saved bucket address or Dropbox namespace ID.
- The user will remove `~/.mallard` and cloud data before manual retesting.
  Implementation work must not delete those resources.

## Word list and shared helper

Use Rust `petname` 3.2 in `mallard-infrastructure`, with command-line features
disabled and built-in word lists enabled. Combine its small English noun,
adjective, and adverb lists. Keep distinct words with 3–10 lowercase ASCII
letters. The pinned release provides **1,155** candidates after filtering.
Examples include `fox`, `brave`, and `gently`.

`storage_names.rs` selects a candidate with the existing `getrandom` dependency.
Selection is unbiased and excludes rejected words. The helper formats provider
names and advances numeric suffixes. It performs no network calls or writes.
See the [library repository](https://github.com/allenap/rust-petname) and
[English API](https://docs.rs/petname/latest/petname/lang/english/struct.Petnames.html).

## Creation and recovery

Reuse the existing setup-journal infrastructure. Add R2 provisioning state and
update Dropbox provisioning state. Save the selected name, creation stage,
attempt count, and rejected candidates. Bind the journal to the exact setup
draft and provider account. Preserve operation locks and draft revision checks.

| Result | Required action |
| --- | --- |
| Candidate selected | Save the full name before any creation request. |
| Creation succeeded | Save the returned identity or verified destination before the next setup step. |
| Confirmed name conflict | Save the rejection, advance the word or numeric suffix, and attempt creation. |
| Permission, authentication, or other failure | Report the specific failure. Do not select another word. |
| Timeout or unknown result | Keep the current attempt. Stop automatic creation. |
| Ten words rejected | Keep the last word and try suffix `-2`, then `-3`, and continue. |

Try 10 distinct words before adding numbers to the last word. For example,
`mallard-otter` becomes `mallard-otter-2`, then `mallard-otter-3`. Dropbox uses
`Mallard otter-2` and `Mallard otter-3`. Continue on confirmed conflicts instead
of reporting an error after the tenth word.

Save at most 10 rejected words. Save the current complete name and its conflict
stage to resume numeric allocation after a restart. The rejected-name list
must not grow with the numeric sequence. Other errors stop the current call;
they do not advance the name. A confirmed request rejection permits a later
retry of that same name.

Save the complete name, not only a random seed. Dependency or dictionary updates
must not change a saved destination.

A process restart after a confirmed creation resumes the remaining setup steps
for that destination. A crash between remote creation and the local success
checkpoint leaves an unknown result. Do not infer ownership from the name or
silently create a second destination.

## R2 changes

Remove `ensure_bucket`, the derived hash name, and the duplicate CLI destination
type. An accessible name alone must not authorize reuse.

Keep Wrangler login and browser consent in `r2_browser_setup.rs`. Obtain the
OAuth token through the supported `wrangler auth token --json` command. Capture
it in memory, require the OAuth response type, and never print it or save it in
a journal. No token is passed in process arguments or environment variables.
Older Wrangler versions must report a dependency error; add no fallback. See
[Wrangler token command](https://developers.cloudflare.com/changelog/post/2025-12-18-wrangler-auth-token/).

`r2_setup.rs` owns management API requests and provisioning recovery. The API
client disables redirects and automatic retries, bounds response reads, and
returns fixed errors. A successful lookup rejects an occupied candidate before
creation. If creation returns an explicit failed response with HTTP 400 or
409, a lookup must confirm the occupied name before Mallard advances it. Other
failures do not select another name. A failed lookup alone does not establish
that a bucket is absent.

The management API uses a different response contract from the S3-compatible
API. Wrangler's own helper documents HTTP 400 for an existing bucket. See
[Cloudflare's bucket helper](https://github.com/cloudflare/workers-sdk/blob/main/packages/wrangler/src/r2/helpers/bucket.ts).
A preliminary lookup cannot prevent a concurrent creation, so the creation
response remains part of conflict handling.

Save the confirmed bucket creation date. On recovery, verify the same name and
creation date before continuing. A changed or missing bucket is an identity
error. `backend.rs` coordinates the exact draft, provisioning, browser consent,
and saving the prepared connection.

After confirmed creation, continue credential authorization and prepare the
same draft for user review. Reconnect must use the saved destination.

Primary files: `crates/mallard-cli/src/r2_browser_setup.rs`,
`crates/mallard-cli/src/backend.rs`, and the infrastructure setup-journal code.

## Dropbox changes

Keep `files/create_folder_v2` with `autorename: false`. Preserve and classify its
structured path-conflict response before generic error mapping discards that
detail. An existing file or folder at the candidate path permits another word.
Other path errors do not.

HTTP `409` alone is insufficient: Dropbox uses it for multiple endpoint errors.
Keep bounded response reads and sanitized error evidence. See the
[Dropbox API specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone)
and [error handling guide](https://developers.dropbox.com/error-handling-guide).

After folder creation, retain the existing identity checks and sharing flow.
Save the returned namespace ID. Reconnect must use that namespace and create
no folder. A confirmed conflict must advance the journal safely; it must not
leave the draft in an unknown-result stage.

Primary files: `crates/mallard-infrastructure/src/dropbox_setup.rs`,
`crates/mallard-infrastructure/src/dropbox_auth.rs`, and their tests.

## Implementation sequence

1. Verify the library data and R2 conflict-response contract.
2. Add the shared helper and tests.
3. Implement Dropbox naming, conflict classification, and journal transitions.
4. Implement R2 naming, conflict classification, and journal transitions.
5. Remove obsolete naming paths and tests. Update README setup behavior and
   affected user-facing messages.
6. Run the checks below. Confirm clean-state setup through the full application.

Keep provider-specific creation and recovery logic separate. Do not add a
general provisioning framework for this change. Update Rust and TypeScript
DTOs together if a command contract must change.

## Verification and acceptance

Use temporary homes and mock services for automated tests. Cover:

- Allowed characters, word length, distinct candidates, and actual list size.
- First-attempt success and confirmed conflict followed by success.
- No repeated rejected word; numeric suffixes start at `-2` after 10 word
  conflicts and continue across restarts.
- Authentication, permission, and unrelated errors do not trigger new names.
- A timeout or interrupted creation does not trigger another destination.
- Confirmed creation resumes without repeating creation.
- Two setups selecting the same candidate do not adopt the same resource.
- Dropbox file and folder conflicts are handled; other `409` errors are not
  misclassified.
- Reconnect uses the saved destination and creates no resource.
- All schema constants remain `1`; obsolete naming paths are removed.

Run:

```sh
cargo test -p mallard-infrastructure
cargo test -p mallard-cli
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

For frontend or shared DTO changes, also run `npm run build` and
`npm run test:frontend-integration`. Verify native setup behavior in the complete
Tauri application. Include a screenshot if the UI changes.

Acceptance requires readable names, automatic numeric conflict resolution,
bounded journal data, stable saved identities, safe unknown-result handling,
and a clean cutover under schema `1`.


## Verification results

- `cargo test -p mallard-infrastructure`: 557 passed, 5 ignored; 1 documentation
  test passed.
- `cargo test -p mallard-cli`: 24 unit tests and 11 integration tests passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 300 passed, 6 ignored.
- `cargo build -p mallard`: passed without compiler warnings.
- Native smoke test: the complete Tauri app opened with isolated temporary
  Mallard, Codex, and Claude homes. R2 and Dropbox setup forms loaded. The test
  app was stopped after verification.
- `git diff --check`: passed.

Cloud creation and recovery tests used loopback mock services. Live Cloudflare
and Dropbox consent and resource creation remain for the user's clean-state
retest. No frontend DTO or UI changes were needed. Existing user state and cloud
resources were not deleted.

## Amendment 2026-09-11: the R2 name is chosen in the lane

The R2 half of the conflict rule above is superseded. The proposal is now made
in the desktop process (`propose_bucket_name`, `mallard-<word>`) and shown in
the setup lane's Bucket line in an editable field, with Suggest another beside
it, before anything is created. Setup passes the exact name to the CLI as
`--bucket`; without it, the CLI proposes one itself.

- A confirmed conflict (a lookup hit, or a creation refused and confirmed by a
  lookup) returns `r2_bucket_name_taken`, creates nothing, and leaves the
  journal at `planned` with that name. There is no rotation and no numeric
  suffix for R2; the user renames or takes another proposal and creates again.
- The R2 journal's recovery is the requested `bucket` and its `stage`
  (`planned`, `creating`, `created`). `attempts`, `rejected`, and the
  `conflict` stage are gone. Schema stays `1`; no migration.
- A journal whose stage is `planned` (or empty) follows the latest request:
  a different account or jurisdiction replaces its facts, and a different name
  replaces the plan. `creating` and `created` pin all three, as before.
- Dropbox keeps the word rotation and numeric suffixes described above.

The clean-cutover rules of this plan apply unchanged.

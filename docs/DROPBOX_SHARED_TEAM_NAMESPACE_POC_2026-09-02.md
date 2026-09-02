# Dropbox shared/team namespace proof of concept

- **Date:** 2026-09-02
- **Scope:** Two-account Dropbox shared-folder or team-namespace capability gate
- **Production status:** Not integrated; gate incomplete
- **Offline status:** Pass, 18 tests
- **Live Dropbox status:** Not run; two authorized test actors, one writable
  namespace ID, and one existing probe root are required

## Decision

The offline evidence supports a focused live test of the Dropbox file `rev` as
Mallard's opaque `etag` token. The live two-account test is still pending. Do
not expose Dropbox as a Mallard storage type from the offline result.

This POC accepts pre-issued credentials. It does not prove that broader
authorization for collaborators or managed teams is feasible. OAuth lifecycle,
administrator consent, and authorization availability are separate gates.

The probe is isolated under
[`poc/dropbox-shared-team-namespace`](../poc/dropbox-shared-team-namespace/README.md).
It does not change `StorageConfig`, Tauri commands, frontend DTOs, or the
portable bundle model.

## Safe probe boundary

The operator must create a dedicated `probe_root` in the selected namespace.
Both actors must have write access to it. The harness does not create or delete
folders.

Each run selects one random, 24-hex-character file-name prefix. The harness
checks that the prefix is not in use. It writes only direct child files with
that prefix. It preserves the probe root and all unrelated files.

Cleanup is file-only. Before each delete, the harness downloads the current
file, validates its Dropbox content hash, and compares standard SHA-256 with a
payload that the probe attempted to write. It also requires a `rev` returned
by a probe write or verified ambiguous-write read-back. It then deletes the
file with that `parent_rev`. If the content or revision changed, the harness
does not delete it. It reports `cleanup.status: "required"` and the probe ID.

This boundary avoids Dropbox recursive folder deletion and avoids deletion of
a collaborator replacement at a known probe path.

## Implemented evidence

The harness uses direct Dropbox API v2 requests. It applies one stable
namespace ID to each file request with `Dropbox-API-Path-Root` in
`namespace_id` mode. It supports a team-linked token with two different
`Dropbox-API-Select-User` or `Dropbox-API-Select-Admin` contexts.

It fails unless `/users/get_current_account` returns two distinct account IDs.
It rejects either account's personal home namespace. A selected common team
root is reported as `team_space`. Another writable common namespace is
reported as `collaborative_non_home_namespace`. The harness does not infer a
shared-folder type without namespace inventory evidence.

The probe sequence covers:

| Gate | Probe behavior | Required result |
| --- | --- | --- |
| Two-account access | Both actors list the existing probe root through the same path root | Both succeed |
| Isolated prefix | Both actors list the generated prefix before the first write | No matching file |
| Absent creation | Upload with `add`, `autorename=false`, and `strict_conflict=true` | One exact file |
| Verified read | Second actor downloads; harness verifies bytes, size, standard SHA-256, Dropbox content hash, and `rev` | Exact match |
| Current CAS | Second actor uploads with `update:<current-rev>` | New `rev` |
| Stale CAS | First actor uploads with the old `rev` | Structured HTTP 409 file conflict |
| Late identical stale CAS | Both actors read one `rev`; first updates; second writes the current bytes with the stale `rev` | Structured HTTP 409 file conflict |
| Strict absent conflict | Second `add` of identical bytes | Structured HTTP 409 file conflict; no conflict copy |
| Delete and recreate | Delete with `parent_rev`, recreate, then use the deleted file's old `rev` | Structured HTTP 409 file conflict |
| Ambiguous commit | Discard one successful upload response and recover by metadata plus verified download | Exact committed bytes and current `rev` |
| Listing | Force small Dropbox pages, validate direct child paths, and sort locally | Same sorted generated set for both actors |
| Edge inputs | Zero-byte file and one exact 255-byte file-name component | Exact read and listing |
| Cleanup | Verify known content and delete each generated file with its current `parent_rev` | No generated file remains; no folder delete |

On success, the harness returns `verdict: "POC_PASS"` and
`productionGate: "INCOMPLETE"`. Errors use fixed codes. Results exclude
tokens, account IDs, member IDs, namespace IDs, provider paths, remote bodies,
and request IDs.

## Offline verification

Command:

```text
node --test poc/dropbox-shared-team-namespace/dropbox-namespace-poc.test.mjs
```

Result:

```text
tests 18
pass 18
fail 0
```

The mock transport checks the exact path-root header, actor selection, strict
upload flags, current and stale revision behavior, two-account access,
pagination, the exact 255-byte file-name component, content-verified file
cleanup, and output redaction. It also checks these failure boundaries:

- The probe rejects a provider that accepts a stale revision.
- The probe does not delete a collaborator replacement.
- Every file delete includes `parent_rev`.
- The probe never deletes the existing root folder.
- A 60-second `Retry-After` is honored without a real test delay.
- A `Retry-After` above the live-probe limit fails closed.
- The request deadline covers response-body reads.
- A summary-only conflict cannot satisfy the stale-revision proof.

The retry tests prove local client behavior only. They do not prove live
Dropbox throttling or recovery under load.

## Required live run

Follow the
[`README`](../poc/dropbox-shared-team-namespace/README.md). Use a dedicated
Full Dropbox test app. Create one dedicated probe root in a namespace that two
test actors can edit. Add `namespace_id` and `probe_root` to the local,
mode-0600 credential file.

The minimum user-linked scopes are:

- `account_info.read`
- `files.metadata.read`
- `files.content.read`
- `files.content.write`

A team-linked token also needs `team_data.member` to act for selected members.
A managed team can block the app or require administrator approval. This POC
does not test that authorization path.

Do not share the credential file. Run the probe locally. A live `POC_PASS` is
valid only for the selected actors and namespace, and only when
`cleanup.status` is `complete`. The result will still include
`productionGate: "INCOMPLETE"`.

## Production work after a live POC pass

A later Dropbox adapter can evaluate these mappings:

- `StoredMetadata.etag` to Dropbox `rev`;
- `CasExpectation::Absent` to strict `add`;
- `CasExpectation::Match(rev)` to strict `update:<rev>`;
- Dropbox conflicts to `CasOutcome::Conflict`;
- downloads to the existing private staged-file and standard SHA-256 checks;
  and
- provider enumeration to validated and locally sorted Mallard key pages.

Production integration still requires these separate layers:

1. A tagged machine-local storage destination model. Do not add more
   provider-specific fields to the current flat `StorageConfig`.
2. A machine-local authorization profile with stable account, team, namespace,
   acting-member, scope, and credential-reference data.
3. OAuth code flow with PKCE and refresh tokens. Store refresh tokens in the
   OS credential store. Never return them to the webview.
4. A credential-aware object-store factory used by ProjectSync, Activity, and
   the storage Library.
5. A direct-HTTP Rust adapter with bounded streaming, retry, redaction, nested
   `.mallard/` key mapping, case-alias rejection, ambiguity recovery, and a
   provider-neutral conformance suite.
6. Focused authorize, list-namespace, test, upsert, and disconnect commands.
   The generic config-save path must not create or replace Dropbox
   authorization or destinations.
7. Exact per-provider frontend branches. Existing code treats several
   non-local branches as R2. A new enum value must not use those branches.

Portable bundle schema 1, storage marker schema 1, immutable object keys,
mutable head format, Push, Pull, and review receipts can remain unchanged only
if the production conformance work confirms these mappings.

## Limits

The current harness does not prove:

- OAuth PKCE, refresh, reconnect, revocation, or OS credential storage;
- administrator consent or broad managed-team authorization;
- namespace discovery or team migration behavior;
- nested `.mallard/` object-key mapping;
- live throttling or sustained rate-limit recovery;
- large transfers or upload sessions;
- Rust transport behavior or BundleEngine parity; or
- packaged desktop behavior.

The 255-byte case proves one direct child file-name component only. It does not
prove nested Mallard path behavior.

## Official sources

- [Dropbox team files guide](https://developers.dropbox.com/dbx-team-files-guide)
- [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide)
- [Official Dropbox API files specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone)
- [Dropbox content-hash specification](https://www.dropbox.com/developers/reference/content-hash)
- [Dropbox performance guide](https://developers.dropbox.com/dbx-performance-guide)
- [Dropbox error-handling guide](https://developers.dropbox.com/error-handling-guide)

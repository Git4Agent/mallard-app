# Dropbox shared/team namespace POC

## Result

This harness tests Mallard revision behavior in one Dropbox shared folder or
team namespace. It uses two distinct Dropbox accounts. It does not change the
Mallard production schema or UI.

All 18 offline tests pass. The live Dropbox run has not been done. A live
`POC_PASS` result will show that the selected, pre-authorized actors passed the
protocol checks. It will not prove the full authorization flow or production
readiness. The output therefore includes `productionGate: "INCOMPLETE"`.

## Prepare the probe root

Create a dedicated folder in the selected namespace before you run the probe.
Both test actors must have write access. Put its Dropbox path in `probe_root`,
for example `/Mallard Dropbox POC`.

The harness does not create or delete a folder. It creates files directly in
the existing probe root. Each file has one random, 24-hex-character probe ID
in its name. The harness checks that this generated name prefix is not in use
before it writes data.

The harness preserves the probe root and all files outside its generated
prefix.

## What the live probe tests

The probe uses one stable namespace ID as `Dropbox-API-Path-Root`. It then:

1. Confirms that two distinct accounts can access the namespace and probe root.
2. Reserves one random file-name prefix under the existing probe root.
3. Creates a file only when it is absent.
4. Reads the file through the second account. It verifies the bytes, size,
   standard SHA-256, Dropbox content hash, and `rev`.
5. Replaces the file with its current `rev`.
6. Rejects a stale `rev`.
7. Rejects a late stale update when its bytes equal the current bytes.
8. Rejects a second strict `add` and does not create a conflict copy.
9. Deletes with the current `rev`, recreates the file, and rejects the old
   `rev`.
10. Simulates a lost successful upload response and resolves it by a verified
    read-back.
11. Tests a zero-byte file, one exact 255-byte file-name component, provider
    pagination, local lexical sorting, and cross-account listing.
12. Cleans only files that have the generated prefix and known probe content.

Before each cleanup delete, the harness downloads the file. It validates the
Dropbox content hash and compares standard SHA-256 with content that the probe
attempted to write. It also confirms that the current `rev` was returned by a
probe write or verified ambiguous-write read-back. It then sends
`files/delete_v2` with that `parent_rev`. It never sends a folder delete.

If a collaborator replaces a probe file, the harness does not delete that
replacement. If it finds unexpected or changed probe data, it returns
`cleanup.status: "required"` and the generated probe ID.

## Dropbox app and scopes

Use a dedicated test app with **Full Dropbox** access. App Folder access cannot
target a shared or team namespace.

For two user-linked authorizations, enable these scopes:

- `account_info.read`
- `files.metadata.read`
- `files.content.read`
- `files.content.write`

For a team-linked authorization, also enable `team_data.member`. The team
token must act as two different members with `Dropbox-API-Select-User`, or as
two different administrators where the endpoint permits
`Dropbox-API-Select-Admin`.

This harness accepts pre-issued access tokens. The production design accepts
only a user-linked token from a user who already has edit access. The
team-linked mode remains POC-only. Mallard does not issue or refresh tokens.

## Select the namespace

Use a namespace that both test accounts can edit.

- For a shared folder, use its stable namespace or shared-folder ID.
- For a managed team, an administrator can enumerate namespace IDs with
  `/team/namespaces/list`.
- For a team space, the account response contains the root namespace ID. Both
  selected members must report that same team root.

Do not use either account's personal home namespace. The probe rejects it.

The credential file stores the raw namespace ID. Do not prefix it with `ns:`.
The script serializes the required `namespace_id` path-root mode.

## Credential file

Create a dedicated file outside the repository. Do not send its contents in a
chat message. Do not commit it.

Two user-linked tokens:

```json
{
  "namespace_id": "123456789",
  "probe_root": "/Mallard Dropbox POC",
  "primary": {
    "access_token": "<FIRST_ACCOUNT_ACCESS_TOKEN>"
  },
  "secondary": {
    "access_token": "<SECOND_ACCOUNT_ACCESS_TOKEN>"
  }
}
```

One team token that acts as two members:

```json
{
  "namespace_id": "123456789",
  "probe_root": "/Mallard Dropbox POC",
  "primary": {
    "access_token": "<TEAM_ACCESS_TOKEN>",
    "select_user": "dbmid:<FIRST_MEMBER_ID>"
  },
  "secondary": {
    "access_token": "<TEAM_ACCESS_TOKEN>",
    "select_user": "dbmid:<SECOND_MEMBER_ID>"
  }
}
```

Set the file mode:

```sh
chmod 600 /absolute/path/to/dropbox-poc.json
```

The reader rejects relative paths, symbolic links, broad file permissions,
unknown JSON fields, duplicate actor contexts, and malformed identifiers.

## Run offline

The mock tests make no network request:

```sh
node --test poc/dropbox-shared-team-namespace/dropbox-namespace-poc.test.mjs
```

The offline suite also proves that the client obeys a 60-second
`Retry-After` value without a real delay. It fails closed when the value is
more than the live-probe limit. It also proves that the request deadline stays
active while the response body is read and that a stale-write proof requires
Dropbox's structured HTTP 409 file-conflict error. These are not live
throttling or service-response tests.

## Run against Dropbox

```sh
node poc/dropbox-shared-team-namespace/dropbox-namespace-poc.mjs live \
  --credentials /absolute/path/to/dropbox-poc.json
```

A successful result has this form:

```json
{
  "verdict": "POC_PASS",
  "productionGate": "INCOMPLETE",
  "namespaceMode": "collaborative_non_home_namespace",
  "distinctAccounts": true,
  "pageCount": 5,
  "objectCount": 10,
  "cleanup": {
    "status": "complete"
  },
  "stages": [
    "two_actor_namespace_access",
    "isolated_probe_prefix",
    "absent_create_and_cross_actor_read",
    "current_revision_and_stale_rejection",
    "late_stale_identical_and_add_conflicts",
    "revision_delete_and_delete_recreate_conflict",
    "ambiguous_commit_readback",
    "zero_byte_max_component_sorted_pagination"
  ]
}
```

The output does not contain access tokens, account IDs, member IDs, namespace
IDs, remote paths, response bodies, or Dropbox request IDs.

After the run, revoke the test tokens or let the short-lived tokens expire.
Delete the local credential file when you no longer need it. Keep or delete
the empty probe root manually.

## Limits

This is a capability probe. It is not a Dropbox storage adapter. It does not
test:

- machine-local token redaction, replacement, or expiry behavior;
- administrator consent policy or authorization availability across managed
  teams;
- namespace discovery UI or team-space migration behavior;
- nested Mallard `.mallard/` object-key mapping;
- the Rust `ObjectStore` implementation;
- Mallard storage-marker, BundleEngine, Library, Push, or Pull parity;
- files larger than 1 MiB or Dropbox upload sessions;
- live throttling, sustained rate limits, or production transfer concurrency;
  or
- packaged macOS, Windows, or Linux behavior.

The 255-byte test applies to one direct child file-name component. It does not
prove nested Mallard key mapping.

Do not add Dropbox to `StorageConfig` or the storage UI from this result alone.
First run the live probe. Then test manual token replacement, nested object
mapping, live throttling, and the production adapter.

## Official references

- [Dropbox team files and namespace guide](https://developers.dropbox.com/dbx-team-files-guide)
- [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide)
- [Dropbox API files specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone)
- [Dropbox content-hash specification](https://www.dropbox.com/developers/reference/content-hash)
- [Dropbox performance guide](https://developers.dropbox.com/dbx-performance-guide)
- [Dropbox error-handling guide](https://developers.dropbox.com/error-handling-guide)

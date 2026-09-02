# Mallard Storage Provider Feasibility

*Dropbox and Google Drive support alongside Cloudflare R2*

**Prepared for:** Mallard maintainers

**Research date:** 1 September 2026

**Scope:** Research only; no implementation or schema change

**Decision status:** Dropbox is feasible after a capability probe; Google Drive is blocked by the current consistency contract

## Executive recommendation

> **Recommendation:** Keep R2 as the primary team-capable storage. Add Dropbox next, subject to a live two-client capability probe. Do not ship Google Drive as an R2-equivalent transport until Google documents an atomic stale-write precondition or Mallard adopts a different publication protocol.

Mallard already has a provider-neutral object-store boundary. A Dropbox adapter can satisfy it with Dropbox revision tokens. Google Drive can transfer and list files, but Drive API v3 does not document the compare-and-swap operation that Mallard uses to protect mutable heads. [M1] [M3] [D2] [G5] [G6]

| **Provider model** | **Concurrency fit** | **Sharing fit** | **Research verdict** |
| --- | --- | --- | --- |
| Cloudflare R2 | Native conditional object writes | Team or organization bucket | Keep as the baseline |
| Dropbox App Folder | Revision-based CAS is documented | One account across its devices | Feasible for personal scope |
| Dropbox shared or team namespace | Same revision model; more namespace work | Collaborators or managed teams | Feasible with broader authorization |
| Google appDataFolder | No documented Drive v3 CAS | Not shareable | Do not implement as a peer store |
| Google selected folder or Shared Drive | No documented Drive v3 CAS | Team capable | Blocked without protocol change |

*Sources: Mallard storage contract [M1-M5]; Dropbox API [D1-D7]; Google Drive API [G1-G8].*

## 1. Scope and decision criteria

*This report tests whether each provider can preserve Mallard's current safety model. It does not test whether the provider can only upload and download files.*

- Immutable publication must create only an absent object and must reject different bytes at an existing logical key.

- Mutable heads must use an atomic compare-and-swap operation. A stale writer must fail.

- Downloads must be bounded and verified against Mallard's standard SHA-256 digest.

- Listing must return validated Mallard keys in stable lexical order, even when the provider has different cursor semantics.

- Deletion must target one exact validated file and treat an absent file as success.

- Credentials, refresh tokens, provider roots, and local paths must remain machine-local and must not enter portable manifests.

These properties are implemented by the narrow ObjectStore interface and by the project and Library publication engines. They are product safety requirements, not optional transport optimizations. [M1] [M3] [M4]

## 2. Current Mallard architecture

### 2.1 Extension point

The ObjectStore trait is the main provider boundary. It defines bounded metadata reads, verified staging, immutable writes, compare-and-swap, exact deletion, and prefix listing. Both ProjectSync and the storage Library use this contract. [M1] [M3] [M4]

- StoredMetadata.etag is already an opaque version token in practice. Dropbox rev can map to it without changing portable data.

- The .mallard/v1 repository, tag, and Library key layout is provider-neutral. [M2]

- The portable bundle manifest has no storage-provider fields. New adapters do not require a portable schema change. [M9]

- ConfiguredObjectStore dispatch currently contains only Local and S3 variants. [M2]

### 2.2 Change surface

The protocol layer is ready for another provider, but setup and configuration are not. StorageKind, StorageConfig, TypeScript DTOs, setup commands, the CLI, and the storage settings UI are Local/R2-specific. The current flat configuration combines local path fields and R2 credentials. [M5] [M6] [M7]

> **Architecture implication:** Do not add Dropbox and Google fields to the flat StorageConfig. Use provider-specific destination variants and a separate authorization profile. This is a machine-local model change only.

### 2.3 Do not use desktop-synchronized folders as the adapter

Pointing LocalObjectStore at a Dropbox or Google Drive desktop-sync folder is not first-class cloud support. Mallard's local store uses a machine-local advisory lock. Two machines can each acquire their own lock, after which the sync client can create conflict copies or apply last-writer behavior outside Mallard's CAS check. [M8]

## 3. Dropbox feasibility

> **Verdict:** Feasible. Dropbox is the best next provider because its file revision token and strict conflict behavior closely match Mallard's compare-and-swap contract.

### 3.1 Storage operation mapping

| **Mallard operation** | **Dropbox mechanism** | **Required adapter behavior** |
| --- | --- | --- |
| Read metadata | files/get_metadata | Return size, rev, and content_hash. |
| Verified download | files/download | Stage privately; verify standard SHA-256 and size. |
| Create if absent | upload mode=add | Set autorename=false and strict_conflict=true; verify on conflict. |
| Conditional replace | upload mode=`update:<rev>` | Use strict conflict; map a stale rev to CAS conflict. |
| Large upload | upload session APIs | Use sessions when a request can exceed 150 MiB. |
| List prefix | list_folder plus continue | Complete recursive listing, validate keys, sort, and page locally. |
| Exact delete | delete_v2 | Verify the target is a file and use its current revision. |

*Sources: Dropbox file specification and revision behavior [D2], transfer guidance [D4], and change cursors [D5].*

### 3.2 Compare-and-swap and immutable writes

Dropbox file metadata contains rev, a unique identifier for the current file revision. Dropbox supports `update:<rev>`. With strict_conflict=true, a stale revision conflicts even after deletion or when the new content is identical. autorename=false prevents a conflicted copy from being created. [D2]

- Initial mutable head: use add with strict conflict handling.

- Existing mutable head: use `update:<rev>` with strict conflict handling.

- Immutable object: use add; after a conflict, read and compare the exact bytes.

- Ambiguous network result: read back and verify instead of repeating an unchecked overwrite.

A live capability probe is still mandatory. It must prove create-if-absent, current-revision replacement, stale-revision rejection, delete-and-recreate behavior, and recovery after a lost final response.

### 3.3 Integrity and transfer limits

Dropbox content_hash is not a standard whole-file SHA-256. It hashes 4 MiB block hashes and then hashes their concatenation. Mallard must keep its existing standard SHA-256 in manifests and verify that digest after download. The adapter can also calculate and compare Dropbox content_hash for transport-level verification. [D3]

- Use direct upload for small objects and upload sessions for larger objects.

- Honor Retry-After and bound parallel writes because Dropbox applies per-authorization rate limits and namespace write locks. [D4] [D9]

- Treat a successful batch as individual entry results, not as one all-or-nothing transaction.

### 3.4 OAuth and credential lifecycle

Dropbox recommends OAuth 2 authorization code flow with PKCE and refresh tokens for desktop applications that need background access. Use S256, state, token_access_type=offline, and a system-browser redirect. Do not embed a client secret. [D1]

- Store the refresh token in operating-system credential storage. Keep access tokens in memory when practical.

- Keep all tokens out of portable bundles, logs, activity history, setup drafts, and frontend responses.

- Revoke the token when the user disconnects the account. Require reconnection after a failed refresh.

- Validate the loopback redirect, cancellation, refresh, revocation, and reconnect flows on macOS, Windows, and Linux.

### 3.5 Personal and team product modes

Dropbox App Folder is the least-privilege first release. It limits Mallard to `/Apps/<Mallard>`, but Dropbox does not permit an app folder to be shared, placed in a shared folder, or contain a shared folder. It therefore supports one Dropbox account across several machines, not teammate sharing. [D1] [D6]

For R2-like collaboration, use Full Dropbox access and root Mallard to a selected shared or team namespace. Team spaces require stable namespace IDs and Dropbox-API-Path-Root handling. Team-admin policies and team-space migrations add product and test scope. [D7]

> **Product decision:** If the first goal is same-account portability, use App Folder. If the first goal is teammate parity with R2, design the selected shared/team namespace flow from the start. Do not describe App Folder as team storage.

### 3.6 Dropbox risks

- Dropbox paths are case-insensitive. The adapter must reject case aliases and enforce a provider path-length limit.

- Dropbox list cursors are change-feed cursors, not Mallard's lexical key cursor. Initial support should enumerate, validate, sort, and page locally. [D5]

- Users can modify or delete data through Dropbox clients. Mallard must continue to treat remote content as untrusted.

- Managed teams can block third-party apps. Public distribution also requires Dropbox production approval and compliant data-use disclosures. [D8]

- Dropbox has no supported Rust SDK suitable for this safety boundary. A narrow direct-HTTP adapter is the lower-risk design.

## 4. Google Drive feasibility

> **Verdict:** File transfer is feasible, but safe R2-equivalent storage is not approved. Drive API v3 does not document an atomic stale-write precondition for files.update.

### 4.1 Capabilities that fit

- Installed applications can use OAuth 2 with PKCE, a loopback redirect, offline access, and refresh tokens. [G1]

- Drive supports simple, multipart, and resumable upload. Resumable upload is suitable for Mallard's larger objects. [G7]

- Binary file metadata can include size and SHA-256, although the field is documented as available only when present. Mallard must still verify its own digest. [G4]

- Shared Drive support is documented for team content through supportsAllDrives and drive-specific listing parameters. [G8]

### 4.2 Storage models

appDataFolder is a hidden, app-only space that uses the non-sensitive drive.appdata scope. It cannot be shared and cannot move between Drive spaces. This is appropriate only for one account across its machines. [G2] [G3]

A visible app-created folder or a Shared Drive is the collaboration direction. The non-sensitive drive.file scope can limit access to files that Mallard creates or that the user selects. Shared Drives require additional support for permissions, corpora, and root identifiers. [G3] [G8]

### 4.3 Blocking consistency gap

Drive API v3 file metadata has an output-only, monotonically increasing version value. The v3 files.update method accepts a file ID and update parameters, but its official contract does not document If-Match, an expected version, or another stale-write precondition. Google's v2-to-v3 reference also states that the File etag field has no v3 equivalent. [G4] [G5] [G6]

- A read-then-update sequence is not CAS. Two writers can both read version N and then overwrite each other.

- An observed but undocumented HTTP header would not be a sufficient production guarantee for Mallard's safety boundary.

- Drive v2 should not be selected as a new stopgap only because it exposed etag. Mallard's repository rules reject temporary compatibility paths.

### 4.4 Logical-key identity gap

Google Drive names are not necessarily unique within one folder. A lookup by .mallard path can therefore return more than one item. File IDs are stable, but a separate mapping from each Mallard key to one file ID would be required. The mapping itself needs safe singleton creation and conditional update. [G4]

- A single global key-to-ID index would be large and highly contended as immutable revisions accumulate.

- A sharded index reduces size but does not solve the missing CAS operation.

- Shared Drives fix collaboration scope, not the stale-write or duplicate-name problems.

### 4.5 Safe options

1. Obtain a documented Drive v3 conditional-update guarantee and prove it with the same capability probe used for R2 and Dropbox.

2. Use a separate CAS coordinator, such as an object store or transactional service, for mutable heads. This is no longer a pure Google Drive destination and adds a new trust and availability dependency.

3. Redesign Mallard publication as append-only, multi-head, and conflict-aware. This is a large protocol change across ProjectSync and the Library.

4. Do not add Google Drive. Keep R2 and Dropbox as the supported cloud stores.

> **Research gate:** The next Google task should be a narrow API proof, not product implementation. Stop if no official v3 stale-write contract exists.

## 5. Recommended multi-provider design

*These are research recommendations for a long-term architecture. They are not implementation work performed in this task.*

### 5.1 Destination and authorization models

- Replace the flat Local/R2 storage record with tagged provider-specific destination variants: local, r2, dropbox, and, only if approved, google_drive.

- Create a storage authorization profile that contains provider kind, stable account ID, stable namespace or root ID, granted scopes, token expiry, credential reference, and connection revision.

- Store refresh tokens in the OS keychain. Store only a credential reference in Mallard's JSON configuration.

- Define storage continuity identity from provider account ID plus stable root or namespace ID. Never include tokens in the identity.

### 5.2 Transport boundary

- Keep the ObjectStore contract and .mallard/v1 logical keys unchanged.

- Treat the current etag field as an opaque version token. A later internal rename to version_token can improve clarity but is not required for Dropbox.

- Use narrow direct REST adapters. Keep streaming, bounds, redaction, retry policy, and conflict mapping inside each adapter.

- Translate provider cursors into Mallard's validated lexical-key pages. Do not leak provider cursors into the protocol without a contract change.

### 5.3 Provider capability probe

| **Stage** | **Required proof** | **Failure result** |
| --- | --- | --- |
| List | List one generated probe prefix | Provider or permission failure |
| Absent create | Create only when absent | Provider is incompatible |
| Read and integrity | Read exact size and bytes | Provider is incompatible |
| Current CAS | Replace with current token | Provider is incompatible |
| Stale CAS | Reject the old token | Hard stop |
| Ambiguous response | Resolve through verified read-back | Do not commit setup |
| Delete | Remove one exact file and confirm absence | Do not commit setup |

*Model: current R2 setup capability probe [M10], extended as a provider-neutral conformance gate.*

### 5.4 Conformance and security tests

- Extract one ObjectStore conformance suite and run it against local, R2, Dropbox, and any future provider.

- Test stale writers, identical-content races, delete-and-recreate, ambiguous commits, pagination, rate limits, token expiry, revocation, and interrupted cleanup.

- Run ProjectSync and Library publication parity tests for each provider.

- Prove that OAuth data never enters portable manifests, logs, frontend responses, or stored activity events.

- Keep one live-provider smoke test outside the normal isolated suite. Do not run ordinary tests against real user homes or production storage.

## 6. Recommended sequence

1. Confirm the first Dropbox product mode: same-account App Folder or team-capable selected shared namespace.

2. Extract the provider-neutral conformance suite and setup capability probe.

3. Run a two-client Dropbox spike. Prove strict absent creation, stale-revision rejection, verified upload, sorted listing, exact deletion, and OAuth lifecycle.

4. If the Dropbox spike passes, design provider-specific destination and authorization records, then plan the adapter and UI work.

5. Run a separate Google Drive v3 CAS proof. Do not build product UI or persistence before that gate passes.

6. If Google cannot provide a documented CAS contract, choose explicitly between a protocol redesign, an external coordinator, or no Google Drive support.

## 7. Relative scope and risk

| **Work area** | **Relative scope** | **Main risk** |
| --- | --- | --- |
| Shared config, OAuth profiles, probe, and conformance | High | Touches backend, IPC, CLI, UI, and credential storage |
| Dropbox personal App Folder | Medium | OAuth callback, cursor mapping, case-insensitive paths |
| Dropbox shared or team namespace | Medium-high | Broader access, namespace roots, admin policy, migration behavior |
| Google Drive direct adapter | Blocked | No documented stale-write precondition |
| Google indexed store or protocol redesign | Very high | New consistency protocol and larger safety surface |

*Scope is directional. A schedule should follow the capability spikes and the personal-versus-team product decision.*

## 8. Final conclusion

**Dropbox:** Technically feasible and compatible with Mallard's current publication model. Proceed to a live capability spike. Select App Folder only for same-account portability; use a selected shared or team namespace for collaborator parity.

**Google Drive:** Not safe as a drop-in provider under the current contract. The missing documented Drive v3 compare-and-swap guarantee is a hard blocker. A team folder or Shared Drive does not remove it.

**R2:** Remain the primary team-capable baseline. Its object-store semantics align directly with Mallard's immutable writes, checksum verification, and conditional head updates.

## Source register

*Official provider documents and repository evidence used for this research. Accessed 1 September 2026.*

### Repository evidence

- **[M1]** crates/mallard-infrastructure/src/storage_core/object_store.rs:18-249 - Object metadata, CAS types, and ObjectStore contract

- **[M2]** crates/mallard-infrastructure/src/storage_core/mod.rs:32-47, 272-386 - Closed key layout and Local/S3 dispatch

- **[M3]** crates/mallard-infrastructure/src/bundle_engine.rs:915-1032 - Immutable publication followed by head CAS

- **[M4]** src-tauri/src/storage_catalog/resource_engine.rs:243-461 - Library publication and head CAS

- **[M5]** crates/mallard-domain/src/lib.rs:777-825 - Local/S3 StorageKind and flat StorageConfig

- **[M6]** src/types.ts:619-631 - TypeScript storage DTO

- **[M7]** crates/mallard-application/src/storage_setup.rs:500-1002 - R2-only setup transaction and probe boundary

- **[M8]** crates/mallard-infrastructure/src/storage_core/local_store.rs:93-235 - Machine-local advisory lock and atomic local writes

- **[M9]** crates/mallard-domain/src/lib.rs:4816-4830 - Portable manifest has no storage-provider fields

- **[M10]** crates/mallard-infrastructure/src/storage_core/s3_store.rs:220-496 - R2 list, create, read, CAS, stale-write, and cleanup probe

### Dropbox official sources

- **[D1]** [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide) - PKCE, refresh tokens, App Folder and Full Dropbox access

- **[D2]** [Dropbox API files specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone) - rev, WriteMode, strict_conflict, content hash, upload sessions, and delete arguments

- **[D3]** [Dropbox content-hash specification](https://www.dropbox.com/developers/reference/content-hash) - 4 MiB block-hash algorithm

- **[D4]** [Dropbox performance guide](https://developers.dropbox.com/dbx-performance-guide) - Upload sessions, namespace locks, rate limits, and Retry-After

- **[D5]** [Dropbox detecting changes guide](https://developers.dropbox.com/detecting-changes-guide) - list_folder cursors, continuation, and reset behavior

- **[D6]** [Dropbox shared-folder FAQ](https://help.dropbox.com/share/shared-folder-faq) - App Folder sharing restrictions

- **[D7]** [Dropbox team files guide](https://developers.dropbox.com/en-us/dbx-team-files-guide) - Team spaces, namespace IDs, and Dropbox-API-Path-Root

- **[D8]** [Dropbox developer guide](https://www.dropbox.com/developers/reference/developer-guide) - Production status, review, data practices, and app distribution

- **[D9]** [Dropbox error handling guide](https://developers.dropbox.com/error-handling-guide) - 409, 429, 5xx, retry, and request identifiers

### Google official sources

- **[G1]** [OAuth 2.0 for iOS and desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app) - PKCE, loopback redirects, refresh tokens, and installed-app security

- **[G2]** [Store application-specific data](https://developers.google.com/workspace/drive/api/guides/appdata) - appDataFolder access and sharing constraints

- **[G3]** [Choose Google Drive API scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth) - drive.appdata, drive.file, scope classification, and refresh-token storage

- **[G4]** [Drive API v3 File resource](https://developers.google.com/workspace/drive/api/reference/rest/v3/files) - File IDs, non-unique names, output-only version, and checksum fields

- **[G5]** [Drive API v3 files.update](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/update) - Official update request and parameters; no documented stale-write precondition

- **[G6]** [Drive API v2 and v3 comparison reference](https://developers.google.com/workspace/drive/api/guides/v2-to-v3-reference) - File etag has no v3 equivalent

- **[G7]** [Upload file data](https://developers.google.com/workspace/drive/api/guides/manage-uploads) - Simple, multipart, and resumable uploads

- **[G8]** [Implement shared drive support](https://developers.google.com/workspace/drive/api/guides/enable-shareddrives) - supportsAllDrives and Shared Drive listing

- **[G9]** [Drive API usage limits](https://developers.google.com/workspace/drive/api/guides/limits) - Quotas and backoff guidance

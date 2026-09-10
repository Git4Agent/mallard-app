# Managed Cloud V2 plan: over-engineering and redundancy review

- **Date:** September 8, 2026
- **Status:** Recommendations for review
- **Scope:** [CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE_V2.md](CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE_V2.md),
  working tree of September 8 (1,864 lines; the committed September 1 draft is
  1,464 lines)
- **Method:** full read of the plan; comparison with the committed draft and
  with [today's plan review](REVIEW_MANAGED_CLOUD_PLAN_2026-09-08.md); checks of
  the plan's format tables against the current Rust registry and codecs. No
  code changed; no tests ran.

## Summary

The architecture is sound and the fixed decisions in section 1.1 should stand.
The problem is proportion. The September 8 rewrite answers every item in
today's review by adding a mechanism and by restating the rule, and it grew by
400 lines doing so. Three patterns account for most of the excess:

1. **Second records of the same fact.** Server-side review plans duplicate the
   upload session; a quota ledger duplicates batch atomicity plus idempotency;
   an opaque head ETag duplicates (revision, commit ID); generation IDs
   duplicate upload IDs; a `staging/` prefix has nothing defined to live in it.
2. **Depth beyond the V1 requirement.** Repository removal carries a
   triple-match claim, two out-of-band R2 records, a per-requester receipt
   route, and a three-phase UI status that exposes platform backup retention.
   SQL trigger mechanics occupy the largest section of an architecture document.
3. **Five-fold restatement.** Most rules appear in their owning section and
   again in Fixed decisions (1.1), Required constraints (7.3), Failure rules
   (10.7), Test groups (12.2), and Production acceptance (13.1). Not included
   (1.3) and Future work (13.2) are the same list twice.

Applying the recommendations below removes four tables, seven routes, one
lifecycle state, one identifier type, and several hundred lines of restatement.
None of them changes a fixed decision in section 1.1.

## Keep

| Design | Why it is proportionate |
| --- | --- |
| One access boundary per Project or Catalog item; Admin and Member only | Matches the product; nothing simpler is safe |
| One head, linear immutable history, compare-and-swap, no automatic combination | Required so a stale writer never overwrites |
| Worker-only access to private R2; D1 authoritative for heads and access | Correct ownership split |
| Content-addressed immutable objects with exact length and SHA-256 on both sides | Cheap and matches the existing engine |
| `publication_objects` join for read authorization (6.5) | Answers review item 6 with the minimum structure |
| Cleanup claim protocol with both orderings tested (6.7) | The race is real; the claim is the minimum fix |
| No resource removal through sync; Pull never deletes (2.7) | Matches the repository safety invariants |
| Idempotency on the seven listed mutations (6.6) | Proportionate |
| Current Rust DTOs and shared serialization fixtures (3.4, 4.3) | Answers review item 1 without inventing formats |
| Disclosure review before public visibility (2.6) | Irreversible disclosure deserves an explicit step |
| OAuth with PKCE, rotating refresh tokens, keychain storage (9.1) | Standard |
| Sanitized audit, monitoring, and alert lists (11.2) | Standard for a service holding conversations |

## A. Over-engineered mechanisms

### A1. Server-side review plans duplicate the upload session

Where: 3.6; 7.1 `review_plans`; 7.2 plan states; 8.3 `push-plans` and
`disclosure-plans`; 6.4 guard "unused matching approval" and step 5; 6.7
expired plans; 11.2; 12.2.

The upload session already pins the actor, repository revision, expected head,
visibility, approval digest, acknowledgements, and expiry (6.4). The review-plan
record pins the same values one call earlier. The service cannot verify that a
person reviewed anything; today's review says so directly. The plan is an echo
of a client claim, and it carries its own table, four states, two routes, a
trigger clause, a cleanup class, an audit category, and tests.

Recommendation: the `POST /uploads` request carries the approval fields, and
the upload session is the approval record. Public disclosure acknowledgement is
one more field on that request; the candidate manifest digest it covers is
already pinned. The visibility route (2.6) already carries its own disclosure
inputs. Delete `review_plans`, both plan routes, the plan states, and the plan
guard.

### A2. Five calls to publish where three suffice

Where: 8.3; 6.4; 4.4 steps 7 to 9; 7.2 upload states.

Sequence today: `push-plans`, `uploads`, `PUT objects`, `complete`, `publish`.
The `complete` call seals the session as `ready`; `publish` runs the batch.
Nothing useful can occur between them. A sealed upload that then loses the head
comparison is worthless, because the user must re-review against the new head
and the retained set changes. The `ready` state has exactly one consumer, the
next call.

Recommendation: one `publish` call validates the candidate, seals it in memory,
and runs the batch. With A1 the sequence is `uploads`, `PUT objects`,
`publish`. Upload states become `created`, `receiving`, `committed`,
`expired`, `failed`. Idempotency covers the lost-response case unchanged.

### A3. Generation IDs duplicate upload IDs, and `staging/` is empty

Where: 6.3; 6.4; 6.7; 7.2 `objects` states; 2.8; 11.1 rows "Upload staging"
and "Unaccepted objects".

Section 6.4 writes payloads directly to `objects/<sha256>/<generation-id>` with
create-only writes. Nothing in the plan ever writes under
`staging/<repository>/<upload-id>/<object-id>`, yet removal erases "staging
data", retention keeps it for 24 hours, and cleanup expires it. Two retention
rows describe one thing.

The generation ID exists so that a delayed delete cannot hit a re-upload of the
same digest. The upload ID is already a never-reused identifier for the write
that produced the bytes.

Recommendation: key the physical object as `objects/<sha256>/<upload-id>` and
record `(repository, sha256, upload_id)` in `objects`. The generation is the
upload. Remove the `staging/` prefix and its retention row, or define what
lives there. The plan cannot keep both.

### A4. Repository removal is specified beyond the V1 need

Where: 2.8 (73 lines); 6.3 `removals/`; 7.4 (three rows); 8.2 (three routes
and three paragraphs); 8.4; 9.2; 10.7 (six rows); 11.1; 11.3; 11.5; 11.6 (two
messages); 12.1; 12.2 (nine bullets); 13.1 (three bullets). "Removal" appears
on 83 lines and "erasure" on 36.

Today's review asked for one controlled operation with authority, recent
authentication, write suspension, confirmation, retention, erasure, and audit.
The plan delivers that and also:

- a claim that must match operation ID, repository revision, and deadline.
  State plus operation ID already excludes a stale job: cancellation changes the
  state, and a new request has a new ID;
- two out-of-band R2 records per removal (`authorized.json`,
  `completed.json`). One append-only removal record outside D1 serves the same
  runbook step in 11.3;
- a post-completion removal receipt readable only by the requesting account
  (2.8, 8.2, 9.2). This is a new authorization rule and read path for
  information the audit log already holds;
- a three-phase status in the desktop UI: access removed, live erasure verified,
  recovery copies expired (2.8, 11.1, 11.6). The third phase exposes
  Cloudflare's backup retention to end users;
- the guarantee "cannot delete local files, Git remotes, or another
  repository" in twelve places.

Recommendation: keep the lifecycle, the seven-day cancel, the erasure worker,
one removal record outside D1, and once-only quota release. Cut the receipt
route, the third UI phase, and the revision and deadline components of the
claim. State the local-content guarantee once, in 2.8.

### A5. Three concurrency tokens where two suffice

Where: 3.1; 6.4; 8.2 `expected_head_etag`; 12.2 "zero-row head updates".

In the user-owned R2 protocol the ETag is real: R2 generates it and
compare-and-swap uses it. In the managed service D1 owns the head. With one
revision and one commit ID per repository (7.3), a linear immutable chain, and
no rollback route, the pair (revision, commit ID) can never recur. An opaque
ETag is a synthetic third token beside publication revision and repository
revision.

Recommendation: the head predicate is `expected_commit_id`, null for no head.
If the engine needs a string for `CasExpectation::Match`, the managed adapter
passes the commit ID. Keep repository revision for metadata mutations.

### A6. The quota ledger guards a property already guaranteed

Where: 6.4 step 4 and the "Quota ledgers" paragraph; 6.7; 7.1; 2.8; 11.1.

Publication is one atomic batch with an idempotency record. Removal completion
is one transaction keyed by operation ID. A retry either replays the recorded
result or finds nothing committed, so double charging is already impossible.
The ledger, its unique transition IDs, and the final-trigger check of "matching
transition and amount" are a second guard for the same property.

Recommendation: `reserved_bytes` on the upload session and `committed_bytes`
on repository and account, updated inside the same batch. Keep a ledger only
if it is wanted for accounting, and label it that.

### A7. SQL trigger mechanics belong in an implementation note

Where: 6.4 (78 lines, the largest section in the plan).

The section names trigger types, forbids `INSERT ... SELECT`, `IGNORE`, and
`REPLACE`, prescribes `RAISE(ABORT, code)`, and orders six statements. This is
one valid implementation of a four-sentence requirement, and it forecloses
others: a guarded single statement whose absence cascades through the batch,
or one Durable Object per repository that serializes publish and removal so
that plain statements suffice.

Recommendation: state the requirement in 6.4. Finalization is one D1 batch;
any failed guard aborts the whole batch; a head update that matches zero rows
aborts; guards read transaction-time state. Move the statement table and the
trigger prescriptions to the Slice 0 implementation note or the test plan.

### A8. Public slugs have no V1 consumer

Where: 2.4; 7.1; 7.3; 8.1 (four routes keyed by slug); 8.2; 11.5.

Slugs exist for readable share links. V1 has no web client (1.3), and discovery
happens inside the desktop listing. A user-changeable unique slug adds a
uniqueness constraint, a rename path, "current slug" semantics, and a collision
story.

Recommendation: key the public routes by the opaque repository ID and keep the
display name free-form. Add slugs with the first web surface.

### A9. Export is a second read path

Where: 1.2; 2.4; 2.8; 6.1; 6.3 `exports/`; 6.6; 6.7; 7.1 `repository_exports`;
8.2 (three routes); 11.1; 11.3; 12.1 Slice 6. "Export" appears on 24 lines.

An Admin can already read the current head, every retained manifest, and every
historical object through the content routes (8.3, 6.5). Export adds an
asynchronous job, an R2 prefix, expiry, a download route, cleanup, and quota
rules to deliver the same bytes. A desktop "pull all history to a folder" over
the existing routes needs no service code.

Recommendation: defer export unless it is a compliance requirement. If it is,
record that in 1.1.

### A10. Recent authentication on three operations

Where: 2.6; 2.8; 8.2; 8.4; 9.2.

Access tokens are short-lived and live in the keychain (9.1); removal has a
seven-day cancel window. Recent authentication adds a session timestamp, an
error code, and a mid-operation browser re-login flow on the desktop.

Recommendation: keep it for the change to public visibility, where disclosure
is irreversible. Drop it for removal request and cancel, where the recovery
window is the safeguard.

### A11. Smaller items

- `devices` table (7.1, 8.1, 12.2): a label column on `refresh_sessions` is
  the device list.
- `POST .../leave` beside `DELETE .../members/{user_id}` (8.2): one route.
- Catalog container type: 5.5 defines a new single-entry manifest while 4.3
  forbids inventing a reduced DTO for projects. `LibraryResourceManifest` with
  `resources.len() == 1` enforced by the managed validator reuses the Rust and
  TypeScript types and fixtures that exist today. The field that does not fit
  is `repository_id`, which the plan would derive from the binding. Whether
  "closed by construction" is worth a second container type is a judgment
  call; the service must check cardinality either way (5.1).

## B. Redundant text

### B1. Five-layer restatement

Most rules appear in their owning section and then in 1.1, 7.3 (27 bullets),
10.7 (33 lines), 12.2 (63 lines), and 13.1 (38 bullets). Examples:

| Rule | Stated in |
| --- | --- |
| No resource omission or tombstone | 1.1, 2.7, 4.3, 7.3, 8.4, 10.7, 11.6, 12.2, 13.1 |
| At least one active Admin | 2.4, 7.3, 7.4, 8.2, 8.4, 12.2, 13.1 |
| Zero or one current head | 3.1, 7.1, 7.3, 12.2, 13.1 |
| Exactly one item per Catalog repository | 1.1, 2.1, 5.1, 5.5, 6.4, 6.7, 12.2, 13.1 |
| Sync and removal never delete local content | twelve lines across 2.7, 2.8, 4.5, 5.7, 10.7, 11.6, 13.1 |

Recommendation: a rule lives in its owning section. Reduce 7.3 to constraints a
schema enforces. Reduce 13.1 to a pointer list or delete it; 1.1 is already the
decision record. Keep in 10.7 only behavior stated nowhere else. Write 12.2 as
"tests for sections 2.7, 2.8, 3.3, ..." rather than re-describing each rule.

### B2. Not included versus Future work

Sixteen of the twenty-one items in 1.3 reappear in 13.2 with slightly different
wording. Keep one list.

### B3. Fixed decisions versus sections 2 and 3 versus acceptance

1.1 summarizes 2.1 to 2.8 and 3.1 to 3.3; 13.1 summarizes 1.1. One summary is
enough.

### B4. Listing tags are a vestige of the user-owned protocol

3.4 lists "non-authoritative listing tags" as a shared property, and 6.7
reconciles "tag drift". The managed key layout (6.3) has no tag key, and 6.2
says an R2 listing decides nothing. Tags exist for the flat `_tags/` discovery
namespace on user-owned stores
([storage_core/mod.rs](../crates/mallard-infrastructure/src/storage_core/mod.rs))
and have no managed counterpart. Delete both mentions.

### B5. Tables copied from code

The provider matrix (2.2), pet dimensions (5.4), plugin policies (5.3), and
skill scopes (5.2) restate
[registry.rs](../src-tauri/src/storage_catalog/registry.rs),
[codex_pet.rs](../src-tauri/src/storage_catalog/codex_pet.rs), and
[plugin_intent.rs](../src-tauri/src/storage_catalog/plugin_intent.rs). They
match today and will drift. The plan's own rule (3.4, 4.3) is to use the
current Rust types. Keep the rule "the service rejects unlisted coordinates;
fixtures are generated from the registry" and point at the code.

### B6. Repeated workflow prose

5.6 and 5.7 restate 4.4 and 4.5 with "item" substituted; 10.1 to 10.5 restate
all four again as numbered workflows. Keep the class-specific differences
(single item, no tombstones, plugin intent cannot be copied) and reference the
common flow once.

### B7. Repeated examples

The `petv2_rika` example table and its "examples only" disclaimer appear in
both 2.1 and 11.4.

## C. Observation outside this review's question

12.1 still builds public listing, featured flags, and public routes (Slice 1)
before any user can write (Slice 2). Today's review recommended proving private
sync first. This is ordering rather than over-engineering, and is noted because
the slug and featured mechanisms above exist to serve that early slice.

## D. Resulting shape

| Area | Before | After |
| --- | --- | --- |
| D1 tables (7.1) | 21 | 17 (drop `devices`, `review_plans`, `quota_ledgers`, `repository_exports`) |
| Calls to publish | 5 | 3 |
| Upload states | 6 | 5 |
| Concurrency tokens | 3 | 2 |
| Content routes (8.3) | 11 | 8 |
| Access routes (8.2) | 15 | 11 |
| Statements of each core rule | 4 to 9 | 1 to 2 |

None of these changes a fixed decision in section 1.1.

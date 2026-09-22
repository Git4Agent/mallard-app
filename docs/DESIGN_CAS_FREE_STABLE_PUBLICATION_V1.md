# Mallard: cheaper CAS-free stable publication protocol

**Status:** Proposed specification; not implemented or production-certified.  
**Protocol / layout / record schema versions:** `1`.  
**Compatibility:** Clean cutover; no migration, dual-read, dual-write, or earliest-sibling fallback.  
**Scope:** One shared storage destination; independent project and Library streams.  
**Preparation:** Per decision and per attempted round. No amortized preparation, shared leader term, lease, or Multi-Paxos optimization.

## 1. Objective and cost

Normal Pull must use only publications whose canonical ancestry cannot subsequently be replaced by another protocol-compliant Push.

A finalized publication may become older when its descendants publish. It must not become a losing sibling. A client may continue using an older verified stable publication while newer work is incomplete.

The uncontended metadata path, after payload preparation and with a known stable parent, is:

```text
WRITE PROPOSAL
      ↓
SCAN decision
      ↓
WRITE ACCEPT
      ↓
SCAN decision
      ↓
WRITE STABLE
```

This is **three small writes and two complete scans**. With one-page scans, all needed bodies already known, and acknowledged writes, it is five requests. Payload uploads, baseline application checks, unknown-record GETs, pagination, retries, and provider folder operations are additional.

PROPOSAL contains the candidate publication, so there is no separate candidate-record write. STABLE is the only publication receipt used by normal Pull. There is no preliminary coordination scan just to allocate a round.

The API surface remains ordinary `put`, `get`, and `list`. `scan_decision` below is shared client code using `list` and, when needed, `get`; it is not a new provider operation or adapter.

## 2. What this replaces

The repository's September 19 plan treats a commit record as the publication point, ranks siblings by writer timestamp, and advances local bases after storage. This specification replaces those decisions.

| Previous design | This design |
|---|---|
| Stored commit is published | Stored PROPOSAL is a candidate only |
| Earliest sibling determines canonical history | A decision fixes one exact successor |
| A late record may replace a selected sibling | Later successful rounds preserve an already-decided value |
| Publication finality inferred from a timing window | Finality comes from write/read ordering and value inheritance |
| Stored candidate advances bases | Verified STABLE for the user's exact candidate permits base advancement |
| Refresh can select solely from commit names | Discovery uses STABLE and verifies its referenced records |

Keep the current payload objects, manifests, scope-mutation firewall, integrity checks, and application-specific review/rebase rules unless separately changed. The decision protocol does not grant permission to publish unreviewed local changes.

This is a custom append-only, single-store adaptation of read/write consensus techniques, not a claim that the exact file protocol is the published Disk Paxos algorithm. See [S1] and [S2].

## 3. Required storage and failure model

These requirements are part of correctness, not optional optimizations.

### 3.1 Immutable, atomic records

Each logical record has one exact byte sequence and one content digest. A successful control write exposes its complete bytes, not a placeholder that is populated later. Confirmed bytes remain available; subsequent deletion or corruption is outside normal protocol execution.

Different attempts use different keys. No two writers intentionally overwrite a shared head, lock, counter, or phase record. Identical physical copies of one logical record may be deduplicated. Different contents for the same `(decision, round, phase)` are an integrity violation, not alternative candidates.

### 3.2 Fresh, complete scans

A successful `scan_decision(D)` must include every immutable record in D whose write was confirmed before the scan started. It may also include records added during the scan. Returned facts must come from real, intact records.

This **complete-at-start** guarantee applies to the whole scan, including pagination. A transactionally atomic snapshot of the entire collection is not required by the proof below. A scan cannot omit a preexisting record because of a stale search index, cached response, incomplete page walk, or changed cursor.

GET must retrieve the exact immutable record being referenced. An unreadable relevant record, incomplete search, page failure, permission error, or timeout means **unknown**, never absence.

Strong consistency for a provider's API name does not by itself verify the pagination implementation. Keep finalization scans fresh; cached data may supplement but must not replace them.

S3 and R2 document strong object-read/list behavior [S3, S4]. This specification does not certify every pagination path or every S3-compatible implementation. Feishu and Google Drive are not qualified merely because they expose `get` and `list`; their integration must establish this same semantic contract. Drive explicitly documents listing/pagination caveats [S5].

### 3.3 Cooperative writers and supported failures

Writers follow the protocol and cannot select two different values in one round. They may pause arbitrarily, time out, crash, disconnect, and restart. An in-flight write may complete arbitrarily late.

Safety does not depend on synchronized clocks, file creation times, a maximum network delay, identifying dead clients, or an always-online laptop. Safety also does not depend on a majority of Mallard clients being online: there is one authoritative shared store in this design.

The protocol is not Byzantine fault tolerant. Hashes verify identities and bytes; they do not prove that a writer honestly performed an absence check. A malicious or incompatible writer that fabricates STABLE can violate the model.

### 3.4 Retention and namespace lifetime

Initial implementation: retain all valid decision records and their referenced data. Do not expire abandoned proposals or accepted values. Do not run generic orphan cleanup over data referenced by proposals or accepts.

A destructive reset must not reuse a decision namespace into which old requests can still arrive. Live in-place clearing is not covered by this protocol. Use a fresh destination/namespace for such a reset, or separately establish a safe administrative reset procedure.

## 4. Decision, round, and value identities

### 4.1 Decision

A decision chooses exactly one canonical successor of an exact stable parent:

```text
D = (configured storage root, stream_id, parent_token)
```

A stream is one project/bundle or one Library repository. Use an implicit `root` token for the first publication. The virtual root is trusted initial state; non-root parents must already have verified stable ancestry.

Two devices extending the same parent compete in the same decision. Different streams and different parents have different decisions. No round covers more than one decision.

A parent need not be proven childless before the initial PROPOSAL write. A stale parent can waste that write; the subsequent mandatory scan must discover and respect its existing stable successor.

### 4.2 Round

```text
round = (counter, attempt_id)
```

Compare counter numerically, then attempt ID lexicographically. Use a canonical unsigned 64-bit counter encoded as a decimal string in JSON and exactly 32 lowercase hexadecimal characters for the attempt ID. Each attempt generates a fresh random ID; collision-free identity is an assumption of the model. Conflicting reuse is an integrity error.

For a decision with no locally known rounds, optimistically start with counter `0`. On a retry, use one more than the largest counter observed for that decision, with a fresh attempt ID. Never wrap a counter; report an error on exhaustion.

Choosing a round that is too small may waste an attempt. It must not bypass either post-write scan. No clock or remotely incremented counter is used.

### 4.3 Value

A value is one exact PROPOSAL's publication, not a mutable snapshot name:

```text
publication_token = (publication.commit_id, SHA256(exact PROPOSAL bytes))
```

Use the existing commit ID grammar `commit-[0-9a-f]{24}`. A non-root parent token is that commit ID, a hyphen, and the 64-character digest of its exact PROPOSAL bytes.

The PROPOSAL's embedded publication names its exact parent, manifest, revision, origin, and other required metadata. Revision is the stable parent's revision plus one; it is display depth, not a vote or tie-breaker.

An ACCEPT may select another round's PROPOSAL. Inheritance retains that proposal's complete identity, commit ID, parent, manifest, and bytes. It does not copy B's data into C's proposal and call C's proposal the winner.

The originating proposal round and the round that produces a stable receipt need not be equal.

## 5. Layout and wire records

The storage marker remains `.mallard/_storage.json`, with these constant bytes interpreted as JSON:

```json
{"format":"mallard-stable-storage","layout_version":1}
```

This identifies the new wire format despite retaining version `1`. It is not a lock or a mutable head. Provision it once in a clean destination; any concurrent identical initialization writes are logically idempotent.

The project and Library control collection locations are:

```text
.mallard/v1/repositories/_decisions/<bundle_id>/<parent-token>/
.mallard/v1/library/<repository_id>/_decisions/<parent-token>/
```

The existing manifest and object locations do not change. There is no separate authoritative `_commits/` collection in this protocol; the candidate record is part of PROPOSAL.

Within the resulting namespace for one stream, use one collection per decision:

```text
_decisions/<parent-token>/
    p.<counter20>.<attempt-id>.<record-sha256>.json
    a.<counter20>.<attempt-id>.<record-sha256>.json
    s.<counter20>.<attempt-id>.<record-sha256>.json
```

`counter20` is the unsigned counter zero-padded to 20 decimal digits. `p`, `a`, and `s` identify PROPOSAL, ACCEPT, and STABLE. The leaf names stay well within a 255-byte component limit. The same pattern applies independently to project and Library namespaces.

All phases are discoverable under **one decision prefix**. Do not put them behind three unrelated mandatory listing calls. A provider implementation may map logical keys to physical file IDs, but must preserve the logical record set and read semantics. A displayed filename alone is not sufficient identity when duplicate physical filenames are possible.

Names reveal the phase, round, and digest. Bodies verify those facts when fetched. Malformed records claiming this protocol inside its owned namespace fail the decision closed. Provider-owned unrelated files may be ignored only by an explicit non-protocol grammar.

A record reference is:

```json
{
  "key": "<logical key relative to this stream>",
  "sha256": "<64 lowercase hex characters>"
}
```

The following examples are schema illustrations; angle-bracket values are placeholders, not literal wire data. All control records must be bounded (initial cap: 64 KiB each), UTF-8 JSON, and parsed with duplicate-key rejection. Serialize each record once; its name hashes those exact bytes. JSON whitespace is not an alternate encoding of the same logical record.

### 5.1 PROPOSAL: candidate plus round announcement

```json
{
  "schema_version": 1,
  "type": "proposal",
  "stream_id": "project/<bundle-id>",
  "parent_token": "<exact parent token, or root>",
  "round": {"counter": "0", "attempt_id": "<32 hex>"},
  "publication": {
    "commit_id": "<new commit id>",
    "revision": 5,
    "created_at_ms": 0,
    "manifest_key": "<existing manifest key>",
    "manifest_sha256": "<64 hex>",
    "manifest_size": 118800,
    "published_by_origin": {},
    "summary": {}
  }
}
```

Retain all domain-required publication metadata; the shortened example does not define the entire project/Library manifest schema. Parent authority is the envelope's exact token. Any retained duplicate parent fields must agree with it.

`created_at_ms` is informational only. It has no selection or expiry role.

PROPOSAL is written only after its immutable manifest and dependencies are durably stored and validated according to the existing integrity path. Its existence is **not publication** and does not force later rounds to adopt it.

### 5.2 ACCEPT: freeze this round's selected value

```json
{
  "schema_version": 1,
  "type": "accept",
  "stream_id": "project/<bundle-id>",
  "parent_token": "<same parent token>",
  "round": {"counter": "0", "attempt_id": "<same attempt ID>"},
  "proposal_ref": {"key": "<this round's proposal>", "sha256": "<hash>"},
  "selected_ref": {"key": "<selected original proposal>", "sha256": "<hash>"}
}
```

`proposal_ref` identifies the round owner. `selected_ref` identifies the chosen publication. They are equal when this round chooses its own input and may differ on inheritance.

This round writes at most one logical ACCEPT. Its selected value never changes. A higher-round ACCEPT is not automatically a stable publication.

### 5.3 STABLE: expose a completed decision

```json
{
  "schema_version": 1,
  "type": "stable",
  "stream_id": "project/<bundle-id>",
  "parent_token": "<same parent token>",
  "round": {"counter": "0", "attempt_id": "<same attempt ID>"},
  "accept_ref": {"key": "<this round's accept>", "sha256": "<hash>"}
}
```

The selected publication is obtained through ACCEPT. Do not duplicate its full payload here.

STABLE is written only after this round's ACCEPT has been confirmed and its second scan has passed. Multiple stable receipts for one decision are allowed, but all valid receipts must resolve to the **same original publication token**.

## 6. Scan interpretation and verification

A scan enumerates the whole decision prefix, deduplicates byte-identical logical records, validates the name grammar, and obtains:

```text
max_round             highest round appearing in ANY phase
accept_refs           exact ACCEPT references and their rounds
stable_refs           exact STABLE references
own phase presence    exact own PROPOSAL / ACCEPT, when required
```

A higher round counts even when only PROPOSAL exists. A visible higher ACCEPT/STABLE also counts even if its PROPOSAL was not included because it arrived during pagination. Do not filter away a higher round simply because it is incomplete.

Unselected proposal bodies need not be downloaded merely to compare rounds. Fetch the highest relevant ACCEPT and its selected PROPOSAL when deciding what to inherit. Fetch relevant STABLE chains before returning a finalized result.

Receipt verification checks exact hashes; schema, stream, parent, round and phase consistency; the corresponding proposer record; the selected proposal's parent and publication metadata; and stable parent ancestry. An inherited selected proposal cannot originate in a higher round than the accepting round.

Readers do not reconstruct a historic scan from a hash. The storage and cooperative-writer assumptions justify that the producer ran the protocol. A receipt is not a cryptographic proof of absence of other records.

Different ACCEPT values at different rounds can be legitimate during competition. Different STABLE values for one decision, or conflicting phase contents at one round, are protocol/integrity failures. Never pick the largest-round stable receipt to resolve such a conflict.

## 7. Complete uncontended and contended Push algorithm

### 7.1 Preparation before the five-step phase

Use the existing application approval, destination, local-revision and scope checks. Work from a verified stable parent. Prepare and upload the snapshot's changed objects and manifest. If there is no semantic change, return `NoChange` against the verified stable publication without opening a round.

Construct the candidate publication and choose a fresh round. Do not add a coordination LIST solely to choose a large counter. Existing normal application discovery is not prohibited; it is simply not an additional required preparation phase.

### 7.2 Step 1 — write PROPOSAL

Write this round's exact PROPOSAL and wait for confirmed storage. No dependent scan or ACCEPT may be issued before that confirmation.

The initial write need not win an atomic creation race: its attempt ID and content-addressed key are unique.

### 7.3 Step 2 — first complete scan; select once

Run a fresh complete scan of this decision.

1. If an existing valid STABLE is found, return that decision. Do not write this round's ACCEPT.
2. Otherwise, if any observed round is higher than this round, stop with `Contended` and remember the maximum counter as a retry hint.
3. Otherwise, select the value of the **highest-round prior ACCEPT**, if any. Verify its exact selected PROPOSAL. If there are no prior ACCEPTs, choose this round's own PROPOSAL.

Only prior ACCEPTs matter for inheritance, not the highest PROPOSAL or the earliest candidate timestamp. An unreadable highest ACCEPT must not be replaced by the next readable ACCEPT or by the writer's own input.

There must be no prior ACCEPT for this newly allocated round. Such a discovery indicates identity reuse or unexpected duplicate execution; fail closed rather than selecting again.

### 7.4 Step 3 — write ACCEPT

Freeze the selected reference, construct this round's ACCEPT once, and write it. Wait for confirmation.

From this point, other attempts may carry this selected value forward even if this client stops. Cancellation cannot guarantee that an accepted value will never be published.

### 7.5 Step 4 — second complete scan; validate without changing the value

Run another fresh complete scan, after ACCEPT confirmation.

1. If an existing valid STABLE is found, return that decision.
2. Otherwise, if any higher round is present, stop with `Contended`.
3. Otherwise, this round has completed the decision protocol and may write STABLE for **the unchanged selected value in its ACCEPT**.

This scan is not a second chance to choose a different candidate. It need not equal the first scan. Newly arrived lower-round records do not require changing the value or restarting the scan.

The scan must contain this round's exact confirmed records. Missing them is an incomplete/invalid read, not evidence of success.

### 7.6 Step 5 — write STABLE

Write STABLE referencing this round's ACCEPT and wait for confirmation.

No further coordination scan is required before reporting that this decision is stable. Higher rounds beginning after step 4 cannot revoke it; they must inherit compatible values. A delayed STABLE is safe for the same reason.

Only this round's originating live execution writes its receipt. A restarted process never guesses that an old round passed step 4. It starts a fresh round and follows all five steps; later rounds can finish the same value.

## 8. Reference pseudocode

This is protocol pseudocode, not a substitute for the existing application Push gate. I/O errors propagate as unknown/unavailable, not empty views.

```python
def finalize(parent, prepared_publication, retry_hint=None):
    require_verified_stable_parent_or_root(parent)
    require_uploaded_verified_dependencies(prepared_publication)

    counter = 0 if retry_hint is None else checked_add_one(retry_hint)
    round = Round(counter, fresh_attempt_id())
    proposal = build_proposal(parent, round, prepared_publication)
    own_ref = put_and_confirm(proposal)                  # WRITE 1

    first = scan_decision(parent)                       # SCAN 1
    first.require_exact_record(own_ref)
    if first.has_stable:
        return learn_verified_decision(first.stable_refs)
    if first.max_round > round:
        return Contended(first.max_round.counter)
    first.require_no_accept_for(round)

    prior = first.highest_accept_below(round)
    if prior is None:
        selected = own_ref
    else:
        accepted = get_and_verify_accept(prior)
        selected = verify_selected_proposal(accepted.selected_ref, parent)

    accept = build_accept(parent, round, own_ref, selected)
    accept_ref = put_and_confirm(accept)                # WRITE 2

    second = scan_decision(parent)                      # SCAN 2
    second.require_exact_record(own_ref)
    second.require_exact_record(accept_ref)
    if second.has_stable:
        return learn_verified_decision(second.stable_refs)
    if second.max_round > round:
        return Contended(second.max_round.counter)

    # Do not recompute 'selected' here.
    receipt = build_stable(parent, round, accept_ref)
    receipt_ref = put_and_confirm(receipt)              # WRITE 3
    return Finalized(selected, receipt_ref)
```

`put_and_confirm` does not mean a CAS write. It sends exact bytes to the unique logical key and succeeds only on a confirmed outcome. The simple implementation ends the round on an ambiguous write. An exact GET may establish the outcome, but absence after a timeout does not prove the request cannot arrive later.

## 9. App-visible outcomes and local bases

| Protocol/application outcome | User-visible meaning | Local bases |
|---|---|---|
| `PublishedMine` | A verified stable receipt selects this operation's exact proposed token | Advance under existing local-state checks |
| `ExistingStable` / `FinalizedOther` | The decision selected another original proposal, possibly completed by this attempt | Do not mark this operation's local changes as pushed |
| `Contended` | A higher round interrupted this attempt | Keep prior bases and local content |
| `OutcomeUnknown` | A write/read outcome was not established | Keep prior bases; fresh discovery on retry |
| `Unavailable` / `IntegrityError` | Required state cannot be safely used | Stop affected mutation; retain local content |

A `FinalizedOther` result advances the known remote history but not the claim that this user's changes were synchronized. The user can review/rebase against that stable successor and Push again. No automatic publication of unreviewed changes is implied.

If the next parent is different, start a wholly new decision and perform all phases again. Preparation is never carried across parents.

## 10. Timeouts, pauses, restart, and retries

No phase record expires. Timeouts limit local waiting or retry policy; they never revoke an ACCEPT or STABLE.

| Interruption | What remains possible | Next attempt |
|---|---|---|
| Before PROPOSAL storage | No record, or a delayed harmless proposal | New round |
| After PROPOSAL, before ACCEPT | Candidate exists but has not been selected by this round | Higher round can proceed; no wait for owner |
| ACCEPT write timed out | ACCEPT may still arrive | New round; never emit old STABLE without old round's required validation |
| After confirmed ACCEPT, before final scan | Another round may inherit this value | New round, using normal inheritance |
| Final scan succeeded; STABLE is delayed | Decision's value is already protected | New rounds preserve it; late receipt is compatible |
| STABLE outcome unknown | It may already be published | Exact GET/discovery can resolve; otherwise retain unknown status |

No persistent local operation journal, outbox, or startup replay is required. Keep the ordinary last stable base and local content. Restarted attempts get new IDs and discover remote records again.

For the initial implementation, send each control phase once; bound automatic retries across new rounds and use backoff. Duplicate delivery of identical immutable bytes must remain logically harmless, but never reuse a round with changed bytes or let a transport invent a later phase.

There is no guaranteed finite completion time under endless competing attempts. After contention subsides and enough operations succeed, a client can select a round above the finite observed set and finish without other clients being online.

## 11. Normal Pull and canonical history

For each stable parent, look in that parent's decision collection for a STABLE receipt. Verify it and follow its selected original proposal. Continue from that proposal's exact token to discover the next decision.

```text
virtual root
    → stable original proposal P1
    → stable original proposal P2
    → ...
```

PROPOSAL and ACCEPT alone are not normal Pull targets. Multiple receipts selecting the same proposal are one publication, not additional versions. Do not rank receipts or candidates by timestamp, round, arrival order, or branch length to derive canonical history.

A Pull review pins the exact publication token and its verified stable proof, not a bare ambiguous commit ID. Apply uses that exact reviewed publication. A new stable descendant does not invalidate the pin's ancestry. Retain ordinary authorization, integrity, local-change and scope checks.

A read-only client need not execute the write-side scans. Delayed discovery of STABLE can leave it on an older stable version. Missing referenced data is `Unavailable` or `IntegrityError`; it is never permission to substitute a losing candidate.

A cold client validates a chain from the virtual root (or an already trusted exact local checkpoint). Cached verified immutable proof chains avoid repeating that work. This protocol does not supply a mutable latest-head shortcut or a new trusted checkpoint service.

## 12. Why finality holds: proof sketch for this variant

Fix one decision. Suppose round r completed its second scan selecting value V.

Its ACCEPT(r,V) was confirmed before that scan began. For every higher round q, PROPOSAL(q) cannot have been confirmed before r's second scan began, or r would have observed the higher round and stopped.

Therefore, any such q that proceeds to its first scan does so after ACCEPT(r,V) was confirmed. Its complete-at-start scan must include that ACCEPT.

Consider the smallest higher round q that writes an ACCEPT with a different value. Its first scan must have seen ACCEPT(r,V). It selects the highest-round earlier ACCEPT, so that chosen ACCEPT has round at least r and less than q. By minimality of q, every such ACCEPT has value V. Thus q cannot accept a different value: contradiction.

Consequently, every higher successful round carries V forward. If a lower round also successfully completed its second scan, applying the same argument from that lower round forces r to share its value. All completed rounds and all protocol-valid STABLE receipts agree.

This argument does not require STABLE(r,V) itself to arrive promptly. It does require the scans to include confirmed preexisting records, phase ordering, unique rounds, immutable state, and the inheritance rule.

Applying the result independently to each exact stable parent makes the root-connected canonical history append-only. The proof is a design argument for this custom variant, not a production verification of the provider implementations.

## 13. Race examples

### Delayed STABLE

```text
B: PROPOSAL(10)
B: scan → selects B
B: ACCEPT(10, B)
B: final scan → succeeds
B: STABLE(10, B) delayed

C: PROPOSAL(11)
C: scan sees ACCEPT(10, B) → must select B
C: ACCEPT(11, B)
C: final scan → succeeds
C: STABLE(11, B) stored

A: safely Pulls B
B: delayed STABLE(10, B) arrives → same result
```

### Delayed ACCEPT

```text
B: PROPOSAL(10), first scan succeeds
B: ACCEPT(10, B) delayed

C: PROPOSAL(11), sees no ACCEPT, selects C
C: ACCEPT(11, C), final scan succeeds, STABLE(11, C)

B: ACCEPT(10, B) finally arrives
B: required final scan sees round 11 / STABLE and stops
```

B's late ACCEPT is not a competing publication. B must never skip its final scan because an upload was slow.

### Two proposals before either ACCEPT

Both may pass their first scan and briefly store different ACCEPT values, depending on timing. The lower round cannot finalize without a second scan that checks for the higher round. A round that already completed that scan has its accepted value carried forward by later rounds. Different ACCEPT values are therefore not automatically an integrity error; different STABLE values are.

## 14. Request, storage, and latency accounting

For an uncontended round on a known stable parent:

```text
requests = 3 writes + pages(scan 1) + pages(scan 2) + verification GETs
```

With one page per scan and no unknown bodies: five requests. PROPOSAL replaces the old standalone publication, so the net record count is two additional control records (ACCEPT and STABLE), plus a slightly larger proposal envelope.

```text
T_metadata ≈ 3 × small_write_latency + 2 × one_page_scan_latency
```

Compared with the old idealized one-list/one-publication path, the delta is two small writes and one scan. These are protocol calculations, not measured provider performance.

A round stopped after its first scan uses one write plus one scan. A round stopped after its second scan uses two writes plus two scans, excluding extra GETs and pagination. Inheriting another writer's value may complete the decision but leave the user's own content requiring a new decision.

Normal Pull discovers STABLE, reads ACCEPT, and reads the selected PROPOSAL; it may also read the round owner's different proposal and uncached ancestry for validation. Cached bodies remove those reads. Do not promise a universal one-GET Pull or include hidden GETs inside a claimed five-request Push budget.

Per-parent scan scope is mandatory for practical performance. Aborted rounds can still accumulate within one decision; retain them for safety and measure that growth. No protocol-record compaction is included in version 1.

## 15. Required acceptance tests

1. One client completes in three control writes and two one-page scans, with no preliminary coordination scan.
2. Concurrent root publications finalize one exact token.
3. Lower round is paused before either write, either scan, and STABLE; no schedule produces conflicting stable values.
4. Delayed lower ACCEPT after a higher STABLE does not enter history.
5. Delayed STABLE after a higher round has started selects the same original proposal.
6. Highest prior ACCEPT is inherited even when its writer crashed, timed out, or has no STABLE.
7. Unreadable highest ACCEPT/selected proposal blocks safely; no fallback to a lower value.
8. Both scans fail closed on incomplete enumeration, missing required own records, permission errors and timeouts.
9. New attempts after restart use fresh IDs and never resume a guessed old phase.
10. Duplicate identical copies deduplicate; one round/phase with different bytes errors.
11. Existing stable successor is respected even when discovered only after optimistic PROPOSAL storage.
12. Pull of a cached stable ancestor remains valid while descendants finalize.
13. Finalizing another writer's candidate does not falsely advance the current user's local bases.
14. Namespace reset cannot mix live delayed writes from an old history into a new decision.
15. Next parent always runs preparation again; no cross-parent ballot reuse or amortization.

## 16. Bounded model check performed for this specification

A small executable state-space model accompanies this specification. It uses four fixed, uniquely ordered attempts for one decision; each proposes a distinct value. Control writes are atomic, both scan snapshots are fresh and atomic, scan-result delivery can be delayed, and any execution may pause between actions. A discovered stable receipt stops a round from issuing further decision records.

The normal model explored **36,214 reachable states and 97,875 transitions**, finding no conflicting STABLE values. Two deliberately broken two-attempt variants produced conflicts: removing inheritance and removing the higher-round check after ACCEPT.

This is bounded supporting evidence, not a proof of arbitrary executions or a real-provider test. It does not model arbitrary torn paginated scans, dynamically allocated retry rounds, deletion, malformed bytes, multiple streams, or application base updates. The complete-at-start pagination argument is in section 12 and still needs implementation-level verification. Pausing models interruptions; the checker is not a full network transport/recovery model.

Run the companion checker with Python 3.9 or later:

```text
python mallard_stable_protocol_model.py --attempts 4
```

## 17. Implementation boundaries

Keep protocol/layout/schema numbers at `1`, but change the storage-format discriminator so an old earliest-sibling writer cannot mistake this for its old format. Use a clean fresh destination at cutover; no compatibility reader, migration, or dual-write path is specified.

Do not promise that existing local pin shapes can remain unchanged: a bare commit ID is insufficient if exact proposal identity and stable receipt references are not already stored. Add only the ordinary stable proof/identity fields needed by review and base tracking; this is not an operation journal.

No extra coordinator, lock owner, peer vote, CAS operation, conditional create, session expiration, file timestamp rule, persistent pending-Push journal, lease, amortized preparation, or leadership service is introduced.

**Publication authority comes from a completed per-parent decision, exposed through STABLE. Later rounds preserve its value instead of trying to invalidate delayed writes.**

## References and evidence

- [S1] Eli Gafni and Leslie Lamport, *Disk Paxos*, sections 3.1–3.2. Shared-storage write/read phases and highest-accepted-value selection are the lineage; this document's append-only protocol is a separate adaptation. https://lamport.azurewebsites.net/pubs/disk-paxos.pdf
- [S2] Leslie Lamport, *Paxos Made Simple*. https://lamport.azurewebsites.net/pubs/paxos-simple.pdf
- [S3] AWS, *Amazon S3 Strong Consistency*. https://aws.amazon.com/s3/consistency/
- [S4] Cloudflare, *R2 consistency model*. Direct APIs versus cache-backed access are distinguished. https://developers.cloudflare.com/r2/reference/consistency/
- [S5] Google Drive, *files.list*. Pagination and incomplete-search behavior require care; this is not certification of the scan contract. https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list
- [S6] Repository baseline read for this request: `Git4Agent/mallard-app`, `docs/DESIGN_CAS_FREE_STORAGE_REFACTOR.md`, status “Planned; not implemented,” last reviewed September 19, 2026. This specification is a standalone proposal and has not been committed to that repository.

# Bounded verification: Minimal portable-tail conflict resolver

- Status: Executable bounded model
- Date: August 10, 2026
- Model: [verify_portable_tail_conflict_model.py](../scripts/verify_portable_tail_conflict_model.py)
- Plan: [PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
- Compatibility: current numeric schema `1` only; no migration or legacy state

## 1. What is modeled

The executable model checks the simplified root-only resolver:

```text
portable tail = codec
              + partition kind
              + start record count
              + end record count
              + cumulative tail state
```

It models:

- one already-flattened canonical root history;
- `WholeThread` and `FixedPaginatedTail` layouts;
- direct equality of validated P/L/C tail values;
- `Sync(Current)`, `Sync(Advanced)`, `LocalOutstanding`,
  `CloudOutstanding`, and `Divergent`;
- Push CAS win, loss, ambiguous win, ambiguous loss, and candidate-descendant
  recovery;
- journal-free atomic zero-target Pull mutations;
- journaled one-target Pull transactions, including Divergent keep-both;
- fixed provider IDs across apply/recovery;
- layout stored directly on the binding;
- bounded local-document mutations with exact before/after/third-value states;
- crashes before every modeled durable transition;
- terminal resource-release crashes;
- precommit staleness, provider target collisions, and local-document corruption;
- preservation of both histories during keep-both; and
- unconditional retention of old provider sources.

Physical `history_base` source files are outside the state model: they are
flattened into the one root history before exploration. Subagent snapshots are
modeled only as deliberately excluded artifact values.

The model deliberately excludes portable subagent/topology state, source
cleanup, and containment refinement between unequal L and C.

Push planning is also outside the transition state graph. The model assumes an
apply step has verified the reviewed source identities/EOFs/raw hashes,
reconstructed once, hashed/verified each output once, and only then created the
exact candidate represented by the initial `CandidateUploaded` state.

Materializing Pull planning is likewise outside the transition graph and is
assumed to be metadata-only: it pins immutable commit/manifest/object
descriptors, fixed IDs/paths, and local preconditions without downloading or
constructing output. The modeled `Prepared` state begins later, after apply has
downloaded/verified each pinned input object once, reconstructed/reidentified
the one remote target once, validated its typed invariants, serialized/hashed
its emitted output once, durably staged it, derived exact local mutations, and
created the journal. Pre-journal
download/reconstruction failures have no visible transition and are outside the
state graph. Divergent C reuses the existing local provider alias and is not a
target. Recovery may rehash the staged target but never downloads or
reconstructs it.

## 2. Tail codec checks

The model independently computes real SHA-256 for three v1 primitives using the
plan's exact `LP_TUPLE` encoding:

```text
empty WholeThread state
  85ca60a860dfac615f75141b865e6191feea154a72dac2b1007c8d5dcdedf5fd

canonical message fingerprint for UTF-8 "m4"
  c594cce30c1ea89769fc90b54bc0608fec7e85e42ba615e619fdd6f2074b8c46

FixedPaginatedTail(start=3) after appending "m4"
  c79c5b174e0f47c22fc57bebe482c8ab6524129b31d1ee0cf82799c4951aca44
```

The five-field value has no second fingerprint. The transition model uses its
collision-free canonical preimage so state exploration tests transition logic,
not SHA collision resistance.

## 3. Checked safety properties

### 3.1 Shared classifier

The executable cases assert:

| P/L/C condition | Result |
| --- | --- |
| `P == L == C` | `Sync(Current)` |
| `L == C != P` | `Sync(Advanced)` |
| `C == P != L` | `LocalOutstanding` |
| `L == P != C` | `CloudOutstanding` |
| all unequal | `Divergent` |

It also checks the accepted conservative false conflict:

```text
P = m0
L = m0,m1
C = m0,m1,m2
=> Divergent
```

No prefix/containment oracle refines that result.

### 3.2 Subagent exclusion

The model evaluates the cross-product of three pin, three local, and three cloud
subagent snapshots: 27 cases. Every case with an equal root tail remains Sync,
regardless of subagent bytes or membership.

### 3.3 Push

For every explored Push state:

1. The fixed provider ID never changes.
2. The binding layout remains the candidate layout.
3. Local accepted state advances only after the candidate is in verified HEAD
   ancestry.
4. CAS loss never promotes the candidate locally.
5. Candidate-descendant recovery installs the exact candidate accepted pin
   before a later Refresh handles the descendant.
6. A local document third value after HEAD commitment blocks rather than being
   overwritten.
7. Complete/abort terminal recovery releases pending resources idempotently.

### 3.4 Pull

For every explored Pull state:

1. Fixed logical/provider IDs never remint.
2. Zero-target actions create no provider target, operation journal, or pending
   resource; exact-before applies, exact-after is idempotent, and a third value
   is stale.
3. Every materializing action installs exactly one verified remote target.
4. Divergent Pull moves the old provider alias and local history unchanged from
   T to C, then installs the one remote target as T.
5. C keeps the old provider ID, local binding layout, and observed tail, has no
   accepted cloud pin, and consumes no newly allocated provider target ID. T
   receives the remote layout and accepted pin.
6. Bindings and SyncConfig accept only exact-before or exact-after mutation
   state. A third value blocks.
7. Precommit staleness safely aborts.
8. Foreign target collision safely aborts without overwriting the target.
9. Old provider source histories remain present in every state.
10. Complete/abort terminal recovery releases pending resources idempotently.
11. Every old-source receipt has the literal outcome `"retained"`; there is no
    source-cleanup transition.
12. The transition graph consumes an apply-generated staged target; it has no
    planning-time transcript download or output-construction state.

## 4. Explored state space

The current run reports:

```text
codec golden vectors: 3
classifier cases: 6
zero-target atomic Pull cases: 6
root-only subagent exclusion cases: 27
push: 10 scenarios, 102 states, 96 transitions, 80 recovery states
pull: 14 scenarios, 180 states, 216 transitions, 102 recovery states
total: 282 states, 312 transitions
terminal outcomes: complete=20, safe_abort=32, corruption_block=18
expected unsafe negative controls: 6
```

Every non-corrupt explored recovery path reaches `Complete` or a safe terminal
abort. Only an explicit third-value document corruption remains operator-blocked.

## 5. Negative controls

The model deliberately demonstrates six unsafe alternatives:

1. overwriting cloud without CAS loses remote history;
2. resetting accepted state loses cumulative tail history;
3. assigning the remote layout to preserved C loses C's fixed boundary;
4. reminting during recovery changes reviewed identity;
5. including subagent snapshot state creates a false root conflict; and
6. deleting the old source can lose a still-needed local/history input.

These controls must remain observable; a model that stops finding one has
probably stopped checking the corresponding invariant.

## 6. Boundaries of the claim

This is not a proof of the Rust implementation or cryptographic primitives. It
does not model:

- filesystem parser correctness, symlink defense, byte/ordinal cutoff handling,
  newline-safe reads, or physical boundary mapping;
- S3/R2 consistency behavior beyond the abstract CAS outcome;
- exact JSON serialization, fsync implementation, or cross-process lock code;
- the complete standalone root/subagent encoder and reidentifier;
- strict schema-1 raw JSON decoding;
- directional missing/Unknown evidence gathering;
- checkpoint `Ready`/`Detached` remapping;
- mixed-resource batch publication;
- actual bounded mutation serialization size; or
- metadata-only Pull planning and the pre-journal apply download/reconstruction
  implementation;
- CLI app-server protocol and UUIDv7 generation.

Those remain conventional unit/integration/crash-matrix obligations in the
backend plan. The bounded result is valid only if:

- the evidence gatherer supplies truthful P/L/C values and ancestry gates;
- document mutation inspection distinguishes exact-before, exact-after, and
  third values correctly;
- immutable object size/SHA checks, typed output-invariant validation,
  single-pass serialization/output hashing, staged target identity, and
  no-clobber checks are authoritative;
- immutable commit/manifest/object verification is correct; and
- old provider sources are never deleted by this transaction.

## 7. Running the model

```bash
python3 scripts/verify_portable_tail_conflict_model.py
```

Expected final line:

```text
PASS: no bounded counterexample in the simplified model
```

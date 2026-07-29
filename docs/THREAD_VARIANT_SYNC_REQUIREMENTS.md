# Thread Variant Sync Requirements

## Contract

Pull may add or fast-forward versions, but MUST NOT silently switch an existing
local conversation to another branch.

- A continues $(a,b,c_1)$ after Pull.
- B continues $(a,b,c_2)$ after Pull.
- Both versions remain explicitly openable and continuable.
- Pushing one version updates only that version.

## Notation

A transcript is a linear message path

$$
S=(m_0,\ldots,m_n), \qquad parent(m_i)=m_{i-1}.
$$

Message ID and parentage determine ancestry. Text and file digest do not;
SHA-256 is an integrity check only.

For transcripts $X,Y$:

$$
X\preceq Y \iff X\text{ is an ID-for-ID prefix of }Y,
\qquad
X\bowtie Y \iff X\npreceq Y\land Y\npreceq X.
$$

Unknown ancestry is treated as divergence.

| Symbol | Meaning |
| --- | --- |
| $V_g$ | Portable variants in conversation group $g$ |
| $C_g(v)$ | Cloud transcript for variant $v$ |
| $L_{r,g}(v)$ | Transcript on replica $r$ |
| $p_g$ | Cloud-primary variant |
| $q_{r,g}$ | Locally preferred variant |
| $\beta_{r,p,g}(v)$ | Provider thread/session ID for $v$ in local profile $p$ |
| $a_{r,p}$ | Provider thread actively opened by the user |

## Invariants

1. Each conversation has one portable **conflict_group_id**.
2. Each branch has one portable **variant_id**, stable across no-op and
   fast-forward operations.
3. Provider thread/session ID is a machine-local handle, not variant identity.
   The same variant may use different provider IDs on different replicas.
4. $\beta$ is injective within a provider profile: visible variants have unique
   provider IDs.
5. Cloud primary, local preference, and active provider thread are independent.
6. Ordinary sync preserves them:

   $$
   p'_g=p_g,\qquad q'_{r,g}\equiv q_{r,g},\qquad a'_{r,p}=a_{r,p}.
   $$

   Here $q'\equiv q$ means the same local branch and provider ID; its transcript
   may only remain equal or fast-forward.
7. All known variants are preserved by default. Replacement, deletion, or
   promotion requires explicit reviewed approval.
8. Ambiguous ancestry or binding recovery MUST block or preserve both versions;
   it MUST NOT guess from title, text, or provider ID alone.

## Push/Pull rules

Let $L$ be the reviewed local version and $C$ the corresponding cloud version.

| Relation | Push | Pull |
| --- | --- | --- |
| $L=C$ | No-op | No-op |
| $C\prec L$ | Fast-forward cloud to $L$ | Keep $L$ |
| $L\prec C$ | Block; Pull required | Fast-forward local to $C$ |
| $L\bowtie C$ | Keep $C$; add $L$ as a variant | Keep $L$; add $C$ as a variant |
| Unknown | Treat as divergence | Treat as divergence |

A later Push from an alternate MUST update its existing **variant_id**, not
create a duplicate.

A local version whose lineage the active codec cannot prove (for example,
id-less legacy Codex rollouts written before CLI 0.146) MUST remain Blocked
for Push — decision of record, 2026-07-28. Store-only archival of such
transcripts is deferred future work (PLAN_LEGACY_ROLLOUT_ARCHIVAL.md).

## Divergence and provider IDs

If local and cloud diverge under the same variant ID $v$:

1. Cloud retains $v$.
2. Local receives $v_L\notin V_g$ without changing its content or provider ID:

   $$
   L'(v_L)=L(v),\qquad \beta'(v_L)=\beta(v).
   $$

3. The incoming cloud version receives
   $t_C\notin ran(\beta_{r,p,*})$.
4. Existing local content always wins an ID collision; the incoming version is
   renamed.

Materialization under a new provider ID MUST rewrite all structural filenames,
metadata, selected sidecars, indexes, and **cwd** references consistently while
preserving message IDs, parentage, content, timestamps, and tool records.

If safe rewriting cannot be verified, preserve the version as stored-only and
do not advertise it as resumable. Repeated Pull MUST create no duplicate
transcript, provider ID, sidecar, index row, or binding.

## A/cloud/B result

Cloud contains:

$$
C_g=\{v_1\mapsto(a,b,c_1),\ v_2\mapsto(a,b,c_2)\}.
$$

| Replica | Before Pull | After Pull |
| --- | --- | --- |
| A | $q_A=v_1,\ \beta_A(v_1)=A\text{-}T1$ | $q'_A=v_1,\ \beta'_A(v_1)=A\text{-}T1,\ \beta'_A(v_2)=A\text{-}T2$ |
| B | $q_B=v_2,\ \beta_B(v_2)=B\text{-}T2$ | $q'_B=v_2,\ \beta'_B(v_2)=B\text{-}T2,\ \beta'_B(v_1)=B\text{-}T1$ |

Therefore A is not switched from $c_1$, B is not switched from $c_2$, and each
can explicitly continue either version. On a fresh replica, Pull creates two
distinct provider IDs but does not launch or activate either thread.

## Missing and deletion

| Local | Cloud | Default |
| --- | --- | --- |
| Missing | Present | Pull materializes when safe |
| Present | Missing | Push adds; Pull preserves local |
| Missing | Missing | No-op |

Absence is not deletion. Deletion requires an explicit tombstone. A
delete/modify race is divergence and preserves the modified content.

## Storage and safety

- Portable identity is (**conflict_group_id**, **variant_id**), not provider ID.
- Cloud stores $p_g$; machine-local state stores $q_{r,g}$ and $\beta_{r,p,g}$.
- Portable state excludes credentials, absolute local paths, preferences,
  plans, backups, receipts, and local bindings.
- Local mappings are bounded, validated, revisioned, and atomically saved.
- A representation unsafe for schema-4 readers uses a new schema, one reviewed
  cutover, and no permanent dual writers.
- Push remains immutable and SHA-256 verified.
- Cloud writes use compare-and-swap against reviewed generation and manifest.
- Pull revalidates cloud and local digests, provider-ID availability, binding
  revision, and expiry.
- Replacement requires a matching reviewed digest and recoverable backup.
- Provider writes use bounded no-follow reads, canonical targets,
  symlink/special-file rejection, staging, verification, and atomic rollback.

## Acceptance

Tests MUST prove:

1. the identical, prefix, divergence, unknown, and missing matrix;
2. the A/cloud/B result above;
3. unique provider IDs on fresh replicas and under collisions;
4. repeat-Pull idempotence;
5. no implicit primary, preference, or active-thread change;
6. stale cloud, local, binding, ID-allocation, and expiry rejection;
7. transactional recovery under failure injection; and
8. safe rewrite and continuation for every supported provider.

# PLAN: Credential redaction on the wire — one-way scrub + dual-pin identity

Status: IMPLEMENTED (2026-08-21, uncommitted; full bar green — workspace
614 tests + frontend 219, clippy -D warnings, fmt; live aimock e2e green
with planted credentials riding the Read-tool fixture). Drafted 2026-08-20;
revised same day (D1 resolved by user: typed deterministic placeholders
like `<password>`, not bare `REDACTED`). Deltas discovered during
implementation are recorded in §15.
Sibling of `PLAN_CODEX_IN_CWD_PATH_REMAP.md`: the same
push/pull-boundary transform architecture and the same dual-pin identity
carry a second, one-way wire transform. Detection algorithm adopted from
`entireio/cli` (MIT, which in turn vendors the `betterleaks` rule
corpus, a gitleaks fork, also MIT) — sources studied at
`redact/redact.go`, `redact/providers.go`, `redact/fingerprint.go`, and
`cmd/entire/cli/checkpoint/redact_cache.go`. The detector is adopted
wholesale; the replacement scheme deliberately diverges (§4.3).

One sentence: **no detectable credential ever reaches the cloud, for
either provider** — every byte stream that leaves the machine (Codex
rollouts, Claude transcripts, sidecars, provider state, content-derived
descriptor metadata) passes a deterministic redaction encoder at the push
boundary, secrets are replaced by typed placeholders from a frozen
vocabulary (`<password>`, `<anthropic-api-key>`, `<secret>`, …), the
wire form is provably a redaction fixpoint at both push and pull, and
the existing dual-pin checkpoint absorbs the local≠wire split with no
new comparison machinery.

Compatibility (user-set, 2026-08-20): **clean cutover; no migration, no
legacy execution path, schema constants unchanged** (`BUNDLE_SCHEMA`
stays 1; field additions ride the wipe). Local state (`~/.mallard`) and
cloud data are wiped for retesting; there are no customers, so no
fail-closed versioning choreography for old data is designed here.

## 1. Problem

- Agent transcripts are the worst credential surface in the ecosystem:
  users paste keys into prompts, tools `cat .env`, shells echo
  `DATABASE_URL`, and every one of those bytes lands verbatim in
  `~/.codex/sessions/**.jsonl` and `~/.claude/projects/**.jsonl`.
- Mallard's Push uploads those bytes to object storage. Locally-held
  secrets are the user's own business; **cloud-held secrets are an
  incident**. R2 buckets outlive laptops, get shared, get misconfigured.
- The transform cannot be reversible: a cloud object from which the
  secret is recoverable (encrypted, encoded, hashed-with-oracle) has not
  solved the problem. Redaction is therefore **lossy by design**, which
  is exactly why it needs the dual-pin treatment — local bytes and wire
  bytes permanently disagree, and every tail/digest comparison must stay
  on its own side of that split.
- Both providers, all record roles: user messages, assistant responses,
  tool calls, tool results, thinking text — plus the auxiliary artifacts
  that ride along (Claude `todos`/`file-history` sidecars, provider-state
  memory files, `display_title`).

What already exists and is reused wholesale:

- The **dual-pin checkpoint** (`ConversationCheckpoint.accepted` = wire
  pin, `accepted_local_tail` + `observed_tail` = local pins,
  `conversation_tail.rs:547`) and the **pin-relative 4-input relation**
  (`conversation_tail_relation.rs:94` — `accepted`, `accepted_local`,
  `local`, `cloud`; never cross-form).
- The **wire/local codec twins**: `PORTABLE_TAIL_CODEC_V1` /
  `PORTABLE_TAIL_CODEC_WIRE_V1` (Codex), `CLAUDE_PORTABLE_TAIL_CODEC_V1`
  / `CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1` (Claude). Claude's wire label
  exists today as a **relabel** of the local fold ("verbatim bytes …
  one shared state", `commands.rs:9337`); this plan makes it a genuine
  second form and deletes the relabel shortcut in the same change.
- The **push/pull-boundary transform slots**: Codex
  `StandaloneBuildMode::{Encode,Decode}`
  (`codex_standalone_builder.rs:46`) with per-record gates
  (`encode_record_gated`, `verify_wire_record`,
  `codex_path_map.rs:417,:437`); Claude's push capture
  (`commands.rs:9290`) and pull staging/materialization.
- Refresh, watcher, cursor riding, byte-pin caches: **zero change**,
  same as the path remap — the transform exists only at push build and
  pull staging.

## 2. Reference algorithm (entireio/cli)

What we adopt, with the reasons it fits mallard:

1. **Layered leaf detection** (`redact.go` `detectAllLayers`), every
   layer a pure function of one string:
   - L1 entropy: candidates `[A-Za-z0-9+_=-]{10,}` (charset excludes
     `/` so whole paths never match as one token), Shannon entropy
     > 4.5. A guard refuses to start a match on the trailing letter of a
     JSON escape (`\n` + `model…`), so replacement can never fabricate an
     invalid escape.
   - L2 vendor rules: the betterleaks corpus — several hundred known
     secret shapes (AWS, GitHub, Slack, Stripe, private-key PEM blocks,
     …). Findings whose value is a known placeholder are skipped **only
     on exact match** (splitting a greedy finding at a placeholder head
     can leak the tail).
   - L3 deterministic provider prefixes for low-entropy formats the
     other layers miss in isolation (`providers.go`: `sb_secret_…`,
     `sbp_…`); deliberately **not** `\b`-anchored, because a glued
     prefix (`FOO_sb_secret_…`, escape-abutting `…\nsb_secret_…`) would
     otherwise slip through. Over-redaction of look-alike identifiers is
     the accepted cost.
   - L4 credentialed URIs (`scheme://user:pass@host`).
   - L5 connection strings: JDBC, database URLs with password query
     keys, keyword DSNs (`host= user= password=`), semicolon strings —
     each gated on an actual non-placeholder password being present.
   - L6 custom user rules (mallard v1: not exposed; see §11).
   - L7 bounded credential key/value pairs
     (`(db|pg|mysql|…)…(password|passwd|pwd)\s*=\s*value`) plus, inside
     JSON, DB-shaped keys and bare `password` keys in objects that also
     carry host+user (`credentialContext`) — forcing redaction even for
     low-entropy values like `hunter2`.
   - (Their L8 PII and network-backed OpenAI Privacy Filter layers are
     **not** adopted — out of scope, and a network call inside a push
     gate is disqualifying.)
2. **Region merge and splice** (`applyRegions`): sort (start asc, end
   desc, label asc), merge overlaps, splice replacements. Deterministic
   by construction. Their regions already carry labels (their PII layer
   emits typed `[REDACTED_<LABEL>]` tokens) — the labeled-region
   machinery is the precedent §4.3 builds on.
3. **JSONL-aware application**: parse each line, walk string leaves,
   collect `(key, original, redacted)` replacements, then apply them to
   the **raw line text** by replacing JSON-encoded originals (keyed
   replacements restricted to value position). Untouched content keeps
   its original bytes; lines that don't parse as JSON get the raw string
   pass.
4. **The skip-list** (`shouldSkipJSONLField` / `shouldSkipJSONLObject`),
   which is hard-won operational knowledge: keys ending in `signature`
   (Claude thinking signatures are high-entropy base64 — redacting one
   breaks transcript replay with "Invalid `signature` in `thinking`
   block"), keys ending `id`/`ids`, path/directory fields (`file_path`,
   `cwd`, `root`, `directory`, `dir`, `path`), and `type:image*/base64`
   objects wholesale.
5. **Placeholder discipline = fixpoint**: their replacement is the
   single token `REDACTED`; the placeholder family (`redacted`,
   `[redacted]`, `<redacted>`, `changeme`, `${VAR}`, bracketed
   `<your-password>` doc shapes, `xxx…` mask runs) is allowlisted in
   every value-gated rule, so `redact(redact(x)) == redact(x)` holds and
   re-scanning wire bytes is a meaningful integrity check rather than
   noise. **We diverge on the token, not the discipline** (§4.3): their
   `isBracketedPlaceholder` rule already recognizes `<lowercase-name>`
   shapes as placeholders, which is precisely what makes our typed
   placeholders natively fixpoint-inert in their own algorithm.
6. **Compositionality** (pinned by their own tests and load-bearing for
   their prefix cache): per-line statelessness gives
   `redact(A+B) == redact(A) + redact(B)` for newline-terminated `A`.
   In mallard's vocabulary: **per-record independence**, the exact
   property that keeps a re-encoded wire prefix byte-stable under
   append, i.e. the property the accepted wire pin depends on.
7. **Ruleset identity** (`redact_cache.go` `redactionFingerprint`,
   `fingerprint.go` `ConfigFingerprint`): they fingerprint CLI version +
   rule config and refuse to reuse cached output across a fingerprint
   change. Mallard has the stricter version of the same problem (§3.3):
   the accepted wire prefix must stay byte-stable across *pushes*, not
   just across cache hits, so the fingerprint becomes part of the cloud
   thread's identity rather than a cache key.

What we do **not** adopt: the bare `REDACTED` replacement token
(replaced by the typed placeholder scheme, §4.3, user decision D1), the
Go `betterleaks` dependency itself (we vendor a curated Rust rule table,
§4.1), the incremental prefix cache (their Stop hook redacts on every
checkpoint; mallard redacts only at user-initiated Push, which already
walks every record — §9), PII, OPF, and their scanner-degradation
machinery (our ruleset is compiled in; it cannot degrade at runtime).

## 3. Architecture

### 3.1 One-way wire transform

Define per provider the wire encoder over one record (Codex) or one
transcript line (Claude):

```
Codex:  W(x) = redact(pathenc(x))      — path tokens first, then scrub
Claude: W(x) = redact(x)               — v1 has no Claude content pathenc
```

- `pathenc`/`pathdec` are the shipped `codex_path_map` codec, untouched.
- `redact` has **no decode**. Pull-side decode is `pathdec` for Codex and
  identity for Claude; placeholder spans (`<password>`,
  `<anthropic-api-key>`, …) materialize verbatim on every machine,
  including the origin machine after a cloud-ahead pull.
- Order is forced: the fixpoint certificate (§5) must hold on the final
  uploaded bytes, so `redact` runs last. It also means the root token
  spelling (`/__mallard_root__`, low entropy, `/`-separated) is what the
  detector sees in place of the project root — it can never be flagged,
  and a secret's accidental containment of the project root has already
  been tokenized before scanning.
- The path-remap HARD INVARIANT (out-of-root paths are never *remapped*)
  stands unchanged. Redaction is a different transform with a different
  contract: a secret-shaped span is scrubbed wherever it appears,
  including inside an out-of-root path spelled in prose
  (`/Users/x/.aws/AKIA…` must not reach the cloud because it is inside a
  path). Structural path *fields* are protected by the skip-list; this
  bites only free-text path mentions whose segments look like secrets —
  over-redaction accepted (D3).
- Like `pathenc`, `redact` is **canonicalizing, not injective**:
  `decode(W(x)) == scrub(canonicalize(x))`, never `== x`. The A→B→A
  round trip restores A's canonical content *with typed placeholders
  where secrets were* — that is the product working as designed, not
  drift.

Consequences to state plainly (they fall out of one-way + the existing
materialization rules, `commands.rs:11344`):

- Machine B never sees the secret at all.
- Machine A keeps its secrets exactly until a cloud-ahead materialization
  of that thread: Codex `MaterializeCloud`/`KeepBoth` mint a fresh
  copy-on-write rollout (the secret-bearing predecessor file remains on
  disk, untracked); Claude `MaterializeCloud` writes **in place**, so the
  tracked transcript itself is scrubbed. Resume keeps working — thinking
  signatures are skip-listed, and history text reading `<password>` is
  legal content.
- `Sync(Current)` / `LocalOutstanding` pulls write nothing, as today —
  local secrets are untouched by any flow that doesn't materialize.

### 3.2 Dual-pin identity (reused as-is; Claude's relabel becomes real)

Nothing structural changes in the pin model — that is the point of
reusing it. What changes is *which bytes* the wire columns fold:

- **Codex**: `accepted.portable_tail` (wire codec) now folds
  `redact(pathenc(bytes))` instead of `pathenc(bytes)`. Local columns
  (`accepted_local_tail`, `observed_tail`, cursor, probe windows,
  byte-pin caches) are untouched — they never see wire bytes.
- **Claude**: today the wire tail is the local fold relabeled
  (`commands.rs:9337` — legitimate only because bytes were verbatim).
  With redaction the two forms genuinely diverge, so the wire tail
  becomes a real fold: `seed_claude_root` runs a second time over the
  redacted transcript bytes, and its tail — carrying
  `CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1` — is what the manifest and
  `accepted` pin. The relabel construction is deleted (clean cutover).
  The push review check (`seeded.observed_tail == action.pins.tail.local`,
  `commands.rs:9310`) stays on the **local** fold of the original bytes.
- The 4-input relation's cross-form layout equality
  (`conversation_tail_relation.rs:141` — partition kind and record
  counts must match between `accepted` and `accepted_local`) holds by
  construction: redaction is per-record and content-only; it never adds,
  removes, splits, or reorders records. This is a frozen requirement on
  the encoder, gate-checked (§5).
- **Form ledger extension (files).** Sidecars, state files, and any
  digest-pinned (non-tail) artifact follow the same one-form-per-column
  rule: manifest digests (`sha256` on file entries, retention dedup)
  are **wire-form** — computed over redacted capture output, exactly as
  they are computed over `published_bytes` today. Pull-apply
  `TargetExpectation`s are **local-form** (review-time reads of local
  files). Verification item V1 (§14) audits that no surviving
  comparison crosses the two.

### 3.3 Determinism and ruleset identity

The load-bearing property: **`redact` is a pure function of (record
bytes, ruleset)** — no cross-record state, no environment input, no
randomness, no time. Combined with per-record independence, a re-push of
a locally-extended thread re-encodes records `1..n` to byte-identical
wire output, the rebuilt wire tail extends the accepted wire pin, and
the existing fast-forward/witness guards pass. Every detector layer in
§4 satisfies purity by construction (the `credentialContext` heuristic
spans one record's own JSON tree only), and the placeholder chosen for a
span is a pure function of the winning rule (§4.3) — labels carry no
per-secret, per-thread, or per-machine state.

That leaves exactly one drift axis: **the ruleset itself** — which,
under D1, includes the rule→label map and the label spellings, because a
label rename changes wire bytes exactly like a pattern change does.
Entireio can re-redact the world when rules change (each checkpoint is a
snapshot); mallard cannot, because other machines hold `accepted` pins
over the old wire bytes and a prefix rewrite classifies as divergence
everywhere. V1 policy, matching the compatibility stance:

- One compiled-in ruleset, identified as codec **`mallard.redact.v1`**
  with a content fingerprint (SHA-256 over the sorted rule table:
  ids, patterns, thresholds, skip-list, **label vocabulary and
  rule→label map** — the `ConfigFingerprint` idea, minus runtime
  config).
- The cloud `ConversationThread` descriptor gains
  `wire_redaction: { codec, fingerprint }`, stamped at every push next
  to `portable_tail`. Not in-band: Claude wire bytes stay pure provider
  JSONL (nothing to strip on materialization), and Codex keeps only its
  existing in-band `__mallard_path_map__` marker.
- Push and pull both require `binary fingerprint == descriptor
  fingerprint`; mismatch is the typed blocker
  `redaction_ruleset_mismatch` (§8). Changing the ruleset — patterns or
  labels — is a breaking change resolved by the standing wipe, the same
  ethos as schema constants. Pinned multi-ruleset coexistence is
  explicitly a non-goal (§11).
- Same-machine repair (`remap_codex_conversation_paths`,
  `commands.rs:16510`) composes **path codecs only**. Repair rewrites
  *local* bytes; running `redact` there would scrub the user's local
  transcript as a side effect of moving a project directory. The repair
  pipeline is `pathdec_new(pathenc_old(x))`, frozen, with a test pinning
  that a planted local secret survives repair.

## 4. Detector: `mallard.redact.v1`

New provider-neutral module `crates/mallard-infrastructure/src/`
`credential_redaction.rs` (detector core + ruleset/label tables + record
walk + raw-line applier + gates + fingerprint). No new crates; `regex`
is already in the tree.

### 4.1 Layers (all always-on, all pure)

| # | Layer | Mallard realization | Replacement (§4.3) |
|---|---|---|---|
| L1 | Entropy | candidate regex `[A-Za-z0-9+_=-]{10,}`, Shannon > 4.5, JSON-escape start guard. Hex-only strings (git SHAs, digests) top out at 4.0 bits/char and UUIDs at ~log2(17) — structurally safe without special-casing. | whole match → `<secret>` |
| L2 | Vendor rules | curated Rust table ported from the betterleaks corpus (MIT, attributed in-module): AWS `AKIA/ASIA` + secret pairs, GitHub `ghp_/gho_/ghu_/ghs_/ghr_/github_pat_`, GitLab `glpat-`, Slack `xox[bpars]-`, Stripe `sk_live_/rk_live_`, Google `AIza`, SendGrid, Twilio, npm `npm_`, PyPI `pypi-`, PEM `-----BEGIN … PRIVATE KEY-----` blocks, JWT `eyJ…`-dotted triples, … Target ~60–120 rules, one `RegexSet` prefilter + per-rule anchored verify. Rule ids stable — they feed the fingerprint and error payloads. | whole match → typed label via the rule→label map (`<aws-secret-key>`, `<github-token>`, `<private-key>`, `<jwt>`, …; generic fallback `<api-key>`) |
| L3 | Provider prefixes | `sb_secret_`, `sbp_`, plus the AI-era set pushes actually contain: `sk-ant-` (Anthropic), `sk-proj-`/`sk-` bounded (OpenAI), `r8_` (Replicate), `hf_` (HuggingFace), `tvly-`, `pplx-`, `gsk_` (Groq), `xai-`. Unanchored on the left, per the entireio rationale. | whole match → typed label (`<anthropic-api-key>`, `<openai-api-key>`, `<supabase-secret-key>`, …) |
| L4 | Credentialed URIs | port of `credentialedURIPattern`, extended with a password capture group. | **surgical**: password sub-span only → `<password>`; scheme/user/host/path survive (`postgres://user:<password>@host/db`) |
| L5 | Connection strings | JDBC / DB URL / keyword DSN / semicolon rules with their placeholder-gated `hasSecret` checks and trailing-punctuation trim. | **surgical**: each password-assignment *value* span (unquoted interior, per their `unquoteRange`) → `<password>`; host/user/db keys survive |
| L7 | Credential key/value | `credentialValuePattern` + JSON `credentialContext` (host+user objects promote bare `password` keys) + DB-shaped key regex, all matching non-placeholder values regardless of entropy. | value span (already surgical in the reference) → `<password>` |

(L6 custom rules and L8 PII deliberately absent in v1 — §11.)

### 4.2 Skip rules (structure survives, content is scanned)

Adopted verbatim: keys ending `signature` (Claude thinking signatures),
keys ending `id`/`ids` (covers `uuid`, `parentUuid`, `sessionId`,
`call_id`, `requestId`), path fields (`filepath`, `file_path`, `cwd`,
`root`, `directory`, `dir`, `path`), objects with `type:image*/base64`.
JSON **keys are never rewritten** (a secret used as a key is a residual,
§11; `patch_apply_end` path-keys therefore stay parallel to the pathmap
story). Mallard additions, to be confirmed and frozen by the corpus scan
(§12, types-and-counts only, never content):

- Codex `encrypted_content` (reasoning payloads — high-entropy base64
  that must survive for resume, the Codex analog of thinking
  signatures).
- The Codex wire head's `__mallard_path_map__` object (skip the marker
  object wholesale; it is mallard's own, and its field name must never
  be perturbed before `Decode` validates and strips it).
- Any further high-entropy structural field the scan surfaces —
  **FROZEN by the 2026-08-21 corpus scan** (8.2 GB Codex + 311 MB
  Claude, types-and-counts only) as a value-shape guard rather than more
  field names: (a) **data-URI values skip every layer** (Codex spells
  inline images as `image_url`/`images` data-URI fields — 41.4 M of the
  corpus's 44.3 M entropy spans — the value-level analog of the
  reference's image-object skip); (b) **bare URL and bare absolute-path
  values skip the entropy layer only** (~2.1 M signed-URL/filename
  spans), with every typed layer still active, so vendor keys, provider
  prefixes, and `user:password@` credentials inside links still redact.
  The guard is a pure function of the string and feeds the ruleset
  fingerprint.

### 4.3 Typed deterministic placeholders (D1, user-decided) and fixpoint

Replacement is a **typed placeholder from a frozen vocabulary**, spelled
`<label>` — the documentation convention (`postgres://user:<password>@…`)
rather than a destruction marker. Design rules:

- **Label shape (frozen)**: interior matches `^[a-z][a-z_-]*$`, total
  length ≥ 5 — *exactly* the reference's `isBracketedPlaceholder`
  shape. Digit-free is load-bearing (their rule rejects digits so
  `<hunter2>` still redacts); every label in the vocabulary is
  machine-checked against the shape at build time.
- **Vocabulary (~20 canonical labels, many-to-one from rules)**:
  `<secret>`, `<password>`, `<api-key>`, `<private-key>`, `<jwt>`,
  `<aws-access-key>`, `<aws-secret-key>`, `<github-token>`,
  `<gitlab-token>`, `<slack-token>`, `<stripe-key>`, `<google-api-key>`,
  `<npm-token>`, `<pypi-token>`, `<sendgrid-key>`, `<twilio-key>`,
  `<anthropic-api-key>`, `<openai-api-key>`, `<supabase-secret-key>`,
  `<supabase-access-token>`, `<replicate-token>`, `<huggingface-token>`.
  The rule→label map and label spellings are part of the ruleset and
  its fingerprint (§3.3): renaming a label is a ruleset bump.
- **Determinism**: the label is a pure function of the winning rule.
  Merge tie-break is fully specified: sort regions by (start asc, end
  desc, priority asc, label lexical asc) where priority is P0 = typed
  rule labels (L2/L3), P1 = `<password>` context layers (L4/L5/L7),
  P2 = `<secret>` (L1); overlapping regions merge into the
  first-sorted region's label (larger-region-first preserved, so a PEM
  block containing high-entropy runs stays one `<private-key>`).
- **Surgical sub-spans** for the password-context layers (table §4.1):
  structure survives, only the credential value is replaced. Hosts,
  usernames, database names, query keys remain readable — deliberate
  (residual noted in §11). Whole-match replacement stays for typed key
  material (L2/L3) and entropy findings (L1), where there is no inner
  structure worth keeping.
- **Why not per-secret distinctness** (`<password_a>`/`<password_b>`,
  hash tags): cross-record numbering would make record `n`'s bytes
  depend on records `1..n-1`, destroying per-record purity — the exact
  property the accepted wire pin and append fast-forward depend on
  (§3.3). Hash-tagged placeholders are an offline-guessing oracle for
  low-entropy secrets and break the digit-free allowlist shape. Two
  same-type secrets in one thread therefore collapse to one label;
  cross-*type* distinctness — the readable part — is preserved.
- **Fixpoint, four mechanisms**, each pinned by tests (§12):
  1. Value-gated layers (L5/L7): `<label>` is bracketed-placeholder
     shaped → `isPlaceholderSecretValue` → never re-flagged.
  2. URI/conn-string patterns: the reference's character classes
     exclude `<` and `>` in password/URI positions — after
     `user:<password>@host` the L4 pattern structurally cannot match.
     **These exclusions are load-bearing and must be preserved in the
     Rust port.**
  3. Entropy: labels are digit-free lowercase words with entropy far
     below 4.5, and `<`/`>` break the L1 candidate charset — a label
     can never seed or extend a candidate (strictly better than
     `REDACTED`, whose charset-internal spelling could glue onto
     neighbors).
  4. Vendor/prefix rules: no rule matches lowercase bracketed words; a
     vocabulary inertness test asserts every label against every layer
     anyway.
- Prior-art placeholder allowlist ported in full
  (`placeholderSecretValues`, `${VAR}`, mask runs) so pre-existing
  documentation-grade fake credentials in transcripts stay readable.

### 4.4 Application modes (one detector, two appliers)

- **Codex — value-tree mode.** The standalone builder already parses
  and re-serializes every record with canonical key order; `redact`
  walks the `serde_json::Value` after `pathenc`, rewriting string leaves
  under the skip rules, before serialization. No raw-text splicing
  needed; determinism is BTreeMap-ordered.
- **Claude — raw-line mode.** Claude wire bytes remain "verbatim except
  redactions": parse each line, collect replacements, apply them to the
  raw line by JSON-encoded-string replacement with keyed replacements
  restricted to value position (port of `applyJSONReplacements` /
  `replaceKeyedJSONValue`; serde_json does not HTML-escape, matching
  their `SetEscapeHTML(false)`, so `<label>` survives encoding
  verbatim). Untouched lines are byte-identical to local — the wire
  fold then differs from the local fold only where redactions actually
  occurred, and the provider's own reader ingests the materialized file
  unchanged.
- Non-JSON lines (should not occur in either provider's transcripts, but
  sidecars/state may): raw `String` pass. Whole-file single-JSON-value
  artifacts (some sidecar/state shapes): field-aware whole-document
  walk, per the entireio `IsLineDelimited` routing — with their warning
  honored: never split a single-JSON-value document and redact
  fragments.

## 5. Gates

Extending the pathmap gate structure; every gate is typed and
fail-closed, and **no gate error ever echoes matched text** — payloads
carry record index, JSON pointer of the leaf, and rule id only.

Push build (per record, in the wire builders):

1. **Span gate** (per stage, keeping each simple): the existing
   pathmap structural diff (`encode_record_gated`) runs unchanged on
   `pathenc`; a second structural diff then proves `redact` changed
   nothing but string leaves, each leaf differing only by
   placeholder-for-span substitution where every substituted token is a
   member of the frozen label vocabulary, and changed no key, no
   non-string value, no record count, no skip-listed field. (The
   vocabulary membership check turns an encoder bug that mints a
   malformed or dynamic label into a visible build failure.)
2. **Wire self-check** (extended `verify_wire_record`):
   `pathenc(pathdec(w)) == w` **and** `redact(w) == w` for Codex;
   `redact(w) == w` for Claude. The second conjunct is the leak
   certificate: the uploaded object provably contains nothing the
   ruleset can detect.
3. **Publish leak gate** (choke point): conversation publish APIs accept
   a `RedactedWireBytes` newtype (the `RedactedBytes` idea from
   entireio, enforced at compile time), producible only by the wire
   builders and the sidecar/state/`display_title` capture scrubbers.
   The no-cursor publish branch (`commands.rs:9328`'s verbatim
   fallback) is thereby unreachable-by-construction for unredacted
   bytes — a future provider without a wire builder cannot push
   conversations at all.

Pull staging (before any materialization plan is built):

4. **Fingerprint check**: descriptor `wire_redaction.fingerprint` ==
   binary fingerprint, else `redaction_ruleset_mismatch`.
5. **Wire self-check again** over the downloaded bytes (same as gate 2;
   for Codex it rides the existing staged refold + `verify_wire_record`
   pass at `commands.rs:11498`; for Claude it is a new scan in staging —
   Claude gains its first pull-side content gate). A cloud object that
   fails is `wire_secret_detected`: tampered, produced by a foreign
   ruleset, or evidence of an encoder bug — never materialized.

Not gated, by design: local bytes are never scanned outside push builds
(no watcher/refresh cost, no local scrubbing except via materialization).

## 6. Flows

- **Push (Codex)** — `commands.rs:9280`: builder mode `Encode` gains the
  redact stage; artifact bytes, `artifact_sha256`, `portable_tail`
  (wire), `lineage_digest`/`semantic_digest` (wire) all now cover
  redacted token-form bytes. Checkpoint pinning unchanged in shape:
  `accepted` = wire, `accepted_local_tail` = seeded local fold.
- **Push (Claude)** — `commands.rs:9290`: seed local fold from original
  bytes (review check, `accepted_local_tail`) → `redact` transcript →
  seed **wire** fold from redacted bytes → publish redacted bytes +
  wire tail + `wire_redaction` stamp. Relabel construction deleted.
- **Pull (Codex)** — `commands.rs:11498`: fingerprint gate → staged wire
  self-check → `Decode` (pathdec; redaction untouched) → copy-on-write
  materialization as today. Local pins recomputed from decoded local
  bytes, as today.
- **Pull (Claude)**: fingerprint gate → wire self-check → materialize
  redacted bytes to the bucket path in place; apply re-derives the
  observed tail from those exact bytes, so on the destination the local
  fold equals the wire fold relabeled — the two columns carry one state
  again *on that machine*, which the relation already models.
- **Refresh / watcher / cursor**: zero change. The origin machine's
  local file keeps its secrets; `observed_tail` keeps riding; `local ==
  accepted_local` still reads Sync without ever touching wire bytes.
- **A→B→A** (the §1 story, end to end): A pushes turn1 containing
  `sk-ant-…` → cloud holds `<anthropic-api-key>`, `accepted` folds
  redacted bytes, A stays Sync. B pulls → materializes placeholders,
  B's two columns agree. B adds turn2 (its own secret) and pushes →
  wire prefix for records `1..n` re-encodes byte-identically (purity +
  per-record independence), fast-forward passes. A pulls →
  `CloudOutstanding` → Codex: fresh rollout with placeholders
  everywhere, predecessor file untracked on disk; Claude: in-place
  scrub. No flow ever compares across forms; no false divergence exists
  in the design.

## 7. Redacted artifact scope (D2)

Everything content-bearing that leaves the machine, uniformly:

| Artifact | Applier | Notes |
|---|---|---|
| Codex rollout records | value-tree | inside the standalone builder |
| Claude transcript lines | raw-line | wire fold over redacted bytes |
| Claude `todos` sidecar | JSONL/single-value routing | captured at push |
| Claude `file-history` sidecar | routing, else raw `Bytes` | snapshots of edited files — the single most secret-dense artifact we ship (`.env` edits live here); leaving it verbatim guts the feature |
| Provider-state memory files | routing | user/model-authored notes |
| `display_title` | raw `String` | the bounded first-user-message fallback (`ConversationThread.display_title`) — "here's my key sk-…" as an opener must not leak via metadata. Applied where the descriptor is built (`commands.rs:8789`). |

`SessionPresentation` is structural (timestamps, cwd, branch, sha,
metrics) — nothing to scrub. Non-conversation resources are out of scope
only because homes-only sync ships no others; the rule of thumb is
stated in §5 gate 3: bytes reach publish only through the newtype.

## 8. Failure behavior (typed first, UI second)

Per the standing rule, new states are classified before any UI work:

| Typed error | Raised at | Meaning / operator action |
|---|---|---|
| `redaction_ruleset_mismatch` | push stamp check, pull gate 4 | binary and cloud disagree on ruleset (patterns or labels); blocker row, banner zone; resolution = align binaries (or the standing wipe) |
| `wire_secret_detected` | push gate 2, pull gate 5 | fail-closed leak/tamper certificate; blocker; payload = record index + pointer + rule id, never content |
| `redaction_gate_failed` | push gate 1 | encoder bug surfaced as a visible local build error (malformed label, non-leaf change, skip-list violation) — the design's stated degradation direction: encoder bugs become build failures, never false divergence |

All three surface through the existing review-row blocker plumbing
(banner zone / gate-exit rules). No new UI concepts; a deferred
enhancement (not v1) may show "N spans redacted" on push rows.

## 9. Performance

- Cost rides exclusively on push build and pull staging, which already
  parse and fold every record. Added work per string leaf: one
  `RegexSet` prefilter over ~100 vendor rules + one linear entropy scan
  + the small always-on regex layers. Entireio's Go numbers (20 MB
  transcript: 43 s naive, 2.1 s sharded) are the scale ceiling for a
  slower stack doing whole-file passes per Stop hook; mallard redacts
  per user-initiated push on an already-walking pipeline in Rust —
  target well under a second for a 20 MB rollout, measured in §12's
  bench before merge.
- No incremental prefix cache in v1 (their cache exists because they
  redact on every checkpoint; we do not). If push latency on the 70 MB
  tail of the corpus ever matters, the entireio prefix-cache design
  (fingerprint + prefix hash + newline boundary) is the known follow-up
  and composes with per-record independence trivially.
- Steady state (refresh, watcher, classification): zero added cost, by
  construction.

## 10. Cutover

Clean, single change, wipe-based:

1. Land detector + label vocabulary + appliers + gates + builder/capture
   integration + descriptor stamp + typed errors + tests.
2. Delete in the same change: the Claude wire-tail relabel
   (`commands.rs:9337` block), the verbatim `published_bytes` fallback
   for cursor providers, and any test asserting wire==local bytes for
   Claude.
3. Always-on (D4): no toggle, no per-project opt-out in v1 — a toggle
   creates mixed-form clouds and re-introduces the false-divergence
   surface this design exists to avoid.
4. User wipes `~/.mallard` + cloud; first pushes stamp
   `wire_redaction` everywhere.

## 11. Non-goals and residuals

- **No un-redaction, ever** (no vault, no escrow, no reversible
  encoding). Origin machines lose tracked-copy secrets on cloud-ahead
  materialization; the predecessor-file escape hatch exists for Codex
  only. Accepted.
- **Same-type collapse**: two distinct passwords in one thread both read
  `<password>`; per-secret distinctness is unimplementable without
  breaking per-record purity or minting an oracle (§4.3). Cross-type
  distinctness is preserved — that is what D1 buys.
- **Surgical layers reveal context**: hosts, usernames, and database
  names in URIs/DSNs survive redaction by design (docs convention).
  Anyone treating a hostname as a secret is out of scope for v1.
- **Secrets as JSON keys** are not scanned (values only). Residual.
- **Cross-record composite rules** (betterleaks' RequiredRules, e.g.
  Supabase key + nearby project URL) run within one record only;
  per-record purity is worth more than the marginal recall. The L3
  prefix layer exists precisely to cover the known low-entropy cases.
- **Over-redaction** of high-entropy non-secrets (base64 blobs in tool
  output, long random identifiers not ending in `id`) is the accepted
  direction — such spans read `<secret>`; the corpus scan quantified
  the rate before freeze (D6) and drove the §4.2 value-shape guard.
- **Opaque bearer tokens inside bare URLs** (S3 presigned signatures,
  signed CDN links) survive redaction under the value-shape guard's
  entropy exemption — the price of not destroying every link a web
  search or upload ever produced. Typed layers still cover the known
  credential shapes inside URLs. Residual, accepted.
- **PII, OPF, custom user rules, per-user allowlists**: out of scope v1.
  L6's slot in the layer numbering is reserved.
- **Ruleset evolution without wipe** (pinned multi-ruleset, per-thread
  re-baseline ops): designed around deliberately; revisit only when
  there are customers.
- **Windows**: already gated out at the binding level by the pathmap
  plan; nothing new.

## 12. Tests

Unit (detector):
- per-layer positive/negative tables — vendor keys, prefixes, URIs,
  DSNs, key/value, credentialContext promotion (`hunter2` →
  `<password>`), placeholder allowlist negatives, UUID/hex/SHA/digest
  safety, thinking-signature and `encrypted_content` skips,
  image-object skip, id-suffix skip, JSON-escape guard, keyed-value
  position restriction.
- **label vocabulary**: every label matches the frozen shape; every
  label is inert under every layer (the four fixpoint mechanisms of
  §4.3 pinned one by one, including the `<>`-exclusion in ported URI /
  conn-string character classes).
- **surgical structure preservation**: `postgres://user:hunter2@host/db`
  → `postgres://user:<password>@host/db`; JDBC and keyword DSNs keep
  host/user; quoted values keep their quotes.
- **merge priority determinism**: an AWS key inside a conn string, a
  high-entropy password, a PEM block containing entropy runs — each
  resolves to one deterministic label per the (start, end, priority,
  label) order.
- fixpoint: `redact(redact(x)) == redact(x)` over fixtures + proptest.
- determinism: repeated runs byte-equal; walk-order independence.
- per-record composition: record-wise == whole-file for raw-line mode.
- purity of repair: planted local secret survives
  `pathdec_new(pathenc_old(x))`.

Builder / flow (extending the existing suites):
- Codex: encode composes pathenc→redact; span gates; wire self-check
  catches a hand-planted post-encode secret; `Decode` never un-redacts;
  `__mallard_path_map__` marker inert under redact.
- Claude: untouched lines byte-identical local↔wire; wire fold is a
  genuine fold over redacted bytes (relabel gone); review check still
  local-form; pull writes redacted bytes and reseeds cleanly.
- `a_to_b_to_a…` sibling test with planted secrets per §6's narrative,
  asserting: cloud bytes never contain the secret (scan), typed labels
  in materialized output, no false divergence at any hop, Codex
  predecessor file retains the secret, Claude in-place scrub, restored
  prefix == scrub(canonicalize(x)).
- `display_title` and each sidecar/state artifact scrubbed end-to-end.
- Gates: fingerprint mismatch and tampered cloud object block with the
  right typed errors; a wire object hand-edited to carry an
  out-of-vocabulary placeholder fails the span gate on re-push; error
  payloads content-free.

Corpus + bench (pre-freeze inputs, not CI):
- **types-and-counts-only scan** of `~/.codex/sessions` and
  `~/.claude/projects` (house rule: never print content): per-layer
  match counts by field name, per-label counts, skip-list hit counts,
  candidate structural fields with entropy > 4.5 — freezes §4.2's
  mallard additions and validates D6's threshold. Script lands in
  `scripts/` beside the existing scan scripts.
- micro-bench: MB/s through the detector; assert the §9 target on a
  synthetic 20 MB rollout.

Live: extend the aimock live integration tests with a planted dummy
secret (`sk-ant-` shaped, fake) proving cloud-object cleanliness, the
`<anthropic-api-key>` label in the cloud object, and round-trip
classification across both providers.

## 13. Touch list

- `crates/mallard-infrastructure/src/credential_redaction.rs` — NEW:
  ruleset table + rule→label map + label vocabulary + fingerprint,
  detector layers, labeled-region merge, value-tree walk, raw-line
  applier, routing, gates, `RedactedWireBytes`.
- `crates/mallard-infrastructure/src/codex_standalone_builder.rs` —
  Encode composes redact; both modes run the extended wire self-check.
- `crates/mallard-infrastructure/src/codex_path_map.rs` — untouched
  (`verify_wire_record` reused; extension lives beside it).
- `crates/mallard-infrastructure/src/claude_history_tail.rs` — reuse
  `seed_claude_root` over redacted bytes (no structural change
  expected).
- `src-tauri/src/project_sync/commands.rs` — Claude push branch
  (redact + second seed + stamp; delete relabel), Codex push stamp,
  pull gates 4–5 both providers, sidecar/state/`display_title` capture
  scrubbing, repair exclusion comment + test hook.
- `crates/mallard-domain/…` — `wire_redaction` on `ConversationThread`
  (+ checkpoint copy if acceptance needs it), typed error variants,
  descriptor validation (stamp required whenever a wire portable_tail
  is present).
- `src/types.ts` + review-row error classification strings — mirror the
  three typed errors into the existing blocker taxonomy.
- `scripts/` — corpus scan script (types+counts only).
- Tests per §12 across the existing suite files.

## 14. Decisions and verification items

- **D1 Placeholder — RESOLVED (user, 2026-08-20)**: typed deterministic
  placeholders, `<label>` from the frozen vocabulary (§4.3), surgical
  sub-spans in password-context layers. Considered and rejected: bare
  `REDACTED` (unreadable, loses type information), per-secret indexed
  labels (breaks per-record purity → breaks the wire pin), hash-tagged
  labels (oracle + digit-bearing shape breaks the allowlist rule).
- **D2 Scope**: all rows of §7, including `file-history` and
  `display_title`. (Alternative: transcripts only — leaves the densest
  secret store unredacted.)
- **D3 Secrets inside out-of-root path text**: scrub. (Alternative:
  never touch path-shaped spans — preserves the letter of the path
  invariant but ships `~/.aws` keys.)
- **D4 Always-on**, no toggle.
- **D5 Ruleset bump = breaking wipe** for v1 (patterns *and* labels);
  fingerprint stamped so the failure is typed, not silent.
- **D6 Entropy threshold 4.5** + over-redaction accepted; corpus scan
  quantifies before freeze.

Verification items (cheap audits during implementation, not design
forks):
- **V1**: confirm pull-apply `TargetExpectation`s for
  transcript/sidecar/state targets are built from review-time local
  reads (local-form) — i.e. no manifest(wire)-digest-vs-local-file
  comparison survives anywhere in apply/refresh for file artifacts.
- **V2**: confirm sidecar/state change detection never compares a raw
  local digest against a manifest digest outside push capture (if one
  exists, that site gets the capture-time scrub or a digest twin —
  §3.2's file ledger).
- **V3**: corpus scan confirms the exact Codex/Claude structural field
  names for §4.2's additions (`encrypted_content` spelling et al.) and
  that no descriptor field beyond `display_title` carries
  content-derived text.

## 15. Implementation notes (deltas from the plan as drafted)

Landed exactly as designed except where noted. Touch list as realized:
`credential_redaction.rs` (NEW, detector + appliers + gates + fingerprint +
`RedactedWireBytes`), `codex_standalone_builder.rs`, `provider_capture.rs`,
`claude_history_tail.rs` (untouched — reuse only), `commands.rs`,
`mallard-domain` (`ConversationWireRedaction`, thread field + validation,
`ConversationBlocker::RedactionRulesetMismatch`,
`blocked_conversation_decision` export), `mod.rs` re-export, `types.ts`,
the one-time credential corpus scan (removed after the rules were frozen),
tests across
`credential_redaction.rs` (31 unit), `codex_history_graph.rs` (A→B→A
secrets sibling + tamper refusal), `commands.rs` (Claude A→B→A end-to-end,
ruleset-mismatch review blocker, repair secret survival, first-contact
adoption), `sync_tests/thread_variants/harness.rs` (real stamp on fixture
rows).

1. **Claude adoption probe** (`adopt_claude_acceptance`) was a fifth
   comparison site the plan's V-items missed: it compared the cloud wire
   pin against the **local** chain fold, legal only in the relabel era. It
   now takes a second `wire_chain` argument — the caller redacts the
   analyzed local slice and re-analyzes it — so the probe compares wire
   against wire and the re-established local pin folds the local chain.
   Codex adoption needed nothing: it already re-encodes through the
   builder, which now composes redact. Pinned by
   `claude_first_contact_adoption_survives_redaction` (remove + re-register
   with a planted secret; Synced, never keep-both).
2. **Sidecar/state scrub happens at capture** (`capture_resource` in
   `provider_capture.rs`), not at the publish site: `ProviderState` files
   and conversation non-transcript files are redacted the moment bytes are
   read, so review diffing, retention dedup, resource digests, and publish
   all see one wire-form stream (this is what discharges V2 — every
   change-detection comparison is wire-vs-wire by construction).
   Transcripts stay local-form at capture (review seeding + pull-time local
   expectations read them) and are redacted by their wire builders.
   `detect_secret_material` stays as an advisory warning layer and now runs
   over the scrubbed bytes (provider files are `opaque_content`, so it
   never blocked provider captures).
3. **Publish choke as certification**: `RedactedWireBytes` is produced by
   `certify_wire_document` (`redact(w) == w` verified, error payloads carry
   line/pointer/rule id only) rather than by constructor privilege alone.
   The Codex artifact is re-certified whole at the publish site (cheap,
   push-only); Claude bytes carry the certificate from the redaction step;
   sidecars and every generic provider-scope file are certified at their
   insert sites. The non-cursor verbatim publish fallback is deleted — a
   provider without a wire builder now errors at push.
4. **Claude cloud-row facts are wire-form**: `lineage_digest`,
   `semantic_digest`, `head_message_id`, and `message_count` on the thread
   row now come from the redacted re-analysis (`ClaudeWirePublication`),
   matching Codex's artifact-derived facts, so a puller's local capture of
   the materialized (redacted) bytes reproduces the row's digests.
5. **Merge nuance (reference parity)**: `=` sits inside the entropy
   candidate charset, so a high-entropy `password=A7fK…` value forms one
   candidate spanning the key text; the earlier-starting entropy region
   wins the merge and the whole assignment reads `<secret>`. Surgical
   `<password>` shapes appear exactly where the reference would have kept
   them: values the entropy layer does not flag on its own. Pinned in the
   unit tables.
6. **Sidecars do not materialize on pull** (pre-existing behavior: the
   apply writes transcript + index fragment only; sidecar targets are
   planned for collision checks but no payloads are written). The
   round-trip test therefore asserts sidecars *ship* scrubbed (store scan)
   and drops the materialization assertion. Not a redaction concern; noted
   for whoever builds sidecar pull.
7. **Bench**: 21 MB synthetic rollout through the raw-line applier in
   0.27 s release-mode (≈79 MB/s) — the §9 target with a wide margin
   (`bench_20mb_synthetic_rollout`, `#[ignore]`, run with `--release`).
8. **Vocabulary**: exactly the 22 labels of §4.3; `gsk_`/`tvly-`/`pplx-`/
   `xai-` map to the generic `<api-key>` per the many-to-one rule. The PEM
   rule uses an unbounded lazy body (`.*?`) — a counted repetition
   explodes the compiled `RegexSet`.
9. **V1 verified**: pull-apply `ProviderStateExpectation`s are
   `bounded_file_expectation` disk reads everywhere (local-form);
   `local_target_expectation_for_action` digests captured **transcript**
   bytes only, which remain local-form. **V2 verified**: see note 2.
   **V3**: the removed types-and-counts corpus scan was the pre-freeze input;
   `encrypted_content` and
   `__mallard_path_map__` are already in the skip-list.
10. **Corpus scan results (V3/D6, 2026-08-21)** — Codex: 1477 files,
    1.33 M records, 11.9 M string leaves, 8.24 GB scanned; Claude: 445
    files, 189 K records, 311 MB. Pre-guard the entropy layer would have
    emitted 44.3 M `<secret>` spans, 41.4 M of them `image_url` data-URI
    payloads and ~2.1 M bare signed-URL values — hence the §4.2
    value-shape guard (data URIs skip all layers; bare URL/path values
    are entropy-exempt with typed layers active). Real typed findings at
    stake in the corpus: 1 717 `<openai-api-key>`, 315 + 172
    `<password>`, 56 `<anthropic-api-key>`, plus JWT/private-key/Stripe/
    AWS/Supabase/HF/Replicate singles — the leak surface the feature
    exists for. Skip shields worked as designed (1.9 M id-suffix,
    185 K `encrypted_content`, 55 K path fields, 1.6 K image objects).
    The post-guard re-run confirms the freeze: Codex `<secret>` spans
    drop 44.3 M → 682 K (98.5 % of the flood eliminated; the remainder
    sits in prose/tool-output fields — `text` 612 K, `result` 50 K —
    exactly D6's accepted over-redaction class) while **every typed
    finding is preserved bit-for-bit** (1 717 openai, 315 password, 74
    replicate, 46 jwt, 42 private-key, …), including typed hits inside
    guarded values (62 `r8_` tokens in bare `thumbnail_url` URLs — the
    entropy-exemption-only design working as intended). Guard shields:
    240 K bare-URL/path values + 10.3 K data-URI leaves on Codex; the
    Claude corpus was never image-flooded (4 K `<secret>`, 25.5 K
    signature shields doing their job). Reports (types and counts only)
    in the session scratchpad.
11. **Live gate (2026-08-21)** —
    `live_claude_aimock_setup_push_pull_divergence` passes with fake
    credentials planted in the Read-tool fixture: every store object scans
    clean of the planted pair while carrying `<anthropic-api-key>` /
    `DB_PASSWORD=<password>` (positive controls), B's materialized
    transcript equals the wire lineage of A's local bytes and holds
    placeholders, and the real Claude CLI resumes redacted transcripts
    through the full divergence/keep-both cycle. The old
    `lineage_of(destination) == lineage_of(source)` assertion was the last
    relabel-era wire==local equality; it now compares against
    `wire_lineage_of(source)`.

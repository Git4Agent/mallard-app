# Plan: Claude Code integration testing through Tuzi

Status: research and one-turn protocol POC complete; the separately gated
three-turn live Mallard integration target is implemented and verified. No
production behavior, schema, DTO, manifest, compatibility path, or lockfile
change is included.

Date: 2026-08-19 (America/Los_Angeles)

## 1. Decision

The smallest useful live POC is feasible with the repository-root
`.env.local` connection, the installed Claude Code CLI, Tuzi's native
Anthropic Messages route, exact model `claude-sonnet-5`, and Claude's lowest
effort setting.

The POC is intentionally a one-model-turn provider compatibility probe. It
does not duplicate the exhaustive Mallard Push/Pull/divergence coverage in the
credentialless Claude AIMock target. It answers these narrower questions:

1. Does the current Tuzi key expose exact model ID `claude-sonnet-5`?
2. Does Claude Code authenticate to Tuzi using its native `X-Api-Key` path?
3. Does Tuzi accept Claude Code's streaming Anthropic Messages request for
   Sonnet 5 at `low` effort?
4. Does the real CLI return a successful bounded JSON result and persist a
   native isolated Claude transcript?

The answer to all four questions was **yes** in the successful POC run.

## 2. Research conclusions

### 2.1 Tuzi exposes the required protocol

Tuzi's official [Anthropic Messages API guide](https://api.tu-zi.com/docs/api/anthropic)
documents:

- `POST $BASE_URL/v1/messages`;
- preferred `Authorization: Bearer <Tuzi key>` authentication;
- compatible native `x-api-key: <Tuzi key>` authentication on Messages;
- an explicit `anthropic-version: 2023-06-01` recommendation;
- required `max_tokens`; and
- the live marketplace as the authority for model IDs and prices.

Claude Code's official [LLM gateway guide](https://code.claude.com/docs/en/llm-gateway-connect)
uses the gateway site root as `ANTHROPIC_BASE_URL`; Claude appends the Messages
path. The POC therefore converts an accepted `.env.local` URL ending in `/` or
`/v1` to its origin. Passing `/v1` through unchanged could produce the wrong
`/v1/v1/messages` path.

### 2.2 Exact Sonnet 5 is currently present

Anthropic documents the exact upstream model ID as `claude-sonnet-5` in
[What's new in Claude Sonnet 5](https://platform.claude.com/docs/en/about-claude/models/whats-new-sonnet-5).

At research time, Tuzi's public [`/api/pricing`](https://api.tu-zi.com/api/pricing)
record for `claude-sonnet-5` reported:

- native Anthropic endpoint `POST /v1/messages`;
- supported endpoint types `anthropic` and `openai`; and
- enabled groups including `default` and `Claude-Code`.

Public marketplace presence is not sufficient by itself: routing can still
depend on the current key and group. The POC consequently performs a bounded,
authenticated `GET /v1/models` and fails closed unless the response contains
the exact ID. The current `.env.local` key passed that check. No alias and no
fallback model is accepted.

### 2.3 "Lowest tier" is not one setting

Three concepts must remain separate:

| Concept | POC choice | Meaning |
| --- | --- | --- |
| Claude effort | `--effort low` | Lowest supported Sonnet 5 reasoning/token effort. It can reduce latency and tokens, but is not a price or priority tier. |
| Anthropic service tier | No override | Standard API capacity is the applicable path. Claude Code has no documented general flag for injecting `service_tier: "standard_only"`, so the POC does not invent a header or body override. |
| Tuzi billing group | Key-controlled | The connection document contains no group. The POC cannot select or prove the account's settlement group from a Claude CLI flag. |

Anthropic documents the effort control in [Effort](https://platform.claude.com/docs/en/build-with-claude/effort)
and API service tiers separately in [Service tiers](https://platform.claude.com/docs/en/api/service-tiers).

Tuzi describes `default` as its lowest-cost group, but also says it does not
support tools. For that reason the cost-minimal POC exposes zero tools. This
tests Claude Code, streaming Messages, Sonnet 5, low effort, authentication,
and native transcript persistence, but not a `tool_use`/`tool_result` round.
The existing local AIMock target remains the deterministic tool-loop oracle.

### 2.4 Claude Code version and request shape

The installed CLI is exactly `2.1.235 (Claude Code)`. Anthropic's
[Claude Code changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)
introduced Sonnet 5 support in 2.1.197; the POC pins 2.1.235 so its evidence is
not silently generalized to a different wire implementation.

A credentialless loopback capture performed before the paid run established
the exact relevant client behavior for this version:

```text
HEAD /api/hello
  no X-Api-Key
  no Authorization

POST /v1/messages?beta=true
  X-Api-Key present
  no Authorization
  model = claude-sonnet-5
  output_config.effort = low
  thinking.type = adaptive
  tools = []
  max_tokens = 1024
  stream = true
```

The POC uses the single-argument spelling `--tools=`. The two-argument form
`--tools ""` is unsafe for this CLI because the variadic option can consume
the following prompt.

## 3. Current command and implementation boundary

The one-turn POC was removed after the package runner replaced it. Invoke the
current paid test with an explicit paid-use acknowledgement:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 npm run test:live-claude-tuzi-e2e
```

The runner:

1. checks the paid-use gate;
2. verifies exact Claude Code version 2.1.235 from a disposable home **before**
   opening `.env.local`;
3. reads `.env.local` as bounded JSON, never as shell input;
4. requires a no-follow, metadata-stable, mode-`0600` regular file with exactly
   `_type`, `key`, and `url` and `_type = newapi_channel_conn`;
5. requires the exact public Tuzi HTTPS origin, with no user info, port, query,
   fragment, or arbitrary path;
6. checks exact model visibility through one bounded catalog request;
7. creates private disposable `HOME`, `USERPROFILE`, `CLAUDE_CONFIG_DIR`,
   working, and process-temporary directories;
8. starts one direct `claude` process with a closed environment and no shell;
9. permits one model turn, zero retries, zero tools, no fallback model, a
   1024-output-token limit, a 120-second timeout, and a USD 0.05 client cap;
10. replaces the large default system prompt with one fixed minimal system
    instruction and sends no Mallard source, conversation, or personal data;
11. requires the exact marker response, one turn, the generated session ID,
    exact Sonnet 5 evidence, and a persisted mode-`0600` native JSONL;
12. scans bounded stdout, stderr, and the complete disposable tree for the
    exact key bytes; and
13. deletes the disposable tree and prints only a redacted fixed-shape result.

The credential enters only:

- the catalog request's recommended Bearer header; and
- the direct Claude child environment as `ANTHROPIC_API_KEY`, which Claude Code
  converts to the Tuzi-supported `X-Api-Key` Messages header.

It never enters argv, logs, the prompt, the transcript, the summary, or a
Mallard artifact. The child receives no inherited proxy, OAuth, Anthropic,
Bedrock, Vertex, Foundry, AWS, Azure, Google, OpenAI, or real Claude-home state
because the environment is constructed from a closed allowlist.

## 4. Observed POC result

The successful run completed in approximately 5.45 seconds and returned:

```json
{
  "result": "passed",
  "claude_code_version": "2.1.235",
  "requested_model": "claude-sonnet-5",
  "observed_models": [
    "claude-sonnet-5"
  ],
  "effort": "low",
  "tools_exposed": 0,
  "model_catalog_exact_match": true,
  "paid_model_turns": 1,
  "retries": 0,
  "session_persisted": true,
  "maximum_budget_usd": 0.05,
  "claude_reported_cost_usd": 0.0235275
}
```

There were two explicit operator executions during development, not an
automatic retry inside one execution:

1. the first ended after approximately 8.24 seconds with a redacted generic
   Claude process failure; its raw provider body was intentionally neither
   logged nor retained; and
2. after improving only fixed-category error classification, the second
   execution passed as shown above.

The first attempt may or may not have reached an upstream model. The
successful run's `total_cost_usd` is Claude Code's estimate, not proof of
Tuzi's settled price, group, refund, or aggregate cost across both operator
executions. Tuzi's [billing and usage guide](https://api.tu-zi.com/en/docs/use-cases/billing-usage)
states that the marketplace, current group, final consumption log, and
request ID are authoritative, and that replay after a lost response can be a
second charge. The Tuzi consumption log should therefore be checked before
any further paid experiment.

## 5. What the POC proves

For Claude Code 2.1.235 and the current `.env.local` connection, the POC proves:

- the key-scoped model catalog exposes exact `claude-sonnet-5`;
- the custom base URL must be the Tuzi origin rather than an origin plus
  `/v1`;
- Tuzi accepts the `X-Api-Key` authentication emitted from
  `ANTHROPIC_API_KEY`;
- Tuzi accepts Claude Code's streamed native Messages request with Sonnet 5,
  adaptive thinking, and `low` effort;
- the model can return a bounded deterministic marker with no tools exposed;
- Claude's result and native transcript both identify Sonnet 5;
- Claude persists the isolated native session; and
- the credential did not appear in captured output or disposable files.

## 6. What the POC does not prove

The result must not be promoted into broader claims. It does **not** prove:

- the complete Mallard setup, Push, Pull, divergence, Keep Both, or fixed-point
  state machine through a live provider;
- cross-machine transcript reidentification and resume through Tuzi;
- a Claude `Read` or other tool loop through Tuzi;
- that the account used Tuzi's cheapest `default` group;
- that Claude Code forced Anthropic `service_tier: "standard_only"`;
- Tuzi's final settled charge or whether the first failed execution was
  billed;
- provider availability beyond the one successful probe; or
- compatibility with another Claude Code, model, endpoint, or Tuzi routing
  configuration.

## 7. Implemented live integration target

The implemented paid gate is:

```sh
MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1 \
  npm run test:live-claude-tuzi-e2e
```

The final gate passed on August 19, 2026 with `1 passed, 0 failed`. The Rust
test body completed in 13.47 seconds and the complete package command in 18.14
seconds. It completed all three paid model-turn processes, the relocated
resume, the divergent Pull/Keep Both path, revision-3 two-lineage
reconciliation, and reviewed Push/Pull fixed points.

One earlier implementation execution completed its first paid source turn and
then failed a local-only plan oracle: a replica with no local binding correctly
plans `CreateLocal`, while the initial assertion expected `MaterializeCloud`.
The assertion was corrected to the production relation contract before the
successful three-turn run. Neither execution automatically retried a model
request. Across development of the full gate, four paid turns completed; each
was independently capped at USD 0.05 by Claude Code. Tuzi's consumption log is
still authoritative for the settled total.

It complements rather than replaces the deterministic AIMock target. AIMock
retains exhaustive five-revision, two-direction Keep Both, tool-call, and six
final-snapshot resume coverage. The Tuzi target spends only on the provider
boundaries that a local fixture cannot establish, plus the requested
divergent Pull case.

### 7.1 Revision and paid-turn matrix

| Revision | Paid turn | Replica | Required behavior |
| --- | ---: | --- | --- |
| 1 | 1 | A | Create one real zero-tool `claude-sonnet-5` session at low effort, validate its native transcript through the production Claude adapter and Session Detail, then reviewed Push. |
| 1 → 2 | 2 | B | Reviewed Pull materializes the cloud thread with new local and provider identities; resume that production-relocated provider ID through Tuzi and reviewed Push. |
| 2 → 3 | 3 | A | Resume A's stale base independently, require a Diverged comparison and Push rejection, then review and apply exactly one Keep Both action before publishing both lineages. |
| 3 | 0 | B | Pull the added preserved lineage without replacing B's stable primary provider ID. Reconcile both replicas and apply reviewed empty Pull plans and no-op Pushes. |

There are exactly three model-turn processes. Each uses:

```text
--bare
--print
--output-format json
--model claude-sonnet-5
--effort low
--max-budget-usd 0.05
--max-turns 1
--permission-mode dontAsk
--tools=
--no-chrome
--system-prompt <fixed minimal instruction>
```

The closed child environment additionally sets:

```text
CLAUDE_CODE_MAX_RETRIES=0
CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1
CLAUDE_CODE_MAX_OUTPUT_TOKENS=1024
API_TIMEOUT_MS=120000
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
```

No process has a retry or fallback-model path. Each successful JSON result
must report one turn, exact model usage containing only `claude-sonnet-5`, the
expected deterministic marker, the expected session ID, and a client cost no
greater than USD 0.05. The aggregate client estimate is bounded by three times
that amount. A dedicated Tuzi key quota and the final Tuzi consumption log are
still the authoritative external cost controls.

### 7.2 Runner preflight

`scripts/run-live-claude-tuzi-e2e.mjs`:

1. requires the explicit three-paid-turn gate;
2. requires Node.js 20.15.0 or newer;
3. probes exact Claude Code 2.1.235 from a disposable home before reading the
   credential;
4. validates `.env.local` with the same bounded, no-follow, stable, mode-`0600`
   three-field contract as the POC;
5. accepts only the exact Tuzi HTTPS origin and normalizes `/v1` back to the
   site root;
6. performs a bounded Bearer-authenticated `/v1/models` preflight for the exact
   model ID;
7. removes inherited provider, provider-home, cloud, proxy, and alternate live
   gate variables before Cargo starts; and
8. deliberately does not place the Tuzi key in Cargo's environment.

The Rust target independently reopens and validates `.env.local`. This avoids
treating the JavaScript preflight as authority and lets the key enter only the
three direct Claude child environments as `ANTHROPIC_API_KEY`.

### 7.3 Mallard and transcript oracles

The target requires all of the following:

- production Claude analysis accepts every authoritative and materialized
  transcript as resumable;
- the codec lineage contains exactly two durable records per text turn;
- every present raw top-level `sessionId` and `cwd` equals the owning
  destination provider ID and canonical project path;
- every assistant record and Claude result identifies exact Sonnet 5;
- the production timeline and paged Session Detail APIs return the exact
  ordered user/assistant marker history and zero tools;
- Pull onto B changes machine-local local/provider identities but preserves the
  semantic lineage and leaves A byte-identical;
- B's relocated native provider ID resumes successfully through the real
  Claude CLI and Tuzi;
- the stale A Push is rejected with `RemoteAdvanced`, and divergent Pull offers
  exactly one nonselectable reviewed Keep Both resolution;
- Keep Both preserves A's local transcript byte-for-byte, materializes the
  remote transcript with a new UUIDv4 provider ID, and records provenance;
- after revision 3, each replica has exactly two bindings, two Synced
  comparison rows, the exact two-cloud set, and root-level JSONL stems equal to
  the two resumable provider IDs;
- all production-minted provider IDs are RFC 4122 UUIDv4 values;
- local and provider namespaces are disjoint across replicas and from the
  shared cloud namespace;
- reviewed empty Pull plans are actually applied, both no-op Pushes report zero
  changed resources, and storage remains byte-identical; and
- full-depth bounded no-follow scans find no API key or inherited secret marker
  in stdout, stderr, Claude homes, process homes, Mallard homes, portable
  manifests, storage object keys, or storage object bodies.

Transcript payload objects legitimately contain the source structural cwd.
The privacy assertion therefore forbids local paths from portable manifests
and object keys, while credential bytes remain forbidden in all object bodies.
This distinction avoids claiming that conversation payload bytes are path-free.

## 8. Implementation scope

The Tuzi work consists of:

- the paid package runner in `scripts/run-live-claude-tuzi-e2e.mjs`;
- the ignored `live_claude_tuzi_setup_push_pull_divergence` function and strict
  fake-only connection parser tests in
  `src-tauri/tests/live_claude_aimock_e2e.rs`;
- the `test:live-claude-tuzi-e2e` package command;
- this plan/result record; and
- the README operator contract.

The implementation intentionally adds no product setting, API key store,
network client, provider fallback, mock/gateway abstraction, schema field,
migration, compatibility layer, automated CI spend, dependency, or lockfile
change.

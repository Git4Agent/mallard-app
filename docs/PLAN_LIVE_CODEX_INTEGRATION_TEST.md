# Plan: Provider-Backed Live Codex Integration Test

Status: implemented and verified. The bounded provider-command and
loopback-capture POCs described below passed, followed by the complete exact
live gate on August 19, 2026.

## 1. Goal and compatibility contract

The live test exercises Mallard's production project-sync and storage-Library
paths with five real provider-backed `gpt-5.6-luna` conversation turns at low
reasoning effort. It uses two isolated Codex homes and one shared temporary
local-folder store to prove
Push, Pull, stale-plan rejection, divergence in both directions, Keep Both,
paginated history materialization, old-history reconstruction, project-path
remapping, and skill and fake-pet convergence.

The provider connection comes only from an ignored repository-root
`.env.local`. The test does not read a user's Codex home, use a source profile,
require an interactive login, or read, copy, or create `auth.json`. There is
no other legacy credential file. The intentionally read `.env.local` is the
sole credential input. There is one current execution path: custom-provider
configuration is supplied directly to each Codex child. There is no migration,
compatibility branch, deprecated option, authentication fallback, or alternate
provider path.

The product formats do not change:

- machine-local Mallard state remains schema 1;
- portable project-bundle manifests remain schema 1;
- storage Library repositories and catalog state remain schema 1; and
- no new portable field or storage namespace is introduced.

## 2. Local provider input

The repository-root `.env.local` is a raw JSON connection document:

```json
{
  "_type": "newapi_channel_conn",
  "key": "<provider-api-key>",
  "url": "<https-provider-base-url>"
}
```

The real key and endpoint are intentionally absent from tracked
documentation. The file is local-only, covered by the repository's `*.local`
ignore rule, and must have mode `0600` on Unix.

The runner and Rust test fail closed unless all of these conditions hold:

- the file is a no-follow regular file at the repository root;
- its size is within a small fixed bound;
- its Unix permissions do not expose it to group or other users;
- the JSON object has exactly `_type`, `key`, and `url`;
- `_type` is the supported connection-document type;
- the key is nonempty, bounded, and contains no control characters; and
- the URL uses HTTPS and contains no user information, query, or fragment.

The accepted base URL is normalized once for the Responses API. Validation
errors use fixed, content-free messages. Neither the parsed document nor an
arbitrary provider response is printed.

`.env.local` does not enable paid execution by itself. The dedicated gate is
still mandatory:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The installed CLI must report `codex-cli 0.147.x`. The rollout fixture writer
targets that current format, so the test rejects any other version before the
first model request. Moving to a later CLI requires updating the one current
fixture format in place; it must not select an older writer.

The test remains `#[ignore]`, is excluded from `npm run test:integration`, and
runs as one exact Rust integration test with one test thread.

## 3. Codex custom-provider execution

Each of the five provider-backed conversation turns uses the installed public
Codex CLI with:

- model `gpt-5.6-luna`;
- low reasoning effort;
- a dedicated custom-provider ID;
- the Responses wire API;
- OpenAI authentication disabled for that provider;
- approvals disabled;
- user configuration and project rules ignored;
- an explicit project cwd and sandbox mode; and
- bounded arguments, prompt, JSONL output, stderr, event count, and wall time.

The complete non-secret provider definition is passed on every provider-backed
invocation as a closed sequence of repeated `--config` overrides. The API key
is not one of those overrides. The provider definition names a dedicated
environment variable, and only the direct provider-backed Codex child receives
the key in that variable. The version probe, Cargo process, build scripts,
plugin probes, loopback resume children, and unrelated children do not receive
it.

Before spawning Codex, the harness removes inherited OpenAI, Codex, Azure, and
test-runner credential variables. The key never enters:

- a command argument;
- a generated Codex configuration file;
- a prompt or model output;
- stdout or stderr diagnostics;
- an authoritative or disposable rollout;
- a project or external fixture file;
- a Mallard manifest, object body, or object key; or
- Library skill or pet content.

The supported command shape deliberately retains `--ignore-user-config`.
During POC work, `--strict-config` was rejected because it also evaluated an
unrelated managed configuration. Allowing a temporary `config.toml` to load
also pulled in generated plugin state whose serialized configuration was
oversized for this bounded harness. Repeated non-secret CLI overrides avoid
depending on either source as configuration input while keeping provider
selection explicit. The same overrides set `features.plugins=false` because
this scenario exercises Mallard's filesystem-backed skill and pet portability,
not the Codex curated marketplace. Disabling that unrelated background
repository synchronization keeps the isolated provider homes deterministic;
the bounded privacy traversal still scans every regular file Codex creates.
There is no fallback to the rejected command shapes.

## 4. Fresh isolated topology

Every run starts its two-replica scenario under one main temporary root:

```text
temporary root
├── machine1/              isolated Mallard/process home for replica A
├── machine2/              isolated Mallard/process home for replica B
├── myconf1/               isolated Codex session home for replica A
├── myconf2/               isolated Codex session home for replica B
├── project1/              temporary project for replica A
├── project1_copy/         matching temporary project for replica B
├── project1_external/     exact additional write root for revision 1
└── mystorage1/            shared local-folder object store
```

Each of the twenty headless verifiers separately owns a fresh disposable
temporary root outside that main topology:

```text
independent verifier root
├── process-home/          isolated HOME and USERPROFILE
└── codex-home/            bounded snapshot for exactly one resume
```

The two replicas have distinct machine homes, Mallard repositories, host IDs,
replica IDs, project bindings, provider profiles, project roots, and Codex
homes. They share only the intended storage ID, bundle ID, and local-folder
store. The real repository, `.env.local`, and all user state remain outside
every model-visible project and writable sandbox root.

The isolated Codex homes contain only state created or materialized by the
test. Provider selection is supplied through the child command, not through a
secret-bearing home file. The main topology and every independently owned
verifier root are removed by their respective temporary-directory owners after
success or failure.

## 5. Codex CLI turn budget

The recurring live gate makes exactly 25 Codex CLI turns, with no retry path:

| Phase | Provider-backed CLI turns | Loopback CLI turns |
| --- | ---: | ---: |
| Revisions 1–5 conversation flow | 5 | 0 |
| Paginated fixture generation and revisions 6–10 | 0 | 0 |
| Skill and fake-pet Library flow | 0 | 0 |
| Ten final logical histories reopened in each of two homes | 0 | 20 |
| **Total** | **5** | **20** |

The CLI-turn counter increments only after a Codex child is successfully
spawned, and the test asserts both the five-turn paid boundary and the exact
final count of 25. This is not described as an HTTP-request count: the
revision-1 `apply_patch` turn can require more than one provider exchange.
Filesystem inspection, Session Detail queries, codec checks, Push/Pull
operations, skills, and pets do not make provider-backed turns.

## 6. Revisions 1–5: symmetric conversation divergence

The five conversation turns produce the original two-replica flow:

1. Replica A creates the base conversation in `project1` and publishes
   revision 1.
2. Replica B Pulls it into `project1_copy`, resumes the materialized alias,
   and publishes revision 2.
3. Replica A independently resumes its revision-1 alias. Its stale Push is
   rejected, its conversation is classified as divergent, and reviewed Pull
   preserves both branches before A publishes revision 3.
4. Both replicas branch from the shared primary. A publishes revision 4
   first.
5. B's stale Push is rejected; reviewed Keep Both preserves its local branch,
   installs A's storage branch as primary, and publishes revision 5 without a
   sixth provider-backed turn.

After A Pulls revision 5, both replicas contain one primary and two preserved
logical histories with distinct provider identities. Comparison, reviewed
Pull, and reviewed Push all reach a no-op fixed point.

## 7. Revision-1 project and outside-path coverage

The first conversation turn is the only write-capable model turn. It runs with
`workspace-write` from `project1` and grants exactly
`project1_external` through `--add-dir`. It must use `apply_patch` once to
create two bounded regular files inside the project and two inside that sibling
external directory. The remaining four conversation turns and all twenty
history checks are read-only.

Before revision 1 is published, the harness copies only the two in-project
files into `project1_copy` at the same relative paths. It does not copy or
modify the outside files. The three representation layers must then agree:

| Surface | In-project path | Outside-project path |
| --- | --- | --- |
| Source rollout | absolute descendant of `project1` | original absolute sibling path |
| Portable conversation object | `/__mallard_root__/` plus relative suffix | byte-identical original path |
| Pulled rollout | absolute descendant of `project1_copy` | byte-identical original path |
| Session Detail on either replica | relative Paths entry and `./` visible form | unchanged only in bounded Arguments/Result |

The prefix-lookalike sibling is intentional: `project1_external` is not a
descendant of `project1`. Session Detail uses the same boundary-aware
exact-root-or-descendant decision as the sync codec. URI, JSON-string, and
object-key occurrences follow that decision; provider-home and transcript
paths stay redacted; and an absolute or escaping value cannot enter
`affected_paths`.

The test checks the source, wire, and target representations independently.
It also verifies every final revision-1-derived alias in both provider homes,
so a successful normalized UI projection cannot hide an incorrect physical
remap.

## 8. Revisions 6–10: paginated dependency graph

Replica A writes a native current-format dependency graph without model calls:

```text
P0 shared root
├── P1 left parent       base = P0 after the first turn
│   ├── P3 left leaf     base = complete initial P1
│   └── P4 empty leaf    base = the same P1 cutoff
└── P2 right parent      base = P0 after the second turn
```

The graph exercises depth, fan-out, different cutoffs into one parent,
cross-segment message parents, and a leaf whose visible history is entirely
inherited. Generated files use current session metadata, canonical IDs,
contiguous ordinals, exact byte/ordinal cutoffs, a final newline, and matching
session-index entries. Production validators reject missing bases, cycles,
bad cutoffs, prefix mutation, noncontiguous ordinals, truncated JSONL, and
projection instability.

The project flow continues:

- revision 6 publishes and materializes P0–P4 on replica B;
- revision 7 creates local-ahead, storage-ahead, and divergent conversations,
  rejects A's stale reviewed Push, and applies reviewed Pull with Keep Both;
- revision 8 publishes A's surviving local and preserved work;
- revision 9 reverses the divergence direction and rejects B's stale plan;
- revision 10 publishes B's root update and preserved reverse branch; and
- both replicas reach a no-op Pull/Push fixed point at revision 10.

Frozen-prefix assertions prove later changes to P0 or P1 cannot leak into a
descendant whose history base selected an earlier prefix. The final graph has
five primary histories plus preserved variants for the two divergent leaves,
for seven graph histories total.

## 9. Deterministic full-history proof and twenty headless captures

The three final revision-1–5 histories and seven final graph histories yield
ten logical histories. Each one is verified from both replicas, covering all
twenty final provider aliases. The verification has two deterministic layers:
the production Session Detail path proves complete displayed marker identity,
role, and order; a public headless resume then proves that Codex serializes the
same restored history at its outbound Responses boundary. The latter uses
twenty `codex exec --json resume --ephemeral` CLI turns against loopback and
makes no remote model request.

Immediately before each resume, the test resolves the binding's exact provider
alias through production `resolve_codex_root` and
`build_standalone_codex_root` encode behavior. It writes the bounded flattened
artifact to a disposable rollout and pages it through the production Codex
Session Detail adapter with a limit of two. Only exact Prompt and Response
events participate. The test rejects truncation, inferred counts, duplicate
event identities, non-increasing ordinals, and non-advancing cursors, then
compares the entire displayed user/assistant role-and-marker vector with the
expected history. Missing inherited turns, wrong roles or order, duplication,
sibling leakage, extra displayed items, and missing preserved branches fail
before Codex is launched.

Codex can append the verification turn even when `--ephemeral` is enabled. The
harness therefore makes a fresh bounded, no-follow snapshot of the relevant
isolated Codex home for each check. It copies only controlled session and
session-index files into a disposable Codex home paired with an isolated
process home. The authoritative `myconf1` and `myconf2` homes are never used as
the resume target, and their complete bounded history/session digest
inventories must remain byte-identical.

Each resume starts a one-request HTTP/1 recorder bound to literal
`127.0.0.1` on an ephemeral port. The test overrides the restored session's
current provider ID in place with a secret-free Responses definition pointing
to that loopback address, retains model `gpt-5.6-luna` and low reasoning,
disables request compression and plugins, removes provider credentials and
proxy variables, and permits no retry. The recorder accepts only one bounded
JSON `POST /v1/responses`, retains no request body on disk, and returns a fixed
valid assistant-plus-completion SSE stream.

The captured request must name the Luna model, request streaming, carry low
reasoning effort, and contain the complete ordered historical fixture marker
vector followed exactly once by the fixed reopen probe. User markers are read
only from position-zero test prefixes in user `input_text`; assistant markers
are exact test tokens in assistant `output_text`. Context, developer/system
instructions, tool state, and unrelated message text are not mistaken for
chat fixtures. The recorder reduces the bounded JSON immediately to closed
counts and comparison facts and never prints request content, marker values,
thread identities, or paths.

For every resume, Codex must emit exactly one `thread.started` event containing
the binding-selected provider thread ID and exactly the recorder's fixed final
assistant response, with no tool event. The recorder must observe exactly one
request. Afterward, the target rollout must be longer while retaining its full
old byte prefix, proving that the exact disposable session was resumed and
appended. The recorder, snapshot, and process home are joined, bounded,
privacy-scanned, and discarded. Authoritative source history remains
byte-identical.

The old positional and adjacent-message LLM reports, output schemas, semantic
model judgments, retries, and alternate verifier paths are removed completely.
They are not retained as compatibility paths or fallbacks. This design proves
both the production Session Detail display projection and the exact restored
history passed to the public headless CLI's Responses boundary. It does not
claim to test terminal pixels or the interactive TUI.

## 10. Shared skills and fake pets

The same two provider homes participate in the storage Library flow without
additional model calls.

For skills, each home starts with a distinct nested test skill. The scenario
publishes, links, installs, and updates both skills; creates simultaneous edits;
rejects stale and divergent Push; preserves each losing edit as a renamed copy;
publishes and cross-Pulls the copies; updates the primaries again; and reaches
a no-op fixed point. Both homes finish with two primaries and two preserved
copies, with exact regular-file trees and stable primary identities.

For pets, replica A starts with a deterministic fake v1 pet and replica B with
a deterministic fake v2 pet. The scenario publishes, installs, updates,
reverses publisher direction, creates and reconciles divergence, preserves
renamed copies, cross-Pulls all four resources, validates manifests and WebP
dimensions, and reaches a no-op fixed point. Pet inventory makes no provider
CLI or model call.

Library repositories remain independent of the project bundle. Every Library
operation must leave the project repository and its revision-10 head
byte-identical.

## 11. Credential, privacy, and process assertions

The raw key, its SHA-256 bytes, and its lowercase hexadecimal SHA-256 form are
used as leak-detection needles. Before temporary state is removed, the harness
scans:

- captured stdout and stderr;
- bounded loopback request headers and bodies before they are reduced to
  closed transport/history comparison facts;
- every bounded regular-file body in both authoritative provider homes and
  both isolated process homes;
- every disposable history snapshot;
- child arguments and generated non-secret configuration material;
- projects and outside fixture files;
- local-folder storage object bodies and keys; and
- skill and pet objects.

The expected root `.env.local` is the only deliberate credential-bearing file
and is outside this scan. Assertions and failure summaries report only fixed
categories, counts, sizes, exit status, and allowlisted lifecycle labels. They
never interpolate provider text, arbitrary model output, paths, fixture bytes,
or secret-derived material.

Additional invariants include:

- all traversals are bounded, no-follow, and regular-file-only;
- each child has null stdin, an explicit cwd and sandbox, and a hard timeout;
- each history recorder binds only literal IPv4 loopback, accepts exactly one
  bounded Responses request, receives no real provider credential, and is
  joined before its snapshot is discarded;
- publication remains immutable and SHA-256 verified;
- mutable heads retain compare-and-swap and reviewed-plan checks;
- no portable manifest or object key contains a machine, project, provider
  home, Mallard home, or temporary-root path;
- expected outside paths may occur only in the bounded provider-native
  conversation bodies under test;
- all project `affected_paths` are clean relative paths;
- headless verification mutates only disposable snapshots and leaves both
  authoritative provider homes' history/session state unchanged; and
- no legacy credential file is read, copied, generated, or published; the
  ignored root `.env.local` remains the sole intentional credential input.

## 12. Provider POC evidence

Three isolated POCs establish the command shapes needed by the live rewrite:

1. Fresh persisted request: model discovery found the exact Luna model, one
   real request returned its exact marker, a durable rollout recorded the
   custom provider, and the credential did not persist in the temporary tree.
2. Workspace-write request: one Luna-low code-mode request completed the
   required `apply_patch`, created the exact in-project and explicitly granted
   outside-project files, and left durable tool evidence.
3. Headless history capture: public
   `codex exec --json resume --ephemeral` reopened a selected inherited
   provider thread, sent its complete restored history to a bounded literal-
   loopback Responses recorder, accepted the recorder's static SSE response,
   and confined the appended verification turn to the disposable snapshot.
   No real provider credential or remote model request was used. Full displayed
   order was independently proved through the production resolver, standalone
   builder, and Session Detail timeline adapter.

All POC trees were isolated and removed. The POCs established the individual
provider request shapes before implementation. The combined project-sync,
Library, Session Detail, and loopback-resume gate subsequently passed in one
run.

A separate public app-server 0.147 POC was not accepted. Although
`thread/read` found the rollout, resume/read/items/turns returned empty history
without an unsynchronized private Codex state database. That interface would
not prove the history under test and is not retained as a compatibility path,
fallback, or alternate execution path.

The POCs also selected the final configuration strategy. Strict loading was
not usable because unrelated managed configuration was evaluated, while a
temporary `config.toml` admitted generated oversized plugin state. The
supported live path therefore ignores ambient user configuration and repeats
the bounded non-secret provider overrides for every fresh and resumed request.
This isolates provider configuration and disables the unrelated curated-plugin
synchronizer for every fresh and resumed request.

## 13. Implementation and verification sequence

Implement the cutover in one direction:

1. make the Node runner validate the root `.env.local` and retain the explicit
   paid-use gate;
2. make the Rust target independently validate the connection before any
   request;
3. remove the source-profile and credential-copy machinery completely;
4. use the real provider definition only for the five conversation turns and a
   same-ID, secret-free literal-loopback provider definition for the twenty
   headless resumes;
5. deterministically compare all twenty complete histories through the
   production alias resolver, standalone builder, and Session Detail adapter
   before each loopback request capture;
6. remove positional and adjacent-message model judgments completely rather
   than retaining a fallback, alternate verifier, or retry;
7. require the dedicated child-only credential environment and scan every
   output/state boundary;
8. keep all existing revision, path, pagination, history, skill, pet, and
   fixed-point assertions; and
9. run offline checks before the single paid gate.

Offline verification:

```sh
git check-ignore -v .env.local
node --check scripts/run-live-codex-e2e.mjs
cargo fmt --all -- --check
git diff --check
cargo test --manifest-path src-tauri/Cargo.toml --test live_codex_e2e --no-run
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:storage-catalog-e2e
```

Paid verification, only after the offline checks pass:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

## 14. Completion gate

The provider rewrite is complete only when the paid command reports one
passing ignored integration test and all of these facts hold in the same run:

- exactly five provider-backed conversation CLI turns succeeded with
  `gpt-5.6-luna` and low reasoning, followed by exactly twenty secret-free
  loopback headless resume turns;
- both replicas converged at project revision 10;
- the project and prefix-lookalike outside paths displayed and remapped at the
  correct boundaries;
- every final provider alias matched its complete ordered user/assistant
  marker vector through the production resolver and standalone builder;
- every final provider thread reopened headlessly, emitted its exact
  `thread.started` ID, sent the complete expected history plus one probe in its
  single captured Responses request, accepted the fixed SSE response, and
  appended only its disposable snapshot;
- both homes converged on the primary and preserved skill and pet resources;
- stale, divergent, Keep Both, no-op, and immutable-publication checks passed;
- the credential appeared nowhere outside the expected ignored input file;
- authoritative session homes and project/Library repositories changed only
  at their explicitly allowed phases;
- every temporary fixture was removed; and
- every machine-local, project-bundle, Library, and catalog schema constant
  remained 1.

The exact gate passed on August 19, 2026 with this command:

```sh
MALLARD_RUN_LIVE_CODEX_E2E=1 npm run test:live-codex-e2e
```

The ignored Rust test reported `1 passed` in 75.91 seconds after all five
provider-backed turns and all twenty loopback resumes completed. The strict
12,000-entry and 256-MiB provider-home privacy bounds remained unchanged, and
the out-of-scope Codex plugin synchronizer remained disabled with
`features.plugins=false` for deterministic isolated homes. All product and
portable schemas remained at 1.

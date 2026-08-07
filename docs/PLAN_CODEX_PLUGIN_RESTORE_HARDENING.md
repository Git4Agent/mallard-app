# Plan: Harden Codex Plugin Restore And Machine-Local Config

> **Historical and superseded.** The implementation described here was later
> removed. Its target-machine resolution and truthful-verification lessons
> remain inputs to the new storage-scoped design, but its profile lock/config
> transport is not a compatibility path:
> [`DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md`](./DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md).

## Status

Formerly implemented on 2026-07-13 as the hardening follow-up to
`PLAN_ENVIRONMENT_RECONCILER.md`.

Automated verification: `cargo test --no-fail-fast`, `cargo check --lib`, and
`npm run build`.

This plan replaces two assumptions that are now disproven:

1. OpenAI-managed marketplaces are not automatically available in every
   custom `CODEX_HOME`.
2. A raw `config.toml` copied into a different Codex home is not immediately
   portable, even when the target is on the same machine.

## Outcome

After a user pulls a `.codex` profile into a fresh or custom Codex home and
runs the explicit **Repair** action:

- every desired portable plugin is either installed, deliberately disabled,
  or reported as a real blocked/manual item;
- `openai-curated`, `openai-bundled`, and `openai-primary-runtime` are resolved
  from this target machine rather than from paths captured on another home;
- the target `config.toml` contains target-valid managed marketplace and
  managed MCP paths;
- the portable cloud representation of `config.toml` contains no
  machine-local managed marketplace roots or Codex-generated managed MCP
  paths;
- the repair summary never reports success while requested plugins were
  skipped; and
- a second Repair run is a no-op.

The concrete regression scenario is:

```text
source home: /Users/hequ/Desktop/project/myconf2/.codex
target home: /Users/hequ/Desktop/project/myconf3/.codex

desired:
  ponytail@ponytail
  google-calendar@openai-curated
  slack@openai-curated
  enabled OpenAI-bundled/runtime plugins
```

## Confirmed Problems

### 1. Curated plugins are silently skipped in a fresh custom home

The synced lock contains:

```text
google-calendar@openai-curated
slack@openai-curated
```

but the target inventory contains only `openai-bundled` and `ponytail`.
`capture_lock` records curated plugin IDs without source information, while
`build_plan` converts a missing `openai-curated` marketplace into a warning:

```text
'google-calendar@openai-curated' skipped: marketplace 'openai-curated'
is neither configured here nor in the lock
```

That warning does not enter `report.failed`, so the final result can still
look successful.

### 2. Managed marketplace availability is scoped to the physical Codex home

The default `~/.codex` and `myconf2/.codex` inventories include
`openai-curated`; `myconf3/.codex` does not. The current CLI also refuses to
register an arbitrary local path as `openai-curated` because the name is
reserved.

The official CLI documentation says marketplace inventory includes both
implicitly discovered default marketplaces and configured marketplace
snapshots. The managed-configuration documentation separately requires a
Codex-managed OpenAI marketplace's reserved name and source to match.

References:

- https://learn.chatgpt.com/docs/developer-commands#codex-plugin-marketplace
- https://learn.chatgpt.com/docs/enterprise/managed-configuration#restrict-plugin-marketplace-sources

### 3. Built-in plugin intent is omitted from the portable lock

`capture_lock` currently skips every plugin from `openai-bundled` and
`openai-primary-runtime`. That is safe only when the target installation has
already provisioned those marketplaces and payloads.

A fresh custom home can contain enabled config entries such as:

```toml
[plugins."sites@openai-bundled"]
enabled = true

[plugins."browser@openai-bundled"]
enabled = true
```

while `codex plugin list --json` reports none of those plugins installed. The
current lock has no way to request them.

### 4. Raw config carries source-home paths into the target

The reproduced target config contains all of these source-home values:

```toml
[marketplaces.openai-bundled]
source = "/Users/hequ/Desktop/project/myconf2/.codex/.tmp/bundled-marketplaces/openai-bundled"

[mcp_servers.node_repl.env]
NODE_REPL_TRUSTED_CODE_PATHS = "/Users/hequ/Desktop/project/myconf2/.codex"
CODEX_HOME = "/Users/hequ/Desktop/project/myconf2/.codex"
SKY_CUA_SERVICE_PATH = "/Users/hequ/Desktop/project/myconf2/.codex/plugins/cache/..."
```

The current file-level allowlist and Tier 3 conflict policy cannot remove
individual machine-local tables. A cloud-only `config.toml` is therefore
materialized verbatim into a fresh target.

### 5. Reporting equates "nothing failed to execute" with success

`apply_plan` logs warnings for unresolved plugins but does not add them to
`failed`. `verified` checks only `plan.missing_plugins`; plugins skipped before
that list are invisible to verification. The frontend then chooses success or
error solely from `report.failed.length`.

### 6. Tests assume the missing prerequisite is always present

The current Codex repair fixtures seed `openai-curated` into the target
marketplace inventory. They cover Git marketplace add/install ordering but do
not cover a fresh custom `CODEX_HOME` with no managed marketplace catalog.

### 7. Cache exclusions are not a hard safety boundary

`.codex/.tmp` and `.codex/plugins/cache` are excluded by default today, but
they are not in `NEVER_SYNC_DIRS`. The current opt-in mechanism can therefore
put plugin caches or managed marketplace snapshots into a cloud profile,
contradicting the target-machine-owned design.

## Design Decisions

### D1. Keep plugin payloads and marketplace snapshots out of cloud sync

The cloud profile continues to carry portable intent only. These remain
machine-local and become hard-denied even for explicit opt-ins:

```text
.codex/.tmp/**
.codex/plugins/cache/**
~/.cache/codex-runtimes/**
```

Provisioning a catalog locally during explicit setup is not the same as
syncing that catalog between machines.

Add `.codex/.tmp`, `.codex/plugins/cache`, and known plugin staging directories
to `NEVER_SYNC_DIRS`. Invert the existing test that currently allows an opt-in
to `.codex/plugins/cache`.

### D2. Treat all OpenAI marketplace names as managed identities

Use one classifier in `codex_plugins.rs`:

```text
openai-curated         managed curated catalog
openai-bundled         managed desktop bundle
openai-primary-runtime managed workspace runtime
```

Their plugin IDs are portable intent. Their local source roots are not.

### D3. Resolve managed sources only from the target machine

Never accept a managed marketplace path from the lock or trust an incoming
absolute path merely because its table name is reserved.

Discover managed sources by running the installed Codex CLI against the
machine's default Codex environment, then validate:

- exact expected marketplace name;
- expected local/implicit source kind;
- supported marketplace manifest with the same name;
- source root under an approved target-machine location; and
- curated SHA sidecar consistency where Codex requires it.

If the target machine has no valid source, Repair remains blocked with an
actionable message to initialize/update Codex on that machine.

### D4. Use the stable CLI surface, not under-development App Server methods

Continue using:

```text
codex plugin marketplace list --json
codex plugin marketplace add ... --json
codex plugin list --json
codex plugin add ... --json
```

The documented App Server `plugin/list`, `plugin/read`, and `plugin/install`
methods are explicitly under development and marked unsuitable for production
clients. Do not make this repair feature depend on them.

Reference:

- https://learn.chatgpt.com/docs/app-server#api-overview

### D5. Add a narrow config portability codec, not a generic path rewriter

The app will understand only explicitly classified machine-local Codex
sections. It will not search-and-replace arbitrary path strings in TOML,
hooks, prompts, or user-authored MCP definitions.

### D6. Keep the explicit consent boundary

Readiness scanning remains read-only. Catalog provisioning, config repair,
and plugin installation happen only through **Setup root** or the explicit
**Repair** action because plugins can add skills, MCP servers, apps, and hooks.

### D7. Preserve non-destructive target policy

- never uninstall an extra target plugin;
- never re-enable a plugin explicitly disabled on the target;
- never overwrite a same-name/different-source custom marketplace;
- never transfer connector credentials or hook trust;
- never replace an unknown existing managed-catalog path automatically; and
- back up `config.toml` before changing target-local sections.

## Target Architecture

```text
portable cloud profile
  config.toml (portable projection only)
  codex-plugins.lock.json (plugin ids + portable Git sources)
             |
             v
pull into target Codex home
             |
             +--> compose portable config with target-local overlay
             |
             +--> discover this machine's managed catalogs
             |
             +--> provision/rebind managed marketplaces
             |
             +--> add portable Git marketplaces
             |
             +--> install missing plugins into target cache
             |
             +--> rebuild known managed MCP paths
             |
             v
final inventory + truthful Ready / Partial / Failed result
```

## Part A: Capture Complete Plugin Intent

### A1. Capture managed plugin IDs

Change `capture_lock` so installed and enabled plugins from all three managed
marketplaces are recorded in `lock.plugins`, just like curated IDs are today.
Do not add a marketplace source record for them.

The existing schema already supports this:

```json
{
  "plugins": [
    { "id": "slack@openai-curated" },
    { "id": "browser@openai-bundled" },
    { "id": "documents@openai-primary-runtime" }
  ]
}
```

No schema bump is required because older readers already validate these IDs
and will at worst report the marketplace unavailable.

### A2. Recover intent from already-published v1 profiles

During planning, union the lock's plugin IDs with managed plugin entries from
the target `config.toml` where `enabled = true`. This handles current locks
that omitted bundled/runtime plugins.

This fallback is deliberately narrow:

- only the three managed marketplace suffixes;
- only an explicit boolean `enabled = true`;
- `enabled = false` remains disabled; and
- custom marketplace intent still comes only from the lock.

On the next successful push from a complete source environment, normal lock
capture makes the fallback unnecessary.

## Part B: Provision Managed Marketplaces Locally

Add a `ManagedMarketplaceResolver` beside `ProcessRunner`.

### B1. Discover source roots

Run a second, unoverridden Codex inventory against the machine's default
environment. Produce:

```rust
struct ManagedMarketplaceSources {
    curated_root: Option<PathBuf>,
    curated_sha: Option<PathBuf>,
    bundled_root: Option<PathBuf>,
    primary_runtime_root: Option<PathBuf>,
}
```

Validate roots before use. Do not fall back to paths read from the pulled
config.

### B2. Make curated available to a custom home

`openai-curated` is implicitly discovered at the custom home's expected
catalog path and requires the matching SHA sidecar.

For a custom home:

1. If its curated catalog is valid, do nothing.
2. Otherwise stage a complete target-owned copy of the validated default
   catalog and its SHA sidecar.
3. Use a platform clone/reflink optimization when available, with a normal
   recursive copy fallback; the final target must not depend on a symlink to
   another Codex home.
4. Never replace an unknown real directory. Move an app-created stale link or
   incomplete staging directory to a timestamped backup first.
5. Publish the staged directory and sidecar as one recoverable operation; if
   the second rename fails, restore the previous pair.
6. Re-run marketplace inventory and require `openai-curated` to appear before
   installing any curated plugin.

The `.tmp` hard deny prevents the local catalog from entering the cloud
profile.

### B3. Rebind bundled and primary-runtime sources

For `openai-bundled` and `openai-primary-runtime`, write target-local
marketplace tables using only the validated roots discovered in B1.

Do this through the config portability layer below so the absolute roots are
active locally but absent from uploaded bytes.

After rebinding, require `codex plugin marketplace list --json` to report the
expected marketplace name and validated source before plugin installation.

### B4. Missing managed source is blocked, not skipped

Use stable error codes such as:

```text
managed_catalog_missing
managed_catalog_invalid
managed_catalog_source_mismatch
managed_catalog_provision_failed
```

Every desired plugin depending on that marketplace enters `blocked_plugins`.
It must not disappear into a warning.

## Part C: Make `config.toml` Portable Without Losing Local State

Add `src-tauri/src/codex_config.rs` with pure parsing/projection/composition
helpers.

### C1. Split physical config into portable and local parts

```rust
struct CodexConfigParts {
    portable: toml::Table,
    local: CodexLocalOverlay,
}
```

Classify these as machine-local:

1. every `[marketplaces.<name>]` table whose `source_type = "local"`;
2. the Codex-generated `mcp_servers.node_repl` block when it matches the
   managed fingerprint; and
3. the Codex-generated `mcp_servers.computer-use` block when it matches the
   managed fingerprint.

Keep these portable:

- model, reasoning, service-tier, feature, and UI preferences;
- `[plugins."id@marketplace"].enabled` values;
- Git marketplace registrations;
- user-authored MCP servers and their bytes; and
- every unknown table or key.

Do not rewrite or remove `[projects."<absolute path>"]` tables in this focused
fix. Cross-machine project/trust portability belongs to the existing sidebar
and readiness work; this plan only surfaces stale project paths as manual
readiness items.

### C2. Project before comparison and upload

Replace direct `read_upload_data` use for `.codex/config.toml` with a
path-aware logical read:

```rust
read_sync_bytes(rel, physical_path) -> portable_bytes
```

The portable bytes, not the physical file bytes, drive:

- local-vs-baseline status;
- SHA and size comparisons;
- pending uploads;
- manifest entries; and
- no-op detection.

Without this change the local overlay would make `config.toml` appear modified
forever after every pull.

If the TOML does not parse, fail closed for this file: keep the last valid
cloud object, log the parse error, and never upload raw bytes that may contain
machine-local paths or secrets.

### C3. Compose on pull

When applying cloud `config.toml`:

1. parse the incoming portable bytes;
2. extract the overlay from the current target config, if one exists;
3. combine incoming portable content with that local overlay;
4. back up the full current file;
5. write the composed file atomically; and
6. retain Tier 3 conflict-copy behavior for conflicting portable content.

If portable local and cloud bytes are equal, advance the baseline without
rewriting or dropping the local overlay.

Conflict siblings contain portable bytes only. They are review artifacts, not
active target config, and must not contain another machine's local overlay.

### C4. Rebuild only known managed MCP values

For a fresh target with no local overlay, copy the known managed MCP templates
from the validated default target config and set only these semantic values:

```text
NODE_REPL_TRUSTED_CODE_PATHS = target CODEX_HOME
CODEX_HOME                    = target CODEX_HOME
SKY_CUA_SERVICE_PATH          = installed target computer-use payload
```

Retain application/runtime paths from the target machine only after verifying
they exist. Do not rewrite arbitrary environment values or custom MCP servers.

Run this step after managed plugin installation so cache paths are known.

## Part D: Repair Flow And Result Semantics

Replace the current warning-driven flow with these ordered phases:

1. Read and validate the portable lock.
2. Parse enabled managed plugin fallback intent from target config.
3. Fetch target inventory.
4. Discover and provision/rebind required managed marketplaces.
5. Fetch inventory again and require provisioned marketplaces to appear.
6. Add missing portable Git marketplaces through the existing CLI path.
7. Install every missing desired plugin.
8. Rebuild known managed MCP config paths.
9. Fetch final inventory.
10. Verify every desired portable plugin independently.

Keep disabled target plugins in `disabled`; do not install or enable them.

### D1. Structured plan/report fields

Extend `CodexPluginPlan` with:

```text
missing_managed_marketplaces
blocked_plugins
config_repairs
```

Extend `CodexPluginRepairReport` with:

```text
managed_marketplaces_provisioned
blocked_plugins
config_paths_repaired
```

Use structured entries with `id`, `code`, and `message` instead of encoding
important state only in log strings.

### D2. Define terminal states precisely

```text
Ready
  every desired portable plugin is present+enabled or deliberately disabled;
  no execution failures or blocked items remain.

Partial
  manual or blocked items remain, but completed installs are valid.

Failed
  a requested catalog/config/install operation failed.
```

`verified` is true only for **Ready**. A warning about a requested plugin can
never coexist with `verified = true`.

The frontend must stop deriving success solely from `failed.length`.
`setup_root` must also gate its `Root ready` message on the structured Ready
state rather than on command completion.

## Part E: Readiness And UI

Extend the read-only readiness scan to report:

- desired plugin references a missing managed catalog;
- managed marketplace source path does not exist;
- managed marketplace source points into a different Codex home;
- managed MCP `CODEX_HOME` or trusted path differs from the selected root;
- managed MCP executable/cache path is missing;
- desired plugin remains unresolved after repair; and
- pulled config contains a machine-local table that has not yet been
  projected/repaired.

Use one **Repair** action for the Codex group. The log should show the ordered
catalog, marketplace, plugin, and config phases, while the status line stays
short:

```text
Codex plugins ready
Codex plugins partially restored
Codex plugin restore failed
```

## Implementation Map

### Backend

- `src-tauri/src/codex_plugins.rs`
  - managed marketplace classifier;
  - complete managed plugin capture;
  - config fallback intent;
  - managed source resolver/provisioner;
  - structured blocked results;
  - final per-plugin verification.
- `src-tauri/src/codex_config.rs` (new)
  - config split/project/compose;
  - managed-table fingerprints;
  - target-local marketplace overlay;
  - managed MCP template rebuild.
- `src-tauri/src/lib.rs`
  - path-aware logical sync bytes for `config.toml`;
  - compose-on-pull integration;
  - projected baseline/hash handling;
  - repaired setup ordering;
  - Tauri result serialization.
- `src-tauri/src/readiness.rs`
  - managed marketplace and stale-path findings.

### Frontend

- `src/types.ts`
  - structured plan/report additions.
- `src/App.tsx`
  - choose Ready/Partial/Failed from the report state.
- `src/components/FinishSetup.tsx`
  - keep one Codex Repair action and display blocked details.

### Documentation

- update `AGENT_SYNC_FILE_SETS.md` to describe `config.toml`'s portable
  projection plus machine-local overlay;
- update `PLAN_ENVIRONMENT_RECONCILER.md` to remove the assumption that every
  target already owns all managed catalogs; and
- retain `.tmp/**`, plugin caches, runtime caches, credentials, and hook trust
  in the Never/machine-local boundaries.

## Delivery Order

### Phase 1: Lock and truthful planning

- Add failing tests for a custom home with no managed catalog.
- Capture managed plugin IDs.
- Parse managed fallback intent from config.
- Add structured blocked plugin state.
- Make `verified` account for every requested ID.

This phase fixes misleading success reporting even before automatic local
catalog provisioning is added.

### Phase 2: Config portability codec

- Add split/project/compose pure functions.
- Project config bytes for status, baseline, manifest, and upload.
- Compose with the target overlay on pull.
- Add fail-closed parse handling and atomic backups.

Land this before writing new target-local marketplace paths, otherwise those
paths will continue entering cloud objects.

### Phase 3: Managed marketplace provisioning

- Discover validated sources from the target default Codex environment.
- Provision curated implicit-catalog links/copy fallback.
- Rebind bundled and primary-runtime local tables.
- Re-inventory before installing.
- Surface initialization requirements when no source is available.

### Phase 4: Plugin and managed MCP repair

- Install missing managed and Git plugins.
- Rebuild known managed MCP paths after installation.
- Run final per-plugin and per-path verification.
- Keep repeated repair idempotent.

### Phase 5: Readiness, UI, and docs

- Add stale-source/path readiness findings.
- Update frontend report handling and terse terminal copy.
- Update file-set and prior-plan documentation.

## Automated Test Matrix

### Lock and planning

- curated, bundled, and primary-runtime enabled plugins are captured by ID;
- no managed local source path appears in lock JSON;
- v1 lock plus enabled managed config entries produces the complete desired
  set;
- explicit `enabled = false` remains disabled;
- missing managed marketplace produces blocked plugins, not warnings-only;
- custom Git marketplace behavior and spoofing guard remain unchanged;
- a custom plugin missing its marketplace source is rejected;
- capture or serialization overflow preserves the last valid lock;
- fresh capture monotonically unions with existing desired intent; unsafe
  source/ref collisions preserve the complete fresh side as a deterministic
  conflict sibling instead of hiding it;
- unsafe or over-cap Tier 2 unions preserve both complete locks through the
  standard conflict-sibling path;
- invalid or future-schema cloud locks never replace the last-good active
  lock;
- target-local explicit disable policy wins for both Codex and Claude, and a
  present unreadable Claude policy/lock fails closed;
- Claude replay requires an exact portable source for every executable plugin
  and blocks same-name source spoofing; and
- local custom marketplace remains manual.

### Managed source resolution

- exact reserved name and valid manifest succeed;
- wrong manifest name fails;
- source outside approved target roots fails;
- curated SHA missing/mismatched fails;
- existing valid custom-home curated catalog is a no-op;
- app-created partial/stale curated snapshot is repaired with backup;
- an unowned or invalid real directory is never replaced;
- missing default source produces an actionable blocked result.

### Config portability

- local marketplace tables are absent from portable bytes;
- Git marketplace tables remain portable;
- recognized managed MCP blocks are local-only;
- similarly named user-authored MCP blocks are preserved unless their managed
  fingerprint matches;
- project tables and other unrelated config remain semantically unchanged;
- model/features/plugin enablement and unknown keys round-trip unchanged;
- compose preserves the target overlay while applying cloud portable values;
- a same-name portable/target-local marketplace collision preserves the
  target table and writes a portable conflict sibling;
- projected equality is a sync no-op despite different local overlays;
- a legacy raw cloud config converges on Pull and is republished as portable
  bytes by the next Push;
- both-changed portable config still creates a deterministic conflict copy;
- malformed TOML never uploads raw bytes;
- backup and atomic-write behavior survive injected write failures.

### Repair and reporting

- managed catalogs are available before their plugins install;
- Git marketplaces install before their plugins;
- one plugin failure does not stop independent plugins;
- every unresolved requested ID appears in `failed` or `blocked_plugins`;
- `verified` cannot be true with failed/blocked/manual portable items;
- managed MCP paths point at the target home after install;
- second repair performs no catalog, marketplace, plugin, or config write.

### Sync integration

Run against both the local-folder and S3-compatible harnesses:

- source config with `/machine-A/.codex` paths uploads portable bytes only;
- fresh machine B pulls and composes machine-B paths;
- source and target overlays never produce ping-pong uploads;
- config conflict copies contain portable bytes only;
- `.tmp/**` and plugin cache trees never enter a manifest, even when explicitly
  opted in;
- selective `.codex` push still force-includes the plugin lock;
- remapped plugin-lock conflict siblings appear in readiness and ride along
  with the next selective push;
- pre-push capture pauses while a lock conflict is unresolved; and
- Resolve publishes a SHA-pinned manifest deletion and durable tombstone that
  removes unchanged copies on other machines, including after baseline reset,
  without propagating ordinary deletions.

## Manual Verification

1. Start with a fully initialized source profile containing one Git plugin,
   two curated plugins, and enabled bundled/runtime plugins.
2. Push and inspect both cloud artifacts:
   - lock has all desired plugin IDs but no local managed paths;
   - portable `config.toml` has no source-home managed marketplace or managed
     MCP paths.
3. Pull into an empty disposable custom Codex home.
4. Confirm readiness reports managed catalogs/path repair before mutation.
5. Click **Repair** once.
6. Verify:

   ```text
   CODEX_HOME=<target> codex plugin marketplace list --json
   CODEX_HOME=<target> codex plugin list --json
   ```

7. Confirm every desired plugin is installed and enabled.
8. Search target `config.toml` for the source root; it must have no match.
9. Start a new Codex task and exercise one curated, bundled, and runtime
   capability.
10. Run Repair again and confirm a no-op Ready result.
11. Temporarily remove the default managed catalog and confirm Repair reports a
    blocked initialization requirement instead of success.

## Acceptance Criteria

- The `myconf2` to `myconf3` scenario installs Ponytail, Google Calendar,
  Slack, and the desired managed plugins with one explicit Repair action.
- No active target config value references `myconf2` afterward.
- No cloud `config.toml` object contains machine-local managed marketplace or
  managed MCP paths.
- Managed marketplace/cache data remains target-owned and absent from cloud
  manifests.
- Missing managed catalogs are actionable blocked states, never silent skips.
- The UI cannot show success while a requested portable plugin is unresolved.
- Existing custom Git marketplace source-mismatch protection remains intact.
- Disabled and extra target plugins remain untouched.
- Repeated repair converges to a no-op.
- `npm run build`, `cd src-tauri && cargo test`, and
  `cd src-tauri && cargo check` pass.

## Out Of Scope

- generic TOML semantic merge;
- arbitrary absolute-path rewriting;
- project path/trust migration or rewriting;
- automatic repair of user-authored MCP servers, hooks, or prompts;
- syncing or copying credentials, OAuth state, connector authorization, or
  hook trust;
- cloud sync of marketplace snapshots or plugin/runtime caches;
- plugin uninstall, disable propagation, downgrade, or exact version pinning;
- automatic installation of the ChatGPT desktop app or primary runtime; and
- production use of under-development App Server plugin methods.

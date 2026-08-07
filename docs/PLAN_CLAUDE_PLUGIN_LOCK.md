# Plan: Claude Plugin Lock — Cross-Machine Plugin Intent That Actually Merges

> **Historical and superseded.** The plugin-lock implementation was removed.
> The new design uses typed provider intent in a storage-scoped catalog and
> does not read or migrate these locks:
> [`DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md`](./DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md).

## Implementation Status (2026-07-10)

Formerly shipped as planned, in `codex_plugins.rs` (shared lock structs, validation,
canonical serialization, and merge driver reused). Deviations:

- `captured_with` field is `agent_version` (agent-neutral), not
  `claude_version`; the Claude capture leaves it empty (no CLI is invoked).
- UI kept two badged buttons (Repair = Claude, Plugins = Codex) rather than
  unifying; the Repair badge shows the Claude plan's missing count.
- Claude replay ignores `git_ref` (Claude's marketplace add has no ref
  parameter; the manager records plugin commit SHAs, not marketplace refs).

## The Question This Answers

"I installed plugins on machine A (`config.toml`, `installed_plugins.json`,
`known_marketplaces.json` all changed). How do they get installed on another
person's machine, and how should merge conflicts on those files resolve?"

## How The App Handles These Files Today

| File | Sync status today | Cross-machine behavior |
|---|---|---|
| `.codex/config.toml` | Default-synced, Tier 3 | Both-changed → conflict copy (local wins the path, cloud lands as `config.sync-conflict-<hash8>.toml`). Plugin enable tables ride inside, but plugin *restore* is owned by `codex-plugins.lock.json` + the Plugins button; `codex plugin add` writes enable state into the target's own config.toml. |
| `.claude/plugins/installed_plugins.json` | Never synced (unlisted, deliberately not offered) | Read locally as the presence check during repair. Carries machine-local `installPath` — syncing it would corrupt the target's plugin manager. Also carries `gitCommitSha` (useful capture input). |
| `.claude/plugins/known_marketplaces.json` | Never synced (same reason) | Read locally as the presence check. Carries marketplace source repos + machine-local `installLocation`. |
| `.claude/settings.json` | Default-synced, Tier 3 | Carries the declarative intent (`enabledPlugins`, `extraKnownMarketplaces`) that the Repair button replays via `claude plugin marketplace add` / `claude plugin install`. |

**These per-file decisions are all still correct.** Manager records must never
sync (absolute paths), and nobody wants a structured merge of another person's
`config.toml` (their model, env, MCP paths). Do not change any of that.

## The Actual Gap

Claude plugin intent lives in `settings.json`, which is Tier 3. On another
*person's* machine, both sides have their own `settings.json`, so every pull
classifies it both-changed → the other person keeps **their** settings at the
canonical path and **your** intent lands in a `.sync-conflict-*` sibling that
Repair never reads. Result: Claude plugin propagation works fresh-machine
only — between two active people it silently never happens.

This is exactly the ping-pong flaw the Codex plugin lock fixed with a Tier 2
keyed-union driver. Claude still has it. The fix is the same fix.

`config.toml` needs nothing new for this goal: the Codex lock already unions
cross-machine, and replay writes enable state into the target's own file.

## Design: `~/.claude/agent-sync/claude-plugins.lock.json`

Mirror the Codex lock — same schema, same canonical serialization, same
Tier 2 keyed-union merge driver, same validation. Reuse the existing code in
`codex_plugins.rs` (structs, `validate_lock`, `save_lock`,
`merge_codex_plugin_lock`, `build_plan`-style diffing); only capture and
replay differ per agent.

```json
{
  "schema": 1,
  "captured_with": { "claude_version": "2.x" },
  "marketplaces": [
    { "name": "ponytail", "repository": "DietrichGebert/ponytail" }
  ],
  "plugins": [
    { "id": "ponytail@ponytail", "observed_version": "4.8.4" }
  ],
  "manual": []
}
```

### Capture (before every `.claude` push, alongside the codex hook)

No child processes — unlike Codex, Claude's inventory is plain files:

1. `settings.json` → `enabledPlugins == true` entries and
   `extraKnownMarketplaces` sources (`repo` / `url`; a `path` source → the
   plugin goes to `manual`, never the absolute path).
2. `known_marketplaces.json` → source repos for enabled plugins whose
   marketplace isn't in `extraKnownMarketplaces` (e.g. marketplaces added by
   CLI without settings declaration). `installLocation` is never captured.
3. `installed_plugins.json` → `observed_version` (informational, same as
   Codex).

Same rules as Codex: empty capture never writes; a good lock is never
replaced by a failed parse; caps and charset validation identical
(the lock is a trust boundary — it arrives from the cloud).

### Merge

Register the path in the existing `merge_driver` dispatch with the existing
keyed-union driver. Two people's locks union: marketplaces by name, plugins
by id. Each person's Plugins plan then shows the other's plugins as missing
— which is the whole point.

### Replay

The existing `repair_plugins_blocking` flow, with one change: read intent
from the lock when it exists, falling back to `settings.json` when it
doesn't (old profiles keep working unchanged). Presence checks
(`marketplace_is_present` / `plugin_is_present`) and `CLAUDE_CONFIG_DIR`
handling stay as they are. Verified: marketplace add + plugin install work
with no login (fresh config dir, no keychain interaction) — restore-before-
sign-in ordering is safe.

Same non-destructive policy as Codex: never uninstall, never re-enable a
plugin the target person deliberately disabled, report version drift without
updating, same-name-different-source marketplace → blocked and surfaced
(spoofing guard — matters more cross-person than cross-machine).

### UI

Fold into what exists: the footer **Repair** action becomes lock-driven with
a missing-count badge like the Codex **Plugins** button (or unify both into
one **Plugins** action showing a combined plan — decide at implementation;
either is small).

## Explicitly Not Doing

- **No sync or merge of `installed_plugins.json` / `known_marketplaces.json`.**
  They stay capture *inputs*, machine-local forever.
- **No structured merge of `config.toml` or `settings.json`.** Tier 3
  conflict-copy stays. Cross-person, overwriting model/env/MCP/hook config
  would be a bug, not a feature. The lock carries the one subset (plugins)
  that should converge.
- **No plugins-only sharing channel.** Sharing a profile shares the Required
  tier too — transcripts, projects, history. Two people sharing a profile
  are sharing conversations, not just plugins; `settings.json` conflict
  copies also land their env values on each other's disks. If "share plugins
  with a teammate, nothing else" becomes a real need, that is a separate
  feature (a plugins-only profile or an exportable lock file), not a merge
  policy change. Flag it in the UI copy if this plan ships.

## Delivery

Roughly 1–2 days, mostly reuse:

1. **Capture + lock (0.5–1d)** — `claude_plugin_intent()` reading the three
   local files into the shared lock structs; `.claude/agent-sync/` allowlist
   entry; extend the pre-push hook to both roots; doc update in
   AGENT_SYNC_FILE_SETS.md (Tier 2 section already describes the driver).
2. **Replay + UI (0.5d)** — repair reads lock-with-fallback; badge.
3. **Tests (0.5d)** — capture from fixture settings/manager JSONs (incl.
   path-source → manual, no absolute path leaked); lock union propagates
   person A's plugin to person B's plan; repair ordering/skip/no-op reuse
   the existing FakeRunner patterns; fresh-root no-op stays green.

## Acceptance

- Person A installs `ponytail@ponytail`; A pushes; B pulls on a machine with
  their own differing `settings.json`; B's footer shows 1 missing plugin;
  one click installs it; B's own plugins and settings are untouched; second
  click is a no-op; A pulls and sees nothing to do.
- No absolute path from either machine appears anywhere in the cloud profile.
- Profiles without the new lock behave exactly as today.

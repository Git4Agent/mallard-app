# Plan: Claude Plugins Across Machines — Repair, Don't Copy

> **Historical and superseded.** The profile-file sync and repair path
> described here was removed with the old capability implementation. The
> replacement design stores provider plugin intent in a storage-scoped catalog:
> [`DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md`](./DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md).

Goal: after a normal pull on machine B, one click makes `ponytail` and the
other installed Claude plugins work — by driving Claude Code's own install
flow, not by copying plugin-manager workspaces.

## The decisive fact

`~/.claude/settings.json` — **already default-synced** — carries the
complete declarative install intent (verified on this machine):

```json
"enabledPlugins":        { "ponytail@ponytail": true, "ui-ux-pro-max@ui-ux-pro-max-skill": true },
"extraKnownMarketplaces": { "ponytail": { "source": { "source": "github", "repo": "DietrichGebert/ponytail" } }, … }
```

So machine B already receives *what to install and from where* through the
normal sync. Nothing else needs to sync for plugins to be restorable.

## What stays exactly as it is (deliberately)

- `plugins/marketplaces/**`, `plugins/repos/**`: **stay in the Never tier.**
  They are the plugin manager's fetch/update workspaces, not runtime
  payload. Syncing them minus `.git` (the `.git` denial is non-negotiable)
  would hand machine B corrupt-looking checkouts that confuse
  `marketplace update`/`remove`. They're also the widest trust surface —
  a file-sync layer shouldn't propagate executable source trees implicitly.
- `installed_plugins.json`, `known_marketplaces.json`: **not synced** —
  they're the manager's own mutable state with machine-local absolute
  paths; overwriting B's copies with A's would corrupt B's manager view.
  The earlier path-rewrite driver idea is **cancelled**: with settings.json
  as the intent source it solves a problem that no longer exists.
- Runtime payload arrives via `claude plugin install`, into B's own
  `plugins/cache` with B's own correct metadata.

## The feature: a "Repair plugins" action

One Tauri command + one button (shown for the `.claude` root, e.g. next to
Pull or in the profile box). CLI surface verified on this machine:

```
claude plugin marketplace add <url|path|github-repo>
claude plugin install <plugin>@<marketplace>
claude plugin list
```

Flow of `repair_plugins`:

1. Parse local `~/.claude/settings.json` → `enabledPlugins` +
   `extraKnownMarketplaces` (both optional; missing → nothing to do).
2. For each marketplace whose entry is absent from the local
   `known_marketplaces.json` (or whose `installLocation` is missing on
   disk) → `claude plugin marketplace add <source.repo>`.
3. For each enabled `plugin@marketplace` absent from the local
   `installed_plugins.json` (or whose `installPath` is missing on disk) →
   `claude plugin install <plugin>@<marketplace>`.
4. Stream child stdout/stderr into the existing sync log panel; finish with
   a per-item installed/skipped/failed summary. No retries.

Edges (the only real work):

- **Binary discovery**: GUI apps on macOS get a minimal `PATH`. Resolve
  `claude` via `$SHELL -lc "command -v claude"` with a fallback list
  (`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`).
- **Versions**: `install` fetches latest, not A's recorded version. Log it,
  don't fight it.
- Needs network + a logged-in `claude`. Failures are reported, not retried.
- Consent: the button is explicit user action — plugins execute arbitrary
  code, so repair must never run automatically after a pull.

Difficulty: low — roughly a day. Parsing two local JSONs + spawning a CLI;
no new sync semantics, no schema changes.

## Small cleanup in the same pass

Remove the `installed_plugins.json` / `known_marketplaces.json` rows from
the Optional-data catalog in SyncPanel — as opt-ins they're now a footgun
(see above), and the generic opt-in mechanism still honors any old configs
that enabled them. Keep a single optional row for `.claude/plugins/cache`
("one-pull offline plugin payload — plugins run with your privileges;
prefer Repair") for users who want plugins working without network.

## Out of scope

- Path-rewriting driver for manager JSONs — cancelled (see above).
- Un-Nevering `marketplaces/`/`repos/` — cancelled.
- Version pinning during repair; `blocklist.json`, `plugin-catalog-cache.json`,
  `plugins/data/` stay unlisted.

## Order

1. `repair_plugins` command in lib.rs (+ pure-fn tests for intent parsing
   and missing-detection against tempdir fixtures) → `cargo test`.
2. Button + log streaming in the UI → `npm run build`.
3. Catalog cleanup + AGENT_SYNC_FILE_SETS.md plugin-caveat update.

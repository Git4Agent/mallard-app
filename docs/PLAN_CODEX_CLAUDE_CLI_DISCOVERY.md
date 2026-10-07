# Codex and Claude CLI discovery plan

Status: implemented 2026-10-06; the live Tauri pass on macOS and Windows is open. Date: 2026-10-05.

Make Codex and Claude CLI selection work when Mallard starts from the desktop
or a terminal. Use one discovery policy for all CLI actions. Let each device
select an executable when automatic discovery cannot find its installation.

Implement environment discovery, executable selection, and provider launch as
separate operations. Use the standalone provider CLI. Keep desktop application
launch independent of CLI selection.

**0. Decisions (2026-10-05)**

| Question | Decision |
| --- | --- |
| Login shell or interactive login shell for Unix discovery? | Interactive login shell: `-i -l -c`, as VS Code runs it. A plain `-l` zsh reads `.zprofile` and `.zlogin` but not `.zshrc`, and `.zshrc` is where nvm and most version managers add their `PATH` entry. The marker, closed stdin, home working directory, and deadline make the interactive run safe. |
| May a terminal launch fall back to the bare program name when discovery fails? | No. One policy for every CLI action: a failure is a short error with the executable selector. The interactive probe sees the same `PATH` the user's terminal does, so the only launches this refuses are alias- or function-based ones, which the selector covers. Codex could not take the fallback anyway, because its resume version gate needs the executable. |
| The activity log records `$ <command>` with absolute paths for every launch. Keep it? | Yes. That `history.session_action` line exists so the log shows what ran, and it is machine-local. The "no full paths" rule in section 8 applies to discovery's own diagnostics and errors, not to that line. |
| Fish? | Not in this change. Fish and every other shell take the unsupported-shell path: inherited `PATH`, standard directories, selector. |

**1. Current behavior and problem**

| Operation | Codex | Claude |
| --- | --- | --- |
| Open a session in Terminal | Resolves an absolute CLI path before launch. | Sends the bare name `claude` to the terminal. |
| Check the CLI version before a terminal resume | Probes the resolved executable with `--version`. | No check. |
| Open a Library profile in Terminal | Uses the Codex resolver. | Sends the bare name `claude` to the terminal. |
| Library plugin commands | Uses `codex_binary::find_binary`. | Uses the same function with the name `claude`. |
| Delete a local session | Resolves Codex and sends `thread/delete`. | Uses filesystem operations; no CLI is required. |
| Rename a Codex session | Resolves Codex and sends a provider request. | No equivalent CLI operation is added by this plan. |
| Open the desktop application | Uses application discovery and a deep link; checks the CLI bundled in that application. | Uses application discovery and a deep link. |

The shared lookup checks inherited `PATH`, a Unix `-lc` shell probe, and three
Unix installation directories. The shell probe has no deadline or output cap.
It treats all stdout as a path. The lookup checks regular-file status but does
not check Unix execute permission. Windows npm handling is specific to Codex.
The `-l` probe misses `.zshrc`, so an nvm installation is invisible to a
desktop-launched Mallard unless the user also exports `PATH` from `.zshenv`.

Claude can therefore open in Terminal while a Library command fails to find
the same installation. Codex can work in a manually opened terminal while a
desktop-launched Mallard cannot find it.

Source: [current resolver](../crates/mallard-infrastructure/src/codex_binary.rs),
[Codex actions](../crates/mallard-infrastructure/src/providers/codex/actions.rs),
[Claude actions](../crates/mallard-infrastructure/src/providers/claude/actions.rs),
[Library runner](../src-tauri/src/storage_catalog/adapters.rs), and
[Claude deletion](../crates/mallard-infrastructure/src/providers/claude/deletion.rs).

**2. Evidence for the design**

| Reference | Relevant behavior | Decision for Mallard |
| --- | --- | --- |
| [VS Code shell discovery](https://github.com/microsoft/vscode/blob/main/src/vs/platform/shell/node/shellEnv.ts) | Runs the shell with `-i -l -c`, captures a marked result, caches it, and defaults to a 10-second deadline. | Bound and cache Unix shell discovery with the same arguments. |
| [VS Code Git discovery](https://github.com/microsoft/vscode/blob/main/extensions/git/src/git.ts) and [settings](https://github.com/microsoft/vscode/blob/main/extensions/git/package.json) | Validates executable candidates and provides a machine-scoped `git.path`. | Add a path setting for each provider on each device. |
| [Zed environment discovery](https://github.com/zed-industries/zed/blob/main/crates/project/src/environment.rs) | Uses an inherited CLI environment or captures a shell environment. | Preserve the environment selected before Mallard starts. Use machine scope for provider maintenance. |
| [IntelliJ Git settings](https://www.jetbrains.com/help/idea/settings-version-control-git.html) | Provides an executable selector and a Test action. | Make failed detection recoverable in Settings. |
| [Dugite](https://github.com/desktop/dugite/blob/main/lib/git-environment.ts) | Selects an embedded Git build and prepares its environment. | Bundling a provider is a separate release responsibility; it is outside this change. |

[OpenAI documents](https://learn.chatgpt.com/docs/codex/cli) standalone, npm,
and Homebrew installations. Its [standalone installer](https://releases.openai.com/codex/install.sh)
places a symlink at `~/.local/bin/codex` (or `$CODEX_INSTALL_DIR`) pointing
into `~/.codex/packages/standalone/releases/`, and adds that directory to the
shell profile. [Anthropic documents](https://code.claude.com/docs/en/setup)
native, Homebrew, WinGet, npm, and Linux package installations. Its npm
package installs the same native binary as the standalone installer and does
not invoke Node at runtime.

These are application design patterns. Platform rules still govern execution.
Zsh reads `.zshrc` only for interactive shells. Windows batch files require a
command interpreter. Recovering a shell environment and resolving an npm
launcher are separate from starting a native executable.
Sources: [Zsh](https://zsh.sourceforge.io/Doc/Release/Files.html) and
[Windows process creation](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw).

**3. Scope**

Cover Codex and Claude session terminal launch, Library terminal launch, and
Library plugin commands. Include existing Codex rename and delete requests and
the Codex resume version gate. Keep the bound `CODEX_HOME` or
`CLAUDE_CONFIG_DIR` explicit in each operation.

Support native macOS and Windows launches. Test generic discovery on Linux.
Linux terminal-window support remains outside this plan because Mallard's
current terminal launcher supports only macOS and Windows. Bash and Zsh are
the supported discovery shells; Fish is not in this change.

This change does not install or update provider software, edit shell files,
repair system links, select WSL automatically, or change desktop deep links.
It does not change Claude transcript validation, size limits, session deletion,
or the presentation of scan warnings. Those are separate concerns.

**4. Selection policy**

Use the following order for each provider:

1. A user-selected absolute executable path saved on this device.
2. Valid candidates in Mallard's inherited `PATH`, in directory order.
3. On Unix, valid candidates in the recovered shell `PATH`, in directory order.
4. A small set of documented installation directories.
5. A short error with access to the executable selector.

An invalid explicit selection stops discovery. It must not silently select a
different installation. In Automatic mode, skip missing links, directories,
and files that cannot execute. Once a valid provider installation is selected,
report a failed version or capability check against that installation.

Do not choose the newest installed version or scan version-manager directories.
Let the configured shell select its installation. Search only absolute `PATH`
entries; a project-local file must not become a CLI through an empty or relative
entry. Do not interpret aliases, shell functions, or shell command strings as
executable selections.

Spend the shell probe once per process, in a background thread right after
the window opens, and in discovery only where it can change the answer: not
for a saved selection, not when the inherited `PATH` names the launcher, and
not in a process a terminal started (`SHLVL` set), whose `PATH` is the
shell's already. The order above does not change when the probe is skipped.
This is what the editors do: VS Code warms the probe after its first window
opens and skips it for a CLI launch, JetBrains IDEs load it in a startup
coroutine and skip it under `SHLVL`, Zed loads it in the background unless
stdout is a terminal. None of them keeps the result across launches. (Added
2026-10-06.)

Use the Rust [which library](https://docs.rs/which/latest/which/) for ordinary
executable search with an explicit path list. Add it as a direct dependency.
Verified against 8.0.6: on Unix it checks `access(X_OK)`; on Windows it tries
every `PATHEXT` extension in one directory before the next, which preserves
directory precedence; `which_in_global` ignores the working directory but
leaves a relative `PATH` entry relative, so Mallard filters the list to
absolute entries before calling it. Keep provider package inspection separate
from this generic search.

**5. Platform environment discovery**

On Unix, use `SHELL` or the account's login shell. Run Bash and Zsh with
`-i -l -c`. Any other shell keeps inherited-path discovery and the executable
selector; do not guess its flags.

Run the shell in the user's home directory with
`MALLARD_RESOLVING_ENVIRONMENT=1` set, so a profile can skip work. Capture only
a marked `PATH` result, not the full environment. Use a random delimiter and
reject missing, duplicate, or malformed result blocks. Ignore startup text
outside the block. Close stdin. Apply a 10-second deadline and a 64 KiB stdout
cap. Reuse [bounded_process](../crates/mallard-infrastructure/src/bounded_process.rs),
which also kills the shell's process group on timeout.

Share an in-flight discovery request. Cache successful shell-path discovery
for the app process. Clear it when the user selects Refresh detection. Do not
cache failures. Resolve and validate the executable again for each
operation, so a provider update can change a launcher target.

On Windows, use inherited `PATH` and documented native installation locations.
Preserve path-directory precedence when checking executable and known npm
launcher candidates. Do not reconstruct the Windows environment from the
registry in this change; Refresh detection therefore cannot see a `PATH`
change made after Mallard started, and the Settings copy says so.

Standard-location checks include Unix user-local and Homebrew bin directories,
Claude's documented Windows user-local directory, and Windows npm's default
prefix. Derive user directories through platform APIs. Custom prefixes use
`PATH` or the explicit selector. Sources: [Claude setup](https://code.claude.com/docs/en/setup)
and [npm directory layout](https://docs.npmjs.com/cli/v11/configuring-npm/folders/).

**6. Provider executable handling**

For Codex, retain native executable resolution for a known `@openai/codex`
launcher. The npm launcher is `bin/codex.js` under that package; it spawns
`node_modules/@openai/codex-<platform>/vendor/<triple>/bin/codex` with the
inherited environment and makes no `PATH` edits of its own. Anchor package
inspection to that installation: the platform package nested under the
launcher's package or hoisted beside it, never an ancestor walk. Bound
metadata reads and verify package identity: the directory is
`@openai/codex-<platform>`, and its manifest's `name` is `@openai/codex`
(npm ships the platform in the version field, not the name). A missing
native package must report an incomplete installation. Do not use an
unrelated package or a JavaScript launcher as an automatic substitute.

For Claude, support current native standalone and package-manager
installations. The npm package's postinstall always places the native binary
at `<package>/bin/claude.exe`, on every platform. On Unix the npm launcher is
a symlink straight to that file, so canonicalizing the launcher is the whole
resolution. On Windows npm's cmd-shim execs it directly; resolve a known
`claude.cmd` to `<prefix>\node_modules\@anthropic-ai\claude-code\bin\claude.exe`
after verifying the package name. Never execute or parse batch code to find
the target.

Anthropic documents a native launcher at `~/.local/bin/claude` on Unix, a
symlink into `~/.local/share/claude/versions/`, and a native executable under
the Windows user's `.local/bin` directory. OpenAI's standalone launcher is the
same shape, with its target under `~/.codex/packages/`. Accept an
installer-managed symlink after target validation, including a target under
the provider's default home or inside an application bundle. Store the
selected launcher path, not its current version target. Resolve the target per
operation. Source: [Claude installation and updates](https://code.claude.com/docs/en/setup).

Use the installed package target to account for architecture and Linux libc.
Do not build a second provider installer or guess a target from Mallard's CPU
architecture alone. A native launch failure must identify the installation
problem. Obsolete JavaScript distributions are outside the supported path.

**7. One resolved launch per operation**

Return the selected launcher, validated absolute execution path, selection
source, and the search `PATH` discovery used. Keep this result local to the
operation.

Nothing needs that `PATH` to start: both native binaries run without it. It
exists for direct-process children, which inherit Mallard's own minimal
desktop environment: a plugin install that clones a marketplace needs `git`,
and an app-server request may spawn helpers. Give direct-process children the
discovery `PATH`; give terminal launches nothing extra, since the terminal's
shell has its own.

Pass the result to the terminal and direct-process adapters. Set the provider
home after the discovery environment, so shell configuration cannot replace
the bound home. Preserve the project directory for session resume. Preserve
the current default working directory for a Library profile launch.

Change Claude session and Library terminal actions to use the resolved absolute
executable. Keep `--resume <session-id>` and `CLAUDE_CONFIG_DIR`. Apply the same
selection to its Library plugin runner. Claude deletion must still work without
a CLI. Source: [Claude CLI arguments](https://code.claude.com/docs/en/cli-reference)
and [configuration directory](https://code.claude.com/docs/en/env-vars).

Keep Codex's resume version gate and one-request mutation behavior. A missing
executable or unsupported operation must not trigger a second mutation attempt.
Keep capability checks separate from discovery. Do not invent a shared minimum
version for Codex and Claude.

Project-opening guidance uses the explicit selection when one is saved and the
bare program name otherwise; the terminal that runs the pasted line resolves
the name. Generating guidance must not start shell discovery or a provider
process. `ProviderCliLauncher` and the descriptor's `cli_launcher` had no
consumer but a test; they are removed, and the descriptor carries `has_cli`
so Settings knows which providers get a row.

**8. Settings and diagnostics**

Add two optional settings: `codex_cli_path` and `claude_cli_path`. A missing
value means Automatic. Store them in existing machine-local app settings.
Accept an absolute file path, not arguments or shell syntax. Bound each UTF-8
path to 4096 bytes and reject control characters. Permit valid spaces and Unicode.

The current settings struct is `Copy` and has a 4 KiB write limit; the loader
reads with no bound. Remove the `Copy` assumption when adding owned paths.
Raise the document limit to 16 KiB and apply it to reads as well, with tests.
The two paths are not tuning values: `tuning_differs` ignores them, so a path
change never purges caches. Pass a settings snapshot through operation
context; avoid new mutable global path state. Update Rust and TypeScript DTOs
together.

For each provider, show Automatic or Selected, the detected path, and Choose
executable, Use automatic, Test, and Refresh detection controls. The version
appears only after Test; opening Settings never runs a provider process. Save
launcher paths exactly as selected. An unavailable CLI must not block
unrelated sync.

Test runs only `--version`, against temporary home and working directories.
Use a minimal environment without provider credentials. Set temporary `HOME`
and `USERPROFILE` where applicable, plus the provider home. Disable Claude
updates and nonessential traffic for this probe. Validate these isolation
rules on native platforms. The probe must not log in, open a session, or make
a model request. Source: [Claude update controls](https://code.claude.com/docs/en/env-vars).

Show short errors such as `Claude CLI not found`, `Selected Codex executable
is unavailable`, or `Could not read the shell PATH`. Local Settings may show
the selected path. Discovery's own diagnostics contain fixed failure codes and
safe version metadata, not full paths, environment contents, or captured
startup output. The existing `$ <command>` history line is unchanged. Adding
the discovery `PATH` to a command must not add it to that line.

The `mallard-cli` binary runs from a terminal and already has the user's
`PATH`; its tool lookups use inherited-path discovery and never the shell probe.

**9. Implementation sequence**

| Step | Files and responsibility | Completion condition |
| --- | --- | --- |
| 1 | Introduce neutral executable discovery in `crates/mallard-infrastructure`; reuse `bounded_process.rs`; add `which`. | Deterministic search and bounded shell fixtures pass. |
| 2 | Put Codex and Claude package handling in provider-specific modules. | Native and npm fixtures resolve the intended installed target. |
| 3 | Update `app_settings.rs`, `src/types.ts`, Settings UI, typed API, and Tauri command registration. | Both explicit selections persist locally and can be tested. |
| 4 | Update provider actions, Library adapters, and Codex app-server callers. Pass resolved launches through operation context. | Each provider's CLI actions use one selection policy. |
| 5 | Move generic `find_binary` consumers for Git, GitHub CLI, Node, and Wrangler to neutral discovery. The clone-tools cache in `cloud_project.rs` stores a failed `gh` lookup for the process; drop it, the shell-path cache makes a per-call lookup cheap. Remove the old `codex_binary` module after all callers move. | No duplicate resolver or compatibility re-export remains. |
| 6 | Verify native behavior and update user guidance. | The platform matrix below passes with temporary homes. |

Keep project-sync and storage-catalog independent. Both depend on infrastructure
discovery; neither imports the other's DTOs. Update tests that import the old
module. Do not attach Codex or Claude path overrides to unrelated tools.

**10. Verification and acceptance**

| Area | Required cases |
| --- | --- |
| Search | Minimal desktop `PATH`; multiple installed versions; explicit path precedence; broken links; non-executable files; spaces and Unicode; relative entries ignored. |
| Unix shells | Bash and Zsh with `-i -l -c`; startup text; invalid result blocks; timeout; excessive output; unsupported shell; selected version-manager path. Use temporary shell configuration. |
| Windows | Native `.exe`; known npm `.cmd` to native target; user-local installation; missing native package; no automatic WSL selection. |
| Provider updates | Retargeted Claude launcher; replaced package binary; stale cached environment; explicit refresh. Re-resolve between operations. |
| Codex flows | Session and Library terminal launch, plugin requests, rename, and delete use the selected executable and bound home. Keep the no-retry mutation assertion. |
| Claude flows | Session and Library terminal launch plus plugin requests use the selected executable and bound home. Deletion requires no CLI. |
| Settings | Both paths persist; invalid selection remains visible; document/path size limits on read and write; a path change purges no cache; no path enters portable manifests. |
| Isolation | Probes and tests cannot read or modify real provider homes. No model calls, credentials, or automatic installation. |

Run focused infrastructure tests first. Then run:

```sh
cargo test -p mallard-infrastructure --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
```

Verify complete Tauri launches on macOS and Windows with disposable profiles.
Capture the Settings result and terminal launch for review. Run Linux discovery
checks separately; they do not prove Linux terminal-window support.

The change is complete when both providers use the selected CLI consistently,
desktop startup finds supported installations, and a custom installation can be
selected without changing the user's shell or provider data.

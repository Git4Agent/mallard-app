# Plan: Windows Support — Paths, Commands, and Links

Status: implemented on `release-package-enhancement` (2026-09-21); Windows
runtime validation pending the first Windows release build
Related: [`PLAN_TRUSTED_DESKTOP_RELEASES.md`](./PLAN_TRUSTED_DESKTOP_RELEASES.md) §3.2,
[`PLAN_CLAUDE_IN_ROOT_PATH_REMAP.md`](./PLAN_CLAUDE_IN_ROOT_PATH_REMAP.md),
[Lossless path references](../formal/pathmap-origins-v1/README.md)

## 1. Problem

Conversation Push and Pull rewrite every spelling of the project root inside
provider transcripts into a portable reference (`/__mallard_root__/<tag>;`)
and back. The root grammar accepted only absolute POSIX paths:
`validate_pathmap_root` in `crates/mallard-infrastructure/src/session_path_map.rs`
rejected anything not starting with `/` or containing `\`. On Windows every
project root is drive-absolute (`C:\Users\x\proj`), so every Push and Pull
failed with *"Windows paths need their own design"*. Around it, seven
production sites read `HOME` (unset on Windows; `USERPROFILE` is the
equivalent), `fs::canonicalize` returns verbatim `\\?\C:\…` paths that would
have leaked into bindings, and the Codex item projection refused a Windows
`cwd` when building `file:` URIs.

Tauri makes the shell cross-platform; this plan makes the product's core
feature cross-platform.

## 2. Design

### 2.1 Root families, decided at runtime

A root belongs to one of two families, detected from the string:

| Family | Grammar | Example |
|---|---|---|
| POSIX | `/` + ≥2 clean components, as before | `/Users/x/proj` |
| Windows | `[A-Z]:\` + ≥2 clean components, `\` separators, no `/`, no `\\?\`, no `< > : " \| ? *` in components, no trailing `\` | `C:\Users\x\proj` |

The family is a property of the root, not of the build. A Mac decoding a
thread pushed from Windows sees Windows tags and renders them with its own
POSIX root; the reverse holds too. Compile-time `cfg(windows)` is used only
where the operating system API differs (home directory, canonical prefix).

Both families share the same limits (6–4096 bytes, no control or quote
bytes, no control-namespace collision), the same wire tokens, and the same
origins mechanism, so `mallard.pathmap.v1` and bundle schema 1 are unchanged.

### 2.2 Spellings and tags

The encoder replaces every boundary-valid occurrence of a root spelling.
POSIX roots keep the eight existing tags. Windows roots use:

| Tag | Spelling of `C:\Users\x\proj` | Where it appears |
|---|---|---|
| `b` / `c` | `C:\Users\x\proj` | JSON string values, Codex `cwd`, Claude `cwd` |
| `bs` / `cs` | `C:/Users/x/proj` | Node, Git, and tool output with forward slashes |
| `bj` / `cj` | `C:\\Users\\x\\proj` | JSON embedded in strings (Codex `arguments`, tool payloads) |
| `bx` / `cx` | `\\?\C:\Users\x\proj` | Output of canonicalising tools |
| `bu` / `cu` | `file:///C:/Users/x/proj` | File URIs |
| `be` / `ce` | `file:///C:/Users/x/proj` percent-encoded, `:` and `/` kept | Encoded file URIs |

`b…` is the binding root, `c…` the canonical alias, as before. Windows has
no tilde spelling; the shell-specific `~` is not stable across `cmd`,
PowerShell, and Git Bash.

Matching is exact-case on both families. NTFS ignores case, but a wire
reference cannot carry the source casing, and the encoder's gate requires
plain decoding (no origins) to restore the source byte for byte. A spelling
in different casing (`c:\users\x\proj`) therefore stays literal text: it is
never wrong, only not portable. Codex and Claude record `cwd` from the
process working directory, which carries the casing the user launched with,
and the binding root comes from the same folder picker, so they agree in
practice.

Decoding a tag whose spelling does not exist for the destination family
(`bt` on Windows, `bj` on POSIX) falls back to the absolute root, exactly as
the tilde tag already does when no home directory is known. The rest of the
path after the reference is literal text and keeps its source separators.
A thread pushed from Windows and pulled on a Mac therefore reads
`/Users/y/proj\src\main.rs`; pulled back to Windows it is exact. Rewriting
the remainder would require scanning text the codec deliberately never
scans, so cross-family display stays a documented limitation, not a bug.

### 2.3 Boundaries

A root match must be followed by a separator or end the value. For POSIX
that separator is `/` (unchanged; the formally checked grammar is not
touched). For Windows spellings it is `\` or `/`, so `C:\Users\x\proj2`
never matches root `C:\Users\x\proj`, while `C:\Users\x\proj\src` and
`C:/Users/x/proj/src` do.

### 2.4 Operating-system seams (`cfg(windows)`)

- **Home directory**: one helper, `home_dir_text()`, replaces every
  `std::env::var("HOME")` in production code. It uses `dirs::home_dir()`,
  which reads `HOME` on Unix (tests keep working) and `USERPROFILE` on
  Windows.
- **Canonical roots**: `fs_canonicalize` in
  `src-tauri/src/project_sync/service.rs` strips the `\\?\` verbatim prefix
  on Windows so `canonical_project_root` is a normal drive path. Local-state
  comparisons already lower-case canonical roots.
- **Codex `file:` URIs**: `path_to_file_uri` in
  `codex_paginated_projection.rs` accepts a Windows drive path and emits
  `file:///C:/…` with `:` and `/` unencoded.

### 2.5 Commands

Session actions used to be rendered as POSIX shell lines and handed to
macOS Terminal through `osascript`. A shell line is the wrong abstraction for
Windows, so `providers/launch.rs` now carries a structured
`TerminalCommand` (working directory, environment, program, arguments) and
renders it per platform:

| Platform | Rendering | Launch |
|---|---|---|
| macOS | `cd 'dir' && NAME='value' 'program' args…` | `osascript` → Terminal `do script` |
| Windows | `cd /d "dir" && set "NAME=value" && "program" args…` | `cmd.exe /K "<line>"` in a new console (`CREATE_NEW_CONSOLE`); Windows 11 routes it to the user's default terminal app |
| Other | — | explicit error |

The Windows renderer fails closed on `"`, `%`, `!`, `^`, and control bytes,
which `cmd` reinterprets even inside quotes; ampersands and spaces are safe
inside the quoted tokens. The rendered line is what the activity log records
on each platform.

Per action:

- **Open in Terminal** (Codex resume, Codex and Claude Library launches,
  Claude resume): works on Windows. The Codex CLI resolves through
  `codex_binary::find_binary`, which already handles `codex.exe` and the npm
  `codex.cmd` launcher without a shell; macOS keeps its login-shell lookup.
- **Open in Codex / Open in Claude**: remain macOS-only with a message that
  points at Open in Terminal. Both depend on `open --env` pinning the
  profile home into the desktop app; a bare `codex://`/`claude://` link on
  Windows would read the wrong home, and the Codex desktop app ships only
  for macOS.
- **Codex App refresh/restart**: `refresh_offer` returns no offer off macOS,
  so nothing is shown; there is no Codex desktop app to restart.

### 2.6 Links

- No production code creates symbolic links; the Claude project alias
  design was never shipped, and every `symlink(` call is in tests. Symlink
  *detection* (`symlink_metadata`, `is_symlink`) is cross-platform and also
  reports Windows junctions, so the no-follow guards hold on Windows.
- Hard-link checks were Unix-only (`nlink`). `persistence::hard_link_count`
  and `open_file_link_count` now answer on Windows through
  `GetFileInformationByHandle`, so shared Claude file-history snapshots are
  excluded, hard-linked project content is blocked, and lock files with
  extra links are refused on every platform.
- Unix permission hardening (`0600`/`0700`, `O_NOFOLLOW`) has no Windows
  equivalent in this code; `%USERPROFILE%` already grants only the user and
  administrators, and the no-symlink checks cover `O_NOFOLLOW`.

### 2.7 Unchanged

- Claude's project bucket name: `encode_claude_project_path` maps every
  non-alphanumeric UTF-16 unit to `-`, which already yields Claude Code's own
  `C--Users-x-proj` for `C:\Users\x\proj`.
- `portable_string_violation` already rejects drive letters and
  `%USERPROFILE%` in portable data.

## 3. Verification

- Unit tests in `session_path_map.rs`: Windows root grammar (accepts
  `C:\Users\x\proj`, rejects lowercase drive, `/` separators, `\\?\`,
  trailing `\`, single component, reserved characters); encode and decode
  round trip for every Windows spelling; exact-case matching and the
  `proj2` boundary; cross-family decoding
  falls back to the destination root; POSIX behaviour byte-for-byte as
  before.
- Unit tests in `providers/launch.rs` for both renderings and the `cmd`
  metacharacter rejections.
- `cargo test --workspace` on macOS; `cargo check --target x86_64-pc-windows-msvc`
  is not possible from macOS (C dependencies), so the Windows release dry
  run compiles and runs the CLI tests.
- Clean Windows 11 acceptance (from `PLAN_TRUSTED_DESKTOP_RELEASES.md`
  §3.2): bind `C:\Users\<user>\proj`, Push a Codex session and a Claude
  session, Pull them to a second Windows binding, confirm `cwd` and tool
  paths render under the new root, then Pull the same thread on a Mac and
  confirm it opens (with the documented separator limitation).

## 4. Out of scope

- Rewriting separators in the remainder of a path across families.
- UNC roots (`\\server\share\proj`) and mapped network drives.
- Opening the Codex or Claude desktop apps from Windows.
- Re-running the TLA+ models for the Windows grammar; the Windows family
  reuses the checked token and escape mechanics and only adds spellings. A
  future formal pass should add the Windows source-word alphabet.

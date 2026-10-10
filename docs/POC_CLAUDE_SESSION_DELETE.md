# Claude SDK session deletion POC

> Historical reference: The standalone deletion probe was removed on 2026-10-09.
> The run commands below no longer work in this checkout. Git history retains
> the probe; the results below describe its original run.

Date: 2026-09-26

The published TypeScript SDK can delete one local session and its session
directory. A single SDK call with a caught error supports the requested
best-effort behavior. The POC adds no deletion lock, digest gate, rollback,
or automatic retry.

## Method

- SDK: `@anthropic-ai/claude-agent-sdk@0.3.280`.
- Runtime: Node 22.22.0 on macOS.
- Entry point: `deleteSession(sessionId, { dir: projectDirectory })`.
- Every transcript was a synthetic fixture in a temporary provider home.
- No Claude model requests or real provider data were used.
- The SDK was installed outside the repository, with install scripts and
  optional dependencies disabled.
- Mallard bindings, Tauri commands, and UI were not changed or exercised.

The SDK version is pinned so a later package change cannot silently alter
the experiment. Each run creates a fresh home and removes its fixtures.

## Results

All six checks passed. Expected SDK errors count as a passed check when
the observed files and return value match the assertion.

| Check | Observed result |
| --- | --- |
| Root transcript | The selected JSONL was deleted. |
| Descendants and scope | The entire adjacent session directory was deleted, including child and grandchild fixtures, deeper directories, child metadata, and a tool-result file. Parent, sibling, independent fork, and the parent's child files were unchanged. A file-history sidecar outside the session directory remained. |
| Repeat delete | A second call returned `session_not_found`. |
| File added after a simulated preview | The added file was also deleted. The SDK does not accept a reviewed file list. |
| Partial failure | Removing directory write permission caused `EACCES`. The root was already deleted; the child remained. A repeat call returned `session_not_found` and left that child in place. |
| Open writer | On macOS, an open append handle could still write to the unlinked file. A simulated writer could then recreate the transcript path. |

The parent and nested-agent relationships are synthetic fixtures. This
proves the SDK's filesystem scope; it does not prove native Claude runtime
coordination or native resume behavior. The writer check uses a file handle,
not a running Claude process.

The SDK deletes the whole adjacent session directory. It does not select
only `.jsonl` files, and it does not remove all state elsewhere in the
provider home.

## Run

Install the SDK in a disposable directory, then pass its module path:

```sh
poc_root="$(mktemp -d)"
npm install --prefix "$poc_root" --ignore-scripts --omit=optional --no-audit --no-fund --package-lock=false @anthropic-ai/claude-agent-sdk@0.3.280
node scripts/run-claude-session-delete-poc.mjs "$poc_root/node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs"
```

The runner writes a sanitized report to
`output/claude-session-delete-poc/results.json`.

## Best-effort integration boundary

Call `deleteSession()` once for the selected root session and catch its
error. Return the deletion outcome to Mallard. Keep binding cleanup as a
separate result; this POC does not implement it.

A failed call can leave some files. A successful call does not prevent a
running writer from recreating them. These outcomes are accepted by the
requested best-effort scope.

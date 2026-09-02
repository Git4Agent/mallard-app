# Qoder custom-provider PoC

## Result

Qoder Agent SDK `1.0.32` exposes a `resolveModel` callback. The callback can
return a custom model with these fields:

- `provider`
- `model`
- `api_key`
- `url`
- `style`

This harness tests that contract with an OpenAI-compatible provider. It can
test Tuzi directly and then test the same endpoint through Qoder CLI.

The Qoder mode uses the SDK BYOK path. In the inspected CLI `1.1.40`, that path
sends a `custom_model` object, including the provider API key, to Qoder's
inference endpoint. Qoder receives the provider key on this path.

Qoder CLI `1.1.40` also contains a separate top-level `providers` setting for
OpenAI-compatible endpoints. The reviewed public Qoder settings documentation
does not define that schema. The repository report records an isolated parser
test, the smallest observed configuration, and its credential risks. The
harness uses the documented SDK model-policy interface for its Qoder mode.
It does not exercise the separate direct-provider transport.

The verdict is always `POC_ONLY`. A successful request does not enable
production Mallard support.

Use Node.js 20 or later. The harness uses built-in `fetch`, `Response`, Web
streams, and modern ECMAScript modules.

## Safety boundary

The harness applies these controls:

- It reads the provider key only from one explicit JSON credential file.
- It requires mode `0600` on the credential file on POSIX systems.
- It rejects symbolic links, non-canonical paths, unknown fields, non-HTTPS
  URLs, URL credentials, query strings, and fragments.
- It does not put either credential in command arguments, the child
  environment, normal output, or its result.
- It does not inherit the caller environment into Qoder CLI.
- It gives Qoder CLI a fresh `HOME`, `QODER_CONFIG_DIR`, temporary directory,
  and empty working directory.
- It supplies the Qoder PAT through the SDK one-shot authentication contract.
  It does not pass the PAT in the child environment.
- SDK `1.0.32` temporarily writes the one-shot authentication payload under
  the isolated temporary directory and removes it.
- It sets `tools: []` and sends the complete tool list observed in SDK
  `1.0.32` as a deny-list.
- It disables session persistence, file checkpoints, Skills, plugins, MCP,
  security scans, partial messages, debug logging, and prompt suggestions.
- It limits the Qoder run to one turn.
- It scans direct responses, SDK messages, SDK standard error, child output,
  and all regular files that remain under the harness-owned disposable root
  after the child exits.
- The scan detects literal UTF-8 credential bytes. It does not detect an
  encoded, encrypted, hashed, or split credential.
- For leak findings, it reports only counts and relative paths. It never
  reports matched bytes.
- It removes only the exact operating-system temporary root that it created.

The scan does not cover operating-system crash reports, system telemetry,
transformed secrets, or files outside the harness-owned root.

The credential file is caller-owned input. The harness does not delete it.
Delete it through a safe local secret process after the probe.

## Verify offline

The tests use mocked HTTP and a fake local SDK module. They make no external
request.

```sh
node --test poc/qoder-custom-provider/qoder-custom-provider.test.mjs
```

## Credential file

Create a dedicated temporary file. Do not use a general credentials file. Use
a restrictive `umask`, secure editor, or API that applies mode `0600` when it
creates the file. Do not write the key to a broad-permission file and then
change the mode.

```json
{
  "_type": "openai",
  "key": "<TUZI_API_KEY>",
  "url": "https://<TUZI_BASE_URL>"
}
```

For direct mode, the URL can end before or after `/v1`. Direct mode adds `/v1`
only when it is absent. Qoder mode forwards the normalized credential URL
unchanged through `resolveModel`.

## Direct Tuzi probe

This command makes two live requests:

1. `GET /v1/models`
2. `POST /v1/chat/completions`

The chat request uses one fixed prompt, `max_tokens: 8`, no stream, and the
explicit model.

```sh
node poc/qoder-custom-provider/qoder-custom-provider.mjs direct \
  --credentials /absolute/path/to/tuzi-qoder-poc.json \
  --model '<EXACT_MODEL_ID>'
```

The result includes HTTP status, response byte count, model count, and choice
count. It does not include model output or response bodies.

## Qoder SDK and CLI probe

This mode makes a live Qoder query. It requires all of these inputs:

- A Qoder personal access token in `MALLARD_QODER_PAT`.
- The absolute path to the installed SDK `dist/index.js` module.
- The absolute path to the installed Qoder CLI JavaScript bundle.
- An explicit model, provider, and style.

Set `MALLARD_QODER_PAT` with the shell or host secret mechanism. Do not put the
token in command history. Then run:

```sh
node poc/qoder-custom-provider/qoder-custom-provider.mjs qoder \
  --credentials /absolute/path/to/tuzi-qoder-poc.json \
  --model '<EXACT_MODEL_ID>' \
  --provider '<CONFIRMED_PROVIDER_ID>' \
  --style openai \
  --sdk-module /absolute/path/to/qoder-agent-sdk/dist/index.js \
  --qodercli /absolute/path/to/qodercli/bundle/qodercli.js
```

The `resolveModel` callback returns this in process memory:

```text
model: { model, provider, api_key, url, style }
```

The result reports callback count, message count, result-message count, and
tool-use-message count. It does not include the prompt response.

Remove `MALLARD_QODER_PAT` from the shell environment after the probe. Delete
the caller-owned credential file through a safe local secret process. Revoke
both disposable credentials.

Use mode `all` to run the direct probe first and the Qoder probe second. If the
direct probe fails, the Qoder probe does not run.

## Direct CLI provider observation

The tested CLI package parsed this version-specific configuration:

```json
{
  "providers": {
    "tuzi": {
      "baseUrl": "${TUZI_BASE_URL}",
      "apiKey": "${TUZI_API_KEY}",
      "model": "${TUZI_MODEL}"
    }
  }
}
```

The tested defaults are `openai-compatible`, OpenAI protocol, and Bearer
authentication. Select the model as `tuzi/<model-id>`. For this transport, use
a Tuzi base URL that ends in `/v1` because the CLI appends
`/chat/completions`.

The parser test used fake values and a clean Qoder home. It accepted the
provider configuration and then stopped because no Qoder login was present.
It did not make a provider request. See the detailed report before any live
test:

```text
docs/QODER_CUSTOM_PROVIDER_POC_2026-09-02.md
```

Do not put a real key in inline `--settings` JSON. Use a mode-`0600` settings
file with braced environment references. Treat this path as PoC evidence only.

## Interpretation

A direct success proves only that the Tuzi endpoint implements the two tested
OpenAI-compatible calls.

A Qoder success proves only that the tested Qoder SDK and CLI versions accepted
the `resolveModel` custom-model payload for one no-tool, non-persistent turn.
It does not prove these items:

- Qoder App support.
- Session restore or application-database projection.
- Every Qoder tool, agent, Skill, plugin, memory, image, or long-context flow.
- Every Tuzi model or endpoint.
- Retry, rate-limit, billing, quota, cancellation, or failure semantics.
- Stable Qoder BYOK behavior across versions.
- Production credential storage, audit, revocation, or rotation.

Do not pass a personal long-lived provider key or Qoder PAT. Use disposable,
scoped test credentials and revoke them after the live probe.

## Files

- `qoder-custom-provider.mjs` contains the dependency-free probe.
- `qoder-custom-provider.test.mjs` contains the offline tests.

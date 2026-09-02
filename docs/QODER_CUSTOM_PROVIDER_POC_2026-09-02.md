# Qoder CLI Custom Provider and Tuzi Proof of Concept

**Date:** 2 September 2026

**Repository:** Mallard

**Result:** `POC_ONLY`

**Production decision:** Do not add Qoder custom-provider configuration to
Mallard yet

**Scope:** Qoder CLI custom models, Qoder Agent SDK model routing, and one
minimal Tuzi OpenAI-compatible API test

**Out of scope:** Qoder App, session restore, pets, production credential
storage, account changes, and changes to the user's Qoder configuration

**Related research:**
[Qoder support feasibility research](QODER_SUPPORT_FEASIBILITY_RESEARCH_2026-09-01.md),
[Qoder session restore proof of concept](QODER_SESSION_RESTORE_POC_2026-09-02.md),
[Qoder CLI resume proof-of-concept study](QODER_CLI_RESUME_POC_2026-09-02.md),
and [Qoder fork mechanisms research and proof of concept](QODER_FORK_MECHANISMS_RESEARCH_2026-09-02.md)

## Executive result

Yes, Qoder CLI can use custom models. The result has three distinct paths:

1. Qoder documents a BYOK Custom wizard in `/model`. The wizard uses the live
   provider catalog for the current Qoder account. Qoder also documents custom
   endpoint support. This is the supported interactive path.
2. Qoder Agent SDK documents a `resolveModel` callback that can return a BYOK
   credential for each model request. The installed SDK also accepts a custom
   base URL. This is the supported programmatic path, subject to the account's
   BYOK permission and provider catalog.
3. Qoder CLI `1.1.40` contains a top-level `providers` setting for direct
   OpenAI-compatible providers. A disposable parser test accepted the
   Tuzi-shaped settings JSON and reached the Qoder authentication gate. Static
   inspection identified the model key `tuzi/<model>`. This path is not in the
   reviewed public Qoder settings documentation. Treat it as version-specific
   PoC evidence only.

Tuzi itself passed the live tests:

- The initial `GET /v1/models` returned HTTP `200` and 834 model identifiers.
- A 2 September 2026 recheck returned HTTP `200` and 836 model identifiers.
- The list contained `deepseek-chat`.
- `POST /v1/chat/completions` returned HTTP `200` and one non-empty choice.
- A separate minimal confirmation returned exactly `TUZI_OK` and used 13 total
  tokens.
- The reusable probe found no literal credential in the HTTP responses, its
  result, or the regular files that remained in its owned disposable root.

The model inventory is dynamic. The change from 834 to 836 identifiers is not
session-restore evidence.

The complete Qoder-to-Tuzi request did not run. Qoder requires valid Qoder
authentication before it starts a CLI or SDK model request. No disposable
Qoder personal access token was present. A fake-token test failed before the
SDK called `resolveModel`. A direct CLI test also parsed the provider
configuration and then stopped with the expected not-logged-in error.

The current result is:

| Question | Result | Evidence strength |
| --- | --- | --- |
| Does Qoder CLI support custom models? | Yes | Public Qoder documentation |
| Does Qoder support a custom endpoint? | Yes | Public Qoder release notes |
| Can the SDK supply a provider key for each request? | Yes | Public SDK documentation and installed types |
| Does CLI `1.1.40` parse a direct OpenAI-compatible provider? | Yes | Isolated installed-package test |
| Does Tuzi expose a working OpenAI-compatible API? | Yes | Live direct test |
| Does Tuzi offer `deepseek-chat` on this account? | Yes | Live model-list test |
| Can a Tuzi key replace Qoder login? | No | Public auth contract and two isolated auth-gate tests |
| Did Qoder complete a request through Tuzi? | No | The later authenticated native-resume run used Qoder's normal service path, not Tuzi |
| Is Mallard production support ready? | No | Qoder contract and credential-safety gates remain open |

Custom-provider support does not change the session-restore decision. It does
not rebuild Qoder App database projections, prove `active-leaf` semantics, or
make Qoder App tasks portable.

## 1. Decision boundary

This PoC answers four questions:

1. Does Qoder expose a supported custom-model interface?
2. Can the tested Qoder CLI parse an arbitrary OpenAI-compatible provider?
3. Does the available Tuzi configuration work against its API?
4. Can the current environment complete a Qoder-to-Tuzi request without using
   the user's real Qoder home?

The first three answers are yes. The fourth answer is no because no combined
authenticated Qoder-to-Tuzi product run was completed.

The PoC does not use the user's existing Qoder login. Mallard's repository
safety rules prohibit tests against a real provider home. Copying authentication
from a real home into a disposable home would also invalidate the credential
boundary.

## 2. Evidence classes

The report uses four evidence classes:

| Evidence class | What it proves | What it does not prove |
| --- | --- | --- |
| Public Qoder documentation | Supported BYOK, model, SDK, and authentication interfaces | Undocumented settings behavior |
| Installed package types and runtime | Behavior in CLI `1.1.40` and SDK `1.0.32` | A stable Qoder compatibility promise |
| Public Tuzi documentation | Bearer authentication and OpenAI-compatible endpoints | Compatibility with Qoder |
| Isolated live probe | The available Tuzi key, endpoint, and selected model work now | Long-term reliability or all model features |

The installed-package observations are diagnostic. Qoder can change them in a
later release without a compatibility period.

## 3. Tested environment

The relevant versions were:

- Qoder App `0.1.4`.
- `@qoder-ai/qodercli` `1.1.40`.
- `@qoder-ai/qoder-agent-sdk` `1.0.32`.
- Node.js `26.7.0`.
- npm `11.19.0`.
- macOS ARM64.

The package probes used isolated temporary package directories. npm lifecycle
scripts did not run. The test invoked the CLI JavaScript bundle directly. It
did not install a global command or change the user's shell path.

Node.js and npm were already available. Brew was not required.

## 4. Qoder's documented custom-model contract

### 4.1 Interactive Qoder CLI BYOK

The [Qoder CLI custom-model guide](https://docs.qoder.com/cli/custom-models)
states that Qoder CLI supports BYOK. The supported workflow is:

1. Start Qoder CLI.
2. Enter `/model`.
3. Open the **Custom** tab.
4. Select an available provider.
5. Select a model.
6. Enter the provider credential fields.
7. Save and select the model.

The same guide states that the live account catalog determines the available
providers, models, and credential fields. It tells users not to configure this
BYOK catalog manually in `settings.json`.

The current [Qoder CLI model guide](https://docs.qoder.com/cli/model) lists these
curated provider families:

- Alibaba Cloud Model Studio.
- DeepSeek.
- Z.ai.
- Kimi.
- MiniMax.
- Xiaomi MIMO.

Tuzi is not a named provider in that list. The list can change because the
custom-model guide makes the live account catalog authoritative.

Qoder CLI release notes for version `0.2.5` state that BYOK supports a custom
URL. See the
[Qoder CLI release notes](https://docs.qoder.com/release-notes/qoder-cli).
This permits an endpoint override in principle. It does not prove that the live
catalog accepts an arbitrary provider identifier.

### 4.2 Noninteractive model selection

The [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference)
documents `--model`. After the user saves a custom model, the CLI can select its
model key in a command.

A bounded noninteractive request must also use these controls:

```text
--print
--tools ""
--no-session-persistence
--permission-mode dont_ask
--max-turns 1
```

The PoC must disable tools through the applicable CLI or SDK interface. The
request must use an empty working directory and a disposable configuration
directory.

### 4.3 No documented generic provider environment contract

The reviewed
[Qoder CLI settings reference](https://docs.qoder.com/cli/settings-reference)
documents `QODER_PERSONAL_ACCESS_TOKEN`, `QODER_CONFIG_DIR`, `QODER_MODEL`, and
other Qoder variables. It does not document generic variables for an arbitrary
provider API key or base URL.

Therefore, these variables are not a Qoder interface by themselves:

```text
TUZI_API_KEY
TUZI_BASE_URL
OPENAI_API_KEY
OPENAI_BASE_URL
```

Qoder can consume variables if an approved configuration refers to them. The
variables alone do not configure a provider.

## 5. Qoder Agent SDK model-policy contract

The
[Qoder Agent SDK model-policy guide](https://docs.qoder.com/cli/sdk/model-policy)
documents two modes:

- Fixed mode uses a selected Qoder model.
- Dynamic mode calls `resolveModel` before each LLM request.

The callback can return a BYOK credential. Qoder then routes that request to a
third-party provider. The account must have BYOK permission. An invalid
provider, model, or credential fails the request. Dynamic mode has no automatic
fallback.

This SDK path is not the same as the CLI's direct `providers` transport. In the
inspected CLI `1.1.40`, the SDK result becomes a `custom_model` object in a
request to Qoder's inference endpoint. That object includes the provider API
key. Therefore, Qoder receives the provider key on this observed SDK path.

The public TypeScript reference documents the core shape:

```ts
interface CustomModel {
  provider: string;
  model: string;
  api_key: string;
  style?: string;
}
```

The installed SDK `1.0.32` type declaration adds these fields:

```ts
type CustomModel = {
  provider: string;
  api_key: string;
  model?: string;
  url?: string;
  style?: string;
  isVl?: boolean;
};
```

The installed type says that `style` defaults to `openai`. It describes `url`
as an optional provider base-URL override.

The PoC uses this return value:

```js
{
  model: {
    provider: "deepseek",
    model: "deepseek-chat",
    api_key: providerKey,
    url: providerBaseUrl,
    style: "openai"
  }
}
```

The `provider` value cannot be `tuzi` unless the live Qoder BYOK catalog returns
that exact key. The strongest candidate is `deepseek` because it is a documented
Qoder provider and the tested Tuzi account exposes `deepseek-chat`. This remains
an inference until an authenticated call to `listByokProviders()` confirms the
exact account catalog.

## 6. Installed Qoder CLI direct-provider path

### 6.1 Observed schema

Static inspection of Qoder CLI `1.1.40` found a separate, top-level `providers`
setting. It accepts a provider identifier and an OpenAI-compatible transport.

The smallest Tuzi-shaped configuration is:

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

In the tested package, omitted fields have these defaults:

```json
{
  "type": "openai-compatible",
  "protocol": "openai",
  "authType": "bearer"
}
```

Static inspection shows that the CLI synthesizes one model entry from the
singular `model` field. The resulting model key is:

```text
tuzi/<model-id>
```

For Tuzi, the direct-provider base URL must include `/v1` because this Qoder
transport appends `/chat/completions`.

An explicit configuration is clearer for a controlled test:

```json
{
  "providers": {
    "tuzi": {
      "type": "openai-compatible",
      "protocol": "openai",
      "authType": "bearer",
      "baseUrl": "${TUZI_BASE_URL}",
      "apiKey": "${TUZI_API_KEY}",
      "model": "${TUZI_MODEL}"
    }
  }
}
```

Use these environment values for the tested route:

```text
TUZI_BASE_URL=https://api.tu-zi.com/v1
TUZI_MODEL=deepseek-chat
```

Do not put the real provider key in documentation, command arguments, or the
settings file.

### 6.2 Parser and authentication-gate result

The isolated test used fake provider values and this command shape:

```sh
qodercli \
  --setting-sources= \
  --settings '<provider JSON>' \
  --list-models
```

The CLI accepted the `providers` settings shape. It then stopped with:

```text
Not logged in. Run qodercli login to authenticate.
```

The result proves that CLI `1.1.40` accepts the settings JSON and reaches the
authentication gate. Static inspection supplies the provider normalization and
model-key evidence. The runtime test did not select `tuzi/<model>` or complete a
Tuzi request.

The test used a disposable `HOME`, `QODER_CONFIG_DIR`, working directory, and
temporary directory. It used fake provider values. It removed the exact
temporary tree after inspection.

### 6.3 Support level

The public Qoder settings reference reviewed for this report does not document
the top-level `providers` schema. The public custom-model guide also tells users
to use the `/model` wizard instead of manual BYOK settings.

Therefore:

- Use `providers` only for an isolated PoC.
- Do not make Mallard depend on this schema.
- Do not capture or restore this setting.
- Require a public Qoder contract before production use.

## 7. Qoder authentication is separate

Qoder authentication and provider authentication have different purposes:

| Credential | Authenticates | Can replace the other credential? |
| --- | --- | --- |
| Valid Qoder authentication | Qoder CLI or Agent SDK runtime | No |
| Tuzi API key | Tuzi model endpoint | No |

The
[Qoder CLI authentication guide](https://docs.qoder.com/cli/authentication)
documents Qoder login and personal access token authentication. The Agent SDK
also requires an explicit authentication option. SDK `1.0.32` additionally
supports service-account and job-token flows. This PoC uses a PAT only. Browser
login and PAT are the two normal direct-CLI methods considered here.

### 7.1 SDK authentication-gate test

An isolated SDK test supplied a deliberately invalid Qoder token and a fake
provider credential. The result was:

```json
{
  "resolveModelCalls": 0,
  "resultSubtype": "error_during_execution",
  "terminalReason": "access_token_invalid",
  "credentialLeakCount": 0
}
```

Qoder rejected the Qoder token before it called `resolveModel`. This proves that
a valid Tuzi key cannot bypass Qoder authentication.

### 7.2 Direct CLI authentication-gate test

The direct CLI provider parser used a clean Qoder home and no Qoder token. It
accepted the provider configuration and returned the not-logged-in error before
any Tuzi request.

Both paths stopped at Qoder authentication. A later isolated native-resume test
authenticated Qoder and passed, but that runner excluded the Tuzi credential
and used Qoder's normal service path. Therefore, it does not close this
provider-routing gate.

## 8. Tuzi API contract

The [Tuzi OpenAI-compatible API guide](https://api.tu-zi.com/docs/api/openai)
documents Bearer authentication and these relevant endpoints:

```text
GET  /v1/models
POST /v1/chat/completions
```

The chat response uses the usual OpenAI-compatible fields, including
`choices[0].message.content`, `finish_reason`, and `usage`.

The provider configuration available to this PoC has this sanitized shape:

```json
{
  "_type": "newapi_channel_conn",
  "key": "<present; not displayed>",
  "url": "https://api.tu-zi.com"
}
```

The source file had mode `0600`. The PoC did not display, log, or commit the
key. It loaded only the three expected fields.

## 9. Live Tuzi results

### 9.1 Initial protocol confirmation

The first minimal test did this:

1. Requested `/v1/models`.
2. Confirmed `deepseek-chat` was present.
3. Sent one chat-completions request.
4. Requested the exact response `TUZI_OK`.

The result was:

| Field | Result |
| --- | --- |
| Models HTTP status | `200` |
| Model count | `834` |
| Selected model present | Yes |
| Chat HTTP status | `200` |
| Returned provider model | `deepseek-v3` |
| Finish reason | `stop` |
| Exact response match | Yes |
| Prompt tokens | `10` |
| Completion tokens | `3` |
| Total tokens | `13` |
| Elapsed time | `1,795 ms` |

The report intentionally omits the provider request identifier because it can
correlate the test with an account record.

### 9.2 Reusable harness confirmation

The repository PoC harness repeated the direct test with a fixed no-tool prompt
and `max_tokens: 8`. It reports counts and status values only.

```json
{
  "schemaVersion": 1,
  "productionReady": false,
  "verdict": "POC_ONLY",
  "qoderAppSupport": "UNPROVED",
  "providerCompatibility": "REQUIRES_LIVE_PROBE",
  "ok": true,
  "mode": "direct",
  "operation": {
    "direct": {
      "modelsRequest": {
        "httpStatus": 200,
        "responseByteCount": 127322,
        "modelCount": 834,
        "selectedModelPresent": true
      },
      "chatRequest": {
        "httpStatus": 200,
        "responseByteCount": 368,
        "choiceCount": 1,
        "contentChoiceCount": 1
      }
    }
  },
  "outputScan": {
    "complete": true,
    "fileCount": 0,
    "byteCount": 0,
    "unsafeEntryCount": 0,
    "leakMatchCount": 0,
    "leakPathCount": 0,
    "leakPaths": [],
    "pathListTruncated": false
  },
  "cleanup": {
    "attempted": true,
    "complete": true
  }
}
```

This result proves only the tested model-list and chat-completions operations.
It does not prove tools, images, long context, streaming, retries, or all Tuzi
models.

### 9.3 Session-gate recheck

The session-restore review reran the reusable direct probe on 2 September 2026.
It used the existing mode-`0600` credential file without displaying its values.

The sanitized result summary is:

```json
{
  "productionReady": false,
  "verdict": "POC_ONLY",
  "ok": true,
  "mode": "direct",
  "modelsRequest": {
    "httpStatus": 200,
    "responseByteCount": 127627,
    "modelCount": 836,
    "selectedModelPresent": true
  },
  "chatRequest": {
    "httpStatus": 200,
    "responseByteCount": 368,
    "choiceCount": 1,
    "contentChoiceCount": 1
  },
  "leakMatchCount": 0,
  "cleanupComplete": true
}
```

This recheck proves that the direct Tuzi prerequisite is currently available.
It does not prove a Qoder-to-Tuzi route. Qoder authentication is separate from
Tuzi authentication and is required for semantic continuation and provider
routing. Normal headless CLI can still resolve a source while it defers the
login error. The current environment did not provide a disposable Qoder PAT.
Therefore, no Qoder CLI session-resume gate changes from blocked to pass.

## 10. Credential and log analysis

### 10.1 Safe configuration form

Use a mode-`0600` settings file that contains environment placeholders:

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

Use the braced `${NAME}` form. In the inspected package, an unresolved braced
reference fails provider validation. A bare `$NAME` reference does not provide
the same failure behavior.

Do not put a real key in inline `--settings` JSON. The key would be visible in
the process arguments. The tested CLI also materializes inline settings in a
temporary file with ordinary file-creation defaults.

### 10.2 Direct-provider logs

The inspected direct-provider request path logs provider, model, protocol, and
request endpoint information. The reviewed normal path did not explicitly log
the authorization header or full key.

This is not a complete non-disclosure proof. The redaction logic uses exact
string matching. A provider error can transform, encode, split, or partially
echo a key. Do not put a secret in a base-URL path, query string, or fragment.

### 10.3 Legacy custom-model path

The installed CLI also contains a legacy `modelConfigs.customModels` path. The
inspected validation logic sends the provider key to a Qoder BYOK validation
endpoint. One diagnostic formatter can include the first eight key characters.

Do not use this legacy path for the Mallard PoC. Do not treat its current
redaction behavior as safe for production credentials.

This finding is package-specific. It is not a statement about all Qoder
versions or every operation of the current `/model` wizard.

The SDK `resolveModel` path also sends the full provider key to Qoder as part of
the observed `custom_model` inference request. It does not use the same local
transport as the direct `providers` path. This behavior is a production
security decision, not an unresolved implementation detail.

### 10.4 Mallard credential boundary

Mallard must apply these rules if this work continues:

- Never put a Qoder PAT or provider key in a portable manifest.
- Never capture complete Qoder settings.
- Never write a provider key to an activity log, receipt, restore plan, or test
  fixture.
- Never use a real provider home for tests.
- Use disposable, scoped test credentials.
- Revoke test credentials after the live proof.
- Store provider intent separately from provider secrets.
- Make secret injection a local runtime operation.

## 11. Reusable PoC harness

The PoC is in:

```text
poc/qoder-custom-provider/qoder-custom-provider.mjs
poc/qoder-custom-provider/qoder-custom-provider.test.mjs
poc/qoder-custom-provider/README.md
```

The harness has three modes:

- `direct` tests Tuzi without Qoder.
- `qoder` tests the SDK `resolveModel` path through the supplied Qoder CLI
  runtime.
- `all` runs the direct test first and the Qoder test second.

The Qoder mode requires a separate `MALLARD_QODER_PAT`. It does not read the
user's Qoder home or inherit the parent environment.

### 11.1 Harness safety controls

The harness:

- Reads one explicit, canonical credential file.
- Requires mode `0600` on POSIX systems.
- Rejects symbolic links and unknown fields.
- Requires HTTPS.
- Rejects URL credentials, query strings, and fragments.
- Uses bounded HTTP and child-process output.
- Gives Qoder a fresh `HOME` and `QODER_CONFIG_DIR`.
- Gives Qoder an empty working directory.
- Sets `tools: []` and sends the complete tool list observed in SDK `1.0.32`
  as a deny-list.
- Disables session persistence and file checkpoints.
- Disables Skills, plugins, MCP, security scans, partial messages, and debug
  output.
- Limits the request to one turn.
- Scans HTTP responses, SDK messages, standard error, child output, and all
  regular files that remain under the harness-owned disposable root after the
  child exits.
- For leak findings, reports only counts and relative paths.
- Removes only the exact temporary root that it created.

The literal scan cannot detect an encoded, encrypted, hashed, or split secret.
It does not cover operating-system crash reports, system telemetry, transformed
secrets, or files outside the harness-owned root. These limits are explicit.

SDK `1.0.32` temporarily writes its one-shot, mode-`0600` authentication
payload under the isolated temporary directory. The SDK removes that file. The
harness scans remaining files and removes the exact disposable root.

### 11.2 Offline verification

Run:

```sh
node --check poc/qoder-custom-provider/qoder-custom-provider.mjs
node --check poc/qoder-custom-provider/qoder-custom-provider.test.mjs
node --test poc/qoder-custom-provider/qoder-custom-provider.test.mjs
```

The result is 14 passed and 0 failed. The tests use mocked HTTP and a fake SDK.
They make no network request.

### 11.3 Direct live mode

Use a dedicated credential file. Create it with a restrictive `umask`, secure
editor, or API that applies mode `0600` at creation time. Do not create it with
broad permissions and correct the mode afterward.

```json
{
  "_type": "newapi_channel_conn",
  "key": "<TUZI_API_KEY>",
  "url": "https://api.tu-zi.com"
}
```

Run:

```sh
node poc/qoder-custom-provider/qoder-custom-provider.mjs direct \
  --credentials /absolute/path/to/tuzi-qoder-poc.json \
  --model deepseek-chat
```

The harness accepts a root URL or a URL that ends in `/v1` for its direct
mode. It adds `/v1` only when necessary.

### 11.4 Authenticated Qoder mode

Do not paste a Qoder PAT into a chat, issue, command argument, or document. Use
a shell or host secret-input mechanism that makes a disposable PAT available
as `MALLARD_QODER_PAT` without putting its value in command history. Then run
the SDK path:

```sh
node poc/qoder-custom-provider/qoder-custom-provider.mjs qoder \
  --credentials /absolute/path/to/tuzi-qoder-poc.json \
  --model deepseek-chat \
  --provider deepseek \
  --style openai \
  --sdk-module /absolute/path/to/qoder-agent-sdk/dist/index.js \
  --qodercli /absolute/path/to/qodercli/bundle/qodercli.js
```

Run this only with a Qoder account that has BYOK permission. First use the SDK
to call `listByokProviders()`. Confirm the exact provider and model keys. Do not
assume that `deepseek` is available only because the public model guide lists
it.

After the probe, remove `MALLARD_QODER_PAT` from the shell environment. Delete
the caller-owned credential file through a safe local secret process. Revoke
both disposable credentials.

## 12. Manual direct CLI command sketch

The direct CLI path requires a valid disposable Qoder login and a protected
settings file. The command below shows the required CLI options. It is not a
safe wrapper. It does not create an isolated environment, scan output, or clean
temporary files by itself.

```sh
qodercli \
  --setting-sources= \
  --settings /absolute/path/to/settings.json \
  --model "tuzi/${TUZI_MODEL}" \
  --print "Reply with exactly: TUZI_OK" \
  --tools "" \
  --no-session-persistence \
  --permission-mode dont_ask \
  --max-turns 1
```

Do not run the sketch until a wrapper implements these controls:

1. Create a fresh `HOME`.
2. Create a fresh `QODER_CONFIG_DIR`.
3. Create an empty working directory.
4. Put only placeholder references in `settings.json`.
5. Set `settings.json` to mode `0600`.
6. Set `TUZI_BASE_URL` to `https://api.tu-zi.com/v1`.
7. Set `TUZI_MODEL` to `deepseek-chat`.
8. Inject a disposable Tuzi key without putting it in command arguments.
9. Authenticate Qoder with a disposable Qoder account. For this direct CLI
   experiment, use `QODER_PERSONAL_ACCESS_TOKEN` or a login that belongs only
   to the disposable Qoder home.
10. Disable all project and user setting sources.
11. Scan output and all regular files that remain in the disposable root for
    both literal credentials.
12. Remove the exact disposable root.
13. Revoke both credentials.

The test must fail closed if Qoder writes either credential to a log or output
file.

## 13. Mallard feasibility

Qoder custom-provider support is feasible as a local execution feature. It is
not a portable provider-home resource.

Mallard must not sync these items:

- Provider API keys.
- Qoder personal access tokens.
- Complete Qoder settings files.
- Provider endpoint paths that contain credentials.
- Provider validation logs.
- Provider-specific temporary settings files.

Mallard can consider a future non-secret intent object:

```json
{
  "provider_id": "tuzi",
  "protocol": "openai",
  "model": "deepseek-chat",
  "credential_reference": null
}
```

This object must not contain the API key. A destination machine must bind a
local secret before use. This is a separate product capability from Qoder
session, Skill, or plugin portability.

Do not add this intent object to schema 1 now. Qoder does not document the
direct `providers` schema, and the authenticated end-to-end test is incomplete.

## 14. Production gates

Keep production custom-provider support disabled until all gates pass:

1. Qoder documents the direct-provider configuration or confirms its support
   contract.
2. A disposable Qoder account has BYOK permission.
3. `listByokProviders()` confirms the required provider and model keys for the
   SDK route.
4. A clean-home Qoder CLI completes one no-tool Tuzi request.
5. Qoder sends the request only to the expected Tuzi endpoint and required
   Qoder control endpoints.
6. No Qoder or Tuzi credential appears in arguments, output, logs, transcripts,
   temporary files, or crash records.
7. Cancellation, timeout, HTTP error, quota, and invalid-key tests also pass.
8. The test passes again after a Qoder CLI upgrade.
9. Qoder documents credential handling for each path. The installed SDK path
   sends the provider key to Qoder. A security review must explicitly accept or
   reject that behavior.
10. Mallard has a machine-local secret binding design with no portable secret
    field.

These gates do not replace the separate Qoder session-restore gates.

## 15. Recommended next test

The next safe action is small:

1. Create or select a disposable Qoder test account.
2. Confirm that its plan permits BYOK.
3. For the reusable SDK harness, supply a disposable Qoder PAT only through
   `MALLARD_QODER_PAT`. For the separate direct CLI experiment, use
   `QODER_PERSONAL_ACCESS_TOKEN` or a login in the disposable Qoder home. Do
   not paste either token into chat or command history.
4. Call `listByokProviders()` first.
5. If the catalog contains `deepseek` and `deepseek-chat`, run the SDK harness
   once.
6. Run the direct CLI `providers` path once as a separate experiment.
7. Compare the destination host, request shape, result, and sanitized logs.
8. Revoke the disposable Qoder and Tuzi credentials.

If a disposable PAT is not available, an interactive login in the disposable
Qoder home is possible only with explicit user authorization. Do not reuse or
copy the current Qoder App login.

## 16. Official sources

### Qoder

- [Qoder CLI custom models](https://docs.qoder.com/cli/custom-models)
- [Qoder CLI models](https://docs.qoder.com/cli/model)
- [Qoder CLI release notes](https://docs.qoder.com/release-notes/qoder-cli)
- [Qoder CLI command reference](https://docs.qoder.com/cli/cli-reference)
- [Qoder CLI settings reference](https://docs.qoder.com/cli/settings-reference)
- [Qoder CLI authentication](https://docs.qoder.com/cli/authentication)
- [Qoder CLI noninteractive use](https://docs.qoder.com/cli/run-in-scripts)
- [Qoder Agent SDK model policy](https://docs.qoder.com/cli/sdk/model-policy)
- [Qoder Agent SDK TypeScript reference](https://docs.qoder.com/cli/sdk/references-typescript)

### Tuzi

- [Tuzi OpenAI-compatible API guide](https://api.tu-zi.com/docs/api/openai)
- [Tuzi API getting started guide](https://api.tu-zi.com/docs/use-cases/getting-started)
- [Tuzi endpoint matrix](https://api.tu-zi.com/en/docs/use-cases/endpoint-matrix)

## Final decision

Qoder CLI can use custom models, and Tuzi is a working OpenAI-compatible
endpoint for the tested model. Qoder CLI `1.1.40` also parses a direct Tuzi
provider configuration. However, Qoder authentication runs before the model
request. No combined authenticated Qoder-to-Tuzi product run was completed, so
the end-to-end proof is incomplete. A later authenticated native resume does
not change this result because it did not use Tuzi.

Keep this result at `POC_ONLY`. Do not add a production Mallard schema,
credential path, capture filter, or restore action for Qoder custom providers.
Complete the clean-home authenticated Qoder-to-Tuzi test and obtain a
documented Qoder provider contract first.

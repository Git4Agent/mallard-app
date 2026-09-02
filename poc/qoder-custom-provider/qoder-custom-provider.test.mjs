import assert from "node:assert/strict";
import {
  chmod,
  lstat,
  mkdtemp,
  realpath,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  POC_STATUS,
  buildIsolatedEnvironment,
  createModelPolicyResult,
  executeProbe,
  readCredentialFile,
  runDirectProbe,
} from "./qoder-custom-provider.mjs";

const PROVIDER_KEY = "tuzi-test-key-123456789";
const QODER_PAT = "qoder-test-pat-123456789";

async function makeFixture() {
  const createdRoot = await mkdtemp(
    join(tmpdir(), "mallard-qoder-provider-test-"),
  );
  const root = await realpath(createdRoot);
  const credentialFile = join(root, "credential.json");
  await writeFile(
    credentialFile,
    JSON.stringify({
      _type: "openai",
      key: PROVIDER_KEY,
      url: "https://tuzi.invalid/api",
    }),
    { mode: 0o600 },
  );
  await chmod(credentialFile, 0o600);
  return { root, credentialFile };
}

async function removeFixture(root) {
  await rm(root, { recursive: true, force: false });
}

function response(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function successfulFetch(requests) {
  return async (url, options) => {
    requests.push({
      url,
      method: options.method,
      authorizationCorrect:
        options.headers.Authorization === `Bearer ${PROVIDER_KEY}`,
      body: options.body ? JSON.parse(options.body) : undefined,
    });
    if (url.endsWith("/v1/models")) {
      return response({ data: [{ id: "test-model" }, { id: "other-model" }] });
    }
    if (url.endsWith("/v1/chat/completions")) {
      return response({
        choices: [{ message: { role: "assistant", content: "OK" } }],
      });
    }
    throw new Error("unexpected mock URL");
  };
}

async function writeFakeSdk(root, { leak = false } = {}) {
  const sdk = join(root, leak ? "fake-sdk-leak.mjs" : "fake-sdk.mjs");
  const qoderCli = join(root, "fake-qodercli.js");
  await writeFile(
    sdk,
    `
import { readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

export function accessToken(token) {
  if (token !== ${JSON.stringify(QODER_PAT)}) throw new Error("bad PAT");
  return { type: "accessToken", accessToken: "accepted" };
}

export function query({ prompt, options }) {
  return (async function* () {
    if (prompt !== "Reply with exactly OK. Do not use tools.") throw new Error("bad prompt");
    if (options.persistSession !== false || options.debug !== false) throw new Error("unsafe flags");
    if (options.debugFile !== undefined || options.enableFileCheckpointing !== false) throw new Error("unsafe storage flags");
    if (!Array.isArray(options.tools) || options.tools.length !== 0) throw new Error("tools enabled");
    if (!Array.isArray(options.disallowedTools) || options.disallowedTools.length < 20) throw new Error("deny list missing");
    if (options.maxTurns !== 1 || options.promptSuggestions !== false) throw new Error("unbounded query");
    if (options.settingSources.length !== 0 || options.skills.length !== 0 || options.plugins.length !== 0) throw new Error("provider state enabled");
    if (Object.keys(options.mcpServers).length !== 0 || options.allowedMcpServerNames.length !== 0) throw new Error("MCP enabled");
    if (Object.values(options.securityScan).some(Boolean)) throw new Error("security tools enabled");
    if (Object.keys(options.env).some((key) => key.startsWith("AWS_"))) throw new Error("broad env");
    if (Object.values(options.env).includes(${JSON.stringify(PROVIDER_KEY)}) || Object.values(options.env).includes(${JSON.stringify(QODER_PAT)})) throw new Error("credential in env");
    if ((await readdir(options.cwd)).length !== 0) throw new Error("cwd not empty");
    if ((await readdir(options.env.QODER_CONFIG_DIR)).length !== 0) throw new Error("config not empty");
    const selected = await options.resolveModel({ purpose: "main", sessionId: "fake", turnIndex: 0, availableModels: [] });
    if (selected.model.model !== "test-model") throw new Error("bad model");
    if (selected.model.provider !== "openai") throw new Error("bad provider");
    if (selected.model.url !== "https://tuzi.invalid/api") throw new Error("bad URL");
    if (selected.model.style !== "openai") throw new Error("bad style");
    ${leak ? "await writeFile(join(options.env.QODER_CONFIG_DIR, \"leak.txt\"), selected.model.api_key);" : "await writeFile(join(options.env.QODER_CONFIG_DIR, \"safe.txt\"), \"safe output\");"}
    yield { type: "system", subtype: "init" };
    yield { type: "result", subtype: "success", result: "OK" };
  })();
}
`,
    { mode: 0o600 },
  );
  await writeFile(qoderCli, "// fake bundle; the fake SDK does not execute it\n", {
    mode: 0o600,
  });
  return { sdk, qoderCli };
}

test("reads one canonical credential file with exact fields", async () => {
  const fixture = await makeFixture();
  try {
    const credential = await readCredentialFile(fixture.credentialFile);
    assert.equal(credential.type, "openai");
    assert.equal(credential.key, PROVIDER_KEY);
    assert.equal(credential.url, "https://tuzi.invalid/api");
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects unknown credential fields", async () => {
  const fixture = await makeFixture();
  try {
    await writeFile(
      fixture.credentialFile,
      JSON.stringify({
        _type: "openai",
        key: PROVIDER_KEY,
        url: "https://tuzi.invalid",
        note: "not allowed",
      }),
      { mode: 0o600 },
    );
    await assert.rejects(
      readCredentialFile(fixture.credentialFile),
      (error) => error.code === "invalid_credential_fields",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects a symbolic-link credential file", async () => {
  const fixture = await makeFixture();
  try {
    const link = join(fixture.root, "credential-link.json");
    await symlink(fixture.credentialFile, link);
    await assert.rejects(
      readCredentialFile(link),
      (error) => error.code === "credential_file_must_be_regular_canonical_file",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects broad credential-file permissions", async () => {
  const fixture = await makeFixture();
  try {
    await chmod(fixture.credentialFile, 0o644);
    await assert.rejects(
      readCredentialFile(fixture.credentialFile),
      (error) => error.code === "credential_file_permissions_too_broad",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects a non-HTTPS provider URL", async () => {
  const fixture = await makeFixture();
  try {
    await writeFile(
      fixture.credentialFile,
      JSON.stringify({
        _type: "openai",
        key: PROVIDER_KEY,
        url: "http://tuzi.invalid",
      }),
      { mode: 0o600 },
    );
    await assert.rejects(
      readCredentialFile(fixture.credentialFile),
      (error) => error.code === "url_requires_https",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("builds the documented resolveModel custom-model shape", () => {
  const result = createModelPolicyResult({
    credential: {
      type: "tuzi",
      key: PROVIDER_KEY,
      url: "https://tuzi.invalid/v1",
    },
    model: "test-model",
    provider: "openai",
    style: "openai",
  });
  assert.deepEqual(result, {
    model: {
      model: "test-model",
      provider: "openai",
      api_key: PROVIDER_KEY,
      url: "https://tuzi.invalid/v1",
      style: "openai",
    },
  });
});

test("uses models and chat endpoints without returning response content", async () => {
  const requests = [];
  const result = await runDirectProbe({
    credential: {
      type: "openai",
      key: PROVIDER_KEY,
      url: "https://tuzi.invalid/api",
    },
    model: "test-model",
    fetchImpl: successfulFetch(requests),
  });
  assert.equal(requests.length, 2);
  assert.equal(requests[0].url, "https://tuzi.invalid/api/v1/models");
  assert.equal(requests[0].method, "GET");
  assert.equal(requests[0].authorizationCorrect, true);
  assert.equal(requests[1].url, "https://tuzi.invalid/api/v1/chat/completions");
  assert.equal(requests[1].method, "POST");
  assert.equal(requests[1].authorizationCorrect, true);
  assert.equal(requests[1].body.model, "test-model");
  assert.equal(requests[1].body.stream, false);
  assert.equal(result.modelsRequest.modelCount, 2);
  assert.equal(result.modelsRequest.selectedModelPresent, true);
  assert.equal(result.chatRequest.choiceCount, 1);
  assert.equal(JSON.stringify(result).includes("OK"), false);
  assert.equal(JSON.stringify(result).includes(PROVIDER_KEY), false);
});

test("does not return an HTTP error body that contains a credential", async () => {
  const fixture = await makeFixture();
  try {
    const result = await executeProbe(
      {
        mode: "direct",
        credentialFile: fixture.credentialFile,
        model: "test-model",
      },
      {
        fetchImpl: async () => response({ error: PROVIDER_KEY }, 401),
      },
    );
    assert.equal(result.ok, false);
    assert.equal(result.error.code, "credential_leak_detected");
    assert.equal(result.outputScan.leakPathCount, 1);
    assert.deepEqual(result.outputScan.leakPaths, ["$direct/models-response"]);
    assert.equal(JSON.stringify(result).includes(PROVIDER_KEY), false);
  } finally {
    await removeFixture(fixture.root);
  }
});

test("runs the direct probe in a disposable root and removes that root", async () => {
  const fixture = await makeFixture();
  let disposableRoot;
  try {
    const result = await executeProbe(
      {
        mode: "direct",
        credentialFile: fixture.credentialFile,
        model: "test-model",
      },
      {
        fetchImpl: successfulFetch([]),
        onTempRoot: (root) => {
          disposableRoot = root;
        },
      },
    );
    assert.equal(result.ok, true);
    assert.equal(result.outputScan.complete, true);
    assert.equal(result.outputScan.leakMatchCount, 0);
    assert.equal(result.cleanup.complete, true);
    await assert.rejects(lstat(disposableRoot), (error) => error.code === "ENOENT");
    assert.equal(JSON.stringify(result).includes(PROVIDER_KEY), false);
  } finally {
    await removeFixture(fixture.root);
  }
});

test("isolated environment excludes inherited variables", () => {
  const env = buildIsolatedEnvironment({
    qoderConfigDir: "/tmp/poc/config",
    emptyCwd: "/tmp/poc/cwd",
    tempDir: "/tmp/poc/tmp",
    homeDir: "/tmp/poc/home",
    nodePath: "/opt/node/bin/node",
  });
  assert.equal(env.QODER_CONFIG_DIR, "/tmp/poc/config");
  assert.equal(env.HOME, "/tmp/poc/home");
  assert.equal(Object.hasOwn(env, "AWS_SECRET_ACCESS_KEY"), false);
  assert.equal(Object.hasOwn(env, "MALLARD_QODER_PAT"), false);
});

test("requires a separate Qoder PAT for the SDK probe", async () => {
  const fixture = await makeFixture();
  try {
    const result = await executeProbe(
      {
        mode: "qoder",
        credentialFile: fixture.credentialFile,
        model: "test-model",
        sdkModule: "/not/reached",
        qoderCli: "/not/reached",
      },
      { env: {} },
    );
    assert.equal(result.ok, false);
    assert.equal(result.error.code, "invalid_qoder_pat");
    assert.equal(result.cleanup.attempted, false);
  } finally {
    await removeFixture(fixture.root);
  }
});

test("runs the caller-supplied SDK through resolveModel and cleans its root", async () => {
  const fixture = await makeFixture();
  let disposableRoot;
  try {
    const files = await writeFakeSdk(fixture.root);
    const result = await executeProbe(
      {
        mode: "qoder",
        credentialFile: fixture.credentialFile,
        model: "test-model",
        provider: "openai",
        style: "openai",
        sdkModule: files.sdk,
        qoderCli: files.qoderCli,
      },
      {
        env: { MALLARD_QODER_PAT: QODER_PAT, AWS_SECRET_ACCESS_KEY: "excluded" },
        onTempRoot: (root) => {
          disposableRoot = root;
        },
      },
    );
    assert.equal(result.ok, true);
    assert.equal(result.operation.qoder.resolveModelCallCount, 1);
    assert.equal(result.operation.qoder.messageCount, 2);
    assert.equal(result.operation.qoder.resultMessageCount, 1);
    assert.equal(result.operation.qoder.toolUseMessageCount, 0);
    assert.deepEqual(result.operation.qoder.controls, {
      toolsDisabled: true,
      sessionPersistenceDisabled: true,
      debugLoggingDisabled: true,
      broadEnvironmentDisabled: true,
    });
    assert.equal(result.outputScan.leakMatchCount, 0);
    assert.equal(result.cleanup.complete, true);
    await assert.rejects(lstat(disposableRoot), (error) => error.code === "ENOENT");
    const serialized = JSON.stringify(result);
    assert.equal(serialized.includes(PROVIDER_KEY), false);
    assert.equal(serialized.includes(QODER_PAT), false);
  } finally {
    await removeFixture(fixture.root);
  }
});

test("detects a credential written by the SDK and still removes its root", async () => {
  const fixture = await makeFixture();
  let disposableRoot;
  try {
    const files = await writeFakeSdk(fixture.root, { leak: true });
    const result = await executeProbe(
      {
        mode: "qoder",
        credentialFile: fixture.credentialFile,
        model: "test-model",
        provider: "openai",
        style: "openai",
        sdkModule: files.sdk,
        qoderCli: files.qoderCli,
      },
      {
        env: { MALLARD_QODER_PAT: QODER_PAT },
        onTempRoot: (root) => {
          disposableRoot = root;
        },
      },
    );
    assert.equal(result.ok, false);
    assert.equal(result.error.code, "credential_leak_detected");
    assert.equal(result.outputScan.leakMatchCount, 1);
    assert.deepEqual(result.outputScan.leakPaths, ["qoder-config/leak.txt"]);
    assert.equal(result.cleanup.complete, true);
    await assert.rejects(lstat(disposableRoot), (error) => error.code === "ENOENT");
    assert.equal(JSON.stringify(result).includes(PROVIDER_KEY), false);
  } finally {
    await removeFixture(fixture.root);
  }
});

test("keeps the PoC verdict explicit", () => {
  assert.deepEqual(POC_STATUS, {
    productionReady: false,
    verdict: "POC_ONLY",
    qoderAppSupport: "UNPROVED",
    providerCompatibility: "REQUIRES_LIVE_PROBE",
  });
});

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";

const execFileAsync = promisify(execFile);
const SDK_PATH = process.env.QODER_SDK_PATH;
const SCRIPT_PATH = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "qoder-sdk-active-leaf-probe.mjs",
);

test(
  "proves isolated SDK active-leaf and duplicate-UUID reader behavior",
  { skip: SDK_PATH ? false : "Set QODER_SDK_PATH to the canonical SDK entry." },
  async () => {
    const { stdout, stderr } = await execFileAsync(
      process.execPath,
      [SCRIPT_PATH, "--sdk", SDK_PATH],
      {
        env: {
          PATH: "/usr/bin:/bin",
        },
        maxBuffer: 1024 * 1024,
        timeout: 30_000,
      },
    );
    assert.equal(stderr, "");
    const result = JSON.parse(stdout);

    assert.equal(result.sdk.name, "@qoder-ai/qoder-agent-sdk");
    assert.equal(result.sdk.version, "1.0.32");
    assert.equal(result.productionReady, false);
    assert.equal(result.verdict, "POC_ONLY");
    assert.deepEqual(result.isolation, {
      syntheticConfigOnly: true,
      inheritedCredentials: false,
      queryInvoked: false,
      mutationApiInvoked: false,
      networkFacingSdkApiInvoked: false,
    });
    assert.equal(result.activeLeaf.lastRecordWins.proved, true);
    assert.deepEqual(result.activeLeaf.lastRecordWins.markers, [
      "ROOT_USER",
      "ROOT_ASSISTANT",
      "OLD_USER",
      "OLD_ASSISTANT",
    ]);
    for (const name of [
      "legacyFallbackToLatestMainLeaf",
      "invalidLeafFallbackToLatestMainLeaf",
      "crossSessionReferencedNodeRejected",
    ]) {
      assert.equal(result.activeLeaf[name].proved, true);
      assert.deepEqual(result.activeLeaf[name].markers, [
        "ROOT_USER",
        "ROOT_ASSISTANT",
        "NEW_USER",
        "NEW_ASSISTANT",
      ]);
    }
    assert.equal(result.duplicateUuid.explicitProjectResolution.proved, true);
    assert.equal(
      result.duplicateUuid.unscopedResolution.listDeduplicatedToOne,
      true,
    );
    assert.equal(
      result.duplicateUuid.unscopedResolution.listSelectedNewestMtime,
      true,
    );
    assert.equal(
      result.duplicateUuid.unscopedResolution.ambiguousForSingleSessionReaders,
      true,
    );
    assert.ok(
      ["A", "B"].includes(
        result.duplicateUuid.unscopedResolution.observed.getSessionInfo,
      ),
    );
    assert.ok(
      ["A", "B"].includes(
        result.duplicateUuid.unscopedResolution.observed.getSessionMessages,
      ),
    );
    assert.equal(result.readOnly.fixtureUnchanged, true);
    assert.equal(result.readOnly.trackedFileCount, 7);
    assert.equal(stdout.includes("mallard-qoder-sdk-leaf-poc-"), false);
    assert.equal(stdout.includes("/Users/"), false);
  },
);

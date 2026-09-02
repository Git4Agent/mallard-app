import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";

const execFileAsync = promisify(execFile);
const SDK_PATH = process.env.QODER_SDK_PATH;
const ROOT = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT_PATH = path.join(ROOT, "qoder-sdk-fork-probe.mjs");
const GUARD_PATH = path.join(ROOT, "qoder-sdk-fork-network-guard.cjs");

test(
  "proves the isolated SDK transcript-only fork contract",
  { skip: SDK_PATH ? false : "Set QODER_SDK_PATH to the canonical SDK entry." },
  async () => {
    const { stdout, stderr } = await execFileAsync(
      process.execPath,
      [SCRIPT_PATH, "--sdk", SDK_PATH],
      {
        env: {
          NODE_OPTIONS: `--require=${GUARD_PATH}`,
          PATH: "/opt/homebrew/bin:/usr/bin:/bin",
        },
        maxBuffer: 1024 * 1024,
        timeout: 30_000,
      },
    );
    assert.equal(stderr, "");
    const result = JSON.parse(stdout);

    assert.deepEqual(result.sdk, {
      name: "@qoder-ai/qoder-agent-sdk",
      version: "1.0.32",
      bundleSha256: "acf8c7f1d2abe7c6b7799b3e1dce43165a1d83b7e9e1de10d13c216afbfcccd3",
    });
    assert.equal(result.verdict, "TRANSCRIPT_ONLY_POC");
    assert.equal(result.productionTaskForkReady, false);
    assert.deepEqual(result.isolation, {
      syntheticConfigOnly: true,
      inheritedCredentials: false,
      queryInvoked: false,
      cliSpawned: false,
      blockedExternalOperationCount: 0,
    });
    assert.deepEqual(result.transcriptTransform, {
      activeHistoricalChainUsed: true,
      unselectedMainBranchExcluded: true,
      selectedSidechainRecordRemoved: true,
      selectedSidechainRemovalCanDetachDescendant: true,
      progressExcluded: true,
      sourceMessageUuidsRemapped: true,
      parentUuidRewiredAcrossProgress: true,
      retainedLogicalParentUuidsRemapped: true,
      contentReplacementsRemapped: true,
      refusalFallbackNeutralized: true,
      providerRuntimeMetadataRemoved: true,
      sourceOriginStamped: true,
      activeLeafRecordCopied: false,
    });
    assert.deepEqual(result.externalStore.callSequence, [
      "load_source",
      "append_target",
    ]);
    assert.equal(result.externalStore.sourcePreserved, true);
    assert.equal(result.externalStore.targetUuidGenerated, true);
    assert.equal(result.externalStore.sdkSeparateTargetPreflightOrClaim, false);
    assert.equal(result.externalStore.collisionPolicyDelegatedToStoreAppend, true);
    assert.equal(result.externalStore.sourceSubkeysRead, false);
    assert.equal(result.externalStore.truncationInclusive, true);
    assert.deepEqual(result.progressLogicalParent, {
      observed: true,
      removedProgressUuidStillReferenced: true,
      targetLogicalParentHasNoEmittedEntry: true,
      targetReplacementHasNoEmittedEntry: true,
      productionGatePassed: false,
    });
    assert.deepEqual(result.selectedSidechain, {
      selectedSidechainRecordRemoved: true,
      retainedDescendantDetachedToRoot: true,
      productionGatePassed: false,
    });
    assert.equal(result.local.sourcePreserved, true);
    assert.equal(result.local.targetTranscriptMode, "0600");
    assert.equal(result.local.sidecarCopied, false);
    assert.equal(result.local.checkpointCopied, false);
    assert.equal(result.local.artifactSetExact, true);
    assert.equal(result.local.completeSourceFixtureTreePreserved, true);
    assert.ok(result.local.trackedSourceEntryCount >= 9);
    assert.deepEqual(result.packageObservations, {
      standaloneTargetUuidCallerSelectable: false,
      localTargetUsesExclusiveCreate: true,
    });
    assert.deepEqual(result.missingProductionEvidence, [
      "authenticated_cli_or_query_fork",
      "model_context_continuation",
      "task_sidecar_copy",
      "subagent_sidecar_copy",
      "file_history_checkpoint_copy",
      "application_database_projection",
      "external_store_compare_and_swap",
      "logical_parent_integrity_after_progress_filter",
      "content_replacement_integrity_after_progress_filter",
      "parent_integrity_after_sidechain_filter",
    ]);
    for (const forbidden of [
      "mallard-qoder-sdk-fork-poc-",
      "/Users/",
      "/private/",
      "/tmp/",
      "11111111-1111-4111-8111-111111111111",
      "ROOT_USER",
      "SIDECHAIN_ROOT",
      "PROGRESS_REPLACEMENT",
      "Source title",
      "Local fork",
    ]) {
      assert.equal(stdout.includes(forbidden), false);
    }
    assert.doesNotMatch(
      stdout,
      /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/i,
    );
  },
);

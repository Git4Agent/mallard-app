#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  access,
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SDK_NAME = "@qoder-ai/qoder-agent-sdk";
const SDK_VERSION = "1.0.32";
const SDK_SHA256 = "acf8c7f1d2abe7c6b7799b3e1dce43165a1d83b7e9e1de10d13c216afbfcccd3";
const ROOT_PREFIX = "mallard-qoder-sdk-fork-poc-";
const ROOT_MARKER = ".mallard-qoder-sdk-fork-synthetic";
const ROOT_MARKER_CONTENT = "synthetic-only\n";
const SOURCE_SESSION_ID = "11111111-1111-4111-8111-111111111111";
const CHECKPOINT_NAME = "0123456789abcdef@v1";

const IDS = Object.freeze({
  rootUser: "20000000-0000-4000-8000-000000000001",
  progress: "20000000-0000-4000-8000-000000000002",
  firstAssistant: "20000000-0000-4000-8000-000000000003",
  secondUser: "20000000-0000-4000-8000-000000000004",
  refusalFallback: "20000000-0000-4000-8000-000000000005",
  finalAssistant: "20000000-0000-4000-8000-000000000006",
  sidechain: "20000000-0000-4000-8000-000000000007",
  abandonedUser: "20000000-0000-4000-8000-000000000008",
  abandonedAssistant: "20000000-0000-4000-8000-000000000009",
  unknownReplacement: "20000000-0000-4000-8000-000000000099",
});

const FORBIDDEN_CREDENTIAL_KEYS = Object.freeze([
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "QODERCN_PERSONAL_ACCESS_TOKEN",
  "QODERCN_SERVICE_ACCOUNT_KEY",
  "QODER_ACCESS_TOKEN",
  "QODER_PERSONAL_ACCESS_TOKEN",
  "QODER_SERVICE_ACCOUNT_KEY",
]);

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function requireCondition(condition, code) {
  if (!condition) {
    fail(code);
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function clone(value) {
  return structuredClone(value);
}

function projectBucket(projectPath) {
  const processed = projectPath.replace(/[^a-zA-Z0-9]/g, "-");
  if (processed.length <= 200) {
    return processed;
  }
  let hash = 5381;
  for (let index = 0; index < projectPath.length; index += 1) {
    hash = (33 * hash) ^ projectPath.charCodeAt(index);
  }
  return `${processed.slice(0, 200)}-${Math.abs(hash).toString(36)}`;
}

async function canonicalSdk(sdkPath) {
  requireCondition(
    typeof sdkPath === "string" &&
      path.isAbsolute(sdkPath) &&
      !sdkPath.includes("\0"),
    "sdk_path_required",
  );
  const canonical = await realpath(sdkPath).catch(() => fail("invalid_sdk_path"));
  requireCondition(canonical === path.resolve(sdkPath), "indirect_sdk_path");
  const metadata = await lstat(canonical);
  requireCondition(metadata.isFile() && !metadata.isSymbolicLink(), "invalid_sdk_path");
  requireCondition(
    path.basename(canonical) === "index.js" &&
      path.basename(path.dirname(canonical)) === "dist",
    "unexpected_sdk_entrypoint",
  );

  const packagePath = path.join(path.dirname(path.dirname(canonical)), "package.json");
  const canonicalPackagePath = await realpath(packagePath).catch(() =>
    fail("sdk_package_missing"),
  );
  requireCondition(canonicalPackagePath === packagePath, "indirect_sdk_package");
  const packageMetadata = await lstat(packagePath);
  requireCondition(
    packageMetadata.isFile() && !packageMetadata.isSymbolicLink(),
    "invalid_sdk_package",
  );
  const packageDocument = JSON.parse(await readFile(packagePath, "utf8"));
  requireCondition(packageDocument.name === SDK_NAME, "unexpected_sdk_package");
  requireCondition(packageDocument.version === SDK_VERSION, "unexpected_sdk_version");
  requireCondition(
    sha256(await readFile(canonical)) === SDK_SHA256,
    "unexpected_sdk_bundle_digest",
  );
  return canonical;
}

function message(type, uuid, parentUuid, content, extra = {}) {
  return {
    type,
    uuid,
    sessionId: SOURCE_SESSION_ID,
    parentUuid,
    logicalParentUuid: parentUuid,
    timestamp: "2026-09-02T12:00:00.000Z",
    isSidechain: false,
    message: {
      role: type === "user" ? "user" : "assistant",
      content,
    },
    teamName: "must-not-copy",
    agentName: "must-not-copy",
    sessionKind: "must-not-copy",
    slug: "must-not-copy",
    sourceToolAssistantUUID: "must-not-copy",
    ...extra,
  };
}

function sourceEntries(projectRoot) {
  return [
    {
      type: "workspace-directories",
      sessionId: SOURCE_SESSION_ID,
      directories: [projectRoot],
      timestamp: "2026-09-02T11:59:00.000Z",
    },
    message("user", IDS.rootUser, null, "ROOT_USER"),
    {
      type: "progress",
      uuid: IDS.progress,
      sessionId: SOURCE_SESSION_ID,
      parentUuid: IDS.rootUser,
      logicalParentUuid: IDS.rootUser,
      timestamp: "2026-09-02T12:01:00.000Z",
      isSidechain: false,
      data: { marker: "PROGRESS" },
    },
    message(
      "assistant",
      IDS.firstAssistant,
      IDS.progress,
      "FIRST_ASSISTANT",
      { logicalParentUuid: IDS.rootUser },
    ),
    message(
      "assistant",
      IDS.sidechain,
      IDS.firstAssistant,
      "SIDECHAIN_ASSISTANT",
      { isSidechain: true },
    ),
    message(
      "user",
      IDS.abandonedUser,
      IDS.firstAssistant,
      "ABANDONED_USER",
    ),
    message(
      "assistant",
      IDS.abandonedAssistant,
      IDS.abandonedUser,
      "ABANDONED_ASSISTANT",
    ),
    message("user", IDS.secondUser, IDS.firstAssistant, "SECOND_USER"),
    message(
      "system",
      IDS.refusalFallback,
      IDS.secondUser,
      "REFUSAL_FALLBACK",
      {
        subtype: "model_refusal_fallback",
        message: { role: "system", content: "REFUSAL_FALLBACK" },
      },
    ),
    message(
      "assistant",
      IDS.finalAssistant,
      IDS.refusalFallback,
      "FINAL_ASSISTANT",
      { logicalParentUuid: IDS.secondUser },
    ),
    {
      type: "content-replacement",
      sessionId: SOURCE_SESSION_ID,
      replacements: [
        { messageUuid: IDS.firstAssistant, replacement: "FIRST_REPLACEMENT" },
        { messageUuid: IDS.secondUser, replacement: "SECOND_REPLACEMENT" },
        { messageUuid: IDS.unknownReplacement, replacement: "DROP_ME" },
      ],
      timestamp: "2026-09-02T12:07:00.000Z",
    },
    {
      type: "relocated",
      sessionId: SOURCE_SESSION_ID,
      relocatedCwd: projectRoot,
      timestamp: "2026-09-02T12:08:00.000Z",
    },
    {
      type: "custom-title",
      sessionId: SOURCE_SESSION_ID,
      customTitle: "Source title",
      timestamp: "2026-09-02T12:09:00.000Z",
    },
    {
      type: "file-history-snapshot",
      sessionId: SOURCE_SESSION_ID,
      messageId: IDS.secondUser,
      snapshot: {
        messageId: IDS.secondUser,
        trackedFileBackups: {
          "synthetic.txt": {
            backupFileName: CHECKPOINT_NAME,
            version: 1,
            backupTime: "2026-09-02T12:09:30.000Z",
          },
        },
        timestamp: "2026-09-02T12:09:30.000Z",
      },
      isSnapshotUpdate: false,
    },
    {
      type: "active-leaf",
      sessionId: SOURCE_SESSION_ID,
      leafUuid: IDS.finalAssistant,
      explicit: true,
      timestamp: "2026-09-02T12:10:00.000Z",
    },
  ];
}

function saveEnvironment(keys) {
  return new Map(
    keys.map((key) => [
      key,
      Object.hasOwn(process.env, key) ? process.env[key] : undefined,
    ]),
  );
}

function restoreEnvironment(saved) {
  for (const [key, value] of saved) {
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
}

function findEntry(entries, type) {
  return entries.find((entry) => entry.type === type);
}

function forkedMessageMap(entries) {
  return new Map(
    entries
      .filter((entry) => entry.forkedFrom)
      .map((entry) => [entry.forkedFrom.messageUuid, entry]),
  );
}

function verifyFork(entries, targetSessionId, options = {}) {
  const expectedSourceIds = options.truncated
    ? [IDS.rootUser, IDS.firstAssistant]
    : [
        IDS.rootUser,
        IDS.firstAssistant,
        IDS.secondUser,
        IDS.refusalFallback,
        IDS.finalAssistant,
      ];
  const copied = entries.filter((entry) => entry.forkedFrom);
  requireCondition(copied.length === expectedSourceIds.length, "copied_count_mismatch");
  requireCondition(
    copied.every(
      (entry) =>
        entry.sessionId === targetSessionId &&
        entry.isSidechain === false &&
        entry.forkedFrom.sessionId === SOURCE_SESSION_ID &&
        expectedSourceIds.includes(entry.forkedFrom.messageUuid),
    ),
    "fork_origin_mismatch",
  );
  requireCondition(
    copied.every(
      (entry) =>
        entry.uuid !== entry.forkedFrom.messageUuid &&
        !Object.hasOwn(entry, "teamName") &&
        !Object.hasOwn(entry, "agentName") &&
        !Object.hasOwn(entry, "sessionKind") &&
        !Object.hasOwn(entry, "slug") &&
        !Object.hasOwn(entry, "sourceToolAssistantUUID"),
    ),
    "fork_identity_or_metadata_mismatch",
  );
  requireCondition(
    entries.every(
      (entry) =>
        entry.type !== "progress" &&
        entry.type !== "active-leaf" &&
        entry.forkedFrom?.messageUuid !== IDS.sidechain &&
        entry.forkedFrom?.messageUuid !== IDS.abandonedUser &&
        entry.forkedFrom?.messageUuid !== IDS.abandonedAssistant,
    ),
    "excluded_record_was_copied",
  );

  const bySource = forkedMessageMap(entries);
  const root = bySource.get(IDS.rootUser);
  const firstAssistant = bySource.get(IDS.firstAssistant);
  requireCondition(root?.parentUuid === null, "root_parent_mismatch");
  requireCondition(
    firstAssistant?.parentUuid === root.uuid &&
      firstAssistant.logicalParentUuid === root.uuid,
    "progress_parent_was_not_rewired",
  );

  if (!options.truncated) {
    const secondUser = bySource.get(IDS.secondUser);
    const fallback = bySource.get(IDS.refusalFallback);
    const finalAssistant = bySource.get(IDS.finalAssistant);
    requireCondition(
      secondUser?.parentUuid === firstAssistant.uuid &&
        fallback?.parentUuid === secondUser.uuid &&
        finalAssistant?.parentUuid === fallback.uuid &&
        finalAssistant.logicalParentUuid === secondUser.uuid,
      "fork_parent_graph_mismatch",
    );
    requireCondition(fallback.neutralizedByFork === true, "fallback_not_neutralized");
  }

  const replacements = findEntry(entries, "content-replacement");
  const expectedReplacementIds = options.truncated
    ? [bySource.get(IDS.firstAssistant).uuid]
    : [
        bySource.get(IDS.firstAssistant).uuid,
        bySource.get(IDS.secondUser).uuid,
      ];
  requireCondition(
    replacements?.sessionId === targetSessionId &&
      JSON.stringify(replacements.replacements.map((item) => item.messageUuid)) ===
        JSON.stringify(expectedReplacementIds),
    "content_replacement_mismatch",
  );

  const relocated = findEntry(entries, "relocated");
  requireCondition(
    relocated?.sessionId === targetSessionId &&
      typeof relocated.relocatedCwd === "string",
    "relocation_mismatch",
  );
  const title = findEntry(entries, "custom-title");
  requireCondition(
    title?.sessionId === targetSessionId &&
      title.customTitle === options.expectedTitle,
    "title_mismatch",
  );
}

class RecordingStore {
  constructor(entries) {
    this.source = clone(entries);
    this.calls = [];
    this.targets = new Map();
  }

  async load(key) {
    this.calls.push({ operation: "load", key: clone(key) });
    if (key.sessionId === SOURCE_SESSION_ID && key.subpath === undefined) {
      return clone(this.source);
    }
    return null;
  }

  async append(key, entries) {
    this.calls.push({ operation: "append", key: clone(key), count: entries.length });
    const serialized = JSON.stringify(key);
    requireCondition(!this.targets.has(serialized), "store_target_collision");
    this.targets.set(serialized, clone(entries));
  }

  async listSubkeys() {
    fail("standalone_fork_read_subkeys");
  }

  target(sessionId) {
    const match = [...this.targets.entries()].find(([serialized]) =>
      serialized.includes(sessionId),
    );
    requireCondition(match, "store_target_missing");
    return match[1];
  }
}

async function snapshotSourceRoots(paths) {
  const result = {};
  async function visit(candidate, logicalPath) {
    const metadata = await lstat(candidate);
    requireCondition(!metadata.isSymbolicLink(), "unsafe_fixture_symlink");
    if (metadata.isDirectory()) {
      result[logicalPath] = {
        type: "directory",
        mode: metadata.mode & 0o777,
      };
      const entries = await readdir(candidate, { withFileTypes: true });
      entries.sort((left, right) => left.name.localeCompare(right.name));
      for (const entry of entries) {
        await visit(
          path.join(candidate, entry.name),
          `${logicalPath}/${entry.name}`,
        );
      }
      return;
    }
    requireCondition(metadata.isFile(), "unsafe_fixture_special_file");
    const bytes = await readFile(candidate);
    result[logicalPath] = {
      type: "file",
      size: bytes.length,
      digest: sha256(bytes),
      mode: metadata.mode & 0o777,
    };
  }
  for (const [name, candidate] of Object.entries(paths)) {
    await visit(candidate, name);
  }
  return result;
}

async function exists(candidate) {
  return access(candidate).then(
    () => true,
    () => false,
  );
}

async function createFixture(root) {
  const paths = {
    config: path.join(root, "qoder-config"),
    home: path.join(root, "home"),
    project: path.join(root, "project"),
    temp: path.join(root, "tmp"),
  };
  for (const directory of Object.values(paths)) {
    await mkdir(directory, { recursive: true, mode: 0o700 });
  }
  const bucketRoot = path.join(paths.config, "projects", projectBucket(paths.project));
  await mkdir(bucketRoot, { recursive: true, mode: 0o700 });
  const sourceTranscript = path.join(bucketRoot, `${SOURCE_SESSION_ID}.jsonl`);
  await writeFile(
    sourceTranscript,
    `${sourceEntries(paths.project).map((entry) => JSON.stringify(entry)).join("\n")}\n`,
    { flag: "wx", mode: 0o600 },
  );

  const sourceSidecarRoot = path.join(bucketRoot, SOURCE_SESSION_ID);
  const sourceState = path.join(sourceSidecarRoot, "state.json");
  const sourceCompression = path.join(sourceSidecarRoot, "compression-v2", "state.json");
  const sourceSubagent = path.join(
    sourceSidecarRoot,
    "subagents",
    "agent-explore-0123456789abcdef.jsonl",
  );
  await mkdir(path.dirname(sourceCompression), { recursive: true, mode: 0o700 });
  await mkdir(path.dirname(sourceSubagent), { recursive: true, mode: 0o700 });
  await writeFile(sourceState, '{"synthetic":"state"}\n', { flag: "wx", mode: 0o600 });
  await writeFile(sourceCompression, '{"synthetic":"compression"}\n', {
    flag: "wx",
    mode: 0o600,
  });
  await writeFile(sourceSubagent, '{"synthetic":"subagent"}\n', {
    flag: "wx",
    mode: 0o600,
  });

  const sourceCheckpoint = path.join(
    paths.config,
    "file-history",
    SOURCE_SESSION_ID,
    CHECKPOINT_NAME,
  );
  await mkdir(path.dirname(sourceCheckpoint), { recursive: true, mode: 0o700 });
  await writeFile(sourceCheckpoint, "synthetic-checkpoint\n", {
    flag: "wx",
    mode: 0o600,
  });

  return {
    ...paths,
    bucketRoot,
    sourceRoots: {
      transcript: sourceTranscript,
      sidecar: sourceSidecarRoot,
      checkpoint: path.dirname(sourceCheckpoint),
    },
  };
}

async function runExternalStoreProbe(sdk, projectRoot) {
  const entries = sourceEntries(projectRoot);
  const sourceBefore = sha256(JSON.stringify(entries));
  const fullStore = new RecordingStore(entries);
  const full = await sdk.forkSession(SOURCE_SESSION_ID, {
    dir: projectRoot,
    sessionStore: fullStore,
  });
  requireCondition(full.sessionId !== SOURCE_SESSION_ID, "store_target_reused_source_id");
  const fullEntries = fullStore.target(full.sessionId);
  verifyFork(fullEntries, full.sessionId, {
    expectedTitle: "Source title (fork)",
    truncated: false,
  });
  requireCondition(
    JSON.stringify(fullStore.calls.map((call) => call.operation)) ===
      JSON.stringify(["load", "append"]),
    "unexpected_store_call_sequence",
  );
  requireCondition(
    fullStore.calls.every(
      (call) => call.key.projectKey === projectBucket(projectRoot),
    ),
    "store_project_key_mismatch",
  );

  const truncatedStore = new RecordingStore(entries);
  const truncated = await sdk.forkSession(SOURCE_SESSION_ID, {
    dir: projectRoot,
    sessionStore: truncatedStore,
    upToMessageId: IDS.firstAssistant,
    title: "  Truncated fork  ",
  });
  requireCondition(
    truncated.sessionId !== SOURCE_SESSION_ID && truncated.sessionId !== full.sessionId,
    "store_target_uuid_not_fresh",
  );
  const truncatedEntries = truncatedStore.target(truncated.sessionId);
  verifyFork(truncatedEntries, truncated.sessionId, {
    expectedTitle: "Truncated fork",
    truncated: true,
  });
  requireCondition(
    sha256(JSON.stringify(entries)) === sourceBefore &&
      sha256(JSON.stringify(fullStore.source)) === sourceBefore &&
      sha256(JSON.stringify(truncatedStore.source)) === sourceBefore,
    "store_source_mutated",
  );

  return {
    sourcePreserved: true,
    targetUuidGenerated: true,
    callSequence: ["load_source", "append_target"],
    sdkSeparateTargetPreflightOrClaim: false,
    collisionPolicyDelegatedToStoreAppend: true,
    projectKeyPreserved: true,
    mainEntriesWritten: fullEntries.length,
    sourceSubkeysRead: false,
    truncationInclusive: true,
    truncatedEntriesWritten: truncatedEntries.length,
  };
}

async function runProgressLogicalParentProbe(sdk, projectRoot) {
  const entries = sourceEntries(projectRoot);
  const firstAssistant = entries.find(
    (entry) => entry.uuid === IDS.firstAssistant,
  );
  requireCondition(firstAssistant, "progress_probe_message_missing");
  firstAssistant.logicalParentUuid = IDS.progress;
  const replacements = entries.find(
    (entry) => entry.type === "content-replacement",
  );
  requireCondition(replacements, "progress_probe_replacements_missing");
  replacements.replacements.push({
    messageUuid: IDS.progress,
    replacement: "PROGRESS_REPLACEMENT",
  });
  const sourceBefore = sha256(JSON.stringify(entries));
  const store = new RecordingStore(entries);
  const result = await sdk.forkSession(SOURCE_SESSION_ID, {
    dir: projectRoot,
    sessionStore: store,
    title: "Progress logical parent fork",
  });
  const targetEntries = store.target(result.sessionId);
  const copiedFirstAssistant = targetEntries.find(
    (entry) => entry.forkedFrom?.messageUuid === IDS.firstAssistant,
  );
  requireCondition(copiedFirstAssistant, "progress_probe_target_message_missing");
  const emittedMessageUuids = new Set(
    targetEntries
      .filter((entry) => entry.forkedFrom)
      .map((entry) => entry.uuid),
  );
  requireCondition(
    typeof copiedFirstAssistant.logicalParentUuid === "string" &&
      !emittedMessageUuids.has(copiedFirstAssistant.logicalParentUuid),
    "progress_logical_parent_did_not_dangle",
  );
  const targetReplacements = targetEntries.find(
    (entry) => entry.type === "content-replacement",
  );
  const progressReplacement = targetReplacements?.replacements.find(
    (item) => item.replacement === "PROGRESS_REPLACEMENT",
  );
  requireCondition(
    typeof progressReplacement?.messageUuid === "string" &&
      !emittedMessageUuids.has(progressReplacement.messageUuid),
    "progress_replacement_did_not_dangle",
  );
  requireCondition(
    sha256(JSON.stringify(entries)) === sourceBefore &&
      sha256(JSON.stringify(store.source)) === sourceBefore,
    "progress_probe_source_mutated",
  );

  return {
    observed: true,
    removedProgressUuidStillReferenced: true,
    targetLogicalParentHasNoEmittedEntry: true,
    targetReplacementHasNoEmittedEntry: true,
    productionGatePassed: false,
  };
}

async function runSelectedSidechainProbe(sdk, projectRoot) {
  const entries = [
    message("user", IDS.rootUser, null, "SIDECHAIN_ROOT"),
    message(
      "assistant",
      IDS.sidechain,
      IDS.rootUser,
      "SELECTED_SIDECHAIN",
      { isSidechain: true },
    ),
    message("user", IDS.secondUser, IDS.sidechain, "AFTER_SIDECHAIN"),
    message(
      "assistant",
      IDS.finalAssistant,
      IDS.secondUser,
      "SIDECHAIN_FINAL",
    ),
    {
      type: "active-leaf",
      sessionId: SOURCE_SESSION_ID,
      leafUuid: IDS.finalAssistant,
      explicit: true,
      timestamp: "2026-09-02T12:10:00.000Z",
    },
  ];
  const sourceBefore = sha256(JSON.stringify(entries));
  const store = new RecordingStore(entries);
  const result = await sdk.forkSession(SOURCE_SESSION_ID, {
    dir: projectRoot,
    sessionStore: store,
    title: "Selected sidechain fork",
  });
  const targetEntries = store.target(result.sessionId);
  requireCondition(
    !targetEntries.some(
      (entry) => entry.forkedFrom?.messageUuid === IDS.sidechain,
    ),
    "selected_sidechain_was_emitted",
  );
  const copiedSecondUser = targetEntries.find(
    (entry) => entry.forkedFrom?.messageUuid === IDS.secondUser,
  );
  requireCondition(
    copiedSecondUser?.parentUuid === null &&
      copiedSecondUser.logicalParentUuid === null,
    "selected_sidechain_descendant_not_detached",
  );
  requireCondition(
    sha256(JSON.stringify(entries)) === sourceBefore &&
      sha256(JSON.stringify(store.source)) === sourceBefore,
    "selected_sidechain_source_mutated",
  );

  return {
    selectedSidechainRecordRemoved: true,
    retainedDescendantDetachedToRoot: true,
    productionGatePassed: false,
  };
}

async function runLocalProbe(sdk, paths) {
  const before = await snapshotSourceRoots(paths.sourceRoots);
  const result = await sdk.forkSession(SOURCE_SESSION_ID, {
    dir: paths.project,
    title: "Local fork",
  });
  requireCondition(result.sessionId !== SOURCE_SESSION_ID, "local_target_reused_source_id");
  const targetTranscript = path.join(paths.bucketRoot, `${result.sessionId}.jsonl`);
  const metadata = await stat(targetTranscript);
  requireCondition(metadata.isFile(), "local_target_not_file");
  requireCondition((metadata.mode & 0o777) === 0o600, "local_target_mode_mismatch");
  const targetEntries = (await readFile(targetTranscript, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  verifyFork(targetEntries, result.sessionId, {
    expectedTitle: "Local fork",
    truncated: false,
  });
  const after = await snapshotSourceRoots(paths.sourceRoots);
  requireCondition(JSON.stringify(after) === JSON.stringify(before), "local_source_mutated");

  const targetSidecar = path.join(paths.bucketRoot, result.sessionId);
  const targetCheckpoint = path.join(
    paths.config,
    "file-history",
    result.sessionId,
  );
  requireCondition(!(await exists(targetSidecar)), "local_sidecar_was_copied");
  requireCondition(!(await exists(targetCheckpoint)), "local_checkpoint_was_copied");

  const siblings = (await readdir(paths.bucketRoot)).sort();
  requireCondition(
    siblings.includes(`${SOURCE_SESSION_ID}.jsonl`) &&
      siblings.includes(`${result.sessionId}.jsonl`) &&
      siblings.includes(SOURCE_SESSION_ID) &&
      siblings.length === 3,
    "unexpected_local_artifact_set",
  );

  return {
    sourcePreserved: true,
    targetUuidGenerated: true,
    targetTranscriptMode: "0600",
    mainEntriesWritten: targetEntries.length,
    sidecarCopied: false,
    checkpointCopied: false,
    artifactSetExact: true,
    completeSourceFixtureTreePreserved: true,
    trackedSourceEntryCount: Object.keys(before).length,
  };
}

export async function runSdkForkProbe(sdkPath) {
  for (const key of FORBIDDEN_CREDENTIAL_KEYS) {
    requireCondition(!Object.hasOwn(process.env, key), "credential_environment_not_clean");
  }
  requireCondition(
    globalThis.__MALLARD_QODER_FORK_GUARD__ &&
      Array.isArray(globalThis.__MALLARD_QODER_FORK_GUARD__.blocked),
    "external_operation_guard_missing",
  );
  const sdkEntry = await canonicalSdk(sdkPath);
  const createdRoot = await mkdtemp(path.join(os.tmpdir(), ROOT_PREFIX));
  const root = await realpath(createdRoot);
  const originalCwd = process.cwd();
  const environmentKeys = ["HOME", "QODER_CLI_HOME", "QODER_CONFIG_DIR", "TMPDIR"];
  const savedEnvironment = saveEnvironment(environmentKeys);
  let cleanupComplete = false;

  try {
    await writeFile(path.join(root, ROOT_MARKER), ROOT_MARKER_CONTENT, {
      flag: "wx",
      mode: 0o600,
    });
    const paths = await createFixture(root);
    process.env.HOME = paths.home;
    process.env.QODER_CLI_HOME = paths.home;
    process.env.QODER_CONFIG_DIR = paths.config;
    process.env.TMPDIR = paths.temp;
    process.chdir(paths.project);

    const sdk = await import(pathToFileURL(sdkEntry).href);
    requireCondition(typeof sdk.forkSession === "function", "sdk_fork_api_missing");
    const externalStore = await runExternalStoreProbe(sdk, paths.project);
    const progressLogicalParent = await runProgressLogicalParentProbe(
      sdk,
      paths.project,
    );
    const selectedSidechain = await runSelectedSidechainProbe(
      sdk,
      paths.project,
    );
    const local = await runLocalProbe(sdk, paths);
    requireCondition(
      globalThis.__MALLARD_QODER_FORK_GUARD__.blocked.length === 0,
      "external_operation_attempted",
    );

    return {
      schemaVersion: 1,
      verdict: "TRANSCRIPT_ONLY_POC",
      productionTaskForkReady: false,
      sdk: {
        name: SDK_NAME,
        version: SDK_VERSION,
        bundleSha256: SDK_SHA256,
      },
      isolation: {
        syntheticConfigOnly: true,
        inheritedCredentials: false,
        queryInvoked: false,
        cliSpawned: false,
        blockedExternalOperationCount: 0,
      },
      transcriptTransform: {
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
      },
      externalStore,
      progressLogicalParent,
      selectedSidechain,
      local,
      packageObservations: {
        standaloneTargetUuidCallerSelectable: false,
        localTargetUsesExclusiveCreate: true,
      },
      missingProductionEvidence: [
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
      ],
    };
  } finally {
    process.chdir(originalCwd);
    restoreEnvironment(savedEnvironment);
    const markerPath = path.join(root, ROOT_MARKER);
    const marker = await lstat(markerPath).catch(() => null);
    const markerContent = marker?.isFile()
      ? await readFile(markerPath, "utf8").catch(() => null)
      : null;
    if (
      marker?.isFile() &&
      !marker.isSymbolicLink() &&
      markerContent === ROOT_MARKER_CONTENT &&
      path.basename(root).startsWith(ROOT_PREFIX)
    ) {
      await chmod(root, 0o700);
      await rm(root, { recursive: true, force: false });
      cleanupComplete = true;
    }
    requireCondition(cleanupComplete, "cleanup_failed");
  }
}

function parseSdkArgument(argv) {
  if (argv.length !== 2 || argv[0] !== "--sdk") {
    fail("usage_qoder_sdk_path_required");
  }
  return argv[1];
}

const isMain =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  const sdkPath = parseSdkArgument(process.argv.slice(2));
  const result = await runSdkForkProbe(sdkPath);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
  utimes,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const SDK_NAME = "@qoder-ai/qoder-agent-sdk";
const SDK_VERSION = "1.0.32";
const ROOT_PREFIX = "mallard-qoder-sdk-leaf-poc-";
const ROOT_MARKER = ".mallard-qoder-sdk-leaf-synthetic";

const LAST_RECORD_SESSION_ID = "55555555-5555-4555-8555-555555555555";
const LEGACY_SESSION_ID = "66666666-6666-4666-8666-666666666666";
const INVALID_LEAF_SESSION_ID = "77777777-7777-4777-8777-777777777777";
const CROSS_SESSION_ID = "88888888-8888-4888-8888-888888888888";
const FOREIGN_SESSION_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const DUPLICATE_SESSION_ID = "99999999-9999-4999-8999-999999999999";

const FORBIDDEN_CREDENTIAL_KEYS = Object.freeze([
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "QODERCN_PERSONAL_ACCESS_TOKEN",
  "QODERCN_SERVICE_ACCOUNT_KEY",
  "QODER_ACCESS_TOKEN",
  "QODER_PERSONAL_ACCESS_TOKEN",
  "QODER_SERVICE_ACCOUNT_KEY",
]);

const BRANCH_IDS = Object.freeze({
  rootUser: "10000000-0000-4000-8000-000000000001",
  rootAssistant: "10000000-0000-4000-8000-000000000002",
  oldUser: "10000000-0000-4000-8000-000000000003",
  oldAssistant: "10000000-0000-4000-8000-000000000004",
  newUser: "10000000-0000-4000-8000-000000000005",
  newAssistant: "10000000-0000-4000-8000-000000000006",
  missing: "10000000-0000-4000-8000-000000000099",
  foreign: "10000000-0000-4000-8000-000000000077",
});

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
  return canonical;
}

function transcriptEntry(type, uuid, sessionId, parentUuid, marker) {
  return {
    type,
    uuid,
    sessionId,
    parentUuid,
    timestamp: "2026-09-02T12:00:00.000Z",
    message: {
      role: type,
      content: marker,
    },
  };
}

function activeLeaf(sessionId, leafUuid, timestamp) {
  return {
    type: "active-leaf",
    sessionId,
    leafUuid,
    explicit: true,
    timestamp,
  };
}

function branchedTranscript(sessionId, leafRecords = []) {
  return [
    transcriptEntry(
      "user",
      BRANCH_IDS.rootUser,
      sessionId,
      null,
      "ROOT_USER",
    ),
    transcriptEntry(
      "assistant",
      BRANCH_IDS.rootAssistant,
      sessionId,
      BRANCH_IDS.rootUser,
      "ROOT_ASSISTANT",
    ),
    transcriptEntry(
      "user",
      BRANCH_IDS.oldUser,
      sessionId,
      BRANCH_IDS.rootAssistant,
      "OLD_USER",
    ),
    transcriptEntry(
      "assistant",
      BRANCH_IDS.oldAssistant,
      sessionId,
      BRANCH_IDS.oldUser,
      "OLD_ASSISTANT",
    ),
    transcriptEntry(
      "user",
      BRANCH_IDS.newUser,
      sessionId,
      BRANCH_IDS.rootAssistant,
      "NEW_USER",
    ),
    transcriptEntry(
      "assistant",
      BRANCH_IDS.newAssistant,
      sessionId,
      BRANCH_IDS.newUser,
      "NEW_ASSISTANT",
    ),
    ...leafRecords,
  ];
}

function duplicateTranscript(sessionId, origin) {
  const userUuid =
    origin === "A"
      ? "20000000-0000-4000-8000-000000000001"
      : "30000000-0000-4000-8000-000000000001";
  const assistantUuid =
    origin === "A"
      ? "20000000-0000-4000-8000-000000000002"
      : "30000000-0000-4000-8000-000000000002";
  return [
    transcriptEntry(
      "user",
      userUuid,
      sessionId,
      null,
      `DUPLICATE_${origin}_ROOT`,
    ),
    transcriptEntry(
      "assistant",
      assistantUuid,
      sessionId,
      userUuid,
      `DUPLICATE_${origin}_ASSISTANT`,
    ),
    activeLeaf(sessionId, assistantUuid, "2026-09-02T12:01:00.000Z"),
  ];
}

async function writeTranscript(configRoot, projectRoot, sessionId, records) {
  const bucketRoot = path.join(
    configRoot,
    "projects",
    projectBucket(projectRoot),
  );
  await mkdir(bucketRoot, { recursive: true, mode: 0o700 });
  const transcriptPath = path.join(bucketRoot, `${sessionId}.jsonl`);
  await writeFile(
    transcriptPath,
    `${records.map((record) => JSON.stringify(record)).join("\n")}\n`,
    { flag: "wx", mode: 0o600 },
  );
  return transcriptPath;
}

async function snapshotFiles(root) {
  const snapshot = {};
  async function visit(directory) {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const candidate = path.join(directory, entry.name);
      const relativePath = path.relative(root, candidate).split(path.sep).join("/");
      const metadata = await lstat(candidate);
      requireCondition(!metadata.isSymbolicLink(), "fixture_symlink_rejected");
      if (metadata.isDirectory()) {
        await visit(candidate);
        continue;
      }
      requireCondition(metadata.isFile(), "fixture_special_file_rejected");
      const bytes = await readFile(candidate);
      snapshot[relativePath] = {
        bytes: bytes.length,
        digest: sha256(bytes),
        mtimeMs: metadata.mtimeMs,
      };
    }
  }
  await visit(root);
  return snapshot;
}

function messageMarkers(messages) {
  return messages.map((message) => {
    requireCondition(
      message &&
        typeof message === "object" &&
        message.message &&
        typeof message.message === "object" &&
        typeof message.message.content === "string",
      "unexpected_message_projection",
    );
    return message.message.content;
  });
}

function originFromSummary(summary) {
  if (summary === "DUPLICATE_A_ROOT") {
    return "A";
  }
  if (summary === "DUPLICATE_B_ROOT") {
    return "B";
  }
  fail("unexpected_duplicate_summary");
}

function originFromMessages(messages) {
  const markers = messageMarkers(messages);
  if (markers.includes("DUPLICATE_A_ROOT")) {
    return "A";
  }
  if (markers.includes("DUPLICATE_B_ROOT")) {
    return "B";
  }
  fail("unexpected_duplicate_messages");
}

function sameArray(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

async function createFixture(root) {
  const paths = {
    config: path.join(root, "qoder-config"),
    home: path.join(root, "home"),
    temp: path.join(root, "tmp"),
    leafProject: path.join(root, "leaf-project"),
    collisionA: path.join(root, "collision-a"),
    collisionB: path.join(root, "collision-b"),
    unscopedCwd: path.join(root, "unscoped-cwd"),
  };
  for (const directory of Object.values(paths)) {
    await mkdir(directory, { recursive: true, mode: 0o700 });
  }

  await writeTranscript(
    paths.config,
    paths.leafProject,
    LAST_RECORD_SESSION_ID,
    branchedTranscript(LAST_RECORD_SESSION_ID, [
      activeLeaf(
        LAST_RECORD_SESSION_ID,
        BRANCH_IDS.newAssistant,
        "2026-09-02T12:02:00.000Z",
      ),
      activeLeaf(
        LAST_RECORD_SESSION_ID,
        BRANCH_IDS.oldAssistant,
        "2026-09-01T12:03:00.000Z",
      ),
    ]),
  );
  await writeTranscript(
    paths.config,
    paths.leafProject,
    LEGACY_SESSION_ID,
    branchedTranscript(LEGACY_SESSION_ID),
  );
  await writeTranscript(
    paths.config,
    paths.leafProject,
    INVALID_LEAF_SESSION_ID,
    branchedTranscript(INVALID_LEAF_SESSION_ID, [
      activeLeaf(
        INVALID_LEAF_SESSION_ID,
        BRANCH_IDS.missing,
        "2026-09-02T12:03:00.000Z",
      ),
    ]),
  );

  const crossSessionRecords = branchedTranscript(CROSS_SESSION_ID);
  crossSessionRecords.splice(
    4,
    0,
    transcriptEntry(
      "assistant",
      BRANCH_IDS.foreign,
      FOREIGN_SESSION_ID,
      BRANCH_IDS.rootAssistant,
      "FOREIGN_SESSION_LEAF",
    ),
  );
  crossSessionRecords.push(
    activeLeaf(
      CROSS_SESSION_ID,
      BRANCH_IDS.foreign,
      "2026-09-02T12:03:00.000Z",
    ),
  );
  await writeTranscript(
    paths.config,
    paths.leafProject,
    CROSS_SESSION_ID,
    crossSessionRecords,
  );

  const collisionAPath = await writeTranscript(
    paths.config,
    paths.collisionA,
    DUPLICATE_SESSION_ID,
    duplicateTranscript(DUPLICATE_SESSION_ID, "A"),
  );
  const collisionBPath = await writeTranscript(
    paths.config,
    paths.collisionB,
    DUPLICATE_SESSION_ID,
    duplicateTranscript(DUPLICATE_SESSION_ID, "B"),
  );
  await utimes(
    collisionAPath,
    new Date("2026-09-02T12:00:00.000Z"),
    new Date("2026-09-02T12:00:00.000Z"),
  );
  await utimes(
    collisionBPath,
    new Date("2026-09-02T12:10:00.000Z"),
    new Date("2026-09-02T12:10:00.000Z"),
  );
  return paths;
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

export async function runSdkActiveLeafProbe(sdkPath) {
  for (const key of FORBIDDEN_CREDENTIAL_KEYS) {
    requireCondition(
      !Object.hasOwn(process.env, key),
      "credential_environment_not_clean",
    );
  }
  const sdkEntry = await canonicalSdk(sdkPath);
  const createdRoot = await mkdtemp(path.join(os.tmpdir(), ROOT_PREFIX));
  const root = await realpath(createdRoot);
  const originalCwd = process.cwd();
  const environmentKeys = ["HOME", "QODER_CLI_HOME", "QODER_CONFIG_DIR", "TMPDIR"];
  const savedEnvironment = saveEnvironment(environmentKeys);
  let cleanupComplete = false;

  try {
    await writeFile(path.join(root, ROOT_MARKER), "synthetic-only\n", {
      flag: "wx",
      mode: 0o600,
    });
    const paths = await createFixture(root);
    process.env.HOME = paths.home;
    process.env.QODER_CLI_HOME = paths.home;
    process.env.QODER_CONFIG_DIR = paths.config;
    process.env.TMPDIR = paths.temp;
    process.chdir(paths.unscopedCwd);

    const before = await snapshotFiles(root);
    const sdk = await import(pathToFileURL(sdkEntry).href);
    for (const name of ["getSessionInfo", "getSessionMessages", "listSessions"]) {
      requireCondition(typeof sdk[name] === "function", "sdk_reader_api_missing");
    }

    const localOptions = {
      dir: paths.leafProject,
      includeWorktrees: false,
    };
    const lastRecord = messageMarkers(
      await sdk.getSessionMessages(LAST_RECORD_SESSION_ID, localOptions),
    );
    const legacy = messageMarkers(
      await sdk.getSessionMessages(LEGACY_SESSION_ID, localOptions),
    );
    const invalidLeaf = messageMarkers(
      await sdk.getSessionMessages(INVALID_LEAF_SESSION_ID, localOptions),
    );
    const crossSession = messageMarkers(
      await sdk.getSessionMessages(CROSS_SESSION_ID, localOptions),
    );

    const expectedOldBranch = [
      "ROOT_USER",
      "ROOT_ASSISTANT",
      "OLD_USER",
      "OLD_ASSISTANT",
    ];
    const expectedNewBranch = [
      "ROOT_USER",
      "ROOT_ASSISTANT",
      "NEW_USER",
      "NEW_ASSISTANT",
    ];
    requireCondition(
      sameArray(lastRecord, expectedOldBranch),
      "last_active_leaf_not_selected",
    );
    requireCondition(
      sameArray(legacy, expectedNewBranch),
      "legacy_leaf_fallback_failed",
    );
    requireCondition(
      sameArray(invalidLeaf, expectedNewBranch),
      "invalid_leaf_fallback_failed",
    );
    requireCondition(
      sameArray(crossSession, expectedNewBranch) &&
        !crossSession.includes("FOREIGN_SESSION_LEAF"),
      "cross_session_leaf_not_rejected",
    );

    const optionsA = { dir: paths.collisionA, includeWorktrees: false };
    const optionsB = { dir: paths.collisionB, includeWorktrees: false };
    const [listA, listB, infoA, infoB, messagesA, messagesB] = await Promise.all([
      sdk.listSessions(optionsA),
      sdk.listSessions(optionsB),
      sdk.getSessionInfo(DUPLICATE_SESSION_ID, optionsA),
      sdk.getSessionInfo(DUPLICATE_SESSION_ID, optionsB),
      sdk.getSessionMessages(DUPLICATE_SESSION_ID, optionsA),
      sdk.getSessionMessages(DUPLICATE_SESSION_ID, optionsB),
    ]);
    requireCondition(
      listA.length === 1 && originFromSummary(listA[0].summary) === "A",
      "explicit_project_a_list_failed",
    );
    requireCondition(
      listB.length === 1 && originFromSummary(listB[0].summary) === "B",
      "explicit_project_b_list_failed",
    );
    requireCondition(infoA && originFromSummary(infoA.summary) === "A", "explicit_project_a_info_failed");
    requireCondition(infoB && originFromSummary(infoB.summary) === "B", "explicit_project_b_info_failed");
    requireCondition(originFromMessages(messagesA) === "A", "explicit_project_a_messages_failed");
    requireCondition(originFromMessages(messagesB) === "B", "explicit_project_b_messages_failed");

    const unscopedList = await sdk.listSessions({ includeWorktrees: false });
    const unscopedDuplicate = unscopedList.filter(
      (session) => session.sessionId === DUPLICATE_SESSION_ID,
    );
    const unscopedInfo = await sdk.getSessionInfo(DUPLICATE_SESSION_ID);
    const unscopedMessages = await sdk.getSessionMessages(DUPLICATE_SESSION_ID);
    requireCondition(unscopedDuplicate.length === 1, "unscoped_duplicate_not_deduplicated");
    requireCondition(unscopedInfo, "unscoped_duplicate_info_missing");
    requireCondition(unscopedMessages.length > 0, "unscoped_duplicate_messages_missing");

    const unscopedOrigins = {
      listSessions: originFromSummary(unscopedDuplicate[0].summary),
      getSessionInfo: originFromSummary(unscopedInfo.summary),
      getSessionMessages: originFromMessages(unscopedMessages),
    };
    requireCondition(
      unscopedOrigins.listSessions === "B",
      "newest_duplicate_not_selected_by_list",
    );
    requireCondition(
      ["A", "B"].includes(unscopedOrigins.getSessionInfo) &&
        ["A", "B"].includes(unscopedOrigins.getSessionMessages),
      "unscoped_duplicate_origin_invalid",
    );

    const after = await snapshotFiles(root);
    const readOnly = JSON.stringify(before) === JSON.stringify(after);
    requireCondition(readOnly, "sdk_reader_mutated_fixture");

    return {
      schemaVersion: 1,
      verdict: "POC_ONLY",
      productionReady: false,
      sdk: {
        name: SDK_NAME,
        version: SDK_VERSION,
      },
      isolation: {
        syntheticConfigOnly: true,
        inheritedCredentials: false,
        queryInvoked: false,
        mutationApiInvoked: false,
        networkFacingSdkApiInvoked: false,
      },
      activeLeaf: {
        lastRecordWins: {
          proved: true,
          markers: lastRecord,
        },
        legacyFallbackToLatestMainLeaf: {
          proved: true,
          markers: legacy,
        },
        invalidLeafFallbackToLatestMainLeaf: {
          proved: true,
          markers: invalidLeaf,
        },
        crossSessionReferencedNodeRejected: {
          proved: true,
          markers: crossSession,
        },
      },
      duplicateUuid: {
        sessionId: DUPLICATE_SESSION_ID,
        explicitProjectResolution: {
          projectA: "A",
          projectB: "B",
          proved: true,
        },
        unscopedResolution: {
          candidates: ["A", "B"],
          observed: unscopedOrigins,
          listDeduplicatedToOne: true,
          listSelectedNewestMtime: true,
          directoryEnumerationOrderNotAsserted: true,
          ambiguousForSingleSessionReaders: true,
        },
      },
      readOnly: {
        fixtureUnchanged: readOnly,
        trackedFileCount: Object.keys(before).length,
      },
    };
  } finally {
    process.chdir(originalCwd);
    restoreEnvironment(savedEnvironment);
    const markerPath = path.join(root, ROOT_MARKER);
    const marker = await stat(markerPath).catch(() => null);
    if (marker?.isFile() && path.basename(root).startsWith(ROOT_PREFIX)) {
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

function safeErrorCode(error) {
  if (
    error &&
    typeof error === "object" &&
    typeof error.code === "string" &&
    /^[a-z0-9_]{1,80}$/i.test(error.code)
  ) {
    return error.code.toLowerCase();
  }
  return "probe_failed";
}

const isMain =
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  try {
    const result = await runSdkActiveLeafProbe(parseSdkArgument(process.argv.slice(2)));
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`${JSON.stringify({ error: safeErrorCode(error) })}\n`);
    process.exitCode = 1;
  }
}

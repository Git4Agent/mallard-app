import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  readFile,
  realpath,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  DEFAULT_STRUCTURAL_PATH_RULES,
  POC_STATUS,
  SYNTHETIC_MARKER_CONTENT,
  SYNTHETIC_MARKER_NAME,
  applyRestore,
  captureSessionGroup,
  planRestore,
  transformSnapshot,
} from "./qoder-session-restore.mjs";

const SESSION_ID = "11111111-1111-4111-8111-111111111111";
const OTHER_SESSION_ID = "22222222-2222-4222-8222-222222222222";
const FIRST_LEAF_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const SECOND_LEAF_ID = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const BUCKET = "synthetic-project";
const OLD_PROJECT = "/synthetic/source/project";
const NEW_PROJECT = "/synthetic/destination/project";
const AGENT_STEM = "agent-explore-0123456789abcdef";
const TASK_STEM = "task-fedcba9876543210";

const PATH_RULES = [
  ...DEFAULT_STRUCTURAL_PATH_RULES,
  {
    logicalPath: "transcript/main.jsonl",
    recordType: "runtime-config",
    path: ["cwd"],
  },
  {
    logicalPath: "state/state.json",
    path: ["workspace", "root"],
  },
];

async function makeSandbox(t) {
  const createdRoot = await mkdtemp(
    path.join(os.tmpdir(), "mallard-qoder-poc-"),
  );
  const root = await realpath(createdRoot);
  await writeFile(
    path.join(root, SYNTHETIC_MARKER_NAME),
    SYNTHETIC_MARKER_CONTENT,
    { mode: 0o600 },
  );
  await mkdir(path.join(root, "projects"), { recursive: true });
  t.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  return {
    root,
    projectsRoot: path.join(root, "projects"),
    operationRoot: path.join(root, "mallard-operations"),
  };
}

function defaultMainRecords(sessionId = SESSION_ID) {
  return [
    {
      type: "workspace-directories",
      sessionId,
      directories: [OLD_PROJECT],
    },
    {
      type: "runtime-config",
      sessionId,
      cwd: OLD_PROJECT,
    },
    {
      type: "user",
      sessionId,
      message: `Historical text keeps ${OLD_PROJECT} byte-for-byte.`,
    },
    {
      type: "active-leaf",
      sessionId,
      leafUuid: FIRST_LEAF_ID,
      timestamp: "2026-01-01T00:00:00Z",
      explicit: false,
    },
    {
      type: "active-leaf",
      sessionId,
      leafUuid: SECOND_LEAF_ID,
      timestamp: "2026-01-02T00:00:00Z",
      explicit: true,
    },
  ];
}

async function writeSession({
  sandbox,
  bucket = BUCKET,
  sessionId = SESSION_ID,
  mainRecords = defaultMainRecords(sessionId),
  rawMain = null,
  withCompression = true,
  withSubagents = true,
  stateSessionId = sessionId,
}) {
  const bucketPath = path.join(sandbox.projectsRoot, bucket);
  const sessionPath = path.join(bucketPath, sessionId);
  await mkdir(sessionPath, { recursive: true });
  const mainText =
    rawMain ?? `${mainRecords.map((record) => JSON.stringify(record)).join("\n")}\n`;
  await writeFile(path.join(bucketPath, `${sessionId}.jsonl`), mainText, {
    mode: 0o600,
  });
  await writeFile(
    path.join(sessionPath, "state.json"),
    `${JSON.stringify({ sessionId: stateSessionId, workspace: { root: OLD_PROJECT } }, null, 2)}\n`,
    { mode: 0o600 },
  );
  if (withCompression) {
    await mkdir(path.join(sessionPath, "compression-v2"), { recursive: true });
    await writeFile(
      path.join(sessionPath, "compression-v2", "state.json"),
      `${JSON.stringify({ session_id: sessionId, compacted: true })}\n`,
      { mode: 0o600 },
    );
  }
  if (withSubagents) {
    const subagents = path.join(sessionPath, "subagents");
    await mkdir(subagents, { recursive: true });
    await writeFile(
      path.join(subagents, `${AGENT_STEM}.jsonl`),
      `${JSON.stringify({ type: "user", session_id: sessionId, message: "synthetic" })}\n${JSON.stringify({ type: "assistant", sessionId, message: "synthetic result" })}\n`,
      { mode: 0o600 },
    );
    await writeFile(
      path.join(subagents, `${AGENT_STEM}.meta.json`),
      `${JSON.stringify({ session_id: sessionId, agent: "synthetic" })}\n`,
      { mode: 0o600 },
    );
    await writeFile(
      path.join(subagents, `${TASK_STEM}.json`),
      `${JSON.stringify({ sessionId, task: "synthetic" })}\n`,
      { mode: 0o600 },
    );
  }
  return { bucketPath, sessionPath, mainText };
}

async function capture(sandbox, options = {}) {
  return captureSessionGroup({
    sandboxRoot: sandbox.root,
    projectsRoot: sandbox.projectsRoot,
    projectBucket: options.bucket ?? BUCKET,
    sessionId: options.sessionId ?? SESSION_ID,
    hooks: options.hooks,
  });
}

async function createPlan(snapshot, destination, options = {}) {
  return planRestore({
    snapshot,
    destinationSandboxRoot: destination.root,
    destinationProjectsRoot: destination.projectsRoot,
    operationRoot: destination.operationRoot,
    destinationProjectBucket: options.bucket ?? BUCKET,
    cloudTaskId: options.cloudTaskId ?? "cloud-task-synthetic",
    existingMapping: options.existingMapping,
    allMappings: options.allMappings,
    reviewedObsolete: options.reviewedObsolete,
    pathMapping: options.pathMapping,
    structuralPathRules: options.structuralPathRules ?? PATH_RULES,
  });
}

test("requires the exact synthetic sandbox boundary", async (t) => {
  const unsafeName = await mkdtemp(path.join(os.tmpdir(), "qoder-poc-"));
  const missingMarker = await mkdtemp(
    path.join(os.tmpdir(), "mallard-qoder-poc-"),
  );
  t.after(async () => {
    await rm(unsafeName, { recursive: true, force: true });
    await rm(missingMarker, { recursive: true, force: true });
  });
  await writeFile(
    path.join(unsafeName, SYNTHETIC_MARKER_NAME),
    SYNTHETIC_MARKER_CONTENT,
  );
  await mkdir(path.join(unsafeName, "projects"));
  await mkdir(path.join(missingMarker, "projects"));

  for (const root of [unsafeName, missingMarker]) {
    await assert.rejects(
      captureSessionGroup({
        sandboxRoot: root,
        projectsRoot: path.join(root, "projects"),
        projectBucket: BUCKET,
        sessionId: SESSION_ID,
      }),
      (error) => error.code === "SYNTHETIC_SANDBOX_REQUIRED",
    );
  }
});

test("captures and restores the complete JSON and JSONL group", async (t) => {
  const source = await makeSandbox(t);
  const destination = await makeSandbox(t);
  await writeSession({ sandbox: source });

  const snapshot = await capture(source);
  assert.equal(snapshot.members.length, 6);
  assert.deepEqual(
    snapshot.members.map((member) => member.logicalPath),
    [
      "state/compression-v2/state.json",
      "state/state.json",
      `subagents/${AGENT_STEM}.jsonl`,
      `subagents/${AGENT_STEM}.meta.json`,
      `subagents/${TASK_STEM}.json`,
      "transcript/main.jsonl",
    ],
  );
  assert.equal(snapshot.analysis.activeLeaf.authority, "UNPROVED");
  assert.equal(snapshot.analysis.activeLeaf.authoritativeCandidate, null);
  assert.equal(snapshot.analysis.activeLeaf.candidates.length, 2);
  assert.equal(snapshot.analysis.uuid.replacementPermitted, false);
  assert.ok(snapshot.analysis.uuid.knownOccurrences.length >= 8);

  const plan = await createPlan(snapshot, destination, {
    pathMapping: { from: OLD_PROJECT, to: NEW_PROJECT },
  });
  assert.equal(plan.mode, "create");
  assert.equal(plan.sessionId, SESSION_ID);
  assert.equal(plan.productionStatus.productionReady, false);

  const result = await applyRestore({ snapshot, plan });
  assert.equal(result.mapping.sessionId, SESSION_ID);
  assert.equal(result.mapping.members.length, 6);
  assert.equal(result.productionStatus.verdict, "POC_ONLY");
  assert.equal(result.restored.analysis.activeLeaf.authority, "UNPROVED");

  const restoredMain = await readFile(
    path.join(destination.projectsRoot, BUCKET, `${SESSION_ID}.jsonl`),
    "utf8",
  );
  assert.match(restoredMain, new RegExp(NEW_PROJECT.replaceAll("/", "\\/")));
  assert.match(restoredMain, new RegExp(`Historical text keeps ${OLD_PROJECT}`));
  const restoredSubagent = await readFile(
    path.join(
      destination.projectsRoot,
      BUCKET,
      SESSION_ID,
      "subagents",
      `${AGENT_STEM}.jsonl`,
    ),
    "utf8",
  );
  assert.equal(restoredSubagent.split("\n").filter(Boolean).length, 2);
  const journal = JSON.parse(await readFile(result.journalPath, "utf8"));
  assert.equal(journal.state, "committed");
  assert.equal(journal.operations.length, 6);
});

test("rejects a member addition between snapshot passes", async (t) => {
  const source = await makeSandbox(t);
  const { sessionPath } = await writeSession({ sandbox: source });
  await assert.rejects(
    capture(source, {
      hooks: {
        afterFirstPass: async () => {
          await writeFile(
            path.join(sessionPath, "subagents", "task-1111111111111111.json"),
            `${JSON.stringify({ sessionId: SESSION_ID })}\n`,
          );
        },
      },
    }),
    (error) => error.code === "SNAPSHOT_CHANGED",
  );
});

test("rejects a member removal between snapshot passes", async (t) => {
  const source = await makeSandbox(t);
  const { sessionPath } = await writeSession({ sandbox: source });
  await assert.rejects(
    capture(source, {
      hooks: {
        afterFirstPass: async () => {
          await unlink(
            path.join(sessionPath, "subagents", `${TASK_STEM}.json`),
          );
        },
      },
    }),
    (error) => error.code === "SNAPSHOT_CHANGED",
  );
});

test("rejects in-place mutation between snapshot passes", async (t) => {
  const source = await makeSandbox(t);
  const { sessionPath } = await writeSession({ sandbox: source });
  const statePath = path.join(sessionPath, "state.json");
  await assert.rejects(
    capture(source, {
      hooks: {
        afterSecondEnumeration: async () => {
          await writeFile(
            statePath,
            `${JSON.stringify({ sessionId: SESSION_ID, changed: true })}\n`,
          );
        },
      },
    }),
    (error) => error.code === "SNAPSHOT_CHANGED",
  );
});

test("rejects replacement of a member between snapshot passes", async (t) => {
  const source = await makeSandbox(t);
  const { bucketPath } = await writeSession({ sandbox: source });
  const mainPath = path.join(bucketPath, `${SESSION_ID}.jsonl`);
  await assert.rejects(
    capture(source, {
      hooks: {
        afterSecondEnumeration: async () => {
          const replacement = path.join(bucketPath, "replacement.jsonl");
          const original = await readFile(mainPath);
          await writeFile(replacement, original);
          await unlink(mainPath);
          await writeFile(mainPath, original);
          await unlink(replacement);
        },
      },
    }),
    (error) => error.code === "SNAPSHOT_CHANGED",
  );
});

test("rejects a sidecar symbolic link", async (t) => {
  const source = await makeSandbox(t);
  const { sessionPath } = await writeSession({ sandbox: source });
  const statePath = path.join(sessionPath, "state.json");
  const target = path.join(source.root, "synthetic-state.json");
  await writeFile(target, `${JSON.stringify({ sessionId: SESSION_ID })}\n`);
  await unlink(statePath);
  await symlink(target, statePath);
  await assert.rejects(
    capture(source),
    (error) => error.code === "SYMLINK_REJECTED",
  );
});

test("blocks a new mapping when the source UUID exists in another bucket", async (t) => {
  const source = await makeSandbox(t);
  const destination = await makeSandbox(t);
  await writeSession({ sandbox: source });
  const collisionBucket = path.join(destination.projectsRoot, "other-project");
  await mkdir(collisionBucket, { recursive: true });
  await writeFile(path.join(collisionBucket, `${SESSION_ID}.jsonl`), "{}\n");
  const snapshot = await capture(source);

  await assert.rejects(
    createPlan(snapshot, destination),
    (error) => error.code === "SESSION_UUID_COLLISION",
  );
});

test("blocks an update when a reviewed destination changes after planning", async (t) => {
  const source = await makeSandbox(t);
  const destination = await makeSandbox(t);
  await writeSession({ sandbox: source });
  const snapshot = await capture(source);
  const create = await createPlan(snapshot, destination);
  const first = await applyRestore({ snapshot, plan: create });
  const update = await createPlan(snapshot, destination, {
    existingMapping: first.mapping,
  });
  const mainPath = path.join(destination.projectsRoot, BUCKET, `${SESSION_ID}.jsonl`);
  await writeFile(
    mainPath,
    `${await readFile(mainPath, "utf8")}${JSON.stringify({ type: "last-prompt", sessionId: SESSION_ID })}\n`,
  );

  await assert.rejects(
    applyRestore({ snapshot, plan: update }),
    (error) => error.code === "TARGET_CHANGED",
  );
});

test("reports a source UUID hidden in an unknown record and forbids remap", async (t) => {
  const source = await makeSandbox(t);
  const records = [
    ...defaultMainRecords(),
    {
      type: "future-record",
      opaqueReference: `qoder://${SESSION_ID}/untyped`,
    },
  ];
  await writeSession({ sandbox: source, mainRecords: records });
  const snapshot = await capture(source);
  assert.equal(snapshot.analysis.uuid.replacementPermitted, false);
  assert.equal(snapshot.analysis.uuid.hiddenReferences.length, 1);
  assert.equal(snapshot.analysis.uuid.hiddenReferences[0].unknownRecord, true);
});

test("rewrites only allowlisted JSON string tokens", async (t) => {
  const source = await makeSandbox(t);
  const preservedTail =
    ',"message":"Keep \\/synthetic\\/source\\/project and \\u002fsynthetic\\u002fsource\\u002fproject exactly."}';
  const rawMain = [
    `{"type":"workspace-directories","sessionId":"${SESSION_ID}","directories":["${OLD_PROJECT}"]}`,
    `{"type":"runtime-config","sessionId":"${SESSION_ID}","cwd":"${OLD_PROJECT}"${preservedTail}`,
    `{"type":"active-leaf","sessionId":"${SESSION_ID}","leafUuid":"${FIRST_LEAF_ID}"}`,
  ].join("\n") + "\n";
  await writeSession({ sandbox: source, rawMain });
  const snapshot = await capture(source);
  const transformed = transformSnapshot(snapshot, {
    pathMapping: { from: OLD_PROJECT, to: NEW_PROJECT },
    structuralPathRules: PATH_RULES,
  });
  const main = transformed.members.find(
    (member) => member.logicalPath === "transcript/main.jsonl",
  ).bytes.toString("utf8");
  assert.match(main, new RegExp(NEW_PROJECT.replaceAll("/", "\\/")));
  assert.ok(main.includes(preservedTail));
  assert.ok(!main.includes(`"cwd":"${OLD_PROJECT}"`));
});

test("blocks path mapping when an unproved integrity field is present", async (t) => {
  const source = await makeSandbox(t);
  await writeSession({
    sandbox: source,
    mainRecords: [
      ...defaultMainRecords(),
      {
        type: "assistant",
        sessionId: SESSION_ID,
        signature: "synthetic-not-a-secret",
      },
    ],
  });
  const snapshot = await capture(source);
  assert.equal(snapshot.analysis.integrityFields.length, 1);
  assert.throws(
    () =>
      transformSnapshot(snapshot, {
        pathMapping: { from: OLD_PROJECT, to: NEW_PROJECT },
        structuralPathRules: PATH_RULES,
      }),
    (error) => error.code === "UNPROVED_INTEGRITY_FIELD",
  );
  assert.doesNotThrow(() => transformSnapshot(snapshot));
});

test("requires exact review before deleting obsolete sidecars", async (t) => {
  const source = await makeSandbox(t);
  const reducedSource = await makeSandbox(t);
  const destination = await makeSandbox(t);
  await writeSession({ sandbox: source });
  await writeSession({ sandbox: reducedSource, withSubagents: false });
  const original = await capture(source);
  const firstPlan = await createPlan(original, destination);
  const first = await applyRestore({ snapshot: original, plan: firstPlan });
  const reduced = await capture(reducedSource);

  await assert.rejects(
    createPlan(reduced, destination, { existingMapping: first.mapping }),
    (error) => error.code === "OBSOLETE_REVIEW_REQUIRED",
  );

  const reviewedObsolete = first.mapping.members.filter((member) =>
    member.logicalPath.startsWith("subagents/"),
  );
  const approvedPlan = await createPlan(reduced, destination, {
    existingMapping: first.mapping,
    reviewedObsolete,
  });
  assert.equal(approvedPlan.deletions.length, 3);
  const result = await applyRestore({ snapshot: reduced, plan: approvedPlan });
  assert.equal(
    result.mapping.members.some((member) => member.logicalPath.startsWith("subagents/")),
    false,
  );
  await assert.rejects(
    readFile(
      path.join(
        destination.projectsRoot,
        BUCKET,
        SESSION_ID,
        "subagents",
        `${AGENT_STEM}.jsonl`,
      ),
    ),
    (error) => error.code === "ENOENT",
  );
});

test("rejects conflicting sessionId and session_id fields", async (t) => {
  const source = await makeSandbox(t);
  await writeSession({ sandbox: source, stateSessionId: OTHER_SESSION_ID });
  await assert.rejects(
    capture(source),
    (error) => error.code === "SESSION_ID_CONFLICT",
  );
});

test("publishes an explicit machine-readable non-production verdict", () => {
  assert.deepEqual(POC_STATUS, {
    productionReady: false,
    verdict: "POC_ONLY",
    gates: {
      activeLeafAuthority: "UNPROVED",
      qoderAppConsumption: "UNPROVED",
      qoderApplicationIndexRebuild: "UNPROVED",
      crossPathIntegrity: "UNPROVED",
    },
  });
});

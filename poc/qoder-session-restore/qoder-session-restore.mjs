import { createHash, randomUUID } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  copyFile,
  lstat,
  mkdir,
  open,
  readFile,
  readdir,
  realpath,
  rename,
  rmdir,
  unlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { TextDecoder } from "node:util";

export const SYNTHETIC_MARKER_NAME = ".mallard-qoder-poc-synthetic";
export const SYNTHETIC_MARKER_CONTENT =
  "mallard-qoder-session-restore-poc-synthetic-only\n";

export const POC_STATUS = Object.freeze({
  productionReady: false,
  verdict: "POC_ONLY",
  gates: Object.freeze({
    activeLeafAuthority: "UNPROVED",
    qoderAppConsumption: "UNPROVED",
    qoderApplicationIndexRebuild: "UNPROVED",
    crossPathIntegrity: "UNPROVED",
  }),
});

export const DEFAULT_STRUCTURAL_PATH_RULES = Object.freeze([
  Object.freeze({
    logicalPath: "transcript/main.jsonl",
    recordType: "workspace-directories",
    path: Object.freeze(["directories", "*"]),
  }),
]);

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SAFE_BUCKET_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,254}$/;
const SAFE_SUBAGENT_PATTERN = /^(?:agent-[a-z0-9_-]+-[0-9a-f]{16}\.(?:jsonl|meta\.json)|task-[0-9a-f]{16}\.json)$/;
const KNOWN_RECORD_TYPES = new Set([
  "workspace-directories",
  "runtime-config",
  "attachment",
  "user",
  "assistant",
  "active-leaf",
  "last-prompt",
]);
const INTEGRITY_FIELD_PATTERN =
  /^(?:signature|hash|digest|replay[_-]?token|server[_-]?correlation)$/i;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

export class PocError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "PocError";
    this.code = code;
    this.details = details;
  }
}

function fail(code, message, details = undefined) {
  throw new PocError(code, message, details);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stableStringify(value) {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableStringify).join(",")}]`;
  }
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
    .join(",")}}`;
}

function sameValue(left, right) {
  return stableStringify(left) === stableStringify(right);
}

function validateUuid(value, label = "session UUID") {
  if (typeof value !== "string" || !UUID_PATTERN.test(value)) {
    fail("INVALID_SESSION_UUID", `${label} is not a canonical UUID.`);
  }
  return value.toLowerCase();
}

function validateBucket(value) {
  if (
    typeof value !== "string" ||
    !SAFE_BUCKET_PATTERN.test(value) ||
    value === "." ||
    value === ".." ||
    value.includes("/") ||
    value.includes("\\")
  ) {
    fail("INVALID_PROJECT_BUCKET", "The project bucket is not safe.");
  }
  return value;
}

function splitSafeRelative(relativePath) {
  if (typeof relativePath !== "string" || path.isAbsolute(relativePath)) {
    fail("UNSAFE_RELATIVE_PATH", "A relative path is required.");
  }
  const parts = relativePath.split("/");
  if (
    parts.length === 0 ||
    parts.some(
      (part) =>
        part.length === 0 ||
        part === "." ||
        part === ".." ||
        part.includes("\\") ||
        part.includes("\0"),
    )
  ) {
    fail("UNSAFE_RELATIVE_PATH", "The relative path is not safe.", {
      relativePath,
    });
  }
  return parts;
}

function safeJoin(root, relativePath) {
  const parts = splitSafeRelative(relativePath);
  const resolvedRoot = path.resolve(root);
  const target = path.resolve(resolvedRoot, ...parts);
  if (target === resolvedRoot || !target.startsWith(`${resolvedRoot}${path.sep}`)) {
    fail("PATH_ESCAPE", "The path escapes its approved root.");
  }
  return target;
}

function relativeInside(root, target) {
  const relative = path.relative(path.resolve(root), path.resolve(target));
  if (
    relative === "" ||
    relative === ".." ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  ) {
    fail("PATH_ESCAPE", "The path is not below the synthetic sandbox.");
  }
  return relative;
}

async function assertSyntheticSandbox(sandboxRoot) {
  const root = path.resolve(sandboxRoot);
  if (!path.basename(root).startsWith("mallard-qoder-poc-")) {
    fail(
      "SYNTHETIC_SANDBOX_REQUIRED",
      "The sandbox name must start with mallard-qoder-poc-.",
    );
  }
  let rootStat;
  try {
    rootStat = await lstat(root);
  } catch (error) {
    if (error?.code === "ENOENT") {
      fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic sandbox does not exist.");
    }
    throw error;
  }
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) {
    fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic sandbox is not a real directory.");
  }
  if ((await realpath(root)) !== root) {
    fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic sandbox path is indirect.");
  }
  const markerPath = path.join(root, SYNTHETIC_MARKER_NAME);
  let markerStat;
  try {
    markerStat = await lstat(markerPath);
  } catch (error) {
    if (error?.code === "ENOENT") {
      fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic marker is absent.");
    }
    throw error;
  }
  if (!markerStat.isFile() || markerStat.isSymbolicLink()) {
    fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic marker is not a regular file.");
  }
  if ((await readFile(markerPath, "utf8")) !== SYNTHETIC_MARKER_CONTENT) {
    fail("SYNTHETIC_SANDBOX_REQUIRED", "The synthetic marker is invalid.");
  }
  return root;
}

async function assertNoSymlinkComponents(sandboxRoot, targetPath, options = {}) {
  const root = await assertSyntheticSandbox(sandboxRoot);
  const relative = relativeInside(root, targetPath);
  const parts = relative.split(path.sep);
  let current = root;
  for (let index = 0; index < parts.length; index += 1) {
    current = path.join(current, parts[index]);
    let stat;
    try {
      stat = await lstat(current);
    } catch (error) {
      if (error?.code === "ENOENT" && options.allowMissingTail) {
        return;
      }
      throw error;
    }
    if (stat.isSymbolicLink()) {
      fail("SYMLINK_REJECTED", "A path component is a symbolic link.", {
        relativePath: path.relative(root, current),
      });
    }
    if (index < parts.length - 1 && !stat.isDirectory()) {
      fail("UNSAFE_FILE_TYPE", "An intermediate path is not a directory.");
    }
  }
}

function metadataFromStat(stat) {
  return {
    dev: stat.dev.toString(),
    ino: stat.ino.toString(),
    mode: stat.mode.toString(),
    nlink: stat.nlink.toString(),
    size: stat.size.toString(),
    mtimeNs: stat.mtimeNs.toString(),
    ctimeNs: stat.ctimeNs.toString(),
  };
}

function assertStableMetadata(before, after, label) {
  if (!sameValue(before, after)) {
    fail("SNAPSHOT_CHANGED", `${label} changed during capture.`, {
      before,
      after,
    });
  }
}

async function readNoFollow(sandboxRoot, filePath) {
  if (!fsConstants.O_NOFOLLOW) {
    fail(
      "NATIVE_NOFOLLOW_UNAVAILABLE",
      "This platform does not expose O_NOFOLLOW to Node.js.",
    );
  }
  await assertNoSymlinkComponents(sandboxRoot, filePath);
  let handle;
  try {
    handle = await open(
      filePath,
      fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW,
    );
  } catch (error) {
    if (error?.code === "ELOOP") {
      fail("SYMLINK_REJECTED", "The file is a symbolic link.");
    }
    throw error;
  }
  try {
    const beforeStat = await handle.stat({ bigint: true });
    if (!beforeStat.isFile()) {
      fail("UNSAFE_FILE_TYPE", "A snapshot member is not a regular file.");
    }
    const before = metadataFromStat(beforeStat);
    const bytes = await handle.readFile();
    const afterStat = await handle.stat({ bigint: true });
    const after = metadataFromStat(afterStat);
    assertStableMetadata(before, after, "A snapshot member");
    if (BigInt(bytes.length) !== beforeStat.size) {
      fail("SNAPSHOT_CHANGED", "A snapshot member size changed during capture.");
    }
    return {
      bytes,
      digest: sha256(bytes),
      identity: { dev: before.dev, ino: before.ino },
      metadata: before,
    };
  } finally {
    await handle.close();
  }
}

async function requireDirectory(sandboxRoot, directoryPath) {
  await assertNoSymlinkComponents(sandboxRoot, directoryPath);
  const stat = await lstat(directoryPath);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail("UNSAFE_FILE_TYPE", "An expected directory is not a real directory.");
  }
}

async function requireRegularFile(sandboxRoot, filePath) {
  await assertNoSymlinkComponents(sandboxRoot, filePath);
  const stat = await lstat(filePath);
  if (!stat.isFile() || stat.isSymbolicLink()) {
    fail("UNSAFE_FILE_TYPE", "An expected file is not a regular file.");
  }
}

async function enumerateMembers({ sandboxRoot, projectsRoot, projectBucket, sessionId }) {
  const root = await assertSyntheticSandbox(sandboxRoot);
  relativeInside(root, projectsRoot);
  const bucket = validateBucket(projectBucket);
  const id = validateUuid(sessionId);
  const bucketPath = safeJoin(projectsRoot, bucket);
  const mainPath = path.join(bucketPath, `${id}.jsonl`);
  const sessionPath = path.join(bucketPath, id);
  await requireDirectory(root, projectsRoot);
  await requireDirectory(root, bucketPath);
  await requireRegularFile(root, mainPath);
  await requireDirectory(root, sessionPath);

  const topEntries = await readdir(sessionPath, { withFileTypes: true });
  const topNames = topEntries.map((entry) => entry.name).sort();
  for (const entry of topEntries) {
    if (!["state.json", "compression-v2", "subagents"].includes(entry.name)) {
      fail("UNEXPECTED_SIDECAR", "The session contains an unreviewed sidecar path.", {
        relativePath: entry.name,
      });
    }
    if (entry.isSymbolicLink()) {
      fail("SYMLINK_REJECTED", "A sidecar entry is a symbolic link.");
    }
  }
  if (!topNames.includes("state.json")) {
    fail("INCOMPLETE_SIDECARS", "The required state.json sidecar is absent.");
  }

  const members = [
    {
      logicalPath: "transcript/main.jsonl",
      sourcePath: mainPath,
      sourceRelativePath: `${bucket}/${id}.jsonl`,
      format: "jsonl",
    },
    {
      logicalPath: "state/state.json",
      sourcePath: path.join(sessionPath, "state.json"),
      sourceRelativePath: `${bucket}/${id}/state.json`,
      format: "json",
    },
  ];
  await requireRegularFile(root, path.join(sessionPath, "state.json"));

  if (topNames.includes("compression-v2")) {
    const compressionPath = path.join(sessionPath, "compression-v2");
    await requireDirectory(root, compressionPath);
    const entries = await readdir(compressionPath, { withFileTypes: true });
    if (
      entries.length !== 1 ||
      entries[0].name !== "state.json" ||
      entries[0].isSymbolicLink()
    ) {
      fail("UNEXPECTED_SIDECAR", "The compression-v2 member set is not allowlisted.");
    }
    const compressionState = path.join(compressionPath, "state.json");
    await requireRegularFile(root, compressionState);
    members.push({
      logicalPath: "state/compression-v2/state.json",
      sourcePath: compressionState,
      sourceRelativePath: `${bucket}/${id}/compression-v2/state.json`,
      format: "json",
    });
  }

  if (topNames.includes("subagents")) {
    const subagentsPath = path.join(sessionPath, "subagents");
    await requireDirectory(root, subagentsPath);
    const entries = await readdir(subagentsPath, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      if (entry.isSymbolicLink()) {
        fail("SYMLINK_REJECTED", "A subagent sidecar is a symbolic link.");
      }
      if (!entry.isFile() || !SAFE_SUBAGENT_PATTERN.test(entry.name)) {
        fail("UNEXPECTED_SIDECAR", "A subagent sidecar is not allowlisted.", {
          relativePath: entry.name,
        });
      }
      const sourcePath = path.join(subagentsPath, entry.name);
      await requireRegularFile(root, sourcePath);
      members.push({
        logicalPath: `subagents/${entry.name}`,
        sourcePath,
        sourceRelativePath: `${bucket}/${id}/subagents/${entry.name}`,
        format: entry.name.endsWith(".jsonl") ? "jsonl" : "json",
      });
    }
  }

  members.sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));
  return members;
}

function memberShape(member) {
  return {
    logicalPath: member.logicalPath,
    sourceRelativePath: member.sourceRelativePath,
    format: member.format,
  };
}

function assertSameMembership(first, second) {
  const firstShape = first.map(memberShape);
  const secondShape = second.map(memberShape);
  if (!sameValue(firstShape, secondShape)) {
    fail("SNAPSHOT_CHANGED", "The snapshot member set changed between passes.", {
      first: firstShape,
      second: secondShape,
    });
  }
}

function decodeUtf8(bytes, logicalPath) {
  try {
    return UTF8_DECODER.decode(bytes);
  } catch {
    fail("INVALID_UTF8", "A JSON member is not valid UTF-8.", { logicalPath });
  }
}

function jsonlSegments(text, logicalPath) {
  const segments = [];
  let cursor = 0;
  let lineNumber = 1;
  while (cursor < text.length) {
    const newline = text.indexOf("\n", cursor);
    const segmentEnd = newline === -1 ? text.length : newline + 1;
    const raw = text.slice(cursor, segmentEnd);
    const ending = raw.endsWith("\r\n") ? "\r\n" : raw.endsWith("\n") ? "\n" : "";
    const body = ending.length === 0 ? raw : raw.slice(0, -ending.length);
    if (body.trim().length === 0) {
      fail("INVALID_JSONL", "A JSONL member contains an empty record.", {
        logicalPath,
        lineNumber,
      });
    }
    let value;
    try {
      value = JSON.parse(body);
    } catch {
      fail("INVALID_JSONL", "A JSONL record is malformed.", {
        logicalPath,
        lineNumber,
      });
    }
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      fail("INVALID_JSONL", "A JSONL record must be an object.", {
        logicalPath,
        lineNumber,
      });
    }
    segments.push({ body, ending, value, lineNumber });
    cursor = segmentEnd;
    lineNumber += 1;
  }
  if (segments.length === 0) {
    fail("INVALID_JSONL", "A JSONL member is empty.", { logicalPath });
  }
  return segments;
}

function parseMember(member) {
  const text = decodeUtf8(member.bytes, member.logicalPath);
  if (member.format === "jsonl") {
    return jsonlSegments(text, member.logicalPath).map((segment) => ({
      value: segment.value,
      lineNumber: segment.lineNumber,
    }));
  }
  try {
    const value = JSON.parse(text);
    if (value === null || typeof value !== "object") {
      fail("INVALID_JSON", "A JSON sidecar must contain an object or array.", {
        logicalPath: member.logicalPath,
      });
    }
    return [{ value, lineNumber: null }];
  } catch (error) {
    if (error instanceof PocError) {
      throw error;
    }
    fail("INVALID_JSON", "A JSON sidecar is malformed.", {
      logicalPath: member.logicalPath,
    });
  }
}

function walkJson(value, jsonPath, visitor) {
  if (Array.isArray(value)) {
    value.forEach((item, index) => walkJson(item, [...jsonPath, index], visitor));
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      visitor({ key, value: child, path: [...jsonPath, key] });
      walkJson(child, [...jsonPath, key], visitor);
    }
  }
}

function analyzeSnapshot(snapshot) {
  const sourceId = snapshot.sessionId;
  const knownOccurrences = [
    { location: "root-filename", value: sourceId },
    { location: "session-directory", value: sourceId },
  ];
  const hiddenReferences = [];
  const integrityFields = [];
  const mainRecords = [];

  for (const member of snapshot.members) {
    const documents = parseMember(member);
    documents.forEach((document, documentIndex) => {
      const recordType =
        typeof document.value?.type === "string" ? document.value.type : null;
      const unknownRecord =
        member.format === "jsonl" && !KNOWN_RECORD_TYPES.has(recordType);
      if (member.logicalPath === "transcript/main.jsonl") {
        mainRecords.push({
          value: document.value,
          recordType,
          lineNumber: document.lineNumber,
        });
      }
      walkJson(document.value, [], ({ key, value, path: valuePath }) => {
        const location = {
          logicalPath: member.logicalPath,
          documentIndex,
          lineNumber: document.lineNumber,
          path: valuePath,
          recordType,
        };
        if (key === "sessionId" || key === "session_id") {
          if (typeof value !== "string" || value.toLowerCase() !== sourceId) {
            fail(
              "SESSION_ID_CONFLICT",
              "A known session-ID field conflicts with the source UUID.",
              { ...location, observed: value },
            );
          }
          knownOccurrences.push({ ...location, value: value.toLowerCase() });
          return;
        }
        if (INTEGRITY_FIELD_PATTERN.test(key)) {
          integrityFields.push(location);
        }
        if (typeof value === "string" && value.toLowerCase().includes(sourceId)) {
          hiddenReferences.push({
            ...location,
            unknownRecord,
          });
        }
      });
    });
  }

  const candidates = mainRecords
    .filter((record) => record.recordType === "active-leaf")
    .map((record, index) => {
      const value = record.value;
      const candidateId = [
        value.leafUuid,
        value.leaf_uuid,
        value.messageUuid,
        value.message_uuid,
        value.uuid,
      ].find((candidate) => typeof candidate === "string");
      return {
        candidateIndex: index,
        lineNumber: record.lineNumber,
        candidateId: candidateId ?? null,
        timestamp:
          typeof value.timestamp === "string" || typeof value.timestamp === "number"
            ? value.timestamp
            : null,
        explicit: typeof value.explicit === "boolean" ? value.explicit : null,
      };
    });

  return {
    uuid: {
      sourceSessionId: sourceId,
      knownOccurrences,
      hiddenReferences,
      replacementPermitted: false,
      replacementBlockers: [
        "Source UUID preservation is mandatory in this PoC.",
        ...(hiddenReferences.length > 0
          ? ["Opaque fields contain the source UUID and do not have a typed rewrite rule."]
          : []),
      ],
    },
    activeLeaf: {
      authority: "UNPROVED",
      authoritativeCandidate: null,
      candidates,
      warning:
        "The records are candidates only. File order, timestamp, and explicit do not select an authoritative leaf in this PoC.",
    },
    integrityFields,
  };
}

function snapshotDigest(sessionId, projectBucket, members) {
  return sha256(
    Buffer.from(
      stableStringify({
        sessionId,
        projectBucket,
        members: members.map((member) => ({
          logicalPath: member.logicalPath,
          format: member.format,
          digest: member.digest,
          identity: member.identity,
          metadata: member.metadata,
        })),
      }),
    ),
  );
}

export async function captureSessionGroup({
  sandboxRoot,
  projectsRoot,
  projectBucket,
  sessionId,
  hooks = {},
}) {
  const root = await assertSyntheticSandbox(sandboxRoot);
  const id = validateUuid(sessionId);
  const firstMembership = await enumerateMembers({
    sandboxRoot: root,
    projectsRoot,
    projectBucket,
    sessionId: id,
  });
  const first = [];
  for (const member of firstMembership) {
    first.push({ ...member, ...(await readNoFollow(root, member.sourcePath)) });
  }
  if (hooks.afterFirstPass) {
    await hooks.afterFirstPass({
      sandboxRoot: root,
      projectsRoot,
      projectBucket,
      sessionId: id,
      members: first.map((member) => ({ ...member, bytes: Buffer.from(member.bytes) })),
    });
  }
  const secondMembership = await enumerateMembers({
    sandboxRoot: root,
    projectsRoot,
    projectBucket,
    sessionId: id,
  });
  assertSameMembership(firstMembership, secondMembership);
  if (hooks.afterSecondEnumeration) {
    await hooks.afterSecondEnumeration({
      sandboxRoot: root,
      projectsRoot,
      projectBucket,
      sessionId: id,
    });
  }
  const second = [];
  for (const member of secondMembership) {
    second.push({ ...member, ...(await readNoFollow(root, member.sourcePath)) });
  }
  const finalMembership = await enumerateMembers({
    sandboxRoot: root,
    projectsRoot,
    projectBucket,
    sessionId: id,
  });
  assertSameMembership(secondMembership, finalMembership);
  for (let index = 0; index < first.length; index += 1) {
    const firstMember = first[index];
    const secondMember = second[index];
    if (
      firstMember.digest !== secondMember.digest ||
      !sameValue(firstMember.identity, secondMember.identity) ||
      !sameValue(firstMember.metadata, secondMember.metadata)
    ) {
      fail("SNAPSHOT_CHANGED", "A snapshot member changed between passes.", {
        logicalPath: firstMember.logicalPath,
      });
    }
  }

  const snapshot = {
    version: 1,
    sandboxRoot: root,
    projectsRoot: path.resolve(projectsRoot),
    projectBucket: validateBucket(projectBucket),
    sessionId: id,
    members: second.map((member) => ({
      logicalPath: member.logicalPath,
      sourceRelativePath: member.sourceRelativePath,
      format: member.format,
      bytes: Buffer.from(member.bytes),
      digest: member.digest,
      identity: member.identity,
      metadata: member.metadata,
    })),
  };
  snapshot.digest = snapshotDigest(
    snapshot.sessionId,
    snapshot.projectBucket,
    snapshot.members,
  );
  snapshot.analysis = analyzeSnapshot(snapshot);
  return snapshot;
}

function parseStringToken(text, start) {
  let cursor = start + 1;
  while (cursor < text.length) {
    const character = text[cursor];
    if (character === "\\") {
      cursor += 2;
      continue;
    }
    if (character === '"') {
      const end = cursor + 1;
      return {
        start,
        end,
        value: JSON.parse(text.slice(start, end)),
      };
    }
    cursor += 1;
  }
  fail("INVALID_JSON", "A JSON string is not terminated.");
}

function collectStringValueTokens(text) {
  let cursor = 0;
  const tokens = [];
  const skipWhitespace = () => {
    while (/\s/.test(text[cursor] ?? "")) {
      cursor += 1;
    }
  };
  const parseValue = (jsonPath) => {
    skipWhitespace();
    const character = text[cursor];
    if (character === '"') {
      const token = parseStringToken(text, cursor);
      tokens.push({ ...token, path: jsonPath });
      cursor = token.end;
      return;
    }
    if (character === "{") {
      cursor += 1;
      skipWhitespace();
      if (text[cursor] === "}") {
        cursor += 1;
        return;
      }
      while (cursor < text.length) {
        skipWhitespace();
        if (text[cursor] !== '"') {
          fail("INVALID_JSON", "A JSON object key is malformed.");
        }
        const keyToken = parseStringToken(text, cursor);
        cursor = keyToken.end;
        skipWhitespace();
        if (text[cursor] !== ":") {
          fail("INVALID_JSON", "A JSON object separator is absent.");
        }
        cursor += 1;
        parseValue([...jsonPath, keyToken.value]);
        skipWhitespace();
        if (text[cursor] === "}") {
          cursor += 1;
          return;
        }
        if (text[cursor] !== ",") {
          fail("INVALID_JSON", "A JSON object delimiter is malformed.");
        }
        cursor += 1;
      }
      fail("INVALID_JSON", "A JSON object is not terminated.");
    }
    if (character === "[") {
      cursor += 1;
      skipWhitespace();
      if (text[cursor] === "]") {
        cursor += 1;
        return;
      }
      let index = 0;
      while (cursor < text.length) {
        parseValue([...jsonPath, index]);
        index += 1;
        skipWhitespace();
        if (text[cursor] === "]") {
          cursor += 1;
          return;
        }
        if (text[cursor] !== ",") {
          fail("INVALID_JSON", "A JSON array delimiter is malformed.");
        }
        cursor += 1;
      }
      fail("INVALID_JSON", "A JSON array is not terminated.");
    }
    const literal = text.slice(cursor).match(/^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/);
    if (!literal) {
      fail("INVALID_JSON", "A JSON value is malformed.");
    }
    cursor += literal[0].length;
  };
  parseValue([]);
  skipWhitespace();
  if (cursor !== text.length) {
    fail("INVALID_JSON", "A JSON document has trailing data.");
  }
  return tokens;
}

function validateStructuralRule(rule) {
  if (
    !rule ||
    typeof rule.logicalPath !== "string" ||
    !Array.isArray(rule.path) ||
    rule.path.length === 0 ||
    rule.path.some(
      (segment) =>
        segment !== "*" && typeof segment !== "string" && !Number.isInteger(segment),
    ) ||
    (rule.recordType !== undefined && typeof rule.recordType !== "string")
  ) {
    fail("INVALID_PATH_RULE", "A structural path rule is malformed.");
  }
}

function ruleMatches(rule, logicalPath, recordType, tokenPath) {
  if (rule.logicalPath !== logicalPath) {
    return false;
  }
  if (rule.recordType !== undefined && rule.recordType !== recordType) {
    return false;
  }
  if (rule.path.length !== tokenPath.length) {
    return false;
  }
  return rule.path.every(
    (segment, index) => segment === "*" || segment === tokenPath[index],
  );
}

function mapPathValue(value, from, to) {
  if (value === from) {
    return to;
  }
  if (
    value.startsWith(from) &&
    (value[from.length] === "/" || value[from.length] === "\\")
  ) {
    return `${to}${value.slice(from.length)}`;
  }
  return value;
}

function rewriteJsonDocument(text, logicalPath, rules, pathMapping) {
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    fail("INVALID_JSON", "A JSON document is malformed during path mapping.", {
      logicalPath,
    });
  }
  const recordType = typeof value?.type === "string" ? value.type : null;
  const tokens = collectStringValueTokens(text);
  const patches = [];
  for (const token of tokens) {
    if (
      rules.some((rule) => ruleMatches(rule, logicalPath, recordType, token.path))
    ) {
      const mapped = mapPathValue(token.value, pathMapping.from, pathMapping.to);
      if (mapped !== token.value) {
        patches.push({
          start: token.start,
          end: token.end,
          replacement: JSON.stringify(mapped),
          path: token.path,
        });
      }
    }
  }
  let output = text;
  for (const patch of patches.sort((left, right) => right.start - left.start)) {
    output = `${output.slice(0, patch.start)}${patch.replacement}${output.slice(patch.end)}`;
  }
  return { text: output, rewrites: patches.map((patch) => patch.path) };
}

function rewriteMember(member, pathMapping, rules) {
  if (!pathMapping) {
    return { bytes: Buffer.from(member.bytes), rewrites: [] };
  }
  if (
    typeof pathMapping.from !== "string" ||
    pathMapping.from.length === 0 ||
    typeof pathMapping.to !== "string" ||
    pathMapping.to.length === 0
  ) {
    fail("INVALID_PATH_MAP", "A path map requires non-empty from and to values.");
  }
  const text = decodeUtf8(member.bytes, member.logicalPath);
  if (member.format === "jsonl") {
    const segments = jsonlSegments(text, member.logicalPath);
    const rewrites = [];
    const output = segments
      .map((segment) => {
        const rewritten = rewriteJsonDocument(
          segment.body,
          member.logicalPath,
          rules,
          pathMapping,
        );
        rewrites.push(
          ...rewritten.rewrites.map((jsonPath) => ({
            lineNumber: segment.lineNumber,
            path: jsonPath,
          })),
        );
        return `${rewritten.text}${segment.ending}`;
      })
      .join("");
    return { bytes: Buffer.from(output), rewrites };
  }
  const rewritten = rewriteJsonDocument(text, member.logicalPath, rules, pathMapping);
  return {
    bytes: Buffer.from(rewritten.text),
    rewrites: rewritten.rewrites.map((jsonPath) => ({ lineNumber: null, path: jsonPath })),
  };
}

export function transformSnapshot(
  snapshot,
  { pathMapping = null, structuralPathRules = DEFAULT_STRUCTURAL_PATH_RULES } = {},
) {
  for (const rule of structuralPathRules) {
    validateStructuralRule(rule);
  }
  if (
    snapshot.digest !==
    snapshotDigest(snapshot.sessionId, snapshot.projectBucket, snapshot.members)
  ) {
    fail("SOURCE_SNAPSHOT_CHANGED", "The in-memory source snapshot changed after capture.");
  }
  const currentAnalysis = analyzeSnapshot(snapshot);
  if (pathMapping && currentAnalysis.integrityFields.length > 0) {
    fail(
      "UNPROVED_INTEGRITY_FIELD",
      "Path mapping is blocked because an accepted record has an unproved integrity field.",
      { fields: currentAnalysis.integrityFields },
    );
  }
  const members = snapshot.members.map((member) => {
    if (sha256(member.bytes) !== member.digest) {
      fail("SOURCE_SNAPSHOT_CHANGED", "A source member changed after capture.", {
        logicalPath: member.logicalPath,
      });
    }
    const rewritten = rewriteMember(member, pathMapping, structuralPathRules);
    return {
      logicalPath: member.logicalPath,
      format: member.format,
      bytes: rewritten.bytes,
      digest: sha256(rewritten.bytes),
      size: rewritten.bytes.length,
      rewrites: rewritten.rewrites,
    };
  });
  return {
    sourceSnapshotDigest: snapshot.digest,
    pathMapping,
    structuralPathRules: structuralPathRules.map((rule) => ({
      logicalPath: rule.logicalPath,
      ...(rule.recordType === undefined ? {} : { recordType: rule.recordType }),
      path: [...rule.path],
    })),
    members,
  };
}

function targetRelativePath(projectBucket, sessionId, logicalPath) {
  if (logicalPath === "transcript/main.jsonl") {
    return `${projectBucket}/${sessionId}.jsonl`;
  }
  if (logicalPath === "state/state.json") {
    return `${projectBucket}/${sessionId}/state.json`;
  }
  if (logicalPath === "state/compression-v2/state.json") {
    return `${projectBucket}/${sessionId}/compression-v2/state.json`;
  }
  if (logicalPath.startsWith("subagents/")) {
    const name = logicalPath.slice("subagents/".length);
    if (!SAFE_SUBAGENT_PATTERN.test(name)) {
      fail("UNSAFE_RELATIVE_PATH", "A subagent target name is not safe.");
    }
    return `${projectBucket}/${sessionId}/subagents/${name}`;
  }
  fail("UNEXPECTED_SIDECAR", "A logical member path has no target mapping.", {
    logicalPath,
  });
}

async function ensureDirectoryTree(sandboxRoot, directoryPath) {
  const root = await assertSyntheticSandbox(sandboxRoot);
  relativeInside(root, directoryPath);
  await assertNoSymlinkComponents(root, directoryPath, { allowMissingTail: true });
  await mkdir(directoryPath, { recursive: true, mode: 0o700 });
  await assertNoSymlinkComponents(root, directoryPath);
  const stat = await lstat(directoryPath);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    fail("UNSAFE_FILE_TYPE", "A created path is not a real directory.");
  }
}

async function listSessionCollisions({
  sandboxRoot,
  projectsRoot,
  sessionId,
}) {
  const root = await assertSyntheticSandbox(sandboxRoot);
  relativeInside(root, projectsRoot);
  await assertNoSymlinkComponents(root, projectsRoot, { allowMissingTail: true });
  let entries;
  try {
    entries = await readdir(projectsRoot, { withFileTypes: true });
  } catch (error) {
    if (error?.code === "ENOENT") {
      return [];
    }
    throw error;
  }
  const collisions = [];
  for (const entry of entries) {
    if (entry.isSymbolicLink()) {
      fail("SYMLINK_REJECTED", "A destination project bucket is a symbolic link.");
    }
    if (!entry.isDirectory()) {
      continue;
    }
    validateBucket(entry.name);
    const bucketPath = path.join(projectsRoot, entry.name);
    await requireDirectory(root, bucketPath);
    const bucketEntries = await readdir(bucketPath, { withFileTypes: true });
    for (const candidate of bucketEntries) {
      if (candidate.name === `${sessionId}.jsonl` || candidate.name === sessionId) {
        collisions.push(`${entry.name}/${candidate.name}`);
      }
    }
  }
  return collisions.sort();
}

function mappingMemberByLogicalPath(mapping) {
  const result = new Map();
  for (const member of mapping.members ?? []) {
    if (result.has(member.logicalPath)) {
      fail("INVALID_MAPPING", "A mapping contains a duplicate logical path.");
    }
    result.set(member.logicalPath, member);
  }
  return result;
}

function validateMapping(mapping, cloudTaskId, projectBucket, sessionId) {
  if (!mapping || mapping.version !== 1) {
    fail("INVALID_MAPPING", "An update requires a version-1 mapping.");
  }
  if (mapping.cloudTaskId !== cloudTaskId) {
    fail("MAPPING_TASK_MISMATCH", "The mapping belongs to another cloud task.");
  }
  if (
    mapping.sessionId !== sessionId ||
    mapping.projectBucket !== projectBucket
  ) {
    fail("MAPPING_SESSION_MISMATCH", "The mapping has a different destination session.");
  }
  mappingMemberByLogicalPath(mapping);
}

function assertCapturedStateMatchesMapping(captured, mapping, errorCode = "TARGET_CHANGED") {
  const expected = mappingMemberByLogicalPath(mapping);
  const observed = new Map(captured.members.map((member) => [member.logicalPath, member]));
  if (!sameValue([...expected.keys()].sort(), [...observed.keys()].sort())) {
    fail(errorCode, "The destination member set no longer matches its reviewed mapping.");
  }
  for (const [logicalPath, expectedMember] of expected) {
    const observedMember = observed.get(logicalPath);
    const expectedTarget = targetRelativePath(
      mapping.projectBucket,
      mapping.sessionId,
      logicalPath,
    );
    if (
      expectedMember.targetRelativePath !== expectedTarget ||
      expectedMember.digest !== observedMember.digest ||
      Number(expectedMember.size) !== observedMember.bytes.length ||
      !sameValue(expectedMember.identity, observedMember.identity)
    ) {
      fail(errorCode, "A destination member no longer matches its reviewed pins.", {
        logicalPath,
      });
    }
  }
}

async function captureMappedDestination({ sandboxRoot, projectsRoot, mapping }) {
  return captureSessionGroup({
    sandboxRoot,
    projectsRoot,
    projectBucket: mapping.projectBucket,
    sessionId: mapping.sessionId,
  });
}

function normalizeReviewedObsolete(reviewedObsolete = []) {
  const normalized = reviewedObsolete.map((member) => ({
    logicalPath: member.logicalPath,
    targetRelativePath: member.targetRelativePath,
    digest: member.digest,
    size: Number(member.size),
    identity: member.identity,
  }));
  normalized.sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));
  return normalized;
}

function planBody(plan) {
  const { planDigest, ...body } = plan;
  return body;
}

function calculatePlanDigest(plan) {
  return sha256(Buffer.from(stableStringify(planBody(plan))));
}

export async function planRestore({
  snapshot,
  destinationSandboxRoot,
  destinationProjectsRoot,
  operationRoot,
  destinationProjectBucket,
  cloudTaskId,
  existingMapping = null,
  allMappings = [],
  reviewedObsolete = [],
  pathMapping = null,
  structuralPathRules = DEFAULT_STRUCTURAL_PATH_RULES,
  now = Date.now(),
  ttlMs = 5 * 60 * 1000,
}) {
  const destinationRoot = await assertSyntheticSandbox(destinationSandboxRoot);
  relativeInside(destinationRoot, destinationProjectsRoot);
  relativeInside(destinationRoot, operationRoot);
  if (typeof cloudTaskId !== "string" || cloudTaskId.length === 0) {
    fail("INVALID_CLOUD_TASK", "A cloud task ID is required.");
  }
  const projectBucket = validateBucket(destinationProjectBucket);
  const sessionId = validateUuid(snapshot.sessionId);
  const transformed = transformSnapshot(snapshot, {
    pathMapping,
    structuralPathRules,
  });
  const mode = existingMapping ? "update" : "create";
  let expectedBefore = [];
  let deletions = [];

  if (mode === "create") {
    const mappingCollision = allMappings.some(
      (mapping) => mapping.sessionId === sessionId,
    );
    const pathCollisions = await listSessionCollisions({
      sandboxRoot: destinationRoot,
      projectsRoot: destinationProjectsRoot,
      sessionId,
    });
    if (mappingCollision || pathCollisions.length > 0) {
      fail(
        "SESSION_UUID_COLLISION",
        "The source UUID already exists at the destination. UUID remapping is not allowed.",
        { mappingCollision, pathCollisions },
      );
    }
    if (reviewedObsolete.length > 0) {
      fail("INVALID_DELETE_REVIEW", "A create plan cannot delete obsolete sidecars.");
    }
  } else {
    validateMapping(existingMapping, cloudTaskId, projectBucket, sessionId);
    const current = await captureMappedDestination({
      sandboxRoot: destinationRoot,
      projectsRoot: destinationProjectsRoot,
      mapping: existingMapping,
    });
    assertCapturedStateMatchesMapping(current, existingMapping);
    expectedBefore = existingMapping.members.map((member) => ({
      logicalPath: member.logicalPath,
      targetRelativePath: member.targetRelativePath,
      digest: member.digest,
      size: Number(member.size),
      identity: member.identity,
    }));
    expectedBefore.sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));

    const nextPaths = new Set(transformed.members.map((member) => member.logicalPath));
    deletions = expectedBefore.filter(
      (member) => !nextPaths.has(member.logicalPath),
    );
    if (deletions.some((member) => member.logicalPath === "transcript/main.jsonl")) {
      fail("UNSAFE_DELETE", "The main transcript cannot be an obsolete-sidecar deletion.");
    }
    const reviewed = normalizeReviewedObsolete(reviewedObsolete);
    if (!sameValue(deletions, reviewed)) {
      fail(
        "OBSOLETE_REVIEW_REQUIRED",
        "Every obsolete sidecar needs an exact reviewed digest and identity action.",
        { required: deletions },
      );
    }
  }

  const outputs = transformed.members.map((member) => ({
    logicalPath: member.logicalPath,
    targetRelativePath: targetRelativePath(projectBucket, sessionId, member.logicalPath),
    digest: member.digest,
    size: member.size,
    rewrites: member.rewrites,
  }));
  outputs.sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));

  const plan = {
    version: 1,
    planId: randomUUID(),
    mode,
    cloudTaskId,
    sourceSnapshotDigest: snapshot.digest,
    sessionId,
    destination: {
      sandboxRoot: destinationRoot,
      projectsRoot: path.resolve(destinationProjectsRoot),
      operationRoot: path.resolve(operationRoot),
      projectBucket,
    },
    pathMapping,
    structuralPathRules: transformed.structuralPathRules,
    outputs,
    expectedBefore,
    deletions,
    createdAtMs: now,
    expiresAtMs: now + ttlMs,
    productionStatus: POC_STATUS,
  };
  plan.planDigest = calculatePlanDigest(plan);
  return plan;
}

async function atomicWrite(filePath, bytes, mode = 0o600) {
  const temporary = path.join(
    path.dirname(filePath),
    `.${path.basename(filePath)}.${randomUUID()}.tmp`,
  );
  const handle = await open(temporary, "wx", mode);
  try {
    await handle.writeFile(bytes);
    await handle.sync();
  } finally {
    await handle.close();
  }
  try {
    await rename(temporary, filePath);
  } catch (error) {
    try {
      await unlink(temporary);
    } catch {
      // Preserve the original failure.
    }
    throw error;
  }
}

async function atomicWriteJson(filePath, value) {
  await atomicWrite(filePath, Buffer.from(`${JSON.stringify(value, null, 2)}\n`));
}

async function currentFilePin(sandboxRoot, filePath) {
  try {
    const captured = await readNoFollow(sandboxRoot, filePath);
    return {
      digest: captured.digest,
      size: captured.bytes.length,
      identity: captured.identity,
      bytes: captured.bytes,
    };
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function writeJournal(journalPath, journal) {
  await atomicWriteJson(journalPath, journal);
}

async function pruneKnownEmptyDirectories(sandboxRoot, projectsRoot, targetRelative) {
  const target = safeJoin(projectsRoot, targetRelative);
  const sessionDirectory = path.dirname(
    targetRelative.endsWith(".jsonl")
      ? target
      : safeJoin(projectsRoot, targetRelative.split("/").slice(0, 2).join("/")),
  );
  const candidates = [path.dirname(target), path.dirname(path.dirname(target))];
  for (const candidate of candidates) {
    if (
      candidate === path.resolve(projectsRoot) ||
      candidate === sessionDirectory ||
      !candidate.startsWith(`${path.resolve(projectsRoot)}${path.sep}`)
    ) {
      continue;
    }
    await assertNoSymlinkComponents(sandboxRoot, candidate);
    try {
      await rmdir(candidate);
    } catch (error) {
      if (error?.code !== "ENOTEMPTY" && error?.code !== "ENOENT") {
        throw error;
      }
    }
  }
}

async function revalidatePlanTarget(plan) {
  if (plan.mode === "create") {
    const collisions = await listSessionCollisions({
      sandboxRoot: plan.destination.sandboxRoot,
      projectsRoot: plan.destination.projectsRoot,
      sessionId: plan.sessionId,
    });
    if (collisions.length > 0) {
      fail("SESSION_UUID_COLLISION", "The destination UUID appeared after review.", {
        collisions,
      });
    }
    return;
  }
  const mapping = {
    version: 1,
    cloudTaskId: plan.cloudTaskId,
    sessionId: plan.sessionId,
    projectBucket: plan.destination.projectBucket,
    members: plan.expectedBefore,
  };
  const current = await captureMappedDestination({
    sandboxRoot: plan.destination.sandboxRoot,
    projectsRoot: plan.destination.projectsRoot,
    mapping,
  });
  assertCapturedStateMatchesMapping(current, mapping);
}

async function rollbackApplied({ plan, applied, backupRoot, journal, journalPath }) {
  const rollbackFailures = [];
  for (const operation of [...applied].reverse()) {
    const target = safeJoin(plan.destination.projectsRoot, operation.targetRelativePath);
    try {
      if (operation.kind === "create") {
        const current = await currentFilePin(plan.destination.sandboxRoot, target);
        if (!current || current.digest !== operation.appliedDigest) {
          fail("ROLLBACK_PIN_MISMATCH", "A created target changed before rollback.");
        }
        await unlink(target);
      } else if (operation.kind === "overwrite") {
        const current = await currentFilePin(plan.destination.sandboxRoot, target);
        if (!current || current.digest !== operation.appliedDigest) {
          fail("ROLLBACK_PIN_MISMATCH", "An overwritten target changed before rollback.");
        }
        const backup = safeJoin(backupRoot, operation.targetRelativePath);
        await atomicWrite(target, await readFile(backup));
      } else if (operation.kind === "delete") {
        const current = await currentFilePin(plan.destination.sandboxRoot, target);
        if (current) {
          fail("ROLLBACK_PIN_MISMATCH", "A deleted target reappeared before rollback.");
        }
        await ensureDirectoryTree(plan.destination.sandboxRoot, path.dirname(target));
        const backup = safeJoin(backupRoot, operation.targetRelativePath);
        await atomicWrite(target, await readFile(backup));
      }
    } catch (error) {
      rollbackFailures.push({
        targetRelativePath: operation.targetRelativePath,
        code: error?.code ?? "ROLLBACK_FAILED",
      });
    }
  }
  journal.state = rollbackFailures.length === 0 ? "rolled_back" : "rollback_failed";
  journal.rollbackFailures = rollbackFailures;
  await writeJournal(journalPath, journal);
  if (rollbackFailures.length > 0) {
    fail("ROLLBACK_FAILED", "Restore failed and rollback was incomplete.", {
      rollbackFailures,
    });
  }
}

export async function applyRestore({ snapshot, plan, now = Date.now() }) {
  if (plan.planDigest !== calculatePlanDigest(plan)) {
    fail("PLAN_CHANGED", "The restore plan changed after review.");
  }
  if (now > plan.expiresAtMs) {
    fail("PLAN_EXPIRED", "The restore plan expired.");
  }
  const destinationRoot = await assertSyntheticSandbox(plan.destination.sandboxRoot);
  relativeInside(destinationRoot, plan.destination.projectsRoot);
  relativeInside(destinationRoot, plan.destination.operationRoot);
  if (snapshot.digest !== plan.sourceSnapshotDigest) {
    fail("SOURCE_SNAPSHOT_CHANGED", "The source snapshot does not match the plan.");
  }
  const transformed = transformSnapshot(snapshot, {
    pathMapping: plan.pathMapping,
    structuralPathRules: plan.structuralPathRules,
  });
  const outputPins = transformed.members
    .map((member) => ({
      logicalPath: member.logicalPath,
      targetRelativePath: targetRelativePath(
        plan.destination.projectBucket,
        plan.sessionId,
        member.logicalPath,
      ),
      digest: member.digest,
      size: member.size,
      rewrites: member.rewrites,
    }))
    .sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));
  if (!sameValue(outputPins, plan.outputs)) {
    fail("PLAN_CHANGED", "The planned output no longer matches the source snapshot.");
  }
  await revalidatePlanTarget(plan);

  const operationRoot = plan.destination.operationRoot;
  const stageRoot = safeJoin(operationRoot, `staging/${plan.planId}`);
  const backupRoot = safeJoin(operationRoot, `backups/${plan.planId}`);
  const journalDirectory = safeJoin(operationRoot, "journals");
  const journalPath = safeJoin(operationRoot, `journals/${plan.planId}.json`);
  await ensureDirectoryTree(destinationRoot, stageRoot);
  await ensureDirectoryTree(destinationRoot, backupRoot);
  await ensureDirectoryTree(destinationRoot, journalDirectory);

  const transformedByLogicalPath = new Map(
    transformed.members.map((member) => [member.logicalPath, member]),
  );
  for (const output of plan.outputs) {
    const stagePath = safeJoin(stageRoot, output.targetRelativePath);
    await ensureDirectoryTree(destinationRoot, path.dirname(stagePath));
    const member = transformedByLogicalPath.get(output.logicalPath);
    await atomicWrite(stagePath, member.bytes);
    const pin = await currentFilePin(destinationRoot, stagePath);
    if (!pin || pin.digest !== output.digest || pin.size !== output.size) {
      fail("STAGE_VERIFY_FAILED", "A staged member does not match its planned digest.");
    }
  }

  for (const expected of plan.expectedBefore) {
    const sourcePath = safeJoin(plan.destination.projectsRoot, expected.targetRelativePath);
    const backupPath = safeJoin(backupRoot, expected.targetRelativePath);
    await ensureDirectoryTree(destinationRoot, path.dirname(backupPath));
    const current = await currentFilePin(destinationRoot, sourcePath);
    if (
      !current ||
      current.digest !== expected.digest ||
      current.size !== expected.size ||
      !sameValue(current.identity, expected.identity)
    ) {
      fail("TARGET_CHANGED", "A target changed before backup.", {
        logicalPath: expected.logicalPath,
      });
    }
    await copyFile(sourcePath, backupPath, fsConstants.COPYFILE_EXCL);
    const backup = await currentFilePin(destinationRoot, backupPath);
    if (!backup || backup.digest !== expected.digest || backup.size !== expected.size) {
      fail("BACKUP_VERIFY_FAILED", "A backup does not match the reviewed target.");
    }
  }

  await revalidatePlanTarget(plan);
  const journal = {
    version: 1,
    planId: plan.planId,
    planDigest: plan.planDigest,
    state: "prepared",
    mode: plan.mode,
    sessionId: plan.sessionId,
    productionStatus: POC_STATUS,
    operations: [
      ...plan.outputs.map((output) => ({
        kind: plan.expectedBefore.some(
          (member) => member.logicalPath === output.logicalPath,
        )
          ? "overwrite"
          : "create",
        logicalPath: output.logicalPath,
        targetRelativePath: output.targetRelativePath,
        digest: output.digest,
        status: "pending",
      })),
      ...plan.deletions.map((deletion) => ({
        kind: "delete",
        logicalPath: deletion.logicalPath,
        targetRelativePath: deletion.targetRelativePath,
        digest: deletion.digest,
        status: "pending",
      })),
    ],
  };
  await writeJournal(journalPath, journal);
  journal.state = "applying";
  await writeJournal(journalPath, journal);

  const applied = [];
  try {
    const orderedOutputs = [...plan.outputs].sort((left, right) => {
      if (left.logicalPath === "transcript/main.jsonl") {
        return 1;
      }
      if (right.logicalPath === "transcript/main.jsonl") {
        return -1;
      }
      return left.logicalPath.localeCompare(right.logicalPath);
    });
    for (const output of orderedOutputs) {
      const operation = journal.operations.find(
        (candidate) =>
          candidate.kind !== "delete" &&
          candidate.logicalPath === output.logicalPath,
      );
      const target = safeJoin(plan.destination.projectsRoot, output.targetRelativePath);
      await ensureDirectoryTree(destinationRoot, path.dirname(target));
      const expected = plan.expectedBefore.find(
        (member) => member.logicalPath === output.logicalPath,
      );
      const current = await currentFilePin(destinationRoot, target);
      if (expected) {
        if (
          !current ||
          current.digest !== expected.digest ||
          current.size !== expected.size ||
          !sameValue(current.identity, expected.identity)
        ) {
          fail("TARGET_CHANGED", "A target changed immediately before overwrite.");
        }
      } else if (current) {
        fail("TARGET_APPEARED", "A new target appeared after review.");
      }
      const stagePath = safeJoin(stageRoot, output.targetRelativePath);
      await atomicWrite(target, await readFile(stagePath));
      const installed = await currentFilePin(destinationRoot, target);
      if (!installed || installed.digest !== output.digest || installed.size !== output.size) {
        fail("APPLY_VERIFY_FAILED", "An installed member does not match the plan.");
      }
      applied.push({
        kind: expected ? "overwrite" : "create",
        targetRelativePath: output.targetRelativePath,
        appliedDigest: output.digest,
      });
      operation.status = "applied";
      await writeJournal(journalPath, journal);
    }

    for (const deletion of plan.deletions) {
      const operation = journal.operations.find(
        (candidate) =>
          candidate.kind === "delete" &&
          candidate.logicalPath === deletion.logicalPath,
      );
      const target = safeJoin(plan.destination.projectsRoot, deletion.targetRelativePath);
      const current = await currentFilePin(destinationRoot, target);
      if (
        !current ||
        current.digest !== deletion.digest ||
        current.size !== deletion.size ||
        !sameValue(current.identity, deletion.identity)
      ) {
        fail("TARGET_CHANGED", "An obsolete sidecar changed before exact deletion.");
      }
      await unlink(target);
      applied.push({
        kind: "delete",
        targetRelativePath: deletion.targetRelativePath,
        appliedDigest: null,
      });
      operation.status = "applied";
      await writeJournal(journalPath, journal);
      await pruneKnownEmptyDirectories(
        destinationRoot,
        plan.destination.projectsRoot,
        deletion.targetRelativePath,
      );
    }
  } catch (error) {
    journal.failure = { code: error?.code ?? "APPLY_FAILED" };
    await rollbackApplied({ plan, applied, backupRoot, journal, journalPath });
    throw error;
  }

  const restored = await captureSessionGroup({
    sandboxRoot: destinationRoot,
    projectsRoot: plan.destination.projectsRoot,
    projectBucket: plan.destination.projectBucket,
    sessionId: plan.sessionId,
  });
  const restoredByLogicalPath = new Map(
    restored.members.map((member) => [member.logicalPath, member]),
  );
  if (
    !sameValue(
      [...restoredByLogicalPath.keys()].sort(),
      plan.outputs.map((output) => output.logicalPath).sort(),
    )
  ) {
    fail("APPLY_VERIFY_FAILED", "The restored member set differs from the reviewed set.");
  }
  for (const output of plan.outputs) {
    if (restoredByLogicalPath.get(output.logicalPath)?.digest !== output.digest) {
      fail("APPLY_VERIFY_FAILED", "A restored member digest differs from the plan.");
    }
  }

  const mapping = {
    version: 1,
    cloudTaskId: plan.cloudTaskId,
    sessionId: plan.sessionId,
    projectBucket: plan.destination.projectBucket,
    members: restored.members
      .map((member) => ({
        logicalPath: member.logicalPath,
        targetRelativePath: targetRelativePath(
          plan.destination.projectBucket,
          plan.sessionId,
          member.logicalPath,
        ),
        digest: member.digest,
        size: member.bytes.length,
        identity: member.identity,
      }))
      .sort((left, right) => left.logicalPath.localeCompare(right.logicalPath)),
  };
  journal.state = "committed";
  journal.mappingDigest = sha256(Buffer.from(stableStringify(mapping)));
  await writeJournal(journalPath, journal);
  return {
    mapping,
    journalPath,
    backupRoot,
    restored,
    productionStatus: POC_STATUS,
  };
}

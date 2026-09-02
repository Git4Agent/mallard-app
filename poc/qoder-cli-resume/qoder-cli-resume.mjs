#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  opendir,
  readFile,
  readlink,
  realpath,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { pathToFileURL } from "node:url";

export const POC_STATUS = Object.freeze({
  productionReady: false,
  verdict: "POC_ONLY",
});

export const SESSION_ID = "44444444-4444-4444-8444-444444444444";
export const SESSION_TITLE = "MALLARD_QODER_CLI_RESUME_SYNTHETIC";
export const PRIOR_ASSISTANT_MARKER = "MALLARD_QODER_PRIOR_ASSISTANT";
export const RESUME_PROMPT = "MALLARD_QODER_RESUME_AUTH_GATE_PROMPT";
export const CREDENTIAL_CANARY =
  "MALLARD_QODER_CREDENTIAL_CANARY_NEVER_EXPORT";

const QODER_CLI_PACKAGE_NAME = "@qoder-ai/qodercli";
const QODER_CLI_VERSION = "1.1.40";
const ROOT_PREFIX = "mallard-qoder-cli-resume-poc-";
const ROOT_MARKER_NAME = ".mallard-qoder-cli-resume-synthetic";
const ROOT_MARKER_CONTENT = "mallard-qoder-cli-resume-poc-v1\n";
const MAX_PROJECT_KEY_PREFIX = 200;
const MAX_MEMBER_BYTES = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES = 1024 * 1024;
const MAX_TREE_FILES = 10_000;
const MAX_TREE_BYTES = 128 * 1024 * 1024;
const MAX_TREE_FILE_BYTES = 32 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS = 30_000;
const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

const FIXTURE_MARKERS = Object.freeze([
  SESSION_TITLE,
  PRIOR_ASSISTANT_MARKER,
  RESUME_PROMPT,
]);

const AUTH_PATTERNS = Object.freeze([
  /not logged in/i,
  /login to authenticate/i,
  /authentication required/i,
  /please (?:log|sign) in/i,
  /access token (?:is )?(?:missing|required)/i,
]);

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
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
  return "unexpected_error";
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stableStringify(value) {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function validateUuid(value) {
  if (typeof value !== "string" || !UUID_PATTERN.test(value)) {
    fail("invalid_session_id");
  }
  return value.toLowerCase();
}

function validateTimeout(value) {
  const timeout = value ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeout) || timeout < 1_000 || timeout > 120_000) {
    fail("invalid_timeout");
  }
  return timeout;
}

function djb2Xor(value) {
  let hash = 5381;
  for (let index = 0; index < value.length; index += 1) {
    hash = (33 * hash) ^ value.charCodeAt(index);
  }
  return hash;
}

/**
 * Reproduces the project-key function observed in Qoder CLI 1.1.40.
 * The caller must supply an already-canonical absolute project path.
 * This is product observation, not a documented compatibility contract.
 */
export function computeCliProjectBucket(projectPath) {
  if (
    typeof projectPath !== "string" ||
    !isAbsolute(projectPath) ||
    projectPath.includes("\0")
  ) {
    fail("invalid_project_path");
  }
  const processed = projectPath.replace(/[^a-zA-Z0-9]/g, "-");
  if (processed.length <= MAX_PROJECT_KEY_PREFIX) {
    return processed;
  }
  const suffix = Math.abs(djb2Xor(projectPath)).toString(36);
  return `${processed.slice(0, MAX_PROJECT_KEY_PREFIX)}-${suffix}`;
}

function normalizeRelativePath(base, candidate) {
  const value = relative(base, candidate);
  if (
    value === "" ||
    value === ".." ||
    value.startsWith(`..${sep}`) ||
    isAbsolute(value)
  ) {
    fail("path_outside_isolation");
  }
  return value.split(sep).join("/");
}

async function validateCanonicalRegularFile(filePath, code) {
  if (
    typeof filePath !== "string" ||
    !isAbsolute(filePath) ||
    filePath.includes("\0")
  ) {
    fail(code);
  }
  const canonical = await realpath(filePath).catch(() => fail(code));
  if (canonical !== resolve(filePath)) {
    fail(code);
  }
  const metadata = await lstat(canonical).catch(() => fail(code));
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(code);
  }
  return canonical;
}

async function validateQoderCliBundle(filePath) {
  const entryPath = await validateCanonicalRegularFile(
    filePath,
    "invalid_cli_bundle",
  );
  if (
    basename(entryPath) !== "qodercli.js" ||
    basename(dirname(entryPath)) !== "bundle"
  ) {
    fail("unexpected_cli_entrypoint");
  }
  const packagePath = join(dirname(dirname(entryPath)), "package.json");
  await validateCanonicalRegularFile(packagePath, "invalid_cli_package");
  const packageBytes = await readFile(packagePath);
  if (packageBytes.length > 1024 * 1024) {
    fail("invalid_cli_package");
  }
  let packageDocument;
  try {
    packageDocument = JSON.parse(packageBytes.toString("utf8"));
  } catch {
    fail("invalid_cli_package");
  }
  if (packageDocument?.name !== QODER_CLI_PACKAGE_NAME) {
    fail("unexpected_cli_package");
  }
  if (packageDocument?.version !== QODER_CLI_VERSION) {
    fail("unexpected_cli_version");
  }
  return {
    entryPath,
    name: QODER_CLI_PACKAGE_NAME,
    version: QODER_CLI_VERSION,
  };
}

async function assertIsolationRoot(root) {
  if (typeof root !== "string" || !isAbsolute(root)) {
    fail("unsafe_isolation_root");
  }
  const canonicalRoot = await realpath(root).catch(() =>
    fail("unsafe_isolation_root"),
  );
  const canonicalTemp = await realpath(tmpdir()).catch(() =>
    fail("unsafe_isolation_root"),
  );
  const metadata = await lstat(canonicalRoot).catch(() =>
    fail("unsafe_isolation_root"),
  );
  if (
    canonicalRoot !== resolve(root) ||
    dirname(canonicalRoot) !== canonicalTemp ||
    !basename(canonicalRoot).startsWith(ROOT_PREFIX) ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink()
  ) {
    fail("unsafe_isolation_root");
  }
  const marker = join(canonicalRoot, ROOT_MARKER_NAME);
  const markerMetadata = await lstat(marker).catch(() =>
    fail("unsafe_isolation_root"),
  );
  if (!markerMetadata.isFile() || markerMetadata.isSymbolicLink()) {
    fail("unsafe_isolation_root");
  }
  if ((await readFile(marker, "utf8")) !== ROOT_MARKER_CONTENT) {
    fail("unsafe_isolation_root");
  }
  return canonicalRoot;
}

async function createIsolation(onIsolationRoot) {
  const created = await mkdtemp(join(tmpdir(), ROOT_PREFIX));
  const root = await realpath(created);
  try {
    await chmod(root, 0o700);
    await writeFile(join(root, ROOT_MARKER_NAME), ROOT_MARKER_CONTENT, {
      flag: "wx",
      mode: 0o600,
    });
    await assertIsolationRoot(root);
    const paths = {
      root,
      sourceRoot: join(root, "source-session-group"),
      stageRoot: join(root, "restore-stage"),
      projectRoot: join(root, "synthetic-project"),
      configRoot: join(root, "qoder-config"),
      homeRoot: join(root, "home"),
      tempRoot: join(root, "tmp"),
      runtimeRoot: join(root, "runtime"),
    };
    for (const directory of Object.values(paths).slice(1)) {
      await mkdir(directory, { mode: 0o700 });
    }
    onIsolationRoot?.(root);
    return paths;
  } catch (error) {
    await rm(root, { recursive: true, force: false }).catch(() => {});
    throw error;
  }
}

async function removeIsolation(root) {
  const canonicalRoot = await assertIsolationRoot(root);
  await rm(canonicalRoot, { recursive: true, force: false });
}

function jsonLine(value) {
  return `${JSON.stringify(value)}\n`;
}

export function createSyntheticMembers({
  projectRoot,
  sessionId = SESSION_ID,
} = {}) {
  if (
    typeof projectRoot !== "string" ||
    !isAbsolute(projectRoot) ||
    projectRoot.includes("\0")
  ) {
    fail("invalid_project_path");
  }
  const checkedSessionId = validateUuid(sessionId);
  const userId = "50000000-0000-4000-8000-000000000002";
  const assistantId = "50000000-0000-4000-8000-000000000003";
  const transcript = [
    {
      type: "workspace-directories",
      uuid: "50000000-0000-4000-8000-000000000001",
      sessionId: checkedSessionId,
      timestamp: "2026-01-02T00:00:00.000Z",
      cwd: projectRoot,
      directories: [projectRoot],
    },
    {
      type: "user",
      uuid: userId,
      sessionId: checkedSessionId,
      timestamp: "2026-01-02T00:00:01.000Z",
      message: { role: "user", content: SESSION_TITLE },
    },
    {
      type: "assistant",
      uuid: assistantId,
      sessionId: checkedSessionId,
      timestamp: "2026-01-02T00:00:02.000Z",
      parentUuid: userId,
      message: { role: "assistant", content: PRIOR_ASSISTANT_MARKER },
    },
    {
      type: "active-leaf",
      uuid: "50000000-0000-4000-8000-000000000004",
      sessionId: checkedSessionId,
      timestamp: "2026-01-02T00:00:03.000Z",
      leafUuid: assistantId,
      explicit: true,
    },
  ]
    .map((record) => jsonLine(record))
    .join("");

  const members = [
    {
      logicalPath: `${checkedSessionId}.jsonl`,
      bytes: Buffer.from(transcript),
    },
    {
      logicalPath: `${checkedSessionId}/state.json`,
      bytes: Buffer.from(
        `${JSON.stringify(
          {
            sessionId: checkedSessionId,
            cwd: projectRoot,
            activeLeafUuid: assistantId,
            workspaceDirectories: [projectRoot],
          },
          null,
          2,
        )}\n`,
      ),
    },
    {
      logicalPath: `${checkedSessionId}/compression-v2/state.json`,
      bytes: Buffer.from(
        jsonLine({ session_id: checkedSessionId, compacted: false }),
      ),
    },
    {
      logicalPath:
        `${checkedSessionId}/subagents/` +
        "agent-explore-0123456789abcdef.jsonl",
      bytes: Buffer.from(
        jsonLine({
          type: "user",
          session_id: checkedSessionId,
          message: "synthetic subagent request",
        }) +
          jsonLine({
            type: "assistant",
            sessionId: checkedSessionId,
            message: "synthetic subagent response",
          }),
      ),
    },
    {
      logicalPath:
        `${checkedSessionId}/subagents/` +
        "agent-explore-0123456789abcdef.meta.json",
      bytes: Buffer.from(
        jsonLine({ session_id: checkedSessionId, agent: "synthetic" }),
      ),
    },
    {
      logicalPath:
        `${checkedSessionId}/subagents/task-fedcba9876543210.json`,
      bytes: Buffer.from(
        jsonLine({ sessionId: checkedSessionId, task: "synthetic" }),
      ),
    },
  ].sort((left, right) => left.logicalPath.localeCompare(right.logicalPath));

  return members.map((member) => ({
    ...member,
    size: member.bytes.length,
    sha256: sha256(member.bytes),
  }));
}

async function ensureOwnedDirectory(root, directory) {
  if (resolve(root) !== resolve(directory)) {
    normalizeRelativePath(root, directory);
  }
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const canonical = await realpath(directory);
  if (resolve(root) !== canonical) {
    normalizeRelativePath(root, canonical);
  }
  if (canonical !== resolve(directory)) {
    fail("unsafe_directory_component");
  }
}

async function writeMembers(root, members) {
  for (const member of members) {
    const target = join(root, ...member.logicalPath.split("/"));
    await ensureOwnedDirectory(root, dirname(target));
    await writeFile(target, member.bytes, { flag: "wx", mode: 0o600 });
  }
}

async function readRegularNoFollow(root, filePath) {
  normalizeRelativePath(root, filePath);
  const handle = await open(
    filePath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  );
  try {
    const before = await handle.stat();
    if (!before.isFile() || before.size > MAX_MEMBER_BYTES) {
      fail("invalid_session_member");
    }
    const bytes = await handle.readFile();
    const after = await handle.stat();
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeNs !== after.mtimeNs ||
      before.ctimeNs !== after.ctimeNs
    ) {
      fail("session_member_changed_during_read");
    }
    return bytes;
  } finally {
    await handle.close();
  }
}

async function manifestForMembers(root, members) {
  const manifest = [];
  for (const member of members) {
    const filePath = join(root, ...member.logicalPath.split("/"));
    const bytes = await readRegularNoFollow(root, filePath);
    manifest.push({
      logicalPath: member.logicalPath,
      size: bytes.length,
      sha256: sha256(bytes),
    });
  }
  return manifest;
}

export async function restoreSyntheticSessionGroup({
  isolationRoot,
  sourceRoot,
  stageRoot,
  destinationBucketRoot,
  members,
}) {
  await assertIsolationRoot(isolationRoot);
  for (const directory of [sourceRoot, stageRoot, destinationBucketRoot]) {
    normalizeRelativePath(isolationRoot, directory);
  }
  await writeMembers(sourceRoot, members);
  const sourceManifest = await manifestForMembers(sourceRoot, members);
  const expectedManifest = members.map(({ logicalPath, size, sha256: digest }) => ({
    logicalPath,
    size,
    sha256: digest,
  }));
  if (stableStringify(sourceManifest) !== stableStringify(expectedManifest)) {
    fail("source_manifest_mismatch");
  }

  await writeMembers(stageRoot, members);
  const stagedManifest = await manifestForMembers(stageRoot, members);
  if (stableStringify(stagedManifest) !== stableStringify(expectedManifest)) {
    fail("staged_manifest_mismatch");
  }

  await ensureOwnedDirectory(isolationRoot, destinationBucketRoot);
  for (const member of members) {
    const staged = join(stageRoot, ...member.logicalPath.split("/"));
    const destination = join(
      destinationBucketRoot,
      ...member.logicalPath.split("/"),
    );
    await ensureOwnedDirectory(isolationRoot, dirname(destination));
    await rename(staged, destination);
  }
  await rm(stageRoot, { recursive: true, force: false });

  const restoredManifest = await manifestForMembers(
    destinationBucketRoot,
    members,
  );
  if (stableStringify(restoredManifest) !== stableStringify(expectedManifest)) {
    fail("restored_manifest_mismatch");
  }
  return {
    memberCount: members.length,
    bytes: members.reduce((sum, member) => sum + member.size, 0),
    manifestDigest: sha256(Buffer.from(stableStringify(expectedManifest))),
    verified: true,
  };
}

function scanBytes(bytes, sensitiveRoot) {
  const text = bytes.toString("utf8");
  const fixtureMarkerMatches = Object.fromEntries(
    FIXTURE_MARKERS.map((marker) => [
      marker,
      text.split(marker).length - 1,
    ]),
  );
  const credentialCanaryMatches = text.split(CREDENTIAL_CANARY).length - 1;
  const rootMatches = sensitiveRoot
    ? text.split(sensitiveRoot).length - 1
    : 0;
  const credentialShapePatterns = [
    /authorization["'\s]*[:=]["'\s]*bearer\s+[a-z0-9._~+\/-]{8,}/gi,
    /(?:access[_-]?token|refresh[_-]?token|api[_-]?key|password|secret)["'\s]*[:=]["'\s]+[^"'\s]{8,}/gi,
  ];
  const credentialShapeMatches = credentialShapePatterns.reduce(
    (count, pattern) => count + (text.match(pattern)?.length ?? 0),
    0,
  );
  return {
    fixtureMarkerMatches,
    credentialCanaryMatches,
    credentialShapeMatches,
    rootMatches,
  };
}

function combineScans(scans) {
  const combined = {
    fixtureMarkerMatches: Object.fromEntries(
      FIXTURE_MARKERS.map((marker) => [marker, 0]),
    ),
    credentialCanaryMatches: 0,
    credentialShapeMatches: 0,
    rootMatches: 0,
  };
  for (const scan of scans) {
    for (const marker of FIXTURE_MARKERS) {
      combined.fixtureMarkerMatches[marker] +=
        scan.fixtureMarkerMatches[marker] ?? 0;
    }
    combined.credentialCanaryMatches += scan.credentialCanaryMatches ?? 0;
    combined.credentialShapeMatches += scan.credentialShapeMatches ?? 0;
    combined.rootMatches += scan.rootMatches ?? 0;
  }
  return combined;
}

async function snapshotTree(root) {
  const entries = [];
  const scans = [];
  let fileCount = 0;
  let byteCount = 0;

  async function visit(directory) {
    const handle = await opendir(directory);
    const children = [];
    for await (const entry of handle) {
      children.push(entry.name);
    }
    children.sort();
    for (const name of children) {
      const fullPath = join(directory, name);
      const logicalPath = normalizeRelativePath(root, fullPath);
      const metadata = await lstat(fullPath, { bigint: true });
      const mode = Number(metadata.mode & 0o7777n);
      if (metadata.isSymbolicLink()) {
        const target = Buffer.from(await readlink(fullPath));
        entries.push({
          path: logicalPath,
          type: "symlink",
          mode,
          targetSha256: sha256(target),
        });
        scans.push(scanBytes(target, root));
        continue;
      }
      if (metadata.isDirectory()) {
        entries.push({ path: logicalPath, type: "directory", mode });
        await visit(fullPath);
        continue;
      }
      if (!metadata.isFile()) {
        fail("special_file_in_isolation");
      }
      fileCount += 1;
      const size = Number(metadata.size);
      byteCount += size;
      if (
        fileCount > MAX_TREE_FILES ||
        size > MAX_TREE_FILE_BYTES ||
        byteCount > MAX_TREE_BYTES
      ) {
        fail("isolation_tree_too_large");
      }
      const bytes = await readFile(fullPath);
      entries.push({
        path: logicalPath,
        type: "file",
        mode,
        size,
        sha256: sha256(bytes),
      });
      scans.push(scanBytes(bytes, root));
    }
  }

  await visit(root);
  entries.sort((left, right) => left.path.localeCompare(right.path));
  return {
    entries,
    fileCount,
    byteCount,
    scan: combineScans(scans),
  };
}

function entryIdentity(entry) {
  return stableStringify(entry);
}

export function diffTreeSnapshots(before, after) {
  const beforeByPath = new Map(before.entries.map((entry) => [entry.path, entry]));
  const afterByPath = new Map(after.entries.map((entry) => [entry.path, entry]));
  const paths = [...new Set([...beforeByPath.keys(), ...afterByPath.keys()])].sort();
  const created = [];
  const deleted = [];
  const modified = [];
  for (const path of paths) {
    const earlier = beforeByPath.get(path);
    const later = afterByPath.get(path);
    if (!earlier) {
      created.push(later);
    } else if (!later) {
      deleted.push(earlier);
    } else if (entryIdentity(earlier) !== entryIdentity(later)) {
      modified.push({ path, before: earlier, after: later });
    }
  }
  return { created, deleted, modified };
}

async function verifySessionGroupContent(bucketRoot, members) {
  const expected = new Set(members.map((member) => member.logicalPath));
  const actual = [];
  const sessionDirectory = join(bucketRoot, SESSION_ID);
  const transcript = join(bucketRoot, `${SESSION_ID}.jsonl`);

  async function collect(directory) {
    const handle = await opendir(directory);
    for await (const entry of handle) {
      const fullPath = join(directory, entry.name);
      const metadata = await lstat(fullPath);
      if (metadata.isSymbolicLink()) {
        fail("symlink_in_session_group");
      }
      if (metadata.isDirectory()) {
        await collect(fullPath);
      } else if (metadata.isFile()) {
        actual.push(relative(bucketRoot, fullPath).split(sep).join("/"));
      } else {
        fail("special_file_in_session_group");
      }
    }
  }

  const transcriptMetadata = await stat(transcript).catch(() => null);
  const sessionMetadata = await stat(sessionDirectory).catch(() => null);
  if (transcriptMetadata?.isFile()) {
    actual.push(`${SESSION_ID}.jsonl`);
  }
  if (sessionMetadata?.isDirectory()) {
    await collect(sessionDirectory);
  }
  actual.sort();
  const missing = [...expected].filter((path) => !actual.includes(path)).sort();
  const unexpected = actual.filter((path) => !expected.has(path)).sort();
  const changedContent = [];
  for (const member of members) {
    if (missing.includes(member.logicalPath)) {
      continue;
    }
    const bytes = await readRegularNoFollow(
      bucketRoot,
      join(bucketRoot, ...member.logicalPath.split("/")),
    );
    const digest = sha256(bytes);
    if (digest !== member.sha256 || bytes.length !== member.size) {
      changedContent.push(member.logicalPath);
    }
  }
  return {
    matchesContentManifest:
      missing.length === 0 &&
      unexpected.length === 0 &&
      changedContent.length === 0,
    verifiedProperties: ["member_set", "size", "sha256"],
    missing,
    unexpected,
    changedContent,
  };
}

function offlineGuardSource() {
  return `"use strict";
const fs = require("node:fs");
const logPath = process.env.MALLARD_OFFLINE_GUARD_LOG;
function record(kind, operation) {
  if (logPath) {
    fs.appendFileSync(logPath, JSON.stringify({ kind, operation }) + "\\n", { mode: 0o600 });
  }
}
function deny(kind, operation) {
  return function blockedOperation() {
    record(kind, operation);
    const error = new Error("MALLARD_OFFLINE_GUARD_BLOCKED:" + operation);
    error.code = "MALLARD_OFFLINE_GUARD_BLOCKED";
    throw error;
  };
}
const net = require("node:net");
net.connect = deny("network", "net.connect");
net.createConnection = deny("network", "net.createConnection");
net.Socket.prototype.connect = deny("network", "net.Socket.connect");
const tls = require("node:tls");
tls.connect = deny("network", "tls.connect");
const http = require("node:http");
http.request = deny("network", "http.request");
http.get = deny("network", "http.get");
const https = require("node:https");
https.request = deny("network", "https.request");
https.get = deny("network", "https.get");
const http2 = require("node:http2");
http2.connect = deny("network", "http2.connect");
const dgram = require("node:dgram");
dgram.createSocket = deny("network", "dgram.createSocket");
const dns = require("node:dns");
for (const name of ["lookup", "resolve", "resolve4", "resolve6", "resolveAny", "reverse"]) {
  if (typeof dns[name] === "function") dns[name] = deny("network", "dns." + name);
}
if (dns.promises) {
  for (const name of ["lookup", "resolve", "resolve4", "resolve6", "resolveAny", "reverse"]) {
    if (typeof dns.promises[name] === "function") dns.promises[name] = deny("network", "dns.promises." + name);
  }
}
globalThis.fetch = deny("network", "fetch");
const child = require("node:child_process");
for (const name of ["exec", "execFile", "execFileSync", "execSync", "fork", "spawn", "spawnSync"]) {
  child[name] = deny("process", "child_process." + name);
}
`;
}

function buildCleanEnvironment(paths, guardLog) {
  return Object.freeze({
    HOME: paths.homeRoot,
    USER: "mallard-poc",
    LOGNAME: "mallard-poc",
    PATH: dirname(process.execPath),
    TMPDIR: paths.tempRoot,
    TMP: paths.tempRoot,
    TEMP: paths.tempRoot,
    XDG_CACHE_HOME: join(paths.homeRoot, ".cache"),
    XDG_CONFIG_HOME: join(paths.homeRoot, ".config"),
    XDG_DATA_HOME: join(paths.homeRoot, ".local", "share"),
    XDG_STATE_HOME: join(paths.homeRoot, ".local", "state"),
    QODER_CONFIG_DIR: paths.configRoot,
    QODER_FORCE_FILE_STORAGE: "true",
    HTTP_PROXY: "http://127.0.0.1:1",
    HTTPS_PROXY: "http://127.0.0.1:1",
    ALL_PROXY: "http://127.0.0.1:1",
    NO_PROXY: "",
    MALLARD_OFFLINE_GUARD_LOG: guardLog,
    NO_COLOR: "1",
    TERM: "dumb",
    LANG: "C",
    CI: "1",
  });
}

function appendBounded(chunks, chunk, state, child) {
  const bytes = Buffer.from(chunk);
  state.bytes += bytes.length;
  if (state.bytes > MAX_OUTPUT_BYTES) {
    state.exceeded = true;
    child.kill("SIGTERM");
    return;
  }
  chunks.push(bytes);
}

function killChild(child, signal) {
  try {
    if (process.platform !== "win32" && child.pid) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch {
    child.kill(signal);
  }
}

async function readGuardEvents(filePath) {
  const text = await readFile(filePath, "utf8").catch((error) => {
    if (error?.code === "ENOENT") {
      return "";
    }
    throw error;
  });
  if (Buffer.byteLength(text) > 64 * 1024) {
    fail("offline_guard_log_too_large");
  }
  const events = [];
  for (const line of text.split("\n")) {
    if (!line) {
      continue;
    }
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      fail("invalid_offline_guard_log");
    }
    if (
      !value ||
      !["network", "process"].includes(value.kind) ||
      typeof value.operation !== "string" ||
      !/^[a-zA-Z0-9_.]{1,80}$/.test(value.operation)
    ) {
      fail("invalid_offline_guard_log");
    }
    events.push(value);
  }
  return events;
}

function outputSummary(stdout, stderr, root) {
  const combined = Buffer.concat([stdout, stderr]);
  const scan = scanBytes(combined, root);
  const text = combined.toString("utf8");
  const authRequired = AUTH_PATTERNS.some((pattern) => pattern.test(text));
  const signals = [];
  if (text.includes(SESSION_ID)) {
    signals.push("SESSION_ID_OBSERVED");
  }
  if (text.includes(SESSION_TITLE)) {
    signals.push("SESSION_TITLE_OBSERVED");
  }
  if (text.includes(PRIOR_ASSISTANT_MARKER)) {
    signals.push("PRIOR_ASSISTANT_MARKER_OBSERVED");
  }
  if (text.includes(RESUME_PROMPT)) {
    signals.push("RESUME_PROMPT_OBSERVED");
  }
  if (authRequired) {
    signals.push("AUTH_REQUIRED");
  }
  if (text.includes("MALLARD_OFFLINE_GUARD_BLOCKED")) {
    signals.push("OFFLINE_GUARD_BLOCKED");
  }
  return {
    stdoutBytes: stdout.length,
    stderrBytes: stderr.length,
    stdoutSha256: sha256(stdout),
    stderrSha256: sha256(stderr),
    signals,
    authRequired,
    scan,
  };
}

async function runCliCommand({
  label,
  cliBundle,
  nodePath,
  guardPath,
  args,
  paths,
  timeoutMs,
}) {
  const guardLog = join(paths.runtimeRoot, `${label}-offline-guard.jsonl`);
  const environment = buildCleanEnvironment(paths, guardLog);
  const child = spawn(
    nodePath,
    ["--require", guardPath, cliBundle, ...args],
    {
      cwd: paths.projectRoot,
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
    },
  );
  const stdoutChunks = [];
  const stderrChunks = [];
  const stdoutState = { bytes: 0, exceeded: false };
  const stderrState = { bytes: 0, exceeded: false };
  child.stdout.on("data", (chunk) =>
    appendBounded(stdoutChunks, chunk, stdoutState, child),
  );
  child.stderr.on("data", (chunk) =>
    appendBounded(stderrChunks, chunk, stderrState, child),
  );

  const outcome = await new Promise((complete) => {
    let settled = false;
    let forceTimer;
    let timedOut = false;
    const finish = (value) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      clearTimeout(forceTimer);
      complete({ ...value, timedOut });
    };
    const timer = setTimeout(() => {
      if (settled) {
        return;
      }
      timedOut = true;
      killChild(child, "SIGTERM");
      forceTimer = setTimeout(() => {
        if (!settled) {
          killChild(child, "SIGKILL");
        }
      }, 2_000);
    }, timeoutMs);
    child.once("error", () =>
      finish({ exitCode: null, signal: null, spawnFailed: true }),
    );
    child.once("close", (exitCode, signal) =>
      finish({ exitCode, signal, spawnFailed: false }),
    );
  });

  const stdout = Buffer.concat(stdoutChunks);
  const stderr = Buffer.concat(stderrChunks);
  const output = outputSummary(stdout, stderr, paths.root);
  const guardEvents = await readGuardEvents(guardLog);
  return {
    invocation: {
      label,
      flags: args.filter((value) => value.startsWith("-")),
      cleanEnvironmentKeys: Object.keys(environment).sort(),
      authEnvironmentKeys: [],
      cwdIsSynthetic: true,
      qoderConfigIsSynthetic: true,
    },
    exitCode: outcome.exitCode,
    signal: outcome.signal,
    spawnFailed: outcome.spawnFailed,
    timedOut: outcome.timedOut,
    outputExceeded: stdoutState.exceeded || stderrState.exceeded,
    output,
    offlineGuard: {
      deniedCount: guardEvents.length,
      networkDeniedCount: guardEvents.filter((event) => event.kind === "network")
        .length,
      processDeniedCount: guardEvents.filter((event) => event.kind === "process")
        .length,
      operations: [...new Set(guardEvents.map((event) => event.operation))].sort(),
    },
  };
}

function sanitizeMutationPath(value, projectBucket) {
  return value
    .split("/")
    .map((segment) => (segment === projectBucket ? "<project-bucket>" : segment))
    .join("/");
}

function sanitizeMutationEntry(entry, projectBucket) {
  return {
    ...entry,
    path: sanitizeMutationPath(entry.path, projectBucket),
  };
}

function sanitizeMutations(mutations, projectBucket) {
  return {
    created: mutations.created.map((entry) =>
      sanitizeMutationEntry(entry, projectBucket),
    ),
    deleted: mutations.deleted.map((entry) =>
      sanitizeMutationEntry(entry, projectBucket),
    ),
    modified: mutations.modified.map((entry) => ({
      path: sanitizeMutationPath(entry.path, projectBucket),
      before: sanitizeMutationEntry(entry.before, projectBucket),
      after: sanitizeMutationEntry(entry.after, projectBucket),
    })),
  };
}

function publicCommandResult(command, mutations, sessionGroup, projectBucket) {
  return {
    ...command,
    mutations: sanitizeMutations(mutations, projectBucket),
    sessionGroup,
  };
}

function countCredentialFindings(...scans) {
  return scans.reduce(
    (total, scan) =>
      total +
      (scan?.credentialCanaryMatches ?? 0) +
      (scan?.credentialShapeMatches ?? 0),
    0,
  );
}

export async function executeCliResumePoc(
  { cliBundle, nodePath = process.execPath, timeoutMs } = {},
  { onIsolationRoot } = {},
) {
  let checkedCli;
  let checkedNode;
  let timeout;
  try {
    checkedCli = await validateQoderCliBundle(cliBundle);
    checkedNode = await validateCanonicalRegularFile(nodePath, "invalid_node_path");
    timeout = validateTimeout(timeoutMs);
  } catch (error) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: safeErrorCode(error) },
      cleanup: { attempted: false, complete: true },
    };
  }

  let paths;
  let cleanupComplete = false;
  let result;
  let failureCode;
  let sensitiveProjectBucket;
  try {
    paths = await createIsolation(onIsolationRoot);
    const projectBucket = computeCliProjectBucket(paths.projectRoot);
    sensitiveProjectBucket = projectBucket;
    const bucketRoot = join(paths.configRoot, "projects", projectBucket);
    const members = createSyntheticMembers({ projectRoot: paths.projectRoot });
    const restore = await restoreSyntheticSessionGroup({
      isolationRoot: paths.root,
      sourceRoot: paths.sourceRoot,
      stageRoot: paths.stageRoot,
      destinationBucketRoot: bucketRoot,
      members,
    });
    const guardPath = join(paths.runtimeRoot, "offline-guard.cjs");
    await writeFile(guardPath, offlineGuardSource(), {
      flag: "wx",
      mode: 0o600,
    });

    const beforeList = await snapshotTree(paths.root);
    const listCommand = await runCliCommand({
      label: "list",
      cliBundle: checkedCli.entryPath,
      nodePath: checkedNode,
      guardPath,
      args: [
        "--config-dir",
        paths.configRoot,
        "--cwd",
        paths.projectRoot,
        "--list-sessions",
      ],
      paths,
      timeoutMs: timeout,
    });
    const afterList = await snapshotTree(paths.root);
    const listMutations = diffTreeSnapshots(beforeList, afterList);
    const listSessionGroup = await verifySessionGroupContent(
      bucketRoot,
      members,
    );

    const resumeCommand = await runCliCommand({
      label: "resume",
      cliBundle: checkedCli.entryPath,
      nodePath: checkedNode,
      guardPath,
      args: [
        "--config-dir",
        paths.configRoot,
        "--cwd",
        paths.projectRoot,
        "--bare",
        "--resume",
        SESSION_ID,
        "--print",
        "--tools",
        "",
        "--permission-mode",
        "dont_ask",
        "--no-session-persistence",
        "--disable-builtin-skills",
        "--strict-mcp-config",
        "--mcp-config",
        '{"mcpServers":{}}',
        "--max-turns",
        "1",
        "--output-format",
        "json",
        RESUME_PROMPT,
      ],
      paths,
      timeoutMs: timeout,
    });
    const afterResume = await snapshotTree(paths.root);
    const resumeMutations = diffTreeSnapshots(afterList, afterResume);
    const resumeSessionGroup = await verifySessionGroupContent(
      bucketRoot,
      members,
    );

    const listDiscovered =
      listCommand.exitCode === 0 &&
      listCommand.output.signals.includes("SESSION_ID_OBSERVED") &&
      listCommand.output.signals.includes("SESSION_TITLE_OBSERVED");
    const authGateObserved = resumeCommand.output.authRequired;
    const credentialFindingCount = countCredentialFindings(
      beforeList.scan,
      afterList.scan,
      afterResume.scan,
      listCommand.output.scan,
      resumeCommand.output.scan,
    );
    const deniedOperationCount =
      listCommand.offlineGuard.deniedCount +
      resumeCommand.offlineGuard.deniedCount;
    const failures = [];
    if (!listDiscovered) failures.push("cli_list_discovery_unproved");
    if (!listSessionGroup.matchesContentManifest) {
      failures.push("list_changed_session_group_content");
    }
    if (!resumeSessionGroup.matchesContentManifest) {
      failures.push("resume_changed_session_group_content");
    }
    if (!authGateObserved) failures.push("resume_auth_gate_unproved");
    if (resumeCommand.exitCode === 0 && !authGateObserved) {
      failures.push("unexpected_resume_without_auth_gate");
    }
    if (credentialFindingCount > 0) failures.push("credential_material_observed");
    if (deniedOperationCount > 0) failures.push("offline_guard_intercepted_operation");
    if (
      listCommand.spawnFailed ||
      resumeCommand.spawnFailed ||
      listCommand.timedOut ||
      resumeCommand.timedOut ||
      listCommand.outputExceeded ||
      resumeCommand.outputExceeded
    ) {
      failures.push("cli_execution_incomplete");
    }

    result = {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: failures.length === 0,
      ...(failures.length > 0 ? { failures: [...new Set(failures)] } : {}),
      cli: {
        name: checkedCli.name,
        version: checkedCli.version,
      },
      session: {
        sessionId: SESSION_ID,
        title: SESSION_TITLE,
        projectBucketSha256: sha256(Buffer.from(projectBucket)),
        projectBucketLength: projectBucket.length,
        projectBucketVerified: true,
        projectBucketContract: "OBSERVED_QODER_CLI_1_1_40",
        memberCount: members.length,
      },
      restore,
      commands: {
        list: publicCommandResult(
          listCommand,
          listMutations,
          listSessionGroup,
          projectBucket,
        ),
        resume: publicCommandResult(
          resumeCommand,
          resumeMutations,
          resumeSessionGroup,
          projectBucket,
        ),
      },
      evidence: {
        listDiscovered,
        authGateObserved,
        authenticatedResumeProved: false,
        transcriptConsumptionDuringResumeProved: false,
        activeLeafSemanticsProved: false,
        sidecarConsumptionProved: false,
        credentialFindingCount,
        interceptedInterfacePermittedOperationCount: 0,
        deniedOperationCount,
      },
      blockers: [
        "AUTHENTICATED_RESUME_NOT_RUN",
        "TRANSCRIPT_CONSUMPTION_DURING_RESUME_UNPROVED",
        "ACTIVE_LEAF_SEMANTICS_UNPROVED",
        "SIDECAR_CONSUMPTION_UNPROVED",
      ],
    };
  } catch (error) {
    failureCode = safeErrorCode(error);
  } finally {
    if (paths) {
      try {
        await removeIsolation(paths.root);
        cleanupComplete = true;
      } catch {
        failureCode ??= "cleanup_failed";
      }
    }
  }

  if (failureCode || !result) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: failureCode ?? "unexpected_error" },
      cleanup: { attempted: Boolean(paths), complete: cleanupComplete },
    };
  }
  result.cleanup = { attempted: true, complete: cleanupComplete };
  if (!cleanupComplete) {
    result.ok = false;
    result.failures = [...new Set([...(result.failures ?? []), "cleanup_failed"])]
      .sort();
  }
  const serialized = JSON.stringify(result);
  if (
    serialized.includes(paths.root) ||
    serialized.includes(checkedCli.entryPath) ||
    (sensitiveProjectBucket !== undefined &&
      serialized.includes(sensitiveProjectBucket))
  ) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: "path_sanitization_failed" },
      cleanup: { attempted: true, complete: cleanupComplete },
    };
  }
  if (serialized.includes(CREDENTIAL_CANARY)) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: "credential_sanitization_failed" },
      cleanup: { attempted: true, complete: cleanupComplete },
    };
  }
  return result;
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      return { help: true };
    }
    const value = argv[index + 1];
    if (argument === "--cli-bundle" && value) {
      options.cliBundle = value;
      index += 1;
    } else if (argument === "--node" && value) {
      options.nodePath = value;
      index += 1;
    } else if (argument === "--timeout-ms" && value) {
      options.timeoutMs = Number(value);
      index += 1;
    } else {
      fail("invalid_argument");
    }
  }
  return options;
}

function usage() {
  return [
    "Usage:",
    "  node qoder-cli-resume.mjs --cli-bundle /absolute/path/to/qodercli.js",
    "",
    "Options:",
    "  --node /absolute/path/to/node",
    "  --timeout-ms 30000",
    "",
    "This PoC never accepts a Qoder token or another credential.",
  ].join("\n");
}

async function main() {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    process.stdout.write(
      `${JSON.stringify({
        schemaVersion: 1,
        ...POC_STATUS,
        ok: false,
        error: { code: safeErrorCode(error) },
      })}\n`,
    );
    process.exitCode = 1;
    return;
  }
  if (options.help) {
    process.stdout.write(`${usage()}\n`);
    return;
  }
  const result = await executeCliResumePoc(options);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  process.exitCode = result.ok ? 0 : 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath && pathToFileURL(invokedPath).href === import.meta.url) {
  await main();
}

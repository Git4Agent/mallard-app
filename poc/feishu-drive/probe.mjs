import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { FeishuDrive, FeishuStore, PocError, RequestGate, expected, requireValue } from "./drive.mjs";
import { buildSelector, REPOSITORY_ROOT } from "./selector.mjs";

const BUNDLE = "0123456789abcdef0123456789abcdef";
export const STREAM = `.mallard/v1/repositories/_commits/${BUNDLE}`;
const DATA_ROOT = `.mallard/v1/repositories/${BUNDLE}`;
const MARKER = Buffer.from('{"format":"mallard-append-storage","layout_version":1}');

export function makeRecord(selector, id, parent = null, createdAt = Date.now(), extra = {}) {
  const commitId = `commit-${id}`;
  // Transport fixture, not a complete BundleManifest/CommitRecord DTO.
  const bytes = Buffer.from(JSON.stringify({
    schema_version: 1, fixture: "feishu-transport-poc", commit_id: commitId,
    parent_name: parent?.name ?? null, created_at_ms: createdAt, ...extra,
  }));
  const { name, revision } = selector({ action: "build", commit_id: commitId,
    parent_name: parent?.name ?? null, created_at_ms: createdAt, body: bytes.toString("utf8") });
  return { name, revision, bytes, parent: parent?.name ?? null };
}

export async function listStream(store, selector) {
  const keys = await store.listKeys(STREAM);
  const names = keys.map((key) => key.slice(STREAM.length + 1)).filter((name) => !name.includes("/"));
  const selected = selector({ action: "select", names });
  return { ...selected, names: selected.valid_names };
}

// One bounded attempt with a fresh complete listing. Directory preparation
// happens before the admission listing. The HTTP layer does not retry.
// This is a transport conformance probe, not the production write_record loop.
export async function publish(store, selector, record, { now = Date.now } = {}) {
  requireValue(record.bytes.length <= 64 * 1024, "record_size_limit");
  const rebuilt = selector({ action: "build", commit_id: JSON.parse(record.bytes).commit_id,
    parent_name: record.parent, created_at_ms: JSON.parse(record.bytes).created_at_ms,
    body: record.bytes.toString("utf8") });
  requireValue(rebuilt.name === record.name, "record_name_mismatch");
  const { parent, name } = await store.location(`${STREAM}/${record.name}`, true);
  const before = await listStream(store, selector);
  const listedAt = now();
  if (before.names.includes(record.name)) {
    requireValue(before.chain.at(-1) === record.name, "remote_changed");
    requireValue(await store.get(`${STREAM}/${record.name}`, expected(record.bytes)), "record_missing");
    return "selected";
  }
  requireValue(record.parent === null ? before.names.length === 0 : before.chain.at(-1) === record.parent,
    "remote_changed");
  const age = now() - listedAt;
  requireValue(age >= 0 && age <= 10_000, "stale_admission_listing");
  let uploadFailed = false;
  try {
    await store.drive.upload(parent, name, record.bytes);
  } catch {
    uploadFailed = true;
  }
  let after;
  try { after = await listStream(store, selector); } catch { throw new PocError("outcome_unknown"); }
  requireValue(after.names.includes(record.name), uploadFailed ? "outcome_unknown" : "record_visibility_gap");
  requireValue(after.chain.at(-1) === record.name, "remote_changed");
  requireValue(await store.get(`${STREAM}/${record.name}`, expected(record.bytes)), "record_missing");
  return uploadFailed ? "selected_after_lost_ack" : "selected";
}

async function checkVisible(store, keys) {
  const listed = new Set(await store.listKeys(STREAM));
  requireValue(keys.every((key) => listed.has(key)), "cross_client_visibility_gap");
}

// All remote data is synthetic. No provider home or project content is read.
// This probe retains its uniquely named folder; it has no delete operation.
export async function runProbe({ writer, reader, parentFolderToken, selector, runId = randomBytes(8).toString("hex") }) {
  requireValue(/^[a-f0-9]{16}$/u.test(runId), "invalid_run_id");
  const probeName = `mallard-feishu-poc-${runId}`;
  const root = await writer.createFolder(parentFolderToken, probeName);
  const parentListing = await reader.listFolder(parentFolderToken);
  requireValue(parentListing.some((entry) => entry.token === root && entry.name === probeName && entry.type === "folder"),
    "cross_client_folder_visibility_gap");
  const a = new FeishuStore(writer, root);
  const b = new FeishuStore(reader, root);
  await a.put(".mallard/_storage.json", MARKER);
  requireValue(await b.get(".mallard/_storage.json", expected(MARKER)), "marker_missing");

  const payload = Buffer.from("Synthetic Mallard Feishu payload. No user data.\n");
  const payloadKey = `${DATA_ROOT}/_objects/${expected(payload).sha256}`;
  await a.put(payloadKey, payload);
  requireValue(await b.get(payloadKey, expected(payload)), "payload_missing");

  // Force multipart at 4 MiB + 17 bytes to exercise two normal 4 MiB parts.
  const large = Buffer.alloc(4 * 1024 * 1024 + 17, 0x6d);
  const largeKey = `${DATA_ROOT}/_objects/${expected(large).sha256}`;
  await a.put(largeKey, large, { multipart: true });
  requireValue(await b.get(largeKey, expected(large)), "multipart_payload_missing");

  const manifest = Buffer.from(JSON.stringify({ fixture: "transport-only", files: [
    { key: payloadKey, ...expected(payload) }, { key: largeKey, ...expected(large) },
  ] }));
  const manifestKey = `${DATA_ROOT}/_manifests/1-commit-${"a".repeat(24)}.json`;
  await a.put(manifestKey, manifest);
  requireValue(await b.get(manifestKey, expected(manifest)), "manifest_missing");
  const first = makeRecord(selector, "a".repeat(24), null, Date.now(), { manifest_key: manifestKey, ...expected(manifest) });
  await publish(a, selector, first);
  const next = makeRecord(selector, "b".repeat(24), first, Date.now());
  const competing = makeRecord(selector, "c".repeat(24), first, nextTime(next));
  requireValue(next.name.length === 223, "record_name_length_changed");

  // Both clients observe the same childless parent before either sends.
  // Uploads remain sequential because Feishu disallows concurrent calls.
  const location = await a.location(`${STREAM}/${next.name}`);
  const bLocation = await b.location(`${STREAM}/${competing.name}`);
  const viewA = await listStream(a, selector);
  const listedAtA = Date.now();
  const viewB = await listStream(b, selector);
  const listedAtB = Date.now();
  requireValue(viewA.chain.at(-1) === first.name && viewB.chain.at(-1) === first.name
    && location.parent === bLocation.parent, "race_parent_mismatch");
  requireValue(Date.now() - listedAtA <= 10_000, "stale_race_listing");
  await writer.upload(location.parent, next.name, next.bytes);
  requireValue(Date.now() - listedAtB <= 10_000, "stale_race_listing");
  await reader.upload(bLocation.parent, competing.name, competing.bytes);
  const recordKeys = [first, next, competing].map((record) => `${STREAM}/${record.name}`);
  await checkVisible(a, recordKeys);
  await checkVisible(b, recordKeys);
  const chainA = await listStream(a, selector);
  const chainB = await listStream(b, selector);
  requireValue(chainA.chain.at(-1) === next.name && JSON.stringify(chainA.chain) === JSON.stringify(chainB.chain),
    "selection_mismatch");
  for (const record of [first, next, competing]) {
    requireValue(await b.get(`${STREAM}/${record.name}`, expected(record.bytes)), "record_missing");
  }

  // Same-name uploads may create distinct file tokens. Check exact names and
  // every candidate; a renamed copy or conflicting bytes fails the probe.
  const original = await writer.upload(location.parent, next.name, next.bytes);
  const repeated = await reader.upload(location.parent, next.name, next.bytes);
  const duplicates = await b.matches(location.parent, next.name);
  requireValue(duplicates.some((entry) => entry.token === original)
    && duplicates.some((entry) => entry.token === repeated), "duplicate_name_changed");
  requireValue(await b.get(`${STREAM}/${next.name}`, expected(next.bytes)), "duplicate_record_missing");
  requireValue((await listStream(b, selector)).chain.at(-1) === next.name, "duplicate_changed_selection");

  // An abandoned payload/manifest does not introduce another record.
  const abandoned = Buffer.from("Synthetic interrupted Push\n");
  await a.put(`${DATA_ROOT}/_objects/${expected(abandoned).sha256}`, abandoned);
  requireValue((await listStream(b, selector)).chain.at(-1) === next.name, "orphan_changed_selection");
  // Repeated reads are observations, not a provider consistency guarantee.
  for (let sample = 0; sample < 3; sample += 1) await checkVisible(b, recordKeys);

  return {
    verdict: "PROBE_PASS", production_ready: false, data: "synthetic_only",
    checks: ["marker", "small_file_sha256", "multipart_sha256", "manifest_before_record",
      "223_character_record_name", "all_listing_pages", "two_client_visibility",
      "earliest_sibling_selected", "losing_record_retained", "duplicate_name_readback", "orphan_not_published"],
    duplicate_mode: original === repeated ? "token_reused" : "distinct_tokens",
    consistency: "sampled_only_not_a_guarantee", cleanup: "retained_no_delete_api", probe_folder_name: probeName,
  };
}

function nextTime(record) {
  return JSON.parse(record.bytes).created_at_ms + 1;
}

export async function readCredentials(filePath) {
  requireValue(typeof filePath === "string" && isAbsolute(filePath), "credential_path_must_be_absolute");
  const normalized = resolve(filePath);
  const repositoryRelative = relative(REPOSITORY_ROOT, normalized);
  requireValue(repositoryRelative.startsWith(".." + (process.platform === "win32" ? "\\" : "/"))
    || isAbsolute(repositoryRelative), "credential_file_must_be_outside_repository");
  requireValue(process.platform !== "win32", "credential_acl_check_unavailable");
  let handle;
  try {
    const before = await lstat(normalized);
    requireValue(before.isFile() && !before.isSymbolicLink() && await realpath(normalized) === normalized,
      "credential_file_not_canonical");
    handle = await open(normalized, constants.O_RDONLY | constants.O_NOFOLLOW);
    const opened = await handle.stat();
    requireValue(opened.isFile() && opened.dev === before.dev && opened.ino === before.ino
      && opened.nlink === 1 && opened.size > 0 && opened.size <= 64 * 1024
      && (opened.mode & 0o077) === 0 && opened.uid === process.getuid(), "unsafe_credential_file");
    const buffer = Buffer.alloc(64 * 1024 + 1);
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
    const after = await handle.stat();
    requireValue(bytesRead === opened.size && after.size === opened.size && after.mtimeMs === opened.mtimeMs
      && after.ctimeMs === opened.ctimeMs, "credential_file_changed");
    let data;
    try { data = JSON.parse(buffer.subarray(0, bytesRead).toString("utf8")); } catch { throw new PocError("invalid_credentials"); }
    requireValue(data && typeof data === "object" && !Array.isArray(data)
      && Object.keys(data).every((key) => ["access_token", "peer_access_token", "parent_folder_token"].includes(key)),
      "invalid_credentials");
    requireValue(/^[a-zA-Z0-9_-]{1,256}$/u.test(data.parent_folder_token ?? ""), "invalid_parent_folder_token");
    for (const value of [data.access_token, data.peer_access_token ?? data.access_token]) {
      requireValue(typeof value === "string" && /^[\x21-\x7e]{1,16384}$/u.test(value), "invalid_access_token");
    }
    return data;
  } catch (error) {
    if (error instanceof PocError) throw error;
    throw new PocError("credential_file_unavailable");
  } finally {
    await handle?.close();
  }
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "mock") {
    const { MockDrive } = await import("./mock.mjs");
    const mock = new MockDrive();
    const gate = new RequestGate(0);
    const writer = new FeishuDrive({ accessToken: "fixture-a", fetchImpl: mock.fetch, gate, pageSize: 2 });
    const reader = new FeishuDrive({ accessToken: "fixture-b", fetchImpl: mock.fetch, gate, pageSize: 2 });
    const result = await runProbe({ writer, reader, parentFolderToken: "root", selector: buildSelector(),
      runId: "0123456789abcdef" });
    console.log(JSON.stringify({ ...result, mode: "mock", http_requests: mock.requests.length }, null, 2));
    return;
  }
  requireValue(args.length === 3 && args[0] === "live" && args[1] === "--credentials",
    "usage_mock_or_live_--credentials_absolute_path");
  const credential = await readCredentials(args[2]);
  const selector = buildSelector();
  const gate = new RequestGate();
  // Small pages force pagination even for this small synthetic stream.
  const writer = new FeishuDrive({ accessToken: credential.access_token, gate, pageSize: 2 });
  const readerToken = credential.peer_access_token ?? credential.access_token;
  const reader = new FeishuDrive({ accessToken: readerToken, gate, pageSize: 2 });
  const runId = randomBytes(8).toString("hex");
  // Print the generated name before the first mutation so failed probes can
  // still be located. Never print a folder token or caller-supplied path.
  console.log(JSON.stringify({ mode: "live", probe_folder_name: `mallard-feishu-poc-${runId}`,
    cleanup: "retained_no_delete_api" }));
  const result = await runProbe({ writer, reader, parentFolderToken: credential.parent_folder_token, selector, runId });
  console.log(JSON.stringify({ ...result, mode: "live", distinct_credentials: readerToken !== credential.access_token }, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(JSON.stringify({ verdict: "FAIL", code: error instanceof PocError ? error.code : "unexpected_failure" }));
    process.exitCode = 1;
  });
}

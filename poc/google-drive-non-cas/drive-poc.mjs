import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";
import { DriveStore, MARKER, MARKER_KEY, PocError, sha256 } from "./drive-store.mjs";
import { MockDrive } from "./mock-drive.mjs";
import { PROJECT_RECORDS, listView, makeRecord, publish, readRecord } from "./publication-model.mjs";

export async function readCredentials(path) {
  if (!isAbsolute(path) || process.platform === "win32") throw new PocError("private_posix_credential_file_required");
  const canonical = resolve(path);
  const repository = fileURLToPath(new URL("../../", import.meta.url));
  if (canonical.startsWith(repository) || await realpath(canonical) !== canonical) {
    throw new PocError("credential_file_location_rejected");
  }
  const initial = await lstat(canonical);
  if (!initial.isFile() || initial.isSymbolicLink()) throw new PocError("credential_file_rejected");
  const handle = await open(canonical, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const before = await handle.stat();
    if (!before.isFile() || before.nlink !== 1 || before.dev !== initial.dev || before.ino !== initial.ino
      || before.uid !== process.getuid() || (before.mode & 0o077) !== 0 || before.size < 1 || before.size > 48 * 1024) {
      throw new PocError("credential_file_rejected");
    }
    // Bounded even if another process grows the file during the read.
    const buffer = Buffer.alloc(48 * 1024 + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, null);
      if (bytesRead === 0) break;
      length += bytesRead;
    }
    const after = await handle.stat();
    if (length !== before.size || before.size !== after.size || before.mtimeMs !== after.mtimeMs
      || before.ctimeMs !== after.ctimeMs) throw new PocError("credential_file_changed");
    let value;
    try { value = JSON.parse(buffer.subarray(0, length)); }
    catch { throw new PocError("invalid_credential_json"); }
    const allowed = ["client_a_token", "client_b_token", "parent_id", "shared_drive_id", "space"];
    if (!value || Array.isArray(value) || Object.keys(value).some((key) => !allowed.includes(key))
      || typeof value.client_a_token !== "string" || typeof value.client_b_token !== "string"
      || value.client_a_token === value.client_b_token || typeof value.parent_id !== "string") {
      throw new PocError("invalid_credentials");
    }
    return value;
  } finally { await handle.close(); }
}

export async function runProbe(a, b) {
  await a.putImmutable(MARKER_KEY, MARKER);
  if (!(await b.get(MARKER_KEY))?.equals(MARKER)) throw new PocError("cross_client_marker_missing");
  const bundle = randomBytes(16).toString("hex");
  const stream = `${PROJECT_RECORDS}/${bundle}`;
  const payload = Buffer.from("Mallard Google Drive synthetic fixture. No provider data.\n");
  const payloadKey = `.mallard/v1/repositories/${bundle}/_objects/${sha256(payload)}`;
  await a.putImmutable(payloadKey, payload);
  const restored = await b.get(payloadKey, { expected: { size: payload.length, sha256: sha256(payload) } });
  if (!restored?.equals(payload)) throw new PocError("cross_client_payload_missing");
  const root = makeRecord(null, "01".repeat(12), 100);
  await publish(a, stream, root);
  await readRecord(b, stream, root);
  const first = makeRecord(root, "02".repeat(12), 200);
  const second = makeRecord(root, "03".repeat(12), 300);
  let firstResult;
  let secondResult;
  try {
    await publish(b, stream, second, { afterList: async ({ attempt }) => {
      if (attempt === 0) firstResult = await publish(a, stream, first);
    } });
    secondResult = "selected";
  } catch (error) {
    if (error.code !== "remote_changed") throw error;
    secondResult = "remote_changed";
  }
  const observedA = await listView(a, stream);
  const observedB = await listView(b, stream);
  if (firstResult?.selected?.token !== first.token || secondResult !== "remote_changed"
    || observedA.selected?.token !== first.token || observedB.selected?.token !== first.token) {
    throw new PocError("cross_client_selection_failed");
  }
  // Two physical files for one logical marker. No index or conditional update.
  await a.writeOnce(await a.prepare(MARKER_KEY, MARKER));
  if ((await b.copies(MARKER_KEY)).length < 2 || !(await b.get(MARKER_KEY))?.equals(MARKER)) {
    throw new PocError("duplicate_probe_failed");
  }
  return { marker: "verified_by_second_client", payload: "sha256_verified_by_second_client",
    publication: "same_selection_after_both_writes", losingSibling: secondResult,
    duplicateKey: "identical_copies_read", conditionalWrites: 0,
    limitations: ["finite_observation_not_consistency_guarantee", "synthetic_records_not_bundle_engine"] };
}

async function main(args) {
  if (args.length === 1 && args[0] === "mock") {
    const mock = new MockDrive();
    const a = new DriveStore({ token: "fixture-a", rootId: "fixture_root", fetchImpl: mock.fetch, pageSize: 1 });
    const b = new DriveStore({ token: "fixture-b", rootId: "fixture_root", fetchImpl: mock.fetch, pageSize: 1 });
    return { evidence: "in_process_http_mock", result: await runProbe(a, b), requests: mock.requests.length,
      liveGoogleDrive: "not_run" };
  }
  if (args.length !== 3 || args[0] !== "live" || args[1] !== "--credentials") {
    throw new PocError("usage_mock_or_live_credentials");
  }
  const credentials = await readCredentials(args[2]);
  const options = { rootId: credentials.parent_id, sharedDriveId: credentials.shared_drive_id,
    space: credentials.space ?? "drive" };
  const parent = new DriveStore({ ...options, token: credentials.client_a_token });
  const secondParent = new DriveStore({ ...options, token: credentials.client_b_token });
  await parent.validateRoot();
  await secondParent.validateRoot();
  if (parent.rootId !== secondParent.rootId) throw new PocError("different_parent_folders");
  const probeName = `mallard-drive-poc-${randomBytes(12).toString("hex")}`;
  try {
    const rootId = await parent.createProbeFolder(probeName);
    const a = new DriveStore({ ...options, rootId, token: credentials.client_a_token, pageSize: 1 });
    const b = new DriveStore({ ...options, rootId, token: credentials.client_b_token, pageSize: 1 });
    await b.validateRoot();
    const result = await runProbe(a, b);
    return { evidence: "live_google_drive_finite_probe", result, probeName,
      cleanup: "synthetic_folder_retained", scope: options.space,
      sharedDrive: Boolean(options.sharedDriveId), physicalDevices: "not_attested" };
  } catch (error) {
    error.probeName = probeName;
    throw error;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log(JSON.stringify(await main(process.argv.slice(2)), null, 2)); }
  catch (error) {
    // Only codes defined in this POC, an HTTP status, and our random probe name.
    console.log(JSON.stringify({ result: "failed", code: error instanceof PocError ? error.code : "probe_failed",
      ...(error.status ? { status: error.status } : {}),
      ...(error.probeName ? { probeName: error.probeName, cleanup: "inspect_synthetic_folder" } : {}) }, null, 2));
    process.exitCode = 1;
  }
}

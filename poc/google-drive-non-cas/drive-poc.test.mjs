import assert from "node:assert/strict";
import test from "node:test";
import { execFileSync } from "node:child_process";
import { chmod, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { DriveStore, MARKER, MARKER_KEY, MAX_OBJECT_BYTES, metadataFor, sha256 } from "./drive-store.mjs";
import { MockDrive } from "./mock-drive.mjs";
import { PROJECT_RECORDS, listView, makeRecord, parseName, publish, readRecord, selectMainline } from "./publication-model.mjs";
import { readCredentials, runProbe } from "./drive-poc.mjs";

const bundle = "01".repeat(16);
const stream = `${PROJECT_RECORDS}/${bundle}`;
const library = `.mallard/v1/library/${"02".repeat(16)}/_commits`;
const record = (parent, id, time = id * 100) => makeRecord(parent, id.toString(16).padStart(2, "0").repeat(12), time);
const keyFor = (bytes, id = bundle) => `.mallard/v1/repositories/${id}/_objects/${sha256(bytes)}`;
const writes = (mock) => mock.requests.filter((r) => r.path === "/upload/drive/v3/files");
const code = (expected) => (error) => error.code === expected;

function setup(options = {}) {
  const mock = new MockDrive();
  const a = new DriveStore({ token: "fixture-a", rootId: "fixture_root", fetchImpl: mock.fetch, ...options });
  const b = new DriveStore({ token: "fixture-b", rootId: "fixture_root", fetchImpl: mock.fetch, ...options });
  return { mock, a, b };
}

test("two clients complete the synthetic publication probe with paginated HTTP requests", async () => {
  const { a, b, mock } = setup({ pageSize: 1 });
  const result = await runProbe(a, b);
  assert.equal(result.losingSibling, "remote_changed");
  assert.equal(result.conditionalWrites, 0);
  assert.ok(mock.requests.some((r) => r.params.pageToken));
  assert.ok(mock.requests.every((r) => ["GET", "POST"].includes(r.method)));
});

test("binary and zero-byte objects keep their exact bytes and SHA-256", async () => {
  const { a, b } = setup();
  for (const bytes of [Buffer.alloc(0), Buffer.from([0, 255, 128, 13, 10, 0])]) {
    const key = keyFor(bytes);
    assert.equal(await a.putImmutable(key, bytes), "written");
    assert.deepEqual(await b.get(key, { expected: { size: bytes.length, sha256: sha256(bytes) } }), bytes);
    assert.equal(await a.putImmutable(key, bytes), "already_present");
  }
});

test("concurrent identical creates produce two physical IDs and one logical key", async () => {
  const { a, b, mock } = setup({ pageSize: 1 });
  const bytes = Buffer.from("same payload");
  const key = keyFor(bytes);
  const prepared = await Promise.all([a.prepare(key, bytes), b.prepare(key, bytes)]);
  await Promise.all([a.writeOnce(prepared[0]), b.writeOnce(prepared[1])]);
  assert.equal(mock.files.size, 2);
  assert.deepEqual(await a.listKeys(`.mallard/v1/repositories/${bundle}`), [key]);
  assert.deepEqual(await b.get(key), bytes);
});

test("conflicting copies fail instead of choosing an arbitrary Drive ID", async () => {
  const { a, b } = setup();
  const key = keyFor("first");
  await a.writeOnce(await a.prepare(key, "first"));
  await b.writeOnce(await b.prepare(key, "second"));
  await assert.rejects(a.get(key), code("duplicate_content_conflict"));
});

test("an existing key cannot be replaced with different bytes", async () => {
  const { a, mock } = setup();
  const key = keyFor("first");
  await a.putImmutable(key, "first");
  await assert.rejects(a.putImmutable(key, "second"), code("immutable_content_conflict"));
  assert.equal(writes(mock).length, 1);
});

test("same file name in different logical keys and folders stays isolated", async () => {
  const { a, mock } = setup();
  const bytes = Buffer.from("shared content hash");
  const one = keyFor(bytes);
  const two = keyFor(bytes, "03".repeat(16));
  const outside = new DriveStore({ token: "fixture-b", rootId: "another_root", fetchImpl: mock.fetch });
  await a.putImmutable(one, bytes);
  await a.putImmutable(two, bytes);
  await outside.putImmutable(keyFor("outside"), "outside");
  assert.deepEqual(await a.listKeys(`.mallard/v1/repositories/${bundle}`), [one]);
  assert.equal((await a.listKeys(".mallard/v1/repositories")).length, 2);
});

test("project discovery and Library streams use the current separate prefixes", async () => {
  const { a } = setup();
  const root = record(null, 1);
  await publish(a, stream, root);
  await publish(a, library, root);
  await a.putImmutable(keyFor("payload"), "payload");
  assert.deepEqual(await a.listKeys(PROJECT_RECORDS), [`${stream}/${root.name}`]);
  assert.equal((await listView(a, library)).selected.token, root.token);
});

test("metadata fits Google property limits for a 223-byte record name", () => {
  const root = record(null, 1);
  const child = record(root, 2);
  assert.equal(child.name.length, 223);
  const metadata = metadataFor(`${stream}/${child.name}`, "root", "id");
  assert.equal(metadata.name.length, 223);
  assert.ok(Object.entries(metadata.properties).every(([k, v]) => Buffer.byteLength(k + v) <= 124));
  assert.ok(Object.keys(metadata.properties).length <= 30);
  assert.equal(metadata.description, `mallard-object-key:${stream}/${child.name}`);
});

test("empty pages with continuation tokens do not truncate listing", async () => {
  const { a, mock } = setup({ pageSize: 1 });
  await a.putImmutable(keyFor("one"), "one");
  await a.putImmutable(keyFor("two"), "two");
  mock.emptyFirstPage = true;
  assert.equal((await a.listKeys(".mallard/v1/repositories")).length, 2);
});

test("incomplete search stops publication before a record write", async () => {
  const { a, mock } = setup();
  mock.incomplete = true;
  await assert.rejects(publish(a, stream, record(null, 1)), code("incomplete_search"));
  assert.equal(writes(mock).length, 0);
});

test("repeated cursors and rejected page tokens cannot produce a partial success", async () => {
  const { a, mock } = setup({ pageSize: 1 });
  await a.putImmutable(keyFor("one"), "one");
  await a.putImmutable(keyFor("two"), "two");
  mock.repeatCursor = true;
  await assert.rejects(a.listKeys(".mallard/v1/repositories"), code("invalid_page_token"));
  mock.repeatCursor = false;
  mock.rejectedCursor = true;
  await assert.rejects(a.listKeys(".mallard/v1/repositories"), code("drive_http_400"));
});

test("page, item, and duplicate limits fail with an error", async () => {
  const { a, mock } = setup({ pageSize: 1 });
  const key = keyFor("one");
  for (let i = 0; i < 3; i += 1) await a.writeOnce(await a.prepare(key, "one"));
  for (const [limit, value, expected] of [["maxPages", 1, "listing_page_limit"],
    ["maxItems", 1, "listing_limit"], ["maxCopies", 1, "duplicate_limit"]]) {
    const b = new DriveStore({ token: "fixture-b", rootId: "fixture_root", fetchImpl: mock.fetch, pageSize: 1, [limit]: value });
    await assert.rejects(b.get(key), code(expected));
  }
});

test("invalid keys fail before any network request", async () => {
  const { a, mock } = setup();
  for (const key of ["/tmp/file", ".mallard/../secret", ".mallard/v1/repositories/x/../x",
    ".mallard/v1/repositories/x/escape\\name", ".mallard/v1/repositories/x/a\n"]) {
    await assert.rejects(a.putImmutable(key, "bytes"), code("invalid_key"));
  }
  assert.equal(mock.requests.length, 0);
});

test("malformed tagged metadata stops listing", async () => {
  const { a, mock } = setup();
  await a.putImmutable(keyFor("one"), "one");
  [...mock.files.values()][0].metadata.description = "mallard-object-key:../../elsewhere";
  await assert.rejects(a.listKeys(".mallard/v1/repositories"), code("invalid_key"));
});

test("verified reads detect corruption and enforce size limits", async () => {
  const { a, mock } = setup();
  const key = keyFor("good");
  await a.putImmutable(key, "good");
  [...mock.files.values()][0].bytes = Buffer.from("evil");
  await assert.rejects(a.get(key, { expected: { size: 4, sha256: sha256("good") } }), code("digest_mismatch"));
  await assert.rejects(a.get(key, { maxBytes: 3 }), code("response_too_large"));
  await assert.rejects(a.putImmutable(keyFor("large"), Buffer.alloc(MAX_OBJECT_BYTES + 1)), code("upload_limit"));
});

test("pre-generated IDs make a repeated physical create conflict without another file", async () => {
  const { a, mock } = setup();
  const prepared = await a.prepare(MARKER_KEY, MARKER);
  await a.writeOnce(prepared);
  await assert.rejects(a.writeOnce(prepared), code("drive_http_409"));
  assert.equal(mock.files.size, 1);
  assert.equal(writes(mock).length, 2);
});

test("My Drive, shared drive, and appDataFolder queries use the correct corpus", async () => {
  for (const mode of [{}, { sharedDriveId: "shared_drive" }, { space: "appDataFolder" }]) {
    const { a, mock } = setup(mode);
    await a.putImmutable(MARKER_KEY, MARKER);
    const query = mock.requests.find((r) => r.params.q).params;
    assert.equal(query.spaces, mode.space ?? "drive");
    assert.equal(query.corpora, mode.sharedDriveId ? "drive" : "user");
    assert.equal(query.supportsAllDrives, "true");
    assert.equal(query.driveId, mode.sharedDriveId);
    assert.equal(query.includeItemsFromAllDrives, mode.sharedDriveId ? "true" : undefined);
    assert.equal(mock.requests.find((r) => r.path.endsWith("generateIds")).params.space, mode.space ?? "drive");
  }
});

test("live setup checks the parent and creates only a new probe folder", async () => {
  const { a, b, mock } = setup();
  mock.files.set("fixture_root", { metadata: { id: "fixture_root", name: "Parent",
    mimeType: "application/vnd.google-apps.folder" }, bytes: Buffer.alloc(0) });
  await a.validateRoot();
  await b.validateRoot();
  const id = await a.createProbeFolder(`mallard-drive-poc-${"ab".repeat(12)}`);
  assert.notEqual(id, "fixture_root");
  assert.deepEqual(mock.files.get(id).metadata.parents, ["fixture_root"]);
  const child = new DriveStore({ token: "fixture-b", rootId: id, fetchImpl: mock.fetch });
  await child.validateRoot();
  mock.files.get(id).metadata.trashed = true;
  await assert.rejects(child.validateRoot(), code("root_not_writable_or_wrong_drive"));
  const wrongDrive = new DriveStore({ token: "fixture-b", rootId: "fixture_root",
    sharedDriveId: "another_drive", fetchImpl: mock.fetch });
  await assert.rejects(wrongDrive.validateRoot(), code("root_not_writable_or_wrong_drive"));
});

test("authorization and throttle errors do not retry or expose provider error bodies", async () => {
  for (const status of [401, 403, 429, 503]) {
    let calls = 0;
    const a = new DriveStore({ token: "private-token-value", rootId: "root", fetchImpl: async () => {
      calls += 1;
      return new Response("private provider response and email", { status });
    } });
    await assert.rejects(a.get(MARKER_KEY), (error) => {
      assert.equal(error.code, `drive_http_${status}`);
      assert.ok(!String(error).includes("private"));
      return true;
    });
    assert.equal(calls, 1);
  }
});

test("stale-parent publication writes no record", async () => {
  const { a, mock } = setup();
  const root = record(null, 1);
  await publish(a, stream, root);
  await publish(a, stream, record(root, 2));
  const before = writes(mock).length;
  await assert.rejects(publish(a, stream, record(root, 3, 1)), code("remote_changed"));
  assert.equal(writes(mock).length, before);
});

test("lost write reply is recovered by listing, with one record POST", async () => {
  const { a, mock } = setup();
  mock.loseNextWrite = true;
  assert.equal((await publish(a, stream, record(null, 1))).selected.revision, 1);
  assert.equal(writes(mock).length, 1);
});

test("a failed record attempt has a new stream listing before the next POST", async () => {
  const { a, mock } = setup();
  mock.failNextWrite = true;
  await publish(a, stream, record(null, 1));
  const paths = mock.requests.filter((r) => !r.path.endsWith("generateIds")).map((r) => r.method);
  assert.deepEqual(paths, ["GET", "POST", "GET", "POST", "GET"]);
});

test("listing failure after a write reports unknown and sends no further record", async () => {
  const { a, mock } = setup();
  mock.afterUpload = () => { mock.failLists = true; };
  await assert.rejects(publish(a, stream, record(null, 1)), code("outcome_unknown"));
  assert.equal(writes(mock).length, 1);
});

test("a corrupt selected record fails without selecting an alternative", async () => {
  const { a, mock } = setup();
  const root = record(null, 1);
  await publish(a, stream, root);
  [...mock.files.values()][0].bytes = Buffer.from("changed after publication");
  assert.equal((await listView(a, stream)).selected.token, root.token);
  await assert.rejects(readRecord(a, stream, root), code("record_digest_mismatch"));
});

test("counterexample: a delayed earlier sibling changes selection after prior success", async () => {
  const { a, b } = setup();
  const root = record(null, 1);
  await publish(a, stream, root);
  const delayed = record(root, 2, 200);
  const fast = record(root, 3, 300);
  let successfulBefore;
  await publish(a, stream, delayed, { afterList: async ({ attempt }) => {
    if (attempt === 0) successfulBefore = await publish(b, stream, fast);
  } });
  assert.equal(successfulBefore.selected.token, fast.token);
  assert.equal((await listView(b, stream)).selected.token, delayed.token);
});

test("counterexample: a cold client cannot recover a record omitted from its listing", async () => {
  const { a, b, mock } = setup();
  const root = record(null, 1);
  await publish(a, stream, root);
  for (const id of mock.files.keys()) mock.hidden.add(`b:${id}`);
  assert.equal((await listView(b, stream)).selected, null);
  assert.equal((await listView(a, stream)).selected.token, root.token);
  // Direct lookup can also lag because this adapter resolves keys by query.
  assert.equal(await b.get(`${stream}/${root.name}`), null);
});

test("native Rust RecordName and Mainline agree with the model on bounded histories", async () => {
  const directory = await mkdtemp(join(tmpdir(), "mallard-drive-native-"));
  try {
    const binary = join(directory, "select-mainline");
    execFileSync("rustc", ["--edition=2021", fileURLToPath(new URL("select-mainline.rs", import.meta.url)), "-o", binary], { stdio: "pipe" });
    const root = record(null, 1);
    const a = record(root, 2, 200);
    const b = record(root, 3, 100);
    const long = record(a, 4, 300);
    const sameId = makeRecord(root, "02".repeat(12), 200, "same id, different digest");
    const pool = [root.name, a.name, b.name, long.name, sameId.name,
      a.name.replace(/^0000000002/u, "0000000003"), "not-a-record.json"];
    const histories = [];
    for (let mask = 0; mask < 128; mask += 1) {
      const names = pool.filter((_, index) => mask & (1 << index));
      histories.push(["invalid", ...names.reverse(), ...names.slice(0, 2)]);
    }
    const output = execFileSync(binary, { input: histories.map((names) => names.join("\n")).join("\n\n"), encoding: "utf8" });
    const native = output.slice(0, -1).split("\n");
    assert.deepEqual(native, histories.map((names) => selectMainline(names).at(-1)?.name ?? ""));
    assert.equal(parseName(a.name).token, a.token);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("credentials require private regular files and distinct tokens", async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "mallard-drive-credential-")));
  try {
    const path = join(directory, "credential.json");
    const link = join(directory, "link.json");
    const value = { client_a_token: "fixture-a", client_b_token: "fixture-b", parent_id: "fixture_root" };
    await writeFile(path, JSON.stringify(value), { mode: 0o600 });
    assert.deepEqual(await readCredentials(path), value);
    await chmod(path, 0o644);
    await assert.rejects(readCredentials(path), code("credential_file_rejected"));
    await chmod(path, 0o600);
    await symlink(path, link);
    await assert.rejects(readCredentials(link), code("credential_file_location_rejected"));
    await writeFile(path, JSON.stringify({ ...value, client_b_token: "fixture-a" }));
    await assert.rejects(readCredentials(path), code("invalid_credentials"));
  } finally { await rm(directory, { recursive: true, force: true }); }
});

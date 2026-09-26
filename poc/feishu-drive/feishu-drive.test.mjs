import assert from "node:assert/strict";
import { mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { FeishuDrive, FeishuStore, PocError, RequestGate, SMALL_FILE_BYTES, expected, keyParts, readBounded } from "./drive.mjs";
import { MockDrive, apiError } from "./mock.mjs";
import { STREAM, listStream, makeRecord, publish, readCredentials, runProbe } from "./probe.mjs";
import { buildSelector } from "./selector.mjs";

const selector = buildSelector();
const id = (character) => character.repeat(24);
const objectKey = (bytes) => `.mallard/v1/repositories/${"0".repeat(32)}/_objects/${expected(bytes).sha256}`;
const rejects = (action, code) => assert.rejects(action, (error) => error instanceof PocError && error.code === code);

function fixture(pageSize = 2) {
  const mock = new MockDrive();
  const gate = new RequestGate(0);
  const writer = new FeishuDrive({ accessToken: "fixture-a", fetchImpl: mock.fetch, gate, pageSize });
  const reader = new FeishuDrive({ accessToken: "fixture-b", fetchImpl: mock.fetch, gate, pageSize });
  return { mock, writer, reader, store: new FeishuStore(writer, "root"), peer: new FeishuStore(reader, "root") };
}

test("complete synthetic probe exercises both HTTP upload paths and the production Rust selector", async () => {
  const { mock, writer, reader } = fixture();
  const result = await runProbe({ writer, reader, parentFolderToken: "root", selector, runId: "0123456789abcdef" });
  assert.equal(result.verdict, "PROBE_PASS");
  assert.equal(result.production_ready, false);
  assert.equal(result.duplicate_mode, "distinct_tokens");
  assert.equal(result.cleanup, "retained_no_delete_api");
  assert.ok(mock.requests.some((call) => call.query.has("page_token")));
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_part").length, 2);
  assert.ok(mock.requests.every((call) => ["GET", "POST"].includes(call.method)));
  assert.ok(!JSON.stringify(result).includes("fixture-a"));
});

test("production record builder accepts 223-character names and selector ignores foreign files", () => {
  const root = makeRecord(selector, id("a"), null, 100);
  const next = makeRecord(selector, id("b"), root, 101);
  assert.equal(next.name.length, 223);
  const result = selector({ action: "select", names: [next.name, "_head.json", root.name, next.name] });
  assert.deepEqual(result.chain, [root.name, next.name]);
  assert.equal(result.ignored, 1);
});

test("all pages and recursive discovery are read, and duplicate tokens across pages are harmless", async () => {
  const { mock, writer, store } = fixture(1);
  const one = Buffer.from("one");
  const two = Buffer.from("two");
  await store.put(objectKey(one), one);
  await store.put(objectKey(two), two);
  assert.deepEqual(await store.listKeys(".mallard/v1/repositories"), [objectKey(one), objectKey(two)].sort());
  let page = 0;
  const entry = { name: "a", token: "a", type: "file", parent_token: "root" };
  mock.hook = () => Response.json({ code: 0, data: { files: [entry], has_more: page++ === 0,
    next_page_token: "again" } });
  assert.equal((await writer.listFolder("root")).length, 1);
});

test("a failed later page fails the whole listing", async () => {
  const { mock, writer } = fixture(1);
  mock.add("root", "a", "file", Buffer.from("a"));
  mock.add("root", "b", "file", Buffer.from("b"));
  mock.hook = (call) => call.query.has("page_token") ? apiError(1061004, 403) : null;
  await rejects(() => writer.listFolder("root"), "http_403");
});

test("repeated and missing continuation tokens fail closed", async () => {
  const { mock, writer } = fixture();
  mock.hook = () => Response.json({ code: 0, data: { files: [], has_more: true, next_page_token: "repeat" } });
  await rejects(() => writer.listFolder("root"), "invalid_cursor");
  mock.hook = () => Response.json({ code: 0, data: { files: [], has_more: true } });
  await rejects(() => writer.listFolder("root"), "invalid_cursor");
});

test("incomplete listing envelopes and entries from another parent are rejected", async () => {
  const { mock, writer } = fixture();
  mock.hook = () => Response.json({ code: 0, data: { files: [] } });
  await rejects(() => writer.listFolder("root"), "invalid_listing");
  mock.hook = () => Response.json({ code: 0, data: { files: [
    { name: "a", token: "a", parent_token: "another", type: "file" },
  ], has_more: false } });
  await rejects(() => writer.listFolder("root"), "invalid_listing_entry");
});

test("HTTP 200 with a Feishu error is failure, and mutation requests are not retried", async () => {
  const { mock, writer } = fixture();
  mock.hook = () => apiError(1061045);
  await rejects(() => writer.upload("root", "record.json", Buffer.from("x")), "feishu_1061045");
  assert.equal(mock.requests.length, 1);
});

test("permission, quota, rate limit, deleted folder, and redirects are not empty success", async () => {
  const { mock, writer } = fixture();
  for (const status of [401, 403, 404, 429, 500, 302]) {
    mock.hook = () => apiError(1061001, status);
    await rejects(() => writer.listFolder("root"), `http_${status}`);
  }
  mock.hook = () => apiError(1061101);
  await rejects(() => writer.upload("root", "a", Buffer.from("x")), "feishu_1061101");
});

test("foreign response bodies and transport exceptions do not appear in errors", async () => {
  const { mock, writer } = fixture();
  mock.hook = () => { throw new Error("secret-token /private/local/path"); };
  await rejects(() => writer.listFolder("root"), "transport_failure");
  mock.hook = () => new Response("secret-token /private/local/path");
  await rejects(() => writer.listFolder("root"), "invalid_json");
});

test("identical duplicate files resolve as one key, and every copy is hash checked", async () => {
  const { mock, writer, store } = fixture();
  const bytes = Buffer.from("same");
  const key = objectKey(bytes);
  await store.put(key, bytes);
  const location = await store.location(key);
  const duplicate = await writer.upload(location.parent, location.name, bytes);
  assert.deepEqual(await store.get(key, expected(bytes)), bytes);
  assert.equal((await store.listKeys(".mallard/v1/repositories")).length, 1);
  mock.entries.get(duplicate).bytes = Buffer.from("evil");
  await rejects(() => store.get(key, expected(bytes)), "integrity_mismatch");
  await rejects(() => store.put(key, bytes), "integrity_mismatch");
});

test("duplicate folder paths fail instead of selecting the first folder", async () => {
  const { mock, store } = fixture();
  mock.add("root", ".mallard", "folder");
  mock.add("root", ".mallard", "folder");
  await rejects(() => store.get(".mallard/_storage.json", expected(Buffer.from("x"))), "ambiguous_folder");
});

test("folder creation races are detected by the post-create listing", async () => {
  const { mock, store } = fixture();
  let inserted = false;
  mock.hook = (call) => {
    if (call.path === "/files/create_folder" && !inserted) {
      inserted = true;
      mock.add("root", ".mallard", "folder");
    }
    return null;
  };
  await rejects(() => store.put(".mallard/_storage.json", Buffer.from("x")), "ambiguous_folder");
});

test("a missing key is distinct from a missing or inaccessible configured folder", async () => {
  const { store, mock } = fixture();
  assert.equal(await store.get(".mallard/_storage.json", expected(Buffer.from("x"))), null);
  mock.entries.delete("root");
  await rejects(() => store.get(".mallard/_storage.json", expected(Buffer.from("x"))), "http_404");
});

test("upload names must remain exact and acknowledged writes must be listed", async () => {
  const { mock, store } = fixture();
  mock.afterUpload = (_token, entry) => { entry.name += "-renamed"; };
  await rejects(() => store.put(".mallard/_storage.json", Buffer.from("x")), "file_visibility_gap");
});

test("automatic multipart handles more than 20 MiB and verifies exact bytes", async () => {
  const { mock, writer } = fixture();
  const bytes = Buffer.alloc(SMALL_FILE_BYTES + 17, 0x41);
  const token = await writer.upload("root", "large.bin", bytes);
  assert.deepEqual(await writer.download(token, expected(bytes)), bytes);
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_part").length, 6);
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_all").length, 0);
});

test("bad multipart plans and failed parts cannot reach upload_finish", async () => {
  const { mock, writer } = fixture();
  mock.hook = (call) => call.path === "/files/upload_prepare"
    ? Response.json({ code: 0, data: { upload_id: "upload", block_size: 0, block_num: 1 } }) : null;
  await rejects(() => writer.upload("root", "a", Buffer.from("a"), { multipart: true }), "invalid_multipart_plan");
  mock.hook = (call) => call.path === "/files/upload_part" ? apiError(1061021, 400) : null;
  await rejects(() => writer.upload("root", "a", Buffer.from("a"), { multipart: true }), "http_400");
  assert.ok(!mock.requests.some((call) => call.path === "/files/upload_finish"));
});

test("missing final acknowledgement is not blindly retried", async () => {
  const { mock, writer } = fixture();
  mock.afterUpload = () => { throw new Error("lost response"); };
  await rejects(() => writer.upload("root", "a", Buffer.from("a"), { multipart: true }), "transport_failure");
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_finish").length, 1);
});

test("download rejects wrong digest, truncated content, excess content, and partial response", async () => {
  const { mock, writer } = fixture();
  const bytes = Buffer.from("safe");
  for (const [actual, code] of [["evil", "integrity_mismatch"], ["saf", "integrity_mismatch"], ["safeextra", "response_too_large"]]) {
    mock.hook = () => new Response(actual);
    await rejects(() => writer.download("file", expected(bytes)), code);
  }
  mock.hook = () => new Response(bytes, { status: 206 });
  await rejects(() => writer.download("file", expected(bytes)), "http_206");
});

test("bounded response reader rejects content-length and streaming overflow", async () => {
  let cancelled = false;
  const oversized = new ReadableStream({ cancel() { cancelled = true; } });
  await rejects(() => readBounded(new Response(oversized, { headers: { "content-length": "999999999" } }), 4), "response_too_large");
  assert.equal(cancelled, true);
  const body = new ReadableStream({ start(controller) { controller.enqueue(Buffer.alloc(5)); controller.close(); } });
  await rejects(() => readBounded(new Response(body), 4), "response_too_large");
});

test("foreign record names do not block the first publication", async () => {
  const { store, writer } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  const location = await store.location(`${STREAM}/${root.name}`, true);
  await writer.upload(location.parent, "_head.json", Buffer.from("unrelated fixture"));
  assert.equal(await publish(store, selector, root), "selected");
  assert.equal((await listStream(store, selector)).ignored, 1);
});

test("stale parents fail before a record upload; malformed record bytes fail before any request", async () => {
  const { mock, store } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  const winner = makeRecord(selector, id("b"), root, 101);
  const stale = makeRecord(selector, id("c"), root, 102);
  await publish(store, selector, root);
  await publish(store, selector, winner);
  mock.requests = [];
  await rejects(() => publish(store, selector, stale), "remote_changed");
  assert.ok(!mock.requests.some((call) => call.path === "/files/upload_all"));
  mock.requests = [];
  stale.bytes = Buffer.from(stale.bytes.toString().replace("transport-poc", "transport-bad"));
  await rejects(() => publish(store, selector, stale), "record_name_mismatch");
  assert.equal(mock.requests.length, 0);
});

test("expired admission listing sends no record", async () => {
  const { mock, store } = fixture();
  let time = 0;
  const root = makeRecord(selector, id("a"), null, 100);
  await rejects(() => publish(store, selector, root, { now: () => { time += 10_001; return time; } }), "stale_admission_listing");
  assert.ok(!mock.requests.some((call) => call.path === "/files/upload_all"));
});

test("a lost acknowledgement is resolved by listing, with exactly one record request", async () => {
  const { mock, store } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  mock.afterUpload = () => { throw new Error("lost acknowledgement"); };
  assert.equal(await publish(store, selector, root), "selected_after_lost_ack");
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_all").length, 1);
});

test("an unacknowledged and invisible record reports unknown; no resend", async () => {
  const { mock, store } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  mock.afterUpload = (token) => { mock.hidden.add(token); throw new Error("lost acknowledgement"); };
  await rejects(() => publish(store, selector, root), "outcome_unknown");
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_all").length, 1);
});

test("acknowledged but unlisted records fail the transport visibility gate", async () => {
  const { mock, store } = fixture();
  mock.afterUpload = (token) => mock.hidden.add(token);
  await rejects(() => publish(store, selector, makeRecord(selector, id("a"), null, 100)), "record_visibility_gap");
});

test("post-write list failure reports unknown and keeps the record", async () => {
  const { mock, store } = fixture();
  const record = makeRecord(selector, id("a"), null, 100);
  mock.afterUpload = () => { mock.hook = () => apiError(1061001, 500); };
  await rejects(() => publish(store, selector, record), "outcome_unknown");
  assert.ok([...mock.entries.values()].some((entry) => entry.name === record.name));
});

test("losing siblings stay stored and a longer losing branch cannot win", async () => {
  const { store, writer } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  await publish(store, selector, root);
  const winner = makeRecord(selector, id("b"), root, 101);
  const loser = makeRecord(selector, id("c"), root, 102);
  const descendant = makeRecord(selector, id("d"), loser, 1);
  const location = await store.location(`${STREAM}/${winner.name}`);
  for (const record of [loser, descendant, winner]) await writer.upload(location.parent, record.name, record.bytes);
  assert.deepEqual((await listStream(store, selector)).chain, [root.name, winner.name]);
  assert.deepEqual(await store.get(`${STREAM}/${loser.name}`, expected(loser.bytes)), loser.bytes);
});

test("a sibling that wins before confirmation fails this push without retry or deletion", async () => {
  const { mock, store, peer } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  await publish(store, selector, root);
  const winner = makeRecord(selector, id("b"), root, 101);
  const loser = makeRecord(selector, id("c"), root, 102);
  mock.requests = [];
  // The other writer lands after this writer's admission listing.
  mock.afterUpload = (_token, entry) => {
    if (entry.name === loser.name) mock.add(entry.parent_token, winner.name, "file", winner.bytes);
  };
  await rejects(() => publish(store, selector, loser), "remote_changed");
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_all").length, 1);
  assert.ok(mock.requests.every((call) => call.method === "GET" || call.method === "POST"));
  assert.equal((await listStream(peer, selector)).chain.at(-1), winner.name);
  assert.deepEqual(await peer.get(`${STREAM}/${loser.name}`, expected(loser.bytes)), loser.bytes);
});

test("a delayed admitted writer can displace a confirmed push even with complete listings", async () => {
  const { mock, store, peer, writer } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  await publish(store, selector, root);
  const delayed = makeRecord(selector, id("b"), root, 101);
  const firstToFinish = makeRecord(selector, id("c"), root, 102);
  const location = await store.location(`${STREAM}/${delayed.name}`);
  assert.equal((await listStream(store, selector)).chain.at(-1), root.name);
  const admittedAt = Date.now();
  mock.requests = [];
  assert.equal(await publish(peer, selector, firstToFinish), "selected");
  assert.equal((await listStream(store, selector)).chain.at(-1), firstToFinish.name);
  // Resume the first writer after admission, within the 10-second window.
  // No listing entry is hidden and each record is sent once.
  assert.ok(Date.now() - admittedAt <= 10_000);
  await writer.upload(location.parent, delayed.name, delayed.bytes);
  assert.equal(mock.hidden.size, 0);
  assert.equal(mock.requests.filter((call) => call.path === "/files/upload_all").length, 2);
  assert.deepEqual((await listStream(store, selector)).chain, [root.name, delayed.name]);
  assert.deepEqual((await listStream(peer, selector)).chain, [root.name, delayed.name]);
  assert.deepEqual(await peer.get(`${STREAM}/${firstToFinish.name}`, expected(firstToFinish.bytes)), firstToFinish.bytes);
});

test("a hidden earlier sibling demonstrates the unresolved cross-client consistency risk", async () => {
  const { mock, store, peer, writer } = fixture();
  const root = makeRecord(selector, id("a"), null, 100);
  await publish(store, selector, root);
  const hidden = makeRecord(selector, id("b"), root, 101);
  const visible = makeRecord(selector, id("c"), root, 102);
  const location = await store.location(`${STREAM}/${hidden.name}`);
  const hiddenToken = await writer.upload(location.parent, hidden.name, hidden.bytes);
  mock.hidden.add(hiddenToken);
  assert.equal(await publish(peer, selector, visible), "selected");
  mock.hidden.clear();
  assert.equal((await listStream(peer, selector)).chain.at(-1), hidden.name);
  // A successful local sample cannot certify globally complete listings.
});

test("folder capacity and unsafe logical keys fail explicitly", async () => {
  const { mock, writer } = fixture(200);
  for (let index = 0; index < 1500; index += 1) mock.add("root", `f${index}`, "file", Buffer.alloc(0));
  await rejects(() => writer.upload("root", "extra", Buffer.from("x")), "http_400");
  mock.add("root", "over-limit", "file", Buffer.alloc(0));
  await rejects(() => writer.listFolder("root"), "folder_entry_limit");
  for (const key of ["/tmp/file", ".mallard/../file", ".mallard/v1/repositories/a/../secret", ".mallard/v1/repositories/a/" + "a".repeat(251)]) {
    assert.throws(() => keyParts(key), PocError);
  }
});

test("the request gate serializes calls from two clients", async () => {
  const gate = new RequestGate(0);
  let running = 0;
  let maximum = 0;
  await Promise.all(Array.from({ length: 4 }, () => gate.run(async () => {
    running += 1;
    maximum = Math.max(maximum, running);
    await new Promise((done) => setImmediate(done));
    running -= 1;
  })));
  assert.equal(maximum, 1);
});

test("credential reader accepts private temporary fixtures and rejects broad modes and symlinks", async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "feishu-credential-test-")));
  try {
    const good = join(directory, "good.json");
    const data = { access_token: "fixture-a", peer_access_token: "fixture-b", parent_folder_token: "root" };
    await writeFile(good, JSON.stringify(data), { mode: 0o600 });
    assert.deepEqual(await readCredentials(good), data);
    const broad = join(directory, "broad.json");
    await writeFile(broad, JSON.stringify(data), { mode: 0o644 });
    await rejects(() => readCredentials(broad), "unsafe_credential_file");
    const link = join(directory, "link.json");
    await symlink(good, link);
    await rejects(() => readCredentials(link), "credential_file_not_canonical");
    const extra = join(directory, "extra.json");
    await writeFile(extra, JSON.stringify({ ...data, api_origin: "https://example.invalid" }), { mode: 0o600 });
    await rejects(() => readCredentials(extra), "invalid_credentials");
    await rejects(() => readCredentials("relative.json"), "credential_path_must_be_absolute");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

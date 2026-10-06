import assert from "node:assert/strict";
import test from "node:test";
import { validateCredential } from "./onedrive-app-folder-poc.mjs";
import { createFileOnce, executeCli, runImmutableProbe } from "./immutable-storage-poc.mjs";

const TOKEN_A = "synthetic-onedrive-immutable-actor-a";
const TOKEN_B = "synthetic-onedrive-immutable-actor-b";
const GRAPH = "https://graph.microsoft.com";
const credential = validateCredential({
  client_a: { access_token: TOKEN_A, granted_scopes: "Files.ReadWrite.AppFolder" },
  client_b: { access_token: TOKEN_B, granted_scopes: "Files.ReadWrite.AppFolder" },
});
const json = (value, status = 200) => new Response(JSON.stringify(value), {
  status, headers: { "Content-Type": "application/json" },
});
const errorResponse = (status) => json({ error: { code: "test_provider_error" } }, status);

function fakeGraph(options = {}) {
  const items = new Map();
  if (options.preexisting) {
    items.set("item-0", { id: "item-0", etag: "preexisting",
      name: `mallard-onedrive-immutable-${"42".repeat(12)}-claim.bin`,
      bytes: Buffer.from("preexisting file") });
  }
  const requests = [];
  let revision = 0;
  let listingFinished = false;
  const describe = (item) => ({ id: item.id, name: item.name,
    eTag: item.etag, size: item.bytes.length, file: {} });
  const fetchImpl = async (target, init = {}) => {
    const url = new URL(target);
    const method = init.method ?? "GET";
    const headers = new Headers(init.headers);
    requests.push({ url: url.href, method, headers });
    if (url.origin === "https://download.test.invalid") {
      assert.equal(headers.has("authorization"), false);
      const item = items.get(url.pathname.slice(1));
      assert.ok(item);
      return new Response(item.bytes);
    }
    assert.equal(url.origin, GRAPH, "No token may leave the Graph origin");
    const auth = headers.get("authorization");
    assert.ok([`Bearer ${TOKEN_A}`, `Bearer ${TOKEN_B}`].includes(auth));
    if (url.pathname === "/v1.0/me/drive/special/approot") {
      return json({ id: options.rootsDiffer && auth === `Bearer ${TOKEN_B}` ? "other" : "root",
        name: "Test app", eTag: "root-etag", folder: {}, specialFolder: { name: "approot" },
        parentReference: { driveId: "drive", driveType: "personal" } });
    }
    if (url.pathname === "/v1.0/me/drive/root") {
      return options.allowOutside ? json({ id: "outside" }) : errorResponse(403);
    }
    if (url.pathname === "/v1.0/drives/drive/items/root/children") {
      const page = Number(url.searchParams.get("page") ?? "0");
      const values = [...items.values()].filter((item) =>
        !(options.omitFile && item.name.endsWith("-empty.bin")));
      const result = { value: values.slice(page, page + 1).map(describe) };
      if (page + 1 < values.length) {
        result["@odata.nextLink"] = `${GRAPH}${url.pathname}?page=${page + 1}`;
      } else {
        listingFinished = true;
      }
      if (options.cursor === "foreign") result["@odata.nextLink"] = "https://foreign.test.invalid/list";
      if (options.cursor === "other_folder") result["@odata.nextLink"] = `${GRAPH}/v1.0/drives/drive/items/other/children`;
      if (options.cursor === "repeat") result["@odata.nextLink"] = url.href;
      if (options.duplicate && page === 1) result.value = [describe(values[0])];
      return json(result);
    }
    const named = url.pathname.match(/^\/v1\.0\/drives\/drive\/items\/root:\/(.+?)(:\/content)?$/u);
    if (named) {
      const name = named[1];
      let item = [...items.values()].find((candidate) => candidate.name === name);
      if (method === "GET") {
        if (listingFinished && options.changeDuringCleanup && item?.name.endsWith("-claim.bin")) {
          item.bytes = Buffer.from("unrelated replacement bytes");
          item.etag = "foreign-revision";
        }
        return item ? json(describe(item)) : errorResponse(404);
      }
      assert.equal(method, "PUT");
      assert.equal(url.searchParams.get("@microsoft.graph.conflictBehavior"), "fail");
      assert.equal(init.redirect, "error");
      if (item && !options.overwrite) return errorResponse(409);
      if (options.unknownEmptyBeforeCommit && name.endsWith("-empty.bin")) {
        throw new Error("Connection ended before the result was known");
      }
      revision += 1;
      item = { id: item?.id ?? `item-${revision}`, name, etag: `rev-${revision}`,
        bytes: Buffer.from(init.body) };
      items.set(item.id, item);
      if (options.loseEmptyResponse && name.endsWith("-empty.bin")) {
        throw new Error(`Private response ${TOKEN_A} https://private.invalid/signed`);
      }
      return json(describe(item), 201);
    }
    const byId = url.pathname.match(/^\/v1\.0\/drives\/drive\/items\/(item-\d+)(\/content)?$/u);
    assert.ok(byId, "Unexpected mock request");
    const item = items.get(byId[1]);
    if (!item) return errorResponse(404);
    if (method === "DELETE") {
      assert.equal(headers.get("if-match"), item.etag);
      if (options.deleteConflict) return errorResponse(412);
      items.delete(item.id);
      return new Response(null, { status: 204 });
    }
    assert.equal(method, "GET");
    if (byId[2]) return new Response(null, { status: 302,
      headers: { location: `https://download.test.invalid/${item.id}` } });
    return json(describe(item));
  };
  return { fetchImpl, requests, items };
}

async function run(options = {}) {
  const fake = fakeGraph(options);
  const result = await runImmutableProbe({ credential, fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {}, randomBytesImpl: () => Buffer.alloc(12, 0x42) });
  return { ...fake, result };
}

test("observes one create winner, preserves bytes, and lists both files across pages", async () => {
  const { result, requests, items } = await run();
  assert.equal(result.verdict, "PASS");
  assert.equal(result.productionReady, false);
  assert.deepEqual(result.listing, { pages: 2, matched: 2 });
  assert.deepEqual(result.cleanup, { status: "complete" });
  assert.equal(items.size, 0);
  assert.equal(requests.filter((request) => request.method === "PUT").length, 4);
  assert.equal(requests.filter((request) => request.method === "DELETE").length, 2);
  assert.equal(requests.filter((request) => request.url.includes("/children")).length, 2);
});

test("fails when the provider overwrites a competing claim", async () => {
  const { result, requests } = await run({ overwrite: true });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "create_race_did_not_have_one_winner");
  assert.equal(requests.filter((request) => request.method === "PUT").length, 2);
});

test("does not resend a write after the server commits and loses its response", async () => {
  const { result, requests, items } = await run({ loseEmptyResponse: true });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "request_result_unknown");
  assert.equal(requests.filter((request) => request.method === "PUT" &&
    request.url.includes("-empty.bin")).length, 1);
  assert.equal(items.size, 0);
  const output = JSON.stringify(result);
  assert.equal(output.includes(TOKEN_A), false);
  assert.equal(output.includes("private.invalid"), false);
});

test("does not report complete cleanup from absence after an unknown write", async () => {
  const { result, requests } = await run({ unknownEmptyBeforeCommit: true });
  assert.equal(result.failureCode, "request_result_unknown");
  assert.equal(result.cleanup.status, "required");
  assert.equal(typeof result.probeId, "string");
  assert.equal(requests.filter((request) => request.method === "PUT" &&
    request.url.includes("-empty.bin")).length, 1);
});

test("does not write or delete when a probe name is already present", async () => {
  const { result, requests, items } = await run({ preexisting: true });
  assert.equal(result.failureCode, "probe_name_already_exists");
  assert.equal(items.size, 1);
  assert.equal(requests.filter((request) => ["PUT", "DELETE"].includes(request.method)).length, 0);
});

test("fails when the other client cannot list a confirmed object", async () => {
  const { result } = await run({ omitFile: true });
  assert.equal(result.failureCode, "listing_omitted_probe_file");
});

for (const [cursor, code] of [
  ["foreign", "list_cursor_left_folder"],
  ["other_folder", "list_cursor_left_folder"],
  ["repeat", "repeated_list_cursor"],
]) {
  test(`rejects a ${cursor} nextLink before following it`, async () => {
    const { result, requests } = await run({ cursor });
    assert.equal(result.failureCode, code);
    assert.equal(requests.filter((request) => request.url.includes("/children")).length, 1);
  });
}

test("rejects duplicate entries across pages", async () => {
  const { result } = await run({ duplicate: true });
  assert.equal(result.failureCode, "duplicate_probe_item");
});

test("does not delete probe content replaced by an unrelated writer", async () => {
  const { result, items } = await run({ changeDuringCleanup: true });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.cleanup.status, "required");
  assert.equal(items.size, 1);
  assert.equal([...items.values()][0].bytes.toString(), "unrelated replacement bytes");
});

test("reports conditional delete conflicts without an unconditional retry", async () => {
  const { result, requests } = await run({ deleteConflict: true });
  assert.equal(result.cleanup.status, "required");
  assert.equal(requests.filter((request) => request.method === "DELETE").length, 2);
});

test("requires two authorizations in the same App Folder before writing", async () => {
  const { result, requests } = await run({ rootsDiffer: true });
  assert.equal(result.failureCode, "client_app_folders_do_not_match");
  assert.equal(requests.filter((request) => request.method === "PUT").length, 0);
});

test("requires denial outside the App Folder before writing", async () => {
  const { result, requests } = await run({ allowOutside: true });
  assert.equal(result.failureCode, "outside_app_folder_access_was_allowed");
  assert.equal(requests.filter((request) => request.method === "PUT").length, 0);
});

test("refuses a non-probe path before making a request", async () => {
  await assert.rejects(createFileOnce({}, {}, "../file", Buffer.from("test")),
    { code: "invalid_probe_name" });
});

test("requires an explicit live command and a credential file", async () => {
  assert.match((await executeCli(["--help"])).help, /live --credentials/u);
  await assert.rejects(executeCli([]), { code: "invalid_arguments" });
});

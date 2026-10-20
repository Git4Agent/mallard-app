import test from "node:test";
import assert from "node:assert/strict";
import { configuration } from "./auth.mjs";
import { GraphClient, folderIdentity, sharingUrl } from "./graph.mjs";
import { runProbe } from "./probe.mjs";

const config = { ...configuration({ clientId: "11111111-1111-4111-8111-111111111111",
  tenant: "22222222-2222-4222-8222-222222222222" }),
sharingUrl: "https://fixture.sharepoint.com/:f:/s/test/PRIVATE_SHARED_LINK" };
const root = { driveId: "private-drive", id: "private-folder" };
const prefix = `mallard-ms-poc-${"12".repeat(12)}`;
const response = (value, status = 200) => new Response(JSON.stringify(value), { status });

function fixture(options = {}) {
  const calls = [];
  const files = new Map();
  const uploads = new Map();
  const sessions = [];
  let serial = 0;
  const userIds = ["33333333-3333-4333-8333-333333333333",
    options.sameUser ? "33333333-3333-4333-8333-333333333333" : "44444444-4444-4444-8444-444444444444"];
  const folder = { id: root.id, name: "PRIVATE_FOLDER_NAME", folder: {},
    parentReference: { driveId: root.driveId } };
  const metadata = (file) => ({ id: file.id, name: file.name, eTag: file.etag, size: file.bytes.length,
    file: {}, parentReference: { driveId: root.driveId, id: root.id },
    "@microsoft.graph.downloadUrl": `https://fixture.sharepoint.com/download/${file.id}?signature=PRIVATE` });
  function commit(name, bytes) {
    const file = { id: `item-${++serial}`, name, bytes, etag: `revision-${serial}` };
    files.set(name, file);
    return file;
  }
  if (options.preexisting) commit(`${prefix}-a.bin`, Buffer.from("existing user file"));
  const fetchImpl = async (target, init) => {
    const url = new URL(target);
    const method = init.method;
    calls.push({ url, method, headers: init.headers });
    assert.equal(init.redirect, "error");
    assert.notEqual(method, "DELETE");
    if (url.hostname === "fixture.sharepoint.com") {
      assert.equal(init.headers?.Authorization, undefined, "capability URL must not receive bearer token");
      if (url.pathname.startsWith("/download/")) {
        const id = url.pathname.split("/").at(-1);
        const file = [...files.values()].find((item) => item.id === id);
        return new Response(options.corruptDownload ? Buffer.alloc(file.bytes.length) : file.bytes);
      }
      const upload = uploads.get(url.pathname);
      assert.ok(upload, "only returned upload URLs may be used");
      assert.equal(method, "PUT");
      const [, start, end, total] = /^bytes (\d+)-(\d+)\/(\d+)$/u.exec(init.headers["Content-Range"]);
      assert.equal(Number(start), upload.received);
      assert.equal(init.body.length, Number(end) - Number(start) + 1);
      assert.equal(Number(init.headers["Content-Length"]), init.body.length);
      upload.chunks.push(init.body);
      upload.received += init.body.length;
      if (upload.received < Number(total)) return response({ nextExpectedRanges: [`${upload.received}-`] }, 202);
      assert.equal(upload.received, Number(total));
      const exists = files.has(upload.name);
      if (exists && !options.overwrite && !options.rename) return response({ error: "PRIVATE_ERROR" }, 409);
      const name = exists && options.rename ? `${upload.name}-renamed` : upload.name;
      const file = commit(name, Buffer.concat(upload.chunks));
      if (options.loseResponse && name.endsWith("-a.bin")) throw new Error("PRIVATE_TOKEN_AND_LINK");
      return response(metadata(file), 201);
    }
    assert.equal(url.origin, "https://graph.microsoft.com");
    const token = init.headers.Authorization;
    const actor = token.endsWith("-b") ? 1 : 0;
    assert.match(token, /^Bearer (original|renewed)-(a|b)$/u);
    assert.equal(init.headers.Prefer, undefined, "probe must not redeem sharing links");
    const path = url.pathname.replace("/v1.0/", "");
    if (path === "me") return response({ id: options.identityMismatch ? "wrong" : userIds[actor] });
    if (path.startsWith("shares/")) {
      const resolved = options.differentRoot && actor === 1 ? { ...folder, id: "another-folder" } : folder;
      return response(options.shortcut ? { id: "shortcut", remoteItem: resolved } : resolved);
    }
    if (path === `drives/${root.driveId}`) return response({ id: root.driveId, driveType: options.driveType ?? "documentLibrary" });
    if (path === `drives/${root.driveId}/items/${root.id}`) return response(folder);
    if (path === `drives/${root.driveId}/items/another-folder`) return response({ ...folder, id: "another-folder" });
    if (path.endsWith("/children")) {
      if (options.denyListing) return response({ error: "PRIVATE_ERROR" }, 403);
      if (options.nextLink) return response({ value: [], "@odata.nextLink": options.nextLink });
      let items = [...files.values()].map(metadata);
      if (options.hideListing && actor === 1) items = items.filter((item) => !item.name.endsWith("-a.bin"));
      const offset = Number(url.searchParams.get("$skiptoken") ?? 0);
      const value = options.noPagination ? items : items.slice(offset, offset + 1);
      return response({ value, ...(!options.noPagination && offset + 1 < items.length
        ? { "@odata.nextLink": `https://graph.microsoft.com/v1.0/${path}?$skiptoken=${offset + 1}` } : {}) });
    }
    if (path.includes(":/")) {
      const name = path.split(":/")[1].split(":")[0];
      if (path.endsWith("/createUploadSession")) {
        assert.equal(method, "POST");
        const body = JSON.parse(init.body);
        assert.equal(body.item["@microsoft.graph.conflictBehavior"], "fail");
        assert.equal(body.item.name, name);
        assert.equal(body.deferCommit, false);
        if (options.conflictAtStart && files.has(name)) return response({}, 409);
        const uploadPath = `/upload/${uploads.size + 1}`;
        uploads.set(uploadPath, { name, chunks: [], received: 0 });
        return response({ uploadUrl: options.uploadUrl ?? `https://fixture.sharepoint.com${uploadPath}?signature=PRIVATE` });
      }
      const file = files.get(name);
      return file ? response(metadata(file)) : response({}, 404);
    }
    const id = path.split("/").at(-1);
    const file = [...files.values()].find((item) => item.id === id);
    assert.ok(file, "unexpected Graph path");
    return response(metadata(file));
  };
  const authorizeImpl = async () => {
    const index = sessions.length;
    const label = index === 0 ? "a" : "b";
    const account = { localAccountId: userIds[index], homeAccountId: `${userIds[index]}.home` };
    const grant = { account, tenantId: config.tenant, scopes: config.scopes,
      expiresOn: new Date(Date.now() + 3600_000), accessToken: `original-${label}` };
    const session = { grant, cleared: false, refreshed: false, pca: {
      async acquireTokenSilent(request) {
        assert.equal(request.forceRefresh, true);
        assert.equal(request.authority, config.authority);
        session.refreshed = true;
        return { ...grant, accessToken: `renewed-${label}` };
      },
      async clearCache() { session.cleared = true; },
    } };
    sessions.push(session);
    return session;
  };
  return { calls, files, sessions, uploads, options: { authorizeImpl,
    graphOptions: { fetchImpl }, randomBytesImpl: () => Buffer.alloc(12, 0x12) } };
}

test("inspect proves two users and a canonical root using only Graph reads", async () => {
  const f = fixture({ shortcut: true });
  const result = await runProbe(config, f.options);
  assert.equal(result.verdict, "PASS");
  assert.equal(result.mode, "inspect");
  assert.equal(result.productionReady, false);
  assert.equal(result.filesMayRemain, false);
  assert.ok(f.calls.every((call) => call.method === "GET"));
  assert.equal(f.sessions.length, 2);
  assert.ok(f.sessions.every((session) => session.refreshed && session.cleared && session.grant.accessToken === ""));
  assert.doesNotMatch(JSON.stringify(result), /PRIVATE|private-|original-|renewed-|33333333|44444444|https:/u);
});

test("write probe exercises cross-user integrity, large create race, conflict and pagination", async () => {
  const f = fixture();
  const result = await runProbe(config, { ...f.options, writeProbe: true });
  assert.equal(result.verdict, "PASS", JSON.stringify(result));
  assert.equal(result.stages.length, 7);
  assert.deepEqual(result.listingPages, [3, 3]);
  assert.equal(f.files.size, 3);
  assert.equal(f.files.get(`${prefix}-claim.bin`).bytes.length, 640 * 1024);
  assert.equal(result.filesMayRemain, true);
  assert.equal(result.cleanup, "no_deletions");
  assert.ok(f.sessions.every((session) => session.cleared));
  assert.doesNotMatch(JSON.stringify(result), /PRIVATE|private-|original-|renewed-|https:/u);
});

test("existing-name conflict can be rejected when opening the upload session", async () => {
  const f = fixture({ conflictAtStart: true });
  const result = await runProbe(config, { ...f.options, writeProbe: true });
  assert.equal(result.verdict, "PASS");
});

for (const [name, options, failureCode] of [
  ["same Microsoft user twice", { sameUser: true }, "two_distinct_users_required"],
  ["mismatched Graph identity", { identityMismatch: true }, "identity_mismatch"],
  ["different canonical roots", { differentRoot: true }, "shared_roots_differ"],
  ["personal OneDrive outside POC scope", { driveType: "personal" }, "unsupported_drive_type"],
  ["denied listing", { denyListing: true }, "access_denied"],
  ["existing probe name", { preexisting: true }, "probe_name_exists"],
]) {
  test(`rejects ${name} before sending any writes`, async () => {
    const f = fixture(options);
    const result = await runProbe(config, { ...f.options, writeProbe: true });
    assert.equal(result.verdict, "FAIL");
    assert.equal(result.failureCode, failureCode);
    assert.ok(f.calls.every((call) => call.method === "GET"));
    assert.ok(f.sessions.every((session) => session.cleared));
  });
}

for (const [name, options, failureCode] of [
  ["replacement at commit", { overwrite: true }, "create_race_did_not_have_one_winner"],
  ["silent rename", { rename: true }, "create_race_did_not_have_one_winner"],
  ["corrupted download", { corruptDownload: true }, "digest_mismatch"],
  ["files hidden from collaborator listing", { hideListing: true }, "missing_or_renamed_probe_file"],
  ["pagination never exercised", { noPagination: true }, "pagination_not_exercised"],
  ["untrusted upload host", { uploadUrl: "https://attacker.invalid/upload" }, "invalid_transfer_url"],
]) {
  test(`does not qualify ${name}`, async () => {
    const f = fixture(options);
    const result = await runProbe(config, { ...f.options, writeProbe: true });
    assert.equal(result.verdict, "FAIL");
    assert.equal(result.failureCode, failureCode);
    assert.equal(result.productionReady, false);
    assert.equal(result.filesMayRemain, true);
    assert.ok(f.sessions.every((session) => session.cleared));
    assert.ok(f.calls.every((call) => !call.url.href.includes("attacker.invalid")));
  });
}

test("a lost final response causes one read-only check and no upload resend", async () => {
  const f = fixture({ loseResponse: true });
  const result = await runProbe(config, { ...f.options, writeProbe: true });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "transport_failed");
  assert.equal(result.afterFailure, "item_observed_unqualified");
  assert.equal(f.calls.filter((call) => call.method === "POST").length, 1);
  assert.equal(f.calls.filter((call) => call.method === "PUT").length, 1);
  assert.equal(f.calls.at(-1).method, "GET");
  assert.equal(f.files.size, 1);
  assert.doesNotMatch(JSON.stringify(result), /PRIVATE/u);
});

test("listing cannot follow a foreign host, another folder, credentials or a cursor loop", async () => {
  for (const nextLink of ["https://attacker.invalid/v1.0/items", "https://graph.microsoft.com/v1.0/me",
    `https://user@graph.microsoft.com/v1.0/drives/${root.driveId}/items/${root.id}/children`,
    `https://graph.microsoft.com/v1.0/drives/${root.driveId}/items/${root.id}/children?loop=1`]) {
    const f = fixture({ nextLink });
    const result = await runProbe(config, f.options);
    assert.equal(result.failureCode, "invalid_next_link");
    assert.ok(f.calls.every((call) => call.url.origin === "https://graph.microsoft.com"));
  }
});

test("listings reject duplicate IDs, case aliases and incomplete bounded scans", async () => {
  const cases = [
    [{ value: [{ id: "id", name: "a" }, { id: "id", name: "b" }] }, {}],
    [{ value: [{ id: "a", name: "file" }, { id: "b", name: "FILE" }] }, {}],
    [{ value: [{ id: "a", name: "a" }, { id: "b", name: "b" }] }, { maxItems: 1 }],
    [{ value: [], "@odata.nextLink": `https://graph.microsoft.com/v1.0/drives/${root.driveId}/items/${root.id}/children?page=2` }, { maxPages: 1 }],
  ];
  for (const [page, options] of cases) {
    const client = new GraphClient("fixture", { ...options, fetchImpl: async () => response(page) });
    await assert.rejects(client.list(root));
  }
});

test("shared roots must be folders with canonical remote identities", () => {
  assert.deepEqual(folderIdentity({ id: "shortcut", remoteItem: { id: "real", folder: {},
    parentReference: { driveId: "drive" } } }), { driveId: "drive", id: "real" });
  for (const item of [{ id: "file", file: {} }, { id: "folder", folder: {} },
    { id: "folder", folder: {}, remoteItem: { id: "file", file: {} } }]) {
    assert.throws(() => folderIdentity(item));
  }
  for (const url of ["http://fixture.sharepoint.com/folder", "https://attacker.invalid/link",
    "https://user@fixture.sharepoint.com/folder", "https://fixture.sharepoint.com:444/folder"])
    assert.throws(() => sharingUrl(url));
});

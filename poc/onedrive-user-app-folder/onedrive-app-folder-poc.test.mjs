import assert from "node:assert/strict";
import { chmod, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  OneDriveAppFolderClient,
  POC_STATUS,
  readCredentialFile,
  runAppFolderProbe,
  validateCredential,
} from "./onedrive-app-folder-poc.mjs";

const TOKEN_A = "onedrive-client-a-token-for-tests";
const TOKEN_B = "onedrive-client-b-token-for-tests";
const APP_FOLDER_SCOPES =
  "openid profile offline_access https://graph.microsoft.com/Files.ReadWrite.AppFolder";
const FRAGMENT_BYTES = 320 * 1024;

function credential() {
  return validateCredential({
    client_a: {
      access_token: TOKEN_A,
      granted_scopes: APP_FOLDER_SCOPES,
    },
    client_b: {
      access_token: TOKEN_B,
      granted_scopes: APP_FOLDER_SCOPES,
    },
  });
}

function jsonResponse(value, status = 200, headers = {}) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

function graphError(code, status) {
  return jsonResponse({ error: { code } }, status);
}

function createFakeGraph({
  acceptStaleFinalModes = [],
  forceStaleFinalStatus = {},
  rootsDiffer = false,
  denyClientB = false,
  allowOutsideAppFolderActors = [],
  cancelFailureStatus,
  loseFirstCreateCompletionResponse = false,
  unrelatedAppRootFile = false,
  mutateKnownContentOnCleanup = false,
} = {}) {
  const items = new Map();
  const sessions = new Map();
  const requests = [];
  let itemSequence = 0;
  let sessionSequence = 0;
  let revision = 0;
  let cleanupMutationApplied = false;
  let staleRejectionCount = 0;
  let completionResponseLost = false;

  function nextEtag() {
    revision += 1;
    return `\"etag-${revision}\"`;
  }

  function actor(headers) {
    const authorization = headers.get("authorization") ?? "";
    if (authorization === `Bearer ${TOKEN_A}`) return "client_a";
    if (authorization === `Bearer ${TOKEN_B}`) return "client_b";
    return undefined;
  }

  function itemJson(item) {
    const common = {
      id: item.id,
      name: item.name,
      eTag: item.etag,
      size: item.kind === "file" ? item.bytes.length : 0,
    };
    return item.kind === "file"
      ? { ...common, file: { mimeType: "application/octet-stream" } }
      : { ...common, folder: { childCount: childItems(item.id).length } };
  }

  function childItems(parentId) {
    return [...items.values()].filter((item) => item.parentId === parentId);
  }

  function createItem({ kind, name, parentId, bytes = Buffer.alloc(0) }) {
    itemSequence += 1;
    const item = {
      id: `item-${itemSequence}`,
      kind,
      name,
      parentId,
      bytes: Buffer.from(bytes),
      etag: nextEtag(),
      lastWriteMode: "create",
    };
    items.set(item.id, item);
    return item;
  }

  function commitSession(session, bytes) {
    if (session.kind === "create") {
      const collision = childItems(session.parentId).find(
        (item) => item.name.toLowerCase() === session.name.toLowerCase(),
      );
      if (collision) return { error: graphError("nameAlreadyExists", 409) };
      const item = createItem({
        kind: "file",
        name: session.name,
        parentId: session.parentId,
        bytes,
      });
      item.lastWriteMode = "upload_session";
      return { item, status: 201 };
    }

    const item = items.get(session.itemId);
    if (!item) return { error: graphError("itemNotFound", 404) };
    const stale = item.etag !== session.baseEtag;
    if (stale && forceStaleFinalStatus[item.lastWriteMode] !== undefined) {
      return {
        error: graphError(
          "serviceNotAvailable",
          forceStaleFinalStatus[item.lastWriteMode],
        ),
      };
    }
    if (stale && !acceptStaleFinalModes.includes(item.lastWriteMode)) {
      staleRejectionCount += 1;
      return { error: graphError("resourceModified", 412) };
    }
    item.bytes = Buffer.from(bytes);
    item.etag = nextEtag();
    item.lastWriteMode = stale ? "stale_session" : "upload_session";
    return { item, status: 200 };
  }

  function parseContentRange(value) {
    const match = /^bytes (\d+)-(\d+)\/(\d+)$/u.exec(value ?? "");
    if (!match) return undefined;
    return {
      start: Number(match[1]),
      end: Number(match[2]),
      total: Number(match[3]),
    };
  }

  if (unrelatedAppRootFile) {
    createItem({
      kind: "file",
      name: "unrelated-user-content.bin",
      parentId: "app-root",
      bytes: Buffer.from("unrelated", "utf8"),
    });
  }

  async function fetchImpl(url, options = {}) {
    const parsedUrl = new URL(url);
    const headers = new Headers(options.headers);
    const method = options.method ?? "GET";
    const isGraph = parsedUrl.origin === "https://graph.microsoft.com";
    const requestActor = actor(headers);

    requests.push({
      origin: parsedUrl.origin,
      path: parsedUrl.pathname,
      method,
      actor: requestActor,
      authorization: headers.get("authorization"),
      contentRange: headers.get("content-range"),
    });

    if (parsedUrl.hostname === "upload.mock.1drv.com") {
      if (headers.has("authorization")) {
        return graphError("authorizationNotAllowed", 401);
      }
      const session = sessions.get(parsedUrl.pathname);
      if (!session) return graphError("itemNotFound", 404);
      if (method === "DELETE") {
        if (cancelFailureStatus !== undefined) {
          return graphError("cannotCancelUploadSession", cancelFailureStatus);
        }
        sessions.delete(parsedUrl.pathname);
        return new Response(null, { status: 204 });
      }
      if (method === "GET") {
        return jsonResponse({
          expirationDateTime: "2099-01-01T00:00:00Z",
          nextExpectedRanges: [`${session.received.length}-`],
        });
      }
      if (method !== "PUT") return graphError("invalidRequest", 400);
      const range = parseContentRange(headers.get("content-range"));
      const bytes = Buffer.from(options.body ?? []);
      if (
        !range ||
        bytes.length !== range.end - range.start + 1 ||
        range.start !== session.received.length ||
        (session.total !== undefined && session.total !== range.total)
      ) {
        return graphError("invalidRange", 416);
      }
      session.total = range.total;
      session.received = Buffer.concat([session.received, bytes]);
      if (session.received.length < range.total) {
        return jsonResponse(
          {
            expirationDateTime: "2099-01-01T00:00:00Z",
            nextExpectedRanges: [`${session.received.length}-`],
          },
          202,
        );
      }
      const committed = commitSession(session, session.received);
      if (committed.error) return committed.error;
      sessions.delete(parsedUrl.pathname);
      if (
        loseFirstCreateCompletionResponse &&
        !completionResponseLost &&
        session.kind === "create"
      ) {
        completionResponseLost = true;
        throw new Error("simulated_lost_completion_response");
      }
      return jsonResponse(itemJson(committed.item), committed.status);
    }

    if (parsedUrl.hostname === "download.mock.1drv.com") {
      if (headers.has("authorization")) {
        return graphError("authorizationNotAllowed", 401);
      }
      const itemId = parsedUrl.pathname.slice(1);
      const item = items.get(itemId);
      if (!item || item.kind !== "file") return graphError("itemNotFound", 404);
      return new Response(item.bytes, {
        status: 200,
        headers: {
          "Content-Type": "application/octet-stream",
          "Content-Length": String(item.bytes.length),
        },
      });
    }

    if (!isGraph || !requestActor) return graphError("invalidAuthenticationToken", 401);
    if (denyClientB && requestActor === "client_b") {
      return graphError("accessDenied", 403);
    }

    const path = parsedUrl.pathname.replace(/^\/v1\.0/u, "");
    if (method === "GET" && path === "/me/drive/root") {
      if (allowOutsideAppFolderActors.includes(requestActor)) {
        return jsonResponse({ id: "root-item" });
      }
      return graphError("accessDenied", 403);
    }
    if (method === "GET" && path === "/me/drive/special/approot") {
      const different = rootsDiffer && requestActor === "client_b";
      return jsonResponse({
        id: different ? "app-root-other" : "app-root",
        name: "Mallard Test",
        eTag: "\"app-root-etag\"",
        folder: { childCount: 0 },
        specialFolder: { name: "approot" },
        parentReference: {
          driveId: different ? "drive-other" : "drive-main",
          driveType: "personal",
        },
      });
    }

    const createFolderMatch =
      /^\/drives\/([^/]+)\/items\/([^/]+)\/children$/u.exec(path);
    if (method === "POST" && createFolderMatch) {
      const body = JSON.parse(String(options.body));
      const collision = childItems(decodeURIComponent(createFolderMatch[2])).find(
        (item) => item.name.toLowerCase() === body.name.toLowerCase(),
      );
      if (collision) return graphError("nameAlreadyExists", 409);
      return jsonResponse(
        itemJson(
          createItem({
            kind: "folder",
            name: body.name,
            parentId: decodeURIComponent(createFolderMatch[2]),
          }),
        ),
        201,
      );
    }

    const createFileSessionMatch =
      /^\/drives\/([^/]+)\/items\/([^/]+):\/([^/]+):\/createUploadSession$/u.exec(
        path,
      );
    if (method === "POST" && createFileSessionMatch) {
      const body = JSON.parse(String(options.body));
      if (body.item?.["@microsoft.graph.conflictBehavior"] !== "fail") {
        return graphError("invalidRequest", 400);
      }
      sessionSequence += 1;
      const sessionPath = `/session-${sessionSequence}`;
      sessions.set(sessionPath, {
        kind: "create",
        parentId: decodeURIComponent(createFileSessionMatch[2]),
        name: decodeURIComponent(createFileSessionMatch[3]),
        received: Buffer.alloc(0),
      });
      return jsonResponse({
        uploadUrl: `https://upload.mock.1drv.com${sessionPath}?opaque=secret`,
        expirationDateTime: "2099-01-01T00:00:00Z",
      });
    }

    const updateSessionMatch =
      /^\/drives\/([^/]+)\/items\/([^/]+)\/createUploadSession$/u.exec(path);
    if (method === "POST" && updateSessionMatch) {
      const itemId = decodeURIComponent(updateSessionMatch[2]);
      const item = items.get(itemId);
      if (!item) return graphError("itemNotFound", 404);
      if (headers.get("if-match") !== item.etag) {
        return graphError("resourceModified", 412);
      }
      const body = JSON.parse(String(options.body));
      if (body.item?.["@microsoft.graph.conflictBehavior"] !== "replace") {
        return graphError("invalidRequest", 400);
      }
      sessionSequence += 1;
      const sessionPath = `/session-${sessionSequence}`;
      sessions.set(sessionPath, {
        kind: "update",
        itemId,
        baseEtag: item.etag,
        received: Buffer.alloc(0),
      });
      return jsonResponse({
        uploadUrl: `https://upload.mock.1drv.com${sessionPath}?opaque=secret`,
        expirationDateTime: "2099-01-01T00:00:00Z",
      });
    }

    const contentMatch = /^\/drives\/([^/]+)\/items\/([^/]+)\/content$/u.exec(
      path,
    );
    if (contentMatch && method === "PUT") {
      const item = items.get(decodeURIComponent(contentMatch[2]));
      if (!item || item.kind !== "file") return graphError("itemNotFound", 404);
      item.bytes = Buffer.from(options.body ?? []);
      item.etag = nextEtag();
      item.lastWriteMode = "simple_upload";
      return jsonResponse(itemJson(item));
    }
    if (contentMatch && method === "GET") {
      const itemId = decodeURIComponent(contentMatch[2]);
      if (!items.has(itemId)) return graphError("itemNotFound", 404);
      return new Response(null, {
        status: 302,
        headers: {
          Location: `https://download.mock.1drv.com/${itemId}?opaque=secret`,
        },
      });
    }

    const childPathMatch =
      /^\/drives\/([^/]+)\/items\/([^/:]+):\/([^/]+)$/u.exec(path);
    if (method === "GET" && childPathMatch) {
      const parentId = decodeURIComponent(childPathMatch[2]);
      const name = decodeURIComponent(childPathMatch[3]);
      const item = childItems(parentId).find(
        (candidate) => candidate.name.toLowerCase() === name.toLowerCase(),
      );
      if (!item) return graphError("itemNotFound", 404);
      if (
        mutateKnownContentOnCleanup &&
        !cleanupMutationApplied &&
        staleRejectionCount === 2 &&
        item.name.startsWith("mallard-onedrive-poc-")
      ) {
        item.bytes = Buffer.from("external-content", "utf8");
        item.etag = nextEtag();
        cleanupMutationApplied = true;
      }
      return jsonResponse(itemJson(item));
    }

    const itemMatch = /^\/drives\/([^/]+)\/items\/([^/]+)$/u.exec(path);
    if (itemMatch && method === "GET") {
      const item = items.get(decodeURIComponent(itemMatch[2]));
      return item ? jsonResponse(itemJson(item)) : graphError("itemNotFound", 404);
    }
    if (itemMatch && method === "DELETE") {
      const itemId = decodeURIComponent(itemMatch[2]);
      const item = items.get(itemId);
      if (!item) return graphError("itemNotFound", 404);
      if (headers.get("if-match") !== item.etag) {
        return graphError("resourceModified", 412);
      }
      if (item.kind === "folder" && childItems(item.id).length !== 0) {
        return graphError("folderNotEmpty", 409);
      }
      items.delete(itemId);
      return new Response(null, { status: 204 });
    }

    return graphError("notSupported", 400);
  }

  return { fetchImpl, items, requests, sessions };
}

async function makeCredentialFixture(value) {
  const createdRoot = await mkdtemp(join(tmpdir(), "mallard-onedrive-poc-test-"));
  const root = await realpath(createdRoot);
  const file = join(root, "credential.json");
  await writeFile(file, JSON.stringify(value), { mode: 0o600 });
  await chmod(file, 0o600);
  return { root, file };
}

async function removeFixture(root) {
  await rm(root, { recursive: true, force: false });
}

test("marks the harness as a non-production live gate", () => {
  assert.deepEqual(POC_STATUS, {
    productionReady: false,
    persistedSchemaChanged: false,
    verdict: "REQUIRES_LIVE_PROBE",
  });
});

test("requires App Folder scope without another file or site scope", () => {
  assert.throws(
    () =>
      validateCredential({
        client_a: {
          access_token: TOKEN_A,
          granted_scopes: "openid Files.ReadWrite",
        },
        client_b: {
          access_token: TOKEN_B,
          granted_scopes: APP_FOLDER_SCOPES,
        },
      }),
    (error) => error.code === "client_a_app_folder_scope_missing",
  );
  assert.throws(
    () =>
      validateCredential({
        client_a: {
          access_token: TOKEN_A,
          granted_scopes: `${APP_FOLDER_SCOPES} Files.ReadWrite.All`,
        },
        client_b: {
          access_token: TOKEN_B,
          granted_scopes: APP_FOLDER_SCOPES,
        },
      }),
    (error) => error.code === "client_a_broader_file_scope_present",
  );
});

test("requires two distinct access tokens", () => {
  assert.throws(
    () =>
      validateCredential({
        client_a: {
          access_token: TOKEN_A,
          granted_scopes: APP_FOLDER_SCOPES,
        },
        client_b: {
          access_token: TOKEN_A,
          granted_scopes: APP_FOLDER_SCOPES,
        },
      }),
    (error) => error.code === "client_authorizations_not_distinct",
  );
});

test("reads only a canonical private credential file", async () => {
  const fixture = await makeCredentialFixture({
    client_a: {
      access_token: TOKEN_A,
      granted_scopes: APP_FOLDER_SCOPES,
    },
    client_b: {
      access_token: TOKEN_B,
      granted_scopes: APP_FOLDER_SCOPES,
    },
  });
  try {
    const value = await readCredentialFile(fixture.file);
    assert.equal(value.clientA.accessToken, TOKEN_A);
    if (process.platform !== "win32") {
      await chmod(fixture.file, 0o644);
      await assert.rejects(
        readCredentialFile(fixture.file),
        (error) => error.code === "credential_file_permissions_too_broad",
      );
    }
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects a symbolic-link credential path", async () => {
  const fixture = await makeCredentialFixture({
    client_a: {
      access_token: TOKEN_A,
      granted_scopes: APP_FOLDER_SCOPES,
    },
    client_b: {
      access_token: TOKEN_B,
      granted_scopes: APP_FOLDER_SCOPES,
    },
  });
  const link = join(fixture.root, "credential-link.json");
  try {
    await symlink(fixture.file, link);
    await assert.rejects(
      readCredentialFile(link),
      (error) => error.code === "credential_file_must_be_regular_canonical_file",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("passes both cross-mode stale final-commit races and cleans up", async () => {
  const fake = createFakeGraph();
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4a),
  });

  assert.deepEqual(result, {
    verdict: "PASS",
    driveType: "personal",
    permissionProbe: "reported_scope_and_outside_root_denial",
    sameAccountNamespace: true,
    distinctAccessTokens: true,
    finalCommitCas: {
      upload_session: "stale_final_rejected",
      simple_upload: "stale_final_rejected",
    },
    cleanup: { status: "complete" },
    stages: [
      "app_folder_access_and_scope_isolation",
      "same_account_namespace_two_tokens",
      "stale_final_rejected_after_session_replace",
      "stale_final_rejected_after_simple_replace",
    ],
  });
  assert.equal(fake.items.size, 0);
  assert.equal(fake.sessions.size, 0);

  const uploadRequests = fake.requests.filter(
    (request) => request.origin === "https://upload.mock.1drv.com",
  );
  assert.ok(uploadRequests.length >= 8);
  assert.ok(uploadRequests.every((request) => request.authorization === null));
  assert.ok(
    uploadRequests.some(
      (request) =>
        request.contentRange === `bytes 0-${FRAGMENT_BYTES - 1}/${2 * FRAGMENT_BYTES}`,
    ),
  );
  assert.ok(
    uploadRequests.some(
      (request) =>
        request.contentRange ===
        `bytes ${FRAGMENT_BYTES}-${2 * FRAGMENT_BYTES - 1}/${2 * FRAGMENT_BYTES}`,
    ),
  );
  const downloadRequests = fake.requests.filter(
    (request) => request.origin === "https://download.mock.1drv.com",
  );
  assert.ok(downloadRequests.length >= 6);
  assert.ok(downloadRequests.every((request) => request.authorization === null));
});

test("fails when a stale session overwrites a session winner", async () => {
  const fake = createFakeGraph({
    acceptStaleFinalModes: ["upload_session"],
  });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4b),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "stale_final_range_was_accepted");
  assert.deepEqual(result.cleanup, { status: "complete" });
});

test("fails when a stale session overwrites a simple-upload winner", async () => {
  const fake = createFakeGraph({
    acceptStaleFinalModes: ["simple_upload"],
  });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4c),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "stale_final_range_was_accepted");
  assert.deepEqual(result.cleanup, { status: "complete" });
});

test("does not treat an unrelated final-range error as CAS proof", async () => {
  const fake = createFakeGraph({
    forceStaleFinalStatus: { upload_session: 503 },
  });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x52),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "graph_service_failure");
  assert.deepEqual(result.cleanup, { status: "complete" });
});

test("cleans a committed file after its completion response is lost", async () => {
  const fake = createFakeGraph({ loseFirstCreateCompletionResponse: true });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x56),
  });
  assert.deepEqual(result, {
    verdict: "FAIL",
    failureCode: "onedrive_transport_ambiguous",
    cleanup: { status: "complete" },
    stages: [
      "app_folder_access_and_scope_isolation",
      "same_account_namespace_two_tokens",
    ],
  });
  assert.equal(fake.items.size, 0);
  assert.equal(fake.sessions.size, 0);
});

test("fails if the two authorizations resolve different App Folders", async () => {
  const fake = createFakeGraph({ rootsDiffer: true });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4d),
  });
  assert.deepEqual(result, {
    verdict: "FAIL",
    failureCode: "client_app_folders_do_not_match",
    cleanup: { status: "not_needed" },
    stages: [],
  });
});

test("reports permission denial without provider response data", async () => {
  const fake = createFakeGraph({ denyClientB: true });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4e),
  });
  assert.deepEqual(result, {
    verdict: "FAIL",
    failureCode: "graph_forbidden",
    cleanup: { status: "not_needed" },
    stages: [],
  });
  const serialized = JSON.stringify(result);
  assert.equal(serialized.includes(TOKEN_A), false);
  assert.equal(serialized.includes(TOKEN_B), false);
  assert.equal(serialized.includes("accessDenied"), false);
});

test("fails if a declared App Folder token can read outside App Folder", async () => {
  const fake = createFakeGraph({
    allowOutsideAppFolderActors: ["client_a"],
  });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x54),
  });
  assert.deepEqual(result, {
    verdict: "FAIL",
    failureCode: "outside_app_folder_access_was_allowed",
    cleanup: { status: "not_needed" },
    stages: [],
  });
});

test("reports an upload session that cannot be canceled", async () => {
  const fake = createFakeGraph({ cancelFailureStatus: 409 });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x55),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "stale_upload_session_cleanup_failed");
  assert.deepEqual(result.cleanup, {
    status: "required",
    reason: "upload_session_cleanup_unresolved",
  });
  assert.equal(result.probeId, "55".repeat(12));
  assert.ok(fake.sessions.size > 0);
});

test("does not inspect or delete unrelated App Folder content", async () => {
  const fake = createFakeGraph({ unrelatedAppRootFile: true });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x4f),
  });
  assert.equal(result.verdict, "PASS");
  assert.deepEqual(result.cleanup, { status: "complete" });
  assert.equal(fake.items.size, 1);
  assert.equal([...fake.items.values()][0].name, "unrelated-user-content.bin");
});

test("does not delete a known file with unexpected content", async () => {
  const fake = createFakeGraph({ mutateKnownContentOnCleanup: true });
  const result = await runAppFolderProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x53),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.cleanup.status, "required");
  assert.equal(result.cleanup.reason, "unexpected_probe_file_content");
  assert.equal(result.probeId, "53".repeat(12));
});

test("client rejects an authorization header on upload URLs by construction", async () => {
  const fake = createFakeGraph();
  const actor = credential().clientA;
  const client = new OneDriveAppFolderClient({
    actor,
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
  });
  const root = await client.appRoot();
  const session = await client.createFileSession(
    root.driveId,
    root.id,
    `mallard-onedrive-poc-${"51".repeat(12)}-head.bin`,
  );
  await client.uploadProgress(
    session.uploadUrl,
    Buffer.alloc(FRAGMENT_BYTES, 0x61),
    0,
    2 * FRAGMENT_BYTES,
  );
  const request = fake.requests.at(-1);
  assert.equal(request.origin, "https://upload.mock.1drv.com");
  assert.equal(request.authorization, null);
});

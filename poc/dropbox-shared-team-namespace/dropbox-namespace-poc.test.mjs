import assert from "node:assert/strict";
import { chmod, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  DropboxNamespaceClient,
  POC_STATUS,
  dropboxContentHash,
  readCredentialFile,
  runNamespaceProbe,
  validateCredential,
} from "./dropbox-namespace-poc.mjs";

const PRIMARY_TOKEN = "dropbox-primary-token-for-tests";
const SECONDARY_TOKEN = "dropbox-secondary-token-for-tests";
const TEAM_NAMESPACE = "ns-team-test";
const SHARED_NAMESPACE = "ns-shared-test";
const PROBE_ROOT = "/Mallard Dropbox POC";

function credential(namespaceId = TEAM_NAMESPACE) {
  return validateCredential({
    namespace_id: namespaceId,
    probe_root: PROBE_ROOT,
    primary: { access_token: PRIMARY_TOKEN },
    secondary: { access_token: SECONDARY_TOKEN },
  });
}

function jsonResponse(value, status = 200, headers = {}) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

function dropboxError(summary, status = 409) {
  return jsonResponse(
    {
      error_summary: summary,
      error: { ".tag": "path", path: { ".tag": summary.split("/")[1] } },
    },
    status,
  );
}

function uploadConflict(conflictTag, structured = true) {
  if (!structured) return dropboxError(`path/conflict/${conflictTag}/`);
  return jsonResponse(
    {
      error_summary: `path/conflict/${conflictTag}/`,
      error: {
        ".tag": "path",
        reason: {
          ".tag": "conflict",
          conflict: { ".tag": conflictTag },
        },
        upload_session_id: "test-upload-session",
      },
    },
    409,
  );
}

function createFakeDropbox({
  namespaceId = TEAM_NAMESPACE,
  namespaceMode = "team_space",
  acceptStaleUpdates = false,
  denySecondaryFiles = false,
  forbiddenBody = undefined,
  tamperKnownPathBeforeCleanup = false,
  unstructuredUploadConflicts = false,
} = {}) {
  const objects = new Map();
  const cursors = new Map();
  const requests = [];
  let revision = 0;
  let cursorId = 0;
  let tampered = false;

  objects.set(PROBE_ROOT.toLowerCase(), { kind: "folder" });

  const actors = new Map([
    [
      PRIMARY_TOKEN,
      {
        name: "primary",
        accountId: "dbid:primary-test-account",
        rootNamespaceId:
          namespaceMode === "team_space" ? namespaceId : "root-primary-test",
        homeNamespaceId: "home-primary-test",
        rootKind: namespaceMode === "team_space" ? "team" : "user",
      },
    ],
    [
      SECONDARY_TOKEN,
      {
        name: "secondary",
        accountId: "dbid:secondary-test-account",
        rootNamespaceId:
          namespaceMode === "team_space" ? namespaceId : "root-secondary-test",
        homeNamespaceId: "home-secondary-test",
        rootKind: namespaceMode === "team_space" ? "team" : "user",
      },
    ],
  ]);

  function nextRevision() {
    revision += 1;
    return revision.toString(16).padStart(9, "0");
  }

  function fileMetadata(path, object) {
    return {
      ".tag": "file",
      name: path.slice(path.lastIndexOf("/") + 1),
      path_lower: path,
      path_display: path,
      id: `id:${object.rev}`,
      client_modified: "2026-09-02T00:00:00Z",
      server_modified: "2026-09-02T00:00:00Z",
      rev: object.rev,
      size: object.bytes.length,
      content_hash: dropboxContentHash(object.bytes),
      is_downloadable: true,
    };
  }

  function folderMetadata(path) {
    return {
      ".tag": "folder",
      name: path.slice(path.lastIndexOf("/") + 1),
      path_lower: path,
      path_display: path,
      id: `id:folder:${path}`,
    };
  }

  function metadata(path, object) {
    return object.kind === "file"
      ? fileMetadata(path, object)
      : folderMetadata(path);
  }

  function actorFrom(headers) {
    const authorization = headers.get("authorization") ?? "";
    const token = authorization.startsWith("Bearer ")
      ? authorization.slice("Bearer ".length)
      : "";
    return actors.get(token);
  }

  function parseJsonBody(options) {
    return JSON.parse(String(options.body));
  }

  function verifyPathRoot(headers) {
    const raw = headers.get("dropbox-api-path-root");
    if (!raw) return false;
    const value = JSON.parse(raw);
    return (
      value[".tag"] === "namespace_id" && value.namespace_id === namespaceId
    );
  }

  function listEntries(folder, recursive) {
    const prefix = folder === "" ? "/" : `${folder}/`;
    return [...objects.entries()]
      .filter(([path]) => {
        if (!path.startsWith(prefix)) return false;
        const remainder = path.slice(prefix.length);
        return remainder.length > 0 && (recursive || !remainder.includes("/"));
      })
      .map(([path, object]) => metadata(path, object))
      .sort((left, right) => right.path_lower.localeCompare(left.path_lower));
  }

  function listPage(entries, start, limit) {
    const page = entries.slice(start, start + limit);
    const next = start + page.length;
    const hasMore = next < entries.length;
    const cursor = `cursor-${(cursorId += 1)}`;
    if (hasMore) cursors.set(cursor, { entries, start: next, limit });
    return { entries: page, cursor, has_more: hasMore };
  }

  async function fetchImpl(url, options) {
    const headers = new Headers(options.headers);
    const actor = actorFrom(headers);
    const endpoint = new URL(url).pathname.replace(/^\/2\//u, "");
    if (!actor) return dropboxError("invalid_access_token/", 401);

    requests.push({
      actor: actor.name,
      endpoint,
      pathRoot: headers.get("dropbox-api-path-root"),
      apiArgument: headers.get("dropbox-api-arg"),
      body: options.body,
    });

    if (endpoint === "users/get_current_account") {
      if (headers.has("dropbox-api-path-root")) {
        return dropboxError("unexpected_path_root/", 400);
      }
      return jsonResponse({
        account_id: actor.accountId,
        name: { display_name: actor.name },
        root_info: {
          ".tag": actor.rootKind,
          root_namespace_id: actor.rootNamespaceId,
          home_namespace_id: actor.homeNamespaceId,
          ...(actor.rootKind === "team" ? { home_path: `/${actor.name}` } : {}),
        },
      });
    }

    if (!verifyPathRoot(headers)) {
      return dropboxError("path_root/no_permission/", 409);
    }
    if (actor.name === "secondary" && denySecondaryFiles) {
      return jsonResponse(
        forbiddenBody ?? { error_summary: "no_permission/" },
        403,
      );
    }

    if (endpoint === "files/get_metadata") {
      const value = parseJsonBody(options);
      const path = value.path.toLowerCase();
      const object = objects.get(path);
      if (!object) return dropboxError("path/not_found/");
      return jsonResponse(metadata(path, object));
    }

    if (endpoint === "files/upload") {
      const argument = JSON.parse(headers.get("dropbox-api-arg"));
      const path = argument.path.toLowerCase();
      const parent = path.slice(0, path.lastIndexOf("/"));
      const existing = objects.get(path);
      if (objects.get(parent)?.kind !== "folder") {
        return uploadConflict(
          "file_ancestor",
          !unstructuredUploadConflicts,
        );
      }
      const bytes = Buffer.from(options.body ?? []);
      if (argument.content_hash !== dropboxContentHash(bytes)) {
        return dropboxError("content_hash_mismatch/", 400);
      }
      const mode =
        typeof argument.mode === "string"
          ? argument.mode
          : argument.mode?.[".tag"];
      if (mode === "add" && existing) {
        return uploadConflict("file", !unstructuredUploadConflicts);
      }
      if (mode === "update") {
        const matches =
          existing?.kind === "file" &&
          existing.rev === argument.mode.update;
        if (!matches && !acceptStaleUpdates) {
          return uploadConflict("file", !unstructuredUploadConflicts);
        }
      }
      if (!argument.strict_conflict || argument.autorename !== false) {
        return dropboxError("unsafe_commit_options/", 400);
      }
      const object = { kind: "file", bytes, rev: nextRevision() };
      objects.set(path, object);
      return jsonResponse(fileMetadata(path, object));
    }

    if (endpoint === "files/download") {
      const argument = JSON.parse(headers.get("dropbox-api-arg"));
      const path = argument.path.toLowerCase();
      const object = objects.get(path);
      if (!object || object.kind !== "file") {
        return dropboxError("path/not_found/");
      }
      return new Response(object.bytes, {
        status: 200,
        headers: {
          "Content-Type": "application/octet-stream",
          "Dropbox-API-Result": JSON.stringify(fileMetadata(path, object)),
        },
      });
    }

    if (endpoint === "files/list_folder") {
      const value = parseJsonBody(options);
      const folder = value.path.toLowerCase();
      if (folder !== "" && objects.get(folder)?.kind !== "folder") {
        return dropboxError("path/not_found/");
      }
      const entries = listEntries(folder, value.recursive === true);
      const response = jsonResponse(
        listPage(entries, 0, value.limit ?? (entries.length || 1)),
      );
      const generatedFiles = [...objects.entries()].filter(
        ([path, object]) =>
          object.kind === "file" &&
          path.includes("/mallard-dropbox-poc-"),
      );
      if (
        tamperKnownPathBeforeCleanup &&
        !tampered &&
        actor.name === "primary" &&
        generatedFiles.length >= 10
      ) {
        const [path] = generatedFiles.find(([candidate]) =>
          candidate.endsWith("-head.bin"),
        );
        objects.set(path, {
          kind: "file",
          bytes: Buffer.from("mallard-dropbox-poc-v4", "utf8"),
          rev: nextRevision(),
        });
        tampered = true;
      }
      return response;
    }

    if (endpoint === "files/list_folder/continue") {
      const value = parseJsonBody(options);
      const cursor = cursors.get(value.cursor);
      if (!cursor) return dropboxError("reset/", 409);
      cursors.delete(value.cursor);
      return jsonResponse(
        listPage(cursor.entries, cursor.start, cursor.limit),
      );
    }

    if (endpoint === "files/delete_v2") {
      const value = parseJsonBody(options);
      const path = value.path.toLowerCase();
      const object = objects.get(path);
      if (!object) return dropboxError("path/not_found/");
      if (object.kind === "file") {
        if (typeof value.parent_rev !== "string" || value.parent_rev !== object.rev) {
          return dropboxError("path/parent_rev_mismatch/");
        }
      } else {
        const prefix = `${path}/`;
        for (const candidate of [...objects.keys()]) {
          if (candidate.startsWith(prefix)) objects.delete(candidate);
        }
      }
      objects.delete(path);
      return jsonResponse({ metadata: metadata(path, object) });
    }

    return dropboxError("unsupported_route/", 400);
  }

  return { fetchImpl, objects, requests };
}

async function makeCredentialFixture(value) {
  const createdRoot = await mkdtemp(join(tmpdir(), "mallard-dropbox-poc-test-"));
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

test("implements Dropbox content hashing without replacing standard SHA-256", () => {
  assert.equal(
    dropboxContentHash(Buffer.alloc(0)),
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  );
  assert.equal(
    dropboxContentHash(Buffer.from("hello", "utf8")),
    "9595c9df90075148eb06860365df33584b75bff782a510c6cd4883a419833d50",
  );
});

test("rejects one authorization reused as both actors", () => {
  assert.throws(
    () =>
      validateCredential({
        namespace_id: TEAM_NAMESPACE,
        probe_root: PROBE_ROOT,
        primary: { access_token: PRIMARY_TOKEN },
        secondary: { access_token: PRIMARY_TOKEN },
      }),
    (error) => error.code === "actors_not_distinct",
  );
});

test("allows one team token with two distinct member selectors", () => {
  const value = validateCredential({
    namespace_id: TEAM_NAMESPACE,
    probe_root: PROBE_ROOT,
    primary: {
      access_token: PRIMARY_TOKEN,
      select_user: "dbmid:primary-member",
    },
    secondary: {
      access_token: PRIMARY_TOKEN,
      select_user: "dbmid:secondary-member",
    },
  });
  assert.equal(value.primary.selectUser, "dbmid:primary-member");
  assert.equal(value.secondary.selectUser, "dbmid:secondary-member");
});

test("sends a team member selector without path-rooting the account call", async () => {
  const value = validateCredential({
    namespace_id: TEAM_NAMESPACE,
    probe_root: PROBE_ROOT,
    primary: {
      access_token: PRIMARY_TOKEN,
      select_user: "dbmid:primary-member",
    },
    secondary: {
      access_token: PRIMARY_TOKEN,
      select_user: "dbmid:secondary-member",
    },
  });
  const seen = [];
  const client = new DropboxNamespaceClient({
    namespaceId: value.namespaceId,
    probeRoot: value.probeRoot,
    actor: value.primary,
    fetchImpl: async (url, options) => {
      const headers = new Headers(options.headers);
      seen.push({ url, headers });
      if (url.endsWith("/users/get_current_account")) {
        return jsonResponse({
          account_id: "dbid:primary-test-account",
          root_info: {
            ".tag": "team",
            root_namespace_id: TEAM_NAMESPACE,
            home_namespace_id: "home-primary-test",
            home_path: "/Primary",
          },
        });
      }
      return jsonResponse({ entries: [], cursor: "cursor", has_more: false });
    },
    sleepImpl: async () => {},
  });
  await client.currentAccount();
  await client.probeNamespaceAccess();

  assert.equal(seen.length, 2);
  assert.equal(
    seen[0].headers.get("dropbox-api-select-user"),
    "dbmid:primary-member",
  );
  assert.equal(seen[0].headers.get("dropbox-api-path-root"), null);
  assert.equal(
    seen[1].headers.get("dropbox-api-select-user"),
    "dbmid:primary-member",
  );
  assert.deepEqual(JSON.parse(seen[1].headers.get("dropbox-api-path-root")), {
    ".tag": "namespace_id",
    namespace_id: TEAM_NAMESPACE,
  });
});

test("honors Dropbox Retry-After without waiting in the test", async () => {
  const waits = [];
  let calls = 0;
  const value = credential();
  const client = new DropboxNamespaceClient({
    namespaceId: value.namespaceId,
    probeRoot: value.probeRoot,
    actor: value.primary,
    fetchImpl: async () => {
      calls += 1;
      if (calls === 1) {
        return jsonResponse(
          { error_summary: "too_many_requests/" },
          429,
          { "Retry-After": "60" },
        );
      }
      return jsonResponse({ entries: [], cursor: "cursor", has_more: false });
    },
    sleepImpl: async (milliseconds) => waits.push(milliseconds),
  });

  await client.probeNamespaceAccess();

  assert.equal(calls, 2);
  assert.deepEqual(waits, [60_000]);
});

test("fails closed when Retry-After exceeds the live probe limit", async () => {
  const waits = [];
  const value = credential();
  const client = new DropboxNamespaceClient({
    namespaceId: value.namespaceId,
    probeRoot: value.probeRoot,
    actor: value.primary,
    fetchImpl: async () =>
      jsonResponse(
        { error_summary: "too_many_requests/" },
        429,
        { "Retry-After": "61" },
      ),
    sleepImpl: async (milliseconds) => waits.push(milliseconds),
  });

  await assert.rejects(
    client.probeNamespaceAccess(),
    (error) => error.code === "dropbox_retry_after_exceeds_probe_limit",
  );
  assert.deepEqual(waits, []);
});

test("keeps the request deadline active while reading the response body", async () => {
  const waits = [];
  let calls = 0;
  const value = credential();
  const client = new DropboxNamespaceClient({
    namespaceId: value.namespaceId,
    probeRoot: value.probeRoot,
    actor: value.primary,
    fetchImpl: async (_url, options) => {
      calls += 1;
      const body = new ReadableStream({
        start(controller) {
          const abort = () => controller.error(new Error("aborted"));
          if (options.signal.aborted) {
            abort();
          } else {
            options.signal.addEventListener("abort", abort, { once: true });
          }
        },
      });
      return new Response(body, {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    },
    sleepImpl: async (milliseconds) => waits.push(milliseconds),
    timeoutMs: 10,
  });

  await assert.rejects(
    client.probeNamespaceAccess(),
    (error) => error.code === "dropbox_transport_ambiguous",
  );
  assert.equal(calls, 3);
  assert.deepEqual(waits, [250, 500]);
});

test("reads only a canonical mode-0600 credential file", async () => {
  const source = {
    namespace_id: TEAM_NAMESPACE,
    probe_root: PROBE_ROOT,
    primary: { access_token: PRIMARY_TOKEN },
    secondary: { access_token: SECONDARY_TOKEN },
  };
  const fixture = await makeCredentialFixture(source);
  try {
    const value = await readCredentialFile(fixture.file);
    assert.equal(value.namespaceId, TEAM_NAMESPACE);
    await chmod(fixture.file, 0o644);
    await assert.rejects(
      readCredentialFile(fixture.file),
      (error) => error.code === "credential_file_permissions_too_broad",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("rejects a symbolic-link credential file", async () => {
  const fixture = await makeCredentialFixture({
    namespace_id: TEAM_NAMESPACE,
    probe_root: PROBE_ROOT,
    primary: { access_token: PRIMARY_TOKEN },
    secondary: { access_token: SECONDARY_TOKEN },
  });
  try {
    const link = join(fixture.root, "link.json");
    await symlink(fixture.file, link);
    await assert.rejects(
      readCredentialFile(link),
      (error) => error.code === "credential_file_must_be_regular_canonical_file",
    );
  } finally {
    await removeFixture(fixture.root);
  }
});

test("proves two-account CAS, ambiguity recovery, pagination, and cleanup", async () => {
  const fake = createFakeDropbox();
  const unrelatedPath = `${PROBE_ROOT.toLowerCase()}/operator-note.txt`;
  const unrelatedBytes = Buffer.from("preserve this file", "utf8");
  fake.objects.set(unrelatedPath, {
    kind: "file",
    bytes: unrelatedBytes,
    rev: "0000000ff",
  });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0xab),
  });

  assert.equal(result.verdict, "POC_PASS", JSON.stringify(result));
  assert.equal(result.productionGate, "INCOMPLETE");
  assert.equal(result.namespaceMode, "team_space");
  assert.equal(result.distinctAccounts, true);
  assert.ok(result.pageCount >= 2);
  assert.equal(result.objectCount, 10);
  assert.equal(result.cleanup.status, "complete");
  assert.equal(fake.objects.size, 2);
  assert.equal(fake.objects.get(PROBE_ROOT.toLowerCase())?.kind, "folder");
  assert.deepEqual(fake.objects.get(unrelatedPath)?.bytes, unrelatedBytes);
  assert.deepEqual(result.stages, [
    "two_actor_namespace_access",
    "isolated_probe_prefix",
    "absent_create_and_cross_actor_read",
    "current_revision_and_stale_rejection",
    "late_stale_identical_and_add_conflicts",
    "revision_delete_and_delete_recreate_conflict",
    "ambiguous_commit_readback",
    "zero_byte_max_component_sorted_pagination",
  ]);

  const accountRequests = fake.requests.filter(
    (request) => request.endpoint === "users/get_current_account",
  );
  assert.equal(accountRequests.length, 2);
  assert.ok(accountRequests.every((request) => request.pathRoot === null));
  const rootedRequests = fake.requests.filter(
    (request) => request.endpoint !== "users/get_current_account",
  );
  assert.ok(
    rootedRequests.every((request) => {
      const value = JSON.parse(request.pathRoot);
      return (
        value[".tag"] === "namespace_id" &&
        value.namespace_id === TEAM_NAMESPACE
      );
    }),
  );

  const deletes = fake.requests.filter(
    (request) => request.endpoint === "files/delete_v2",
  );
  const fileDeletes = deletes.map((request) =>
    JSON.parse(String(request.body)),
  );
  assert.ok(fileDeletes.length >= 10);
  assert.ok(
    fileDeletes.every(
      (value) =>
        value.path !== PROBE_ROOT &&
        typeof value.parent_rev === "string" &&
        value.parent_rev.length >= 9,
    ),
  );

  const uploadArguments = fake.requests
    .filter((request) => request.endpoint === "files/upload")
    .map((request) => JSON.parse(request.apiArgument));
  assert.ok(uploadArguments.some((value) => value.mode === "add"));
  assert.ok(
    uploadArguments.some(
      (value) =>
        value.mode?.[".tag"] === "update" &&
        typeof value.mode.update === "string",
    ),
  );
  assert.ok(
    uploadArguments.every(
      (value) => value.autorename === false && value.strict_conflict === true,
    ),
  );
  assert.equal(
    Math.max(
      ...uploadArguments.map((value) =>
        Buffer.byteLength(value.path.slice(value.path.lastIndexOf("/") + 1)),
      ),
    ),
    255,
  );
});

test("probes a two-account non-home namespace without inferring its type", async () => {
  const fake = createFakeDropbox({
    namespaceId: SHARED_NAMESPACE,
    namespaceMode: "shared_folder",
  });
  const result = await runNamespaceProbe({
    credential: credential(SHARED_NAMESPACE),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0xcd),
  });
  assert.equal(result.verdict, "POC_PASS", JSON.stringify(result));
  assert.equal(result.namespaceMode, "collaborative_non_home_namespace");
  assert.equal(fake.objects.size, 1);
});

test("rejects a personal home namespace before it creates probe data", async () => {
  const personalNamespace = "home-primary-test";
  const fake = createFakeDropbox({
    namespaceId: personalNamespace,
    namespaceMode: "shared_folder",
  });
  const result = await runNamespaceProbe({
    credential: credential(personalNamespace),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0xde),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "personal_home_namespace_not_allowed");
  assert.equal(result.cleanup.status, "not_needed");
  assert.equal(fake.objects.size, 1);
});

test("fails the POC if Dropbox accepts a stale revision", async () => {
  const fake = createFakeDropbox({ acceptStaleUpdates: true });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0xef),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "stale_revision_was_accepted");
  assert.equal(result.cleanup.status, "complete");
  assert.equal(fake.objects.size, 1);
});

test("requires the structured Dropbox file-conflict error", async () => {
  const fake = createFakeDropbox({ unstructuredUploadConflicts: true });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x45),
  });

  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "dropbox_conflict");
  assert.equal(result.cleanup.status, "complete");
  assert.equal(fake.objects.size, 1);
});

test("does not delete a collaborator replacement at a probe path", async () => {
  const fake = createFakeDropbox({ tamperKnownPathBeforeCleanup: true });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x56),
  });

  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "cleanup_required");
  assert.deepEqual(result.cleanup, {
    status: "required",
    reason: "probe_artifact_changed",
  });
  assert.equal(result.probeId, "56".repeat(12));
  const head = fake.objects.get(
    `${PROBE_ROOT.toLowerCase()}/mallard-dropbox-poc-${result.probeId}-head.bin`,
  );
  assert.equal(head?.kind, "file");
  assert.deepEqual(head?.bytes, Buffer.from("mallard-dropbox-poc-v4", "utf8"));
  assert.equal(fake.objects.get(PROBE_ROOT.toLowerCase())?.kind, "folder");
});

test("fails without creating data when the second account lacks access", async () => {
  const fake = createFakeDropbox({ denySecondaryFiles: true });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x12),
  });
  assert.equal(result.verdict, "FAIL");
  assert.equal(result.failureCode, "dropbox_forbidden");
  assert.equal(result.cleanup.status, "not_needed");
  assert.equal(fake.objects.size, 1);
});

test("does not return tokens or remote error bodies", async () => {
  const fake = createFakeDropbox({
    denySecondaryFiles: true,
    forbiddenBody: {
      error_summary: `no_permission/${SECONDARY_TOKEN}`,
      credential: SECONDARY_TOKEN,
    },
  });
  const result = await runNamespaceProbe({
    credential: credential(),
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
    randomBytesImpl: () => Buffer.alloc(12, 0x34),
  });
  const serialized = JSON.stringify(result);
  assert.equal(result.failureCode, "dropbox_forbidden");
  assert.equal(serialized.includes(PRIMARY_TOKEN), false);
  assert.equal(serialized.includes(SECONDARY_TOKEN), false);
});

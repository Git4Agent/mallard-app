// Probe the single-request create operation needed by Mallard's parent claims.
// This module is isolated from the product. Only the explicit live CLI uses Graph.
import { randomBytes } from "node:crypto";
import { pathToFileURL } from "node:url";
import {
  GraphApiError,
  OneDriveAppFolderClient,
  PocError,
  readCredentialFile,
  sha256Hex,
} from "./onedrive-app-folder-poc.mjs";

const GRAPH = "https://graph.microsoft.com";
const MAX_PAGES = 500;
const MAX_ITEMS = 5_000;

function requireValue(condition, code) {
  if (!condition) throw new PocError(code);
}

function failureCode(error) {
  if (error instanceof PocError) return error.code;
  if (error instanceof GraphApiError) return `graph_${error.classification}`;
  // Do not print a fetch error, remote body, token, path, or signed URL.
  return "request_result_unknown";
}

function itemPath(root) {
  return `/drives/${encodeURIComponent(root.driveId)}/items/${encodeURIComponent(root.id)}`;
}

export async function createFileOnce(client, root, name, bytes) {
  requireValue(/^mallard-onedrive-immutable-[0-9a-f]{24}-(claim|empty)\.bin$/u.test(name),
    "invalid_probe_name");
  requireValue(Buffer.isBuffer(bytes) && bytes.length <= 1024, "invalid_probe_bytes");
  // graphJson does not retry a write. Do not add a retry or use default replace.
  const value = await client.graphJson(
    `${itemPath(root)}:/${name}:/content?@microsoft.graph.conflictBehavior=fail`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/octet-stream" },
      body: bytes,
      redirect: "error",
    },
  );
  requireValue(
    value?.name === name && typeof value.id === "string" && value.id.length > 0 &&
    value.id.length <= 1024 && typeof value.eTag === "string" &&
    value.eTag.length > 0 && value.eTag.length <= 2048 &&
    value.size === bytes.length && value.file && !value.folder && !value.remoteItem,
    "invalid_created_file",
  );
  return { id: value.id, name, etag: value.eTag, size: value.size };
}

export async function listProbeFiles(client, root, expected) {
  const pathname = `/v1.0${itemPath(root)}/children`;
  let target = `${GRAPH}${pathname}?$top=1&$select=id,name,eTag,size,file,folder,remoteItem`;
  const visited = new Set();
  const found = new Map();
  let count = 0;
  while (target !== undefined) {
    let url;
    try { url = new URL(target); } catch { throw new PocError("invalid_list_cursor"); }
    requireValue(url.origin === GRAPH && url.pathname === pathname &&
      !url.username && !url.password && !url.hash, "list_cursor_left_folder");
    requireValue(!visited.has(url.href), "repeated_list_cursor");
    requireValue(visited.size < MAX_PAGES, "list_page_limit");
    visited.add(url.href);
    const page = await client.graphJson(url.href, { readOnly: true, redirect: "error" });
    requireValue(Array.isArray(page?.value), "invalid_list_page");
    count += page.value.length;
    requireValue(count <= MAX_ITEMS, "list_item_limit");
    for (const item of page.value) {
      requireValue(typeof item?.name === "string", "invalid_list_item");
      if (!expected.has(item.name)) continue;
      const wanted = expected.get(item.name);
      requireValue(!found.has(item.name), "duplicate_probe_item");
      requireValue(item.id === wanted.id && item.eTag === wanted.etag &&
        item.size === wanted.size && item.file && !item.folder && !item.remoteItem,
      "listed_probe_changed");
      found.set(item.name, item);
    }
    const next = page["@odata.nextLink"];
    requireValue(next === undefined ||
      (typeof next === "string" && next.length > 0 && next.length <= 16_384),
    "invalid_list_cursor");
    target = next;
  }
  requireValue(found.size === expected.size, "listing_omitted_probe_file");
  requireValue(visited.size >= 2, "pagination_not_exercised");
  return { pages: visited.size, matched: found.size };
}

async function cleanup(client, root, known, unknownWrites) {
  if (!root) return { status: "not_needed" };
  let unresolved = 0;
  for (const [name, allowed] of known) {
    try {
      const item = await client.getFileByPath(root.driveId, root.id, name);
      if (!item) {
        // A lost response can leave a request in progress on the server.
        // Absence at this instant does not prove that no file will appear.
        if (unknownWrites.has(name)) unresolved += 1;
        continue;
      }
      const bytes = await client.download(root.driveId, item.id);
      requireValue(allowed.has(sha256Hex(bytes)), "cleanup_content_changed");
      // The eTag binds deletion to the revision inspected before the download.
      await client.deleteItem(root.driveId, item.id, item.etag);
    } catch {
      unresolved += 1;
    }
  }
  return unresolved ? { status: "required", unresolved } : { status: "complete" };
}

export async function runImmutableProbe({ credential, fetchImpl = globalThis.fetch,
  sleepImpl, randomBytesImpl = randomBytes }) {
  const probeId = Buffer.from(randomBytesImpl(12)).toString("hex");
  requireValue(/^[0-9a-f]{24}$/u.test(probeId), "invalid_probe_id");
  requireValue(credential.clientA.accessToken !== credential.clientB.accessToken,
    "client_authorizations_not_distinct");
  const a = new OneDriveAppFolderClient({ actor: credential.clientA, fetchImpl, sleepImpl });
  const b = new OneDriveAppFolderClient({ actor: credential.clientB, fetchImpl, sleepImpl });
  const name = `mallard-onedrive-immutable-${probeId}-claim.bin`;
  const emptyName = `mallard-onedrive-immutable-${probeId}-empty.bin`;
  const payloads = [Buffer.from("synthetic claim A"), Buffer.from("synthetic claim B")];
  const known = new Map();
  const unknownWrites = new Set();
  const stages = [];
  let root;
  let failure;
  let listing;
  const create = async (client, fileName, bytes) => {
    try { return await createFileOnce(client, root, fileName, bytes); }
    catch (error) {
      if (!(error instanceof GraphApiError && error.status >= 400 && error.status < 500)) {
        unknownWrites.add(fileName);
      }
      throw error;
    }
  };
  try {
    const [rootA, rootB] = await Promise.all([a.appRoot(), b.appRoot()]);
    requireValue(rootA.id === rootB.id && rootA.driveId === rootB.driveId &&
      rootA.driveType === rootB.driveType, "client_app_folders_do_not_match");
    root = rootA;
    await Promise.all([a.requireOutsideAppFolderDenial(), b.requireOutsideAppFolderDenial()]);
    stages.push("same_app_folder_and_scope_isolation");

    // Refuse any name already present. Only later attempted names can be cleaned.
    for (const fileName of [name, emptyName]) {
      requireValue(await a.getFileByPath(root.driveId, root.id, fileName) === null,
        "probe_name_already_exists");
    }
    known.set(name, new Set(payloads.map(sha256Hex)));
    const writes = await Promise.allSettled([
      create(a, name, payloads[0]),
      create(b, name, payloads[1]),
    ]);
    // Both requests must settle before cleanup can start.
    const winners = writes.flatMap((result, index) =>
      result.status === "fulfilled" ? [index] : []);
    requireValue(!unknownWrites.has(name), "request_result_unknown");
    requireValue(winners.length === 1, "create_race_did_not_have_one_winner");
    const winner = winners[0];
    const loser = writes[1 - winner];
    requireValue(loser.status === "rejected" && loser.reason instanceof GraphApiError &&
      loser.reason.status === 409, "create_race_not_proved");
    const item = writes[winner].value;
    await b.verifyDownload(root.driveId, item.id, payloads[winner], item.etag);
    stages.push("one_create_winner_other_client_verified_bytes");

    let rejected = false;
    try { await create(a, name, payloads[1 - winner]); }
    catch (error) {
      if (!(error instanceof GraphApiError && error.status === 409)) throw error;
      rejected = true;
    }
    requireValue(rejected, "existing_file_was_replaced");
    await b.verifyDownload(root.driveId, item.id, payloads[winner], item.etag);
    stages.push("existing_name_rejected_and_winner_preserved");

    const empty = Buffer.alloc(0);
    known.set(emptyName, new Set([sha256Hex(empty)]));
    const emptyItem = await create(a, emptyName, empty);
    await b.verifyDownload(root.driveId, emptyItem.id, empty, emptyItem.etag);
    stages.push("zero_byte_object_verified");
    listing = await listProbeFiles(b, root, new Map([[name, item], [emptyName, emptyItem]]));
    stages.push("other_client_paginated_listing_observed");
  } catch (error) {
    failure = failureCode(error);
  }
  const cleaned = await cleanup(a, root, known, unknownWrites);
  if (cleaned.status === "required" && !failure) failure = "cleanup_required";
  return {
    verdict: failure ? "FAIL" : "PASS",
    productionReady: false,
    ...(root ? { driveType: root.driveType } : {}),
    ...(failure ? { failureCode: failure } : {}),
    ...(listing ? { listing } : {}),
    cleanup: cleaned,
    ...(cleaned.status === "required" ? { probeId } : {}),
    stages,
  };
}

export async function executeCli(argv = process.argv.slice(2)) {
  if (argv.length === 1 && argv[0] === "--help") return { help:
    "node immutable-storage-poc.mjs live --credentials /absolute/path/to/onedrive-poc.json" };
  requireValue(argv.length === 3 && argv[0] === "live" && argv[1] === "--credentials",
    "invalid_arguments");
  const credential = await readCredentialFile(argv[2]);
  return runImmutableProbe({ credential });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  executeCli().then((result) => {
    process.stdout.write(`${result.help ?? JSON.stringify(result, null, 2)}\n`);
    if (!result.help && result.verdict !== "PASS") process.exitCode = 1;
  }).catch((error) => {
    process.stdout.write(`${JSON.stringify({ verdict: "FAIL", failureCode: failureCode(error) })}\n`);
    process.exitCode = 1;
  });
}

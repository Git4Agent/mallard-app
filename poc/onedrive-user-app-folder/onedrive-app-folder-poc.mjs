import { createHash, randomBytes } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const GRAPH_ORIGIN = "https://graph.microsoft.com";
const GRAPH_API_ROOT = `${GRAPH_ORIGIN}/v1.0`;
const CREDENTIAL_FILE_MAX_BYTES = 64 * 1024;
const JSON_RESPONSE_MAX_BYTES = 1024 * 1024;
const PROBE_FILE_MAX_BYTES = 2 * 1024 * 1024;
const HTTP_ATTEMPTS = 3;
const HTTP_TIMEOUT_MS = 30_000;
const RETRY_DELAY_MAX_MS = 5_000;
const UPLOAD_FRAGMENT_BYTES = 320 * 1024;
const RACE_FILE_BYTES = 2 * UPLOAD_FRAGMENT_BYTES;
const PROBE_FILE_PATTERN = /^[a-z0-9][a-z0-9.-]{0,127}$/u;
const APP_FOLDER_SCOPE = "files.readwrite.appfolder";

export const POC_STATUS = Object.freeze({
  productionReady: false,
  persistedSchemaChanged: false,
  verdict: "REQUIRES_LIVE_PROBE",
});

export class PocError extends Error {
  constructor(code) {
    super(code);
    this.name = "PocError";
    this.code = code;
  }
}

export class GraphApiError extends Error {
  constructor(status, classification) {
    super("graph_api_failure");
    this.name = "GraphApiError";
    this.status = status;
    this.classification = classification;
  }
}

class AmbiguousTransportError extends Error {
  constructor(code) {
    super(code);
    this.name = "AmbiguousTransportError";
    this.code = code;
  }
}

function fail(code) {
  throw new PocError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function exactKeys(value, required, optional, label) {
  if (!isPlainObject(value)) fail(`invalid_${label}`);
  const allowed = new Set([...required, ...optional]);
  const keys = Object.keys(value);
  if (
    required.some((key) => !Object.hasOwn(value, key)) ||
    keys.some((key) => !allowed.has(key))
  ) {
    fail(`invalid_${label}_fields`);
  }
}

function boundedString(value, label, maximum, pattern) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maximum ||
    value.trim() !== value ||
    /[\u0000-\u001f\u007f]/u.test(value) ||
    (pattern && !pattern.test(value))
  ) {
    fail(`invalid_${label}`);
  }
  return value;
}

function validateAccessToken(value) {
  return boundedString(value, "access_token", 16_384);
}

function normalizeScope(value) {
  const lower = value.toLowerCase();
  const graphPrefix = "https://graph.microsoft.com/";
  return lower.startsWith(graphPrefix) ? lower.slice(graphPrefix.length) : lower;
}

function validateGrantedScopes(value, label) {
  const source = boundedString(value, `${label}_granted_scopes`, 4096);
  const scopes = new Set(source.split(/\s+/u).map(normalizeScope));
  if (!scopes.has(APP_FOLDER_SCOPE)) {
    fail(`${label}_app_folder_scope_missing`);
  }
  if (
    [...scopes].some(
      (scope) =>
        scope !== APP_FOLDER_SCOPE &&
        (scope.startsWith("files.") || scope.startsWith("sites.")),
    )
  ) {
    fail(`${label}_broader_file_scope_present`);
  }
  return source;
}

function validateActor(value, label) {
  exactKeys(value, ["access_token", "granted_scopes"], [], `${label}_actor`);
  return Object.freeze({
    accessToken: validateAccessToken(value.access_token),
    grantedScopes: validateGrantedScopes(value.granted_scopes, label),
  });
}

export function validateCredential(value) {
  exactKeys(value, ["client_a", "client_b"], [], "credential");
  const credential = Object.freeze({
    clientA: validateActor(value.client_a, "client_a"),
    clientB: validateActor(value.client_b, "client_b"),
  });
  if (credential.clientA.accessToken === credential.clientB.accessToken) {
    fail("client_authorizations_not_distinct");
  }
  return credential;
}

export async function readCredentialFile(filePath) {
  if (typeof filePath !== "string" || !isAbsolute(filePath)) {
    fail("credential_file_must_be_absolute");
  }
  const normalized = resolve(filePath);
  let metadata;
  let canonical;
  try {
    metadata = await lstat(normalized);
    canonical = await realpath(normalized);
  } catch {
    fail("credential_file_unavailable");
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || canonical !== normalized) {
    fail("credential_file_must_be_regular_canonical_file");
  }

  let handle;
  try {
    handle = await open(
      normalized,
      fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
    );
    const before = await handle.stat();
    if (
      !before.isFile() ||
      before.dev !== metadata.dev ||
      before.ino !== metadata.ino ||
      before.nlink !== 1 ||
      before.size <= 0 ||
      before.size > CREDENTIAL_FILE_MAX_BYTES
    ) {
      fail("invalid_credential_file_size");
    }
    if (process.platform === "win32") {
      fail("windows_credential_acl_probe_not_implemented");
    }
    if ((before.mode & 0o077) !== 0) {
      fail("credential_file_permissions_too_broad");
    }
    const source = await handle.readFile({ encoding: "utf8" });
    const after = await handle.stat();
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeMs !== after.mtimeMs ||
      before.ctimeMs !== after.ctimeMs
    ) {
      fail("credential_file_changed_during_read");
    }
    let parsed;
    try {
      parsed = JSON.parse(source);
    } catch {
      fail("invalid_credential_json");
    }
    return validateCredential(parsed);
  } finally {
    await handle?.close().catch(() => {});
  }
}

export function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function safeErrorCode(error) {
  if (error instanceof PocError || error instanceof AmbiguousTransportError) {
    return error.code;
  }
  if (error instanceof GraphApiError) {
    return `graph_${error.classification}`;
  }
  return "unexpected_failure";
}

function validateOpaqueId(value, label) {
  return boundedString(value, label, 1024);
}

function validateEtag(value) {
  return boundedString(value, "graph_etag", 2048);
}

function validateProbeFileName(value) {
  return boundedString(value, "probe_file", 128, PROBE_FILE_PATTERN);
}

function encodeSegment(value) {
  return encodeURIComponent(validateOpaqueId(value, "graph_path_segment"));
}

async function readBytesBounded(response, maximum, errorCode = "graph_response_too_large") {
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    const parsed = Number(contentLength);
    if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > maximum) {
      fail(errorCode);
    }
  }
  if (!response.body) return Buffer.alloc(0);
  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > maximum) {
      await reader.cancel().catch(() => {});
      fail(errorCode);
    }
    chunks.push(Buffer.from(value));
  }
  return Buffer.concat(chunks, total);
}

async function readJsonBounded(response) {
  const bytes = await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES);
  if (bytes.length === 0) fail("graph_empty_json_response");
  try {
    const value = JSON.parse(bytes.toString("utf8"));
    if (!isPlainObject(value)) fail("invalid_graph_json_response");
    return value;
  } catch (error) {
    if (error instanceof PocError) throw error;
    fail("invalid_graph_json_response");
  }
}

function graphErrorCodes(value) {
  const codes = [];
  let cursor = isPlainObject(value) && isPlainObject(value.error) ? value.error : value;
  for (let depth = 0; depth < 8 && isPlainObject(cursor); depth += 1) {
    if (typeof cursor.code === "string") codes.push(cursor.code.toLowerCase());
    cursor = isPlainObject(cursor.innerError)
      ? cursor.innerError
      : isPlainObject(cursor.innererror)
        ? cursor.innererror
        : undefined;
  }
  return codes;
}

function classifyGraphError(status, value) {
  const codes = graphErrorCodes(value);
  if (status === 401) return "unauthorized";
  if (codes.includes("insufficient_claims")) return "conditional_access";
  if (status === 403) return "forbidden";
  if (status === 404) return "not_found";
  if (status === 409) return "conflict";
  if (status === 412) return "precondition_failed";
  if (status === 416) return "range_rejected";
  if (status === 423) return "locked";
  if (status === 429) return "rate_limited";
  if (status === 507 || codes.includes("quotalimitreached")) return "quota_exceeded";
  if (status >= 500) return "service_failure";
  return "request_rejected";
}

async function graphApiError(response) {
  let value = {};
  try {
    value = await readJsonBounded(response);
  } catch {
    // The status code still maps to a sanitized failure.
  }
  return new GraphApiError(
    response.status,
    classifyGraphError(response.status, value),
  );
}

function retryDelayMs(response, attempt) {
  const retryAfter = response.headers.get("retry-after");
  if (retryAfter !== null && /^\d+$/u.test(retryAfter)) {
    return Math.min(Number(retryAfter) * 1000, RETRY_DELAY_MAX_MS);
  }
  return Math.min(250 * 2 ** attempt, RETRY_DELAY_MAX_MS);
}

function validateGraphTarget(target) {
  if (typeof target !== "string" || target.length === 0 || target.length > 16_384) {
    fail("invalid_graph_target");
  }
  if (target.startsWith("/")) return `${GRAPH_API_ROOT}${target}`;
  let url;
  try {
    url = new URL(target);
  } catch {
    fail("invalid_graph_target");
  }
  if (
    url.origin !== GRAPH_ORIGIN ||
    !url.pathname.startsWith("/v1.0/") ||
    url.username ||
    url.password ||
    url.hash
  ) {
    fail("invalid_graph_target");
  }
  return url.href;
}

function validateUploadUrl(value) {
  const source = boundedString(value, "upload_url", 16_384);
  let url;
  try {
    url = new URL(source);
  } catch {
    fail("invalid_upload_url");
  }
  if (
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.hash ||
    url.hostname.length === 0
  ) {
    fail("invalid_upload_url");
  }
  return url.href;
}

function validateFileItem(value, expected = {}) {
  if (!isPlainObject(value) || !isPlainObject(value.file)) {
    fail("invalid_graph_file_item");
  }
  const item = Object.freeze({
    id: validateOpaqueId(value.id, "graph_item_id"),
    name: boundedString(value.name, "graph_item_name", 255),
    etag: validateEtag(value.eTag),
    size: value.size,
  });
  if (!Number.isSafeInteger(item.size) || item.size < 0 || item.size > PROBE_FILE_MAX_BYTES) {
    fail("invalid_graph_file_size");
  }
  if (expected.id !== undefined && item.id !== expected.id) {
    fail("graph_item_identity_changed");
  }
  if (expected.name !== undefined && item.name !== expected.name) {
    fail("graph_item_name_changed");
  }
  return item;
}

function validateFolderItem(value, expected = {}) {
  if (!isPlainObject(value) || !isPlainObject(value.folder)) {
    fail("invalid_graph_folder_item");
  }
  const item = Object.freeze({
    id: validateOpaqueId(value.id, "graph_item_id"),
    name: boundedString(value.name, "graph_item_name", 255),
    etag: validateEtag(value.eTag),
  });
  if (expected.id !== undefined && item.id !== expected.id) {
    fail("graph_folder_identity_changed");
  }
  if (expected.name !== undefined && item.name !== expected.name) {
    fail("graph_folder_name_changed");
  }
  return item;
}

function validateAppRoot(value) {
  const folder = validateFolderItem(value);
  if (
    !isPlainObject(value.specialFolder) ||
    value.specialFolder.name !== "approot" ||
    !isPlainObject(value.parentReference)
  ) {
    fail("invalid_app_folder_response");
  }
  const driveType = value.parentReference.driveType;
  if (!["personal", "business"].includes(driveType)) {
    fail("unsupported_user_drive_type");
  }
  return Object.freeze({
    ...folder,
    driveId: validateOpaqueId(value.parentReference.driveId, "graph_drive_id"),
    driveType,
  });
}

function validateUploadSession(value) {
  if (!isPlainObject(value)) fail("invalid_upload_session_response");
  return Object.freeze({ uploadUrl: validateUploadUrl(value.uploadUrl) });
}

function makeProbeId(randomBytesImpl) {
  const value = Buffer.from(randomBytesImpl(12)).toString("hex");
  if (!/^[0-9a-f]{24}$/u.test(value)) fail("invalid_probe_randomness");
  return value;
}

function patternBytes(byte) {
  return Buffer.alloc(RACE_FILE_BYTES, byte);
}

function assertExpectedBytes(actual, expected) {
  const wanted = Buffer.from(expected);
  if (
    actual.length !== wanted.length ||
    sha256Hex(actual) !== sha256Hex(wanted) ||
    !actual.equals(wanted)
  ) {
    fail("onedrive_verified_read_mismatch");
  }
}

function nextExpectedRangeStartsAt(ranges, offset) {
  if (!Array.isArray(ranges)) return false;
  return ranges.some((range) => {
    if (typeof range !== "string") return false;
    const match = /^(\d+)(?:-(\d*))?$/u.exec(range);
    return match !== null && Number(match[1]) === offset;
  });
}

export class OneDriveAppFolderClient {
  constructor({
    actor,
    fetchImpl = globalThis.fetch,
    sleepImpl = (milliseconds) =>
      new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)),
    timeoutMs = HTTP_TIMEOUT_MS,
  }) {
    this.actor = actor;
    this.fetchImpl = fetchImpl;
    this.sleepImpl = sleepImpl;
    this.timeoutMs = timeoutMs;
  }

  async fetchResponse(url, options) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      return await this.fetchImpl(url, { ...options, signal: controller.signal });
    } catch {
      throw new AmbiguousTransportError("onedrive_transport_ambiguous");
    } finally {
      clearTimeout(timer);
    }
  }

  async graphResponse(
    target,
    { method = "GET", headers = {}, body, readOnly = false, redirect } = {},
  ) {
    const url = validateGraphTarget(target);
    for (let attempt = 0; attempt < HTTP_ATTEMPTS; attempt += 1) {
      let response;
      try {
        response = await this.fetchResponse(url, {
          method,
          headers: {
            Authorization: `Bearer ${this.actor.accessToken}`,
            ...headers,
          },
          ...(body === undefined ? {} : { body }),
          ...(redirect === undefined ? {} : { redirect }),
        });
      } catch (error) {
        if (!(readOnly && attempt + 1 < HTTP_ATTEMPTS)) throw error;
        await this.sleepImpl(Math.min(250 * 2 ** attempt, RETRY_DELAY_MAX_MS));
        continue;
      }
      const retryable =
        readOnly &&
        attempt + 1 < HTTP_ATTEMPTS &&
        (response.status === 429 || response.status >= 500);
      if (!retryable) return response;
      const delay = retryDelayMs(response, attempt);
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      await this.sleepImpl(delay);
    }
    fail("graph_retry_exhausted");
  }

  async graphJson(target, options = {}) {
    const response = await this.graphResponse(target, options);
    if (!response.ok) throw await graphApiError(response);
    return readJsonBounded(response);
  }

  async appRoot() {
    const value = await this.graphJson(
      "/me/drive/special/approot?$select=id,name,eTag,parentReference,folder,specialFolder",
      { readOnly: true },
    );
    return validateAppRoot(value);
  }

  async requireOutsideAppFolderDenial() {
    const response = await this.graphResponse("/me/drive/root?$select=id", {
      readOnly: true,
    });
    if (response.ok) {
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      fail("outside_app_folder_access_was_allowed");
    }
    const error = await graphApiError(response);
    if (error.status !== 403 || error.classification !== "forbidden") {
      throw error;
    }
  }

  async getItem(driveId, itemId) {
    let value;
    try {
      value = await this.graphJson(
        `/drives/${encodeSegment(driveId)}/items/${encodeSegment(itemId)}?$select=id,name,eTag,size,file,folder`,
        { readOnly: true },
      );
    } catch (error) {
      if (error instanceof GraphApiError && error.status === 404) return null;
      throw error;
    }
    return isPlainObject(value.file)
      ? validateFileItem(value, { id: itemId })
      : validateFolderItem(value, { id: itemId });
  }

  async getFileByPath(driveId, parentId, name) {
    const fileName = validateProbeFileName(name);
    let value;
    try {
      value = await this.graphJson(
        `/drives/${encodeSegment(driveId)}/items/${encodeSegment(parentId)}:/${encodeURIComponent(fileName)}?$select=id,name,eTag,size,file`,
        { readOnly: true },
      );
    } catch (error) {
      if (error instanceof GraphApiError && error.status === 404) return null;
      throw error;
    }
    return validateFileItem(value, { name: fileName });
  }

  async createFileSession(driveId, parentId, name) {
    const fileName = validateProbeFileName(name);
    const value = await this.graphJson(
      `/drives/${encodeSegment(driveId)}/items/${encodeSegment(parentId)}:/${encodeURIComponent(fileName)}:/createUploadSession`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          item: {
            "@microsoft.graph.conflictBehavior": "fail",
            name: fileName,
          },
        }),
      },
    );
    return validateUploadSession(value);
  }

  async createUpdateSession(driveId, itemId, etag) {
    const value = await this.graphJson(
      `/drives/${encodeSegment(driveId)}/items/${encodeSegment(itemId)}/createUploadSession`,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "If-Match": validateEtag(etag),
        },
        body: JSON.stringify({
          item: { "@microsoft.graph.conflictBehavior": "replace" },
        }),
      },
    );
    return validateUploadSession(value);
  }

  async uploadRangeResponse(uploadUrl, bytes, start, total) {
    const body = Buffer.from(bytes);
    if (
      !Number.isSafeInteger(start) ||
      start < 0 ||
      !Number.isSafeInteger(total) ||
      total <= 0 ||
      total > PROBE_FILE_MAX_BYTES ||
      body.length <= 0 ||
      start + body.length > total
    ) {
      fail("invalid_upload_range");
    }
    return this.fetchResponse(validateUploadUrl(uploadUrl), {
      method: "PUT",
      headers: {
        "Content-Length": String(body.length),
        "Content-Range": `bytes ${start}-${start + body.length - 1}/${total}`,
      },
      body,
    });
  }

  async uploadProgress(uploadUrl, bytes, start, total) {
    const response = await this.uploadRangeResponse(uploadUrl, bytes, start, total);
    if (response.status !== 202) {
      if (!response.ok) throw await graphApiError(response);
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      fail("nonfinal_upload_range_committed");
    }
    const value = await readJsonBounded(response);
    if (!nextExpectedRangeStartsAt(
      value.nextExpectedRanges,
      start + Buffer.byteLength(bytes),
    )) {
      fail("invalid_upload_progress_response");
    }
  }

  async uploadComplete(uploadUrl, bytes) {
    const body = Buffer.from(bytes);
    if (body.length <= 0 || body.length > PROBE_FILE_MAX_BYTES) {
      fail("invalid_complete_upload_size");
    }
    const response = await this.uploadRangeResponse(uploadUrl, body, 0, body.length);
    if (!response.ok) throw await graphApiError(response);
    if (![200, 201].includes(response.status)) {
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      fail("invalid_upload_completion_status");
    }
    const item = validateFileItem(await readJsonBounded(response));
    if (item.size !== body.length) fail("upload_completion_size_mismatch");
    return item;
  }

  async expectStaleFinalRejection(uploadUrl, bytes, start, total) {
    const response = await this.uploadRangeResponse(uploadUrl, bytes, start, total);
    if (response.ok) {
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      fail("stale_final_range_was_accepted");
    }
    const error = await graphApiError(response);
    if (![404, 409, 412].includes(error.status)) {
      throw error;
    }
    return Object.freeze({ status: error.status });
  }

  async cancelUploadSession(uploadUrl) {
    const validatedUrl = validateUploadUrl(uploadUrl);
    let response;
    try {
      response = await this.fetchResponse(validatedUrl, {
        method: "DELETE",
      });
    } catch {
      response = undefined;
    }
    if (response && [204, 404, 410].includes(response.status)) {
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
      return true;
    }
    if (response) {
      await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(() => {});
    }
    let statusResponse;
    try {
      statusResponse = await this.fetchResponse(validatedUrl, { method: "GET" });
    } catch {
      return false;
    }
    if ([404, 410].includes(statusResponse.status)) {
      await readBytesBounded(statusResponse, JSON_RESPONSE_MAX_BYTES).catch(
        () => {},
      );
      return true;
    }
    await readBytesBounded(statusResponse, JSON_RESPONSE_MAX_BYTES).catch(
      () => {},
    );
    return false;
  }

  async simpleReplace(driveId, itemId, name, bytes) {
    const body = Buffer.from(bytes);
    if (body.length > PROBE_FILE_MAX_BYTES) fail("probe_file_too_large");
    const response = await this.graphResponse(
      `/drives/${encodeSegment(driveId)}/items/${encodeSegment(itemId)}/content`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/octet-stream" },
        body,
      },
    );
    if (!response.ok) throw await graphApiError(response);
    const item = validateFileItem(await readJsonBounded(response), {
      id: itemId,
      name: validateProbeFileName(name),
    });
    if (item.size !== body.length) fail("simple_upload_size_mismatch");
    return item;
  }

  async download(driveId, itemId) {
    const response = await this.graphResponse(
      `/drives/${encodeSegment(driveId)}/items/${encodeSegment(itemId)}/content`,
      { readOnly: true, redirect: "manual" },
    );
    let contentResponse = response;
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      const location = response.headers.get("location");
      if (!location) fail("onedrive_download_redirect_missing");
      contentResponse = await this.fetchResponse(validateUploadUrl(location), {
        method: "GET",
        redirect: "error",
      });
    }
    if (!contentResponse.ok) throw await graphApiError(contentResponse);
    return readBytesBounded(
      contentResponse,
      PROBE_FILE_MAX_BYTES,
      "onedrive_download_too_large",
    );
  }

  async verifyDownload(driveId, itemId, expectedBytes, expectedEtag) {
    const metadata = await this.getItem(driveId, itemId);
    if (
      metadata === null ||
      !Object.hasOwn(metadata, "size") ||
      metadata.etag !== expectedEtag ||
      metadata.size !== Buffer.byteLength(expectedBytes)
    ) {
      fail("onedrive_verified_metadata_mismatch");
    }
    const bytes = await this.download(driveId, itemId);
    assertExpectedBytes(bytes, expectedBytes);
    return metadata;
  }

  async deleteItem(driveId, itemId, etag) {
    const response = await this.graphResponse(
      `/drives/${encodeSegment(driveId)}/items/${encodeSegment(itemId)}`,
      {
        method: "DELETE",
        headers: { "If-Match": validateEtag(etag) },
      },
    );
    if (response.status !== 204) throw await graphApiError(response);
  }
}

function sameAppRoot(left, right) {
  return (
    left.id === right.id &&
    left.driveId === right.driveId &&
    left.driveType === right.driveType
  );
}

async function expectStaleSessionStart(operation) {
  try {
    await operation();
  } catch (error) {
    if (error instanceof GraphApiError && error.status === 412) return;
    throw error;
  }
  fail("stale_session_start_was_accepted");
}

async function uploadCompleteOrCancel(client, session, bytes) {
  try {
    return await client.uploadComplete(session.uploadUrl, bytes);
  } catch (error) {
    if (!(await client.cancelUploadSession(session.uploadUrl))) {
      fail("upload_session_cleanup_failed");
    }
    throw error;
  }
}

async function runFinalCommitRace({
  clientA,
  clientB,
  appRoot,
  fileName,
  winnerMode,
  bytes,
  rememberItem,
}) {
  rememberItem(fileName, [bytes.baseline, bytes.stale, bytes.winner]);
  const createSession = await clientA.createFileSession(
    appRoot.driveId,
    appRoot.id,
    fileName,
  );
  const baseline = await uploadCompleteOrCancel(
    clientA,
    createSession,
    bytes.baseline,
  );
  if (baseline.name !== fileName) fail("created_file_name_mismatch");
  rememberItem(fileName, [bytes.baseline, bytes.stale, bytes.winner], baseline.id);
  await clientB.verifyDownload(
    appRoot.driveId,
    baseline.id,
    bytes.baseline,
    baseline.etag,
  );

  const staleSession = await clientA.createUpdateSession(
    appRoot.driveId,
    baseline.id,
    baseline.etag,
  );
  await clientA.uploadProgress(
    staleSession.uploadUrl,
    bytes.stale.subarray(0, UPLOAD_FRAGMENT_BYTES),
    0,
    bytes.stale.length,
  );

  let winner;
  let rejection;
  let raceError;
  try {
    if (winnerMode === "upload_session") {
      const winnerSession = await clientB.createUpdateSession(
        appRoot.driveId,
        baseline.id,
        baseline.etag,
      );
      winner = await uploadCompleteOrCancel(
        clientB,
        winnerSession,
        bytes.winner,
      );
    } else if (winnerMode === "simple_upload") {
      winner = await clientB.simpleReplace(
        appRoot.driveId,
        baseline.id,
        fileName,
        bytes.winner,
      );
    } else {
      fail("invalid_winner_mode");
    }
    if (winner.id !== baseline.id || winner.name !== fileName) {
      fail("winner_item_identity_changed");
    }
    if (winner.etag === baseline.etag) fail("winner_etag_did_not_advance");
    await clientA.verifyDownload(
      appRoot.driveId,
      baseline.id,
      bytes.winner,
      winner.etag,
    );

    rejection = await clientA.expectStaleFinalRejection(
      staleSession.uploadUrl,
      bytes.stale.subarray(UPLOAD_FRAGMENT_BYTES),
      UPLOAD_FRAGMENT_BYTES,
      bytes.stale.length,
    );
    await clientB.verifyDownload(
      appRoot.driveId,
      baseline.id,
      bytes.winner,
      winner.etag,
    );
  } catch (error) {
    raceError = error;
  }
  if (!(await clientA.cancelUploadSession(staleSession.uploadUrl))) {
    fail("stale_upload_session_cleanup_failed");
  }
  if (raceError) throw raceError;
  await expectStaleSessionStart(() =>
    clientA.createUpdateSession(
      appRoot.driveId,
      baseline.id,
      baseline.etag,
    ),
  );

  return Object.freeze({
    winnerMode,
    staleFinalStatus: rejection.status,
  });
}

async function cleanupProbe({ client, appRoot, knownItems }) {
  if (!appRoot || knownItems.size === 0) {
    return Object.freeze({ status: "not_needed" });
  }
  try {
    for (const [name, expected] of knownItems) {
      const current = await client.getFileByPath(
        appRoot.driveId,
        appRoot.id,
        name,
      );
      if (current === null) {
        if (expected.id !== undefined) {
          const moved = await client.getItem(appRoot.driveId, expected.id);
          if (moved !== null) {
            return Object.freeze({
              status: "required",
              reason: "probe_file_moved",
            });
          }
        }
        continue;
      }
      if (expected.id !== undefined && current.id !== expected.id) {
        return Object.freeze({
          status: "required",
          reason: "probe_path_identity_changed",
        });
      }
      const bytes = await client.download(appRoot.driveId, current.id);
      if (!expected.allowedDigests.has(sha256Hex(bytes))) {
        return Object.freeze({
          status: "required",
          reason: "unexpected_probe_file_content",
        });
      }
      await client.deleteItem(appRoot.driveId, current.id, current.etag);
      if (
        (await client.getFileByPath(appRoot.driveId, appRoot.id, name)) !== null
      ) {
        return Object.freeze({
          status: "required",
          reason: "probe_file_still_present",
        });
      }
    }
    return Object.freeze({ status: "complete" });
  } catch (error) {
    return Object.freeze({
      status: "required",
      reason: safeErrorCode(error),
    });
  }
}

export async function runAppFolderProbe({
  credential,
  fetchImpl = globalThis.fetch,
  sleepImpl,
  randomBytesImpl = randomBytes,
}) {
  const validated = validateCredential({
    client_a: {
      access_token: credential.clientA.accessToken,
      granted_scopes: credential.clientA.grantedScopes,
    },
    client_b: {
      access_token: credential.clientB.accessToken,
      granted_scopes: credential.clientB.grantedScopes,
    },
  });
  const clientA = new OneDriveAppFolderClient({
    actor: validated.clientA,
    fetchImpl,
    ...(sleepImpl ? { sleepImpl } : {}),
  });
  const clientB = new OneDriveAppFolderClient({
    actor: validated.clientB,
    fetchImpl,
    ...(sleepImpl ? { sleepImpl } : {}),
  });
  const probeId = makeProbeId(randomBytesImpl);
  const knownItems = new Map();
  const stages = [];
  let appRoot;
  let failureCode;
  let uploadSessionCleanupUnresolved = false;
  const raceResults = [];

  const rememberItem = (name, allowedBytes, id) => {
    const fileName = validateProbeFileName(name);
    if (!Array.isArray(allowedBytes) || allowedBytes.length === 0) {
      fail("invalid_cleanup_digest_set");
    }
    const allowedDigests = new Set(
      allowedBytes.map((bytes) => sha256Hex(Buffer.from(bytes))),
    );
    const existing = knownItems.get(fileName);
    if (existing !== undefined) {
      if (
        existing.allowedDigests.size !== allowedDigests.size ||
        [...existing.allowedDigests].some((digest) => !allowedDigests.has(digest))
      ) {
        fail("probe_cleanup_digest_changed");
      }
      if (id !== undefined) existing.id = validateOpaqueId(id, "graph_item_id");
      return;
    }
    knownItems.set(fileName, {
      allowedDigests,
      ...(id === undefined ? {} : { id: validateOpaqueId(id, "graph_item_id") }),
    });
  };

  try {
    const [rootA, rootB] = await Promise.all([
      clientA.appRoot(),
      clientB.appRoot(),
    ]);
    if (!sameAppRoot(rootA, rootB)) fail("client_app_folders_do_not_match");
    appRoot = rootA;
    await Promise.all([
      clientA.requireOutsideAppFolderDenial(),
      clientB.requireOutsideAppFolderDenial(),
    ]);
    stages.push("app_folder_access_and_scope_isolation");
    stages.push("same_account_namespace_two_tokens");

    raceResults.push(
      await runFinalCommitRace({
        clientA,
        clientB,
        appRoot,
        fileName: `mallard-onedrive-poc-${probeId}-session.bin`,
        winnerMode: "upload_session",
        bytes: {
          baseline: patternBytes(0x11),
          stale: patternBytes(0xa1),
          winner: patternBytes(0xb1),
        },
        rememberItem,
      }),
    );
    stages.push("stale_final_rejected_after_session_replace");

    raceResults.push(
      await runFinalCommitRace({
        clientA,
        clientB,
        appRoot,
        fileName: `mallard-onedrive-poc-${probeId}-simple.bin`,
        winnerMode: "simple_upload",
        bytes: {
          baseline: patternBytes(0x12),
          stale: patternBytes(0xa2),
          winner: patternBytes(0xb2),
        },
        rememberItem,
      }),
    );
    stages.push("stale_final_rejected_after_simple_replace");
  } catch (error) {
    failureCode = safeErrorCode(error);
    uploadSessionCleanupUnresolved =
      failureCode === "stale_upload_session_cleanup_failed" ||
      failureCode === "upload_session_cleanup_failed";
  }

  let cleanup = await cleanupProbe({
    client: clientA,
    appRoot,
    knownItems,
  });
  if (uploadSessionCleanupUnresolved && cleanup.status !== "required") {
    cleanup = Object.freeze({
      status: "required",
      reason: "upload_session_cleanup_unresolved",
    });
  }
  if (cleanup.status === "required") {
    return Object.freeze({
      verdict: "FAIL",
      failureCode: failureCode ?? "cleanup_required",
      cleanup,
      probeId,
      stages,
    });
  }
  if (failureCode) {
    return Object.freeze({
      verdict: "FAIL",
      failureCode,
      cleanup,
      stages,
    });
  }
  return Object.freeze({
    verdict: "PASS",
    driveType: appRoot.driveType,
    permissionProbe: "reported_scope_and_outside_root_denial",
    sameAccountNamespace: true,
    distinctAccessTokens: true,
    finalCommitCas: Object.freeze(
      Object.fromEntries(
        raceResults.map((result) => [result.winnerMode, "stale_final_rejected"]),
      ),
    ),
    cleanup,
    stages,
  });
}

function usage() {
  return [
    "Usage:",
    "  node onedrive-app-folder-poc.mjs live --credentials /absolute/path/to/onedrive-poc.json",
    "",
    "The credential file must have mode 0600 and two distinct delegated authorizations.",
  ].join("\n");
}

function parseArguments(argv) {
  if (argv.length === 1 && ["--help", "-h"].includes(argv[0])) {
    return { help: true };
  }
  if (
    argv.length !== 3 ||
    argv[0] !== "live" ||
    argv[1] !== "--credentials"
  ) {
    fail("invalid_arguments");
  }
  return { help: false, credentialFile: argv[2] };
}

export async function executeCli(argv = process.argv.slice(2)) {
  const argumentsValue = parseArguments(argv);
  if (argumentsValue.help) return { help: usage() };
  const credential = await readCredentialFile(argumentsValue.credentialFile);
  return runAppFolderProbe({ credential });
}

function isMainModule() {
  return Boolean(
    process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href,
  );
}

if (isMainModule()) {
  executeCli()
    .then((result) => {
      if (result.help) {
        process.stdout.write(`${result.help}\n`);
        return;
      }
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
      if (result.verdict !== "PASS") process.exitCode = 1;
    })
    .catch((error) => {
      process.stdout.write(
        `${JSON.stringify(
          { verdict: "FAIL", failureCode: safeErrorCode(error) },
          null,
          2,
        )}\n`,
      );
      process.exitCode = 1;
    });
}

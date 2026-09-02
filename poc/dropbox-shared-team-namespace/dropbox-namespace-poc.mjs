import { createHash, randomBytes } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const API_BASE_URL = "https://api.dropboxapi.com/2";
const CONTENT_BASE_URL = "https://content.dropboxapi.com/2";
const CREDENTIAL_FILE_MAX_BYTES = 64 * 1024;
const JSON_RESPONSE_MAX_BYTES = 1024 * 1024;
const PROBE_FILE_MAX_BYTES = 1024 * 1024;
const LIST_ENTRY_LIMIT = 1_000;
const LIST_PAGE_LIMIT = 100;
const LIST_REQUEST_LIMIT = 2;
const HTTP_ATTEMPTS = 3;
const HTTP_TIMEOUT_MS = 30_000;
const RETRY_BACKOFF_MAX_MS = 5_000;
const RETRY_AFTER_MAX_SECONDS = 60;
const DROPBOX_CONTENT_BLOCK_BYTES = 4 * 1024 * 1024;
const PROBE_FILE_NAME_PATTERN =
  /^mallard-dropbox-poc-([0-9a-f]{24})-([a-z0-9][a-z0-9.-]{0,254})$/u;
const NAMESPACE_ID_PATTERN = /^[-_0-9A-Za-z:]+$/u;
const ACCOUNT_ID_PATTERN = /^dbid:[-_0-9A-Za-z]+$/u;
const SELECTOR_PATTERN = /^dbmid:[-_0-9A-Za-z]+$/u;

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

class DropboxApiError extends Error {
  constructor(status, classification) {
    super("dropbox_api_failure");
    this.name = "DropboxApiError";
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

function validateToken(value) {
  return boundedString(value, "access_token", 8192);
}

function validateNamespaceId(value) {
  return boundedString(value, "namespace_id", 256, NAMESPACE_ID_PATTERN);
}

function validateActor(value, label) {
  exactKeys(
    value,
    ["access_token"],
    ["select_user", "select_admin"],
    `${label}_actor`,
  );
  if (value.select_user !== undefined && value.select_admin !== undefined) {
    fail(`invalid_${label}_actor_selector`);
  }
  return Object.freeze({
    accessToken: validateToken(value.access_token),
    selectUser:
      value.select_user === undefined
        ? undefined
        : boundedString(
            value.select_user,
            `${label}_select_user`,
            256,
            SELECTOR_PATTERN,
          ),
    selectAdmin:
      value.select_admin === undefined
        ? undefined
        : boundedString(
            value.select_admin,
            `${label}_select_admin`,
            256,
            SELECTOR_PATTERN,
          ),
  });
}

function actorIdentity(actor) {
  return `${actor.accessToken}\u0000${actor.selectUser ?? ""}\u0000${actor.selectAdmin ?? ""}`;
}

export function validateCredential(value) {
  exactKeys(
    value,
    ["namespace_id", "probe_root", "primary", "secondary"],
    [],
    "credential",
  );
  const credential = Object.freeze({
    namespaceId: validateNamespaceId(value.namespace_id),
    probeRoot: validateProbeRoot(value.probe_root),
    primary: validateActor(value.primary, "primary"),
    secondary: validateActor(value.secondary, "secondary"),
  });
  if (actorIdentity(credential.primary) === actorIdentity(credential.secondary)) {
    fail("actors_not_distinct");
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
    if (process.platform !== "win32" && (before.mode & 0o077) !== 0) {
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

export function dropboxContentHash(bytes) {
  const source = Buffer.from(bytes);
  const blockDigests = [];
  for (let offset = 0; offset < source.length; offset += DROPBOX_CONTENT_BLOCK_BYTES) {
    blockDigests.push(
      createHash("sha256")
        .update(source.subarray(offset, offset + DROPBOX_CONTENT_BLOCK_BYTES))
        .digest(),
    );
  }
  return createHash("sha256").update(Buffer.concat(blockDigests)).digest("hex");
}

function safeErrorCode(error) {
  if (error instanceof PocError || error instanceof AmbiguousTransportError) {
    return error.code;
  }
  if (error instanceof DropboxApiError) {
    return `dropbox_${error.classification}`;
  }
  return "unexpected_failure";
}

function validateProbeRoot(value) {
  if (
    typeof value !== "string" ||
    value.length < 2 ||
    value.length > 1024 ||
    !value.startsWith("/") ||
    value.endsWith("/") ||
    value.includes("//") ||
    value.includes("\\") ||
    /[\u0000-\u001f\u007f]/u.test(value) ||
    !/^[\x20-\x7e]+$/u.test(value)
  ) {
    fail("invalid_probe_root");
  }
  const components = value.slice(1).split("/");
  if (
    components.some(
      (component) =>
        component === "" ||
        component === "." ||
        component === ".." ||
        Buffer.byteLength(component, "utf8") > 255,
    )
  ) {
    fail("invalid_probe_root");
  }
  return value;
}

function validateProbePath(probeRoot, path) {
  const prefix = `${probeRoot.toLowerCase()}/`;
  const lowerPath = typeof path === "string" ? path.toLowerCase() : "";
  const fileName = lowerPath.startsWith(prefix) ? path.slice(prefix.length) : "";
  if (
    typeof path !== "string" ||
    !lowerPath.startsWith(prefix) ||
    !PROBE_FILE_NAME_PATTERN.test(fileName) ||
    Buffer.byteLength(fileName, "utf8") > 255 ||
    Buffer.byteLength(path, "utf8") > 2_000
  ) {
    fail("invalid_probe_path");
  }
  return path;
}

function metadataTag(value) {
  return isPlainObject(value) && typeof value[".tag"] === "string"
    ? value[".tag"]
    : undefined;
}

function validateRevision(value) {
  return boundedString(value, "dropbox_revision", 256, /^[0-9a-f]{9,}$/u);
}

function validateFileMetadata(value, expectedPath) {
  if (
    metadataTag(value) !== "file" ||
    value.path_lower !== expectedPath.toLowerCase() ||
    !Number.isSafeInteger(value.size) ||
    value.size < 0 ||
    typeof value.content_hash !== "string" ||
    !/^[0-9a-f]{64}$/u.test(value.content_hash)
  ) {
    fail("invalid_dropbox_file_metadata");
  }
  return Object.freeze({
    path: expectedPath,
    rev: validateRevision(value.rev),
    size: value.size,
    contentHash: value.content_hash,
  });
}

function classifyApiError(status, value) {
  const summary =
    isPlainObject(value) && typeof value.error_summary === "string"
      ? value.error_summary.toLowerCase()
      : "";
  const endpointError = isPlainObject(value) ? value.error : undefined;
  const reason = isPlainObject(endpointError) ? endpointError.reason : undefined;
  const conflict = isPlainObject(reason) ? reason.conflict : undefined;
  if (
    status === 409 &&
    metadataTag(endpointError) === "path" &&
    metadataTag(reason) === "conflict" &&
    metadataTag(conflict) === "file"
  ) {
    return "file_conflict";
  }
  if (status === 401) return "unauthorized";
  if (status === 403) return "forbidden";
  if (status === 429) return "rate_limited";
  if (summary.includes("not_found")) return "not_found";
  if (summary.includes("conflict") || summary.includes("parent_rev")) {
    return "conflict";
  }
  if (summary.includes("no_permission")) return "no_permission";
  if (summary.includes("invalid_root")) return "invalid_root";
  if (status >= 500) return "service_failure";
  return "request_rejected";
}

async function readBytesBounded(response, maximum) {
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    const parsed = Number(contentLength);
    if (!Number.isSafeInteger(parsed) || parsed < 0 || parsed > maximum) {
      fail("dropbox_response_too_large");
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
      fail("dropbox_response_too_large");
    }
    chunks.push(Buffer.from(value));
  }
  return Buffer.concat(chunks, total);
}

async function readJsonBounded(response) {
  const bytes = await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES);
  if (bytes.length === 0) fail("dropbox_empty_json_response");
  try {
    const value = JSON.parse(bytes.toString("utf8"));
    if (!isPlainObject(value)) fail("invalid_dropbox_json_response");
    return value;
  } catch (error) {
    if (error instanceof PocError) throw error;
    fail("invalid_dropbox_json_response");
  }
}

function retryDelayMs(response, attempt) {
  const retryAfter = response.headers.get("retry-after");
  if (retryAfter !== null && /^\d+$/u.test(retryAfter)) {
    const seconds = Number(retryAfter);
    if (!Number.isSafeInteger(seconds) || seconds > RETRY_AFTER_MAX_SECONDS) {
      fail("dropbox_retry_after_exceeds_probe_limit");
    }
    return seconds * 1000;
  }
  return Math.min(250 * 2 ** attempt, RETRY_BACKOFF_MAX_MS);
}

function assertExpectedContent(metadata, bytes) {
  if (
    metadata.size !== bytes.length ||
    metadata.contentHash !== dropboxContentHash(bytes)
  ) {
    fail("dropbox_content_metadata_mismatch");
  }
}

function relativeProbePath(probeRoot, path) {
  const prefix = `${probeRoot.toLowerCase()}/`;
  const lower = path.toLowerCase();
  if (!lower.startsWith(prefix) || lower.length <= prefix.length) {
    fail("dropbox_list_path_escaped_probe");
  }
  const relative = lower.slice(prefix.length);
  if (relative.includes("/")) fail("dropbox_list_nested_entry");
  return relative;
}

function lexicalCompare(left, right) {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

export class DropboxNamespaceClient {
  constructor({
    namespaceId,
    probeRoot,
    actor,
    fetchImpl = globalThis.fetch,
    sleepImpl = (milliseconds) =>
      new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)),
    timeoutMs = HTTP_TIMEOUT_MS,
  }) {
    this.namespaceId = validateNamespaceId(namespaceId);
    this.probeRoot = validateProbeRoot(probeRoot);
    this.actor = actor;
    this.fetchImpl = fetchImpl;
    this.sleepImpl = sleepImpl;
    this.timeoutMs = timeoutMs;
  }

  headers({ pathRoot = true, contentType } = {}) {
    const headers = {
      Authorization: `Bearer ${this.actor.accessToken}`,
    };
    if (pathRoot) {
      headers["Dropbox-API-Path-Root"] = JSON.stringify({
        ".tag": "namespace_id",
        namespace_id: this.namespaceId,
      });
    }
    if (this.actor.selectUser) {
      headers["Dropbox-API-Select-User"] = this.actor.selectUser;
    }
    if (this.actor.selectAdmin) {
      headers["Dropbox-API-Select-Admin"] = this.actor.selectAdmin;
    }
    if (contentType) headers["Content-Type"] = contentType;
    return headers;
  }

  async withResponse(url, options, handler) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      let response;
      try {
        response = await this.fetchImpl(url, {
          ...options,
          signal: controller.signal,
        });
      } catch {
        throw new AmbiguousTransportError("dropbox_transport_ambiguous");
      }
      try {
        return await handler(response);
      } catch (error) {
        if (
          error instanceof PocError ||
          error instanceof DropboxApiError ||
          error instanceof AmbiguousTransportError
        ) {
          throw error;
        }
        throw new AmbiguousTransportError("dropbox_transport_ambiguous");
      }
    } finally {
      clearTimeout(timer);
      controller.abort();
    }
  }

  async rpc(endpoint, body, { pathRoot = true, readOnly = false } = {}) {
    const url = `${API_BASE_URL}/${endpoint}`;
    for (let attempt = 0; attempt < HTTP_ATTEMPTS; attempt += 1) {
      let outcome;
      try {
        outcome = await this.withResponse(
          url,
          {
            method: "POST",
            headers: this.headers({
              pathRoot,
              contentType: "application/json",
            }),
            body: JSON.stringify(body),
          },
          async (response) => {
            if (response.ok) {
              return { kind: "success", value: await readJsonBounded(response) };
            }
            const shouldRetry =
              readOnly &&
              attempt + 1 < HTTP_ATTEMPTS &&
              (response.status === 429 || response.status >= 500);
            if (shouldRetry) {
              const delay = retryDelayMs(response, attempt);
              await readBytesBounded(response, JSON_RESPONSE_MAX_BYTES).catch(
                () => {},
              );
              return { kind: "retry", delay };
            }
            let value = {};
            try {
              value = await readJsonBounded(response);
            } catch {
              // The status code still maps to a sanitized API failure.
            }
            return {
              kind: "failure",
              status: response.status,
              classification: classifyApiError(response.status, value),
            };
          },
        );
      } catch (error) {
        if (
          readOnly &&
          attempt + 1 < HTTP_ATTEMPTS &&
          error instanceof AmbiguousTransportError
        ) {
          await this.sleepImpl(
            Math.min(250 * 2 ** attempt, RETRY_BACKOFF_MAX_MS),
          );
          continue;
        }
        throw error;
      }
      if (outcome.kind === "success") return outcome.value;
      if (outcome.kind === "retry") {
        await this.sleepImpl(outcome.delay);
        continue;
      }
      throw new DropboxApiError(
        outcome.status,
        outcome.classification,
      );
    }
    fail("dropbox_retry_exhausted");
  }

  async currentAccount() {
    const value = await this.rpc("users/get_current_account", null, {
      pathRoot: false,
      readOnly: true,
    });
    const rootInfo = value.root_info;
    if (
      !isPlainObject(rootInfo) ||
      !["team", "user"].includes(rootInfo[".tag"]) ||
      typeof rootInfo.root_namespace_id !== "string" ||
      typeof rootInfo.home_namespace_id !== "string"
    ) {
      fail("invalid_dropbox_account_response");
    }
    return Object.freeze({
      accountId: boundedString(
        value.account_id,
        "dropbox_account_id",
        256,
        ACCOUNT_ID_PATTERN,
      ),
      rootKind: rootInfo[".tag"],
      rootNamespaceId: validateNamespaceId(rootInfo.root_namespace_id),
      homeNamespaceId: validateNamespaceId(rootInfo.home_namespace_id),
    });
  }

  async probeNamespaceAccess() {
    const value = await this.rpc(
      "files/list_folder",
      {
        path: this.probeRoot,
        recursive: false,
        include_deleted: false,
        include_non_downloadable_files: true,
        limit: 1,
      },
      { readOnly: true },
    );
    if (!Array.isArray(value.entries) || typeof value.has_more !== "boolean") {
      fail("invalid_dropbox_list_response");
    }
  }

  async getMetadata(path) {
    validateProbePath(this.probeRoot, path);
    let value;
    try {
      value = await this.rpc(
        "files/get_metadata",
        {
          path,
          include_deleted: false,
          include_media_info: false,
          include_has_explicit_shared_members: false,
        },
        { readOnly: true },
      );
    } catch (error) {
      if (
        error instanceof DropboxApiError &&
        error.classification === "not_found"
      ) {
        return null;
      }
      throw error;
    }
    return validateFileMetadata(value, path);
  }

  async upload(path, bytes, mode, { simulateLostResponse = false } = {}) {
    validateProbePath(this.probeRoot, path);
    const body = Buffer.from(bytes);
    if (body.length > PROBE_FILE_MAX_BYTES) fail("probe_file_too_large");
    let modeValue;
    if (mode === "add") {
      modeValue = "add";
    } else if (isPlainObject(mode) && typeof mode.update === "string") {
      modeValue = { ".tag": "update", update: validateRevision(mode.update) };
    } else {
      fail("invalid_upload_mode");
    }
    return this.withResponse(
      `${CONTENT_BASE_URL}/files/upload`,
      {
        method: "POST",
        headers: {
          ...this.headers({ contentType: "application/octet-stream" }),
          "Dropbox-API-Arg": JSON.stringify({
            path,
            mode: modeValue,
            autorename: false,
            mute: true,
            strict_conflict: true,
            content_hash: dropboxContentHash(body),
          }),
        },
        body,
      },
      async (response) => {
        if (!response.ok) {
          let value = {};
          try {
            value = await readJsonBounded(response);
          } catch {
            // The status still maps to a sanitized API failure.
          }
          throw new DropboxApiError(
            response.status,
            classifyApiError(response.status, value),
          );
        }
        if (simulateLostResponse) {
          await response.body?.cancel().catch(() => {});
          throw new AmbiguousTransportError("simulated_lost_upload_response");
        }
        const value = await readJsonBounded(response);
        const metadata = validateFileMetadata(value, path);
        assertExpectedContent(metadata, body);
        return metadata;
      },
    );
  }

  async uploadResolvingAmbiguous(path, bytes, mode, options = {}) {
    try {
      return Object.freeze({
        metadata: await this.upload(path, bytes, mode, options),
        recovered: false,
      });
    } catch (error) {
      if (!(error instanceof AmbiguousTransportError)) throw error;
      const metadata = await this.getMetadata(path);
      if (metadata === null) throw error;
      await this.verifyDownload(path, bytes, metadata.rev);
      return Object.freeze({ metadata, recovered: true });
    }
  }

  async download(path) {
    validateProbePath(this.probeRoot, path);
    return this.withResponse(
      `${CONTENT_BASE_URL}/files/download`,
      {
        method: "POST",
        headers: {
          ...this.headers(),
          "Dropbox-API-Arg": JSON.stringify({ path }),
        },
      },
      async (response) => {
        if (!response.ok) {
          let value = {};
          try {
            value = await readJsonBounded(response);
          } catch {
            // The status still maps to a sanitized API failure.
          }
          throw new DropboxApiError(
            response.status,
            classifyApiError(response.status, value),
          );
        }
        const metadataHeader = response.headers.get("dropbox-api-result");
        if (!metadataHeader || metadataHeader.length > JSON_RESPONSE_MAX_BYTES) {
          fail("invalid_dropbox_download_metadata");
        }
        let rawMetadata;
        try {
          rawMetadata = JSON.parse(metadataHeader);
        } catch {
          fail("invalid_dropbox_download_metadata");
        }
        const metadata = validateFileMetadata(rawMetadata, path);
        const bytes = await readBytesBounded(response, PROBE_FILE_MAX_BYTES);
        assertExpectedContent(metadata, bytes);
        return Object.freeze({ metadata, bytes, sha256: sha256Hex(bytes) });
      },
    );
  }

  async verifyDownload(path, expectedBytes, expectedRevision) {
    const expected = Buffer.from(expectedBytes);
    const downloaded = await this.download(path);
    if (
      downloaded.metadata.rev !== expectedRevision ||
      downloaded.bytes.length !== expected.length ||
      downloaded.sha256 !== sha256Hex(expected) ||
      !downloaded.bytes.equals(expected)
    ) {
      fail("dropbox_verified_read_mismatch");
    }
    return downloaded.metadata;
  }

  async deleteFile(path, revision) {
    validateProbePath(this.probeRoot, path);
    const value = await this.rpc("files/delete_v2", {
      path,
      parent_rev: validateRevision(revision),
    });
    if (!isPlainObject(value.metadata)) fail("invalid_dropbox_delete_response");
    return validateFileMetadata(value.metadata, path);
  }

  async listProbeFiles(probeId) {
    if (typeof probeId !== "string" || !/^[0-9a-f]{24}$/u.test(probeId)) {
      fail("invalid_probe_id");
    }
    const entries = [];
    const lowerPaths = new Set();
    const probePrefix = `mallard-dropbox-poc-${probeId}-`;
    let cursor;
    let pageCount = 0;
    let scannedEntries = 0;
    do {
      pageCount += 1;
      if (pageCount > LIST_PAGE_LIMIT) fail("dropbox_list_page_limit_exceeded");
      const value =
        cursor === undefined
          ? await this.rpc(
              "files/list_folder",
              {
                path: this.probeRoot,
                recursive: false,
                include_deleted: false,
                include_non_downloadable_files: true,
                limit: LIST_REQUEST_LIMIT,
              },
              { readOnly: true },
            )
          : await this.rpc(
              "files/list_folder/continue",
              { cursor },
              { readOnly: true },
            );
      if (
        !Array.isArray(value.entries) ||
        typeof value.has_more !== "boolean" ||
        (value.has_more && typeof value.cursor !== "string")
      ) {
        fail("invalid_dropbox_list_response");
      }
      for (const raw of value.entries) {
        scannedEntries += 1;
        if (scannedEntries > LIST_ENTRY_LIMIT) {
          fail("dropbox_list_entry_limit_exceeded");
        }
        if (!isPlainObject(raw) || typeof raw.path_lower !== "string") {
          fail("invalid_dropbox_list_entry");
        }
        const relative = relativeProbePath(this.probeRoot, raw.path_lower);
        if (!relative.startsWith(probePrefix)) continue;
        const match = PROBE_FILE_NAME_PATTERN.exec(relative);
        if (!match || match[1] !== probeId) fail("invalid_dropbox_list_entry");
        if (lowerPaths.has(raw.path_lower)) fail("dropbox_case_alias_detected");
        lowerPaths.add(raw.path_lower);
        if (metadataTag(raw) === "file") {
          entries.push({
            kind: "file",
            relative,
            path: raw.path_lower,
            metadata: validateFileMetadata(raw, raw.path_lower),
          });
        } else if (metadataTag(raw) === "folder") {
          entries.push({ kind: "folder", relative, path: raw.path_lower });
        } else {
          fail("invalid_dropbox_list_entry");
        }
      }
      cursor = value.has_more ? value.cursor : undefined;
    } while (cursor !== undefined);
    entries.sort((left, right) =>
      lexicalCompare(left.relative, right.relative),
    );
    return Object.freeze({ entries, pageCount });
  }
}

async function expectConflict(operation, failureCode) {
  try {
    await operation();
  } catch (error) {
    if (
      error instanceof DropboxApiError &&
      error.status === 409 &&
      error.classification === "file_conflict"
    ) {
      return;
    }
    throw error;
  }
  fail(failureCode);
}

function classifyNamespace(namespaceId, primaryAccount, secondaryAccount) {
  if (primaryAccount.accountId === secondaryAccount.accountId) {
    fail("dropbox_accounts_not_distinct");
  }
  if (
    namespaceId === primaryAccount.homeNamespaceId ||
    namespaceId === secondaryAccount.homeNamespaceId
  ) {
    fail("personal_home_namespace_not_allowed");
  }
  const sharedTeamRoot =
    primaryAccount.rootKind === "team" &&
    secondaryAccount.rootKind === "team" &&
    primaryAccount.rootNamespaceId === namespaceId &&
    secondaryAccount.rootNamespaceId === namespaceId;
  return sharedTeamRoot ? "team_space" : "collaborative_non_home_namespace";
}

function makeProbeId(randomBytesImpl) {
  const value = Buffer.from(randomBytesImpl(12)).toString("hex");
  if (!/^[0-9a-f]{24}$/u.test(value)) fail("invalid_probe_randomness");
  return value;
}

function filePath(probeRoot, probeId, suffix) {
  return validateProbePath(
    probeRoot,
    `${probeRoot}/mallard-dropbox-poc-${probeId}-${suffix}`,
  );
}

function rememberArtifact(knownArtifacts, path, bytes) {
  const lower = path.toLowerCase();
  let artifact = knownArtifacts.get(lower);
  if (!artifact) {
    artifact = { hashes: new Set(), revisions: new Set() };
    knownArtifacts.set(lower, artifact);
  }
  artifact.hashes.add(sha256Hex(bytes));
  return path;
}

function rememberRevision(knownArtifacts, path, revision) {
  const artifact = knownArtifacts.get(path.toLowerCase());
  if (!artifact) fail("unknown_probe_artifact");
  artifact.revisions.add(validateRevision(revision));
}

async function cleanupProbe({
  client,
  probeId,
  knownArtifacts,
  ownsPrefix,
}) {
  if (!ownsPrefix) return Object.freeze({ status: "not_needed" });
  try {
    const listing = await client.listProbeFiles(probeId);
    for (const entry of listing.entries) {
      if (entry.kind !== "file" || !knownArtifacts.has(entry.path)) {
        return Object.freeze({
          status: "required",
          reason: "unexpected_probe_entry",
        });
      }
    }
    for (const entry of listing.entries) {
      let downloaded;
      try {
        downloaded = await client.download(entry.path);
      } catch (error) {
        if (
          error instanceof DropboxApiError &&
          error.classification === "not_found"
        ) {
          continue;
        }
        throw error;
      }
      const artifact = knownArtifacts.get(entry.path);
      if (
        !artifact.hashes.has(downloaded.sha256) ||
        !artifact.revisions.has(downloaded.metadata.rev)
      ) {
        return Object.freeze({
          status: "required",
          reason: "probe_artifact_changed",
        });
      }
      await client.deleteFile(entry.path, downloaded.metadata.rev);
    }
    const empty = await client.listProbeFiles(probeId);
    if (empty.entries.length !== 0) {
      return Object.freeze({
        status: "required",
        reason: "probe_files_remain",
      });
    }
    return Object.freeze({ status: "complete" });
  } catch (error) {
    return Object.freeze({
      status: "required",
      reason: safeErrorCode(error),
    });
  }
}

export async function runNamespaceProbe({
  credential,
  fetchImpl = globalThis.fetch,
  sleepImpl,
  randomBytesImpl = randomBytes,
}) {
  const validated = validateCredential({
    namespace_id: credential.namespaceId,
    probe_root: credential.probeRoot,
    primary: {
      access_token: credential.primary.accessToken,
      ...(credential.primary.selectUser
        ? { select_user: credential.primary.selectUser }
        : {}),
      ...(credential.primary.selectAdmin
        ? { select_admin: credential.primary.selectAdmin }
        : {}),
    },
    secondary: {
      access_token: credential.secondary.accessToken,
      ...(credential.secondary.selectUser
        ? { select_user: credential.secondary.selectUser }
        : {}),
      ...(credential.secondary.selectAdmin
        ? { select_admin: credential.secondary.selectAdmin }
        : {}),
    },
  });
  const primary = new DropboxNamespaceClient({
    namespaceId: validated.namespaceId,
    probeRoot: validated.probeRoot,
    actor: validated.primary,
    fetchImpl,
    ...(sleepImpl ? { sleepImpl } : {}),
  });
  const secondary = new DropboxNamespaceClient({
    namespaceId: validated.namespaceId,
    probeRoot: validated.probeRoot,
    actor: validated.secondary,
    fetchImpl,
    ...(sleepImpl ? { sleepImpl } : {}),
  });
  const probeId = makeProbeId(randomBytesImpl);
  const knownArtifacts = new Map();
  const stages = [];
  let ownsPrefix = false;
  let namespaceMode;
  let pageCount = 0;
  let failureCode;

  try {
    const [primaryAccount, secondaryAccount] = await Promise.all([
      primary.currentAccount(),
      secondary.currentAccount(),
    ]);
    namespaceMode = classifyNamespace(
      validated.namespaceId,
      primaryAccount,
      secondaryAccount,
    );
    await Promise.all([
      primary.probeNamespaceAccess(),
      secondary.probeNamespaceAccess(),
    ]);
    stages.push("two_actor_namespace_access");

    const [primaryInitial, secondaryInitial] = await Promise.all([
      primary.listProbeFiles(probeId),
      secondary.listProbeFiles(probeId),
    ]);
    if (
      primaryInitial.entries.length !== 0 ||
      secondaryInitial.entries.length !== 0
    ) {
      fail("probe_prefix_not_empty");
    }
    ownsPrefix = true;
    stages.push("isolated_probe_prefix");

    const firstBytes = Buffer.from("mallard-dropbox-poc-v1", "utf8");
    const headPath = rememberArtifact(
      knownArtifacts,
      filePath(validated.probeRoot, probeId, "head.bin"),
      firstBytes,
    );
    const first = await primary.upload(headPath, firstBytes, "add");
    rememberRevision(knownArtifacts, headPath, first.rev);
    await secondary.verifyDownload(headPath, firstBytes, first.rev);
    stages.push("absent_create_and_cross_actor_read");

    const secondBytes = Buffer.from("mallard-dropbox-poc-v2", "utf8");
    rememberArtifact(knownArtifacts, headPath, secondBytes);
    const second = await secondary.upload(
      headPath,
      secondBytes,
      { update: first.rev },
    );
    rememberRevision(knownArtifacts, headPath, second.rev);
    const staleBytes = Buffer.from("stale-writer", "utf8");
    rememberArtifact(knownArtifacts, headPath, staleBytes);
    await expectConflict(
      async () => {
        const metadata = await primary.upload(headPath, staleBytes, {
          update: first.rev,
        });
        rememberRevision(knownArtifacts, headPath, metadata.rev);
      },
      "stale_revision_was_accepted",
    );
    await primary.verifyDownload(headPath, secondBytes, second.rev);
    stages.push("current_revision_and_stale_rejection");

    const [primaryObserved, secondaryObserved] = await Promise.all([
      primary.getMetadata(headPath),
      secondary.getMetadata(headPath),
    ]);
    if (
      primaryObserved === null ||
      secondaryObserved === null ||
      primaryObserved.rev !== second.rev ||
      secondaryObserved.rev !== second.rev
    ) {
      fail("late_race_observation_mismatch");
    }
    const thirdBytes = Buffer.from("mallard-dropbox-poc-v3", "utf8");
    rememberArtifact(knownArtifacts, headPath, thirdBytes);
    const third = await primary.upload(
      headPath,
      thirdBytes,
      { update: primaryObserved.rev },
    );
    rememberRevision(knownArtifacts, headPath, third.rev);
    await expectConflict(
      async () => {
        const metadata = await secondary.upload(
          headPath,
          thirdBytes,
          { update: secondaryObserved.rev },
        );
        rememberRevision(knownArtifacts, headPath, metadata.rev);
      },
      "late_stale_identical_revision_was_accepted",
    );
    await expectConflict(
      async () => {
        const metadata = await secondary.upload(headPath, thirdBytes, "add");
        rememberRevision(knownArtifacts, headPath, metadata.rev);
      },
      "strict_add_conflict_was_accepted",
    );
    await secondary.verifyDownload(headPath, thirdBytes, third.rev);
    stages.push("late_stale_identical_and_add_conflicts");

    await primary.deleteFile(headPath, third.rev);
    if ((await secondary.getMetadata(headPath)) !== null) {
      fail("revision_delete_did_not_remove_file");
    }
    const fourthBytes = Buffer.from("mallard-dropbox-poc-v4", "utf8");
    rememberArtifact(knownArtifacts, headPath, fourthBytes);
    const fourth = await secondary.upload(headPath, fourthBytes, "add");
    rememberRevision(knownArtifacts, headPath, fourth.rev);
    await expectConflict(
      async () => {
        const metadata = await primary.upload(headPath, fourthBytes, {
          update: third.rev,
        });
        rememberRevision(knownArtifacts, headPath, metadata.rev);
      },
      "delete_recreate_stale_revision_was_accepted",
    );
    await primary.verifyDownload(headPath, fourthBytes, fourth.rev);
    stages.push("revision_delete_and_delete_recreate_conflict");

    const ambiguousBytes = Buffer.from("committed-with-lost-response", "utf8");
    const ambiguousPath = rememberArtifact(
      knownArtifacts,
      filePath(validated.probeRoot, probeId, "ambiguous.bin"),
      ambiguousBytes,
    );
    const ambiguous = await primary.uploadResolvingAmbiguous(
      ambiguousPath,
      ambiguousBytes,
      "add",
      { simulateLostResponse: true },
    );
    if (!ambiguous.recovered) fail("ambiguous_upload_not_recovered");
    rememberRevision(
      knownArtifacts,
      ambiguousPath,
      ambiguous.metadata.rev,
    );
    await secondary.verifyDownload(
      ambiguousPath,
      ambiguousBytes,
      ambiguous.metadata.rev,
    );
    stages.push("ambiguous_commit_readback");

    const zeroBytes = Buffer.alloc(0);
    const zeroPath = rememberArtifact(
      knownArtifacts,
      filePath(validated.probeRoot, probeId, "zero.bin"),
      zeroBytes,
    );
    const zero = await secondary.upload(zeroPath, zeroBytes, "add");
    rememberRevision(knownArtifacts, zeroPath, zero.rev);
    await primary.verifyDownload(zeroPath, zeroBytes, zero.rev);

    for (let index = 0; index < 6; index += 1) {
      const bytes = Buffer.from(`page-${index}`, "utf8");
      const path = rememberArtifact(
        knownArtifacts,
        filePath(
          validated.probeRoot,
          probeId,
          `page-${String(index).padStart(2, "0")}.bin`,
        ),
        bytes,
      );
      const actor = index % 2 === 0 ? primary : secondary;
      const metadata = await actor.upload(path, bytes, "add");
      rememberRevision(knownArtifacts, path, metadata.rev);
    }
    const filePrefix = `mallard-dropbox-poc-${probeId}-`;
    const maxNameFill =
      255 - Buffer.byteLength(filePrefix) - "limit-".length - ".bin".length;
    const maxNameBytes = Buffer.from("max-component", "utf8");
    const maxNamePath = rememberArtifact(
      knownArtifacts,
      filePath(
        validated.probeRoot,
        probeId,
        `limit-${"a".repeat(maxNameFill)}.bin`,
      ),
      maxNameBytes,
    );
    const maxName = await primary.upload(maxNamePath, maxNameBytes, "add");
    rememberRevision(knownArtifacts, maxNamePath, maxName.rev);

    const [primaryList, secondaryList] = await Promise.all([
      primary.listProbeFiles(probeId),
      secondary.listProbeFiles(probeId),
    ]);
    pageCount = Math.min(primaryList.pageCount, secondaryList.pageCount);
    const primaryPaths = primaryList.entries.map((entry) => entry.path);
    const secondaryPaths = secondaryList.entries.map((entry) => entry.path);
    const expectedPaths = [...knownArtifacts.keys()].sort(lexicalCompare);
    if (
      pageCount < 2 ||
      JSON.stringify(primaryPaths) !== JSON.stringify(expectedPaths) ||
      JSON.stringify(secondaryPaths) !== JSON.stringify(expectedPaths)
    ) {
      fail("dropbox_sorted_pagination_mismatch");
    }
    stages.push("zero_byte_max_component_sorted_pagination");
  } catch (error) {
    failureCode = safeErrorCode(error);
  }

  const cleanup = await cleanupProbe({
    client: primary,
    probeId,
    knownArtifacts,
    ownsPrefix,
  });
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
    verdict: "POC_PASS",
    productionGate: "INCOMPLETE",
    namespaceMode,
    distinctAccounts: true,
    pageCount,
    objectCount: knownArtifacts.size,
    cleanup,
    stages,
  });
}

function usage() {
  return [
    "Usage:",
    "  node dropbox-namespace-poc.mjs live --credentials /absolute/path/to/dropbox-poc.json",
    "",
    "The credential file must have mode 0600 and two distinct Dropbox actors.",
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
  return runNamespaceProbe({ credential });
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
      if (result.verdict !== "POC_PASS") process.exitCode = 1;
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

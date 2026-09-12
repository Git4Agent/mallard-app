// Standalone protocol POC. The application does not import this code.
export const MARKER_KEY = ".mallard/_storage.json";
export const MAX_MARKER_BYTES = 4 * 1024;
export const MAX_RESPONSE_BYTES = 1024 * 1024;

export class PocError extends Error {
  constructor(code, status) {
    super(code);
    this.code = code;
    this.status = status;
  }
}

export function requireValue(condition, code) {
  if (!condition) throw new PocError(code);
}

export function credential(value) {
  requireValue(typeof value === "string" && /^[\x21-\x7e]{1,16384}$/u.test(value), "credential_required");
  return value;
}

export function json(bytes) {
  try {
    const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    requireValue(value !== null && typeof value === "object" && !Array.isArray(value), "invalid_response");
    return value;
  } catch {
    throw new PocError("invalid_response");
  }
}

async function readBounded(response, maxBytes) {
  const length = response.headers.get("content-length");
  if (length !== null && Number(length) > maxBytes) {
    await response.body?.cancel();
    throw new PocError("response_too_large");
  }
  if (!response.body) return Buffer.alloc(0);
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > maxBytes) {
        await reader.cancel();
        throw new PocError("response_too_large");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  return Buffer.concat(chunks, size);
}

export async function request(url, init, {
  fetchImpl = fetch,
  signal,
  timeoutMs = 15_000,
  maxBytes = MAX_RESPONSE_BYTES,
} = {}) {
  const timeout = AbortSignal.timeout(timeoutMs);
  const combined = signal ? AbortSignal.any([signal, timeout]) : timeout;
  try {
    const response = await fetchImpl(url, { ...init, redirect: "error", signal: combined });
    // Error bodies have their own small bound. Never return them to the CLI.
    let bytes;
    try { bytes = await readBounded(response, response.ok ? maxBytes : 16 * 1024); }
    catch (error) {
      if (!response.ok && error instanceof PocError && error.code === "response_too_large") {
        throw new PocError("error_response_too_large", response.status);
      }
      throw error;
    }
    return { status: response.status, ok: response.ok, bytes };
  } catch (error) {
    if (error instanceof PocError) throw error;
    throw new PocError(combined.aborted ? "request_timeout" : "transport_failed");
  }
}

export function responseError(response) {
  let code = response.status === 401 ? "authorization_failed"
    : response.status === 403 ? "access_denied"
    : response.status === 429 ? "rate_limited" : "request_rejected";
  // Inspect only the structured Dropbox error tag, never its free text.
  try {
    const error = json(response.bytes).error;
    if (error?.[".tag"] === "missing_scope") code = "missing_scope";
    if (error?.[".tag"] === "expired_access_token") code = "token_expired";
  } catch { /* A non-JSON error still has a safe HTTP classification. */ }
  return new PocError(code, response.status);
}

export function markerState(bytes) {
  if (bytes.byteLength > MAX_MARKER_BYTES) return "invalid_marker";
  let marker;
  try { marker = json(bytes); }
  catch { return "invalid_marker"; }
  if (marker.format !== "mallard-storage"
    || !Number.isInteger(marker.layout_version)
    || marker.layout_version < 0 || marker.layout_version > 0xffffffff) return "invalid_marker";
  return marker.layout_version === 1 ? "mallard_storage" : "unsupported_layout";
}

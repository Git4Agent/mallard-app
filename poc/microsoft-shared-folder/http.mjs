// All errors exposed by the CLI are fixed codes, never remote response text.
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
  } catch { throw new PocError("invalid_response"); }
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
  } finally { reader.releaseLock(); }
  return Buffer.concat(chunks, size);
}

export async function request(url, init, {
  fetchImpl = fetch, timeoutMs = 15_000, maxBytes = 1024 * 1024,
} = {}) {
  const signal = AbortSignal.timeout(timeoutMs);
  try {
    const response = await fetchImpl(url, { ...init, redirect: "error", signal });
    // Consume the body under the same deadline as the headers. Never retry.
    return { status: response.status, ok: response.ok,
      bytes: await readBounded(response, response.ok ? maxBytes : 16 * 1024) };
  } catch (error) {
    if (error instanceof PocError) throw error;
    throw new PocError(signal.aborted ? "request_timeout" : "transport_failed");
  }
}

export function responseError(response) {
  const code = { 401: "authorization_failed", 403: "access_denied", 404: "not_found",
    409: "conflict", 412: "precondition_failed", 429: "rate_limited" }[response.status]
    ?? "request_rejected";
  return new PocError(code, response.status);
}

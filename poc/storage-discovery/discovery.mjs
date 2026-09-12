import {
  MARKER_KEY, MAX_MARKER_BYTES, PocError, credential, json,
  markerState, request, requireValue, responseError,
} from "./http.mjs";

export const JURISDICTIONS = Object.freeze(["default", "eu", "us", "fedramp"]);
const MAX_PAGES = 100;
const MAX_CANDIDATES = 1000;
const PAGE_SIZE = 100;
const CF_API = "https://api.cloudflare.com/client/v4";
const DBX_API = "https://api.dropboxapi.com/2";
const DBX_CONTENT = "https://content.dropboxapi.com/2";

export function accountId(value) {
  requireValue(typeof value === "string" && /^[a-fA-F0-9]{32}$/u.test(value), "invalid_account_id");
  return value.toLowerCase();
}

function namespaceId(value) {
  requireValue(typeof value === "string" && /^[-_a-zA-Z0-9:]{1,256}$/u.test(value), "invalid_namespace_id");
  return value;
}

function cursorAfter(value, seen) {
  if (value === undefined || value === null || value === "") return undefined;
  requireValue(typeof value === "string" && value.length <= 16 * 1024, "invalid_cursor");
  requireValue(!seen.has(value), "repeated_cursor");
  seen.add(value);
  return value;
}

function scanContext(provider, options) {
  const maxPages = options.maxPages ?? MAX_PAGES;
  const maxCandidates = options.maxCandidates ?? MAX_CANDIDATES;
  requireValue(Number.isInteger(maxPages) && maxPages > 0 && maxPages <= MAX_PAGES, "invalid_scan_limit");
  requireValue(Number.isInteger(maxCandidates) && maxCandidates > 0 && maxCandidates <= MAX_CANDIDATES, "invalid_scan_limit");
  const deadline = AbortSignal.timeout(120_000);
  return {
    result: { provider, enumerationComplete: true, candidates: [], issues: [] },
    maxPages, maxCandidates,
    http: { fetchImpl: options.fetchImpl, signal: options.signal
      ? AbortSignal.any([options.signal, deadline]) : deadline, timeoutMs: options.timeoutMs },
  };
}

function issue(context, error) {
  context.result.enumerationComplete = false;
  context.result.issues.push({ code: error instanceof PocError ? error.code : "discovery_failed",
    ...(error instanceof PocError && error.status ? { status: error.status } : {}) });
}

function addCandidate(context, seen, key, candidate) {
  if (seen.has(key)) return;
  requireValue(context.result.candidates.length < context.maxCandidates, "candidate_limit_reached");
  seen.add(key);
  context.result.candidates.push({ ...candidate, state: "not_checked" });
}

async function checkCandidates(context, inspect) {
  // Four independent marker reads at a time. No file-tree scans or object lists.
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(4, context.result.candidates.length) }, async () => {
    while (next < context.result.candidates.length) {
      const candidate = context.result.candidates[next++];
      if (context.http.signal.aborted) {
        candidate.state = "unreadable";
        candidate.error = "request_timeout";
        continue;
      }
      try { candidate.state = await inspect(candidate); }
      catch (error) {
        candidate.state = error instanceof PocError && error.code === "response_too_large"
          ? "invalid_marker" : "unreadable";
        candidate.error = error instanceof PocError ? error.code : "discovery_failed";
      }
    }
  }));
  return context.result;
}

export async function discoverR2({ token, accountId: account, jurisdiction = "default", ...options }) {
  const id = accountId(account);
  credential(token);
  requireValue(JURISDICTIONS.includes(jurisdiction), "invalid_jurisdiction");
  const context = scanContext("r2", options);
  context.result.jurisdiction = jurisdiction;
  const headers = { Authorization: `Bearer ${token}`, "cf-r2-jurisdiction": jurisdiction };
  const base = `${CF_API}/accounts/${id}/r2/buckets`;
  const seen = new Set();
  const cursors = new Set();
  let cursor;
  try {
    for (let page = 0; ; page++) {
      requireValue(page < context.maxPages, "page_limit_reached");
      const url = new URL(base);
      url.searchParams.set("per_page", String(PAGE_SIZE));
      if (cursor) url.searchParams.set("cursor", cursor);
      const response = await request(url, { method: "GET", headers }, context.http);
      if (!response.ok) throw responseError(response);
      const data = json(response.bytes);
      requireValue(data.success === true && Array.isArray(data.result?.buckets)
        && data.result.buckets.length <= PAGE_SIZE, "invalid_bucket_list");
      for (const bucket of data.result.buckets) {
        requireValue(typeof bucket?.name === "string"
          && /^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$/u.test(bucket.name), "invalid_bucket_name");
        requireValue(bucket.jurisdiction === undefined || bucket.jurisdiction === jurisdiction,
          "jurisdiction_mismatch");
        const suffix = jurisdiction === "default" ? "" : `.${jurisdiction}`;
        addCandidate(context, seen, bucket.name, {
          name: bucket.name,
          destination: { account_id: id, bucket: bucket.name, jurisdiction,
            api_url: `https://${id}${suffix}.r2.cloudflarestorage.com/${bucket.name}` },
          // The current Rust adapter accepts default, EU, and FedRAMP.
          supportedByApp: jurisdiction !== "us",
        });
      }
      cursor = cursorAfter(data.result_info?.cursor, cursors);
      if (!cursor) break;
    }
  } catch (error) { issue(context, error); }
  return checkCandidates(context, async (candidate) => {
    // Cloudflare requires literal slashes within the object key.
    const response = await request(`${base}/${candidate.destination.bucket}/objects/${MARKER_KEY}`,
      { method: "GET", headers }, { ...context.http, maxBytes: MAX_MARKER_BYTES });
    if (response.status === 404) return "marker_missing";
    if (!response.ok) throw responseError(response);
    return markerState(response.bytes);
  });
}

export async function discoverDropbox({ token, ...options }) {
  credential(token);
  const context = scanContext("dropbox", options);
  const seen = new Set();
  const headers = { Authorization: `Bearer ${token}`, "Content-Type": "application/json" };
  const rpc = async (route, body) => {
    const response = await request(`${DBX_API}/${route}`, { method: "POST", headers,
      body: JSON.stringify(body) }, context.http);
    if (!response.ok) throw responseError(response);
    return json(response.bytes);
  };
  let home;
  try {
    const account = await rpc("users/get_current_account", null);
    const root = account.root_info;
    requireValue(root?.[".tag"] === "user" || root?.[".tag"] === "team", "invalid_account_root");
    home = namespaceId(root.home_namespace_id);
    namespaceId(root.root_namespace_id);
    if (root[".tag"] === "team" && root.root_namespace_id !== home) {
      addCandidate(context, seen, root.root_namespace_id, {
        name: "Team space root", kind: "team_root", access: "unknown", mounted: true,
        destination: { namespace_id: root.root_namespace_id }, supportedByApp: true,
      });
    }
  } catch (error) {
    issue(context, error);
    // The account root is required to exclude a personal home namespace.
    return context.result;
  }
  // Include shares that are not mounted. Dedupe by namespace, never display path.
  for (const route of ["sharing/list_folders", "sharing/list_mountable_folders"]) {
    const cursors = new Set();
    let cursor;
    try {
      for (let page = 0; ; page++) {
        requireValue(page < context.maxPages, "page_limit_reached");
        const data = await rpc(cursor ? `${route}/continue` : route,
          cursor ? { cursor } : { limit: PAGE_SIZE });
        requireValue(Array.isArray(data.entries) && data.entries.length <= PAGE_SIZE, "invalid_folder_list");
        for (const folder of data.entries) {
          const namespace = namespaceId(folder?.shared_folder_id);
          requireValue(namespace !== home, "personal_namespace_rejected");
          requireValue(typeof folder.name === "string" && Buffer.byteLength(folder.name) <= 1024,
            "invalid_folder_name");
          const access = folder.access_type?.[".tag"];
          addCandidate(context, seen, namespace, {
            name: folder.name, kind: "shared_folder",
            access: ["owner", "editor", "viewer", "viewer_no_comment"].includes(access) ? access : "unknown",
            mounted: typeof folder.path_lower === "string",
            destination: { namespace_id: namespace }, supportedByApp: true,
          });
        }
        cursor = cursorAfter(data.cursor, cursors);
        if (!cursor) break;
      }
    } catch (error) { issue(context, error); }
  }
  return checkCandidates(context, async (candidate) => {
    const response = await request(`${DBX_CONTENT}/files/download`, {
      method: "POST",
      headers: { Authorization: headers.Authorization,
        "Dropbox-API-Path-Root": JSON.stringify({ ".tag": "namespace_id", ...candidate.destination }),
        "Dropbox-API-Arg": JSON.stringify({ path: `/${MARKER_KEY}` }) },
    }, { ...context.http, maxBytes: MAX_MARKER_BYTES });
    if (response.status === 409) {
      const error = json(response.bytes).error;
      if (error?.[".tag"] === "path" && error.path?.[".tag"] === "not_found") return "marker_missing";
    }
    if (!response.ok) throw responseError(response);
    return markerState(response.bytes);
  });
}

// Names, destinations, credentials, response text, and paths stay in memory.
export function publicReport(result, mode) {
  const counts = { mallard_storage: 0, marker_missing: 0, invalid_marker: 0,
    unsupported_layout: 0, unreadable: 0, not_checked: 0 };
  for (const candidate of result.candidates) counts[candidate.state]++;
  const complete = result.enumerationComplete && counts.unreadable === 0 && counts.not_checked === 0;
  return {
    mode, provider: result.provider,
    ...(result.jurisdiction ? { jurisdiction: result.jurisdiction } : {}),
    verdict: complete ? "SCAN_COMPLETE" : "SCAN_PARTIAL",
    enumerationComplete: result.enumerationComplete,
    candidatesChecked: result.candidates.length - counts.not_checked,
    counts,
    discovered: result.candidates.filter((candidate) => candidate.state === "mallard_storage")
      .map((candidate, index) => ({ ref: `${result.provider}-${index + 1}`,
        supportedByApp: candidate.supportedByApp,
        ...(candidate.access ? { access: candidate.access, mounted: candidate.mounted } : {}) })),
    issues: result.issues,
    candidateErrors: [...new Set(result.candidates.map((candidate) => candidate.error).filter(Boolean))],
    remoteWrites: 0,
    localConfigWrites: 0,
  };
}

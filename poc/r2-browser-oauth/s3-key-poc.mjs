// Isolated permission check. No production code imports this module.
import { createHash, randomBytes } from "node:crypto";
import { credential, json, request, requireValue } from "../storage-discovery/http.mjs";

// Workers R2 Storage Bucket Item Read / Write, from Cloudflare's R2 API docs.
export const R2_OBJECT_READ = "6a018a9f2fc74eb6b293b0c548f38b39";
export const R2_OBJECT_WRITE = "2efd5506f9c8494dacb1fa10a3e7d5b6";

export function keyPolicy(accountId, bucket, now = Date.now()) {
  requireValue(/^[a-f0-9]{32}$/u.test(accountId), "invalid_account_id");
  requireValue(/^mallard-poc-oauth-[a-f0-9]{16}$/u.test(bucket), "invalid_poc_bucket");
  return {
    name: bucket,
    expires_on: new Date(Math.floor(now / 1000) * 1000 + 60 * 60 * 1000).toISOString(),
    policies: [{ effect: "allow",
      permission_groups: [{ id: R2_OBJECT_READ }, { id: R2_OBJECT_WRITE }],
      resources: { [`com.cloudflare.edge.r2.bucket.${accountId}_default_${bucket}`]: "*" },
    }],
  };
}

export function issuedS3Key(token, policy) {
  requireValue(token && /^[a-f0-9]{32}$/u.test(token.id), "invalid_issued_token");
  requireValue(token.status === "active" && token.policies?.length === 1,
    "invalid_issued_token_policy");
  const actual = token.policies[0];
  const expected = policy.policies[0];
  requireValue(actual.effect === "allow"
    && JSON.stringify(actual.resources) === JSON.stringify(expected.resources)
    && actual.permission_groups?.length === 2
    && actual.permission_groups.every((group) => [R2_OBJECT_READ, R2_OBJECT_WRITE].includes(group.id))
    && new Set(actual.permission_groups.map((group) => group.id)).size === 2
    && Date.parse(token.expires_on) === Date.parse(policy.expires_on), "invalid_issued_token_policy");
  // The API token returned by the create endpoint supplies these values.
  // An OAuth bearer is never passed here or converted into an S3 secret.
  return { accessKeyId: token.id,
    secretAccessKey: createHash("sha256").update(credential(token.value)).digest("hex") };
}

function errorCodes(value) {
  return Array.isArray(value.errors)
    ? value.errors.slice(0, 16).map((error) => error.code).filter(Number.isSafeInteger) : [];
}

export async function runS3KeyPoc({ accessToken, accountId, api, signal, catalogOnly = false }) {
  const report = { operation: catalogOnly ? "scope_catalog" : "issue_s3_key", oauth_token_received: true,
    s3_credentials_received: false, s3_probe_passed: false,
    token_create_requests: 0, bucket_create_requests: 0, remote_token_created: false };
  const headers = { Authorization: `Bearer ${accessToken}`, "Content-Type": "application/json" };
  const catalog = await request(`${api}/oauth/scopes`, { method: "GET", headers },
    { signal, maxBytes: 1024 * 1024 });
  report.scope_catalog_http_status = catalog.status;
  if (catalog.ok) {
    const value = json(catalog.bytes);
    requireValue(value.success === true && Array.isArray(value.result)
      && value.result.length <= 4096, "invalid_scope_catalog");
    const info = value.result_info;
    report.scope_catalog_count = value.result.length;
    report.scope_catalog_complete = !(info?.total_count > value.result.length || info?.total_pages > 1);
    // Scope IDs are public protocol data. Do not return account or token fields.
    const ids = value.result.filter((item) => typeof item.id === "string"
      && /^[A-Za-z0-9_.:-]{1,128}$/u.test(item.id));
    report.token_scope_ids = ids.filter((item) => /api.?token/iu.test(`${item.id} ${item.name}`))
      .map((item) => item.id).sort();
    report.r2_scope_ids = ids.filter((item) => /workers.?r2/iu.test(item.id)).map((item) => item.id).sort();
    report.documented_provisioning_scope_available = ids.some((item) => item.id === "account_api_tokens:create");
  }
  if (catalogOnly) return { ...report, status: catalog.ok ? "POC_PASSED" : "POC_BLOCKED" };

  // Use a new, absent bucket name. A failed permission check leaves no bucket.
  const bucket = `mallard-poc-oauth-${randomBytes(8).toString("hex")}`;
  const policy = keyPolicy(accountId, bucket);
  report.test_token_name = bucket;
  report.test_token_expires_on = policy.expires_on;
  report.token_create_requests = 1;
  let response;
  try {
    response = await request(`${api}/accounts/${accountId}/tokens`, {
      method: "POST", headers, body: JSON.stringify(policy),
    }, { signal, maxBytes: 64 * 1024 });
  } catch {
    // Do not send again or claim that the server made no change.
    return { ...report, status: "POC_STOPPED", code: "token_create_outcome_unknown",
      remote_token_created: "unknown" };
  }
  report.token_create_http_status = response.status;
  let body;
  try { body = json(response.bytes); }
  catch {
    return { ...report, status: "POC_STOPPED", code: "token_create_outcome_unknown",
      remote_token_created: "unknown" };
  }
  if (!response.ok || body.success !== true) {
    return { ...report, status: "POC_BLOCKED", code: "account_token_creation_rejected",
      provider_error_codes: errorCodes(body) };
  }
  report.remote_token_created = true;
  try {
    const key = issuedS3Key(body.result, policy);
    key.accessKeyId = "";
    key.secretAccessKey = "";
    return { ...report, status: "POC_PASSED", s3_credentials_received: true,
      issued_key_policy_verified: true };
  } catch {
    return { ...report, status: "POC_STOPPED", code: "invalid_issued_token_policy" };
  } finally {
    if (body.result) body.result.value = "";
    response.bytes.fill(0);
  }
}

// Loopback fixture only. These scope labels do not establish Cloudflare scope availability.
import { createHash, randomBytes } from "node:crypto";
import { createServer } from "node:http";
import { sendPage } from "./browser-oauth.mjs";

export async function startMockProvider({ tokenStatus = 200, bucketStatus = 200,
  accountCount = 1, tokenRedirect, tokenScopes, tokenBytes, bucketCursor,
  keyStatus = 403, keyMalformed = false } = {}) {
  const clientId = "mallard-poc-fixture";
  const scopes = ["fixture.account.read", "fixture.r2.read"];
  const access = "fixture-access-token-canary";
  let code = randomBytes(24).toString("hex");
  const calls = [];
  const keyPolicies = [];
  let pending;
  let used = false;
  let origin;
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, origin);
    calls.push(`${req.method} ${url.pathname}`);
    const reply = (status, value) => res.writeHead(status, { "Content-Type": "application/json" }).end(JSON.stringify(value));
    if (req.method === "GET" && url.pathname === "/authorize") {
      const params = url.searchParams;
      let redirect;
      try { redirect = new URL(params.get("redirect_uri")); } catch { return reply(400, {}); }
      if (params.get("client_id") !== clientId || params.get("code_challenge_method") !== "S256"
        || params.get("response_type") !== "code" || params.get("scope") !== scopes.join(" ")
        || redirect.protocol !== "http:" || redirect.hostname !== "127.0.0.1"
        || redirect.pathname !== "/r2-poc/callback") return reply(400, {});
      pending = { challenge: params.get("code_challenge"), redirect: redirect.toString(), state: params.get("state") };
      used = false;
      code = randomBytes(24).toString("hex");
      sendPage(res, 200, "Mock Cloudflare consent", `<p>This local fixture simulates read access to ${accountCount} account(s) and one R2 bucket.</p><p>No real account, bucket, or token is used.</p><a href="/allow">Allow mock read access</a> <a href="/deny">Cancel</a>`);
      return;
    }
    if (req.method === "GET" && ["/allow", "/deny"].includes(url.pathname) && pending) {
      const redirect = new URL(pending.redirect);
      redirect.search = new URLSearchParams({ state: pending.state,
        ...(url.pathname === "/allow" ? { code } : { error: "access_denied" }) }).toString();
      res.writeHead(302, { Location: redirect.toString(), "Cache-Control": "no-store" }).end();
      return;
    }
    if (req.method === "POST" && url.pathname === "/token") {
      const parts = [];
      let length = 0;
      for await (const chunk of req) {
        length += chunk.length;
        if (length > 8192) return reply(413, {});
        parts.push(chunk);
      }
      const form = new URLSearchParams(Buffer.concat(parts).toString());
      if (tokenRedirect) return res.writeHead(302, { Location: tokenRedirect }).end();
      if (tokenStatus !== 200) return reply(tokenStatus, { error: "fixture-provider-private-canary" });
      if (!pending || used || form.has("client_secret") || form.get("client_id") !== clientId
        || form.get("grant_type") !== "authorization_code" || form.get("code") !== code
        || form.get("redirect_uri") !== pending.redirect
        || createHash("sha256").update(form.get("code_verifier") ?? "").digest("base64url") !== pending.challenge) {
        return reply(400, { error: "invalid_grant" });
      }
      used = true;
      if (tokenBytes) return res.writeHead(200).end("x".repeat(tokenBytes));
      return reply(200, { token_type: "bearer", access_token: access, expires_in: 300,
        scope: tokenScopes ?? scopes.join(" ") });
    }
    if (req.headers.authorization !== `Bearer ${access}`) return reply(401, {});
    if (req.method === "GET" && url.pathname === "/client/v4/accounts") {
      return reply(200, { success: true,
        result: Array.from({ length: accountCount }, (_, index) => ({ id: String(index + 1).padStart(32, "0") })),
        result_info: { total_pages: 1 } });
    }
    if (req.method === "GET" && url.pathname === "/client/v4/oauth/scopes") {
      return reply(200, { success: true, result: [
        { id: "workers-r2.read", name: "Workers R2 Storage Read" },
        { id: "workers-r2.write", name: "Workers R2 Storage Write" },
        { id: "scim-provisioning.write", name: "SCIM Provisioning Write" },
      ], result_info: { total_count: 3 } });
    }
    if (req.method === "POST" && /^\/client\/v4\/accounts\/[a-f0-9]{32}\/tokens$/u.test(url.pathname)) {
      const parts = [];
      for await (const chunk of req) parts.push(chunk);
      const policy = JSON.parse(Buffer.concat(parts).toString());
      keyPolicies.push(policy);
      if (keyMalformed) return res.writeHead(200).end("not-json");
      if (keyStatus !== 200) return reply(keyStatus, { success: false,
        errors: [{ code: 10000, message: "fixture-provider-private-canary" }] });
      return reply(200, { success: true, result: { ...policy,
        id: "a".repeat(32), value: "fixture-issued-token-private-canary", status: "active" } });
    }
    if (req.method === "GET" && /^\/client\/v4\/accounts\/[a-f0-9]{32}\/r2\/buckets$/u.test(url.pathname)) {
      return reply(bucketStatus, { success: bucketStatus === 200,
        result: { buckets: [{ name: "mallard-poc-fixture", creation_date: "2026-01-01T00:00:00Z" }] },
        result_info: { cursor: bucketCursor } });
    }
    reply(404, {});
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  return { clientId, scopes, calls, keyPolicies,
    endpoints: { authorization: `${origin}/authorize`, token: `${origin}/token`, api: `${origin}/client/v4` },
    async close() {
      server.closeAllConnections();
      if (server.listening) await new Promise((resolve) => server.close(resolve));
    },
  };
}

import assert from "node:assert/strict";
import test from "node:test";
import { request } from "node:http";
import { startBrowserProbe } from "./browser-probe.mjs";

async function fixture(t, run) {
  const probe = await startBrowserProbe({ parentFolderToken: "synthetic_folder", run });
  t.after(async () => {
    probe.server.closeAllConnections();
    await new Promise((resolve) => probe.server.close(resolve));
  });
  const response = await fetch(probe.origin);
  assert.equal(response.headers.get("cache-control"), "no-store");
  const html = await response.text();
  const nonce = html.match(/name="nonce" value="([a-f0-9]{64})"/u)?.[1];
  assert.ok(nonce);
  const post = (body, origin = probe.origin, headers = {}) => fetch(probe.origin + "/probe", {
    method: "POST", redirect: "manual", headers: { Origin: origin, ...headers }, body,
  });
  return { ...probe, nonce, post };
}

test("loopback probe rejects foreign origins, hosts, bad nonces, duplicate tokens, and oversized forms", async (t) => {
  let calls = 0;
  const probe = await fixture(t, () => { calls += 1; });
  const form = () => new URLSearchParams({ nonce: probe.nonce, token: "synthetic-token" });
  assert.equal((await probe.post(form(), "https://untrusted.invalid")).status, 403);
  // Native fetch supplies its own Host header. Use HTTP to exercise this gate.
  const badHost = await new Promise((resolve, reject) => {
    const req = request(probe.origin, { headers: { Host: "untrusted.invalid" } }, (res) => {
      res.resume();
      res.on("end", () => resolve(res.statusCode));
    });
    req.on("error", reject);
    req.end();
  });
  assert.equal(badHost, 403);
  assert.equal((await probe.post(new URLSearchParams({ nonce: "wrong", token: "synthetic-token" }))).status, 403);
  const duplicate = form();
  duplicate.append("token", "another-token");
  assert.equal((await probe.post(duplicate)).status, 400);
  assert.equal((await probe.post(new URLSearchParams({ nonce: probe.nonce, token: "a".repeat(65_536) }))).status, 413);
  assert.equal(calls, 0);
  assert.equal(probe.status().state, "ready");
});

test("loopback probe runs once and does not return a token or raw failure text", async (t) => {
  let finish;
  const completed = new Promise((resolve) => { finish = resolve; });
  let calls = 0;
  const probe = await fixture(t, async (token, folder) => {
    calls += 1;
    assert.equal(token, "synthetic-secret-token");
    assert.equal(folder, "synthetic_folder");
    await completed;
    throw new Error("synthetic-secret-token must not be returned");
  });
  const form = () => new URLSearchParams({ nonce: probe.nonce, token: "synthetic-secret-token" });
  assert.equal((await probe.post(form())).status, 303);
  assert.equal((await probe.post(form())).status, 409);
  assert.equal(calls, 1);
  finish();
  await new Promise((resolve) => setImmediate(resolve));
  const body = await (await fetch(probe.origin)).text();
  assert.ok(body.includes("unexpected_failure"));
  assert.ok(!body.includes("synthetic-secret-token"));
  assert.ok(!body.includes('name="token"'));
  assert.equal(probe.status().state, "finished");
});

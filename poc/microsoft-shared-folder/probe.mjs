import { randomBytes } from "node:crypto";
import { authorize, validateGrant, safeAuthError } from "./auth.mjs";
import { CHUNK_BYTES, GraphClient, fileIdentity } from "./graph.mjs";
import { PocError, requireValue } from "./http.mjs";

export async function runProbe(config, {
  writeProbe = false, authorizeImpl = authorize, graphOptions,
  progress = () => {}, randomBytesImpl = randomBytes,
} = {}) {
  const sessions = [];
  const clients = [];
  const result = { verdict: "FAIL", mode: writeProbe ? "synthetic-writes" : "inspect",
    productionReady: false, stages: [], filesMayRemain: false, cleanup: "no_deletions" };
  const stages = result.stages;
  let root;
  let pendingName;
  try {
    for (const label of ["A", "B"]) {
      progress(`Sign in as test user ${label}.`);
      const session = await authorizeImpl(config);
      sessions.push(session);
      validateGrant(session.grant, config);
      const client = new GraphClient(session.grant.accessToken, graphOptions);
      clients.push(client);
      const identity = await client.call("me?$select=id");
      requireValue(identity.id === session.grant.account.localAccountId, "identity_mismatch");
      const renewed = await session.pca.acquireTokenSilent({ account: session.grant.account,
        authority: config.authority, scopes: config.scopes, forceRefresh: true });
      validateGrant(renewed, config, { ...session.grant.account, tenantId: session.grant.tenantId });
      client.token = renewed.accessToken;
      renewed.accessToken = "";
      session.grant.accessToken = "";
      requireValue((await client.call("me?$select=id")).id === identity.id, "identity_changed");
    }
    requireValue(sessions[0].grant.account.homeAccountId !== sessions[1].grant.account.homeAccountId
      && sessions[0].grant.account.localAccountId !== sessions[1].grant.account.localAccountId,
    "two_distinct_users_required");
    stages.push("two_distinct_users_and_forced_renewal");
    const [a] = clients;
    const resolved = await Promise.allSettled(clients.map((client) => client.resolveRoot(config.sharingUrl)));
    for (const entry of resolved) if (entry.status === "rejected") throw entry.reason;
    const roots = resolved.map((entry) => entry.value);
    requireValue(roots[0].id === roots[1].id && roots[0].driveId === roots[1].driveId
      && roots[0].driveType === roots[1].driveType, "shared_roots_differ");
    root = roots[0];
    result.driveType = root.driveType;
    stages.push("same_shared_folder_by_canonical_id");
    // Drain both read requests before token cleanup, including a failed listing.
    const initial = await Promise.allSettled(clients.map((client) => client.list(root)));
    for (const scan of initial) if (scan.status === "rejected") throw scan.reason;
    stages.push("both_users_can_list_folder");
    if (!writeProbe) {
      result.verdict = "PASS";
      return result;
    }

    const runId = Buffer.from(randomBytesImpl(12)).toString("hex");
    requireValue(/^[0-9a-f]{24}$/u.test(runId), "invalid_run_id");
    const prefix = `mallard-ms-poc-${runId}`;
    const names = ["a", "b", "claim"].map((suffix) => `${prefix}-${suffix}.bin`);
    for (const name of names) {
      for (const client of clients) requireValue(await client.getFile(root, name) === null, "probe_name_exists");
    }
    // Synthetic names are safe recovery hints. No remote IDs or links are emitted.
    result.probePrefix = prefix;
    const expected = new Map();
    for (let actor = 0; actor < 2; actor += 1) {
      const name = names[actor];
      const bytes = Buffer.from(`Synthetic Microsoft shared-folder POC ${runId}, user ${actor}.\n`);
      pendingName = name;
      result.filesMayRemain = true;
      const file = await clients[actor].create(root, name, bytes);
      pendingName = undefined;
      await clients[1 - actor].verify(root, file, bytes);
      expected.set(name, file);
    }
    stages.push("a_writes_b_verifies_and_b_writes_a_verifies");

    pendingName = names[2];
    const payloads = [Buffer.alloc(2 * CHUNK_BYTES, 0x41), Buffer.alloc(2 * CHUNK_BYTES, 0x42)];
    // Open both sessions before either commits; their final ranges compete.
    const uploads = [];
    for (const client of clients) uploads.push(await client.startUpload(root, pendingName));
    for (let i = 0; i < 2; i += 1) await clients[i].uploadChunk(uploads[i], payloads[i], 0, CHUNK_BYTES);
    const race = await Promise.allSettled(clients.map((client, index) =>
      client.uploadChunk(uploads[index], payloads[index], CHUNK_BYTES)));
    const winnerIndices = race.flatMap((entry, index) => entry.status === "fulfilled" ? [index] : []);
    for (const entry of race) {
      if (entry.status === "rejected" && !(entry.reason instanceof PocError && entry.reason.status === 409)) {
        throw entry.reason;
      }
    }
    requireValue(winnerIndices.length === 1, "create_race_did_not_have_one_winner");
    const winner = winnerIndices[0];
    const claim = fileIdentity(race[winner].value, root, pendingName, payloads[winner].length);
    const claimName = pendingName;
    pendingName = undefined;
    for (const client of clients) await client.verify(root, claim, payloads[winner]);
    stages.push("concurrent_uploads_one_winner_both_users_verify");
    expected.set(claimName, claim);

    pendingName = claimName;
    let conflict = false;
    try { await a.create(root, claimName, payloads[1 - winner]); }
    catch (error) {
      if (!(error instanceof PocError && error.status === 409)) throw error;
      conflict = true;
    }
    pendingName = undefined;
    requireValue(conflict, "existing_file_was_not_rejected");
    for (const client of clients) await client.verify(root, claim, payloads[winner]);
    stages.push("existing_name_rejected_winner_unchanged");

    const scans = [];
    for (const client of clients) {
      const scan = await client.list(root);
      const found = scan.items.filter((item) => item.name.startsWith(prefix));
      requireValue(found.length === expected.size, "missing_or_renamed_probe_file");
      for (const item of found) {
        const wanted = expected.get(item.name);
        requireValue(wanted && item.id === wanted.id && item.eTag === wanted.etag
          && item.size === wanted.size && !item.remoteItem && item.file && !item.folder,
        "listed_file_changed");
      }
      requireValue(scan.pages >= 2, "pagination_not_exercised");
      scans.push(scan.pages);
    }
    result.listingPages = scans;
    stages.push("both_users_see_all_probe_files_across_pages");
    result.verdict = "PASS";
  } catch (error) {
    result.failureCode = safeAuthError(error).code;
    if (pendingName && root && clients[1]) {
      // One read-only reconciliation; an absent item does not prove a failed write.
      // Never retry an upload or complete somebody else's session.
      try {
        const observed = await clients[1].getFile(root, pendingName);
        result.afterFailure = observed ? "item_observed_unqualified" : "item_not_observed_outcome_unknown";
      } catch { result.afterFailure = "read_failed_outcome_unknown"; }
    }
  } finally {
    for (const client of clients) client.token = "";
    for (const session of sessions) {
      session.grant.accessToken = "";
      try { await session.pca.clearCache(); }
      catch { result.verdict = "FAIL"; result.failureCode = "cache_cleanup_failed"; }
    }
  }
  return result;
}

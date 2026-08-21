import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  isCloudflareR2AccountId,
  isCloudflareR2BucketName,
  parseR2S3ApiUrl,
  parseR2Endpoint,
  r2EndpointForAccount,
  r2S3ApiUrl,
  storageConfigReady,
  storageNameForKind,
  StorageEditor,
} from "../../src/components/project-sync/StorageSettings";
import {
  agentSetupEditorAdoption,
  buildR2AgentSetupInstruction,
  createAgentSetupMarker,
  reconcileAgentSetup,
  reconcileStorageDraft,
  StorageRepositoryDetails,
  StorageRepositoryRow,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import StorageIndexPage from "../../src/components/project-sync/StorageIndexPage";
import {
  applyStorageDraft,
  shouldReloadConfigRevision,
} from "../../src/components/project-sync/ProjectSync";
import { storageAccessErrorMessage } from "../../src/components/project-sync/model";
import type { RemoteBundleSummary, StorageConfig } from "../../src/types";

const R2_ACCOUNT_ID = "0123456789abcdef0123456789abcdef";
const OTHER_R2_ACCOUNT_ID = "fedcba9876543210fedcba9876543210";

test("agent setup instructions contain only the name and resolved CLI path", () => {
  const instruction = buildR2AgentSetupInstruction({
    storageName: "Team backup",
    cliPath: "/Applications/Mallard.app/Contents/MacOS/mallard-cli",
  });
  assert.match(instruction, /Team backup/);
  assert.match(instruction, /\/Applications\/Mallard\.app\/Contents\/MacOS\/mallard-cli/);
  assert.match(instruction, /connect an existing bucket or provision a new one/);
  assert.match(instruction, /do not edit Mallard’s open setup form or JSON/);
  assert.match(instruction, /automatically populate the open setup page/);
  assert.match(instruction, /hidden prompt/);
  for (const secretValue of ["canary-access-key", "canary-secret-key", R2_ACCOUNT_ID, "private-bucket"]) {
    assert.equal(instruction.includes(secretValue), false);
  }
});

test("external config refresh adopts clean drafts and preserves dirty drafts", () => {
  const baseline = r2Storage({ name: "Before" });
  const incoming = r2Storage({ name: "After" });
  assert.deepEqual(reconcileStorageDraft(baseline, { ...baseline }, incoming), {
    kind: "adopt",
    storage: incoming,
  });
  assert.deepEqual(
    reconcileStorageDraft(baseline, { ...baseline, bucket: "local-edit" }, incoming),
    { kind: "conflict", reason: "changed" },
  );
  assert.deepEqual(reconcileStorageDraft(baseline, { ...incoming }, incoming), {
    kind: "adopt",
    storage: incoming,
  });
  assert.deepEqual(
    reconcileStorageDraft(baseline, { ...baseline, bucket: "local-edit" }, null),
    { kind: "conflict", reason: "removed" },
  );
  assert.deepEqual(reconcileStorageDraft(baseline, { ...baseline }, null), { kind: "close" });
});

test("agent setup reconciliation adopts one unchanged matching storage into the editor", () => {
  const existing = r2Storage({ id: "existing", name: "Existing" });
  const draft = r2Storage({
    id: "draft-client",
    name: " Team backup ",
    access_key_id: "canary-access-key",
    secret_access_key: "canary-secret-key",
  });
  const marker = createAgentSetupMarker(draft, [existing], draft.id);
  const markerWithDifferentCredentials = createAgentSetupMarker({
    ...draft,
    access_key_id: "different-access-key",
    secret_access_key: "different-secret-key",
  }, [existing], draft.id);
  const created = r2Storage({ id: "created-by-cli", name: "Team backup" });

  assert.equal(JSON.stringify(marker).includes("canary-access-key"), false);
  assert.equal(JSON.stringify(marker).includes("canary-secret-key"), false);
  assert.deepEqual(markerWithDifferentCredentials, marker);
  assert.deepEqual(reconcileAgentSetup(marker, { ...draft }, draft.id, [existing, created]), {
    kind: "adopt",
    storage: created,
  });
  const adoption = agentSetupEditorAdoption(created);
  assert.deepEqual(adoption.editingStorage, { projectId: "", storageId: created.id });
  assert.deepEqual(adoption.storageDraft, created);
  assert.deepEqual(adoption.storageBaseline, created);
  assert.notEqual(adoption.storageDraft, adoption.storageBaseline);
  assert.equal(adoption.storageDetailsExpanded, true);
  assert.equal(adoption.status, "Connected Team backup. Settings filled in automatically.");
});

test("agent setup reconciliation preserves a draft changed after copy", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, [], draft.id);
  const created = r2Storage({ id: "created-by-cli", name: "Team backup" });

  assert.deepEqual(
    reconcileAgentSetup(marker, { ...draft, bucket: "edited-locally" }, draft.id, [created]),
    { kind: "preserve", reason: "draft_changed" },
  );
});

test("agent setup reconciliation preserves a draft when no new storage matches", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, [], draft.id);

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    r2Storage({ id: "other", name: "Other storage" }),
  ]), { kind: "preserve", reason: "no_match" });
});

test("agent setup reconciliation preserves an ambiguous pair of new matches", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, [], draft.id);

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    r2Storage({ id: "first", name: "Team backup" }),
    r2Storage({ id: "second", name: "Team backup" }),
  ]), { kind: "preserve", reason: "ambiguous" });
});

test("agent setup reconciliation does not adopt a pre-existing same-name storage", () => {
  const existing = r2Storage({ id: "existing", name: "Team backup" });
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, [existing], draft.id);

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [existing]), {
    kind: "preserve",
    reason: "no_match",
  });
});

function r2Storage(patch: Partial<StorageConfig> = {}): StorageConfig {
  return {
    id: "storage-r2",
    name: "R2 storage",
    kind: "s3",
    bucket: "",
    access_key_id: "",
    secret_access_key: "",
    account_id: "",
    s3_endpoint: "",
    region: "auto",
    local_dir: "",
    ...patch,
  };
}

test("storage draft application rejects stale baselines and creation collisions", () => {
  const baseline = r2Storage({ id: "storage", name: "Before" });
  const draft = { ...baseline, name: "After" };
  assert.deepEqual(applyStorageDraft([baseline], draft, { ...baseline }), [draft]);

  assert.throws(
    () => applyStorageDraft([{ ...baseline, bucket: "external-change" }], draft, baseline),
    /changed outside the editor/,
  );
  assert.throws(
    () => applyStorageDraft([], draft, baseline),
    /changed outside the editor/,
  );
  assert.throws(
    () => applyStorageDraft([baseline], draft, null),
    /identifier was added outside the editor/,
  );
  assert.deepEqual(applyStorageDraft([], draft, null), [draft]);
});

test("config revision events recover invalid startup snapshots without reloading valid peers", () => {
  assert.equal(shouldReloadConfigRevision(5, 4, true), false);
  assert.equal(shouldReloadConfigRevision(5, 5, true), false);
  assert.equal(shouldReloadConfigRevision(5, 6, true), true);
  assert.equal(shouldReloadConfigRevision(0, 0, false), true);
});

test("R2 configuration requires a canonical endpoint and the four user-provided values", () => {
  assert.equal(
    r2EndpointForAccount(` ${R2_ACCOUNT_ID.toUpperCase()} `),
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
  );
  assert.equal(r2EndpointForAccount("0123456789abcdef"), "");
  assert.equal(storageConfigReady(r2Storage()), false);
  assert.equal(storageConfigReady(r2Storage({
    bucket: "agent-sync",
    account_id: R2_ACCOUNT_ID,
    s3_endpoint: `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    access_key_id: "access-key",
    secret_access_key: "secret-key",
  })), true);
  assert.equal(storageConfigReady(r2Storage({
    bucket: "agent-sync",
    account_id: R2_ACCOUNT_ID,
    s3_endpoint: `https://${OTHER_R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    access_key_id: "access-key",
    secret_access_key: "secret-key",
  })), false);
});

test("R2 connection failures use actionable storage language", () => {
  assert.equal(
    storageAccessErrorMessage(
      r2Storage(),
      "S3 get '.mallard/_storage.json': dispatch failure",
    ),
    "Couldn’t connect to Cloudflare R2. Check the S3 API URL, bucket name, Account ID, and credentials, then try again.",
  );
  assert.equal(
    storageAccessErrorMessage(r2Storage(), "remote bundle cursor did not advance"),
    "remote repo cursor did not advance",
  );
});

test("switching storage type updates only its generated default name", () => {
  assert.equal(storageNameForKind("Local storage 2", "s3"), "R2 storage 2");
  assert.equal(storageNameForKind("R2 storage 2", "local"), "Local storage 2");
  assert.equal(storageNameForKind("Team backup", "s3"), "Team backup");
});

test("R2 S3 API URLs round-trip every Cloudflare jurisdiction", () => {
  const cases = [
    { jurisdiction: "default" as const, hostnameLabel: "" },
    { jurisdiction: "eu" as const, hostnameLabel: ".eu" },
    { jurisdiction: "fedramp" as const, hostnameLabel: ".fedramp" },
  ];

  for (const { jurisdiction, hostnameLabel } of cases) {
    const endpoint = `https://${R2_ACCOUNT_ID}${hostnameLabel}.r2.cloudflarestorage.com`;
    const fullUrl = `${endpoint}/agent-sync`;
    assert.deepEqual(parseR2Endpoint(endpoint), {
      accountId: R2_ACCOUNT_ID,
      jurisdiction,
    });
    assert.deepEqual(parseR2S3ApiUrl(fullUrl), {
      accountId: R2_ACCOUNT_ID,
      bucket: "agent-sync",
      jurisdiction,
    });
    assert.equal(parseR2S3ApiUrl(`${fullUrl}/`), null);
    assert.equal(r2EndpointForAccount(R2_ACCOUNT_ID, jurisdiction), endpoint);
    assert.equal(r2S3ApiUrl(R2_ACCOUNT_ID, "agent-sync", endpoint), fullUrl);
  }
  assert.equal(
    r2S3ApiUrl(R2_ACCOUNT_ID, "agent-sync"),
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent-sync`,
  );
});

test("R2 endpoint helpers preserve jurisdiction while account and bucket values change", () => {
  const euEndpoint = `https://${R2_ACCOUNT_ID}.eu.r2.cloudflarestorage.com`;
  const parsed = parseR2Endpoint(euEndpoint);
  assert.ok(parsed);

  const changedAccountEndpoint = r2EndpointForAccount(OTHER_R2_ACCOUNT_ID, parsed.jurisdiction);
  assert.equal(
    changedAccountEndpoint,
    `https://${OTHER_R2_ACCOUNT_ID}.eu.r2.cloudflarestorage.com`,
  );
  assert.equal(
    r2S3ApiUrl(OTHER_R2_ACCOUNT_ID, "new-bucket", changedAccountEndpoint),
    `https://${OTHER_R2_ACCOUNT_ID}.eu.r2.cloudflarestorage.com/new-bucket`,
  );
  assert.equal(
    r2S3ApiUrl(R2_ACCOUNT_ID, "new-bucket", euEndpoint),
    `https://${R2_ACCOUNT_ID}.eu.r2.cloudflarestorage.com/new-bucket`,
  );
  assert.equal(r2S3ApiUrl(OTHER_R2_ACCOUNT_ID, "new-bucket", euEndpoint), "");
});

test("R2 validation enforces exact accounts, canonical hosts, and Cloudflare bucket names", () => {
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID), true);
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID.toUpperCase()), true);
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID.slice(1)), false);
  assert.equal(isCloudflareR2AccountId(`${R2_ACCOUNT_ID}0`), false);
  assert.equal(isCloudflareR2AccountId(`${R2_ACCOUNT_ID.slice(0, -1)}g`), false);

  for (const bucket of ["abc", "a-b", "a".repeat(63)]) {
    assert.equal(isCloudflareR2BucketName(bucket), true, bucket);
  }
  for (const bucket of [
    "ab",
    "a".repeat(64),
    "-agent",
    "agent-",
    "Agent",
    "agent.backup",
    "agent_backup",
    "agent backup",
  ]) {
    assert.equal(isCloudflareR2BucketName(bucket), false, bucket);
  }

  const validUrl = `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`;
  for (const invalidUrl of [
    `http://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`,
    `https://user@${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com:443/agent`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com:8443/agent`,
    `${validUrl}?list=true`,
    `${validUrl}#fragment`,
    `https://${R2_ACCOUNT_ID.slice(1)}.r2.cloudflarestorage.com/agent`,
    `https://${R2_ACCOUNT_ID.slice(0, -1)}g.r2.cloudflarestorage.com/agent`,
    `https://${R2_ACCOUNT_ID}.other.r2.cloudflarestorage.com/agent`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    `${validUrl}/nested`,
    ` ${validUrl}`,
    `${validUrl} `,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent%2Dsync`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/Agent`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/ab`,
    "https://example.com/agent",
  ]) {
    assert.equal(parseR2S3ApiUrl(invalidUrl), null, invalidUrl);
  }
  assert.equal(
    parseR2Endpoint(`https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/`),
    null,
  );
});

test("cloud storage editor keeps a one-line guide and states each example once", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage()}
      disabled={false}
      onChange={() => undefined}
    />,
  );

  assert.match(html, /Cloudflare R2/);
  // The four-part guide block is gone; one line carries the requirement, the
  // doc link, and the only unrecoverable fact.
  assert.match(html, /sync-r2-guide/);
  assert.match(html, /Object Read &amp; Write/);
  assert.match(html, /Setup guide/);
  assert.match(html, /Secret shown once/);
  assert.doesNotMatch(html, /Connect a Cloudflare R2 bucket/);
  assert.doesNotMatch(html, /Create an R2 bucket/);
  assert.match(html, /Bucket name/);
  assert.match(html, /Access Key ID/);
  assert.match(html, /Secret Access Key/);
  assert.match(html, /S3 API URL/);
  // Each example appears exactly once, as the placeholder — no doubled hint.
  assert.equal((html.match(/9cc0c910ec\*\*\*41511aca1\.r2\.cloudflarestorage\.com\/agent/g) ?? []).length, 1);
  assert.equal((html.match(/codex-sync-backups/g) ?? []).length, 1);
  assert.equal((html.match(/023e105f4ecef8ad9ca31a8372d0c353/g) ?? []).length, 1);
  assert.equal((html.match(/4a2b…91ef/g) ?? []).length, 1);
  assert.equal((html.match(/a1B2…x9Y0/g) ?? []).length, 1);
  assert.doesNotMatch(html, /-hint"/);
  // The secret can be shown while typing, and the empty form says so.
  assert.match(html, /sync-secret-reveal/);
  assert.match(html, />Show</);
  assert.match(html, /type="password"/);
  assert.match(html, /0 of 5 set/);
  assert.doesNotMatch(html, />S3 \/ R2</);
  assert.doesNotMatch(html, />Region</);
});

test("cloud storage editor displays the API URL from its stored jurisdictional endpoint", () => {
  const endpoint = `https://${R2_ACCOUNT_ID}.fedramp.r2.cloudflarestorage.com`;
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage({
        account_id: R2_ACCOUNT_ID,
        bucket: "agent-sync",
        s3_endpoint: endpoint,
      })}
      disabled={false}
      onChange={() => undefined}
    />,
  );

  assert.equal(html.includes(`value="${endpoint}/agent-sync"`), true);
  assert.equal(
    html.includes(`value="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent-sync"`),
    false,
  );
});

test("local storage keeps browse in the path row and save in the footer", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={{ ...r2Storage(), id: "storage-local", kind: "local", local_dir: "/tmp/storage" }}
      disabled={false}
      onChange={() => undefined}
      primaryAction={<button type="button">Save storage</button>}
    />,
  );

  assert.match(html, /<div><input[^>]*\/tmp\/storage[^>]*><button[^>]*>.*Browse…<\/button><\/div>/);
  assert.match(html, /sync-storage-editor-footer[^>]*><button type="button">Save storage<\/button>/);
  // The field counter belongs to the R2 branch; a single path needs no tally.
  assert.doesNotMatch(html, /of 5 set/);
  assert.doesNotMatch(html, /sync-storage-primary-action/);
});

test("a hoisted tally leaves the cloud editor with no footer band", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage()}
      disabled={false}
      onChange={() => undefined}
      showProgress={false}
    />,
  );

  assert.doesNotMatch(html, /of 5 set/);
  assert.doesNotMatch(html, /sync-storage-editor-footer/);
});

test("configured storage can move name editing into its page heading", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={{ ...r2Storage(), bucket: "agent" }}
      disabled={false}
      onChange={() => undefined}
      showNameField={false}
    />,
  );

  assert.doesNotMatch(html, /aria-label="Storage name"/);
  assert.match(html, /Cloudflare R2/);
});

test("the repositories heading is text-only", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  // Refresh moved to the path bar; the heading holds nothing but its name.
  assert.match(
    workspace,
    /className="sync-storage-repositories-heading">\s*<h2 id="storage-repositories-heading">Repositories<\/h2>\s*<\/div>/,
  );
});

test("repository rows spell their chips and fold full details behind the history chevron", () => {
  const bundle: RemoteBundleSummary = {
    bundle_id: "440ed684dbb6034cf32c3bdf04a8f0ea",
    display_name: "mallardInternal",
    revision: 3,
    resource_count: 9,
    updated_at: Date.now() - 29 * 60 * 1000,
    providers: ["codex", "claude"],
    repository_fingerprint: "3f9c1a77b2e845d06c1fa2b9d47e8350",
  };
  const html = renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />);

  assert.match(html, /sync-storage-repository-item/);
  assert.match(html, /sync-storage-repository-row/);
  assert.match(html, /mallardInternal/);
  // Chips spell their words — no Rev/Upd abbreviations anywhere on the row.
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Revision<\/span><b>3<\/b>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Updated<\/span><b>29m ago<\/b>/);
  assert.doesNotMatch(html, /<span>Rev<\/span>/);
  assert.doesNotMatch(html, /<span>Upd<\/span>/);
  // The id lives in the fold now: no stub on the collapsed row, and the fold
  // itself is closed until the history table's own toggle opens it.
  assert.doesNotMatch(html, /440ed684…a8f0ea/);
  assert.doesNotMatch(html, /sync-storage-repository-details/);
  assert.match(html, /sync-history-commit-toggle/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-controls="repo-details-440ed684dbb6034cf32c3bdf04a8f0ea"/);
  assert.match(html, /aria-label="Show details for mallardInternal"/);
});

test("repository rows wear the git folder icon exactly when a fingerprint marks a git remote", () => {
  const bundle: RemoteBundleSummary = {
    bundle_id: "440ed684dbb6034cf32c3bdf04a8f0ea",
    display_name: "mallardInternal",
    repository_fingerprint: "3f9c1a77b2e845d06c1fa2b9d47e8350",
  };
  const iconSvg = (html: string) =>
    html.match(/sync-storage-repository-icon"><svg.*?<\/svg>/)?.[0];

  const gitIcon = iconSvg(renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />));
  const plainIcon = iconSvg(renderToStaticMarkup(
    <StorageRepositoryRow bundle={{ ...bundle, repository_fingerprint: null }} />,
  ));
  assert.ok(gitIcon && plainIcon);
  assert.notEqual(gitIcon, plainIcon);
});

test("the details fold carries the complete id and skips absent facts", () => {
  const bundle: RemoteBundleSummary = {
    bundle_id: "440ed684dbb6034cf32c3bdf04a8f0ea",
    display_name: "mallardInternal",
    revision: 3,
    resource_count: 9,
    updated_at: 1_754_400_000,
    providers: ["codex", "claude"],
    repository_fingerprint: "3f9c1a77b2e845d06c1fa2b9d47e8350",
  };
  const html = renderToStaticMarkup(<StorageRepositoryDetails bundle={bundle} />);

  assert.match(html, /Repository ID<\/dt><dd><code>440ed684dbb6034cf32c3bdf04a8f0ea<\/code>/);
  assert.match(html, /<dt>Updated<\/dt>/);
  assert.match(html, /<dt>Resources<\/dt><dd>9<\/dd>/);
  assert.match(html, /<dt>Agents<\/dt><dd>Codex · Claude<\/dd>/);
  assert.match(html, /Fingerprint<\/dt><dd><code>3f9c1a77b2e845d06c1fa2b9d47e8350<\/code>/);

  // Absent facts don't render — no dash placeholders.
  const bare = renderToStaticMarkup(
    <StorageRepositoryDetails
      bundle={{ bundle_id: "440ed684dbb6034cf32c3bdf04a8f0ea", display_name: "mallardInternal" }}
    />,
  );
  assert.match(bare, /Repository ID/);
  assert.doesNotMatch(bare, /Updated|Resources|Agents|Fingerprint|—/);
});

test("the Storages index lists every storage as a row that opens its page", () => {
  const html = renderToStaticMarkup(
    <StorageIndexPage
      storages={[
        r2Storage({ id: "storage-local", name: "Local storage 1", kind: "local", local_dir: "/Users/test/storage" }),
        r2Storage({ bucket: "mallard-team" }),
      ]}
      links={[
        { local_project_id: "p1", storage_id: "storage-local", state: "active", revision: 1, updated_at: 1 },
        { local_project_id: "p2", storage_id: "storage-local", state: "active", revision: 1, updated_at: 1 },
      ] as never}
      storageIssues={{ "storage-r2": "Bucket unreachable" }}
      busy={false}
      appActions={[{ label: "Mallard", items: [{ id: "theme", label: "Switch theme", onSelect: () => undefined }] }]}
      onOpenStorage={() => undefined}
      onAddStorage={() => undefined}
    />,
  );

  // The bar is the page's whole chrome: a single root crumb that is also the
  // leaf, the one page verb, and the app scope behind ⋯.
  assert.match(html, /sync-pathbar sync-titlebar/);
  assert.match(html, /sync-path-crumb root leaf[^>]*aria-current="page"/);
  assert.match(html, /aria-label="Storages path"/);
  assert.match(html, /sync-path-sync-label">Add storage/);
  assert.match(html, /aria-label="App actions"/);

  // Rows: name, subtitle, linked-project tally; the issue marks its row.
  assert.match(html, /aria-label="Open Local storage 1"[\s\S]*?<strong>Local storage 1<\/strong><small>~\/storage<\/small>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>2<\/b>/);
  assert.match(html, /aria-label="Open R2 storage\. Storage issue: Bucket unreachable"/);
  assert.match(html, /sync-storage-index-warning/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>0<\/b>/);
});

test("the Storages index offers Add storage as its empty state", () => {
  const html = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onAddStorage={() => undefined}
    />,
  );

  assert.match(html, /profile-links-empty/);
  assert.match(html, /Add a storage to choose where projects sync\./);
  assert.doesNotMatch(html, /sync-storage-index-row/);
});

test("the storage group is skipped when it would frame nothing", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  // The group is a bordered box. Once the storage identity and the Push/Pull
  // buttons moved to the path bar, a settled project leaves it empty, and an
  // empty frame collapses into a rule above the history. It renders only when
  // it has a repair notice, a storage banner, an open review, an unlinked
  // project to prompt, or the link picker open.
  assert.match(workspace, /shouldShowConversationPathRepairNotice\(\s*conversationPathAudit,\s*conversationPathAuditError,\s*\)/);
  assert.match(workspace, /const storageGroupEmpty = !storageGroupNotice\s*&& !!selectedStorage\s*&& projectLinks\.length > 0\s*&& !storageBlockShown\s*&& linkingProjectId !== project\.local_project_id;/);
  assert.match(workspace, /if \(storageGroupEmpty\) return null;/);
  // …and the block inside it goes the same way when it has no banner to
  // carry — the sibling-repo convergence banner counts as one.
  assert.match(workspace, /if \(!activeNotice && !reviewOpen && !activeSibling\) return null;/);
});

test("the storage page is a rooted trail; the sidebar opens it and removes directly", () => {
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(sidebar, /onOpenStorage\(storage\.id, false\)/);
  assert.match(sidebar, /className="sidebar-profile-remove"[\s\S]*?onClick=\{\(\) => onRemoveStorage\(storage\.id\)\}/);
  assert.match(sidebar, /disabled=\{busy \|\| usage > 0\}/);
  assert.match(sidebar, /aria-label=\{`Remove \$\{storageName\} from Mallard`\}/);
  assert.match(sidebar, /<Icon name="trash" size=\{13\} \/>/);
  assert.doesNotMatch(sidebar, /storageMenuId|sidebar-storage-actions|Actions for \$\{storageName\}/);
  assert.doesNotMatch(sidebar, /onClearStorage/);

  // One route into a storage page, shared by every caller; every storage
  // screen suppresses the titlebar because the trail is the titlebar.
  assert.match(shell, /onOpenStorage=\{openStorageWorkspace\}/);
  assert.match(shell, /mode: "open",\s*storageId,\s*settingsExpanded,/);
  assert.match(shell, /onRemoveStorage=\{\(storageId\) => void removeStorage\(storageId\)\}/);
  assert.match(shell, /onClearStorage=\{\(storageId\) => void clearStorageData\(storageId\)\}/);
  assert.match(shell, /storageIndexOpen \? \(\s*<StorageIndexPage/);
  assert.match(shell, /\(!!activeSummary\s*\|\| !!activeCatalogScope\s*\|\| storageIndexOpen\s*\|\| settingsOpen\s*\|\| activeStorageSettingsId != null\)/);
  // Removing the storage whose page is open lands on the trail's root.
  assert.match(shell, /if \(activeStorageSettingsId === storageId\) setStorageIndexOpen\(true\);/);

  // The storage page: Storages / name ⌄ / Repositories, with the old header's
  // gear, pencil, and ✕ folded into the bar's single overflow.
  assert.match(workspace, /label: "Storages",\s*icon: "drive",\s*tone: "root"/);
  assert.match(workspace, /onActivate: onOpenStorageIndex/);
  assert.match(workspace, /label: "Repositories",\s*tone: "section"/);
  assert.match(workspace, /menuLabel: "Storage"/);
  // The crumb menu is a header naming the storage plus switch commands — the
  // current storage is the header block, never a checked row.
  assert.match(workspace, /menuHeader: \{\s*label: storageName,/);
  assert.match(workspace, /label: "Switch to"/);
  assert.doesNotMatch(workspace, /menuitemradio|checked: candidate\.id === editedStorageConfig\.id/);
  assert.match(workspace, /\{ id: "storage", label: "New storage" \}/);
  assert.match(workspace, /label: storageDetailsExpanded \? "Hide storage settings" : "Storage settings"/);
  assert.match(workspace, /label: "Rename storage…"/);
  assert.match(workspace, /label: "Clear storage data…"/);
  assert.match(workspace, /label: "Remove storage…"/);
  assert.match(workspace, /onClearStorage\(editedStorageConfig\.id\)/);
  assert.match(workspace, /onRemoveStorage\(editedStorageConfig\.id\)/);
  assert.match(workspace, /detail: compactProjectPath\(editedStorageConfig\.local_dir\)/);
  assert.match(workspace, /htmlFor="sync-storage-rename-input"/);
  assert.match(workspace, /`rename-storage:\$\{storageDraft\.id\}`/);
  assert.match(workspace, /const renamedStorage = \{ \.\.\.editedStorageConfig, name \}/);
  assert.match(workspace, /aria-label=\{`Refresh repositories in \$\{storageName\}`\}/);

  // The drawer still opens where the request says and commits from its own
  // footer; only the draft's create moved into the bar.
  assert.match(workspace, /setStorageDetailsExpanded\(storageEditorRequest\.settingsExpanded\)/);
  assert.match(
    workspace,
    /const openStorageEditorWithSettings = async[\s\S]*setStorageDetailsExpanded\(true\);/,
  );
  assert.match(workspace, /void openStorageEditorWithSettings\(workspaceStorage\);/);
  assert.match(workspace, /showStorageDetails \? " settings-expanded" : " settings-collapsed"/);
  assert.match(workspace, /hidden=\{!showStorageDetails\}/);
  assert.match(workspace, /\{showStorageDetails && \(/);
  assert.match(workspace, /setStorageDetailsExpanded\(false\);\s*await loadStorageBundles/);
  assert.match(workspace, /primaryAction=\{creatingStorage \? undefined : \(/);
  assert.match(workspace, /"Creating…" : "Create storage"/);

  // A draft commits from the bar under its own fields, the way the project
  // setup page finishes: the trail holds no action at all while creating, and
  // the R2 tally moves out of the card so it has one band, not two.
  assert.match(workspace, /const storagePathActions = creatingStorage \? undefined : \(/);
  assert.match(workspace, /showProgress=\{!creatingStorage\}/);
  assert.match(workspace, /\{creatingStorage && \(\s*<footer className="sync-storage-setup-footer">/);
  assert.match(workspace, /\{r2FieldsSet\(storageDraft, secretStored\)\} of 5 set/);
  assert.match(workspace, /className="btn btn-ghost"\s*disabled=\{busy\}\s*onClick=\{onOpenStorageIndex\}\s*>\s*Cancel/);
  assert.doesNotMatch(workspace, /sync-path-sync-label">\s*\{runningAction === `storage:/);

  // The header block the trail replaced stays gone.
  assert.doesNotMatch(workspace, /sync-project-heading/);
  assert.doesNotMatch(workspace, /sync-storage-settings-icon/);
  assert.doesNotMatch(workspace, /sync-storage-settings-actions/);
  assert.doesNotMatch(workspace, /StorageSettingsMeta/);
  assert.doesNotMatch(workspace, /Close storage settings/);

  // One home per scope: the project verbs ride the project crumb's own menu
  // (no path detail on Reveal — the menu header already shows it), and the
  // workspace ⋯ is left holding app scope alone.
  assert.match(workspace, /const pathOverflow: PathBarMenuGroup\[\] = workspaceProject \? appActions : \[\];/);
  assert.match(workspace, /label: "Reveal in Finder",\s*icon: "folder" as const/);
  assert.match(workspace, /label: "Agent profile…",\s*detail: workspaceProfileLabel/);
  assert.match(workspace, /label: "Rename project…"/);
  assert.match(workspace, /label: "Reset local sync state…"/);
  assert.match(workspace, /onResetProjectMetadata\(workspaceProject\.local_project_id\)/);
  assert.match(workspace, /isIrrecoverableSyncError\(activeNotice\.message\)/);
});

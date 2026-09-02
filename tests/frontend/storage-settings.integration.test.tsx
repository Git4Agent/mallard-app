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
  r2FieldsSet,
  r2S3ApiUrl,
  storageConfigReady,
  storageNameForKind,
  StorageEditor,
} from "../../src/components/project-sync/StorageSettings";
import {
  agentSetupEditorAdoption,
  buildR2AgentSetupInstruction,
  CatalogStorageRepositoryRow,
  createAgentSetupMarker,
  reconcileAgentSetup,
  reconcileStorageDraft,
  storageDraftPayload,
  StorageRepositoryDetails,
  StorageRepositoryRow,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import type { CatalogStorageRepository } from "../../src/components/storage-catalog/types";
import StorageIndexPage from "../../src/components/project-sync/StorageIndexPage";
import {
  applyStorageDraft,
  shouldReloadConfigRevision,
} from "../../src/components/project-sync/ProjectSync";
import { storageAccessErrorMessage } from "../../src/storageError";
import type { RemoteBundleSummary, StorageConfig, StorageSetupDraft } from "../../src/types";

const R2_ACCOUNT_ID = "0123456789abcdef0123456789abcdef";
const OTHER_R2_ACCOUNT_ID = "fedcba9876543210fedcba9876543210";

test("agent setup instructions keep UI, browser consent, and credentials outside the agent", () => {
  const instruction = buildR2AgentSetupInstruction({
    storageName: "Team backup",
    cliPath: "/Applications/Mallard.app/Contents/MacOS/mallard-cli",
    draftId: "draft-exact-123",
    expectedRevision: 7,
  });
  assert.match(instruction, /Team backup/);
  assert.match(instruction, /\/Applications\/Mallard\.app\/Contents\/MacOS\/mallard-cli/);
  assert.match(instruction, /storage r2 --help/);
  assert.match(instruction, /First check the required runtime dependency without installing anything/);
  assert.match(instruction, /wrangler --version/);
  assert.match(instruction, /confirm that `wrangler` is available on PATH/);
  assert.match(instruction, /If Wrangler is missing or `wrangler --version` fails/);
  assert.match(instruction, /stop before invoking Mallard setup/);
  assert.match(instruction, /ask me to approve installing it/);
  assert.match(instruction, /npm install --global wrangler@latest/);
  assert.match(instruction, /Do not run that installation unless I explicitly approve it/);
  assert.match(instruction, /Before an approved installation, run `npm --version`/);
  assert.match(instruction, /if npm is missing, report that separately/);
  assert.match(instruction, /do not choose or install a package manager on your own/);
  assert.match(instruction, /do not substitute `npx wrangler` or a project-local-only installation/);
  assert.match(instruction, /Once the Wrangler dependency check succeeds/);
  assert.match(instruction, /check whether your command-execution environment restricts writing to the current user’s ~\/\.mallard directory or launching the system browser/);
  assert.match(instruction, /ask me for one-time approval to run the exact, fully quoted Mallard setup command/);
  assert.match(instruction, /permission to write to the current user’s ~\/\.mallard directory and launch the system browser/);
  assert.match(instruction, /only for that exact setup command, not for a shell or a broader command prefix/);
  assert.match(instruction, /Do not invoke setup until I explicitly approve it; if I decline, stop/);
  assert.match(instruction, /storage r2 setup/);
  assert.match(instruction, /--name/);
  assert.match(instruction, /--draft-id/);
  assert.match(instruction, /draft-exact-123/);
  assert.match(instruction, /--expected-revision/);
  assert.match(instruction, /exact decimal value `7`/);
  assert.match(instruction, /exact storage name shown above as one literal argument/);
  assert.match(instruction, /do not interpret any value as shell syntax/);
  assert.match(instruction, /I—not the agent—will review and complete Cloudflare login, MFA, and consent/);
  assert.match(instruction, /Do not control the browser or inspect its contents/);
  assert.match(instruction, /observe only its success or failure result/);
  assert.match(instruction, /Do not work around an execution restriction by changing HOME/);
  assert.match(instruction, /using temporary or project-local Mallard state/);
  assert.match(instruction, /using sudo/);
  assert.match(instruction, /editing ~\/\.mallard/);
  assert.match(instruction, /editing Mallard’s open setup form/);
  assert.match(instruction, /Do not request or handle credentials/);
  assert.match(instruction, /put a secret in an argument or environment variable/);
  assert.match(instruction, /print credentials/);
  assert.match(instruction, /automatically fill the existing editable setup form/);
  assert.match(instruction, /only prepares the exact schema-1 draft/);
  assert.match(instruction, /does not test the connection or commit the storage/);
  assert.match(instruction, /CLI success or failure result is the end of the agent’s task/);
  assert.match(instruction, /Do not invoke Computer Use for any purpose, including read-only verification/);
  assert.match(instruction, /Do not use screenshots, browser automation, UI automation, or direct inspection of Mallard’s window or form/);
  assert.match(instruction, /Report only the CLI success or failure result, then stop/);
  assert.match(instruction, /I will confirm that the fields appeared/);
  assert.match(instruction, /personally click Finish setup/);
  assert.doesNotMatch(instruction, /Confirm that the fields appeared in the running app/);
  assert.ok(instruction.indexOf("Do not invoke Computer Use") < instruction.indexOf("wrangler --version"));
  assert.ok(instruction.indexOf("wrangler --version") < instruction.indexOf("storage r2 --help"));
  const helpIndex = instruction.indexOf("storage r2 --help");
  const executionApprovalIndex = instruction.indexOf("one-time approval");
  const setupIndex = instruction.indexOf("storage r2 setup");
  assert.ok(helpIndex < executionApprovalIndex);
  assert.ok(executionApprovalIndex < setupIndex);
  assert.doesNotMatch(instruction, /connect an existing bucket|provision|hidden prompt|bootstrap token/i);
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

test("renaming advances the baseline without dropping unsaved R2 fields", () => {
  const baseline = r2Storage({ id: "storage", name: "Before" });
  const dirtyDraft = {
    ...baseline,
    bucket: "unsaved-bucket",
    access_key_id: "unsaved-access-key",
    secret_access_key: "typed-secret",
  };
  const renamedBaseline = { ...baseline, name: "After" };
  const renamedDraft = { ...dirtyDraft, name: "After" };

  assert.equal(renamedDraft.bucket, "unsaved-bucket");
  assert.equal(renamedDraft.access_key_id, "unsaved-access-key");
  assert.equal(renamedDraft.secret_access_key, "typed-secret");
  assert.deepEqual(
    reconcileStorageDraft(renamedBaseline, renamedDraft, { ...renamedBaseline }),
    { kind: "unchanged" },
  );
});

test("agent setup reconciliation adopts only a newer revision of the exact draft", () => {
  const draft = r2Storage({
    id: "draft-client",
    name: " Team backup ",
    access_key_id: "canary-access-key",
    secret_access_key: "canary-secret-key",
  });
  const marker = createAgentSetupMarker(draft, "draft-exact", 4, draft.id, "persisted-storage");
  const markerWithDifferentCredentials = createAgentSetupMarker({
    ...draft,
    access_key_id: "different-access-key",
    secret_access_key: "different-secret-key",
  }, "draft-exact", 4, draft.id, "persisted-storage");
  const prepared = storageSetupDraft({
    draft_id: "draft-exact",
    revision: 5,
    secret_present: true,
    storage: r2Storage({
      id: "persisted-storage",
      name: "Team backup",
      bucket: "team-backup",
      account_id: R2_ACCOUNT_ID,
      s3_endpoint: `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
      access_key_id: "prepared-access-key",
      secret_access_key: "",
    }),
  });
  const sameNameOtherDraft = storageSetupDraft({
    draft_id: "draft-other",
    revision: 50,
    secret_present: true,
    storage: { ...prepared.storage, id: "other-client" },
  });

  assert.equal(JSON.stringify(marker).includes("canary-access-key"), false);
  assert.equal(JSON.stringify(marker).includes("canary-secret-key"), false);
  assert.deepEqual(markerWithDifferentCredentials, marker);
  assert.deepEqual(reconcileAgentSetup(marker, { ...draft }, draft.id, [sameNameOtherDraft, prepared]), {
    kind: "adopt",
    draft: prepared,
  });
  // The background list candidate stays redacted. Only the exact follow-up
  // read used for adoption contains the secret the user can reveal and edit.
  assert.equal(prepared.storage.secret_access_key, "");
  const revealedPrepared = {
    ...prepared,
    storage: { ...prepared.storage, secret_access_key: "prepared-secret-key" },
  };
  const adoption = agentSetupEditorAdoption(revealedPrepared);
  assert.deepEqual(adoption.editingStorage, { projectId: "", storageId: "persisted-storage" });
  assert.deepEqual(adoption.storageDraft, revealedPrepared.storage);
  assert.equal(adoption.storageBaseline, null);
  assert.equal(adoption.storageSetupDraftId, "draft-exact");
  assert.equal(adoption.storageSetupSecretPresent, true);
  assert.equal(adoption.storageSetupDraftRevision, 5);
  assert.equal(adoption.storageDetailsExpanded, true);
  assert.match(adoption.status, /Review or edit them/);
  assert.match(adoption.status, /finish setup yourself/);

  const adoptedHtml = renderToStaticMarkup(
    <StorageEditor
      storage={adoption.storageDraft}
      disabled={false}
      onChange={() => undefined}
      secretStored={true}
    />,
  );
  assert.match(adoptedHtml, new RegExp(`${R2_ACCOUNT_ID}\\.r2\\.cloudflarestorage\\.com/team-backup`));
  assert.match(adoptedHtml, /prepared-access-key/);
  assert.match(adoptedHtml, /type="password"/);
  assert.match(adoptedHtml, /value="prepared-secret-key"/);
  assert.match(adoptedHtml, />Show</);
});

test("agent setup reconciliation preserves a draft changed after copy", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, "draft-exact", 2, draft.id, "persisted-storage");
  const prepared = storageSetupDraft({
    draft_id: "draft-exact",
    revision: 3,
    secret_present: true,
    storage: readyR2Storage({ id: "persisted-storage", name: "Team backup" }),
  });

  assert.deepEqual(
    reconcileAgentSetup(marker, { ...draft, bucket: "edited-locally" }, draft.id, [prepared]),
    { kind: "preserve", reason: "draft_changed" },
  );
});

test("agent setup reconciliation waits for a newer revision of the exact draft", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, "draft-exact", 2, draft.id, "persisted-storage");

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, []), {
    kind: "preserve",
    reason: "no_update",
  });
  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    storageSetupDraft({ draft_id: "draft-exact", revision: 2, storage: draft }),
  ]), { kind: "preserve", reason: "no_update" });
});

test("agent setup reconciliation rejects an incomplete or identity-changing update", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, "draft-exact", 2, draft.id, "persisted-storage");

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    storageSetupDraft({ draft_id: "draft-exact", revision: 3, storage: draft }),
  ]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    storageSetupDraft({
      draft_id: "draft-exact",
      revision: 4,
      secret_present: true,
      storage: readyR2Storage({ id: "persisted-storage", name: "Team backup" }),
    }),
  ]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [
    storageSetupDraft({
      draft_id: "draft-exact",
      revision: 3,
      secret_present: true,
      storage: readyR2Storage({ id: "changed-client", name: "Team backup" }),
    }),
  ]), { kind: "preserve", reason: "invalid" });
});

test("agent setup reconciliation never falls back to another same-name draft", () => {
  const draft = r2Storage({ id: "draft-client", name: "Team backup" });
  const marker = createAgentSetupMarker(draft, "draft-exact", 2, draft.id, "persisted-storage");

  assert.deepEqual(reconcileAgentSetup(marker, draft, draft.id, [storageSetupDraft({
    draft_id: "different-draft",
    revision: 3,
    secret_present: true,
    storage: readyR2Storage({ id: "persisted-storage", name: "Team backup" }),
  })]), {
    kind: "preserve",
    reason: "no_update",
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

function readyR2Storage(patch: Partial<StorageConfig> = {}): StorageConfig {
  return r2Storage({
    bucket: "team-backup",
    access_key_id: "prepared-access-key",
    account_id: R2_ACCOUNT_ID,
    s3_endpoint: `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    ...patch,
  });
}

function storageSetupDraft(patch: Partial<StorageSetupDraft> = {}): StorageSetupDraft {
  return {
    draft_id: "draft-exact",
    secret_present: false,
    storage: r2Storage({ id: "draft-client" }),
    revision: 1,
    created_at: 1,
    updated_at: 1,
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

test("R2 readiness accepts either a typed secret or a backend-stored secret", () => {
  const configured = r2Storage({
    bucket: "agent-sync",
    account_id: R2_ACCOUNT_ID,
    s3_endpoint: `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    access_key_id: "access-key",
  });
  assert.equal(storageConfigReady(configured, false), false);
  assert.equal(storageConfigReady({ ...configured, secret_access_key: "typed-secret" }, false), true);
  assert.equal(storageConfigReady(configured, true), true);
  assert.equal(storageConfigReady({ ...configured, name: "  " }, true), false);
  assert.equal(storageConfigReady({
    ...configured,
    s3_endpoint: `https://${OTHER_R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
  }, true), false);
});

test("R2 progress counts typed and backend-stored secrets", () => {
  const configured = r2Storage({
    bucket: "agent-sync",
    account_id: R2_ACCOUNT_ID,
    s3_endpoint: `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
    access_key_id: "access-key",
  });
  assert.equal(r2FieldsSet(r2Storage(), false), 0);
  assert.equal(r2FieldsSet(configured, false), 4);
  assert.equal(r2FieldsSet({ ...configured, secret_access_key: "typed-secret" }, false), 5);
  assert.equal(r2FieldsSet(configured, true), 5);
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
    assert.deepEqual(parseR2Endpoint(endpoint), { accountId: R2_ACCOUNT_ID, jurisdiction });
    assert.deepEqual(parseR2S3ApiUrl(fullUrl), {
      accountId: R2_ACCOUNT_ID,
      bucket: "agent-sync",
      jurisdiction,
    });
    assert.equal(parseR2S3ApiUrl(`${fullUrl}/`), null);
    assert.equal(r2EndpointForAccount(R2_ACCOUNT_ID, jurisdiction), endpoint);
    assert.equal(r2S3ApiUrl(R2_ACCOUNT_ID, "agent-sync", endpoint), fullUrl);
  }
});

test("R2 validation rejects noncanonical accounts, hosts, and bucket names", () => {
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID), true);
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID.toUpperCase()), true);
  assert.equal(isCloudflareR2AccountId(R2_ACCOUNT_ID.slice(1)), false);
  assert.equal(isCloudflareR2AccountId(`${R2_ACCOUNT_ID.slice(0, -1)}g`), false);

  for (const bucket of ["abc", "a-b", "a".repeat(63)]) {
    assert.equal(isCloudflareR2BucketName(bucket), true, bucket);
  }
  for (const bucket of ["ab", "a".repeat(64), "-agent", "agent-", "Agent", "agent.backup"]) {
    assert.equal(isCloudflareR2BucketName(bucket), false, bucket);
  }

  const validUrl = `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`;
  for (const invalidUrl of [
    `http://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`,
    `https://user@${R2_ACCOUNT_ID}.r2.cloudflarestorage.com/agent`,
    `https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com:443/agent`,
    `${validUrl}?list=true`,
    `${validUrl}/nested`,
    ` ${validUrl}`,
    "https://example.com/agent",
  ]) {
    assert.equal(parseR2S3ApiUrl(invalidUrl), null, invalidUrl);
  }
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

test("storage type switching is locked after commit and clears fresh connection drafts", () => {
  const editor = readFileSync("src/components/project-sync/StorageSettings.tsx", "utf8");

  assert.match(editor, /const storageTypeLocked = disabled \|\| secretStored \|\| !showNameField;/);
  assert.match(editor, /name: storageNameForKind\(storage\.name, "s3"\),\s*kind: "s3",\s*bucket: "",\s*access_key_id: "",\s*secret_access_key: "",\s*account_id: "",\s*s3_endpoint: "",\s*region: "auto",\s*local_dir: "",\s*supports_conditional_writes: undefined,/);
  assert.match(editor, /name: storageNameForKind\(storage\.name, "local"\),\s*kind: "local",\s*bucket: "",\s*access_key_id: "",\s*secret_access_key: "",\s*account_id: "",\s*s3_endpoint: "",\s*region: "",\s*supports_conditional_writes: undefined,/);
  assert.equal((editor.match(/disabled=\{storageTypeLocked\}/g) ?? []).length, 2);
});

test("new cloud storage supports agent fill or controlled manual entry", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage()}
      disabled={false}
      onChange={() => undefined}
      secretStored={false}
      agentSlot={<button type="button">Run with Claude Code</button>}
    />,
  );

  assert.match(html, /Cloudflare R2/);
  // Two named routes replace the one run-on guide sentence: the agent lane
  // seats the hand-off cluster, the manual lane introduces the fields.
  assert.match(html, /sync-r2-route-row agent/);
  assert.match(html, /Agent setup/);
  assert.match(html, /Fills these values for your review/);
  assert.match(html, /you complete Cloudflare sign-in, MFA, and consent in the browser/);
  assert.match(html, /Run with Claude Code/);
  assert.match(html, /Manual setup/);
  assert.match(html, /Enter an existing R2 connection in the fields below\./);
  assert.match(html, /Setup guide/);
  assert.doesNotMatch(html, /sync-r2-guide"/);
  assert.match(html, /S3 API URL/);
  assert.match(html, /Bucket name/);
  assert.match(html, /Account ID/);
  assert.match(html, /Access Key ID/);
  assert.match(html, /Secret Access Key/);
  assert.equal((html.match(/class="field-help"/g) ?? []).length, 5);
  assert.match(html, /0 of 5 set/);
  assert.match(html, /type="password"/);
  assert.match(html, /sync-secret-reveal/);
  assert.match(html, />Show</);
  assert.doesNotMatch(html, /readonly|Stored securely|canary-secret|Create storage/);
});

test("configured cloud storage keeps public values editable and the backend secret redacted", () => {
  const endpoint = `https://${R2_ACCOUNT_ID}.fedramp.r2.cloudflarestorage.com`;
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage({
        account_id: R2_ACCOUNT_ID,
        bucket: "agent-sync",
        s3_endpoint: endpoint,
        access_key_id: "key-id",
      })}
      disabled={false}
      onChange={() => undefined}
      secretStored={true}
      showNameField={false}
    />,
  );

  assert.match(html, /Configured\./);
  assert.match(html, /review or update the connection values below/);
  assert.match(html, /S3 API URL/);
  assert.equal(html.includes(`${R2_ACCOUNT_ID}.fedramp.r2.cloudflarestorage.com/agent-sync`), true);
  assert.match(html, /Bucket name/);
  assert.match(html, /Account ID/);
  assert.match(html, /Access Key ID/);
  assert.match(html, /Secret Access Key/);
  assert.match(html, /Stored — type to replace/);
  assert.doesNotMatch(html, /readonly/);
  assert.match(html, /type="password"/);
  assert.match(html, />Show</);
  assert.match(html, /5 of 5 set/);
  assert.doesNotMatch(html, /secret-key|Set up with an agent/);
});

test("local storage keeps browse in the path row and save in the footer", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={{ ...r2Storage(), id: "storage-local", kind: "local", local_dir: "/tmp/storage" }}
      disabled={false}
      onChange={() => undefined}
      secretStored={false}
      primaryAction={<button type="button">Save storage</button>}
    />,
  );

  assert.match(html, /<div><input[^>]*\/tmp\/storage[^>]*><button[^>]*>.*Browse…<\/button><\/div>/);
  assert.match(html, /sync-storage-editor-footer[^>]*><button type="button">Save storage<\/button>/);
  // The field counter belongs to the R2 branch; a single path needs no tally.
  assert.doesNotMatch(html, /of 5 set/);
  assert.doesNotMatch(html, /sync-storage-primary-action/);
});

test("a hoisted R2 tally leaves the editor without its own footer", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage()}
      disabled={false}
      onChange={() => undefined}
      secretStored={false}
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
      secretStored={true}
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

test("Catalog kind repositories join the listing in the project row's grammar", () => {
  const repo: CatalogStorageRepository = {
    repository_id: "library-3a91f04c77d2b8e6a5c31f9d04b7e28c",
    provider: "codex",
    resource_kind: "agent_skill",
    revision: 10,
    updated_at: Math.floor(Date.now() / 1000) - 29 * 60,
    resource_names: ["changelog-writer", "deploy-checklist", "triage"],
    error: null,
  };
  const html = renderToStaticMarkup(<CatalogStorageRepositoryRow repo={repo} />);

  // Same item, row, chips, and fold affordance as a project repository; the
  // identity is the kind icon and the scope-label name one level up.
  assert.match(html, /sync-storage-repository-item/);
  assert.match(html, /sync-storage-repository-row/);
  assert.match(html, /Skills · Codex/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Revision<\/span><b>10<\/b>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Updated<\/span><b>29m ago<\/b>/);
  assert.match(html, /sync-history-commit-toggle/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-controls="catalog-repo-details-library-3a91f04c77d2b8e6a5c31f9d04b7e28c"/);
  // The fold is closed until opened; no id stub or contents on the row.
  assert.doesNotMatch(html, /sync-storage-repository-details/);
  assert.doesNotMatch(html, /changelog-writer/);
  assert.doesNotMatch(html, /Can’t be read/);

  // The workspace lists Catalog rows after the project ones, loads them with
  // the same gate, and keeps the empty line honest.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /storageCatalogApi\.listStorageRepositories\(storageId\)/);
  assert.match(workspace, /<CatalogStorageRepositoryRow key=\{repo\.repository_id\} repo=\{repo\} \/>/);
  // The empty line stays honest, and the two listings stay independent: rows
  // that loaded still render when their sibling request failed, and a failure
  // suppresses only the "nothing here" sentence.
  assert.match(
    workspace,
    /orderedRemoteBundles\.length === 0 && orderedCatalogRepos\.length === 0 \? \(/,
  );
  assert.match(workspace, /bundleError \|\| catalogRepoError \? null : \(/);
});

test("an unreadable Catalog repository states its refusal instead of chips", () => {
  const repo: CatalogStorageRepository = {
    repository_id: "library-e29a51c8d40b76f3a1c58e02b9d47f61",
    provider: "codex",
    resource_kind: "codex_pet_v1",
    revision: null,
    updated_at: null,
    resource_names: [],
    error: "library tag does not match its provider and kind derivation",
  };
  const html = renderToStaticMarkup(<CatalogStorageRepositoryRow repo={repo} />);

  assert.match(html, /Pets v1 · Codex/);
  assert.match(html, /sync-storage-repo-unreadable[^>]*>Can’t be read/);
  assert.match(html, /sync-storage-repository-icon danger/);
  // No chips on a refusal row — the phrase takes their place — and the
  // backend's sentence waits in the fold.
  assert.doesNotMatch(html, /<span>Revision<\/span>/);
  assert.doesNotMatch(html, /<span>Updated<\/span>/);
  assert.doesNotMatch(html, /provider and kind derivation/);
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
      drafts={[{
        draft_id: "storagedraft-0123456789abcdef01234567",
        secret_present: true,
        storage: r2Storage({ id: "storage-drafted", name: "Half-typed R2" }),
        revision: 2,
        created_at: 1,
        updated_at: 2,
      }]}
      busy={false}
      appActions={[{ label: "Mallard", items: [{ id: "theme", label: "Switch theme", onSelect: () => undefined }] }]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );

  // The bar is the page's whole chrome: a single root crumb that is also the
  // leaf, the one page verb, and the app scope behind ⋯.
  assert.match(html, /sync-pathbar sync-titlebar/);
  assert.match(html, /sync-path-crumb root leaf[^>]*aria-current="page"/);
  assert.match(html, /aria-label="Storages path"/);
  assert.match(html, /landing-picker-head[\s\S]*?class="btn btn-primary"[^>]*>[\s\S]*?Add storage…/);
  assert.match(html, /aria-label="App actions"/);

  // Cards: name, location, linked-project tally; the issue marks its card.
  assert.match(html, /aria-label="Open Local storage 1"[\s\S]*?<strong>Local storage 1<\/strong>[\s\S]*?landing-picker-card-location">~\/storage</);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>2<\/b>/);
  assert.match(html, /aria-label="Open R2 storage\. Storage issue: Bucket unreachable"/);
  assert.match(html, /sync-storage-index-warning/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>0<\/b>/);

  // A persisted setup draft is a card too: badged, tally-less, opening its
  // draft — and never leaking a secret (the view arrives redacted).
  assert.match(html, /aria-label="Continue setting up Half-typed R2 — resumable storage draft"/);
  assert.match(html, /<strong>Half-typed R2<\/strong>[\s\S]*?landing-picker-card-location">Setup not finished — not connected to Mallard yet</);
  assert.match(html, /sync-draft-badge">Draft/);
  assert.equal((html.match(/sync-storage-repo-chip/g) ?? []).length, 2);
});

test("the Storages index says what is missing and offers Add storage, with no explainer", () => {
  const html = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      drafts={[]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );

  assert.match(html, /landing-picker-empty/);
  assert.match(html, /No storages yet/);
  assert.match(html, /Add storage…/);
  assert.doesNotMatch(html, /Add a storage to choose|landing-picker-card"/);

  // A draft alone is content: the empty state yields to its row.
  const draftsOnly = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      drafts={[{
        draft_id: "storagedraft-0123456789abcdef01234567",
        secret_present: false,
        storage: r2Storage({ id: "storage-drafted", name: "" }),
        revision: 1,
        created_at: 1,
        updated_at: 1,
      }]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );
  assert.doesNotMatch(draftsOnly, /landing-picker-empty/);
  assert.match(draftsOnly, /<strong>\(unnamed\)<\/strong>/);
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
  assert.match(shell, /\(!!activeSummary\s*\|\| !!activeCatalogScope\s*\|\| catalogIndexOpen\s*\|\| storageIndexOpen\s*\|\| settingsOpen\s*\|\| activeStorageSettingsId != null\)/);
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
  assert.match(workspace, /if \(saved === false\) return;\s*setStorageDraft\(\(current\) => current\?\.id === renamedStorage\.id \? \{ \.\.\.current, name \} : current\);[\s\S]*setStorageBaseline\(\{ \.\.\.renamedStorage \}\);/);
  assert.match(workspace, /aria-label=\{`Refresh repositories in \$\{storageName\}`\}/);

  // The drawer still opens where the request says and existing local or R2
  // settings commit from the footer they own.
  assert.match(workspace, /setStorageDetailsExpanded\(storageEditorRequest\.settingsExpanded\)/);
  assert.match(
    workspace,
    /const openStorageEditorWithSettings = async[\s\S]*setStorageDetailsExpanded\(true\);/,
  );
  assert.match(workspace, /void openStorageEditorWithSettings\(workspaceStorage\);/);
  assert.match(workspace, /showStorageDetails \? " settings-expanded" : " settings-collapsed"/);
  assert.match(workspace, /hidden=\{!showStorageDetails\}/);
  assert.match(workspace, /\{showStorageDetails && \(/);
  assert.match(workspace, /setStorageDetailsExpanded\(false\);\s*await loadStorageBundles\(savedStorage\)/);
  assert.match(workspace, /primaryAction=\{creatingStorage \? undefined : \(/);
  assert.match(workspace, /"Creating…" : "Create storage"/);
  assert.match(workspace, /if \(saved === false\) return;[\s\S]*storageSnapshot\.kind === "s3"[\s\S]*secret_access_key: ""/);
  assert.match(workspace, /setStorageDraft\(\{ \.\.\.savedStorage \}\);\s*setStorageBaseline\(\{ \.\.\.savedStorage \}\);/);
  assert.match(workspace, /await loadStorageBundles\(savedStorage\)/);

  // A draft commits from the bar under its fields. R2 keeps its tally and the
  // optional agent/browser handoff alongside the controlled manual form.
  assert.match(workspace, /const storagePathActions = creatingStorage \? undefined : \(/);
  assert.match(workspace, /showProgress=\{!creatingStorage\}/);
  assert.match(workspace, /\{creatingStorage && \(\s*<footer className="sync-storage-setup-footer">/);
  assert.match(workspace, /\{r2FieldsSet\(storageDraft, secretStored\)\} of 5 set/);
  assert.match(workspace, /className="btn btn-ghost"\s*disabled=\{busy\}\s*onClick=\{onOpenStorageIndex\}\s*aria-label=\{storageSetupDraftId \? "Close setup; the draft is kept" : "Close setup"\}\s*>\s*Cancel/);
  assert.doesNotMatch(workspace, /\{storageDraft\.kind === "local" && \(\s*<button/);
  assert.match(workspace, /Run with \{defaultAgentLauncher\.launcher\.display_name\}/);
  assert.match(workspace, /"Finishing…" : "Finish setup"/);
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
  // The failed-action notice still routes an irrecoverable failure to the reset
  // path rather than to a retry that cannot succeed. It reads the message off a
  // local now that the notice is one item in the storage card's zone.
  assert.match(workspace, /const failedMessage = activeNotice\.message;/);
  assert.match(workspace, /isIrrecoverableSyncError\(failedMessage\)/);
});

test("the draft payload is the form minus the server-owned fields", () => {
  const payload = storageDraftPayload(r2Storage({
    name: "Half-typed",
    bucket: "team",
    secret_access_key: "typed-secret",
    supports_conditional_writes: true,
  }));
  assert.deepEqual(payload, {
    name: "Half-typed",
    kind: "s3",
    bucket: "team",
    access_key_id: "",
    secret_access_key: "typed-secret",
    account_id: "",
    s3_endpoint: "",
    region: "auto",
    local_dir: "",
  });
  assert.equal("id" in payload, false);
  assert.equal("supports_conditional_writes" in payload, false);
});

test("the setup form autosaves a persisted draft and the close funnel flushes it", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  // The recipe is project setup's: a 700ms debounce armed by edits, created
  // lazily on the first one, cancelled (not flushed) when identity changes.
  assert.match(workspace, /const STORAGE_DRAFT_AUTOSAVE_DELAY_MS = 700;/);
  assert.match(workspace, /if \(creatingStorage\) draftDirtyRef\.current = true;/);
  assert.match(workspace, /const draftId = storageSetupDraftIdRef\.current;\s*const saved = draftId\s*\? await projectSyncApi\.updateStorageSetupDraft\(\s*draftId,\s*storageSetupDraftRevisionRef\.current,\s*storageDraftPayload\(storage\),\s*\)\s*: await projectSyncApi\.createStorageSetupDraft\(storageDraftPayload\(storage\)\);/);
  assert.match(workspace, /\}, STORAGE_DRAFT_AUTOSAVE_DELAY_MS\);\s*return \(\) => window\.clearTimeout\(timer\);/);
  // Only server-owned fields come back; the editor key must not move.
  assert.match(workspace, /storageSetupDraftIdRef\.current = saved\.draft_id;\s*storageSetupDraftRevisionRef\.current = saved\.revision;\s*setStorageSetupDraftId\(saved\.draft_id\);\s*setStorageSetupSecretPresent\(saved\.secret_present\);/);

  // Every in-app exit passes the close request; it flushes best-effort and
  // keeps the file. The resume request rehydrates including the secret flag.
  assert.match(workspace, /if \(storageEditorRequest\.mode === "close"\) \{\s*bundleRequestRef\.current \+= 1;\s*flushStorageSetupDraft\(\);/);
  assert.match(workspace, /if \(storageEditorRequest\.mode === "resume-draft"\) \{/);
  assert.match(workspace, /setStorageSetupDraftId\(draft\.draft_id\);\s*storageSetupDraftIdRef\.current = draft\.draft_id;\s*setStorageSetupSecretPresent\(draft\.secret_present\);\s*storageSetupDraftRevisionRef\.current = draft\.revision;/);

  // A resumed draft's server-held secret counts as stored, so the gate and
  // the tally read "set" without the value ever crossing IPC.
  assert.match(workspace, /const secretStored = !creatingStorage \|\| storageSetupSecretPresent;/);

  // Only the user's successful Finish action retires the R2 draft. Before
  // that action, the exact current snapshot is serialized behind autosave and
  // all three pin values from the save response cross into ProjectSync.
  assert.match(workspace, /const savedDraft = await persistStorageSetupDraftForAction\(\s*storageSnapshot,\s*session,\s*editVersion,\s*"finishing setup",\s*\);/);
  assert.match(workspace, /setupDraftId = savedDraft\.draft_id;\s*setupDraftRevision = savedDraft\.revision;\s*setupDraftStorageId = savedDraft\.storage\.id;/);
  assert.match(workspace, /const saved = await onSaveStorage\(\s*storageSnapshot,\s*baselineSnapshot,\s*setupDraftId,\s*setupDraftRevision,\s*setupDraftStorageId,\s*\);/);
  assert.match(workspace, /if \(creatingStorage\) resetStorageSetupDraftState\(\);/);
  assert.match(workspace, /const adoption = agentSetupEditorAdoption\(revealedReconciliation\.draft\);/);
  assert.match(workspace, /setStorageBaseline\(adoption\.storageBaseline\);/);
  assert.match(workspace, /setStorageSetupDraftId\(adoption\.storageSetupDraftId\);/);
  assert.match(workspace, /setStorageSetupSecretPresent\(adoption\.storageSetupSecretPresent\);/);
  assert.doesNotMatch(workspace, /CLI committed the storage|loadStorageBundles\(reconciliation\.storage\)/);

  // Discard is the page's verb, behind a native confirm.
  assert.match(workspace, /Discard the storage draft for \$\{storageDraft\.name \|\| "\(unnamed\)"\}\?/);
  assert.match(workspace, /This removes only Mallard’s local setup draft\./);
  assert.match(workspace, /It does not delete any Cloudflare bucket or access key that may have been created during preparation\./);
  assert.match(workspace, /The selected folder and its contents are not changed\./);
  assert.match(workspace, /Discard draft…/);
});

test("agent setup cluster is registry-driven with launch, repeatable copy, and cancel", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const editor = readFileSync("src/components/project-sync/StorageSettings.tsx", "utf8");

  // Run buttons derive from provider descriptors carrying a CLI launcher;
  // the cluster never holds a fixed agent pair.
  assert.match(workspace, /providers\.flatMap\(\(descriptor\) => \(\s*descriptor\.cli_launcher/);
  assert.match(workspace, /className="sync-r2-split"/);
  // The visible half runs the session's preferred agent; picking another from
  // the menu makes it the face. The menu always ends in the copy route.
  assert.match(workspace, /setPreferredLauncher\(key\);\s*void launchAgentSetup\(key, launcher\.display_name\);/);
  assert.match(workspace, /launchR2AgentSetup\(provider, instruction\)/);
  assert.match(workspace, /Copy agent instruction/);
  // Both waiting states can be backed out of, and a landed copy stays
  // repeatable instead of dissolving into the listening line.
  assert.match(workspace, /is running the setup — finish Cloudflare sign-in when the browser opens…/);
  assert.match(workspace, /\{copyAgainFlashed \? "Copied" : "Copy again"\}/);
  assert.match(workspace, /onClick=\{cancelAgentSetup\}>Cancel<\/button>/);
  // An edit kills the launched hand-off exactly like the listening one.
  assert.match(workspace, /current\?\.kind === "listening" \|\| current\?\.kind === "launched" \|\| current\?\.kind === "ready"/);
  // The editor names the two routes for new drafts; a settled storage keeps
  // the one-line configured bar.
  assert.match(editor, /Agent setup<\/span>/);
  assert.match(editor, /Manual setup<\/span>/);
  assert.match(editor, /Configured\. You can review or update the connection values below\./);
});

test("agent handoff pins and flushes the exact open draft before copying", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(workspace, /const draftSaveQueueRef = useRef<Promise<void>>\(Promise\.resolve\(\)\);/);
  assert.match(workspace, /const saved = await persistStorageSetupDraft\(draftSnapshot, session\);/);
  assert.match(workspace, /draftId: saved\.draft_id,\s*expectedRevision: saved\.revision,/);
  assert.match(workspace, /createAgentSetupMarker\(\s*draftSnapshot,\s*saved\.draft_id,\s*saved\.revision,/);
  assert.match(workspace, /draftSnapshot\.id,\s*saved\.storage\.id,/);
  assert.match(workspace, /setupDrafts\.find\(\(\{ draft_id \}\) => draft_id === marker\.setupDraftId\)/);
  assert.match(workspace, /candidate\.revision <= marker\.setupDraftRevision/);
  assert.match(workspace, /candidate\.revision !== marker\.setupDraftRevision \+ 1/);
  assert.match(workspace, /\|\| !storageConfigReady\(candidate\.storage, candidate\.secret_present\)/);
  assert.match(workspace, /setAgentNotice\(\{ kind: "ready", message: adoption\.status \}\);/);
  assert.match(workspace, /storageSetupDrafts,\s*\);/);
  assert.match(workspace, /const revealed = await projectSyncApi\.getStorageSetupDraft\(marker\.setupDraftId\);/);
  assert.match(workspace, /agentSetupMarkerRef\.current !== marker\s*\|\| draftSessionRef\.current !== session\s*\|\| draftEditVersionRef\.current !== editVersion/);
  assert.match(workspace, /reconcileAgentSetup\(marker, formSnapshot, editingDraftId, \[revealed\]\)/);
  assert.match(workspace, /!revealed\.secret_present\s*\|\| !revealed\.storage\.secret_access_key/);
  assert.match(workspace, /agentSetupEditorAdoption\(revealedReconciliation\.draft\)/);
});

test("Test connection pins the saved snapshot without committing and its verdict dies with the next edit", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  // The button shares Create's readiness gate and busy plumbing; the label
  // flips while the probe runs.
  assert.match(workspace, /const testStorageConnection = \(\) => run\(`test-storage:\$\{editedStorageConfig\.id\}`/);
  assert.match(workspace, /disabled=\{busy \|\| !!storageConflict \|\| !storageReady \|\| !!runningAction\}/);
  assert.match(workspace, /"Testing…" : "Test connection"/);
  assert.match(workspace, /\{storageDraft\.kind === "s3" && \(/);
  // The verdict is a status line, cleared by edits, saves, and test starts.
  assert.match(workspace, /\{storageTestVerified && \(\s*<span className="sync-r2-guide-connected" role="status">/);
  assert.match(workspace, /Connection verified/);
  assert.match(workspace, /setStorageTestVerified\(false\);\s*setStorageDraftActionError\(null\);\s*if \(creatingStorage\) draftDirtyRef\.current = true;\s*setStorageDraft\(nextDraft\);/);

  // Test is gated by the same queued, awaited snapshot persistence as Finish.
  // A concurrent edit or failed save surfaces locally and prevents the probe.
  assert.match(workspace, /const savedDraft = await persistStorageSetupDraftForAction\(\s*storageSnapshot,\s*session,\s*editVersion,\s*"testing the connection",\s*\);/);
  assert.match(workspace, /if \(!savedDraft\) return;\s*const verified = await onTestStorage\(\s*storageSnapshot,\s*savedDraft\.draft_id,\s*savedDraft\.revision,\s*savedDraft\.storage\.id,\s*\);/);
  assert.match(workspace, /The storage form changed while Mallard was preparing it for \$\{action\}/);
  assert.match(workspace, /Mallard could not save the current storage draft before \$\{action\}/);

  // The shell's handler is the save path minus the commit: same URL guard,
  // fresh config revision, and the full draft pin rides along whenever the
  // action follows the form's exact awaited draft save. The visible form
  // secret is deliberately blanked in this request so the backend consumes
  // the pinned draft copy rather than trusting a second credential copy.
  assert.match(shell, /const testInlineStorage = async \(/);
  assert.match(shell, /Enter a valid Cloudflare R2 S3 API URL before testing\./);
  assert.match(shell, /const usesPinnedDraft = !!setupDraftId\s*&& setupDraftRevision != null\s*&& !!setupDraftStorageId;/);
  assert.match(shell, /projectSyncApi\.testR2Storage\(\{\s*storage_id: null,\s*draft_id: usesPinnedDraft \? setupDraftId : null,\s*draft_revision: usesPinnedDraft \? setupDraftRevision : null,\s*draft_storage_id: usesPinnedDraft \? setupDraftStorageId : null,/);
  assert.match(shell, /secret_access_key: usesPinnedDraft \? "" : storage\.secret_access_key,/);
  assert.match(shell, /onTestStorage=\{testInlineStorage\}/);
  // Finish uses the same all-or-none pinning rule. Existing-storage edits and
  // a hypothetical manual call with no draft retain their direct-secret path.
  assert.match(shell, /const usesPinnedDraft = !expectedBaseline\s*&& !!setupDraftId\s*&& setupDraftRevision != null\s*&& !!setupDraftStorageId;/);
  assert.match(shell, /draft_id: usesPinnedDraft \? setupDraftId : null,\s*draft_revision: usesPinnedDraft \? setupDraftRevision : null,\s*draft_storage_id: usesPinnedDraft \? setupDraftStorageId : null,/);
});

test("storage draft rows reach the index and the shell resumes them by fetching first", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  assert.match(shell, /const \[storageSetupDrafts, setStorageSetupDrafts\] = useState<StorageSetupDraft\[\]>\(\[\]\);/);
  assert.match(shell, /const refreshStorageSetupDrafts = async/);
  assert.match(shell, /Promise\.allSettled\(\[[\s\S]*refreshStorageSetupDrafts\(requestId\),/);
  assert.match(shell, /drafts=\{storageSetupDrafts\}/);
  assert.match(shell, /onOpenStorageDraft=\{\(draftId\) => void openStorageSetupDraft\(draftId\)\}/);
  // The fetch happens in the shell so the workspace request handler stays
  // synchronous; a missing draft reports and refreshes instead of opening.
  assert.match(shell, /const openStorageSetupDraft = async \(draftId: string\)/);
  assert.match(shell, /This storage draft no longer exists\./);
  assert.match(shell, /mode: "resume-draft",\s*draft: loaded,/);
  // A successful local-kind create retires the draft frontend-side.
  assert.match(shell, /await projectSyncApi\.discardStorageSetupDraft\(setupDraftId\);/);
});

test("storage drafts ride the sidebar rail like project drafts do", () => {
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  // The Storages pane lists drafts after real storages: same row recipe as
  // project drafts (badge, open-on-click, trash discards only the draft),
  // and a draft alone defeats the pane's empty state.
  assert.match(sidebar, /\{storages\.length === 0 && storageDrafts\.length === 0 \? \(/);
  assert.match(sidebar, /\{storageDrafts\.map\(\(draft\) => \{/);
  assert.match(sidebar, /sync-sidebar-draft\$\{activeStorageId === draft\.storage\.id \? " active" : ""\}/);
  assert.match(sidebar, /onClick=\{\(\) => onOpenStorageDraft\(draft\.draft_id\)\}/);
  assert.match(sidebar, /aria-label=\{`Continue setting up \$\{draftName\} — resumable storage draft`\}/);
  assert.match(sidebar, /aria-label=\{`Discard the storage draft for \$\{draftName\}`\}/);
  assert.match(sidebar, /Cloudflare bucket and access-key resources are kept/);
  assert.match(sidebar, /the selected folder and its contents are kept/);
  assert.doesNotMatch(sidebar, /nothing was created in the storage/);
  assert.match(sidebar, /<span className="sync-draft-badge">Draft<\/span>/);

  // The shell wires the rail and owns discard: confirm first, and when the
  // discarded draft's form is open (the workspace reports its storage id as
  // the active editor) the editor closes with it.
  assert.match(shell, /storageDrafts=\{storageSetupDrafts\}/);
  assert.match(shell, /onDiscardStorageDraft=\{\(draftId\) => void discardStorageSetupDraft\(draftId\)\}/);
  assert.match(shell, /This removes only Mallard’s local setup draft\./);
  assert.match(shell, /It does not delete any Cloudflare bucket or access key that may have been created during preparation\./);
  assert.match(shell, /The selected folder and its contents are not changed\./);
  assert.match(shell, /if \(draft && activeStorageSettingsId === draft\.storage\.id\) \{\s*setStorageEditorRequest\(\(current\) => \(\{\s*mode: "close",/);
  assert.match(shell, /Storage draft discarded\./);
});

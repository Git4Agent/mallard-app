import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  parseR2S3ApiUrl,
  r2EndpointForAccount,
  r2S3ApiUrl,
  storageConfigReady,
  StorageEditor,
} from "../../src/components/project-sync/StorageSettingsV3";
import {
  StorageRepositoryRow,
  StorageSettingsMeta,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import type { RemoteBundleSummary, StorageConfigV3 } from "../../src/types";

function r2Storage(patch: Partial<StorageConfigV3> = {}): StorageConfigV3 {
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
    included_default_exclusions: [],
    ...patch,
  };
}

test("R2 configuration derives the endpoint and requires the four user-provided values", () => {
  assert.equal(
    r2EndpointForAccount(" 0123456789abcdef "),
    "https://0123456789abcdef.r2.cloudflarestorage.com",
  );
  assert.equal(storageConfigReady(r2Storage()), false);
  assert.equal(storageConfigReady(r2Storage({
    bucket: "agent-sync",
    account_id: "0123456789abcdef",
    access_key_id: "access-key",
    secret_access_key: "secret-key",
  })), true);
});

test("R2 S3 API URLs map to and from an Account ID and bucket", () => {
  const accountId = "0123456789abcdef0123456789abcdef";
  const fullUrl = `https://${accountId}.r2.cloudflarestorage.com/agent`;
  assert.deepEqual(parseR2S3ApiUrl(fullUrl), {
    accountId,
    bucket: "agent",
  });
  assert.deepEqual(parseR2S3ApiUrl(`${fullUrl}/`), {
    accountId,
    bucket: "agent",
  });
  assert.equal(
    r2S3ApiUrl(accountId, "agent"),
    fullUrl,
  );
  assert.equal(parseR2S3ApiUrl("https://example.com/agent"), null);
  assert.equal(parseR2S3ApiUrl(`${fullUrl}/nested`), null);
});

test("cloud storage editor exposes the guided R2 form instead of generic S3 fields", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={r2Storage()}
      disabled={false}
      onChange={() => undefined}
    />,
  );

  assert.match(html, /Cloudflare R2/);
  assert.match(html, /Connect a Cloudflare R2 bucket/);
  assert.doesNotMatch(html, /v3-r2-guide-icon/);
  assert.match(html, /Create an R2 bucket/);
  assert.match(html, /Create R2 credentials/);
  assert.match(html, /Find your Account ID/);
  assert.match(html, /Bucket name/);
  assert.match(html, /Access Key ID/);
  assert.match(html, /Secret Access Key/);
  assert.match(html, /S3 API URL/);
  assert.match(html, /9cc0c910ec\*\*\*41511aca1\.r2\.cloudflarestorage\.com\/agent/);
  assert.match(html, /codex-sync-backups/);
  assert.match(html, /023e…c353/);
  assert.match(html, /4a2b…91ef/);
  assert.match(html, /a1B2…x9Y0/);
  assert.doesNotMatch(html, />S3 \/ R2</);
  assert.doesNotMatch(html, />Region</);
});

test("local storage keeps browse and save actions in the path row", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      storage={{ ...r2Storage(), id: "storage-local", kind: "local", local_dir: "/tmp/storage" }}
      disabled={false}
      onChange={() => undefined}
      primaryAction={<button type="button">Save storage</button>}
    />,
  );

  assert.match(html, /<div><input[^>]*\/tmp\/storage[^>]*><button[^>]*>.*Browse…<\/button><button[^>]*>Save storage<\/button><\/div>/);
  assert.doesNotMatch(html, /v3-storage-primary-action/);
});

test("repository rows keep metadata behind an icon disclosure", () => {
  const bundle: RemoteBundleSummary = {
    bundle_id: "440ed684dbb6034cf32c3bdf04a8f0ea",
    display_name: "mallardInternal",
    generation: 3,
    resource_count: 9,
    updated_at: Date.now() - 29 * 60 * 1000,
  };
  const html = renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />);

  assert.match(html, /<details[^>]*v3-storage-repository-row/);
  assert.match(html, /Show details for mallardInternal/);
  assert.match(html, /v3-storage-repository-details-icon/);
  assert.match(html, />Repository ID</);
  assert.match(html, />Generation</);
  assert.match(html, />Resources</);
  assert.match(html, />Updated</);
  assert.doesNotMatch(html, /Generation 3/);
});

test("storage settings identify the destination with its storage icon", () => {
  const localHtml = renderToStaticMarkup(
    <StorageSettingsMeta
      storage={{
        ...r2Storage(),
        id: "storage-local",
        name: "Local storage 1",
        kind: "local",
        local_dir: "/Users/test/storage",
      }}
    />,
  );
  const cloudHtml = renderToStaticMarkup(
    <StorageSettingsMeta storage={r2Storage()} />,
  );

  assert.match(localHtml, /v3-storage-settings-meta/);
  assert.match(localHtml, /title="Local folder: Local storage 1"/);
  assert.match(localHtml, />Local storage 1</);
  assert.match(cloudHtml, /title="Cloudflare R2: R2 storage"/);
});

test("configured storage opens repository-first with details folded", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(workspace, /setStorageDetailsExpanded\(!storageConfigReady\(storage\)\)/);
  assert.match(workspace, /storageDetailsExpanded \? "Hide settings" : "Edit settings"/);
  assert.match(workspace, /aria-expanded=\{storageDetailsExpanded\}/);
  assert.match(workspace, /showStorageDetails \? " settings-expanded" : " settings-collapsed"/);
  assert.match(workspace, /hidden=\{!showStorageDetails\}/);
  assert.match(workspace, /\{showStorageDetails && \(/);
  assert.match(workspace, /setStorageDetailsExpanded\(false\);\s*await loadStorageBundles/);
});

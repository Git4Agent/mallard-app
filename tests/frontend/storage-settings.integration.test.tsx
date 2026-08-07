import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  parseR2S3ApiUrl,
  r2EndpointForAccount,
  r2S3ApiUrl,
  storageConfigReady,
  storageNameForKind,
  StorageEditor,
} from "../../src/components/project-sync/StorageSettings";
import {
  StorageRepositoryDetails,
  StorageRepositoryRow,
  StorageSettingsMeta,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import { storageAccessErrorMessage } from "../../src/components/project-sync/model";
import type { RemoteBundleSummary, StorageConfig } from "../../src/types";

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

  assert.match(localHtml, /sync-storage-settings-meta/);
  assert.match(localHtml, /title="Local folder: Local storage 1"/);
  assert.match(localHtml, />Local storage 1</);
  assert.match(cloudHtml, /title="Cloudflare R2: R2 storage"/);
});

test("the storage sidebar opens the overview folded and the gear opens settings", () => {
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(sidebar, /onClick=\{\(\) => onOpenStorage\(storage\.id, false\)\}/);
  assert.match(sidebar, /onClick=\{\(\) => onOpenStorage\(storage\.id, true\)\}/);
  assert.match(shell, /onOpenStorage=\{\(storageId, settingsExpanded\) => \{/);
  assert.match(shell, /mode: "open",\s*storageId,\s*settingsExpanded,/);
  assert.match(workspace, /setStorageDetailsExpanded\(storageEditorRequest\.settingsExpanded\)/);
  assert.match(
    workspace,
    /const openStorageEditorWithSettings = async[\s\S]*setStorageDetailsExpanded\(true\);/,
  );
  assert.match(workspace, /void openStorageEditorWithSettings\(storage\);/);
  assert.match(workspace, /aria-expanded=\{storageDetailsExpanded\}/);
  assert.match(workspace, /aria-label=\{storageDetailsExpanded \? "Hide storage settings" : "Edit storage settings"\}/);
  assert.match(workspace, /showStorageDetails \? " settings-expanded" : " settings-collapsed"/);
  assert.match(workspace, /hidden=\{!showStorageDetails\}/);
  assert.match(workspace, /\{showStorageDetails && \(/);
  assert.match(workspace, /setStorageDetailsExpanded\(false\);\s*await loadStorageBundles/);
  assert.match(workspace, /className="sync-project-heading-name"/);
  assert.match(workspace, /sync-project-heading-meta-item sync-project-heading-path/);
  assert.match(workspace, /sync-project-heading-meta-item sync-project-heading-agent/);
  assert.match(workspace, /<Icon name="folder" size=\{12\} \/>/);
  assert.match(workspace, /<Icon name="terminal" size=\{12\} \/>/);
  assert.match(workspace, /aria-label="Rename storage"/);
  assert.match(workspace, /aria-label="Rename project"/);
  assert.match(workspace, /<Icon name="edit" size=\{13\} \/>/);
  assert.match(workspace, /`rename-storage:\$\{storageDraft\.id\}`/);
  assert.match(workspace, /const renamedStorage = \{ \.\.\.editedStorageConfig, name \}/);
});

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleSnapshotSummary,
  SiblingBundleStatus,
  StorageSummary,
} from "../../src/types";
import SiblingRepoNotice from "../../src/components/project-sync/SiblingRepoNotice";
import BundleConnectionDialog from "../../src/components/project-sync/BundleConnectionDialog";

const storage: StorageSummary = {
  storage_id: "storage-convergence",
  name: "Local storage 1",
  provider: "local",
  config_version: 1,
  provider_label: "Local folder",
  icon: "drive",
  available: true,
  summary: [{ field_id: "directory", label: "Folder", value: "/tmp/storage", is_path: true }],
};

function summary(overrides: Partial<BundleSnapshotSummary>): BundleSnapshotSummary {
  return {
    storage_id: storage.storage_id,
    bundle_id: "19e9afda78c650335f05301a3c0d09e9",
    display_name: "HorrorGame",
    revision: 1,
    updated_at: 1_787_298_240,
    resource_count: 2,
    registration_only: false,
    repository_fingerprint: "a".repeat(64),
    ...overrides,
  };
}

function renderNotice(status: SiblingBundleStatus) {
  return renderToStaticMarkup(
    <SiblingRepoNotice status={status} busy={false} onConnect={() => undefined} />,
  );
}

test("the sibling notice names the repo and offers Connect for every split state", () => {
  const content = renderNotice({ state: "sibling_found", matches: [summary({})] });
  assert.match(content, /already has this repository/);
  assert.match(content, /HorrorGame/);
  assert.match(content, /carries updates this link cannot see/);
  assert.match(content, /Connect…/);

  const registered = renderNotice({
    state: "sibling_found",
    matches: [summary({ registration_only: true, resource_count: 0 })],
  });
  assert.match(registered, /registered this repository here without pushing yet/);

  const twin = renderNotice({
    state: "registration_twin",
    survivor: summary({ registration_only: true, resource_count: 0 }),
  });
  assert.match(twin, /registered for this repository at the same time/);
  assert.match(twin, /Connect…/);

  assert.equal(renderNotice({ state: "none" }), "");
});

test("the connect dialog carries dedicated sibling copy and a quiet dismissal", () => {
  const html = renderToStaticMarkup(
    <BundleConnectionDialog
      projectName="HorrorGame (Claude)"
      currentBundleId="b751caac4627e21d5809e0cf925a7c1f"
      projectFingerprint={"a".repeat(64)}
      storage={storage}
      matches={[summary({})]}
      reason="sibling"
      busy={false}
      error={null}
      onCancel={() => undefined}
      onUseExisting={() => undefined}
      onKeepCurrent={() => undefined}
    />,
  );
  assert.match(html, /Connect to this repository(&#x27;|')s repo/);
  assert.match(html, /already has this repository in another repo/);
  assert.match(html, /Not now/);
  assert.match(html, /Recommended/, "the fingerprint match stays recommended");
});

test("refresh wires sibling detection and the workspace banner into the connect flow", () => {
  const projectSync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(projectSync, /getSiblingBundleStatus\(projectId, storageId\)/);
  assert.match(projectSync, /reason: "sibling"/);
  assert.match(projectSync, /siblingNotice=\{siblingNotice\}/);

  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /<SiblingRepoNotice/);

  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(api, /get_sibling_bundle_status/);
});

test("the setup repository choice is always explicit once a storage is chosen", () => {
  const setup = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  // The section renders for any chosen storage, not only when matches exist.
  assert.match(setup, /const showRepositoryChoice = Boolean\(chosenStorageId\)/);
  // An empty storage gets the explicit creation statement instead of
  // silence, rendered as the section's own selected option card.
  assert.match(setup, /Creates “\$\{draft\.display_name\}” in/);
  assert.match(setup, /listRemoteBundleSnapshots\(chosenStorageId\)/);
});
